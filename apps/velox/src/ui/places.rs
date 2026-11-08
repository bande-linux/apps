use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::tr;
use bande_toolkit::ui::{Alert, Cell, Heading, Menu, MenuItem, PopUpButton, Row, TextField, Ui};
use bande_toolkit::{Image, Palette};

use super::{Intent, Intents, push};
use crate::engine::{DockerContext, RemoteHost, new_host_id};
use crate::resources::{Field, Fields};

pub const HOST_PREFIX: &str = "place:";
pub const CONTEXT_PREFIX: &str = "context:";
const LOCAL: &str = "local";
const ADD_HOST: &str = "add-host";
const NEW_WORKSPACE: &str = "new-workspace";
const REMOVE_HOST: &str = "remove-host";
const RENAME_HOST: &str = "rename-host";
const RENAME_WORKSPACE: &str = "rename-workspace";
const DELETE_WORKSPACE: &str = "delete-workspace";
const DEFAULT_WORKSPACE: &str = "default";
const COPY_SSH: &str = "copy-ssh";
const SSH_PORT: u16 = 22;
const DEFAULT_SOCKET: &str = "/var/run/docker.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Local,
    Host(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    pub hosts: Vec<RemoteHost>,
    pub contexts: Vec<DockerContext>,
    pub current: String,
    pub place: Option<Place>,
}

impl Places {
    pub fn is_remote(&self) -> bool {
        matches!(self.place, Some(Place::Host(_)))
    }

    pub fn host(&self, id: &str) -> Option<&RemoteHost> {
        self.hosts.iter().find(|host| host.id == id)
    }

    pub fn sections(
        &self,
        palette: &Palette,
        running: Option<String>,
        add: Option<&Image>,
    ) -> [(Heading, Vec<Row>); 2] {
        let heading = |title: String, name: String, command: &str| {
            let heading = Heading::new(0, title);
            match add {
                Some(symbol) => heading.button(symbol.clone(), &name, command),
                None => heading,
            }
        };
        let marker = |on: bool| super::dot(palette.accent, on);
        let local = !self.is_remote();
        let mut hosts = vec![Row::new(
            format!("{HOST_PREFIX}{LOCAL}"),
            vec![
                Cell::new(tr!("This Computer")).icon(marker(local)),
                Cell::new(match local {
                    true => running.clone().unwrap_or_default(),
                    false => String::new(),
                })
                .muted()
                .tabular(),
            ],
        )];
        hosts.extend(self.hosts.iter().map(|host| {
            let on = self.place == Some(Place::Host(host.id.clone()));
            Row::new(
                format!("{HOST_PREFIX}{}", host.id),
                vec![
                    Cell::new(host.name.as_str()).icon(marker(on)),
                    Cell::new(match on {
                        true => running.clone().unwrap_or_default(),
                        false => String::new(),
                    })
                    .muted()
                    .tabular(),
                ],
            )
        }));
        let workspaces = self.contexts.iter().map(|context| {
            let row = Row::new(
                format!("{CONTEXT_PREFIX}{}", context.name),
                vec![
                    Cell::new(context.name.as_str()).icon(marker(context.name == self.current)),
                    Cell::new(""),
                ],
            );
            match local {
                true => row,
                false => row.dimmed(),
            }
        });
        [
            (
                heading(tr!("Hosts"), tr!("Add a Remote Host"), ADD_HOST),
                hosts,
            ),
            (
                heading(tr!("Workspaces"), tr!("New Workspace"), NEW_WORKSPACE),
                workspaces.collect(),
            ),
        ]
    }

    pub fn menu(&self, keys: &[String]) -> Option<Menu> {
        let host = keys
            .first()
            .and_then(|key| key.strip_prefix(HOST_PREFIX))
            .filter(|id| *id != LOCAL);
        let workspace = keys
            .first()
            .and_then(|key| key.strip_prefix(CONTEXT_PREFIX))
            .filter(|name| *name != DEFAULT_WORKSPACE);
        let mut items = Vec::new();
        if host.is_some() {
            items.extend([
                MenuItem::new(&tr!("Copy SSH Command"), COPY_SSH),
                MenuItem::separator(),
                MenuItem::new(&tr!("Rename…"), RENAME_HOST),
                MenuItem::new(&tr!("Remove Host…"), REMOVE_HOST),
                MenuItem::separator(),
            ]);
        }
        if workspace.is_some() {
            items.extend([
                MenuItem::new(&tr!("Rename…"), RENAME_WORKSPACE),
                MenuItem::new(&tr!("Delete…"), DELETE_WORKSPACE),
                MenuItem::separator(),
            ]);
        }
        items.extend([
            MenuItem::new(&tr!("Add Remote Host…"), ADD_HOST),
            MenuItem::new(&tr!("New Workspace…"), NEW_WORKSPACE),
        ]);
        Some(Menu::new(items))
    }
}

pub fn place_of(key: &str) -> Option<Place> {
    key.strip_prefix(HOST_PREFIX).map(|id| match id {
        LOCAL => Place::Local,
        id => Place::Host(id.to_owned()),
    })
}

pub fn menu_intent(command: &str, keys: &[String]) -> Option<Intent> {
    let host = keys
        .first()
        .and_then(|key| key.strip_prefix(HOST_PREFIX))
        .map(str::to_owned);
    let workspace = keys
        .first()
        .and_then(|key| key.strip_prefix(CONTEXT_PREFIX))
        .map(str::to_owned);
    match command {
        ADD_HOST => Some(Intent::AddHostSheet),
        NEW_WORKSPACE => Some(Intent::NewWorkspaceSheet),
        REMOVE_HOST => host.map(Intent::RemoveHostSheet),
        RENAME_HOST => host.map(Intent::RenameHostSheet),
        COPY_SSH => host.map(Intent::CopySsh),
        RENAME_WORKSPACE => workspace.map(Intent::RenameWorkspaceSheet),
        DELETE_WORKSPACE => workspace.map(Intent::DeleteWorkspaceSheet),
        _ => None,
    }
}

fn valid_workspace(name: &str) -> bool {
    !name.is_empty()
        && name != DEFAULT_WORKSPACE
        && name
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || "-_.".contains(char))
}

pub fn rename_host_sheet(ui: &mut Ui, intents: &Intents, host: &RemoteHost) {
    let id = host.id.clone();
    rename_sheet(
        ui,
        intents,
        &host.name,
        &tr!("Only the name shown in Velox changes."),
        move |name| Some(Intent::RenameHost(id.clone(), name)),
    );
}

pub fn rename_workspace_sheet(ui: &mut Ui, intents: &Intents, name: &str) {
    let old = name.to_owned();
    rename_sheet(
        ui,
        intents,
        name,
        &tr!("The Docker context is renamed too, so the docker command sees the new name."),
        move |name| valid_workspace(&name).then(|| Intent::RenameWorkspace(old.clone(), name)),
    );
}

fn rename_sheet(
    ui: &mut Ui,
    intents: &Intents,
    current: &str,
    message: &str,
    renamed: impl Fn(String) -> Option<Intent> + 'static,
) {
    let typed = Rc::new(RefCell::new(current.to_owned()));
    let field = {
        let typed = Rc::clone(&typed);
        TextField::new(&tr!("Name"), current, move |text| {
            *typed.borrow_mut() = text.trim().to_owned();
        })
    };
    let intents = Rc::clone(intents);
    let old = current.to_owned();
    ui.present_sheet(
        Alert::new(
            &tr!("Rename “{name}”", name = current.to_owned()),
            message,
            move |choice| {
                let name = typed.borrow().clone();
                if choice != "rename" || name == old || name.is_empty() {
                    return;
                }
                if let Some(intent) = renamed(name) {
                    push(&intents, intent);
                }
            },
        )
        .accessory(field)
        .primary(&tr!("Rename"), "rename")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}

pub fn delete_workspace_sheet(ui: &mut Ui, intents: &Intents, name: &str, current: bool) {
    let intents = Rc::clone(intents);
    let owned = name.to_owned();
    let message = match current {
        true => tr!(
            "The Docker context is deleted and Velox switches to the default workspace. Containers, images and volumes stay on their engine."
        ),
        false => tr!(
            "The Docker context is deleted. Containers, images and volumes stay on their engine."
        ),
    };
    ui.present_sheet(
        Alert::new(
            &tr!("Delete “{name}”?", name = name.to_owned()),
            &message,
            move |choice| {
                if choice == "delete" {
                    push(&intents, Intent::DeleteWorkspace(owned.clone()));
                }
            },
        )
        .destructive(&tr!("Delete"), "delete")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}

#[derive(Debug, Clone, Default)]
struct HostDraft {
    host: String,
    port: String,
    user: String,
    name: String,
    socket: String,
}

fn text_field(
    draft: &Rc<RefCell<HostDraft>>,
    placeholder: &str,
    text: &str,
    set: fn(&mut HostDraft, String),
) -> TextField {
    let draft = Rc::clone(draft);
    TextField::new(placeholder, text, move |typed| {
        set(&mut draft.borrow_mut(), typed.trim().to_owned());
    })
}

pub fn add_host_sheet(ui: &mut Ui, intents: &Intents) {
    let user = std::env::var("USER").unwrap_or_default();
    let draft = Rc::new(RefCell::new(HostDraft {
        user: user.clone(),
        port: SSH_PORT.to_string(),
        socket: DEFAULT_SOCKET.to_owned(),
        ..HostDraft::default()
    }));
    let fields = Fields::new(vec![
        Field::new(
            &tr!("Host"),
            text_field(&draft, "server.example.com", "", |draft, text| {
                draft.host = text
            }),
        ),
        Field::new(
            &tr!("Port"),
            text_field(&draft, "22", &SSH_PORT.to_string(), |draft, text| {
                draft.port = text
            }),
        ),
        Field::new(
            &tr!("User"),
            text_field(&draft, &user, &user, |draft, text| draft.user = text),
        ),
        Field::new(
            &tr!("Name"),
            text_field(&draft, &tr!("Optional"), "", |draft, text| {
                draft.name = text
            }),
        ),
        Field::new(
            &tr!("Docker socket"),
            text_field(&draft, DEFAULT_SOCKET, DEFAULT_SOCKET, |draft, text| {
                draft.socket = text
            }),
        ),
    ]);
    let intents = Rc::clone(intents);
    ui.present_sheet(
        Alert::new(
            &tr!("Add Remote Host"),
            &tr!("Velox connects over SSH with your keys and ~/.ssh/config, and stores no password. Your user on the server must be able to use its Docker socket."),
            move |choice| {
                let draft = draft.borrow().clone();
                let Ok(port) = draft.port.parse::<u16>() else {
                    return;
                };
                if choice != "add" || draft.host.is_empty() || draft.host.contains(' ') {
                    return;
                }
                let name = match draft.name.is_empty() {
                    true => draft.host.clone(),
                    false => draft.name,
                };
                let socket = match draft.socket.starts_with('/') {
                    true => draft.socket,
                    false => DEFAULT_SOCKET.to_owned(),
                };
                push(
                    &intents,
                    Intent::AddHost(RemoteHost {
                        id: new_host_id(),
                        name,
                        user: draft.user,
                        host: draft.host,
                        port,
                        socket,
                    }),
                );
            },
        )
        .accessory(fields)
        .primary(&tr!("Add"), "add")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}

pub fn remove_host_sheet(ui: &mut Ui, intents: &Intents, host: &RemoteHost) {
    let intents = Rc::clone(intents);
    let id = host.id.clone();
    ui.present_sheet(
        Alert::new(
            &tr!("Remove “{name}”?", name = host.name.clone()),
            &tr!("It is removed from Velox only. Nothing on the server changes."),
            move |choice| {
                if choice == "remove" {
                    push(&intents, Intent::RemoveHost(id.clone()));
                }
            },
        )
        .destructive(&tr!("Remove"), "remove")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}

pub fn new_workspace_sheet(ui: &mut Ui, intents: &Intents, hosts: &[RemoteHost]) {
    let name = Rc::new(RefCell::new(String::new()));
    let chosen = Rc::new(RefCell::new(0usize));
    let mut choices = vec![tr!("This Computer")];
    choices.extend(hosts.iter().map(|host| host.name.clone()));
    let endpoints: Vec<String> = std::iter::once(String::new())
        .chain(hosts.iter().map(|host| {
            let port = match host.port {
                SSH_PORT => String::new(),
                port => format!(":{port}"),
            };
            format!("ssh://{}{port}", host.ssh_target())
        }))
        .collect();
    let typed = Rc::clone(&name);
    let picked = Rc::clone(&chosen);
    let fields = Fields::new(vec![
        Field::new(
            &tr!("Name"),
            TextField::new(&tr!("project-a"), "", move |text| {
                *typed.borrow_mut() = text.trim().to_owned();
            }),
        ),
        Field::new(
            &tr!("Engine"),
            PopUpButton::new(&tr!("Engine"), &choices, 0, move |index| {
                *picked.borrow_mut() = index;
            }),
        ),
    ]);
    let intents = Rc::clone(intents);
    ui.present_sheet(
        Alert::new(
            &tr!("New Workspace"),
            &tr!("A workspace is a Docker context: the docker command and Velox both use the engine it points to."),
            move |choice| {
                let name = name.borrow().clone();
                if choice != "create" || !valid_workspace(&name) {
                    return;
                }
                let endpoint = endpoints.get(*chosen.borrow()).cloned().unwrap_or_default();
                push(&intents, Intent::CreateWorkspace(name, endpoint));
            },
        )
        .accessory(fields)
        .primary(&tr!("Create"), "create")
        .cancel(&tr!("Cancel"), "cancel"),
    );
}
