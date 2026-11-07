mod mpris;
mod pane;

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::LazyLock;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, WindowId};
use bande_app_menu::Entry;
use bande_inbox::Inbox;
use bande_l10n::Localizer;
use bande_media::Track;

use mpris::{Mpris, Remote};
use pane::Pane;

static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-player")));

static INFO: Info = Info {
    id: "org.bande.Player",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

const WINDOW_SIZE: (u32, u32) = (720, 480);
const OPEN: &str = "open";
const TOGGLE: &str = "toggle";
const NEXT: &str = "next";
const PREVIOUS: &str = "previous";
const LOOP: &str = "loop";
const FULL_SCREEN: &str = "full-screen";
const RATE_PREFIX: &str = "rate:";
const AUDIO_PREFIX: &str = "audio:";
const SUBTITLES_PREFIX: &str = "subtitles:";
const SUBTITLES_OFF: &str = "subtitles-off";
const RATES: [(f64, &str); 5] = [
    (0.5, "0.5×"),
    (1.0, "1×"),
    (1.25, "1.25×"),
    (1.5, "1.5×"),
    (2.0, "2×"),
];
const MEDIA_MIMES: [&str; 2] = ["audio/*", "video/*"];

struct Player {
    chosen: Option<Inbox<PathBuf>>,
    remote: Option<Inbox<Remote>>,
    mpris: Option<Mpris>,
    current: Option<WindowId>,
}

impl Player {
    fn open_window(&mut self, cx: &mut Cx<Self>, paths: Vec<PathBuf>) {
        let title = paths
            .first()
            .map(|path| pane::file_title(path))
            .unwrap_or_else(|| L10N.tr("app-name"));
        match cx.open(Kind::Main, &title, WINDOW_SIZE, |appearance| {
            Pane::new(appearance, paths)
        }) {
            Ok(window) => self.current = Some(window),
            Err(error) => tracing::warn!("cannot open a window: {error}"),
        }
        cx.refresh_menus();
    }

    fn open_paths(&mut self, cx: &mut Cx<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let empty = cx
            .windows()
            .find(|window| cx.view(*window).is_some_and(Pane::is_empty));
        if let Some(window) = empty
            && let Some((shown, pane)) = cx.window(window)
        {
            pane.add(paths);
            shown.request_redraw();
            self.current = Some(window);
            return cx.raise(window, None);
        }
        self.open_window(cx, paths);
    }

    fn choose(&mut self, cx: &mut Cx<Self>) {
        let Some(session) = cx.session().cloned() else {
            return;
        };
        if self.chosen.is_none() {
            self.chosen = Inbox::new()
                .inspect_err(|error| tracing::warn!("cannot open the chooser: {error}"))
                .ok();
        }
        let Some(outbox) = self.chosen.as_ref().map(Inbox::outbox) else {
            return;
        };
        let title = L10N.tr("open-title");
        let filter = (
            L10N.tr("open-filter"),
            MEDIA_MIMES
                .into_iter()
                .map(|mime| (1, mime.to_owned()))
                .collect(),
        );
        let spawned = std::thread::Builder::new()
            .name("open".to_owned())
            .spawn(
                move || match bande_app::choose_file(&session, &title, Some(filter)) {
                    Ok(Some(path)) => {
                        outbox.send(path);
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!("cannot choose a file: {error}"),
                },
            );
        if let Err(error) = spawned {
            tracing::warn!("cannot open the chooser: {error}");
        }
    }

    fn start_mpris(&mut self, cx: &Cx<Self>) {
        let Some(connection) = cx.session().cloned() else {
            return;
        };
        let inbox = match Inbox::new() {
            Ok(inbox) => inbox,
            Err(error) => return tracing::warn!("cannot serve media controls: {error}"),
        };
        match Mpris::serve(connection, inbox.outbox()) {
            Ok(mpris) => {
                self.mpris = Some(mpris);
                self.remote = Some(inbox);
            }
            Err(error) => tracing::warn!("cannot serve media controls: {error}"),
        }
    }

    fn sync_mpris(&mut self, cx: &Cx<Self>) {
        let Some(mpris) = &self.mpris else {
            return;
        };
        let playing_window = cx
            .windows()
            .find(|window| cx.view(*window).is_some_and(Pane::is_playing));
        if let Some(window) = playing_window {
            self.current = Some(window);
        }
        let current = self
            .current
            .filter(|window| cx.view(*window).is_some())
            .or_else(|| cx.focused());
        mpris.update(
            current
                .and_then(|window| cx.view(window))
                .map(Pane::now_playing),
        );
    }

    fn remote(&mut self, cx: &mut Cx<Self>, remote: Remote) {
        let Some(window) = self.current.filter(|window| cx.view(*window).is_some()) else {
            return;
        };
        if matches!(remote, Remote::Raise) {
            return cx.raise(window, None);
        }
        if matches!(remote, Remote::Quit) {
            return cx.close(window);
        }
        if let Some((shown, pane)) = cx.window(window) {
            pane.remote(remote);
            shown.request_redraw();
        }
    }

    fn pane_of(cx: &Cx<Self>) -> Option<&Pane> {
        cx.view(cx.focused()?)
    }
}

fn paths(arguments: &[String], directory: &Path) -> Vec<PathBuf> {
    arguments
        .iter()
        .filter(|argument| !argument.starts_with('-'))
        .map(|argument| match argument.strip_prefix("file://") {
            Some(path) => PathBuf::from(pane::percent_decoded(path)),
            None => directory.join(argument),
        })
        .collect()
}

fn track_label(track: &Track, index: usize) -> String {
    match (&track.title, &track.language) {
        (Some(title), Some(language)) => format!("{title} ({language})"),
        (Some(title), None) => title.clone(),
        (None, Some(language)) => language.clone(),
        (None, None) => format!("{}", index + 1),
    }
}

impl Program for Player {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut player = Self {
            chosen: None,
            remote: None,
            mpris: None,
            current: None,
        };
        player.start_mpris(cx);
        let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        player.open_window(cx, paths(&arguments, &directory));
        Ok(player)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let pane = Self::pane_of(cx);
        let loaded = pane.is_some_and(Pane::is_loaded);
        let item = |key: &str, action: &str, shortcut: Option<&str>, enabled: bool| {
            let entry = Entry::item(&L10N.tr(key), action).enabled(enabled);
            match shortcut {
                Some(shortcut) => entry.shortcut(shortcut),
                None => entry,
            }
        };
        let rate = pane.map_or(1.0, Pane::rate);
        let rates = RATES
            .iter()
            .map(|(value, label)| {
                Entry::item(label, &format!("{RATE_PREFIX}{value}"))
                    .enabled(loaded)
                    .radio((rate - value).abs() < f64::EPSILON)
            })
            .collect();
        let info = pane.and_then(Pane::info);
        let audio_tracks = info.map_or_else(Vec::new, |info| {
            info.audio_tracks
                .iter()
                .enumerate()
                .map(|(index, track)| {
                    Entry::item(
                        &track_label(track, index),
                        &format!("{AUDIO_PREFIX}{}", track.id),
                    )
                    .radio(info.selected_audio.as_ref() == Some(&track.id))
                })
                .collect()
        });
        let mut subtitles = vec![
            Entry::item(&L10N.tr("menu-subtitles-off"), SUBTITLES_OFF)
                .enabled(loaded)
                .radio(info.is_none_or(|info| info.selected_subtitles.is_none())),
        ];
        if let Some(info) = info {
            subtitles.extend(
                info.subtitle_tracks
                    .iter()
                    .enumerate()
                    .map(|(index, track)| {
                        Entry::item(
                            &track_label(track, index),
                            &format!("{SUBTITLES_PREFIX}{}", track.id),
                        )
                        .radio(info.selected_subtitles.as_ref() == Some(&track.id))
                    }),
            );
        }
        let toggle_key = match pane.is_some_and(Pane::is_playing) {
            true => "menu-pause",
            false => "menu-play",
        };
        let queue = pane.is_some_and(Pane::has_queue);
        let file = vec![item("menu-open", OPEN, Some("Ctrl+O"), true)];
        let mut playback = vec![
            item(toggle_key, TOGGLE, None, loaded),
            item("menu-next", NEXT, Some("Ctrl+Right"), queue),
            item("menu-previous", PREVIOUS, Some("Ctrl+Left"), queue),
            Entry::separator(),
            Entry::submenu(&L10N.tr("menu-speed"), rates),
        ];
        if audio_tracks.len() > 1 {
            playback.push(Entry::submenu(&L10N.tr("menu-audio-track"), audio_tracks));
        }
        if subtitles.len() > 1 {
            playback.push(Entry::submenu(&L10N.tr("menu-subtitles"), subtitles));
        }
        playback.extend([
            Entry::separator(),
            item("menu-loop", LOOP, Some("Ctrl+L"), loaded).radio(pane.is_some_and(Pane::loops)),
        ]);
        let view = vec![item("menu-full-screen", FULL_SCREEN, Some("F11"), true)];
        Menus {
            menus: vec![
                Entry::submenu(&L10N.tr("menu-file"), file),
                Entry::submenu(&L10N.tr("menu-playback"), playback),
                Entry::submenu(&L10N.tr("menu-view"), view),
            ],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        if command == OPEN {
            return self.choose(cx);
        }
        let Some(window) = window.or_else(|| cx.focused()) else {
            return;
        };
        let Some((shown, pane)) = cx.window(window) else {
            return;
        };
        if let Some(rate) = command
            .strip_prefix(RATE_PREFIX)
            .and_then(|rate| rate.parse().ok())
        {
            pane.set_rate(rate);
        } else if let Some(audio) = command.strip_prefix(AUDIO_PREFIX) {
            pane.select_audio(audio);
        } else if let Some(subtitles) = command.strip_prefix(SUBTITLES_PREFIX) {
            pane.select_subtitles(Some(subtitles));
        } else {
            match command {
                TOGGLE => pane.toggle(),
                NEXT => pane.go(true),
                PREVIOUS => pane.go(false),
                LOOP => pane.toggle_loop(),
                SUBTITLES_OFF => pane.select_subtitles(None),
                FULL_SCREEN => {
                    if let Err(error) = shown.toggle_full_screen() {
                        tracing::warn!("cannot go full screen: {error}");
                    }
                }
                _ => return,
            }
        }
        shown.request_redraw();
        cx.refresh_menus();
        self.sync_mpris(cx);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        self.open_paths(cx, paths(&arguments, directory));
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.open_window(cx, Vec::new());
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, window: WindowId, _view: Pane) {
        if self.current == Some(window) {
            self.current = None;
        }
        self.sync_mpris(cx);
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        if let Some((shown, pane)) = cx.window(window)
            && pane.take_menus_changed()
        {
            shown.request_redraw();
            cx.refresh_menus();
        }
        self.sync_mpris(cx);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let chosen = self.chosen.iter().map(Inbox::fd);
        chosen.chain(self.remote.iter().map(Inbox::fd)).collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        let chosen = self.chosen.as_mut().map(Inbox::drain).unwrap_or_default();
        self.open_paths(cx, chosen);
        let remote = self.remote.as_mut().map(Inbox::drain).unwrap_or_default();
        for command in remote {
            self.remote(cx, command);
        }
        self.sync_mpris(cx);
    }
}

fn main() -> ExitCode {
    bande_app::run::<Player>(&INFO)
}
