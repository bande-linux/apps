use std::rc::Rc;

use bande_toolkit::ui::{Axis, Button, Label, ScrollView, Stack, Widget};

use super::readiness::{Blocker, INSTALL, KVM_GROUP};
use super::{Intent, Intents, push};
use crate::L10N;
use crate::ui::{Centered, Code, Ends};

const PAGE_WIDTH: f32 = 560.0;
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;
const EMPTY_WIDTH: f32 = 360.0;

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn steps(title: &str, commands: &'static str, intents: &Intents) -> Box<dyn Widget> {
    let copy = Rc::clone(intents);
    let button = boxed(Button::new(&L10N.tr("copy"), move || {
        push(&copy, Intent::Copy(commands));
    }));
    boxed(Stack::new(
        Axis::Vertical,
        6.0,
        0.0,
        vec![
            boxed(Ends::new(boxed(Label::new(title)), button)),
            boxed(Code::new(commands)),
        ],
    ))
}

fn emulate(intents: &Intents) -> Box<dyn Widget> {
    let emulate = Rc::clone(intents);
    boxed(Stack::new(
        Axis::Vertical,
        6.0,
        0.0,
        vec![
            boxed(Label::new(&L10N.tr("machine-emulation-note")).secondary()),
            boxed(Stack::new(
                Axis::Horizontal,
                0.0,
                0.0,
                vec![boxed(Button::new(
                    &L10N.tr("machine-use-emulation"),
                    move || push(&emulate, Intent::Emulate),
                ))],
            )),
        ],
    ))
}

pub fn not_ready(blocker: &Blocker, intents: &Intents) -> Box<dyn Widget> {
    let mut children = Vec::new();
    match blocker {
        Blocker::Missing(packages) => {
            children.push(boxed(Label::heading(&L10N.tr("machine-install"))));
            children.push(boxed(
                Label::new(&L10N.tr_args(
                    "machine-missing",
                    [("packages", packages.join(", ").into())],
                ))
                .secondary(),
            ));
            children.push(steps(&L10N.tr("machine-install-steps"), INSTALL, intents));
        }
        Blocker::NoKvm => {
            children.push(boxed(Label::heading(&L10N.tr("machine-no-kvm"))));
            children.push(boxed(
                Label::new(&L10N.tr("machine-no-kvm-why")).secondary(),
            ));
            children.push(steps(&L10N.tr("machine-kvm-steps"), KVM_GROUP, intents));
            children.push(emulate(intents));
        }
        Blocker::Relogin => {
            children.push(boxed(Label::heading(&L10N.tr("relogin"))));
            children.push(boxed(
                Label::new(&L10N.tr("machine-relogin-why")).secondary(),
            ));
            children.push(emulate(intents));
        }
        Blocker::NoManager => {
            children.push(boxed(Label::heading(&L10N.tr("machine-no-manager"))));
            children.push(boxed(
                Label::new(&L10N.tr("machine-no-manager-why")).secondary(),
            ));
        }
    }
    boxed(ScrollView::new(Centered::new(
        boxed(Stack::new(Axis::Vertical, SPACING, PADDING, children)),
        PAGE_WIDTH,
    )))
}

pub fn empty(intents: &Intents) -> Box<dyn Widget> {
    let new = Rc::clone(intents);
    boxed(Centered::new(
        boxed(Stack::new(
            Axis::Vertical,
            6.0,
            0.0,
            vec![
                boxed(Label::new(&L10N.tr("machine-empty-title"))),
                boxed(Label::new(&L10N.tr("machine-empty-hint")).secondary()),
                boxed(Stack::new(
                    Axis::Horizontal,
                    0.0,
                    0.0,
                    vec![boxed(Button::new(&L10N.tr("machine-new"), move || {
                        push(&new, Intent::New)
                    }))],
                )),
            ],
        )),
        EMPTY_WIDTH,
    ))
}

pub fn blank() -> Box<dyn Widget> {
    boxed(Label::new(""))
}
