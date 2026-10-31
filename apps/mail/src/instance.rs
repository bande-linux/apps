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
        let connection = Connection::session()?;
        let reply = connection.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into());
        match reply {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
            Ok(_) | Err(zbus::Error::NameTaken) => {
                connection.call_method(Some(NAME), PATH, Some(NAME), "Open", &(arguments,))?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        let (awake, wake) = UnixStream::pair()?;
        awake.set_nonblocking(true)?;
        let (sender, received) = mpsc::channel();
        connection.object_server().at(
            PATH,
            Opener {
                arguments: Mutex::new(sender),
                wake,
            },
        )?;
        Ok(Some(Self {
            _connection: connection,
            arguments: received,
            awake,
        }))
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
