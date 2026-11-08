use bande_l10n::{Text, text, tr};

use crate::engine::{Angle, Base, Binary, Constant, Engine, Input, Memory, Mode, Unary};
use bande_toolkit::ui::{Event, Key, Modifiers};

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

    fn named(&self, input: Input, label: &str, message: Text, binding: Binding) -> KeySpec {
        KeySpec {
            name: message.tr(),
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
            true => ("AC", text!("All Clear")),
            false => ("C", text!("Clear")),
        };
        self.named(Input::Clear, label, message, Binding::Named(Key::Escape))
    }

    fn equals(&self) -> KeySpec {
        match self.engine.rpn {
            true => self.named(
                Input::Equals,
                &tr!("enter"),
                text!("Enter"),
                Binding::Named(Key::Enter),
            ),
            false => self.named(Input::Equals, "=", text!("Equals"), Binding::Character('=')),
        }
    }

    fn operator(&self, operator: Binary) -> KeySpec {
        let (message, binding) = match operator {
            Binary::Add => (text!("Add"), '+'),
            Binary::Subtract => (text!("Subtract"), '-'),
            Binary::Multiply => (text!("Multiply"), '*'),
            _ => (text!("Divide"), '/'),
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
            true => self.named(
                Input::Swap,
                "x⇄y",
                text!("Swap X and Y"),
                Binding::Character('('),
            ),
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
                text!("Roll Down"),
                Binding::Character(')'),
            ),
            (true, true) => self.named(
                Input::RollUp,
                "R↑",
                text!("Roll Up"),
                Binding::Character(')'),
            ),
            (false, _) => self.key(Input::Close, ")", Binding::Character(')')),
        }
    }

    fn unary(&self, function: Unary, label: &str, message: Text, binding: char) -> KeySpec {
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
                self.unary(Unary::Negate, "±", text!("Change Sign"), 'n'),
                self.unary(Unary::Percent, "%", text!("Percent"), '%'),
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
        let function = |function, label: &str, message: Text, binding| {
            self.unary(function, label, message, binding)
        };
        let paired = |plain: (Unary, &str, Text), second: (Unary, &str, Text), binding| {
            self.second(
                function(plain.0, plain.1, plain.2, binding),
                function(second.0, second.1, second.2, binding),
            )
        };
        let binary = |operator: Binary, label: &str, message: Text, binding| {
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
                &tr!("Rad"),
                text!("Use Radians"),
                Binding::Character('d'),
            ),
            Angle::Radians => self.named(
                Input::ToggleAngle,
                &tr!("Deg"),
                text!("Use Degrees"),
                Binding::Character('d'),
            ),
        };
        let memory = |memory, label: &str, message: Text, binding| {
            self.named(
                Input::Memory(memory),
                label,
                message,
                Binding::Character(binding),
            )
        };
        let constant = |constant, label: &str, message: Text, binding| {
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
                memory(Memory::Clear, "mc", text!("Memory Clear"), 'z'),
                memory(Memory::Add, "m+", text!("Memory Add"), 'm'),
                memory(Memory::Subtract, "m−", text!("Memory Subtract"), 'M'),
                memory(Memory::Recall, "mr", text!("Memory Recall"), 'r'),
            ],
            vec![
                self.named(
                    Input::Second,
                    "2ⁿᵈ",
                    text!("Second Functions"),
                    Binding::Character('`'),
                ),
                function(Unary::Square, "x²", text!("Square"), '@'),
                function(Unary::Cube, "x³", text!("Cube"), '#'),
                binary(Binary::Power, "xʸ", text!("X to the Power of Y"), '^'),
                self.second(
                    function(Unary::Exp, "eˣ", text!("e to the Power of X"), 'x'),
                    binary(Binary::PowerOf, "yˣ", text!("Y to the Power of X"), 'x'),
                ),
                paired(
                    (Unary::TenPower, "10ˣ", text!("10 to the Power of X")),
                    (Unary::TwoPower, "2ˣ", text!("2 to the Power of X")),
                    'X',
                ),
            ],
            vec![
                function(Unary::Reciprocal, "¹⁄ₓ", text!("Reciprocal"), 'i'),
                function(Unary::SquareRoot, "²√x", text!("Square Root"), 'v'),
                function(Unary::CubeRoot, "³√x", text!("Cube Root"), 'V'),
                binary(Binary::Root, "ʸ√x", text!("Y-th Root of X"), 'y'),
                self.second(
                    function(Unary::Ln, "ln", text!("Natural Logarithm"), 'l'),
                    binary(Binary::Log, "logᵧ", text!("Logarithm to Base Y"), 'l'),
                ),
                paired(
                    (Unary::Log10, "log₁₀", text!("Logarithm to Base 10")),
                    (Unary::Log2, "log₂", text!("Logarithm to Base 2")),
                    'L',
                ),
            ],
            vec![
                function(Unary::Factorial, "x!", text!("Factorial"), '!'),
                paired(
                    (Unary::Sin, "sin", text!("Sine")),
                    (Unary::Asin, "sin⁻¹", text!("Inverse Sine")),
                    's',
                ),
                paired(
                    (Unary::Cos, "cos", text!("Cosine")),
                    (Unary::Acos, "cos⁻¹", text!("Inverse Cosine")),
                    'c',
                ),
                paired(
                    (Unary::Tan, "tan", text!("Tangent")),
                    (Unary::Atan, "tan⁻¹", text!("Inverse Tangent")),
                    't',
                ),
                constant(Constant::E, "e", text!("Euler's Number"), 'e'),
                self.named(
                    Input::Exponent,
                    "EE",
                    text!("Exponent"),
                    Binding::Character('E'),
                ),
            ],
            vec![
                angle,
                paired(
                    (Unary::Sinh, "sinh", text!("Hyperbolic Sine")),
                    (Unary::Asinh, "sinh⁻¹", text!("Inverse Hyperbolic Sine")),
                    'S',
                ),
                paired(
                    (Unary::Cosh, "cosh", text!("Hyperbolic Cosine")),
                    (Unary::Acosh, "cosh⁻¹", text!("Inverse Hyperbolic Cosine")),
                    'C',
                ),
                paired(
                    (Unary::Tanh, "tanh", text!("Hyperbolic Tangent")),
                    (Unary::Atanh, "tanh⁻¹", text!("Inverse Hyperbolic Tangent")),
                    'T',
                ),
                constant(Constant::Pi, "π", text!("Pi"), 'p'),
                constant(Constant::Random, "Rand", text!("Random Number"), '?'),
            ],
        ];
        for (row, basic) in rows.iter_mut().zip(self.basic_rows()) {
            row.extend(basic);
        }
        rows
    }

    fn programmer_rows(&self) -> Vec<Vec<KeySpec>> {
        let function = |function, label: &str, message: Text, binding| {
            self.unary(function, label, message, binding)
        };
        let binary = |operator: Binary, message: Text, binding| {
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
                binary(Binary::And, text!("And"), '&'),
                binary(Binary::Or, text!("Or"), '|'),
                self.digit(13),
                self.digit(14),
                self.digit(15),
                self.clear(),
                self.named(
                    Input::Backspace,
                    "⌫",
                    text!("Delete"),
                    Binding::Named(Key::Backspace),
                ),
            ],
            vec![
                binary(Binary::Nor, text!("Nor"), 'N'),
                binary(Binary::Xor, text!("Exclusive Or"), '^'),
                self.digit(10),
                self.digit(11),
                self.digit(12),
                function(Unary::RotateLeft, "RoL", text!("Rotate Left"), '{'),
                self.operator(Binary::Divide),
            ],
            vec![
                function(Unary::ShiftLeftOne, "<<", text!("Shift Left by One"), '['),
                function(Unary::ShiftRightOne, ">>", text!("Shift Right by One"), ']'),
                self.digit(7),
                self.digit(8),
                self.digit(9),
                function(Unary::RotateRight, "RoR", text!("Rotate Right"), '}'),
                self.operator(Binary::Multiply),
            ],
            vec![
                self.named(
                    Input::Binary(Binary::ShiftLeft),
                    "X<<Y",
                    text!("Shift X Left by Y"),
                    Binding::Character('<'),
                ),
                self.named(
                    Input::Binary(Binary::ShiftRight),
                    "X>>Y",
                    text!("Shift X Right by Y"),
                    Binding::Character('>'),
                ),
                self.digit(4),
                self.digit(5),
                self.digit(6),
                function(Unary::TwosComplement, "2's", text!("Two's Complement"), 'n'),
                self.operator(Binary::Subtract),
            ],
            vec![
                function(Unary::ByteFlip, "flip₈", text!("Flip Bytes"), 'u'),
                function(Unary::WordFlip, "flip₁₆", text!("Flip Words"), 'U'),
                self.digit(1),
                self.digit(2),
                self.digit(3),
                function(Unary::OnesComplement, "1's", text!("One's Complement"), '~'),
                self.operator(Binary::Add),
            ],
            vec![
                self.named(Input::DoubleF, "FF", text!("FF"), Binding::Character('g')),
                self.named(
                    Input::DoubleZero,
                    "00",
                    text!("Double Zero"),
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
    match base {
        Base::Hexadecimal => tr!("HEX"),
        Base::Decimal => tr!("DEC"),
        Base::Octal => tr!("OCT"),
        Base::Binary => tr!("BIN"),
    }
}

pub fn binding_label(binding: Binding) -> String {
    match binding {
        Binding::Character(character) => character.to_string(),
        Binding::Named(Key::Enter) => tr!("Enter"),
        Binding::Named(Key::Escape) => tr!("Esc"),
        Binding::Named(Key::Backspace) => tr!("Backspace"),
        Binding::Named(_) => String::new(),
    }
}
