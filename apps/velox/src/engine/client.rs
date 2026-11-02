use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::http::{self, Body};
use super::informer::Action;

const COMPOSE_PROJECT: &str = "com.docker.compose.project";
const MIN_API: (u32, u32) = (1, 41);
const PODMAN_COMPONENT: &str = "Podman Engine";
const ROOTLESS_OPTION: &str = "name=rootless";
const CONTAINER_EVENTS: &str = "/events?filters=%7B%22type%22%3A%5B%22container%22%5D%7D";

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
pub struct Attachment {
    #[serde(rename = "IPAddress", default)]
    pub ip_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Networks {
    #[serde(default)]
    pub networks: Option<HashMap<String, Attachment>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Container {
    pub id: String,
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
    #[serde(default)]
    pub network_settings: Option<Networks>,
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
    pub rootless: bool,
    pub supported: bool,
    pub socket: PathBuf,
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
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Debug, Deserialize)]
pub struct Event {
    #[serde(rename = "Action", default)]
    pub action: String,
    #[serde(rename = "Actor")]
    pub actor: Option<Actor>,
}

#[derive(Debug, Deserialize)]
pub struct Actor {
    #[serde(rename = "ID")]
    pub id: String,
}

fn api_supported(version: &str) -> bool {
    let mut parts = version.split('.').map(|part| part.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) >= MIN_API,
        _ => false,
    }
}

pub(crate) fn is_container_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|char| char.is_ascii_alphanumeric())
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

    pub(crate) fn call(&self, method: &str, path: &str) -> Result<Body<UnixStream>, Error> {
        let response = http::exchange(UnixStream::connect(&self.socket)?, method, path)?;
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
            rootless: info
                .security_options
                .iter()
                .flatten()
                .any(|option| option == ROOTLESS_OPTION),
            supported: api_supported(&version.api_version),
            socket: self.socket.clone(),
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

    fn post(&self, method: &str, path: &str) -> Result<(), Error> {
        let mut drained = Vec::new();
        self.call(method, path)?.read_to_end(&mut drained)?;
        Ok(())
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
                self.post("POST", &format!("/containers/{id}/stop"))?;
                return self.post("DELETE", &format!("/containers/{id}?force=1"));
            }
        };
        self.post("POST", &format!("/containers/{id}/{verb}"))
    }

    pub fn stream(&self, path: &str) -> Result<(UnixStream, Body<UnixStream>), Error> {
        let stream = UnixStream::connect(&self.socket)?;
        let handle = stream.try_clone()?;
        let response = http::exchange(stream, "GET", path)?;
        match response.status {
            200 => Ok((handle, response.body)),
            status => Err(Error::Status(status, format!("the engine refused {path}"))),
        }
    }

    pub fn events(&self) -> Result<(UnixStream, Body<UnixStream>), Error> {
        self.stream(CONTAINER_EVENTS)
    }
}
