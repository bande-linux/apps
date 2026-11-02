#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_calculator::{Base, Binary, Engine, Failure, Input, Keypad, Mode, Unary};
use bande_toolkit::ui::{Context, Event, Key, Modifiers, Ui, Widget};
use bande_toolkit::{Palette, Scheme, TextSystem};

fn typed(ui: &mut Ui, keys: &str) {
    for character in keys.chars() {
        let key = match character {
            '\n' => Key::Enter,
            '\u{1b}' => Key::Escape,
            character => Key::Character(character),
        };
        ui.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }
}

fn shown(ui: &mut Ui) -> String {
    ui.scene(620, 380);
    let tree = ui.accessibility();
    let (_, display) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Label && node.label() == Some("Result"))
        .unwrap();
    display.value().unwrap().to_owned()
}

fn check(ui: &mut Ui, keys: &str, expected: &str) {
    typed(ui, keys);
    assert_eq!(shown(ui), expected, "after {keys:?}");
}

fn click(ui: &mut Ui, name: &str) {
    ui.scene(620, 380);
    let tree = ui.accessibility();
    let (button, _) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some(name))
        .unwrap();
    ui.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: *button,
        data: None,
    });
}

#[test]
fn typing_and_clicking_calculate_exactly_with_precedence_in_every_mode() {
    let engine = Rc::new(RefCell::new(Engine::new()));
    let keypad = Keypad::new(engine.clone());
    let id = keypad.id();
    let palette = Palette::for_scheme(Scheme::Dark);
    let mut ui = Ui::new(Box::new(keypad), Context::new(TextSystem::new(), palette));
    ui.set_focus(id);
    check(&mut ui, "0.1+0.2=", "0.3");
    check(&mut ui, "1/3*3=", "1");
    check(&mut ui, "2+3*4\n", "14");
    check(&mut ui, "1234567*1000=", "1,234,567,000");
    check(&mut ui, "50+10%=", "55");
    engine.borrow_mut().set_mode(Mode::Scientific);
    check(&mut ui, "2^3^2=", "512");
    check(&mut ui, "(2+3)*4=", "20");
    check(&mut ui, "30s", "0.5");
    check(&mut ui, "180s", "0");
    check(&mut ui, "dps", "0");
    check(&mut ui, "8y3=", "2");
    check(&mut ui, "1000L", "3");
    click(&mut ui, "Use Degrees");
    check(&mut ui, "\u{1b}`.5s", "30");
    engine.borrow_mut().set_rpn(true);
    check(&mut ui, "3\n4+5*", "35");
    check(&mut ui, "200\n5%", "10");
    engine.borrow_mut().set_rpn(false);
    engine.borrow_mut().set_mode(Mode::Programmer);
    click(&mut ui, "8");
    check(&mut ui, "5-7=", "-2");
    click(&mut ui, "HEX");
    check(&mut ui, "", "FE");
    check(&mut ui, "\u{1b}FF+1=", "0");
    click(&mut ui, "Bit 7");
    check(&mut ui, "", "80");
}

#[test]
fn errors_and_word_sizes_wrap_like_a_real_calculator() {
    let mut engine = Engine::new();
    for input in [
        Input::Digit(1),
        Input::Binary(Binary::Divide),
        Input::Digit(0),
        Input::Equals,
    ] {
        engine.press(input);
    }
    assert_eq!(engine.error(), Some(Failure::Undefined));
    engine.press(Input::Digit(4));
    assert_eq!((engine.error(), engine.display()), (None, "4".to_owned()));
    engine.set_mode(Mode::Programmer);
    engine.set_base(Base::Hexadecimal);
    engine.set_bits(32);
    for digit in [1, 2, 3, 4, 5, 6, 7, 8] {
        engine.press(Input::Digit(digit));
    }
    engine.press(Input::Digit(9));
    assert_eq!(engine.display(), "1234 5678");
    engine.press(Input::Unary(Unary::WordFlip));
    assert_eq!(engine.display(), "5678 1234");
    engine.set_bits(16);
    assert_eq!(engine.display(), "1234");
    assert!(engine.paste("-1"));
    assert_eq!(engine.display(), "FFFF");
    engine.set_mode(Mode::Basic);
    assert_eq!(engine.display(), "−1");
}
