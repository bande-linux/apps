use std::collections::HashMap;
use std::fs;
use std::path::Path;

use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{MatchRule, names::ErrorName};

use crate::library::Machine;
use crate::manifest::Preset;
use crate::qemu::{Host, utf8};
use crate::qmp::Qmp;
use crate::ready::CreationWatch;
use crate::{Error, Result};

const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const UNIT: &str = "org.freedesktop.systemd1.Unit";
const SERVICE: &str = "org.freedesktop.systemd1.Service";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";
const SIGKILL: i32 = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveState {
    Active,
    Activating,
    Deactivating,
    Inactive,
    Failed,
}

pub struct Units {
    bus: Connection,
}

pub struct UnitEvents {
    messages: MessageIterator,
    last: ActiveState,
}

impl ActiveState {
    fn parse(name: &str) -> Self {
        match name {
            "active" | "reloading" | "refreshing" => Self::Active,
            "activating" => Self::Activating,
            "deactivating" => Self::Deactivating,
            "failed" => Self::Failed,
            _ => Self::Inactive,
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, Self::Active | Self::Activating | Self::Deactivating)
    }
}

impl Units {
    pub fn connect() -> Result<Self> {
        let bus = Connection::session()?;
        bus.call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), "Subscribe", &())?;
        Ok(Self { bus })
    }

    pub fn start(&self, host: &Host, machine: &Machine) -> Result<Qmp> {
        let unit = machine.unit();
        if self.state(&unit)?.is_running() {
            return Err(Error::Unit {
                unit,
                result: "already running".into(),
            });
        }
        machine.reset_runtime()?;
        let manifest = &machine.manifest;
        let network = machine.network_unit();
        let network_properties: Vec<(&str, Value)> = vec![
            (
                "Description",
                Value::from(format!("Velox network for {}", manifest.name)),
            ),
            ("Type", Value::from("forking")),
            (
                "PIDFile",
                Value::from(utf8(&machine.passt_pid_file())?.to_owned()),
            ),
            ("ExecStart", exec(host.passt_command(machine)?)),
            ("StopWhenUnneeded", Value::from(true)),
            ("CollectMode", Value::from("inactive-or-failed")),
            ("NoNewPrivileges", Value::from(true)),
        ];
        let properties: Vec<(&str, Value)> = vec![
            (
                "Description",
                Value::from(format!("Velox virtual machine {}", manifest.name)),
            ),
            ("Type", Value::from("exec")),
            ("ExecStart", exec(host.qemu_command(machine)?)),
            ("Requires", Value::from(vec![network.clone()])),
            ("After", Value::from(vec![network.clone()])),
            ("CollectMode", Value::from("inactive-or-failed")),
            ("NoNewPrivileges", Value::from(true)),
            ("MemoryMax", Value::from(host.memory_max(machine))),
            (
                "CPUQuotaPerSecUSec",
                Value::from(u64::from(manifest.cpus) * 1_000_000),
            ),
        ];
        let watch = CreationWatch::new(machine.runtime_dir())?;
        let helpers = self.start_helpers(host, machine, &watch)?;
        let started = self.run_job(
            &unit,
            "StartTransientUnit",
            &(
                &unit,
                "fail",
                properties,
                vec![(&network, network_properties)],
            ),
        );
        if let Err(error) = started {
            for helper in helpers {
                let _ = self.stop(&helper);
            }
            return Err(error);
        }
        let pid = self.main_pid(&unit)?;
        let exited = || Error::Unit {
            unit: unit.clone(),
            result: "QEMU exited while starting".into(),
        };
        if !watch.appears_before_exit(&machine.qmp_listening_marker(), pid)? {
            return Err(exited());
        }
        Qmp::connect(&machine.qmp_socket()).map_err(|_| exited())
    }

    fn start_helpers(
        &self,
        host: &Host,
        machine: &Machine,
        watch: &CreationWatch,
    ) -> Result<Vec<String>> {
        let mut helpers = Vec::new();
        if machine.manifest.preset == Preset::Windows {
            fs::create_dir_all(machine.tpm_state())?;
            let unit = machine.tpm_unit();
            self.start_helper(
                &unit,
                host.tpm_command(machine)?,
                machine,
                &machine.tpm_socket(),
                watch,
            )?;
            helpers.push(unit);
        }
        for index in 0..machine.manifest.shares.len() {
            let unit = machine.share_unit(index);
            self.start_helper(
                &unit,
                host.share_command(machine, index)?,
                machine,
                &machine.share_socket(index),
                watch,
            )?;
            helpers.push(unit);
        }
        Ok(helpers)
    }

    fn start_helper(
        &self,
        unit: &str,
        argv: Vec<String>,
        machine: &Machine,
        socket: &Path,
        watch: &CreationWatch,
    ) -> Result<()> {
        let properties: Vec<(&str, Value)> = vec![
            (
                "Description",
                Value::from(format!("Velox helper for {}", machine.manifest.name)),
            ),
            ("Type", Value::from("exec")),
            ("ExecStart", exec(argv)),
            ("CollectMode", Value::from("inactive-or-failed")),
            ("NoNewPrivileges", Value::from(true)),
        ];
        let none: Vec<(&str, Vec<(&str, Value)>)> = Vec::new();
        self.run_job(
            unit,
            "StartTransientUnit",
            &(unit, "fail", properties, none),
        )?;
        let pid = self.main_pid(unit)?;
        if !watch.appears_before_exit(socket, pid)? {
            return Err(Error::Unit {
                unit: unit.to_owned(),
                result: "the helper exited while starting".into(),
            });
        }
        Ok(())
    }

    pub fn stop(&self, unit: &str) -> Result<()> {
        self.manager("StopUnit", &(unit, "replace"))
    }

    pub fn kill(&self, unit: &str) -> Result<()> {
        self.manager("KillUnit", &(unit, "all", SIGKILL))
    }

    pub fn state(&self, unit: &str) -> Result<ActiveState> {
        let reply =
            self.bus
                .call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), "GetUnit", &unit);
        let path: OwnedObjectPath = match reply {
            Err(zbus::Error::MethodError(name, _, _))
                if name == ErrorName::from_static_str_unchecked(NO_SUCH_UNIT) =>
            {
                return Ok(ActiveState::Inactive);
            }
            reply => reply?.body().deserialize()?,
        };
        let state: OwnedValue = self
            .bus
            .call_method(
                Some(SYSTEMD),
                &path,
                Some(PROPERTIES),
                "Get",
                &(UNIT, "ActiveState"),
            )?
            .body()
            .deserialize()?;
        let state: String = state.try_into().map_err(zbus::Error::from)?;
        Ok(ActiveState::parse(&state))
    }

    pub fn watch(&self, unit: &str) -> Result<(ActiveState, UnitEvents)> {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(SYSTEMD)?
            .interface(PROPERTIES)?
            .member("PropertiesChanged")?
            .path(unit_path(unit))?
            .build();
        let messages = MessageIterator::for_match_rule(rule, &self.bus, None)?;
        let state = self.state(unit)?;
        Ok((
            state,
            UnitEvents {
                messages,
                last: state,
            },
        ))
    }

    fn run_job<B>(&self, unit: &str, method: &str, body: &B) -> Result<()>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(SYSTEMD)?
            .interface(MANAGER)?
            .member("JobRemoved")?
            .build();
        let jobs = MessageIterator::for_match_rule(rule, &self.bus, None)?;
        let job: OwnedObjectPath = self
            .bus
            .call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), method, body)?
            .body()
            .deserialize()?;
        for message in jobs {
            let (_, path, _, result): (u32, OwnedObjectPath, String, String) =
                message?.body().deserialize()?;
            if path == job {
                return match result.as_str() {
                    "done" => Ok(()),
                    _ => Err(Error::Unit {
                        unit: unit.to_owned(),
                        result,
                    }),
                };
            }
        }
        Err(Error::Protocol("the session bus closed".into()))
    }

    fn main_pid(&self, unit: &str) -> Result<i32> {
        let path: OwnedObjectPath = self
            .bus
            .call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), "GetUnit", &unit)?
            .body()
            .deserialize()?;
        let pid: OwnedValue = self
            .bus
            .call_method(
                Some(SYSTEMD),
                &path,
                Some(PROPERTIES),
                "Get",
                &(SERVICE, "MainPID"),
            )?
            .body()
            .deserialize()?;
        let pid: u32 = pid.try_into().map_err(zbus::Error::from)?;
        i32::try_from(pid).map_err(|_| Error::Protocol(format!("{unit} has no main process")))
    }

    fn manager<B>(&self, method: &str, body: &B) -> Result<()>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.bus
            .call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), method, body)?;
        Ok(())
    }
}

impl Iterator for UnitEvents {
    type Item = ActiveState;

    fn next(&mut self) -> Option<ActiveState> {
        let last = &mut self.last;
        self.messages.by_ref().flatten().find_map(|message| {
            let (interface, changed, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
                message.body().deserialize().ok()?;
            let state =
                ActiveState::parse(changed.get("ActiveState")?.downcast_ref::<&str>().ok()?);
            (interface == UNIT && state != *last).then(|| {
                *last = state;
                state
            })
        })
    }
}

fn exec(argv: Vec<String>) -> Value<'static> {
    let program = argv.first().cloned().unwrap_or_default();
    Value::from(vec![(program, argv, false)])
}

fn unit_path(unit: &str) -> String {
    let escaped: String = unit
        .bytes()
        .enumerate()
        .map(|(index, byte)| match byte {
            b'a'..=b'z' | b'A'..=b'Z' => (byte as char).to_string(),
            b'0'..=b'9' if index > 0 => (byte as char).to_string(),
            _ => format!("_{byte:02x}"),
        })
        .collect();
    format!("{SYSTEMD_PATH}/unit/{escaped}")
}
