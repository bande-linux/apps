use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

const NAME: &str = "org.bande.Mail";
const PATH: &str = "/org/bande/Mail";

struct Opener {
    arguments: Mutex<Sender<String>>,
    wake: UnixStream,
}

#[zbus::interface(name = "org.bande.Mail")]
impl Opener {
    fn open(&self, arguments: Vec<String>) {
        if let Ok(sender) = self.arguments.lock() {
            for argument in arguments {
                let _ = sender.send(argument);
            }
        }
        let _ = (&self.wake).write(&[1]);
    }
}

pub struct Instance {
    _connection: Connection,
    arguments: Receiver<String>,
    awake: UnixStream,
}

impl Instance {
    pub fn claim(arguments: &[String]) -> zbus::Result<Option<Self>> {
        let (awake, wake) = UnixStream::pair()?;
        awake.set_nonblocking(true)?;
        let (sender, received) = mpsc::channel();
        let opener = Opener {
            arguments: Mutex::new(sender),
            wake,
        };
        let connection = Connection::session()?;
        connection.object_server().at(PATH, opener)?;
        let reply = connection.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into());
        match reply {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(Some(Self {
                _connection: connection,
                arguments: received,
                awake,
            })),
            Ok(_) | Err(zbus::Error::NameTaken) => {
                if let Err(error) =
                    connection.call_method(Some(NAME), PATH, Some(NAME), "Open", &(arguments,))
                {
                    tracing::warn!("cannot reach the running Mail: {error}");
                }
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.awake.as_fd()
    }

    pub fn take(&mut self) -> Vec<String> {
        let mut buffer = [0u8; 64];
        while matches!((&self.awake).read(&mut buffer), Ok(count) if count > 0) {}
        self.arguments.try_iter().collect()
    }
}
