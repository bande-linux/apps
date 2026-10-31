use std::time::Duration;

use base64::Engine;
use ureq::tls::{RootCerts, TlsConfig};

const TIMEOUT: Duration = Duration::from_secs(30);
const BODY_LIMIT: u64 = 64 * 1024 * 1024;
const USER_AGENT: &str = concat!("Bande-Calendar/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Network(String),
    Unauthorized,
    Status { status: u16, url: String },
    PreconditionFailed,
    Xml(String),
    NotFound(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(error) => write!(f, "cannot reach the server: {error}"),
            Self::Unauthorized => write!(f, "the user name or password is wrong"),
            Self::Status { status, url } => write!(f, "{url} answered {status}"),
            Self::PreconditionFailed => write!(f, "the event changed on the server"),
            Self::Xml(error) => write!(f, "the server's answer is not valid: {error}"),
            Self::NotFound(what) => write!(f, "no calendar server found for {what}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub body: String,
    pub etag: Option<String>,
    pub location: Option<String>,
}

impl Response {
    pub fn success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    authorization: Option<String>,
}

impl Http {
    pub fn new(credentials: Option<&Credentials>) -> Self {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .allow_non_standard_methods(true)
            .max_redirects(0)
            .timeout_global(Some(TIMEOUT))
            .user_agent(USER_AGENT)
            .tls_config(tls)
            .build()
            .new_agent();
        let authorization = credentials.map(|credentials| {
            let pair = format!("{}:{}", credentials.user, credentials.password);
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(pair)
            )
        });
        Self {
            agent,
            authorization,
        }
    }

    pub fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response, Error> {
        let network = |error: &dyn std::fmt::Display| Error::Network(error.to_string());
        let mut builder = ureq::http::Request::builder().method(method).uri(url);
        if let Some(authorization) = &self.authorization {
            builder = builder.header("Authorization", authorization);
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        if body.is_some()
            && !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
        {
            builder = builder.header("Content-Type", "application/xml; charset=utf-8");
        }
        let request = builder
            .body(body.unwrap_or_default().to_owned())
            .map_err(|error| network(&error))?;
        let mut response = match self.agent.run(request.clone()) {
            Err(ureq::Error::Io(_)) => self.agent.run(request),
            other => other,
        }
        .map_err(|error| network(&error))?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let status = response.status().as_u16();
        let etag = header("ETag");
        let location = header("Location");
        let body = response
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()
            .unwrap_or_default();
        if status == 401 {
            return Err(Error::Unauthorized);
        }
        Ok(Response {
            status,
            body,
            etag,
            location,
        })
    }
}

pub fn origin(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(0, |index| index + 3);
    match url[after_scheme..].find('/') {
        Some(index) => &url[..after_scheme + index],
        None => url,
    }
}

pub fn join(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_owned();
    }
    if href.starts_with('/') {
        return format!("{}{href}", origin(base));
    }
    let directory = match base.rfind('/') {
        Some(index) if index >= origin(base).len() => &base[..=index],
        _ => return format!("{}/{href}", origin(base)),
    };
    format!("{directory}{href}")
}

pub fn same_resource(a: &str, b: &str) -> bool {
    let normal = |url: &str| {
        let decoded = percent_decode(url);
        decoded.trim_end_matches('/').to_owned()
    };
    normal(a) == normal(b)
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let decoded = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_hrefs_like_a_browser() {
        let base = "https://dav.example.com:8443/cal/jane/";
        assert_eq!(
            join(base, "/cal/jane/work/"),
            "https://dav.example.com:8443/cal/jane/work/"
        );
        assert_eq!(
            join(base, "work/a.ics"),
            "https://dav.example.com:8443/cal/jane/work/a.ics"
        );
        assert_eq!(join("https://example.com", "x/"), "https://example.com/x/");
        assert_eq!(
            join(base, "https://other.example/"),
            "https://other.example/"
        );
        assert!(same_resource("https://e.com/a%40b/", "https://e.com/a@b"));
    }
}
