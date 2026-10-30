#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty::Shell;
use unbound_term::{
    Axis, BANNER_HEIGHT, Banner, BannerAction, CellPoint, Command, CursorStyle, DEFAULT_FONT_SIZE,
    GridSize, HEADER_LEADING, Layout, MenuCommand, MouseButton, MouseKind, Notice, Painter, Pane,
    Profile, Report, Reports, SETTINGS_SIZE, STRIP_HEIGHT, SavedWindow, SearchDirection,
    SelectionKind, Session, Settings, Side, TabBar, TabBarLayout, TabTarget, Terminal,
    TerminalContext, bracketed, claim, encode_key, is_openable, left_behind, mouse_report,
    next_font_size, settings_view, shortcut, tab_menu, terminal_menu, window_file,
};
use unbound_toolkit::ui::{Key, Modifiers};
use unbound_toolkit::{
    Appearance, CpuRenderer, Header, Item, Palette, Point, Radius, Rect, SHEET_INSET, Scene,
    Scheme, Shape, framed_scene,
};

const MIN_SSIM: f64 = 0.99;

fn plain() -> Modifiers {
    Modifiers::default()
}

fn control() -> Modifiers {
    Modifiers {
        control: true,
        ..Modifiers::default()
    }
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::default()
    }
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

fn keys(key: Key, modifiers: Modifiers, mode: TermMode) -> Vec<u8> {
    encode_key(key, modifiers, mode).expect("bytes")
}

#[test]
fn keys_are_sent_as_xterm_sequences() {
    let normal = TermMode::empty();
    assert_eq!(keys(Key::Up, plain(), normal), b"\x1b[A");
    assert_eq!(keys(Key::Up, plain(), TermMode::APP_CURSOR), b"\x1bOA");
    assert_eq!(keys(Key::Up, control(), normal), b"\x1b[1;5A");
    assert_eq!(keys(Key::Right, shift(), normal), b"\x1b[1;2C");
    assert_eq!(keys(Key::Home, plain(), normal), b"\x1b[H");
    assert_eq!(keys(Key::Delete, plain(), normal), b"\x1b[3~");
    assert_eq!(keys(Key::PageDown, control(), normal), b"\x1b[6;5~");
    assert_eq!(keys(Key::Function(1), plain(), normal), b"\x1bOP");
    assert_eq!(keys(Key::Function(5), plain(), normal), b"\x1b[15~");
    assert_eq!(keys(Key::Function(12), plain(), normal), b"\x1b[24~");
    assert_eq!(keys(Key::Tab, shift(), normal), b"\x1b[Z");
    assert_eq!(keys(Key::Enter, plain(), normal), b"\r");
    assert_eq!(keys(Key::Backspace, plain(), normal), b"\x7f");
    assert_eq!(keys(Key::Escape, plain(), normal), b"\x1b");
}

#[test]
fn control_sends_signals_and_super_is_kept_for_the_app() {
    let normal = TermMode::empty();
    assert_eq!(keys(Key::Character('c'), control(), normal), [0x03]);
    assert_eq!(keys(Key::Character('d'), control(), normal), [0x04]);
    assert_eq!(keys(Key::Character('z'), control(), normal), [0x1a]);
    assert_eq!(keys(Key::Character('['), control(), normal), [0x1b]);
    assert_eq!(keys(Key::Space, control(), normal), [0x00]);
    assert_eq!(keys(Key::Character('é'), plain(), normal), "é".as_bytes());
    let logo = Modifiers {
        logo: true,
        ..Modifiers::default()
    };
    assert_eq!(encode_key(Key::Character('c'), logo, normal), None);
    assert_eq!(encode_key(Key::Character('v'), logo, normal), None);
}

fn logo(shift: bool) -> Modifiers {
    Modifiers {
        logo: true,
        shift,
        ..Modifiers::default()
    }
}

#[test]
fn super_chords_are_app_commands_whatever_the_shift_state_of_plus() {
    let bigger = Some(Command::BiggerText);
    assert_eq!(shortcut(Key::Character('='), logo(false)), bigger);
    assert_eq!(shortcut(Key::Character('+'), logo(true)), bigger);
    assert_eq!(
        shortcut(Key::Character('-'), logo(false)),
        Some(Command::SmallerText)
    );
    assert_eq!(
        shortcut(Key::Character('0'), logo(false)),
        Some(Command::DefaultTextSize)
    );
    assert_eq!(
        shortcut(Key::Character('C'), logo(false)),
        Some(Command::Copy)
    );
    assert_eq!(shortcut(Key::Character('c'), control()), None);
}

#[test]
fn text_size_steps_through_a_ladder_and_stops_at_its_ends() {
    assert_eq!(next_font_size(DEFAULT_FONT_SIZE, true), 14.0);
    assert_eq!(next_font_size(DEFAULT_FONT_SIZE, false), 12.0);
    assert_eq!(next_font_size(8.0, false), 8.0);
    assert_eq!(next_font_size(40.0, true), 40.0);
    assert_eq!(next_font_size(12.5, true), 13.0);
}

#[test]
fn the_mouse_is_reported_in_the_encoding_and_detail_the_program_asked_for() {
    let left = MouseButton::Left;
    let report = |kind, mode| mouse_report(kind, 4, 9, plain(), mode);
    assert_eq!(report(MouseKind::Press(left), TermMode::empty()), None);

    let clicks = TermMode::MOUSE_REPORT_CLICK;
    assert_eq!(
        report(MouseKind::Press(left), clicks).unwrap(),
        b"\x1b[M\x20\x25\x2a"
    );
    assert_eq!(
        report(MouseKind::Release(left), clicks).unwrap(),
        b"\x1b[M\x23\x25\x2a"
    );
    assert_eq!(report(MouseKind::Drag(left), clicks), None);
    assert_eq!(
        mouse_report(MouseKind::Press(left), 4, 9, control(), clicks).unwrap(),
        b"\x1b[M\x30\x25\x2a"
    );

    let sgr = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
    assert_eq!(
        report(MouseKind::Press(left), sgr).unwrap(),
        b"\x1b[<0;5;10M"
    );
    assert_eq!(
        report(MouseKind::Release(left), sgr).unwrap(),
        b"\x1b[<0;5;10m"
    );
    assert_eq!(
        report(MouseKind::Drag(left), sgr).unwrap(),
        b"\x1b[<32;5;10M"
    );
    assert_eq!(report(MouseKind::Move, sgr), None);
    assert_eq!(
        report(MouseKind::Press(MouseButton::WheelDown), sgr).unwrap(),
        b"\x1b[<65;5;10M"
    );
    assert_eq!(
        report(
            MouseKind::Move,
            TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE
        )
        .unwrap(),
        b"\x1b[<35;5;10M"
    );

    let far = mouse_report(
        MouseKind::Press(left),
        299,
        0,
        plain(),
        clicks | TermMode::UTF8_MOUSE,
    )
    .unwrap();
    assert_eq!(far, "\x1b[M\u{20}\u{14c}\u{21}".as_bytes());
}

#[test]
fn alt_moves_by_word_and_prefixes_escape() {
    let normal = TermMode::empty();
    assert_eq!(keys(Key::Left, alt(), normal), b"\x1bb");
    assert_eq!(keys(Key::Right, alt(), normal), b"\x1bf");
    assert_eq!(keys(Key::Character('d'), alt(), normal), b"\x1bd");
    assert_eq!(keys(Key::Backspace, alt(), normal), b"\x1b\x7f");
}

#[test]
fn pastes_are_bracketed_when_the_program_asks() {
    assert_eq!(
        bracketed("echo a\nb", TermMode::BRACKETED_PASTE),
        b"\x1b[200~echo a\nb\x1b[201~"
    );
    assert_eq!(
        bracketed("x\x1b[201~y", TermMode::BRACKETED_PASTE),
        b"\x1b[200~x[201~y\x1b[201~"
    );
    assert_eq!(bracketed("echo a\nb", TermMode::empty()), b"echo a\rb");
}

fn terminal(columns: usize, lines: usize) -> Terminal {
    Terminal::new(GridSize { columns, lines })
}

fn appearance(scheme: Scheme) -> Appearance {
    Appearance {
        scheme,
        ..Appearance::default()
    }
}

fn painter(scheme: Scheme) -> Painter {
    Painter::new(appearance(scheme), Profile::Datum, DEFAULT_FONT_SIZE, 1.0)
}

const SCHEMES: [(Scheme, &str); 2] = [(Scheme::Light, "light"), (Scheme::Dark, "dark")];

fn shoot(name: &str, scene: &Scene) {
    for (scale, suffix) in [(1.0, ""), (1.5, "@1.5x")] {
        let width = (f64::from(scene.width) * scale).round() as u32;
        let height = (f64::from(scene.height) * scale).round() as u32;
        let mut rgba = vec![0; (width * height * 4) as usize];
        CpuRenderer::new()
            .render_into(scene, scale, &mut rgba)
            .expect("render");
        check_golden(&format!("{name}{suffix}"), (width, height), rgba);
    }
}

#[test]
fn output_lands_on_the_grid_and_scrolls_into_the_scrollback() {
    let mut terminal = terminal(20, 3);
    terminal.feed(b"one\r\ntwo\r\nthree\r\nfour");

    assert_eq!(terminal.screen_text(), ["two", "three", "four"]);

    terminal.scroll_lines(1);
    assert_eq!(terminal.screen_text(), ["one", "two", "three"]);

    terminal.scroll_to_bottom();
    terminal.clear_scrollback();
    terminal.scroll_lines(1);
    assert_eq!(terminal.screen_text(), ["two", "three", "four"]);
}

#[test]
fn a_drag_selects_text_for_copying() {
    let mut terminal = terminal(20, 3);
    terminal.feed(b"hello world");
    let metrics = painter(Scheme::Light).metrics;
    let at = |column: f32| {
        metrics.cell_at(Point::new(
            unbound_term::PADDING + column * metrics.cell_width + 1.0,
            unbound_term::PADDING + metrics.cell_height / 2.0,
        ))
    };

    terminal.start_selection(at(6.0), SelectionKind::Characters);
    terminal.extend_selection(at(10.6));
    terminal.finish_selection();

    assert_eq!(terminal.selected_text().as_deref(), Some("world"));

    terminal.start_selection(at(2.0), SelectionKind::Characters);
    terminal.finish_selection();
    assert_eq!(terminal.selected_text(), None);
}

fn cell(column: usize, line: usize) -> CellPoint {
    CellPoint {
        column,
        line,
        side: alacritty_terminal::index::Side::Left,
    }
}

#[test]
fn two_clicks_select_a_word_three_a_line_and_alt_drag_a_block() {
    let mut terminal = terminal(20, 3);
    terminal.feed(b"cargo build --release\r\nsrc/main.rs ok\r\nabc def");
    terminal.start_selection(cell(2, 1), SelectionKind::for_clicks(2, false));
    terminal.finish_selection();
    assert_eq!(terminal.selected_text().as_deref(), Some("src/main.rs"));

    terminal.start_selection(cell(14, 1), SelectionKind::for_clicks(3, false));
    terminal.finish_selection();
    assert_eq!(
        terminal.selected_text().as_deref(),
        Some("src/main.rs ok\n")
    );

    terminal.start_selection(cell(1, 1), SelectionKind::for_clicks(1, true));
    terminal.extend_selection(CellPoint {
        side: alacritty_terminal::index::Side::Right,
        ..cell(2, 2)
    });
    assert_eq!(terminal.selected_text().as_deref(), Some("rc\nbc"));
}

#[test]
fn select_all_copies_the_scrollback_and_the_screen() {
    let mut terminal = terminal(20, 2);
    terminal.feed(b"one\r\ntwo\r\nthree");
    terminal.select_all();
    assert_eq!(terminal.selected_text().as_deref(), Some("one\ntwo\nthree"));
}

fn wait_until(session: &mut Session, what: &str, done: impl Fn(&Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(session) {
        assert!(
            Instant::now() < deadline,
            "{what}: {:?}",
            session.terminal.screen_text()
        );
        session.read();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_shell_runs_on_a_pty_and_reports_its_folder_and_running_program() {
    let start = std::env::temp_dir().canonicalize().unwrap();
    let script = "printf 'ready %s' \"$TERM\"; set -m; read line; \
                  printf '\\033]7;file://localhost/usr/sh%%61re\\007'; sleep 30";
    let mut session = Session::spawn(
        terminal(40, 5),
        Some(Shell::new(
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), script.to_owned()],
        )),
        Some(start.clone()),
    )
    .expect("session");
    wait_until(&mut session, "the shell starts", |session| {
        session.terminal.screen_text()[0].contains("ready xterm-256color")
    });
    assert_eq!(session.working_directory(), Some(start));
    assert_eq!(session.running_program(), None);

    session.send(b"\r");
    wait_until(
        &mut session,
        "sleep runs in the reported folder",
        |session| {
            session.running_program() == Some("sleep")
                && session.working_directory() == Some(PathBuf::from("/usr/share"))
        },
    );
    assert_eq!(
        session.working_directory(),
        Some(PathBuf::from("/usr/share"))
    );
    assert_eq!(session.title(None), "share — sleep");
}

fn directory(reports: Vec<Report>) -> Option<PathBuf> {
    reports.into_iter().find_map(|report| match report {
        Report::Directory(directory) => Some(directory),
        _ => None,
    })
}

#[test]
fn folder_reports_are_found_across_reads_and_only_for_this_machine() {
    let mut reports = Reports::new(Some("unbound".to_owned()));
    assert_eq!(directory(reports.scan(b"ls\r\n\x1b]7;file://unb")), None);
    assert_eq!(
        directory(reports.scan(b"ound/home/me/My%20Files\x1b\\$ ")),
        Some(PathBuf::from("/home/me/My Files"))
    );
    assert_eq!(
        directory(reports.scan(b"\x1b]7;file:///tmp\x07")),
        Some(PathBuf::from("/tmp"))
    );
    assert_eq!(
        directory(reports.scan(b"\x1b]7;file://elsewhere/tmp\x07")),
        None
    );
    assert_eq!(directory(reports.scan(b"\x1b]2;file:///tmp\x07")), None);
}

#[test]
fn prompt_marks_and_notifications_are_reported() {
    let mut reports = Reports::new(None);
    assert_eq!(
        reports.scan(b"\x1b]133;A\x07$ make\x1b]133;C\x07building"),
        [Report::CommandStarted]
    );
    assert_eq!(
        reports.scan(b"done\x1b]133;D;2\x1b\\\x1b]777;notify;Build;All targets\x07"),
        [
            Report::CommandFinished(Some(2)),
            Report::Notify {
                title: "Build".to_owned(),
                body: "All targets".to_owned()
            }
        ]
    );
    assert_eq!(
        reports.scan(b"\x1b]133;D\x07"),
        [Report::CommandFinished(None)]
    );
}

#[test]
fn the_tab_strip_shares_the_header_after_the_window_controls_and_keeps_room_for_new_tab() {
    let layout = TabBarLayout::new(3, 700.0, HEADER_LEADING);
    assert_eq!(layout.tabs.len(), 3);
    assert_eq!(layout.tabs[0].x, HEADER_LEADING);
    assert!(layout.tabs[2].x + layout.tabs[2].width < layout.new_tab.x);
    let middle = layout.tabs[1];
    let inside = Point::new(middle.x + middle.width / 2.0, STRIP_HEIGHT / 2.0);
    assert_eq!(layout.hit(inside), Some(TabTarget::Select(1)));
    assert_eq!(
        layout.hit(Point::new(middle.x + 10.0, STRIP_HEIGHT / 2.0)),
        Some(TabTarget::Close(1))
    );
    assert_eq!(
        layout.hit(Point::new(layout.new_tab.x + 4.0, layout.new_tab.y + 4.0)),
        Some(TabTarget::NewTab)
    );
    assert_eq!(layout.hit(Point::new(40.0, STRIP_HEIGHT / 2.0)), None);

    let alone = TabBarLayout::new(1, 700.0, HEADER_LEADING);
    assert!(alone.tabs.is_empty());
    assert_eq!(alone.items(), [alone.new_tab]);
}

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden directory")).expect("directory");
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

const SESSION: &[u8] = b"\x1b[1mmikael@unbound\x1b[0m:\x1b[34m~\x1b[0m$ ls --color\r\n\
\x1b[01;34mDesktop\x1b[0m  \x1b[01;34mDocuments\x1b[0m  notes.txt  \x1b[01;32mbuild.sh\x1b[0m\r\n\
\x1b[1mmikael@unbound\x1b[0m:\x1b[34m~\x1b[0m$ echo \x1b[7mselected\x1b[0m \x1b[4munderlined\x1b[0m \x1b[31mred\x1b[0m \x1b[38;5;208m256\x1b[0m \x1b[38;2;0;128;128mtruecolor\x1b[0m\r\n\
selected underlined red 256 truecolor\r\n\
\x1b[1mmikael@unbound\x1b[0m:\x1b[34m~\x1b[0m$ ";

#[test]
fn the_terminal_matches_its_goldens_focused_and_unfocused() {
    for (scheme, suffix) in SCHEMES {
        let mut painter = painter(scheme);
        let mut terminal = terminal(80, 8);
        terminal.feed(SESSION);
        let (width, height) = painter.metrics.content_size(terminal.size());
        let size = (width.ceil() as u32, height.ceil() as u32);
        terminal.set_focus(true);
        shoot(
            &format!("term-focused-{suffix}"),
            &painter.scene(&terminal, size.0, size.1),
        );
        terminal.set_focus(false);
        shoot(
            &format!("term-unfocused-{suffix}"),
            &painter.scene(&terminal, size.0, size.1),
        );
    }
}

#[test]
fn every_profile_keeps_text_and_ansi_colours_legible() {
    for profile in Profile::ALL {
        for (scheme, _) in SCHEMES {
            let theme = unbound_term::Theme::new(appearance(scheme), profile);
            let contrast = unbound_design::wcag(theme.foreground, theme.background);
            assert!(contrast >= 7.0, "{profile:?} {scheme:?}: {contrast}");
            for index in 1..7 {
                let color = unbound_term::ansi_colors(&theme.palette)[index];
                let contrast = unbound_design::wcag(color, theme.background);
                assert!(
                    contrast >= 3.0,
                    "{profile:?} {scheme:?} {index}: {contrast}"
                );
            }
        }
    }
}

fn window_scene(painter: &mut Painter, titles: &[String], bells: &[bool], width: f32) -> Scene {
    let mut terminal = terminal(72, 9);
    terminal.feed(SESSION);
    terminal.set_focus(true);
    let (_, height) = painter.metrics.content_size(terminal.size());
    let sheet = Rect::new(
        SHEET_INSET as f32,
        STRIP_HEIGHT,
        width - 2.0 * SHEET_INSET as f32,
        height.ceil(),
    );
    let bar = TabBar {
        titles,
        bells,
        active: 1,
        hover: Some(TabTarget::Close(2)),
        focused: true,
        leading: HEADER_LEADING,
    };
    let mut items = painter.tab_bar(&bar, width);
    items.push(Item::Clip {
        shape: Shape::rounded(sheet, Radius::concentric(Radius::XL, SHEET_INSET as f32)),
        items: painter.terminal(&terminal, Point::new(sheet.x, sheet.y), &Default::default()),
    });
    Scene {
        width: width as u32,
        height: (sheet.y + sheet.height + SHEET_INSET as f32) as u32,
        background: painter.theme.background,
        items,
    }
}

#[test]
fn the_window_with_tabs_in_its_header_matches_its_goldens() {
    let titles = [
        "~ — bash".to_owned(),
        "unbound — cargo build --release --workspace --all-targets".to_owned(),
        "src — vim".to_owned(),
    ];
    let bells = [true, false, false];
    for (scheme, suffix) in SCHEMES {
        let mut painter = painter(scheme);
        let width = 720.0;
        let content = window_scene(&mut painter, &titles, &bells, width);
        let layout = TabBarLayout::new(titles.len(), width, HEADER_LEADING);
        let framed = framed_scene(
            &content,
            "",
            Some(Default::default()),
            Some(Header {
                title_x: width,
                items: layout.items(),
            }),
            &Palette::for_scheme(scheme),
        );
        shoot(&format!("term-window-{suffix}"), &framed);
    }
}

#[test]
fn find_starts_at_the_newest_match_and_steps_through_the_scrollback() {
    let mut terminal = terminal(30, 3);
    terminal.feed(b"error: one\r\nok\r\nerror: two (x+1)\r\nok\r\nERROR: three\r\nok");
    assert!(terminal.find("error"));
    let line_of = |terminal: &mut Terminal| terminal.highlights().current.unwrap().start().line.0;
    assert_eq!(line_of(&mut terminal), 1);
    assert_eq!(terminal.highlights().matches.len(), 1);

    assert!(terminal.find_again(SearchDirection::Older));
    assert_eq!(line_of(&mut terminal), -1);
    assert_eq!(terminal.screen_text()[0], "error: two (x+1)");
    assert!(terminal.find_again(SearchDirection::Older));
    assert_eq!(line_of(&mut terminal), -3);
    assert!(terminal.find_again(SearchDirection::Newer));
    assert_eq!(line_of(&mut terminal), -1);

    assert!(terminal.find("(x+1)"));
    assert!(!terminal.find("Error"));
    assert!(!terminal.find("missing"));
    terminal.end_find();
    assert_eq!(terminal.highlights().matches.len(), 0);
}

#[test]
fn the_find_bar_reports_typing_and_its_keys() {
    let mut bar = Banner::find(unbound_toolkit::Palette::for_scheme(Scheme::Light), "old");
    bar.handle(&unbound_toolkit::ui::Event::Key {
        key: Key::Character('n'),
        modifiers: plain(),
    });
    assert_eq!(bar.query(), "n");
    bar.handle(&unbound_toolkit::ui::Event::Key {
        key: Key::Enter,
        modifiers: plain(),
    });
    bar.handle(&unbound_toolkit::ui::Event::Key {
        key: Key::Enter,
        modifiers: shift(),
    });
    bar.handle(&unbound_toolkit::ui::Event::Key {
        key: Key::Escape,
        modifiers: plain(),
    });
    assert_eq!(
        bar.take_actions(),
        [
            BannerAction::QueryChanged,
            BannerAction::Older,
            BannerAction::Newer,
            BannerAction::Dismiss
        ]
    );
}

#[test]
fn found_text_and_the_find_bar_match_their_goldens() {
    for (scheme, suffix) in SCHEMES {
        let mut painter = painter(scheme);
        let mut terminal = terminal(60, 4);
        terminal.feed(SESSION);
        terminal.set_focus(false);
        assert!(terminal.find("unbound"));
        let mut bar = Banner::find(unbound_toolkit::Palette::for_scheme(scheme), "unbound");
        bar.set_found(true);
        let (width, height) = painter.metrics.content_size(terminal.size());
        let size = (width.ceil() as u32, (height + BANNER_HEIGHT).ceil() as u32);
        bar.set_area(Rect::new(0.0, 0.0, size.0 as f32, BANNER_HEIGHT));
        let highlights = terminal.highlights();
        let mut items = painter.terminal(&terminal, Point::new(0.0, BANNER_HEIGHT), &highlights);
        items.extend(bar.items(size.0, size.1));
        let scene = Scene {
            width: size.0,
            height: size.1,
            background: painter.theme.background,
            items,
        };
        shoot(&format!("term-find-{suffix}"), &scene);
    }
}

#[test]
fn links_are_found_in_plain_text_and_in_hyperlink_escapes() {
    let mut terminal = terminal(20, 4);
    terminal.feed(b"see (https://example.org/a_(b)). ok\r\n");
    terminal.feed(b"\x1b]8;;https://unbound.dev/docs\x1b\\the docs\x1b]8;;\x1b\\ here");
    let uri = |terminal: &mut Terminal, column, line| {
        terminal.link_at(cell(column, line)).map(|link| link.uri)
    };
    let example = Some("https://example.org/a_(b)".to_owned());
    assert_eq!(uri(&mut terminal, 8, 0), example);
    assert_eq!(uri(&mut terminal, 6, 1), example);
    assert_eq!(uri(&mut terminal, 9, 1), example);
    assert_eq!(uri(&mut terminal, 10, 1), None);
    assert_eq!(uri(&mut terminal, 1, 0), None);

    let link = terminal.link_at(cell(4, 2)).unwrap();
    assert_eq!(link.uri, "https://unbound.dev/docs");
    assert_eq!(link.range.start().column.0, 0);
    assert_eq!(link.range.end().column.0, 7);
    assert_eq!(uri(&mut terminal, 9, 2), None);

    assert!(is_openable("HTTPS://example.org"));
    assert!(is_openable("mailto:me@example.org"));
    assert!(!is_openable("javascript:alert(1)"));
}

#[test]
fn the_close_confirmation_answers_enter_and_escape_and_matches_its_goldens() {
    let message = "Close this tab? \u{201C}vim\u{201D} is still running in it.";
    for (scheme, suffix) in SCHEMES {
        let palette = Palette::for_scheme(scheme);
        let mut banner = Banner::confirm(palette, message, "Close");
        let size = (480, BANNER_HEIGHT as u32);
        banner.set_area(Rect::new(0.0, 0.0, 480.0, BANNER_HEIGHT));
        let scene = Scene {
            width: size.0,
            height: size.1,
            background: palette.sheet,
            items: banner.items(size.0, size.1),
        };
        shoot(&format!("term-confirm-{suffix}"), &scene);

        for key in [Key::Enter, Key::Escape] {
            banner.handle(&unbound_toolkit::ui::Event::Key {
                key,
                modifiers: plain(),
            });
        }
        assert_eq!(
            banner.take_actions(),
            [BannerAction::Confirm, BannerAction::Dismiss]
        );
    }
}

#[test]
fn precise_scrolling_moves_by_points_and_stops_at_both_ends_of_the_scrollback() {
    let mut terminal = terminal(20, 2);
    terminal.feed(b"one\r\ntwo\r\nthree\r\nfour");
    let line = 10.0;

    terminal.scroll_smoothly(4.0, line);
    assert_eq!(terminal.scroll_shift(), 4.0);
    assert_eq!(terminal.screen_text(), ["three", "four"]);

    terminal.scroll_smoothly(7.0, line);
    assert_eq!(terminal.scroll_shift(), 1.0);
    assert_eq!(terminal.screen_text(), ["two", "three"]);

    terminal.scroll_smoothly(25.0, line);
    assert_eq!(terminal.scroll_shift(), 0.0);
    assert_eq!(terminal.screen_text(), ["one", "two"]);

    terminal.scroll_smoothly(-16.0, line);
    assert_eq!(terminal.scroll_shift(), 4.0);
    assert_eq!(terminal.screen_text(), ["three", "four"]);

    terminal.scroll_smoothly(-30.0, line);
    assert_eq!(terminal.scroll_shift(), 0.0);
    assert_eq!(terminal.screen_text(), ["three", "four"]);
}

#[test]
fn menu_actions_name_the_command_they_run() {
    let commands = [
        MenuCommand::Run(Command::Copy),
        MenuCommand::Run(Command::FindPrevious),
        MenuCommand::OpenLink("https://example.com/a:b?c=d".to_owned()),
        MenuCommand::CopyLink("mailto:someone@example.com".to_owned()),
        MenuCommand::ShowInFiles,
        MenuCommand::CloseTab(7),
        MenuCommand::CloseOtherTabs(3),
    ];
    for command in commands {
        assert_eq!(MenuCommand::parse(&command.action()), Some(command));
    }
    assert_eq!(MenuCommand::parse("close-tab:x"), None);
    assert_eq!(MenuCommand::parse("unknown"), None);
}

#[test]
fn the_terminal_menu_offers_the_link_under_the_pointer_and_copy_only_with_a_selection() {
    let mut context = TerminalContext {
        link: None,
        has_selection: false,
        knows_directory: true,
        tab: 4,
    };
    let plain = terminal_menu(&context);
    let copy = plain
        .items
        .iter()
        .find(|item| item.action == "copy")
        .expect("copy");
    assert!(!copy.enabled);
    assert!(
        !plain
            .items
            .iter()
            .any(|item| item.action.starts_with("open-link:"))
    );
    assert!(plain.items.iter().any(|item| item.action == "close-tab:4"));

    context.link = Some("https://example.com".to_owned());
    context.has_selection = true;
    let on_link = terminal_menu(&context);
    assert_eq!(on_link.items[0].action, "open-link:https://example.com");
    assert_eq!(on_link.items[1].action, "copy-link:https://example.com");
    assert!(
        on_link
            .items
            .iter()
            .any(|item| item.action == "copy" && item.enabled)
    );
}

#[test]
fn closing_other_tabs_is_offered_only_when_there_are_others() {
    let enabled = |menu: unbound_toolkit::ui::Menu| {
        menu.items
            .iter()
            .find(|item| item.action == "close-other-tabs:2")
            .map(|item| item.enabled)
    };
    assert_eq!(enabled(tab_menu(Some(2), 1)), Some(false));
    assert_eq!(enabled(tab_menu(Some(2), 3)), Some(true));
    assert_eq!(enabled(tab_menu(None, 3)), None);
}

#[test]
fn splitting_nests_panes_and_closing_one_gives_its_room_back() {
    let area = Rect::new(0.0, 0.0, 801.0, 401.0);
    let mut layout = Layout::Pane(1);
    assert!(layout.split(1, Axis::SideBySide, 2));
    assert!(layout.split(2, Axis::Stacked, 3));
    assert!(!layout.split(9, Axis::Stacked, 4));
    assert_eq!(layout.panes(), [1, 2, 3]);
    let frames = layout.frames(area);
    assert_eq!(frames[0], (1, Rect::new(0.0, 0.0, 400.0, 401.0)));
    assert_eq!(frames[1], (2, Rect::new(401.0, 0.0, 400.0, 200.0)));
    assert_eq!(frames[2], (3, Rect::new(401.0, 201.0, 400.0, 200.0)));
    assert_eq!(layout.neighbour(3, true), 1);
    assert_eq!(layout.neighbour(1, false), 3);

    let dividers = layout.dividers(area);
    assert_eq!(dividers.len(), 2);
    let inner = &dividers[1];
    assert_eq!(inner.path, [Side::Second]);
    let ratio = inner.ratio_at(Point::new(600.0, 100.0));
    layout.set_ratio(&inner.path, ratio);
    assert_eq!(layout.frames(area)[1].1.height, 100.0);
    assert_eq!(dividers[0].ratio_at(Point::new(-50.0, 0.0)), 0.1);

    assert!(layout.remove(2));
    assert_eq!(layout.panes(), [1, 3]);
    assert_eq!(
        layout.frames(area)[1],
        (3, Rect::new(401.0, 0.0, 400.0, 401.0))
    );
    assert!(layout.remove(1));
    assert_eq!(layout, Layout::Pane(3));
    assert!(!layout.remove(3));
}

#[test]
fn settings_keep_their_defaults_for_unknown_or_broken_values_and_round_trip() {
    let settings = Settings::parse(
        "profile = ink\nfont-size = 99\ncursor = bar\nflash-on-bell = false\nshape = round\nnotify-finished = maybe\n",
    );
    assert_eq!(settings.profile, Profile::Ink);
    assert_eq!(settings.font_size, Settings::default().font_size);
    assert_eq!(settings.cursor, CursorStyle::Bar);
    assert!(!settings.flash_on_bell);
    assert!(settings.notify_finished);
    assert_eq!(Settings::parse(&settings.to_text()), settings);
}

#[test]
fn windows_left_behind_are_restored_once_and_running_ones_are_left_alone() {
    let folder = tempfile::tempdir().expect("folder");
    let saved = SavedWindow {
        tabs: vec![PathBuf::from("/tmp"), PathBuf::from("/usr/share")],
        active: 1,
    };
    saved.save(&window_file(folder.path(), 10)).expect("save");
    saved.save(&window_file(folder.path(), 11)).expect("save");
    std::fs::write(folder.path().join("notes.txt"), "x").expect("write");
    let left = left_behind(folder.path(), |pid| pid == 11);
    assert_eq!(left, [window_file(folder.path(), 10)]);
    let mine = window_file(folder.path(), 12);
    assert_eq!(claim(&left[0], &mine), Some(saved));
    assert_eq!(claim(&left[0], &mine), None);
    assert!(left_behind(folder.path(), |pid| pid == 11 || pid == 12).is_empty());
    assert_eq!(SavedWindow::parse("active 5\ntab relative\n"), None);
    assert_eq!(
        SavedWindow::parse("active 5\ntab /srv\n").map(|window| window.active),
        Some(0)
    );
}

#[test]
fn a_command_that_finishes_is_reported_with_its_name() {
    let script = "set -m; read line; sleep 0.2; read line";
    let mut session = Session::spawn(
        terminal(40, 5),
        Some(Shell::new(
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), script.to_owned()],
        )),
        None,
    )
    .expect("session");
    std::thread::sleep(Duration::from_millis(100));
    session.read();
    session.send(b"\r");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        seen.extend(session.read());
        if seen
            .iter()
            .any(|notice| matches!(notice, Notice::Finished { command, .. } if command == "sleep"))
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no finished command among {seen:?}");
}

#[test]
fn the_settings_window_matches_its_goldens() {
    for (scheme, suffix) in SCHEMES {
        let settings = std::rc::Rc::new(std::cell::RefCell::new(Settings {
            profile: Profile::Midnight,
            ..Settings::default()
        }));
        let mut ui = settings_view(appearance(scheme), &settings);
        let mut content = ui.scene(SETTINGS_SIZE.0, SETTINGS_SIZE.1);
        let palette = Palette::for_scheme(scheme);
        content.background = palette.sheet;
        let framed = framed_scene(&content, "Terminal Settings", None, None, &palette);
        shoot(&format!("term-settings-{suffix}"), &framed);
    }
}

#[test]
fn split_panes_match_their_goldens() {
    for (scheme, suffix) in SCHEMES {
        let mut painter = painter(scheme);
        let mut left = terminal(40, 8);
        left.feed(SESSION);
        left.set_focus(true);
        let mut right = terminal(40, 8);
        right.feed(b"top - 14:32:07 up 3 days\r\n\x1b[7m  PID USER  %CPU\x1b[0m\r\n  812 mikael 12.5\r\n$ ");
        let (width, height) = painter.metrics.content_size(left.size());
        let area = Rect::new(0.0, 0.0, (2.0 * width + 1.0).ceil(), height.ceil());
        let mut layout = Layout::Pane(1);
        layout.split(1, Axis::SideBySide, 2);
        let frames = layout.frames(area);
        let panes = [
            Pane {
                frame: frames[0].1,
                terminal: &left,
                focused: true,
            },
            Pane {
                frame: frames[1].1,
                terminal: &right,
                focused: false,
            },
        ];
        let items = painter.panes(&panes, &layout.dividers(area), &Default::default());
        let scene = Scene {
            width: area.width as u32,
            height: area.height as u32,
            background: painter.theme.background,
            items,
        };
        shoot(&format!("term-split-{suffix}"), &scene);
    }
}
