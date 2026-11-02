use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rustix::fs::OFlags;

pub const MAX_FILE_BYTES: u64 = 512 << 20;

pub fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(OFlags::NONBLOCK.bits() as i32)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if metadata.len() > limit {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    Ok(bytes)
}

fn staging(target: &Path) -> io::Result<PathBuf> {
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut staged = std::ffi::OsString::from(".");
    staged.push(name);
    staged.push(format!(".{}.saving", std::process::id()));
    Ok(target.with_file_name(staged))
}

pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let staged = staging(&target)?;
    let permissions = fs::metadata(&target)
        .ok()
        .map(|metadata| metadata.permissions());
    let written = (|| {
        let mut file = File::create(&staged)?;
        file.write_all(bytes)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        fs::rename(&staged, &target)?;
        if let Some(folder) = target.parent()
            && let Ok(folder) = File::open(folder)
        {
            folder.sync_all()?;
        }
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    written
}
