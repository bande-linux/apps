use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use fatfs::{FileSystem, FormatVolumeOptions, FsOptions};
use serde_json::{Value, json};

use crate::Result;

const SEED_BYTES: u64 = 2 * 1024 * 1024;
const LABEL: [u8; 11] = *b"CIDATA     ";

#[derive(Clone, Debug)]
pub struct CloudInit {
    pub hostname: String,
    pub user: String,
    pub ssh_keys: Vec<String>,
    pub packages: Vec<String>,
    pub password: Option<String>,
}

const NEW_MACHINE_ID: &str = "rm -f /etc/machine-id && systemd-machine-id-setup";
const START_AGENT: &str = "systemctl start qemu-guest-agent || true";

pub(crate) fn rewrite_seed(
    source: &Path,
    target: &Path,
    instance_id: &str,
    hostname: &str,
) -> Result<()> {
    let mut user_data = String::new();
    {
        let mut image = OpenOptions::new().read(true).write(true).open(source)?;
        let fs = FileSystem::new(&mut image, FsOptions::new())?;
        fs.root_dir()
            .open_file("user-data")?
            .read_to_string(&mut user_data)?;
    }
    let mut config: Value = serde_json::from_str(
        user_data
            .strip_prefix("#cloud-config\n")
            .unwrap_or(&user_data),
    )?;
    config["hostname"] = json!(hostname);
    config["runcmd"] = json!([NEW_MACHINE_ID, START_AGENT]);
    fs::remove_file(target).or_else(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(error),
    })?;
    write_seed_files(target, instance_id, hostname, &config)
}

fn write_seed_files(
    path: &Path,
    instance_id: &str,
    hostname: &str,
    user_data: &Value,
) -> Result<()> {
    let meta_data = json!({"instance-id": instance_id, "local-hostname": hostname});
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.set_len(SEED_BYTES)?;
    fatfs::format_volume(&mut file, FormatVolumeOptions::new().volume_label(LABEL))?;
    {
        let fs = FileSystem::new(&mut file, FsOptions::new())?;
        fs.root_dir()
            .create_file("meta-data")?
            .write_all(meta_data.to_string().as_bytes())?;
        fs.root_dir()
            .create_file("user-data")?
            .write_all(format!("#cloud-config\n{user_data}\n").as_bytes())?;
        fs.unmount()?;
    }
    file.sync_all()?;
    Ok(())
}

impl CloudInit {
    pub(crate) fn write_seed(&self, path: &Path, instance_id: &str) -> Result<()> {
        let password = match &self.password {
            Some(password) => Some(crate::password::hash_password(password)?),
            None => None,
        };
        let user_data = json!({
            "hostname": self.hostname,
            "users": [{
                "name": self.user,
                "shell": "/bin/bash",
                "sudo": "ALL=(ALL) NOPASSWD:ALL",
                "lock_passwd": password.is_none(),
                "hashed_passwd": password,
                "ssh_authorized_keys": self.ssh_keys,
            }],
            "ssh_pwauth": false,
            "package_update": !self.packages.is_empty(),
            "packages": self.packages,
            "runcmd": [START_AGENT],
        });
        write_seed_files(path, instance_id, &self.hostname, &user_data)
    }
}
