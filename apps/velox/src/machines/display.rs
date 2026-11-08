use std::os::fd::BorrowedFd;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_toolkit::ui::{Context, Label, Ui, Widget, WidgetId};
use bande_toolkit::{
    Appearance, FrameRegions, HEADER_LEADING, Header, Rect, Scene, TextSystem,
    WINDOW_HEADER_HEIGHT, Window, WindowEvent,
};

use super::console::{Console, Outcome};
use super::widgets::{Bar, Slot};
use crate::L10N;
use crate::ui::Centered;

const TOOLBAR: f32 = WINDOW_HEADER_HEIGHT as f32;
const CENTER_WIDTH: f32 = 360.0;

pub struct DisplayWindow {
    id: String,
    display: u32,
    title: String,
    ui: Ui,
    slot: WidgetId,
    session: Option<Console>,
}

fn message(key: &str) -> Box<dyn Widget> {
    Box::new(Centered::new(
        Box::new(Label::new(&L10N.tr(key)).secondary()),
        CENTER_WIDTH,
    ))
}

impl DisplayWindow {
    pub fn new(id: &str, index: u32, machine: &str, appearance: Appearance) -> Self {
        let slot = Slot::new(message("machine-console-connecting"));
        let slot_id = slot.id();
        let bar = Bar::new(Box::new(slot), Vec::new(), TOOLBAR, HEADER_LEADING);
        let mut ui = Ui::new(
            Box::new(bar),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        let session = Console::open(id, index)
            .inspect_err(|error| tracing::warn!("cannot show display {}: {error}", index + 1))
            .ok();
        Self {
            id: id.to_owned(),
            display: index,
            title: L10N.tr_args(
                "machine-display-title",
                [
                    ("name", machine.to_owned().into()),
                    ("number", (index + 1).into()),
                ],
            ),
            ui,
            slot: slot_id,
            session,
        }
    }

    pub fn machine(&self) -> &str {
        &self.id
    }

    pub fn display(&self) -> u32 {
        self.display
    }

    fn show(&mut self, content: Box<dyn Widget>) {
        if let Some(slot) = self.ui.widget_as::<Slot>(self.slot) {
            slot.set(content);
        }
    }
}

impl View for DisplayWindow {
    fn scene(&mut self, window: &mut Window) -> Scene {
        window.set_frame_regions(FrameRegions {
            sidebar: 0.0,
            toolbar: 0.0,
        });
        window.set_title(&self.title);
        let scene = View::scene(&mut self.ui, window);
        if let (Some(console), Some(frame)) = (
            self.session.as_mut(),
            self.ui.widget_ref::<Slot>(self.slot).map(Widget::frame),
        ) {
            console.place(frame, window.scale());
        }
        window.set_header(Header {
            title_x: HEADER_LEADING,
            items: Vec::new(),
            tabs: Vec::new(),
        });
        scene
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.session.iter().map(Console::fd).collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) -> bool {
        let Some(console) = self.session.as_mut() else {
            return false;
        };
        let changed = match console.pump(window, &mut self.ui) {
            Outcome::Nothing => false,
            Outcome::Redraw => true,
            Outcome::Show(view) => {
                self.show(view);
                true
            }
            Outcome::NoPicture => {
                self.show(message("machine-display-no-picture"));
                true
            }
            Outcome::Closed(_) => {
                self.session = None;
                self.show(message("machine-console-off"));
                true
            }
        };
        window.set_cursor(self.ui.cursor());
        changed
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        View::event(&mut self.ui, window, event)
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn deadline(&self) -> Option<Instant> {
        let console = self.session.as_ref().and_then(Console::deadline);
        match (self.ui.deadline(), console) {
            (Some(ui), Some(console)) => Some(ui.min(console)),
            (ui, console) => ui.or(console),
        }
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        if let Some(console) = &mut self.session {
            console.tick(now);
        }
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
