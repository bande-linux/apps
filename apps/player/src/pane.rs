use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_app::View;
use bande_media::{FailureKind, MediaInfo, Playback, VideoInfo};
use bande_media_view::{Session, StageWidget};
use bande_toolkit::ui::{Context, Event, Key, Ui, Widget, WidgetId};
use bande_toolkit::{Appearance, Rect, Scene, Sizing, TextSystem, Window, WindowEvent};

use crate::mpris::{NowPlaying, Remote};

pub fn file_title(path: &Path) -> String {
    path.file_stem()
        .or(path.file_name())
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

const AUDIO_SIZE: (u32, u32) = (420, 360);
const LARGEST_FIT: (f64, f64) = (1120.0, 700.0);

fn fitted_size(window: &Window, video: VideoInfo) -> (u32, u32) {
    let scale = window.scale().max(1.0);
    let (width, height) = (
        f64::from(video.display_width) / scale,
        f64::from(video.height) / scale,
    );
    let shrink = (LARGEST_FIT.0 / width).min(LARGEST_FIT.1 / height).min(1.0);
    ((width * shrink) as u32, (height * shrink) as u32)
}

pub struct Pane {
    ui: Ui,
    stage: WidgetId,
    queue: Vec<PathBuf>,
    at: usize,
    looping: bool,
    remembered: Option<Playback>,
    fitted: bool,
    shown_title: Option<String>,
    menus_changed: bool,
    pending_load: bool,
}

impl Pane {
    pub fn new(appearance: Appearance, queue: Vec<PathBuf>) -> Self {
        let widget = StageWidget::new();
        let stage = Widget::id(&widget);
        let mut ui = Ui::new(
            Box::new(widget),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui.set_focus(stage);
        let mut pane = Self {
            ui,
            stage,
            queue,
            at: 0,
            looping: false,
            remembered: None,
            fitted: false,
            shown_title: None,
            menus_changed: false,
            pending_load: false,
        };
        pane.load();
        pane
    }

    fn load(&mut self) {
        let Some(path) = self.queue.get(self.at).cloned() else {
            return;
        };
        let loaded = self.with_stage(|widget| widget.stage_mut().session_mut().load(&path, true));
        if let Some(Err(error)) = loaded {
            tracing::warn!("cannot start the media engine: {error}");
        }
        self.fitted = false;
        self.remembered = None;
        self.menus_changed = true;
    }

    fn widget(&self) -> Option<&StageWidget> {
        self.ui.widget_ref(self.stage)
    }

    fn with_stage<R>(&mut self, change: impl FnOnce(&mut StageWidget) -> R) -> Option<R> {
        self.ui.widget_as::<StageWidget>(self.stage).map(change)
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn add(&mut self, paths: Vec<PathBuf>) {
        self.queue = paths;
        self.at = 0;
        self.load();
    }

    fn session(&self) -> Option<&Session> {
        self.widget().map(|widget| widget.stage().session())
    }

    pub fn info(&self) -> Option<&MediaInfo> {
        self.session().map(Session::info)
    }

    pub fn is_loaded(&self) -> bool {
        self.session().is_some_and(Session::is_loaded)
    }

    pub fn is_playing(&self) -> bool {
        self.session().is_some_and(Session::is_playing)
    }

    pub fn has_queue(&self) -> bool {
        self.queue.len() > 1
    }

    pub fn loops(&self) -> bool {
        self.looping
    }

    pub fn rate(&self) -> f64 {
        self.session().map_or(1.0, Session::rate)
    }

    pub fn take_menus_changed(&mut self) -> bool {
        std::mem::take(&mut self.menus_changed)
    }

    pub fn toggle(&mut self) {
        self.with_stage(|widget| widget.stage_mut().session_mut().toggle());
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.with_stage(|widget| widget.stage_mut().session_mut().set_rate(rate));
    }

    pub fn toggle_loop(&mut self) {
        self.looping = !self.looping;
    }

    pub fn select_audio(&mut self, id: &str) {
        let subtitles = self.info().and_then(|info| info.selected_subtitles.clone());
        let audio = Some(id.to_owned());
        self.with_stage(|widget| widget.stage().session().select_tracks(audio, subtitles));
    }

    pub fn select_subtitles(&mut self, id: Option<&str>) {
        let audio = self.info().and_then(|info| info.selected_audio.clone());
        let subtitles = id.map(str::to_owned);
        self.with_stage(|widget| widget.stage().session().select_tracks(audio, subtitles));
    }

    pub fn go(&mut self, forward: bool) {
        if self.queue.is_empty() {
            return;
        }
        let last = self.queue.len() - 1;
        self.at = match (forward, self.at) {
            (true, at) if at == last => 0,
            (true, at) => at + 1,
            (false, 0) => last,
            (false, at) => at - 1,
        };
        self.pending_load = true;
    }

    pub fn remote(&mut self, remote: Remote) {
        match remote {
            Remote::Toggle => self.toggle(),
            Remote::Play => {
                self.with_stage(|widget| widget.stage_mut().session_mut().play());
            }
            Remote::Pause => {
                self.with_stage(|widget| widget.stage_mut().session_mut().pause());
            }
            Remote::Next => self.go(true),
            Remote::Previous => self.go(false),
            Remote::Seek(offset_us) => {
                let now = Instant::now();
                self.with_stage(|widget| {
                    let session = widget.stage_mut().session_mut();
                    let position = session.position(now).as_micros() as i64;
                    let target = position.saturating_add(offset_us).max(0) as u64;
                    session.seek(Duration::from_micros(target), true);
                });
            }
            Remote::SetPosition(position_us) => {
                self.with_stage(|widget| {
                    let position = Duration::from_micros(position_us.max(0) as u64);
                    widget.stage_mut().session_mut().seek(position, true);
                });
            }
            Remote::Raise | Remote::Quit => {}
        }
    }

    pub fn now_playing(&self) -> NowPlaying {
        let path = self.queue.get(self.at);
        let session = self.session();
        let info = session.map(Session::info);
        NowPlaying {
            playback: session.map(Session::playback),
            title: info
                .and_then(|info| info.title.clone())
                .or_else(|| path.map(|path| file_title(path))),
            artist: info.and_then(|info| info.artist.clone()),
            album: info.and_then(|info| info.album.clone()),
            path: path.cloned(),
            duration_ms: info.and_then(|info| info.duration_ms),
            position: session.map_or(Duration::ZERO, |session| session.position(Instant::now())),
            rate: session.map_or(1.0, Session::rate),
            volume: session.map_or(1.0, Session::volume),
            seekable: info.is_some_and(|info| info.seekable),
            has_queue: self.has_queue(),
        }
    }

    fn fit_window(&mut self, window: &mut Window) {
        if self.fitted {
            return;
        }
        let playback = self.session().map(Session::playback);
        let Some(info) = self.info().filter(|_| playback != Some(Playback::Opening)) else {
            return;
        };
        let size = match info.video {
            Some(video) => fitted_size(window, video),
            None if info.has_audio => AUDIO_SIZE,
            None => return,
        };
        self.fitted = true;
        window.resize_content(size, Sizing::Resizable);
    }

    fn follow(&mut self, window: &mut Window) {
        let playback = self.session().map(Session::playback);
        if playback != self.remembered {
            self.remembered = playback;
            self.menus_changed = true;
            if playback == Some(Playback::Ended) {
                self.ended();
            }
        }
        if std::mem::take(&mut self.pending_load) {
            self.load();
        }
        if let Some(path) = self.queue.get(self.at) {
            let title = self
                .info()
                .and_then(|info| info.title.clone())
                .unwrap_or_else(|| file_title(path));
            if self.shown_title.as_ref() != Some(&title) {
                window.set_title(&title);
                self.shown_title = Some(title);
            }
        }
        self.fit_window(window);
    }

    fn ended(&mut self) {
        let failed = self
            .session()
            .and_then(Session::failure)
            .is_some_and(|kind| kind != FailureKind::Failed);
        if failed {
            return;
        }
        match (self.at + 1 < self.queue.len(), self.looping) {
            (true, _) => self.go(true),
            (false, true) if self.queue.len() > 1 => self.go(true),
            (false, true) => {
                self.with_stage(|widget| widget.stage_mut().session_mut().play());
            }
            (false, false) => {}
        }
    }
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Ui::scene(&mut self.ui, width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let double_click = matches!(
            &event,
            WindowEvent::Input(Event::PointerDown(pointer))
                if pointer.count == 2 && !self.widget().is_some_and(|widget| widget.stage().in_bar(pointer.position))
        );
        if double_click && self.session().is_some_and(Session::has_video) {
            if let Err(error) = window.toggle_full_screen() {
                tracing::warn!("cannot go full screen: {error}");
            }
            return true;
        }
        let escape = matches!(
            &event,
            WindowEvent::Input(Event::Key {
                key: Key::Escape,
                ..
            })
        );
        if escape && window.toplevel_state().fullscreen {
            if let Err(error) = window.toggle_full_screen() {
                tracing::warn!("cannot leave full screen: {error}");
            }
            return true;
        }
        let handled = match window.drive(&mut self.ui, event) {
            Ok(unhandled) => unhandled.is_none(),
            Err(error) => {
                tracing::warn!("{error}");
                false
            }
        };
        let now = Instant::now();
        self.with_stage(|widget| widget.stage_mut().settle(now));
        self.follow(window);
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.session().and_then(Session::fd).into_iter().collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) -> bool {
        let changed = self
            .with_stage(|widget| widget.stage_mut().session_mut().ready(window))
            .unwrap_or(false);
        let now = Instant::now();
        self.with_stage(|widget| widget.stage_mut().settle(now));
        self.follow(window);
        changed
    }

    fn deadline(&self) -> Option<Instant> {
        let stage = self.widget().and_then(|widget| widget.stage().deadline());
        [stage, self.ui.deadline()].into_iter().flatten().min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) -> bool {
        let changed = self
            .with_stage(|widget| widget.stage_mut().tick(now))
            .unwrap_or(false);
        self.follow(window);
        changed
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
