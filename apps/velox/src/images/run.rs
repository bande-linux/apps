use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::{Text, text, tr};

use crate::engine::{Restart, RunSpec, Task};
use crate::resources::{Act, Acts, Field, Fields, push};
use bande_toolkit::ui::{Alert, CheckState, Checkbox, PopUpButton, TextField, Ui};

type Spec = Rc<RefCell<RunSpec>>;

fn field(
    spec: &Spec,
    label: Text,
    placeholder: Text,
    text: &str,
    set: fn(&mut RunSpec, &str),
) -> Field {
    let spec = Rc::clone(spec);
    Field::new(
        &label.tr(),
        TextField::new(&placeholder.tr(), text, move |text| {
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
        .map(|restart| match restart {
            Restart::No => tr!("Never"),
            Restart::Always => tr!("Always"),
            Restart::UnlessStopped => tr!("Unless stopped"),
            Restart::OnFailure => tr!("On failure"),
        })
        .collect();
    let chosen = Rc::clone(&spec);
    let restart = PopUpButton::new(
        &tr!(context = "restart policy", "Restart"),
        &restarts,
        0,
        move |index| {
            if let Some(restart) = Restart::ALL.get(index) {
                chosen.borrow_mut().restart = *restart;
            }
        },
    );
    let removed = Rc::clone(&spec);
    let remove = Checkbox::new(
        &tr!("Remove when it exits (--rm)"),
        CheckState::Off,
        move |on| {
            removed.borrow_mut().remove = on;
        },
    );
    let fields = Fields::new(vec![
        field(
            &spec,
            text!("Image"),
            text!("nginx:latest"),
            image,
            |spec, text| {
                text.trim().clone_into(&mut spec.image);
            },
        ),
        field(&spec, text!("Name"), text!("Optional"), "", |spec, text| {
            text.clone_into(&mut spec.name);
        }),
        field(
            &spec,
            text!("Ports"),
            text!("8080:80, 5432:5432"),
            "",
            |spec, text| {
                text.clone_into(&mut spec.ports);
            },
        ),
        field(
            &spec,
            text!("Environment"),
            text!("KEY=value, OTHER=value"),
            "",
            |spec, text| {
                text.clone_into(&mut spec.env);
            },
        ),
        field(
            &spec,
            text!("Volumes"),
            text!("data:/var/lib/data"),
            "",
            |spec, text| {
                text.clone_into(&mut spec.volumes);
            },
        ),
        Field::new(&tr!(context = "restart policy", "Restart"), restart),
        Field::new("", remove),
    ]);
    let acts = Rc::clone(acts);
    let alert = Alert::new(
        &tr!("Run Image"),
        &tr!("Starts detached. The new container appears in Containers."),
        move |choice| {
            let spec = spec.borrow().clone();
            if choice == "run" && !spec.image.is_empty() {
                push(&acts, Act::Task(Task::Run(spec)));
            }
        },
    )
    .accessory(fields)
    .primary(&tr!("Run"), "run")
    .cancel(&tr!("Cancel"), "cancel");
    ui.present_sheet(alert);
}
