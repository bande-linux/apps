use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Saved, View, WindowId};
use bande_app_menu::Entry;
use bande_inbox::Inbox;
use bande_preview::{
    InfoPanel, L10N, PreviewView, Request, Row, SidebarMode, WORKER_FLAG, Zoom, argument_path,
    document_rows, file_rows, info_ui, picture_rows,
};
use bande_toolkit::ui::{Ui, WidgetId};
use bande_toolkit::{Appearance, Rect, Scene, Window, WindowEvent};
use bande_xdg::spawn_detached;

static INFO: Info = Info {
    id: "org.bande.Preview",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

const WINDOW_SIZE: (u32, u32) = (900, 680);
const INFO_SIZE: (u32, u32) = (380, 460);
const INFO_TAG: &str = "info";
const FILES_APP_ID: &str = "org.bande.Files";
const FILES: &str = "bande-files";

const OPEN: &str = "open";
const SHOW_IN_FILES: &str = "show-in-files";
const GET_INFO: &str = "get-info";
const COPY: &str = "copy";
const SIDEBAR: &str = "sidebar";
const ACTUAL_SIZE: &str = "actual-size";
const ZOOM_TO_FIT: &str = "zoom-to-fit";
const ZOOM_IN: &str = "zoom-in";
const ZOOM_OUT: &str = "zoom-out";
const SLIDESHOW: &str = "slideshow";
const PLAY: &str = "play";
const ROTATE_LEFT: &str = "rotate-left";
const ROTATE_RIGHT: &str = "rotate-right";
const FLIP_HORIZONTAL: &str = "flip-horizontal";
const FLIP_VERTICAL: &str = "flip-vertical";
const FIND: &str = "find";
const FIND_NEXT: &str = "find-next";
const FIND_PREVIOUS: &str = "find-previous";
const THUMBNAILS: &str = "thumbnails";
const CONTENTS: &str = "contents";
const NEXT: &str = "next";
const PREVIOUS: &str = "previous";

enum Pane {
    Main(Box<PreviewView>),
    Info(Box<Ui>, WidgetId),
}

struct Preview {
    chosen: Option<Inbox<PathBuf>>,
    info: Option<WindowId>,
}

impl Preview {
    fn open_window(&mut self, cx: &mut Cx<Self>, paths: Vec<PathBuf>) {
        let title = paths
            .first()
            .map(|path| bande_preview::file_name(path))
            .unwrap_or_else(|| L10N.tr("app-name"));
        let opened = cx.open(Kind::Main, &title, WINDOW_SIZE, |appearance| {
            Pane::Main(Box::new(PreviewView::new(appearance, paths)))
        });
        if let Err(error) = opened {
            tracing::warn!("cannot open a window: {error}");
        }
        cx.refresh_menus();
    }

    fn open_paths(&mut self, cx: &mut Cx<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let empty = cx.windows().find(
            |window| matches!(cx.view(*window), Some(Pane::Main(view)) if view.paths().is_empty()),
        );
        if let Some(window) = empty
            && let Some((shown, Pane::Main(view))) = cx.window(window)
        {
            view.add(paths);
            shown.request_redraw();
            return cx.raise(window, None);
        }
        self.open_window(cx, paths);
    }

    fn choose(&mut self, cx: &mut Cx<Self>) {
        let Some(session) = cx.session().cloned() else {
            return;
        };
        if self.chosen.is_none() {
            self.chosen = Inbox::new()
                .inspect_err(|error| tracing::warn!("cannot open the chooser: {error}"))
                .ok();
        }
        let Some(outbox) = self.chosen.as_ref().map(Inbox::outbox) else {
            return;
        };
        let title = L10N.tr("open-title");
        let filter = (
            L10N.tr("open-filter"),
            [
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/webp",
                "image/svg+xml",
                "application/pdf",
            ]
            .into_iter()
            .map(|mime| (1, mime.to_owned()))
            .collect(),
        );
        let spawned = std::thread::Builder::new()
            .name("open".to_owned())
            .spawn(
                move || match bande_app::choose_file(&session, &title, Some(filter)) {
                    Ok(Some(path)) => {
                        outbox.send(path);
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!("cannot choose a file: {error}"),
                },
            );
        if let Err(error) = spawned {
            tracing::warn!("cannot open the chooser: {error}");
        }
    }

    fn main_view(cx: &Cx<Self>) -> Option<&PreviewView> {
        match cx.view(Self::last_main(cx)?)? {
            Pane::Main(view) => Some(view),
            Pane::Info(..) => None,
        }
    }

    fn last_main(cx: &Cx<Self>) -> Option<WindowId> {
        let main = |window: &WindowId| matches!(cx.view(*window), Some(Pane::Main(_)));
        cx.focused()
            .filter(main)
            .or_else(|| cx.windows().filter(main).last())
    }

    fn info_sections(cx: &Cx<Self>) -> Vec<Vec<Row>> {
        let Some(view) = Self::main_view(cx) else {
            return Vec::new();
        };
        let mut sections = Vec::new();
        if let Some(path) = view.path() {
            sections.push(file_rows(path));
        }
        if let Some(picture) = view.picture() {
            sections.push(picture_rows(picture));
        }
        if let Some(opened) = view.document() {
            sections.push(document_rows(opened));
        }
        sections
    }

    fn show_info(&mut self, cx: &mut Cx<Self>) {
        let sections = Self::info_sections(cx);
        if let Some(window) = self.info.filter(|window| cx.view(*window).is_some()) {
            self.refresh_info(cx, sections);
            return cx.raise(window, None);
        }
        let opened = cx.open(
            Kind::Utility(INFO_TAG),
            &L10N.tr("inspector-title"),
            INFO_SIZE,
            |appearance| {
                let (ui, panel) = info_ui(appearance, sections);
                Pane::Info(Box::new(ui), panel)
            },
        );
        match opened {
            Ok(window) => self.info = Some(window),
            Err(error) => tracing::warn!("cannot open the inspector: {error}"),
        }
    }

    fn refresh_info(&mut self, cx: &mut Cx<Self>, sections: Vec<Vec<Row>>) {
        let Some(window) = self.info else {
            return;
        };
        if let Some((shown, Pane::Info(ui, panel))) = cx.window(window) {
            ui.update_widget(*panel, |panel: &mut InfoPanel, _| {
                panel.set_sections(sections)
            });
            shown.request_redraw();
        }
    }

    fn print(cx: &mut Cx<Self>, window: WindowId) {
        let Some((_, Pane::Main(view))) = cx.window(window) else {
            return;
        };
        let Some(path) = view.path().map(Path::to_path_buf) else {
            return;
        };
        let title = view.title();
        let document = match view.is_document() {
            true => Some(bande_app::Document::File(path)),
            false => bande_app::image_document(path),
        };
        if let Some(document) = document {
            cx.print(&title, document);
        }
    }

    fn show_in_files(cx: &mut Cx<Self>, window: WindowId) {
        let Some((shown, Pane::Main(view))) = cx.window(window) else {
            return;
        };
        let Some(path) = view.path().map(Path::to_path_buf) else {
            return;
        };
        let token = shown.activation_token(FILES_APP_ID).ok();
        let command = [FILES.to_owned(), path.to_string_lossy().into_owned()];
        if let Err(error) = spawn_detached(FILES_APP_ID, &command, None, token.as_deref()) {
            tracing::warn!("cannot show {} in Files: {error}", path.display());
        }
    }

    fn serve(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let requests = match cx.window(window) {
            Some((_, Pane::Main(view))) => view.take_requests(),
            _ => return,
        };
        for request in requests {
            match request {
                Request::ShowInfo => self.show_info(cx),
                Request::Changed if self.info.is_some() => {
                    let sections = Self::info_sections(cx);
                    self.refresh_info(cx, sections);
                }
                Request::Changed => {}
            }
        }
        cx.refresh_menus();
    }
}

fn paths(arguments: &[String], directory: &Path) -> Vec<PathBuf> {
    arguments
        .iter()
        .filter(|argument| !argument.starts_with('-'))
        .map(|argument| argument_path(argument, directory))
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedWindow {
    paths: Vec<PathBuf>,
    selected: Option<PathBuf>,
}

impl Program for Preview {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut preview = Self {
            chosen: None,
            info: None,
        };
        if !arguments.is_empty() {
            let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            preview.open_window(cx, paths(&arguments, &directory));
        }
        Ok(preview)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = Self::main_view(cx);
        let picture = view.is_some_and(PreviewView::has_picture);
        let document = view.is_some_and(PreviewView::is_document);
        let file = view.is_some_and(|view| view.path().is_some());
        let animated = view.is_some_and(PreviewView::is_animated);
        let copyable = view.is_some_and(PreviewView::can_copy);
        let contents = view.is_some_and(PreviewView::has_contents);
        let several = view.is_some_and(PreviewView::has_sidebar);
        let item = |key: &str, action: &str, shortcut: Option<&str>, enabled: bool| {
            let entry = Entry::item(&L10N.tr(key), action).enabled(enabled);
            match shortcut {
                Some(shortcut) => entry.shortcut(shortcut),
                None => entry,
            }
        };
        let file_menu = vec![
            item("menu-open", OPEN, Some("Ctrl+O"), true),
            item("menu-show-in-files", SHOW_IN_FILES, None, file),
            item("menu-get-info", GET_INFO, Some("Ctrl+I"), file),
            Entry::separator(),
            bande_app_menu::print_item(file),
        ];
        let edit = vec![
            item("menu-copy", COPY, Some("Ctrl+C"), copyable),
            Entry::separator(),
            item("menu-find", FIND, Some("Ctrl+F"), document),
            item("menu-find-next", FIND_NEXT, Some("Ctrl+G"), document),
            item(
                "menu-find-previous",
                FIND_PREVIOUS,
                Some("Shift+Ctrl+G"),
                document,
            ),
        ];
        let sidebar_key = match view.is_some_and(PreviewView::sidebar_shown) && several {
            true => "menu-hide-sidebar",
            false => "menu-show-sidebar",
        };
        let slideshow_key = match view.is_some_and(PreviewView::in_slideshow) {
            true => "menu-stop-slideshow",
            false => "menu-slideshow",
        };
        let mut view_menu = vec![item(sidebar_key, SIDEBAR, Some("F9"), several)];
        if document {
            view_menu.extend([
                item("menu-thumbnails", THUMBNAILS, None, several),
                item("menu-contents", CONTENTS, None, contents),
            ]);
        }
        let zoomable = picture || document;
        view_menu.extend([
            Entry::separator(),
            item("menu-actual-size", ACTUAL_SIZE, Some("Ctrl+0"), zoomable),
            item("menu-zoom-to-fit", ZOOM_TO_FIT, None, zoomable),
            item("menu-zoom-in", ZOOM_IN, Some("Ctrl+="), zoomable),
            item("menu-zoom-out", ZOOM_OUT, Some("Ctrl+-"), zoomable),
            Entry::separator(),
            item("menu-play", PLAY, None, animated),
            item(slideshow_key, SLIDESHOW, Some("F5"), file && !document),
        ]);
        let go = vec![
            item("menu-next", NEXT, Some("Right"), file),
            item("menu-previous", PREVIOUS, Some("Left"), file),
        ];
        let tools = vec![
            item(
                "menu-rotate-left",
                ROTATE_LEFT,
                Some("Shift+Ctrl+R"),
                picture,
            ),
            item("menu-rotate-right", ROTATE_RIGHT, Some("Ctrl+R"), picture),
            Entry::separator(),
            item("menu-flip-horizontal", FLIP_HORIZONTAL, None, picture),
            item("menu-flip-vertical", FLIP_VERTICAL, None, picture),
        ];
        Menus {
            menus: vec![
                Entry::submenu(&L10N.tr("menu-file"), file_menu),
                Entry::submenu(&L10N.tr("menu-edit"), edit),
                Entry::submenu(&L10N.tr("menu-view"), view_menu),
                Entry::submenu(&L10N.tr("menu-go"), go),
                Entry::submenu(&L10N.tr("menu-tools"), tools),
            ],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        match command {
            OPEN => return self.choose(cx),
            GET_INFO => return self.show_info(cx),
            _ => {}
        }
        let Some(window) = window
            .filter(|window| matches!(cx.view(*window), Some(Pane::Main(_))))
            .or_else(|| Self::last_main(cx))
        else {
            return;
        };
        if command == SHOW_IN_FILES {
            return Self::show_in_files(cx, window);
        }
        if command == bande_app_menu::PRINT {
            return Self::print(cx, window);
        }
        if let Some((shown, Pane::Main(view))) = cx.window(window) {
            match command {
                COPY => view.copy(Some(shown)),
                FIND => view.focus_search(),
                FIND_NEXT => view.find_next(true),
                FIND_PREVIOUS => view.find_next(false),
                THUMBNAILS => view.show_sidebar(SidebarMode::Thumbnails),
                CONTENTS => view.show_sidebar(SidebarMode::Contents),
                SIDEBAR => view.toggle_sidebar(),
                ACTUAL_SIZE => view.set_zoom(Zoom::Scale(1.0)),
                ZOOM_TO_FIT => view.set_zoom(Zoom::Fit),
                ZOOM_IN => view.zoom_by(1.0),
                ZOOM_OUT => view.zoom_by(-1.0),
                SLIDESHOW => view.toggle_slideshow(),
                PLAY => view.toggle_playing(),
                ROTATE_LEFT => view.rotate(false),
                ROTATE_RIGHT => view.rotate(true),
                FLIP_HORIZONTAL => view.flip(false),
                FLIP_VERTICAL => view.flip(true),
                NEXT => view.go(true),
                PREVIOUS => view.go(false),
                _ => return,
            }
            shown.request_redraw();
        }
        self.serve(cx, window);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        self.open_paths(cx, paths(&arguments, directory));
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.open_window(cx, Vec::new());
    }

    fn save_window(&self, cx: &Cx<Self>, window: WindowId) -> Option<Saved> {
        let Pane::Main(view) = cx.view(window)? else {
            return None;
        };
        Saved::of(&SavedWindow {
            paths: view.paths().to_vec(),
            selected: view.path().map(Path::to_path_buf),
        })
    }

    fn restore_window(&mut self, cx: &mut Cx<Self>, saved: Saved) {
        let Some(mut window) = saved.read::<SavedWindow>() else {
            return;
        };
        window.paths.retain(|path| path.exists());
        if window.paths.is_empty() {
            return;
        }
        let selected = window
            .paths
            .iter()
            .position(|path| Some(path) == window.selected.as_ref());
        self.open_window(cx, window.paths);
        if let Some(selected) = selected
            && let Some(opened) = cx.windows().last()
            && let Some((_, Pane::Main(view))) = cx.window(opened)
        {
            view.select(selected);
        }
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, _view: Pane) {
        if self.info == Some(window) {
            self.info = None;
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.chosen.iter().map(Inbox::fd).collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        let chosen = self.chosen.as_mut().map(Inbox::drain).unwrap_or_default();
        self.open_paths(cx, chosen);
    }
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        match self {
            Self::Main(view) => view.scene(window),
            Self::Info(ui, _) => View::scene(ui.as_mut(), window),
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match self {
            Self::Main(view) => view.event(window, event),
            Self::Info(ui, _) => View::event(ui.as_mut(), window, event),
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        match self {
            Self::Main(view) => view.set_appearance(appearance),
            Self::Info(ui, _) => ui.set_appearance(appearance),
        }
    }

    fn set_active(&mut self, active: bool) {
        match self {
            Self::Main(view) => view.set_active(active),
            Self::Info(ui, _) => View::set_active(ui.as_mut(), active),
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        match self {
            Self::Main(view) => view.sources(),
            Self::Info(..) => Vec::new(),
        }
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) -> bool {
        match self {
            Self::Main(view) => view.source_ready(window, index),
            Self::Info(..) => false,
        }
    }

    fn deadline(&self) -> Option<Instant> {
        match self {
            Self::Main(view) => View::deadline(view.as_ref()),
            Self::Info(ui, _) => ui.deadline(),
        }
    }

    fn tick(&mut self, window: &mut Window, now: Instant) -> bool {
        match self {
            Self::Main(view) => View::tick(view.as_mut(), window, now),
            Self::Info(..) => true,
        }
    }

    fn animating(&self) -> bool {
        match self {
            Self::Main(view) => view.animating(),
            Self::Info(ui, _) => ui.is_animating(),
        }
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        match self {
            Self::Main(view) => view.accessibility(),
            Self::Info(ui, _) => Some(ui.accessibility()),
        }
    }

    fn text_input_area(&self) -> Option<Rect> {
        match self {
            Self::Main(view) => view.text_input_area(),
            Self::Info(ui, _) => ui.text_input_area(),
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if let [flag, path] = arguments.as_slice()
        && flag == WORKER_FLAG
    {
        return bande_preview::run_pdf_worker(Path::new(path));
    }
    bande_app::run::<Preview>(&INFO)
}
