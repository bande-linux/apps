use std::process::ExitCode;

use unbound_monitor::{Catalog, Sampler};
use unbound_xdg::{Locale, applications, current_desktops, data_dirs};

fn main() -> ExitCode {
    unbound_log::init("unbound-monitor");
    let catalog = Catalog::new(&applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    ));
    let mut sampler = Sampler::live();
    match sampler.sample() {
        Ok(snapshot) => {
            for group in catalog.group(&snapshot.processes) {
                tracing::info!("{} {:?}", group.name, group.members.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
