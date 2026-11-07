use std::fs::{self, File};
use std::io::{Read, Write};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use rustls::crypto::aws_lc_rs;
use sha2::{Digest, Sha256, Sha512};
use ureq::tls::{RootCerts, TlsConfig};

use crate::manifest::persist;
use crate::{Error, Result};

const CHUNK: usize = 1024 * 1024;
const CHECKSUMS_LIMIT: u64 = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Sha256,
    Sha512,
}

#[derive(Clone, Copy, Debug)]
enum Signed {
    Https,
    Detached {
        url: &'static str,
        keys: &'static [u8],
    },
    Inline {
        keys: &'static [u8],
    },
}

#[derive(Clone, Copy, Debug)]
pub struct CloudImage {
    pub os: &'static str,
    pub url: &'static str,
    pub checksums: &'static str,
    pub algorithm: Algorithm,
    signed: Signed,
}

const GPGV: &str = "gpgv";

const CATALOG: &[CloudImage] = &[
    CloudImage {
        os: "Debian 13",
        url: "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-generic-arm64.qcow2",
        checksums: "https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS",
        algorithm: Algorithm::Sha512,
        signed: Signed::Https,
    },
    CloudImage {
        os: "Ubuntu 26.04 LTS",
        url: "https://cloud-images.ubuntu.com/resolute/current/resolute-server-cloudimg-arm64.img",
        checksums: "https://cloud-images.ubuntu.com/resolute/current/SHA256SUMS",
        algorithm: Algorithm::Sha256,
        signed: Signed::Detached {
            url: "https://cloud-images.ubuntu.com/resolute/current/SHA256SUMS.gpg",
            keys: include_bytes!("../data/ubuntu.gpg"),
        },
    },
    CloudImage {
        os: "Fedora Cloud 43",
        url: "https://download.fedoraproject.org/pub/fedora/linux/releases/43/Cloud/aarch64/images/Fedora-Cloud-Base-Generic-43-1.6.aarch64.qcow2",
        checksums: "https://download.fedoraproject.org/pub/fedora/linux/releases/43/Cloud/aarch64/images/Fedora-Cloud-43-1.6-aarch64-CHECKSUM",
        algorithm: Algorithm::Sha256,
        signed: Signed::Inline {
            keys: include_bytes!("../data/fedora.gpg"),
        },
    },
];

pub fn catalog() -> &'static [CloudImage] {
    CATALOG
}

pub struct ImageStore {
    dir: PathBuf,
    agent: ureq::Agent,
}

enum Hasher {
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Algorithm {
    fn hex_len(self) -> usize {
        match self {
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }

    fn hasher(self) -> Hasher {
        match self {
            Self::Sha256 => Hasher::Sha256(Sha256::new()),
            Self::Sha512 => Hasher::Sha512(Sha512::new()),
        }
    }
}

impl Hasher {
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(hasher) => hasher.update(bytes),
            Self::Sha512(hasher) => hasher.update(bytes),
        }
    }

    fn hex(self) -> String {
        let digest = match self {
            Self::Sha256(hasher) => hasher.finalize().to_vec(),
            Self::Sha512(hasher) => hasher.finalize().to_vec(),
        };
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl CloudImage {
    pub fn file_name(&self) -> &'static str {
        self.url.rsplit('/').next().unwrap_or(self.url)
    }

    fn listed_checksum(&self, checksums: &str) -> Option<String> {
        let file = self.file_name();
        let hex = checksums.lines().find_map(|line| {
            let line = line.trim();
            match line.split_once(") = ") {
                Some((head, hex)) => (head.split_once(" (")?.1 == file).then_some(hex),
                None => {
                    let (hex, name) = line.split_once(char::is_whitespace)?;
                    (name.trim_start().trim_start_matches('*') == file).then_some(hex)
                }
            }
        })?;
        (hex.len() == self.algorithm.hex_len() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| hex.to_ascii_lowercase())
    }
}

impl ImageStore {
    pub fn open() -> Result<Self> {
        let data = bande_xdg::data_home().ok_or(Error::Missing("XDG_DATA_HOME".into()))?;
        Ok(Self::at(data.join("velox/images")))
    }

    pub fn at(dir: PathBuf) -> Self {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(Arc::new(aws_lc_rs::default_provider()))
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(CONNECT_TIMEOUT))
            .user_agent("bande-vm")
            .build()
            .into();
        Self { dir, agent }
    }

    pub fn fetch(
        &self,
        image: &CloudImage,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<PathBuf> {
        let checksums = self
            .agent
            .get(image.checksums)
            .call()?
            .body_mut()
            .with_config()
            .limit(CHECKSUMS_LIMIT)
            .read_to_string()?;
        let checksums = self.verified(image, checksums)?;
        let expected = image
            .listed_checksum(&checksums)
            .ok_or_else(|| Error::NotListed(image.file_name().into()))?;
        let path = self
            .dir
            .join(format!("{}-{}", &expected[..16], image.file_name()));
        if path.is_file() {
            return Ok(path);
        }
        fs::create_dir_all(&self.dir)?;
        let staging = path.with_extension("partial");
        let downloaded = self.download(image, &expected, &staging, progress);
        match downloaded {
            Ok(file) => persist(&file, &staging, &path).map(|()| path),
            Err(error) => {
                let _ = fs::remove_file(&staging);
                Err(error)
            }
        }
    }

    fn verified(&self, image: &CloudImage, checksums: String) -> Result<String> {
        let keys = match image.signed {
            Signed::Https => return Ok(checksums),
            Signed::Detached { keys, .. } | Signed::Inline { keys } => keys,
        };
        fs::create_dir_all(&self.dir)?;
        let folder = self.dir.join(format!(".verify-{}", std::process::id()));
        fs::create_dir_all(&folder)?;
        let checked = self.run_gpgv(image, &checksums, keys, &folder);
        let _ = fs::remove_dir_all(&folder);
        checked
    }

    fn run_gpgv(
        &self,
        image: &CloudImage,
        checksums: &str,
        keys: &[u8],
        folder: &std::path::Path,
    ) -> Result<String> {
        let keyring = folder.join("keys.gpg");
        let signed = folder.join("sums");
        fs::write(&keyring, keys)?;
        fs::write(&signed, checksums)?;
        let mut command = Command::new(GPGV);
        command.arg("--keyring").arg(&keyring);
        if let Signed::Inline { .. } = image.signed {
            command.args(["--output", "-"]);
        }
        if let Signed::Detached { url, .. } = image.signed {
            let signature = folder.join("sums.sig");
            let mut bytes = Vec::new();
            self.agent
                .get(url)
                .call()?
                .into_body()
                .into_reader()
                .take(CHECKSUMS_LIMIT)
                .read_to_end(&mut bytes)?;
            fs::write(&signature, bytes)?;
            command.arg(signature);
        }
        let output = command.arg(&signed).stdin(Stdio::null()).output()?;
        if !output.status.success() {
            return Err(Error::Signature(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(match image.signed {
            Signed::Inline { .. } => String::from_utf8_lossy(&output.stdout).into_owned(),
            _ => checksums.to_owned(),
        })
    }

    fn download(
        &self,
        image: &CloudImage,
        expected: &str,
        staging: &std::path::Path,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<File> {
        let response = self.agent.get(image.url).call()?;
        let total = response.body().content_length();
        let mut body = response.into_body().into_reader();
        let mut file = File::create(staging)?;
        let mut hasher = image.algorithm.hasher();
        let mut buffer = vec![0; CHUNK];
        let mut done = 0;
        loop {
            let read = body.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])?;
            done += read as u64;
            if progress(done, total).is_break() {
                return Err(Error::Cancelled);
            }
        }
        let actual = hasher.hex();
        if actual != expected {
            return Err(Error::Checksum {
                expected: expected.into(),
                actual,
            });
        }
        Ok(file)
    }
}
