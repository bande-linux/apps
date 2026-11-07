pub mod compose;
pub mod discover;
pub mod dns;
pub mod donate;
pub mod error;
pub mod html;
pub mod http;
pub mod imap;
pub mod message;
pub mod net;
pub mod picture;
pub mod portal;
pub mod settings;
pub mod setup;
pub mod smtp;
pub mod store;
pub mod sync;
pub mod ui;
pub mod worker;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub const KEYRING: &str = "org.bande.Mail";

pub fn keyring_key<'a>(email: &'a str, host: &'a str) -> [(&'a str, &'a str); 2] {
    [("account", email), ("server", host)]
}

pub fn keyring_label(email: &str) -> String {
    format!("Mail password for {email}")
}

const ENGLISH: &str = include_str!("../locales/en-US/bande-mail.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
