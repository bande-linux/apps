use std::cell::{Cell as Flag, RefCell};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use bande_design::Accent;
use bande_fs::format_size;
use bande_toolkit::ui::{
    Alert, Axis, Button, Cell, Column, Context, Form, FormLine, Label, PopUpButton, ProgressBar,
    Row, ScrollView, SecureField, Stack, Table, TextField, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Color, Header, Image, Palette, SHEET_INSET, TextSystem};
use zeroize::Zeroizing;

use crate::L10N;
use crate::jobs::{Abilities, Erase, Failure, Format, Job, Outcome, Report};
use crate::model::{Apple, Disk, Group, Model, Scheme, Selected, Volume};
use crate::size::parse_size;
use crate::widgets::{
    CapacityBar, Closable, Heading, InfoGrid, Root, SIDEBAR_WIDTH, Segment, TOOLBAR_HEIGHT, symbol,
};

const DISK_SYMBOL: &str = include_str!("../data/symbols/disk.svg");
const EXTERNAL_SYMBOL: &str = include_str!("../data/symbols/external.svg");
const VOLUME_SYMBOL: &str = include_str!("../data/symbols/volume.svg");
const IMAGE_SYMBOL: &str = include_str!("../data/symbols/image.svg");
const APPLE_SYMBOL: &str = include_str!("../data/symbols/apple.svg");
const SIDEBAR_PIXELS: u32 = 32;
const HEADING_PIXELS: u32 = 96;
const TITLE_INSET: f32 = 16.0;
const CONTENT_SPACING: f32 = 20.0;
const CONTENT_PADDING: f32 = 24.0;
const FIELD_FONT: f32 = 13.0;
const MIN_PARTITION: u64 = 16 << 20;
const SEGMENT_ACCENTS: [Accent; 6] = [
    Accent::Ultramarine,
    Accent::Teal,
    Accent::Amber,
    Accent::Plum,
    Accent::Moss,
    Accent::Rose,
];

pub const MOUNT: &str = "mount";
pub const EJECT: &str = "eject";
pub const ERASE: &str = "erase";
pub const ADD_PARTITION: &str = "add-partition";
pub const DELETE_PARTITION: &str = "delete-partition";
pub const RESIZE: &str = "resize";
pub const FIRST_AID: &str = "first-aid";
pub const RENAME: &str = "rename";
pub const RESTORE: &str = "restore";
pub const IMAGE_FROM: &str = "image-from";
pub const OPEN_IMAGE: &str = "open-image";
pub const COMMANDS: [&str; 11] = [
    MOUNT,
    EJECT,
    ERASE,
    ADD_PARTITION,
    DELETE_PARTITION,
    RESIZE,
    FIRST_AID,
    RENAME,
    RESTORE,
    IMAGE_FROM,
    OPEN_IMAGE,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choose {
    SaveImage(String, String),
    Restore(String),
    MountImage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Run(Job),
    Choose(Choose),
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    EraseVolume(String, Erase),
    EraseDisk(String, Scheme, Erase),
    Add {
        table: String,
        scheme: Scheme,
        offset: u64,
        size: u64,
        erase: Erase,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Sheet {
    Erase(String),
    Passphrase(Box<Pending>),
    Add(String),
    Delete(String),
    Resize(String),
    Rename(String),
    FirstAid(String),
    Unlock(String),
    Restore(String, PathBuf),
    Stop,
    Notice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Select(String),
    Expand(Vec<String>, bool),
    Command(&'static str),
    Choice(Sheet, String),
}

type Intents = Rc<RefCell<Vec<Intent>>>;

#[derive(Default)]
struct Fields {
    name: Option<WidgetId>,
    format: Option<WidgetId>,
    scheme: Option<WidgetId>,
    size: Option<WidgetId>,
    secrets: Vec<WidgetId>,
}

struct Running {
    sheet: Option<(Rc<Flag<bool>>, Option<WidgetId>)>,
    done: Option<(String, String)>,
    failed: String,
}

pub struct DisksView {
    ui: Ui,
    root: WidgetId,
    sidebar: WidgetId,
    palette: Palette,
    model: Model,
    abilities: Abilities,
    selected: Option<String>,
    collapsed: HashSet<String>,
    intents: Intents,
    fields: Fields,
    formats: Vec<Format>,
    requests: Vec<Request>,
    running: VecDeque<Running>,
    pending_images: HashSet<String>,
    mounted_images: HashSet<String>,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn push(intents: &Intents, intent: Intent) {
    intents.borrow_mut().push(intent);
}

fn choice(intents: &Intents, sheet: Sheet) -> impl FnMut(&str) + 'static {
    let intents = Rc::clone(intents);
    move |action: &str| push(&intents, Intent::Choice(sheet.clone(), action.to_owned()))
}

fn quoted(name: &str) -> String {
    format!("\u{201c}{name}\u{201d}")
}

fn described(name: &str, device: &str, size: u64) -> String {
    L10N.tr_args(
        "device-described",
        [
            ("name", quoted(name).into()),
            ("device", device.into()),
            ("size", format_size(size).into()),
        ],
    )
}

pub fn format_name(volume: &Volume) -> String {
    let key = match volume.fs_type.as_str() {
        "ext4" | "ext3" | "ext2" => {
            return L10N.tr_args("fs-ext", [("kind", volume.fs_type.as_str().into())]);
        }
        "vfat" => "fs-fat",
        "exfat" => "fs-exfat",
        "ntfs" => "fs-ntfs",
        "apfs" => "fs-apfs",
        "hfsplus" => "fs-hfs",
        "crypto_LUKS" => "fs-luks",
        "swap" => "fs-swap",
        "iso9660" => "fs-iso",
        "" => "fs-none",
        other => return other.to_owned(),
    };
    let name = L10N.tr(key);
    match volume.encrypted && !volume.locked {
        true => L10N.tr_args("fs-encrypted", [("kind", name.into())]),
        false => name,
    }
}

fn free_space(point: &str) -> Option<u64> {
    let stat = rustix::fs::statvfs(point).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

fn location(group: Group) -> String {
    L10N.tr(match group {
        Group::Internal => "location-internal",
        Group::External => "location-external",
        Group::Images => "location-image",
    })
}

impl DisksView {
    pub fn new(appearance: Appearance) -> Self {
        let palette = appearance.palette();
        let intents = Intents::default();
        let selecting = Rc::clone(&intents);
        let expanding = Rc::clone(&intents);
        let sidebar = Table::new(vec![Column::new("", SIDEBAR_WIDTH).flexible()])
            .without_header()
            .sidebar()
            .on_selection(move |keys| {
                if let Some(key) = keys.first() {
                    push(&selecting, Intent::Select(key.clone()));
                }
            })
            .on_expand(move |keys, expanded, _| {
                push(&expanding, Intent::Expand(keys.to_vec(), expanded));
            });
        let sidebar_id = sidebar.id();
        let root = Root::new(boxed(sidebar), boxed(Label::new("")));
        let root_id = root.id();
        let mut ui = Ui::new(Box::new(root), Context::new(TextSystem::new(), palette));
        ui.set_appearance(appearance);
        ui.set_sheet_top(TOOLBAR_HEIGHT);
        let mut view = Self {
            ui,
            root: root_id,
            sidebar: sidebar_id,
            palette,
            model: Model::default(),
            abilities: Abilities::default(),
            selected: None,
            collapsed: HashSet::new(),
            intents,
            fields: Fields::default(),
            formats: Vec::new(),
            requests: Vec::new(),
            running: VecDeque::new(),
            pending_images: HashSet::new(),
            mounted_images: HashSet::new(),
        };
        view.refresh();
        view
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn ui_ref(&self) -> &Ui {
        &self.ui
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
        self.refresh();
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        let inset = match full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
        if let Some(root) = self.ui.widget_as::<Root>(self.root) {
            root.set_inset(inset);
        }
    }

    pub fn header(&self) -> Header {
        Header {
            title_x: SIDEBAR_WIDTH + TITLE_INSET,
            items: self
                .ui
                .widget_ref::<Root>(self.root)
                .map(Root::tool_frames)
                .unwrap_or_default(),
            tabs: Vec::new(),
        }
    }

    pub fn busy(&self) -> bool {
        self.running.iter().any(|running| running.sheet.is_some())
    }

    pub fn warn_busy(&mut self) {
        self.notice(&L10N.tr("quit-busy"), &L10N.tr("quit-busy-message"));
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn set_abilities(&mut self, abilities: Abilities) {
        self.abilities = abilities;
        self.refresh();
    }

    pub fn set_model(&mut self, model: Model) {
        if self.model == model {
            return;
        }
        self.model = model;
        if self
            .selected
            .as_deref()
            .is_none_or(|key| self.model.find(key).is_none())
        {
            self.selected = self.model.disks.first().map(Disk::key);
        }
        self.mount_new_images();
        self.refresh();
    }

    pub fn select(&mut self, key: &str) {
        if self.model.find(key).is_some() {
            self.selected = Some(key.to_owned());
            self.refresh();
        }
    }

    fn selection(&self) -> Option<Selected<'_>> {
        self.model.find(self.selected.as_deref()?)
    }

    fn mount_new_images(&mut self) {
        self.pending_images
            .retain(|path| self.model.disks.iter().any(|disk| disk.path == *path));
        let mut mounts = Vec::new();
        for disk in &self.model.disks {
            if !self.pending_images.contains(&disk.path) {
                continue;
            }
            for volume in &disk.volumes {
                if let Some(filesystem) = &volume.filesystem
                    && !volume.mounted()
                    && self.mounted_images.insert(filesystem.clone())
                {
                    mounts.push(Job::Mount(filesystem.clone()));
                }
            }
        }
        for job in mounts {
            self.quiet(job, "failed-mount");
        }
    }

    fn quiet(&mut self, job: Job, failed: &str) {
        self.running.push_back(Running {
            sheet: None,
            done: None,
            failed: L10N.tr(failed),
        });
        self.requests.push(Request::Run(job));
    }

    fn start(&mut self, job: Job, title: &str, done: (String, String), failed: &str, copy: bool) {
        let closed = Rc::new(Flag::new(false));
        let mut alert = Alert::new(
            title,
            &L10N.tr("working"),
            choice(&self.intents, Sheet::Stop),
        );
        let progress = match copy {
            true => {
                let bar = ProgressBar::new(title, Some(0.0));
                let id = bar.id();
                alert = alert.accessory(bar).cancel(&L10N.tr("button-stop"), "stop");
                Some(id)
            }
            false => {
                alert = alert.accessory(ProgressBar::new(title, None));
                None
            }
        };
        self.ui
            .present_sheet(Closable::new(alert, Rc::clone(&closed)));
        self.running.push_back(Running {
            sheet: Some((closed, progress)),
            done: Some(done),
            failed: L10N.tr(failed),
        });
        self.requests.push(Request::Run(job));
    }

    fn notice(&mut self, title: &str, message: &str) {
        self.ui.present_sheet(
            Alert::new(title, message, choice(&self.intents, Sheet::Notice))
                .primary(&L10N.tr("button-done"), "done"),
        );
    }

    pub fn report(&mut self, report: Report) {
        match report {
            Report::Abilities(abilities) => self.set_abilities(abilities),
            Report::Chosen(choose, file) => self.chosen(choose, file),
            Report::Progress(value) => {
                if let Some((_, Some(bar))) =
                    self.running.front().and_then(|run| run.sheet.as_ref())
                {
                    let bar = *bar;
                    self.ui.update_widget::<ProgressBar>(bar, |bar, cx| {
                        bar.set_value(cx, Some(value))
                    });
                }
            }
            Report::Finished(result) => {
                let Some(running) = self.running.pop_front() else {
                    return;
                };
                if let Some((closed, _)) = &running.sheet {
                    closed.set(true);
                }
                match (result, running.done) {
                    (Ok(Outcome::Loop(device)), _) => {
                        self.pending_images.insert(device);
                        self.mount_new_images();
                    }
                    (Ok(Outcome::Checked { repaired }), _) => {
                        let message = match repaired {
                            true => "first-aid-repaired",
                            false => "first-aid-ok",
                        };
                        self.notice(&L10N.tr("first-aid-complete"), &L10N.tr(message));
                    }
                    (Ok(Outcome::Done), Some((title, message))) => self.notice(&title, &message),
                    (Ok(Outcome::Done), None) | (Err(Failure::Dismissed), _) => {}
                    (Err(Failure::Failed(message)), _) => self.notice(&running.failed, &message),
                }
                self.refresh();
            }
        }
    }

    pub fn chosen(&mut self, choose: Choose, file: Option<PathBuf>) {
        let Some(file) = file else {
            return;
        };
        match choose {
            Choose::SaveImage(source, name) => {
                let title = L10N.tr_args("imaging", [("name", quoted(&name).into())]);
                let done = (
                    L10N.tr("image-complete"),
                    L10N.tr_args("image-saved", [("file", file.display().to_string().into())]),
                );
                self.start(
                    Job::CreateImage(source, file),
                    &title,
                    done,
                    "failed-image",
                    true,
                );
            }
            Choose::Restore(target) => self.ask_restore(&target, file),
            Choose::MountImage => self.quiet(Job::MountImage(file), "failed-open-image"),
        }
    }

    fn ask_restore(&mut self, target: &str, file: PathBuf) {
        let Some((name, device, size)) = self.target(target) else {
            return;
        };
        let image_size = match bande_volumes::on_connected_volume(&file) {
            true => 0,
            false => std::fs::metadata(&file)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        };
        let image_name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if image_size > size {
            let message = L10N.tr_args(
                "restore-too-big-message",
                [
                    ("image", quoted(&image_name).into()),
                    ("target", described(&name, &device, size).into()),
                ],
            );
            return self.notice(&L10N.tr("restore-too-big"), &message);
        }
        let title = L10N.tr_args("restore-title", [("name", quoted(&name).into())]);
        let message = L10N.tr_args(
            "restore-message",
            [
                ("target", described(&name, &device, size).into()),
                ("image", quoted(&image_name).into()),
                ("image_size", format_size(image_size).into()),
            ],
        ) + &self.startup_warning(target);
        self.ui.present_sheet(
            Alert::new(
                &title,
                &message,
                choice(&self.intents, Sheet::Restore(target.to_owned(), file)),
            )
            .cancel(&L10N.tr("button-cancel"), "cancel")
            .destructive(&L10N.tr("button-restore"), "restore"),
        );
    }

    fn target(&self, path: &str) -> Option<(String, String, u64)> {
        self.model.disks.iter().find_map(|disk| {
            if disk.path == path {
                return Some((disk.name.clone(), disk.device.clone(), disk.size));
            }
            disk.volumes
                .iter()
                .find(|volume| volume.path == path)
                .map(|volume| (volume.name.clone(), volume.device.clone(), volume.size))
        })
    }

    fn startup_warning(&self, path: &str) -> String {
        let on_startup_disk = self
            .model
            .disks
            .iter()
            .any(|disk| disk.system && disk.volumes.iter().any(|volume| volume.path == path));
        match on_startup_disk {
            true => format!("\n\n{}", L10N.tr("startup-disk-warning")),
            false => String::new(),
        }
    }

    fn volume(&self) -> Option<(&Disk, &Volume)> {
        match self.selection()? {
            Selected::Volume(disk, volume) => Some((disk, volume)),
            Selected::Disk(disk) => disk.whole_volume().map(|volume| (disk, volume)),
        }
    }

    fn disk(&self) -> Option<&Disk> {
        match self.selection()? {
            Selected::Disk(disk) | Selected::Volume(disk, _) => Some(disk),
        }
    }

    fn mount_targets(&self) -> (Vec<String>, Vec<String>) {
        let volumes: Vec<&Volume> = match self.selection() {
            Some(Selected::Volume(_, volume)) => vec![volume],
            Some(Selected::Disk(disk)) => disk.volumes.iter().collect(),
            None => Vec::new(),
        };
        let usable: Vec<&&Volume> = volumes
            .iter()
            .filter(|volume| {
                volume.apple.is_none() && (volume.filesystem.is_some() || volume.encrypted)
            })
            .collect();
        if usable.iter().any(|volume| volume.mounted()) {
            if usable.iter().any(|volume| volume.in_use) {
                return (Vec::new(), Vec::new());
            }
            let mounted = usable
                .iter()
                .filter(|volume| volume.mounted() || (volume.encrypted && !volume.locked))
                .map(|volume| volume.path.clone())
                .collect();
            return (Vec::new(), mounted);
        }
        let mountable = usable
            .iter()
            .filter(|volume| volume.locked || volume.filesystem.is_some())
            .filter(|volume| volume.fs_type != "swap")
            .map(|volume| volume.path.clone())
            .collect();
        (mountable, Vec::new())
    }

    pub fn mount_title(&self) -> String {
        match self.mount_targets() {
            (_, unmount) if !unmount.is_empty() => L10N.tr("action-unmount"),
            _ => L10N.tr("action-mount"),
        }
    }

    pub fn can(&self, command: &str) -> bool {
        if self.busy() {
            return command == OPEN_IMAGE;
        }
        let volume = self.volume();
        let disk = self.disk();
        let whole_disk = matches!(self.selection(), Some(Selected::Disk(_)));
        match command {
            MOUNT => {
                let (mount, unmount) = self.mount_targets();
                !mount.is_empty() || !unmount.is_empty()
            }
            EJECT => disk.is_some_and(|disk| disk.ejectable && !disk.system),
            ERASE if whole_disk => {
                disk.is_some_and(|disk| !disk.protected()) && !self.abilities.formats.is_empty()
            }
            ERASE => {
                volume.is_some_and(|(_, volume)| !volume.protected())
                    && !self.abilities.formats.is_empty()
            }
            ADD_PARTITION => {
                disk.is_some_and(|disk| {
                    !disk.apple && disk.scheme.is_some() && disk.largest_free().is_some()
                }) && !self.abilities.formats.is_empty()
            }
            DELETE_PARTITION => {
                !whole_disk
                    && volume.is_some_and(|(_, volume)| volume.partition && !volume.protected())
            }
            RESIZE => {
                !whole_disk
                    && volume.is_some_and(|(_, volume)| {
                        volume.partition
                            && !volume.protected()
                            && !volume.encrypted
                            && (self.abilities.can_resize(&volume.fs_type, true, false)
                                || self.abilities.can_resize(&volume.fs_type, false, false))
                    })
            }
            FIRST_AID => volume.is_some_and(|(_, volume)| {
                volume.filesystem.is_some()
                    && !volume.protected()
                    && self.abilities.check.contains(&volume.fs_type)
            }),
            RENAME => {
                volume.is_some_and(|(_, volume)| volume.filesystem.is_some() && !volume.protected())
            }
            RESTORE if whole_disk => disk.is_some_and(|disk| !disk.protected()),
            RESTORE => volume.is_some_and(|(_, volume)| !volume.protected()),
            IMAGE_FROM if whole_disk => disk.is_some_and(|disk| !disk.system),
            IMAGE_FROM => volume.is_some_and(|(_, volume)| !volume.in_use),
            OPEN_IMAGE => true,
            _ => false,
        }
    }

    pub fn command(&mut self, command: &str) -> bool {
        if !COMMANDS.contains(&command) {
            return false;
        }
        if !self.can(command) {
            return true;
        }
        let whole_disk = matches!(self.selection(), Some(Selected::Disk(_)));
        let volume = self.volume().map(|(_, volume)| volume.clone());
        let disk = self.disk().cloned();
        match (command, disk, volume) {
            (MOUNT, _, _) => {
                let (mount, unmount) = self.mount_targets();
                for path in unmount {
                    self.quiet(Job::Unmount(path), "failed-unmount");
                }
                for path in mount {
                    self.mount(&path);
                }
            }
            (EJECT, Some(disk), _) => self.quiet(Job::Eject(disk.path), "failed-eject"),
            (ERASE, Some(disk), _) if whole_disk => {
                self.ask_erase(&disk.path, &disk.name, &disk.device, disk.size, true)
            }
            (ERASE, _, Some(volume)) => self.ask_erase(
                &volume.path,
                &volume.name,
                &volume.device,
                volume.size,
                false,
            ),
            (ADD_PARTITION, Some(disk), _) => self.ask_add(&disk),
            (DELETE_PARTITION, _, Some(volume)) => self.ask_delete(&volume),
            (RESIZE, Some(disk), Some(volume)) => self.ask_resize(&disk, &volume),
            (FIRST_AID, _, Some(volume)) => self.ask_first_aid(&volume),
            (RENAME, _, Some(volume)) => self.ask_rename(&volume),
            (RESTORE, Some(disk), _) if whole_disk => self
                .requests
                .push(Request::Choose(Choose::Restore(disk.path))),
            (RESTORE, _, Some(volume)) => self
                .requests
                .push(Request::Choose(Choose::Restore(volume.path))),
            (IMAGE_FROM, Some(disk), _) if whole_disk => self
                .requests
                .push(Request::Choose(Choose::SaveImage(disk.path, disk.name))),
            (IMAGE_FROM, _, Some(volume)) => self
                .requests
                .push(Request::Choose(Choose::SaveImage(volume.path, volume.name))),
            (OPEN_IMAGE, _, _) => self.requests.push(Request::Choose(Choose::MountImage)),
            _ => {}
        }
        self.refresh();
        true
    }

    fn mount(&mut self, path: &str) {
        let Some((_, volume)) = self.model.disks.iter().find_map(|disk| {
            disk.volumes
                .iter()
                .find(|volume| volume.path == path)
                .map(|volume| (disk, volume))
        }) else {
            return;
        };
        match (volume.locked, volume.filesystem.clone()) {
            (true, _) => {
                let title = L10N.tr_args("unlock-title", [("name", quoted(&volume.name).into())]);
                let field = SecureField::new(&L10N.tr("field-password"), FIELD_FONT, |_| {});
                self.fields = Fields {
                    secrets: vec![field.id()],
                    ..Fields::default()
                };
                let sheet = Sheet::Unlock(path.to_owned());
                self.ui.present_sheet(
                    Alert::new(
                        &title,
                        &L10N.tr("unlock-message"),
                        choice(&self.intents, sheet),
                    )
                    .accessory(field)
                    .cancel(&L10N.tr("button-cancel"), "cancel")
                    .primary(&L10N.tr("button-unlock"), "unlock"),
                );
            }
            (false, Some(filesystem)) => self.quiet(Job::Mount(filesystem), "failed-mount"),
            (false, None) => {}
        }
    }

    fn formats_popup(&mut self, encrypted_allowed: bool) -> PopUpButton {
        self.formats = self
            .abilities
            .formats
            .iter()
            .copied()
            .filter(|format| encrypted_allowed || !format.encrypted())
            .collect();
        let titles: Vec<String> = self.formats.iter().map(|format| format.title()).collect();
        let popup = PopUpButton::new(&L10N.tr("field-format"), &titles, 0, |_| {});
        self.fields.format = Some(popup.id());
        popup
    }

    fn name_field(&mut self, name: &str) -> TextField {
        let field = TextField::new(&L10N.tr("field-name"), name, |_| {});
        self.fields.name = Some(field.id());
        field
    }

    fn ask_erase(&mut self, path: &str, name: &str, device: &str, size: u64, whole_disk: bool) {
        self.fields = Fields::default();
        let title = L10N.tr_args("erase-title", [("name", quoted(name).into())]);
        let message = L10N.tr_args(
            "erase-message",
            [("target", described(name, device, size).into())],
        ) + &self.startup_warning(path);
        let default_name = match whole_disk {
            true => L10N.tr("volume-untitled"),
            false => name.to_owned(),
        };
        let mut lines = vec![
            FormLine::Row(L10N.tr("field-name"), boxed(self.name_field(&default_name))),
            FormLine::Row(L10N.tr("field-format"), boxed(self.formats_popup(true))),
        ];
        if whole_disk {
            let schemes = [L10N.tr("scheme-gpt"), L10N.tr("scheme-mbr")];
            let popup = PopUpButton::new(&L10N.tr("field-scheme"), &schemes, 0, |_| {});
            self.fields.scheme = Some(popup.id());
            lines.push(FormLine::Row(L10N.tr("field-scheme"), boxed(popup)));
        }
        let sheet = Sheet::Erase(path.to_owned());
        self.ui.present_sheet(
            Alert::new(&title, &message, choice(&self.intents, sheet))
                .accessory(Form::new(&title, lines))
                .cancel(&L10N.tr("button-cancel"), "cancel")
                .destructive(&L10N.tr("button-erase"), "erase"),
        );
    }

    fn ask_add(&mut self, disk: &Disk) {
        let Some((_, free)) = disk.largest_free() else {
            return;
        };
        self.fields = Fields::default();
        let title = L10N.tr_args("add-title", [("name", quoted(&disk.name).into())]);
        let message = L10N.tr_args("add-message", [("free", format_size(free).into())]);
        let size = TextField::new(&L10N.tr("field-size"), &format_size(free), |_| {})
            .without_text_checking();
        self.fields.size = Some(size.id());
        let lines = vec![
            FormLine::Row(
                L10N.tr("field-name"),
                boxed(self.name_field(&L10N.tr("volume-untitled"))),
            ),
            FormLine::Row(L10N.tr("field-format"), boxed(self.formats_popup(true))),
            FormLine::Row(L10N.tr("field-size"), boxed(size)),
        ];
        self.ui.present_sheet(
            Alert::new(
                &title,
                &message,
                choice(&self.intents, Sheet::Add(disk.path.clone())),
            )
            .accessory(Form::new(&title, lines))
            .cancel(&L10N.tr("button-cancel"), "cancel")
            .primary(&L10N.tr("button-add"), "add"),
        );
    }

    fn ask_delete(&mut self, volume: &Volume) {
        let title = L10N.tr_args("delete-title", [("name", quoted(&volume.name).into())]);
        let message = L10N.tr_args(
            "delete-message",
            [(
                "target",
                described(&volume.name, &volume.device, volume.size).into(),
            )],
        ) + &self.startup_warning(&volume.path);
        self.ui.present_sheet(
            Alert::new(
                &title,
                &message,
                choice(&self.intents, Sheet::Delete(volume.path.clone())),
            )
            .cancel(&L10N.tr("button-cancel"), "cancel")
            .destructive(&L10N.tr("button-delete"), "delete"),
        );
    }

    fn ask_resize(&mut self, disk: &Disk, volume: &Volume) {
        self.fields = Fields::default();
        let largest = volume.size + disk.free_after(volume);
        let title = L10N.tr_args("resize-title", [("name", quoted(&volume.name).into())]);
        let message = L10N.tr_args(
            "resize-message",
            [
                (
                    "target",
                    described(&volume.name, &volume.device, volume.size).into(),
                ),
                ("largest", format_size(largest).into()),
            ],
        );
        let size = TextField::new(&L10N.tr("field-size"), &format_size(volume.size), |_| {})
            .without_text_checking();
        self.fields.size = Some(size.id());
        let lines = vec![FormLine::Row(L10N.tr("field-size"), boxed(size))];
        self.ui.present_sheet(
            Alert::new(
                &title,
                &message,
                choice(&self.intents, Sheet::Resize(volume.path.clone())),
            )
            .accessory(Form::new(&title, lines))
            .cancel(&L10N.tr("button-cancel"), "cancel")
            .primary(&L10N.tr("button-resize"), "resize"),
        );
    }

    fn ask_first_aid(&mut self, volume: &Volume) {
        let Some(filesystem) = volume.filesystem.clone() else {
            return;
        };
        let title = L10N.tr_args("first-aid-title", [("name", quoted(&volume.name).into())]);
        let message = match volume.mounted() {
            true => L10N.tr("first-aid-unmounts"),
            false => L10N.tr("first-aid-message"),
        };
        self.ui.present_sheet(
            Alert::new(
                &title,
                &message,
                choice(&self.intents, Sheet::FirstAid(filesystem)),
            )
            .cancel(&L10N.tr("button-cancel"), "cancel")
            .primary(&L10N.tr("button-run"), "run"),
        );
    }

    fn ask_rename(&mut self, volume: &Volume) {
        let Some(filesystem) = volume.filesystem.clone() else {
            return;
        };
        self.fields = Fields::default();
        let title = L10N.tr_args("rename-title", [("name", quoted(&volume.name).into())]);
        let field = self.name_field(&volume.name);
        self.ui.present_sheet(
            Alert::new(&title, "", choice(&self.intents, Sheet::Rename(filesystem)))
                .accessory(field)
                .cancel(&L10N.tr("button-cancel"), "cancel")
                .primary(&L10N.tr("button-rename"), "rename"),
        );
    }

    fn ask_passphrase(&mut self, pending: Pending, problem: Option<&str>) {
        let first = SecureField::new(&L10N.tr("field-password"), FIELD_FONT, |_| {});
        let second = SecureField::new(&L10N.tr("field-verify"), FIELD_FONT, |_| {});
        self.fields = Fields {
            secrets: vec![first.id(), second.id()],
            ..Fields::default()
        };
        let title = L10N.tr("passphrase-title");
        let message = problem
            .map(str::to_owned)
            .unwrap_or_else(|| L10N.tr("passphrase-message"));
        let lines = vec![
            FormLine::Row(L10N.tr("field-password"), boxed(first)),
            FormLine::Row(L10N.tr("field-verify"), boxed(second)),
        ];
        let sheet = Sheet::Passphrase(Box::new(pending));
        self.ui.present_sheet(
            Alert::new(&title, &message, choice(&self.intents, sheet))
                .accessory(Form::new(&title, lines))
                .cancel(&L10N.tr("button-cancel"), "cancel")
                .primary(&L10N.tr("button-choose"), "choose"),
        );
    }

    fn text(&self, field: Option<WidgetId>) -> String {
        field
            .and_then(|id| self.ui.widget_ref::<TextField>(id))
            .map(TextField::text)
            .unwrap_or_default()
    }

    fn picked(&self, field: Option<WidgetId>) -> usize {
        field
            .and_then(|id| self.ui.widget_ref::<PopUpButton>(id))
            .map_or(0, PopUpButton::selected)
    }

    fn secrets(&self) -> Vec<Zeroizing<String>> {
        self.fields
            .secrets
            .iter()
            .filter_map(|id| self.ui.widget_ref::<SecureField>(*id))
            .map(|field| Zeroizing::new(field.secret().to_owned()))
            .collect()
    }

    fn erase_choice(&self) -> Option<Erase> {
        let format = *self.formats.get(self.picked(self.fields.format))?;
        Some(Erase {
            format,
            name: self.text(self.fields.name),
            passphrase: None,
        })
    }

    fn proceed(&mut self, pending: Pending) {
        let erase = match &pending {
            Pending::EraseVolume(_, erase)
            | Pending::EraseDisk(_, _, erase)
            | Pending::Add { erase, .. } => erase,
        };
        if erase.format.encrypted() && erase.passphrase.is_none() {
            return self.ask_passphrase(pending, None);
        }
        let name = quoted(&erase.format.label(&erase.name));
        match pending {
            Pending::EraseVolume(path, erase) => {
                let title = L10N.tr_args("erasing", [("name", name.into())]);
                let done = (L10N.tr("erase-complete"), L10N.tr("erase-complete-message"));
                self.start(
                    Job::EraseVolume(path, erase),
                    &title,
                    done,
                    "failed-erase",
                    false,
                );
            }
            Pending::EraseDisk(path, scheme, erase) => {
                let title = L10N.tr_args("erasing", [("name", name.into())]);
                let done = (L10N.tr("erase-complete"), L10N.tr("erase-complete-message"));
                self.start(
                    Job::EraseDisk(path, scheme, erase),
                    &title,
                    done,
                    "failed-erase",
                    false,
                );
            }
            Pending::Add {
                table,
                scheme,
                offset,
                size,
                erase,
            } => {
                let title = L10N.tr_args("adding", [("name", name.into())]);
                let done = (
                    L10N.tr("partition-complete"),
                    L10N.tr("partition-complete-message"),
                );
                let job = Job::AddPartition {
                    table,
                    scheme,
                    offset,
                    size,
                    erase,
                };
                self.start(job, &title, done, "failed-partition", false);
            }
        }
    }

    fn chose(&mut self, sheet: Sheet, action: &str) {
        let partition_done = || {
            (
                L10N.tr("partition-complete"),
                L10N.tr("partition-complete-message"),
            )
        };
        match (sheet, action) {
            (Sheet::Stop, "stop") => self.requests.push(Request::Stop),
            (Sheet::Erase(path), "erase") => {
                let Some(erase) = self.erase_choice() else {
                    return;
                };
                let scheme = match self.picked(self.fields.scheme) {
                    0 => Scheme::Gpt,
                    _ => Scheme::Mbr,
                };
                let whole_disk = self.model.disks.iter().any(|disk| disk.path == path)
                    && self.fields.scheme.is_some();
                self.proceed(match whole_disk {
                    true => Pending::EraseDisk(path, scheme, erase),
                    false => Pending::EraseVolume(path, erase),
                });
            }
            (Sheet::Passphrase(pending), "choose") => {
                let secrets = self.secrets();
                let problem = match secrets.as_slice() {
                    [first, _] if first.is_empty() => Some(L10N.tr("passphrase-empty")),
                    [first, second] if first != second => Some(L10N.tr("passphrase-mismatch")),
                    [_, _] => None,
                    _ => return,
                };
                if let Some(problem) = problem {
                    return self.ask_passphrase(*pending, Some(&problem));
                }
                let passphrase = secrets.into_iter().next();
                let mut pending = *pending;
                match &mut pending {
                    Pending::EraseVolume(_, erase)
                    | Pending::EraseDisk(_, _, erase)
                    | Pending::Add { erase, .. } => erase.passphrase = passphrase,
                }
                self.proceed(pending);
            }
            (Sheet::Add(table), "add") => {
                let Some(disk) = self.model.disks.iter().find(|disk| disk.path == table) else {
                    return;
                };
                let (Some((offset, free)), Some(scheme)) = (disk.largest_free(), disk.scheme)
                else {
                    return;
                };
                let wanted = parse_size(&self.text(self.fields.size));
                let Some(size) = wanted.filter(|size| (MIN_PARTITION..=free).contains(size)) else {
                    let message =
                        L10N.tr_args("size-invalid", [("largest", format_size(free).into())]);
                    return self.notice(&L10N.tr("failed-partition"), &message);
                };
                let Some(erase) = self.erase_choice() else {
                    return;
                };
                let size = match free - size < MIN_PARTITION {
                    true => free,
                    false => size,
                };
                self.proceed(Pending::Add {
                    table,
                    scheme,
                    offset,
                    size,
                    erase,
                });
            }
            (Sheet::Delete(path), "delete") => {
                let title = L10N.tr("deleting");
                self.start(
                    Job::DeletePartition(path),
                    &title,
                    partition_done(),
                    "failed-partition",
                    false,
                );
            }
            (Sheet::Resize(path), "resize") => {
                let Some((disk, volume)) = self.model.disks.iter().find_map(|disk| {
                    disk.volumes
                        .iter()
                        .find(|volume| volume.path == path)
                        .map(|volume| (disk, volume))
                }) else {
                    return;
                };
                let largest = volume.size + disk.free_after(volume);
                let wanted = parse_size(&self.text(self.fields.size));
                let Some(size) = wanted.filter(|size| (MIN_PARTITION..=largest).contains(size))
                else {
                    let message =
                        L10N.tr_args("size-invalid", [("largest", format_size(largest).into())]);
                    return self.notice(&L10N.tr("failed-partition"), &message);
                };
                let grow = size > volume.size;
                if !self.abilities.can_resize(&volume.fs_type, grow, false) {
                    let message = L10N.tr(match grow {
                        true => "resize-cannot-grow",
                        false => "resize-cannot-shrink",
                    });
                    return self.notice(&L10N.tr("failed-partition"), &message);
                }
                let title = L10N.tr_args("resizing", [("name", quoted(&volume.name).into())]);
                let job = Job::Resize {
                    partition: path,
                    size,
                    grow,
                };
                self.start(job, &title, partition_done(), "failed-partition", false);
            }
            (Sheet::FirstAid(filesystem), "run") => {
                let title = L10N.tr("checking");
                let done = (String::new(), String::new());
                self.start(
                    Job::FirstAid(filesystem),
                    &title,
                    done,
                    "failed-first-aid",
                    false,
                );
            }
            (Sheet::Rename(filesystem), "rename") => {
                let name = self.text(self.fields.name);
                self.quiet(Job::Rename(filesystem, name), "failed-rename");
            }
            (Sheet::Unlock(encrypted), "unlock") => {
                if let Some(passphrase) = self.secrets().into_iter().next() {
                    self.quiet(Job::Unlock(encrypted, passphrase), "failed-unlock");
                }
            }
            (Sheet::Restore(target, file), "restore") => {
                let name = self
                    .target(&target)
                    .map(|(name, ..)| name)
                    .unwrap_or_default();
                let title = L10N.tr_args("restoring", [("name", quoted(&name).into())]);
                let done = (
                    L10N.tr("restore-complete"),
                    L10N.tr("restore-complete-message"),
                );
                self.start(
                    Job::Restore(target, file),
                    &title,
                    done,
                    "failed-restore",
                    true,
                );
            }
            _ => {}
        }
    }

    pub fn pump(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if intents.is_empty() {
            return false;
        }
        for intent in intents {
            match intent {
                Intent::Select(key) => self.selected = Some(key),
                Intent::Expand(keys, expanded) => {
                    for key in keys {
                        match expanded {
                            true => self.collapsed.remove(&key),
                            false => self.collapsed.insert(key),
                        };
                    }
                }
                Intent::Command(command) => {
                    self.command(command);
                }
                Intent::Choice(sheet, action) => self.chose(sheet, &action),
            }
        }
        self.refresh();
        true
    }

    fn icon(&self, svg: &str, pixels: u32) -> Option<Image> {
        symbol(svg, self.palette.ink2, pixels)
    }

    fn disk_symbol(disk: &Disk) -> &'static str {
        match disk.group {
            Group::Internal => DISK_SYMBOL,
            Group::External => EXTERNAL_SYMBOL,
            Group::Images => IMAGE_SYMBOL,
        }
    }

    fn volume_symbol(volume: &Volume) -> &'static str {
        match volume.apple {
            Some(_) => APPLE_SYMBOL,
            None => VOLUME_SYMBOL,
        }
    }

    fn sidebar_rows(&self) -> (Vec<Row>, Vec<(usize, String)>, Vec<String>) {
        let mut rows = Vec::new();
        let mut headings = Vec::new();
        let mut expanded = Vec::new();
        let mut line = 0;
        let mut group = None;
        for disk in &self.model.disks {
            if group != Some(disk.group) {
                group = Some(disk.group);
                headings.push((line, location(disk.group)));
            }
            let children: Vec<Row> = disk
                .volumes
                .iter()
                .map(|volume| {
                    let icon = self.icon(Self::volume_symbol(volume), SIDEBAR_PIXELS);
                    Row::new(
                        volume.key(),
                        vec![Cell::new(volume.name.clone()).icon(icon)],
                    )
                })
                .collect();
            line += 1 + children.len();
            if !self.collapsed.contains(&disk.key()) {
                expanded.push(disk.key());
            }
            let icon = self.icon(Self::disk_symbol(disk), SIDEBAR_PIXELS);
            rows.push(
                Row::new(disk.key(), vec![Cell::new(disk.name.clone()).icon(icon)])
                    .children(children),
            );
        }
        (rows, headings, expanded)
    }

    fn color(&self, index: usize, apple: bool) -> Color {
        let accent = match apple {
            true => Accent::Graphite,
            false => SEGMENT_ACCENTS[index % SEGMENT_ACCENTS.len()],
        };
        accent.color(self.palette.scheme)
    }

    fn disk_detail(&self, disk: &Disk) -> Vec<Box<dyn Widget>> {
        let scheme = match disk.scheme {
            Some(Scheme::Gpt) => L10N.tr("scheme-gpt"),
            Some(Scheme::Mbr) => L10N.tr("scheme-mbr"),
            None => L10N.tr("scheme-none"),
        };
        let subtitle = match &disk.image {
            Some(file) => L10N.tr_args("subtitle-image", [("file", file.as_str().into())]),
            None => L10N.tr_args(
                "subtitle-disk",
                [
                    ("connection", disk.connection.as_str().into()),
                    ("location", location(disk.group).into()),
                    ("scheme", scheme.as_str().into()),
                ],
            ),
        };
        let mut segments: Vec<Segment> = disk
            .volumes
            .iter()
            .enumerate()
            .map(|(index, volume)| Segment {
                name: volume.name.clone(),
                detail: format_size(volume.size),
                size: volume.size,
                color: self.color(index, volume.apple.is_some()),
            })
            .collect();
        let free: u64 = disk.free.iter().map(|(_, size)| size).sum();
        if free > 0 {
            segments.push(Segment {
                name: L10N.tr("free-space"),
                detail: format_size(free),
                size: free,
                color: self.palette.hairline,
            });
        }
        let smart = match &disk.smart {
            Some(smart) if smart.failing => L10N.tr("smart-failing"),
            Some(_) => L10N.tr("smart-verified"),
            None => L10N.tr("smart-unsupported"),
        };
        let mut pairs = vec![
            (L10N.tr("info-location"), location(disk.group)),
            (L10N.tr("info-capacity"), format_size(disk.size)),
            (L10N.tr("info-connection"), disk.connection.clone()),
            (L10N.tr("info-children"), disk.volumes.len().to_string()),
            (L10N.tr("info-scheme"), scheme),
            (L10N.tr("info-smart"), smart),
            (L10N.tr("info-device"), disk.device.clone()),
            (
                L10N.tr("info-type"),
                L10N.tr(match (disk.image.is_some(), disk.solid_state) {
                    (true, _) => "type-image",
                    (false, true) => "type-ssd",
                    (false, false) => "type-disk",
                }),
            ),
        ];
        if !disk.model.is_empty() {
            pairs.push((L10N.tr("info-model"), disk.model.clone()));
        }
        if !disk.serial.is_empty() {
            pairs.push((L10N.tr("info-serial"), disk.serial.clone()));
        }
        if let Some(smart) = &disk.smart {
            if let Some(celsius) = smart.celsius {
                pairs.push((L10N.tr("info-temperature"), format!("{celsius:.0} °C")));
            }
            if let Some(hours) = smart.hours {
                pairs.push((L10N.tr("info-hours"), hours.to_string()));
            }
        }
        if disk.apple {
            pairs.push((L10N.tr("info-access"), L10N.tr("access-apple")));
        } else if disk.system {
            pairs.push((L10N.tr("info-access"), L10N.tr("access-startup")));
        }
        vec![
            boxed(Heading::new(
                self.icon(Self::disk_symbol(disk), HEADING_PIXELS),
                &disk.name,
                &subtitle,
                &format_size(disk.size),
            )),
            boxed(CapacityBar::new(segments, disk.size)),
            boxed(InfoGrid::new(pairs)),
        ]
    }

    fn volume_detail(&self, disk: &Disk, volume: &Volume) -> Vec<Box<dyn Widget>> {
        let format = format_name(volume);
        let subtitle = L10N.tr_args(
            "subtitle-volume",
            [
                ("location", location(disk.group).into()),
                ("format", format.as_str().into()),
            ],
        );
        let available = volume
            .mount_points
            .first()
            .and_then(|point| free_space(point));
        let segments = match available {
            Some(available) => vec![
                Segment {
                    name: L10N.tr("used-space"),
                    detail: format_size(volume.size.saturating_sub(available)),
                    size: volume.size.saturating_sub(available),
                    color: self.color(0, volume.apple.is_some()),
                },
                Segment {
                    name: L10N.tr("free-space"),
                    detail: format_size(available),
                    size: available,
                    color: self.palette.hairline,
                },
            ],
            None => vec![Segment {
                name: volume.name.clone(),
                detail: format_size(volume.size),
                size: volume.size,
                color: self.color(0, volume.apple.is_some()),
            }],
        };
        let mount_point = match (volume.mount_points.first(), volume.locked) {
            (Some(point), _) => point.clone(),
            (None, true) => L10N.tr("locked"),
            (None, false) => L10N.tr("not-mounted"),
        };
        let mut pairs = vec![
            (L10N.tr("info-mount-point"), mount_point),
            (L10N.tr("info-capacity"), format_size(volume.size)),
            (L10N.tr("info-format"), format),
            (L10N.tr("info-device"), volume.device.clone()),
            (L10N.tr("info-connection"), disk.connection.clone()),
            (L10N.tr("info-location"), location(disk.group)),
        ];
        if let Some(available) = available {
            pairs.push((L10N.tr("info-available"), format_size(available)));
        }
        if !volume.uuid.is_empty() {
            pairs.push((L10N.tr("info-uuid"), volume.uuid.clone()));
        }
        match (volume.apple, volume.in_use) {
            (Some(Apple::MacOs), _) => {
                pairs.push((L10N.tr("info-access"), L10N.tr("access-macos")))
            }
            (Some(Apple::System), _) => {
                pairs.push((L10N.tr("info-access"), L10N.tr("access-apple")))
            }
            (None, true) => pairs.push((L10N.tr("info-access"), L10N.tr("access-in-use"))),
            (None, false) => {}
        }
        vec![
            boxed(Heading::new(
                self.icon(Self::volume_symbol(volume), HEADING_PIXELS),
                &volume.name,
                &subtitle,
                &format_size(volume.size),
            )),
            boxed(CapacityBar::new(segments, volume.size)),
            boxed(InfoGrid::new(pairs)),
        ]
    }

    fn tools(&self) -> Vec<Box<dyn Widget>> {
        let tool = |title: String, command: &'static str| {
            let intents = Rc::clone(&self.intents);
            let mut button =
                Button::new(&title, move || push(&intents, Intent::Command(command))).quiet();
            button.set_enabled(self.can(command));
            boxed(button)
        };
        let partition = match self.selection() {
            Some(Selected::Disk(_)) => ADD_PARTITION,
            _ => RESIZE,
        };
        vec![
            tool(L10N.tr("tool-add"), ADD_PARTITION),
            tool(L10N.tr("tool-delete"), DELETE_PARTITION),
            tool(L10N.tr("action-first-aid"), FIRST_AID),
            tool(L10N.tr("action-partition"), partition),
            tool(L10N.tr("action-erase"), ERASE),
            tool(L10N.tr("action-restore"), RESTORE),
            tool(self.mount_title(), MOUNT),
            tool(L10N.tr("action-eject"), EJECT),
        ]
    }

    fn refresh(&mut self) {
        let (rows, headings, expanded) = self.sidebar_rows();
        let selected = self.selected.clone();
        if let Some(sidebar) = self.ui.widget_as::<Table>(self.sidebar) {
            sidebar.set_rows(rows);
            sidebar.set_headings(headings);
            sidebar.set_expanded_rows(expanded);
            if let Some(key) = selected {
                sidebar.select(&[key]);
            }
        }
        let children = match self.selection() {
            Some(Selected::Disk(disk)) => self.disk_detail(disk),
            Some(Selected::Volume(disk, volume)) => self.volume_detail(disk, volume),
            None => vec![boxed(Label::new(&L10N.tr("no-disks")))],
        };
        let content = ScrollView::new(Stack::new(
            Axis::Vertical,
            CONTENT_SPACING,
            CONTENT_PADDING,
            children,
        ));
        let tools = self.tools();
        if let Some(root) = self.ui.widget_as::<Root>(self.root) {
            root.set_content(boxed(content));
            root.set_tools(tools);
        }
    }
}
