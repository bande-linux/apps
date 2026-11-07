use std::cell::{Cell, RefCell};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Button, Context, Event, FreeLayout, Label, ProgressBar, Size, Ui, Widget, WidgetId,
};
use bande_toolkit::{Elevation, Item, Radius, Rect, Shape};
use rustix::fs::{CWD, RenameFlags, renameat_with};

use crate::L10N;
use crate::renderer::{Command, DownloadId, Link, Offer};

const MAX_NAME: usize = 200;
const MAX_TRIES: u32 = 1000;
const PART: &str = ".part";
const FILE_MANAGER: &str = "org.freedesktop.FileManager1";
const FILE_MANAGER_PATH: &str = "/org/freedesktop/FileManager1";
pub(crate) const PANEL_WIDTH: f32 = 360.0;
const PANEL_INSET: f32 = 12.0;
const ROW_HEIGHT: f32 = 58.0;
const ACTION_WIDTH: f32 = 112.0;
const ACTION_HEIGHT: f32 = 26.0;
const BAR_HEIGHT: f32 = 6.0;
const TEXT_HEIGHT: f32 = 18.0;
const SHOWN_ROWS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum State {
    Active,
    Done,
    Failed(String),
}

pub(crate) struct Download {
    pub(crate) id: DownloadId,
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    part: PathBuf,
    pub(crate) received: u64,
    pub(crate) length: Option<u64>,
    pub(crate) state: State,
}

impl Download {
    pub(crate) fn fraction(&self) -> Option<f32> {
        let length = self.length.filter(|length| *length > 0)?;
        Some((self.received as f32 / length as f32).clamp(0.0, 1.0))
    }

    fn status(&self) -> String {
        match &self.state {
            State::Done => bande_fs::format_size(self.received),
            State::Failed(error) => error.clone(),
            State::Active => match self.length {
                Some(length) => L10N.tr_args(
                    "download-progress",
                    [
                        ("done", bande_fs::format_size(self.received).into()),
                        ("total", bande_fs::format_size(length).into()),
                    ],
                ),
                None => bande_fs::format_size(self.received),
            },
        }
    }
}

pub(crate) fn safe_name(name: Option<&str>) -> String {
    let cleaned: String = name
        .unwrap_or_default()
        .chars()
        .map(|character| match character {
            '/' | '\\' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let cleaned = cleaned
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    if cleaned.is_empty() {
        return L10N.tr("download-unnamed");
    }
    let (stem, extension) = split_name(cleaned);
    let room = MAX_NAME.saturating_sub(extension.len());
    let mut end = stem.len().min(room);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{extension}", &stem[..end])
}

fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot <= 16 => name.split_at(dot),
        _ => (name, ""),
    }
}

fn numbered(name: &str, number: u32) -> String {
    if number == 0 {
        return name.to_owned();
    }
    let (stem, extension) = split_name(name);
    format!("{stem} ({number}){extension}")
}

fn create(folder: &Path, name: &str) -> io::Result<(String, PathBuf, File)> {
    std::fs::create_dir_all(folder)?;
    for number in 0..MAX_TRIES {
        let shown = numbered(name, number);
        if folder.join(&shown).exists() {
            continue;
        }
        let part = folder.join(format!("{shown}{PART}"));
        match OpenOptions::new().write(true).create_new(true).open(&part) {
            Ok(file) => return Ok((shown, part, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

fn finish_file(part: &Path, folder: &Path, name: &str) -> io::Result<(String, PathBuf)> {
    for number in 0..MAX_TRIES {
        let shown = numbered(name, number);
        let path = folder.join(&shown);
        match renameat_with(CWD, part, CWD, &path, RenameFlags::NOREPLACE) {
            Ok(()) => return Ok((shown, path)),
            Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

pub(crate) struct Downloads {
    link: Link,
    items: RefCell<Vec<Download>>,
    listing: Cell<u64>,
}

impl Downloads {
    pub(crate) fn new(link: Link) -> Self {
        Self {
            link,
            items: RefCell::default(),
            listing: Cell::new(0),
        }
    }

    pub(crate) fn listing(&self) -> u64 {
        self.listing.get()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.borrow().is_empty()
    }

    pub(crate) fn overall(&self) -> Option<f32> {
        let items = self.items.borrow();
        let active: Vec<&Download> = items
            .iter()
            .filter(|item| item.state == State::Active)
            .collect();
        if active.is_empty() {
            return None;
        }
        let received: u64 = active.iter().map(|item| item.received).sum();
        let length: u64 = active.iter().filter_map(|item| item.length).sum();
        Some(match length {
            0 => 0.0,
            length => (received as f32 / length as f32).clamp(0.0, 1.0),
        })
    }

    pub(crate) fn offered(&self, offer: &Offer, folder: &Path) {
        let name = safe_name(offer.name.as_deref());
        let created = create(folder, &name);
        let mut items = self.items.borrow_mut();
        match created {
            Ok((shown, part, file)) => {
                self.link.send(Command::Save {
                    download: offer.download,
                    file: OwnedFd::from(file),
                });
                items.insert(
                    0,
                    Download {
                        id: offer.download,
                        path: folder.join(&shown),
                        name: shown,
                        part,
                        received: 0,
                        length: offer.length,
                        state: State::Active,
                    },
                );
            }
            Err(error) => {
                self.link.send(Command::CancelDownload {
                    download: offer.download,
                });
                items.insert(
                    0,
                    Download {
                        id: offer.download,
                        path: folder.join(&name),
                        part: PathBuf::new(),
                        name,
                        received: 0,
                        length: offer.length,
                        state: State::Failed(error.to_string()),
                    },
                );
            }
        }
        self.listing.set(self.listing.get() + 1);
    }

    pub(crate) fn progress(&self, id: DownloadId, received: u64) {
        if let Some(item) = self.items.borrow_mut().iter_mut().find(|item| item.id == id) {
            item.received = received;
        }
    }

    pub(crate) fn saved(&self, id: DownloadId, error: Option<String>) {
        let mut items = self.items.borrow_mut();
        let Some(item) = items.iter_mut().find(|item| item.id == id) else {
            return;
        };
        item.state = match error {
            Some(error) => {
                let _ = std::fs::remove_file(&item.part);
                State::Failed(error)
            }
            None => {
                let folder = item.path.parent().map(Path::to_path_buf).unwrap_or_default();
                match finish_file(&item.part, &folder, &item.name) {
                    Ok((name, path)) => {
                        item.received = item.received.max(file_size(&path));
                        item.name = name;
                        item.path = path;
                        State::Done
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(&item.part);
                        State::Failed(error.to_string())
                    }
                }
            }
        };
        self.listing.set(self.listing.get() + 1);
    }

    pub(crate) fn cancel(&self, id: DownloadId) {
        let mut items = self.items.borrow_mut();
        let Some(index) = items.iter().position(|item| item.id == id) else {
            return;
        };
        let item = items.remove(index);
        if item.state == State::Active {
            self.link.send(Command::CancelDownload { download: id });
            let _ = std::fs::remove_file(&item.part);
        }
        self.listing.set(self.listing.get() + 1);
    }

    pub(crate) fn clear_finished(&self) {
        self.items
            .borrow_mut()
            .retain(|item| item.state == State::Active);
        self.listing.set(self.listing.get() + 1);
    }

    pub(crate) fn path(&self, id: DownloadId) -> Option<PathBuf> {
        let items = self.items.borrow();
        let item = items
            .iter()
            .find(|item| item.id == id && item.state == State::Done)?;
        Some(item.path.clone())
    }

    fn with_items<T>(&self, read: impl FnOnce(&[Download]) -> T) -> T {
        read(&self.items.borrow())
    }

    pub(crate) fn panel_height(&self) -> f32 {
        let rows = self.items.borrow().len().min(SHOWN_ROWS);
        2.0 * PANEL_INSET + rows as f32 * ROW_HEIGHT
    }
}

impl Drop for Downloads {
    fn drop(&mut self) {
        for item in self.items.borrow().iter() {
            if item.state == State::Active {
                let _ = std::fs::remove_file(&item.part);
            }
        }
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |metadata| metadata.len())
}

pub(crate) fn show_in_files(path: PathBuf) {
    let uris = vec![bande_xdg::file_uri(&path)];
    let spawned = std::thread::Builder::new()
        .name("show-in-files".to_owned())
        .spawn(move || {
            let shown = zbus::blocking::Connection::session().and_then(|session| {
                session
                    .call_method(
                        Some(FILE_MANAGER),
                        FILE_MANAGER_PATH,
                        Some(FILE_MANAGER),
                        "ShowItems",
                        &(uris, ""),
                    )
                    .map(drop)
            });
            if let Err(error) = shown {
                tracing::warn!("cannot show the download in Files: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot show the download in Files: {error}");
    }
}

pub(crate) struct Row {
    id: DownloadId,
    status: WidgetId,
    bar: Option<WidgetId>,
    shown: String,
}

pub(crate) struct Panel {
    id: WidgetId,
    frame: Rect,
    children: Vec<Box<dyn Widget>>,
    rows: Vec<Row>,
    layout: WidgetId,
}

pub(crate) enum PanelAction {
    Cancel(DownloadId),
    Show(DownloadId),
}

impl Panel {
    pub(crate) fn new(
        downloads: &Downloads,
        act: impl Fn(PanelAction) + Clone + 'static,
    ) -> Self {
        let mut layout = FreeLayout::new();
        let mut rows = Vec::new();
        downloads.with_items(|items| {
            for (index, item) in items.iter().take(SHOWN_ROWS).enumerate() {
                let top = PANEL_INSET + index as f32 * ROW_HEIGHT;
                let text = PANEL_WIDTH - 3.0 * PANEL_INSET - ACTION_WIDTH;
                layout.insert(
                    Label::new(&item.name),
                    Rect::new(PANEL_INSET, top, text, TEXT_HEIGHT),
                );
                let shown = item.status();
                let status = layout.insert(
                    Label::new(&shown).secondary(),
                    Rect::new(PANEL_INSET, top + TEXT_HEIGHT, text, TEXT_HEIGHT),
                );
                let bar = (item.state == State::Active).then(|| {
                    layout.insert(
                        ProgressBar::new(&item.name, item.fraction()),
                        Rect::new(PANEL_INSET, top + 2.0 * TEXT_HEIGHT + 4.0, text, BAR_HEIGHT),
                    )
                });
                let (id, action) = (item.id, act.clone());
                let (label, press): (String, Box<dyn Fn()>) = match &item.state {
                    State::Active => (
                        L10N.tr("cancel"),
                        Box::new(move || action(PanelAction::Cancel(id))),
                    ),
                    State::Done => (
                        L10N.tr("show-in-files"),
                        Box::new(move || action(PanelAction::Show(id))),
                    ),
                    State::Failed(_) => (
                        L10N.tr("remove"),
                        Box::new(move || action(PanelAction::Cancel(id))),
                    ),
                };
                layout.insert(
                    Button::new(&label, move || press()),
                    Rect::new(
                        PANEL_WIDTH - PANEL_INSET - ACTION_WIDTH,
                        top + 4.0,
                        ACTION_WIDTH,
                        ACTION_HEIGHT,
                    ),
                );
                rows.push(Row {
                    id: item.id,
                    status,
                    bar,
                    shown,
                });
            }
        });
        let layout_id = layout.id();
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            children: vec![Box::new(layout)],
            rows,
            layout: layout_id,
        }
    }

    pub(crate) fn refresh(ui: &mut Ui, panel: WidgetId, downloads: &Downloads) {
        let Some((mut rows, layout)) = ui
            .widget_as::<Panel>(panel)
            .map(|panel| (std::mem::take(&mut panel.rows), panel.layout))
        else {
            return;
        };
        downloads.with_items(|items| {
            for row in &mut rows {
                let Some(item) = items.iter().find(|item| item.id == row.id) else {
                    continue;
                };
                let status = item.status();
                if status != row.shown
                    && let Some(id) = ui
                        .widget_as::<FreeLayout>(layout)
                        .and_then(|layout| layout.replace(row.status, Label::new(&status).secondary()))
                {
                    row.status = id;
                    row.shown = status;
                }
                if let Some(bar) = row.bar {
                    ui.update_widget(bar, |bar: &mut ProgressBar, cx| {
                        bar.set_value(cx, item.fraction());
                    });
                }
            }
        });
        if let Some(panel) = ui.widget_as::<Panel>(panel) {
            panel.rows = rows;
        }
    }
}

impl Widget for Panel {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        for child in &mut self.children {
            child.measure(cx, Size::new(frame.width, frame.height));
            child.place(cx, frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let palette = cx.palette;
        items.extend(Item::shadows(
            Shape::rounded(self.frame, Radius::M),
            Elevation::Overlay,
            palette.scheme,
            true,
        ));
        items.push(Item::Fill {
            shape: Shape::rounded(self.frame, Radius::M),
            paint: palette.raised.into(),
        });
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(L10N.tr("downloads"));
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        matches!(
            event,
            Event::PointerDown(_) | Event::PointerUp(_) | Event::Scroll(_)
        )
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

#[cfg(test)]
mod tests {
    use super::safe_name;

    #[test]
    fn a_hostile_file_name_stays_in_the_downloads_folder() {
        assert_eq!(safe_name(Some("../../.bashrc")), "_.._.bashrc");
        assert_eq!(safe_name(Some("..")), L10N_UNNAMED);
        assert_eq!(safe_name(Some("a\u{0}b/c.txt")), "a_b_c.txt");
        assert!(safe_name(Some(&"x".repeat(1000))).len() <= super::MAX_NAME);
    }

    const L10N_UNNAMED: &str = "download";
}
