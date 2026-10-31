use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Timeout,
    Tls(String),
    Insecure,
    Protocol(String),
    Refused(String),
    Login(String),
    Store(rusqlite::Error),
    Secret(String),
    Closed,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn is_connection(&self) -> bool {
        matches!(
            self,
            Self::Io(_) | Self::Timeout | Self::Tls(_) | Self::Protocol(_) | Self::Closed
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Timeout => write!(f, "the server did not answer in time"),
            Self::Tls(error) => write!(f, "secure connection failed: {error}"),
            Self::Insecure => write!(f, "the server offers no secure connection"),
            Self::Protocol(error) => write!(f, "the server sent something unexpected: {error}"),
            Self::Refused(text) => write!(f, "the server refused: {text}"),
            Self::Login(text) => write!(f, "the server refused the login: {text}"),
            Self::Store(error) => write!(f, "the mail store failed: {error}"),
            Self::Secret(error) => write!(f, "the keyring failed: {error}"),
            Self::Closed => write!(f, "the server closed the connection"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Self::Timeout,
            io::ErrorKind::UnexpectedEof => Self::Closed,
            _ => Self::Io(error),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(error)
    }
}

impl From<crate::imap::wire::WireError> for Error {
    fn from(error: crate::imap::wire::WireError) -> Self {
        Self::Protocol(error.0)
    }
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        Self::Secret(error.to_string())
    }
}
