//! Conversions on a wire and the values of literals (spec 3.1, 3.3, 3.4), shared by lowering, which evaluates each constant
//! once in the type of the use it reaches, and by the runtime, which converts on the wire: the same steps give the same bits.

use labcraft_graph::{Numeral, NumeralKind};
use labcraft_types::{Interner, NumberKind, Ty, TyId, TypeTable, Unit};

use crate::{ConstValue, Conversion, Scale};

/// A value that a conversion cannot make: a duration beyond `i64` nanoseconds, or not finite (Q-25).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversionError;

/// The steps from one unit to another (spec 3.3). The same unit takes none.
pub fn scale_between(from: &Unit, to: &Unit) -> Scale {
    if from == to {
        return Scale::default();
    }
    let (numerator, denominator) = from.scale_ratio(to);
    Scale {
        add: from.offset(),
        multiply: (numerator != 1.0).then_some(numerator),
        divide: (denominator != 1.0).then_some(denominator),
        subtract: to.offset(),
    }
}

/// The steps of a scale, each one operation in `f64`, whose integers are the nearest `f64` (spec 3.3). For a value of
/// `f32` (`single`), held in an `f64` exactly, the steps run in `f64` too, and the result is rounded to `f32` once.
pub fn apply_scale(value: f64, scale: &Scale, single: bool) -> f64 {
    let x = steps(value, scale);
    if single { f64::from(x as f32) } else { x }
}

fn steps(value: f64, scale: &Scale) -> f64 {
    let mut x = value;
    if let Some(add) = scale.add {
        x += add;
    }
    if let Some(multiply) = scale.multiply {
        x *= multiply;
    }
    if let Some(divide) = scale.divide {
        x /= divide;
    }
    if let Some(subtract) = scale.subtract {
        x -= subtract;
    }
    x
}

/// What a wire does from `from` to `to`, as `check` recorded it. `None` when it does nothing a conversion does.
pub(crate) fn conversion(types: &TypeTable, from: TyId, to: TyId) -> Option<Conversion> {
    match (types.get(from), types.get(to)) {
        (Ty::Number { unit: from_unit, .. }, Ty::Number { kind, unit }) => {
            Some(Conversion::Number { to: *kind, scale: scale_between(from_unit, unit) })
        }
        (Ty::Duration, Ty::Number { kind, unit }) => Some(Conversion::FromDuration { to: *kind, scale: scale_between(&Unit::SECOND, unit) }),
        (Ty::Number { unit, .. }, Ty::Duration) => Some(Conversion::ToDuration { scale: scale_between(unit, &Unit::SECOND) }),
        _ => None,
    }
}

fn is_single(kind: NumberKind) -> bool {
    matches!(kind, NumberKind::F32 | NumberKind::C64)
}

/// A number as an `f64`, for a conversion that goes through one: an integer of a kind that widens to it is exact.
fn as_float(value: &ConstValue) -> Option<f64> {
    match value {
        ConstValue::Int(value) => Some(*value as f64),
        ConstValue::Float(value) => Some(*value),
        _ => None,
    }
}

/// From an `f64[s]` to whole nanoseconds (spec 3.1).
pub(crate) fn nanoseconds(seconds: f64) -> Result<i64, ConversionError> {
    let ns = (seconds * 1e9).round_ties_even();
    // `i64::MAX` is not an `f64`: 2^63 is the first value past it.
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&ns) { Ok(ns as i64) } else { Err(ConversionError) }
}

/// A constant once a conversion is applied, as the runtime does on the wire.
pub(crate) fn convert(value: ConstValue, conversion: &Conversion) -> Result<ConstValue, ConversionError> {
    match conversion {
        Conversion::Number { to, scale } => {
            if to.is_complex() {
                let (re, im) = match value {
                    ConstValue::Complex { re, im } => (re, im),
                    other => (as_float(&other).ok_or(ConversionError)?, 0.0),
                };
                let single = is_single(*to);
                return Ok(ConstValue::Complex { re: apply_scale(re, scale, single), im: apply_scale(im, scale, single) });
            }
            if to.is_integer() {
                // Integers only widen: a change of scale goes through a floating-point number (3.4), which `check` asked for.
                return match value {
                    ConstValue::Int(value) if *scale == Scale::default() => Ok(ConstValue::Int(value)),
                    _ => Err(ConversionError),
                };
            }
            let x = as_float(&value).ok_or(ConversionError)?;
            Ok(ConstValue::Float(apply_scale(x, scale, is_single(*to))))
        }
        Conversion::ToDuration { scale } => {
            let x = as_float(&value).ok_or(ConversionError)?;
            Ok(ConstValue::Duration(nanoseconds(apply_scale(x, scale, false))?))
        }
        Conversion::FromDuration { to, scale } => {
            let ConstValue::Duration(ns) = value else { return Err(ConversionError) };
            let x = apply_scale(ns as f64 / 1e9, scale, false);
            match to {
                kind if kind.is_complex() => {
                    let x = if is_single(*kind) { f64::from(x as f32) } else { x };
                    Ok(ConstValue::Complex { re: x, im: 0.0 })
                }
                kind if kind.is_integer() => Err(ConversionError),
                kind => Ok(ConstValue::Float(if is_single(*kind) { f64::from(x as f32) } else { x })),
            }
        }
    }
}

fn digits_of(text: &str) -> String {
    text.chars().filter(|c| *c != '_').collect()
}

/// A number literal written in the unit `written`, in the type it reaches: read once in the kind of that type, then
/// converted as a wire would convert it (spec 3.3, 3.4). `check` accepted it, so what does not fit is a bug.
pub(crate) fn literal_number(numeral: Numeral, written: &Unit, target: &Ty, names: &Interner) -> Result<ConstValue, ConversionError> {
    match target {
        // In its own unit, it is read once in the kind it goes to. Otherwise it is read as an `f64`, then converted, and rounded
        // to its kind once: `1e39 mV` is an `f32[V]` of 1e36, which an `f32[mV]` could not hold (3.4).
        Ty::Number { kind, unit } if written == unit => number_in(numeral, *kind, names),
        Ty::Number { kind, unit } => {
            let wide = match kind {
                NumberKind::F32 => NumberKind::F64,
                NumberKind::C64 => NumberKind::C128,
                other => *other,
            };
            convert(number_in(numeral, wide, names)?, &Conversion::Number { to: *kind, scale: scale_between(written, unit) })
        }
        Ty::Duration => {
            convert(number_in(numeral, NumberKind::F64, names)?, &Conversion::ToDuration { scale: scale_between(written, &Unit::SECOND) })
        }
        _ => Err(ConversionError),
    }
}

/// A number literal in a kind, before any change of unit.
fn number_in(numeral: Numeral, kind: NumberKind, names: &Interner) -> Result<ConstValue, ConversionError> {
    let sign = |x: f64| if numeral.negative { -x } else { x };
    let float = |x: f64| if kind.is_complex() { ConstValue::Complex { re: x, im: 0.0 } } else { ConstValue::Float(x) };
    match numeral.kind {
        NumeralKind::Int(digits) => {
            let text = digits_of(names.resolve(digits));
            let magnitude: i128 = text.parse().map_err(|_| ConversionError)?;
            let value = if numeral.negative { -magnitude } else { magnitude };
            if kind.is_integer() {
                Ok(ConstValue::Int(value))
            } else if is_single(kind) {
                Ok(float(f64::from(value as f32)))
            } else {
                Ok(float(value as f64))
            }
        }
        NumeralKind::Float(digits) => {
            if kind.is_integer() {
                return Err(ConversionError);
            }
            let text = digits_of(names.resolve(digits));
            // Read once in its own kind: an `f32` read through an `f64` would round twice.
            let x = if is_single(kind) {
                f64::from(text.parse::<f32>().map_err(|_| ConversionError)?)
            } else {
                text.parse::<f64>().map_err(|_| ConversionError)?
            };
            Ok(float(sign(x)))
        }
        NumeralKind::Inf if !kind.is_integer() => Ok(float(sign(f64::INFINITY))),
        NumeralKind::Nan if !kind.is_integer() => Ok(float(sign(f64::NAN))),
        NumeralKind::Inf | NumeralKind::Nan => Err(ConversionError),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(text: &str) -> Unit {
        labcraft_types::parse_unit(text, 8).unwrap()
    }

    /// Spec 3.3: to a unit *n* times larger, one division by *n*; to one *n* times smaller, one multiplication; otherwise the
    /// numerator, then the denominator. Offsets are added and taken away, each in one operation.
    #[test]
    fn a_scale_takes_one_operation_per_step() {
        assert_eq!(scale_between(&unit("mV"), &unit("V")), Scale { divide: Some(1000.0), ..Scale::default() });
        assert_eq!(scale_between(&unit("kV"), &unit("V")), Scale { multiply: Some(1000.0), ..Scale::default() });
        assert_eq!(scale_between(&unit("min"), &unit("h")), Scale { divide: Some(60.0), ..Scale::default() });
        assert_eq!(scale_between(&unit("V"), &unit("V")), Scale::default());
        // °F to °C: + 459.67, × 5, ÷ 9, − 273.15.
        assert_eq!(
            scale_between(&unit("°F"), &unit("°C")),
            Scale { add: Some(459.67), multiply: Some(5.0), divide: Some(9.0), subtract: Some(273.15) }
        );
        // min (2^2·3·5) to ms (2^-3·5^-3): 60 000, a multiplication.
        assert_eq!(scale_between(&unit("min"), &unit("ms")), Scale { multiply: Some(60_000.0), ..Scale::default() });
    }

    /// 9 mV to V is 9 ÷ 1000, the example of spec 3.3; 0.009 is the nearest `f64` of that quotient.
    #[test]
    fn a_wire_converts_as_the_spec_says() {
        let scale = scale_between(&unit("mV"), &unit("V"));
        assert_eq!(apply_scale(9.0, &scale, false), 0.009);
        // 20 °C is 293.15 K: 20 + 273.15, worked out by hand; `f64` gives 293.15 exactly as its nearest value.
        assert_eq!(apply_scale(20.0, &scale_between(&unit("°C"), &unit("K")), false), 293.15);
        // In `f32`, 1 min is 1 ÷ 60 h in `f64`, then rounded to `f32` once: 0.01666666753590107, by Python's `struct`.
        let hours = scale_between(&unit("min"), &unit("h"));
        assert_eq!(apply_scale(1.0, &hours, true), 0.01666666753590107);
        // 57 pV in V: 57 ÷ 10^12 in `f64`, then to `f32`: 5.6999998471196633e-11, the nearest `f32` of 57e-12 (Python).
        assert_eq!(apply_scale(57.0, &scale_between(&unit("pV"), &unit("V")), true), 5.6999998471196633e-11);
        // 1 min is 60 s, 1 ks is 1000 s: the ratio 60/1000 is 3/50, by hand. 5 min is 5 × 3 ÷ 50 = 0.3 ks.
        let kiloseconds = scale_between(&unit("min"), &unit("ks"));
        assert_eq!(kiloseconds, Scale { multiply: Some(3.0), divide: Some(50.0), ..Scale::default() });
        assert_eq!(apply_scale(5.0, &kiloseconds, false), 0.3);
    }

    /// A literal in another unit is read as an `f64`, so that `1e39 mV` fits an `f32[V]` (3.4): 1e36 as an `f32` is
    /// 9.999999616903162e35 (Python's `struct`).
    #[test]
    fn a_literal_is_scaled_before_it_is_rounded() {
        let mut names = Interner::new();
        let numeral = Numeral { negative: false, kind: NumeralKind::Float(names.intern("1e39")) };
        let target = Ty::Number { kind: NumberKind::F32, unit: unit("V") };
        let value = literal_number(numeral, &unit("mV"), &target, &names).unwrap();
        assert!(matches!(value, ConstValue::Float(x) if x == 9.999999616903162e35), "{value:?}");
    }

    /// Spec 3.1: times 10^9 as an `f64`, then to the nearest integer, ties to even.
    #[test]
    fn a_duration_is_whole_nanoseconds() {
        assert_eq!(nanoseconds(2.0), Ok(2_000_000_000));
        assert_eq!(nanoseconds(0.5e-9), Ok(0));
        assert_eq!(nanoseconds(1.5e-9), Ok(2));
        assert_eq!(nanoseconds(2.5e-9), Ok(2));
        assert_eq!(nanoseconds(f64::NAN), Err(ConversionError));
        assert_eq!(nanoseconds(1e10), Err(ConversionError));
    }
}
