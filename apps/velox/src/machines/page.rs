use std::rc::Rc;

use bande_l10n::tr;
use bande_toolkit::ui::{Axis, Button, Label, ScrollView, Stack, Widget};

use super::readiness::{Blocker, INSTALL, KVM_GROUP};
use super::{Intent, Intents, push};
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
    let button = boxed(Button::new(&tr!("Copy"), move || {
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
            boxed(
                Label::new(&tr!("Without KVM, machines run emulated and much slower.")).secondary(),
            ),
            boxed(Stack::new(
                Axis::Horizontal,
                0.0,
                0.0,
                vec![boxed(Button::new(&tr!("Use Emulation"), move || {
                    push(&emulate, Intent::Emulate)
                }))],
            )),
        ],
    ))
}

pub fn not_ready(blocker: &Blocker, intents: &Intents) -> Box<dyn Widget> {
    let mut children = Vec::new();
    match blocker {
        Blocker::Missing(packages) => {
            children.push(boxed(Label::heading(&tr!(
                "Install virtualization to continue"
            ))));
            children.push(boxed(
                Label::new(&tr!("Missing: {packages}", packages = packages.join(", "))).secondary(),
            ));
            children.push(steps(
                &tr!("Install QEMU, its firmware and passt"),
                INSTALL,
                intents,
            ));
        }
        Blocker::NoKvm => {
            children.push(boxed(Label::heading(&tr!("No access to KVM"))));
            children.push(boxed(
                Label::new(&tr!("Virtual machines run on /dev/kvm. It is open to the person signed in at this computer; over SSH you need the kvm group.")).secondary(),
            ));
            children.push(steps(
                &tr!("Add yourself to the kvm group, then log out and back in"),
                KVM_GROUP,
                intents,
            ));
            children.push(emulate(intents));
        }
        Blocker::Relogin => {
            children.push(boxed(Label::heading(&tr!("Log out and back in to finish"))));
            children.push(boxed(
                Label::new(&tr!("You joined the kvm group after your session started. Your services keep running after you log out, so restart the computer if logging out isn’t enough.")).secondary(),
            ));
            children.push(emulate(intents));
        }
        Blocker::NoManager => {
            children.push(boxed(Label::heading(&tr!("No systemd user manager"))));
            children.push(boxed(
                Label::new(&tr!("Virtual machines run as systemd user services, and the session has no user manager.")).secondary(),
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
                boxed(Label::new(&tr!("No Virtual Machines"))),
                boxed(
                    Label::new(&tr!(
                        "Make one from a Debian, Ubuntu or Fedora cloud image."
                    ))
                    .secondary(),
                ),
                boxed(Stack::new(
                    Axis::Horizontal,
                    0.0,
                    0.0,
                    vec![boxed(Button::new(
                        &tr!("New Virtual Machine…"),
                        move || push(&new, Intent::New),
                    ))],
                )),
            ],
        )),
        EMPTY_WIDTH,
    ))
}

pub fn blank() -> Box<dyn Widget> {
    boxed(Label::new(""))
}
