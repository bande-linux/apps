use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::time::Instant;


use bande_l10n::{text, tr};

use crate::settings::ArchiveSettings;
use bande_fs::{
    Change, Job, Outcome, Progress, Refusal, Report, Task, Trash, archive_path, format_size,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Expand {
        archive: PathBuf,
        password: Option<String>,
    },
    Compress {
        sources: Vec<PathBuf>,
        archive: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Done(Vec<PathBuf>),
    Password { archive: PathBuf, wrong: bool },
    Failed(String),
    Cancelled,
}

pub struct Work {
    pub kind: Kind,
    pub progress: Progress,
    pub started: Instant,
    job: Job,
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn trash() -> io::Result<Trash> {
    Trash::for_user().ok_or_else(|| io::Error::other("there is no home folder"))
}

impl Work {
    fn start(kind: Kind, task: Task) -> io::Result<Self> {
        Ok(Self {
            kind,
            progress: Progress::default(),
            started: Instant::now(),
            job: Job::start(task, trash()?, false)?,
        })
    }

    pub fn expand(
        archive: PathBuf,
        password: Option<String>,
        settings: &ArchiveSettings,
    ) -> io::Result<Self> {
        let task = Task::Extract {
            archives: vec![archive.clone()],
            into: settings.expand_into.clone(),
            password: password.clone(),
        };
        Self::start(Kind::Expand { archive, password }, task)
    }

    pub fn compress(sources: Vec<PathBuf>, settings: &ArchiveSettings) -> io::Result<Self> {
        let folder = settings
            .archive_into
            .clone()
            .or_else(|| sources.first()?.parent().map(Path::to_path_buf))
            .ok_or_else(|| io::Error::other("nothing to compress"))?;
        let archive = archive_path(&sources, &folder, settings.format)
            .ok_or_else(|| io::Error::other("the items have no name"))?;
        let task = Task::Compress {
            sources: sources.clone(),
            archive: archive.clone(),
        };
        Self::start(Kind::Compress { sources, archive }, task)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.job.fd()
    }

    pub fn cancel(&self) {
        self.job.cancel();
    }

    pub fn wait(&self) -> Option<Ended> {
        match self.job.wait() {
            Report::Finished(outcome) => Some(self.ended(outcome)),
            _ => None,
        }
    }

    pub fn pump(&mut self) -> Option<Ended> {
        let mut ended = None;
        for report in self.job.take_reports() {
            match report {
                Report::Progress(progress) => self.progress = progress,
                Report::Finished(outcome) => ended = Some(self.ended(outcome)),
                Report::Conflict(_) | Report::Failure(_) => {}
            }
        }
        ended
    }

    fn ended(&self, outcome: Outcome) -> Ended {
        if outcome.cancelled {
            return Ended::Cancelled;
        }
        if let Some(shortfall) = outcome.shortfall {
            return Ended::Failed(tr!(
                "{size} more free space is needed.",
                size = format_size(shortfall.needed)
            ));
        }
        if let Some(failure) = outcome.failures.first() {
            return match (failure.refusal(), &self.kind) {
                (Some(Refusal::NeedsPassword), Kind::Expand { archive, .. }) => Ended::Password {
                    archive: archive.clone(),
                    wrong: false,
                },
                (Some(Refusal::WrongPassword), Kind::Expand { archive, .. }) => Ended::Password {
                    archive: archive.clone(),
                    wrong: true,
                },
                _ => Ended::Failed(failure.error.to_string()),
            };
        }
        Ended::Done(
            outcome
                .changes
                .into_iter()
                .filter_map(|change| match change {
                    Change::Created(path) => Some(path),
                    _ => None,
                })
                .collect(),
        )
    }

    pub fn subject(&self) -> String {
        match &self.kind {
            Kind::Expand { archive, .. } => name_of(archive),
            Kind::Compress { sources, archive } => match sources.as_slice() {
                [only] => name_of(only),
                _ => name_of(archive),
            },
        }
    }

    pub fn title(&self) -> String {
        let id = match self.kind {
            Kind::Expand { .. } => text!("Expanding “{name}”"),
            Kind::Compress { .. } => text!("Compressing “{name}”"),
        };
        id.tr_args(&[("name", &self.subject() as &dyn std::fmt::Display)])
    }

    pub fn failure_title(&self) -> String {
        let id = match self.kind {
            Kind::Expand { .. } => text!("Unable to expand “{name}”"),
            Kind::Compress { .. } => text!("Unable to compress “{name}”"),
        };
        id.tr_args(&[("name", &self.subject() as &dyn std::fmt::Display)])
    }

    pub fn detail(&self) -> String {
        let progress = &self.progress;
        tr!(
            "{done} of {total}",
            done = format_size(progress.bytes_done),
            total = format_size(progress.bytes_total)
        )
    }

    pub fn fraction(&self) -> Option<f32> {
        let progress = &self.progress;
        (progress.bytes_total > 0)
            .then(|| (progress.bytes_done as f64 / progress.bytes_total as f64).min(1.0) as f32)
    }

    pub fn originals(&self) -> Vec<PathBuf> {
        match &self.kind {
            Kind::Expand { archive, .. } => vec![archive.clone()],
            Kind::Compress { sources, .. } => sources.clone(),
        }
    }
}
