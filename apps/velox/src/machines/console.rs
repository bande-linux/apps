use std::cell::Cell;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use bande_toolkit::ui::{
    ClipboardRequest, Cursor, PointerButton, PointerCapture, SurfaceInput, SurfaceScaling,
    SurfaceView, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    CursorImage, Dmabuf, DmabufPlane, ExternalSurface, Point, Rect, Region, ShmLayout, Window,
};
use bande_vm::{
    Damage, Display, DisplayEvent, DisplayInput, GuestDmabuf, Library, MouseButton, Scanout, Wheel,
};

use crate::L10N;

const RESIZE_SETTLE: Duration = Duration::from_millis(200);
const WHEEL_STEP: f32 = 15.0;

pub enum Outcome {
    Nothing,
    Redraw,
    Show(Box<dyn Widget>),
    NoPicture,
    Closed(Option<String>),
}

pub struct Console {
    display: Display,
    events: Receiver<DisplayEvent>,
    wake: UnixStream,
    surface: Option<ExternalSurface>,
    _memory: Option<OwnedFd>,
    _planes: Vec<OwnedFd>,
    picture: Option<(u32, u32)>,
    view: Option<WidgetId>,
    cursor: Cursor,
    has_guest_cursor: bool,
    wanted_size: Option<(u32, u32, f64)>,
    sent_size: Option<(u32, u32, f64)>,
    resize_at: Option<Instant>,
    reading_clipboard: bool,
}

fn scroll_guest(input: &DisplayInput, gathered: &Cell<Point>, delta: Point) {
    let mut total = Point::new(gathered.get().x + delta.x, gathered.get().y + delta.y);
    let wheel = |value: &mut f32, negative: Wheel, positive: Wheel| {
        while value.abs() >= WHEEL_STEP {
            input.wheel(match *value < 0.0 {
                true => negative,
                false => positive,
            });
            *value -= WHEEL_STEP.copysign(*value);
        }
    };
    wheel(&mut total.y, Wheel::Up, Wheel::Down);
    wheel(&mut total.x, Wheel::Left, Wheel::Right);
    gathered.set(total);
}

fn guest_button(button: PointerButton) -> MouseButton {
    match button {
        PointerButton::Primary => MouseButton::Left,
        PointerButton::Secondary => MouseButton::Right,
        PointerButton::Middle => MouseButton::Middle,
        PointerButton::Back => MouseButton::Side,
        PointerButton::Forward => MouseButton::Extra,
    }
}

fn pass_to_guest(input: &DisplayInput, gathered: &Cell<Point>, event: SurfaceInput) {
    match event {
        SurfaceInput::PointerMoved(at) => input.pointer(at.x as u32, at.y as u32),
        SurfaceInput::Button { button, pressed } => input.button(guest_button(button), pressed),
        SurfaceInput::Scroll { delta, .. } => scroll_guest(input, gathered, delta),
        SurfaceInput::Key { code, pressed } => input.key(code, pressed),
        SurfaceInput::PointerLeft | SurfaceInput::Motion { .. } | SurfaceInput::Captured(_) => {}
    }
}

impl Console {
    pub fn open(machine: &str) -> Result<Self, bande_vm::Error> {
        let control = Library::open()?.control_socket(machine);
        let (wake, ring) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        ring.set_nonblocking(true)?;
        let (sender, events) = mpsc::channel();
        let display = Display::open(control, move |event| {
            if sender.send(event).is_ok()
                && let Err(error) = (&ring).write_all(&[1])
                && error.kind() != ErrorKind::WouldBlock
            {
                tracing::debug!("cannot wake the console: {error}");
            }
        });
        Ok(Self {
            display,
            events,
            wake,
            surface: None,
            _memory: None,
            _planes: Vec::new(),
            picture: None,
            view: None,
            cursor: Cursor::Default,
            has_guest_cursor: false,
            wanted_size: None,
            sent_size: None,
            resize_at: None,
            reading_clipboard: false,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn has_closed_picture(&self) -> bool {
        self.surface.is_some() && self.picture.is_none()
    }

    pub fn send_ctrl_alt_delete(&self) {
        self.display.send_ctrl_alt_delete();
    }

    pub fn view(&mut self) -> Option<Box<dyn Widget>> {
        let surface = self.surface.as_ref()?;
        let (width, height) = self.picture?;
        let input = self.display.input();
        let gathered = Cell::new(Point::new(0.0, 0.0));
        let mut view = SurfaceView::new(surface.id(), &L10N.tr("machine-console"), move |event| {
            pass_to_guest(&input, &gathered, event)
        })
        .scaling(SurfaceScaling::Fit)
        .capture_on_click(PointerCapture::Free);
        view.set_size(width, height);
        view.set_cursor(self.cursor);
        self.view = Some(view.id());
        Some(Box::new(view))
    }

    pub fn place(&mut self, frame: Rect, scale: f64) {
        let size = (
            (f64::from(frame.width) * scale).round() as u32,
            (f64::from(frame.height) * scale).round() as u32,
            scale,
        );
        if size.0 == 0 || size.1 == 0 || self.wanted_size == Some(size) {
            return;
        }
        self.resize_at = Some(match self.wanted_size {
            None => Instant::now(),
            Some(_) => Instant::now() + RESIZE_SETTLE,
        });
        self.wanted_size = Some(size);
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.resize_at
    }

    pub fn tick(&mut self, now: Instant) {
        if self.resize_at.is_some_and(|at| at <= now) {
            self.resize_at = None;
            if self.wanted_size != self.sent_size
                && let Some(size) = self.wanted_size
            {
                self.sent_size = Some(size);
                self.display.resize(size.0, size.1, size.2);
            }
        }
    }

    pub fn clipboard_changed(&mut self, window: &mut Window) {
        self.reading_clipboard = true;
        if let Err(error) = window.paste() {
            tracing::warn!("cannot read the clipboard for the guest: {error}");
        }
    }

    pub fn pasted(&mut self, text: &str) {
        if std::mem::take(&mut self.reading_clipboard) {
            self.display.offer_clipboard(text.to_owned());
        }
    }

    fn show_dmabuf(&mut self, window: &mut Window, dmabuf: GuestDmabuf) -> Result<(), String> {
        if self.surface.is_none() {
            self.surface = Some(
                window
                    .create_external_surface()
                    .map_err(|error| error.to_string())?,
            );
        }
        let Some(surface) = self.surface.as_mut() else {
            return Ok(());
        };
        let planes: Vec<DmabufPlane<'_>> = dmabuf
            .planes
            .iter()
            .map(|(fd, offset, stride)| DmabufPlane {
                fd: fd.as_fd(),
                offset: *offset,
                stride: *stride,
            })
            .collect();
        surface
            .show_dmabuf(&Dmabuf {
                planes: &planes,
                width: dmabuf.width,
                height: dmabuf.height,
                fourcc: dmabuf.fourcc,
                modifier: dmabuf.modifier,
                flipped: !dmabuf.y0_top,
            })
            .map_err(|error| error.to_string())?;
        self.picture = Some((dmabuf.width, dmabuf.height));
        self._planes = dmabuf.planes.into_iter().map(|(fd, _, _)| fd).collect();
        Ok(())
    }

    fn show(&mut self, window: &mut Window, scanout: Scanout) -> Result<bool, String> {
        if self.surface.is_none() {
            self.surface = Some(
                window
                    .create_external_surface()
                    .map_err(|error| error.to_string())?,
            );
        }
        let Some(surface) = self.surface.as_mut() else {
            return Ok(false);
        };
        surface
            .show_shm(
                scanout.memory.as_fd(),
                ShmLayout {
                    width: scanout.width,
                    height: scanout.height,
                    stride: scanout.stride,
                    offset: scanout.offset,
                    fourcc: scanout.fourcc,
                },
            )
            .map_err(|error| error.to_string())?;
        self._memory = Some(scanout.memory);
        self.picture = Some((scanout.width, scanout.height));
        Ok(true)
    }

    pub fn pump(&mut self, window: &mut Window, ui: &mut Ui) -> Outcome {
        let mut bytes = [0; 256];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        let mut damage: Vec<Region> = Vec::new();
        let mut outcome = Outcome::Nothing;
        let events: Vec<DisplayEvent> = self.events.try_iter().collect();
        for event in events {
            match event {
                DisplayEvent::Scanout(scanout) => {
                    if let Err(reason) = self.show(window, scanout) {
                        return Outcome::Closed(Some(reason));
                    }
                    damage.clear();
                    let shown = self
                        .picture
                        .zip(self.view.and_then(|view| ui.widget_as::<SurfaceView>(view)));
                    outcome = match shown {
                        Some(((width, height), view)) => {
                            view.set_size(width, height);
                            Outcome::Redraw
                        }
                        None => {
                            self.clipboard_changed(window);
                            self.view().map_or(Outcome::Nothing, Outcome::Show)
                        }
                    };
                }
                DisplayEvent::Dmabuf(dmabuf) => {
                    if let Err(reason) = self.show_dmabuf(window, dmabuf) {
                        return Outcome::Closed(Some(reason));
                    }
                    damage.clear();
                    let shown = self
                        .picture
                        .zip(self.view.and_then(|view| ui.widget_as::<SurfaceView>(view)));
                    outcome = match shown {
                        Some(((width, height), view)) => {
                            view.set_size(width, height);
                            Outcome::Redraw
                        }
                        None => self.view().map_or(Outcome::Nothing, Outcome::Show),
                    };
                }
                DisplayEvent::Damage(area) => damage.push(region(area)),
                DisplayEvent::Disabled => {
                    self.picture = None;
                    self.view = None;
                    outcome = Outcome::NoPicture;
                }
                DisplayEvent::Cursor {
                    width,
                    height,
                    hotspot,
                    pixels,
                } => {
                    let image = CursorImage {
                        width,
                        height,
                        hotspot,
                        pixels: &pixels,
                    };
                    if let Err(error) = window.set_cursor_image(image) {
                        tracing::warn!("cannot show the guest's cursor: {error}");
                    }
                    self.has_guest_cursor = true;
                    self.set_cursor(ui, Cursor::Image);
                }
                DisplayEvent::CursorShown(shown) if self.has_guest_cursor => {
                    self.set_cursor(
                        ui,
                        match shown {
                            true => Cursor::Image,
                            false => Cursor::Hidden,
                        },
                    );
                }
                DisplayEvent::CursorShown(_) => {}
                DisplayEvent::GuestCopied(text) => {
                    if let Err(error) = window.serve_clipboard(ClipboardRequest::Copy(text)) {
                        tracing::warn!("cannot copy from the guest: {error}");
                    }
                }
                DisplayEvent::Closed(reason) => return Outcome::Closed(reason),
            }
        }
        if !damage.is_empty()
            && let Some(surface) = self.surface.as_mut()
            && let Err(error) = surface.damage(&damage)
        {
            tracing::warn!("cannot show the guest's picture: {error}");
        }
        outcome
    }

    fn set_cursor(&mut self, ui: &mut Ui, cursor: Cursor) {
        self.cursor = cursor;
        if let Some(view) = self.view.and_then(|view| ui.widget_as::<SurfaceView>(view)) {
            view.set_cursor(cursor);
        }
    }
}

fn region(area: Damage) -> Region {
    let side = |value: i32| u32::try_from(value).unwrap_or_default();
    Region {
        x: side(area.x),
        y: side(area.y),
        width: side(area.width),
        height: side(area.height),
    }
}
