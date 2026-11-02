use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use accesskit::{ActionRequest, TreeUpdate};
use bande_toolkit::ui::{
    Alert, BUTTON_HEIGHT, Button, ClipboardRequest, Context, Cursor, Event, FreeLayout, Label,
    Menu, PopUpButton, Size, TEXT_FIELD_HEIGHT, TextField, TextView, Ui, Widget, WidgetId,
    shortcut_pressed,
};
use bande_toolkit::{
    Appearance, Header, Image, Item, Palette, Point, Radius, Rect, Scene, Shape, TextAlign,
    TextRole, TextStyle, TextSystem, window_controls_end,
};

use crate::L10N;
use crate::compose::{Draft, Original, Outgoing, keep_unsent, unsent_folder};
use crate::message::{Address, Attachment, parse_addresses};
use crate::store::Account;
use crate::ui::symbols::Symbol;
use crate::ui::view::{SHORTCUTS, TOOLBAR_HEIGHT};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub const COMPOSE_SIZE: (u32, u32) = (720, 620);
const MARGIN: f32 = 16.0;
const LABEL_WIDTH: f32 = 64.0;
const FIELD_ROW: f32 = 36.0;
const FIELD_FONT: f32 = 13.0;
const SYMBOL_PIXELS: u32 = 32;
const TRAILING_INSET: f32 = 12.0;
const TITLE_X: f32 = window_controls_end(true) + 16.0;
const BODY_GAP: f32 = 10.0;
const CHIP_HEIGHT: f32 = 26.0;
const CHIP_PADDING: f32 = 10.0;
const CHIP_GAP: f32 = 8.0;
const AUTOSAVE_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    Send,
    Attach,
    From(usize),
    Dismissed,
}

pub struct ComposeView {
    ui: Ui,
    host: WidgetId,
    send: WidgetId,
    attach: WidgetId,
    to: WidgetId,
    cc: WidgetId,
    bcc: WidgetId,
    bcc_shown: bool,
    subject: WidgetId,
    from: Option<WidgetId>,
    body: WidgetId,
    signals: Rc<RefCell<Vec<Signal>>>,
    senders: Vec<Account>,
    sender: usize,
    draft: Draft,
    answering: Option<Original>,
    replaces: Option<Original>,
    initial: Draft,
    palette: Palette,
    size: (f32, f32),
    sent: Option<Outgoing>,
    requests: Vec<ComposeRequest>,
    chips: Vec<(Rect, usize)>,
    token: u64,
    closed: bool,
    unsent: Option<PathBuf>,
    recovered: bool,
    autosaved: Option<Draft>,
    autosave_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComposeRequest {
    ChooseFiles,
    ShowMenu(Menu, Point),
    Clipboard(ClipboardRequest),
}

fn joined(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(Address::full)
        .collect::<Vec<_>>()
        .join(", ")
}

fn rows(from: bool, bcc: bool) -> Vec<&'static str> {
    let mut rows = vec!["compose-to", "compose-cc"];
    if bcc {
        rows.push("compose-bcc");
    }
    rows.push("compose-subject");
    if from {
        rows.push("compose-from");
    }
    rows
}

impl ComposeView {
    pub fn new(
        outgoing: Outgoing,
        senders: Vec<Account>,
        bcc_field: bool,
        palette: Palette,
        text: TextSystem,
    ) -> Self {
        let Outgoing {
            draft,
            answering,
            replaces,
            unsent,
        } = outgoing;
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        let recovered = unsent.is_some();
        let unsent = unsent.or_else(|| {
            let started = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            unsent_folder().map(|folder| folder.join(format!("{started}-{token}.eml")))
        });
        let signals = Rc::new(RefCell::new(Vec::new()));
        let mut host = FreeLayout::new();
        let host_id = Widget::id(&host);
        let send_signal = Rc::clone(&signals);
        let click = move || send_signal.borrow_mut().push(Signal::Send);
        let send_button = match Image::from_svg(Symbol::Sent.svg(), SYMBOL_PIXELS, SYMBOL_PIXELS) {
            Some(image) => Button::symbol(image, &L10N.tr("compose-send"), click),
            None => Button::new(&L10N.tr("compose-send"), click),
        };
        let send = host.insert(send_button, Rect::default());
        let attach_signal = Rc::clone(&signals);
        let click = move || attach_signal.borrow_mut().push(Signal::Attach);
        let attach_button =
            match Image::from_svg(Symbol::Attachment.svg(), SYMBOL_PIXELS, SYMBOL_PIXELS) {
                Some(image) => Button::symbol(image, &L10N.tr("compose-attach"), click),
                None => Button::new(&L10N.tr("compose-attach"), click),
            };
        let attach = host.insert(attach_button, Rect::default());
        let field = |text: &str| TextField::new("", text, |_| {}).borderless(FIELD_FONT);
        let to = host.insert(field(&joined(&draft.to)), Rect::default());
        let cc = host.insert(field(&joined(&draft.cc)), Rect::default());
        let bcc_shown = bcc_field || !draft.bcc.is_empty();
        let bcc = match bcc_shown {
            true => host.insert(field(&joined(&draft.bcc)), Rect::default()),
            false => host.insert(Label::new(""), Rect::default()),
        };
        let subject = host.insert(field(&draft.subject), Rect::default());
        let sender = senders
            .iter()
            .position(|account| account.email.eq_ignore_ascii_case(&draft.from.email))
            .unwrap_or(0);
        let from = (senders.len() > 1).then(|| {
            let choices = senders
                .iter()
                .map(|account| account.address().full())
                .collect::<Vec<_>>();
            let chosen = Rc::clone(&signals);
            host.insert(
                PopUpButton::new(&L10N.tr("compose-from"), &choices, sender, move |index| {
                    chosen.borrow_mut().push(Signal::From(index));
                }),
                Rect::default(),
            )
        });
        let body = host.insert(
            TextView::new(&L10N.tr("compose-placeholder"), &draft.body, |_| {}).borderless(),
            Rect::default(),
        );
        let mut view = Self {
            ui: Ui::new(Box::new(host), Context::new(text, palette)),
            host: host_id,
            send,
            attach,
            to,
            cc,
            bcc,
            bcc_shown,
            subject,
            from,
            body,
            signals,
            senders,
            sender,
            draft,
            answering,
            replaces,
            initial: Draft::default(),
            palette,
            size: (0.0, 0.0),
            sent: None,
            requests: Vec::new(),
            chips: Vec::new(),
            token,
            closed: false,
            unsent,
            recovered,
            autosaved: None,
            autosave_at: None,
        };
        let first = match view.draft.to.is_empty() {
            true => view.to,
            false => view.body,
        };
        view.ui.set_focus(first);
        view.ui.set_sheet_top(TOOLBAR_HEIGHT);
        view.initial = view.compose_draft();
        view
    }

    pub fn title(&self) -> String {
        let subject = self
            .ui
            .widget_ref::<TextField>(self.subject)
            .map(TextField::text)
            .unwrap_or_default();
        match subject.trim() {
            "" => L10N.tr("compose-title"),
            subject => subject.to_owned(),
        }
    }

    pub fn set_bcc_field(&mut self, shown: bool) {
        if shown == self.bcc_shown || (!shown && !self.field_text(self.bcc).trim().is_empty()) {
            return;
        }
        let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) else {
            return;
        };
        let replaced = match shown {
            true => host.replace(
                self.bcc,
                TextField::new("", "", |_| {}).borderless(FIELD_FONT),
            ),
            false => host.replace(self.bcc, Label::new("")),
        };
        if let Some(bcc) = replaced {
            self.bcc = bcc;
            self.bcc_shown = shown;
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.ui.context().palette = palette;
    }

    fn frames(&self) -> Vec<(WidgetId, Rect)> {
        let (width, height) = self.size;
        let button = |slot: f32| {
            Rect::new(
                width - TRAILING_INSET - slot * BUTTON_HEIGHT,
                ((TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0).round(),
                BUTTON_HEIGHT,
                BUTTON_HEIGHT,
            )
        };
        let mut frames = vec![(self.send, button(1.0)), (self.attach, button(2.0))];
        let field_x = MARGIN + LABEL_WIDTH;
        let field_width = (width - field_x - MARGIN).max(40.0);
        let mut y = TOOLBAR_HEIGHT;
        let mut fields = vec![self.to, self.cc];
        match self.bcc_shown {
            true => fields.push(self.bcc),
            false => frames.push((self.bcc, Rect::default())),
        }
        fields.push(self.subject);
        fields.extend(self.from);
        for id in fields {
            let height = self
                .ui
                .widget_ref::<TextField>(id)
                .map(|field| field.frame().height)
                .filter(|height| *height > 0.0)
                .unwrap_or(TEXT_FIELD_HEIGHT);
            frames.push((
                id,
                Rect::new(
                    field_x,
                    y + ((FIELD_ROW - height) / 2.0).round(),
                    field_width,
                    height,
                ),
            ));
            y += FIELD_ROW;
        }
        if !self.draft.attachments.is_empty() {
            y += FIELD_ROW;
        }
        frames.push((
            self.body,
            Rect::new(
                MARGIN - 8.0,
                y + BODY_GAP,
                width - 2.0 * MARGIN + 16.0,
                (height - y - BODY_GAP - MARGIN).max(40.0),
            ),
        ));
        frames
    }

    pub fn header(&self) -> Header {
        Header {
            title_x: TITLE_X,
            items: self
                .frames()
                .into_iter()
                .take(2)
                .map(|(_, frame)| frame)
                .collect(),
            tabs: Vec::new(),
        }
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        let mut placed = Vec::new();
        for _ in 0..3 {
            let frames = self.frames();
            if frames == placed {
                break;
            }
            let host = self.host;
            if let Some(host) = self.ui.widget_as::<FreeLayout>(host) {
                for (id, frame) in &frames {
                    host.set_frame(*id, *frame);
                }
            }
            self.ui.layout(Size::new(self.size.0, self.size.1));
            placed = frames;
        }
        let (controls, sheet) = self.ui.paint_layers(width, height);
        let palette = self.palette;
        let mut items = Vec::new();
        let mut y = TOOLBAR_HEIGHT;
        for label in rows(self.from.is_some(), self.bcc_shown) {
            let paragraph = self.ui.context().text.paragraph(
                &L10N.tr(label),
                TextStyle::role(TextRole::Body, palette.ink3),
                Some(LABEL_WIDTH - 8.0),
                TextAlign::End,
            );
            items.push(Item::Text {
                origin: Point::new(
                    MARGIN - 8.0,
                    y + ((FIELD_ROW - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            });
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    MARGIN,
                    y + FIELD_ROW - 1.0,
                    self.size.0 - 2.0 * MARGIN,
                    1.0,
                )),
                paint: palette.hairline.into(),
            });
            y += FIELD_ROW;
        }
        self.paint_attachments(&mut items, y);
        items.extend(controls);
        items.extend(sheet);
        Scene {
            width,
            height,
            background: palette.sheet,
            items,
        }
    }

    fn paint_attachments(&mut self, items: &mut Vec<Item>, y: f32) {
        self.chips.clear();
        let palette = self.palette;
        let mut x = MARGIN + LABEL_WIDTH;
        let chip_y = y + ((FIELD_ROW - CHIP_HEIGHT) / 2.0).round();
        for (index, attachment) in self.draft.attachments.iter().enumerate() {
            let label = format!(
                "{}  {}  ×",
                attachment.name,
                bande_fs::format_size(attachment.data.len() as u64)
            );
            let paragraph = self.ui.context().text.paragraph(
                &label,
                TextStyle::role(TextRole::Callout, palette.ink),
                None,
                TextAlign::Start,
            );
            let width = paragraph.width() + 2.0 * CHIP_PADDING;
            if x + width > self.size.0 - MARGIN {
                break;
            }
            let chip = Rect::new(x, chip_y, width, CHIP_HEIGHT);
            items.push(Item::Fill {
                shape: Shape::rounded(chip, Radius::S),
                paint: palette.well.into(),
            });
            items.push(Item::Text {
                origin: Point::new(
                    chip.x + CHIP_PADDING,
                    chip.y + ((CHIP_HEIGHT - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            });
            self.chips.push((chip, index));
            x += width + CHIP_GAP;
        }
    }

    pub fn token(&self) -> u64 {
        self.token
    }

    pub fn add_attachments(&mut self, attachments: Vec<Attachment>) {
        self.draft.attachments.extend(attachments);
        self.schedule_autosave();
    }

    pub fn attachments(&self) -> &[Attachment] {
        &self.draft.attachments
    }

    fn field_text(&self, id: WidgetId) -> String {
        self.ui
            .widget_ref::<TextField>(id)
            .map(TextField::text)
            .unwrap_or_default()
    }

    fn compose_draft(&self) -> Draft {
        Draft {
            from: self
                .senders
                .get(self.sender)
                .map(Account::address)
                .unwrap_or_else(|| self.draft.from.clone()),
            to: parse_addresses(&self.field_text(self.to)),
            cc: parse_addresses(&self.field_text(self.cc)),
            bcc: match self.bcc_shown {
                true => parse_addresses(&self.field_text(self.bcc)),
                false => self.draft.bcc.clone(),
            },
            subject: self.field_text(self.subject),
            body: self
                .ui
                .widget_ref::<TextView>(self.body)
                .map(TextView::text)
                .unwrap_or_default(),
            ..self.draft.clone()
        }
    }

    fn try_send(&mut self) {
        let draft = self.compose_draft();
        let typed = self.field_text(self.to);
        if draft.to.is_empty() && draft.cc.is_empty() {
            let message = match typed.trim().is_empty() {
                true => L10N.tr("compose-no-recipients"),
                false => L10N.tr_args("compose-bad-recipients", [("text", typed.into())]),
            };
            let signals = Rc::clone(&self.signals);
            self.ui.present_sheet(
                Alert::new(&L10N.tr("compose-cannot-send"), &message, move |_| {
                    signals.borrow_mut().push(Signal::Dismissed);
                })
                .primary(&L10N.tr("button-ok"), "ok"),
            );
            return;
        }
        self.sent = Some(self.outgoing(draft));
        self.closed = true;
    }

    pub fn show_error(&mut self, error: &str) {
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(
            Alert::new(&L10N.tr("compose-not-sent"), error, move |_| {
                signals.borrow_mut().push(Signal::Dismissed);
            })
            .primary(&L10N.tr("button-ok"), "ok"),
        );
    }

    fn change_sender(&mut self, index: usize) {
        let signature = |index: usize| {
            self.senders
                .get(index)
                .map(|account| account.signature.trim_end().to_owned())
                .unwrap_or_default()
        };
        let (old, new) = (signature(self.sender), signature(index));
        self.sender = index;
        let Some(body) = self.ui.widget_as::<TextView>(self.body) else {
            return;
        };
        let text = body.text();
        let swapped = match (old.is_empty(), text.find(&old)) {
            (false, Some(at)) => format!("{}{new}{}", &text[..at], &text[at + old.len()..]),
            _ => return,
        };
        body.set_text(&swapped);
    }

    fn after_ui(&mut self) {
        let signals = std::mem::take(&mut *self.signals.borrow_mut());
        for signal in signals {
            match signal {
                Signal::Send => self.try_send(),
                Signal::Attach => self.requests.push(ComposeRequest::ChooseFiles),
                Signal::From(index) => self.change_sender(index),
                Signal::Dismissed => {}
            }
        }
        if let Some(request) = self.ui.take_clipboard_request() {
            self.requests.push(ComposeRequest::Clipboard(request));
        }
        if let Some((menu, at)) = self.ui.take_menu_request() {
            self.requests.push(ComposeRequest::ShowMenu(menu, at));
        }
        self.schedule_autosave();
    }

    fn schedule_autosave(&mut self) {
        if self.autosave_at.is_none() && !self.closed {
            self.autosave_at = Some(Instant::now() + AUTOSAVE_DELAY);
        }
    }

    pub fn autosave(&mut self, now: Instant) {
        if self.autosave_at.is_none_or(|at| at > now) || self.closed {
            return;
        }
        self.autosave_at = None;
        if self.unsent.is_none() {
            return;
        }
        let draft = self.compose_draft();
        let unchanged = !self.recovered && self.autosaved.is_none() && draft == self.initial;
        if unchanged || self.autosaved.as_ref() == Some(&draft) {
            return;
        }
        match keep_unsent(&self.outgoing(draft.clone())) {
            Ok(()) => self.autosaved = Some(draft),
            Err(error) => tracing::warn!("cannot keep the unsent message: {error}"),
        }
    }

    pub fn discard_unsent(&self) {
        if !self.closed {
            crate::compose::forget_unsent(&self.outgoing(Draft::default()));
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if SHORTCUTS
            .iter()
            .any(|(action, shortcut)| *action == "send" && shortcut_pressed(event, shortcut))
            && !self.ui.has_sheet()
        {
            self.try_send();
            return true;
        }
        if let Event::PointerDown(pointer) = event
            && !self.ui.has_sheet()
            && let Some(index) = self
                .chips
                .iter()
                .find(|(chip, _)| chip.contains(pointer.position))
                .map(|(_, index)| *index)
        {
            self.draft.attachments.remove(index);
            self.schedule_autosave();
            return true;
        }
        let handled = self.ui.handle(event);
        self.after_ui();
        handled
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        if action == "send" {
            self.try_send();
            return true;
        }
        let handled = self.ui.menu_action(action);
        self.after_ui();
        handled
    }

    pub fn pasted(&mut self, text: &str) -> bool {
        let handled = self.ui.pasted(text);
        self.after_ui();
        handled
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let handled = self.ui.perform(request);
        self.after_ui();
        handled
    }

    pub fn menu_closed(&mut self) -> bool {
        self.ui.menu_closed()
    }

    fn outgoing(&self, draft: Draft) -> Outgoing {
        Outgoing {
            draft,
            answering: self.answering.clone(),
            replaces: self.replaces.clone(),
            unsent: self.unsent.clone(),
        }
    }

    pub fn take_sent(&mut self) -> Option<Outgoing> {
        self.sent.take()
    }

    pub fn unsent(&self) -> Option<Outgoing> {
        let draft = self.compose_draft();
        (self.sent.is_none() && !self.closed && (self.recovered || draft != self.initial))
            .then(|| self.outgoing(draft))
    }

    pub fn take_requests(&mut self) -> Vec<ComposeRequest> {
        std::mem::take(&mut self.requests)
    }

    pub fn finished(&self) -> bool {
        self.closed
    }

    pub fn cursor(&self) -> Cursor {
        self.ui.cursor()
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn deadline(&self) -> Option<Instant> {
        [self.ui.deadline(), self.autosave_at]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
