use std::cell::RefCell;
use std::rc::Rc;

use bande_toolkit::ui::{Alert, CheckState, Checkbox, PopUpButton, TextField, Ui};

use crate::L10N;
use crate::engine::{Restart, RunSpec, Task};
use crate::resources::{Act, Acts, Field, Fields, push};

type Spec = Rc<RefCell<RunSpec>>;

fn field(
    spec: &Spec,
    label: &str,
    placeholder: &str,
    text: &str,
    set: fn(&mut RunSpec, &str),
) -> Field {
    let spec = Rc::clone(spec);
    Field::new(
        &L10N.tr(label),
        TextField::new(&L10N.tr(placeholder), text, move |text| {
            set(&mut spec.borrow_mut(), text);
        }),
    )
}

pub fn sheet(ui: &mut Ui, acts: &Acts, image: &str) {
    let spec: Spec = Rc::new(RefCell::new(RunSpec {
        image: image.to_owned(),
        name: String::new(),
        ports: String::new(),
        env: String::new(),
        volumes: String::new(),
        restart: Restart::No,
        remove: false,
    }));
    let restarts: Vec<String> = Restart::ALL
        .iter()
        .map(|restart| {
            L10N.tr(match restart {
                Restart::No => "restart-no",
                Restart::Always => "restart-always",
                Restart::UnlessStopped => "restart-unless-stopped",
                Restart::OnFailure => "restart-on-failure",
            })
        })
        .collect();
    let chosen = Rc::clone(&spec);
    let restart = PopUpButton::new(&L10N.tr("run-restart"), &restarts, 0, move |index| {
        if let Some(restart) = Restart::ALL.get(index) {
            chosen.borrow_mut().restart = *restart;
        }
    });
    let removed = Rc::clone(&spec);
    let remove = Checkbox::new(&L10N.tr("run-remove"), CheckState::Off, move |on| {
        removed.borrow_mut().remove = on;
    });
    let fields = Fields::new(vec![
        field(
            &spec,
            "run-image-label",
            "run-image-placeholder",
            image,
            |spec, text| {
                text.trim().clone_into(&mut spec.image);
            },
        ),
        field(
            &spec,
            "run-name",
            "run-name-placeholder",
            "",
            |spec, text| {
                text.clone_into(&mut spec.name);
            },
        ),
        field(
            &spec,
            "run-ports",
            "run-ports-placeholder",
            "",
            |spec, text| {
                text.clone_into(&mut spec.ports);
            },
        ),
        field(&spec, "run-env", "run-env-placeholder", "", |spec, text| {
            text.clone_into(&mut spec.env);
        }),
        field(
            &spec,
            "run-volumes",
            "run-volumes-placeholder",
            "",
            |spec, text| {
                text.clone_into(&mut spec.volumes);
            },
        ),
        Field::new(&L10N.tr("run-restart"), restart),
        Field::new("", remove),
    ]);
    let acts = Rc::clone(acts);
    let alert = Alert::new(
        &L10N.tr("run-title"),
        &L10N.tr("run-message"),
        move |choice| {
            let spec = spec.borrow().clone();
            if choice == "run" && !spec.image.is_empty() {
                push(&acts, Act::Task(Task::Run(spec)));
            }
        },
    )
    .accessory(fields)
    .primary(&L10N.tr("run"), "run")
    .cancel(&L10N.tr("button-cancel"), "cancel");
    ui.present_sheet(alert);
}
