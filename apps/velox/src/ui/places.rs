use std::cell::RefCell;
use std::rc::Rc;

use bande_toolkit::Palette;
use bande_toolkit::ui::{Alert, Cell, Menu, MenuItem, PopUpButton, Row, TextField, Ui};

use super::{Intent, Intents, push};
use crate::L10N;
use crate::engine::{DockerContext, RemoteHost, new_host_id};
use crate::resources::{Field, Fields};

pub const HOST_PREFIX: &str = "place:";
pub const CONTEXT_PREFIX: &str = "context:";
const LOCAL: &str = "local";
const ADD_HOST: &str = "add-host";
const NEW_WORKSPACE: &str = "new-workspace";
const REMOVE_HOST: &str = "remove-host";
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

    pub fn rows(&self, palette: &Palette, running: Option<String>) -> Vec<Row> {
        let marker = |on: bool| super::dot(palette.accent, on);
        let local = !self.is_remote();
        let mut rows = vec![
            Row::new(
                "hosts-heading",
                vec![Cell::new(L10N.tr("sidebar-hosts")).muted().label()],
            ),
            Row::new(
                format!("{HOST_PREFIX}{LOCAL}"),
                vec![
                    Cell::new(L10N.tr("host-local")).icon(marker(local)),
                    Cell::new(match local {
                        true => running.clone().unwrap_or_default(),
                        false => String::new(),
                    })
                    .muted()
                    .tabular(),
                ],
            ),
        ];
        rows.extend(self.hosts.iter().map(|host| {
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
        rows.push(Row::new(
            "workspaces-heading",
            vec![Cell::new(L10N.tr("sidebar-workspaces")).muted().label()],
        ));
        rows.extend(self.contexts.iter().map(|context| {
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
        }));
        rows
    }

    pub fn menu(&self, keys: &[String]) -> Option<Menu> {
        let host = keys
            .first()
            .and_then(|key| key.strip_prefix(HOST_PREFIX))
            .filter(|id| *id != LOCAL);
        let mut items = Vec::new();
        if host.is_some() {
            items.extend([
                MenuItem::new(&L10N.tr("host-copy-ssh"), COPY_SSH),
                MenuItem::new(&L10N.tr("host-remove-ellipsis"), REMOVE_HOST),
                MenuItem::separator(),
            ]);
        }
        items.extend([
            MenuItem::new(&L10N.tr("host-add-ellipsis"), ADD_HOST),
            MenuItem::new(&L10N.tr("workspace-new-ellipsis"), NEW_WORKSPACE),
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
    match command {
        ADD_HOST => Some(Intent::AddHostSheet),
        NEW_WORKSPACE => Some(Intent::NewWorkspaceSheet),
        REMOVE_HOST => host.map(Intent::RemoveHostSheet),
        COPY_SSH => host.map(Intent::CopySsh),
        _ => None,
    }
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
            &L10N.tr("host-address"),
            text_field(&draft, "server.example.com", "", |draft, text| {
                draft.host = text
            }),
        ),
        Field::new(
            &L10N.tr("host-port"),
            text_field(&draft, "22", &SSH_PORT.to_string(), |draft, text| {
                draft.port = text
            }),
        ),
        Field::new(
            &L10N.tr("host-user"),
            text_field(&draft, &user, &user, |draft, text| draft.user = text),
        ),
        Field::new(
            &L10N.tr("host-name"),
            text_field(
                &draft,
                &L10N.tr("host-name-placeholder"),
                "",
                |draft, text| draft.name = text,
            ),
        ),
        Field::new(
            &L10N.tr("host-socket"),
            text_field(&draft, DEFAULT_SOCKET, DEFAULT_SOCKET, |draft, text| {
                draft.socket = text
            }),
        ),
    ]);
    let intents = Rc::clone(intents);
    ui.present_sheet(
        Alert::new(
            &L10N.tr("host-add-title"),
            &L10N.tr("host-add-message"),
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
        .primary(&L10N.tr("host-add"), "add")
        .cancel(&L10N.tr("button-cancel"), "cancel"),
    );
}

pub fn remove_host_sheet(ui: &mut Ui, intents: &Intents, host: &RemoteHost) {
    let intents = Rc::clone(intents);
    let id = host.id.clone();
    ui.present_sheet(
        Alert::new(
            &L10N.tr_args("host-remove-title", [("name", host.name.clone().into())]),
            &L10N.tr("host-remove-message"),
            move |choice| {
                if choice == "remove" {
                    push(&intents, Intent::RemoveHost(id.clone()));
                }
            },
        )
        .destructive(&L10N.tr("remove"), "remove")
        .cancel(&L10N.tr("button-cancel"), "cancel"),
    );
}

pub fn new_workspace_sheet(ui: &mut Ui, intents: &Intents, hosts: &[RemoteHost]) {
    let name = Rc::new(RefCell::new(String::new()));
    let chosen = Rc::new(RefCell::new(0usize));
    let mut choices = vec![L10N.tr("host-local")];
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
            &L10N.tr("workspace-name"),
            TextField::new(&L10N.tr("workspace-name-placeholder"), "", move |text| {
                *typed.borrow_mut() = text.trim().to_owned();
            }),
        ),
        Field::new(
            &L10N.tr("workspace-engine"),
            PopUpButton::new(&L10N.tr("workspace-engine"), &choices, 0, move |index| {
                *picked.borrow_mut() = index;
            }),
        ),
    ]);
    let intents = Rc::clone(intents);
    ui.present_sheet(
        Alert::new(
            &L10N.tr("workspace-new-title"),
            &L10N.tr("workspace-new-message"),
            move |choice| {
                let name = name.borrow().clone();
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|char| char.is_ascii_alphanumeric() || "-_.".contains(char));
                if choice != "create" || !valid {
                    return;
                }
                let endpoint = endpoints.get(*chosen.borrow()).cloned().unwrap_or_default();
                push(&intents, Intent::CreateWorkspace(name, endpoint));
            },
        )
        .accessory(fields)
        .primary(&L10N.tr("create"), "create")
        .cancel(&L10N.tr("button-cancel"), "cancel"),
    );
}
