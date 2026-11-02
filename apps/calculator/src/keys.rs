use bande_toolkit::ui::{Event, Key, Modifiers};

use crate::L10N;
use crate::engine::{Angle, Base, Binary, Constant, Engine, Input, Memory, Mode, Unary};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Digit,
    Function,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Character(char),
    Named(Key),
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeySpec {
    pub input: Input,
    pub label: String,
    pub name: String,
    pub tone: Tone,
    pub span: usize,
    pub binding: Binding,
}

pub struct Layout {
    pub columns: usize,
    pub rows: Vec<Vec<KeySpec>>,
}

struct Builder<'a> {
    engine: &'a Engine,
}

fn text(message: &str) -> String {
    L10N.tr(message)
}

impl Builder<'_> {
    fn key(&self, input: Input, label: &str, binding: Binding) -> KeySpec {
        let tone = match input {
            Input::Digit(_) | Input::DoubleZero | Input::DoubleF | Input::Point => Tone::Digit,
            Input::Equals
            | Input::Binary(Binary::Add | Binary::Subtract | Binary::Multiply | Binary::Divide) => {
                Tone::Operator
            }
            _ => Tone::Function,
        };
        KeySpec {
            input,
            label: label.to_owned(),
            name: label.to_owned(),
            tone,
            span: 1,
            binding,
        }
    }

    fn named(&self, input: Input, label: &str, message: &str, binding: Binding) -> KeySpec {
        KeySpec {
            name: text(message),
            ..self.key(input, label, binding)
        }
    }

    fn digit(&self, digit: u8) -> KeySpec {
        let label = char::from_digit(u32::from(digit), 16)
            .map(|character| character.to_ascii_uppercase())
            .unwrap_or('0');
        let binding = Binding::Character(label.to_ascii_lowercase());
        self.key(Input::Digit(digit), &label.to_string(), binding)
    }

    fn second(&self, plain: KeySpec, second: KeySpec) -> KeySpec {
        match self.engine.second {
            true => KeySpec {
                binding: plain.binding,
                ..second
            },
            false => plain,
        }
    }

    fn clear(&self) -> KeySpec {
        let (label, message) = match self.engine.clears_all() {
            true => ("AC", "key-all-clear"),
            false => ("C", "key-clear"),
        };
        self.named(Input::Clear, label, message, Binding::Named(Key::Escape))
    }

    fn equals(&self) -> KeySpec {
        match self.engine.rpn {
            true => self.named(
                Input::Equals,
                &text("key-enter-label"),
                "key-enter",
                Binding::Named(Key::Enter),
            ),
            false => self.named(Input::Equals, "=", "key-equals", Binding::Character('=')),
        }
    }

    fn operator(&self, operator: Binary) -> KeySpec {
        let (message, binding) = match operator {
            Binary::Add => ("key-add", '+'),
            Binary::Subtract => ("key-subtract", '-'),
            Binary::Multiply => ("key-multiply", '*'),
            _ => ("key-divide", '/'),
        };
        self.named(
            Input::Binary(operator),
            operator.symbol(),
            message,
            Binding::Character(binding),
        )
    }

    fn open(&self) -> KeySpec {
        match self.engine.rpn {
            true => self.named(Input::Swap, "x⇄y", "key-swap", Binding::Character('(')),
            false => self.key(Input::Open, "(", Binding::Character('(')),
        }
    }

    fn close(&self) -> KeySpec {
        match (
            self.engine.rpn,
            self.engine.second && self.engine.mode == Mode::Scientific,
        ) {
            (true, false) => self.named(
                Input::RollDown,
                "R↓",
                "key-roll-down",
                Binding::Character(')'),
            ),
            (true, true) => self.named(Input::RollUp, "R↑", "key-roll-up", Binding::Character(')')),
            (false, _) => self.key(Input::Close, ")", Binding::Character(')')),
        }
    }

    fn unary(&self, function: Unary, label: &str, message: &str, binding: char) -> KeySpec {
        self.named(
            Input::Unary(function),
            label,
            message,
            Binding::Character(binding),
        )
    }

    fn basic_rows(&self) -> Vec<Vec<KeySpec>> {
        let zero = KeySpec {
            span: 2,
            ..self.digit(0)
        };
        vec![
            vec![
                self.clear(),
                self.unary(Unary::Negate, "±", "key-negate", 'n'),
                self.unary(Unary::Percent, "%", "key-percent", '%'),
                self.operator(Binary::Divide),
            ],
            vec![
                self.digit(7),
                self.digit(8),
                self.digit(9),
                self.operator(Binary::Multiply),
            ],
            vec![
                self.digit(4),
                self.digit(5),
                self.digit(6),
                self.operator(Binary::Subtract),
            ],
            vec![
                self.digit(1),
                self.digit(2),
                self.digit(3),
                self.operator(Binary::Add),
            ],
            vec![
                zero,
                self.key(Input::Point, ".", Binding::Character('.')),
                self.equals(),
            ],
        ]
    }

    fn scientific_rows(&self) -> Vec<Vec<KeySpec>> {
        let function = |function, label: &str, message: &str, binding| {
            self.unary(function, label, message, binding)
        };
        let paired = |plain: (Unary, &str, &str), second: (Unary, &str, &str), binding| {
            self.second(
                function(plain.0, plain.1, plain.2, binding),
                function(second.0, second.1, second.2, binding),
            )
        };
        let binary = |operator: Binary, label: &str, message: &str, binding| {
            self.named(
                Input::Binary(operator),
                label,
                message,
                Binding::Character(binding),
            )
        };
        let angle = match self.engine.angle {
            Angle::Degrees => self.named(
                Input::ToggleAngle,
                &text("key-radians-label"),
                "key-radians",
                Binding::Character('d'),
            ),
            Angle::Radians => self.named(
                Input::ToggleAngle,
                &text("key-degrees-label"),
                "key-degrees",
                Binding::Character('d'),
            ),
        };
        let memory = |memory, label: &str, message: &str, binding| {
            self.named(
                Input::Memory(memory),
                label,
                message,
                Binding::Character(binding),
            )
        };
        let constant = |constant, label: &str, message: &str, binding| {
            self.named(
                Input::Constant(constant),
                label,
                message,
                Binding::Character(binding),
            )
        };
        let mut rows = vec![
            vec![
                self.open(),
                self.close(),
                memory(Memory::Clear, "mc", "key-memory-clear", 'z'),
                memory(Memory::Add, "m+", "key-memory-add", 'm'),
                memory(Memory::Subtract, "m−", "key-memory-subtract", 'M'),
                memory(Memory::Recall, "mr", "key-memory-recall", 'r'),
            ],
            vec![
                self.named(Input::Second, "2ⁿᵈ", "key-second", Binding::Character('`')),
                function(Unary::Square, "x²", "key-square", '@'),
                function(Unary::Cube, "x³", "key-cube", '#'),
                binary(Binary::Power, "xʸ", "key-power", '^'),
                self.second(
                    function(Unary::Exp, "eˣ", "key-exp", 'x'),
                    binary(Binary::PowerOf, "yˣ", "key-power-of", 'x'),
                ),
                paired(
                    (Unary::TenPower, "10ˣ", "key-ten-power"),
                    (Unary::TwoPower, "2ˣ", "key-two-power"),
                    'X',
                ),
            ],
            vec![
                function(Unary::Reciprocal, "¹⁄ₓ", "key-reciprocal", 'i'),
                function(Unary::SquareRoot, "²√x", "key-square-root", 'v'),
                function(Unary::CubeRoot, "³√x", "key-cube-root", 'V'),
                binary(Binary::Root, "ʸ√x", "key-root", 'y'),
                self.second(
                    function(Unary::Ln, "ln", "key-ln", 'l'),
                    binary(Binary::Log, "logᵧ", "key-log", 'l'),
                ),
                paired(
                    (Unary::Log10, "log₁₀", "key-log10"),
                    (Unary::Log2, "log₂", "key-log2"),
                    'L',
                ),
            ],
            vec![
                function(Unary::Factorial, "x!", "key-factorial", '!'),
                paired(
                    (Unary::Sin, "sin", "key-sin"),
                    (Unary::Asin, "sin⁻¹", "key-asin"),
                    's',
                ),
                paired(
                    (Unary::Cos, "cos", "key-cos"),
                    (Unary::Acos, "cos⁻¹", "key-acos"),
                    'c',
                ),
                paired(
                    (Unary::Tan, "tan", "key-tan"),
                    (Unary::Atan, "tan⁻¹", "key-atan"),
                    't',
                ),
                constant(Constant::E, "e", "key-e", 'e'),
                self.named(
                    Input::Exponent,
                    "EE",
                    "key-exponent",
                    Binding::Character('E'),
                ),
            ],
            vec![
                angle,
                paired(
                    (Unary::Sinh, "sinh", "key-sinh"),
                    (Unary::Asinh, "sinh⁻¹", "key-asinh"),
                    'S',
                ),
                paired(
                    (Unary::Cosh, "cosh", "key-cosh"),
                    (Unary::Acosh, "cosh⁻¹", "key-acosh"),
                    'C',
                ),
                paired(
                    (Unary::Tanh, "tanh", "key-tanh"),
                    (Unary::Atanh, "tanh⁻¹", "key-atanh"),
                    'T',
                ),
                constant(Constant::Pi, "π", "key-pi", 'p'),
                constant(Constant::Random, "Rand", "key-random", '?'),
            ],
        ];
        for (row, basic) in rows.iter_mut().zip(self.basic_rows()) {
            row.extend(basic);
        }
        rows
    }

    fn programmer_rows(&self) -> Vec<Vec<KeySpec>> {
        let function = |function, label: &str, message: &str, binding| {
            self.unary(function, label, message, binding)
        };
        let binary = |operator: Binary, message: &str, binding| {
            self.named(
                Input::Binary(operator),
                operator.symbol(),
                message,
                Binding::Character(binding),
            )
        };
        let equals = KeySpec {
            span: 2,
            ..self.equals()
        };
        vec![
            vec![
                binary(Binary::And, "key-and", '&'),
                binary(Binary::Or, "key-or", '|'),
                self.digit(13),
                self.digit(14),
                self.digit(15),
                self.clear(),
                self.named(
                    Input::Backspace,
                    "⌫",
                    "key-backspace",
                    Binding::Named(Key::Backspace),
                ),
            ],
            vec![
                binary(Binary::Nor, "key-nor", 'N'),
                binary(Binary::Xor, "key-xor", '^'),
                self.digit(10),
                self.digit(11),
                self.digit(12),
                function(Unary::RotateLeft, "RoL", "key-rotate-left", '{'),
                self.operator(Binary::Divide),
            ],
            vec![
                function(Unary::ShiftLeftOne, "<<", "key-shift-left-one", '['),
                function(Unary::ShiftRightOne, ">>", "key-shift-right-one", ']'),
                self.digit(7),
                self.digit(8),
                self.digit(9),
                function(Unary::RotateRight, "RoR", "key-rotate-right", '}'),
                self.operator(Binary::Multiply),
            ],
            vec![
                self.named(
                    Input::Binary(Binary::ShiftLeft),
                    "X<<Y",
                    "key-shift-left",
                    Binding::Character('<'),
                ),
                self.named(
                    Input::Binary(Binary::ShiftRight),
                    "X>>Y",
                    "key-shift-right",
                    Binding::Character('>'),
                ),
                self.digit(4),
                self.digit(5),
                self.digit(6),
                function(Unary::TwosComplement, "2's", "key-twos-complement", 'n'),
                self.operator(Binary::Subtract),
            ],
            vec![
                function(Unary::ByteFlip, "flip₈", "key-byte-flip", 'u'),
                function(Unary::WordFlip, "flip₁₆", "key-word-flip", 'U'),
                self.digit(1),
                self.digit(2),
                self.digit(3),
                function(Unary::OnesComplement, "1's", "key-ones-complement", '~'),
                self.operator(Binary::Add),
            ],
            vec![
                self.named(
                    Input::DoubleF,
                    "FF",
                    "key-double-f",
                    Binding::Character('g'),
                ),
                self.named(
                    Input::DoubleZero,
                    "00",
                    "key-double-zero",
                    Binding::Character('G'),
                ),
                self.digit(0),
                self.open(),
                self.close(),
                equals,
            ],
        ]
    }
}

pub fn layout(engine: &Engine) -> Layout {
    let builder = Builder { engine };
    match engine.mode {
        Mode::Basic => Layout {
            columns: 4,
            rows: builder.basic_rows(),
        },
        Mode::Scientific => Layout {
            columns: 10,
            rows: builder.scientific_rows(),
        },
        Mode::Programmer => Layout {
            columns: 7,
            rows: builder.programmer_rows(),
        },
    }
}

fn plain(modifiers: Modifiers) -> bool {
    !modifiers.control && !modifiers.alt && !modifiers.logo
}

pub fn typed(engine: &Engine, layout: &Layout, event: &Event) -> Option<Input> {
    let Event::Key { key, modifiers } = event else {
        return None;
    };
    if !plain(*modifiers) {
        return None;
    }
    let key = match key {
        Key::Character(',') => Key::Character('.'),
        Key::Character('x') if engine.mode == Mode::Basic => Key::Character('*'),
        Key::Character(character @ 'A'..='F') if engine.mode == Mode::Programmer => {
            Key::Character(character.to_ascii_lowercase())
        }
        key => *key,
    };
    let found = layout
        .rows
        .iter()
        .flatten()
        .find(|spec| match spec.binding {
            Binding::Character(character) => key == Key::Character(character),
            Binding::Named(named) => key == named,
        });
    if let Some(spec) = found {
        return Some(spec.input);
    }
    match key {
        Key::Enter | Key::Character('=') => Some(Input::Equals),
        Key::Backspace => Some(Input::Backspace),
        Key::Delete => Some(Input::Clear),
        Key::Character('w') if engine.rpn => Some(Input::Swap),
        Key::Character('R') if engine.rpn => Some(Input::RollDown),
        Key::Character('n') => Some(Input::Unary(Unary::Negate)),
        _ => None,
    }
}

pub fn base_label(base: Base) -> String {
    text(match base {
        Base::Hexadecimal => "base-hexadecimal-short",
        Base::Decimal => "base-decimal-short",
        Base::Octal => "base-octal-short",
        Base::Binary => "base-binary-short",
    })
}

pub fn binding_label(binding: Binding) -> String {
    match binding {
        Binding::Character(character) => character.to_string(),
        Binding::Named(Key::Enter) => text("binding-enter"),
        Binding::Named(Key::Escape) => text("binding-escape"),
        Binding::Named(Key::Backspace) => text("binding-backspace"),
        Binding::Named(_) => String::new(),
    }
}
