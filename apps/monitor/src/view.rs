use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use accesskit::{ActionRequest, TreeUpdate};
use unbound_toolkit::ui::{
    Alert, Button, ClipboardRequest, Cursor, Event, Key, Menu, Modifiers, Pointer, PointerButton,
};
use unbound_toolkit::{
    Color, Header, Image, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

use unbound_app_menu::Entry;

use crate::L10N;
use crate::model::{StopTarget, stop_target};
use crate::sampler::Snapshot;
use crate::signal::{SignalError, Stop};

pub(crate) const QUIT: &str = "quit";
pub(crate) const FORCE_QUIT: &str = "force-quit";
const CANCEL: &str = "cancel";
pub(crate) const ALERT_ICON: &str = "org.unbound.ActivityMonitor";
pub(crate) const INSPECT_SYMBOL: &str = "dialog-information-symbolic";
const STOP_SYMBOL: &[u8] = include_bytes!("../data/process-stop-symbolic.svg");
const SYMBOL_PIXELS: u32 = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Resample,
    ShowInFiles(PathBuf),
    Copy(String),
    Clipboard(ClipboardRequest),
    EmojiPicker,
    Menu(Menu, Point),
    Inspect(u32),
    SaveState(String),
    QuitApp {
        app_id: String,
        roots: Vec<(u32, u64)>,
    },
}

type FindIcon = Box<dyn Fn(&str) -> Option<Image>>;

pub struct Icons {
    loaded: HashMap<String, Option<Image>>,
    find: FindIcon,
}

impl Icons {
    pub fn new(find: impl Fn(&str) -> Option<Image> + 'static) -> Self {
        Self {
            loaded: HashMap::new(),
            find: Box::new(find),
        }
    }

    pub fn get(&mut self, name: &str) -> Option<Image> {
        if let Some(image) = self.loaded.get(name) {
            return image.clone();
        }
        let image = (self.find)(name);
        self.loaded.insert(name.to_owned(), image.clone());
        image
    }
}

pub(crate) fn contains(rect: Rect, at: Point) -> bool {
    at.x >= rect.x && at.x < rect.x + rect.width && at.y >= rect.y && at.y < rect.y + rect.height
}

pub fn is_secondary(pointer: &Pointer) -> bool {
    pointer.button == PointerButton::Secondary
        || (pointer.button == PointerButton::Primary && pointer.modifiers.control)
}

pub fn opens_menu(key: Key, modifiers: Modifiers) -> bool {
    key == Key::Menu || (key == Key::Function(10) && modifiers.shift && !modifiers.logo)
}

pub struct Painter<'a> {
    pub text: &'a mut TextSystem,
    pub items: Vec<Item>,
}

impl Painter<'_> {
    pub fn text_in(&mut self, text: &str, style: TextStyle, frame: Rect, align: TextAlign) {
        let paragraph = self.text.paragraph(text, style, Some(frame.width), align);
        let y = frame.y + ((frame.height - paragraph.height()) / 2.0).round();
        self.items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Text {
                origin: Point::new(frame.x, y),
                paragraph,
            }],
        });
    }

    pub fn fill(&mut self, shape: Shape, color: Color) {
        self.items.push(Item::Fill {
            shape,
            paint: color.into(),
        });
    }
}

pub(crate) fn symbol_button(
    symbol: Option<Image>,
    label: &str,
    on_click: impl FnMut() + 'static,
) -> Button {
    match symbol {
        Some(symbol) => Button::symbol(symbol, label, on_click),
        None => Button::new(label, on_click),
    }
}

pub(crate) fn stop_button(on_click: impl FnMut() + 'static) -> Button {
    symbol_button(
        Image::from_svg(STOP_SYMBOL, SYMBOL_PIXELS, SYMBOL_PIXELS),
        &L10N.tr("stop-label"),
        on_click,
    )
}

fn with_icon(alert: Alert, icon: Option<Image>) -> Alert {
    match icon {
        Some(icon) => alert.icon(icon),
        None => alert,
    }
}

pub(crate) fn stop_alert(
    target: &StopTarget,
    force: bool,
    icon: Option<Image>,
    choose: impl FnMut(&str) + 'static,
) -> Alert {
    let names = || {
        [
            ("name", target.name.clone().into()),
            ("count", target.count.into()),
        ]
    };
    let alert = match force {
        true => Alert::new(
            &L10N.tr_args("force-quit-confirm", names()),
            &L10N.tr("force-quit-confirm-message"),
            choose,
        )
        .destructive(&L10N.tr("button-force-quit"), FORCE_QUIT),
        false => Alert::new(
            &L10N.tr_args("quit-title", names()),
            &L10N.tr("quit-message"),
            choose,
        )
        .other(&L10N.tr("button-force-quit"), FORCE_QUIT)
        .primary(&L10N.tr("button-quit"), QUIT),
    };
    with_icon(alert.cancel(&L10N.tr("button-cancel"), CANCEL), icon)
}

pub(crate) fn answer_stop(
    target: &StopTarget,
    choice: &str,
    icon: Option<Image>,
    choose: impl FnMut(&str) + 'static,
) -> Option<Alert> {
    let how = match choice {
        QUIT => Stop::Quit,
        FORCE_QUIT => Stop::ForceQuit,
        _ => return None,
    };
    match stop_target(target, how) {
        Ok(()) => None,
        Err(SignalError::PermissionDenied) => Some(with_icon(
            Alert::new(
                &L10N.tr_args("quit-denied", [("name", target.name.clone().into())]),
                &L10N.tr("quit-denied-message"),
                choose,
            )
            .primary(&L10N.tr("button-ok"), CANCEL),
            icon,
        )),
        Err(error) => {
            tracing::warn!("cannot stop {}: {error:?}", target.name);
            None
        }
    }
}

pub fn due_redraws(screens: &[Box<dyn Screen>], now: Instant) -> Vec<bool> {
    screens
        .iter()
        .map(|screen| screen.deadline().is_some_and(|deadline| deadline <= now))
        .collect()
}

pub trait Screen {
    fn scene(&mut self, width: u32, height: u32) -> Scene;
    fn set_palette(&mut self, palette: Palette);
    fn set_focused(&mut self, focused: bool);
    fn handle(&mut self, event: &Event) -> bool;
    fn update(&mut self, snapshot: Snapshot);
    fn measures_proportional_memory(&self) -> bool;
    fn take_requests(&mut self) -> Vec<Request>;
    fn menu_action(&mut self, action: &str) -> bool;
    fn menu_closed(&mut self) -> bool;
    fn pasted(&mut self, text: &str) -> bool;
    fn finished(&self) -> bool;
    fn set_responding(&mut self, pid: u32, app_id: &str, responding: bool) -> bool;
    fn accessibility(&self) -> Option<TreeUpdate>;
    fn perform(&mut self, request: &ActionRequest) -> bool;
    fn cursor(&self) -> Cursor;
    fn animating(&self) -> bool;
    fn deadline(&self) -> Option<Instant>;

    fn header(&self) -> Option<Header> {
        None
    }

    fn set_full_screen(&mut self, _full_screen: bool) {}

    fn measures_gpu(&self) -> bool {
        false
    }

    fn measures_wakeups(&self) -> bool {
        false
    }

    fn measures_sleep_blockers(&self) -> bool {
        false
    }

    fn app_menus(&self) -> Option<Vec<Entry>> {
        None
    }

    fn measures_disk(&self) -> bool {
        false
    }

    fn inspects(&self, _pid: u32) -> bool {
        false
    }
}
