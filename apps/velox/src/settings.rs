use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use accesskit::{ActionRequest, Node};
use bande_app_settings::{Shared, Stored, flag, pairs, settings_folder};
use bande_l10n::{Text, text, tr};
use bande_toolkit::ui::{
    Button, Context, Event, Form, FormLine, ScrollView, Size, Toggle, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Item, Rect, TextSystem};

use crate::diagnostics::diagnostics;

pub const SETTINGS_SIZE: (u32, u32) = (560, 440);
const DESKTOP_ENTRY: &str = "org.bande.Velox.desktop";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VeloxSettings {
    pub notify_crashes: bool,
    pub command_palette: bool,
    pub status_item: bool,
}

impl VeloxSettings {
    pub fn current() -> Self {
        settings_folder()
            .as_deref()
            .map(Self::load)
            .unwrap_or_default()
    }
}

impl Stored for VeloxSettings {
    const FILE: &'static str = "velox.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            let on = flag(value).unwrap_or_default();
            match key {
                "notify-crashes" => settings.notify_crashes = on,
                "command-palette" => settings.command_palette = on,
                "status-item" => settings.status_item = on,
                _ => {}
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        format!(
            "notify-crashes = {}\ncommand-palette = {}\nstatus-item = {}\n",
            self.notify_crashes, self.command_palette, self.status_item
        )
    }
}

fn autostart() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    bande_xdg::autostart_dir(config.as_deref(), home.as_deref())
}

fn installed_entry() -> Option<PathBuf> {
    bande_xdg::data_dirs()
        .into_iter()
        .map(|dir| dir.join("applications").join(DESKTOP_ENTRY))
        .find(|path| path.is_file())
}

fn login_toggle() -> Box<dyn Widget> {
    let on = autostart().is_some_and(|dir| bande_xdg::opens_at_login(&dir, DESKTOP_ENTRY));
    Box::new(Toggle::new(&tr!("Launch Velox at login"), on, |on| {
        let (Some(dir), Some(entry)) = (autostart(), installed_entry()) else {
            return tracing::warn!("cannot find Velox's desktop entry");
        };
        if let Err(error) = bande_xdg::set_open_at_login(&dir, DESKTOP_ENTRY, &entry, on) {
            tracing::warn!("cannot change launch at login: {error}");
        }
    }))
}

struct CopyDiagnostics {
    button: Button,
    clicked: Rc<Cell<bool>>,
}

impl CopyDiagnostics {
    fn new() -> Self {
        let clicked = Rc::new(Cell::new(false));
        let click = Rc::clone(&clicked);
        Self {
            button: Button::new(&tr!("Copy Diagnostics"), move || click.set(true)).tooltip(&tr!(
                "Copies versions, the engine's state and the end of its log, for a bug report"
            )),
            clicked,
        }
    }

    fn copy_if_clicked(&self, cx: &mut Context) {
        if self.clicked.replace(false) {
            cx.copy(&diagnostics());
        }
    }
}

impl Widget for CopyDiagnostics {
    fn id(&self) -> WidgetId {
        self.button.id()
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.button.measure(cx, available)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.button.place(cx, frame);
    }

    fn frame(&self) -> Rect {
        self.button.frame()
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.button.paint(cx, items);
    }

    fn access(&self, cx: &Context) -> Node {
        self.button.access(cx)
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let handled = self.button.handle(cx, event);
        self.copy_if_clicked(cx);
        handled
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        let handled = self.button.perform(cx, request);
        self.copy_if_clicked(cx);
        handled
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        self.button.accepts_focus(cx)
    }

    fn tooltip(&self) -> Option<&str> {
        Widget::tooltip(&self.button)
    }
}

fn toggle(
    settings: &Shared<VeloxSettings>,
    label: Text,
    field: fn(&mut VeloxSettings) -> &mut bool,
) -> Box<dyn Widget> {
    let shared = settings.clone();
    let on = *field(&mut settings.borrow_mut());
    Box::new(Toggle::new(&label.tr(), on, move |on| {
        *field(&mut shared.borrow_mut()) = on;
    }))
}

pub fn settings_form(appearance: Appearance, settings: &Shared<VeloxSettings>) -> Ui {
    let form = Form::new(
        &tr!("Velox Settings"),
        vec![
            FormLine::Heading(tr!("Startup")),
            FormLine::Row(tr!("Launch Velox at login"), login_toggle()),
            FormLine::Heading(tr!("Notifications")),
            FormLine::Row(
                tr!("Notify when a container stops unexpectedly"),
                toggle(
                    settings,
                    text!("Notify when a container stops unexpectedly"),
                    |settings| &mut settings.notify_crashes,
                ),
            ),
            FormLine::Heading(tr!("Quick Access")),
            FormLine::Row(
                tr!("Command palette (Ctrl+K)"),
                toggle(settings, text!("Command palette (Ctrl+K)"), |settings| {
                    &mut settings.command_palette
                }),
            ),
            FormLine::Row(
                tr!("Status item in the Taskbar"),
                toggle(settings, text!("Status item in the Taskbar"), |settings| {
                    &mut settings.status_item
                }),
            ),
            FormLine::Heading(tr!("About")),
            FormLine::Row(tr!("Diagnostics"), Box::new(CopyDiagnostics::new())),
        ],
    );
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form)),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_notices_are_off_until_turned_on() {
        assert!(!VeloxSettings::parse("").notify_crashes);
        let on = VeloxSettings {
            notify_crashes: true,
            ..VeloxSettings::default()
        };
        assert_eq!(VeloxSettings::parse(&on.to_text()), on);
    }
}
