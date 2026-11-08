use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::tr;
use bande_toolkit::ui::{
    Alert, Axis, Button, Context, Event, Key, Label, Modifiers, ProgressBar, ScrollView,
    SecureField, Stack, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, TextSystem};

pub const PANEL_SIZE: (u32, u32) = (460, 240);
pub const PROMPT_SIZE: (u32, u32) = (420, 200);
const SPACING: f32 = 10.0;
const PADDING: f32 = 20.0;
const FIELD_FONT: f32 = 13.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Cancel(u64),
    Dismiss,
    Password(Option<String>),
    Submit,
}

pub struct Row {
    pub id: u64,
    pub title: String,
    pub detail: String,
    pub fraction: Option<f32>,
}

type Intents = Rc<RefCell<Vec<Intent>>>;

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn push(intents: &Intents, intent: Intent) -> impl FnMut() + 'static {
    let intents = intents.clone();
    move || intents.borrow_mut().push(intent.clone())
}

fn page(widgets: Vec<Box<dyn Widget>>) -> Box<dyn Widget> {
    boxed(Stack::new(Axis::Vertical, SPACING, PADDING, widgets))
}

#[derive(Clone, Copy)]
struct Shown {
    id: u64,
    bar: WidgetId,
    detail: WidgetId,
}

pub struct Panel {
    pub ui: Ui,
    intents: Intents,
    content: WidgetId,
    field: Option<WidgetId>,
    shown: Vec<Shown>,
}

impl Panel {
    pub fn progress(appearance: Appearance) -> Self {
        let content = Stack::new(Axis::Vertical, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let id = content.id();
        let mut ui = Ui::new(
            Box::new(ScrollView::new(content)),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        Self {
            ui,
            intents: Rc::default(),
            content: id,
            field: None,
            shown: Vec::new(),
        }
    }

    pub fn password(appearance: Appearance, name: &str, wrong: bool) -> Self {
        let mut panel = Self::progress(appearance);
        let intents = panel.intents.clone();
        let field = SecureField::new(&tr!("Password"), FIELD_FONT, move |secret| {
            intents
                .borrow_mut()
                .push(Intent::Password(Some(secret.to_owned())));
        });
        panel.field = Some(field.id());
        let mut widgets = vec![boxed(Label::heading(&tr!(
            "Enter a password to expand “{name}”",
            name = name.to_owned()
        )))];
        if wrong {
            widgets.push(boxed(
                Label::new(&tr!("The password is incorrect. Try again.")).secondary(),
            ));
        }
        widgets.push(boxed(field));
        widgets.push(boxed(
            Stack::new(
                Axis::Horizontal,
                SPACING,
                0.0,
                vec![
                    boxed(
                        Button::new(&tr!("Cancel"), push(&panel.intents, Intent::Password(None)))
                            .cancel(),
                    ),
                    boxed(
                        Button::new(&tr!("Expand"), push(&panel.intents, Intent::Submit)).primary(),
                    ),
                ],
            )
            .trailing(),
        ));
        panel.set_page(page(widgets));
        if let Some(field) = panel.field {
            panel.ui.set_focus(field);
        }
        panel
    }

    fn set_page(&mut self, page: Box<dyn Widget>) {
        self.replace(self.content, page);
    }

    fn replace(&mut self, slot: WidgetId, widget: Box<dyn Widget>) {
        if let Some(content) = self.ui.widget(slot)
            && let Some(child) = content.children_mut().first_mut()
        {
            *child = widget;
        }
    }

    fn detail(row: &Row) -> Box<dyn Widget> {
        boxed(Label::new(&row.detail).secondary())
    }

    pub fn show(&mut self, rows: &[Row]) {
        let same = rows.len() == self.shown.len()
            && rows
                .iter()
                .zip(&self.shown)
                .all(|(row, shown)| row.id == shown.id);
        if same && !rows.is_empty() {
            for (row, shown) in rows.iter().zip(self.shown.clone()) {
                self.ui.update_widget::<ProgressBar>(shown.bar, |bar, cx| {
                    bar.set_value(cx, row.fraction);
                });
                self.replace(shown.detail, Self::detail(row));
            }
            return;
        }
        self.shown.clear();
        let widgets = match rows {
            [] => vec![boxed(Label::new(&tr!("Archives you open and items you compress from Files appear here while they are worked on.")).secondary())],
            rows => rows.iter().map(|row| self.row(row)).collect(),
        };
        self.set_page(page(widgets));
    }

    fn row(&mut self, row: &Row) -> Box<dyn Widget> {
        let stop = Button::new(&tr!("Stop"), push(&self.intents, Intent::Cancel(row.id)));
        let bar = ProgressBar::new(&row.title, row.fraction);
        let detail = Stack::new(Axis::Vertical, 0.0, 0.0, vec![Self::detail(row)]);
        self.shown.push(Shown {
            id: row.id,
            bar: bar.id(),
            detail: detail.id(),
        });
        let line = Stack::new(
            Axis::Horizontal,
            SPACING,
            0.0,
            vec![boxed(bar), boxed(stop)],
        );
        boxed(Stack::new(
            Axis::Vertical,
            4.0,
            0.0,
            vec![
                boxed(Label::new(&row.title)),
                boxed(line.trailing()),
                boxed(detail),
            ],
        ))
    }

    pub fn alert(&mut self, title: &str, message: &str) {
        let intents = self.intents.clone();
        let alert = Alert::new(title, message, move |_| {
            intents.borrow_mut().push(Intent::Dismiss);
        })
        .primary(&tr!("OK"), "ok");
        self.ui.present_dialog(alert);
    }

    pub fn alerting(&self) -> bool {
        self.ui.has_sheet()
    }

    pub fn take_intents(&mut self) -> Vec<Intent> {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if !intents.contains(&Intent::Submit) {
            return intents;
        }
        if let Some(field) = self.field {
            self.ui.set_focus(field);
            self.ui.handle(&Event::Key {
                key: Key::Enter,
                modifiers: Modifiers::default(),
            });
        }
        let mut submitted: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        submitted.extend(
            intents
                .into_iter()
                .filter(|intent| *intent != Intent::Submit),
        );
        submitted
    }
}
