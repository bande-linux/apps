mod component;
mod lang;
mod xml;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use bande_xdg::Locale;
use flate2::read::MultiGzDecoder;

pub use component::{Block, Bundle, Component, Icon, Release, Screenshot};
pub use xml::{Source, parse as parse_xml};

pub const SYSTEM_ROOTS: [&str; 2] = ["/usr/share/swcatalog", "/var/lib/swcatalog"];
pub const FLATPAK_ROOT: &str = "/var/lib/flatpak/appstream";

pub fn read_text(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    let file = File::open(path)?;
    if path.extension().is_some_and(|extension| extension == "gz") {
        MultiGzDecoder::new(file).read_to_string(&mut text)?;
    } else {
        io::BufReader::new(file).read_to_string(&mut text)?;
    }
    Ok(text)
}

fn sorted_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

pub fn system_xml(root: &Path) -> Vec<(PathBuf, Source)> {
    sorted_files(&root.join("xml"))
        .into_iter()
        .map(|path| {
            (
                path,
                Source {
                    remote: None,
                    icon_root: root.join("icons"),
                },
            )
        })
        .collect()
}

pub fn flatpak_xml(root: &Path, arch: &str) -> Vec<(PathBuf, Source)> {
    let mut remotes: Vec<_> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    remotes.sort();
    remotes
        .into_iter()
        .filter_map(|remote| {
            let active = root.join(&remote).join(arch).join("active");
            ["appstream.xml.gz", "appstream.xml"]
                .into_iter()
                .map(|name| active.join(name))
                .find(|path| path.is_file())
                .map(|path| {
                    (
                        path,
                        Source {
                            remote: Some(remote),
                            icon_root: active.join("icons"),
                        },
                    )
                })
        })
        .collect()
}

pub fn merge(components: impl IntoIterator<Item = Component>) -> Vec<Component> {
    let mut merged: Vec<Component> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for component in components {
        let Some(known) = index.get(&component.id).and_then(|at| merged.get_mut(*at)) else {
            index.insert(component.id.clone(), merged.len());
            merged.push(component);
            continue;
        };
        for bundle in component.bundles {
            if !known.bundles.contains(&bundle) {
                known.bundles.push(bundle);
            }
        }
        if known.icons.is_empty() {
            known.icons = component.icons;
        }
        if known.screenshots.is_empty() {
            known.screenshots = component.screenshots;
        }
    }
    merged
}

pub fn load_xml(files: &[(PathBuf, Source)], locale: Option<&Locale>) -> Vec<Component> {
    files
        .iter()
        .flat_map(|(path, source)| {
            let parsed = read_text(path)
                .map_err(|error| error.to_string())
                .and_then(|text| {
                    parse_xml(&text, source, locale).map_err(|error| error.to_string())
                });
            parsed.unwrap_or_else(|error| {
                tracing::warn!(path = %path.display(), %error, "skipping AppStream catalogue");
                Vec::new()
            })
        })
        .collect()
}
