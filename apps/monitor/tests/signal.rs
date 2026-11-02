#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

use bande_monitor::{Sampler, SignalError, Stop, stop};

#[test]
fn quit_and_force_quit_signal_the_sampled_process_and_never_a_reused_pid() {
    for (how, signal) in [(Stop::Quit, 15), (Stop::ForceQuit, 9)] {
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let snapshot = Sampler::live().sample().unwrap();
        let sampled = snapshot.processes.iter().find(|p| p.pid == child.id());
        let start_ticks = sampled.unwrap().start_ticks;
        let reused = stop(child.id(), start_ticks + 1, how);
        assert!(matches!(reused, Err(SignalError::Exited)));
        assert_eq!(child.try_wait().unwrap(), None);
        stop(child.id(), start_ticks, how).unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(signal));
        let gone = stop(child.id(), start_ticks, how);
        assert!(matches!(gone, Err(SignalError::Exited)));
    }
}
