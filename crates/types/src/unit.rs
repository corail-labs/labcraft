//! Physical units (spec 3.3): a vector of exponents over the 7 base dimensions, and a scale.

use std::fmt;
use std::iter::Peekable;
use std::str::Chars;

use crate::code::Code;
use crate::limits::MAX_NESTING;
use crate::name::normalize;

const DIMENSIONS: usize = 7;
const DIMENSION_NAMES: [&str; DIMENSIONS] = ["m", "kg", "s", "A", "K", "mol", "cd"];

/// The scale of a unit, as exponents of 2, 3 and 5: every unit of the table is exact in these
/// (`min` is 2^2 * 3 * 5, `°F` is 5 / 3^2), so two units are equal whatever the order of the
/// operations that made them, and on every target. A floating-point factor gives neither.
const SCALE_PRIMES: usize = 3;
type Scale = [i32; SCALE_PRIMES];
const PRIMES: [f64; SCALE_PRIMES] = [2.0, 3.0, 5.0];
const PRIMES_LOG10: [f64; SCALE_PRIMES] = [std::f64::consts::LOG10_2, 0.477_121_254_719_662_4, 0.698_970_004_336_018_8];

/// Beyond this power of ten, a scale cannot be applied to a number (`LC3016`). It is checked on each
/// side of the fraction, which also bounds each exponent, and with it the work of writing a scale.
const MAX_SCALE_LOG10: f64 = 300.0;

const fn ten(power: i32) -> Scale {
    [power, 0, power]
}

/// A temperature scale: kelvins = (value + 273.15) for `°C`, (value + 459.67) * 5 / 9 for `°F`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Offset {
    None,
    Celsius,
    Fahrenheit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Unit {
    exponents: [i8; DIMENSIONS],
    scale: Scale,
    offset: Offset,
}

const fn unit(exponents: [i8; DIMENSIONS], scale: Scale, offset: Offset) -> Unit {
    Unit { exponents, scale, offset }
}

struct NamedUnit {
    name: &'static str,
    unit: Unit,
    takes_a_prefix: bool,
}

const fn entry(name: &'static str, exponents: [i8; DIMENSIONS]) -> NamedUnit {
    NamedUnit { name, unit: unit(exponents, ten(0), Offset::None), takes_a_prefix: true }
}

const fn no_prefix(name: &'static str, exponents: [i8; DIMENSIONS], scale: Scale, offset: Offset) -> NamedUnit {
    NamedUnit { name, unit: unit(exponents, scale, offset), takes_a_prefix: false }
}

/// The table of spec 3.3, in the order that display prefers. Exponents: m, kg, s, A, K, mol, cd.
const NAMED_UNITS: &[NamedUnit] = &[
    entry("m", [1, 0, 0, 0, 0, 0, 0]),
    no_prefix("kg", [0, 1, 0, 0, 0, 0, 0], ten(0), Offset::None),
    NamedUnit { name: "g", unit: unit([0, 1, 0, 0, 0, 0, 0], ten(-3), Offset::None), takes_a_prefix: true },
    entry("s", [0, 0, 1, 0, 0, 0, 0]),
    entry("A", [0, 0, 0, 1, 0, 0, 0]),
    entry("K", [0, 0, 0, 0, 1, 0, 0]),
    entry("mol", [0, 0, 0, 0, 0, 1, 0]),
    entry("cd", [0, 0, 0, 0, 0, 0, 1]),
    entry("Hz", [0, 0, -1, 0, 0, 0, 0]),
    entry("N", [1, 1, -2, 0, 0, 0, 0]),
    entry("Pa", [-1, 1, -2, 0, 0, 0, 0]),
    entry("J", [2, 1, -2, 0, 0, 0, 0]),
    entry("W", [2, 1, -3, 0, 0, 0, 0]),
    entry("C", [0, 0, 1, 1, 0, 0, 0]),
    entry("V", [2, 1, -3, -1, 0, 0, 0]),
    entry("F", [-2, -1, 4, 2, 0, 0, 0]),
    entry("\u{03A9}", [2, 1, -3, -2, 0, 0, 0]),
    entry("S", [-2, -1, 3, 2, 0, 0, 0]),
    entry("Wb", [2, 1, -2, -1, 0, 0, 0]),
    entry("T", [0, 1, -2, -1, 0, 0, 0]),
    entry("H", [2, 1, -2, -2, 0, 0, 0]),
    no_prefix("min", [0, 0, 1, 0, 0, 0, 0], [2, 1, 1], Offset::None),
    no_prefix("h", [0, 0, 1, 0, 0, 0, 0], [4, 2, 2], Offset::None),
    no_prefix("\u{00B0}C", [0, 0, 0, 0, 1, 0, 0], ten(0), Offset::Celsius),
    no_prefix("\u{00B0}F", [0, 0, 0, 0, 1, 0, 0], [0, -2, 1], Offset::Fahrenheit),
];

const PREFIXES: [(char, i32); 7] = [('p', -12), ('n', -9), ('\u{00B5}', -6), ('m', -3), ('k', 3), ('M', 6), ('G', 9)];

impl NamedUnit {
    fn with_prefix(&self, power: i32) -> Unit {
        let [two, three, five] = self.unit.scale;
        Unit { scale: [two + power, three, five + power], ..self.unit }
    }
}

/// A whole number in base 10^9, least significant first: the side of a ratio of scales can reach 10^300, and its
/// nearest `f64` comes from its decimal text, which `parse` rounds correctly on every target.
struct Decimal(Vec<u64>);

impl Decimal {
    const BASE: u64 = 1_000_000_000;

    fn one() -> Self {
        Decimal(vec![1])
    }

    fn multiply(&mut self, by: u32) {
        let mut carry = 0_u64;
        for limb in &mut self.0 {
            let product = *limb * u64::from(by) + carry;
            *limb = product % Self::BASE;
            carry = product / Self::BASE;
        }
        if carry > 0 {
            self.0.push(carry);
        }
    }

    fn nearest(&self) -> f64 {
        let mut limbs = self.0.iter().rev();
        let mut text = limbs.next().map_or_else(String::new, u64::to_string);
        for limb in limbs {
            text.push_str(&format!("{limb:09}"));
        }
        text.parse().unwrap_or(f64::INFINITY)
    }
}

/// An exponent stays within ±127 (`docs/cli.md`, `LC1006`), which `i8` alone does not say.
fn exponent_in_bounds(exponent: i8) -> Option<i8> {
    (exponent != i8::MIN).then_some(exponent)
}

fn scale_in_bounds(scale: Scale) -> Option<Scale> {
    let side = |sign: f64| -> f64 { scale.iter().zip(PRIMES_LOG10).map(|(exponent, log)| (f64::from(*exponent) * sign).max(0.0) * log).sum() };
    (side(1.0) <= MAX_SCALE_LOG10 && side(-1.0) <= MAX_SCALE_LOG10).then_some(scale)
}

/// `mul` and `div`: the same operation on the exponents and on the scale. The result has no offset.
fn combine(a: &Unit, b: &Unit, on_exponents: fn(i8, i8) -> Option<i8>, on_scale: fn(i32, i32) -> Option<i32>) -> Option<Unit> {
    let mut exponents = [0; DIMENSIONS];
    for ((slot, x), y) in exponents.iter_mut().zip(&a.exponents).zip(&b.exponents) {
        *slot = exponent_in_bounds(on_exponents(*x, *y)?)?;
    }
    let mut scale = ten(0);
    for ((slot, x), y) in scale.iter_mut().zip(&a.scale).zip(&b.scale) {
        *slot = on_scale(*x, *y)?;
    }
    Some(Unit { exponents, scale: scale_in_bounds(scale)?, offset: Offset::None })
}

impl Unit {
    /// A dimensionless number, the default.
    pub const ONE: Unit = unit([0; DIMENSIONS], ten(0), Offset::None);

    /// What a `duration` converts to and from (spec 3.1).
    pub const SECOND: Unit = unit([0, 0, 1, 0, 0, 0, 0], ten(0), Offset::None);

    /// The difference of two temperatures (spec 3.3).
    pub const KELVIN: Unit = unit([0, 0, 0, 0, 1, 0, 0], ten(0), Offset::None);

    /// `°C` or `°F`, a scale with an offset.
    pub fn is_temperature_scale(&self) -> bool {
        self.offset != Offset::None
    }

    /// A unit of the table of named units, with its prefix, in its one spelling (R-20): `V`, `mV`.
    pub fn from_name(name: &str) -> Option<Unit> {
        let exact = |name: &str| NAMED_UNITS.iter().find(|named| named.name == name);
        if let Some(named) = exact(name) {
            return Some(named.unit);
        }
        let mut chars = name.chars();
        let first = chars.next()?;
        let (_, power) = PREFIXES.iter().find(|(prefix, _)| *prefix == first)?;
        let named = exact(chars.as_str()).filter(|named| named.takes_a_prefix)?;
        Some(named.with_prefix(*power))
    }

    /// `mul`: adds the exponents. `None` when an exponent or the scale goes out of bounds (`LC3016`).
    /// An operation drops the offset: in `V/°C`, a temperature counts as a difference.
    #[must_use]
    pub fn mul(&self, other: &Unit) -> Option<Unit> {
        combine(self, other, i8::checked_add, i32::checked_add)
    }

    /// `div`: subtracts the exponents.
    #[must_use]
    pub fn div(&self, other: &Unit) -> Option<Unit> {
        combine(self, other, i8::checked_sub, i32::checked_sub)
    }

    /// `pow`: multiplies the exponents by a constant integer.
    #[must_use]
    pub fn pow(&self, exponent: i32) -> Option<Unit> {
        let mut exponents = [0; DIMENSIONS];
        for (slot, x) in exponents.iter_mut().zip(&self.exponents) {
            *slot = exponent_in_bounds(i8::try_from(i32::from(*x).checked_mul(exponent)?).ok()?)?;
        }
        let mut scale = ten(0);
        for (slot, x) in scale.iter_mut().zip(&self.scale) {
            *slot = x.checked_mul(exponent)?;
        }
        Some(Unit { exponents, scale: scale_in_bounds(scale)?, offset: Offset::None })
    }

    /// `sqrt`: divides the exponents by 2. `None` when an exponent is odd, or when the scale is not an
    /// exact square, as in `sqrt(kV*V)`, whose value would not be exact either (`LC3016`).
    #[must_use]
    pub fn sqrt(&self) -> Option<Unit> {
        let mut exponents = [0; DIMENSIONS];
        for (slot, x) in exponents.iter_mut().zip(&self.exponents) {
            *slot = (x % 2 == 0).then_some(x / 2)?;
        }
        let mut scale = ten(0);
        for (slot, x) in scale.iter_mut().zip(&self.scale) {
            *slot = (x % 2 == 0).then_some(x / 2)?;
        }
        Some(Unit { exponents, scale, offset: Offset::None })
    }

    /// What a temperature scale adds to a value to make it start at absolute zero, in its own unit: kelvins are
    /// (value + offset) × scale, 273.15 for `°C` and 459.67 for `°F`. `None` for every other unit (spec 3.3).
    pub fn offset(&self) -> Option<f64> {
        match self.offset {
            Offset::None => None,
            Offset::Celsius => Some(273.15),
            Offset::Fahrenheit => Some(459.67),
        }
    }

    /// The ratio of the scale of this unit to that of `to`, as a numerator and a denominator in lowest terms, each the
    /// nearest `f64` of its integer (spec 3.3): `mV` to `V` is (1, 1000), `min` to `h` is (1, 60), `°F` to `K` is (5, 9).
    pub fn scale_ratio(&self, to: &Unit) -> (f64, f64) {
        let (mut numerator, mut denominator) = (Decimal::one(), Decimal::one());
        for ((from, to), prime) in self.scale.iter().zip(&to.scale).zip([2_u32, 3, 5]) {
            let exponent = i64::from(*from) - i64::from(*to);
            let side = if exponent > 0 { &mut numerator } else { &mut denominator };
            for _ in 0..exponent.unsigned_abs() {
                side.multiply(prime);
            }
        }
        (numerator.nearest(), denominator.nearest())
    }

    /// Whether a value can go from this unit to `to`: the ratio of their scales is a scale too, and stays within the bounds
    /// of one, 1e300 on each side of its fraction (spec 3.3, `LC3016`). `GV^33` to `mV^33` is 1e396, which no `f64` holds. Each
    /// unit is within those bounds, so a `duration`, in seconds, always converts.
    pub fn scales_to(&self, to: &Unit) -> bool {
        let mut ratio = ten(0);
        for ((slot, from), to) in ratio.iter_mut().zip(&self.scale).zip(&to.scale) {
            let Some(exponent) = from.checked_sub(*to) else { return false };
            *slot = exponent;
        }
        scale_in_bounds(ratio).is_some()
    }

    /// The condition of `add`, `sub` and comparisons, and of an implicit conversion (spec 3.3).
    pub fn same_dimension(&self, other: &Unit) -> bool {
        self.exponents == other.exponents
    }

    /// The name of this unit in the table, with its prefix.
    fn name(&self) -> Option<(Option<char>, &'static str)> {
        for named in NAMED_UNITS {
            if named.unit == *self {
                return Some((None, named.name));
            }
            if named.takes_a_prefix {
                for (prefix, power) in PREFIXES {
                    if named.with_prefix(power) == *self {
                        return Some((Some(prefix), named.name));
                    }
                }
            }
        }
        None
    }

    /// How many of `to` one of this unit is worth, offsets aside: what a literal holds once it is in `to` (spec 3.4).
    pub fn ratio_to(&self, to: &Unit) -> f64 {
        self.scale_value() / to.scale_value()
    }

    /// The scale as a number. The powers of ten are set apart first, since multiplying primes one at
    /// a time would round: `GV/V` cubed must be `1e27`, not `9.999999999999999e26`. They then join what
    /// is left through its decimal text, which is correctly rounded, on every target. `scale_in_bounds`
    /// keeps each side of the fraction under 1e300, and the exponents small.
    fn scale_value(&self) -> f64 {
        let [two, three, five] = self.scale;
        let ten = match (two > 0 && five > 0, two < 0 && five < 0) {
            (true, _) => two.min(five),
            (_, true) => two.max(five),
            _ => 0,
        };
        let (mut numerator, mut denominator) = (1.0_f64, 1.0_f64);
        for (exponent, prime) in [two - ten, three, five - ten].iter().zip(PRIMES) {
            for _ in 0..exponent.unsigned_abs() {
                if *exponent > 0 {
                    numerator *= prime;
                } else {
                    denominator *= prime;
                }
            }
        }
        let rest = numerator / denominator;
        let text = format!("{rest:e}");
        let Some((mantissa, power)) = text.split_once('e') else {
            return rest;
        };
        format!("{mantissa}e{}", power.parse::<i32>().unwrap_or(0) + ten).parse().unwrap_or(rest)
    }
}

impl fmt::Display for Unit {
    /// The name of the unit when the table has one (`V`, `mV`), otherwise the expanded form, which
    /// reads back with [`parse_unit`], unless it has to start with a scale that no unit name gives
    /// (`1000*m^2`). A dimensionless number with no scale is empty.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((prefix, name)) = self.name() {
            if let Some(prefix) = prefix {
                write!(f, "{prefix}")?;
            }
            return f.write_str(name);
        }
        let mut written = false;
        if self.scale != ten(0) {
            let value = self.scale_value();
            if (1e-6..1e7).contains(&value) {
                write!(f, "{value}")?;
            } else {
                write!(f, "{value:e}")?;
            }
            written = true;
        }
        for (name, exponent) in DIMENSION_NAMES.iter().zip(&self.exponents).filter(|(_, exponent)| **exponent > 0) {
            if written {
                f.write_str("*")?;
            }
            f.write_str(name)?;
            if *exponent != 1 {
                write!(f, "^{exponent}")?;
            }
            written = true;
        }
        for (name, exponent) in DIMENSION_NAMES.iter().zip(&self.exponents).filter(|(_, exponent)| **exponent < 0) {
            if !written {
                f.write_str("1")?;
                written = true;
            }
            write!(f, "/{name}")?;
            let power = -i16::from(*exponent);
            if power != 1 {
                write!(f, "^{power}")?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitError {
    Syntax,
    /// A name that the table of units does not have.
    Unknown(String),
    /// An exponent beyond ±127, written or computed, or a scale too large for a number.
    OutOfBounds,
    TooDeep,
}

impl UnitError {
    pub fn code(&self) -> Code {
        match self {
            UnitError::Syntax => Code::Lc1001,
            UnitError::Unknown(_) => Code::Lc2008,
            UnitError::OutOfBounds => Code::Lc1006,
            UnitError::TooDeep => Code::Lc0007,
        }
    }
}

/// Reads a compound unit (spec 14.3, `unit_expr`), as `check` does for a `UnitExpr` and for the
/// fields of a `string.format` template.
pub fn parse_unit(text: &str, max_nesting: u32) -> Result<Unit, UnitError> {
    let text = normalize(text);
    let mut parser = UnitParser { chars: text.chars().peekable(), max_nesting: max_nesting.min(MAX_NESTING) };
    let unit = parser.expression(0)?;
    parser.skip_spaces();
    if parser.chars.peek().is_some() {
        return Err(UnitError::Syntax);
    }
    Ok(unit)
}

struct UnitParser<'a> {
    chars: Peekable<Chars<'a>>,
    max_nesting: u32,
}

impl UnitParser<'_> {
    fn skip_spaces(&mut self) {
        while self.chars.next_if(|c| matches!(c, ' ' | '\t')).is_some() {}
    }

    /// `*`, `·` and `/` have equal precedence and apply from left to right.
    fn expression(&mut self, depth: u32) -> Result<Unit, UnitError> {
        let mut unit = self.term(depth)?;
        loop {
            self.skip_spaces();
            let divide = match self.chars.peek() {
                Some('*' | '\u{00B7}') => false,
                Some('/') => true,
                _ => return Ok(unit),
            };
            self.chars.next();
            let term = self.term(depth)?;
            unit = if divide { unit.div(&term) } else { unit.mul(&term) }.ok_or(UnitError::OutOfBounds)?;
        }
    }

    fn term(&mut self, depth: u32) -> Result<Unit, UnitError> {
        self.skip_spaces();
        if self.chars.next_if_eq(&'(').is_some() {
            if depth >= self.max_nesting {
                return Err(UnitError::TooDeep);
            }
            let unit = self.expression(depth + 1)?;
            self.skip_spaces();
            return if self.chars.next_if_eq(&')').is_some() { Ok(unit) } else { Err(UnitError::Syntax) };
        }
        let unit = self.atom()?;
        self.skip_spaces();
        if self.chars.next_if_eq(&'^').is_none() {
            return Ok(unit);
        }
        unit.pow(self.exponent()?).ok_or(UnitError::OutOfBounds)
    }

    fn atom(&mut self) -> Result<Unit, UnitError> {
        if self.chars.next_if_eq(&'1').is_some() {
            return if self.chars.peek().is_some_and(|c| is_name_char(*c)) { Err(UnitError::Syntax) } else { Ok(Unit::ONE) };
        }
        // `UNIT_NAME` and `IDENT` start with a letter or `°` (spec 14.1).
        if !self.chars.peek().is_some_and(|c| c.is_alphabetic() || *c == '\u{00B0}') {
            return Err(UnitError::Syntax);
        }
        let mut name = String::new();
        while let Some(c) = self.chars.next_if(|c| is_name_char(*c)) {
            name.push(c);
        }
        Unit::from_name(&name).ok_or(UnitError::Unknown(name))
    }

    fn exponent(&mut self) -> Result<i32, UnitError> {
        self.skip_spaces();
        let negative = self.chars.next_if_eq(&'-').is_some();
        self.skip_spaces();
        if !self.chars.peek().is_some_and(char::is_ascii_digit) {
            return Err(UnitError::Syntax);
        }
        // Saturating: `m^99999999…` stays out of bounds, however long it is written.
        let mut value = 0_i32;
        while let Some(c) = self.chars.next_if(|c| c.is_ascii_digit() || *c == '_') {
            if let Some(digit) = c.to_digit(10) {
                value = value.saturating_mul(10).saturating_add(digit as i32);
            }
        }
        if value > i32::from(i8::MAX) {
            return Err(UnitError::OutOfBounds);
        }
        Ok(if negative { -value } else { value })
    }
}

/// Whether a name exists is for the table to say, so this only finds where a name ends: at an
/// operator, a bracket or a space.
fn is_name_char(c: char) -> bool {
    !c.is_whitespace() && c != '\u{00B7}' && (c == '_' || !c.is_ascii_punctuation())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Unit {
        parse_unit(text, 256).unwrap()
    }

    fn shown(text: &str) -> String {
        parsed(text).to_string()
    }

    /// Worked out by hand from the scales of spec 3.3: a numerator and a denominator in lowest terms, each an integer.
    #[test]
    fn a_ratio_of_scales_is_two_integers() {
        assert_eq!(parsed("mV").scale_ratio(&parsed("V")), (1.0, 1000.0));
        assert_eq!(parsed("kV").scale_ratio(&parsed("mV")), (1_000_000.0, 1.0));
        // 1 h is 3600 s and 1 min is 60 s: 60.
        assert_eq!(parsed("h").scale_ratio(&parsed("min")), (60.0, 1.0));
        // °F is 5/9 of a kelvin.
        assert_eq!(parsed("°F").scale_ratio(&parsed("K")), (5.0, 9.0));
        assert_eq!(parsed("V").scale_ratio(&parsed("V")), (1.0, 1.0));
        // GV/mV is 10^12, cubed 10^36, which `u64` cannot hold: its nearest `f64` is that of its decimal text.
        assert_eq!(parsed("GV^3").scale_ratio(&parsed("mV^3")), ("1e36".parse::<f64>().unwrap(), 1.0));
        assert_eq!((parsed("°C").offset(), parsed("°F").offset(), parsed("K").offset()), (Some(273.15), Some(459.67), None));
    }

    /// Worked out by hand: `GV` to `mV` is 10^12, so `GV^25` to `mV^25` is 10^300, the last power within bounds, and `^26` is
    /// 10^312. `°F` to `K` is 5/9 whatever the power, below the bounds; `GV^33` alone is 10^297.
    #[test]
    fn a_ratio_of_scales_beyond_bounds_does_not_convert() {
        assert!(parsed("GV^25").scales_to(&parsed("mV^25")));
        assert!(parsed("mV^25").scales_to(&parsed("GV^25")));
        assert!(!parsed("GV^26").scales_to(&parsed("mV^26")));
        assert!(!parsed("mV^26").scales_to(&parsed("GV^26")));
        assert!(!parsed("GV^33").scales_to(&parsed("mV^33")));
        assert!(parsed("GV^33").scales_to(&parsed("V^33")));
        assert!(parsed("°F").scales_to(&parsed("K")));
        assert!(parsed("V").scales_to(&parsed("V")));
        assert_eq!(parsed("GV^25").scale_ratio(&parsed("mV^25")), ("1e300".parse::<f64>().unwrap(), 1.0));
    }

    #[test]
    fn dimensional_analysis_of_the_spec() {
        assert_eq!(parsed("V/A"), parsed("\u{03A9}"));
        assert_eq!(parsed("\u{03A9}").mul(&parsed("A")), Some(parsed("V")));
        assert_eq!(parsed("V").mul(&parsed("A")), Some(parsed("W")));
        assert_eq!(parsed("V").div(&parsed("A")), Some(parsed("\u{03A9}")));
        assert_eq!(parsed("V").pow(2).unwrap().sqrt(), Some(parsed("V")));
    }

    #[test]
    fn the_operators_have_equal_precedence() {
        assert_eq!(parsed("kg/m*s"), parsed("kg/m").mul(&parsed("s")).unwrap());
        assert_eq!(parsed("kg/(m*s)"), parsed("kg").div(&parsed("m*s")).unwrap());
        assert_ne!(parsed("kg/m*s"), parsed("kg/(m*s)"));
        assert_eq!(parsed("kg\u{00B7}m"), parsed("kg*m"));
    }

    #[test]
    fn a_caret_applies_to_the_unit_just_before_it() {
        assert_eq!(parsed("m/s^2"), parsed("m").div(&parsed("s").pow(2).unwrap()).unwrap());
        assert_eq!(parsed("s^-1"), parsed("Hz"));
        assert_eq!(parsed("1/s"), parsed("Hz"));
        assert_eq!(parsed("m^0"), Unit::ONE);
    }

    #[test]
    fn spellings_are_one_unit() {
        assert_eq!(parsed("\u{2126}"), parsed("\u{03A9}"));
        assert_eq!(parsed("\u{03BC}V"), parsed("\u{00B5}V"));
        assert_eq!(parsed("k\u{2126}").to_string(), "k\u{03A9}");
    }

    #[test]
    fn prefixes_scale_and_keep_the_dimension() {
        assert!(parsed("mV").same_dimension(&parsed("V")));
        assert!(!parsed("V").same_dimension(&parsed("A")));
        assert_ne!(parsed("mV"), parsed("V"));
        assert_eq!(parsed("kV").mul(&parsed("mV")), parsed("V").pow(2));
        assert_eq!(parsed("kg"), parsed("g").mul(&parsed("kV/V")).unwrap());
        assert_eq!(parsed("Mg"), parsed("kg").mul(&parsed("kV/V")).unwrap());
    }

    #[test]
    fn minutes_and_hours_are_exact() {
        assert_eq!(parsed("h"), parsed("min*min/s"));
        assert_eq!(parsed("min").div(&parsed("s")).unwrap().scale_value(), 60.0);
        assert_eq!(parsed("h").div(&parsed("s")).unwrap().scale_value(), 3600.0);
    }

    #[test]
    fn equality_does_not_depend_on_the_order_of_operations() {
        assert_eq!(parsed("V/\u{00B0}F/\u{00B0}F*\u{00B0}F*\u{00B0}F"), parsed("V"));
        assert_eq!(parsed("\u{00B0}F^-2"), parsed("1/\u{00B0}F/\u{00B0}F"));
        let units = ["min", "h", "\u{00B0}F", "mV", "kV", "\u{00B5}A", "g"];
        for a in units {
            for b in units {
                let (a, b) = (parsed(a), parsed(b));
                assert_eq!(a.mul(&b), b.mul(&a));
                assert_eq!(a.mul(&b).unwrap().div(&b).unwrap().div(&a), Some(Unit::ONE));
                assert_eq!(a.div(&b).unwrap().mul(&b).unwrap().div(&a), Some(Unit::ONE));
                assert_eq!(a.mul(&b).unwrap().pow(2).unwrap().sqrt(), a.mul(&b));
            }
        }
    }

    #[test]
    fn names_of_the_table_win_over_a_prefix() {
        assert_eq!(shown("min"), "min");
        assert_eq!(shown("mol"), "mol");
        assert_eq!(shown("pF"), "pF");
        assert_eq!(parsed("pF").mul(&parsed("MV/V")), Some(parsed("\u{00B5}F")));
        for refused in ["kkg", "kmin", "kh", "k\u{00B0}C", "fF", "THz", "v", "Volt", "m2", "U"] {
            assert_eq!(parse_unit(refused, 256), Err(UnitError::Unknown(refused.to_owned())));
        }
    }

    #[test]
    fn a_result_takes_its_name_from_the_table() {
        assert_eq!(parsed("\u{03A9}").mul(&parsed("A")).unwrap().to_string(), "V");
        assert_eq!(parsed("V").mul(&parsed("A")).unwrap().to_string(), "W");
        assert_eq!(parsed("mV").mul(&parsed("A")).unwrap().to_string(), "mW");
        assert_eq!(shown("N*m"), "J");
        assert_eq!(shown("1/s"), "Hz");
        assert_eq!(shown("1/ms"), "kHz");
    }

    #[test]
    fn otherwise_the_expanded_form() {
        assert_eq!(shown("m/s"), "m/s");
        assert_eq!(shown("m/s^2"), "m/s^2");
        assert_eq!(shown("kg/m"), "kg/m");
        assert_eq!(shown("1/m"), "1/m");
        assert_eq!(shown("1/(m*s^2)"), "1/m/s^2");
        assert_eq!(shown("V*V"), "m^4*kg^2/s^6/A^2");
        assert_eq!(shown("V/V"), "");
    }

    #[test]
    fn a_scale_with_no_name_shows_as_a_number() {
        assert_eq!(shown("km*m"), "1000*m^2");
        assert_eq!(shown("mV/V"), "0.001");
        assert_eq!(shown("GV/mV"), "1e12");
        assert_eq!(shown("nV/GV"), "1e-18");
        assert_eq!(shown("km/h"), "0.2777777777777778*m/s");
        assert_eq!(shown("mg"), "mg");
    }

    #[test]
    fn a_scale_in_powers_of_ten_does_not_round() {
        for (prefix, power) in [("G", 9), ("M", 6), ("k", 3), ("m", -3), ("\u{00B5}", -6), ("n", -9), ("p", -12)] {
            let base = parsed(&format!("{prefix}V/V"));
            let mut exponent = 1;
            while let Some(scale) = base.pow(exponent) {
                let total = power * exponent;
                assert_eq!(scale.scale_value(), format!("1e{total}").parse::<f64>().unwrap(), "{prefix} ^ {exponent}");
                exponent += 1;
            }
            assert!(exponent > 20);
        }
        assert_eq!(parsed("GV/V").pow(3).unwrap().to_string(), "1e27");
        assert_eq!(parsed("GV/V").pow(32).unwrap().to_string(), "1e288");
        assert_eq!(parsed("mV/V").pow(8).unwrap().to_string(), "1e-24");
    }

    #[test]
    fn every_name_reads_back() {
        for named in NAMED_UNITS {
            assert_eq!(parsed(named.name), named.unit, "{}", named.name);
            assert_eq!(shown(named.name), named.name);
            if named.takes_a_prefix {
                for (prefix, _) in PREFIXES {
                    let name = format!("{prefix}{}", named.name);
                    assert_eq!(parsed(&parsed(&name).to_string()), parsed(&name), "{name}");
                }
            }
        }
    }

    #[test]
    fn the_expanded_form_reads_back() {
        for text in ["m/s", "V*V", "1/m", "m^-3", "kg*m^2/s^3/A^2/K", "mol/cd"] {
            assert_eq!(parsed(&shown(text)), parsed(text), "{text}");
        }
    }

    #[test]
    fn temperatures_are_scales_with_an_offset() {
        assert!(parsed("\u{00B0}C").same_dimension(&parsed("K")));
        assert_ne!(parsed("\u{00B0}C"), parsed("K"));
        assert_eq!(shown("\u{00B0}C"), "\u{00B0}C");
        assert_eq!(shown("\u{00B0}F"), "\u{00B0}F");
        assert_eq!(parsed("(\u{00B0}C)"), parsed("\u{00B0}C"));
    }

    #[test]
    fn any_operator_makes_a_temperature_a_difference() {
        assert_eq!(shown("V/\u{00B0}C"), "m^2*kg/s^3/A/K");
        assert_eq!(parsed("\u{00B0}C^1"), parsed("K"));
        assert_eq!(parsed("\u{00B0}C*1"), parsed("K"));
        assert_eq!(parsed("\u{00B0}F*1"), parsed("K").mul(&parsed("\u{00B0}F/K")).unwrap());
    }

    #[test]
    fn sqrt_refuses_an_odd_exponent_or_an_inexact_scale() {
        assert_eq!(parsed("V").sqrt(), None);
        assert_eq!(parsed("mV*mV").sqrt(), Some(parsed("mV")));
        assert_eq!(parsed("km*km").sqrt(), Some(parsed("km")));
        assert_eq!(parsed("kV*V").sqrt(), None);
        assert_eq!(parsed("mV/V").sqrt(), None);
    }

    #[test]
    fn a_chain_of_mul_fails_instead_of_overflowing() {
        let mut square = parsed("m");
        let mut steps = 0;
        while let Some(next) = square.mul(&square) {
            square = next;
            steps += 1;
            assert!(steps < 16);
        }
        assert_eq!(steps, 6, "m^64 is the last power of two under 127");
        assert_eq!(parsed("m^127").mul(&parsed("m")), None);
        assert_eq!(parsed("1/m^127").div(&parsed("m")), None, "-128 fits an i8, not the bound of ±127");
        assert_eq!(parsed("m^-127").pow(1), Some(parsed("1/m^127")));
        assert_eq!(parsed("m^127").pow(-1), Some(parsed("1/m^127")));
    }

    #[test]
    fn a_scale_out_of_bounds_fails() {
        for huge in ["GV/V", "nV/V", "h/s", "\u{00B0}F/K"] {
            assert_eq!(parsed(huge).pow(i32::MAX), None, "{huge}");
            assert_eq!(parsed(huge).pow(i32::MIN), None, "{huge}");
            assert_eq!(parsed(huge).pow(100_000), None, "{huge}");
        }
        assert_eq!(parse_unit("GV^40", 256), Err(UnitError::OutOfBounds));
        assert_eq!(parse_unit("nV^40", 256), Err(UnitError::OutOfBounds));
        let mut scale = parsed("GV/V");
        let mut steps = 0;
        while let Some(next) = scale.mul(&parsed("GV/V")) {
            scale = next;
            steps += 1;
        }
        assert_eq!(steps, 32, "1e297 is the last power of 1e9 under 1e300");
        assert!(scale.scale_value().is_finite());
        assert!(!scale.to_string().contains("inf"));
    }

    #[test]
    fn bad_input_gives_its_code() {
        assert_eq!(parse_unit("m^99999999", 256).map_err(|error| error.code()), Err(Code::Lc1006));
        assert_eq!(parse_unit(&format!("m^{}", "9".repeat(100_000)), 256).map_err(|error| error.code()), Err(Code::Lc1006));
        assert_eq!(parse_unit("m^128", 256).map_err(|error| error.code()), Err(Code::Lc1006));
        assert_eq!(parse_unit("m^-128", 256).map_err(|error| error.code()), Err(Code::Lc1006));
        assert_eq!(parse_unit("V^100", 256).map_err(|error| error.code()), Err(Code::Lc1006));
        assert_eq!(parse_unit("nope", 256).map_err(|error| error.code()), Err(Code::Lc2008));
        for bad in ["", "*", "V*", "V/", "(V", "V)", "()", "V^", "V^x", "V^-", "12", "1V", "V A", "2", "V*2", "2V", "V\u{A0}*\u{A0}A", "\u{0301}"] {
            assert_eq!(parse_unit(bad, 256), Err(UnitError::Syntax), "{bad}");
        }
    }

    #[test]
    fn spaces_between_tokens_are_allowed() {
        assert_eq!(parsed(" V / A "), parsed("\u{03A9}"));
        assert_eq!(parsed("m\t^ - 2"), parsed("1/m^2"));
    }

    #[test]
    fn nesting_is_limited_before_the_stack_is() {
        let nested = |depth: usize| format!("{}V{}", "(".repeat(depth), ")".repeat(depth));
        assert_eq!(parse_unit(&nested(3), 3), Ok(parsed("V")));
        assert_eq!(parse_unit(&nested(4), 3), Err(UnitError::TooDeep));
        assert_eq!(parse_unit(&nested(100_000), u32::MAX), Err(UnitError::TooDeep));
        assert_eq!(UnitError::TooDeep.code(), Code::Lc0007);
    }
}
