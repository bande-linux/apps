use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_calculator::{
    Angle, Base, Engine, Keypad, L10N, Mode, WORD_SIZES, base_name, mode_name, parse_base,
    parse_mode, restore, save,
};
use bande_toolkit::ui::{Context, Event, Key, TextView, Ui, Widget, WidgetId};
use bande_toolkit::{
    Appearance, Rect, Scene, Sizing, TextSystem, Window, WindowError, WindowEvent,
};
use bande_xdg::replace_file;

const APP_ID: &str = "org.bande.Calculator";
const STATE_FILE: &str = "calculator";
const TAPE_SIZE: (u32, u32) = (300, 420);
const MODE_ACTION: &str = "mode-";
const BASE_ACTION: &str = "base-";
const WORD_ACTION: &str = "word-";
const RPN: &str = "rpn";
const GROUPING: &str = "grouping";
const DEGREES: &str = "angle-degrees";
const RADIANS: &str = "angle-radians";
const PAPER_TAPE: &str = "paper-tape";
const CLEAR_TAPE: &str = "clear-tape";
const SHOW_CALCULATOR: &str = "show-calculator";
const BASE_SHORTCUTS: [&str; 4] = [
    "Shift+Ctrl+H",
    "Shift+Ctrl+D",
    "Shift+Ctrl+O",
    "Shift+Ctrl+B",
];

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

enum Pane {
    Keypad {
        ui: Ui,
        engine: Rc<RefCell<Engine>>,
    },
    Tape {
        ui: Ui,
        view: WidgetId,
        shown: String,
    },
}

impl Pane {
    fn ui(&mut self) -> &mut Ui {
        match self {
            Self::Keypad { ui, .. } | Self::Tape { ui, .. } => ui,
        }
    }

    fn ui_ref(&self) -> &Ui {
        match self {
            Self::Keypad { ui, .. } | Self::Tape { ui, .. } => ui,
        }
    }
}

fn pastes(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::Input(Event::Key {
            key: Key::Character('v'),
            modifiers,
        }) if modifiers.control && !modifiers.shift && !modifiers.alt
    )
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui().scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let Self::Keypad { engine, .. } = self {
            if pastes(&event) {
                if let Err(error) = window.paste() {
                    tracing::warn!("cannot paste: {error}");
                }
                return true;
            }
            if let WindowEvent::Pasted(text) = &event {
                return engine.borrow_mut().paste(text);
            }
        }
        window
            .drive(self.ui(), event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none())
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui().set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui().set_window_active(active);
    }

    fn animating(&self) -> bool {
        self.ui_ref().is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui_ref().accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui_ref().text_input_area()
    }
}

fn window_size(mode: Mode) -> (u32, u32) {
    match mode {
        Mode::Basic => (240, 380),
        Mode::Scientific => (620, 380),
        Mode::Programmer => (500, 480),
    }
}

fn new_ui(root: Box<dyn Widget>, appearance: Appearance) -> Ui {
    let id = root.id();
    let mut ui = Ui::new(root, Context::new(TextSystem::new(), appearance.palette()));
    ui.set_appearance(appearance);
    ui.set_focus(id);
    ui
}

fn tape_text(engine: &Engine) -> String {
    engine.tape().join("\n")
}

struct Calculator {
    engine: Rc<RefCell<Engine>>,
    main: Option<WindowId>,
    tape: Option<WindowId>,
    saved: String,
    had_tape: bool,
    quitting: bool,
}

impl Calculator {
    fn open_main(&mut self, cx: &mut Cx<Self>) {
        let mode = self.engine.borrow().mode;
        let engine = self.engine.clone();
        let opened = cx.open(
            Kind::Main,
            &L10N.tr("app-name"),
            window_size(mode),
            |appearance| Pane::Keypad {
                ui: new_ui(Box::new(Keypad::new(engine.clone())), appearance),
                engine,
            },
        );
        match opened {
            Ok(main) => {
                self.main = Some(main);
                self.fit_main(cx);
            }
            Err(error) => tracing::warn!("cannot open the calculator: {error}"),
        }
    }

    fn fit_main(&mut self, cx: &mut Cx<Self>) {
        let mode = self.engine.borrow().mode;
        if let Some((window, _)) = self.main.and_then(|main| cx.window(main)) {
            window.resize_content(window_size(mode), Sizing::Fixed);
        }
    }

    fn open_tape(&mut self, cx: &mut Cx<Self>) -> Result<(), WindowError> {
        let shown = tape_text(&self.engine.borrow());
        let tape = cx.open(
            Kind::Utility("tape"),
            &L10N.tr("tape-title"),
            TAPE_SIZE,
            |appearance| {
                let mut text = TextView::new(&L10N.tr("tape-empty"), &shown, |_| {})
                    .read_only()
                    .borderless();
                text.select(shown.len()..shown.len());
                let view = text.id();
                Pane::Tape {
                    ui: new_ui(Box::new(text), appearance),
                    view,
                    shown,
                }
            },
        )?;
        self.tape = Some(tape);
        Ok(())
    }

    fn switch(&mut self, cx: &mut Cx<Self>, mode: Mode) {
        if self.engine.borrow().mode == mode && self.main.is_some() {
            return;
        }
        self.engine.borrow_mut().set_mode(mode);
        match self.main {
            Some(_) => self.fit_main(cx),
            None => self.open_main(cx),
        }
    }

    fn toggle_tape(&mut self, cx: &mut Cx<Self>) {
        match self.tape.take() {
            Some(tape) => cx.close(tape),
            None => {
                if let Err(error) = self.open_tape(cx) {
                    tracing::warn!("cannot open the paper tape: {error}");
                }
            }
        }
    }

    fn set(&mut self, command: &str) {
        let mut engine = self.engine.borrow_mut();
        match command {
            CLEAR_TAPE => engine.clear_tape(),
            RPN => {
                let rpn = !engine.rpn;
                engine.set_rpn(rpn);
            }
            GROUPING => engine.grouping = !engine.grouping,
            DEGREES => engine.angle = Angle::Degrees,
            RADIANS => engine.angle = Angle::Radians,
            command => {
                if let Some(base) = command.strip_prefix(BASE_ACTION).and_then(parse_base) {
                    engine.set_base(base);
                } else if let Some(bits) = command
                    .strip_prefix(WORD_ACTION)
                    .and_then(|bits| bits.parse().ok())
                    .filter(|bits| WORD_SIZES.contains(bits))
                {
                    engine.set_bits(bits);
                }
            }
        }
    }

    fn refresh(&mut self, cx: &mut Cx<Self>) {
        let text = tape_text(&self.engine.borrow());
        let has_tape = !text.is_empty();
        if let Some((window, Pane::Tape { ui, view, shown })) =
            self.tape.and_then(|tape| cx.window(tape))
            && *shown != text
        {
            ui.update_widget::<TextView>(*view, |view, _| {
                view.set_text(&text);
                view.select(text.len()..text.len());
            });
            *shown = text;
            window.request_redraw();
        }
        if let Some((window, _)) = self.main.and_then(|main| cx.window(main)) {
            window.request_redraw();
        }
        let state = save(&self.engine.borrow(), self.tape.is_some());
        if state != self.saved {
            if let Some(file) = cx.state_file(STATE_FILE)
                && let Err(error) = replace_file(&file, &state)
            {
                tracing::warn!("cannot remember the calculator's settings: {error}");
            }
            self.saved = state;
            cx.refresh_menus();
        }
        if has_tape != self.had_tape {
            self.had_tape = has_tape;
            cx.refresh_menus();
        }
    }

    fn view_menu(&self) -> Entry {
        let engine = self.engine.borrow();
        let mode = |mode: Mode, message: &str, shortcut: &str| {
            Entry::item(
                &L10N.tr(message),
                &format!("{MODE_ACTION}{}", mode_name(mode)),
            )
            .shortcut(shortcut)
            .radio(engine.mode == mode)
        };
        let scientific = engine.mode == Mode::Scientific;
        let programmer = engine.mode == Mode::Programmer;
        let mut entries = vec![
            mode(Mode::Basic, "menu-basic", "Ctrl+1"),
            mode(Mode::Scientific, "menu-scientific", "Ctrl+2"),
            mode(Mode::Programmer, "menu-programmer", "Ctrl+3"),
            Entry::separator(),
            Entry::item(&L10N.tr("menu-rpn"), RPN)
                .shortcut("Ctrl+R")
                .radio(engine.rpn),
            Entry::item(&L10N.tr("menu-grouping"), GROUPING).radio(engine.grouping),
            Entry::separator(),
            Entry::item(&L10N.tr("menu-degrees"), DEGREES)
                .radio(engine.angle == Angle::Degrees)
                .enabled(scientific),
            Entry::item(&L10N.tr("menu-radians"), RADIANS)
                .radio(engine.angle == Angle::Radians)
                .enabled(scientific),
            Entry::separator(),
        ];
        entries.extend(
            Base::ALL
                .into_iter()
                .zip(BASE_SHORTCUTS)
                .map(|(base, shortcut)| {
                    Entry::item(
                        &L10N.tr(&format!("menu-{}", base_name(base))),
                        &format!("{BASE_ACTION}{}", base_name(base)),
                    )
                    .shortcut(shortcut)
                    .radio(engine.base == base)
                    .enabled(programmer)
                }),
        );
        entries.push(
            Entry::submenu(
                &L10N.tr("menu-word-size"),
                WORD_SIZES
                    .iter()
                    .map(|bits| {
                        Entry::item(
                            &L10N.tr_args("menu-bits", [("bits", bits.to_string().into())]),
                            &format!("{WORD_ACTION}{bits}"),
                        )
                        .radio(engine.bits == *bits)
                    })
                    .collect(),
            )
            .enabled(programmer),
        );
        Entry::submenu(&L10N.tr("menu-view"), entries)
    }
}

impl Program for Calculator {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut engine = Engine::new();
        let tape = cx
            .state_file(STATE_FILE)
            .and_then(|file| std::fs::read_to_string(file).ok())
            .is_some_and(|text| restore(&mut engine, &text));
        let saved = save(&engine, tape);
        let mut calculator = Self {
            engine: Rc::new(RefCell::new(engine)),
            main: None,
            tape: None,
            saved,
            had_tape: false,
            quitting: false,
        };
        calculator.open_main(cx);
        if calculator.main.is_none() {
            return Err(L10N.tr("cannot-open").into());
        }
        if tape && let Err(error) = calculator.open_tape(cx) {
            tracing::warn!("cannot open the paper tape: {error}");
        }
        Ok(calculator)
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        let has_tape = !self.engine.borrow().tape().is_empty();
        Menus {
            menus: vec![
                edit_submenu(vec![
                    Entry::item(&L10N.tr("menu-clear-tape"), CLEAR_TAPE).enabled(has_tape),
                ]),
                self.view_menu(),
            ],
            window: vec![
                Entry::item(&L10N.tr("app-name"), SHOW_CALCULATOR),
                Entry::item(&L10N.tr("tape-title"), PAPER_TAPE)
                    .shortcut("Ctrl+T")
                    .radio(self.tape.is_some()),
            ],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        match command {
            PAPER_TAPE => self.toggle_tape(cx),
            SHOW_CALCULATOR => match self.main {
                Some(main) => cx.raise(main, None),
                None => self.open_main(cx),
            },
            command => match command.strip_prefix(MODE_ACTION).and_then(parse_mode) {
                Some(mode) => self.switch(cx, mode),
                None => {
                    self.set(command);
                    if self.main.is_none() && command != CLEAR_TAPE {
                        self.open_main(cx);
                    }
                }
            },
        }
        self.refresh(cx);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if self.main.is_none() {
            self.open_main(cx);
        }
    }

    fn window_closed(&mut self, cx: &mut Cx<Self>, window: WindowId, _view: Pane) {
        if self.main == Some(window) {
            self.main = None;
        }
        if self.tape == Some(window) {
            self.tape = None;
        }
        if !self.quitting {
            self.refresh(cx);
        }
    }

    fn should_quit(&mut self, _cx: &mut Cx<Self>) -> bool {
        self.quitting = true;
        true
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.refresh(cx);
    }
}

fn main() -> ExitCode {
    bande_app::run::<Calculator>(&INFO)
}
