use std::net::TcpStream;
use std::time::Duration;

use crate::dns;
use crate::http;
use crate::net::{Network, Security, Server};

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const ISPDB: &str = "https://autoconfig.thunderbird.net/v1.1/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub imap: Server,
    pub smtp: Server,
    pub user: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Provider,
    Directory,
    Services,
    Guess,
}

fn domain_of(email: &str) -> Option<&str> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    let valid = !local.is_empty()
        && domain.contains('.')
        && domain
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character));
    valid.then_some(domain)
}

fn expand(template: &str, email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    template
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

fn security_of(socket: &str) -> Option<Security> {
    match socket.to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Some(Security::Tls),
        "STARTTLS" => Some(Security::StartTls),
        _ => None,
    }
}

fn rank(security: Security) -> u8 {
    match security {
        Security::Tls => 0,
        Security::StartTls => 1,
        Security::Plain => 2,
    }
}

pub fn parse_config(xml: &str, email: &str) -> Option<Settings> {
    let document = roxmltree::Document::parse(xml).ok()?;
    let provider = document
        .descendants()
        .find(|node| node.has_tag_name("emailProvider"))?;
    let child_text = |node: roxmltree::Node<'_, '_>, name: &str| {
        node.children()
            .find(|child| child.has_tag_name(name))
            .and_then(|child| child.text())
            .map(|text| text.trim().to_owned())
    };
    let servers = |tag: &str, kind: &str| {
        let mut found = provider
            .children()
            .filter(|node| node.has_tag_name(tag) && node.attribute("type") == Some(kind))
            .filter_map(|node| {
                let security = security_of(&child_text(node, "socketType")?)?;
                let server = Server {
                    host: expand(&child_text(node, "hostname")?, email).to_ascii_lowercase(),
                    port: child_text(node, "port")?.parse().ok()?,
                    security,
                };
                let user = expand(
                    &child_text(node, "username").unwrap_or_else(|| "%EMAILADDRESS%".to_owned()),
                    email,
                );
                Some((server, user))
            })
            .collect::<Vec<_>>();
        found.sort_by_key(|(server, _)| rank(server.security));
        found.into_iter().next()
    };
    let (imap, user) = servers("incomingServer", "imap")?;
    let (smtp, _) = servers("outgoingServer", "smtp")?;
    Some(Settings { imap, smtp, user })
}

fn from_config(url: &str, email: &str, network: &Network) -> Option<Settings> {
    let body = http::get(url, network)
        .inspect_err(|error| tracing::debug!("{url}: {error}"))
        .ok()?;
    parse_config(&String::from_utf8_lossy(&body), email)
}

fn query_value(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'@' | b'.' | b'-' | b'_' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn base_domain(host: &str) -> String {
    let parts = host.trim_end_matches('.').split('.').collect::<Vec<_>>();
    let keep = match parts.len() >= 3 && parts[parts.len() - 2].len() <= 3 {
        true => 3,
        false => 2,
    };
    parts[parts.len().saturating_sub(keep)..].join(".")
}

fn from_services(domain: &str, email: &str) -> Option<Settings> {
    let first = |names: [(&str, Security); 2]| {
        names.into_iter().find_map(|(service, security)| {
            dns::services(&format!("{service}.{domain}"))
                .into_iter()
                .next()
                .map(|target| Server {
                    host: target.host.to_ascii_lowercase(),
                    port: target.port,
                    security,
                })
        })
    };
    let imap = first([
        ("_imaps._tcp", Security::Tls),
        ("_imap._tcp", Security::StartTls),
    ])?;
    let smtp = first([
        ("_submissions._tcp", Security::Tls),
        ("_submission._tcp", Security::StartTls),
    ])?;
    Some(Settings {
        imap,
        smtp,
        user: email.to_owned(),
    })
}

fn answers(host: &str, port: u16) -> bool {
    use std::net::ToSocketAddrs;
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .is_some_and(|address| TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok())
}

fn guess(domain: &str, email: &str) -> Option<Settings> {
    let probe = |candidates: &[(&str, u16, Security)]| {
        candidates.iter().find_map(|(prefix, port, security)| {
            let host = format!("{prefix}.{domain}");
            answers(&host, *port).then_some(Server {
                host,
                port: *port,
                security: *security,
            })
        })
    };
    let imap = probe(&[
        ("imap", 993, Security::Tls),
        ("mail", 993, Security::Tls),
        ("imap", 143, Security::StartTls),
        ("mail", 143, Security::StartTls),
    ])?;
    let smtp = probe(&[
        ("smtp", 465, Security::Tls),
        ("mail", 465, Security::Tls),
        ("smtp", 587, Security::StartTls),
        ("mail", 587, Security::StartTls),
    ])?;
    Some(Settings {
        imap,
        smtp,
        user: email.to_owned(),
    })
}

pub fn discover(email: &str, network: &Network) -> Option<(Settings, Source)> {
    let email = email.trim();
    let domain = domain_of(email)?.to_ascii_lowercase();
    let provider = [
        format!(
            "https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={}",
            query_value(email)
        ),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ];
    if let Some(found) = provider
        .iter()
        .find_map(|url| from_config(url, email, network))
    {
        return Some((found, Source::Provider));
    }
    if let Some(found) = from_config(&format!("{ISPDB}{domain}"), email, network) {
        return Some((found, Source::Directory));
    }
    if let Some(found) = from_services(&domain, email) {
        return Some((found, Source::Services));
    }
    let exchanger = dns::exchangers(&domain)
        .into_iter()
        .next()
        .map(|target| base_domain(&target.host))
        .filter(|exchanger| *exchanger != domain);
    if let Some(found) =
        exchanger.and_then(|exchanger| from_config(&format!("{ISPDB}{exchanger}"), email, network))
    {
        return Some((found, Source::Directory));
    }
    guess(&domain, email).map(|found| (found, Source::Guess))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.com">
    <domain>example.com</domain>
    <incomingServer type="pop3">
      <hostname>pop.example.com</hostname><port>995</port><socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>imap.example.com</hostname><port>143</port><socketType>STARTTLS</socketType>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>IMAP.%EMAILDOMAIN%</hostname><port>993</port><socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>old.example.com</hostname><port>143</port><socketType>plain</socketType>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.com</hostname><port>587</port><socketType>STARTTLS</socketType>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn a_provider_config_picks_the_most_secure_imap_and_smtp() {
        assert_eq!(
            parse_config(CONFIG, "ann@example.com"),
            Some(Settings {
                imap: Server {
                    host: "imap.example.com".to_owned(),
                    port: 993,
                    security: Security::Tls,
                },
                smtp: Server {
                    host: "smtp.example.com".to_owned(),
                    port: 587,
                    security: Security::StartTls,
                },
                user: "ann".to_owned(),
            })
        );
    }

    #[test]
    fn configs_without_secure_imap_or_any_smtp_are_not_used() {
        let plain_only = CONFIG
            .replace(
                "<socketType>STARTTLS</socketType>\n      <username>%EMAILLOCALPART%",
                "<socketType>plain</socketType>\n      <username>%EMAILLOCALPART%",
            )
            .replace(
                "<socketType>SSL</socketType>\n      <username>%EMAILLOCALPART%",
                "<socketType>plain</socketType>\n      <username>%EMAILLOCALPART%",
            );
        assert_eq!(parse_config(&plain_only, "ann@example.com"), None);
        let no_smtp = CONFIG.replace("outgoingServer", "otherServer");
        assert_eq!(parse_config(&no_smtp, "ann@example.com"), None);
        assert_eq!(parse_config("<html>not xml", "ann@example.com"), None);
    }

    #[test]
    fn addresses_and_exchangers_are_reduced_to_domains() {
        assert_eq!(domain_of(" ann@Example.com "), Some("Example.com"));
        assert_eq!(domain_of("ann@localhost"), None);
        assert_eq!(domain_of("@example.com"), None);
        assert_eq!(domain_of("ann@exa/mple.com"), None);
        assert_eq!(base_domain("aspmx.l.google.com."), "google.com");
        assert_eq!(
            base_domain("example-com.mail.protection.outlook.com"),
            "outlook.com"
        );
        assert_eq!(base_domain("mx1.mail.co.uk"), "mail.co.uk");
    }
}
