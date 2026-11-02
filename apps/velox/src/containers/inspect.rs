use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{Engine, Error};

const DEFAULT_NETWORKS: [&str; 4] = ["default", "bridge", "podman", ""];
const DEFAULT_POLICY: [&str; 2] = ["", "no"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct State {
    pub status: String,
    pub restarting: bool,
    pub exit_code: i64,
    pub started_at: String,
    pub finished_at: String,
    pub error: String,
    #[serde(rename = "OOMKilled")]
    pub oom_killed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Config {
    pub image: String,
    pub env: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub working_dir: String,
    pub user: String,
    pub labels: Option<BTreeMap<String, String>>,
    pub tty: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Binding {
    pub host_ip: String,
    pub host_port: String,
}

type Bindings = Option<BTreeMap<String, Option<Vec<Binding>>>>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RestartPolicy {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct HostConfig {
    pub restart_policy: Option<RestartPolicy>,
    pub auto_remove: bool,
    pub port_bindings: Bindings,
    pub network_mode: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Mount {
    #[serde(rename = "Type")]
    pub kind: String,
    pub name: Option<String>,
    pub source: String,
    pub destination: String,
    #[serde(rename = "RW")]
    pub writable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Network {
    #[serde(rename = "IPAddress")]
    pub ip_address: String,
    #[serde(rename = "Gateway")]
    pub gateway: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct NetworkSettings {
    pub networks: Option<BTreeMap<String, Network>>,
    pub ports: Bindings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Inspect {
    pub id: String,
    pub name: String,
    pub created: String,
    pub image: String,
    pub state: State,
    pub restart_count: u32,
    pub config: Config,
    pub host_config: HostConfig,
    pub mounts: Vec<Mount>,
    pub network_settings: NetworkSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct Image {
    config: Config,
}

pub fn inspect(engine: &Engine, id: &str) -> Result<Inspect, Error> {
    engine.get(&format!("/containers/{id}/json"))
}

impl Inspect {
    pub fn name(&self) -> &str {
        self.name.trim_start_matches('/')
    }

    pub fn command(&self) -> String {
        self.config
            .entrypoint
            .iter()
            .flatten()
            .chain(self.config.cmd.iter().flatten())
            .map(|word| quoted(word))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn ports(&self) -> Vec<(String, String)> {
        let mut ports: Vec<(String, String)> = self
            .network_settings
            .ports
            .iter()
            .flatten()
            .flat_map(|(inside, bindings)| {
                bindings
                    .iter()
                    .flatten()
                    .map(move |binding| (binding.host_port.clone(), inside.clone()))
            })
            .filter(|(host, _)| !host.is_empty())
            .collect();
        ports.sort();
        ports.dedup();
        ports
    }

    pub fn ip(&self) -> Option<&str> {
        self.network_settings
            .networks
            .iter()
            .flatten()
            .map(|(_, network)| network.ip_address.as_str())
            .find(|ip| !ip.is_empty())
    }

    pub fn run_command(&self, engine: &Engine) -> Result<String, Error> {
        let image: Image = engine.get(&format!("/images/{}/json", self.image))?;
        let mut words = vec!["docker".to_owned(), "run".to_owned(), "--detach".to_owned()];
        let flag = |words: &mut Vec<String>, name: &str, value: &str| {
            words.push(name.to_owned());
            words.push(quoted(value));
        };
        flag(&mut words, "--name", self.name());
        if let Some(policy) = &self.host_config.restart_policy
            && !DEFAULT_POLICY.contains(&policy.name.as_str())
        {
            flag(&mut words, "--restart", &policy.name);
        }
        let mode = self.host_config.network_mode.as_str();
        if !DEFAULT_NETWORKS.contains(&mode) {
            flag(&mut words, "--network", mode);
        }
        let bindings: Vec<(String, String)> = self
            .host_config
            .port_bindings
            .iter()
            .flatten()
            .flat_map(|(inside, bindings)| {
                bindings.iter().flatten().map(move |binding| {
                    let host = match binding.host_ip.as_str() {
                        "" | "0.0.0.0" | "::" => binding.host_port.clone(),
                        ip => format!("{ip}:{}", binding.host_port),
                    };
                    (host, inside.trim_end_matches("/tcp").to_owned())
                })
            })
            .collect();
        for (host, inside) in bindings {
            match host.is_empty() {
                true => flag(&mut words, "--publish", &inside),
                false => flag(&mut words, "--publish", &format!("{host}:{inside}")),
            }
        }
        let inherited: Vec<&String> = image.config.env.iter().flatten().collect();
        for variable in self.config.env.iter().flatten() {
            if !inherited.contains(&variable) {
                flag(&mut words, "--env", variable);
            }
        }
        for mount in &self.mounts {
            let source = match (&mount.name, mount.kind.as_str()) {
                (Some(name), "volume") => name.as_str(),
                _ => mount.source.as_str(),
            };
            let mode = match mount.writable {
                true => "",
                false => ":ro",
            };
            flag(
                &mut words,
                "--volume",
                &format!("{source}:{}{mode}", mount.destination),
            );
        }
        if self.config.working_dir != image.config.working_dir {
            flag(&mut words, "--workdir", &self.config.working_dir);
        }
        if self.host_config.auto_remove {
            words.push("--rm".to_owned());
        }
        if self.config.tty {
            words.push("--tty".to_owned());
        }
        let entrypoint = self.config.entrypoint != image.config.entrypoint;
        let mut tail: Vec<&String> = Vec::new();
        if entrypoint {
            let mut parts = self.config.entrypoint.iter().flatten();
            flag(
                &mut words,
                "--entrypoint",
                parts.next().map_or("", String::as_str),
            );
            tail.extend(parts);
        }
        if entrypoint || self.config.cmd != image.config.cmd {
            tail.extend(self.config.cmd.iter().flatten());
        }
        words.push(quoted(&self.config.image));
        words.extend(tail.into_iter().map(|word| quoted(word)));
        Ok(words.join(" "))
    }

    pub fn labels(&self) -> Vec<(&str, &str)> {
        self.config
            .labels
            .iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect()
    }
}

pub fn quoted(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || "-_./:=@,+%".contains(char));
    match plain {
        true => word.to_owned(),
        false => format!("'{}'", word.replace('\'', r"'\''")),
    }
}
