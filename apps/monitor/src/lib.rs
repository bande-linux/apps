mod apps;
mod cpu;
mod force_quit;
mod format;
mod inhibitors;
mod inspect;
mod inspector;
mod list;
mod menu;
mod model;
mod monitor_view;
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
pub use inhibitors::{Inhibitor, Inhibitors, SleepBlocker, list_sleep_blockers, sleep_blockers};
pub use inspect::{Details, Mapping, OpenFile, Thread, read_details};
pub use inspector::{INSPECTOR_SIZE, InspectorView, Section};
pub use menu::{MenuCommand, Selected, app_menus, force_quit_menu, process_menu};
pub use model::{
    AppList, AppRow, Column, CoreGroup, CoreKind, Filter, Graph, Monitor, Row, RowKey, StopTarget,
    Tab,
};
pub use monitor_view::{MonitorView, TOOLBAR_HEIGHT};
pub use sampler::{GpuUse, Identity, Interface, Process, Rates, Sampler, Snapshot};
pub use signal::{SignalError, Stop, stop};
pub use system::{Memory, Pressure, PressureLevel};
pub use view::{Icons, Request, Screen};

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
            include_str!("inhibitors.rs"),
            include_str!("inspect.rs"),
            include_str!("inspector.rs"),
            include_str!("main.rs"),
            include_str!("menu.rs"),
            include_str!("model.rs"),
            include_str!("monitor_view.rs"),
            include_str!("signal.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
