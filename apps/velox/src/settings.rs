use bande_app_settings::{Shared, Stored, flag, pairs, settings_folder};
use bande_l10n::{Text, text, tr};
use bande_toolkit::ui::{Context, Form, FormLine, ScrollView, Toggle, Ui, Widget};
use bande_toolkit::{Appearance, TextSystem};

pub const SETTINGS_SIZE: (u32, u32) = (520, 320);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VeloxSettings {
    pub notify_crashes: bool,
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
            if key == "notify-crashes" {
                settings.notify_crashes = flag(value).unwrap_or_default();
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        format!("notify-crashes = {}\n", self.notify_crashes)
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
            FormLine::Heading(tr!("Notifications")),
            FormLine::Row(
                tr!("Notify when a container stops unexpectedly"),
                toggle(
                    settings,
                    text!("Notify when a container stops unexpectedly"),
                    |settings| &mut settings.notify_crashes,
                ),
            ),
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
        };
        assert_eq!(VeloxSettings::parse(&on.to_text()), on);
    }
}
