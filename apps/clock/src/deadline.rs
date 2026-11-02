use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::Duration;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeline {
    Wall,
    Boot,
}

pub struct Deadline {
    timer: OwnedFd,
    clock: Timeline,
    wakes_system: bool,
    armed: Option<Timespec>,
}

fn create(clock: TimerfdClockId) -> rustix::io::Result<OwnedFd> {
    timerfd_create(clock, TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK)
}

impl Deadline {
    pub fn new(clock: Timeline) -> rustix::io::Result<Self> {
        let (waking, plain) = match clock {
            Timeline::Wall => (TimerfdClockId::RealtimeAlarm, TimerfdClockId::Realtime),
            Timeline::Boot => (TimerfdClockId::BoottimeAlarm, TimerfdClockId::Boottime),
        };
        let (timer, wakes_system) = match create(waking) {
            Ok(timer) => (timer, true),
            Err(error) => {
                tracing::debug!("no {clock:?} wake alarm ({error}), the timer waits for a resume");
                (create(plain)?, false)
            }
        };
        Ok(Self {
            timer,
            clock,
            wakes_system,
            armed: None,
        })
    }

    pub fn wakes_system(&self) -> bool {
        self.wakes_system
    }

    pub fn set_wall(&mut self, at: Option<Timestamp>) {
        self.set(at.map(|at| Timespec {
            tv_sec: at.as_second().max(1),
            tv_nsec: i64::from(at.subsec_nanosecond().max(0)),
        }));
    }

    pub fn set_boot(&mut self, at: Option<Duration>) {
        self.set(at.map(|at| Timespec {
            tv_sec: i64::try_from(at.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(at.subsec_nanos()).max(1),
        }));
    }

    fn set(&mut self, at: Option<Timespec>) {
        if at == self.armed {
            return;
        }
        let flags = match self.clock {
            Timeline::Wall => TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            Timeline::Boot => TimerfdTimerFlags::ABSTIME,
        };
        let armed = timerfd_settime(
            &self.timer,
            flags,
            &Itimerspec {
                it_interval: DISARMED,
                it_value: at.unwrap_or(DISARMED),
            },
        );
        match armed {
            Ok(_) => self.armed = at,
            Err(error) => tracing::warn!("cannot set the {:?} timer: {error}", self.clock),
        }
    }

    pub fn take(&mut self) {
        let mut expirations = [0; 8];
        loop {
            match read(&self.timer, &mut expirations) {
                Ok(_) | Err(Errno::CANCELED) | Err(Errno::AGAIN) => break,
                Err(Errno::INTR) => continue,
                Err(error) => {
                    tracing::warn!("cannot read the {:?} timer: {error}", self.clock);
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
