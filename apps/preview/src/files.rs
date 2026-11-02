use std::path::{Path, PathBuf};

use bande_fs::{Entry, Sort};

const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp", "svg"];

pub fn is_image_name(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

pub fn mime_of_name(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "image/jpeg",
    }
}

pub fn neighbour(path: &Path, forward: bool) -> Option<PathBuf> {
    let folder = path.parent()?;
    let entries: Vec<Entry> = bande_fs::list(folder, false, Sort::default())
        .ok()?
        .into_iter()
        .filter(|entry| !entry.is_folder() && is_image_name(&entry.path))
        .collect();
    let at = entries.iter().position(|entry| entry.path == path);
    let count = entries.len();
    let next = match (at, forward) {
        (_, _) if count == 0 => return None,
        (Some(at), true) => (at + 1) % count,
        (Some(at), false) => (at + count - 1) % count,
        (None, true) => 0,
        (None, false) => count - 1,
    };
    entries
        .into_iter()
        .nth(next)
        .map(|entry| entry.path)
        .filter(|next| next != path)
}

pub fn argument_path(argument: &str, directory: &Path) -> PathBuf {
    match argument.starts_with("file://") {
        true => bande_xdg::path_from_file_uri(argument).unwrap_or_else(|| PathBuf::from(argument)),
        false => directory.join(argument),
    }
}
