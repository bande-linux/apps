use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bande_calendar::caldav::{Client, Credentials};

pub const USER: &str = "jane";
pub const PASSWORD: &str = "correct horse";

pub struct Radicale {
    child: Child,
    pub url: String,
    _storage: tempfile::TempDir,
}

impl Drop for Radicale {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .expect("a free port")
}

impl Radicale {
    pub fn start() -> Self {
        let storage = tempfile::tempdir().expect("a storage folder");
        let users = storage.path().join("users");
        std::fs::write(&users, format!("{USER}:{PASSWORD}\nbob:builder\n"))
            .expect("an htpasswd file");
        for _ in 0..10 {
            let port = free_port();
            let mut child = Command::new("python3")
                .args(["-m", "radicale", "--config", ""])
                .arg("--server-hosts")
                .arg(format!("127.0.0.1:{port}"))
                .arg("--storage-filesystem-folder")
                .arg(storage.path().join("collections"))
                .args([
                    "--auth-type",
                    "htpasswd",
                    "--auth-htpasswd-encryption",
                    "plain",
                ])
                .arg("--auth-htpasswd-filename")
                .arg(&users)
                .args(["--rights-type", "owner_only", "--logging-level", "error"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("radicale (apt install python3-radicale) to start");
            let deadline = Instant::now() + Duration::from_secs(20);
            while TcpStream::connect(("127.0.0.1", port)).is_err()
                && child.try_wait().ok().flatten().is_none()
            {
                assert!(Instant::now() < deadline, "radicale did not start");
                std::thread::sleep(Duration::from_millis(50));
            }
            std::thread::sleep(Duration::from_millis(300));
            if child.try_wait().ok().flatten().is_none() {
                return Self {
                    child,
                    url: format!("http://127.0.0.1:{port}/"),
                    _storage: storage,
                };
            }
        }
        panic!("radicale (apt install python3-radicale) did not start on a free port");
    }

    pub fn client(&self) -> Client {
        Client::new(Some(&Credentials {
            user: USER.to_owned(),
            password: PASSWORD.to_owned(),
        }))
    }

    pub fn calendar(&self, slug: &str, name: &str) -> String {
        let client = self.client();
        let home = format!("{}{USER}/", self.url);
        let _ = client.calendars(&home);
        let url = format!("{}{USER}/{slug}/", self.url);
        let mut made = client.make_calendar(&url, name, "#FF9500FF");
        for _ in 0..5 {
            if !matches!(made, Err(bande_calendar::caldav::Error::Network(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
            made = client.make_calendar(&url, name, "#FF9500FF");
        }
        made.expect("a new calendar");
        url
    }
}
