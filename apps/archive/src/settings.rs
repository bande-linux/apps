use std::path::{Path, PathBuf};

use bande_l10n::{Text, text, tr};
use bande_app_settings::{Shared, Stored, flag, pairs, settings_folder};
use bande_fs::Format;
use bande_toolkit::ui::{Context, Form, FormLine, PopUpButton, ScrollView, Toggle, Ui, Widget};
use bande_toolkit::{Appearance, TextSystem};

pub const SETTINGS_SIZE: (u32, u32) = (560, 420);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum After {
    #[default]
    Keep,
    Trash,
}

impl After {
    fn key(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Trash => "trash",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        [Self::Keep, Self::Trash]
            .into_iter()
            .find(|after| after.key() == value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSettings {
    pub expand_into: Option<PathBuf>,
    pub after_expanding: After,
    pub reveal_expanded: bool,
    pub archive_into: Option<PathBuf>,
    pub format: Format,
    pub after_archiving: After,
    pub reveal_archive: bool,
}

impl Default for ArchiveSettings {
    fn default() -> Self {
        Self {
            expand_into: None,
            after_expanding: After::Keep,
            reveal_expanded: false,
            archive_into: None,
            format: Format::Zip,
            after_archiving: After::Keep,
            reveal_archive: false,
        }
    }
}

fn folder(value: &str) -> Option<PathBuf> {
    Some(PathBuf::from(value)).filter(|path| path.is_absolute())
}

fn folder_text(folder: Option<&Path>) -> String {
    folder
        .map(|folder| folder.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl ArchiveSettings {
    pub fn current() -> Self {
        settings_folder()
            .as_deref()
            .map(Self::load)
            .unwrap_or_default()
    }
}

impl Stored for ArchiveSettings {
    const FILE: &'static str = "archive-utility.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            match key {
                "expand-into" => settings.expand_into = folder(value),
                "after-expanding" => {
                    settings.after_expanding = After::parse(value).unwrap_or_default();
                }
                "reveal-expanded" => {
                    settings.reveal_expanded = flag(value).unwrap_or_default();
                }
                "archive-into" => settings.archive_into = folder(value),
                "format" => {
                    settings.format = Format::CREATED
                        .into_iter()
                        .find(|format| format.suffix() == value)
                        .unwrap_or(Format::Zip);
                }
                "after-archiving" => {
                    settings.after_archiving = After::parse(value).unwrap_or_default();
                }
                "reveal-archive" => settings.reveal_archive = flag(value).unwrap_or_default(),
                _ => {}
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        format!(
            "expand-into = {}\nafter-expanding = {}\nreveal-expanded = {}\n\
             archive-into = {}\nformat = {}\nafter-archiving = {}\nreveal-archive = {}\n",
            folder_text(self.expand_into.as_deref()),
            self.after_expanding.key(),
            self.reveal_expanded,
            folder_text(self.archive_into.as_deref()),
            self.format.suffix(),
            self.after_archiving.key(),
            self.reveal_archive,
        )
    }
}

type Field = fn(&mut ArchiveSettings) -> &mut Option<PathBuf>;

fn choose_folder(field: Field) {
    let chosen = std::thread::Builder::new()
        .name("choose folder".to_owned())
        .spawn(move || {
            let Some(folder) = settings_folder() else {
                return;
            };
            let chosen = zbus::blocking::Connection::session()
                .and_then(|session| bande_app::choose_file(&session, &tr!("Choose Folder…"), None));
            match chosen {
                Ok(Some(path)) => {
                    let mut settings = ArchiveSettings::load(&folder);
                    *field(&mut settings) = Some(path);
                    if let Err(error) = settings.save(&folder) {
                        tracing::warn!("cannot save the folder: {error}");
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("cannot choose a folder: {error}"),
            }
        });
    if let Err(error) = chosen {
        tracing::warn!("cannot choose a folder: {error}");
    }
}

fn folder_choice(settings: &Shared<ArchiveSettings>, same: Text, field: Field) -> Box<dyn Widget> {
    let current = field(&mut settings.borrow_mut()).clone();
    let mut choices = vec![same.tr()];
    choices.extend(current.as_deref().map(|folder| {
        folder
            .file_name()
            .map_or_else(|| folder.to_string_lossy(), |name| name.to_string_lossy())
            .into_owned()
    }));
    choices.push(tr!("Choose Folder…"));
    let other = choices.len() - 1;
    let shared = settings.clone();
    Box::new(PopUpButton::new(
        &same.tr(),
        &choices,
        usize::from(current.is_some()),
        move |index| match index {
            0 => *field(&mut shared.borrow_mut()) = None,
            index if index == other => choose_folder(field),
            _ => {}
        },
    ))
}

fn after_choice(
    settings: &Shared<ArchiveSettings>,
    labels: [String; 2],
    field: fn(&mut ArchiveSettings) -> &mut After,
) -> Box<dyn Widget> {
    let choices = labels;
    let selected = usize::from(*field(&mut settings.borrow_mut()) == After::Trash);
    let shared = settings.clone();
    Box::new(PopUpButton::new(
        &choices[0],
        &choices,
        selected,
        move |index| {
            *field(&mut shared.borrow_mut()) = match index {
                0 => After::Keep,
                _ => After::Trash,
            }
        },
    ))
}

fn reveal_toggle(
    settings: &Shared<ArchiveSettings>,
    label: Text,
    field: fn(&mut ArchiveSettings) -> &mut bool,
) -> Box<dyn Widget> {
    let shared = settings.clone();
    let on = *field(&mut settings.borrow_mut());
    Box::new(Toggle::new(&label.tr(), on, move |on| {
        *field(&mut shared.borrow_mut()) = on;
    }))
}

fn format_choice(settings: &Shared<ArchiveSettings>) -> Box<dyn Widget> {
    let choices = Format::CREATED.map(|format| match format {
        Format::Zip => tr!("ZIP archive"),
        other => tr!("Compressed tar archive ({suffix})", suffix = other.suffix()),
    });
    let selected = Format::CREATED
        .iter()
        .position(|format| *format == settings.borrow().format)
        .unwrap_or_default();
    let shared = settings.clone();
    Box::new(PopUpButton::new(
        &tr!("Archive format"),
        &choices,
        selected,
        move |index| {
            if let Some(format) = Format::CREATED.get(index) {
                shared.borrow_mut().format = *format;
            }
        },
    ))
}

pub fn settings_form(appearance: Appearance, settings: &Shared<ArchiveSettings>) -> Ui {
    let form = Form::new(
        &tr!("Archive Utility Settings"),
        vec![
            FormLine::Heading(tr!("Expanding")),
            FormLine::Row(
                tr!("Save expanded files"),
                folder_choice(settings, text!("Next to the archive"), |settings| {
                    &mut settings.expand_into
                }),
            ),
            FormLine::Row(
                tr!("After expanding"),
                after_choice(
                    settings,
                    [
                        tr!("Leave the archive alone"),
                        tr!("Move the archive to the Trash"),
                    ],
                    |settings| &mut settings.after_expanding,
                ),
            ),
            FormLine::Row(
                tr!("Reveal expanded items in Files"),
                reveal_toggle(
                    settings,
                    text!("Reveal expanded items in Files"),
                    |settings| &mut settings.reveal_expanded,
                ),
            ),
            FormLine::Heading(tr!("Archiving")),
            FormLine::Row(
                tr!("Save archive"),
                folder_choice(settings, text!("Next to the files"), |settings| {
                    &mut settings.archive_into
                }),
            ),
            FormLine::Row(tr!("Archive format"), format_choice(settings)),
            FormLine::Row(
                tr!("After archiving"),
                after_choice(
                    settings,
                    [
                        tr!("Leave the files alone"),
                        tr!("Move the files to the Trash"),
                    ],
                    |settings| &mut settings.after_archiving,
                ),
            ),
            FormLine::Row(
                tr!("Reveal archive in Files"),
                reveal_toggle(settings, text!("Reveal archive in Files"), |settings| {
                    &mut settings.reveal_archive
                }),
            ),
        ],
    );
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form)),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
