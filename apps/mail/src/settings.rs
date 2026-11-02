use std::sync::OnceLock;

use bande_app_settings::{Stored, flag, pairs};
use bande_inbox::Outbox;

use crate::store::AccountId;

pub const MAX_PREVIEW_LINES: u8 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailSettings {
    pub notify: bool,
    pub sound: bool,
    pub load_remote: bool,
    pub preview_lines: u8,
    pub conversations: bool,
    pub plain_text: bool,
    pub fixed_width: bool,
    pub quote: bool,
    pub send_from: String,
}

impl Default for MailSettings {
    fn default() -> Self {
        Self {
            notify: true,
            sound: true,
            load_remote: false,
            preview_lines: 2,
            conversations: true,
            plain_text: false,
            fixed_width: false,
            quote: true,
            send_from: String::new(),
        }
    }
}

impl Stored for MailSettings {
    const FILE: &'static str = "mail.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            match key {
                "notify" => settings.notify = flag(value).unwrap_or(settings.notify),
                "sound" => settings.sound = flag(value).unwrap_or(settings.sound),
                "load-remote" => settings.load_remote = flag(value).unwrap_or(settings.load_remote),
                "preview-lines" => {
                    settings.preview_lines = value
                        .parse()
                        .ok()
                        .filter(|lines| *lines <= MAX_PREVIEW_LINES)
                        .unwrap_or(settings.preview_lines);
                }
                "conversations" => {
                    settings.conversations = flag(value).unwrap_or(settings.conversations);
                }
                "plain-text" => settings.plain_text = flag(value).unwrap_or(settings.plain_text),
                "fixed-width" => {
                    settings.fixed_width = flag(value).unwrap_or(settings.fixed_width);
                }
                "quote" => settings.quote = flag(value).unwrap_or(settings.quote),
                "send-from" => settings.send_from = value.to_owned(),
                _ => {}
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        format!(
            "notify = {}\nsound = {}\nload-remote = {}\npreview-lines = {}\nconversations = {}\nplain-text = {}\nfixed-width = {}\nquote = {}\nsend-from = {}\n",
            self.notify,
            self.sound,
            self.load_remote,
            self.preview_lines,
            self.conversations,
            self.plain_text,
            self.fixed_width,
            self.quote,
            self.send_from.trim(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Settings,
    Accounts,
    Remove(AccountId),
    AddAccount,
}

static CHANGES: OnceLock<Outbox<Change>> = OnceLock::new();

pub fn listen(outbox: Outbox<Change>) {
    if CHANGES.set(outbox).is_err() {
        tracing::warn!("the settings already have a listener");
    }
}

pub fn announce(change: Change) {
    if let Some(outbox) = CHANGES.get() {
        outbox.send(change);
    }
}
