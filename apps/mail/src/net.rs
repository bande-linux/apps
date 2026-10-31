use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;

use native_tls::{Certificate, TlsConnector, TlsStream};

use crate::error::{Error, Result};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const READ_CHUNK: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Security {
    Tls,
    StartTls,
    Plain,
}

impl Security {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
            Self::Plain => "plain",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Tls, Self::StartTls, Self::Plain]
            .into_iter()
            .find(|security| security.name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

#[derive(Clone)]
pub struct Network {
    pub extra_roots: Vec<Vec<u8>>,
    pub timeout: Duration,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            extra_roots: Vec::new(),
            timeout: COMMAND_TIMEOUT,
        }
    }
}

impl Network {
    fn connector(&self) -> Result<TlsConnector> {
        let mut builder = TlsConnector::builder();
        for pem in &self.extra_roots {
            let certificate =
                Certificate::from_pem(pem).map_err(|error| Error::Tls(error.to_string()))?;
            builder.add_root_certificate(certificate);
        }
        builder
            .build()
            .map_err(|error| Error::Tls(error.to_string()))
    }
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => stream.get_ref(),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(bytes),
            Self::Tls(stream) => stream.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

pub struct Link {
    stream: Stream,
    buffer: Vec<u8>,
    network: Network,
    host: String,
}

pub fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn open_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let addresses = (host, port).to_socket_addrs()?.collect::<Vec<_>>();
    let mut last = Error::Io(io::Error::new(
        io::ErrorKind::NotFound,
        format!("{host} has no address"),
    ));
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout.min(CONNECT_TIMEOUT)) {
            Ok(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
            Err(error) => last = error.into(),
        }
    }
    Err(last)
}

impl Link {
    pub fn connect(server: &Server, network: &Network) -> Result<Self> {
        if server.security == Security::Plain && !is_loopback(&server.host) {
            return Err(Error::Insecure);
        }
        let tcp = open_tcp(&server.host, server.port, network.timeout)?;
        let link = Self {
            stream: Stream::Plain(tcp),
            buffer: Vec::new(),
            network: network.clone(),
            host: server.host.clone(),
        };
        match server.security {
            Security::Tls => link.start_tls(),
            Security::StartTls | Security::Plain => Ok(link),
        }
    }

    pub fn start_tls(self) -> Result<Self> {
        let Stream::Plain(tcp) = self.stream else {
            return Ok(self);
        };
        if !self.buffer.is_empty() {
            return Err(Error::Protocol(
                "data arrived before the secure connection started".to_owned(),
            ));
        }
        let connector = self.network.connector()?;
        let host = self.host.trim_start_matches('[').trim_end_matches(']');
        let tls = connector
            .connect(host, tcp)
            .map_err(|error| Error::Tls(error.to_string()))?;
        Ok(Self {
            stream: Stream::Tls(Box::new(tls)),
            buffer: Vec::new(),
            network: self.network,
            host: self.host,
        })
    }

    pub fn is_secure(&self) -> bool {
        matches!(self.stream, Stream::Tls(_))
    }

    pub fn set_read_timeout(&self, timeout: Duration) -> Result<()> {
        Ok(self.stream.tcp().set_read_timeout(Some(timeout))?)
    }

    pub fn restore_read_timeout(&self) -> Result<()> {
        self.set_read_timeout(self.network.timeout)
    }

    pub fn closer(&self) -> Result<Closer> {
        Ok(Closer(self.stream.tcp().try_clone()?))
    }

    fn fill(&mut self) -> Result<()> {
        let start = self.buffer.len();
        self.buffer.resize(start + READ_CHUNK, 0);
        let read = self.stream.read(&mut self.buffer[start..]);
        match read {
            Ok(0) => {
                self.buffer.truncate(start);
                Err(Error::Closed)
            }
            Ok(count) => {
                self.buffer.truncate(start + count);
                Ok(())
            }
            Err(error) => {
                self.buffer.truncate(start);
                Err(error.into())
            }
        }
    }

    pub fn read_line(&mut self, limit: usize) -> Result<Vec<u8>> {
        let mut searched = 0;
        loop {
            if let Some(end) = self.buffer[searched..]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let line = self.buffer.drain(..searched + end + 1).collect();
                return Ok(line);
            }
            searched = self.buffer.len();
            if searched > limit {
                return Err(Error::Protocol(format!("a line longer than {limit} bytes")));
            }
            self.fill()?;
        }
    }

    pub fn read_to_end(&mut self, limit: usize) -> Result<Vec<u8>> {
        loop {
            if self.buffer.len() > limit {
                return Err(Error::Protocol(format!("more than {limit} bytes")));
            }
            match self.fill() {
                Ok(()) => {}
                Err(Error::Closed) => return Ok(std::mem::take(&mut self.buffer)),
                Err(error) => return Err(error),
            }
        }
    }

    pub fn read_exact(&mut self, length: usize) -> Result<Vec<u8>> {
        while self.buffer.len() < length {
            self.fill()?;
        }
        Ok(self.buffer.drain(..length).collect())
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes)?;
        Ok(self.stream.flush()?)
    }
}

pub struct Closer(TcpStream);

impl Closer {
    pub fn close(&self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(bytes: &'static [u8]) -> Server {
        let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else {
            unreachable!("a loopback port")
        };
        let port = listener.local_addr().map_or(0, |address| address.port());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(bytes);
            }
        });
        Server {
            host: "127.0.0.1".to_owned(),
            port,
            security: Security::Plain,
        }
    }

    #[test]
    fn binary_data_without_a_final_line_break_is_read_whole() -> Result<()> {
        let mut link = Link::connect(&served(b"line\n\x89PNG\x00tail"), &Network::default())?;
        assert_eq!(link.read_line(64)?, b"line\n");
        assert_eq!(link.read_to_end(64)?, b"\x89PNG\x00tail");
        let mut limited = Link::connect(&served(&[7; 100]), &Network::default())?;
        assert!(matches!(limited.read_to_end(10), Err(Error::Protocol(_))));
        Ok(())
    }

    #[test]
    fn only_loopback_hosts_may_go_without_encryption() {
        assert!(is_loopback("localhost"));
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("[::1]"));
        assert!(!is_loopback("imap.example.com"));
        assert!(!is_loopback("192.168.1.2"));
        let server = Server {
            host: "imap.example.com".to_owned(),
            port: 143,
            security: Security::Plain,
        };
        assert!(matches!(
            Link::connect(&server, &Network::default()),
            Err(Error::Insecure)
        ));
    }
}
