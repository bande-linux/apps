mod category;
mod component;
mod dep11;
mod lang;
mod search;
mod x11;
mod xml;

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use bande_xdg::Locale;
use flate2::read::MultiGzDecoder;

pub use category::Category;
pub use component::{Block, Bundle, Component, Icon, Release, Screenshot};
pub use dep11::parse as parse_dep11;
pub use search::search;
pub use x11::{flatpak_needs_x11, x11_only_packages};
pub use xml::{Source, parse as parse_xml};

pub const SYSTEM_ROOTS: [&str; 2] = ["/usr/share/swcatalog", "/var/lib/swcatalog"];
pub const FLATPAK_ROOT: &str = "/var/lib/flatpak/appstream";
pub const APT_LISTS: &str = "/var/lib/apt/lists";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Xml,
    Dep11,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogFile {
    pub path: PathBuf,
    pub format: Format,
    pub source: Source,
}

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

pub fn system_files(root: &Path) -> Vec<CatalogFile> {
    let source = Source {
        remote: None,
        icon_root: root.join("icons"),
    };
    [("xml", Format::Xml), ("yaml", Format::Dep11)]
        .into_iter()
        .flat_map(|(dir, format)| {
            sorted_files(&root.join(dir))
                .into_iter()
                .map(move |path| (path, format))
        })
        .map(|(path, format)| CatalogFile {
            path,
            format,
            source: source.clone(),
        })
        .collect()
}

pub fn flatpak_files(root: &Path, arch: &str) -> Vec<CatalogFile> {
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
                .map(|path| CatalogFile {
                    path,
                    format: Format::Xml,
                    source: Source {
                        remote: Some(remote),
                        icon_root: active.join("icons"),
                    },
                })
        })
        .collect()
}

pub fn merge(components: impl IntoIterator<Item = Component>) -> Vec<Component> {
    let mut merged: Vec<Component> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for component in components {
        let Some(known) = index
            .get(component.key())
            .and_then(|at| merged.get_mut(*at))
        else {
            index.insert(component.key().to_owned(), merged.len());
            merged.push(component);
            continue;
        };
        for bundle in component.bundles {
            if !known.bundles.contains(&bundle) {
                known.bundles.push(bundle);
            }
        }
        for icon in component.icons {
            if !known.icons.contains(&icon) {
                known.icons.push(icon);
            }
        }
        if known.screenshots.is_empty() {
            known.screenshots = component.screenshots;
        }
    }
    merged
}

fn parse_file(file: &CatalogFile, locale: Option<&Locale>) -> Result<Vec<Component>, String> {
    let text = read_text(&file.path).map_err(|error| error.to_string())?;
    match file.format {
        Format::Xml => parse_xml(&text, &file.source, locale).map_err(|error| error.to_string()),
        Format::Dep11 => Ok(parse_dep11(&text, &file.source.icon_root, locale)),
    }
}

pub fn load(files: &[CatalogFile], locale: Option<&Locale>) -> Vec<Component> {
    merge(files.iter().flat_map(|file| {
        parse_file(file, locale).unwrap_or_else(|error| {
            tracing::warn!(path = %file.path.display(), %error, "skipping AppStream catalogue");
            Vec::new()
        })
    }))
}

pub fn mark_x11(apps: &mut [Component], x11_only: &HashSet<String>) {
    for app in apps {
        app.needs_x11 = app.bundles.iter().all(|bundle| match bundle {
            Bundle::Deb(package) => x11_only.contains(package),
            Bundle::Flatpak { .. } => false,
        }) && !app.bundles.is_empty();
    }
}

pub fn package_indexes(lists: &Path) -> Vec<PathBuf> {
    sorted_files(lists)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_Packages"))
        })
        .collect()
}

pub fn flatpak_arch() -> &'static str {
    std::env::consts::ARCH
}

pub fn system_catalog() -> Vec<Component> {
    let locale = Locale::from_env();
    let mut files: Vec<_> = SYSTEM_ROOTS
        .iter()
        .flat_map(|root| system_files(Path::new(root)))
        .collect();
    files.extend(flatpak_files(Path::new(FLATPAK_ROOT), flatpak_arch()));
    let mut apps = load(&files, locale.as_ref());
    let x11_only: HashSet<String> = package_indexes(Path::new(APT_LISTS))
        .iter()
        .filter_map(|path| read_text(path).ok())
        .flat_map(|index| x11_only_packages(&index))
        .collect();
    mark_x11(&mut apps, &x11_only);
    apps
}

pub fn icon_path(icons: &[Icon], size: u32, data_dirs: &[PathBuf]) -> Option<PathBuf> {
    let mut cached: Vec<_> = icons
        .iter()
        .filter_map(|icon| match icon {
            Icon::Cached { path, size } if path.is_file() => Some((*size, path)),
            _ => None,
        })
        .collect();
    cached.sort_by_key(|(found, _)| (*found < size, found.abs_diff(size)));
    cached
        .first()
        .map(|(_, path)| (*path).clone())
        .or_else(|| {
            icons.iter().find_map(|icon| match icon {
                Icon::Local(path) if path.is_file() => Some(path.clone()),
                _ => None,
            })
        })
        .or_else(|| {
            icons.iter().find_map(|icon| match icon {
                Icon::Stock(name) => {
                    bande_xdg::find_icon(name, size, &bande_xdg::ICON_THEMES, data_dirs)
                }
                _ => None,
            })
        })
}
