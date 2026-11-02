use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::L10N;
use crate::engine::{Engine, EngineInfo, Error, Flavor};

const DOCKER_INSTALL: &str = "\
sudo apt install ca-certificates curl
sudo install -m 0755 -d /etc/apt/keyrings
sudo curl -fsSL https://download.docker.com/linux/debian/gpg \\
  -o /etc/apt/keyrings/docker.asc
sudo tee /etc/apt/sources.list.d/docker.sources <<EOF
Types: deb
URIs: https://download.docker.com/linux/debian
Suites: $(. /etc/os-release && echo \"$VERSION_CODENAME\")
Components: stable
Signed-By: /etc/apt/keyrings/docker.asc
EOF
sudo apt update
sudo apt install docker-ce docker-ce-cli containerd.io \\
  docker-buildx-plugin docker-compose-plugin
sudo usermod -aG docker $USER";
const CODENAME: &str = "$(. /etc/os-release && echo \"$VERSION_CODENAME\")";
const STABLE_SUITE: &str = "trixie";
const DOCKER_SUITES: [&str; 4] = ["buster", "bullseye", "bookworm", "trixie"];
const OS_RELEASE: &str = "/etc/os-release";
static DOCKER_INSTALL_TESTING: LazyLock<String> =
    LazyLock::new(|| DOCKER_INSTALL.replace(CODENAME, STABLE_SUITE));
const PODMAN_INSTALL: &str = "\
sudo apt install podman
systemctl --user enable --now podman.socket";
const DOCKER_START: &str = "sudo systemctl enable --now docker";
const PODMAN_START: &str = "systemctl --user enable --now podman.socket";
const DOCKER_GROUP: &str = "sudo usermod -aG docker $USER";
const INSTALL_LINK: &str = "https://docs.docker.com/engine/install/debian/";
const ACCESS_LINK: &str = "https://docs.docker.com/engine/install/linux-postinstall/";
const BINARY_FOLDERS: [&str; 4] = ["/usr/bin", "/usr/sbin", "/usr/local/bin", "/usr/local/sbin"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    NotInstalled,
    NotRunning(Flavor),
    NoAccess,
    Relogin,
    TooOld,
}

pub enum Probe {
    Ready(EngineInfo),
    Blocked(Readiness),
}

pub struct Steps {
    pub title: String,
    pub commands: &'static str,
}

pub struct Advice {
    pub headline: String,
    pub steps: Option<Steps>,
    pub note: Option<String>,
    pub link: Option<&'static str>,
    pub alternative: Option<Steps>,
}

fn podman_alternative() -> Steps {
    Steps {
        title: L10N.tr("steps-podman"),
        commands: PODMAN_INSTALL,
    }
}

fn debian_testing() -> bool {
    let release = std::fs::read_to_string(OS_RELEASE).unwrap_or_default();
    let field = |name: &str| {
        release.lines().find_map(|line| {
            line.strip_prefix(name)?
                .strip_prefix('=')
                .map(|value| value.trim_matches('"'))
        })
    };
    field("ID") == Some("debian")
        && field("VERSION_CODENAME")
            .is_none_or(|codename| codename == "testing" || !DOCKER_SUITES.contains(&codename))
}

impl Readiness {
    pub fn advice(&self) -> Advice {
        let advice = |headline: &str| Advice {
            headline: L10N.tr(headline),
            steps: None,
            note: None,
            link: None,
            alternative: None,
        };
        let steps = |title: &str, commands| {
            Some(Steps {
                title: L10N.tr(title),
                commands,
            })
        };
        match self {
            Self::NotInstalled if debian_testing() => Advice {
                steps: steps("steps-docker", DOCKER_INSTALL_TESTING.as_str()),
                note: Some(L10N.tr("steps-testing-note")),
                link: Some(INSTALL_LINK),
                alternative: Some(podman_alternative()),
                ..advice("not-installed")
            },
            Self::NotInstalled => Advice {
                steps: steps("steps-docker", DOCKER_INSTALL),
                link: Some(INSTALL_LINK),
                alternative: Some(podman_alternative()),
                ..advice("not-installed")
            },
            Self::NotRunning(Flavor::Docker) => Advice {
                steps: steps("steps-start-docker", DOCKER_START),
                ..advice("docker-not-running")
            },
            Self::NotRunning(Flavor::Podman) => Advice {
                steps: steps("steps-start-podman", PODMAN_START),
                ..advice("podman-not-running")
            },
            Self::NoAccess => Advice {
                steps: steps("steps-access", DOCKER_GROUP),
                note: Some(L10N.tr("docker-group-note")),
                link: Some(ACCESS_LINK),
                alternative: Some(podman_alternative()),
                ..advice("no-access")
            },
            Self::Relogin => advice("relogin"),
            Self::TooOld => Advice {
                link: Some(INSTALL_LINK),
                ..advice("too-old")
            },
        }
    }
}

fn installed(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(BINARY_FOLDERS.iter().map(PathBuf::from))
        .any(|folder| folder.join(binary).is_file())
}

fn status_field(name: &str) -> Vec<u32> {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .map(|values| {
            values
                .split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn listed_in_group(gid: u32) -> bool {
    let Ok(user) = std::env::var("USER") else {
        return false;
    };
    std::fs::read_to_string("/etc/group")
        .unwrap_or_default()
        .lines()
        .map(|line| line.split(':').collect::<Vec<_>>())
        .any(|fields| {
            matches!(fields.as_slice(), [_, _, id, members]
                if id.parse() == Ok(gid) && members.split(',').any(|member| member == user))
        })
}

fn needs_relogin(socket: &Path) -> bool {
    let Ok(gid) = std::fs::metadata(socket).map(|metadata| metadata.gid()) else {
        return false;
    };
    let session = status_field("Groups:");
    let primary = status_field("Gid:");
    !session.contains(&gid) && !primary.contains(&gid) && listed_in_group(gid)
}

pub fn probe(candidates: &[PathBuf]) -> Probe {
    let mut too_old = false;
    let mut denied = None;
    for socket in candidates
        .iter()
        .filter(|socket| socket.symlink_metadata().is_ok())
    {
        match Engine::new(socket).info() {
            Ok(info) if info.supported => return Probe::Ready(info),
            Ok(_) => too_old = true,
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied => {
                denied.get_or_insert(socket);
            }
            Err(error) => tracing::debug!("{} does not answer: {error}", socket.display()),
        }
    }
    Probe::Blocked(match (too_old, denied) {
        (true, _) => Readiness::TooOld,
        (false, Some(socket)) if needs_relogin(socket) => Readiness::Relogin,
        (false, Some(_)) => Readiness::NoAccess,
        _ if installed("dockerd") => Readiness::NotRunning(Flavor::Docker),
        _ if installed("podman") => Readiness::NotRunning(Flavor::Podman),
        _ => Readiness::NotInstalled,
    })
}
