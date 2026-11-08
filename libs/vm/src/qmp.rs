use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, IoSlice, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
use serde_json::{Value, json};

use crate::{Error, Result};

pub struct Qmp {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    line: Vec<u8>,
    events: VecDeque<Event>,
    next_id: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub name: String,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    Running,
    Paused,
    Shutdown,
    Saving,
    Restoring,
    Suspended,
    Other(String),
}

impl Event {
    fn from_message(message: &mut Value) -> Option<Self> {
        let name = message.get("event")?.as_str()?.to_owned();
        let data = message.get_mut("data").map(Value::take).unwrap_or_default();
        Some(Self { name, data })
    }
}

impl RunState {
    fn parse(status: &str) -> Self {
        match status {
            "running" => Self::Running,
            "paused" => Self::Paused,
            "shutdown" => Self::Shutdown,
            "save-vm" => Self::Saving,
            "restore-vm" => Self::Restoring,
            "suspended" => Self::Suspended,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl Qmp {
    pub fn connect(path: &Path) -> Result<Self> {
        Self::handshake(UnixStream::connect(path)?)
    }

    pub fn handshake(stream: UnixStream) -> Result<Self> {
        let mut qmp = Self {
            writer: stream.try_clone()?,
            reader: BufReader::new(stream),
            line: Vec::new(),
            events: VecDeque::new(),
            next_id: 0,
        };
        if qmp.read_message()?.get("QMP").is_none() {
            return Err(Error::Protocol("no QMP greeting".into()));
        }
        qmp.execute("qmp_capabilities", json!({}))?;
        Ok(qmp)
    }

    pub fn execute(&mut self, command: &str, arguments: Value) -> Result<Value> {
        let (id, request) = self.request(command, arguments)?;
        self.writer.write_all(&request)?;
        self.reply(id)
    }

    pub(crate) fn execute_with_fd(
        &mut self,
        command: &str,
        arguments: Value,
        fd: BorrowedFd<'_>,
    ) -> Result<Value> {
        let (id, request) = self.request(command, arguments)?;
        let fds = [fd];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        control.push(SendAncillaryMessage::ScmRights(&fds));
        let mut sent = sendmsg(
            &self.writer,
            &[IoSlice::new(&request)],
            &mut control,
            SendFlags::empty(),
        )?;
        while sent < request.len() {
            sent += self.writer.write(&request[sent..])?;
        }
        self.reply(id)
    }

    fn request(&mut self, command: &str, arguments: Value) -> Result<(u64, Vec<u8>)> {
        let id = self.next_id;
        self.next_id += 1;
        let mut request =
            serde_json::to_vec(&json!({"execute": command, "arguments": arguments, "id": id}))?;
        request.push(b'\n');
        Ok((id, request))
    }

    fn reply(&mut self, id: u64) -> Result<Value> {
        loop {
            let mut message = self.read_message()?;
            if let Some(event) = Event::from_message(&mut message) {
                self.events.push_back(event);
                continue;
            }
            if message.get("id") != Some(&json!(id)) {
                continue;
            }
            if let Some(value) = message.get_mut("return") {
                return Ok(value.take());
            }
            let error = &message["error"];
            let text = |key: &str| error[key].as_str().unwrap_or_default().to_owned();
            return Err(Error::Qmp {
                class: text("class"),
                description: text("desc"),
            });
        }
    }

    pub fn next_event(&mut self) -> Result<Event> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        loop {
            if let Some(event) = Event::from_message(&mut self.read_message()?) {
                return Ok(event);
            }
        }
    }

    pub fn drain_events(&mut self) -> Result<Vec<Event>> {
        self.writer.set_nonblocking(true)?;
        let read = loop {
            match self.read_message() {
                Ok(mut message) => {
                    if let Some(event) = Event::from_message(&mut message) {
                        self.events.push_back(event);
                    }
                }
                Err(Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => break Ok(()),
                Err(error) => break Err(error),
            }
        };
        self.writer.set_nonblocking(false)?;
        read.map(|()| self.events.drain(..).collect())
    }

    pub fn status(&mut self) -> Result<RunState> {
        let status = self.execute("query-status", json!({}))?;
        Ok(RunState::parse(
            status["status"].as_str().unwrap_or_default(),
        ))
    }

    pub fn pause(&mut self) -> Result<()> {
        self.execute("stop", json!({})).map(drop)
    }

    pub fn resume(&mut self) -> Result<()> {
        self.execute("cont", json!({})).map(drop)
    }

    pub fn shut_down(&mut self) -> Result<()> {
        self.execute("system_powerdown", json!({})).map(drop)
    }

    pub fn quit(&mut self) -> Result<()> {
        match self.execute("quit", json!({})) {
            Err(Error::Protocol(_)) => Ok(()),
            result => result.map(drop),
        }
    }

    pub fn reset(&mut self) -> Result<()> {
        self.execute("system_reset", json!({})).map(drop)
    }

    pub fn snapshot_save(&mut self, tag: &str, vmstate: &str, devices: &[&str]) -> Result<()> {
        self.run_job(
            "snapshot-save",
            json!({"tag": tag, "vmstate": vmstate, "devices": devices}),
        )
    }

    pub fn snapshot_load(&mut self, tag: &str, vmstate: &str, devices: &[&str]) -> Result<()> {
        self.run_job(
            "snapshot-load",
            json!({"tag": tag, "vmstate": vmstate, "devices": devices}),
        )
    }

    pub fn snapshot_delete(&mut self, tag: &str, devices: &[&str]) -> Result<()> {
        self.run_job("snapshot-delete", json!({"tag": tag, "devices": devices}))
    }

    fn run_job(&mut self, command: &str, mut arguments: Value) -> Result<()> {
        let job = format!("{command}-{}", self.next_id);
        arguments["job-id"] = json!(job);
        self.execute(command, arguments)?;
        let mut pending = std::mem::take(&mut self.events);
        let mut kept = VecDeque::new();
        loop {
            let event = match pending.pop_front() {
                Some(event) => event,
                None => self.next_event()?,
            };
            if event.name != "JOB_STATUS_CHANGE" || event.data["id"] != json!(job) {
                kept.push_back(event);
            } else if event.data["status"] == "concluded" {
                break;
            }
        }
        kept.append(&mut pending);
        kept.append(&mut self.events);
        self.events = kept;
        let jobs = self.execute("query-jobs", json!({}))?;
        let error = jobs
            .as_array()
            .and_then(|jobs| jobs.iter().find(|entry| entry["id"] == json!(job)))
            .and_then(|entry| entry["error"].as_str())
            .map(str::to_owned);
        self.execute("job-dismiss", json!({"id": job}))?;
        match error {
            Some(description) => Err(Error::Qmp {
                class: "JobFailed".into(),
                description,
            }),
            None => Ok(()),
        }
    }

    fn read_message(&mut self) -> Result<Value> {
        let read = self.reader.read_until(b'\n', &mut self.line)?;
        if read == 0 || self.line.last() != Some(&b'\n') {
            return Err(Error::Protocol("QEMU closed the connection".into()));
        }
        let message = serde_json::from_slice(&self.line);
        self.line.clear();
        Ok(message?)
    }
}

impl AsFd for Qmp {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.writer.as_fd()
    }
}
