use bande_app_settings::{Shared, Stored, flag, pairs};
use bande_l10n::tr;
use bande_toolkit::ui::{Context, Form, FormLine, Label, ScrollView, Toggle, Ui};
use bande_toolkit::{Appearance, TextSystem};

pub const SETTINGS_SIZE: (u32, u32) = (520, 280);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SoftwareSettings {
    pub flathub: bool,
}

impl Stored for SoftwareSettings {
    const FILE: &'static str = "software.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            if key == "flathub" {
                settings.flathub = flag(value).unwrap_or(settings.flathub);
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        format!("flathub = {}\n", self.flathub)
    }
}

pub fn settings_form(appearance: Appearance, settings: &Shared<SoftwareSettings>) -> Ui {
    let shared = settings.clone();
    let flathub = Toggle::new(
        &tr!("Get apps from Flathub"),
        settings.borrow().flathub,
        move |on| shared.borrow_mut().flathub = on,
    );
    let form = Form::new(
        &tr!("Software Settings"),
        vec![
            FormLine::Heading(tr!("Sources")),
            FormLine::Row(tr!("Get apps from Flathub"), Box::new(flathub)),
            FormLine::Wide(Box::new(
                Label::new(&tr!("Flathub offers apps Debian doesn’t have, and newer versions, packaged by their developers or the community. They install for you only, without a password, run in a sandbox and update with Software Update.")).secondary(),
            )),
        ],
    );
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form)),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
