use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;
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

const APPLICATIONS: &str = "usr/share/applications/";
const HICOLOR: &str = "usr/share/icons/hicolor/";
const PIXMAPS: &str = "usr/share/pixmaps/";
const BLOCK: u64 = 512;
const FILE_LIMIT: u64 = 4 << 20;
const TOTAL_LIMIT: u64 = 16 << 20;
const ICON_PIXELS: u32 = 128;

fn wanted(name: &str) -> bool {
    let direct = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
    };
    let icon = name
        .strip_prefix(HICOLOR)
        .and_then(|rest| rest.split_once("/apps/"))
        .is_some_and(|(size, file)| !size.contains('/') && !file.is_empty() && !file.contains('/'));
    (direct(APPLICATIONS) && name.ends_with(".desktop")) || direct(PIXMAPS) || icon
}

fn field(header: &[u8], range: std::ops::Range<usize>) -> &str {
    let bytes = header.get(range).unwrap_or_default();
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(bytes.get(..end).unwrap_or_default()).unwrap_or_default()
}

fn octal(header: &[u8]) -> Option<u64> {
    let size = header.get(124..136)?;
    if size.first().is_some_and(|byte| byte & 0x80 != 0) {
        return size.iter().skip(1).try_fold(0u64, |total, byte| {
            total.checked_mul(256)?.checked_add(u64::from(*byte))
        });
    }
    u64::from_str_radix(field(header, 124..136).trim(), 8).ok()
}

fn discard(reader: &mut impl Read, length: u64) -> io::Result<()> {
    let copied = io::copy(&mut reader.take(length), &mut io::sink())?;
    match copied == length {
        true => Ok(()),
        false => Err(io::ErrorKind::UnexpectedEof.into()),
    }
}

fn skip(reader: &mut impl Read, size: u64) -> io::Result<()> {
    discard(reader, size.div_ceil(BLOCK) * BLOCK)
}

fn read_member(reader: &mut impl Read, size: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(size).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    discard(reader, size.div_ceil(BLOCK) * BLOCK - size)?;
    Ok(bytes)
}

fn pax_path(data: &[u8]) -> Option<String> {
    String::from_utf8_lossy(data).lines().find_map(|line| {
        let (_, record) = line.split_once(' ')?;
        record.strip_prefix("path=").map(str::to_owned)
    })
}

fn members(reader: &mut impl Read) -> HashMap<String, Vec<u8>> {
    let mut found = HashMap::new();
    let mut kept = 0u64;
    let mut long_name: Option<String> = None;
    let mut header = [0u8; BLOCK as usize];
    while reader.read_exact(&mut header).is_ok() && header.iter().any(|byte| *byte != 0) {
        let Some(size) = octal(&header) else {
            break;
        };
        let kind = header[156];
        let read = match kind {
            b'L' | b'x' => read_member(reader, size.min(FILE_LIMIT)).map(|data| {
                long_name = match kind {
                    b'L' => Some(field(&data, 0..data.len()).to_owned()),
                    _ => pax_path(&data),
                };
            }),
            b'0' | 0 => {
                let prefix = field(&header, 345..500);
                let name = long_name.take().unwrap_or_else(|| match prefix.is_empty() {
                    true => field(&header, 0..100).to_owned(),
                    false => format!("{prefix}/{}", field(&header, 0..100)),
                });
                let name = name.trim_start_matches("./").to_owned();
                if wanted(&name) && size <= FILE_LIMIT && kept + size <= TOTAL_LIMIT {
                    kept += size;
                    read_member(reader, size).map(|data| {
                        found.insert(name, data);
                    })
                } else {
                    skip(reader, size)
                }
            }
            _ => {
                long_name = None;
                skip(reader, size)
            }
        };
        if read.is_err() {
            break;
        }
    }
    if let Err(error) = io::copy(reader, &mut io::sink()) {
        tracing::debug!("cannot read the rest of the package: {error}");
    }
    found
}

fn archive_members(path: &Path) -> Option<HashMap<String, Vec<u8>>> {
    let mut archive = Command::new("dpkg-deb")
        .arg("--fsys-tarfile")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let found = archive.stdout.take().map(|mut stdout| members(&mut stdout));
    let finished = archive.wait().ok()?;
    found.filter(|_| finished.success())
}

fn decode_icon(name: &str, bytes: &[u8]) -> Option<Image> {
    if name.ends_with(".svg") {
        return Image::from_svg(bytes, ICON_PIXELS, ICON_PIXELS);
    }
    let decoded = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    Image::from_rgba(width, height, decoded.into_raw())
}

fn icon_member<'a>(files: &'a HashMap<String, Vec<u8>>, icon: &str) -> Option<(&'a str, &'a [u8])> {
    let stem_is = |file: &str| {
        [".svg", ".png"]
            .iter()
            .any(|extension| file.strip_suffix(extension) == Some(icon))
    };
    let rank = |name: &str| -> Option<u32> {
        if let Some((size, file)) = name.strip_prefix(HICOLOR)?.split_once("/apps/") {
            return stem_is(file).then(|| match size {
                "scalable" => Some(u32::MAX),
                _ => size.split('x').next()?.parse().ok(),
            })?;
        }
        None
    };
    files
        .iter()
        .filter_map(|(name, bytes)| Some((rank(name)?, name, bytes)))
        .max_by_key(|(rank, ..)| *rank)
        .map(|(_, name, bytes)| (name.as_str(), bytes.as_slice()))
        .or_else(|| {
            files
                .iter()
                .find(|(name, _)| name.strip_prefix(PIXMAPS).is_some_and(stem_is))
                .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        })
}

pub fn identity(path: &Path) -> Option<Identity> {
    let files = archive_members(path)?;
    let locale = bande_xdg::Locale::from_env();
    let desktops = bande_xdg::current_desktops();
    let mut entries: Vec<(&String, &Vec<u8>)> = files
        .iter()
        .filter(|(name, _)| name.starts_with(APPLICATIONS))
        .collect();
    entries.sort();
    let entry = entries.into_iter().find_map(|(name, bytes)| {
        let text = std::str::from_utf8(bytes).ok()?;
        let id = name.strip_prefix(APPLICATIONS)?;
        let file = Path::new("/").join(name);
        DesktopEntry::parse(id, &file, text, locale.as_ref(), &desktops)
    })?;
    let icon = entry
        .icon
        .as_deref()
        .filter(|icon| !icon.contains('/'))
        .and_then(|icon| icon_member(&files, icon))
        .and_then(|(name, bytes)| decode_icon(name, bytes));
    Some(Identity {
        name: entry.name,
        icon,
    })
}
