use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    Stock(String),
    Cached { path: PathBuf, size: u32 },
    Local(PathBuf),
    Remote { url: String, size: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub image: String,
    pub thumbnail: Option<String>,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bundle {
    Deb(String),
    Flatpak { remote: String, reference: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub description: Vec<Block>,
    pub developer: Option<String>,
    pub license: Option<String>,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub icons: Vec<Icon>,
    pub screenshots: Vec<Screenshot>,
    pub launchable: Option<String>,
    pub releases: Vec<Release>,
    pub bundles: Vec<Bundle>,
    pub needs_x11: bool,
}

pub const APPLICATION_KINDS: [&str; 2] = ["desktop-application", "desktop"];

pub const THUMBNAIL_WIDTH: u32 = 752;

impl Component {
    pub fn key(&self) -> &str {
        self.id.strip_suffix(".desktop").unwrap_or(&self.id)
    }
}
