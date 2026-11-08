use bande_app_settings::{Shared, Stored, flag, pairs, settings_folder};
use bande_toolkit::ui::{Context, Form, FormLine, ScrollView, Toggle, Ui, Widget};
use bande_toolkit::{Appearance, TextSystem};

use crate::L10N;

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
    label: &str,
    field: fn(&mut VeloxSettings) -> &mut bool,
) -> Box<dyn Widget> {
    let shared = settings.clone();
    let on = *field(&mut settings.borrow_mut());
    Box::new(Toggle::new(&L10N.tr(label), on, move |on| {
        *field(&mut shared.borrow_mut()) = on;
    }))
}

pub fn settings_form(appearance: Appearance, settings: &Shared<VeloxSettings>) -> Ui {
    let form = Form::new(
        &L10N.tr("settings-title"),
        vec![
            FormLine::Heading(L10N.tr("settings-notifications")),
            FormLine::Row(
                L10N.tr("settings-notify-crashes"),
                toggle(settings, "settings-notify-crashes", |settings| {
                    &mut settings.notify_crashes
                }),
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
