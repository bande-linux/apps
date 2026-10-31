use std::cell::RefCell;
use std::rc::Rc;

use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, FreeLayout, SearchField, TEXT_FIELD_HEIGHT, WidgetId,
};
use bande_toolkit::{Header, Image, Point, Rect};

use crate::L10N;
use crate::imap::Role;
use crate::ui::symbols::Symbol;
use crate::ui::view::{LIST_WIDTH, MailView, SIDEBAR_WIDTH, TOOLBAR_HEIGHT};

const SYMBOL_PIXELS: u32 = 32;
const GROUP_GAP: f32 = 14.0;
const TRAILING_INSET: f32 = 12.0;
const TITLE_INSET: f32 = 16.0;
const SEARCH_WIDTH: f32 = 200.0;
const MIN_SEARCH_WIDTH: f32 = 120.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    GetMail,
    Compose,
    Archive,
    Delete,
    Junk,
    Reply,
    ReplyAll,
    Forward,
    Flag,
    Search(String),
    RemoveAccount(crate::store::AccountId),
}

pub type Signals = Rc<RefCell<Vec<Signal>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Toolbar {
    pub get_mail: WidgetId,
    pub compose: WidgetId,
    pub archive: WidgetId,
    pub delete: WidgetId,
    pub junk: WidgetId,
    pub reply: WidgetId,
    pub reply_all: WidgetId,
    pub forward: WidgetId,
    pub flag: WidgetId,
    pub search: WidgetId,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self {
            get_mail: WidgetId::next(),
            compose: WidgetId::next(),
            archive: WidgetId::next(),
            delete: WidgetId::next(),
            junk: WidgetId::next(),
            reply: WidgetId::next(),
            reply_all: WidgetId::next(),
            forward: WidgetId::next(),
            flag: WidgetId::next(),
            search: WidgetId::next(),
        }
    }
}

fn centred(height: f32) -> f32 {
    ((TOOLBAR_HEIGHT - height) / 2.0).round()
}

fn sender(signals: &Signals) -> impl Fn(Signal) + 'static {
    let signals = Rc::clone(signals);
    move |signal| signals.borrow_mut().push(signal)
}

impl MailView {
    fn symbol_button(&mut self, symbol: Symbol, label: &str, signal: Signal) -> Button {
        let send = sender(&self.signals);
        let click = move || send(signal.clone());
        let image = Image::from_svg(symbol.svg(), SYMBOL_PIXELS, SYMBOL_PIXELS);
        match image {
            Some(image) => Button::symbol(image, label, click),
            None => Button::new(label, click),
        }
    }

    pub(crate) fn host_mut(&mut self) -> Option<&mut FreeLayout> {
        let host = self.host;
        self.ui.widget_as::<FreeLayout>(host)
    }

    pub(crate) fn insert_toolbar(&mut self) {
        let buttons = [
            (Symbol::GetMail, "action-get-mail", Signal::GetMail),
            (Symbol::Compose, "action-compose", Signal::Compose),
            (Symbol::Archive, "action-archive", Signal::Archive),
            (Symbol::Trash, "action-delete", Signal::Delete),
            (Symbol::Junk, "action-junk", Signal::Junk),
            (Symbol::Reply, "action-reply", Signal::Reply),
            (Symbol::ReplyAll, "action-reply-all", Signal::ReplyAll),
            (Symbol::Forward, "action-forward", Signal::Forward),
            (Symbol::Flag, "action-flag", Signal::Flag),
        ];
        let mut ids = Vec::new();
        for (symbol, label, signal) in buttons {
            let button = self.symbol_button(symbol, &L10N.tr(label), signal);
            if let Some(host) = self.host_mut() {
                ids.push(host.insert(button, Rect::default()));
            }
        }
        let send = sender(&self.signals);
        let search = SearchField::new(&L10N.tr("search-placeholder"), "", move |text| {
            send(Signal::Search(text.to_owned()));
        });
        let search = self
            .host_mut()
            .map(|host| host.insert(search, Rect::default()));
        if let (
            [
                get_mail,
                compose,
                archive,
                delete,
                junk,
                reply,
                reply_all,
                forward,
                flag,
            ],
            Some(search),
        ) = (ids.as_slice(), search)
        {
            self.toolbar = Toolbar {
                get_mail: *get_mail,
                compose: *compose,
                archive: *archive,
                delete: *delete,
                junk: *junk,
                reply: *reply,
                reply_all: *reply_all,
                forward: *forward,
                flag: *flag,
                search,
            };
        }
    }

    pub(crate) fn toolbar_frames(&self) -> Vec<(WidgetId, Rect)> {
        let toolbar = self.toolbar;
        let button_y = centred(BUTTON_HEIGHT);
        let list_end = SIDEBAR_WIDTH + LIST_WIDTH;
        let button = |x: f32| Rect::new(x, button_y, BUTTON_HEIGHT, BUTTON_HEIGHT);
        let mut frames = vec![
            (
                toolbar.get_mail,
                button(list_end - 2.0 * BUTTON_HEIGHT - TRAILING_INSET),
            ),
            (
                toolbar.compose,
                button(list_end - BUTTON_HEIGHT - TRAILING_INSET),
            ),
        ];
        let right = self.size.0 - TRAILING_INSET;
        let groups: [&[WidgetId]; 3] = [
            &[toolbar.archive, toolbar.delete, toolbar.junk],
            &[toolbar.reply, toolbar.reply_all, toolbar.forward],
            &[toolbar.flag],
        ];
        let buttons_width = groups
            .iter()
            .map(|group| group.len() as f32 * BUTTON_HEIGHT)
            .sum::<f32>()
            + GROUP_GAP * groups.len() as f32;
        let room = right - (list_end + GROUP_GAP) - buttons_width;
        let search_width = SEARCH_WIDTH.min(room).max(MIN_SEARCH_WIDTH);
        let mut x = right - search_width - buttons_width;
        for group in groups {
            for id in group {
                frames.push((*id, button(x)));
                x += BUTTON_HEIGHT;
            }
            x += GROUP_GAP;
        }
        frames.push((
            toolbar.search,
            Rect::new(
                right - search_width,
                centred(TEXT_FIELD_HEIGHT),
                search_width,
                TEXT_FIELD_HEIGHT,
            ),
        ));
        frames
    }

    pub(crate) fn layout_toolbar(&mut self) {
        let frames = self.toolbar_frames();
        if let Some(host) = self.host_mut() {
            for (id, frame) in frames {
                host.set_frame(id, frame);
            }
        }
    }

    pub(crate) fn toolbar_header(&self) -> Header {
        Header {
            title_x: SIDEBAR_WIDTH + TITLE_INSET,
            items: self
                .toolbar_frames()
                .into_iter()
                .map(|(_, frame)| frame)
                .collect(),
            tabs: Vec::new(),
        }
    }

    pub(crate) fn over_control(&self, at: Point) -> bool {
        self.toolbar_frames()
            .iter()
            .any(|(_, frame)| frame.contains(at))
    }

    pub(crate) fn refresh_toolbar(&mut self) {
        let selected = self.selected.is_some();
        let has_account = self.has_accounts();
        let reading = self
            .reading
            .as_ref()
            .is_some_and(|reading| reading.downloaded);
        let states = [
            (self.toolbar.get_mail, has_account),
            (self.toolbar.compose, has_account),
            (self.toolbar.archive, self.can_move_to(Role::Archive)),
            (self.toolbar.delete, selected),
            (self.toolbar.junk, self.can_move_to(Role::Junk)),
            (self.toolbar.reply, reading),
            (self.toolbar.reply_all, reading),
            (self.toolbar.forward, reading),
            (self.toolbar.flag, selected),
        ];
        for (id, enabled) in states {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
    }
}
