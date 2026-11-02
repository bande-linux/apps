use std::cmp::Ordering;

pub const PRECISION: u32 = 18;
const GUARD: u32 = 19;
const MAX_EXPONENT: i64 = 9999;
const FLOAT_DIGITS: usize = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Overflow,
    Undefined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decimal {
    negative: bool,
    mantissa: u64,
    exponent: i32,
}

fn power(digits: u32) -> u128 {
    10u128.pow(digits)
}

fn digits(value: u128) -> u32 {
    value.checked_ilog10().map_or(1, |log| log + 1)
}

fn build(negative: bool, magnitude: u128, exponent: i64, sticky: bool) -> Result<Decimal, Failure> {
    if magnitude == 0 {
        return Ok(Decimal::ZERO);
    }
    let (mut magnitude, mut exponent) = (magnitude, exponent);
    let count = digits(magnitude);
    if count > PRECISION {
        let dropped = count - PRECISION;
        let divisor = power(dropped);
        let (quotient, remainder) = (magnitude / divisor, magnitude % divisor);
        let half = divisor / 2;
        let up = remainder > half || (remainder == half && (sticky || quotient % 2 == 1));
        magnitude = quotient + u128::from(up);
        exponent += i64::from(dropped);
        if magnitude == power(PRECISION) {
            magnitude /= 10;
            exponent += 1;
        }
    }
    while magnitude % 10 == 0 {
        magnitude /= 10;
        exponent += 1;
    }
    let leading = exponent + i64::from(digits(magnitude)) - 1;
    if leading > MAX_EXPONENT {
        return Err(Failure::Overflow);
    }
    if leading < -MAX_EXPONENT {
        return Ok(Decimal::ZERO);
    }
    Ok(Decimal {
        negative,
        mantissa: u64::try_from(magnitude).map_err(|_| Failure::Overflow)?,
        exponent: i32::try_from(exponent).map_err(|_| Failure::Overflow)?,
    })
}

impl Decimal {
    pub const ZERO: Self = Self {
        negative: false,
        mantissa: 0,
        exponent: 0,
    };
    pub const ONE: Self = Self {
        negative: false,
        mantissa: 1,
        exponent: 0,
    };

    pub fn integer(value: i128) -> Result<Self, Failure> {
        build(value < 0, value.unsigned_abs(), 0, false)
    }

    pub fn power_of_ten(exponent: i64) -> Result<Self, Failure> {
        build(false, 1, exponent, false)
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (negative, text) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (number, scale) = match text.split_once(['e', 'E']) {
            Some((number, scale)) => (number, scale.parse::<i64>().ok()?),
            None => (text, 0),
        };
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        let mut magnitude: u128 = 0;
        let mut exponent = scale;
        let mut sticky = false;
        for (index, character) in whole.chars().chain(fraction.chars()).enumerate() {
            let digit = u128::from(character.to_digit(10)?);
            if index >= whole.len() {
                exponent -= 1;
            }
            if digits(magnitude) > GUARD + 1 {
                sticky |= digit != 0;
                exponent += 1;
            } else {
                magnitude = magnitude * 10 + digit;
            }
        }
        build(negative, magnitude, exponent, sticky).ok()
    }

    pub fn from_float(value: f64) -> Result<Self, Failure> {
        if value.is_nan() {
            return Err(Failure::Undefined);
        }
        if value.is_infinite() {
            return Err(Failure::Overflow);
        }
        Self::parse(&format!(
            "{value:.precision$e}",
            precision = FLOAT_DIGITS - 1
        ))
        .ok_or(Failure::Undefined)
    }

    pub fn to_float(self) -> f64 {
        format!("{}{}e{}", self.sign(), self.mantissa, self.exponent)
            .parse()
            .unwrap_or(f64::NAN)
    }

    fn sign(self) -> &'static str {
        match self.negative {
            true => "-",
            false => "",
        }
    }

    pub fn is_zero(self) -> bool {
        self.mantissa == 0
    }

    pub fn is_negative(self) -> bool {
        self.negative && !self.is_zero()
    }

    pub fn negate(self) -> Self {
        Self {
            negative: !self.negative && !self.is_zero(),
            ..self
        }
    }

    pub fn abs(self) -> Self {
        Self {
            negative: false,
            ..self
        }
    }

    pub fn is_power_of_ten(self) -> bool {
        self.mantissa == 1 && !self.negative
    }

    pub fn is_integer(self) -> bool {
        self.exponent >= 0
    }

    pub fn to_integer(self) -> Option<i128> {
        let exponent = u32::try_from(self.exponent).ok()?;
        let magnitude = i128::from(self.mantissa).checked_mul(10i128.checked_pow(exponent)?)?;
        Some(if self.negative { -magnitude } else { magnitude })
    }

    pub fn trunc(self) -> Self {
        if self.exponent >= 0 {
            return self;
        }
        let dropped = self.exponent.unsigned_abs();
        if dropped > PRECISION {
            return Self::ZERO;
        }
        build(
            self.negative,
            u128::from(self.mantissa) / power(dropped),
            0,
            false,
        )
        .unwrap_or(Self::ZERO)
    }

    pub fn plus(self, other: Self) -> Result<Self, Failure> {
        if self.is_zero() {
            return Ok(other);
        }
        if other.is_zero() {
            return Ok(self);
        }
        let (big, small) = match self.exponent >= other.exponent {
            true => (self, other),
            false => (other, self),
        };
        let gap = big.exponent.abs_diff(small.exponent);
        let shift = gap.min(GUARD);
        let rest = gap - shift;
        let signed = |negative: bool, value: u128| match negative {
            true => -(value as i128),
            false => value as i128,
        };
        let big_part = signed(big.negative, u128::from(big.mantissa) * power(shift));
        let (small_part, sticky) = match rest {
            0 => (u128::from(small.mantissa), false),
            rest if rest > GUARD => (0, true),
            rest => (
                u128::from(small.mantissa) / power(rest),
                u128::from(small.mantissa) % power(rest) != 0,
            ),
        };
        let sum = big_part + signed(small.negative, small_part);
        let negative = sum < 0;
        let mut magnitude = sum.unsigned_abs();
        if sticky && small.negative != negative {
            magnitude -= 1;
        }
        build(
            negative,
            magnitude,
            i64::from(big.exponent) - i64::from(shift),
            sticky,
        )
    }

    pub fn subtract(self, other: Self) -> Result<Self, Failure> {
        self.plus(other.negate())
    }

    pub fn multiply(self, other: Self) -> Result<Self, Failure> {
        build(
            self.negative != other.negative,
            u128::from(self.mantissa) * u128::from(other.mantissa),
            i64::from(self.exponent) + i64::from(other.exponent),
            false,
        )
    }

    pub fn divide(self, other: Self) -> Result<Self, Failure> {
        if other.is_zero() {
            return Err(Failure::Undefined);
        }
        if self.is_zero() {
            return Ok(Self::ZERO);
        }
        let mantissa = u128::from(self.mantissa);
        let scale = PRECISION - digits(mantissa);
        let numerator = mantissa * power(scale + GUARD);
        let divisor = u128::from(other.mantissa);
        build(
            self.negative != other.negative,
            numerator / divisor,
            i64::from(self.exponent) - i64::from(scale + GUARD) - i64::from(other.exponent),
            !numerator.is_multiple_of(divisor),
        )
    }

    pub fn sqrt(self) -> Result<Self, Failure> {
        if self.is_negative() {
            return Err(Failure::Undefined);
        }
        if self.is_zero() {
            return Ok(Self::ZERO);
        }
        let mantissa = u128::from(self.mantissa);
        let mut scale = 2 * GUARD - digits(mantissa);
        if (i64::from(self.exponent) - i64::from(scale)) % 2 != 0 {
            scale -= 1;
        }
        let widened = mantissa * power(scale);
        let root = widened.isqrt();
        build(
            false,
            root,
            (i64::from(self.exponent) - i64::from(scale)) / 2,
            root * root != widened,
        )
    }

    pub fn powi(self, exponent: i64) -> Result<Self, Failure> {
        if exponent < 0 {
            return Self::ONE.divide(self.powi(-exponent)?);
        }
        let (mut result, mut base, mut left) = (Self::ONE, self, exponent.unsigned_abs());
        while left > 0 {
            if left & 1 == 1 {
                result = result.multiply(base)?;
            }
            left >>= 1;
            if left > 0 {
                base = base.multiply(base)?;
            }
        }
        Ok(result)
    }

    pub fn compare(self, other: Self) -> Ordering {
        match self.subtract(other) {
            Ok(difference) if difference.is_zero() => Ordering::Equal,
            Ok(difference) if difference.is_negative() => Ordering::Less,
            Ok(_) => Ordering::Greater,
            Err(_) => self.to_float().total_cmp(&other.to_float()),
        }
    }

    pub fn rounded(self, significant: u32) -> Self {
        let count = digits(u128::from(self.mantissa));
        if count <= significant {
            return self;
        }
        let divisor = power(count - significant);
        let magnitude = u128::from(self.mantissa);
        let quotient = magnitude / divisor;
        let up = magnitude % divisor >= divisor / 2;
        build(
            self.negative,
            quotient + u128::from(up),
            i64::from(self.exponent) + i64::from(count - significant),
            false,
        )
        .unwrap_or(self)
    }

    pub fn leading_exponent(self) -> i64 {
        i64::from(self.exponent) + i64::from(digits(u128::from(self.mantissa))) - 1
    }

    pub fn plain(self) -> String {
        let digits = self.mantissa.to_string();
        let body = match self.exponent {
            exponent if exponent >= 0 => {
                format!("{digits}{}", "0".repeat(exponent.unsigned_abs() as usize))
            }
            exponent => {
                let fraction = exponent.unsigned_abs() as usize;
                match digits.len() > fraction {
                    true => format!(
                        "{}.{}",
                        &digits[..digits.len() - fraction],
                        &digits[digits.len() - fraction..]
                    ),
                    false => format!("0.{}{digits}", "0".repeat(fraction - digits.len())),
                }
            }
        };
        format!("{}{body}", self.sign())
    }

    pub fn scientific(self) -> String {
        let digits = self.mantissa.to_string();
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        format!(
            "{}{first}{point}{rest}e{}",
            self.sign(),
            self.leading_exponent()
        )
    }
}
