use super::dns;
use super::http::{Credentials, Error, Http, Response, join, origin, same_resource};
use super::xml::{self, APPLE, CALDAV, CALSERVER, DAV, Multistatus, parse_multistatus};

const MAX_REDIRECTS: usize = 5;
const MULTIGET_BATCH: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCalendar {
    pub url: String,
    pub name: String,
    pub color: Option<String>,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub url: String,
    pub etag: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    pub changed: Vec<(String, Option<String>)>,
    pub removed: Vec<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    Create,
    Match(String),
    None,
}

#[derive(Clone)]
pub struct Client {
    http: Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub principal: String,
    pub home: String,
    pub calendars: Vec<RemoteCalendar>,
}

fn has_scheme(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://")
}

pub fn candidates(
    server: &str,
    user: &str,
    lookup: impl Fn(&str) -> Vec<dns::Service>,
) -> Vec<String> {
    let server = server.trim();
    if has_scheme(server) {
        let mut found = vec![server.to_owned()];
        let well_known = format!("{}/.well-known/caldav", origin(server));
        if !found.contains(&well_known) {
            found.push(well_known);
        }
        return found;
    }
    let host = match server.is_empty() {
        true => user.rsplit_once('@').map_or("", |(_, domain)| domain),
        false => server.trim_end_matches('/'),
    };
    if host.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let domain = host.split(['/', ':']).next().unwrap_or(host);
    for record in lookup(&format!("_caldavs._tcp.{domain}")) {
        let port = match record.port {
            443 => String::new(),
            port => format!(":{port}"),
        };
        found.push(format!("https://{}{port}/.well-known/caldav", record.target));
    }
    found.push(format!("https://{host}/.well-known/caldav"));
    found.push(format!("https://{host}/"));
    found
}

impl Client {
    pub fn new(credentials: Option<&Credentials>) -> Self {
        Self {
            http: Http::new(credentials),
        }
    }

    fn follow(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<(String, Response), Error> {
        let mut url = url.to_owned();
        for _ in 0..MAX_REDIRECTS {
            let response = self.http.send(method, &url, headers, body)?;
            match (&response.location, response.status) {
                (Some(location), 301 | 302 | 303 | 307 | 308) => url = join(&url, location),
                _ => return Ok((url, response)),
            }
        }
        Err(Error::Network(format!("too many redirects from {url}")))
    }

    fn multistatus(
        &self,
        method: &str,
        url: &str,
        depth: &str,
        body: &str,
    ) -> Result<(String, Multistatus), Error> {
        let (url, response) = self.follow(method, url, &[("Depth", depth)], Some(body))?;
        if response.status != 207 {
            return Err(Error::Status {
                status: response.status,
                url,
            });
        }
        Ok((url.clone(), parse_multistatus(&response.body)?))
    }

    fn first_href(
        &self,
        url: &str,
        namespace: &str,
        name: &str,
        prop: &str,
    ) -> Result<Option<(String, String)>, Error> {
        let (url, status) = self.multistatus("PROPFIND", url, "0", &xml::propfind(&[prop]))?;
        Ok(status
            .entries
            .iter()
            .find_map(|entry| entry.prop(namespace, name)?.hrefs().into_iter().next())
            .map(|href| (url.clone(), join(&url, &href))))
    }

    pub fn discover(&self, server: &str, user: &str) -> Result<Discovered, Error> {
        self.discover_with(server, user, dns::lookup)
    }

    pub fn discover_with(
        &self,
        server: &str,
        user: &str,
        lookup: impl Fn(&str) -> Vec<dns::Service>,
    ) -> Result<Discovered, Error> {
        let mut last = Error::NotFound(match server.is_empty() {
            true => user.to_owned(),
            false => server.to_owned(),
        });
        for candidate in candidates(server, user, lookup) {
            match self.discover_at(&candidate) {
                Ok(found) => return Ok(found),
                Err(Error::Unauthorized) => return Err(Error::Unauthorized),
                Err(error) => {
                    tracing::debug!("no calendars at {candidate}: {error}");
                    last = error;
                }
            }
        }
        Err(last)
    }

    fn discover_at(&self, url: &str) -> Result<Discovered, Error> {
        let principal = self.first_href(
            url,
            DAV,
            "current-user-principal",
            "d:current-user-principal",
        )?;
        let (url, principal) = match principal {
            Some(found) => found,
            None => (url.to_owned(), url.to_owned()),
        };
        let home = self
            .first_href(
                &principal,
                CALDAV,
                "calendar-home-set",
                "c:calendar-home-set",
            )?
            .map(|(_, home)| home);
        let home = match home {
            Some(home) => home,
            None => url,
        };
        let calendars = self.calendars(&home)?;
        if calendars.is_empty() {
            return Err(Error::NotFound(home));
        }
        Ok(Discovered {
            principal,
            home,
            calendars,
        })
    }

    pub fn calendars(&self, home: &str) -> Result<Vec<RemoteCalendar>, Error> {
        let body = xml::propfind(&[
            "d:resourcetype",
            "d:displayname",
            "a:calendar-color",
            "cs:getctag",
            "d:sync-token",
            "d:current-user-privilege-set",
            "c:supported-calendar-component-set",
        ]);
        let (url, status) = self.multistatus("PROPFIND", home, "1", &body)?;
        Ok(status
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .prop(DAV, "resourcetype")
                    .is_some_and(|kind| kind.child(CALDAV, "calendar").is_some())
            })
            .filter(|entry| {
                entry
                    .prop(CALDAV, "supported-calendar-component-set")
                    .is_none_or(|set| {
                        set.children.is_empty()
                            || set.children.iter().any(|component| {
                                component
                                    .attribute("name")
                                    .is_some_and(|name| name.eq_ignore_ascii_case("VEVENT"))
                            })
                    })
            })
            .map(|entry| {
                let calendar_url = join(&url, &entry.href);
                let read_only = entry
                    .prop(DAV, "current-user-privilege-set")
                    .is_some_and(|set| {
                        set.find(DAV, "write").is_none()
                            && set.find(DAV, "write-content").is_none()
                            && set.find(DAV, "all").is_none()
                    });
                RemoteCalendar {
                    name: entry
                        .text(DAV, "displayname")
                        .map(str::to_owned)
                        .unwrap_or_else(|| last_segment(&calendar_url)),
                    color: entry.text(APPLE, "calendar-color").map(str::to_owned),
                    ctag: entry.text(CALSERVER, "getctag").map(str::to_owned),
                    sync_token: entry.text(DAV, "sync-token").map(str::to_owned),
                    read_only,
                    url: calendar_url,
                }
            })
            .collect())
    }

    pub fn collection_tags(
        &self,
        calendar: &str,
    ) -> Result<(Option<String>, Option<String>), Error> {
        let (_, status) = self.multistatus(
            "PROPFIND",
            calendar,
            "0",
            &xml::propfind(&["cs:getctag", "d:sync-token"]),
        )?;
        let entry = status.entries.first();
        Ok((
            entry
                .and_then(|entry| entry.text(CALSERVER, "getctag"))
                .map(str::to_owned),
            entry
                .and_then(|entry| entry.text(DAV, "sync-token"))
                .map(str::to_owned),
        ))
    }

    pub fn sync_collection(&self, calendar: &str, token: Option<&str>) -> Result<Delta, Error> {
        let (url, status) =
            self.multistatus("REPORT", calendar, "1", &xml::sync_collection(token))?;
        let mut delta = Delta {
            token: status.sync_token.clone(),
            ..Delta::default()
        };
        for entry in status.entries {
            let href = join(&url, &entry.href);
            if same_resource(&href, &url) {
                continue;
            }
            match entry.status {
                Some(404) => delta.removed.push(href),
                _ => delta
                    .changed
                    .push((href, entry.text(DAV, "getetag").map(str::to_owned))),
            }
        }
        Ok(delta)
    }

    pub fn etags(&self, calendar: &str) -> Result<Vec<(String, Option<String>)>, Error> {
        let (url, status) =
            self.multistatus("REPORT", calendar, "1", &xml::calendar_query_events())?;
        Ok(status
            .entries
            .iter()
            .map(|entry| {
                (
                    join(&url, &entry.href),
                    entry.text(DAV, "getetag").map(str::to_owned),
                )
            })
            .filter(|(href, _)| !same_resource(href, &url))
            .collect())
    }

    pub fn multiget(&self, calendar: &str, urls: &[String]) -> Result<Vec<Resource>, Error> {
        let mut found = Vec::new();
        for batch in urls.chunks(MULTIGET_BATCH) {
            let hrefs: Vec<String> = batch.iter().map(|url| path_of(url).to_owned()).collect();
            let (url, status) =
                self.multistatus("REPORT", calendar, "1", &xml::multiget(&hrefs))?;
            for entry in status.entries {
                if let Some(data) = entry.text(CALDAV, "calendar-data") {
                    found.push(Resource {
                        url: join(&url, &entry.href),
                        etag: entry.text(DAV, "getetag").map(str::to_owned),
                        data: data.to_owned(),
                    });
                }
            }
        }
        Ok(found)
    }

    pub fn get(&self, url: &str) -> Result<Resource, Error> {
        let (url, response) = self.follow("GET", url, &[("Accept", "text/calendar")], None)?;
        match response.success() {
            true => Ok(Resource {
                url,
                etag: response.etag,
                data: response.body,
            }),
            false => Err(Error::Status {
                status: response.status,
                url,
            }),
        }
    }

    pub fn put(
        &self,
        url: &str,
        data: &str,
        precondition: &Precondition,
    ) -> Result<Option<String>, Error> {
        let mut headers = vec![("Content-Type", "text/calendar; charset=utf-8")];
        match precondition {
            Precondition::Create => headers.push(("If-None-Match", "*")),
            Precondition::Match(etag) => headers.push(("If-Match", etag.as_str())),
            Precondition::None => {}
        }
        let response = self.http.send("PUT", url, &headers, Some(data))?;
        match response.status {
            412 => Err(Error::PreconditionFailed),
            status if (200..300).contains(&status) => Ok(response.etag),
            status => Err(Error::Status {
                status,
                url: url.to_owned(),
            }),
        }
    }

    pub fn delete(&self, url: &str, etag: Option<&str>) -> Result<(), Error> {
        let headers: Vec<(&str, &str)> = etag.map(|etag| ("If-Match", etag)).into_iter().collect();
        let response = self.http.send("DELETE", url, &headers, None)?;
        match response.status {
            412 => Err(Error::PreconditionFailed),
            404 => Ok(()),
            status if (200..300).contains(&status) => Ok(()),
            status => Err(Error::Status {
                status,
                url: url.to_owned(),
            }),
        }
    }

    fn set_property(
        &self,
        url: &str,
        namespace: &str,
        name: &str,
        element: &str,
        value: &str,
    ) -> Result<(), Error> {
        let (url, status) =
            self.multistatus("PROPPATCH", url, "0", &xml::set_property(element, value))?;
        match status
            .entries
            .iter()
            .any(|entry| entry.prop(namespace, name).is_some())
        {
            true => Ok(()),
            false => Err(Error::Status { status: 403, url }),
        }
    }

    pub fn set_color(&self, url: &str, color: &str) -> Result<(), Error> {
        self.set_property(url, APPLE, "calendar-color", "a:calendar-color", color)
    }

    pub fn rename(&self, url: &str, name: &str) -> Result<(), Error> {
        self.set_property(url, DAV, "displayname", "d:displayname", name)
    }

    pub fn make_calendar(&self, url: &str, name: &str, color: &str) -> Result<(), Error> {
        let response =
            self.http
                .send("MKCALENDAR", url, &[], Some(&xml::mkcalendar(name, color)))?;
        match response.success() {
            true => Ok(()),
            false => Err(Error::Status {
                status: response.status,
                url: url.to_owned(),
            }),
        }
    }
}

fn path_of(url: &str) -> &str {
    &url[origin(url).len()..]
}

fn last_segment(url: &str) -> String {
    super::http::percent_decode(url.trim_end_matches('/').rsplit('/').next().unwrap_or(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_try_srv_then_well_known() {
        let lookup = |name: &str| match name {
            "_caldavs._tcp.example.com" => vec![dns::Service {
                priority: 0,
                weight: 0,
                port: 8443,
                target: "dav.example.com".to_owned(),
            }],
            _ => Vec::new(),
        };
        assert_eq!(
            candidates("", "jane@example.com", lookup),
            [
                "https://dav.example.com:8443/.well-known/caldav",
                "https://example.com/.well-known/caldav",
                "https://example.com/",
            ]
        );
        assert_eq!(
            candidates("http://localhost:5232/jane/", "jane", lookup),
            [
                "http://localhost:5232/jane/",
                "http://localhost:5232/.well-known/caldav"
            ]
        );
    }
}
