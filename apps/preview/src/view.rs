use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_app::View;
use bande_inbox::Inbox;
use bande_thumbnails::{Request as ThumbRequest, ThumbnailSize, Thumbnails};
use bande_toolkit::ui::{Button, Context, Event, FreeLayout, Key, Ui, WidgetId};
use bande_toolkit::{
    Appearance, FrameRegions, Header, Image, Radius, Rect, SHEET_INSET, Scene, TextSystem,
    Transfer, WINDOW_HEADER_HEIGHT, Window, WindowEvent, window_controls_end,
};

use crate::canvas::{Canvas, Zoom};
use crate::copy::encode_png;
use crate::decode::{Decoded, Failure};
use crate::files::{mime_of_name, neighbour};
use crate::loader::{Loader, Opened};
use crate::sidebar::{SIDEBAR_WIDTH, Sidebar, Thumb};
use crate::{L10N, tr_with};

const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const SIDE: f32 = 12.0;
const GAP: f32 = 4.0;
const SYMBOL_BUTTON: f32 = 28.0;
const SYMBOL_PIXELS: u32 = 16;
const TITLE_GAP: f32 = 16.0;
const SLIDE: Duration = Duration::from_secs(3);
const THUMB_PIXELS: u32 = 256;

const SIDEBAR_SYMBOL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><rect x="1.8" y="2.8" width="12.4" height="10.4" rx="2" stroke="#201E1A" stroke-width="1.4" fill="none"/><path d="M6 3v10" stroke="#201E1A" stroke-width="1.4"/></svg>"##;
const ZOOM_OUT_SYMBOL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.6" stroke="#201E1A" stroke-width="1.4" fill="none"/><path d="M10.4 10.4 14 14M4.8 7h4.4" stroke="#201E1A" stroke-width="1.4" stroke-linecap="round"/></svg>"##;
const ZOOM_IN_SYMBOL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.6" stroke="#201E1A" stroke-width="1.4" fill="none"/><path d="M10.4 10.4 14 14M4.8 7h4.4M7 4.8v4.4" stroke="#201E1A" stroke-width="1.4" stroke-linecap="round"/></svg>"##;
const ROTATE_SYMBOL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M3.5 7.5A5 5 0 1 1 5 11.5" stroke="#201E1A" stroke-width="1.4" stroke-linecap="round" fill="none"/><path d="M1.8 5.2 3.5 7.6 5.9 5.9" stroke="#201E1A" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##;
const INFO_SYMBOL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.2" stroke="#201E1A" stroke-width="1.4" fill="none"/><path d="M8 7.2v4" stroke="#201E1A" stroke-width="1.5" stroke-linecap="round"/><circle cx="8" cy="4.9" r="0.95" fill="#201E1A"/></svg>"##;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Sidebar,
    ZoomIn,
    ZoomOut,
    RotateLeft,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    ShowInfo,
    Changed,
}

type Signals = Rc<RefCell<Vec<Signal>>>;

fn symbol(svg: &str, label: &str, signals: &Signals, signal: Signal) -> Button {
    let signals = Rc::clone(signals);
    let click = move || signals.borrow_mut().push(signal);
    match Image::from_svg(svg.as_bytes(), SYMBOL_PIXELS, SYMBOL_PIXELS) {
        Some(image) => Button::symbol(image, label, click),
        None => Button::new(label, click),
    }
    .tooltip(label)
}

struct Toolbar {
    sidebar: WidgetId,
    tools: [WidgetId; 4],
}

pub struct PreviewView {
    ui: Ui,
    signals: Signals,
    host: WidgetId,
    canvas: WidgetId,
    sidebar: WidgetId,
    toolbar: Toolbar,
    paths: Vec<PathBuf>,
    current: usize,
    loader: Option<Loader>,
    loading: bool,
    thumbnails: Option<Thumbnails>,
    copies: Option<Inbox<Option<Vec<u8>>>>,
    sidebar_shown: bool,
    slideshow: Option<Instant>,
    hidden: bool,
    full_screen: bool,
    requests: Vec<Request>,
}

fn failure_text(failure: &Failure) -> String {
    match failure {
        Failure::Unreadable => L10N.tr("failure-unreadable"),
        Failure::NotSupported => L10N.tr("failure-not-supported"),
        Failure::Damaged => L10N.tr("failure-damaged"),
        Failure::FileTooLarge => L10N.tr("failure-file-too-large"),
        Failure::TooLarge { width, height } => tr_with(
            "failure-too-large",
            &[("width", width.to_string()), ("height", height.to_string())],
        ),
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl PreviewView {
    pub fn new(appearance: Appearance, paths: Vec<PathBuf>) -> Self {
        let signals: Signals = Rc::default();
        let mut host = FreeLayout::new();
        let sidebar_button = host.insert_in_header(
            symbol(
                SIDEBAR_SYMBOL,
                &L10N.tr("toggle-sidebar"),
                &signals,
                Signal::Sidebar,
            ),
            Rect::default(),
        );
        let tools = [
            (INFO_SYMBOL, "get-info", Signal::Info),
            (ZOOM_OUT_SYMBOL, "zoom-out", Signal::ZoomOut),
            (ZOOM_IN_SYMBOL, "zoom-in", Signal::ZoomIn),
            (ROTATE_SYMBOL, "rotate-left", Signal::RotateLeft),
        ]
        .map(|(svg, label, signal)| {
            host.insert_in_header(
                symbol(svg, &L10N.tr(label), &signals, signal),
                Rect::default(),
            )
        });
        let sidebar = host.insert(Sidebar::new(), Rect::default());
        let canvas = host.insert(Canvas::new(), Rect::default());
        let host_id = bande_toolkit::ui::Widget::id(&host);
        let mut ui = Ui::new(
            Box::new(host),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        let several = paths.len() > 1;
        let mut view = Self {
            ui,
            signals,
            host: host_id,
            canvas,
            sidebar,
            toolbar: Toolbar {
                sidebar: sidebar_button,
                tools,
            },
            paths,
            current: 0,
            loader: Loader::new()
                .inspect_err(|error| tracing::warn!("cannot start decoding: {error}"))
                .ok(),
            loading: false,
            thumbnails: None,
            copies: None,
            sidebar_shown: several,
            slideshow: None,
            hidden: false,
            full_screen: false,
            requests: Vec::new(),
        };
        view.fill_sidebar();
        view.load_current();
        if view.paths.is_empty() {
            let (title, detail) = (L10N.tr("empty-title"), L10N.tr("empty-detail"));
            view.with_canvas(|canvas| canvas.show_message(&title, &detail));
        }
        view.ui.set_focus(canvas);
        view
    }

    pub fn ui_ref(&self) -> &Ui {
        &self.ui
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn path(&self) -> Option<&Path> {
        self.paths.get(self.current).map(PathBuf::as_path)
    }

    pub fn title(&self) -> String {
        self.path()
            .map(file_name)
            .unwrap_or_else(|| L10N.tr("app-name"))
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn add(&mut self, paths: Vec<PathBuf>) {
        let first = self.paths.len();
        for path in paths {
            if !self.paths.contains(&path) {
                self.paths.push(path);
            }
        }
        if self.paths.len() > first {
            self.current = first;
            self.sidebar_shown = self.paths.len() > 1;
            self.fill_sidebar();
            self.load_current();
        }
    }

    fn with_canvas<T>(&mut self, update: impl FnOnce(&mut Canvas) -> T) -> Option<T> {
        self.ui.widget_as::<Canvas>(self.canvas).map(update)
    }

    pub fn canvas(&self) -> Option<&Canvas> {
        self.ui.widget_ref::<Canvas>(self.canvas)
    }

    pub fn picture(&self) -> Option<&Decoded> {
        self.canvas().and_then(Canvas::picture)
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    fn load_current(&mut self) {
        let Some(path) = self.path().map(Path::to_path_buf) else {
            return;
        };
        let name = file_name(&path);
        self.with_canvas(|canvas| canvas.clear(&name));
        self.loading = match self.loader.as_mut() {
            Some(loader) => {
                loader.load(&path);
                true
            }
            None => false,
        };
        let current = self.current;
        if let Some(sidebar) = self.ui.widget_as::<Sidebar>(self.sidebar) {
            sidebar.select(current);
        }
        self.requests.push(Request::Changed);
    }

    fn take_loaded(&mut self) -> bool {
        let Some(loaded) = self.loader.as_mut().and_then(Loader::take) else {
            return false;
        };
        if Some(loaded.path.as_path()) != self.path() {
            return false;
        }
        self.loading = false;
        let name = file_name(&loaded.path);
        let now = Instant::now();
        match loaded.result {
            Ok(Opened::Image(decoded)) => {
                let label = tr_with(
                    "image-label",
                    &[
                        ("name", name),
                        ("width", decoded.width.to_string()),
                        ("height", decoded.height.to_string()),
                    ],
                );
                self.with_canvas(|canvas| canvas.show(decoded, &label, now));
            }
            Ok(Opened::Document) => {
                let detail = failure_text(&Failure::NotSupported);
                let title = tr_with("cannot-open", &[("name", name)]);
                self.with_canvas(|canvas| canvas.show_message(&title, &detail));
            }
            Err(failure) => {
                let detail = failure_text(&failure);
                let title = tr_with("cannot-open", &[("name", name)]);
                self.with_canvas(|canvas| canvas.show_message(&title, &detail));
            }
        }
        self.requests.push(Request::Changed);
        true
    }

    fn fill_sidebar(&mut self) {
        let thumbs = match self.paths.len() > 1 {
            true => self
                .paths
                .iter()
                .map(|path| Thumb {
                    name: file_name(path),
                    image: None,
                })
                .collect(),
            false => Vec::new(),
        };
        let current = self.current;
        if let Some(sidebar) = self.ui.widget_as::<Sidebar>(self.sidebar) {
            sidebar.set_thumbs(thumbs);
            sidebar.select(current);
        }
        if self.paths.len() < 2 {
            return;
        }
        if self.thumbnails.is_none() {
            self.thumbnails = Thumbnails::for_user(&bande_xdg::data_dirs());
        }
        let size = ThumbnailSize::for_pixels(THUMB_PIXELS);
        let requests = self
            .paths
            .iter()
            .map(|path| ThumbRequest {
                path: path.clone(),
                mime: mime_of_name(path).to_owned(),
                size,
            })
            .collect();
        if let Some(thumbnails) = &self.thumbnails {
            thumbnails.want(requests);
        }
    }

    fn take_thumbnails(&mut self) -> bool {
        let Some(thumbnails) = self.thumbnails.as_mut() else {
            return false;
        };
        let done = thumbnails.take();
        let mut changed = false;
        for thumbnailed in done {
            let Some(picture) = thumbnailed.picture else {
                continue;
            };
            let Some(index) = self.paths.iter().position(|path| *path == thumbnailed.path) else {
                continue;
            };
            if let Some(image) = Image::from_rgba(picture.width, picture.height, picture.rgba)
                && let Some(sidebar) = self.ui.widget_as::<Sidebar>(self.sidebar)
            {
                sidebar.set_image(index, image);
                changed = true;
            }
        }
        changed
    }

    pub fn go(&mut self, forward: bool) {
        if self.paths.len() > 1 {
            let count = self.paths.len();
            self.current = match forward {
                true => (self.current + 1) % count,
                false => (self.current + count - 1) % count,
            };
            return self.load_current();
        }
        let Some(next) = self.path().and_then(|path| neighbour(path, forward)) else {
            return;
        };
        if let Some(path) = self.paths.get_mut(self.current) {
            *path = next;
        }
        self.fill_sidebar();
        self.load_current();
    }

    pub fn select(&mut self, index: usize) {
        if index < self.paths.len() && index != self.current {
            self.current = index;
            self.load_current();
        }
    }

    pub fn zoom_by(&mut self, steps: f32) {
        self.with_canvas(|canvas| canvas.zoom_by(steps));
    }

    pub fn set_zoom(&mut self, zoom: Zoom) {
        self.with_canvas(|canvas| canvas.set_zoom(zoom));
    }

    pub fn rotate(&mut self, clockwise: bool) {
        self.with_canvas(|canvas| canvas.orient(canvas.orientation().rotated(clockwise)));
    }

    pub fn flip(&mut self, vertical: bool) {
        self.with_canvas(|canvas| canvas.orient(canvas.orientation().flipped(vertical)));
    }

    pub fn has_picture(&self) -> bool {
        self.picture().is_some()
    }

    pub fn is_animated(&self) -> bool {
        self.canvas().is_some_and(Canvas::is_animated)
    }

    pub fn toggle_playing(&mut self) {
        let now = Instant::now();
        self.with_canvas(|canvas| canvas.toggle_playing(now));
    }

    pub fn sidebar_shown(&self) -> bool {
        self.sidebar_shown
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_shown = !self.sidebar_shown;
    }

    pub fn in_slideshow(&self) -> bool {
        self.slideshow.is_some()
    }

    pub fn toggle_slideshow(&mut self) {
        self.slideshow = match self.slideshow {
            Some(_) => None,
            None => Some(Instant::now() + SLIDE),
        };
        let dark = self.slideshow.is_some();
        self.with_canvas(|canvas| {
            canvas.set_dark(dark);
            canvas.set_zoom(Zoom::Fit);
        });
    }

    pub fn copy(&mut self) {
        let Some(picture) = self.picture() else {
            return;
        };
        let Some(source) = encode_source(picture, self.canvas().map(Canvas::current_frame)) else {
            return;
        };
        let orientation = self.canvas().map(Canvas::orientation).unwrap_or_default();
        if self.copies.is_none() {
            self.copies = Inbox::new()
                .inspect_err(|error| tracing::warn!("cannot copy: {error}"))
                .ok();
        }
        let Some(outbox) = self.copies.as_ref().map(Inbox::outbox) else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("copy".to_owned())
            .spawn(move || {
                outbox.send(encode_png(source, orientation));
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot copy: {error}");
        }
    }

    fn serve_copies(&mut self, window: &mut Window) -> bool {
        let Some(copies) = self.copies.as_mut() else {
            return false;
        };
        let uris = self
            .paths
            .get(self.current)
            .map(|path| vec![bande_xdg::file_uri(path)])
            .unwrap_or_default();
        for png in copies.drain().into_iter().flatten() {
            window.copy_transfer(Transfer::uris(uris.clone()).with_png(png));
        }
        false
    }

    fn settle(&mut self) {
        loop {
            let signals = std::mem::take(&mut *self.signals.borrow_mut());
            if signals.is_empty() {
                break;
            }
            for signal in signals {
                match signal {
                    Signal::Sidebar => self.toggle_sidebar(),
                    Signal::ZoomIn => self.zoom_by(1.0),
                    Signal::ZoomOut => self.zoom_by(-1.0),
                    Signal::RotateLeft => self.rotate(false),
                    Signal::Info => self.requests.push(Request::ShowInfo),
                }
            }
        }
        let chosen = self
            .ui
            .widget_as::<Sidebar>(self.sidebar)
            .and_then(Sidebar::take_chosen);
        if let Some(index) = chosen {
            self.select(index);
        }
        let has_picture = self.has_picture();
        for id in self.toolbar.tools.iter().skip(1) {
            if let Some(button) = self.ui.widget_as::<Button>(*id) {
                button.set_enabled(has_picture);
            }
        }
        let several = self.paths.len() > 1;
        if let Some(button) = self.ui.widget_as::<Button>(self.toolbar.sidebar) {
            button.set_enabled(several);
        }
    }

    fn sidebar_visible(&self) -> bool {
        self.sidebar_shown && self.paths.len() > 1 && self.slideshow.is_none()
    }

    fn sheet(&self, width: f32, height: f32) -> Rect {
        let inset = if self.full_screen || self.slideshow.is_some() {
            0.0
        } else {
            SHEET_INSET as f32
        };
        let left = if self.sidebar_visible() {
            SIDEBAR_WIDTH
        } else {
            inset
        };
        Rect::new(
            left,
            TOOLBAR_HEIGHT,
            (width - left - inset).max(0.0),
            (height - TOOLBAR_HEIGHT - inset).max(0.0),
        )
    }

    fn tool_frames(&self, width: f32) -> Vec<(WidgetId, Rect)> {
        let middle = ((TOOLBAR_HEIGHT - SYMBOL_BUTTON) / 2.0).round();
        let mut frames = vec![(
            self.toolbar.sidebar,
            Rect::new(
                window_controls_end(true) + SIDE,
                middle,
                SYMBOL_BUTTON,
                SYMBOL_BUTTON,
            ),
        )];
        let count = self.toolbar.tools.len() as f32;
        let mut x = width - SIDE - count * SYMBOL_BUTTON - (count - 1.0) * GAP;
        for id in self.toolbar.tools {
            frames.push((id, Rect::new(x, middle, SYMBOL_BUTTON, SYMBOL_BUTTON)));
            x += SYMBOL_BUTTON + GAP;
        }
        frames
    }

    pub fn header(&self, width: f32) -> Header {
        let frames = self.tool_frames(width);
        let title_x = match self.sidebar_visible() {
            true => SIDEBAR_WIDTH + TITLE_GAP,
            false => window_controls_end(true) + SIDE + SYMBOL_BUTTON + TITLE_GAP,
        };
        Header {
            title_x,
            items: frames.iter().map(|(_, frame)| *frame).collect(),
            tabs: Vec::new(),
        }
    }

    pub fn paint(&mut self, width: u32, height: u32) -> Scene {
        let (w, h) = (width as f32, height as f32);
        let sheet = self.sheet(w, h);
        let sidebar_frame = match self.sidebar_visible() {
            true => Rect::new(
                0.0,
                TOOLBAR_HEIGHT,
                SIDEBAR_WIDTH,
                (h - TOOLBAR_HEIGHT).max(0.0),
            ),
            false => Rect::default(),
        };
        let frames = self.tool_frames(w);
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            for (id, frame) in frames {
                host.set_frame(id, frame);
            }
            host.set_frame(self.sidebar, sidebar_frame);
            host.set_frame(self.canvas, sheet);
        }
        let palette = self.ui.context().palette;
        let radius = match sheet.x > 0.0 && !self.full_screen && self.slideshow.is_none() {
            true => Radius::concentric(Radius::XL, SHEET_INSET as f32),
            false => 0.0,
        };
        self.with_canvas(|canvas| canvas.set_radius(radius));
        let (mut items, dialog) = self.ui.paint_layers(width, height);
        items.extend(dialog);
        Scene {
            width,
            height,
            background: palette.chassis,
            items,
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let handled = self.ui.handle(event);
        let unhandled = !handled && self.key(event);
        self.settle();
        handled || unhandled
    }

    fn key(&mut self, event: &Event) -> bool {
        let Event::Key { key, modifiers } = event else {
            return false;
        };
        if modifiers.control || modifiers.alt || modifiers.logo {
            return false;
        }
        match key {
            Key::Right | Key::PageDown => self.go(true),
            Key::Left | Key::PageUp => self.go(false),
            Key::Space if self.is_animated() => self.toggle_playing(),
            Key::Escape if self.slideshow.is_some() => self.toggle_slideshow(),
            _ => return false,
        }
        true
    }

    pub fn pump(&mut self) -> bool {
        let loaded = self.take_loaded();
        let thumbs = self.take_thumbnails();
        self.settle();
        loaded || thumbs
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.hidden {
            return self.ui.deadline();
        }
        let animation = self.canvas().and_then(Canvas::deadline);
        [self.ui.deadline(), animation, self.slideshow]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = self.with_canvas(|canvas| canvas.tick(now)).unwrap_or(false);
        if self.slideshow.is_some_and(|next| next <= now) && !self.loading {
            self.slideshow = Some(now + SLIDE);
            self.go(true);
            changed = true;
        }
        changed
    }

    fn set_hidden(&mut self, hidden: bool) {
        if self.hidden == hidden {
            return;
        }
        self.hidden = hidden;
        if !hidden {
            let now = Instant::now();
            self.with_canvas(|canvas| canvas.resume(now));
            if self.slideshow.is_some() {
                self.slideshow = Some(now + SLIDE);
            }
        }
    }
}

fn encode_source(
    picture: &Decoded,
    frame: Option<usize>,
) -> Option<(u32, u32, std::sync::Arc<Vec<u8>>)> {
    let rgba = match &picture.pixels {
        crate::decode::Pixels::Still(levels) => levels.first().map(|level| level.rgba.clone()),
        crate::decode::Pixels::Animation(frames) => frames
            .get(frame.unwrap_or(0))
            .map(|frame| frame.rgba.clone()),
    }?;
    Some((picture.width, picture.height, rgba))
}

impl View for PreviewView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.full_screen = window.header_height() == 0;
        window.set_frame_regions(FrameRegions {
            sidebar: if self.sidebar_visible() {
                SIDEBAR_WIDTH
            } else {
                0.0
            },
            toolbar: if self.full_screen {
                TOOLBAR_HEIGHT
            } else {
                0.0
            },
        });
        window.set_title(&self.title());
        let (width, height) = window.size();
        window.set_header(self.header(width as f32));
        self.paint(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::ToplevelChanged(state) = &event {
            self.set_hidden(state.suspended);
        }
        let handled = match window.drive(&mut self.ui, event) {
            Ok(Some(WindowEvent::Input(input))) => self.key(&input),
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(error) => {
                tracing::warn!("{error}");
                false
            }
        };
        self.settle();
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut fds = Vec::new();
        if let Some(loader) = &self.loader {
            fds.push(loader.fd());
        }
        if let Some(thumbnails) = &self.thumbnails {
            fds.push(thumbnails.fd());
        }
        if let Some(copies) = &self.copies {
            fds.push(copies.fd());
        }
        fds
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) -> bool {
        self.serve_copies(window);
        self.pump()
    }

    fn deadline(&self) -> Option<Instant> {
        PreviewView::deadline(self)
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        PreviewView::tick(self, now);
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<bande_toolkit::Rect> {
        self.ui.text_input_area()
    }
}
