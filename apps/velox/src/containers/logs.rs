use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use accesskit::{Node, Role, TreeUpdate};
use bande_app::View;
use bande_toolkit::ui::{
    Button, CheckState, Checkbox, Context, SearchField, Size, TEXT_FIELD_HEIGHT, Ui, Widget,
    WidgetId,
};
use bande_toolkit::{
    Appearance, FrameRegions, Header, Item, Rect, Scene, Shape, TextRole, TextSystem,
    WINDOW_HEADER_HEIGHT, Window, WindowEvent, window_controls_end,
};
use serde::Deserialize;

use super::ansi::{self, Pen};
use super::compose::Compose;
use super::log_text::LogText;
use super::widgets::Text;
use crate::L10N;
use crate::engine::{Container, Engine};

const TOOLBAR: f32 = WINDOW_HEADER_HEIGHT as f32;
const FOOTER: f32 = 26.0;
const INSET: f32 = 12.0;
const GAP: f32 = 10.0;
const SEARCH_WIDTH: f32 = 180.0;
const TAIL: u32 = 1000;
const TITLE_X: f32 = window_controls_end(true) + 16.0;
const HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogsTarget {
    Container {
        engine: Engine,
        id: String,
        name: String,
    },
    Project(Compose),
}

impl LogsTarget {
    pub fn key(&self) -> &str {
        match self {
            Self::Container { id, .. } => id,
            Self::Project(compose) => &compose.project,
        }
    }

    pub fn title(&self) -> String {
        let name = match self {
            Self::Container { name, .. } => name,
            Self::Project(compose) => &compose.project,
        };
        L10N.tr_args("logs-title", [("name", name.as_str().into())])
    }
}

enum Chunk {
    Bytes(Vec<u8>),
    Ended,
}

#[derive(Clone)]
struct Out {
    sender: Sender<Chunk>,
    wake: Arc<UnixStream>,
}

impl Out {
    fn send(&self, chunk: Chunk) -> bool {
        self.sender.send(chunk).is_ok() && (&*self.wake).write_all(&[1]).is_ok()
    }
}

#[derive(Default)]
struct Held {
    stopped: bool,
    socket: Option<UnixStream>,
    child: Option<Child>,
}

type Handle = Arc<Mutex<Held>>;

fn held(handle: &Handle) -> std::sync::MutexGuard<'_, Held> {
    handle.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Follow {
    chunks: Receiver<Chunk>,
    wake: UnixStream,
    handle: Handle,
}

impl Drop for Follow {
    fn drop(&mut self) {
        let mut held = held(&self.handle);
        held.stopped = true;
        if let Some(socket) = held.socket.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        if let Some(child) = held.child.as_mut() {
            let _ = child.kill();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Probe {
    config: ProbeConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ProbeConfig {
    #[serde(default)]
    tty: bool,
}

fn pump(mut reader: impl Read, out: &Out) -> io::Result<()> {
    let mut buffer = vec![0; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 || !out.send(Chunk::Bytes(buffer[..read].to_vec())) {
            return Ok(());
        }
    }
}

fn demux(mut reader: impl Read, out: &Out) -> io::Result<()> {
    let mut header = [0; HEADER_SIZE];
    loop {
        match reader.read_exact(&mut header) {
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            result => result?,
        }
        let size = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        let mut payload = vec![0; size as usize];
        reader.read_exact(&mut payload)?;
        if !out.send(Chunk::Bytes(payload)) {
            return Ok(());
        }
    }
}

fn since_query(since: Option<SystemTime>) -> String {
    match since.and_then(|since| since.duration_since(UNIX_EPOCH).ok()) {
        Some(at) => format!("&since={}.{:09}", at.as_secs(), at.subsec_nanos()),
        None => format!("&tail={TAIL}"),
    }
}

fn follow_container(
    engine: &Engine,
    id: &str,
    since: Option<SystemTime>,
    handle: &Handle,
    out: &Out,
) -> Result<(), crate::engine::Error> {
    let probe: Probe = engine.get(&format!("/containers/{id}/json"))?;
    let (socket, body) = engine.stream(&format!(
        "/containers/{id}/logs?follow=1&stdout=1&stderr=1{}",
        since_query(since)
    ))?;
    {
        let mut held = held(handle);
        if held.stopped {
            return Ok(());
        }
        held.socket = Some(socket);
    }
    match probe.config.tty {
        true => pump(body, out)?,
        false => demux(body, out)?,
    }
    Ok(())
}

fn follow_project(
    compose: &Compose,
    since: Option<SystemTime>,
    handle: &Handle,
    out: &Out,
) -> io::Result<()> {
    let window = match since.and_then(|since| since.elapsed().ok()) {
        Some(elapsed) => vec!["--since".to_owned(), format!("{}s", elapsed.as_secs() + 1)],
        None => vec!["--tail".to_owned(), TAIL.to_string()],
    };
    let mut arguments = compose.colours();
    arguments.extend(["logs".to_owned(), "--follow".to_owned()]);
    arguments.extend(window);
    let (reader, writer) = io::pipe()?;
    let mut command: Command = compose.command(&arguments);
    command
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    {
        let mut held = held(handle);
        if held.stopped {
            return Ok(());
        }
        held.child = Some(command.spawn()?);
    }
    drop(command);
    let pumped = pump(reader, out);
    if let Some(mut child) = held(handle).child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    pumped
}

impl Follow {
    fn start(target: &LogsTarget, since: Option<SystemTime>) -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        let (sender, chunks) = mpsc::channel();
        let out = Out {
            sender,
            wake: Arc::new(writer),
        };
        let handle = Handle::default();
        let shared = Arc::clone(&handle);
        let target = target.clone();
        std::thread::Builder::new()
            .name("velox-logs".to_owned())
            .spawn(move || {
                let result = match &target {
                    LogsTarget::Container { engine, id, .. } => {
                        follow_container(engine, id, since, &shared, &out)
                            .map_err(|error| error.to_string())
                    }
                    LogsTarget::Project(compose) => follow_project(compose, since, &shared, &out)
                        .map_err(|error| error.to_string()),
                };
                if let Err(error) = result
                    && !held(&shared).stopped
                {
                    tracing::info!("the logs ended: {error}");
                }
                out.send(Chunk::Ended);
            })?;
        Ok(Self {
            chunks,
            wake: reader,
            handle,
        })
    }

    fn drain(&self) -> Vec<Chunk> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.chunks.try_iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Filter(String),
    Follow(bool),
    Wrap(bool),
    Copy,
    Clear,
}

type Intents = Rc<RefCell<Vec<Intent>>>;

#[derive(Clone, Copy)]
struct Parts {
    text: WidgetId,
    footer: WidgetId,
}

pub struct LogsView {
    ui: Ui,
    root: WidgetId,
    parts: Parts,
    target: LogsTarget,
    follow: Option<Follow>,
    following: bool,
    detached: Option<SystemTime>,
    hidden: bool,
    pen: Pen,
    partial: Vec<u8>,
    intents: Intents,
}

impl LogsView {
    pub fn new(appearance: Appearance, target: LogsTarget) -> Self {
        let intents = Intents::default();
        let push = |intent: fn(bool) -> Intent| {
            let intents = Rc::clone(&intents);
            move |on| intents.borrow_mut().push(intent(on))
        };
        let filter = Rc::clone(&intents);
        let copy = Rc::clone(&intents);
        let clear = Rc::clone(&intents);
        let text = LogText::new(&target.title(), true, true);
        let footer = Text::new("", TextRole::Callout).secondary();
        let parts = Parts {
            text: text.id(),
            footer: footer.id(),
        };
        let tools: Vec<Box<dyn Widget>> = vec![
            Box::new(SearchField::new(&L10N.tr("logs-filter"), "", move |text| {
                filter.borrow_mut().push(Intent::Filter(text.to_owned()));
            })),
            Box::new(Checkbox::new(
                &L10N.tr("logs-follow"),
                CheckState::On,
                push(Intent::Follow),
            )),
            Box::new(Checkbox::new(
                &L10N.tr("logs-wrap"),
                CheckState::On,
                push(Intent::Wrap),
            )),
            Box::new(Button::new(&L10N.tr("logs-copy"), move || {
                copy.borrow_mut().push(Intent::Copy);
            })),
            Box::new(Button::new(&L10N.tr("logs-clear"), move || {
                clear.borrow_mut().push(Intent::Clear);
            })),
        ];
        let root = LogsRoot::new(Box::new(text), Box::new(footer), tools);
        let root_id = root.id();
        let mut ui = Ui::new(
            Box::new(root),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        let mut view = Self {
            ui,
            root: root_id,
            parts,
            target,
            follow: None,
            following: false,
            detached: None,
            hidden: false,
            pen: Pen::default(),
            partial: Vec::new(),
            intents,
        };
        view.attach(None);
        view.update_footer();
        view
    }

    pub fn target(&self) -> &LogsTarget {
        &self.target
    }

    fn attach(&mut self, since: Option<SystemTime>) {
        self.follow = None;
        match Follow::start(&self.target, since) {
            Ok(follow) => {
                self.follow = Some(follow);
                self.following = true;
                self.detached = None;
            }
            Err(error) => tracing::warn!("cannot follow the logs: {error}"),
        }
    }

    fn detach(&mut self) {
        if self.follow.take().is_some() && self.following {
            self.detached = Some(SystemTime::now());
        }
        self.following = false;
    }

    pub fn container_changed(&mut self, container: &Container) -> bool {
        let watched = match &self.target {
            LogsTarget::Container { id, .. } => *id == container.id,
            LogsTarget::Project(compose) => container.project() == Some(compose.project.as_str()),
        };
        if !watched || !container.running() || self.following || self.hidden {
            return false;
        }
        let since = self.detached.unwrap_or_else(SystemTime::now);
        self.attach(Some(since));
        true
    }

    fn set_hidden(&mut self, hidden: bool) {
        if hidden == self.hidden {
            return;
        }
        self.hidden = hidden;
        match hidden {
            true => self.detach(),
            false => {
                if let Some(since) = self.detached {
                    self.attach(Some(since));
                }
            }
        }
    }

    fn text(&mut self) -> Option<&mut LogText> {
        self.ui.widget_as::<LogText>(self.parts.text)
    }

    fn update_footer(&mut self) {
        let (shown, total) = self
            .ui
            .widget_ref::<LogText>(self.parts.text)
            .map(LogText::counts)
            .unwrap_or_default();
        let line = L10N.tr_args(
            "logs-count",
            [("shown", shown.into()), ("total", total.into())],
        );
        if let Some(footer) = self.ui.widget_as::<Text>(self.parts.footer) {
            footer.set_text(&line);
        }
    }

    fn take(&mut self, bytes: &[u8]) {
        self.partial.extend_from_slice(bytes);
        let Some(end) = self.partial.iter().rposition(|byte| *byte == b'\n') else {
            return;
        };
        let complete: Vec<u8> = self.partial.drain(..=end).collect();
        let text = String::from_utf8_lossy(&complete[..end]).into_owned();
        let pen = &mut self.pen;
        let lines: Vec<_> = text
            .split('\n')
            .map(|line| ansi::parse(line, pen))
            .collect();
        if let Some(widget) = self.text() {
            widget.push(lines);
        }
    }

    fn pump(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        for intent in &intents {
            match intent {
                Intent::Filter(filter) => {
                    if let Some(text) = self.text() {
                        text.set_filter(filter);
                    }
                }
                Intent::Follow(on) => {
                    if let Some(text) = self.text() {
                        text.set_follow(*on);
                    }
                }
                Intent::Wrap(on) => {
                    if let Some(text) = self.text() {
                        text.set_wrap(*on);
                    }
                }
                Intent::Copy => {
                    let shown = self.text().map(|text| text.shown_text());
                    if let Some(shown) = shown {
                        self.ui.context().copy(&shown);
                    }
                }
                Intent::Clear => {
                    if let Some(text) = self.text() {
                        text.clear();
                    }
                }
            }
        }
        if !intents.is_empty() {
            self.update_footer();
        }
        !intents.is_empty()
    }

    fn header(&self) -> Header {
        Header {
            title_x: TITLE_X,
            items: self
                .ui
                .widget_ref::<LogsRoot>(self.root)
                .map(LogsRoot::tool_frames)
                .unwrap_or_default(),
            tabs: Vec::new(),
        }
    }

    fn drain(&mut self) -> bool {
        let chunks = self.follow.as_ref().map(Follow::drain).unwrap_or_default();
        for chunk in chunks {
            match chunk {
                Chunk::Bytes(bytes) => self.take(&bytes),
                Chunk::Ended => {
                    self.follow = None;
                    self.following = false;
                    self.detached = Some(SystemTime::now());
                    if !self.partial.is_empty() {
                        self.partial.push(b'\n');
                        self.take(&[]);
                    }
                }
            }
        }
        self.update_footer();
        true
    }
}

impl View for LogsView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        window.set_frame_regions(FrameRegions {
            sidebar: 0.0,
            toolbar: match full_screen {
                true => TOOLBAR,
                false => 0.0,
            },
        });
        let scene = View::scene(&mut self.ui, window);
        window.set_header(self.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::ToplevelChanged(state) = &event {
            self.set_hidden(state.suspended);
        }
        let handled = View::event(&mut self.ui, window, event);
        let pumped = self.pump();
        if let Some(request) = self.ui.take_clipboard_request()
            && let Err(error) = window.serve_clipboard(request)
        {
            tracing::warn!("cannot copy: {error}");
        }
        handled || pumped
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.follow
            .iter()
            .map(|follow| follow.wake.as_fd())
            .collect()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        self.drain()
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}

struct LogsRoot {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl LogsRoot {
    const FOOTER: usize = 1;
    const TOOLS: usize = 2;

    fn new(text: Box<dyn Widget>, footer: Box<dyn Widget>, tools: Vec<Box<dyn Widget>>) -> Self {
        let mut children = vec![text, footer];
        children.extend(tools);
        Self {
            id: WidgetId::next(),
            children,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn tool_frames(&self) -> Vec<Rect> {
        let mut right = self.frame.width - INSET;
        let mut frames: Vec<Rect> = self.sizes[Self::TOOLS.min(self.sizes.len())..]
            .iter()
            .rev()
            .map(|size| {
                right -= size.width;
                let frame = Rect::new(
                    right,
                    ((TOOLBAR - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                );
                right -= GAP;
                frame
            })
            .collect();
        frames.reverse();
        frames
    }
}

impl Widget for LogsRoot {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.sizes = self
            .children
            .iter_mut()
            .enumerate()
            .map(|(index, child)| match index {
                Self::TOOLS => {
                    let size = child.measure(cx, Size::new(SEARCH_WIDTH, TEXT_FIELD_HEIGHT));
                    Size::new(SEARCH_WIDTH, size.height.max(TEXT_FIELD_HEIGHT))
                }
                _ => child.measure(cx, available),
            })
            .collect();
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let body = (frame.height - TOOLBAR - FOOTER).max(0.0);
        let footer = self.sizes.get(Self::FOOTER).copied().unwrap_or_default();
        let mut frames = vec![
            Rect::new(0.0, TOOLBAR, frame.width, body),
            Rect::new(
                INSET,
                TOOLBAR + body + ((FOOTER - footer.height) / 2.0).round(),
                (frame.width - 2.0 * INSET).max(0.0),
                footer.height,
            ),
        ];
        frames.extend(self.tool_frames());
        for (child, frame) in self.children.iter_mut().zip(frames) {
            if child.frame() != frame {
                child.measure(cx, Size::new(frame.width, frame.height));
            }
            child.place(cx, frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (y, height) in [(TOOLBAR, 1.0), (self.frame.height - FOOTER, 1.0)] {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(0.0, y, self.frame.width, height)),
                paint: cx.palette.hairline.into(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Window)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
