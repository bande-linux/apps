use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use bande_inbox::Outbox;
use bande_packages::flatpak::{self, FLATHUB, Installed};
use bande_toolkit::Image;
use bande_xdg::DesktopEntry;
use jxl_oxide::JxlImage;
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig};

use crate::catalog::{self, Bundle, Component, Icon};
use crate::worker::Message;

pub const ICON_PIXELS: u32 = 128;
const ICON_FILE_LIMIT: u64 = 4 << 20;
const SHOT_LIMIT: u64 = 8 << 20;
const PICTURE_SIDE: u32 = 8192;
const PICTURE_BYTES: u64 = 256 << 20;
const TIMEOUT: Duration = Duration::from_secs(20);
const SHOT_UNUSED: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const SHOT_CACHE_BYTES: u64 = 200 << 20;
const SYSTEM_APPLICATIONS: &str = "/usr/share/applications";
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
        flatpaks: HashMap<String, Installed>,
        flathub: bool,
    },
    Icons(Vec<(String, Option<Image>)>),
    Shot {
        url: String,
        image: Result<Image, ShotError>,
    },
}

fn decode_jxl(bytes: &[u8]) -> Option<Image> {
    let image = JxlImage::builder().read(bytes).ok()?;
    if image.width() > PICTURE_SIDE || image.height() > PICTURE_SIDE {
        return None;
    }
    let render = image.render_frame(0).ok()?;
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels());
    let size = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(channels as usize)?;
    let mut samples = vec![0u8; size];
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
    let decoded = decode_limited(bytes, PICTURE_SIDE, PICTURE_BYTES)?;
    let (width, height) = decoded.dimensions();
    Image::from_rgba(width, height, decoded.into_raw())
}

fn decode_limited(bytes: &[u8], side: u32, total: u64) -> Option<image::RgbaImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(side);
    limits.max_image_height = Some(side);
    limits.max_alloc = Some(total);
    let mut reader = image::ImageReader::new(io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(limits);
    Some(reader.decode().ok()?.to_rgba8())
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

fn shot_cache() -> Option<PathBuf> {
    Some(
        bande_xdg::cache_home()?
            .join("bande-software")
            .join("screenshots"),
    )
}

fn cache_file(url: &str) -> Option<PathBuf> {
    let name = Sha256::digest(url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Some(shot_cache()?.join(name))
}

fn mark_used(path: &Path) {
    let touched = fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()));
    if let Err(error) = touched {
        tracing::debug!("cannot mark a cached screenshot as used: {error}");
    }
}

fn prune_shots() {
    let Some(folder) = shot_cache() else {
        return;
    };
    let Ok(entries) = fs::read_dir(&folder) else {
        return;
    };
    let now = SystemTime::now();
    let mut kept = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let used = metadata.modified().ok()?;
            Some((used, metadata.len(), entry.path()))
        })
        .filter(|(used, _, path)| {
            let stale = now.duration_since(*used).is_ok_and(|age| age > SHOT_UNUSED);
            !(stale && fs::remove_file(path).is_ok())
        })
        .collect::<Vec<_>>();
    kept.sort_unstable_by_key(|(used, _, _)| *used);
    let mut total = kept.iter().map(|(_, size, _)| size).sum::<u64>();
    for (_, size, path) in kept {
        if total <= SHOT_CACHE_BYTES {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

fn agent() -> ureq::Agent {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .https_only(true)
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
        ureq::Error::StatusCode(_) | ureq::Error::RequireHttpsOnly(_) => ShotError::Unreadable,
        _ => ShotError::Offline,
    })?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(SHOT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ShotError::Offline)?;
    match bytes.len() as u64 > SHOT_LIMIT {
        true => Err(ShotError::Unreadable),
        false => Ok(bytes),
    }
}

fn screenshot(agent: &ureq::Agent, url: &str) -> Result<Image, ShotError> {
    let cached = cache_file(url);
    if let Some(path) = &cached
        && let Some(bytes) = read_limited(path, SHOT_LIMIT)
    {
        if let Some(image) = decode(&bytes) {
            mark_used(path);
            return Ok(image);
        }
        if let Err(error) = fs::remove_file(path) {
            tracing::debug!("cannot remove a broken screenshot: {error}");
        }
    }
    let bytes = download(agent, url)?;
    let image = decode(&bytes).ok_or(ShotError::Unreadable)?;
    if let Some(path) = &cached
        && let Err(error) = bande_xdg::replace_file(path, &bytes)
    {
        tracing::debug!("cannot cache the screenshot: {error}");
    }
    Ok(image)
}

fn desktop_id(app: &Component) -> String {
    app.launchable
        .clone()
        .unwrap_or_else(|| format!("{}.desktop", app.key()))
}

fn entry_icon(icon: &str) -> Icon {
    match icon.starts_with('/') {
        true => Icon::Local(PathBuf::from(icon)),
        false => Icon::Stock(icon.to_owned()),
    }
}

fn entry_app(entry: &DesktopEntry, package: Option<&String>) -> Component {
    Component {
        id: entry.id.clone(),
        name: entry.name.clone(),
        summary: entry
            .comment
            .clone()
            .or_else(|| entry.generic_name.clone())
            .unwrap_or_default(),
        keywords: entry.keywords.clone(),
        icons: entry.icon.as_deref().map(entry_icon).into_iter().collect(),
        launchable: Some(entry.id.clone()),
        bundles: package.cloned().map(Bundle::Deb).into_iter().collect(),
        ..Component::default()
    }
}

fn exported(path: &Path) -> bool {
    path.components()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair[0].as_os_str() == "flatpak" && pair[1].as_os_str() == "exports")
}

fn catalogue() -> Loaded {
    let remotes = flatpak::remotes().unwrap_or_else(|error| {
        if error != flatpak::Error::Missing {
            tracing::warn!("cannot list the Flatpak remotes: {error}");
        }
        Vec::new()
    });
    let flatpaks: HashMap<String, Installed> = flatpak::installed()
        .unwrap_or_default()
        .into_iter()
        .map(|installed| (installed.id.clone(), installed))
        .collect();
    let mut apps = catalog::system_catalog(&remotes);
    let entries = bande_xdg::applications(
        &bande_xdg::data_dirs(),
        None,
        &bande_xdg::current_desktops(),
    );
    let by_id: HashMap<&str, &DesktopEntry> = entries
        .iter()
        .filter_map(|entry| Some((entry.path.file_name()?.to_str()?, entry)))
        .collect();
    let mut installed = HashSet::new();
    let mut listed = HashSet::new();
    for app in &mut apps {
        let Some(entry) = desktop_id(app)
            .split_whitespace()
            .find_map(|id| by_id.get(id))
        else {
            continue;
        };
        listed.insert(entry.path.clone());
        if !exported(&entry.path) {
            installed.insert(app.key().to_owned());
        }
        if app.icons.is_empty() {
            app.icons.extend(entry.icon.as_deref().map(entry_icon));
        }
    }
    let unlisted: Vec<&DesktopEntry> = by_id
        .values()
        .copied()
        .filter(|entry| {
            entry.path.starts_with(SYSTEM_APPLICATIONS) && !listed.contains(&entry.path)
        })
        .collect();
    let paths: Vec<PathBuf> = unlisted.iter().map(|entry| entry.path.clone()).collect();
    let owners = catalog::owners(&paths);
    for entry in unlisted {
        let app = entry_app(entry, owners.get(&entry.path));
        installed.insert(app.key().to_owned());
        apps.push(app);
    }
    let known: HashSet<String> = apps.iter().map(|app| app.key().to_owned()).collect();
    for found in flatpaks.values().filter(|found| !known.contains(&found.id)) {
        if let Some(entry) = by_id.get(format!("{}.desktop", found.id).as_str()) {
            let mut app = entry_app(entry, None);
            app.bundles.push(Bundle::Flatpak {
                remote: found.origin.clone(),
                reference: found.reference.clone(),
            });
            apps.push(app);
        }
    }
    Loaded::Catalogue {
        apps,
        installed,
        flatpaks,
        flathub: remotes.iter().any(|remote| remote == FLATHUB),
    }
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

fn flatpak_paths() -> Vec<PathBuf> {
    let Some(dir) = catalog::flatpak_user_dir() else {
        return Vec::new();
    };
    let appstream = dir.join("appstream");
    let mut paths: Vec<PathBuf> = fs::read_dir(&appstream)
        .into_iter()
        .flatten()
        .flatten()
        .map(|remote| remote.path().join(catalog::flatpak_arch()))
        .collect();
    paths.extend([
        appstream,
        dir.join(".changed"),
        dir.join("repo").join("config"),
    ]);
    paths
}

fn fingerprint() -> Option<SystemTime> {
    catalog::SYSTEM_ROOTS
        .iter()
        .flat_map(|root| [Path::new(root).join("xml"), Path::new(root).join("yaml")])
        .chain([
            PathBuf::from(DPKG_STATUS),
            PathBuf::from(catalog::METAINFO),
            PathBuf::from(SYSTEM_APPLICATIONS),
        ])
        .chain(flatpak_paths())
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

#[derive(Default)]
struct ShotQueue {
    wanted: Vec<String>,
    loading: Option<String>,
    closed: bool,
}

#[derive(Clone, Default)]
pub struct Shots(Arc<(Mutex<ShotQueue>, Condvar)>);

impl Shots {
    fn queue(&self) -> MutexGuard<'_, ShotQueue> {
        self.0.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn want(&self, url: &str) {
        let mut queue = self.queue();
        if queue.loading.as_deref() == Some(url) {
            return;
        }
        queue.wanted.retain(|known| known != url);
        queue.wanted.push(url.to_owned());
        self.0.1.notify_one();
    }

    fn next(&self) -> Option<String> {
        let mut queue = self.queue();
        queue.loading = None;
        loop {
            if queue.closed {
                return None;
            }
            if let Some(url) = queue.wanted.pop() {
                queue.loading = Some(url.clone());
                return Some(url);
            }
            queue = self.0.1.wait(queue).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn close(&self) {
        self.queue().closed = true;
        self.0.1.notify_one();
    }
}

fn load_shots(shots: &Shots, out: &Outbox<Message>) {
    let agent = agent();
    while let Some(url) = shots.next() {
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
    pub shots: Shots,
}

impl Drop for Media {
    fn drop(&mut self) {
        self.shots.close();
    }
}

pub fn start(out: &Outbox<Message>) -> io::Result<Media> {
    let (icons, icon_requests) = mpsc::channel();
    let shots = Shots::default();
    let icon_out = out.clone();
    std::thread::Builder::new()
        .name("software-icons".to_owned())
        .spawn(move || load_icons(&icon_requests, &icon_out))?;
    let shot_out = out.clone();
    let loader = shots.clone();
    std::thread::Builder::new()
        .name("software-shots".to_owned())
        .spawn(move || load_shots(&loader, &shot_out))?;
    std::thread::Builder::new()
        .name("software-cache".to_owned())
        .spawn(prune_shots)?;
    Ok(Media { icons, shots })
}
