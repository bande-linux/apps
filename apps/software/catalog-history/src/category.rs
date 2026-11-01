#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Internet,
    Productivity,
    Graphics,
    MusicAndVideo,
    Games,
    Education,
    Development,
    Utilities,
    System,
}

impl Category {
    pub const ALL: [Self; 9] = [
        Self::Internet,
        Self::Productivity,
        Self::Graphics,
        Self::MusicAndVideo,
        Self::Games,
        Self::Education,
        Self::Development,
        Self::Utilities,
        Self::System,
    ];

    pub fn from_freedesktop(name: &str) -> Option<Self> {
        Some(match name {
            "Network" => Self::Internet,
            "Office" => Self::Productivity,
            "Graphics" => Self::Graphics,
            "AudioVideo" | "Audio" | "Video" => Self::MusicAndVideo,
            "Game" => Self::Games,
            "Education" | "Science" => Self::Education,
            "Development" => Self::Development,
            "Utility" => Self::Utilities,
            "System" | "Settings" => Self::System,
            _ => return None,
        })
    }

    pub fn of(categories: &[String]) -> Vec<Self> {
        let mut found: Vec<Self> = Vec::new();
        for category in categories
            .iter()
            .filter_map(|name| Self::from_freedesktop(name))
        {
            if !found.contains(&category) {
                found.push(category);
            }
        }
        found
    }
}
