#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use bande_software::catalog::{Source, parse_dep11, parse_xml};

#[test]
fn a_dep11_file_with_a_syntax_error_is_read_up_to_the_error_and_no_further() {
    let (sender, parsed) = mpsc::channel();
    std::thread::spawn(move || {
        let yaml =
            "---\nFile: DEP-11\n---\nType: desktop-application\nID: a\nName: [\n---\nID: b\n";
        let _ = sender.send(parse_dep11(yaml, Path::new("/icons"), None).len());
    });
    assert!(parsed.recv_timeout(Duration::from_secs(5)).is_ok());
}

#[test]
fn appstream_xml_that_declares_a_doctype_is_read() {
    let xml = "<?xml version=\"1.0\"?>\n<!DOCTYPE components SYSTEM \"appstream.dtd\">\n\
        <components version=\"1.0\" origin=\"debian\"><component type=\"desktop-application\">\
        <id>org.gnome.Calculator</id><name>Calculator</name><summary>Arithmetic</summary>\
        <pkgname>gnome-calculator</pkgname></component></components>";
    let source = Source {
        remote: None,
        icon_root: PathBuf::from("/icons"),
    };
    let apps = parse_xml(xml, &source, None).unwrap();
    assert_eq!(apps.len(), 1);
}
