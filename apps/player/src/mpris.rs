use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bande_inbox::Outbox;
use bande_media::Playback;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.bande_player";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const TRACK_PATH: &str = "/org/bande/Player/Track";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
const SEEK_DRIFT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Remote {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
    Seek(i64),
    SetPosition(i64),
    Raise,
    Quit,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowPlaying {
    pub playback: Option<Playback>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub path: Option<PathBuf>,
    pub duration_ms: Option<u64>,
    pub position: Duration,
    pub rate: f64,
    pub volume: f64,
    pub seekable: bool,
    pub has_queue: bool,
}

impl NowPlaying {
    fn status(&self) -> &'static str {
        match self.playback {
            Some(Playback::Playing) => "Playing",
            Some(Playback::Paused | Playback::Opening) => "Paused",
            Some(Playback::Ended) | None => "Stopped",
        }
    }

    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let mut metadata = HashMap::new();
        let mut put = |key: &str, value: Value<'_>| {
            if let Ok(value) = OwnedValue::try_from(value) {
                metadata.insert(key.to_owned(), value);
            }
        };
        let track = match self.path.is_some() {
            true => TRACK_PATH,
            false => NO_TRACK,
        };
        if let Ok(track) = ObjectPath::try_from(track) {
            put("mpris:trackid", Value::from(track));
        }
        if let Some(title) = &self.title {
            put("xesam:title", Value::from(title.as_str()));
        }
        if let Some(artist) = &self.artist {
            put("xesam:artist", Value::from(vec![artist.as_str()]));
        }
        if let Some(album) = &self.album {
            put("xesam:album", Value::from(album.as_str()));
        }
        if let Some(duration) = self.duration_ms {
            put(
                "mpris:length",
                Value::from(duration.saturating_mul(1000) as i64),
            );
        }
        if let Some(path) = &self.path {
            put(
                "xesam:url",
                Value::from(format!("file://{}", path.display())),
            );
        }
        metadata
    }
}

struct Root {
    remote: Outbox<Remote>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        self.remote.send(Remote::Raise);
    }

    fn quit(&self) {
        self.remote.send(Remote::Quit);
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        crate::L10N.tr("app-name")
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "org.bande.Player".to_owned()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".to_owned()]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    remote: Outbox<Remote>,
    now: Arc<Mutex<NowPlaying>>,
}

impl Player {
    fn read<T>(&self, read: impl FnOnce(&NowPlaying) -> T, fallback: T) -> T {
        self.now.lock().map_or(fallback, |now| read(&now))
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.remote.send(Remote::Next);
    }

    fn previous(&self) {
        self.remote.send(Remote::Previous);
    }

    fn pause(&self) {
        self.remote.send(Remote::Pause);
    }

    fn play_pause(&self) {
        self.remote.send(Remote::Toggle);
    }

    fn stop(&self) {
        self.remote.send(Remote::Pause);
    }

    fn play(&self) {
        self.remote.send(Remote::Play);
    }

    fn seek(&self, offset: i64) {
        self.remote.send(Remote::Seek(offset));
    }

    fn set_position(&self, _track: ObjectPath<'_>, position: i64) {
        self.remote.send(Remote::SetPosition(position));
    }

    #[zbus(signal)]
    async fn seeked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        position: i64,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.read(|now| now.status().to_owned(), "Stopped".to_owned())
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.read(|now| now.rate, 1.0)
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.read(NowPlaying::metadata, HashMap::new())
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.read(|now| now.volume, 1.0)
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.read(|now| now.position.as_micros() as i64, 0)
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        0.5
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        2.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.read(|now| now.has_queue, false)
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.read(|now| now.has_queue, false)
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.read(|now| now.playback.is_some(), false)
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.read(|now| now.playback.is_some(), false)
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.read(|now| now.seekable, false)
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

pub struct Mpris {
    connection: Connection,
    now: Arc<Mutex<NowPlaying>>,
}

impl Mpris {
    pub fn serve(connection: Connection, remote: Outbox<Remote>) -> zbus::Result<Self> {
        let now = Arc::new(Mutex::new(NowPlaying {
            rate: 1.0,
            volume: 1.0,
            ..NowPlaying::default()
        }));
        {
            let server = connection.object_server();
            server.at(
                OBJECT_PATH,
                Root {
                    remote: remote.clone(),
                },
            )?;
            server.at(
                OBJECT_PATH,
                Player {
                    remote,
                    now: Arc::clone(&now),
                },
            )?;
        }
        connection.request_name(BUS_NAME)?;
        Ok(Self { connection, now })
    }

    pub fn update(&self, playing: Option<NowPlaying>) {
        let playing = playing.unwrap_or_default();
        let Ok(mut now) = self.now.lock() else {
            return;
        };
        let moved = now.playback == Some(Playback::Playing)
            && playing.playback == now.playback
            && playing.position.abs_diff(now.position) > SEEK_DRIFT;
        let changed = now.status() != playing.status()
            || now.metadata() != playing.metadata()
            || now.rate != playing.rate
            || now.volume != playing.volume
            || now.seekable != playing.seekable
            || now.has_queue != playing.has_queue;
        let position = playing.position;
        let same_track = now.path == playing.path;
        *now = playing;
        let properties = changed.then(|| {
            let mut properties: HashMap<&str, OwnedValue> = HashMap::new();
            let mut put = |key: &'static str, value: Value<'_>| {
                if let Ok(value) = OwnedValue::try_from(value) {
                    properties.insert(key, value);
                }
            };
            put("PlaybackStatus", Value::from(now.status()));
            put("Metadata", Value::from(now.metadata()));
            put("Rate", Value::from(now.rate));
            put("Volume", Value::from(now.volume));
            put("CanSeek", Value::from(now.seekable));
            put("CanGoNext", Value::from(now.has_queue));
            put("CanGoPrevious", Value::from(now.has_queue));
            put("CanPlay", Value::from(now.playback.is_some()));
            put("CanPause", Value::from(now.playback.is_some()));
            properties
        });
        drop(now);
        if let Some(properties) = properties {
            let body = (PLAYER_INTERFACE, properties, Vec::<String>::new());
            if let Err(error) = self.connection.emit_signal(
                None::<&str>,
                OBJECT_PATH,
                PROPERTIES_INTERFACE,
                "PropertiesChanged",
                &body,
            ) {
                tracing::warn!("cannot announce the player's state: {error}");
            }
        }
        if moved && same_track {
            let micros = position.as_micros() as i64;
            let sent = self.connection.emit_signal(
                None::<&str>,
                OBJECT_PATH,
                PLAYER_INTERFACE,
                "Seeked",
                &micros,
            );
            if let Err(error) = sent {
                tracing::warn!("cannot announce a seek: {error}");
            }
        }
    }
}
