use crate::discover::{Settings, discover};
use crate::error::Error;
use crate::imap::Session;
use crate::net::Network;
use crate::smtp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub name: String,
    pub email: String,
    pub password: String,
    pub servers: Option<Settings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ready(Settings),
    WrongPassword(Settings, String),
    NeedsServers(Option<Settings>, String),
}

fn check_servers(settings: &Settings, password: &str, network: &Network) -> Result<(), Error> {
    let mut session = Session::connect(&settings.imap, network)?;
    session.login(&settings.user, password)?;
    session.list()?;
    session.logout();
    smtp::verify(&settings.smtp, network, &settings.user, password)
}

pub fn check(request: &Request, network: &Network) -> Outcome {
    let settings = match &request.servers {
        Some(settings) => settings.clone(),
        None => match discover(&request.email, network) {
            Some((settings, _)) => settings,
            None => {
                return Outcome::NeedsServers(
                    None,
                    "no mail servers were found for this address".to_owned(),
                );
            }
        },
    };
    match check_servers(&settings, &request.password, network) {
        Ok(()) => Outcome::Ready(settings),
        Err(Error::Login(text)) => Outcome::WrongPassword(settings, text),
        Err(error) => Outcome::NeedsServers(Some(settings), error.to_string()),
    }
}
