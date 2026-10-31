use std::time::Duration;

use data_encoding::BASE64;

use super::wire::{Response, Status, Value, literal_length, parse_response, quote};
use super::{Flag, Flags, Role, utf7};
use crate::error::{Error, Result};
use crate::net::{COMMAND_TIMEOUT, Closer, Link, Security, Server, Trust};

const MAX_LINE: usize = 1 << 20;
const MAX_RESPONSE: usize = 128 << 20;
const MAX_SET_LENGTH: usize = 900;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub path: String,
    pub name: String,
    pub delimiter: Option<char>,
    pub role: Option<Role>,
    pub selectable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selected {
    pub uid_validity: u32,
    pub uid_next: Option<u32>,
    pub exists: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fetched {
    pub uid: u32,
    pub flags: Option<Flags>,
    pub size: Option<u64>,
    pub internal_date: Option<i64>,
    pub header: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
}

struct Reply {
    data: Vec<Vec<Value>>,
    codes: Vec<Vec<Value>>,
}

pub struct Session {
    link: Link,
    tag: u32,
    capabilities: Vec<String>,
    changed: bool,
}

pub fn uid_sets(uids: &[u32]) -> Vec<String> {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut sets = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    while index < sorted.len() {
        let start = sorted[index];
        let mut end = start;
        while index + 1 < sorted.len() && sorted[index + 1] == end + 1 {
            index += 1;
            end = sorted[index];
        }
        let range = match start == end {
            true => start.to_string(),
            false => format!("{start}:{end}"),
        };
        if !current.is_empty() && current.len() + range.len() + 1 > MAX_SET_LENGTH {
            sets.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(',');
        }
        current.push_str(&range);
        index += 1;
    }
    if !current.is_empty() {
        sets.push(current);
    }
    sets
}

fn mailbox_argument(path: &str) -> Result<String> {
    Some(path)
        .filter(|path| !path.chars().any(char::is_control))
        .and_then(|path| quote(&utf7::encode(path)))
        .ok_or_else(|| Error::Refused(format!("the mailbox name {path:?} is not allowed")))
}

fn atoms(value: &Value) -> Vec<String> {
    value
        .list()
        .unwrap_or_default()
        .iter()
        .filter_map(Value::text)
        .collect()
}

fn parse_internal_date(text: &str) -> Option<i64> {
    let text = text.trim();
    jiff::fmt::strtime::parse("%d-%b-%Y %H:%M:%S %z", text)
        .and_then(|parsed| parsed.to_timestamp())
        .ok()
        .map(|timestamp| timestamp.as_second())
}

fn fetched(values: &[Value]) -> Option<Fetched> {
    if !values.get(1)?.is_atom("FETCH") {
        return None;
    }
    let items = values.get(2)?.list()?;
    let mut fetched = Fetched::default();
    for pair in items.chunks_exact(2) {
        let Some(key) = pair[0].atom() else {
            continue;
        };
        let key = key.to_ascii_uppercase();
        let value = &pair[1];
        match key.as_str() {
            "UID" => fetched.uid = value.number().and_then(|uid| u32::try_from(uid).ok())?,
            "FLAGS" => {
                let atoms = atoms(value);
                fetched.flags = Some(Flags::from_atoms(atoms.iter().map(String::as_str)));
            }
            "RFC822.SIZE" => fetched.size = value.number(),
            "INTERNALDATE" => {
                fetched.internal_date = value.text().as_deref().and_then(parse_internal_date);
            }
            "BODY[]" | "RFC822" | "BINARY[]" => fetched.body = value.bytes().map(<[u8]>::to_vec),
            key if key.starts_with("BODY[HEADER") || key == "RFC822.HEADER" => {
                fetched.header = value.bytes().map(<[u8]>::to_vec);
            }
            _ => {}
        }
    }
    (fetched.uid != 0).then_some(fetched)
}

fn code_number(codes: &[Vec<Value>], name: &str) -> Option<u64> {
    codes
        .iter()
        .find(|code| code.first().is_some_and(|first| first.is_atom(name)))
        .and_then(|code| code.get(1))
        .and_then(Value::number)
}

impl Session {
    pub fn connect(server: &Server, trust: &Trust) -> Result<Self> {
        let link = Link::connect(server, trust)?;
        let mut session = Self {
            link,
            tag: 0,
            capabilities: Vec::new(),
            changed: false,
        };
        let greeting = session.read_response()?;
        let greeting_code = match greeting {
            Response::Status {
                tag: None,
                status: Status::Ok | Status::PreAuth,
                code,
                ..
            } => code,
            Response::Status {
                status: Status::Bye,
                text,
                ..
            } => return Err(Error::Refused(text)),
            other => return Err(Error::Protocol(format!("greeting {other:?}"))),
        };
        session.take_capabilities(&greeting_code);
        if session.capabilities.is_empty() {
            session.refresh_capabilities()?;
        }
        if server.security == Security::StartTls {
            if !session.has("STARTTLS") {
                return Err(Error::Insecure);
            }
            session.run("STARTTLS")?;
            session.link = session.link.start_tls()?;
            session.refresh_capabilities()?;
        }
        Ok(session)
    }

    pub fn closer(&self) -> Result<Closer> {
        self.link.closer()
    }

    pub fn has(&self, capability: &str) -> bool {
        self.capabilities
            .iter()
            .any(|known| known.eq_ignore_ascii_case(capability))
    }

    fn take_capabilities(&mut self, code: &[Value]) -> bool {
        if !code.first().is_some_and(|first| first.is_atom("CAPABILITY")) {
            return false;
        }
        self.capabilities = code[1..].iter().filter_map(Value::text).collect();
        true
    }

    fn refresh_capabilities(&mut self) -> Result<()> {
        let reply = self.run("CAPABILITY")?;
        for data in reply.data.iter().chain(&reply.codes) {
            if self.take_capabilities(data) {
                break;
            }
        }
        Ok(())
    }

    fn next_tag(&mut self) -> String {
        self.tag += 1;
        format!("b{}", self.tag)
    }

    fn read_raw(&mut self) -> Result<Vec<u8>> {
        let mut raw = self.link.read_line(MAX_LINE)?;
        while let Some(length) = literal_length(&raw) {
            if raw.len() + length > MAX_RESPONSE {
                return Err(Error::Protocol(format!(
                    "a response larger than {MAX_RESPONSE} bytes"
                )));
            }
            let literal = self.link.read_exact(length)?;
            raw.extend_from_slice(&literal);
            let rest = self.link.read_line(MAX_LINE)?;
            raw.extend_from_slice(&rest);
        }
        Ok(raw)
    }

    fn read_response(&mut self) -> Result<Response> {
        let raw = self.read_raw()?;
        Ok(parse_response(&raw)?)
    }

    fn note(&mut self, values: &[Value]) {
        if values
            .get(1)
            .is_some_and(|word| word.is_atom("EXISTS") || word.is_atom("EXPUNGE"))
            || values.first().is_some_and(|word| word.is_atom("VANISHED"))
        {
            self.changed = true;
        }
    }

    fn finish(&mut self, tag: &str) -> Result<Reply> {
        let mut reply = Reply {
            data: Vec::new(),
            codes: Vec::new(),
        };
        let mut bye = None;
        loop {
            match self.read_response()? {
                Response::Data(values) => {
                    self.note(&values);
                    reply.data.push(values);
                }
                Response::Status {
                    tag: None,
                    status: Status::Bye,
                    text,
                    ..
                } => bye = Some(text),
                Response::Status {
                    tag: None, code, ..
                } => {
                    if !code.is_empty() {
                        reply.codes.push(code);
                    }
                }
                Response::Status {
                    tag: Some(answered),
                    status,
                    code,
                    text,
                } if answered == tag => {
                    return match status {
                        Status::Ok => {
                            if !code.is_empty() {
                                reply.codes.push(code);
                            }
                            Ok(reply)
                        }
                        Status::No => Err(Error::Refused(text)),
                        _ => Err(Error::Protocol(text)),
                    };
                }
                Response::Status { tag: Some(other), .. } => {
                    return Err(Error::Protocol(format!("an answer to {other}, not {tag}")));
                }
                Response::Continue(text) => {
                    return Err(Error::Protocol(format!("an unexpected continuation {text}")));
                }
            }
            if let Some(text) = bye.take() {
                return Err(Error::Refused(text));
            }
        }
    }

    fn run(&mut self, command: &str) -> Result<Reply> {
        let tag = self.next_tag();
        self.link.send(format!("{tag} {command}\r\n").as_bytes())?;
        self.finish(&tag)
    }

    fn await_continuation(&mut self, tag: &str) -> Result<()> {
        loop {
            match self.read_response()? {
                Response::Continue(_) => return Ok(()),
                Response::Data(values) => self.note(&values),
                Response::Status { tag: None, .. } => {}
                Response::Status {
                    tag: Some(answered),
                    status,
                    text,
                    ..
                } if answered == tag => {
                    return Err(match status {
                        Status::No => Error::Refused(text),
                        _ => Error::Protocol(text),
                    });
                }
                Response::Status { .. } => return Err(Error::Protocol("a stray answer".into())),
            }
        }
    }

    pub fn login(&mut self, user: &str, password: &str) -> Result<()> {
        let result = if self.has("AUTH=PLAIN") {
            let token = BASE64.encode(format!("\0{user}\0{password}").as_bytes());
            let tag = self.next_tag();
            if self.has("SASL-IR") {
                self.link
                    .send(format!("{tag} AUTHENTICATE PLAIN {token}\r\n").as_bytes())?;
            } else {
                self.link
                    .send(format!("{tag} AUTHENTICATE PLAIN\r\n").as_bytes())?;
                self.await_continuation(&tag)?;
                self.link.send(format!("{token}\r\n").as_bytes())?;
            }
            self.finish(&tag)
        } else if self.has("LOGINDISABLED") {
            return Err(Error::Insecure);
        } else {
            let (Some(user), Some(password)) = (quote(user), quote(password)) else {
                return Err(Error::Login("the name or password has a line break".into()));
            };
            self.run(&format!("LOGIN {user} {password}"))
        };
        let reply = result.map_err(|error| match error {
            Error::Refused(text) | Error::Protocol(text) => Error::Login(text),
            other => other,
        })?;
        let fresh = reply
            .codes
            .iter()
            .any(|code| self.take_capabilities(&code.clone()));
        if !fresh {
            self.refresh_capabilities()?;
        }
        Ok(())
    }

    pub fn list(&mut self) -> Result<Vec<Folder>> {
        let command = match self.has("SPECIAL-USE") {
            true => "LIST \"\" \"*\" RETURN (SPECIAL-USE)",
            false => "LIST \"\" \"*\"",
        };
        let reply = self.run(command)?;
        let mut folders = reply
            .data
            .iter()
            .filter(|values| values.first().is_some_and(|word| word.is_atom("LIST")))
            .filter_map(|values| {
                let attributes = atoms(values.get(1)?);
                let delimiter = values
                    .get(2)?
                    .text()
                    .and_then(|text| text.chars().next());
                let path = utf7::decode(&values.get(3)?.text()?);
                let selectable = !attributes.iter().any(|attribute| {
                    attribute.eq_ignore_ascii_case("\\Noselect")
                        || attribute.eq_ignore_ascii_case("\\NonExistent")
                });
                let role = match path.eq_ignore_ascii_case("INBOX") {
                    true => Some(Role::Inbox),
                    false => attributes
                        .iter()
                        .find_map(|attribute| Role::from_attribute(attribute))
                        .or_else(|| Role::guess(&path, delimiter)),
                };
                let name = delimiter
                    .and_then(|delimiter| path.rsplit(delimiter).next())
                    .unwrap_or(&path)
                    .to_owned();
                Some(Folder {
                    path,
                    name,
                    delimiter,
                    role,
                    selectable,
                })
            })
            .collect::<Vec<_>>();
        folders.sort_by(|a, b| a.path.cmp(&b.path));
        folders.dedup_by(|a, b| a.path == b.path);
        Ok(folders)
    }

    pub fn select(&mut self, path: &str) -> Result<Selected> {
        let reply = self.run(&format!("SELECT {}", mailbox_argument(path)?))?;
        let exists = reply
            .data
            .iter()
            .find(|values| values.get(1).is_some_and(|word| word.is_atom("EXISTS")))
            .and_then(|values| values.first()?.number())
            .unwrap_or(0);
        self.changed = false;
        Ok(Selected {
            uid_validity: code_number(&reply.codes, "UIDVALIDITY")
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| Error::Protocol(format!("{path} has no UIDVALIDITY")))?,
            uid_next: code_number(&reply.codes, "UIDNEXT")
                .and_then(|value| u32::try_from(value).ok()),
            exists: u32::try_from(exists).unwrap_or(u32::MAX),
        })
    }

    pub fn uids(&mut self) -> Result<Vec<u32>> {
        let reply = self.run("UID SEARCH ALL")?;
        Ok(reply
            .data
            .iter()
            .filter(|values| values.first().is_some_and(|word| word.is_atom("SEARCH")))
            .flat_map(|values| values[1..].iter().filter_map(Value::number))
            .filter_map(|uid| u32::try_from(uid).ok())
            .collect())
    }

    fn fetch(&mut self, uids: &[u32], items: &str) -> Result<Vec<Fetched>> {
        let mut found = Vec::new();
        for set in uid_sets(uids) {
            let reply = self.run(&format!("UID FETCH {set} ({items})"))?;
            found.extend(reply.data.iter().filter_map(|values| fetched(values)));
        }
        Ok(found)
    }

    pub fn fetch_headers(&mut self, uids: &[u32]) -> Result<Vec<Fetched>> {
        self.fetch(
            uids,
            "UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER]",
        )
    }

    pub fn fetch_flags(&mut self, uids: &[u32]) -> Result<Vec<Fetched>> {
        self.fetch(uids, "UID FLAGS")
    }

    pub fn fetch_bodies(&mut self, uids: &[u32]) -> Result<Vec<Fetched>> {
        self.fetch(uids, "UID BODY.PEEK[]")
    }

    pub fn store(&mut self, uids: &[u32], flag: Flag, set: bool) -> Result<()> {
        let sign = if set { '+' } else { '-' };
        for uids in uid_sets(uids) {
            self.run(&format!(
                "UID STORE {uids} {sign}FLAGS.SILENT ({})",
                flag.atom()
            ))?;
        }
        Ok(())
    }

    pub fn expunge(&mut self, uids: &[u32]) -> Result<()> {
        self.store(uids, Flag::Deleted, true)?;
        match self.has("UIDPLUS") {
            true => {
                for set in uid_sets(uids) {
                    self.run(&format!("UID EXPUNGE {set}"))?;
                }
            }
            false => {
                self.run("EXPUNGE")?;
            }
        }
        Ok(())
    }

    pub fn move_to(&mut self, uids: &[u32], target: &str) -> Result<()> {
        let target = mailbox_argument(target)?;
        if self.has("MOVE") {
            for set in uid_sets(uids) {
                self.run(&format!("UID MOVE {set} {target}"))?;
            }
            return Ok(());
        }
        for set in uid_sets(uids) {
            self.run(&format!("UID COPY {set} {target}"))?;
        }
        self.expunge(uids)
    }

    pub fn append(&mut self, path: &str, flags: &[Flag], message: &[u8]) -> Result<()> {
        let flags = flags
            .iter()
            .map(|flag| flag.atom())
            .collect::<Vec<_>>()
            .join(" ");
        let tag = self.next_tag();
        let mailbox = mailbox_argument(path)?;
        let length = message.len();
        if self.has("LITERAL+") {
            self.link
                .send(format!("{tag} APPEND {mailbox} ({flags}) {{{length}+}}\r\n").as_bytes())?;
        } else {
            self.link
                .send(format!("{tag} APPEND {mailbox} ({flags}) {{{length}}}\r\n").as_bytes())?;
            self.await_continuation(&tag)?;
        }
        self.link.send(message)?;
        self.link.send(b"\r\n")?;
        self.finish(&tag).map(drop)
    }

    pub fn noop(&mut self) -> Result<bool> {
        self.run("NOOP")?;
        Ok(std::mem::take(&mut self.changed))
    }

    pub fn idle(&mut self, wait: Duration) -> Result<bool> {
        if !self.has("IDLE") {
            return Err(Error::Refused("the server cannot IDLE".into()));
        }
        let tag = self.next_tag();
        self.link.send(format!("{tag} IDLE\r\n").as_bytes())?;
        self.await_continuation(&tag)?;
        let changed = std::mem::take(&mut self.changed);
        self.link.set_read_timeout(wait)?;
        let waited = match changed {
            true => Ok(()),
            false => loop {
                match self.read_response() {
                    Ok(Response::Data(values)) => {
                        self.note(&values);
                        if self.changed {
                            break Ok(());
                        }
                    }
                    Ok(Response::Status {
                        tag: None,
                        status: Status::Bye,
                        text,
                        ..
                    }) => break Err(Error::Refused(text)),
                    Ok(_) => {}
                    Err(Error::Timeout) => break Ok(()),
                    Err(error) => break Err(error),
                }
            },
        };
        self.link.set_read_timeout(COMMAND_TIMEOUT)?;
        waited?;
        self.link.send(b"DONE\r\n")?;
        self.finish(&tag)?;
        Ok(std::mem::take(&mut self.changed))
    }

    pub fn logout(mut self) {
        let _ = self.run("LOGOUT");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::wire::parse_values;

    #[test]
    fn uid_sets_collapse_runs_and_split_long_sets() {
        assert_eq!(uid_sets(&[5, 1, 2, 3, 9, 7, 8, 2]), ["1:3,5,7:9"]);
        assert!(uid_sets(&[]).is_empty());
        let scattered = (0..2000).map(|uid| uid * 2).collect::<Vec<_>>();
        let sets = uid_sets(&scattered);
        assert!(sets.len() > 1);
        assert!(sets.iter().all(|set| set.len() <= MAX_SET_LENGTH));
        let count: usize = sets.iter().map(|set| set.split(',').count()).sum();
        assert_eq!(count, 2000);
    }

    #[test]
    fn a_fetch_response_becomes_a_message_summary() {
        let values = parse_values(
            b"3 FETCH (FLAGS (\\Seen \\Answered) UID 42 RFC822.SIZE 2048 INTERNALDATE \" 7-Jul-2026 02:44:25 -0700\" BODY[HEADER] {8}\r\nA: b\r\n\r\n)",
        )
        .unwrap_or_default();
        let found = fetched(&values);
        assert_eq!(
            found,
            Some(Fetched {
                uid: 42,
                flags: Some(Flags {
                    seen: true,
                    answered: true,
                    ..Flags::default()
                }),
                size: Some(2048),
                internal_date: Some(1_783_417_465),
                header: Some(b"A: b\r\n\r\n".to_vec()),
                body: None,
            })
        );
        let unsolicited = parse_values(b"4 FETCH (FLAGS (\\Seen))").unwrap_or_default();
        assert_eq!(fetched(&unsolicited), None);
    }

    #[test]
    fn mailbox_arguments_are_quoted_and_utf7_encoded() {
        assert_eq!(
            mailbox_argument("Entwürfe").ok().as_deref(),
            Some("\"Entw&APw-rfe\"")
        );
        assert!(mailbox_argument("a\r\nb").is_err());
    }
}
