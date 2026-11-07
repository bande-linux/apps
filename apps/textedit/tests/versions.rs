#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use bande_textedit::Session;

#[test]
fn saving_over_a_file_keeps_old_versions_thinned_with_age() {
    let folder = tempfile::tempdir().expect("folder");
    let session = Session::new(folder.path().to_path_buf());
    let document = Path::new("/notes/plan.txt");
    let now = 40 * 86_400;
    session
        .keep_version(document, b"old", now - 35 * 86_400)
        .unwrap();
    session
        .keep_version(document, b"older still", now - 35 * 86_400 + 60)
        .unwrap();
    session
        .keep_version(document, b"this morning", now - 7_200)
        .unwrap();
    session
        .keep_version(document, b"just now", now - 60)
        .unwrap();
    session
        .keep_version(document, b"just now", now - 30)
        .unwrap();
    session
        .keep_version(document, b"a moment ago", now)
        .unwrap();
    let kept = session.versions(document);
    assert_eq!(kept.first(), Some(&now));
    assert_eq!(session.version(document, now).unwrap(), b"a moment ago");
    assert!(kept.contains(&(now - 7_200)));
    assert_eq!(kept.iter().filter(|at| now - **at > 86_400).count(), 1);
    assert_eq!(session.versions(Path::new("/other.txt")), Vec::<u64>::new());
}
