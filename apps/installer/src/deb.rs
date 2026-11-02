use std::cmp::Ordering;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bande_toolkit::Image;
use bande_xdg::DesktopEntry;

const SHOW_FORMAT: &str = "${Package}\\n${Version}\\n${Architecture}\\n${Maintainer}\\n${Installed-Size}\\n${Homepage}\\n${Description}";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Deb {
    pub name: String,
    pub version: String,
    pub arch: String,
    pub maintainer: String,
    pub installed_size: u64,
    pub homepage: String,
    pub summary: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub native: String,
    pub foreign: Vec<String>,
}

impl Machine {
    pub fn current() -> io::Result<Self> {
        let words = |argument: &str| -> io::Result<Vec<String>> {
            let output = Command::new("dpkg").arg(argument).output()?;
            Ok(String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .map(str::to_owned)
                .collect())
        };
        Ok(Self {
            native: words("--print-architecture")?
                .into_iter()
                .next()
                .unwrap_or_default(),
            foreign: words("--print-foreign-architectures")?,
        })
    }

    pub fn fits(&self, arch: &str) -> bool {
        arch == "all" || arch == self.native || self.foreign.iter().any(|known| known == arch)
    }
}

fn description(lines: &[&str]) -> (String, String) {
    let summary = lines.first().map(|line| line.trim()).unwrap_or_default();
    let body = lines
        .iter()
        .skip(1)
        .copied()
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .map(|line| if line == "." { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n");
    (summary.to_owned(), body.trim().to_owned())
}

pub fn read(path: &Path) -> Option<Deb> {
    let output = Command::new("dpkg-deb")
        .arg(format!("--showformat={SHOW_FORMAT}"))
        .arg("--show")
        .arg(path)
        .output()
        .inspect_err(|error| tracing::warn!("cannot run dpkg-deb: {error}"))
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let [name, version, arch, maintainer, size, homepage, rest @ ..] = lines.as_slice() else {
        return None;
    };
    if name.is_empty() || version.is_empty() || arch.is_empty() {
        return None;
    }
    let (summary, description) = description(rest);
    Some(Deb {
        name: (*name).to_owned(),
        version: (*version).to_owned(),
        arch: (*arch).to_owned(),
        maintainer: (*maintainer).to_owned(),
        installed_size: size.trim().parse::<u64>().unwrap_or(0).saturating_mul(1024),
        homepage: (*homepage).to_owned(),
        summary,
        description,
    })
}

fn dpkg_says(older: &str, relation: &str, newer: &str) -> bool {
    Command::new("dpkg")
        .args(["--compare-versions", older, relation, newer])
        .status()
        .is_ok_and(|status| status.success())
}

pub fn compare(version: &str, other: &str) -> Ordering {
    if dpkg_says(version, "lt", other) {
        Ordering::Less
    } else if dpkg_says(version, "gt", other) {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

pub fn missing_requirements(details: &str) -> Vec<String> {
    let mut missing: Vec<String> = details
        .lines()
        .filter_map(|line| {
            let (_, after) = line.split_once("Depends:")?;
            let requirement = after.split(" but ").next().unwrap_or(after).trim();
            (!requirement.is_empty()).then(|| requirement.to_owned())
        })
        .collect();
    missing.dedup();
    missing
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub name: String,
    pub icon: Option<Image>,
}

const APPLICATIONS: &str = "usr/share/applications";
const HICOLOR: &str = "usr/share/icons/hicolor";
const PIXMAPS: &str = "usr/share/pixmaps";
const WANTED: [&str; 3] = [
    "./usr/share/applications/*.desktop",
    "./usr/share/icons/hicolor/*/apps/*",
    "./usr/share/pixmaps/*",
];
const ICON_PIXELS: u32 = 128;

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            tracing::debug!("cannot remove {}: {error}", self.0.display());
        }
    }
}

fn extract(path: &Path) -> Option<Scratch> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let dir = Scratch(
        std::env::temp_dir().join(format!("bande-installer-{}-{stamp}", std::process::id())),
    );
    std::fs::create_dir(&dir.0).ok()?;
    let mut archive = Command::new("dpkg-deb")
        .arg("--fsys-tarfile")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let extracted = Command::new("tar")
        .args([
            "--extract",
            "--no-same-owner",
            "--no-same-permissions",
            "--wildcards",
            "--no-wildcards-match-slash",
            "--file",
            "-",
            "--directory",
        ])
        .arg(&dir.0)
        .args(WANTED)
        .stdin(archive.stdout.take()?)
        .stderr(Stdio::null())
        .status();
    if let Err(error) = archive.wait() {
        tracing::debug!("dpkg-deb did not finish: {error}");
    }
    extracted.ok().map(|_| dir)
}

fn regular(path: PathBuf) -> Option<PathBuf> {
    std::fs::symlink_metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|_| path)
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| regular(entry.path()))
        .collect();
    found.sort();
    found
}

fn icon_file(root: &Path, icon: &str) -> Option<PathBuf> {
    let named = |dir: &Path| {
        ["svg", "png"]
            .into_iter()
            .find_map(|extension| regular(dir.join(format!("{icon}.{extension}"))))
    };
    let mut sizes: Vec<(u32, PathBuf)> = std::fs::read_dir(root.join(HICOLOR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let size = entry.file_name().into_string().ok()?;
            let rank = match size.as_str() {
                "scalable" => u32::MAX,
                _ => size.split('x').next()?.parse().ok()?,
            };
            Some((rank, named(&entry.path().join("apps"))?))
        })
        .collect();
    sizes.sort();
    sizes
        .pop()
        .map(|(_, path)| path)
        .or_else(|| named(&root.join(PIXMAPS)))
}

pub fn identity(path: &Path) -> Option<Identity> {
    let scratch = extract(path)?;
    let locale = bande_xdg::Locale::from_env();
    let desktops = bande_xdg::current_desktops();
    let entry = files(&scratch.0.join(APPLICATIONS))
        .into_iter()
        .filter(|file| {
            file.extension()
                .is_some_and(|extension| extension == "desktop")
        })
        .find_map(|file| {
            let text = std::fs::read_to_string(&file).ok()?;
            let id = file.file_name()?.to_str()?.to_owned();
            DesktopEntry::parse(&id, &file, &text, locale.as_ref(), &desktops)
        })?;
    let icon = entry
        .icon
        .as_deref()
        .filter(|icon| !icon.contains('/'))
        .and_then(|icon| icon_file(&scratch.0, icon))
        .and_then(|file| Image::load(&file, ICON_PIXELS));
    Some(Identity {
        name: entry.name,
        icon,
    })
}
