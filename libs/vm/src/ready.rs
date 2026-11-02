use std::path::Path;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fd::OwnedFd;
use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use rustix::process::{Pid, PidfdFlags, pidfd_open};

use crate::Result;

pub(crate) struct CreationWatch {
    inotify: OwnedFd,
}

impl CreationWatch {
    pub(crate) fn new(folder: &Path) -> Result<Self> {
        let inotify = inotify::init(CreateFlags::CLOEXEC)?;
        inotify::add_watch(&inotify, folder, WatchFlags::CREATE | WatchFlags::MOVED_TO)?;
        Ok(Self { inotify })
    }

    pub(crate) fn appears_before_exit(&self, file: &Path, pid: i32) -> Result<bool> {
        let Some(Ok(process)) = Pid::from_raw(pid).map(|pid| pidfd_open(pid, PidfdFlags::empty()))
        else {
            return Ok(file.exists());
        };
        let mut events = [0; 4096];
        loop {
            if file.exists() {
                return Ok(true);
            }
            let mut fds = [
                PollFd::new(&self.inotify, PollFlags::IN),
                PollFd::new(&process, PollFlags::IN),
            ];
            poll(&mut fds, None)?;
            if fds[1].revents().contains(PollFlags::IN) {
                return Ok(file.exists());
            }
            rustix::io::read(&self.inotify, &mut events)?;
        }
    }
}
