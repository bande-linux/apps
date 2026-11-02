use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use bande_inbox::Outbox;
use bande_toolkit::Image;
use jxl_oxide::JxlImage;
use ureq::tls::{RootCerts, TlsConfig};

use crate::catalog::{self, Component, Icon};
use crate::worker::Message;

pub const ICON_PIXELS: u32 = 128;
const ICON_FILE_LIMIT: u64 = 4 << 20;
const SHOT_LIMIT: u64 = 8 << 20;
const TIMEOUT: Duration = Duration::from_secs(20);
const DPKG_STATUS: &str = "/var/lib/dpkg/status";
const USER_AGENT: &str = concat!("bande-software/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotError {
    Offline,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Loaded {
    Catalogue {
        apps: Vec<Component>,
        installed: HashSet<String>,
    },
    Icons(Vec<(String, Option<Image>)>),
    Shot {
        url: String,
        image: Result<Image, ShotError>,
    },
}

fn decode_jxl(bytes: &[u8]) -> Option<Image> {
    let image = JxlImage::builder().read(bytes).ok()?;
    let render = image.render_frame(0).ok()?;
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels());
    let mut samples = vec![0u8; (width * height * channels) as usize];
    stream.write_to_buffer(&mut samples);
    let rgba: Vec<u8> = samples
        .chunks_exact(channels as usize)
        .flat_map(|pixel| match *pixel {
            [grey] => [grey, grey, grey, u8::MAX],
            [grey, alpha] => [grey, grey, grey, alpha],
            [red, green, blue] => [red, green, blue, u8::MAX],
            [red, green, blue, alpha, ..] => [red, green, blue, alpha],
            [] => [0; 4],
        })
        .collect();
    Image::from_rgba(width, height, rgba)
}

const JXL_CODESTREAM: &[u8] = &[0xff, 0x0a];
const JXL_CONTAINER: &[u8] = b"\0\0\0\x0cJXL ";

fn decode(bytes: &[u8]) -> Option<Image> {
    if bytes.starts_with(JXL_CODESTREAM) || bytes.starts_with(JXL_CONTAINER) {
        return decode_jxl(bytes);
    }
    let decoded = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    Image::from_rgba(width, height, decoded.into_raw())
}

fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

fn load_file(path: &Path) -> Option<Image> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("jxl") => decode(&read_limited(path, ICON_FILE_LIMIT)?),
        _ => Image::load(path, ICON_PIXELS),
    }
}

fn icon_rank(icon: &Icon) -> (u8, u32) {
    match icon {
        Icon::Cached { size, .. } => (0, u32::MAX - size),
        Icon::Local(_) => (1, 0),
        Icon::Stock(_) => (2, 0),
        Icon::Remote { .. } => (3, 0),
    }
}

pub fn load_icon(icons: &[Icon], data_dirs: &[PathBuf]) -> Option<Image> {
    let mut ranked: Vec<&Icon> = icons.iter().collect();
    ranked.sort_by_key(|icon| icon_rank(icon));
    ranked.into_iter().find_map(|icon| match icon {
        Icon::Cached { path, .. } | Icon::Local(path) => load_file(path),
        Icon::Stock(name) => {
            bande_xdg::find_icon(name, ICON_PIXELS, &bande_xdg::ICON_THEMES, data_dirs)
                .and_then(|path| load_file(&path))
        }
        Icon::Remote { .. } => None,
    })
}

fn cache_file(url: &str) -> Option<PathBuf> {
    let name: String = url
        .trim_start_matches("https://")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let tail = name.get(name.len().saturating_sub(160)..).unwrap_or(&name);
    Some(
        bande_xdg::cache_home()?
            .join("bande-software")
            .join("screenshots")
            .join(tail),
    )
}

fn agent() -> ureq::Agent {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(USER_AGENT)
        .tls_config(tls)
        .build()
        .new_agent()
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, ShotError> {
    if !url.starts_with("https://") {
        return Err(ShotError::Unreadable);
    }
    let response = agent.get(url).call().map_err(|error| match error {
        ureq::Error::StatusCode(_) => ShotError::Unreadable,
        _ => ShotError::Offline,
    })?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(SHOT_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|_| ShotError::Offline)?;
    Ok(bytes)
}

fn screenshot(agent: &ureq::Agent, url: &str) -> Result<Image, ShotError> {
    let cached = cache_file(url);
    let bytes = match cached
        .as_deref()
        .and_then(|path| read_limited(path, SHOT_LIMIT))
    {
        Some(bytes) => bytes,
        None => {
            let bytes = download(agent, url)?;
            if let Some(path) = &cached
                && let Some(parent) = path.parent()
                && let Err(error) =
                    fs::create_dir_all(parent).and_then(|()| fs::write(path, &bytes))
            {
                tracing::debug!("cannot cache the screenshot: {error}");
            }
            bytes
        }
    };
    decode(&bytes).ok_or(ShotError::Unreadable)
}

fn desktop_id(app: &Component) -> String {
    app.launchable
        .clone()
        .unwrap_or_else(|| format!("{}.desktop", app.key()))
}

fn catalogue() -> Loaded {
    let mut apps = catalog::system_catalog();
    let icons: HashMap<String, Option<String>> = bande_xdg::applications(
        &bande_xdg::data_dirs(),
        None,
        &bande_xdg::current_desktops(),
    )
    .into_iter()
    .filter_map(|entry| {
        let id = entry.path.file_name()?.to_str()?.to_owned();
        Some((id, entry.icon))
    })
    .collect();
    let mut installed = HashSet::new();
    for app in &mut apps {
        let Some(icon) = desktop_id(app)
            .split_whitespace()
            .find_map(|id| icons.get(id))
        else {
            continue;
        };
        installed.insert(app.key().to_owned());
        if app.icons.is_empty()
            && let Some(icon) = icon
        {
            app.icons.push(match icon.starts_with('/') {
                true => Icon::Local(PathBuf::from(icon)),
                false => Icon::Stock(icon.clone()),
            });
        }
    }
    Loaded::Catalogue { apps, installed }
}

fn icons(requested: Vec<(String, Vec<Icon>)>, data_dirs: &[PathBuf]) -> Loaded {
    Loaded::Icons(
        requested
            .into_iter()
            .map(|(key, icons)| {
                let image = load_icon(&icons, data_dirs);
                (key, image)
            })
            .collect(),
    )
}

fn fingerprint() -> Option<SystemTime> {
    catalog::SYSTEM_ROOTS
        .iter()
        .flat_map(|root| [Path::new(root).join("xml"), Path::new(root).join("yaml")])
        .chain([
            PathBuf::from(DPKG_STATUS),
            PathBuf::from(catalog::FLATPAK_ROOT),
            PathBuf::from(catalog::METAINFO),
        ])
        .filter_map(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        })
        .max()
}

fn load_icons(requests: &Receiver<Request>, out: &Outbox<Message>) {
    let mut loaded = fingerprint();
    if !out.send(Message::Loaded(catalogue())) {
        return;
    }
    let data_dirs = bande_xdg::data_dirs();
    for request in requests {
        let reply = match request {
            Request::Icons(requested) => icons(requested, &data_dirs),
            Request::Catalogue => {
                let now = fingerprint();
                if now == loaded {
                    continue;
                }
                loaded = now;
                catalogue()
            }
        };
        if !out.send(Message::Loaded(reply)) {
            return;
        }
    }
}

fn load_shots(requests: &Receiver<String>, out: &Outbox<Message>) {
    let agent = agent();
    for url in requests {
        let image = screenshot(&agent, &url);
        if !out.send(Message::Loaded(Loaded::Shot { url, image })) {
            return;
        }
    }
}

pub enum Request {
    Catalogue,
    Icons(Vec<(String, Vec<Icon>)>),
}

pub struct Media {
    pub icons: Sender<Request>,
    pub shots: Sender<String>,
}

pub fn start(out: &Outbox<Message>) -> io::Result<Media> {
    let (icons, icon_requests) = mpsc::channel();
    let (shots, shot_requests) = mpsc::channel();
    let icon_out = out.clone();
    std::thread::Builder::new()
        .name("software-icons".to_owned())
        .spawn(move || load_icons(&icon_requests, &icon_out))?;
    let shot_out = out.clone();
    std::thread::Builder::new()
        .name("software-shots".to_owned())
        .spawn(move || load_shots(&shot_requests, &shot_out))?;
    Ok(Media { icons, shots })
}
