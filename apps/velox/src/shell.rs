use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use bande_l10n::tr;

use crate::engine::Engine;
use accesskit::TreeUpdate;
use bande_app::{Keys, View};
use bande_terminal_view::{GridSize, TerminalView};
use bande_toolkit::ui::{Context, Ui, Widget, WidgetId};
use bande_toolkit::{Appearance, Rect, Scene, TextSystem, Window, WindowEvent};

const READ_CHUNK: usize = 65536;
const SHELLS: [&str; 3] = ["/bin/bash", "/bin/ash", "/bin/sh"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellTarget {
    Serial {
        machine: String,
        name: String,
        socket: PathBuf,
    },
    Exec {
        engine: Engine,
        container: String,
        name: String,
    },
}

impl ShellTarget {
    pub fn key(&self) -> &str {
        match self {
            Self::Serial { machine, .. } => machine,
            Self::Exec { container, .. } => container,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Serial { name, .. } => {
                tr!("{name} — Serial Console", name = name.clone())
            }
            Self::Exec { name, .. } => {
                tr!("{name} — Shell", name = name.clone())
            }
        }
    }

    fn connect(&self, execution: &Mutex<Option<String>>) -> Result<UnixStream, String> {
        match self {
            Self::Serial { socket, .. } => {
                UnixStream::connect(socket).map_err(|error| error.to_string())
            }
            Self::Exec {
                engine, container, ..
            } => {
                let id = engine
                    .create_exec(container, &SHELLS)
                    .map_err(|error| error.to_string())?;
                let stream = engine.start_exec(&id).map_err(|error| error.to_string())?;
                *execution.lock().unwrap_or_else(PoisonError::into_inner) = Some(id);
                Ok(stream)
            }
        }
    }
}

type Connected = Result<UnixStream, String>;

pub struct ShellWindow {
    target: ShellTarget,
    ui: Ui,
    view: WidgetId,
    stream: Option<UnixStream>,
    connected: Receiver<Connected>,
    wake: UnixStream,
    writer: Rc<RefCell<Option<UnixStream>>>,
    size: Arc<Mutex<Option<GridSize>>>,
    execution: Arc<Mutex<Option<String>>>,
    finished: bool,
}

fn send_size(target: &ShellTarget, execution: &Mutex<Option<String>>, size: GridSize) {
    let ShellTarget::Exec { engine, .. } = target else {
        return;
    };
    let id = execution
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(id) = id {
        let engine = engine.clone();
        let spawned = std::thread::Builder::new()
            .name("velox-shell-resize".to_owned())
            .spawn(move || {
                if let Err(error) = engine.resize_exec(&id, size.columns, size.lines) {
                    tracing::debug!("cannot resize the shell: {error}");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot resize the shell: {error}");
        }
    }
}

pub struct ShellLink {
    target: ShellTarget,
    wake: UnixStream,
    connected: Receiver<Connected>,
    execution: Arc<Mutex<Option<String>>>,
}

impl ShellLink {
    pub fn connect(target: ShellTarget) -> std::io::Result<Self> {
        let (wake, ring) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        let (sender, connected) = mpsc::channel();
        let execution: Arc<Mutex<Option<String>>> = Arc::default();
        let thread_target = target.clone();
        let thread_execution = Arc::clone(&execution);
        std::thread::Builder::new()
            .name("velox-shell".to_owned())
            .spawn(move || {
                let connected = thread_target.connect(&thread_execution);
                if sender.send(connected).is_ok() {
                    let mut ring = ring;
                    if let Err(error) = ring.write_all(&[1]) {
                        tracing::debug!("cannot wake the shell window: {error}");
                    }
                }
            })?;
        Ok(Self {
            target,
            wake,
            connected,
            execution,
        })
    }
}

impl ShellWindow {
    pub fn new(link: ShellLink, appearance: Appearance) -> Self {
        let ShellLink {
            target,
            wake,
            connected,
            execution,
        } = link;
        let writer: Rc<RefCell<Option<UnixStream>>> = Rc::default();
        let input = Rc::clone(&writer);
        let size: Arc<Mutex<Option<GridSize>>> = Arc::default();
        let resize_target = target.clone();
        let resize_execution = Arc::clone(&execution);
        let wanted_size = Arc::clone(&size);
        let view = TerminalView::new(
            &target.title(),
            appearance,
            move |bytes| {
                if let Some(stream) = input.borrow_mut().as_mut()
                    && let Err(error) = stream.write_all(bytes)
                {
                    tracing::debug!("cannot write to the shell: {error}");
                }
            },
            move |grid| {
                *wanted_size.lock().unwrap_or_else(PoisonError::into_inner) = Some(grid);
                send_size(&resize_target, &resize_execution, grid);
            },
        );
        let view_id = view.id();
        let mut ui = Ui::new(
            Box::new(view),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui.set_focus(view_id);
        Self {
            target,
            ui,
            view: view_id,
            stream: None,
            connected,
            wake,
            writer,
            size,
            execution,
            finished: false,
        }
    }

    pub fn target(&self) -> &ShellTarget {
        &self.target
    }

    fn feed(&mut self, bytes: &[u8]) {
        if let Some(view) = self.ui.widget_as::<TerminalView>(self.view) {
            view.feed(bytes);
        }
    }

    fn take_connection(&mut self) {
        let mut byte = [0; 1];
        if !matches!((&self.wake).read(&mut byte), Ok(read) if read > 0) {
            return;
        }
        match self.connected.try_recv() {
            Ok(Ok(stream)) => {
                let (reader, writer) = (stream.try_clone(), stream);
                match reader.and_then(|reader| reader.set_nonblocking(true).map(|()| reader)) {
                    Ok(reader) => {
                        self.stream = Some(reader);
                        *self.writer.borrow_mut() = Some(writer);
                        let grid = *self.size.lock().unwrap_or_else(PoisonError::into_inner);
                        if let Some(grid) = grid {
                            send_size(&self.target, &self.execution, grid);
                        }
                    }
                    Err(error) => self.closed(Some(error.to_string())),
                }
            }
            Ok(Err(reason)) => self.closed(Some(reason)),
            Err(_) => {}
        }
    }

    fn closed(&mut self, reason: Option<String>) {
        self.stream = None;
        *self.writer.borrow_mut() = None;
        self.finished = true;
        let text = match reason {
            Some(reason) => tr!("Cannot connect: {reason}", reason = reason),
            None => tr!("The connection closed."),
        };
        self.feed(format!("\r\n{text}\r\n").as_bytes());
    }

    fn read_stream(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let mut bytes = vec![0; READ_CHUNK];
        match stream.read(&mut bytes) {
            Ok(0) => self.closed(None),
            Ok(read) => self.feed(&bytes[..read]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => self.closed(Some(error.to_string())),
        }
    }
}

impl View for ShellWindow {
    fn keys(&self) -> Option<Keys> {
        Some(Keys::Shifted)
    }

    fn scene(&mut self, window: &mut Window) -> Scene {
        window.set_title(&self.target.title());
        View::scene(&mut self.ui, window)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = View::event(&mut self.ui, window, event);
        if let Some(request) = self.ui.take_clipboard_request()
            && let Err(error) = window.serve_clipboard(request)
        {
            tracing::warn!("cannot use the clipboard: {error}");
        }
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
        if let Some(view) = self.ui.widget_as::<TerminalView>(self.view) {
            view.set_appearance(appearance);
        }
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        match &self.stream {
            Some(stream) => vec![stream.as_fd()],
            None if self.finished => Vec::new(),
            None => vec![self.wake.as_fd()],
        }
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        match self.stream.is_some() {
            true => self.read_stream(),
            false => self.take_connection(),
        }
        true
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
