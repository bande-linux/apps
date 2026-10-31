use std::collections::HashMap;

use bande_toolkit::{Color, Image};

use crate::imap::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Symbol {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Junk,
    Trash,
    Flag,
    AllMail,
    Folder,
    Compose,
    GetMail,
    Reply,
    ReplyAll,
    Forward,
    Attachment,
    Remote,
}

impl Symbol {
    pub(crate) fn svg(self) -> &'static [u8] {
        match self {
            Self::Inbox => include_bytes!("../../data/symbols/inbox.svg"),
            Self::Sent => include_bytes!("../../data/symbols/sent.svg"),
            Self::Drafts => include_bytes!("../../data/symbols/drafts.svg"),
            Self::Archive => include_bytes!("../../data/symbols/archive.svg"),
            Self::Junk => include_bytes!("../../data/symbols/junk.svg"),
            Self::Trash => include_bytes!("../../data/symbols/trash.svg"),
            Self::Flag => include_bytes!("../../data/symbols/flag.svg"),
            Self::AllMail => include_bytes!("../../data/symbols/all-mail.svg"),
            Self::Folder => include_bytes!("../../data/symbols/folder.svg"),
            Self::Compose => include_bytes!("../../data/symbols/compose.svg"),
            Self::GetMail => include_bytes!("../../data/symbols/get-mail.svg"),
            Self::Reply => include_bytes!("../../data/symbols/reply.svg"),
            Self::ReplyAll => include_bytes!("../../data/symbols/reply-all.svg"),
            Self::Forward => include_bytes!("../../data/symbols/forward.svg"),
            Self::Attachment => include_bytes!("../../data/symbols/attachment.svg"),
            Self::Remote => include_bytes!("../../data/symbols/remote.svg"),
        }
    }

    pub fn for_role(role: Option<Role>) -> Self {
        match role {
            Some(Role::Inbox) => Self::Inbox,
            Some(Role::Sent) => Self::Sent,
            Some(Role::Drafts) => Self::Drafts,
            Some(Role::Archive) => Self::Archive,
            Some(Role::Junk) => Self::Junk,
            Some(Role::Trash) => Self::Trash,
            Some(Role::Flagged) => Self::Flag,
            Some(Role::All) => Self::AllMail,
            None => Self::Folder,
        }
    }
}

#[derive(Default)]
pub struct Symbols {
    images: HashMap<(Symbol, u32, [u8; 4]), Option<Image>>,
}

impl Symbols {
    pub fn get(&mut self, symbol: Symbol, pixels: u32, color: Color) -> Option<Image> {
        self.images
            .entry((symbol, pixels, [color.r, color.g, color.b, color.a]))
            .or_insert_with(|| {
                Image::from_svg(symbol.svg(), pixels, pixels).and_then(|image| image.tinted(color))
            })
            .clone()
    }
}
