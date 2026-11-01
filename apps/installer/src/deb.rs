use std::cmp::Ordering;
use std::io;
use std::path::Path;
use std::process::Command;

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
        installed_size: size.trim().parse::<u64>().unwrap_or(0) * 1024,
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
