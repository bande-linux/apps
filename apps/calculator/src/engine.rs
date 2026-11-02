use std::hash::{BuildHasher, RandomState};
use std::time::Instant;

use crate::decimal::{Decimal, Failure};

pub const SHOWN_DIGITS: u32 = 16;
const SCIENTIFIC_DIGITS: u32 = 16;
const SMALLEST_PLAIN: i64 = -8;
const EXPONENT_DIGITS: usize = 4;
const LARGEST_FACTORIAL: i128 = 3300;
const NEGLIGIBLE: f64 = 1e-15;
const RANDOM_BITS: u32 = 53;
const PI: &str = "3.14159265358979324";
const E: &str = "2.71828182845904524";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Basic,
    Scientific,
    Programmer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Angle {
    Degrees,
    Radians,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Binary,
    Octal,
    Decimal,
    Hexadecimal,
}

impl Base {
    pub const ALL: [Self; 4] = [Self::Hexadecimal, Self::Decimal, Self::Octal, Self::Binary];

    pub fn radix(self) -> u32 {
        match self {
            Self::Binary => 2,
            Self::Octal => 8,
            Self::Decimal => 10,
            Self::Hexadecimal => 16,
        }
    }
}

pub const WORD_SIZES: [u32; 4] = [8, 16, 32, 64];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    PowerOf,
    Root,
    Log,
    And,
    Or,
    Xor,
    Nor,
    ShiftLeft,
    ShiftRight,
}

impl Binary {
    fn precedence(self) -> u8 {
        match self {
            Self::Or | Self::Nor => 1,
            Self::Xor => 2,
            Self::And => 3,
            Self::ShiftLeft | Self::ShiftRight => 4,
            Self::Add | Self::Subtract => 5,
            Self::Multiply | Self::Divide => 6,
            Self::Power | Self::PowerOf | Self::Root | Self::Log => 7,
        }
    }

    fn right_associative(self) -> bool {
        matches!(self, Self::Power | Self::PowerOf | Self::Root | Self::Log)
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "−",
            Self::Multiply => "×",
            Self::Divide => "÷",
            Self::Power => "^",
            Self::PowerOf => "yˣ",
            Self::Root => "ʸ√",
            Self::Log => "logᵧ",
            Self::And => "AND",
            Self::Or => "OR",
            Self::Xor => "XOR",
            Self::Nor => "NOR",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    Negate,
    Percent,
    Square,
    Cube,
    Exp,
    TenPower,
    TwoPower,
    Reciprocal,
    SquareRoot,
    CubeRoot,
    Ln,
    Log10,
    Log2,
    Factorial,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    ShiftLeftOne,
    ShiftRightOne,
    RotateLeft,
    RotateRight,
    OnesComplement,
    TwosComplement,
    ByteFlip,
    WordFlip,
}

impl Unary {
    fn notation(self, operand: &str) -> String {
        let wrapped = match operand.contains(' ') {
            true => format!("({operand})"),
            false => operand.to_owned(),
        };
        let prefix = |name: &str| format!("{name}({operand})");
        match self {
            Self::Negate | Self::TwosComplement => format!("−{wrapped}"),
            Self::Percent => format!("{wrapped}%"),
            Self::Square => format!("{wrapped}²"),
            Self::Cube => format!("{wrapped}³"),
            Self::Factorial => format!("{wrapped}!"),
            Self::Exp => format!("e^{wrapped}"),
            Self::TenPower => format!("10^{wrapped}"),
            Self::TwoPower => format!("2^{wrapped}"),
            Self::Reciprocal => format!("1/{wrapped}"),
            Self::SquareRoot => format!("√{wrapped}"),
            Self::CubeRoot => format!("∛{wrapped}"),
            Self::Ln => prefix("ln"),
            Self::Log10 => prefix("log₁₀"),
            Self::Log2 => prefix("log₂"),
            Self::Sin => prefix("sin"),
            Self::Cos => prefix("cos"),
            Self::Tan => prefix("tan"),
            Self::Asin => prefix("sin⁻¹"),
            Self::Acos => prefix("cos⁻¹"),
            Self::Atan => prefix("tan⁻¹"),
            Self::Sinh => prefix("sinh"),
            Self::Cosh => prefix("cosh"),
            Self::Tanh => prefix("tanh"),
            Self::Asinh => prefix("sinh⁻¹"),
            Self::Acosh => prefix("cosh⁻¹"),
            Self::Atanh => prefix("tanh⁻¹"),
            Self::ShiftLeftOne => format!("{wrapped} << 1"),
            Self::ShiftRightOne => format!("{wrapped} >> 1"),
            Self::RotateLeft => prefix("RoL"),
            Self::RotateRight => prefix("RoR"),
            Self::OnesComplement => format!("~{wrapped}"),
            Self::ByteFlip => prefix("flip₈"),
            Self::WordFlip => prefix("flip₁₆"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constant {
    Pi,
    E,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory {
    Clear,
    Add,
    Subtract,
    Recall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Digit(u8),
    DoubleZero,
    DoubleF,
    Point,
    Exponent,
    Backspace,
    Clear,
    Equals,
    Open,
    Close,
    Binary(Binary),
    Unary(Unary),
    Constant(Constant),
    Memory(Memory),
    Second,
    ToggleAngle,
    Swap,
    RollDown,
    RollUp,
    Drop,
    Base(Base),
    Word(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Real(Decimal),
    Word(u64),
}

struct Pending {
    left: Value,
    operator: Binary,
}

pub struct Engine {
    pub mode: Mode,
    pub angle: Angle,
    pub base: Base,
    pub bits: u32,
    pub rpn: bool,
    pub second: bool,
    pub grouping: bool,
    x: Value,
    entry: Option<String>,
    shown: Option<String>,
    levels: Vec<Vec<Pending>>,
    tokens: Vec<String>,
    awaiting: bool,
    repeat: Option<(Binary, Value)>,
    memory: Decimal,
    stack: Vec<Value>,
    lift: bool,
    error: Option<Failure>,
    tape: Vec<String>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

fn mask(bits: u32) -> u64 {
    match bits {
        64 => u64::MAX,
        bits => (1u64 << bits) - 1,
    }
}

fn signed(value: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((value << shift) as i64) >> shift
}

fn group(digits: &str, size: usize, separator: char) -> String {
    let count = digits.chars().count();
    let mut grouped = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (count - index).is_multiple_of(size) {
            grouped.push(separator);
        }
        grouped.push(character);
    }
    grouped
}

fn group_number(text: &str, grouping: bool) -> String {
    if !grouping {
        return text.to_owned();
    }
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text),
    };
    let split = rest.find(['.', 'e']).unwrap_or(rest.len());
    let (whole, tail) = rest.split_at(split);
    format!("{sign}{}{tail}", group(whole, 3, ','))
}

pub fn format_real(value: Decimal, grouping: bool) -> String {
    let rounded = value.rounded(SHOWN_DIGITS);
    let leading = rounded.leading_exponent();
    let text = if rounded.is_zero() {
        "0".to_owned()
    } else if leading >= i64::from(SHOWN_DIGITS) || leading < SMALLEST_PLAIN {
        rounded.rounded(SCIENTIFIC_DIGITS).scientific()
    } else if leading < 0 {
        let room = i64::from(SHOWN_DIGITS) + leading + 1;
        value.rounded(u32::try_from(room).unwrap_or(1)).plain()
    } else {
        rounded.plain()
    };
    group_number(&text, grouping).replace('-', "−")
}

fn format_word(value: u64, base: Base, bits: u32, grouping: bool) -> String {
    match base {
        Base::Decimal => group_number(&signed(value, bits).to_string(), grouping).replace('-', "−"),
        Base::Hexadecimal => group(&format!("{value:X}"), 4, ' '),
        Base::Octal => format!("{value:o}"),
        Base::Binary => group(&format!("{value:b}"), 4, ' '),
    }
}

fn random() -> Result<Decimal, Failure> {
    let bits = RandomState::new().hash_one(Instant::now()) >> (64 - RANDOM_BITS);
    Decimal::from_float(bits as f64 / (1u64 << RANDOM_BITS) as f64)
}

fn reduced_degrees(value: Decimal) -> Result<Decimal, Failure> {
    let full = Decimal::integer(360)?;
    value.subtract(full.multiply(value.divide(full)?.trunc())?)
}

fn exact_degrees(value: Decimal) -> Result<Option<u32>, Failure> {
    let full = Decimal::integer(360)?;
    let mut turned = reduced_degrees(value)?;
    if turned.is_negative() {
        turned = turned.plus(full)?;
    }
    Ok(turned
        .to_integer()
        .filter(|degrees| turned.is_integer() && degrees % 90 == 0)
        .and_then(|degrees| u32::try_from(degrees % 360).ok()))
}

impl Engine {
    pub fn new() -> Self {
        Self {
            mode: Mode::Basic,
            angle: Angle::Degrees,
            base: Base::Decimal,
            bits: 64,
            rpn: false,
            second: false,
            grouping: true,
            x: Value::Real(Decimal::ZERO),
            entry: None,
            shown: None,
            levels: vec![Vec::new()],
            tokens: Vec::new(),
            awaiting: false,
            repeat: None,
            memory: Decimal::ZERO,
            stack: Vec::new(),
            lift: false,
            error: None,
            tape: Vec::new(),
        }
    }

    pub fn tape(&self) -> &[String] {
        &self.tape
    }

    pub fn clear_tape(&mut self) {
        self.tape.clear();
    }

    pub fn has_memory(&self) -> bool {
        !self.memory.is_zero()
    }

    pub fn error(&self) -> Option<Failure> {
        self.error
    }

    pub fn clears_all(&self) -> bool {
        self.entry.is_none() && (self.awaiting || self.levels.iter().all(Vec::is_empty))
    }

    pub fn pending(&self) -> Option<Binary> {
        match self.awaiting {
            true => self.levels.last()?.last().map(|pending| pending.operator),
            false => None,
        }
    }

    pub fn word(&self) -> Option<u64> {
        match self.x {
            Value::Word(word) if self.error.is_none() => Some(word),
            _ => None,
        }
    }

    fn format(&self, value: Value) -> String {
        match value {
            Value::Real(real) => format_real(real, self.grouping),
            Value::Word(word) => format_word(word, self.base, self.bits, self.grouping),
        }
    }

    pub fn display(&self) -> String {
        match &self.entry {
            Some(entry) => match self.mode {
                Mode::Programmer => match self.base {
                    Base::Hexadecimal | Base::Binary => group(entry, 4, ' '),
                    _ => group_number(entry, self.grouping),
                },
                _ => group_number(entry, self.grouping).replace('-', "−"),
            },
            None => self.format(self.x),
        }
    }

    pub fn expression(&self) -> String {
        let mut text = String::new();
        for token in &self.tokens {
            if !text.is_empty() && !text.ends_with('(') && token != ")" {
                text.push(' ');
            }
            text.push_str(token);
        }
        text
    }

    pub fn stack(&self) -> Vec<String> {
        self.stack.iter().map(|value| self.format(*value)).collect()
    }

    pub fn copy_text(&self) -> String {
        match (self.entry.as_ref(), self.x) {
            (Some(entry), _) if self.mode != Mode::Programmer => entry.clone(),
            (_, Value::Real(real)) => format_real(real, false).replace('−', "-"),
            (_, Value::Word(word)) => format_word(word, self.base, self.bits, false)
                .replace(' ', "")
                .replace('−', "-"),
        }
    }

    pub fn paste(&mut self, text: &str) -> bool {
        let cleaned: String = text
            .trim()
            .replace('−', "-")
            .chars()
            .filter(|character| !matches!(character, ',' | ' ' | '_' | '\u{a0}'))
            .collect();
        let value = match self.mode {
            Mode::Programmer => {
                let (negative, digits) = match cleaned.strip_prefix('-') {
                    Some(rest) => (true, rest),
                    None => (false, cleaned.as_str()),
                };
                let digits = match self.base {
                    Base::Hexadecimal => digits.trim_start_matches("0x"),
                    _ => digits,
                };
                let Ok(magnitude) = u64::from_str_radix(digits, self.base.radix()) else {
                    return false;
                };
                let word = match negative {
                    true => magnitude.wrapping_neg(),
                    false => magnitude,
                };
                Value::Word(word & mask(self.bits))
            }
            _ => match Decimal::parse(&cleaned) {
                Some(real) => Value::Real(real),
                None => return false,
            },
        };
        self.error = None;
        self.load(value, None);
        true
    }

    pub fn toggle_bit(&mut self, bit: u32) {
        if bit >= self.bits || self.error.is_some() {
            return;
        }
        let Ok(Value::Word(word)) = self.commit() else {
            return;
        };
        self.x = Value::Word(word ^ (1 << bit));
        self.shown = None;
        self.awaiting = false;
    }

    fn zero(&self) -> Value {
        match self.mode {
            Mode::Programmer => Value::Word(0),
            _ => Value::Real(Decimal::ZERO),
        }
    }

    fn convert(&self, value: Value) -> Value {
        match (self.mode, value) {
            (Mode::Programmer, Value::Real(real)) => {
                let integer = real.trunc().to_integer().unwrap_or(0);
                Value::Word(integer as u64 & mask(self.bits))
            }
            (Mode::Programmer, Value::Word(word)) => Value::Word(word & mask(self.bits)),
            (_, Value::Word(word)) => Value::Real(
                Decimal::integer(i128::from(signed(word, self.bits))).unwrap_or_default(),
            ),
            (_, real) => real,
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        let x = self.commit().unwrap_or(self.x);
        self.mode = mode;
        self.x = self.convert(x);
        self.stack = self
            .stack
            .iter()
            .map(|value| self.convert(*value))
            .collect();
        self.reset_expression();
    }

    pub fn set_base(&mut self, base: Base) {
        if let Ok(x) = self.commit() {
            self.x = x;
        }
        self.base = base;
    }

    pub fn set_bits(&mut self, bits: u32) {
        if let Ok(x) = self.commit() {
            self.x = x;
        }
        self.bits = bits;
        self.x = self.convert(self.x);
        self.stack = self
            .stack
            .iter()
            .map(|value| self.convert(*value))
            .collect();
        self.levels.iter_mut().flatten().for_each(|pending| {
            if let Value::Word(word) = pending.left {
                pending.left = Value::Word(word & mask(bits));
            }
        });
    }

    pub fn set_rpn(&mut self, rpn: bool) {
        let x = self.commit().unwrap_or(self.x);
        self.rpn = rpn;
        self.x = x;
        self.stack.clear();
        self.lift = false;
        self.reset_expression();
    }

    fn reset_expression(&mut self) {
        self.entry = None;
        self.shown = None;
        self.levels = vec![Vec::new()];
        self.tokens.clear();
        self.awaiting = false;
        self.repeat = None;
        self.error = None;
    }

    fn commit(&mut self) -> Result<Value, Failure> {
        let Some(entry) = self.entry.take() else {
            return Ok(self.x);
        };
        let value = match self.mode {
            Mode::Programmer => Value::Word(
                u64::from_str_radix(&entry, self.base.radix()).map_err(|_| Failure::Undefined)?,
            ),
            _ => Value::Real(
                Decimal::parse(entry.trim_end_matches(['e', '-'])).ok_or(Failure::Undefined)?,
            ),
        };
        self.x = value;
        Ok(value)
    }

    fn operand_text(&self) -> String {
        self.shown.clone().unwrap_or_else(|| self.display())
    }

    fn load(&mut self, value: Value, shown: Option<String>) {
        if self.rpn && self.lift && self.entry.is_none() {
            self.stack.push(self.x);
        }
        self.entry = None;
        self.x = value;
        self.shown = shown;
        self.awaiting = false;
        self.lift = true;
    }

    pub fn press(&mut self, input: Input) {
        if self.error.is_some() {
            self.reset_expression();
            self.stack.clear();
            self.lift = false;
            if input == Input::Clear {
                return;
            }
        }
        if let Err(failure) = self.apply_input(input) {
            self.error = Some(failure);
            self.entry = None;
            self.x = self.zero();
        }
    }

    fn apply_input(&mut self, input: Input) -> Result<(), Failure> {
        match input {
            Input::Digit(digit) => self.digits(&[digit]),
            Input::DoubleZero => self.digits(&[0, 0]),
            Input::DoubleF => self.digits(&[15, 15]),
            Input::Point => self.point(),
            Input::Exponent => self.exponent(),
            Input::Backspace => self.backspace(),
            Input::Clear => {
                self.clear();
                Ok(())
            }
            Input::Equals if self.rpn => self.enter(),
            Input::Equals => self.equals(),
            Input::Open => {
                self.open();
                Ok(())
            }
            Input::Close => self.close(),
            Input::Binary(operator) if self.rpn => self.rpn_binary(operator),
            Input::Binary(operator) => self.binary(operator),
            Input::Unary(Unary::Negate) if self.entry.is_some() => {
                self.negate_entry();
                Ok(())
            }
            Input::Unary(Unary::Percent) => self.percent(),
            Input::Unary(function) => self.unary(function),
            Input::Constant(constant) => self.constant(constant),
            Input::Memory(memory) => self.memory(memory),
            Input::Second => {
                self.second = !self.second;
                Ok(())
            }
            Input::ToggleAngle => {
                self.angle = match self.angle {
                    Angle::Degrees => Angle::Radians,
                    Angle::Radians => Angle::Degrees,
                };
                Ok(())
            }
            Input::Swap => self.swap(),
            Input::RollDown => self.roll(true),
            Input::RollUp => self.roll(false),
            Input::Drop => self.drop_x(),
            Input::Base(base) => {
                self.set_base(base);
                Ok(())
            }
            Input::Word(bits) => {
                self.set_bits(bits);
                Ok(())
            }
        }
    }

    fn start_entry(&mut self) {
        if self.entry.is_some() {
            return;
        }
        if self.rpn && self.lift {
            self.stack.push(self.x);
        }
        self.entry = Some(String::new());
        self.shown = None;
        self.awaiting = false;
        self.lift = true;
    }

    fn digits(&mut self, digits: &[u8]) -> Result<(), Failure> {
        let radix = match self.mode {
            Mode::Programmer => self.base.radix(),
            _ => 10,
        };
        if digits.iter().any(|digit| u32::from(*digit) >= radix) {
            return Ok(());
        }
        self.start_entry();
        for digit in digits {
            let character = char::from_digit(u32::from(*digit), radix)
                .map(|character| character.to_ascii_uppercase())
                .ok_or(Failure::Undefined)?;
            let Some(entry) = self.entry.as_mut() else {
                return Ok(());
            };
            let candidate = match entry.as_str() {
                "0" => character.to_string(),
                "-0" => format!("-{character}"),
                _ => format!("{entry}{character}"),
            };
            let fits = match self.mode {
                Mode::Programmer => u64::from_str_radix(&candidate, radix)
                    .is_ok_and(|value| value & !mask(self.bits) == 0),
                _ => match candidate.split_once('e') {
                    Some((_, exponent)) => {
                        exponent.trim_start_matches('-').len() <= EXPONENT_DIGITS
                    }
                    None => {
                        candidate.chars().filter(char::is_ascii_digit).count()
                            <= SHOWN_DIGITS as usize
                    }
                },
            };
            if fits {
                *entry = candidate;
            }
        }
        Ok(())
    }

    fn point(&mut self) -> Result<(), Failure> {
        if self.mode == Mode::Programmer {
            return Ok(());
        }
        self.start_entry();
        if let Some(entry) = self.entry.as_mut()
            && !entry.contains(['.', 'e'])
        {
            if entry.is_empty() || entry == "-" {
                entry.push('0');
            }
            entry.push('.');
        }
        Ok(())
    }

    fn exponent(&mut self) -> Result<(), Failure> {
        if self.mode == Mode::Programmer {
            return Ok(());
        }
        self.start_entry();
        if let Some(entry) = self.entry.as_mut()
            && !entry.contains('e')
        {
            if entry.is_empty() || entry == "-" {
                entry.push('1');
            }
            entry.push('e');
        }
        Ok(())
    }

    fn negate_entry(&mut self) {
        let Some(entry) = self.entry.as_mut() else {
            return;
        };
        if self.mode == Mode::Programmer {
            if let Ok(value) = u64::from_str_radix(entry, self.base.radix()) {
                *entry = match self.base {
                    Base::Hexadecimal => format!("{:X}", value.wrapping_neg() & mask(self.bits)),
                    Base::Octal => format!("{:o}", value.wrapping_neg() & mask(self.bits)),
                    Base::Binary => format!("{:b}", value.wrapping_neg() & mask(self.bits)),
                    Base::Decimal => {
                        self.x = Value::Word(value.wrapping_neg() & mask(self.bits));
                        self.entry = None;
                        return;
                    }
                };
            }
            return;
        }
        *entry = match entry.split_once('e') {
            Some((number, exponent)) => match exponent.strip_prefix('-') {
                Some(positive) => format!("{number}e{positive}"),
                None => format!("{number}e-{exponent}"),
            },
            None => match entry.strip_prefix('-') {
                Some(positive) => positive.to_owned(),
                None => format!("-{entry}"),
            },
        };
    }

    fn backspace(&mut self) -> Result<(), Failure> {
        let Some(entry) = self.entry.as_mut() else {
            return match self.rpn {
                true => self.drop_x(),
                false => Ok(()),
            };
        };
        entry.pop();
        if entry.is_empty() || entry == "-" {
            *entry = "0".to_owned();
        }
        Ok(())
    }

    fn clear(&mut self) {
        if self.clears_all() {
            self.reset_expression();
            self.stack.clear();
        }
        self.entry = None;
        self.shown = None;
        self.x = self.zero();
        self.lift = false;
    }

    fn open(&mut self) {
        if self.rpn {
            return;
        }
        self.entry = None;
        self.shown = None;
        self.awaiting = false;
        self.x = self.zero();
        self.levels.push(Vec::new());
        self.tokens.push("(".to_owned());
    }

    fn close(&mut self) -> Result<(), Failure> {
        if self.rpn || self.levels.len() < 2 {
            return Ok(());
        }
        let text = self.operand_text();
        let mut x = self.commit()?;
        if let Some(level) = self.levels.pop() {
            for pending in level.into_iter().rev() {
                x = self.calculate(pending.left, pending.operator, x)?;
            }
        }
        self.tokens.extend([text, ")".to_owned()]);
        self.x = x;
        self.shown = Some(String::new());
        self.awaiting = false;
        Ok(())
    }

    fn reduce(&mut self, mut x: Value, before: Option<Binary>) -> Result<Value, Failure> {
        while let Some(level) = self.levels.last_mut() {
            let Some(last) = level.last() else {
                break;
            };
            let reduces = match before {
                None => true,
                Some(next) => {
                    last.operator.precedence() > next.precedence()
                        || (last.operator.precedence() == next.precedence()
                            && !next.right_associative())
                }
            };
            if !reduces {
                break;
            }
            let (left, operator) = (last.left, last.operator);
            level.pop();
            x = self.calculate(left, operator, x)?;
        }
        Ok(x)
    }

    fn binary(&mut self, operator: Binary) -> Result<(), Failure> {
        if self.awaiting
            && let Some(last) = self.levels.last_mut().and_then(|level| level.last_mut())
        {
            last.operator = operator;
            if let Some(token) = self.tokens.last_mut() {
                *token = operator.symbol().to_owned();
            }
            return Ok(());
        }
        let text = self.operand_text();
        let x = self.commit()?;
        let x = self.reduce(x, Some(operator))?;
        if let Some(level) = self.levels.last_mut() {
            level.push(Pending { left: x, operator });
        }
        if !text.is_empty() {
            self.tokens.push(text);
        }
        self.tokens.push(operator.symbol().to_owned());
        self.x = x;
        self.shown = None;
        self.awaiting = true;
        self.repeat = None;
        Ok(())
    }

    fn equals(&mut self) -> Result<(), Failure> {
        let text = self.operand_text();
        let x = self.commit()?;
        let nothing_pending = self.levels.iter().all(Vec::is_empty);
        if nothing_pending {
            if let Some((operator, operand)) = self.repeat {
                let result = self.calculate(x, operator, operand)?;
                self.record(
                    format!("{text} {} {}", operator.symbol(), self.format(operand)),
                    result,
                );
                self.x = result;
                self.shown = None;
            }
            return Ok(());
        }
        let mut tokens = std::mem::take(&mut self.tokens);
        if !text.is_empty() {
            tokens.push(text);
        }
        let closing = self.levels.len() - 1;
        tokens.extend(std::iter::repeat_n(")".to_owned(), closing));
        self.tokens = tokens;
        let expression = self.expression();
        self.repeat = self
            .levels
            .first()
            .filter(|_| closing == 0)
            .and_then(|level| level.last())
            .map(|pending| (pending.operator, x));
        let mut result = x;
        while !self.levels.is_empty() {
            result = self.reduce(result, None)?;
            self.levels.pop();
        }
        self.levels.push(Vec::new());
        self.tokens.clear();
        self.record(expression, result);
        self.x = result;
        self.shown = None;
        self.awaiting = false;
        Ok(())
    }

    fn record(&mut self, expression: String, result: Value) {
        let result = self.format(result);
        self.tape.push(format!("{expression} ="));
        self.tape.push(result);
    }

    fn percent(&mut self) -> Result<(), Failure> {
        let text = self.operand_text();
        let x = self.commit()?;
        let Value::Real(real) = x else {
            return Ok(());
        };
        let hundred = Decimal::integer(100)?;
        let base = match self.rpn {
            true => self.stack.last().copied(),
            false => self
                .levels
                .last()
                .and_then(|level| level.last())
                .filter(|pending| matches!(pending.operator, Binary::Add | Binary::Subtract))
                .map(|pending| pending.left),
        };
        let result = match base {
            Some(Value::Real(base)) => base.multiply(real)?.divide(hundred)?,
            _ => real.divide(hundred)?,
        };
        self.x = Value::Real(result);
        self.shown = Some(Unary::Percent.notation(&text));
        self.awaiting = false;
        self.lift = true;
        if self.rpn {
            self.record(Unary::Percent.notation(&text), self.x);
        }
        Ok(())
    }

    fn unary(&mut self, function: Unary) -> Result<(), Failure> {
        let text = self.operand_text();
        let x = self.commit()?;
        let result = match x {
            Value::Real(real) => Value::Real(self.real_function(function, real)?),
            Value::Word(word) => Value::Word(self.word_function(function, word)),
        };
        let shown = function.notation(&text);
        if self.rpn {
            self.record(shown.clone(), result);
        }
        self.x = result;
        self.shown = Some(shown);
        self.awaiting = false;
        self.lift = true;
        Ok(())
    }

    fn constant(&mut self, constant: Constant) -> Result<(), Failure> {
        if self.mode == Mode::Programmer {
            return Ok(());
        }
        let (value, shown) = match constant {
            Constant::Pi => (Decimal::parse(PI), Some("π".to_owned())),
            Constant::E => (Decimal::parse(E), Some("e".to_owned())),
            Constant::Random => (Some(random()?), None),
        };
        let value = value.ok_or(Failure::Undefined)?;
        self.load(Value::Real(value), shown);
        Ok(())
    }

    fn memory(&mut self, memory: Memory) -> Result<(), Failure> {
        let x = self.commit()?;
        let Value::Real(real) = x else {
            return Ok(());
        };
        match memory {
            Memory::Clear => self.memory = Decimal::ZERO,
            Memory::Add => self.memory = self.memory.plus(real)?,
            Memory::Subtract => self.memory = self.memory.subtract(real)?,
            Memory::Recall => self.load(Value::Real(self.memory), None),
        }
        if memory != Memory::Recall {
            self.shown = None;
            self.lift = true;
        }
        Ok(())
    }

    fn enter(&mut self) -> Result<(), Failure> {
        let x = self.commit()?;
        self.stack.push(x);
        self.shown = None;
        self.lift = false;
        Ok(())
    }

    fn rpn_binary(&mut self, operator: Binary) -> Result<(), Failure> {
        let x = self.commit()?;
        let y = self.stack.pop().unwrap_or(self.zero());
        let result = self.calculate(y, operator, x)?;
        self.record(
            format!(
                "{} {} {}",
                self.format(y),
                operator.symbol(),
                self.format(x)
            ),
            result,
        );
        self.x = result;
        self.shown = None;
        self.lift = true;
        Ok(())
    }

    fn swap(&mut self) -> Result<(), Failure> {
        let x = self.commit()?;
        let y = self.stack.pop().unwrap_or(self.zero());
        self.stack.push(x);
        self.x = y;
        self.shown = None;
        self.lift = true;
        Ok(())
    }

    fn roll(&mut self, down: bool) -> Result<(), Failure> {
        let x = self.commit()?;
        self.stack.push(x);
        match down {
            true => self.stack.rotate_right(1),
            false => self.stack.rotate_left(1),
        }
        self.x = self.stack.pop().unwrap_or(x);
        self.shown = None;
        self.lift = true;
        Ok(())
    }

    fn drop_x(&mut self) -> Result<(), Failure> {
        self.entry = None;
        self.x = self.stack.pop().unwrap_or(self.zero());
        self.shown = None;
        self.lift = true;
        Ok(())
    }

    fn calculate(&self, left: Value, operator: Binary, right: Value) -> Result<Value, Failure> {
        match (left, right) {
            (Value::Real(left), Value::Real(right)) => {
                Ok(Value::Real(real_binary(left, operator, right)?))
            }
            (Value::Word(left), Value::Word(right)) => {
                Ok(Value::Word(self.word_binary(left, operator, right)?))
            }
            _ => Err(Failure::Undefined),
        }
    }

    fn word_binary(&self, left: u64, operator: Binary, right: u64) -> Result<u64, Failure> {
        let bits = self.bits;
        let value = match operator {
            Binary::Add => left.wrapping_add(right),
            Binary::Subtract => left.wrapping_sub(right),
            Binary::Multiply => left.wrapping_mul(right),
            Binary::Divide => {
                if right == 0 {
                    return Err(Failure::Undefined);
                }
                signed(left, bits).wrapping_div(signed(right, bits)) as u64
            }
            Binary::And => left & right,
            Binary::Or => left | right,
            Binary::Xor => left ^ right,
            Binary::Nor => !(left | right),
            Binary::ShiftLeft => match right < u64::from(bits) {
                true => left << right,
                false => 0,
            },
            Binary::ShiftRight => match right < u64::from(bits) {
                true => left >> right,
                false => 0,
            },
            Binary::Power | Binary::PowerOf | Binary::Root | Binary::Log => {
                return Err(Failure::Undefined);
            }
        };
        Ok(value & mask(bits))
    }

    fn word_function(&self, function: Unary, word: u64) -> u64 {
        let bits = self.bits;
        let value = match function {
            Unary::Negate | Unary::TwosComplement => word.wrapping_neg(),
            Unary::OnesComplement => !word,
            Unary::ShiftLeftOne => word << 1,
            Unary::ShiftRightOne => word >> 1,
            Unary::RotateLeft => (word << 1) | (word >> (bits - 1)),
            Unary::RotateRight => (word >> 1) | ((word & 1) << (bits - 1)),
            Unary::ByteFlip => word.swap_bytes() >> (64 - bits),
            Unary::WordFlip => (0..bits / 16).fold(0, |flipped, index| {
                let piece = (word >> (index * 16)) & 0xFFFF;
                flipped | (piece << (bits - 16 - index * 16))
            }),
            _ => word,
        };
        match (function, bits) {
            (Unary::WordFlip, 8) => word,
            _ => value & mask(bits),
        }
    }

    fn real_function(&self, function: Unary, x: Decimal) -> Result<Decimal, Failure> {
        let float = x.to_float();
        let angle = |value: f64| match self.angle {
            Angle::Degrees => value.to_degrees(),
            Angle::Radians => value,
        };
        let floated = |value: f64| Decimal::from_float(value);
        let positive = |value: Decimal| match value.is_negative() || value.is_zero() {
            true => Err(Failure::Undefined),
            false => Ok(value.to_float()),
        };
        match function {
            Unary::Negate | Unary::TwosComplement => Ok(x.negate()),
            Unary::Percent => x.divide(Decimal::integer(100)?),
            Unary::Square => x.multiply(x),
            Unary::Cube => x.powi(3),
            Unary::Reciprocal => Decimal::ONE.divide(x),
            Unary::SquareRoot => x.sqrt(),
            Unary::CubeRoot => floated(float.cbrt()),
            Unary::Exp => floated(float.exp()),
            Unary::TenPower => power(Decimal::integer(10)?, x),
            Unary::TwoPower => power(Decimal::integer(2)?, x),
            Unary::Ln => floated(positive(x)?.ln()),
            Unary::Log10 if x.is_power_of_ten() => {
                Decimal::integer(i128::from(x.leading_exponent()))
            }
            Unary::Log10 => floated(positive(x)?.log10()),
            Unary::Log2 => floated(positive(x)?.log2()),
            Unary::Factorial => factorial(x),
            Unary::Sin | Unary::Cos | Unary::Tan => self.trigonometry(function, x),
            Unary::Asin => floated(angle(float.asin())),
            Unary::Acos => floated(angle(float.acos())),
            Unary::Atan => floated(angle(float.atan())),
            Unary::Sinh => floated(float.sinh()),
            Unary::Cosh => floated(float.cosh()),
            Unary::Tanh => floated(float.tanh()),
            Unary::Asinh => floated(float.asinh()),
            Unary::Acosh => floated(float.acosh()),
            Unary::Atanh => match float.abs() < 1.0 {
                true => floated(float.atanh()),
                false => Err(Failure::Undefined),
            },
            Unary::ShiftLeftOne
            | Unary::ShiftRightOne
            | Unary::RotateLeft
            | Unary::RotateRight
            | Unary::OnesComplement
            | Unary::ByteFlip
            | Unary::WordFlip => Ok(x),
        }
    }

    fn trigonometry(&self, function: Unary, x: Decimal) -> Result<Decimal, Failure> {
        let exact = match self.angle {
            Angle::Degrees => exact_degrees(x)?,
            Angle::Radians => None,
        };
        if let Some(degrees) = exact {
            let (sine, cosine) = match degrees {
                0 => (0, 1),
                90 => (1, 0),
                180 => (0, -1),
                _ => (-1, 0),
            };
            return match function {
                Unary::Sin => Decimal::integer(sine),
                Unary::Cos => Decimal::integer(cosine),
                _ if cosine == 0 => Err(Failure::Undefined),
                _ => Decimal::integer(sine * cosine),
            };
        }
        let radians = match self.angle {
            Angle::Degrees => reduced_degrees(x)?.to_float().to_radians(),
            Angle::Radians => x.to_float(),
        };
        let value = match function {
            Unary::Sin => radians.sin(),
            Unary::Cos => radians.cos(),
            _ => radians.tan(),
        };
        if value.abs() < radians.abs() * NEGLIGIBLE {
            return Ok(Decimal::ZERO);
        }
        Decimal::from_float(value)
    }
}

fn power(base: Decimal, exponent: Decimal) -> Result<Decimal, Failure> {
    if exponent.is_integer()
        && let Some(whole) = exponent
            .to_integer()
            .and_then(|whole| i64::try_from(whole).ok())
        && whole.unsigned_abs() <= 100_000
    {
        if base.is_zero() && whole < 0 {
            return Err(Failure::Undefined);
        }
        return base.powi(whole);
    }
    if base.is_negative() {
        return Err(Failure::Undefined);
    }
    Decimal::from_float(base.to_float().powf(exponent.to_float()))
}

fn root(value: Decimal, degree: Decimal) -> Result<Decimal, Failure> {
    if degree.is_zero() {
        return Err(Failure::Undefined);
    }
    if degree == Decimal::integer(2)? {
        return value.sqrt();
    }
    let odd = degree
        .to_integer()
        .is_some_and(|whole| degree.is_integer() && whole % 2 != 0);
    match (value.is_negative(), odd) {
        (true, true) => Ok(root(value.negate(), degree)?.negate()),
        (true, false) => Err(Failure::Undefined),
        (false, _) => Decimal::from_float(value.to_float().powf(1.0 / degree.to_float())),
    }
}

fn real_binary(left: Decimal, operator: Binary, right: Decimal) -> Result<Decimal, Failure> {
    match operator {
        Binary::Add => left.plus(right),
        Binary::Subtract => left.subtract(right),
        Binary::Multiply => left.multiply(right),
        Binary::Divide => left.divide(right),
        Binary::Power => power(left, right),
        Binary::PowerOf => power(right, left),
        Binary::Root => root(left, right),
        Binary::Log => {
            if left.is_negative() || left.is_zero() || right.is_negative() || right.is_zero() {
                return Err(Failure::Undefined);
            }
            let base = right.to_float().ln();
            if base == 0.0 {
                return Err(Failure::Undefined);
            }
            Decimal::from_float(left.to_float().ln() / base)
        }
        Binary::And
        | Binary::Or
        | Binary::Xor
        | Binary::Nor
        | Binary::ShiftLeft
        | Binary::ShiftRight => Err(Failure::Undefined),
    }
}

fn factorial(x: Decimal) -> Result<Decimal, Failure> {
    let whole = x
        .to_integer()
        .filter(|whole| x.is_integer() && (0..=LARGEST_FACTORIAL).contains(whole))
        .ok_or(Failure::Undefined)?;
    (2..=whole).try_fold(Decimal::ONE, |product, factor| {
        product.multiply(Decimal::integer(factor)?)
    })
}
