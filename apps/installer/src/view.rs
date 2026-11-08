use std::cell::RefCell;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;


use bande_l10n::{text, tr, trn};

use crate::deb::{Deb, Identity, Machine};
use crate::flow::{Action, Preview, Problem, State};
use crate::icon::Icon;
use crate::worker::{self, Feed, Job, Message};
use bande_fs::format_size;
use bande_packages::{Canceller, Client, Progress, Status};
use bande_toolkit::ui::{
    Axis, Button, Context, Disclosure, Label, ProgressBar, ScrollView, Spinner, Stack, Ui, Widget,
    WidgetId,
};
use bande_toolkit::{Palette, TextSystem};

pub const WINDOW_SIZE: (u32, u32) = (520, 480);
const ICON_SIZE: f32 = 64.0;
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Install,
    InstallSigned,
    Cancel,
    AskRemove,
    Remove,
    Back,
    Retry,
    Open(usize),
    Close,
}

type Widgets = Vec<Box<dyn Widget>>;

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn arch_name(arch: &str) -> String {
    match arch {
        "amd64" => tr!("Intel and AMD computers (amd64)"),
        "arm64" => tr!("64-bit ARM (arm64)"),
        "armhf" => tr!("32-bit ARM (armhf)"),
        "i386" => tr!("older 32-bit Intel and AMD computers (i386)"),
        other => other.to_owned(),
    }
}

fn origin_name(origin: &str) -> String {
    let mut parts = origin.splitn(3, '-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("debian"), Some(suite), Some(component)) => tr!(
            "Debian {suite}, {component}",
            suite = suite.to_owned(),
            component = component.to_owned()
        ),
        _ => origin.to_owned(),
    }
}

fn status_line(status: Status) -> String {
    match status {
        Status::Waiting => tr!("Waiting for another installation to finish…"),
        Status::Authorizing => tr!("Waiting for your password…"),
        Status::Downloading => tr!("Downloading…"),
        Status::Installing => tr!("Installing…"),
        Status::Removing => tr!("Removing…"),
        Status::Working => tr!("Working…"),
    }
}

fn row(children: Widgets) -> Box<dyn Widget> {
    boxed(Stack::new(Axis::Horizontal, SPACING, 0.0, children).trailing())
}

fn disclosure(title: &str, text: &str) -> Box<dyn Widget> {
    boxed(Disclosure::new(title, false, Label::new(text), |_| {}))
}

pub struct Installer {
    deb: Option<Deb>,
    identity: Option<Identity>,
    state: State,
    ready: Option<State>,
    refused: bool,
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
            identity: None,
            state: State::Reading,
            ready: None,
            refused: false,
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

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
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
            Message::Identity(identity) => self.identity = identity,
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
            Message::Refused => {
                self.refused = true;
                self.back();
            }
        }
    }

    fn back(&mut self) {
        self.state = self.ready.clone().unwrap_or(State::Reading);
    }

    fn act(&mut self, intent: Intent) {
        self.refused = false;
        match intent {
            Intent::Install => {
                self.state = State::Installing(Progress::default());
                self.ask(Job::Install(None));
            }
            Intent::InstallSigned => {
                let Some(State::Ready(Preview {
                    signed: Some(signed),
                    ..
                })) = &self.ready
                else {
                    return;
                };
                let id = signed.id.clone();
                self.state = State::Installing(Progress::default());
                self.ask(Job::Install(Some(id)));
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
            Intent::Retry => {
                self.state = State::Reading;
                self.ask(Job::Inspect);
            }
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
        let facts = tr!(
            "Version {version} · {size}",
            version = deb.version.clone(),
            size = format_size(deb.installed_size)
        );
        let mut titles: Widgets = match &self.identity {
            Some(identity) => vec![
                boxed(Label::heading(&identity.name)),
                boxed(Label::new(&tr!(
                    "{package} · {facts}",
                    package = deb.name.clone(),
                    facts = facts
                ))),
            ],
            None => vec![boxed(Label::heading(&deb.name)), boxed(Label::new(&facts))],
        };
        if !deb.summary.is_empty() {
            titles.push(boxed(Label::new(&deb.summary).secondary()));
        }
        let titles = boxed(Stack::new(Axis::Vertical, 4.0, 0.0, titles));
        match self
            .identity
            .as_ref()
            .and_then(|identity| identity.icon.clone())
        {
            Some(icon) => vec![boxed(Stack::new(
                Axis::Horizontal,
                SPACING,
                0.0,
                vec![boxed(Icon::new(icon, ICON_SIZE)), titles],
            ))],
            None => vec![titles],
        }
    }

    fn preview(&self, preview: &Preview) -> Widgets {
        let Some(deb) = &self.deb else {
            return Vec::new();
        };
        let change = match &preview.action {
            Action::Install => None,
            Action::Upgrade(from) => Some((text!("Upgrades from {from} to {to}"), from)),
            Action::Downgrade(from) => Some((text!("Downgrades from {from} to {to}"), from)),
        };
        let mut widgets: Widgets = change
            .map(|(id, from)| {
                boxed(Label::new(&id.tr_args(&[
                    ("from", &from.clone() as &dyn std::fmt::Display),
                    ("to", &deb.version.clone() as &dyn std::fmt::Display),
                ])))
            })
            .into_iter()
            .collect();
        if let Action::Downgrade(from) = &preview.action {
            widgets.push(boxed(Label::new(&tr!(
                "This is older than the installed version {from}.",
                from = from.clone()
            ))));
        }
        if !preview.also.is_empty() {
            let title = match preview.download {
                0 => trn!(
                    "Also installs 1 package",
                    "Also installs {count} packages",
                    count = preview.also.len()
                ),
                size => trn!(
                    "Also installs 1 package · {size} to download",
                    "Also installs {count} packages · {size} to download",
                    count = preview.also.len(),
                    size = format_size(size)
                ),
            };
            widgets.push(disclosure(&title, &preview.also.join("\n")));
        }
        if !preview.removes.is_empty() {
            widgets.push(boxed(Label::new(&trn!(
                "Removes 1 package: {names}",
                "Removes {count} packages: {names}",
                count = preview.removes.len(),
                names = preview.removes.join(", ")
            ))));
        }
        match &preview.signed {
            Some(signed) => widgets.push(boxed(Label::new(&tr!("The same version is in your software sources ({source}), where it is signed. Installing it from there is safer than installing this file.", source = origin_name(&signed.origin))))),
            None => widgets.push(boxed(Label::new(&tr!("This package doesn't come from your software sources. Its install scripts get full control of this computer, and it may add its own software source. Install it only if you trust where you got it.")))),
        }
        if !deb.maintainer.is_empty() {
            widgets.push(boxed(Label::new(&tr!(
                "From {maintainer}",
                maintainer = deb.maintainer.clone()
            ))));
        }
        if !deb.homepage.is_empty() {
            widgets.push(boxed(Label::new(&deb.homepage)));
        }
        if !deb.description.is_empty() {
            widgets.push(disclosure(&tr!("Details"), &deb.description));
        }
        let install = match preview.removes.is_empty() {
            true => self.button(&tr!("Install"), Intent::Install).primary(),
            false => self
                .button(&tr!("Install and Remove"), Intent::Install)
                .destructive(),
        };
        widgets.push(boxed(
            Label::new(&tr!("Installing asks for an administrator’s password.")).secondary(),
        ));
        let mut buttons = vec![boxed(self.button(&tr!("Cancel"), Intent::Close))];
        match preview.signed.is_some() && preview.removes.is_empty() {
            true => buttons.extend([
                boxed(self.button(&tr!("Install This File"), Intent::Install)),
                boxed(
                    self.button(&tr!("Install from Software Sources"), Intent::InstallSigned)
                        .primary(),
                ),
            ]),
            false => buttons.push(boxed(install)),
        }
        widgets.push(row(buttons));
        widgets
    }

    fn working(&self, progress: &Progress) -> Widgets {
        let status = status_line(progress.status);
        let mut cancel = self.button(&tr!("Cancel"), Intent::Cancel);
        cancel.set_enabled(progress.allow_cancel);
        let fraction = match progress.status {
            Status::Authorizing | Status::Waiting => None,
            _ => progress.percent.map(|percent| percent as f32 / 100.0),
        };
        let mut widgets = vec![
            boxed(Label::new(&status)),
            boxed(ProgressBar::new(&status, fraction)),
        ];
        if progress.status == Status::Authorizing {
            widgets.push(boxed(Label::new(&tr!("Enter an administrator’s password in the dialog to continue, or cancel it to stop.")).secondary()));
        }
        widgets.push(row(vec![boxed(cancel)]));
        widgets
    }

    fn body(&self) -> Widgets {
        let close = || boxed(self.button(&tr!("Close"), Intent::Close));
        let done = || boxed(self.button(&tr!("Done"), Intent::Close).primary());
        let name = self.name();
        match &self.state {
            State::Reading => vec![boxed(Spinner::new(&tr!("Reading the package…")))],
            State::Invalid => vec![
                boxed(Label::new(&tr!("This file isn't a valid Debian package."))),
                boxed(Label::new(&tr!(
                    "The download may be incomplete. Try downloading it again."
                ))),
                row(vec![close()]),
            ],
            State::WrongArchitecture { arch, native } => vec![
                boxed(Label::new(&tr!(
                    "“{name}” is built for {arch}. This computer needs a package for {native}.",
                    name = name,
                    arch = arch_name(arch),
                    native = arch_name(native)
                ))),
                boxed(Label::new(&tr!(
                    "Look for a download for {native} on the developer's website.",
                    native = native.clone()
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
                    boxed(Label::new(&tr!(
                        "Version {version} is installed.",
                        version = version
                    ))),
                    row(vec![
                        boxed(self.button(&tr!("Remove…"), Intent::AskRemove)),
                        done(),
                    ]),
                ]
            }
            State::Ready(preview) => self.preview(preview),
            State::Blocked { missing, details } => {
                let mut widgets = vec![boxed(Label::new(&tr!(
                    "Can't install: it needs packages that aren't available"
                )))];
                if !missing.is_empty() {
                    widgets.push(boxed(Label::new(&tr!(
                        "Needs {names}",
                        names = missing.join(", ")
                    ))));
                }
                widgets.push(disclosure(&tr!("Details"), details));
                widgets.push(row(vec![close()]));
                widgets
            }
            State::Installing(progress) | State::Removing(progress) => self.working(progress),
            State::Installed { apps, restart } => {
                let mut widgets = vec![boxed(Label::new(&tr!(
                    "“{name}” is installed.",
                    name = name
                )))];
                if *restart {
                    widgets.push(boxed(Label::new(&tr!(
                        "Restart the computer to finish installing."
                    ))));
                }
                let mut buttons: Widgets = apps
                    .iter()
                    .enumerate()
                    .map(|(index, app)| {
                        boxed(self.button(
                            &tr!("Open {app}", app = app.name.clone()),
                            Intent::Open(index),
                        ))
                    })
                    .collect();
                buttons.push(done());
                widgets.push(row(buttons));
                widgets
            }
            State::ConfirmRemove(also) => {
                let mut widgets = vec![boxed(Label::new(&tr!("Remove “{name}”?", name = name)))];
                if !also.is_empty() {
                    widgets.push(boxed(Label::new(&trn!(
                        "This also removes 1 package: {names}",
                        "This also removes {count} packages: {names}",
                        count = also.len(),
                        names = also.join(", ")
                    ))));
                }
                widgets.push(row(vec![
                    boxed(self.button(&tr!("Cancel"), Intent::Back)),
                    boxed(self.button(&tr!("Remove"), Intent::Remove).destructive()),
                ]));
                widgets
            }
            State::Removed => vec![
                boxed(Label::new(&tr!("“{name}” was removed.", name = name))),
                row(vec![done()]),
            ],
            State::Failed { problem, details } => {
                let sentence = match problem {
                    Problem::Locked => {
                        tr!("Another installation is running. Try again when it finishes.")
                    }
                    Problem::SystemPackage => {
                        tr!("This package is part of the system and can't be removed.")
                    }
                    Problem::Unavailable => {
                        tr!("Installing packages needs PackageKit, which isn't installed.")
                    }
                    Problem::NoNetwork => tr!(
                        "The packages it needs couldn't be downloaded. Check your internet connection and try again."
                    ),
                    Problem::NoSpace => {
                        tr!("There isn't enough disk space. Free up some space and try again.")
                    }
                    Problem::Dependencies => tr!(
                        "It needs packages that can't be installed right now. Install pending updates in Settings › Software Update, then try again."
                    ),
                    Problem::Install => tr!(
                        "The package couldn't be installed. Try again, or open Details to see what went wrong."
                    ),
                    Problem::Remove => tr!(
                        "The package couldn't be removed. Try again, or open Details to see what went wrong."
                    ),
                };
                let mut widgets = vec![boxed(Label::new(&sentence))];
                if !details.is_empty() {
                    widgets.push(disclosure(&tr!("Details"), details));
                }
                widgets.push(row(vec![
                    close(),
                    boxed(self.button(&tr!("Try Again"), Intent::Retry).primary()),
                ]));
                widgets
            }
        }
    }

    fn rebuild(&mut self) {
        let mut widgets = self.header();
        if self.refused {
            widgets.push(boxed(Label::new(&tr!("Nothing was changed, because no administrator's password was given. If no password window appeared, log out and back in, then try again."))));
        }
        widgets.extend(self.body());
        let page = boxed(Stack::new(Axis::Vertical, SPACING, PADDING, widgets));
        if let Some(content) = self.ui.widget(self.content)
            && let Some(slot) = content.children_mut().first_mut()
        {
            *slot = page;
        }
    }
}
