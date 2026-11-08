use std::cell::RefCell;
use std::rc::Rc;


use bande_l10n::tr;

use crate::discover::Settings;
use crate::net::{Security, Server, is_loopback};
use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Button, Context, SPINNER_SIZE, SecureField, Size, Spinner, TextField, Widget, WidgetId,
};
use bande_toolkit::{
    ControlSize, Image, Item, Paragraph, Point, Rect, Shape, TextAlign, TextRole, TextStyle,
};
use zeroize::Zeroizing;

const WIDTH: f32 = 480.0;
const PADDING: f32 = 24.0;
const ICON_SIZE: f32 = 48.0;
const ICON_GAP: f32 = 16.0;
const TITLE_GAP: f32 = 6.0;
const LABEL_WIDTH: f32 = 128.0;
const LABEL_GAP: f32 = 10.0;
const ROW_GAP: f32 = 10.0;
const PORT_WIDTH: f32 = 64.0;
const SECTION_GAP: f32 = 18.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 96.0;
const STATUS_GAP: f32 = 8.0;
const FIELD_FONT: f32 = 13.0;
const ICON: &[u8] = include_bytes!("../../data/org.bande.Mail.svg");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Email,
    Password,
    Incoming,
    IncomingPort,
    Outgoing,
    OutgoingPort,
    User,
}

impl Field {
    fn label(self) -> String {
        match self {
            Self::Name => tr!("Name:"),
            Self::Email => tr!("Email Address:"),
            Self::Password => tr!("Password:"),
            Self::Incoming => tr!("Incoming (IMAP):"),
            Self::IncomingPort | Self::OutgoingPort => tr!("Port:"),
            Self::Outgoing => tr!("Outgoing (SMTP):"),
            Self::User => tr!("User Name:"),
        }
    }

    fn placeholder(self) -> String {
        match self {
            Self::Name => tr!("Ann Lee"),
            Self::Email => tr!("name@example.com"),
            Self::Password => tr!("Required"),
            Self::Incoming => tr!("imap.example.com"),
            Self::Outgoing => tr!("smtp.example.com"),
            Self::IncomingPort | Self::OutgoingPort => tr!("Port"),
            Self::User => tr!("Same as the email address"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Add,
    Manual,
    Password(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SignIn,
    Cancel,
}

#[derive(Default)]
pub struct SheetState {
    pub action: Option<Action>,
    pub password: Zeroizing<String>,
    pub values: Vec<(Field, String)>,
    pub status: Option<(String, bool)>,
    pub busy: bool,
    pub closed: bool,
}

impl SheetState {
    pub fn value(&self, field: Field) -> String {
        self.values
            .iter()
            .find(|(known, _)| *known == field)
            .map(|(_, value)| value.trim().to_owned())
            .unwrap_or_default()
    }

    fn set(&mut self, field: Field, value: &str) {
        match self.values.iter_mut().find(|(known, _)| *known == field) {
            Some((_, known)) => value.clone_into(known),
            None => self.values.push((field, value.to_owned())),
        }
    }

    pub fn servers(&self) -> Option<Settings> {
        let server = |host: Field, port: Field, secure: u16| {
            let host = self.value(host).to_ascii_lowercase();
            let port = self.value(port).parse().unwrap_or(secure);
            let security = match port {
                143 | 587 | 25 => Security::StartTls,
                _ if is_loopback(&host) && port != secure => Security::Plain,
                _ => Security::Tls,
            };
            (!host.is_empty()).then_some(Server {
                host,
                port,
                security,
            })
        };
        let user = match self.value(Field::User) {
            user if user.is_empty() => self.value(Field::Email),
            user => user,
        };
        Some(Settings {
            imap: server(Field::Incoming, Field::IncomingPort, 993)?,
            smtp: server(Field::Outgoing, Field::OutgoingPort, 465)?,
            user,
        })
    }
}

pub type Shared = Rc<RefCell<SheetState>>;

pub struct AccountSheet {
    id: WidgetId,
    state: Shared,
    fields: Vec<Field>,
    children: Vec<Box<dyn Widget>>,
    password_field: WidgetId,
    mode: Mode,
    icon: Option<Image>,
    title: Option<Paragraph>,
    message: Option<Paragraph>,
    labels: Vec<Paragraph>,
    status: Option<Paragraph>,
    row_height: f32,
    button_sizes: [Size; 2],
    frame: Rect,
}

fn text_field(state: &Shared, field: Field) -> TextField {
    let shared = Rc::clone(state);
    let value = state.borrow().value(field);
    TextField::new(&field.placeholder(), &value, move |text| {
        shared.borrow_mut().set(field, text);
    })
    .without_text_checking()
}

impl AccountSheet {
    pub fn new(state: Shared, mode: Mode) -> Self {
        let mut fields = match mode {
            Mode::Password(_) => vec![Field::Password],
            _ => vec![Field::Name, Field::Email, Field::Password],
        };
        if mode == Mode::Manual {
            fields.extend([
                Field::Incoming,
                Field::IncomingPort,
                Field::Outgoing,
                Field::OutgoingPort,
                Field::User,
            ]);
        }
        let mut children: Vec<Box<dyn Widget>> = Vec::new();
        let mut password_field = WidgetId::next();
        for field in &fields {
            if *field == Field::Password {
                let shared = Rc::clone(&state);
                let secure = SecureField::new(&field.placeholder(), FIELD_FONT, move |secret| {
                    let mut state = shared.borrow_mut();
                    if !secret.is_empty() {
                        state.password = Zeroizing::new(secret.to_owned());
                    }
                    state.action = Some(Action::SignIn);
                });
                password_field = secure.id();
                children.push(Box::new(secure));
            } else {
                children.push(Box::new(text_field(&state, *field)));
            }
        }
        let cancel = Rc::clone(&state);
        children.push(Box::new(
            Button::new(&tr!("Cancel"), move || {
                cancel.borrow_mut().action = Some(Action::Cancel);
            })
            .cancel(),
        ));
        let sign_in = Rc::clone(&state);
        children.push(Box::new(
            Button::new(&tr!("Sign In"), move || {
                sign_in.borrow_mut().action = Some(Action::SignIn);
            })
            .primary(),
        ));
        children.push(Box::new(Spinner::new(&tr!("Checking the account…"))));
        Self {
            id: WidgetId::next(),
            state,
            fields,
            children,
            password_field,
            mode,
            icon: Image::from_svg(ICON, 96, 96),
            title: None,
            message: None,
            labels: Vec::new(),
            status: None,
            row_height: 0.0,
            button_sizes: [Size::default(); 2],
            frame: Rect::default(),
        }
    }

    pub fn password_field(&self) -> WidgetId {
        self.password_field
    }

    fn text_x(&self) -> f32 {
        self.frame.x + PADDING + ICON_SIZE + ICON_GAP
    }

    fn rows(&self) -> Vec<Vec<usize>> {
        let mut rows = Vec::new();
        let mut index = 0;
        while index < self.fields.len() {
            let paired = matches!(self.fields[index], Field::Incoming | Field::Outgoing)
                && index + 1 < self.fields.len();
            match paired {
                true => {
                    rows.push(vec![index, index + 1]);
                    index += 2;
                }
                false => {
                    rows.push(vec![index]);
                    index += 1;
                }
            }
        }
        rows
    }

    fn header_height(&self) -> f32 {
        let text = self.title.as_ref().map_or(0.0, Paragraph::height)
            + self
                .message
                .as_ref()
                .map_or(0.0, |message| TITLE_GAP + message.height());
        text.max(ICON_SIZE)
    }

    fn fields_height(&self) -> f32 {
        let rows = self.rows().len() as f32;
        rows * self.row_height + (rows - 1.0).max(0.0) * ROW_GAP
    }
}

impl Widget for AccountSheet {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = WIDTH.min(available.width);
        let text_width = width - (PADDING + ICON_SIZE + ICON_GAP) - PADDING;
        let (title, message) = match &self.mode {
            Mode::Add => (
                tr!("Add a Mail Account"),
                tr!("Enter your name, email address and password. Mail finds the servers for you."),
            ),
            Mode::Manual => (
                tr!("Add a Mail Account"),
                tr!("Mail could not sign in by itself. Check the server settings."),
            ),
            Mode::Password(email) => (
                tr!("Enter Your Password"),
                tr!(
                    "The server did not accept the password for {email}.",
                    email = email.clone()
                ),
            ),
        };
        self.title = Some(cx.text.paragraph(
            &title,
            TextStyle::role(TextRole::Heading, cx.palette.ink),
            Some(text_width),
            TextAlign::Start,
        ));
        self.message = Some(cx.text.paragraph(
            &message,
            TextStyle::role(TextRole::Body, cx.palette.ink2),
            Some(text_width),
            TextAlign::Start,
        ));
        self.labels = self
            .fields
            .iter()
            .map(|field| {
                cx.text.paragraph(
                    &field.label(),
                    TextStyle::role(TextRole::Body, cx.palette.ink2),
                    Some(LABEL_WIDTH),
                    TextAlign::End,
                )
            })
            .collect();
        let (status, error) = self
            .state
            .borrow()
            .status
            .clone()
            .map_or((None, false), |(text, error)| (Some(text), error));
        let color = match error {
            true => cx.palette.danger_text,
            false => cx.palette.ink2,
        };
        self.status = status.map(|status| {
            cx.text.paragraph(
                &status,
                TextStyle::role(TextRole::Callout, color),
                Some(width - 2.0 * PADDING - SPINNER_SIZE - STATUS_GAP),
                TextAlign::Start,
            )
        });
        self.row_height = cx.control_height(ControlSize::Regular);
        let fields = self.fields.len();
        for (index, child) in self.children.iter_mut().enumerate() {
            let size = child.measure(cx, Size::new(width, self.row_height));
            if let Some(slot) = index.checked_sub(fields).filter(|slot| *slot < 2) {
                self.button_sizes[slot] =
                    Size::new(size.width.max(BUTTON_MIN_WIDTH), self.row_height);
            }
        }
        let status = self
            .status
            .as_ref()
            .map_or(SPINNER_SIZE, |status| status.height().max(SPINNER_SIZE));
        let height = PADDING
            + self.header_height()
            + SECTION_GAP
            + self.fields_height()
            + SECTION_GAP
            + status
            + SECTION_GAP
            + self.row_height
            + PADDING;
        Size::new(width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let field_x = frame.x + PADDING + LABEL_WIDTH + LABEL_GAP;
        let field_width = frame.x + frame.width - PADDING - field_x;
        let mut y = frame.y + PADDING + self.header_height() + SECTION_GAP;
        for row in self.rows() {
            match row.as_slice() {
                [host, port] => {
                    let host_width = field_width - PORT_WIDTH - LABEL_GAP;
                    self.children[*host]
                        .place(cx, Rect::new(field_x, y, host_width, self.row_height));
                    self.children[*port].place(
                        cx,
                        Rect::new(
                            field_x + host_width + LABEL_GAP,
                            y,
                            PORT_WIDTH,
                            self.row_height,
                        ),
                    );
                }
                [single] => {
                    self.children[*single]
                        .place(cx, Rect::new(field_x, y, field_width, self.row_height));
                }
                _ => {}
            }
            y += self.row_height + ROW_GAP;
        }
        let fields = self.fields.len();
        let buttons_y = frame.y + frame.height - PADDING - self.row_height;
        let sign_in = self.button_sizes[1];
        let cancel = self.button_sizes[0];
        let right = frame.x + frame.width - PADDING;
        self.children[fields + 1].place(
            cx,
            Rect::new(
                right - sign_in.width,
                buttons_y,
                sign_in.width,
                sign_in.height,
            ),
        );
        self.children[fields].place(
            cx,
            Rect::new(
                right - sign_in.width - BUTTON_SPACING - cancel.width,
                buttons_y,
                cancel.width,
                cancel.height,
            ),
        );
        let status_y = buttons_y - SECTION_GAP - SPINNER_SIZE;
        self.children[fields + 2].place(
            cx,
            Rect::new(frame.x + PADDING, status_y, SPINNER_SIZE, SPINNER_SIZE),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: Rect::new(
                    self.frame.x + PADDING,
                    self.frame.y + PADDING,
                    ICON_SIZE,
                    ICON_SIZE,
                ),
                image: icon.clone(),
            });
        }
        let x = self.text_x();
        let mut y = self.frame.y + PADDING;
        if let Some(title) = &self.title {
            items.push(Item::Text {
                origin: Point::new(x, y),
                paragraph: title.clone(),
            });
            y += title.height() + TITLE_GAP;
        }
        if let Some(message) = &self.message {
            items.push(Item::Text {
                origin: Point::new(x, y),
                paragraph: message.clone(),
            });
        }
        let mut y = self.frame.y + PADDING + self.header_height() + SECTION_GAP;
        for row in self.rows() {
            if let Some(label) = row.first().and_then(|index| self.labels.get(*index)) {
                items.push(Item::Text {
                    origin: Point::new(
                        self.frame.x + PADDING,
                        y + ((self.row_height - label.height()) / 2.0).round(),
                    ),
                    paragraph: label.clone(),
                });
            }
            y += self.row_height + ROW_GAP;
        }
        let fields = self.fields.len();
        let busy = self.state.borrow().busy;
        for (index, child) in self.children.iter().enumerate() {
            if index == fields + 2 && !busy {
                continue;
            }
            child.paint(cx, items);
        }
        if let Some(status) = &self.status {
            let spinner = self.children[fields + 2].frame();
            let x = match busy {
                true => spinner.x + SPINNER_SIZE + STATUS_GAP,
                false => spinner.x,
            };
            items.push(Item::Clip {
                shape: Shape::rect(Rect::new(
                    x,
                    spinner.y - status.height(),
                    self.frame.width,
                    status.height() * 3.0,
                )),
                items: vec![Item::Text {
                    origin: Point::new(
                        x,
                        spinner.y + ((SPINNER_SIZE - status.height()) / 2.0).round(),
                    ),
                    paragraph: status.clone(),
                }],
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(tr!("Add a Mail Account"));
        node.set_modal();
        node
    }

    fn dismissed(&self) -> bool {
        self.state.borrow().closed
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
