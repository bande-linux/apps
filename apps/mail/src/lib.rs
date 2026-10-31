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
pub mod setup;
pub mod smtp;
pub mod store;
pub mod sync;
pub mod ui;
pub mod worker;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-mail.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("ui/compose_view.rs"),
            include_str!("ui/format.rs"),
            include_str!("ui/paint.rs"),
            include_str!("ui/reader.rs"),
            include_str!("ui/setup_sheet.rs"),
            include_str!("ui/sidebar.rs"),
            include_str!("ui/toolbar.rs"),
            include_str!("ui/view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
