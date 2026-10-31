use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::caldav::{Client, Credentials, Discovered, Error};
use crate::secrets::Keyring;
use crate::store::{AccountKind, Calendar};
use crate::sync::{Outcome, sync_calendar};

#[derive(Debug)]
pub enum Job {
    Discover {
        id: String,
        server: String,
        user: String,
        password: String,
    },
    Subscribe {
        url: String,
    },
    Sync(Vec<(AccountKind, String, Calendar)>),
    Forget(String),
    SetColor {
        account: String,
        user: String,
        url: String,
        color: String,
    },
}

#[derive(Debug)]
pub enum Reply {
    Discovered {
        id: String,
        server: String,
        user: String,
        found: Discovered,
    },
    DiscoveryFailed(String),
    Subscribed {
        url: String,
        data: String,
    },
    Synced(Outcome),
    Refreshed {
        calendar: String,
        data: String,
    },
    Failed {
        calendar: String,
        error: String,
    },
    Finished,
}

pub struct Worker {
    jobs: Sender<Job>,
    replies: Receiver<Reply>,
    wake: UnixStream,
}

pub fn http_url(url: &str) -> String {
    let url = url.trim();
    match url.split_once("://") {
        Some((scheme, rest))
            if scheme.eq_ignore_ascii_case("webcal") || scheme.eq_ignore_ascii_case("webcals") =>
        {
            format!("https://{rest}")
        }
        Some(_) => url.to_owned(),
        None => format!("https://{url}"),
    }
}

struct Passwords {
    keyring: Option<Keyring>,
    known: HashMap<String, String>,
}

impl Passwords {
    fn keyring(&mut self) -> Option<&Keyring> {
        if self.keyring.is_none() {
            self.keyring = Keyring::open()
                .map_err(|error| tracing::warn!("cannot open the keyring: {error}"))
                .ok();
        }
        self.keyring.as_ref()
    }

    fn get(&mut self, account: &str) -> Option<String> {
        if let Some(password) = self.known.get(account) {
            return Some(password.clone());
        }
        let password = self
            .keyring()?
            .password(account)
            .map_err(|error| tracing::warn!("cannot read the password: {error}"))
            .ok()
            .flatten()?;
        self.known.insert(account.to_owned(), password.clone());
        Some(password)
    }
}

fn run(job: Job, passwords: &mut Passwords, reply: &mut dyn FnMut(Reply)) {
    match job {
        Job::Discover {
            id,
            server,
            user,
            password,
        } => {
            let client = Client::new(Some(&Credentials {
                user: user.clone(),
                password: password.clone(),
            }));
            match client.discover(&server, &user) {
                Ok(found) => {
                    let label = format!("Calendar: {user}");
                    if let Some(keyring) = passwords.keyring()
                        && let Err(error) = keyring.store(&id, &label, &password)
                    {
                        tracing::warn!("cannot keep the password: {error}");
                    }
                    passwords.known.insert(id.clone(), password);
                    reply(Reply::Discovered {
                        id,
                        server,
                        user,
                        found,
                    });
                }
                Err(error) => reply(Reply::DiscoveryFailed(error.to_string())),
            }
        }
        Job::Subscribe { url } => match Client::new(None).get(&http_url(&url)) {
            Ok(resource) => reply(Reply::Subscribed {
                url,
                data: resource.data,
            }),
            Err(error) => reply(Reply::DiscoveryFailed(error.to_string())),
        },
        Job::Sync(calendars) => {
            for (kind, account, calendar) in calendars {
                let result =
                    match &kind {
                        AccountKind::CalDav { user, .. } => {
                            let credentials = passwords.get(&account).map(|password| Credentials {
                                user: user.clone(),
                                password,
                            });
                            sync_calendar(&Client::new(credentials.as_ref()), &calendar)
                                .map(Reply::Synced)
                        }
                        AccountKind::Subscription { url } => Client::new(None)
                            .get(&http_url(url))
                            .map(|resource| Reply::Refreshed {
                                calendar: calendar.info.id.clone(),
                                data: resource.data,
                            }),
                        AccountKind::Local => continue,
                    };
                reply(result.unwrap_or_else(|error: Error| Reply::Failed {
                    calendar: calendar.info.id.clone(),
                    error: error.to_string(),
                }));
            }
            reply(Reply::Finished);
        }
        Job::SetColor {
            account,
            user,
            url,
            color,
        } => {
            let credentials = passwords
                .get(&account)
                .map(|password| Credentials { user, password });
            if let Err(error) = Client::new(credentials.as_ref()).set_color(&url, &color) {
                tracing::warn!("cannot change the colour of {url}: {error}");
            }
        }
        Job::Forget(account) => {
            passwords.known.remove(&account);
            if let Some(keyring) = passwords.keyring()
                && let Err(error) = keyring.forget(&account)
            {
                tracing::warn!("cannot forget the password: {error}");
            }
        }
    }
}

impl Worker {
    pub fn start() -> std::io::Result<Self> {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, replies) = channel::<Reply>();
        let (wake, mut waker) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("sync".to_owned())
            .spawn(move || {
                let mut passwords = Passwords {
                    keyring: None,
                    known: HashMap::new(),
                };
                for job in inbox {
                    run(job, &mut passwords, &mut |reply| {
                        if outbox.send(reply).is_ok() {
                            let _ = waker.write_all(&[1]);
                        }
                    });
                }
            })?;
        Ok(Self {
            jobs,
            replies,
            wake,
        })
    }

    pub fn send(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::warn!("the sync thread stopped");
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn take(&mut self) -> Vec<Reply> {
        let mut buffer = [0u8; 64];
        while matches!((&self.wake).read(&mut buffer), Ok(read) if read > 0) {}
        self.replies.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webcal_links_become_http() {
        assert_eq!(
            http_url("webcal://example.com/a.ics"),
            "https://example.com/a.ics"
        );
        assert_eq!(
            http_url("webcals://example.com/a.ics"),
            "https://example.com/a.ics"
        );
        assert_eq!(http_url("example.com/a.ics"), "https://example.com/a.ics");
        assert_eq!(
            http_url("https://example.com/a.ics"),
            "https://example.com/a.ics"
        );
    }
}
