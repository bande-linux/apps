#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{self, File};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

use bande_archive::{ArchiveSettings, Ended, Work};
use bande_fs::{Format, Refusal};
use tar::{EntryType, Header};
use zip::write::SimpleFileOptions;

fn made(work: std::io::Result<Work>) -> Ended {
    work.unwrap().wait().unwrap()
}

fn done(ended: Ended) -> PathBuf {
    let Ended::Done(made) = ended else {
        panic!("{ended:?}")
    };
    made[0].clone()
}

fn expand(archive: &Path, password: Option<&str>) -> Ended {
    let (password, settings) = (password.map(str::to_owned), ArchiveSettings::default());
    made(Work::expand(archive.into(), password, &settings))
}

fn compress(source: &Path, into: &Path, format: Format) -> PathBuf {
    let mut settings = ArchiveSettings::default();
    (settings.archive_into, settings.format) = (Some(into.to_path_buf()), format);
    done(made(Work::compress(vec![source.into()], &settings)))
}

fn refuses_tar(path: &Path, entries: &[(&str, EntryType, &str)]) {
    let mut builder = tar::Builder::new(File::create(path).unwrap());
    for (name, kind, target) in entries {
        let mut header = Header::new_gnu();
        let gnu = header.as_gnu_mut().unwrap();
        gnu.name[..name.len()].copy_from_slice(name.as_bytes());
        gnu.linkname[..target.len()].copy_from_slice(target.as_bytes());
        header.set_entry_type(*kind);
        header.set_size(1);
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
    }
    builder.finish().unwrap();
    refused(path, Refusal::Outside);
}

fn one_entry_zip(path: &Path, name: &str) {
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    zip.start_file(name, SimpleFileOptions::default()).unwrap();
    zip.finish().unwrap();
}

fn refused(archive: &Path, refusal: Refusal) {
    let expected = Ended::Failed(refusal.to_string());
    assert_eq!(expand(archive, None), expected, "{archive:?}");
    let left = fs::read_dir(archive.parent().unwrap()).unwrap().count();
    assert_eq!(left, 1, "{archive:?}");
}

#[test]
fn compressed_items_expand_back_without_setuid_and_encrypted_archives_ask_for_the_password() {
    let dir = tempfile::tempdir().unwrap();
    let photos = dir.path().join("Photos");
    fs::create_dir_all(photos.join("trip")).unwrap();
    fs::write(photos.join("trip/note.txt"), "hello").unwrap();
    fs::write(photos.join("run.sh"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(photos.join("run.sh"), fs::Permissions::from_mode(0o4755)).unwrap();
    symlink("trip/note.txt", photos.join("latest")).unwrap();
    for format in Format::CREATED {
        let out = dir.path().join(format.suffix());
        fs::create_dir(&out).unwrap();
        let expanded = done(expand(&compress(&photos, &out, format), None));
        assert_eq!(expanded, out.join("Photos"));
        assert_eq!(fs::read(expanded.join("latest")).unwrap(), b"hello");
        let mode = fs::metadata(expanded.join("run.sh")).unwrap().permissions();
        assert_eq!(mode.mode() & 0o7777, 0o755, "{format:?}");
    }
    let Ok(_) = Command::new("7z").arg("i").output() else {
        return;
    };
    for (name, kind) in [("locked.zip", "-tzip"), ("locked.7z", "-mhe=on")] {
        let archive = dir.path().join(name);
        let mut seven = Command::new("7z");
        seven
            .args(["a", kind, "-psecret"])
            .arg(&archive)
            .arg(&photos);
        assert!(seven.output().unwrap().status.success());
        let asked = |password, wrong| match expand(&archive, password) {
            Ended::Password { wrong: asked, .. } => asked == wrong,
            _ => false,
        };
        assert!(asked(None, false) && asked(Some("nope"), true), "{name}");
        let expanded = done(expand(&archive, Some("secret")));
        assert_eq!(fs::read(expanded.join("trip/note.txt")).unwrap(), b"hello");
    }
}

#[test]
fn hostile_archives_are_refused_and_leave_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("evil");
    let case = |name: &str| {
        let folder = dir.path().join(name.replace('.', "-"));
        fs::create_dir_all(&folder).unwrap();
        folder.join(name)
    };
    let (file, link) = (EntryType::Regular, EntryType::Symlink);
    let absolute = outside.to_string_lossy();
    refuses_tar(&case("up.tar"), &[("../evil", file, "")]);
    refuses_tar(&case("absolute.tar"), &[(&absolute, file, "")]);
    refuses_tar(&case("in.tar"), &[("a", link, ".."), ("a/evil", file, "")]);
    refuses_tar(&case("escaping.tar"), &[("a", link, "../../../etc")]);
    one_entry_zip(&case("up.zip"), "../evil");
    refused(&case("up.zip"), Refusal::Outside);
    let lying = case("lying.zip");
    one_entry_zip(&lying, "big");
    let mut bytes = fs::read(&lying).unwrap();
    for (signature, at) in [(b"PK\x03\x04", 22), (b"PK\x01\x02", 24)] {
        let found = bytes.windows(4).position(|window| window == signature);
        let start = found.unwrap() + at;
        bytes[start..start + 4].copy_from_slice(&0xF000_0000u32.to_le_bytes());
    }
    fs::write(&lying, bytes).unwrap();
    refused(&lying, Refusal::Bomb);
    let zeros = dir.path().join("zeros");
    File::create(&zeros).unwrap().set_len(65 << 20).unwrap();
    let bombs = case("bomb.tar.bz2");
    let bomb = compress(&zeros, bombs.parent().unwrap(), Format::TarBzip2);
    refused(&bomb, Refusal::Bomb);
    assert!(!outside.exists());
}
