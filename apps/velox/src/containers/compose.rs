use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::engine::{Container, EngineInfo, Flavor};

const WORKING_DIR: &str = "com.docker.compose.project.working_dir";
const CONFIG_FILES: &str = "com.docker.compose.project.config_files";
const MESSAGE_LINES: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Up,
    Down,
    Restart,
}

impl Verb {
    fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::Up => &["up", "--detach"],
            Self::Down => &["down"],
            Self::Restart => &["restart"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compose {
    pub project: String,
    directory: PathBuf,
    files: Vec<PathBuf>,
    host: String,
    flavor: Flavor,
}

impl Compose {
    pub fn of(project: &str, members: &[&Container], info: &EngineInfo) -> Option<Self> {
        let labelled = members
            .iter()
            .find_map(|member| Some((member.label(WORKING_DIR)?, member.label(CONFIG_FILES)?)))?;
        let (directory, files) = labelled;
        Some(Self {
            project: project.to_owned(),
            directory: PathBuf::from(directory),
            files: files
                .split(',')
                .filter(|file| !file.is_empty())
                .map(PathBuf::from)
                .collect(),
            host: format!("unix://{}", info.socket.display()),
            flavor: info.flavor,
        })
    }

    pub fn command(&self, arguments: &[String]) -> Command {
        let program = match self.flavor {
            Flavor::Docker => "docker",
            Flavor::Podman => "podman",
        };
        let mut command = Command::new(program);
        command
            .arg("compose")
            .arg("--project-name")
            .arg(&self.project)
            .arg("--project-directory")
            .arg(&self.directory)
            .env("DOCKER_HOST", &self.host);
        for file in &self.files {
            command.arg("--file").arg(file);
        }
        if self.directory.is_dir() {
            command.current_dir(&self.directory);
        }
        command.args(arguments);
        command
    }

    pub fn colours(&self) -> Vec<String> {
        match self.flavor {
            Flavor::Docker => vec!["--ansi".to_owned(), "always".to_owned()],
            Flavor::Podman => Vec::new(),
        }
    }

    pub fn run(&self, verb: Verb) -> Result<(), String> {
        let arguments: Vec<String> = verb
            .arguments()
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect();
        let output = self
            .command(&arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        Err(lines[lines.len().saturating_sub(MESSAGE_LINES)..].join("\n"))
    }
}
