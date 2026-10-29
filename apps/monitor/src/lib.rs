mod apps;
mod cpu;
mod format;
mod model;
mod proc;
mod sampler;
mod signal;
mod system;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use apps::{AppInfo, Catalog, Group, GroupKey};
pub use cpu::{CpuShare, CpuTicks, read_cpu_ticks};
pub use format::{format_bytes, format_cpu_time, format_rate};
pub use model::{Column, Disclosure, Graph, HISTORY, Monitor, Row, RowKey, StopTarget, Tab};
pub use sampler::{Identity, Interface, Process, Rates, Sampler, Snapshot};
pub use signal::{SignalError, Stop, stop};
pub use system::Memory;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-monitor.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("apps.rs"),
            include_str!("format.rs"),
            include_str!("main.rs"),
            include_str!("model.rs"),
            include_str!("signal.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
