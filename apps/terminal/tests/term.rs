use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty::{self, Options, Shell};
use unbound_term::{GridSize, Terminal, bracketed, encode_key};
use unbound_toolkit::ui::{Key, Modifiers};
use unbound_toolkit::{CpuRenderer, Point, Scheme};

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
    Terminal::new(GridSize { columns, lines }, Scheme::Light)
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
    let metrics = terminal.metrics;
    let at = |column: f32| {
        Point::new(
            unbound_term::PADDING + column * metrics.cell_width + 1.0,
            unbound_term::PADDING + metrics.cell_height / 2.0,
        )
    };

    terminal.start_selection(at(6.0));
    terminal.extend_selection(at(10.6));
    terminal.finish_selection();

    assert_eq!(terminal.selected_text().as_deref(), Some("world"));

    terminal.start_selection(at(2.0));
    terminal.finish_selection();
    assert_eq!(terminal.selected_text(), None);
}

#[test]
fn a_shell_runs_on_a_pty_and_its_output_is_shown() {
    let mut terminal = terminal(40, 5);
    let options = Options {
        shell: Some(Shell::new(
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), "printf 'ready %s' \"$TERM\"".to_owned()],
        )),
        working_directory: None,
        drain_on_exit: true,
        env: HashMap::from([("TERM".to_owned(), "xterm-256color".to_owned())]),
    };
    let pty = tty::new(&options, terminal.window_size(), 0).expect("pty");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buffer = [0u8; 4096];
    while !terminal.screen_text()[0].contains("ready xterm-256color") {
        assert!(Instant::now() < deadline, "{:?}", terminal.screen_text());
        match pty.file().read(&mut buffer) {
            Ok(read) => terminal.feed(&buffer[..read]),
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
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
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut terminal = Terminal::new(
            GridSize {
                columns: 80,
                lines: 8,
            },
            scheme,
        );
        terminal.feed(SESSION);
        let size = terminal.metrics.window_size(terminal.size());
        terminal.set_focus(true);
        let scene = terminal.scene(size.0, size.1);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        check_golden(&format!("term-focused-{suffix}"), size, frame.rgba);
        terminal.set_focus(false);
        let scene = terminal.scene(size.0, size.1);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        check_golden(&format!("term-unfocused-{suffix}"), size, frame.rgba);
    }
}
