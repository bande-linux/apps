mod frames;
mod info;

use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use bande_media::{Channel, Command, Event, FailureKind, MediaInfo, Playback};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use rustix::fs::{MemfdFlags, memfd_create};
use rustix::net::{RecvFlags, SendFlags};

use frames::Frames;

const ONE_MILLISECOND: gst::ClockTime = gst::ClockTime::from_mseconds(1);
const COMPOSITOR_DELAY: gst::ClockTime = gst::ClockTime::from_mseconds(16);
const STEP_BACK_FALLBACK_MS: u64 = 40;
const SIDECAR_EXTENSIONS: [&str; 3] = ["srt", "vtt", "ass"];

struct Status {
    info: MediaInfo,
    rate: f64,
    ended: bool,
    collection: Option<gst::StreamCollection>,
}

#[derive(Clone)]
struct Engine {
    channel: Arc<Channel>,
    playbin: gst::Element,
    video: gst_app::AppSink,
    status: Arc<Mutex<Status>>,
    frames: Arc<Frames>,
}

fn emit(channel: &Channel, event: &Event) {
    if channel.send(event, &[], SendFlags::empty()).is_err() {
        std::process::exit(0);
    }
}

fn make(factory: &str) -> Result<gst::Element, gst::glib::BoolError> {
    gst::ElementFactory::make(factory).build()
}

impl Engine {
    fn new(channel: Arc<Channel>) -> Result<Self, gst::glib::BoolError> {
        let frames = Arc::new(Frames::new(Arc::clone(&channel)));
        let video = frames::video_sink(Arc::clone(&frames), COMPOSITOR_DELAY);
        let subtitles = frames::subtitle_sink(Arc::clone(&channel));
        let playbin = make("playbin3")?;
        playbin.set_property("video-sink", &video);
        playbin.set_property("text-sink", &subtitles);
        if let Ok(pitch) = make("scaletempo") {
            playbin.set_property("audio-filter", &pitch);
        }
        Ok(Self {
            channel,
            playbin,
            video,
            status: Arc::new(Mutex::new(Status {
                info: MediaInfo::default(),
                rate: 1.0,
                ended: false,
                collection: None,
            })),
            frames,
        })
    }

    fn failed(&self, kind: FailureKind, detail: String) {
        emit(&self.channel, &Event::Failed { kind, detail });
    }

    fn with_info(&self, change: impl FnOnce(&mut MediaInfo)) {
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        let before = status.info.clone();
        change(&mut status.info);
        if status.info != before {
            emit(&self.channel, &Event::Info(Box::new(status.info.clone())));
        }
    }

    fn position(&self) -> Option<gst::ClockTime> {
        self.playbin.query_position::<gst::ClockTime>()
    }

    fn seek_to(&self, position: gst::ClockTime, flags: gst::SeekFlags, rate: f64) {
        let sought = self.playbin.seek(
            rate,
            flags,
            gst::SeekType::Set,
            position,
            gst::SeekType::None,
            gst::ClockTime::NONE,
        );
        if let Err(error) = sought {
            eprintln!("bande-media-engine: cannot seek: {error}");
        }
    }

    fn rate(&self) -> f64 {
        self.status.lock().map_or(1.0, |status| status.rate)
    }

    fn open(&self, path: &Path) {
        let Ok(uri) = gst::glib::filename_to_uri(path, None) else {
            return self.failed(FailureKind::Unreadable, path.display().to_string());
        };
        let _ = self.playbin.set_state(gst::State::Ready);
        if let Ok(mut status) = self.status.lock() {
            status.info = MediaInfo::default();
            status.ended = false;
            status.collection = None;
        }
        self.frames.new_media();
        self.playbin.set_property("uri", uri.as_str());
        let sidecar = SIDECAR_EXTENSIONS
            .iter()
            .map(|extension| path.with_extension(extension))
            .find(|candidate| candidate.is_file())
            .and_then(|candidate| gst::glib::filename_to_uri(candidate, None).ok());
        self.playbin
            .set_property("suburi", sidecar.as_ref().map(|uri| uri.as_str()));
        emit(&self.channel, &Event::State(Playback::Opening));
        if self.playbin.set_state(gst::State::Paused).is_err() {
            eprintln!("bande-media-engine: the pipeline refused to start");
        }
    }

    fn play(&self) {
        let ended = self
            .status
            .lock()
            .is_ok_and(|mut status| std::mem::take(&mut status.ended));
        if ended {
            self.seek_to(gst::ClockTime::ZERO, gst::SeekFlags::FLUSH, self.rate());
        }
        if self.playbin.set_state(gst::State::Playing).is_err() {
            eprintln!("bande-media-engine: cannot play");
        }
    }

    fn step(&self, forward: bool) {
        let _ = self.playbin.set_state(gst::State::Paused);
        if forward {
            let step = gst::event::Step::new(gst::format::Buffers::ONE, 1.0, true, false);
            self.video.send_event(step);
            return;
        }
        let back = self
            .frames
            .frame_duration()
            .unwrap_or(gst::ClockTime::from_mseconds(STEP_BACK_FALLBACK_MS));
        let target = self.position().unwrap_or_default().saturating_sub(back);
        self.seek_to(
            target,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            self.rate(),
        );
    }

    fn select_tracks(&self, audio: Option<String>, subtitles: Option<String>) {
        let Ok(status) = self.status.lock() else {
            return;
        };
        let Some(collection) = status.collection.as_ref() else {
            return;
        };
        let ids = info::chosen_streams(collection, audio.as_deref(), subtitles.as_deref());
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        self.playbin
            .send_event(gst::event::SelectStreams::new(ids.iter().copied()));
    }

    fn command(&self, command: Command) {
        match command {
            Command::Configure { nv12_dmabuf } => self.frames.configure(&self.video, nv12_dmabuf),
            Command::Open(path) => self.open(&path),
            Command::Play => self.play(),
            Command::Pause => {
                let _ = self.playbin.set_state(gst::State::Paused);
            }
            Command::Seek { position_ms, exact } => {
                if let Ok(mut status) = self.status.lock() {
                    status.ended = false;
                }
                let accuracy = match exact {
                    true => gst::SeekFlags::ACCURATE,
                    false => gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_NEAREST,
                };
                self.seek_to(
                    ONE_MILLISECOND * position_ms,
                    gst::SeekFlags::FLUSH | accuracy,
                    self.rate(),
                );
            }
            Command::Step { forward } => self.step(forward),
            Command::SetRate(rate) => {
                if let Ok(mut status) = self.status.lock() {
                    status.rate = rate;
                }
                self.seek_to(
                    self.position().unwrap_or_default(),
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                    rate,
                );
            }
            Command::SetVolume(volume) => self.playbin.set_property("volume", volume),
            Command::SelectTracks { audio, subtitles } => self.select_tracks(audio, subtitles),
            Command::QueryPosition => {
                if let Some(position) = self.position() {
                    emit(&self.channel, &Event::Position(position.mseconds()));
                }
            }
            Command::Released(id) => self.frames.released(id),
        }
    }

    fn refresh_extent(&self) {
        let duration = self
            .playbin
            .query_duration::<gst::ClockTime>()
            .map(gst::ClockTime::mseconds);
        let mut query = gst::query::Seeking::new(gst::Format::Time);
        let seekable = self.playbin.query(&mut query) && query.result().0;
        self.with_info(|info| {
            info.duration_ms = duration.or(info.duration_ms);
            info.seekable = seekable;
        });
    }

    fn send_art(&self, encoded: &[u8]) {
        let sent = memfd_create("bande-media-art", MemfdFlags::CLOEXEC)
            .map_err(std::io::Error::from)
            .and_then(|fd: OwnedFd| {
                std::fs::File::from(fd.try_clone()?).write_all(encoded)?;
                let event = Event::Art {
                    length: encoded.len() as u64,
                };
                self.channel.send(&event, &[fd.as_fd()], SendFlags::empty())
            });
        if let Err(error) = sent {
            eprintln!("bande-media-engine: cannot pass the cover art: {error}");
        }
    }

    fn state_changed(&self, new: gst::State) {
        let ended = self.status.lock().is_ok_and(|status| status.ended);
        match new {
            gst::State::Playing => emit(&self.channel, &Event::State(Playback::Playing)),
            gst::State::Paused if !ended => {
                self.refresh_extent();
                emit(&self.channel, &Event::State(Playback::Paused));
            }
            _ => {}
        }
    }

    fn handle(&self, message: &gst::Message) {
        use gst::MessageView;
        let from_playbin = message.src() == Some(self.playbin.upcast_ref());
        match message.view() {
            MessageView::Eos(_) => {
                if let Ok(mut status) = self.status.lock() {
                    status.ended = true;
                }
                let _ = self.playbin.set_state(gst::State::Paused);
                emit(&self.channel, &Event::State(Playback::Ended));
            }
            MessageView::Error(error) => {
                let (kind, detail) = info::failure(error.error(), error.debug().as_deref());
                self.failed(kind, detail);
            }
            MessageView::Element(element) => {
                if let Some(detail) = info::missing_plugin(element.structure()) {
                    self.failed(FailureKind::Unsupported, detail);
                }
            }
            MessageView::StateChanged(changed) if from_playbin => {
                self.state_changed(changed.current());
            }
            MessageView::AsyncDone(_) | MessageView::DurationChanged(_) => self.refresh_extent(),
            MessageView::Tag(tag) => {
                let tags = tag.tags();
                self.with_info(|media| info::apply_tags(media, &tags));
                if let Some(art) = info::cover_art(&tags) {
                    self.send_art(&art);
                }
            }
            MessageView::StreamCollection(message) => {
                let collection = message.stream_collection();
                if let Ok(mut status) = self.status.lock() {
                    status.collection = Some(collection.clone());
                }
                self.with_info(|media| info::apply_collection(media, &collection));
                self.select_tracks(None, None);
            }
            MessageView::StreamsSelected(message) => {
                let selected: Vec<gst::Stream> = message.streams().collect();
                self.with_info(|media| info::apply_selection(media, &selected));
            }
            _ => {}
        }
    }

    fn follow_bus(&self) -> Result<(), std::io::Error> {
        let Some(bus) = self.playbin.bus() else {
            return Err(std::io::ErrorKind::NotFound.into());
        };
        let engine = self.clone();
        std::thread::Builder::new()
            .name("bus".to_owned())
            .spawn(move || {
                for message in bus.iter_timed(gst::ClockTime::NONE) {
                    engine.handle(&message);
                }
            })?;
        Ok(())
    }

    fn set_video(&self, video: bande_media::VideoInfo) {
        self.with_info(|media| media.video = Some(video));
    }
}

fn run(channel: Arc<Channel>) -> Result<(), Box<dyn std::error::Error>> {
    gst::init()?;
    let engine = Engine::new(Arc::clone(&channel))?;
    let reporter = engine.clone();
    engine
        .frames
        .on_video(Box::new(move |video| reporter.set_video(video)));
    engine.follow_bus()?;
    emit(&channel, &Event::Ready);
    while let Ok(Some((command, _))) = channel.receive::<Command>(RecvFlags::empty()) {
        engine.command(command);
    }
    Ok(())
}

fn main() -> ExitCode {
    let channel = match std::io::stdin().as_fd().try_clone_to_owned() {
        Ok(socket) => Arc::new(Channel::from_fd(socket)),
        Err(error) => {
            eprintln!("bande-media-engine: no socket: {error}");
            return ExitCode::FAILURE;
        }
    };
    match run(channel) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("bande-media-engine: {error}");
            ExitCode::FAILURE
        }
    }
}
