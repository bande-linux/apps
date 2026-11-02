use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use jiff::Timestamp;
use rustix::io::{Errno, read};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

const DISARMED: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

pub struct WallAlarm {
    timer: OwnedFd,
    armed: Option<Timestamp>,
}

impl WallAlarm {
    pub fn new() -> rustix::io::Result<Self> {
        Ok(Self {
            timer: timerfd_create(
                TimerfdClockId::Realtime,
                TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
            )?,
            armed: None,
        })
    }

    pub fn set(&mut self, at: Option<Timestamp>) {
        if at == self.armed {
            return;
        }
        let value = at.map_or(DISARMED, |at| Timespec {
            tv_sec: at.as_second().max(1),
            tv_nsec: i64::from(at.subsec_nanosecond().max(0)),
        });
        let armed = timerfd_settime(
            &self.timer,
            TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            &Itimerspec {
                it_interval: DISARMED,
                it_value: value,
            },
        );
        match armed {
            Ok(_) => self.armed = at,
            Err(error) => tracing::warn!("cannot set the calendar alarm: {error}"),
        }
    }

    pub fn take(&mut self) {
        let mut expirations = [0; 8];
        loop {
            match read(&self.timer, &mut expirations) {
                Ok(_) | Err(Errno::CANCELED) | Err(Errno::AGAIN) => break,
                Err(Errno::INTR) => continue,
                Err(error) => {
                    tracing::warn!("cannot read the calendar alarm: {error}");
                    break;
                }
            }
        }
        self.armed = None;
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.timer.as_fd()
    }
}
