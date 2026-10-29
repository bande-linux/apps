use std::process::ExitCode;
use std::time::Instant;

use unbound_monitor::Sampler;

fn main() -> ExitCode {
    unbound_log::init("unbound-monitor");
    let mut sampler = Sampler::live();
    for _ in 0..3 {
        let started = Instant::now();
        match sampler.sample() {
            Ok(snapshot) => tracing::info!(
                "{} processes in {:?}",
                snapshot.processes.len(),
                started.elapsed()
            ),
            Err(error) => {
                tracing::error!("{error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
