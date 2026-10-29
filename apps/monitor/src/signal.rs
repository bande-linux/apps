use std::fs;
use std::io;

use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};

use crate::proc::parse_stat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Quit,
    ForceQuit,
}

#[derive(Debug)]
pub enum SignalError {
    PermissionDenied,
    Exited,
    Failed(io::Error),
}

impl From<Errno> for SignalError {
    fn from(errno: Errno) -> Self {
        match errno {
            Errno::PERM | Errno::ACCESS => Self::PermissionDenied,
            Errno::SRCH => Self::Exited,
            other => Self::Failed(other.into()),
        }
    }
}

pub fn stop(pid: u32, start_ticks: u64, stop: Stop) -> Result<(), SignalError> {
    let raw = i32::try_from(pid).map_err(|_| SignalError::Exited)?;
    let pid = Pid::from_raw(raw).ok_or(SignalError::Exited)?;
    let pidfd = pidfd_open(pid, PidfdFlags::empty())?;
    let stat = fs::read_to_string(format!("/proc/{raw}/stat")).map_err(|_| SignalError::Exited)?;
    if parse_stat(&stat).is_none_or(|stat| stat.start_ticks != start_ticks) {
        return Err(SignalError::Exited);
    }
    let signal = match stop {
        Stop::Quit => Signal::TERM,
        Stop::ForceQuit => Signal::KILL,
    };
    pidfd_send_signal(&pidfd, signal)?;
    Ok(())
}
