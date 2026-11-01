use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rustix::fs::OFlags;

pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub fn read_file(path: &Path) -> io::Result<String> {
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
    if metadata.len() > MAX_FILE_BYTES {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::{MAX_FILE_BYTES, read_file};

    #[test]
    fn a_calendar_file_reads_whole() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("party.ics");
        std::fs::write(&path, "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")?;
        assert_eq!(read_file(&path)?, "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");
        Ok(())
    }

    #[test]
    fn a_fifo_is_refused_instead_of_waiting_for_a_writer() -> Result<(), Box<dyn std::error::Error>>
    {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("pipe.ics");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()?
                .success()
        );
        assert_eq!(
            read_file(&path).map_err(|error| error.kind()),
            Err(std::io::ErrorKind::InvalidInput)
        );
        Ok(())
    }

    #[test]
    fn a_file_over_the_limit_is_refused_before_it_is_read() -> Result<(), Box<dyn std::error::Error>>
    {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("huge.ics");
        std::fs::File::create(&path)?.set_len(MAX_FILE_BYTES + 1)?;
        assert_eq!(
            read_file(&path).map_err(|error| error.kind()),
            Err(std::io::ErrorKind::FileTooLarge)
        );
        Ok(())
    }
}
