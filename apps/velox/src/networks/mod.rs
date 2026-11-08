use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use bande_app_menu::Entry;
use bande_toolkit::Palette;
use bande_toolkit::ui::{
    Alert, Button, Cell, Column, Menu, MenuItem, Row, Table, TextField, Ui, Widget, WidgetId,
};

use crate::L10N;
use crate::engine::{Network, Task};
use crate::overview::Tone;
use crate::resources::{
    Act, Acts, Field, Fields, Pane, Scope, boxed, confirm, matches, push, search, tell, tool,
};
use crate::ui::symbol;

const USER_PREFIX: &str = "net:";
const BUILTIN_PREFIX: &str = "builtin:";
const MEMBER_PREFIX: &str = "member:";
const CREATE: &str = "network-create";
const REMOVE: &str = "network-remove";
const COPY_NAME: &str = "network-copy-name";
const CREATE_SYMBOL: &str = include_str!("../../data/symbols/add.svg");
const REMOVE_SYMBOL: &str = include_str!("../../data/symbols/remove.svg");
const NETWORKS_SYMBOL: &str = include_str!("../../data/symbols/networks.svg");
const NAME_WIDTH: f32 = 220.0;
const DRIVER_WIDTH: f32 = 90.0;
const SUBNET_WIDTH: f32 = 160.0;
const CONTAINERS_WIDTH: f32 = 110.0;

#[derive(Default)]
pub struct Networks {
    table: Option<WidgetId>,
    remove: Option<WidgetId>,
    search: Option<WidgetId>,
    selected: Vec<String>,
    expanded: HashSet<String>,
    filter: String,
}

fn key(network: &Network) -> String {
    match network.predefined() {
        true => format!("{BUILTIN_PREFIX}{}", network.id),
        false => format!("{USER_PREFIX}{}", network.id),
    }
}

fn driver_tone(driver: &str) -> Tone {
    match driver {
        "bridge" => Tone::Blue,
        "host" => Tone::Purple,
        "null" | "none" => Tone::Grey,
        _ => Tone::Teal,
    }
}

fn row(scope: &Scope, network: &Network) -> Row {
    let mut members: Vec<(&str, String)> = scope
        .containers
        .values()
        .filter_map(|container| {
            let endpoints = container.network_settings.as_ref()?.networks.as_ref()?;
            let (_, endpoint) = endpoints.iter().find(|(name, endpoint)| {
                endpoint.network_id == network.id
                    || (endpoint.network_id.is_empty() && **name == network.name)
            })?;
            Some((container.name(), endpoint.ip_address.clone()))
        })
        .collect();
    members.sort_unstable();
    let subnets = network.subnets();
    let children = match members.is_empty() {
        true => vec![Row::new(
            format!("{MEMBER_PREFIX}{}:", network.id),
            vec![
                Cell::new(L10N.tr("no-attached-containers")).muted(),
                Cell::new(""),
                Cell::new(""),
                Cell::new(""),
            ],
        )],
        false => members
            .iter()
            .map(|(name, address)| {
                let address = match address.is_empty() {
                    true => Cell::new("—").muted(),
                    false => Cell::new(address.as_str()).tabular(),
                };
                Row::new(
                    format!("{MEMBER_PREFIX}{}:{name}", network.id),
                    vec![Cell::new(*name), Cell::new(""), address, Cell::new("")],
                )
            })
            .collect(),
    };
    let subnet = match subnets.is_empty() {
        true => Cell::new("—").muted(),
        false => Cell::new(subnets.join(", ")).tabular(),
    };
    Row::new(
        key(network),
        vec![
            Cell::new(network.name.as_str()).icon(symbol(
                NETWORKS_SYMBOL,
                driver_tone(&network.driver).color(scope.palette),
            )),
            Cell::new(network.driver.as_str()).muted(),
            subnet,
            Cell::new(members.len().to_string()).tabular(),
        ],
    )
    .children(children)
}

fn create_sheet(ui: &mut Ui, acts: &Acts) {
    let typed = Rc::new(RefCell::new((String::new(), String::new())));
    let name = {
        let typed = Rc::clone(&typed);
        TextField::new(&L10N.tr("network-name-placeholder"), "", move |text| {
            typed.borrow_mut().0 = text.trim().to_owned();
        })
    };
    let subnet = {
        let typed = Rc::clone(&typed);
        TextField::new(&L10N.tr("network-subnet-placeholder"), "", move |text| {
            typed.borrow_mut().1 = text.trim().to_owned();
        })
        .without_text_checking()
    };
    let acts = Rc::clone(acts);
    ui.present_sheet(
        Alert::new(
            &L10N.tr("create-network-title"),
            &L10N.tr("create-network-message"),
            move |choice| {
                let (name, subnet) = typed.borrow().clone();
                if choice == "create" && !name.is_empty() {
                    push(&acts, Act::Task(Task::CreateNetwork(name, subnet)));
                }
            },
        )
        .accessory(Fields::new(vec![
            Field::new(&L10N.tr("network-name"), name),
            Field::new(&L10N.tr("network-subnet"), subnet),
        ]))
        .primary(&L10N.tr("create"), "create")
        .cancel(&L10N.tr("button-cancel"), "cancel"),
    );
}

impl Networks {
    fn removable(&self) -> Vec<String> {
        self.selected
            .iter()
            .filter_map(|key| key.strip_prefix(USER_PREFIX))
            .map(str::to_owned)
            .collect()
    }
}

impl Pane for Networks {
    fn build(&mut self, acts: &Acts, palette: &Palette) -> (Vec<Box<dyn Widget>>, Box<dyn Widget>) {
        let create = tool(
            CREATE_SYMBOL,
            "create-network",
            palette,
            acts,
            Act::Command(CREATE),
        );
        let remove = tool(
            REMOVE_SYMBOL,
            "remove-selected",
            palette,
            acts,
            Act::Command(REMOVE),
        );
        let field = search("filter-networks", &self.filter, acts);
        self.remove = Some(remove.id());
        self.search = Some(field.id());
        let select = Rc::clone(acts);
        let expand = Rc::clone(acts);
        let menu_acts = Rc::clone(acts);
        let table = Table::new(vec![
            Column::new(&L10N.tr("column-name"), NAME_WIDTH)
                .flexible()
                .unsortable(),
            Column::new(&L10N.tr("column-driver"), DRIVER_WIDTH).unsortable(),
            Column::new(&L10N.tr("column-subnet"), SUBNET_WIDTH).unsortable(),
            Column::new(&L10N.tr("column-containers"), CONTAINERS_WIDTH)
                .numeric()
                .unsortable(),
        ])
        .on_selection(move |keys| push(&select, Act::Select(keys.to_vec())))
        .on_expand(move |keys, expanded, _| push(&expand, Act::Expand(keys.to_vec(), expanded)))
        .context_menu(
            |keys| {
                let user = keys.iter().any(|key| key.starts_with(USER_PREFIX));
                let network = keys
                    .iter()
                    .any(|key| key.starts_with(USER_PREFIX) || key.starts_with(BUILTIN_PREFIX));
                network.then(|| {
                    Menu::new(vec![
                        MenuItem::new(&L10N.tr("copy-name"), COPY_NAME),
                        MenuItem::separator(),
                        MenuItem::new(&L10N.tr("remove-ellipsis"), REMOVE).enabled(user),
                    ])
                })
            },
            move |action, keys| push(&menu_acts, Act::Menu(action.to_owned(), keys.to_vec())),
        );
        self.table = Some(table.id());
        (
            vec![boxed(create), boxed(remove), boxed(field)],
            boxed(table),
        )
    }

    fn fill(&mut self, ui: &mut Ui, scope: &Scope) {
        let mut networks: Vec<&Network> = scope
            .store
            .networks
            .iter()
            .filter(|network| matches(&self.filter, &[&network.name, &network.driver]))
            .collect();
        networks.sort_by_key(|network| (network.predefined(), network.name.to_lowercase()));
        let rows: Vec<Row> = networks
            .into_iter()
            .map(|network| row(scope, network))
            .collect();
        let expanded: Vec<String> = rows
            .iter()
            .filter(|row| self.expanded.contains(&row.key))
            .map(|row| row.key.clone())
            .collect();
        if let Some(table) = self.table.and_then(|id| ui.widget_as::<Table>(id)) {
            table.set_rows(rows);
            table.set_expanded_rows(expanded);
        }
        let removable = !self.removable().is_empty();
        if let Some(button) = self.remove.and_then(|id| ui.widget_as::<Button>(id)) {
            button.set_enabled(removable);
        }
    }

    fn act(&mut self, act: Act) {
        match act {
            Act::Select(keys) => self.selected = keys,
            Act::Filter(text) => self.filter = text,
            Act::Expand(keys, expanded) => {
                for key in keys {
                    match expanded {
                        true => self.expanded.insert(key),
                        false => self.expanded.remove(&key),
                    };
                }
            }
            _ => {}
        }
    }

    fn search(&self) -> Option<WidgetId> {
        self.search
    }

    fn menu(&self, _scope: &Scope) -> Option<Entry> {
        Some(Entry::submenu(
            &L10N.tr("menu-network"),
            vec![
                Entry::item(&L10N.tr("create-network-ellipsis"), CREATE),
                Entry::separator(),
                Entry::item(&L10N.tr("remove-ellipsis"), REMOVE)
                    .enabled(!self.removable().is_empty()),
            ],
        ))
    }

    fn command(&mut self, ui: &mut Ui, acts: &Acts, scope: &Scope, command: &str) -> bool {
        match command {
            CREATE => create_sheet(ui, acts),
            _ => {
                let keys = self.selected.clone();
                self.menu_action(ui, acts, scope, command, &keys);
            }
        }
        true
    }

    fn menu_action(
        &mut self,
        ui: &mut Ui,
        acts: &Acts,
        scope: &Scope,
        action: &str,
        keys: &[String],
    ) {
        let chosen: Vec<&Network> = scope
            .store
            .networks
            .iter()
            .filter(|network| keys.contains(&key(network)))
            .collect();
        match action {
            COPY_NAME => {
                if let Some(network) = chosen.first() {
                    ui.context().copy(&network.name);
                }
            }
            REMOVE => {
                let removable: Vec<&Network> = chosen
                    .into_iter()
                    .filter(|network| !network.predefined())
                    .collect();
                let title = match removable.as_slice() {
                    [] => return,
                    [only] => {
                        L10N.tr_args("remove-network-title", [("name", only.name.clone().into())])
                    }
                    _ => L10N.tr_args("remove-networks-title", [("count", removable.len().into())]),
                };
                confirm(
                    ui,
                    acts,
                    &title,
                    &L10N.tr("remove-network-message"),
                    &L10N.tr("remove"),
                    Task::RemoveNetworks(
                        removable.iter().map(|network| network.id.clone()).collect(),
                    ),
                );
            }
            _ => {}
        }
    }

    fn finished(&mut self, ui: &mut Ui, task: &Task, result: &Result<u64, String>) -> bool {
        let title = match (task, result) {
            (Task::CreateNetwork(name, _), Err(_)) => {
                L10N.tr_args("cannot-create-network", [("name", name.clone().into())])
            }
            (Task::RemoveNetworks(_), Err(_)) => L10N.tr("cannot-remove-networks"),
            _ => return false,
        };
        if let Err(message) = result {
            tell(ui, &title, message);
        }
        true
    }
}
