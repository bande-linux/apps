use mailparse::{
    DispositionType, MailAddr, MailHeader, MailHeaderMap, ParsedMail, addrparse, addrparse_header,
    dateparse, parse_headers, parse_mail,
};

use crate::html::{Document, render_html, render_plain};

const PREVIEW_LENGTH: usize = 240;
const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Address {
    pub name: String,
    pub email: String,
}

impl Address {
    pub fn shown(&self) -> &str {
        match self.name.trim().is_empty() {
            true => &self.email,
            false => &self.name,
        }
    }

    pub fn full(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            return self.email.clone();
        }
        let plain = name
            .chars()
            .all(|character| character.is_alphanumeric() || " .-'".contains(character));
        match plain {
            true => format!("{name} <{}>", self.email),
            false => format!("\"{}\" <{}>", name.replace(['"', '\\'], ""), self.email),
        }
    }
}

fn addresses_from(list: mailparse::MailAddrList) -> Vec<Address> {
    list.into_inner()
        .into_iter()
        .flat_map(|address| match address {
            MailAddr::Single(single) => vec![single],
            MailAddr::Group(group) => group.addrs,
        })
        .map(|single| Address {
            name: single.display_name.unwrap_or_default(),
            email: single.addr,
        })
        .collect()
}

pub fn parse_addresses(text: &str) -> Vec<Address> {
    addrparse(text).map(addresses_from).unwrap_or_default()
}

fn header_addresses(headers: &[MailHeader<'_>], key: &str) -> Vec<Address> {
    headers
        .get_all_headers(key)
        .into_iter()
        .filter_map(|header| addrparse_header(header).ok())
        .flat_map(addresses_from)
        .collect()
}

fn message_ids(text: &str) -> Vec<String> {
    text.split('<')
        .skip(1)
        .filter_map(|part| part.split_once('>'))
        .map(|(id, _)| format!("<{}>", id.trim()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Envelope {
    pub subject: String,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub references: Vec<String>,
}

impl Envelope {
    fn from_headers(headers: &[MailHeader<'_>]) -> Self {
        let mut references = headers
            .get_first_value("References")
            .map(|text| message_ids(&text))
            .unwrap_or_default();
        if let Some(parent) = headers
            .get_first_value("In-Reply-To")
            .and_then(|text| message_ids(&text).pop())
            && !references.contains(&parent)
        {
            references.push(parent);
        }
        Self {
            subject: headers
                .get_first_value("Subject")
                .map(|subject| subject.split_whitespace().collect::<Vec<_>>().join(" "))
                .unwrap_or_default(),
            from: header_addresses(headers, "From"),
            to: header_addresses(headers, "To"),
            cc: header_addresses(headers, "Cc"),
            reply_to: header_addresses(headers, "Reply-To"),
            date: headers
                .get_first_value("Date")
                .and_then(|date| dateparse(&date).ok()),
            message_id: headers
                .get_first_value("Message-ID")
                .and_then(|text| message_ids(&text).pop()),
            references,
        }
    }

    pub fn parse(header: &[u8]) -> Self {
        parse_headers(header)
            .map(|(headers, _)| Self::from_headers(&headers))
            .unwrap_or_default()
    }

    pub fn sender(&self) -> Address {
        self.from.first().cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Content {
    pub envelope: Envelope,
    pub plain: Option<String>,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

fn file_name(part: &ParsedMail<'_>) -> Option<String> {
    let disposition = part.get_content_disposition();
    disposition
        .params
        .get("filename")
        .or_else(|| part.ctype.params.get("name"))
        .map(|name| {
            name.rsplit(['/', '\\'])
                .next()
                .unwrap_or(name)
                .trim()
                .to_owned()
        })
        .filter(|name| !name.is_empty())
}

impl Content {
    pub fn parse(raw: &[u8]) -> Self {
        let headerless = |mail: &ParsedMail<'_>| {
            mail.headers
                .iter()
                .all(|header| header.get_key_ref().contains(char::is_whitespace))
        };
        let Some(mail) = parse_mail(raw).ok().filter(|mail| !headerless(mail)) else {
            return Self {
                plain: Some(String::from_utf8_lossy(raw).into_owned()),
                ..Self::default()
            };
        };
        let mut content = Self {
            envelope: Envelope::from_headers(&mail.headers),
            ..Self::default()
        };
        content.walk(&mail, 0);
        content
    }

    fn walk(&mut self, part: &ParsedMail<'_>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mime = part.ctype.mimetype.to_ascii_lowercase();
        let attached = part.get_content_disposition().disposition == DispositionType::Attachment;
        if mime.starts_with("multipart/") {
            for child in &part.subparts {
                self.walk(child, depth + 1);
            }
            return;
        }
        let name = file_name(part);
        match mime.as_str() {
            "text/plain" if !attached && name.is_none() && self.plain.is_none() => {
                self.plain = part.get_body().ok();
            }
            "text/html" if !attached && name.is_none() && self.html.is_none() => {
                self.html = part.get_body().ok();
            }
            _ => {
                let data = part.get_body_raw().unwrap_or_default();
                let name = name.unwrap_or_else(|| match mime.as_str() {
                    "message/rfc822" => "message.eml".to_owned(),
                    _ => "attachment".to_owned(),
                });
                self.attachments.push(Attachment { name, mime, data });
            }
        }
    }

    pub fn document(&self, prefer_plain: bool) -> Document {
        match (&self.plain, &self.html, prefer_plain) {
            (Some(plain), _, true) | (Some(plain), None, _) => render_plain(plain),
            (_, Some(html), _) => render_html(html),
            (None, None, _) => Document::default(),
        }
    }

    pub fn text(&self) -> String {
        match (&self.plain, &self.html) {
            (Some(plain), _) => plain.clone(),
            (None, Some(html)) => render_html(html).text(),
            (None, None) => String::new(),
        }
    }

    pub fn preview(&self) -> String {
        preview(&self.text())
    }
}

pub fn preview(text: &str) -> String {
    let mut preview = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('>') || line == "--" || line.is_empty() {
            if line == "--" {
                break;
            }
            continue;
        }
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(&line.split_whitespace().collect::<Vec<_>>().join(" "));
        if preview.chars().count() >= PREVIEW_LENGTH {
            break;
        }
    }
    preview.chars().take(PREVIEW_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: &[u8] = b"From: =?UTF-8?Q?J=C3=B6rg_M=C3=BCller?= <jorg@example.de>\r\n\
To: Ann <ann@example.com>, bob@example.com\r\n\
Subject: =?UTF-8?B?w4RwZmVs?= and\r\n pears\r\n\
Date: Tue, 27 Oct 2026 09:30:00 +0100\r\n\
Message-ID: <abc@example.de>\r\n\
In-Reply-To: <parent@example.com>\r\n\
References: <root@example.com> <parent@example.com>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
\r\n\
--outer\r\n\
Content-Type: multipart/alternative; boundary=\"inner\"\r\n\
\r\n\
--inner\r\n\
Content-Type: text/plain; charset=iso-8859-1\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Gr=FC=DFe aus Berlin\r\n\
> quoted\r\n\
--inner\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Gr\xc3\xbc\xc3\x9fe <b>aus</b> Berlin</p>\r\n\
--inner--\r\n\
--outer\r\n\
Content-Type: application/pdf; name=\"plan.pdf\"\r\n\
Content-Disposition: attachment; filename=\"../../plan.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0=\r\n\
--outer--\r\n";

    #[test]
    fn a_multipart_message_yields_both_bodies_and_its_attachment() {
        let content = Content::parse(MIXED);
        assert_eq!(content.envelope.subject, "Äpfel and pears");
        assert_eq!(
            content.envelope.from,
            [Address {
                name: "Jörg Müller".to_owned(),
                email: "jorg@example.de".to_owned()
            }]
        );
        assert_eq!(content.envelope.to.len(), 2);
        assert_eq!(content.envelope.date, Some(1_793_089_800));
        assert_eq!(
            content.envelope.message_id.as_deref(),
            Some("<abc@example.de>")
        );
        assert_eq!(
            content.envelope.references,
            ["<root@example.com>", "<parent@example.com>"]
        );
        assert_eq!(
            content.plain.as_deref(),
            Some("Grüße aus Berlin\r\n> quoted")
        );
        assert!(
            content
                .html
                .as_deref()
                .is_some_and(|html| html.contains("<b>aus</b>"))
        );
        assert_eq!(
            content.attachments,
            [Attachment {
                name: "plan.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                data: b"%PDF-".to_vec(),
            }]
        );
        assert_eq!(content.preview(), "Grüße aus Berlin");
        assert_eq!(content.document(false).blocks[0].bold, vec![8..11]);
    }

    #[test]
    fn previews_skip_quotes_and_stop_at_the_signature() {
        assert_eq!(
            preview("\n\nHello  there,\n> old text\nnew   line\n--\nsig"),
            "Hello there, new line"
        );
        let long = "word ".repeat(200);
        assert_eq!(preview(&long).chars().count(), PREVIEW_LENGTH);
    }

    #[test]
    fn broken_messages_still_show_something() {
        let content = Content::parse(b"no headers at all, just text");
        assert!(content.text().contains("just text"));
        let envelope = Envelope::parse(b"Subject: hi\r\nFrom: nonsense\r\n\r\n");
        assert_eq!(envelope.subject, "hi");
        assert!(envelope.from.is_empty());
    }

    #[test]
    fn addresses_show_names_and_quote_awkward_ones() {
        let plain = Address {
            name: "Ann Lee".to_owned(),
            email: "ann@example.com".to_owned(),
        };
        assert_eq!(plain.full(), "Ann Lee <ann@example.com>");
        let awkward = Address {
            name: "Lee, Ann".to_owned(),
            email: "ann@example.com".to_owned(),
        };
        assert_eq!(awkward.full(), "\"Lee, Ann\" <ann@example.com>");
        assert_eq!(
            parse_addresses("\"Lee, Ann\" <ann@example.com>, bob@example.com"),
            [
                awkward,
                Address {
                    name: String::new(),
                    email: "bob@example.com".to_owned()
                }
            ]
        );
    }
}
