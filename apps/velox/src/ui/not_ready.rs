use std::rc::Rc;

use bande_toolkit::ui::{Axis, Button, Disclosure, Label, ScrollView, Stack, Widget};

use super::widgets::{Centered, Code, Ends};
use super::{Intent, Intents, boxed, push};
use crate::L10N;
use crate::readiness::Readiness;

const PAGE_WIDTH: f32 = 560.0;
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;

fn steps(title: &str, commands: &'static str, intents: &Intents) -> Box<dyn Widget> {
    let copy = Rc::clone(intents);
    let title = boxed(Label::new(title));
    let button = boxed(Button::new(&L10N.tr("copy"), move || {
        push(&copy, Intent::Copy(commands));
    }));
    boxed(Stack::new(
        Axis::Vertical,
        6.0,
        0.0,
        vec![boxed(Ends::new(title, button)), boxed(Code::new(commands))],
    ))
}

pub(super) fn page(readiness: &Readiness, intents: &Intents, alternative: bool) -> Box<dyn Widget> {
    let advice = readiness.advice();
    let mut children = vec![boxed(Label::heading(&advice.headline))];
    if let Some(shown) = &advice.steps {
        children.push(steps(&shown.title, shown.commands, intents));
    }
    if let Some(note) = &advice.note {
        children.push(boxed(Label::new(note).secondary()));
    }
    if let Some(link) = advice.link {
        let open = Rc::clone(intents);
        let shown = link.trim_start_matches("https://").trim_end_matches('/');
        children.push(boxed(Stack::new(
            Axis::Horizontal,
            0.0,
            0.0,
            vec![boxed(
                Button::new(shown, move || push(&open, Intent::Open(link))).link(),
            )],
        )));
    }
    if let Some(other) = &advice.alternative {
        let toggled = Rc::clone(intents);
        children.push(boxed(Disclosure::new(
            &other.title,
            alternative,
            Stack::new(
                Axis::Vertical,
                0.0,
                0.0,
                vec![steps(&L10N.tr("podman-rootless"), other.commands, intents)],
            ),
            move |open| push(&toggled, Intent::Alternative(open)),
        )));
    }
    if let Readiness::Unreachable { .. } = readiness {
        let retry = Rc::clone(intents);
        let local = Rc::clone(intents);
        children.push(boxed(Stack::new(
            Axis::Horizontal,
            8.0,
            0.0,
            vec![
                boxed(Button::new(&L10N.tr("host-retry"), move || {
                    push(&retry, Intent::Reconnect);
                })),
                boxed(Button::new(&L10N.tr("host-show-local"), move || {
                    push(&local, Intent::ShowLocal);
                })),
            ],
        )));
    }
    boxed(ScrollView::new(Centered::new(
        boxed(Stack::new(Axis::Vertical, SPACING, PADDING, children)),
        PAGE_WIDTH,
    )))
}
