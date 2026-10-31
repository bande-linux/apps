use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const EXTENSION: &str = "window";
const ACTIVE: &str = "active ";
const TAB: &str = "tab ";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedWindow {
    pub tabs: Vec<PathBuf>,
    pub active: usize,
}

impl SavedWindow {
    pub fn parse(text: &str) -> Option<Self> {
        let mut window = Self::default();
        for line in text.lines() {
            if let Some(active) = line.strip_prefix(ACTIVE) {
                window.active = active.parse().ok()?;
            } else if let Some(folder) = line.strip_prefix(TAB) {
                let folder = PathBuf::from(folder);
                if folder.is_absolute() {
                    window.tabs.push(folder);
                }
            }
        }
        if window.tabs.is_empty() {
            return None;
        }
        window.active = window.active.min(window.tabs.len() - 1);
        Some(window)
    }

    pub fn to_text(&self) -> String {
        let mut text = format!("{ACTIVE}{}\n", self.active);
        for folder in &self.tabs {
            if let Some(folder) = folder.to_str().filter(|folder| !folder.contains('\n')) {
                text.push_str(TAB);
                text.push_str(folder);
                text.push('\n');
            }
        }
        text
    }

    pub fn save(&self, file: &Path) -> io::Result<()> {
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder)?;
        }
        let staging = file.with_extension("new");
        fs::write(&staging, self.to_text())?;
        fs::rename(staging, file)
    }
}

pub fn windows_folder() -> Option<PathBuf> {
    unbound_xdg::state_home().map(|state| state.join("unbound/terminal-windows"))
}

pub fn window_file(folder: &Path, pid: u32) -> PathBuf {
    folder.join(format!("{pid}.{EXTENSION}"))
}

pub fn left_behind(folder: &Path, running: impl Fn(u32) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == EXTENSION)
        })
        .filter(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse().ok())
                .is_none_or(|pid| !running(pid))
        })
        .collect();
    files.sort();
    files
}

pub fn claim(file: &Path, mine: &Path) -> Option<SavedWindow> {
    fs::rename(file, mine).ok()?;
    let window = fs::read_to_string(mine)
        .ok()
        .and_then(|text| SavedWindow::parse(&text));
    if window.is_none() {
        fs::remove_file(mine).ok();
    }
    window
}
