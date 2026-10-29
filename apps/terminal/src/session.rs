use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

use alacritty_terminal::event::{Event as TermEvent, OnResize};
use alacritty_terminal::tty::{self, Options, Pty, Shell};

use crate::process::{self, DirectoryReports};
use crate::terminal::{GridSize, Terminal};

const READ_BUFFER: usize = 65536;
const TITLE_SEPARATOR: &str = " — ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Copy(String),
    Bell,
}

pub struct Session {
    pub terminal: Terminal,
    pty: Pty,
    shell: u32,
    output: Vec<u8>,
    directories: DirectoryReports,
    reported_directory: Option<PathBuf>,
    program_title: Option<String>,
    foreground: Option<(u32, String)>,
    exited: bool,
}

impl Session {
    pub fn spawn(
        terminal: Terminal,
        shell: Option<Shell>,
        directory: Option<PathBuf>,
    ) -> io::Result<Self> {
        let options = Options {
            shell,
            working_directory: directory,
            drain_on_exit: true,
            env: HashMap::from([
                ("TERM".to_owned(), "xterm-256color".to_owned()),
                ("COLORTERM".to_owned(), "truecolor".to_owned()),
                ("TERM_PROGRAM".to_owned(), "unbound-term".to_owned()),
            ]),
        };
        let pty = tty::new(&options, terminal.window_size(), 0)?;
        let shell = pty.child().id();
        let mut session = Self {
            terminal,
            pty,
            shell,
            output: Vec::new(),
            directories: DirectoryReports::new(process::host_name()),
            reported_directory: None,
            program_title: None,
            foreground: None,
            exited: false,
        };
        session.refresh_foreground();
        Ok(session)
    }

    pub fn id(&self) -> u32 {
        self.shell
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.pty.file().as_fd()
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn has_pending_output(&self) -> bool {
        !self.output.is_empty()
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
        self.flush();
    }

    pub fn flush(&mut self) {
        while !self.output.is_empty() {
            let mut master = self.pty.file();
            match master.write(&self.output) {
                Ok(0) => break,
                Ok(written) => {
                    self.output.drain(..written);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    }

    pub fn read(&mut self) -> Vec<Notice> {
        let mut buffer = [0u8; READ_BUFFER];
        loop {
            let mut master = self.pty.file();
            match master.read(&mut buffer) {
                Ok(0) => {
                    self.exited = true;
                    break;
                }
                Ok(read) => {
                    let chunk = &buffer[..read];
                    if let Some(directory) = self.directories.scan(chunk) {
                        self.reported_directory = Some(directory);
                    }
                    self.terminal.feed(chunk);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.exited = true;
                    break;
                }
            }
        }
        self.refresh_foreground();
        let notices = self.take_notices();
        self.flush();
        notices
    }

    pub fn take_notices(&mut self) -> Vec<Notice> {
        let mut notices = Vec::new();
        for event in self.terminal.take_events() {
            match event {
                TermEvent::PtyWrite(text) => self.send(text.as_bytes()),
                TermEvent::TextAreaSizeRequest(reply) => {
                    let text = reply(self.terminal.window_size());
                    self.send(text.as_bytes());
                }
                TermEvent::Title(title) => self.program_title = Some(title),
                TermEvent::ResetTitle => self.program_title = None,
                TermEvent::ClipboardStore(_, text) => notices.push(Notice::Copy(text)),
                TermEvent::Bell => notices.push(Notice::Bell),
                TermEvent::ChildExit(_) | TermEvent::Exit => self.exited = true,
                _ => {}
            }
        }
        notices
    }

    pub fn resize(&mut self, size: GridSize, cell_size: (u16, u16)) {
        if self.terminal.resize(size, cell_size) {
            self.pty.on_resize(self.terminal.window_size());
        }
    }

    pub fn refresh_foreground(&mut self) -> bool {
        let group = process::foreground_group(self.shell).unwrap_or(self.shell);
        if self
            .foreground
            .as_ref()
            .is_some_and(|(pid, _)| *pid == group)
        {
            return false;
        }
        self.foreground = process::name(group).map(|name| (group, name));
        true
    }

    pub fn running_program(&self) -> Option<&str> {
        self.foreground
            .as_ref()
            .filter(|(pid, _)| *pid != self.shell)
            .map(|(_, name)| name.as_str())
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        self.reported_directory
            .clone()
            .or_else(|| process::working_directory(self.shell))
    }

    pub fn title(&self, home: Option<&Path>) -> String {
        if let Some(title) = self
            .program_title
            .as_ref()
            .filter(|title| !title.is_empty())
        {
            return title.clone();
        }
        let directory = self
            .working_directory()
            .map(|directory| display_directory(&directory, home));
        let program = self.foreground.as_ref().map(|(_, name)| name.clone());
        [directory, program]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(TITLE_SEPARATOR)
    }
}

pub fn display_directory(directory: &Path, home: Option<&Path>) -> String {
    if home == Some(directory) {
        return "~".to_owned();
    }
    directory.file_name().map_or_else(
        || directory.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
