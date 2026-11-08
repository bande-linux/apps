use std::net::TcpListener;
use std::path::PathBuf;
use std::rc::Rc;

use bande_l10n::{Text, text, tr};
use bande_toolkit::ui::{
    Alert, Button, Form, FormLine, Label, PopUpButton, RadioGroup, SecureField, Stepper, TextField,
    Widget,
};
use bande_vm::{CloudInit, Forward, Host, Installer, NewMachine, Preset, catalog};

use super::backend::{Login, Plan, Source};
use super::fleet::Fleet;
use super::{Intent, Intents, push};

pub const CONTINUE: &str = "continue";
pub const BACK: &str = "back";
pub const CANCEL: &str = "cancel";
const FIRST_PORT: u16 = 2222;
const LOWEST_PORT: u16 = 1024;
const SSH_PORT: u16 = 22;
const DEFAULT_CPUS: u32 = 4;
const DEFAULT_MEMORY_GIB: u64 = 4;
const DEFAULT_DISK_GIB: u64 = 32;
const DISK_RANGE: (f64, f64) = (8.0, 1024.0);
const DISK_STEP: f64 = 8.0;
const NAME_LIMIT: usize = 63;
const GUEST_TOOLS: [&str; 2] = ["qemu-guest-agent", "spice-vdagent"];
const FIELD_TEXT: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Image,
    Installer,
    Resources,
    Identity,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Image(usize),
    Cpus(f64),
    Memory(f64),
    Disk(f64),
    Name(String),
    User(String),
    Key(usize),
    Port(String),
    Preset(usize),
    Password(String),
}

pub struct Draft {
    pub step: Step,
    image: usize,
    cpus: u32,
    memory_gib: u64,
    disk_gib: u64,
    name: String,
    named: bool,
    user: String,
    keys: Vec<PathBuf>,
    key: usize,
    port: String,
    limits: (u32, u64),
    iso: Option<PathBuf>,
    preset: Preset,
    password: String,
}

fn host_limits() -> (u32, u64) {
    let cpus = std::thread::available_parallelism().map_or(1, |cpus| cpus.get() as u32);
    let memory_kib = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find_map(|line| line.strip_prefix("MemTotal:"))
                .and_then(|value| {
                    value
                        .trim()
                        .trim_end_matches("kB")
                        .trim()
                        .parse::<u64>()
                        .ok()
                })
        })
        .unwrap_or(0);
    (cpus, (memory_kib / 1024 / 1024).saturating_sub(1).max(1))
}

fn ssh_keys() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut keys: Vec<PathBuf> = std::fs::read_dir(home.join(".ssh"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "pub"))
                .collect()
        })
        .unwrap_or_default();
    keys.sort_by_key(|key| (!key.ends_with("id_ed25519.pub"), key.clone()));
    keys
}

fn port_taken(fleet: &Fleet, port: u16) -> bool {
    fleet.entries.values().any(|entry| {
        entry
            .listed
            .manifest
            .forwards
            .iter()
            .any(|forward| forward.host == port)
    })
}

pub fn port_free(fleet: &Fleet, port: u16) -> bool {
    !port_taken(fleet, port) && TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_LIMIT
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_user(user: &str) -> bool {
    user != "root"
        && user
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first == b'_')
        && user.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

impl Draft {
    pub fn new(fleet: &Fleet) -> Self {
        let limits = host_limits();
        let port = (FIRST_PORT..u16::MAX)
            .find(|port| port_free(fleet, *port))
            .unwrap_or(FIRST_PORT);
        let user = std::env::var("USER")
            .ok()
            .filter(|user| valid_user(user))
            .unwrap_or_default();
        Self {
            step: Step::Image,
            image: 0,
            cpus: DEFAULT_CPUS.min(limits.0),
            memory_gib: DEFAULT_MEMORY_GIB.min(limits.1),
            disk_gib: DEFAULT_DISK_GIB,
            name: String::new(),
            named: false,
            user,
            keys: ssh_keys(),
            key: 0,
            port: port.to_string(),
            limits,
            iso: None,
            preset: Preset::Linux,
            password: String::new(),
        }
    }

    fn installs_from_iso(&self) -> bool {
        self.image >= catalog().len()
    }

    fn imports_disk(&self) -> bool {
        self.image == catalog().len() + 1
    }

    pub fn set_iso(&mut self, path: PathBuf) {
        self.iso = Some(path);
    }

    fn os(&self) -> String {
        match (self.installs_from_iso(), self.preset) {
            (true, Preset::Windows) => tr!("Windows 11"),
            (true, Preset::Linux) => tr!("Linux"),
            (false, _) => catalog()
                .get(self.image)
                .map(|image| image.os.to_owned())
                .unwrap_or_default(),
        }
    }

    fn default_name(&self, fleet: &Fleet) -> String {
        let base: String = self
            .os()
            .split_whitespace()
            .next()
            .unwrap_or("machine")
            .to_lowercase();
        let taken = |name: &str| fleet.entries.values().any(|entry| entry.name() == name);
        (1..)
            .map(|index| match index {
                1 => base.clone(),
                index => format!("{base}-{index}"),
            })
            .find(|name| !taken(name))
            .unwrap_or(base)
    }

    pub fn set(&mut self, field: Field) {
        match field {
            Field::Image(image) => self.image = image,
            Field::Cpus(cpus) => self.cpus = cpus as u32,
            Field::Memory(memory) => self.memory_gib = memory as u64,
            Field::Disk(disk) => self.disk_gib = disk as u64,
            Field::Name(name) => {
                self.name = name;
                self.named = true;
            }
            Field::User(user) => self.user = user,
            Field::Key(key) => self.key = key,
            Field::Port(port) => self.port = port,
            Field::Password(password) => self.password = password,
            Field::Preset(index) => {
                self.preset = match index {
                    1 => Preset::Windows,
                    _ => Preset::Linux,
                };
            }
        }
    }

    pub fn advance(&mut self, fleet: &Fleet) {
        self.step = match self.step {
            Step::Image if self.installs_from_iso() => Step::Installer,
            Step::Image | Step::Installer => Step::Resources,
            _ => Step::Identity,
        };
        if self.step == Step::Identity && !self.named {
            self.name = self.default_name(fleet);
        }
    }

    pub fn back(&mut self) {
        self.step = match self.step {
            Step::Identity => Step::Resources,
            Step::Resources if self.installs_from_iso() => Step::Installer,
            _ => Step::Image,
        };
    }

    pub fn alert(&self, intents: &Intents, error: Option<&str>) -> Alert {
        let choice = Rc::clone(intents);
        let field = |make: fn(String) -> Field| {
            let intents = Rc::clone(intents);
            move |text: &str| push(&intents, Intent::Field(make(text.to_owned())))
        };
        let number = |make: fn(f64) -> Field| {
            let intents = Rc::clone(intents);
            move |value: f64| push(&intents, Intent::Field(make(value)))
        };
        let index = |make: fn(usize) -> Field| {
            let intents = Rc::clone(intents);
            move |value: usize| push(&intents, Intent::Field(make(value)))
        };
        let new = |key: Text| {
            let choice = Rc::clone(&choice);
            Alert::new(
                &tr!("New Virtual Machine"),
                &error.map_or_else(|| key.tr(), str::to_owned),
                move |action| push(&choice, Intent::Sheet(action.to_owned())),
            )
        };
        let alert = match self.step {
            Step::Image => {
                let iso = tr!("Install from a disc image…");
                let import = tr!("Import a disk image…");
                let mut names: Vec<&str> = catalog().iter().map(|image| image.os).collect();
                names.push(&iso);
                names.push(&import);
                new(text!("Choose what the machine starts from.")).accessory(RadioGroup::new(
                    &tr!("Cloud image"),
                    &names,
                    Some(self.image),
                    index(Field::Image),
                ))
            }
            Step::Installer => {
                let chooser = Rc::clone(intents);
                let disk = self.imports_disk();
                let presets = [tr!("Linux"), tr!("Windows 11")];
                let chosen = self.iso.as_ref().map_or_else(
                    || match disk {
                        true => tr!("No disk image chosen"),
                        false => tr!("No disc image chosen"),
                    },
                    |path| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    },
                );
                let mut lines = vec![
                    FormLine::Row(
                        match disk {
                            true => tr!("Disk image"),
                            false => tr!("Disc image"),
                        },
                        Box::new(Label::new(&chosen)) as Box<dyn Widget>,
                    ),
                    FormLine::Row(
                        String::new(),
                        Box::new(Button::new(
                            &match disk {
                                true => tr!("Choose Disk Image…"),
                                false => tr!("Choose Disc Image…"),
                            },
                            move || push(&chooser, Intent::ChooseIso(disk)),
                        )),
                    ),
                    FormLine::Row(
                        tr!("System"),
                        Box::new(PopUpButton::new(
                            &tr!("System"),
                            &presets,
                            usize::from(self.preset == Preset::Windows),
                            index(Field::Preset),
                        )),
                    ),
                ];
                if self.preset == Preset::Windows && Host::swtpm().is_none() {
                    lines.push(FormLine::Row(
                        String::new(),
                        Box::new(
                            Label::new(&tr!("Windows 11 needs a TPM: sudo apt install swtpm"))
                                .secondary(),
                        ),
                    ));
                }
                new(match disk {
                    true => text!("Choose the disk image to import. Velox copies it."),
                    false => text!("Choose the disc image to install from."),
                })
                .accessory(Form::new(
                    &tr!("Choose the disc image to install from."),
                    lines,
                ))
            }
            Step::Resources => {
                let gigabytes = |value: f64| tr!("{count} GB", count = value);
                let lines = vec![
                    FormLine::Row(
                        tr!("CPUs"),
                        Box::new(Stepper::new(
                            &tr!("CPUs"),
                            f64::from(self.cpus),
                            (1.0, f64::from(self.limits.0)),
                            1.0,
                            number(Field::Cpus),
                        )),
                    ),
                    FormLine::Row(
                        tr!("Memory"),
                        Box::new(
                            Stepper::new(
                                &tr!("Memory"),
                                self.memory_gib as f64,
                                (1.0, self.limits.1 as f64),
                                1.0,
                                number(Field::Memory),
                            )
                            .format(gigabytes),
                        ),
                    ),
                    FormLine::Row(
                        tr!("Disk"),
                        Box::new(
                            Stepper::new(
                                &tr!("Disk"),
                                self.disk_gib as f64,
                                DISK_RANGE,
                                DISK_STEP,
                                number(Field::Disk),
                            )
                            .format(gigabytes),
                        ),
                    ),
                ];
                new(text!("Choose the machine’s processors, memory and disk.")).accessory(
                    Form::new(
                        &tr!("Choose the machine’s processors, memory and disk."),
                        lines,
                    ),
                )
            }
            Step::Identity => {
                let key: Box<dyn Widget> = match self.keys.is_empty() {
                    true => Box::new(
                        Label::new(&tr!("No SSH key in ~/.ssh. Make one with ssh-keygen."))
                            .secondary(),
                    ),
                    false => {
                        let names: Vec<String> = self
                            .keys
                            .iter()
                            .map(|key| {
                                key.file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_default()
                            })
                            .collect();
                        Box::new(PopUpButton::new(
                            &tr!("SSH key"),
                            &names,
                            self.key,
                            index(Field::Key),
                        ))
                    }
                };
                if self.installs_from_iso() {
                    let lines = vec![FormLine::Row(
                        tr!("Name"),
                        Box::new(TextField::new(&tr!("Name"), &self.name, field(Field::Name)))
                            as Box<dyn Widget>,
                    )];
                    return new(text!("Name the machine."))
                        .accessory(Form::new(&tr!("Name the machine."), lines))
                        .other(&tr!("Back"), BACK)
                        .cancel(&tr!("Cancel"), CANCEL)
                        .primary(&tr!("Create"), CONTINUE);
                }
                let lines = vec![
                    FormLine::Row(
                        tr!("Name"),
                        Box::new(TextField::new(&tr!("Name"), &self.name, field(Field::Name))),
                    ),
                    FormLine::Row(
                        tr!("User"),
                        Box::new(TextField::new(&tr!("User"), &self.user, field(Field::User))),
                    ),
                    FormLine::Row(
                        tr!("Password"),
                        Box::new(
                            SecureField::new(&tr!("Optional"), FIELD_TEXT, |_| {})
                                .on_edit(field(Field::Password)),
                        ),
                    ),
                    FormLine::Row(tr!("SSH key"), key),
                    FormLine::Row(
                        tr!("SSH port on this computer"),
                        Box::new(TextField::new(
                            &tr!("SSH port on this computer"),
                            &self.port,
                            field(Field::Port),
                        )),
                    ),
                ];
                new(text!("Name the machine and choose how you sign in.")).accessory(Form::new(
                    &tr!("Name the machine and choose how you sign in."),
                    lines,
                ))
            }
        };
        let mut alert = alert;
        if self.step != Step::Image {
            alert = alert.other(&tr!("Back"), BACK);
        }
        let primary = match self.step {
            Step::Identity => text!("Create"),
            _ => text!("Continue"),
        };
        alert
            .cancel(&tr!("Cancel"), CANCEL)
            .primary(&primary.tr(), CONTINUE)
    }

    pub fn plan(&self, fleet: &Fleet) -> Result<Plan, String> {
        let name = self.name.trim().to_owned();
        if !valid_name(&name) {
            return Err(tr!(
                "Use lowercase letters, digits and hyphens for the name."
            ));
        }
        if fleet.entries.values().any(|entry| entry.name() == name)
            || fleet.jobs.values().any(|job| job.name == name)
        {
            return Err(tr!("A machine named “{name}” already exists.", name = name));
        }
        if self.installs_from_iso() {
            let iso = self
                .iso
                .clone()
                .ok_or_else(|| tr!("No disc image chosen"))?;
            return Ok(Plan {
                source: Source::Iso(Installer {
                    iso,
                    preset: self.preset,
                    import: self.imports_disk(),
                }),
                new: NewMachine {
                    name,
                    os: self.os(),
                    cpus: self.cpus,
                    memory_mib: self.memory_gib * 1024,
                    disk_gib: self.disk_gib,
                    forwards: Vec::new(),
                },
                login: Login::default(),
            });
        }
        let user = self.user.trim().to_owned();
        if !valid_user(&user) {
            return Err(tr!(
                "Use lowercase letters, digits, hyphens and underscores for the user, and not root."
            ));
        }
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port >= LOWEST_PORT)
            .ok_or_else(|| tr!("Choose a port from 1024 to 65535."))?;
        if !port_free(fleet, port) {
            return Err(tr!("Port {port} is in use.", port = port));
        }
        let key_file = self
            .keys
            .get(self.key)
            .ok_or_else(|| tr!("No SSH key in ~/.ssh. Make one with ssh-keygen."))?;
        let key = std::fs::read_to_string(key_file)
            .map(|key| key.trim().to_owned())
            .map_err(|error| error.to_string())?;
        let image = *catalog()
            .get(self.image)
            .ok_or_else(|| tr!("Cloud image"))?;
        let identity = key_file.with_extension("");
        Ok(Plan {
            source: Source::Cloud(
                image,
                CloudInit {
                    hostname: name.clone(),
                    user: user.clone(),
                    ssh_keys: vec![key],
                    packages: GUEST_TOOLS.map(str::to_owned).to_vec(),
                    password: Some(self.password.clone()).filter(|password| !password.is_empty()),
                },
            ),
            new: NewMachine {
                name,
                os: image.os.to_owned(),
                cpus: self.cpus,
                memory_mib: self.memory_gib * 1024,
                disk_gib: self.disk_gib,
                forwards: vec![Forward {
                    host: port,
                    guest: SSH_PORT,
                }],
            },
            login: Login {
                user,
                identity: identity.is_file().then_some(identity),
            },
        })
    }
}
