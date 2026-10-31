use std::hash::{BuildHasher, RandomState};

use data_encoding::BASE64;
use jiff::Zoned;

use crate::message::{Address, Attachment, Content};

const LINE_LENGTH: usize = 76;
const WORD_BYTES: usize = 45;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<Attachment>,
}

impl Draft {
    pub fn recipients(&self) -> Vec<String> {
        let mut recipients = Vec::new();
        for address in self.to.iter().chain(&self.cc).chain(&self.bcc) {
            let email = address.email.trim().to_owned();
            if !email.is_empty() && !recipients.contains(&email) {
                recipients.push(email);
            }
        }
        recipients
    }
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let trimmed = subject.trim();
    let already = trimmed
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix));
    match already {
        true => trimmed.to_owned(),
        false => format!("{prefix} {trimmed}"),
    }
}

fn quoted(text: &str) -> String {
    text.replace("\r\n", "\n")
        .trim_end()
        .lines()
        .map(|line| match line.starts_with('>') {
            true => format!(">{line}"),
            false => format!("> {line}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn same_person(a: &Address, b: &Address) -> bool {
    a.email.eq_ignore_ascii_case(&b.email)
}

pub fn reply(original: &Content, me: &Address, all: bool, attribution: &str) -> Draft {
    let envelope = &original.envelope;
    let primary = match envelope.reply_to.is_empty() {
        true => envelope.from.clone(),
        false => envelope.reply_to.clone(),
    };
    let mut to = primary;
    let mut cc = Vec::new();
    if all {
        for address in &envelope.to {
            if !same_person(address, me) && !to.iter().any(|known| same_person(known, address)) {
                to.push(address.clone());
            }
        }
        for address in &envelope.cc {
            if !same_person(address, me) && !to.iter().any(|known| same_person(known, address)) {
                cc.push(address.clone());
            }
        }
    }
    if to.is_empty() {
        to = envelope.from.clone();
    }
    let mut references = envelope.references.clone();
    references.extend(envelope.message_id.clone());
    Draft {
        from: me.clone(),
        to,
        cc,
        subject: prefixed("Re:", &envelope.subject),
        body: format!("\n\n{attribution}\n{}\n", quoted(&original.text())),
        in_reply_to: envelope.message_id.clone(),
        references,
        ..Draft::default()
    }
}

pub fn forward(original: &Content, me: &Address, heading: &str) -> Draft {
    Draft {
        from: me.clone(),
        subject: prefixed("Fwd:", &original.envelope.subject),
        body: format!("\n\n{heading}\n\n{}\n", original.text().trim_end()),
        attachments: original.attachments.clone(),
        ..Draft::default()
    }
}

fn encoded_words(text: &str) -> String {
    if text.is_ascii() && !text.contains("=?") {
        return text.to_owned();
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for character in text.chars() {
        if chunk.len() + character.len_utf8() > WORD_BYTES {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(character);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words
        .iter()
        .map(|word| format!("=?UTF-8?B?{}?=", BASE64.encode(word.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

fn header_address(address: &Address) -> String {
    let name = address.name.trim();
    if name.is_empty() {
        return format!("<{}>", address.email);
    }
    if !name.is_ascii() {
        return format!("{} <{}>", encoded_words(name), address.email);
    }
    Address {
        name: name.to_owned(),
        email: address.email.clone(),
    }
    .full()
}

fn address_header(name: &str, addresses: &[Address]) -> Option<String> {
    (!addresses.is_empty()).then(|| {
        let joined = addresses
            .iter()
            .map(header_address)
            .collect::<Vec<_>>()
            .join(",\r\n ");
        format!("{name}: {joined}\r\n")
    })
}

fn sanitized_header_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn quoted_printable(text: &str) -> String {
    let mut encoded = String::new();
    let text = text.replace("\r\n", "\n");
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            encoded.push_str("\r\n");
        }
        let mut width = 0;
        let bytes = line.as_bytes();
        for (position, byte) in bytes.iter().enumerate() {
            let last = position + 1 == bytes.len();
            let literal = match byte {
                b'=' => false,
                b' ' | b'\t' => !last,
                33..=126 => !(width == 0 && *byte == b'.'),
                _ => false,
            };
            let piece = match literal {
                true => (*byte as char).to_string(),
                false => format!("={byte:02X}"),
            };
            if width + piece.len() > LINE_LENGTH - 1 {
                encoded.push_str("=\r\n");
                width = 0;
            }
            width += piece.len();
            encoded.push_str(&piece);
        }
    }
    encoded
}

fn wrapped_base64(data: &[u8]) -> String {
    let encoded = BASE64.encode(data);
    encoded
        .as_bytes()
        .chunks(LINE_LENGTH)
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn unique_token() -> String {
    let state = RandomState::new();
    let now = jiff::Timestamp::now().as_nanosecond();
    format!(
        "{:016x}{:08x}",
        state.hash_one((now, std::process::id())),
        now as u32
    )
}

fn domain_of(address: &Address) -> &str {
    address
        .email
        .rsplit_once('@')
        .map_or("localhost", |(_, domain)| domain)
}

fn parameter(key: &str, value: &str) -> String {
    if value.is_ascii() {
        return format!("{key}=\"{}\"", value.replace(['"', '\\', '\r', '\n'], "_"));
    }
    let encoded = value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect::<String>();
    format!("{key}*=utf-8''{encoded}")
}

pub fn build(draft: &Draft, now: &Zoned) -> Vec<u8> {
    let mut message = String::new();
    message.push_str(&format!(
        "Date: {}\r\n",
        now.strftime("%a, %d %b %Y %H:%M:%S %z")
    ));
    message.extend(address_header("From", std::slice::from_ref(&draft.from)));
    message.extend(address_header("To", &draft.to));
    message.extend(address_header("Cc", &draft.cc));
    message.push_str(&format!(
        "Subject: {}\r\n",
        encoded_words(&sanitized_header_text(&draft.subject))
    ));
    message.push_str(&format!(
        "Message-ID: <{}@{}>\r\n",
        unique_token(),
        domain_of(&draft.from)
    ));
    if let Some(parent) = &draft.in_reply_to {
        message.push_str(&format!(
            "In-Reply-To: {}\r\n",
            sanitized_header_text(parent)
        ));
    }
    if !draft.references.is_empty() {
        message.push_str(&format!(
            "References: {}\r\n",
            sanitized_header_text(&draft.references.join(" "))
        ));
    }
    message.push_str("MIME-Version: 1.0\r\nUser-Agent: Bande Mail\r\n");
    let text_part = format!(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n{}\r\n",
        quoted_printable(&draft.body)
    );
    if draft.attachments.is_empty() {
        message.push_str(&text_part);
        return message.into_bytes();
    }
    let boundary = format!("=_bande_{}", unique_token());
    message.push_str(&format!(
        "Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"
    ));
    message.push_str(&format!("--{boundary}\r\n{text_part}"));
    for attachment in &draft.attachments {
        message.push_str(&format!(
            "--{boundary}\r\nContent-Type: {}; {}\r\nContent-Disposition: attachment; {}\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
            attachment.mime,
            parameter("name", &attachment.name),
            parameter("filename", &attachment.name),
            wrapped_base64(&attachment.data),
        ));
    }
    message.push_str(&format!("--{boundary}--\r\n"));
    message.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, email: &str) -> Address {
        Address {
            name: name.to_owned(),
            email: email.to_owned(),
        }
    }

    #[test]
    fn a_built_message_parses_back_to_the_same_content() {
        let draft = Draft {
            from: person("Jörg", "jorg@example.de"),
            to: vec![person("Ann Lee", "ann@example.com")],
            cc: vec![person("", "cc@example.com")],
            bcc: vec![person("", "hidden@example.com")],
            subject: "Grüße\r\nInjected: header".to_owned(),
            body: "Line one = equal\n.dot first\ntrailing space \n".to_owned() + &"x".repeat(200),
            in_reply_to: Some("<p@x>".to_owned()),
            references: vec!["<r@x>".to_owned(), "<p@x>".to_owned()],
            attachments: vec![Attachment {
                name: "Übersicht.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                data: vec![0, 1, 2, 255],
            }],
        };
        let now = jiff::Timestamp::from_second(1_793_089_800)
            .map(|timestamp| timestamp.to_zoned(jiff::tz::TimeZone::UTC))
            .unwrap_or_default();
        let raw = build(&draft, &now);
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("hidden@example.com"));
        assert!(!text.contains("\r\nInjected:"));
        assert!(text.lines().all(|line| line.len() <= 998));
        let parsed = Content::parse(&raw);
        assert_eq!(parsed.envelope.subject, "Grüße Injected: header");
        assert_eq!(parsed.envelope.from, [person("Jörg", "jorg@example.de")]);
        assert_eq!(parsed.envelope.cc, [person("", "cc@example.com")]);
        assert_eq!(parsed.envelope.date, Some(1_793_089_800));
        assert_eq!(parsed.envelope.references, ["<r@x>", "<p@x>"]);
        assert_eq!(
            parsed
                .plain
                .as_deref()
                .map(|body| body.replace("\r\n", "\n")),
            Some(draft.body.clone())
        );
        assert_eq!(parsed.attachments, draft.attachments);
        assert_eq!(
            draft.recipients(),
            ["ann@example.com", "cc@example.com", "hidden@example.com"]
        );
    }

    #[test]
    fn reply_all_leaves_me_out_and_threads_the_message() {
        let original = Content::parse(
            b"From: Ann <ann@example.com>\r\nTo: me@example.com, bob@example.com\r\n\
Cc: Cat <cat@example.com>, ME@example.com\r\nSubject: RE: plans\r\n\
Message-ID: <m1@example.com>\r\nReferences: <m0@example.com>\r\n\r\nHi\r\n> earlier\r\n",
        );
        let me = person("Me", "me@example.com");
        let draft = reply(&original, &me, true, "Ann wrote:");
        assert_eq!(
            draft.to,
            [
                person("Ann", "ann@example.com"),
                person("", "bob@example.com")
            ]
        );
        assert_eq!(draft.cc, [person("Cat", "cat@example.com")]);
        assert_eq!(draft.subject, "RE: plans");
        assert_eq!(draft.in_reply_to.as_deref(), Some("<m1@example.com>"));
        assert_eq!(draft.references, ["<m0@example.com>", "<m1@example.com>"]);
        assert_eq!(draft.body, "\n\nAnn wrote:\n> Hi\n>> earlier\n");
        let single = reply(&original, &me, false, "");
        assert_eq!(single.to, [person("Ann", "ann@example.com")]);
        assert!(single.cc.is_empty());
        let forwarded = forward(&original, &me, "Forwarded");
        assert_eq!(forwarded.subject, "Fwd: RE: plans");
        assert!(forwarded.to.is_empty());
    }
}
