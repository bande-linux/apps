use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::http::{self, Body, Upload};
use super::informer::Action;

const COMPOSE_PROJECT: &str = "com.docker.compose.project";
const MIN_API: (u32, u32) = (1, 41);
const PODMAN_COMPONENT: &str = "Podman Engine";
const ROOTLESS_OPTION: &str = "name=rootless";
const EVENTS: &str = "/events?filters=%7B%22type%22%3A%5B%22container%22%2C%22image%22%2C%22volume%22%2C%22network%22%5D%7D";

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Status(u16, String),
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Status(status, message) => write!(formatter, "{message} ({status})"),
            Self::Json(error) => write!(formatter, "unexpected answer: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Port {
    #[serde(default, rename = "IP")]
    pub ip: Option<String>,
    #[serde(default)]
    pub private_port: u16,
    #[serde(default)]
    pub public_port: Option<u16>,
    #[serde(default, rename = "Type")]
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Mount {
    #[serde(rename = "Type", default)]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub destination: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Endpoint {
    #[serde(rename = "NetworkID", default)]
    pub network_id: String,
    #[serde(rename = "IPAddress", default)]
    pub ip_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NetworkSettings {
    #[serde(default)]
    pub networks: Option<HashMap<String, Endpoint>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Container {
    pub id: String,
    #[serde(rename = "ImageID", default)]
    pub image_id: String,
    #[serde(default)]
    pub mounts: Option<Vec<Mount>>,
    #[serde(default)]
    pub network_settings: Option<NetworkSettings>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub ports: Option<Vec<Port>>,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
}

impl Container {
    pub fn name(&self) -> &str {
        self.names
            .first()
            .map_or(self.id.get(..12).unwrap_or(&self.id), |name| {
                name.trim_start_matches('/')
            })
    }

    pub fn project(&self) -> Option<&str> {
        self.labels
            .as_ref()?
            .get(COMPOSE_PROJECT)
            .map(String::as_str)
            .filter(|project| !project.is_empty())
    }

    pub fn running(&self) -> bool {
        matches!(self.state.as_str(), "running" | "restarting")
    }

    pub fn paused(&self) -> bool {
        self.state == "paused"
    }

    pub fn label(&self, name: &str) -> Option<&str> {
        self.labels.as_ref()?.get(name).map(String::as_str)
    }

    pub fn ip(&self) -> Option<&str> {
        self.network_settings
            .as_ref()?
            .networks
            .as_ref()?
            .values()
            .map(|network| network.ip_address.as_str())
            .find(|ip| !ip.is_empty())
    }

    pub fn published_ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self
            .ports
            .iter()
            .flatten()
            .filter_map(|port| port.public_port)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flavor {
    Docker,
    Podman,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInfo {
    pub flavor: Flavor,
    pub version: String,
    pub api: String,
    pub rootless: bool,
    pub supported: bool,
    pub socket: PathBuf,
    pub cpus: u32,
    pub memory: u64,
    pub disk: Option<u64>,
    pub started: Option<SystemTime>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Component {
    name: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Version {
    version: String,
    api_version: String,
    #[serde(default)]
    components: Option<Vec<Component>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Info {
    #[serde(default)]
    security_options: Option<Vec<String>>,
    #[serde(default, rename = "NCPU")]
    cpus: u32,
    #[serde(default)]
    mem_total: u64,
    #[serde(default)]
    docker_root_dir: Option<PathBuf>,
}

fn disk_size(root: &Path) -> Option<u64> {
    root.ancestors().find_map(|folder| {
        rustix::fs::statvfs(folder)
            .ok()
            .map(|stat| stat.f_blocks * stat.f_frsize)
    })
}

fn started(socket: &Path) -> Option<SystemTime> {
    let stream = UnixStream::connect(socket).ok()?;
    let pid = rustix::net::sockopt::socket_peercred(&stream).ok()?.pid;
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero())).ok()?;
    let after_name = stat.rsplit_once(')')?.1;
    let ticks: u64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let booted: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let per_second = rustix::param::clock_ticks_per_second() as f64;
    let running = (booted - ticks as f64 / per_second).max(0.0);
    SystemTime::now().checked_sub(Duration::from_secs_f64(running))
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Debug, Deserialize)]
pub struct Event {
    #[serde(rename = "Type", default)]
    pub kind: String,
    #[serde(rename = "Action", default)]
    pub action: String,
    #[serde(rename = "Actor")]
    pub actor: Option<Actor>,
}

#[derive(Debug, Deserialize)]
pub struct Actor {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Attributes", default)]
    pub attributes: HashMap<String, String>,
}

pub(super) fn api_at_least(version: &str, wanted: (u32, u32)) -> bool {
    let mut parts = version.split('.').map(|part| part.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) >= wanted,
        _ => false,
    }
}

pub(crate) fn is_container_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|char| char.is_ascii_alphanumeric())
}

#[derive(Deserialize)]
struct ExecCreated {
    #[serde(rename = "Id")]
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    socket: PathBuf,
}

impl Engine {
    pub fn new(socket: &Path) -> Self {
        Self {
            socket: socket.to_path_buf(),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    fn call(&self, method: &str, path: &str) -> Result<Body<UnixStream>, Error> {
        self.request(method, path, None)
    }

    pub(super) fn request(
        &self,
        method: &str,
        path: &str,
        upload: Option<Upload<'_>>,
    ) -> Result<Body<UnixStream>, Error> {
        let response = http::send(UnixStream::connect(&self.socket)?, method, path, upload)?;
        let mut body = response.body;
        if (200..300).contains(&response.status) || response.status == 304 {
            return Ok(body);
        }
        let mut text = Vec::new();
        body.read_to_end(&mut text)?;
        let message = serde_json::from_slice::<ErrorBody>(&text)
            .map(|error| error.message)
            .unwrap_or_else(|_| String::from_utf8_lossy(&text).trim().to_owned());
        Err(Error::Status(response.status, message))
    }

    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let mut bytes = Vec::new();
        self.call("GET", path)?.read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn info(&self) -> Result<EngineInfo, Error> {
        let version: Version = self.get("/version")?;
        let info: Info = self.get("/info")?;
        let podman = version
            .components
            .iter()
            .flatten()
            .find(|component| component.name == PODMAN_COMPONENT);
        Ok(EngineInfo {
            flavor: match podman {
                Some(_) => Flavor::Podman,
                None => Flavor::Docker,
            },
            version: podman.map_or(version.version, |podman| podman.version.clone()),
            api: version.api_version.clone(),
            rootless: info
                .security_options
                .iter()
                .flatten()
                .any(|option| option == ROOTLESS_OPTION),
            supported: api_at_least(&version.api_version, MIN_API),
            socket: self.socket.clone(),
            cpus: info.cpus,
            memory: info.mem_total,
            disk: info.docker_root_dir.as_deref().and_then(disk_size),
            started: started(&self.socket),
        })
    }

    pub fn containers(&self) -> Result<Vec<Container>, Error> {
        self.get("/containers/json?all=1")
    }

    pub fn container(&self, id: &str) -> Result<Option<Container>, Error> {
        if !is_container_id(id) {
            return Ok(None);
        }
        let found: Vec<Container> = self.get(&format!(
            "/containers/json?all=1&filters=%7B%22id%22%3A%5B%22{id}%22%5D%7D"
        ))?;
        Ok(found.into_iter().find(|container| container.id == id))
    }

    pub fn act(&self, id: &str, action: Action) -> Result<(), Error> {
        if !is_container_id(id) {
            return Ok(());
        }
        let verb = match action {
            Action::Start => "start",
            Action::Stop => "stop",
            Action::Restart => "restart",
            Action::Pause => "pause",
            Action::Resume => "unpause",
            Action::Kill => "kill",
            Action::Remove => {
                self.post(&format!("/containers/{id}/stop"))?;
                return self.delete(&format!("/containers/{id}?force=1"));
            }
        };
        self.post(&format!("/containers/{id}/{verb}")).map(drop)
    }

    pub(crate) fn stream(&self, path: &str) -> Result<(UnixStream, Body<UnixStream>), Error> {
        let stream = UnixStream::connect(&self.socket)?;
        let handle = stream.try_clone()?;
        let response = http::exchange(stream, "GET", path)?;
        match response.status {
            200 => Ok((handle, response.body)),
            status => Err(Error::Status(status, format!("the engine refused {path}"))),
        }
    }

    pub fn create_exec(&self, container: &str, shells: &[&str]) -> Result<String, Error> {
        let command = shells
            .iter()
            .map(|shell| format!("[ -x {shell} ] && exec {shell};"))
            .collect::<String>();
        let body = serde_json::to_vec(&serde_json::json!({
            "AttachStdin": true,
            "AttachStdout": true,
            "AttachStderr": true,
            "Tty": true,
            "Cmd": ["/bin/sh", "-c", command],
            "Env": ["TERM=xterm-256color"],
        }))?;
        let mut data: &[u8] = &body;
        let mut reply = Vec::new();
        self.request(
            "POST",
            &format!("/containers/{container}/exec"),
            Some(Upload {
                kind: "application/json",
                length: body.len() as u64,
                data: &mut data,
            }),
        )?
        .read_to_end(&mut reply)?;
        let created: ExecCreated = serde_json::from_slice(&reply)?;
        Ok(created.id)
    }

    pub fn start_exec(&self, id: &str) -> Result<UnixStream, Error> {
        let mut stream = UnixStream::connect(&self.socket)?;
        let body = r#"{"Detach":false,"Tty":true}"#;
        write!(
            stream,
            "POST /exec/{id}/start HTTP/1.1\r\nHost: docker\r\nUser-Agent: Velox\r\n\
             Connection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        )?;
        let mut head = Vec::new();
        let mut byte = [0; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte)? == 0 {
                return Err(Error::Status(0, "the engine closed the shell".to_owned()));
            }
            head.push(byte[0]);
        }
        let status = String::from_utf8_lossy(&head)
            .split_whitespace()
            .nth(1)
            .and_then(|status| status.parse::<u16>().ok())
            .unwrap_or_default();
        match status {
            101 | 200 => Ok(stream),
            status => Err(Error::Status(
                status,
                "the engine refused the shell".to_owned(),
            )),
        }
    }

    pub fn resize_exec(&self, id: &str, columns: usize, lines: usize) -> Result<(), Error> {
        self.post(&format!("/exec/{id}/resize?h={lines}&w={columns}"))
            .map(drop)
    }

    pub fn events(&self) -> Result<(UnixStream, Body<UnixStream>), Error> {
        self.stream(EVENTS)
    }
}
