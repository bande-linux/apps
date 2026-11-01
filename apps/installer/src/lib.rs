mod deb;
mod flow;
mod view;
mod worker;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use deb::{Deb, Machine, compare, missing_requirements, read};
pub use flow::{Action, Preview, Problem, State, apps};
pub use view::{Installer, WINDOW_SIZE};

const ENGLISH: &str = include_str!("../locales/en-US/bande-installer.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_the_installer_uses_is_translated() {
        let sources = [include_str!("view.rs"), include_str!("main.rs")];
        assert_eq!(
            bande_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
