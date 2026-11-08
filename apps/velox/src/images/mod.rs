mod run;

use std::rc::Rc;

use bande_app_menu::Entry;
use bande_l10n::{text, tr, trn};
use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Button, Cell, Column, Label, Menu, MenuItem, ProgressBar, Row, Table, TextField, Ui, Widget,
    WidgetId,
};

use crate::engine::{Image, Task};
use crate::resources::{
    Act, Acts, Banded, Pane, Scope, Strip, age, boxed, confirm, matches, push, search, size, tell,
    tool,
};
use crate::ui::dot;

pub const PULL: &str = "pull-image";
pub const RUN: &str = "run-image";
const PULL_NOW: &str = "pull-now";
const REMOVE: &str = "image-remove";
const PRUNE: &str = "image-prune";
const COPY_REFERENCE: &str = "image-copy-reference";
const COPY_ID: &str = "image-copy-id";
const PULL_ROW: &str = "pulling";
const PRUNE_SYMBOL: &str = include_str!("../../data/symbols/prune.svg");
const REMOVE_SYMBOL: &str = include_str!("../../data/symbols/remove.svg");
const RUN_SYMBOL: &str = include_str!("../../data/symbols/start.svg");
const REPOSITORY_WIDTH: f32 = 180.0;
const TAG_WIDTH: f32 = 110.0;
const ARCH_WIDTH: f32 = 70.0;
const IN_USE_WIDTH: f32 = 60.0;
const CREATED_WIDTH: f32 = 130.0;
const SIZE_WIDTH: f32 = 96.0;

pub fn opens_images(command: &str) -> bool {
    command == PULL
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Bar {
    Field,
    Pulling(String, String, Option<u32>),
}

#[derive(Default)]
pub struct Images {
    table: Option<WidgetId>,
    strip: Option<WidgetId>,
    field: Option<WidgetId>,
    tools: Vec<(WidgetId, bool)>,
    search: Option<WidgetId>,
    bar: Option<Bar>,
    selected: Vec<String>,
    filter: String,
    text: String,
    acts: Option<Acts>,
}

fn split(reference: &str) -> (&str, &str) {
    match reference.rsplit_once(':') {
        Some((repository, tag)) if !tag.contains('/') => (repository, tag),
        _ => (reference, ""),
    }
}

fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

fn arch(palette: &Palette, image: &Image) -> Cell {
    let mut all = image.architectures.iter();
    match (all.next(), all.next()) {
        (None, _) => Cell::new("—").muted(),
        (Some(_), Some(_)) => Cell::new(tr!("multi")),
        (Some(arch), None) if arch == host_arch() => Cell::new(arch.as_str()),
        (Some(arch), None) => Cell::new(arch.as_str()).badge(palette.warning),
    }
}

fn rows(scope: &Scope, filter: &str) -> Vec<Row> {
    let mut rows: Vec<(bool, String, String, Row)> = Vec::new();
    for image in &scope.store.images {
        let used = scope
            .containers
            .values()
            .any(|container| container.image_id == image.id);
        let tags = image.tags();
        let references: Vec<(String, &str, &str)> = match tags.is_empty() {
            true => vec![(image.id.clone(), "<none>", "<none>")],
            false => tags
                .iter()
                .map(|tag| {
                    let (repository, name) = split(tag);
                    ((*tag).to_owned(), repository, name)
                })
                .collect(),
        };
        for (key, repository, tag) in references {
            if !matches(filter, &[repository, tag, image.short_id()]) {
                continue;
            }
            let in_use = match used {
                true => Cell::new("").icon(dot(scope.palette.success, true)),
                false => Cell::new(""),
            };
            let row = Row::new(
                key,
                vec![
                    Cell::new(repository).cut_in_middle(),
                    Cell::new(tag).muted(),
                    arch(scope.palette, image),
                    in_use,
                    Cell::new(age(image.created)).muted(),
                    Cell::new(size(u64::try_from(image.size).unwrap_or(0))).tabular(),
                ],
            );
            rows.push((
                repository == "<none>",
                repository.to_lowercase(),
                tag.to_lowercase(),
                row,
            ));
        }
    }
    rows.sort_by(|left, right| (left.0, &left.1, &left.2).cmp(&(right.0, &right.1, &right.2)));
    let mut rows: Vec<Row> = rows.into_iter().map(|(_, _, _, row)| row).collect();
    if let Some((reference, pulling)) = &scope.store.pulling {
        let percent = pulling
            .fraction
            .map(|fraction| format!("{}%", (fraction * 100.0).round()))
            .unwrap_or_default();
        let (repository, tag) = split(reference);
        rows.insert(
            0,
            Row::new(
                PULL_ROW,
                vec![
                    Cell::new(repository).tinted(scope.palette.accent),
                    Cell::new(tag).muted(),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(pulling.status.as_str()).muted(),
                    Cell::new(percent).tabular(),
                ],
            ),
        );
    }
    rows
}

impl Images {
    fn references(&self) -> Vec<String> {
        self.selected
            .iter()
            .filter(|key| *key != PULL_ROW)
            .cloned()
            .collect()
    }

    fn bar_widgets(&self, bar: &Bar) -> (Vec<Box<dyn Widget>>, Option<WidgetId>) {
        let Some(acts) = &self.acts else {
            return (Vec::new(), None);
        };
        match bar {
            Bar::Field => {
                let typed = Rc::clone(acts);
                let field = TextField::new(
                    &tr!("Pull image (e.g. nginx:latest)"),
                    &self.text,
                    move |text| {
                        push(&typed, Act::Text(text.to_owned()));
                    },
                );
                let id = field.id();
                let pull = Rc::clone(acts);
                let button = Button::new(&tr!("Pull"), move || {
                    push(&pull, Act::Command(PULL_NOW));
                })
                .primary();
                (vec![boxed(field), boxed(button)], Some(id))
            }
            Bar::Pulling(reference, status, percent) => {
                let label = match percent {
                    Some(percent) => tr!(
                        "Pulling {reference}… {percent}%",
                        reference = reference.clone(),
                        percent = (*percent)
                    ),
                    None => tr!(
                        "Pulling {reference}: {status}",
                        reference = reference.clone(),
                        status = status.clone()
                    ),
                };
                let value = percent.map_or(0.0, |percent| percent as f32 / 100.0);
                (
                    vec![
                        boxed(ProgressBar::new(&label, Some(value))),
                        boxed(Label::new(&label).secondary()),
                    ],
                    None,
                )
            }
        }
    }

    fn remove(&self, ui: &mut Ui, acts: &Acts, references: Vec<String>) {
        if references.is_empty() {
            return;
        }
        let title = match references.as_slice() {
            [only] => tr!("Remove “{name}”?", name = only.clone()),
            _ => tr!("Remove {count} images?", count = references.len()),
        };
        confirm(
            ui,
            acts,
            &title,
            &tr!("An image still used by a container cannot be removed."),
            &tr!("Remove"),
            Task::RemoveImages(references),
        );
    }

    fn image_id(scope: &Scope, key: &str) -> String {
        scope
            .store
            .images
            .iter()
            .find(|image| image.id == key || image.tags().contains(&key))
            .map_or_else(|| key.to_owned(), |image| image.id.clone())
    }
}

impl Pane for Images {
    fn build(&mut self, acts: &Acts, palette: &Palette) -> (Vec<Box<dyn Widget>>, Box<dyn Widget>) {
        self.acts = Some(Rc::clone(acts));
        let prune = tool(
            PRUNE_SYMBOL,
            text!("Prune Unused Images"),
            palette,
            acts,
            Act::Command(PRUNE),
        );
        let remove = tool(
            REMOVE_SYMBOL,
            text!("Remove Selected"),
            palette,
            acts,
            Act::Command(REMOVE),
        );
        let run = tool(RUN_SYMBOL, text!("Run…"), palette, acts, Act::Command(RUN));
        let field = search(text!("Filter images"), &self.filter, acts);
        self.tools = vec![(run.id(), true), (prune.id(), false), (remove.id(), true)];
        self.search = Some(field.id());
        let select = Rc::clone(acts);
        let menu_acts = Rc::clone(acts);
        let table = Table::new(vec![
            Column::new(&tr!("Repository"), REPOSITORY_WIDTH)
                .flexible()
                .unsortable(),
            Column::new(&tr!("Tag"), TAG_WIDTH).unsortable(),
            Column::new(&tr!("Arch"), ARCH_WIDTH).unsortable(),
            Column::new(&tr!("In Use"), IN_USE_WIDTH).unsortable(),
            Column::new(&tr!("Created"), CREATED_WIDTH).unsortable(),
            Column::new(&tr!("Size"), SIZE_WIDTH).numeric().unsortable(),
        ])
        .on_selection(move |keys| push(&select, Act::Select(keys.to_vec())))
        .context_menu(
            |keys| {
                let keys: Vec<&String> = keys.iter().filter(|key| *key != PULL_ROW).collect();
                match keys.len() {
                    0 => None,
                    1 => Some(Menu::new(vec![
                        MenuItem::new(&tr!("Run…"), RUN),
                        MenuItem::separator(),
                        MenuItem::new(&tr!("Copy Reference"), COPY_REFERENCE),
                        MenuItem::new(&tr!("Copy Image ID"), COPY_ID),
                        MenuItem::separator(),
                        MenuItem::new(&tr!("Remove…"), REMOVE),
                    ])),
                    count => Some(Menu::new(vec![MenuItem::new(
                        &tr!("Remove {count} Images", count = count),
                        REMOVE,
                    )])),
                }
            },
            move |action, keys| {
                push(&menu_acts, Act::Menu(action.to_owned(), keys.to_vec()));
            },
        );
        self.table = Some(table.id());
        let strip = Strip::new(Vec::new());
        self.strip = Some(strip.id());
        self.bar = None;
        (
            vec![boxed(run), boxed(prune), boxed(remove), boxed(field)],
            boxed(Banded::new(boxed(strip), boxed(table))),
        )
    }

    fn fill(&mut self, ui: &mut Ui, scope: &Scope) {
        let bar = match &scope.store.pulling {
            Some((reference, pulling)) => Bar::Pulling(
                reference.clone(),
                pulling.status.clone(),
                pulling
                    .fraction
                    .map(|fraction| (fraction * 100.0).round() as u32),
            ),
            None => Bar::Field,
        };
        if self.bar.as_ref() != Some(&bar) {
            let (widgets, field) = self.bar_widgets(&bar);
            if let Some(strip) = self.strip.and_then(|id| ui.widget_as::<Strip>(id)) {
                strip.set_children(widgets);
            }
            self.field = field;
            self.bar = Some(bar);
        }
        let rows = rows(scope, &self.filter);
        if let Some(table) = self.table.and_then(|id| ui.widget_as::<Table>(id)) {
            table.set_rows(rows);
        }
        let selected = !self.references().is_empty();
        for (id, needs_selection) in &self.tools {
            if let Some(button) = ui.widget_as::<Button>(*id) {
                button.set_enabled(!needs_selection || selected);
            }
        }
    }

    fn act(&mut self, act: Act) {
        match act {
            Act::Select(keys) => self.selected = keys,
            Act::Filter(text) => self.filter = text,
            Act::Text(text) => self.text = text,
            _ => {}
        }
    }

    fn search(&self) -> Option<WidgetId> {
        self.search
    }

    fn menu(&self, _scope: &Scope) -> Option<Entry> {
        let selected = !self.references().is_empty();
        Some(Entry::submenu(
            &tr!("Image"),
            vec![
                Entry::item(&tr!("Run…"), RUN),
                Entry::item(&tr!("Copy Reference"), COPY_REFERENCE).enabled(selected),
                Entry::separator(),
                Entry::item(&tr!("Remove…"), REMOVE).enabled(selected),
                Entry::item(&tr!("Prune Unused…"), PRUNE),
            ],
        ))
    }

    fn command(&mut self, ui: &mut Ui, acts: &Acts, scope: &Scope, command: &str) -> bool {
        let keys = self.references();
        match command {
            PULL => {
                if let Some(field) = self.field {
                    ui.set_focus(field);
                }
            }
            PULL_NOW => {
                let reference = self.text.trim().to_owned();
                if reference.is_empty() || scope.store.pulling.is_some() {
                    return true;
                }
                self.text.clear();
                self.bar = None;
                push(acts, Act::Task(Task::Pull(reference)));
            }
            PRUNE => confirm(
                ui,
                acts,
                &tr!("Prune unused images?"),
                &tr!("Removes every image no container uses. This cannot be undone."),
                &tr!("Prune Unused Images"),
                Task::PruneImages,
            ),
            _ => {
                self.menu_action(ui, acts, scope, command, &keys);
            }
        }
        true
    }

    fn menu_action(
        &mut self,
        ui: &mut Ui,
        acts: &Acts,
        scope: &Scope,
        action: &str,
        keys: &[String],
    ) {
        let keys: Vec<String> = keys
            .iter()
            .filter(|key| *key != PULL_ROW)
            .cloned()
            .collect();
        match (action, keys.first()) {
            (RUN, first) => run::sheet(ui, acts, first.map_or("", String::as_str)),
            (REMOVE, _) => self.remove(ui, acts, keys),
            (COPY_REFERENCE, Some(key)) => ui.context().copy(key),
            (COPY_ID, Some(key)) => ui.context().copy(&Self::image_id(scope, key)),
            _ => {}
        }
    }

    fn finished(&mut self, ui: &mut Ui, task: &Task, result: &Result<u64, String>) -> bool {
        let title = match (task, result) {
            (Task::PruneImages, Ok(reclaimed)) => {
                let message = tr!("Reclaimed {size}.", size = size(*reclaimed));
                tell(ui, &tr!("Pruned unused images"), &message);
                return true;
            }
            (Task::Pull(reference), Err(_)) => {
                tr!("Cannot pull “{name}”", name = reference.clone())
            }
            (Task::Run(spec), Err(_)) => {
                tr!("Cannot run “{name}”", name = spec.image.clone())
            }
            (Task::RemoveImages(_) | Task::PruneImages, Err(_)) => tr!("Cannot remove every image"),
            (Task::LoadImages(archives), Ok(_)) => {
                tell(
                    ui,
                    &trn!(
                        "Loaded the image archive",
                        "Loaded {count} image archives",
                        count = archives.len()
                    ),
                    "",
                );
                return true;
            }
            (Task::LoadImages(_), Err(_)) => tr!("Cannot load the images"),
            _ => return false,
        };
        if let Err(message) = result {
            tell(ui, &title, message);
        }
        true
    }
}
