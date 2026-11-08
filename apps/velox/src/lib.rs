mod containers;
mod diagnostics;
mod engine;
mod images;
mod machines;
mod networks;
mod notify;
mod overview;
mod readiness;
mod resources;
mod settings;
mod shell;
mod tray;
mod ui;
pub(crate) mod volumes;

pub use containers::{LogsTarget, LogsView, Request as ContainerRequest, Work};
pub use diagnostics::{engine_connected, engine_missing};
pub use engine::{
    Action, Endpoint, Failure, Informer, RemoteHost, Target, Tunnel, Update, context_endpoint,
    contexts, create_context, current_context, load_hosts, remove_context, rename_context,
    save_hosts, tunnel_socket, use_context,
};
pub use machines::{
    Act, DisplayWindow, Effect, Hub, INSPECTOR_COMMAND as MACHINE_INSPECTOR, MACHINE_WINDOW_SIZE,
    MachineWindow, MachinesPane, Notice, Request as MachineRequest,
};
pub use notify::notify_crash;
pub use readiness::Readiness;
pub use resources::file_menu;
pub use settings::{SETTINGS_SIZE, VeloxSettings, settings_form};
pub use shell::{ShellLink, ShellTarget, ShellWindow};
pub use tray::{Tray, TrayAction};
pub use ui::{Place, Places, Request, SIDEBAR_WIDTH, Section, VeloxView};

pub const TERMINAL_APP_ID: &str = "org.bande.Terminal";
