mod session;
pub mod utf7;
pub mod wire;

pub use session::{Fetched, Folder, Selected, Session, uid_sets};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    All,
    Flagged,
}

impl Role {
    pub const ALL: [Self; 8] = [
        Self::Inbox,
        Self::Drafts,
        Self::Sent,
        Self::Archive,
        Self::Junk,
        Self::Trash,
        Self::All,
        Self::Flagged,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Drafts => "drafts",
            Self::Sent => "sent",
            Self::Archive => "archive",
            Self::Junk => "junk",
            Self::Trash => "trash",
            Self::All => "all",
            Self::Flagged => "flagged",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    fn from_attribute(attribute: &str) -> Option<Self> {
        let attribute = attribute.strip_prefix('\\')?;
        Self::ALL
            .into_iter()
            .filter(|role| *role != Self::Inbox)
            .find(|role| attribute.eq_ignore_ascii_case(role.name()))
    }

    fn guess(path: &str, delimiter: Option<char>) -> Option<Self> {
        if path.eq_ignore_ascii_case("INBOX") {
            return Some(Self::Inbox);
        }
        let leaf = delimiter
            .and_then(|delimiter| path.rsplit(delimiter).next())
            .unwrap_or(path)
            .to_lowercase();
        let guesses: [(&[&str], Self); 5] = [
            (
                &["sent", "sent items", "sent mail", "sent messages"],
                Self::Sent,
            ),
            (&["drafts", "draft"], Self::Drafts),
            (
                &["trash", "deleted items", "deleted messages", "bin"],
                Self::Trash,
            ),
            (&["junk", "spam", "junk e-mail", "junk email"], Self::Junk),
            (&["archive", "archives"], Self::Archive),
        ];
        guesses
            .into_iter()
            .find_map(|(names, role)| names.contains(&leaf.as_str()).then_some(role))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub deleted: bool,
    pub draft: bool,
    pub color: u8,
}

impl Flags {
    pub fn set(&mut self, flag: Flag, value: bool) {
        match flag {
            Flag::Seen => self.seen = value,
            Flag::Flagged => self.flagged = value,
            Flag::Answered => self.answered = value,
            Flag::Deleted => self.deleted = value,
            Flag::Draft => self.draft = value,
            Flag::Color(bit) => {
                let mask = 1 << bit;
                self.color = match value {
                    true => self.color | mask,
                    false => self.color & !mask,
                };
            }
        }
    }

    pub fn from_atoms<'a>(atoms: impl IntoIterator<Item = &'a str>) -> Self {
        let mut flags = Self::default();
        for atom in atoms {
            match atom.to_ascii_lowercase().as_str() {
                "\\seen" => flags.seen = true,
                "\\flagged" => flags.flagged = true,
                "\\answered" => flags.answered = true,
                "\\deleted" => flags.deleted = true,
                "\\draft" => flags.draft = true,
                "$mailflagbit0" => flags.color |= 1,
                "$mailflagbit1" => flags.color |= 2,
                "$mailflagbit2" => flags.color |= 4,
                _ => {}
            }
        }
        flags
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flag {
    Seen,
    Flagged,
    Answered,
    Deleted,
    Draft,
    Color(u8),
}

impl Flag {
    pub const COLORS: [Self; 3] = [Self::Color(0), Self::Color(1), Self::Color(2)];

    pub fn atom(self) -> &'static str {
        match self {
            Self::Seen => "\\Seen",
            Self::Flagged => "\\Flagged",
            Self::Answered => "\\Answered",
            Self::Deleted => "\\Deleted",
            Self::Draft => "\\Draft",
            Self::Color(0) => "$MailFlagBit0",
            Self::Color(1) => "$MailFlagBit1",
            Self::Color(_) => "$MailFlagBit2",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Seen => "seen",
            Self::Flagged => "flagged",
            Self::Answered => "answered",
            Self::Deleted => "deleted",
            Self::Draft => "draft",
            Self::Color(0) => "color0",
            Self::Color(1) => "color1",
            Self::Color(_) => "color2",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [
            Self::Seen,
            Self::Flagged,
            Self::Answered,
            Self::Deleted,
            Self::Draft,
            Self::Color(0),
            Self::Color(1),
            Self::Color(2),
        ]
        .into_iter()
        .find(|flag| flag.name() == name)
    }
}
