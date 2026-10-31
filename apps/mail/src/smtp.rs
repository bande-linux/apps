use data_encoding::BASE64;

use crate::error::{Error, Result};
use crate::net::{Link, Network, Security, Server};

const MAX_LINE: usize = 64 * 1024;

struct Reply {
    code: u16,
    lines: Vec<String>,
}

impl Reply {
    fn text(&self) -> String {
        format!("{} {}", self.code, self.lines.join(" "))
    }
}

struct Smtp {
    link: Link,
}

impl Smtp {
    fn read(&mut self) -> Result<Reply> {
        let mut lines = Vec::new();
        loop {
            let raw = self.link.read_line(MAX_LINE)?;
            let line = String::from_utf8_lossy(&raw).trim_end().to_owned();
            let code = line
                .get(..3)
                .and_then(|code| code.parse::<u16>().ok())
                .ok_or_else(|| Error::Protocol(format!("SMTP reply {line:?}")))?;
            let more = line.as_bytes().get(3) == Some(&b'-');
            lines.push(line.get(4..).unwrap_or_default().to_owned());
            if !more {
                return Ok(Reply { code, lines });
            }
        }
    }

    fn expect(&mut self, wanted: &[u16]) -> Result<Reply> {
        let reply = self.read()?;
        match wanted.contains(&reply.code) {
            true => Ok(reply),
            false if reply.code == 535 || reply.code == 534 => Err(Error::Login(reply.text())),
            false if reply.code >= 400 => Err(Error::Refused(reply.text())),
            false => Err(Error::Protocol(reply.text())),
        }
    }

    fn command(&mut self, line: &str, wanted: &[u16]) -> Result<Reply> {
        self.link.send(format!("{line}\r\n").as_bytes())?;
        self.expect(wanted)
    }

    fn hello(&mut self, name: &str) -> Result<Vec<String>> {
        let reply = self.command(&format!("EHLO {name}"), &[250])?;
        Ok(reply.lines.into_iter().skip(1).collect())
    }
}

fn has(extensions: &[String], name: &str) -> bool {
    extensions.iter().any(|extension| {
        extension
            .split_whitespace()
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case(name))
    })
}

fn auth_mechanisms(extensions: &[String]) -> Vec<String> {
    extensions
        .iter()
        .filter(|extension| {
            extension
                .split_whitespace()
                .next()
                .is_some_and(|word| word.eq_ignore_ascii_case("AUTH"))
        })
        .flat_map(|extension| extension.split_whitespace().skip(1))
        .map(str::to_ascii_uppercase)
        .collect()
}

pub fn dot_stuffed(message: &[u8]) -> Vec<u8> {
    let text = message
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .collect::<Vec<_>>();
    let mut stuffed = Vec::with_capacity(message.len() + 64);
    let count = match text.last() {
        Some([]) => text.len() - 1,
        _ => text.len(),
    };
    for line in &text[..count] {
        if line.first() == Some(&b'.') {
            stuffed.push(b'.');
        }
        stuffed.extend_from_slice(line);
        stuffed.extend_from_slice(b"\r\n");
    }
    stuffed.extend_from_slice(b".\r\n");
    stuffed
}

pub struct Envelope<'a> {
    pub from: &'a str,
    pub recipients: &'a [String],
    pub message: &'a [u8],
}

fn checked(address: &str) -> Result<&str> {
    match address.contains(['<', '>', '\r', '\n', ' ']) || !address.contains('@') {
        true => Err(Error::Refused(format!("{address:?} is not a mail address"))),
        false => Ok(address),
    }
}

pub fn send(
    server: &Server,
    network: &Network,
    login: Option<(&str, &str)>,
    envelope: &Envelope<'_>,
) -> Result<()> {
    if envelope.recipients.is_empty() {
        return Err(Error::Refused("the message has no recipients".to_owned()));
    }
    let mut smtp = Smtp {
        link: Link::connect(server, network)?,
    };
    smtp.expect(&[220])?;
    let name = "localhost";
    let mut extensions = smtp.hello(name)?;
    if server.security == Security::StartTls {
        if !has(&extensions, "STARTTLS") {
            return Err(Error::Insecure);
        }
        smtp.command("STARTTLS", &[220])?;
        smtp.link = smtp.link.start_tls()?;
        extensions = smtp.hello(name)?;
    }
    let mechanisms = auth_mechanisms(&extensions);
    if let Some((user, password)) = login.filter(|_| !mechanisms.is_empty()) {
        if mechanisms.iter().any(|mechanism| mechanism == "PLAIN") {
            let token = BASE64.encode(format!("\0{user}\0{password}").as_bytes());
            smtp.command(&format!("AUTH PLAIN {token}"), &[235])?;
        } else if mechanisms.iter().any(|mechanism| mechanism == "LOGIN") {
            smtp.command("AUTH LOGIN", &[334])?;
            smtp.command(&BASE64.encode(user.as_bytes()), &[334])?;
            smtp.command(&BASE64.encode(password.as_bytes()), &[235])?;
        } else {
            return Err(Error::Login(format!(
                "no supported sign-in method among {}",
                mechanisms.join(" ")
            )));
        }
    }
    let utf8 = match has(&extensions, "SMTPUTF8") && !envelope.from.is_ascii() {
        true => " SMTPUTF8",
        false => "",
    };
    smtp.command(
        &format!("MAIL FROM:<{}>{utf8}", checked(envelope.from)?),
        &[250],
    )?;
    for recipient in envelope.recipients {
        smtp.command(&format!("RCPT TO:<{}>", checked(recipient)?), &[250, 251])?;
    }
    smtp.command("DATA", &[354])?;
    smtp.link.send(&dot_stuffed(envelope.message))?;
    smtp.expect(&[250])?;
    let _ = smtp.command("QUIT", &[221]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_is_dot_stuffed_and_normalised_to_crlf() {
        assert_eq!(
            dot_stuffed(b"Subject: x\n\n.hidden\r\nplain\n."),
            b"Subject: x\r\n\r\n..hidden\r\nplain\r\n..\r\n.\r\n"
        );
        assert_eq!(dot_stuffed(b"a\r\n"), b"a\r\n.\r\n");
    }

    #[test]
    fn auth_mechanisms_are_read_from_the_extensions() {
        let extensions = vec![
            "SIZE 1000".to_owned(),
            "AUTH login Plain".to_owned(),
            "STARTTLS".to_owned(),
        ];
        assert_eq!(auth_mechanisms(&extensions), ["LOGIN", "PLAIN"]);
        assert!(has(&extensions, "starttls"));
        assert!(!has(&extensions, "SMTPUTF8"));
    }

    #[test]
    fn addresses_cannot_smuggle_commands() {
        assert!(checked("a@b.c").is_ok());
        assert!(checked("a@b.c>\r\nRCPT TO:<x@y.z").is_err());
        assert!(checked("nobody").is_err());
    }
}
