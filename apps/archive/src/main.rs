use std::collections::VecDeque;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bande_l10n::tr;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Settings, View, WindowId};
use bande_app_menu::Entry;
use bande_app_settings::{SettingsWindow, settings_folder};
use bande_archive::{
    After, ArchiveSettings, Ended, Intent, PANEL_SIZE, PROMPT_SIZE, Panel, Row, SETTINGS_SIZE,
    Work, settings_form,
};
use bande_fs::{Trash, can_extract};
use bande_inbox::Inbox;
use bande_toolkit::{Appearance, Rect, Scene, Window, WindowEvent};
use zbus::blocking::Connection;

static INFO: Info = Info {
    id: "org.bande.ArchiveUtility",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Archive Utility"),
    keys: Keys::Standard,
    settings: Some(Settings {
        title: || tr!("Archive Utility Settings"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

const SHOW_AFTER: Duration = Duration::from_millis(500);
const COMPRESS_FLAG: &str = "--compress";
const EXPAND_ARCHIVE: &str = "expand-archive";
const SHOW_PANEL: &str = "show-panel";
const FILE_MANAGER: &str = "org.freedesktop.FileManager1";
const FILE_MANAGER_PATH: &str = "/org/freedesktop/FileManager1";
const MIME_FILTER: u32 = 1;
const ARCHIVE_TYPES: [&str; 11] = [
    "application/zip",
    "application/x-tar",
    "application/x-compressed-tar",
    "application/x-bzip2-compressed-tar",
    "application/x-xz-compressed-tar",
    "application/x-zstd-compressed-tar",
    "application/gzip",
    "application/x-bzip2",
    "application/x-xz",
    "application/zstd",
    "application/x-7z-compressed",
];

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsWindow::<ArchiveSettings>::new(
        appearance,
        settings_folder(),
        settings_form,
    ))
}

struct Pane(Panel);

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        View::scene(&mut self.0.ui, window)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        View::event(&mut self.0.ui, window, event)
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.0.ui.set_window_active(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.0.ui.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.0.ui.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.0.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.0.ui.text_input_area()
    }
}

fn reveal(paths: Vec<PathBuf>) {
    let uris: Vec<String> = paths.iter().map(|path| bande_xdg::file_uri(path)).collect();
    let spawned = std::thread::Builder::new()
        .name("reveal".to_owned())
        .spawn(move || {
            let shown = Connection::session().and_then(|session| {
                session
                    .call_method(
                        Some(FILE_MANAGER),
                        FILE_MANAGER_PATH,
                        Some(FILE_MANAGER),
                        "ShowItems",
                        &(uris, ""),
                    )
                    .map(drop)
            });
            if let Err(error) = shown {
                tracing::warn!("cannot reveal the items in Files: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot reveal the items in Files: {error}");
    }
}

fn put_in_trash(paths: &[PathBuf]) {
    let Some(trash) = Trash::for_user() else {
        return tracing::warn!("there is no Trash without a home folder");
    };
    for path in paths {
        if let Err(error) = trash.put(path) {
            tracing::warn!("cannot move {} to the Trash: {error}", path.display());
        }
    }
}

struct Prompt {
    window: WindowId,
    archive: PathBuf,
}

struct Utility {
    works: Vec<(u64, Work)>,
    next: u64,
    panel: Option<WindowId>,
    wanted: bool,
    hidden: bool,
    alerts: VecDeque<(String, String)>,
    questions: VecDeque<(PathBuf, bool)>,
    prompt: Option<Prompt>,
    chosen: Inbox<PathBuf>,
}

impl Utility {
    fn open_paths(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        let paths = arguments.iter().map(|argument| {
            bande_xdg::path_from_file_uri(argument).unwrap_or_else(|| directory.join(argument))
        });
        if arguments
            .first()
            .is_some_and(|first| first == COMPRESS_FLAG)
        {
            let sources: Vec<PathBuf> = paths.skip(1).collect();
            if !sources.is_empty() {
                self.compress(cx, sources);
            }
            return;
        }
        for path in paths {
            match can_extract(&path) {
                true => self.expand(cx, path, None),
                false => self.compress(cx, vec![path]),
            }
        }
    }

    fn expand(&mut self, cx: &mut Cx<Self>, archive: PathBuf, password: Option<String>) {
        let started = Work::expand(archive, password, &ArchiveSettings::current());
        self.begin(cx, started);
    }

    fn compress(&mut self, cx: &mut Cx<Self>, sources: Vec<PathBuf>) {
        let started = Work::compress(sources, &ArchiveSettings::current());
        self.begin(cx, started);
    }

    fn begin(&mut self, cx: &mut Cx<Self>, started: io::Result<Work>) {
        match started {
            Ok(work) => {
                self.works.push((self.next, work));
                self.next += 1;
            }
            Err(error) => self.alerts.push_back((
                tr!(
                    "Archive Utility could not start: {error}",
                    error = error.to_string()
                ),
                String::new(),
            )),
        }
        self.refresh(cx);
    }

    fn finish(&mut self, cx: &mut Cx<Self>, work: &Work, ended: Ended) {
        match ended {
            Ended::Done(made) => {
                let settings = ArchiveSettings::current();
                let (after, shown) = match work.kind {
                    bande_archive::Kind::Expand { .. } => {
                        (settings.after_expanding, settings.reveal_expanded)
                    }
                    bande_archive::Kind::Compress { .. } => {
                        (settings.after_archiving, settings.reveal_archive)
                    }
                };
                if after == After::Trash {
                    put_in_trash(&work.originals());
                }
                if shown {
                    reveal(made);
                }
            }
            Ended::Password { archive, wrong } => {
                self.questions.push_back((archive, wrong));
                self.ask(cx);
            }
            Ended::Failed(message) => self.alerts.push_back((work.failure_title(), message)),
            Ended::Cancelled => {}
        }
    }

    fn ask(&mut self, cx: &mut Cx<Self>) {
        if self.prompt.is_some() {
            return;
        }
        let Some((archive, wrong)) = self.questions.pop_front() else {
            return;
        };
        let name = archive
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let opened = cx.open(
            Kind::Dialog("password"),
            &tr!("Archive Utility"),
            PROMPT_SIZE,
            |appearance| Pane(Panel::password(appearance, &name, wrong)),
        );
        match opened {
            Ok(window) => self.prompt = Some(Prompt { window, archive }),
            Err(error) => tracing::warn!("cannot ask for the password: {error}"),
        }
    }

    fn rows(&self) -> Vec<Row> {
        self.works
            .iter()
            .map(|(id, work)| Row {
                id: *id,
                title: work.title(),
                detail: work.detail(),
                fraction: work.fraction(),
            })
            .collect()
    }

    fn show_panel(&mut self, cx: &mut Cx<Self>) {
        if self.panel.is_some() {
            return;
        }
        let rows = self.rows();
        let opened = cx.open(
            Kind::Main,
            &tr!("Archive Utility"),
            PANEL_SIZE,
            |appearance| {
                let mut panel = Panel::progress(appearance);
                panel.show(&rows);
                Pane(panel)
            },
        );
        match opened {
            Ok(window) => self.panel = Some(window),
            Err(error) => tracing::warn!("cannot show the progress: {error}"),
        }
    }

    fn overdue(&self) -> bool {
        let now = Instant::now();
        !self.hidden
            && self
                .works
                .iter()
                .any(|(_, work)| work.started + SHOW_AFTER <= now)
    }

    fn refresh(&mut self, cx: &mut Cx<Self>) {
        if self.works.is_empty() {
            self.hidden = false;
        }
        if !self.alerts.is_empty() || self.wanted || self.overdue() {
            self.show_panel(cx);
        }
        let Some(window) = self.panel else {
            return;
        };
        let rows = self.rows();
        let Some((shown, pane)) = cx.window(window) else {
            return;
        };
        pane.0.show(&rows);
        if !pane.0.alerting()
            && let Some((title, message)) = self.alerts.pop_front()
        {
            pane.0.alert(&title, &message);
        }
        let idle = rows.is_empty() && !pane.0.alerting() && !self.wanted;
        shown.request_redraw();
        if idle {
            cx.close(window);
            self.panel = None;
        }
    }

    fn answer(&mut self, cx: &mut Cx<Self>, password: Option<String>) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        cx.close(prompt.window);
        if let Some(password) = password {
            self.expand(cx, prompt.archive, Some(password));
        }
        self.ask(cx);
    }

    fn choose_archive(&self) {
        let outbox = self.chosen.outbox();
        let spawned = std::thread::Builder::new()
            .name("choose archive".to_owned())
            .spawn(move || {
                let filter = (
                    tr!("Archives"),
                    ARCHIVE_TYPES
                        .iter()
                        .map(|mime| (MIME_FILTER, (*mime).to_owned()))
                        .collect(),
                );
                let chosen = Connection::session().and_then(|session| {
                    bande_app::choose_file(
                        &session,
                        &tr!("Choose an archive to expand"),
                        Some(filter),
                    )
                });
                match chosen {
                    Ok(Some(path)) => {
                        outbox.send(path);
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!("cannot choose an archive: {error}"),
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot choose an archive: {error}");
        }
    }
}

impl Program for Utility {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut utility = Self {
            works: Vec::new(),
            next: 0,
            panel: None,
            wanted: arguments.is_empty(),
            hidden: false,
            alerts: VecDeque::new(),
            questions: VecDeque::new(),
            prompt: None,
            chosen: Inbox::new()?,
        };
        let directory = std::env::current_dir()?;
        utility.open_paths(cx, arguments, &directory);
        utility.show_panel(cx);
        utility.refresh(cx);
        Ok(utility)
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        let expand = Entry::item(&tr!("Expand Archive…"), EXPAND_ARCHIVE).shortcut("Ctrl+O");
        Menus {
            menus: vec![Entry::submenu(&tr!("File"), vec![expand])],
            window: vec![Entry::item(&tr!("Archive Utility"), SHOW_PANEL)],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        match command {
            EXPAND_ARCHIVE => self.choose_archive(),
            SHOW_PANEL => self.reopen(cx),
            _ => {}
        }
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        self.open_paths(cx, arguments, directory);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.wanted = true;
        self.hidden = false;
        match self.panel {
            Some(panel) => cx.raise(panel, None),
            None => self.refresh(cx),
        }
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, window: WindowId, _view: Pane) {
        if self.panel == Some(window) {
            self.panel = None;
            self.wanted = false;
            self.hidden = !self.works.is_empty();
        }
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.window == window)
        {
            self.prompt = None;
            self.ask(cx);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let intents = match cx.window(window) {
            Some((_, pane)) => pane.0.take_intents(),
            None => return,
        };
        for intent in intents {
            match intent {
                Intent::Cancel(id) => {
                    if let Some((_, work)) = self.works.iter().find(|(known, _)| *known == id) {
                        work.cancel();
                    }
                }
                Intent::Dismiss => self.refresh(cx),
                Intent::Password(password) => self.answer(cx, password),
                Intent::Submit => {}
            }
        }
    }

    fn should_quit(&mut self, _cx: &mut Cx<Self>) -> bool {
        for (_, work) in &self.works {
            work.cancel();
        }
        for (_, work) in self.works.drain(..) {
            work.wait();
        }
        true
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.works
            .iter()
            .map(|(_, work)| work.fd())
            .chain([self.chosen.fd()])
            .collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        if index == self.works.len() {
            for archive in self.chosen.drain() {
                self.expand(cx, archive, None);
            }
            return;
        }
        let Some((_, work)) = self.works.get_mut(index) else {
            return;
        };
        if let Some(ended) = work.pump() {
            let (_, work) = self.works.remove(index);
            self.finish(cx, &work, ended);
        }
        self.refresh(cx);
    }

    fn deadline(&self) -> Option<Instant> {
        match (self.panel, self.hidden) {
            (None, false) => self
                .works
                .iter()
                .map(|(_, work)| work.started + SHOW_AFTER)
                .min(),
            _ => None,
        }
    }

    fn tick(&mut self, cx: &mut Cx<Self>, _now: Instant) {
        self.refresh(cx);
    }
}

fn main() -> ExitCode {
    bande_app::run::<Utility>(&INFO)
}
