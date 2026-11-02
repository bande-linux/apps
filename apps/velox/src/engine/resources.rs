use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Value, json};

use super::client::{Engine, Error, api_at_least};
use super::http::{Body, Upload};
use crate::L10N;

const COMPOSE_PROJECT: &str = "com.docker.compose.project";
const HELPER_LABEL: &str = "org.bande.velox.helper";
const HELPER_IMAGE: &str = "velox-volume-helper";
const HELPER_MOUNT: &str = "/volume";
const EMPTY_TAR: [u8; 1024] = [0; 1024];
const JSON: &str = "application/json";
const TAR: &str = "application/x-tar";
const NOT_FOUND: u16 = 404;
const DOWNLOADED: f32 = 0.8;

pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn labels_filter(label: Option<&str>, extra: &[(&str, &str)]) -> String {
    let mut filters = serde_json::Map::new();
    if let Some(label) = label {
        filters.insert("label".to_owned(), json!([label]));
    }
    for (key, value) in extra {
        filters.insert((*key).to_owned(), json!([value]));
    }
    match filters.is_empty() {
        true => String::new(),
        false => format!("?filters={}", encode(&Value::Object(filters).to_string())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Platform {
    #[serde(default)]
    architecture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ImageData {
    #[serde(default)]
    platform: Option<Platform>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Manifest {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    available: bool,
    #[serde(default)]
    image_data: Option<ImageData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Image {
    pub id: String,
    #[serde(default)]
    pub repo_tags: Option<Vec<String>>,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
    #[serde(default)]
    manifests: Option<Vec<Manifest>>,
    #[serde(skip)]
    pub architectures: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ImageDetails {
    #[serde(default)]
    architecture: String,
}

impl Image {
    pub fn tags(&self) -> Vec<&str> {
        self.repo_tags
            .iter()
            .flatten()
            .map(String::as_str)
            .filter(|tag| *tag != "<none>:<none>")
            .collect()
    }

    pub fn short_id(&self) -> &str {
        let id = self.id.trim_start_matches("sha256:");
        id.get(..12).unwrap_or(id)
    }

    fn helper(&self) -> bool {
        self.labels
            .as_ref()
            .is_some_and(|labels| labels.contains_key(HELPER_LABEL))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Volume {
    pub name: String,
    #[serde(default)]
    pub driver: String,
    #[serde(default)]
    pub mountpoint: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
}

impl Volume {
    pub fn project(&self) -> Option<&str> {
        self.labels
            .as_ref()?
            .get(COMPOSE_PROJECT)
            .map(String::as_str)
            .filter(|project| !project.is_empty())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolumeList {
    #[serde(default)]
    volumes: Option<Vec<Volume>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Subnet {
    #[serde(default)]
    pub subnet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Ipam {
    #[serde(default)]
    pub config: Option<Vec<Subnet>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Network {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub driver: String,
    #[serde(rename = "IPAM", default)]
    pub ipam: Option<Ipam>,
}

impl Network {
    pub fn subnets(&self) -> Vec<&str> {
        self.ipam
            .iter()
            .filter_map(|ipam| ipam.config.as_ref())
            .flatten()
            .filter_map(|config| config.subnet.as_deref())
            .collect()
    }

    pub fn predefined(&self) -> bool {
        matches!(self.name.as_str(), "bridge" | "host" | "none" | "podman")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub total: u64,
    pub reclaimable: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskUsage {
    pub images: Usage,
    pub containers: Usage,
    pub volumes: Usage,
    pub build_cache: Usage,
    pub volume_sizes: HashMap<String, u64>,
}

impl DiskUsage {
    pub fn total(&self) -> u64 {
        self.images.total + self.containers.total + self.volumes.total + self.build_cache.total
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsedImage {
    #[serde(default)]
    containers: i64,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    shared_size: i64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsedContainer {
    #[serde(default)]
    size_rw: i64,
    #[serde(default)]
    state: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolumeData {
    #[serde(default)]
    ref_count: i64,
    #[serde(default)]
    size: i64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsedVolume {
    name: String,
    #[serde(default)]
    usage_data: Option<VolumeData>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsedCache {
    #[serde(default)]
    size: i64,
    #[serde(default)]
    in_use: bool,
    #[serde(default)]
    shared: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Group<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawUsage {
    #[serde(default)]
    layers_size: Option<i64>,
    #[serde(default)]
    images: Option<Vec<UsedImage>>,
    #[serde(default)]
    containers: Option<Vec<UsedContainer>>,
    #[serde(default)]
    volumes: Option<Vec<UsedVolume>>,
    #[serde(default)]
    build_cache: Option<Vec<UsedCache>>,
    #[serde(default)]
    image_usage: Option<Group<UsedImage>>,
    #[serde(default)]
    container_usage: Option<Group<UsedContainer>>,
    #[serde(default)]
    volume_usage: Option<Group<UsedVolume>>,
    #[serde(default)]
    build_cache_usage: Option<Group<UsedCache>>,
}

fn bytes(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn items<T>(legacy: Option<Vec<T>>, grouped: Option<Group<T>>) -> Vec<T> {
    legacy
        .filter(|items| !items.is_empty())
        .or_else(|| grouped.map(|group| group.items))
        .unwrap_or_default()
}

impl From<RawUsage> for DiskUsage {
    fn from(raw: RawUsage) -> Self {
        let images = items(raw.images, raw.image_usage);
        let containers = items(raw.containers, raw.container_usage);
        let volumes = items(raw.volumes, raw.volume_usage);
        let caches = items(raw.build_cache, raw.build_cache_usage);
        let image_total = raw
            .layers_size
            .map_or_else(|| images.iter().map(|image| bytes(image.size)).sum(), bytes);
        let image_used: u64 = images
            .iter()
            .filter(|image| image.containers > 0 && image.shared_size >= 0)
            .map(|image| bytes(image.size - image.shared_size))
            .sum();
        let stopped = |container: &&UsedContainer| {
            !matches!(
                container.state.as_str(),
                "running" | "paused" | "restarting"
            )
        };
        let volume_sizes: HashMap<String, u64> = volumes
            .iter()
            .filter_map(|volume| {
                let data = volume.usage_data.as_ref()?;
                (data.size >= 0).then(|| (volume.name.clone(), bytes(data.size)))
            })
            .collect();
        let unused_volumes = volumes
            .iter()
            .filter_map(|volume| volume.usage_data.as_ref())
            .filter(|data| data.ref_count == 0)
            .map(|data| bytes(data.size))
            .sum();
        let own_caches = || caches.iter().filter(|cache| !cache.shared);
        Self {
            images: Usage {
                total: image_total,
                reclaimable: image_total.saturating_sub(image_used),
            },
            containers: Usage {
                total: containers.iter().map(|item| bytes(item.size_rw)).sum(),
                reclaimable: containers
                    .iter()
                    .filter(stopped)
                    .map(|item| bytes(item.size_rw))
                    .sum(),
            },
            volumes: Usage {
                total: volume_sizes.values().sum(),
                reclaimable: unused_volumes,
            },
            build_cache: Usage {
                total: own_caches().map(|cache| bytes(cache.size)).sum(),
                reclaimable: own_caches()
                    .filter(|cache| !cache.in_use)
                    .map(|cache| bytes(cache.size))
                    .sum(),
            },
            volume_sizes,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Pruned {
    #[serde(default)]
    space_reclaimed: i64,
}

#[derive(Default, Deserialize)]
struct Detail {
    #[serde(default)]
    current: Option<u64>,
    #[serde(default)]
    total: Option<u64>,
}

#[derive(Deserialize)]
struct PullLine {
    #[serde(default)]
    status: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "progressDetail", default)]
    detail: Option<Detail>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pulling {
    pub status: String,
    pub fraction: Option<f32>,
}

#[derive(Default, Deserialize)]
struct CpuUsage {
    #[serde(default)]
    total_usage: u64,
    #[serde(default)]
    percpu_usage: Option<Vec<u64>>,
}

#[derive(Default, Deserialize)]
struct Cpu {
    #[serde(default)]
    cpu_usage: CpuUsage,
    #[serde(default)]
    system_cpu_usage: Option<u64>,
    #[serde(default)]
    online_cpus: Option<u64>,
}

#[derive(Default, Deserialize)]
struct Memory {
    #[serde(default)]
    usage: Option<u64>,
    #[serde(default)]
    stats: Option<HashMap<String, u64>>,
}

#[derive(Default, Deserialize)]
struct NetCount {
    #[serde(default)]
    rx_bytes: u64,
    #[serde(default)]
    tx_bytes: u64,
}

#[derive(Deserialize)]
struct BlockCount {
    #[serde(default)]
    op: String,
    #[serde(default)]
    value: u64,
}

#[derive(Default, Deserialize)]
struct Block {
    #[serde(default)]
    io_service_bytes_recursive: Option<Vec<BlockCount>>,
}

#[derive(Deserialize)]
struct RawStats {
    #[serde(default)]
    cpu_stats: Cpu,
    #[serde(default)]
    precpu_stats: Cpu,
    #[serde(default)]
    memory_stats: Memory,
    #[serde(default)]
    networks: Option<HashMap<String, NetCount>>,
    #[serde(default)]
    blkio_stats: Block,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub at: Instant,
    pub cpu: f64,
    pub memory: u64,
    pub read: u64,
    pub written: u64,
    pub received: u64,
    pub sent: u64,
}

impl Sample {
    fn new(raw: RawStats, at: Instant) -> Self {
        let cpu = &raw.cpu_stats;
        let before = &raw.precpu_stats;
        let busy = cpu
            .cpu_usage
            .total_usage
            .saturating_sub(before.cpu_usage.total_usage) as f64;
        let system = cpu
            .system_cpu_usage
            .unwrap_or(0)
            .saturating_sub(before.system_cpu_usage.unwrap_or(0)) as f64;
        let cores = cpu
            .online_cpus
            .or_else(|| {
                cpu.cpu_usage
                    .percpu_usage
                    .as_ref()
                    .map(|all| all.len() as u64)
            })
            .unwrap_or(1) as f64;
        let stats = raw.memory_stats.stats.unwrap_or_default();
        let cache = stats
            .get("inactive_file")
            .or_else(|| stats.get("total_inactive_file"))
            .copied()
            .unwrap_or(0);
        let block = raw
            .blkio_stats
            .io_service_bytes_recursive
            .unwrap_or_default();
        let counted = |op: &str| {
            block
                .iter()
                .filter(|count| count.op.eq_ignore_ascii_case(op))
                .map(|count| count.value)
                .sum()
        };
        let networks = raw.networks.unwrap_or_default();
        Self {
            at,
            cpu: match system > 0.0 {
                true => busy / system * cores * 100.0,
                false => 0.0,
            },
            memory: raw.memory_stats.usage.unwrap_or(0).saturating_sub(cache),
            read: counted("read"),
            written: counted("write"),
            received: networks.values().map(|net| net.rx_bytes).sum(),
            sent: networks.values().map(|net| net.tx_bytes).sum(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    No,
    Always,
    UnlessStopped,
    OnFailure,
}

impl Restart {
    pub const ALL: [Self; 4] = [Self::No, Self::Always, Self::UnlessStopped, Self::OnFailure];

    fn name(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::UnlessStopped => "unless-stopped",
            Self::OnFailure => "on-failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub image: String,
    pub name: String,
    pub ports: String,
    pub env: String,
    pub volumes: String,
    pub restart: Restart,
    pub remove: bool,
}

fn entries(text: &str) -> impl Iterator<Item = &str> {
    text.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
}

fn port_number(text: &str) -> Option<u16> {
    text.parse().ok().filter(|port| *port > 0)
}

pub fn parse_ports(text: &str) -> Result<Vec<(String, String, String)>, String> {
    text.split([',', ' '])
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (mapping, protocol) = entry.split_once('/').unwrap_or((entry, "tcp"));
            let parts: Vec<&str> = mapping.rsplitn(3, ':').collect();
            let protocol = protocol.to_ascii_lowercase();
            let valid = matches!(protocol.as_str(), "tcp" | "udp" | "sctp");
            let parsed = match parts.as_slice() {
                [inside] => port_number(inside).map(|_| (String::new(), String::new())),
                [inside, host] => port_number(inside)
                    .and(port_number(host))
                    .map(|_| (String::new(), (*host).to_owned())),
                [inside, host, address] => port_number(inside).and(port_number(host)).map(|_| {
                    (
                        address.trim_matches(['[', ']']).to_owned(),
                        (*host).to_owned(),
                    )
                }),
                _ => None,
            };
            match (parsed, parts.first(), valid) {
                (Some((address, host)), Some(inside), true) => {
                    Ok((format!("{inside}/{protocol}"), address, host))
                }
                _ => Err(entry.to_owned()),
            }
        })
        .collect()
}

impl RunSpec {
    pub fn body(&self) -> Result<Value, String> {
        let ports = parse_ports(&self.ports)?;
        let mut exposed = serde_json::Map::new();
        let mut bindings = serde_json::Map::new();
        for (inside, address, host) in ports {
            exposed.insert(inside.clone(), json!({}));
            let binding = json!({"HostIp": address, "HostPort": host});
            match bindings.get_mut(&inside).and_then(Value::as_array_mut) {
                Some(list) => list.push(binding),
                None => {
                    bindings.insert(inside, json!([binding]));
                }
            }
        }
        let env: Vec<&str> = entries(&self.env).collect();
        if let Some(bad) = env.iter().find(|entry| !entry.contains('=')) {
            return Err((*bad).to_owned());
        }
        let binds: Vec<&str> = entries(&self.volumes).collect();
        if let Some(bad) = binds.iter().find(|bind| !bind.contains(':')) {
            return Err((*bad).to_owned());
        }
        Ok(json!({
            "Image": self.image,
            "Env": env,
            "ExposedPorts": exposed,
            "HostConfig": {
                "PortBindings": bindings,
                "Binds": binds,
                "RestartPolicy": {"Name": self.restart.name()},
                "AutoRemove": self.remove,
            },
        }))
    }
}

fn split_reference(reference: &str) -> (&str, &str) {
    if reference.contains('@') {
        return (reference, "");
    }
    match reference.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => (name, tag),
        _ => (reference, "latest"),
    }
}

fn drain(mut body: Body<UnixStream>) -> Result<(), Error> {
    io::copy(&mut body, &mut io::sink())?;
    Ok(())
}

impl Engine {
    fn send_json(&self, method: &str, path: &str, body: &Value) -> Result<Body<UnixStream>, Error> {
        let bytes = body.to_string().into_bytes();
        self.request(
            method,
            path,
            Some(Upload {
                kind: JSON,
                length: bytes.len() as u64,
                data: &mut bytes.as_slice(),
            }),
        )
    }

    fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, Error> {
        let mut bytes = Vec::new();
        self.send_json("POST", path, body)?
            .read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub(super) fn post(&self, path: &str) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        self.request("POST", path, None)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(super) fn delete(&self, path: &str) -> Result<(), Error> {
        drain(self.request("DELETE", path, None)?)
    }

    pub fn images(&self) -> Result<Vec<Image>, Error> {
        let listed: Vec<Image> = self.get("/images/json?manifests=1")?;
        let mut images = Vec::with_capacity(listed.len());
        for mut image in listed.into_iter().filter(|image| !image.helper()) {
            image.architectures = match &image.manifests {
                Some(manifests) => manifests
                    .iter()
                    .filter(|manifest| manifest.available && manifest.kind == "image")
                    .filter_map(|manifest| manifest.image_data.as_ref()?.platform.as_ref())
                    .map(|platform| platform.architecture.clone())
                    .filter(|arch| !arch.is_empty() && arch != "unknown")
                    .collect(),
                None => {
                    let details: ImageDetails =
                        self.get(&format!("/images/{}/json", encode(&image.id)))?;
                    BTreeSet::from_iter(Some(details.architecture).filter(|arch| !arch.is_empty()))
                }
            };
            images.push(image);
        }
        Ok(images)
    }

    pub fn volumes(&self) -> Result<Vec<Volume>, Error> {
        let list: VolumeList = self.get("/volumes")?;
        Ok(list.volumes.unwrap_or_default())
    }

    pub fn networks(&self) -> Result<Vec<Network>, Error> {
        self.get("/networks")
    }

    pub fn disk_usage(&self) -> Result<DiskUsage, Error> {
        let raw: RawUsage = self.get("/system/df")?;
        Ok(raw.into())
    }

    pub fn pull(&self, reference: &str, mut progress: impl FnMut(Pulling)) -> Result<(), Error> {
        let (name, tag) = split_reference(reference);
        let mut path = format!("/images/create?fromImage={}", encode(name));
        if !tag.is_empty() {
            path.push_str(&format!("&tag={}", encode(tag)));
        }
        let body = self.request("POST", &path, None)?;
        let mut layers: HashMap<String, f32> = HashMap::new();
        let mut shown = None;
        for line in BufReader::new(body).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let line: PullLine = serde_json::from_str(&line)?;
            if let Some(error) = line.error {
                return Err(Error::Status(500, error));
            }
            let detail = line.detail.unwrap_or_default();
            let done = match (line.status.as_str(), detail.current, detail.total) {
                ("Pulling fs layer" | "Waiting", _, _) => Some(0.0),
                ("Downloading", Some(current), Some(total)) if total > 0 => {
                    Some(DOWNLOADED * current as f32 / total as f32)
                }
                ("Verifying Checksum" | "Download complete" | "Extracting", _, _) => {
                    Some(DOWNLOADED)
                }
                ("Pull complete" | "Already exists", _, _) => Some(1.0),
                _ => None,
            };
            if let (Some(id), Some(done)) = (&line.id, done) {
                let layer = layers.entry(id.clone()).or_default();
                *layer = layer.max(done);
            }
            let fraction =
                (!layers.is_empty()).then(|| layers.values().sum::<f32>() / layers.len() as f32);
            let percent = fraction.map(|fraction| (fraction * 100.0).round() as u32);
            let status = match line.id {
                Some(_) if done.is_some() => L10N.tr("pull-downloading"),
                _ => line.status,
            };
            let key = (status.clone(), percent);
            if shown.as_ref() != Some(&key) {
                shown = Some(key);
                progress(Pulling { status, fraction });
            }
        }
        Ok(())
    }

    pub fn run(&self, spec: &RunSpec) -> Result<String, Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Created {
            id: String,
        }
        let body = spec
            .body()
            .map_err(|entry| Error::Status(400, format!("cannot read “{entry}”")))?;
        let mut path = "/containers/create".to_owned();
        if !spec.name.trim().is_empty() {
            path.push_str(&format!("?name={}", encode(spec.name.trim())));
        }
        let created: Created = self.post_json(&path, &body)?;
        self.post(&format!("/containers/{}/start", encode(&created.id)))?;
        Ok(created.id)
    }

    pub fn remove_image(&self, reference: &str) -> Result<(), Error> {
        self.delete(&format!("/images/{}", encode(reference)))
    }

    pub fn remove_volume(&self, name: &str) -> Result<(), Error> {
        self.delete(&format!("/volumes/{}", encode(name)))
    }

    pub fn remove_network(&self, id: &str) -> Result<(), Error> {
        self.delete(&format!("/networks/{}", encode(id)))
    }

    pub fn create_network(&self, name: &str, subnet: &str) -> Result<(), Error> {
        let mut body = json!({"Name": name, "Driver": "bridge", "CheckDuplicate": true});
        if !subnet.trim().is_empty() {
            body["IPAM"] = json!({"Config": [{"Subnet": subnet.trim()}]});
        }
        drain(self.send_json("POST", "/networks/create", &body)?)
    }

    fn prune(&self, path: &str) -> Result<u64, Error> {
        match self.post(path) {
            Ok(bytes) => Ok(bytes_of(&serde_json::from_slice::<Pruned>(&bytes)?)),
            Err(Error::Status(NOT_FOUND, _)) => Ok(0),
            Err(error) => Err(error),
        }
    }

    pub fn prune_containers(&self, label: Option<&str>) -> Result<u64, Error> {
        self.prune(&format!("/containers/prune{}", labels_filter(label, &[])))
    }

    pub fn prune_images(&self, label: Option<&str>) -> Result<u64, Error> {
        self.prune(&format!(
            "/images/prune{}",
            labels_filter(label, &[("dangling", "false")])
        ))
    }

    pub fn prune_volumes(&self, label: Option<&str>, api: &str) -> Result<u64, Error> {
        let all: &[(&str, &str)] = match api_at_least(api, (1, 42)) {
            true => &[("all", "true")],
            false => &[],
        };
        self.prune(&format!("/volumes/prune{}", labels_filter(label, all)))
    }

    pub fn prune_build_cache(&self) -> Result<u64, Error> {
        self.prune("/build/prune")
    }

    pub fn stats(&self, id: &str) -> Result<(UnixStream, impl Iterator<Item = Sample>), Error> {
        let (handle, body) = self.stream(&format!("/containers/{}/stats?stream=1", encode(id)))?;
        let samples = serde_json::Deserializer::from_reader(body)
            .into_iter::<RawStats>()
            .map_while(Result::ok)
            .map(|raw| Sample::new(raw, Instant::now()));
        Ok((handle, samples))
    }

    pub fn create_volume(&self, name: &str) -> Result<(), Error> {
        drain(self.send_json("POST", "/volumes/create", &json!({"Name": name}))?)
    }

    fn helper(&self, volume: &str) -> Result<Helper<'_>, Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Created {
            id: String,
        }
        let changes = encode(&format!("LABEL {HELPER_LABEL}=1"));
        let path =
            format!("/images/create?fromSrc=-&repo={HELPER_IMAGE}&tag=latest&changes={changes}");
        drain(self.request(
            "POST",
            &path,
            Some(Upload {
                kind: TAR,
                length: EMPTY_TAR.len() as u64,
                data: &mut EMPTY_TAR.as_slice(),
            }),
        )?)?;
        let body = json!({
            "Image": HELPER_IMAGE,
            "Cmd": ["velox"],
            "Labels": {HELPER_LABEL: "1"},
            "NetworkDisabled": true,
            "HostConfig": {"Binds": [format!("{volume}:{HELPER_MOUNT}")]},
        });
        let created: Created = self.post_json("/containers/create", &body)?;
        Ok(Helper {
            engine: self,
            id: created.id,
        })
    }

    pub fn export_volume(&self, volume: &str, archive: &Path) -> Result<(), Error> {
        let helper = self.helper(volume)?;
        let mut body = self.request(
            "GET",
            &format!(
                "/containers/{}/archive?path={}",
                encode(&helper.id),
                encode(&format!("{HELPER_MOUNT}/."))
            ),
            None,
        )?;
        let mut file = File::create(archive)?;
        io::copy(&mut body, &mut file)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }

    pub fn import_volume(&self, volume: &str, archive: &Path) -> Result<(), Error> {
        let mut file = File::open(archive)?;
        let length = file.metadata()?.len();
        self.create_volume(volume)?;
        let helper = self.helper(volume)?;
        drain(self.request(
            "PUT",
            &format!(
                "/containers/{}/archive?path={}",
                encode(&helper.id),
                encode(HELPER_MOUNT)
            ),
            Some(Upload {
                kind: TAR,
                length,
                data: &mut file,
            }),
        )?)
    }
}

fn bytes_of(pruned: &Pruned) -> u64 {
    bytes(pruned.space_reclaimed)
}

struct Helper<'a> {
    engine: &'a Engine,
    id: String,
}

impl Drop for Helper<'_> {
    fn drop(&mut self) {
        let removed = self
            .engine
            .delete(&format!("/containers/{}?v=0&force=1", encode(&self.id)))
            .and_then(|()| self.engine.remove_image(HELPER_IMAGE));
        if let Err(error) = removed {
            tracing::debug!("cannot remove the volume helper: {error}");
        }
    }
}
