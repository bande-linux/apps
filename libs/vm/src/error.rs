use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Json(serde_json::Error),
    Bus(zbus::Error),
    Http(ureq::Error),
    Qmp {
        class: String,
        description: String,
    },
    Protocol(String),
    Checksum {
        expected: String,
        actual: String,
    },
    NotListed(String),
    Tool {
        program: String,
        status: ExitStatus,
        stderr: String,
    },
    Unit {
        unit: String,
        result: String,
    },
    Missing(String),
    NotUtf8(PathBuf),
    Cancelled,
    NeedsShutDown,
    Signature(String),
    HasSnapshots,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => write!(f, "invalid JSON: {error}"),
            Self::Bus(error) => write!(f, "D-Bus: {error}"),
            Self::Http(error) => write!(f, "download: {error}"),
            Self::Qmp { class, description } => write!(f, "QEMU: {description} ({class})"),
            Self::Protocol(message) => write!(f, "QMP protocol: {message}"),
            Self::Checksum { expected, actual } => {
                write!(f, "checksum mismatch: expected {expected}, got {actual}")
            }
            Self::NotListed(file) => write!(f, "{file} is not in the vendor checksums"),
            Self::Tool {
                program,
                status,
                stderr,
            } => write!(f, "{program} failed ({status}): {}", stderr.trim()),
            Self::Unit { unit, result } => write!(f, "{unit} did not start: {result}"),
            Self::Missing(what) => write!(f, "{what} is not available"),
            Self::NotUtf8(path) => write!(f, "{} is not a UTF-8 path", path.display()),
            Self::Signature(message) => {
                write!(
                    f,
                    "the vendor's checksums are not signed correctly: {message}"
                )
            }
            Self::NeedsShutDown => write!(f, "shut the machine down first"),
            Self::HasSnapshots => {
                write!(f, "a machine with snapshots can only be cloned in full")
            }
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        Self::Bus(error)
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(error: zbus::fdo::Error) -> Self {
        Self::Bus(error.into())
    }
}

impl From<ureq::Error> for Error {
    fn from(error: ureq::Error) -> Self {
        Self::Http(error)
    }
}

impl From<rustix::io::Errno> for Error {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}
