use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use std::os::fd::OwnedFd;

use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const HOSTS_FILE: &str = "bande/velox-hosts.json";
const DEFAULT_CONTEXT: &str = "default";
const CURRENT_CONTEXT: &str = "currentContext";
const UNIX_SCHEME: &str = "unix://";
const SSH_SCHEME: &str = "ssh://";
pub const REMOTE_SOCKET: &str = "/var/run/docker.sock";
const SSH_PORT: u16 = 22;
const KEEPALIVE_SECONDS: u32 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteHost {
    pub id: String,
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub socket: String,
}

impl RemoteHost {
    pub fn ssh_target(&self) -> String {
        match self.user.is_empty() {
            true => self.host.clone(),
            false => format!("{}@{}", self.user, self.host),
        }
    }

    pub fn ssh_command(&self) -> Vec<String> {
        let mut command = vec!["ssh".to_owned()];
        if self.port != SSH_PORT {
            command.extend(["-p".to_owned(), self.port.to_string()]);
        }
        command.push(self.ssh_target());
        command
    }
}

fn hosts_file() -> Option<PathBuf> {
    bande_xdg::config_home().map(|config| config.join(HOSTS_FILE))
}

pub fn load_hosts() -> Vec<RemoteHost> {
    hosts_file()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_hosts(hosts: &[RemoteHost]) -> io::Result<()> {
    let path = hosts_file().ok_or_else(|| io::Error::other("no configuration folder"))?;
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    bande_xdg::replace_file(&path, serde_json::to_vec_pretty(hosts)?)
}

pub fn new_host_id() -> String {
    let mut bytes = [0u8; 4];
    if rustix::rand::getrandom(&mut bytes, rustix::rand::GetRandomFlags::empty()).is_err() {
        bytes = std::process::id().to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Local(Option<PathBuf>),
    Ssh(RemoteHost),
    Unsupported(String),
}

pub fn endpoint(host: &str, id: &str) -> Endpoint {
    if let Some(path) = host.strip_prefix(UNIX_SCHEME) {
        return Endpoint::Local(Some(PathBuf::from(path)));
    }
    let Some(rest) = host.strip_prefix(SSH_SCHEME) else {
        return Endpoint::Unsupported(host.to_owned());
    };
    let rest = rest.trim_end_matches('/');
    let (user, address) = match rest.rsplit_once('@') {
        Some((user, address)) => (user.to_owned(), address),
        None => (String::new(), rest),
    };
    let (address, port) = match address.rsplit_once(':') {
        Some((address, port)) if !address.ends_with(']') || address.starts_with('[') => {
            (address, port.parse().unwrap_or(SSH_PORT))
        }
        _ => (address, SSH_PORT),
    };
    Endpoint::Ssh(RemoteHost {
        id: id.to_owned(),
        name: address.to_owned(),
        user,
        host: address
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        port,
        socket: REMOTE_SOCKET.to_owned(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerContext {
    pub name: String,
    pub host: String,
}

#[derive(Deserialize)]
struct ContextMeta {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Endpoints")]
    endpoints: Endpoints,
}

#[derive(Deserialize)]
struct Endpoints {
    docker: DockerEndpoint,
}

#[derive(Deserialize)]
struct DockerEndpoint {
    #[serde(rename = "Host")]
    host: String,
}

pub fn docker_config() -> Option<PathBuf> {
    std::env::var_os("DOCKER_CONFIG")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".docker")))
}

fn config_json(folder: &Path) -> Map<String, Value> {
    fs::read(folder.join("config.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn current_context() -> String {
    docker_config()
        .map(|folder| config_json(&folder))
        .and_then(|config| config.get(CURRENT_CONTEXT)?.as_str().map(str::to_owned))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| DEFAULT_CONTEXT.to_owned())
}

pub fn contexts() -> Vec<DockerContext> {
    let mut found = vec![DockerContext {
        name: DEFAULT_CONTEXT.to_owned(),
        host: String::new(),
    }];
    let Some(folder) = docker_config() else {
        return found;
    };
    let Ok(entries) = fs::read_dir(folder.join("contexts/meta")) else {
        return found;
    };
    let mut named: Vec<DockerContext> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path().join("meta.json")).ok())
        .filter_map(|bytes| serde_json::from_slice::<ContextMeta>(&bytes).ok())
        .map(|meta| DockerContext {
            name: meta.name,
            host: meta.endpoints.docker.host,
        })
        .collect();
    named.sort_by(|a, b| a.name.cmp(&b.name));
    found.extend(named);
    found
}

pub fn use_context(name: &str) -> io::Result<()> {
    let folder = configured()?;
    let mut config = config_json(&folder);
    match name == DEFAULT_CONTEXT {
        true => config.remove(CURRENT_CONTEXT),
        false => config.insert(CURRENT_CONTEXT.to_owned(), Value::from(name)),
    };
    fs::create_dir_all(&folder)?;
    bande_xdg::replace_file(
        &folder.join("config.json"),
        serde_json::to_vec_pretty(&Value::Object(config))?,
    )
}

fn context_folder(folder: &Path, kind: &str, name: &str) -> PathBuf {
    let digest = Sha256::digest(name.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    folder.join("contexts").join(kind).join(hex)
}

fn configured() -> io::Result<PathBuf> {
    docker_config().ok_or_else(|| io::Error::other("no Docker configuration"))
}

pub fn create_context(name: &str, host: &str) -> io::Result<()> {
    let folder = configured()?;
    let meta = context_folder(&folder, "meta", name);
    fs::create_dir_all(&meta)?;
    let body = serde_json::json!({
        "Name": name,
        "Metadata": {},
        "Endpoints": {"docker": {"Host": host, "SkipTLSVerify": false}},
    });
    bande_xdg::replace_file(&meta.join("meta.json"), serde_json::to_vec_pretty(&body)?)
}

pub fn rename_context(old: &str, new: &str) -> io::Result<()> {
    let folder = configured()?;
    let (from, to) = (
        context_folder(&folder, "meta", old),
        context_folder(&folder, "meta", new),
    );
    if new == DEFAULT_CONTEXT || to.exists() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    let mut meta: Value = serde_json::from_slice(&fs::read(from.join("meta.json"))?)?;
    meta.as_object_mut()
        .ok_or_else(|| io::Error::other("the context's meta.json is not an object"))?
        .insert("Name".to_owned(), Value::from(new));
    fs::create_dir_all(&to)?;
    bande_xdg::replace_file(&to.join("meta.json"), serde_json::to_vec_pretty(&meta)?)?;
    let tls = context_folder(&folder, "tls", old);
    if tls.exists() {
        fs::rename(&tls, context_folder(&folder, "tls", new))?;
    }
    fs::remove_dir_all(&from)?;
    match current_context() == old {
        true => use_context(new),
        false => Ok(()),
    }
}

pub fn remove_context(name: &str) -> io::Result<()> {
    let folder = configured()?;
    if current_context() == name {
        use_context(DEFAULT_CONTEXT)?;
    }
    for kind in ["meta", "tls"] {
        match fs::remove_dir_all(context_folder(&folder, kind, name)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

pub fn context_endpoint(name: &str) -> Endpoint {
    contexts()
        .into_iter()
        .find(|context| context.name == name)
        .filter(|context| !context.host.is_empty())
        .map_or(Endpoint::Local(None), |context| {
            endpoint(&context.host, &format!("context-{}", short_hash(name)))
        })
}

fn short_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub type Failure = Arc<Mutex<Option<String>>>;

pub fn tunnel_socket(id: &str) -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|runtime| PathBuf::from(runtime).join(format!("velox/hosts/{id}.sock")))
}

pub struct Tunnel {
    process: Option<OwnedFd>,
    socket: PathBuf,
}

impl Tunnel {
    pub fn open(
        host: &RemoteHost,
        failure: Failure,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        let socket =
            tunnel_socket(&host.id).ok_or_else(|| io::Error::other("no runtime folder"))?;
        if let Some(folder) = socket.parent() {
            fs::create_dir_all(folder)?;
        }
        let _ = fs::remove_file(&socket);
        let mut child: Child = Command::new("ssh")
            .args([
                "-N",
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StreamLocalBindUnlink=yes",
                "-o",
                &format!("ServerAliveInterval={KEEPALIVE_SECONDS}"),
                "-p",
                &host.port.to_string(),
                "-L",
                &format!("{}:{}", socket.display(), host.socket),
                &host.ssh_target(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let process = Pid::from_raw(child.id() as i32)
            .and_then(|pid| pidfd_open(pid, PidfdFlags::empty()).ok());
        let noted = Arc::clone(&failure);
        let stderr = child.stderr.take();
        std::thread::Builder::new()
            .name("velox-tunnel".to_owned())
            .spawn(move || {
                let mut text = String::new();
                if let Some(mut stderr) = stderr {
                    let _ = stderr.read_to_string(&mut text);
                }
                let _ = child.wait();
                *noted.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(text.trim().to_owned());
                on_exit();
            })?;
        Ok(Self { process, socket })
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(process) = &self.process {
            let _ = pidfd_send_signal(process, Signal::TERM);
        }
        let _ = fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(user: &str, address: &str, port: u16) -> Endpoint {
        Endpoint::Ssh(RemoteHost {
            id: "x".to_owned(),
            name: address.to_owned(),
            user: user.to_owned(),
            host: address.to_owned(),
            port,
            socket: REMOTE_SOCKET.to_owned(),
        })
    }

    #[test]
    fn ssh_endpoints_keep_user_host_and_port() {
        assert_eq!(
            endpoint("ssh://deploy@build.example:2222", "x"),
            host("deploy", "build.example", 2222)
        );
        assert_eq!(endpoint("ssh://server", "x"), host("", "server", SSH_PORT));
    }
}
