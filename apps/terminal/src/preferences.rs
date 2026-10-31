use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use accesskit::TreeUpdate;
use bande_term::{L10N, SETTINGS_SIZE, Settings, SharedSettings, settings_folder, settings_view};
use bande_toolkit::ui::Ui;
use bande_toolkit::{App, Appearance, Rect, Role, Scene, Window, WindowError, WindowEvent};

use crate::app::APP_ID;

pub struct Preferences {
    ui: Ui,
    settings: SharedSettings,
    saved: Settings,
    folder: Option<PathBuf>,
}

impl Preferences {
    pub fn new(appearance: Appearance, settings: Settings, folder: Option<PathBuf>) -> Self {
        let shared: SharedSettings = Rc::new(RefCell::new(settings));
        Self {
            ui: settings_view(appearance, &shared),
            settings: shared,
            saved: settings,
            folder,
        }
    }

    fn save(&mut self) {
        let settings = *self.settings.borrow();
        if settings == self.saved {
            return;
        }
        let Some(folder) = &self.folder else {
            return;
        };
        match settings.save(folder) {
            Ok(()) => self.saved = settings,
            Err(error) => tracing::warn!("cannot save the terminal settings: {error}"),
        }
    }
}

impl App for Preferences {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.ui.scene(width, height);
        scene.background = window.appearance().palette().sheet;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let Err(error) = window.drive(&mut self.ui, event) {
            tracing::warn!("{error}");
        }
        self.save();
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

pub fn run() -> Result<(), WindowError> {
    let folder = settings_folder();
    let settings = folder
        .as_ref()
        .map(|folder| Settings::load(folder))
        .unwrap_or_default();
    let mut window = Window::open(
        Role::toplevel(&L10N.tr("settings-title"), APP_ID),
        SETTINGS_SIZE,
    )?;
    let mut preferences = Preferences::new(window.appearance(), settings, folder);
    bande_toolkit::run(&mut window, &mut preferences)
}
