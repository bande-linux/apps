use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bande_mail::imap::wire::{Value, parse_values};

pub const USER: &str = "ann@example.com";
pub const PASSWORD: &str = "pässword \"1\"";

#[derive(Clone)]
pub struct Message {
    pub uid: u32,
    pub flags: BTreeSet<String>,
    pub raw: Vec<u8>,
}

#[derive(Clone)]
pub struct Folder {
    pub validity: u32,
    pub next_uid: u32,
    pub special: Option<&'static str>,
    pub messages: Vec<Message>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    Normal,
    Silent,
    Garbage,
    HangUpAfterLogin,
}

pub struct State {
    pub folders: BTreeMap<String, Folder>,
    pub idle: bool,
    pub moves: bool,
    pub special_use: bool,
    pub behaviour: Behaviour,
    pub log: Vec<String>,
    pub sent: Vec<(String, Vec<String>, Vec<u8>)>,
    pub generation: u64,
}

impl State {
    fn deliver(&mut self, folder: &str, raw: &[u8], flags: &[&str]) -> u32 {
        let folder = self.folders.entry(folder.to_owned()).or_insert(Folder {
            validity: 1,
            next_uid: 1,
            special: None,
            messages: Vec::new(),
        });
        let uid = folder.next_uid;
        folder.next_uid += 1;
        folder.messages.push(Message {
            uid,
            flags: flags.iter().map(|flag| (*flag).to_owned()).collect(),
            raw: raw.to_vec(),
        });
        self.generation += 1;
        uid
    }
}

#[derive(Clone)]
pub struct Fake {
    pub state: Arc<Mutex<State>>,
    pub imap_port: u16,
    pub smtp_port: u16,
}

pub fn message(subject: &str, body: &str) -> Vec<u8> {
    format!(
        "From: Bob <bob@example.com>\r\nTo: Ann <ann@example.com>\r\nSubject: {subject}\r\n\
Date: Tue, 27 Oct 2026 09:30:00 +0100\r\nMessage-ID: <{}@example.com>\r\n\r\n{body}\r\n",
        subject.replace(' ', ".")
    )
    .into_bytes()
}

fn folder(validity: u32, special: Option<&'static str>) -> Folder {
    Folder {
        validity,
        next_uid: 1,
        special,
        messages: Vec::new(),
    }
}

impl Fake {
    pub fn start() -> Self {
        let mut folders = BTreeMap::new();
        folders.insert("INBOX".to_owned(), folder(7, None));
        folders.insert("Sent".to_owned(), folder(8, Some("\\Sent")));
        folders.insert("Papierkorb".to_owned(), folder(9, Some("\\Trash")));
        folders.insert("Archive".to_owned(), folder(10, Some("\\Archive")));
        folders.insert("Entw&APw-rfe".to_owned(), folder(11, Some("\\Drafts")));
        let state = Arc::new(Mutex::new(State {
            folders,
            idle: true,
            moves: true,
            special_use: true,
            behaviour: Behaviour::Normal,
            log: Vec::new(),
            sent: Vec::new(),
            generation: 0,
        }));
        let imap = TcpListener::bind("127.0.0.1:0").unwrap();
        let smtp = TcpListener::bind("127.0.0.1:0").unwrap();
        let fake = Self {
            state: Arc::clone(&state),
            imap_port: imap.local_addr().unwrap().port(),
            smtp_port: smtp.local_addr().unwrap().port(),
        };
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in imap.incoming().flatten() {
                let state = Arc::clone(&shared);
                thread::spawn(move || {
                    let _ = Connection::new(stream, state).serve();
                });
            }
        });
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in smtp.incoming().flatten() {
                let state = Arc::clone(&shared);
                thread::spawn(move || {
                    let _ = serve_smtp(stream, &state);
                });
            }
        });
        fake
    }

    pub fn deliver(&self, folder: &str, raw: &[u8], flags: &[&str]) -> u32 {
        self.state.lock().unwrap().deliver(folder, raw, flags)
    }

    pub fn flags(&self, folder: &str, uid: u32) -> Option<BTreeSet<String>> {
        let state = self.state.lock().unwrap();
        state.folders[folder]
            .messages
            .iter()
            .find(|message| message.uid == uid)
            .map(|message| message.flags.clone())
    }

    pub fn set_flag(&self, folder: &str, uid: u32, flag: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(message) = state.folders.get_mut(folder).and_then(|folder| {
            folder
                .messages
                .iter_mut()
                .find(|message| message.uid == uid)
        }) {
            message.flags.insert(flag.to_owned());
        }
        state.generation += 1;
    }

    pub fn subjects(&self, folder: &str) -> Vec<String> {
        let state = self.state.lock().unwrap();
        state.folders[folder]
            .messages
            .iter()
            .map(|message| bande_mail::message::Envelope::parse(&message.raw).subject)
            .collect()
    }

    pub fn with<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        change(&mut self.state.lock().unwrap())
    }
}

struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    state: Arc<Mutex<State>>,
    selected: Option<String>,
}

fn parse_set(set: &str, highest: u32) -> Vec<(u32, u32)> {
    set.split(',')
        .map(|range| {
            let bound = |text: &str| match text {
                "*" => highest,
                number => number.parse().unwrap_or(0),
            };
            match range.split_once(':') {
                Some((start, end)) => {
                    let (start, end) = (bound(start), bound(end));
                    (start.min(end), start.max(end))
                }
                None => (bound(range), bound(range)),
            }
        })
        .collect()
}

fn in_set(set: &[(u32, u32)], uid: u32) -> bool {
    set.iter()
        .any(|(start, end)| (*start..=*end).contains(&uid))
}

fn literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("{{{}}}\r\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out
}

fn header_of(raw: &[u8]) -> &[u8] {
    raw.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map_or(raw, |end| &raw[..end + 4])
}

impl Connection {
    fn new(stream: TcpStream, state: Arc<Mutex<State>>) -> Self {
        Self {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
            state,
            selected: None,
        }
    }

    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)
    }

    fn line(&mut self, text: &str) -> std::io::Result<()> {
        self.send(format!("{text}\r\n").as_bytes())
    }

    fn read_command(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        let mut raw = Vec::new();
        if self.reader.read_until(b'\n', &mut raw)? == 0 {
            return Ok(None);
        }
        while let Some(length) = bande_mail::imap::wire::literal_length(&raw) {
            if !raw.ends_with(b"+}\r\n") {
                self.line("+ go ahead")?;
            }
            let mut literal = vec![0; length];
            self.reader.read_exact(&mut literal)?;
            raw.extend_from_slice(&literal);
            self.reader.read_until(b'\n', &mut raw)?;
        }
        Ok(Some(raw))
    }

    fn capabilities(&self) -> String {
        let state = self.state.lock().unwrap();
        let mut capabilities = vec!["IMAP4rev1", "AUTH=PLAIN", "SASL-IR", "UIDPLUS", "LITERAL+"];
        if state.idle {
            capabilities.push("IDLE");
        }
        if state.moves {
            capabilities.push("MOVE");
        }
        if state.special_use {
            capabilities.push("SPECIAL-USE");
        }
        capabilities.join(" ")
    }

    fn serve(mut self) -> std::io::Result<()> {
        let behaviour = self.state.lock().unwrap().behaviour;
        match behaviour {
            Behaviour::Silent => {
                thread::sleep(Duration::from_secs(30));
                return Ok(());
            }
            Behaviour::Garbage => {
                self.send(b"\x00\x01 garbage (((\r\n")?;
                thread::sleep(Duration::from_secs(5));
                return Ok(());
            }
            _ => {}
        }
        let greeting = format!("* OK [CAPABILITY {}] fake ready", self.capabilities());
        self.line(&greeting)?;
        while let Some(raw) = self.read_command()? {
            let text = String::from_utf8_lossy(&raw).trim_end().to_owned();
            self.state.lock().unwrap().log.push(text.clone());
            let Some((tag, rest)) = text.split_once(' ') else {
                self.line("* BAD no tag")?;
                continue;
            };
            let tag = tag.to_owned();
            let (command, arguments) = rest.split_once(' ').unwrap_or((rest, ""));
            let command = command.to_ascii_uppercase();
            let arguments = arguments.to_owned();
            let body = raw[raw.len() - arguments.len().min(raw.len())..].to_vec();
            let done = match command.as_str() {
                "UID" => {
                    let (sub, rest) = arguments.split_once(' ').unwrap_or((&arguments, ""));
                    self.uid_command(&tag, &sub.to_ascii_uppercase(), rest)
                }
                other => self.command(&tag, other, &arguments, &raw, &body),
            };
            match done {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn command(
        &mut self,
        tag: &str,
        command: &str,
        arguments: &str,
        raw: &[u8],
        _body: &[u8],
    ) -> std::io::Result<bool> {
        match command {
            "CAPABILITY" => {
                let capabilities = self.capabilities();
                self.line(&format!("* CAPABILITY {capabilities}"))?;
                self.line(&format!("{tag} OK done"))?;
            }
            "NOOP" => self.line(&format!("{tag} OK done"))?,
            "LOGOUT" => {
                self.line("* BYE bye")?;
                self.line(&format!("{tag} OK done"))?;
                return Ok(true);
            }
            "AUTHENTICATE" => {
                let token = arguments.split_whitespace().nth(1).unwrap_or_default();
                let decoded = data_encoding::BASE64
                    .decode(token.as_bytes())
                    .unwrap_or_default();
                let expected = format!("\0{USER}\0{PASSWORD}");
                if decoded == expected.as_bytes() {
                    let capabilities = self.capabilities();
                    self.line(&format!("{tag} OK [CAPABILITY {capabilities}] welcome"))?;
                    if self.state.lock().unwrap().behaviour == Behaviour::HangUpAfterLogin {
                        return Ok(true);
                    }
                } else {
                    self.line(&format!(
                        "{tag} NO [AUTHENTICATIONFAILED] Invalid credentials"
                    ))?;
                }
            }
            "LIST" => {
                let state = self.state.lock().unwrap();
                let lines = state
                    .folders
                    .iter()
                    .map(|(name, folder)| {
                        let special = match (state.special_use, folder.special) {
                            (true, Some(special)) => format!(" {special}"),
                            _ => String::new(),
                        };
                        format!("* LIST (\\HasNoChildren{special}) \"/\" \"{name}\"")
                    })
                    .collect::<Vec<_>>();
                drop(state);
                for line in lines {
                    self.line(&line)?;
                }
                self.line(&format!("{tag} OK done"))?;
            }
            "SELECT" | "EXAMINE" => {
                let name = parse_values(arguments.as_bytes())
                    .ok()
                    .and_then(|values| values.first().and_then(Value::text))
                    .unwrap_or_default();
                let state = self.state.lock().unwrap();
                let Some(folder) = state.folders.get(&name).cloned() else {
                    drop(state);
                    self.line(&format!("{tag} NO no such mailbox"))?;
                    return Ok(false);
                };
                drop(state);
                self.selected = Some(name);
                self.line(&format!("* {} EXISTS", folder.messages.len()))?;
                self.line("* FLAGS (\\Seen \\Flagged \\Answered \\Deleted \\Draft)")?;
                self.line(&format!("* OK [UIDVALIDITY {}] ok", folder.validity))?;
                self.line(&format!("* OK [UIDNEXT {}] ok", folder.next_uid))?;
                self.line(&format!("{tag} OK [READ-WRITE] selected"))?;
            }
            "APPEND" => {
                let values = parse_values(
                    raw.splitn(3, |byte| *byte == b' ')
                        .nth(2)
                        .unwrap_or_default(),
                )
                .unwrap_or_default();
                let name = values.first().and_then(Value::text).unwrap_or_default();
                let flags = values
                    .get(1)
                    .and_then(Value::list)
                    .map(|flags| flags.iter().filter_map(Value::text).collect::<Vec<_>>())
                    .unwrap_or_default();
                let message = values
                    .last()
                    .and_then(Value::bytes)
                    .unwrap_or_default()
                    .to_vec();
                let flags = flags.iter().map(String::as_str).collect::<Vec<_>>();
                self.state.lock().unwrap().deliver(&name, &message, &flags);
                self.line(&format!("{tag} OK appended"))?;
            }
            "IDLE" => {
                self.line("+ idling")?;
                let start = self.state.lock().unwrap().generation;
                self.reader
                    .get_ref()
                    .set_read_timeout(Some(Duration::from_millis(50)))?;
                let mut line = Vec::new();
                loop {
                    let generation = self.state.lock().unwrap().generation;
                    if generation != start {
                        let count = self.selected_messages().len();
                        self.line(&format!("* {count} EXISTS"))?;
                        self.state.lock().unwrap().generation = generation;
                    }
                    match self.reader.read_until(b'\n', &mut line) {
                        Ok(0) => return Ok(true),
                        Ok(_) if line.ends_with(b"\n") => break,
                        Ok(_) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(error) => return Err(error),
                    }
                }
                self.reader.get_ref().set_read_timeout(None)?;
                self.line(&format!("{tag} OK idle done"))?;
            }
            "EXPUNGE" => {
                self.expunge(None)?;
                self.line(&format!("{tag} OK done"))?;
            }
            _ => self.line(&format!("{tag} BAD unknown command"))?,
        }
        Ok(false)
    }

    fn selected_messages(&self) -> Vec<Message> {
        let state = self.state.lock().unwrap();
        self.selected
            .as_ref()
            .and_then(|name| state.folders.get(name))
            .map(|folder| folder.messages.clone())
            .unwrap_or_default()
    }

    fn expunge(&mut self, only: Option<&[(u32, u32)]>) -> std::io::Result<()> {
        let Some(name) = self.selected.clone() else {
            return Ok(());
        };
        let mut gone = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            let folder = state.folders.get_mut(&name).unwrap();
            for (index, message) in folder.messages.iter().enumerate().rev() {
                if message.flags.contains("\\Deleted")
                    && only.is_none_or(|set| in_set(set, message.uid))
                {
                    gone.push(index + 1);
                }
            }
            folder.messages.retain(|message| {
                !(message.flags.contains("\\Deleted")
                    && only.is_none_or(|set| in_set(set, message.uid)))
            });
            state.generation += 1;
        }
        for sequence in gone {
            self.line(&format!("* {sequence} EXPUNGE"))?;
        }
        Ok(())
    }

    fn uid_command(&mut self, tag: &str, command: &str, arguments: &str) -> std::io::Result<bool> {
        let Some(name) = self.selected.clone() else {
            self.line(&format!("{tag} BAD nothing selected"))?;
            return Ok(false);
        };
        let messages = self.selected_messages();
        let highest = messages.last().map_or(0, |message| message.uid);
        let (set_text, rest) = arguments.split_once(' ').unwrap_or((arguments, ""));
        match command {
            "SEARCH" => {
                let uids = messages
                    .iter()
                    .map(|message| message.uid.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.line(format!("* SEARCH {uids}").trim_end())?;
            }
            "FETCH" => {
                let set = parse_set(set_text, highest);
                let items = parse_values(rest.as_bytes()).unwrap_or_default();
                let items = items
                    .first()
                    .and_then(Value::list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::atom)
                            .map(str::to_ascii_uppercase)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for (index, message) in messages.iter().enumerate() {
                    if !in_set(&set, message.uid) {
                        continue;
                    }
                    let mut response =
                        format!("* {} FETCH (UID {}", index + 1, message.uid).into_bytes();
                    for item in &items {
                        match item.as_str() {
                            "FLAGS" => {
                                let flags =
                                    message.flags.iter().cloned().collect::<Vec<_>>().join(" ");
                                response.extend(format!(" FLAGS ({flags})").into_bytes());
                            }
                            "RFC822.SIZE" => {
                                response.extend(
                                    format!(" RFC822.SIZE {}", message.raw.len()).into_bytes(),
                                );
                            }
                            "INTERNALDATE" => {
                                response.extend(b" INTERNALDATE \"27-Oct-2026 09:30:00 +0100\"");
                            }
                            "BODY.PEEK[HEADER]" => {
                                response.extend(b" BODY[HEADER] ");
                                response.extend(literal(header_of(&message.raw)));
                            }
                            "BODY.PEEK[]" => {
                                response.extend(b" BODY[] ");
                                response.extend(literal(&message.raw));
                            }
                            _ => {}
                        }
                    }
                    response.extend(b")\r\n");
                    self.send(&response)?;
                }
            }
            "STORE" => {
                let set = parse_set(set_text, highest);
                let (mode, flags) = rest.split_once(' ').unwrap_or((rest, ""));
                let flags = flags
                    .trim_matches(['(', ')'])
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let mut state = self.state.lock().unwrap();
                let folder = state.folders.get_mut(&name).unwrap();
                for message in folder
                    .messages
                    .iter_mut()
                    .filter(|message| in_set(&set, message.uid))
                {
                    for flag in &flags {
                        match mode.starts_with('+') {
                            true => message.flags.insert(flag.clone()),
                            false => message.flags.remove(flag),
                        };
                    }
                }
            }
            "MOVE" | "COPY" => {
                let set = parse_set(set_text, highest);
                let target = parse_values(rest.as_bytes())
                    .ok()
                    .and_then(|values| values.first().and_then(Value::text))
                    .unwrap_or_default();
                {
                    let mut state = self.state.lock().unwrap();
                    if !state.folders.contains_key(&target) {
                        drop(state);
                        self.line(&format!("{tag} NO [TRYCREATE] no such mailbox"))?;
                        return Ok(false);
                    }
                    for message in messages.iter().filter(|message| in_set(&set, message.uid)) {
                        let flags = message.flags.iter().map(String::as_str).collect::<Vec<_>>();
                        state.deliver(&target, &message.raw, &flags);
                    }
                    if command == "MOVE" {
                        let folder = state.folders.get_mut(&name).unwrap();
                        for message in folder
                            .messages
                            .iter_mut()
                            .filter(|message| in_set(&set, message.uid))
                        {
                            message.flags.insert("\\Deleted".to_owned());
                        }
                    }
                }
                if command == "MOVE" {
                    self.expunge(Some(&set))?;
                }
            }
            "EXPUNGE" => {
                let set = parse_set(set_text, highest);
                self.expunge(Some(&set))?;
            }
            _ => {
                self.line(&format!("{tag} BAD unknown UID command"))?;
                return Ok(false);
            }
        }
        self.line(&format!("{tag} OK done"))?;
        Ok(false)
    }
}

fn serve_smtp(stream: TcpStream, state: &Arc<Mutex<State>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let line =
        |writer: &mut TcpStream, text: &str| writer.write_all(format!("{text}\r\n").as_bytes());
    line(&mut writer, "220 fake smtp")?;
    let mut from = String::new();
    let mut recipients = Vec::new();
    let mut authenticated = false;
    loop {
        let mut raw = String::new();
        if reader.read_line(&mut raw)? == 0 {
            return Ok(());
        }
        let command = raw.trim_end().to_owned();
        let upper = command.to_ascii_uppercase();
        if upper.starts_with("EHLO") {
            line(&mut writer, "250-fake")?;
            line(&mut writer, "250-AUTH PLAIN LOGIN")?;
            line(&mut writer, "250 8BITMIME")?;
        } else if upper.starts_with("AUTH PLAIN ") {
            let decoded = data_encoding::BASE64
                .decode(&command.as_bytes()["AUTH PLAIN ".len()..])
                .unwrap_or_default();
            authenticated = decoded == format!("\0{USER}\0{PASSWORD}").as_bytes();
            match authenticated {
                true => line(&mut writer, "235 ok")?,
                false => line(&mut writer, "535 5.7.8 bad credentials")?,
            }
        } else if upper.starts_with("MAIL FROM:") {
            match authenticated {
                true => {
                    from = command[10..].trim_matches(['<', '>']).to_owned();
                    line(&mut writer, "250 ok")?;
                }
                false => line(&mut writer, "530 authenticate first")?,
            }
        } else if upper.starts_with("RCPT TO:") {
            recipients.push(command[8..].trim_matches(['<', '>']).to_owned());
            line(&mut writer, "250 ok")?;
        } else if upper == "DATA" {
            line(&mut writer, "354 go")?;
            let mut data = Vec::new();
            loop {
                let mut chunk = Vec::new();
                reader.read_until(b'\n', &mut chunk)?;
                if chunk == b".\r\n" || chunk.is_empty() {
                    break;
                }
                let chunk = chunk
                    .strip_prefix(b".")
                    .map(<[u8]>::to_vec)
                    .unwrap_or(chunk);
                data.extend(chunk);
            }
            state
                .lock()
                .unwrap()
                .sent
                .push((from.clone(), recipients.clone(), data));
            line(&mut writer, "250 queued")?;
        } else if upper == "QUIT" {
            line(&mut writer, "221 bye")?;
            return Ok(());
        } else {
            line(&mut writer, "502 unknown")?;
        }
    }
}
