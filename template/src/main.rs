use std::process::ExitCode;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, WindowId};
use bande_app_menu::Entry;
use bande_l10n::tr;
use bande_toolkit::TextSystem;
use bande_toolkit::ui::{Axis, Context, Label, Stack, Ui, Widget};

static INFO: Info = Info {
    id: "org.bande.Template",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Template"),
    keys: Keys::Standard,
    settings: None,
};

const NEW_WINDOW: &str = "new-window";
const WINDOW_SIZE: (u32, u32) = (480, 320);

struct Template;

fn open_window(cx: &mut Cx<Template>) -> Result<WindowId, bande_toolkit::WindowError> {
    cx.open(Kind::Main, &tr!("Template"), WINDOW_SIZE, |appearance| {
        let hello: Box<dyn Widget> = Box::new(Label::new(&tr!("Hello from a Bande app")));
        let mut ui = Ui::new(
            Box::new(Stack::new(Axis::Vertical, 0.0, 24.0, vec![hello])),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui
    })
}

impl Program for Template {
    type View = Ui;

    fn start(_cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        Ok(Self)
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        let new_window = Entry::item(&tr!("New Window"), NEW_WINDOW).shortcut("Ctrl+N");
        Menus {
            menus: vec![Entry::submenu(&tr!("File"), vec![new_window])],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        if command == NEW_WINDOW {
            self.reopen(cx);
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if let Err(error) = open_window(cx) {
            tracing::warn!("cannot open a window: {error}");
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Template>(&INFO)
}
