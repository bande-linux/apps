pub mod compose;
pub mod discover;
pub mod dns;
pub mod error;
pub mod html;
pub mod http;
pub mod imap;
pub mod message;
pub mod net;
pub mod picture;
pub mod portal;
pub mod secrets;
pub mod settings;
pub mod setup;
pub mod smtp;
pub mod store;
pub mod sync;
pub mod ui;
pub mod worker;

use std::sync::LazyLock;

use bande_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-mail.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
