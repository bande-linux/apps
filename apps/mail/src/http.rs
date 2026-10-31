use crate::error::{Error, Result};
use crate::net::{Link, Network, Security, Server};

const MAX_HEADER_LINE: usize = 8 * 1024;
const MAX_REDIRECTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Url {
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> Option<Url> {
    let rest = url.strip_prefix("https://")?;
    let (authority, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, port.parse().ok()?),
        _ => (authority, 443),
    };
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character));
    valid.then(|| Url {
        host: host.to_ascii_lowercase(),
        port,
        path: path.to_owned(),
    })
}

fn request(url: &Url, network: &Network, limit: usize) -> Result<(u16, Option<String>, Vec<u8>)> {
    let server = Server {
        host: url.host.clone(),
        port: url.port,
        security: Security::Tls,
    };
    let mut link = Link::connect(&server, network)?;
    link.send(
        format!(
            "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: Bande Mail\r\nAccept: application/xml, text/xml\r\nConnection: close\r\n\r\n",
            url.path, url.host
        )
        .as_bytes(),
    )?;
    let status_line = String::from_utf8_lossy(&link.read_line(MAX_HEADER_LINE)?).into_owned();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| Error::Protocol(format!("HTTP status {status_line:?}")))?;
    let mut location = None;
    let mut length = None;
    loop {
        let line = link.read_line(MAX_HEADER_LINE)?;
        let line = String::from_utf8_lossy(&line).trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("location") {
                location = Some(value.to_owned());
            } else if name.eq_ignore_ascii_case("content-length") {
                length = value.parse::<usize>().ok();
            }
        }
    }
    let too_large = || Error::Protocol("an HTTP answer that is too large".to_owned());
    let body = match length {
        Some(length) if length > limit => return Err(too_large()),
        Some(length) => link.read_exact(length)?,
        None => link.read_to_end(limit).map_err(|error| match error {
            Error::Protocol(_) => too_large(),
            other => other,
        })?,
    };
    Ok((status, location, body))
}

pub fn get(url: &str, network: &Network, limit: usize) -> Result<Vec<u8>> {
    let mut current = parse_url(url).ok_or_else(|| Error::Protocol(format!("bad URL {url}")))?;
    for _ in 0..=MAX_REDIRECTS {
        let (status, location, body) = request(&current, network, limit)?;
        match (status, location) {
            (200, _) => return Ok(body),
            (301 | 302 | 303 | 307 | 308, Some(location)) => {
                let next = match location.starts_with('/') {
                    true => Url {
                        path: location,
                        ..current.clone()
                    },
                    false => parse_url(&location)
                        .ok_or_else(|| Error::Protocol(format!("bad redirect {location}")))?,
                };
                current = next;
            }
            (status, _) => return Err(Error::Refused(format!("HTTP {status}"))),
        }
    }
    Err(Error::Protocol("too many redirects".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_split_and_odd_hosts_refused() {
        assert_eq!(
            parse_url("https://autoconfig.thunderbird.net/v1.1/example.com"),
            Some(Url {
                host: "autoconfig.thunderbird.net".to_owned(),
                port: 443,
                path: "/v1.1/example.com".to_owned(),
            })
        );
        assert_eq!(
            parse_url("https://Example.com:8080").map(|url| (url.host, url.port, url.path)),
            Some(("example.com".to_owned(), 8080, "/".to_owned()))
        );
        assert_eq!(parse_url("http://example.com/"), None);
        assert_eq!(parse_url("https://exa mple.com/"), None);
        assert_eq!(parse_url("https://a\r\nHost: b/"), None);
    }
}
