use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use fatfs::{FileSystem, FormatVolumeOptions, FsOptions};
use serde_json::json;

use crate::Result;

const SEED_BYTES: u64 = 2 * 1024 * 1024;
const LABEL: [u8; 11] = *b"CIDATA     ";

#[derive(Clone, Debug)]
pub struct CloudInit {
    pub hostname: String,
    pub user: String,
    pub ssh_keys: Vec<String>,
    pub packages: Vec<String>,
}

impl CloudInit {
    pub(crate) fn write_seed(&self, path: &Path, instance_id: &str) -> Result<()> {
        let meta_data = json!({"instance-id": instance_id, "local-hostname": self.hostname});
        let user_data = json!({
            "hostname": self.hostname,
            "users": [{
                "name": self.user,
                "shell": "/bin/bash",
                "sudo": "ALL=(ALL) NOPASSWD:ALL",
                "lock_passwd": true,
                "ssh_authorized_keys": self.ssh_keys,
            }],
            "ssh_pwauth": false,
            "package_update": !self.packages.is_empty(),
            "packages": self.packages,
        });
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
}
