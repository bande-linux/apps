use std::fs::File;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use rustix::fs::{MemfdFlags, memfd_create};
use serde_json::json;
use zbus::blocking::connection::Builder;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Fd;
use zbus::{fdo, interface};

use crate::Result;
use crate::keycodes::qemu_keycode;
use crate::qmp::Qmp;

const QEMU: &str = "org.qemu";
const CONSOLE_PATH: &str = "/org/qemu/Display1/Console_0";
const CLIPBOARD_PATH: &str = "/org/qemu/Display1/Clipboard";
const LISTENER_PATH: &str = "/org/qemu/Display1/Listener";
const CONSOLE: &str = "org.qemu.Display1.Console";
const KEYBOARD: &str = "org.qemu.Display1.Keyboard";
const MOUSE: &str = "org.qemu.Display1.Mouse";
const CLIPBOARD: &str = "org.qemu.Display1.Clipboard";
const LISTENER_MAP: &str = "org.qemu.Display1.Listener.Unix.Map";
const LISTENER_DMABUF: &str = "org.qemu.Display1.Listener.Unix.ScanoutDMABUF2";
const TEXT: &str = "text/plain;charset=utf-8";
const CLIPBOARD_SELECTION: u32 = 0;
const FD_NAME: &str = "velox-display";
const MM_PER_INCH: f64 = 25.4;
const BASE_DPI: f64 = 96.0;
const PIXMAN_X8R8G8B8: u32 = 0x2002_0888;
const PIXMAN_A8R8G8B8: u32 = 0x2002_8888;
const PIXMAN_X8B8G8R8: u32 = 0x2003_0888;
const PIXMAN_A8B8G8R8: u32 = 0x2003_8888;
const BUTTON_WHEEL_UP: u32 = 3;
const BUTTON_WHEEL_DOWN: u32 = 4;
const BUTTON_WHEEL_LEFT: u32 = 7;
const BUTTON_WHEEL_RIGHT: u32 = 8;
const KEY_LEFT_CTRL: u32 = 29;
const KEY_LEFT_ALT: u32 = 56;
const KEY_DELETE: u32 = 111;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    Side = 5,
    Extra = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheel {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug)]
pub struct Scanout {
    pub memory: OwnedFd,
    pub offset: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub fourcc: u32,
}

#[derive(Debug)]
pub struct GuestDmabuf {
    pub planes: Vec<(OwnedFd, u32, u32)>,
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub modifier: u64,
    pub y0_top: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damage {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug)]
pub enum DisplayEvent {
    Scanout(Scanout),
    Dmabuf(GuestDmabuf),
    Damage(Damage),
    Disabled,
    Cursor {
        width: u32,
        height: u32,
        hotspot: (u32, u32),
        pixels: Vec<u8>,
    },
    CursorShown(bool),
    GuestCopied(String),
    Closed(Option<String>),
}

enum Command {
    Close,
    Key(u32, bool),
    Pointer(u32, u32),
    Button(u32, bool),
    Resize { width: u32, height: u32, scale: f64 },
    Offer,
    GuestGrabbed,
}

type Sink = Arc<dyn Fn(DisplayEvent) + Send + Sync>;

#[derive(Default)]
struct ClipboardState {
    serial: AtomicU32,
    offered: Mutex<Option<String>>,
}

pub struct Display {
    input: DisplayInput,
    clipboard: Arc<ClipboardState>,
}

#[derive(Clone)]
pub struct DisplayInput {
    commands: Sender<Command>,
}

impl Display {
    pub fn open(control: PathBuf, sink: impl Fn(DisplayEvent) + Send + Sync + 'static) -> Self {
        let (commands, received) = mpsc::channel();
        let clipboard = Arc::new(ClipboardState::default());
        let sink: Sink = Arc::new(sink);
        let session = Session {
            control,
            sink: Arc::clone(&sink),
            clipboard: Arc::clone(&clipboard),
            commands: commands.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("velox-display".to_owned())
            .spawn(move || session.run(&received));
        if let Err(error) = spawned {
            sink(DisplayEvent::Closed(Some(error.to_string())));
        }
        Self {
            input: DisplayInput { commands },
            clipboard,
        }
    }

    pub fn input(&self) -> DisplayInput {
        self.input.clone()
    }

    pub fn resize(&self, width: u32, height: u32, scale: f64) {
        self.input.send(Command::Resize {
            width,
            height,
            scale,
        });
    }

    pub fn offer_clipboard(&self, text: String) {
        *self
            .clipboard
            .offered
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(text);
        self.input.send(Command::Offer);
    }

    pub fn send_ctrl_alt_delete(&self) {
        let chord = [KEY_LEFT_CTRL, KEY_LEFT_ALT, KEY_DELETE];
        for key in chord {
            self.input.key(key, true);
        }
        for key in chord.into_iter().rev() {
            self.input.key(key, false);
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        self.input.send(Command::Close);
    }
}

impl DisplayInput {
    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            tracing::debug!("the guest display has closed");
        }
    }

    pub fn key(&self, linux: u32, pressed: bool) {
        if let Some(code) = qemu_keycode(linux) {
            self.send(Command::Key(code, pressed));
        }
    }

    pub fn pointer(&self, x: u32, y: u32) {
        self.send(Command::Pointer(x, y));
    }

    pub fn button(&self, button: MouseButton, pressed: bool) {
        self.send(Command::Button(button as u32, pressed));
    }

    pub fn wheel(&self, wheel: Wheel) {
        let button = match wheel {
            Wheel::Up => BUTTON_WHEEL_UP,
            Wheel::Down => BUTTON_WHEEL_DOWN,
            Wheel::Left => BUTTON_WHEEL_LEFT,
            Wheel::Right => BUTTON_WHEEL_RIGHT,
        };
        self.send(Command::Button(button, true));
        self.send(Command::Button(button, false));
    }
}

struct Session {
    control: PathBuf,
    sink: Sink,
    clipboard: Arc<ClipboardState>,
    commands: Sender<Command>,
}

impl Session {
    fn run(self, commands: &Receiver<Command>) {
        let reason = match self.serve(commands) {
            Ok(()) => None,
            Err(error) => Some(error.to_string()),
        };
        (self.sink)(DisplayEvent::Closed(reason));
    }

    fn serve(&self, commands: &Receiver<Command>) -> Result<()> {
        let mut qmp = Qmp::connect(&self.control)?;
        let (ours, theirs) = UnixStream::pair()?;
        qmp.execute_with_fd("getfd", json!({"fdname": FD_NAME}), theirs.as_fd())?;
        qmp.execute(
            "add_client",
            json!({"protocol": "@dbus-display", "fdname": FD_NAME}),
        )?;
        drop(qmp);
        drop(theirs);
        let display_socket = ours.try_clone()?;
        let connection = Builder::async_io_unix_stream(ours)
            .p2p()
            .serve_at(
                CLIPBOARD_PATH,
                GuestClipboard {
                    clipboard: Arc::clone(&self.clipboard),
                    commands: self.commands.clone(),
                },
            )?
            .build()?;
        let (ours, theirs) = UnixStream::pair()?;
        let listener_socket = ours.try_clone()?;
        connection.call_method(
            Some(QEMU),
            CONSOLE_PATH,
            Some(CONSOLE),
            "RegisterListener",
            &Fd::from(theirs.as_fd()),
        )?;
        drop(theirs);
        let listener = Builder::async_io_unix_stream(ours)
            .p2p()
            .serve_at(
                LISTENER_PATH,
                Listener {
                    sink: Arc::clone(&self.sink),
                    bytes: None,
                },
            )?
            .serve_at(
                LISTENER_PATH,
                ListenerMap {
                    sink: Arc::clone(&self.sink),
                },
            )?
            .serve_at(
                LISTENER_PATH,
                ListenerDmabuf {
                    sink: Arc::clone(&self.sink),
                },
            )?
            .build()?;
        if let Err(error) =
            connection.call_method(Some(QEMU), CLIPBOARD_PATH, Some(CLIPBOARD), "Register", &())
        {
            tracing::warn!("the guest's clipboard is not shared: {error}");
        }
        let watched = connection.clone();
        let sink = Arc::clone(&self.sink);
        std::thread::Builder::new()
            .name("velox-display-watch".to_owned())
            .spawn(move || {
                for message in MessageIterator::from(&watched) {
                    if message.is_err() {
                        break;
                    }
                }
                sink(DisplayEvent::Closed(None));
            })?;
        self.forward(&connection, commands);
        let _ = display_socket.shutdown(std::net::Shutdown::Both);
        let _ = listener_socket.shutdown(std::net::Shutdown::Both);
        drop(listener);
        Ok(())
    }

    fn forward(&self, connection: &Connection, commands: &Receiver<Command>) {
        while let Ok(command) = commands.recv() {
            let sent = match command {
                Command::Close => return,
                Command::Key(code, true) => notify(connection, KEYBOARD, "Press", &(code,)),
                Command::Key(code, false) => notify(connection, KEYBOARD, "Release", &(code,)),
                Command::Button(button, true) => notify(connection, MOUSE, "Press", &(button,)),
                Command::Button(button, false) => notify(connection, MOUSE, "Release", &(button,)),
                Command::Pointer(x, y) => notify(connection, MOUSE, "SetAbsPosition", &(x, y)),
                Command::Resize {
                    width,
                    height,
                    scale,
                } => {
                    let millimetres = |pixels: u32| {
                        (f64::from(pixels) * MM_PER_INCH / (BASE_DPI * scale)).round() as u16
                    };
                    notify(
                        connection,
                        CONSOLE,
                        "SetUIInfo",
                        &(
                            millimetres(width),
                            millimetres(height),
                            0i32,
                            0i32,
                            width,
                            height,
                        ),
                    )
                }
                Command::Offer => {
                    let serial = self.clipboard.serial.fetch_add(1, Ordering::SeqCst);
                    connection
                        .call_method(
                            Some(QEMU),
                            CLIPBOARD_PATH,
                            Some(CLIPBOARD),
                            "Grab",
                            &(CLIPBOARD_SELECTION, serial, vec![TEXT]),
                        )
                        .map(drop)
                }
                Command::GuestGrabbed => {
                    self.read_guest_clipboard(connection);
                    Ok(())
                }
            };
            if let Err(error) = sent {
                tracing::warn!("the guest display did not take a command: {error}");
            }
        }
    }

    fn read_guest_clipboard(&self, connection: &Connection) {
        let connection = connection.clone();
        let sink = Arc::clone(&self.sink);
        let spawned = std::thread::Builder::new()
            .name("velox-display-clipboard".to_owned())
            .spawn(move || {
                let reply = connection.call_method(
                    Some(QEMU),
                    CLIPBOARD_PATH,
                    Some(CLIPBOARD),
                    "Request",
                    &(CLIPBOARD_SELECTION, vec![TEXT]),
                );
                let Ok(reply) = reply else {
                    return;
                };
                let Ok((_, data)) = reply.body().deserialize::<(String, Vec<u8>)>() else {
                    return;
                };
                if let Ok(text) = String::from_utf8(data) {
                    sink(DisplayEvent::GuestCopied(
                        text.trim_end_matches('\0').to_owned(),
                    ));
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot read the guest's clipboard: {error}");
        }
    }
}

fn notify<B>(connection: &Connection, interface: &str, method: &str, body: &B) -> zbus::Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let message = zbus::Message::method_call(CONSOLE_PATH, method)?
        .destination(QEMU)?
        .interface(interface)?
        .with_flags(zbus::message::Flags::NoReplyExpected)?
        .build(body)?;
    connection.send(&message)
}

fn fourcc_of(pixman: u32) -> Option<u32> {
    let code: &[u8; 4] = match pixman {
        PIXMAN_X8R8G8B8 => b"XR24",
        PIXMAN_A8R8G8B8 => b"AR24",
        PIXMAN_X8B8G8R8 => b"XB24",
        PIXMAN_A8B8G8R8 => b"AB24",
        _ => return None,
    };
    Some(u32::from_le_bytes(*code))
}

struct ByteScanout {
    memory: File,
    stride: u32,
}

struct Listener {
    sink: Sink,
    bytes: Option<ByteScanout>,
}

impl Listener {
    fn unsupported(pixman: u32) -> fdo::Error {
        fdo::Error::NotSupported(format!("pixel format {pixman:#x}"))
    }

    fn copy_scanout(
        &mut self,
        width: u32,
        height: u32,
        stride: u32,
        pixman: u32,
        data: &[u8],
    ) -> std::io::Result<Option<Scanout>> {
        let Some(fourcc) = fourcc_of(pixman) else {
            return Ok(None);
        };
        let memory = File::from(memfd_create("velox-display", MemfdFlags::CLOEXEC)?);
        memory.set_len(u64::from(stride) * u64::from(height))?;
        memory.write_all_at(data, 0)?;
        let shared = memory.try_clone()?;
        self.bytes = Some(ByteScanout { memory, stride });
        Ok(Some(Scanout {
            memory: OwnedFd::from(shared),
            offset: 0,
            width,
            height,
            stride,
            fourcc,
        }))
    }
}

#[interface(name = "org.qemu.Display1.Listener")]
impl Listener {
    async fn scanout(
        &mut self,
        width: u32,
        height: u32,
        stride: u32,
        pixman_format: u32,
        data: Vec<u8>,
    ) -> fdo::Result<()> {
        let scanout = self
            .copy_scanout(width, height, stride, pixman_format, &data)
            .map_err(|error| fdo::Error::Failed(error.to_string()))?
            .ok_or_else(|| Self::unsupported(pixman_format))?;
        (self.sink)(DisplayEvent::Scanout(scanout));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn update(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        stride: u32,
        pixman_format: u32,
        data: Vec<u8>,
    ) -> fdo::Result<()> {
        let Some(bytes) = &self.bytes else {
            return Ok(());
        };
        if fourcc_of(pixman_format).is_none() {
            return Err(Self::unsupported(pixman_format));
        }
        let row = usize::try_from(width).unwrap_or_default() * 4;
        for (index, line) in data
            .chunks(stride as usize)
            .take(usize::try_from(height).unwrap_or_default())
            .enumerate()
        {
            let at = i64::from(y + index as i32) * i64::from(bytes.stride) + i64::from(x) * 4;
            let Ok(at) = u64::try_from(at) else {
                continue;
            };
            let end = row.min(line.len());
            bytes
                .memory
                .write_all_at(&line[..end], at)
                .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        }
        (self.sink)(DisplayEvent::Damage(Damage {
            x,
            y,
            width,
            height,
        }));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[zbus(name = "ScanoutDMABUF")]
    async fn scanout_dmabuf(
        &self,
        dmabuf: zbus::zvariant::OwnedFd,
        width: u32,
        height: u32,
        stride: u32,
        fourcc: u32,
        modifier: u64,
        y0_top: bool,
    ) {
        (self.sink)(DisplayEvent::Dmabuf(GuestDmabuf {
            planes: vec![(dmabuf.into(), 0, stride)],
            width,
            height,
            fourcc,
            modifier,
            y0_top,
        }));
    }

    #[zbus(name = "UpdateDMABUF")]
    async fn update_dmabuf(&self, x: i32, y: i32, width: i32, height: i32) {
        (self.sink)(DisplayEvent::Damage(Damage {
            x,
            y,
            width,
            height,
        }));
    }

    async fn disable(&mut self) {
        self.bytes = None;
        (self.sink)(DisplayEvent::Disabled);
    }

    async fn mouse_set(&self, _x: i32, _y: i32, on: i32) {
        (self.sink)(DisplayEvent::CursorShown(on != 0));
    }

    async fn cursor_define(&self, width: i32, height: i32, hot_x: i32, hot_y: i32, data: Vec<u8>) {
        let side = |value: i32| u32::try_from(value).unwrap_or_default();
        (self.sink)(DisplayEvent::Cursor {
            width: side(width),
            height: side(height),
            hotspot: (side(hot_x), side(hot_y)),
            pixels: data,
        });
    }

    #[zbus(property)]
    async fn interfaces(&self) -> Vec<String> {
        vec![LISTENER_MAP.to_owned(), LISTENER_DMABUF.to_owned()]
    }
}

struct ListenerMap {
    sink: Sink,
}

#[interface(name = "org.qemu.Display1.Listener.Unix.Map")]
impl ListenerMap {
    async fn scanout_map(
        &self,
        handle: zbus::zvariant::OwnedFd,
        offset: u32,
        width: u32,
        height: u32,
        stride: u32,
        pixman_format: u32,
    ) -> fdo::Result<()> {
        let fourcc = fourcc_of(pixman_format)
            .ok_or_else(|| fdo::Error::NotSupported(format!("pixel format {pixman_format:#x}")))?;
        (self.sink)(DisplayEvent::Scanout(Scanout {
            memory: handle.into(),
            offset,
            width,
            height,
            stride,
            fourcc,
        }));
        Ok(())
    }

    async fn update_map(&self, x: i32, y: i32, width: i32, height: i32) {
        (self.sink)(DisplayEvent::Damage(Damage {
            x,
            y,
            width,
            height,
        }));
    }
}

struct ListenerDmabuf {
    sink: Sink,
}

#[interface(name = "org.qemu.Display1.Listener.Unix.ScanoutDMABUF2")]
impl ListenerDmabuf {
    #[allow(clippy::too_many_arguments)]
    #[zbus(name = "ScanoutDMABUF2")]
    async fn scanout_dmabuf2(
        &self,
        dmabuf: Vec<zbus::zvariant::OwnedFd>,
        _x: u32,
        _y: u32,
        width: u32,
        height: u32,
        offset: Vec<u32>,
        stride: Vec<u32>,
        num_planes: u32,
        fourcc: u32,
        _backing_width: u32,
        _backing_height: u32,
        modifier: u64,
        y0_top: bool,
    ) {
        let planes = dmabuf
            .into_iter()
            .zip(offset)
            .zip(stride)
            .take(num_planes as usize)
            .map(|((fd, offset), stride)| (OwnedFd::from(fd), offset, stride))
            .collect();
        (self.sink)(DisplayEvent::Dmabuf(GuestDmabuf {
            planes,
            width,
            height,
            fourcc,
            modifier,
            y0_top,
        }));
    }
}

struct GuestClipboard {
    clipboard: Arc<ClipboardState>,
    commands: Sender<Command>,
}

#[interface(name = "org.qemu.Display1.Clipboard")]
impl GuestClipboard {
    async fn register(&self) {
        self.clipboard.serial.store(0, Ordering::SeqCst);
    }

    async fn unregister(&self) {}

    async fn grab(&self, selection: u32, serial: u32, mimes: Vec<String>) {
        self.clipboard.serial.store(serial + 1, Ordering::SeqCst);
        if selection == CLIPBOARD_SELECTION && mimes.iter().any(|mime| mime == TEXT) {
            let _ = self.commands.send(Command::GuestGrabbed);
        }
    }

    async fn release(&self, _selection: u32) {}

    async fn request(&self, selection: u32, mimes: Vec<String>) -> fdo::Result<(String, Vec<u8>)> {
        let offered = self
            .clipboard
            .offered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match offered {
            Some(text)
                if selection == CLIPBOARD_SELECTION && mimes.iter().any(|mime| mime == TEXT) =>
            {
                Ok((TEXT.to_owned(), text.into_bytes()))
            }
            _ => Err(fdo::Error::Failed("nothing to paste".to_owned())),
        }
    }

    #[zbus(property)]
    async fn interfaces(&self) -> Vec<String> {
        Vec::new()
    }
}
