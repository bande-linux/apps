use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use crate::library::{Library, Machine, autostart_unit_name};
use crate::qemu::{Host, utf8};
use crate::qmp::Qmp;
use crate::unit::Units;
use crate::{Error, Result};

pub const START_FLAG: &str = "--start-machine";
const WANTED_BY: &str = "default.target";

fn quoted(text: &str) -> String {
    let escaped: String = text
        .chars()
        .flat_map(|char| match char {
            '\\' => vec!['\\', '\\'],
            '"' => vec!['\\', '"'],
            '%' => vec!['%', '%'],
            char if char.is_control() => vec![' '],
            char => vec![char],
        })
        .collect();
    format!("\"{escaped}\"")
}

fn description(name: &str) -> String {
    name.chars()
        .map(|char| if char.is_control() { ' ' } else { char })
        .collect::<String>()
        .replace('%', "%%")
}

pub(crate) fn unit_text(name: &str, program: &Path, id: &str) -> Result<String> {
    Ok(format!(
        "[Unit]\nDescription=Start the Velox virtual machine {}\n\n\
         [Service]\nType=oneshot\nExecStart={} {START_FLAG} {}\n\n\
         [Install]\nWantedBy={WANTED_BY}\n",
        description(name),
        quoted(utf8(program)?),
        quoted(id),
    ))
}

impl Machine {
    fn autostart_file(&self) -> PathBuf {
        self.dir().join(self.autostart_unit())
    }

    pub fn starts_at_login(&self) -> bool {
        self.autostart_file().is_file()
    }

    pub fn autostart_unit(&self) -> String {
        autostart_unit_name(&self.manifest.id)
    }
}

impl Units {
    pub fn set_starts_at_login(&self, machine: &Machine, program: &Path, on: bool) -> Result<()> {
        if !on {
            return self.remove_autostart(machine);
        }
        let path = machine.autostart_file();
        let text = unit_text(&machine.manifest.name, program, &machine.manifest.id)?;
        bande_xdg::replace_file(&path, text)?;
        self.enable(utf8(&path)?)?;
        self.reload()
    }

    pub fn remove_autostart(&self, machine: &Machine) -> Result<()> {
        let path = machine.autostart_file();
        if !path.is_file() {
            return Ok(());
        }
        self.disable(&machine.autostart_unit())?;
        match fs::remove_file(&path) {
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        self.reload()
    }

    pub fn boot(&self, host: &Host, machine: &Machine) -> Result<(Qmp, bool)> {
        let mut started = self.start(host, machine).and_then(|mut qmp| {
            machine.resume_snapshot(&mut qmp)?;
            Ok(qmp)
        });
        let mut without_3d = false;
        if started.is_err() && host.uses_3d(machine) && machine.manifest.resume_from.is_none() {
            let flat = Host {
                allow_3d: false,
                ..host.clone()
            };
            started = self.start(&flat, machine);
            without_3d = started.is_ok();
        }
        if machine.manifest.resume_from.is_some() {
            let mut resumed = find(&Library::open()?, &machine.manifest.id)?;
            resumed.manifest.resume_from = None;
            resumed.save()?;
        }
        Ok((started?, without_3d))
    }
}

fn find(library: &Library, id: &str) -> Result<Machine> {
    library
        .machines()?
        .into_iter()
        .find(|machine| machine.manifest.id == id)
        .ok_or_else(|| Error::Io(io::Error::new(ErrorKind::NotFound, format!("no machine {id}"))))
}

pub fn start_at_login(id: &str) -> Result<()> {
    let machine = find(&Library::open()?, id)?;
    let units = Units::connect()?;
    if units.state(&machine.unit())?.is_running() {
        return Ok(());
    }
    units.boot(&Host::detect()?, &machine).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_units_quote_what_systemd_would_expand() {
        let text = unit_text("web 100%\nx", Path::new("/opt/my apps/velox"), "abc");
        assert_eq!(
            text.ok().as_deref(),
            Some(
                "[Unit]\nDescription=Start the Velox virtual machine web 100%% x\n\n\
                 [Service]\nType=oneshot\nExecStart=\"/opt/my apps/velox\" --start-machine \"abc\"\n\n\
                 [Install]\nWantedBy=default.target\n"
            )
        );
    }
}
