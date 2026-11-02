use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const UNIX_SCHEME: &str = "unix://";
const DEFAULT_CONTEXT: &str = "default";

#[derive(Deserialize)]
struct Config {
    #[serde(rename = "currentContext")]
    current_context: Option<String>,
}

#[derive(Deserialize)]
struct ContextMeta {
    #[serde(rename = "Endpoints")]
    endpoints: Endpoints,
}

#[derive(Deserialize)]
struct Endpoints {
    docker: Endpoint,
}

#[derive(Deserialize)]
struct Endpoint {
    #[serde(rename = "Host")]
    host: String,
}

fn unix_path(host: &str) -> Option<PathBuf> {
    host.strip_prefix(UNIX_SCHEME)
        .filter(|path| path.starts_with('/'))
        .map(PathBuf::from)
}

fn context_host(docker_config: &Path) -> Option<String> {
    let config: Config =
        serde_json::from_slice(&std::fs::read(docker_config.join("config.json")).ok()?).ok()?;
    let name = config
        .current_context
        .filter(|name| name != DEFAULT_CONTEXT)?;
    let digest = Sha256::digest(name.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let meta = docker_config
        .join("contexts/meta")
        .join(hex)
        .join("meta.json");
    let meta: ContextMeta = serde_json::from_slice(&std::fs::read(meta).ok()?).ok()?;
    Some(meta.endpoints.docker.host)
}

pub fn candidates() -> Vec<PathBuf> {
    let explicit = std::env::var("DOCKER_HOST").ok().or_else(|| {
        let config = std::env::var_os("DOCKER_CONFIG")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".docker")))?;
        context_host(&config)
    });
    if let Some(host) = explicit.filter(|host| !host.is_empty()) {
        match unix_path(&host) {
            Some(path) => return vec![path],
            None => tracing::warn!("Velox uses only local unix sockets, not {host}"),
        }
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let mut paths: Vec<PathBuf> = runtime
        .iter()
        .flat_map(|runtime| {
            [
                runtime.join("docker.sock"),
                runtime.join("podman/podman.sock"),
            ]
        })
        .collect();
    paths.extend([
        PathBuf::from("/run/docker.sock"),
        PathBuf::from("/run/podman/podman.sock"),
    ]);
    paths
}

pub fn watched_folders(candidates: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = candidates
        .iter()
        .filter_map(|socket| {
            socket
                .ancestors()
                .skip(1)
                .find(|folder| folder.is_dir())
                .map(Path::to_path_buf)
        })
        .collect();
    folders.push(PathBuf::from("/var/lib/dpkg"));
    folders.sort();
    folders.dedup();
    folders
}
