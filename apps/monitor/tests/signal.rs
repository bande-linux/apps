#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command};

use unbound_monitor::{Sampler, SignalError, Stop, stop};

fn sleeper() -> (Child, u64) {
    let child = Command::new("sleep").arg("60").spawn().expect("sleep");
    let snapshot = Sampler::live().sample().expect("sample");
    let start_ticks = snapshot
        .processes
        .iter()
        .find(|process| process.pid == child.id())
        .expect("the sleeper")
        .start_ticks;
    (child, start_ticks)
}

#[test]
fn quit_and_force_quit_send_term_and_kill_to_the_sampled_process() {
    for (how, signal) in [(Stop::Quit, 15), (Stop::ForceQuit, 9)] {
        let (mut child, start_ticks) = sleeper();
        stop(child.id(), start_ticks, how).expect("signalled");
        assert_eq!(child.wait().expect("exit").signal(), Some(signal));
    }
}

#[test]
fn a_process_with_another_start_time_is_treated_as_already_exited() {
    let (mut child, start_ticks) = sleeper();
    let result = stop(child.id(), start_ticks + 1, Stop::ForceQuit);
    assert!(matches!(result, Err(SignalError::Exited)));
    assert_eq!(child.try_wait().expect("status"), None);
    child.kill().expect("clean up");
    child.wait().expect("reaped");
    assert!(matches!(
        stop(child.id(), start_ticks, Stop::ForceQuit),
        Err(SignalError::Exited)
    ));
}
