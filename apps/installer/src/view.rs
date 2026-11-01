use std::cell::RefCell;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use bande_fs::format_size;
use bande_packages::{Canceller, Client, Progress, Status};
use bande_toolkit::ui::{
    Axis, Button, Context, Disclosure, Label, ProgressBar, ScrollView, Spinner, Stack, Ui, Widget,
    WidgetId,
};
use bande_toolkit::{Palette, TextSystem};

use crate::L10N;
use crate::deb::{Deb, Machine};
use crate::flow::{Action, Preview, Problem, State};
use crate::worker::{self, Feed, Job, Message};

pub const WINDOW_SIZE: (u32, u32) = (520, 480);
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Install,
    Cancel,
    AskRemove,
    Remove,
    Back,
    Open(usize),
    Close,
}

type Widgets = Vec<Box<dyn Widget>>;

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn arch_name(arch: &str) -> String {
    match arch {
        "amd64" => L10N.tr("arch-amd64"),
        "arm64" => L10N.tr("arch-arm64"),
        "armhf" => L10N.tr("arch-armhf"),
        "i386" => L10N.tr("arch-i386"),
        other => other.to_owned(),
    }
}

fn status_line(status: Status) -> String {
    match status {
        Status::Waiting => L10N.tr("status-waiting"),
        Status::Downloading => L10N.tr("status-downloading"),
        Status::Installing => L10N.tr("status-installing"),
        Status::Removing => L10N.tr("status-removing"),
        Status::Working => L10N.tr("status-working"),
    }
}

fn row(children: Widgets) -> Box<dyn Widget> {
    boxed(Stack::new(Axis::Horizontal, SPACING, 0.0, children))
}

fn disclosure(title: &str, text: &str) -> Box<dyn Widget> {
    boxed(Disclosure::new(title, false, Label::new(text), |_| {}))
}

pub struct Installer {
    deb: Option<Deb>,
    state: State,
    ready: Option<State>,
    jobs: Sender<Job>,
    feed: Feed,
    canceller: Canceller,
    intents: Rc<RefCell<Vec<Intent>>>,
    ui: Ui,
    content: WidgetId,
    closed: bool,
}

impl Installer {
    pub fn new(
        client: Client,
        path: PathBuf,
        machine: Machine,
        palette: Palette,
        text: TextSystem,
    ) -> io::Result<Self> {
        let canceller = client.canceller();
        let (jobs, feed) = worker::start(client, path, machine)?;
        let content = Stack::new(Axis::Vertical, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let id = content.id();
        let ui = Ui::new(
            Box::new(ScrollView::new(content)),
            Context::new(text, palette),
        );
        let mut installer = Self {
            deb: None,
            state: State::Reading,
            ready: None,
            jobs,
            feed,
            canceller,
            intents: Rc::default(),
            ui,
            content: id,
            closed: false,
        };
        installer.ask(Job::Inspect);
        installer.rebuild();
        Ok(installer)
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }

    pub fn accessibility(&self) -> accesskit::TreeUpdate {
        self.ui.accessibility()
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.feed.fd()
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    fn ask(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::warn!("the installer worker stopped");
        }
    }

    fn receive(&mut self, message: Message) {
        match message {
            Message::Read(deb) => self.deb = deb,
            Message::State(state) => {
                if matches!(state, State::Ready(_) | State::Present) {
                    self.ready = Some(state.clone());
                }
                self.state = state;
            }
            Message::Progress(progress) => {
                if let State::Installing(current) | State::Removing(current) = &mut self.state {
                    *current = progress;
                }
            }
            Message::Back => self.back(),
        }
    }

    fn back(&mut self) {
        self.state = self.ready.clone().unwrap_or(State::Reading);
    }

    fn act(&mut self, intent: Intent) {
        match intent {
            Intent::Install => {
                self.state = State::Installing(Progress::default());
                self.ask(Job::Install);
            }
            Intent::Cancel => {
                if let Err(error) = self.canceller.cancel() {
                    tracing::warn!("cannot cancel: {error}");
                }
            }
            Intent::AskRemove => {
                self.state = State::Reading;
                self.ask(Job::AskRemove);
            }
            Intent::Remove => {
                self.state = State::Removing(Progress::default());
                self.ask(Job::Remove);
            }
            Intent::Back => self.back(),
            Intent::Open(index) => {
                if let State::Installed { apps, .. } = &self.state
                    && let Some(entry) = apps.get(index)
                {
                    if let Err(error) = bande_xdg::launch(entry, None) {
                        tracing::warn!("cannot open {}: {error}", entry.name);
                    }
                    self.closed = true;
                }
            }
            Intent::Close => self.closed = true,
        }
    }

    pub fn pump(&mut self) -> bool {
        let messages = self.feed.drain();
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if messages.is_empty() && intents.is_empty() {
            return false;
        }
        for message in messages {
            self.receive(message);
        }
        for intent in intents {
            self.act(intent);
        }
        self.rebuild();
        true
    }

    fn button(&self, title: &str, intent: Intent) -> Button {
        let intents = Rc::clone(&self.intents);
        Button::new(title, move || intents.borrow_mut().push(intent))
    }

    fn name(&self) -> String {
        self.deb
            .as_ref()
            .map(|deb| deb.name.clone())
            .unwrap_or_default()
    }

    fn header(&self) -> Widgets {
        let Some(deb) = &self.deb else {
            return Vec::new();
        };
        let mut widgets = vec![
            boxed(Label::heading(&deb.name)),
            boxed(Label::new(&L10N.tr_args(
                "version-size",
                [
                    ("version", deb.version.clone().into()),
                    ("size", format_size(deb.installed_size).into()),
                ],
            ))),
        ];
        if !deb.summary.is_empty() {
            widgets.push(boxed(Label::new(&deb.summary)));
        }
        widgets
    }

    fn preview(&self, preview: &Preview) -> Widgets {
        let Some(deb) = &self.deb else {
            return Vec::new();
        };
        let mut widgets = vec![boxed(Label::new(&match &preview.action {
            Action::Install => {
                L10N.tr_args("new-install", [("version", deb.version.clone().into())])
            }
            Action::Upgrade(from) => L10N.tr_args(
                "upgrade-from",
                [
                    ("from", from.clone().into()),
                    ("to", deb.version.clone().into()),
                ],
            ),
            Action::Downgrade(from) => L10N.tr_args(
                "downgrade-from",
                [
                    ("from", from.clone().into()),
                    ("to", deb.version.clone().into()),
                ],
            ),
        }))];
        if let Action::Downgrade(from) = &preview.action {
            widgets.push(boxed(Label::new(
                &L10N.tr_args("downgrade-caution", [("from", from.clone().into())]),
            )));
        }
        if !preview.also.is_empty() {
            widgets.push(disclosure(
                &L10N.tr_args("also-installs", [("count", preview.also.len().into())]),
                &preview.also.join("\n"),
            ));
        }
        if !preview.removes.is_empty() {
            widgets.push(boxed(Label::new(&L10N.tr_args(
                "removes",
                [
                    ("count", preview.removes.len().into()),
                    ("names", preview.removes.join(", ").into()),
                ],
            ))));
        }
        widgets.push(boxed(Label::new(&L10N.tr("trust"))));
        if !deb.maintainer.is_empty() {
            widgets.push(boxed(Label::new(&L10N.tr_args(
                "from-maintainer",
                [("maintainer", deb.maintainer.clone().into())],
            ))));
        }
        if !deb.homepage.is_empty() {
            widgets.push(boxed(Label::new(&deb.homepage)));
        }
        if !deb.description.is_empty() {
            widgets.push(disclosure(&L10N.tr("details"), &deb.description));
        }
        let install = match preview.removes.is_empty() {
            true => self.button(&L10N.tr("install"), Intent::Install).primary(),
            false => self
                .button(&L10N.tr("install-and-remove"), Intent::Install)
                .destructive(),
        };
        widgets.push(row(vec![
            boxed(self.button(&L10N.tr("cancel"), Intent::Close)),
            boxed(install),
        ]));
        widgets
    }

    fn working(&self, progress: &Progress) -> Widgets {
        let status = status_line(progress.status);
        let mut cancel = self.button(&L10N.tr("cancel"), Intent::Cancel);
        cancel.set_enabled(progress.allow_cancel);
        vec![
            boxed(Label::new(&status)),
            boxed(ProgressBar::new(
                &status,
                progress.percent.map(|percent| percent as f32 / 100.0),
            )),
            row(vec![boxed(cancel)]),
        ]
    }

    fn body(&self) -> Widgets {
        let close = || boxed(self.button(&L10N.tr("close"), Intent::Close));
        let done = || boxed(self.button(&L10N.tr("done"), Intent::Close).primary());
        let name = self.name();
        match &self.state {
            State::Reading => vec![boxed(Spinner::new(&L10N.tr("reading")))],
            State::Invalid => vec![
                boxed(Label::new(&L10N.tr("invalid"))),
                boxed(Label::new(&L10N.tr("invalid-hint"))),
                row(vec![close()]),
            ],
            State::WrongArchitecture { arch, native } => vec![
                boxed(Label::new(&L10N.tr_args(
                    "wrong-architecture",
                    [
                        ("name", name.into()),
                        ("arch", arch_name(arch).into()),
                        ("native", arch_name(native).into()),
                    ],
                ))),
                boxed(Label::new(&L10N.tr_args(
                    "wrong-architecture-hint",
                    [("native", native.clone().into())],
                ))),
                row(vec![close()]),
            ],
            State::Present => {
                let version = self
                    .deb
                    .as_ref()
                    .map(|deb| deb.version.clone())
                    .unwrap_or_default();
                vec![
                    boxed(Label::new(
                        &L10N.tr_args("present", [("version", version.into())]),
                    )),
                    row(vec![
                        boxed(self.button(&L10N.tr("remove"), Intent::AskRemove)),
                        done(),
                    ]),
                ]
            }
            State::Ready(preview) => self.preview(preview),
            State::Blocked { missing, details } => {
                let mut widgets = vec![boxed(Label::new(&L10N.tr("blocked")))];
                if !missing.is_empty() {
                    widgets.push(boxed(Label::new(
                        &L10N.tr_args("needs", [("names", missing.join(", ").into())]),
                    )));
                }
                widgets.push(disclosure(&L10N.tr("details"), details));
                widgets.push(row(vec![close()]));
                widgets
            }
            State::Installing(progress) | State::Removing(progress) => self.working(progress),
            State::Installed { apps, restart } => {
                let mut widgets = vec![boxed(Label::new(
                    &L10N.tr_args("installed", [("name", name.into())]),
                ))];
                if *restart {
                    widgets.push(boxed(Label::new(&L10N.tr("restart-to-finish"))));
                }
                let mut buttons: Widgets = apps
                    .iter()
                    .enumerate()
                    .map(|(index, app)| {
                        boxed(self.button(
                            &L10N.tr_args("open-app", [("app", app.name.clone().into())]),
                            Intent::Open(index),
                        ))
                    })
                    .collect();
                buttons.push(done());
                widgets.push(row(buttons));
                widgets
            }
            State::ConfirmRemove(also) => {
                let mut widgets = vec![boxed(Label::new(
                    &L10N.tr_args("remove-confirm", [("name", name.into())]),
                ))];
                if !also.is_empty() {
                    widgets.push(boxed(Label::new(&L10N.tr_args(
                        "remove-also",
                        [
                            ("count", also.len().into()),
                            ("names", also.join(", ").into()),
                        ],
                    ))));
                }
                widgets.push(row(vec![
                    boxed(self.button(&L10N.tr("cancel"), Intent::Back)),
                    boxed(
                        self.button(&L10N.tr("remove-button"), Intent::Remove)
                            .destructive(),
                    ),
                ]));
                widgets
            }
            State::Removed => vec![
                boxed(Label::new(
                    &L10N.tr_args("removed", [("name", name.into())]),
                )),
                row(vec![done()]),
            ],
            State::Failed { problem, details } => {
                let sentence = match problem {
                    Problem::Locked => L10N.tr("failed-locked"),
                    Problem::SystemPackage => L10N.tr("failed-system"),
                    Problem::Unavailable => L10N.tr("failed-unavailable"),
                    Problem::Other => L10N.tr("failed-other"),
                };
                let mut widgets = vec![boxed(Label::new(&sentence))];
                if !details.is_empty() {
                    widgets.push(disclosure(&L10N.tr("details"), details));
                }
                widgets.push(row(vec![close()]));
                widgets
            }
        }
    }

    fn rebuild(&mut self) {
        let mut widgets = self.header();
        widgets.extend(self.body());
        let page = boxed(Stack::new(Axis::Vertical, SPACING, PADDING, widgets));
        if let Some(content) = self.ui.widget(self.content)
            && let Some(slot) = content.children_mut().first_mut()
        {
            *slot = page;
        }
    }
}
