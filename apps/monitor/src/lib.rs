mod apps;
mod cpu;
mod force_quit;
mod format;
mod menu;
mod model;
mod proc;
mod sampler;
mod signal;
mod system;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use apps::{AppInfo, Catalog, Group, GroupKey};
pub use cpu::{CpuShare, read_cpu_ticks};
pub use force_quit::ForceQuitView;
pub use format::{format_bytes, format_cpu_time, format_rate};
pub use menu::{MenuCommand, Selected, force_quit_menu, process_menu};
pub use model::{
    AppList, AppRow, Column, Disclosure, Graph, Monitor, Row, RowKey, StopTarget, Tab,
};
pub use sampler::{Identity, Interface, Process, Rates, Sampler, Snapshot};
pub use signal::{SignalError, Stop, stop};
pub use system::Memory;
pub use view::{Icons, MonitorView, ROW_HEIGHT, Request, Screen};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-monitor.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("apps.rs"),
            include_str!("force_quit.rs"),
            include_str!("format.rs"),
            include_str!("main.rs"),
            include_str!("menu.rs"),
            include_str!("model.rs"),
            include_str!("signal.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
