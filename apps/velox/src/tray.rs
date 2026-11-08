use std::collections::BTreeMap;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;

use bande_app_menu::{Entry, STATUS_ACTIVATE, StatusItem};
use bande_l10n::tr;
use zbus::blocking::Connection;

use crate::engine::{Action, Container, Engine, Update};
use crate::ui::engine_name;

const SHOW: &str = "tray-show";
const SETTINGS: &str = "tray-settings";
const QUIT: &str = "tray-quit";
const STOP: &str = "tray-stop:";
const OPEN: &str = "tray-open:";
const SHOWN_CONTAINERS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    Settings,
    Quit,
    Open(String),
}

pub struct Tray {
    item: StatusItem,
    engine: Option<(String, PathBuf)>,
    containers: BTreeMap<String, Container>,
}

impl Tray {
    pub fn start(connection: Connection, app_id: &str) -> Option<Self> {
        let item = StatusItem::serve(connection, app_id, "Velox", app_id, Vec::new())
            .inspect_err(|error| tracing::warn!("cannot show the status item: {error}"))
            .ok()?;
        let tray = Self {
            item,
            engine: None,
            containers: BTreeMap::new(),
        };
        tray.refresh();
        Some(tray)
    }

    pub fn apply(&mut self, update: &Update) -> bool {
        match update {
            Update::Ready(info) => self.engine = Some((engine_name(info), info.socket.clone())),
            Update::Blocked(_) => {
                self.engine = None;
                self.containers.clear();
            }
            Update::Listed(containers) => {
                self.containers = containers
                    .iter()
                    .map(|container| (container.id.clone(), container.clone()))
                    .collect();
            }
            Update::Changed(container) => {
                self.containers
                    .insert(container.id.clone(), container.clone());
            }
            Update::Removed(id) => {
                self.containers.remove(id);
            }
            _ => return false,
        }
        true
    }

    fn entries(&self) -> Vec<Entry> {
        let state = match &self.engine {
            Some((name, _)) => name.clone(),
            None => tr!("No engine"),
        };
        let mut entries = vec![Entry::item(&state, "").enabled(false), Entry::separator()];
        let mut running: Vec<&Container> = self
            .containers
            .values()
            .filter(|container| container.running())
            .collect();
        running.sort_by_key(|container| container.name().to_lowercase());
        if running.is_empty() && self.engine.is_some() {
            entries.push(Entry::item(&tr!("No running containers"), "").enabled(false));
        }
        entries.extend(running.into_iter().take(SHOWN_CONTAINERS).map(|container| {
            let ports = container.published_ports();
            let title = match ports.first() {
                Some(port) => format!("{}  :{port}", container.name()),
                None => container.name().to_owned(),
            };
            let mut actions: Vec<Entry> = ports
                .iter()
                .map(|port| {
                    Entry::item(
                        &tr!("Open localhost:{port}", port = *port),
                        &format!("{OPEN}{port}"),
                    )
                })
                .collect();
            actions.push(Entry::item(
                &tr!("Stop"),
                &format!("{STOP}{}", container.id),
            ));
            Entry::submenu(&title, actions)
        }));
        entries.extend([
            Entry::separator(),
            Entry::item(&tr!("Open Velox"), SHOW),
            Entry::item(&tr!("Settings…"), SETTINGS),
            Entry::separator(),
            Entry::item(&tr!("Quit Velox"), QUIT),
        ]);
        entries
    }

    pub fn refresh(&self) {
        if let Err(error) = self.item.update(self.entries()) {
            tracing::warn!("cannot update the status item: {error}");
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.item.fd()
    }

    pub fn take_actions(&mut self) -> Vec<TrayAction> {
        let actions = self.item.take_actions();
        actions
            .into_iter()
            .filter_map(|action| self.act(&action))
            .collect()
    }

    fn act(&self, action: &str) -> Option<TrayAction> {
        if let Some(id) = action.strip_prefix(STOP) {
            let socket = self.engine.as_ref()?.1.clone();
            let id = id.to_owned();
            std::thread::spawn(move || {
                if let Err(error) = Engine::new(&socket).act(&id, Action::Stop) {
                    tracing::warn!("cannot stop {id}: {error}");
                }
            });
            return None;
        }
        if let Some(port) = action.strip_prefix(OPEN) {
            return Some(TrayAction::Open(format!("http://localhost:{port}")));
        }
        match action {
            SHOW | STATUS_ACTIVATE => Some(TrayAction::Show),
            SETTINGS => Some(TrayAction::Settings),
            QUIT => Some(TrayAction::Quit),
            _ => None,
        }
    }
}
