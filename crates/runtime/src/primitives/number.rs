//! Numbers of every kind, brought to one kind before an operation (spec 3.4: two kinds that meet give the smallest kind
//! that both widen to).

use labcraft_types::NumberKind;

use crate::convert::integer;
use crate::value::Value;

pub fn kind_of(value: &Value) -> Option<NumberKind> {
    Some(match value {
        Value::I8(_) => NumberKind::I8,
        Value::I16(_) => NumberKind::I16,
        Value::I32(_) => NumberKind::I32,
        Value::I64(_) => NumberKind::I64,
        Value::U8(_) => NumberKind::U8,
        Value::U16(_) => NumberKind::U16,
        Value::U32(_) => NumberKind::U32,
        Value::U64(_) => NumberKind::U64,
        Value::F32(_) => NumberKind::F32,
        Value::F64(_) => NumberKind::F64,
        Value::C64(..) => NumberKind::C64,
        Value::C128(..) => NumberKind::C128,
        _ => return None,
    })
}

pub fn as_i128(value: &Value) -> Option<i128> {
    Some(match value {
        Value::I8(x) => i128::from(*x),
        Value::I16(x) => i128::from(*x),
        Value::I32(x) => i128::from(*x),
        Value::I64(x) => i128::from(*x),
        Value::U8(x) => i128::from(*x),
        Value::U16(x) => i128::from(*x),
        Value::U32(x) => i128::from(*x),
        Value::U64(x) => i128::from(*x),
        _ => return None,
    })
}

/// A real number as an `f64`: exact for an integer of up to 32 bits and for an `f32`; the nearest for a 64-bit integer.
pub fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::F32(x) => Some(f64::from(*x)),
        Value::F64(x) => Some(*x),
        other => as_i128(other).map(|x| x as f64),
    }
}

/// The value in another kind, if nothing is lost: what a widening does (3.4).
pub fn to_kind(value: &Value, kind: NumberKind) -> Option<Value> {
    if kind_of(value)? == kind {
        return Some(value.clone());
    }
    match kind {
        _ if kind.is_integer() => integer(kind, as_i128(value)?),
        NumberKind::F32 => match value {
            Value::F32(x) => Some(Value::F32(*x)),
            other => Some(Value::F32(as_f64(other)? as f32)),
        },
        NumberKind::F64 => as_f64(value).map(Value::F64),
        NumberKind::C64 => match value {
            Value::C64(re, im) => Some(Value::C64(*re, *im)),
            other => Some(Value::C64(as_f64(other)? as f32, 0.0)),
        },
        NumberKind::C128 => match value {
            Value::C64(re, im) => Some(Value::C128(f64::from(*re), f64::from(*im))),
            Value::C128(re, im) => Some(Value::C128(*re, *im)),
            other => Some(Value::C128(as_f64(other)?, 0.0)),
        },
        _ => None,
    }
}

/// Both in the kind where they meet.
pub fn meet(a: &Value, b: &Value) -> Option<(Value, Value)> {
    let kind = kind_of(a)?.meeting(kind_of(b)?)?;
    Some((to_kind(a, kind)?, to_kind(b, kind)?))
}

/// `|z|` by a fixed sequence of operations that IEEE 754 rounds, so that every platform gives the same bits: the larger
/// part times the square root of one plus the square of their ratio, which neither overflows nor underflows early.
pub fn magnitude(re: f64, im: f64) -> f64 {
    let (a, b) = (re.abs(), im.abs());
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let (large, small) = if a >= b { (a, b) } else { (b, a) };
    if large == 0.0 {
        return 0.0;
    }
    let ratio = small / large;
    large * (1.0 + ratio * ratio).sqrt()
}

pub fn magnitude_f32(re: f32, im: f32) -> f32 {
    let (a, b) = (re.abs(), im.abs());
    if a.is_infinite() || b.is_infinite() {
        return f32::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f32::NAN;
    }
    let (large, small) = if a >= b { (a, b) } else { (b, a) };
    if large == 0.0 {
        return 0.0;
    }
    let ratio = small / large;
    large * (1.0 + ratio * ratio).sqrt()
}

/// Operations on a kind of real number, so that `f32` and `f64` share one sequence of steps.
pub trait Real:
    Copy
    + PartialOrd
    + std::ops::Add<Output = Self>
    + std::ops::Sub<Output = Self>
    + std::ops::Mul<Output = Self>
    + std::ops::Div<Output = Self>
    + std::ops::Neg<Output = Self>
{
    const ZERO: Self;
    const ONE: Self;
    fn abs(self) -> Self;
    fn sqrt(self) -> Self;
    fn is_nan(self) -> bool;
    fn is_sign_negative(self) -> bool;
    fn copysign(self, sign: Self) -> Self;
    fn magnitude(re: Self, im: Self) -> Self;
}

impl Real for f64 {
    const ZERO: f64 = 0.0;
    const ONE: f64 = 1.0;
    fn abs(self) -> f64 {
        f64::abs(self)
    }
    fn sqrt(self) -> f64 {
        f64::sqrt(self)
    }
    fn is_nan(self) -> bool {
        f64::is_nan(self)
    }
    fn is_sign_negative(self) -> bool {
        f64::is_sign_negative(self)
    }
    fn copysign(self, sign: f64) -> f64 {
        f64::copysign(self, sign)
    }
    fn magnitude(re: f64, im: f64) -> f64 {
        magnitude(re, im)
    }
}

impl Real for f32 {
    const ZERO: f32 = 0.0;
    const ONE: f32 = 1.0;
    fn abs(self) -> f32 {
        f32::abs(self)
    }
    fn sqrt(self) -> f32 {
        f32::sqrt(self)
    }
    fn is_nan(self) -> bool {
        f32::is_nan(self)
    }
    fn is_sign_negative(self) -> bool {
        f32::is_sign_negative(self)
    }
    fn copysign(self, sign: f32) -> f32 {
        f32::copysign(self, sign)
    }
    fn magnitude(re: f32, im: f32) -> f32 {
        magnitude_f32(re, im)
    }
}

/// `(a + bi)(c + di) = (ac − bd) + (ad + bc)i`, with no fused step (Rust never fuses).
pub fn complex_mul<R: Real>((a, b): (R, R), (c, d): (R, R)) -> (R, R) {
    (a * c - b * d, a * d + b * c)
}

/// Smith's division, which scales by the larger part of the divisor: a fixed sequence, the same on every platform.
pub fn complex_div<R: Real>((a, b): (R, R), (c, d): (R, R)) -> (R, R) {
    if c.abs() >= d.abs() {
        let r = d / c;
        let den = c + d * r;
        ((a + b * r) / den, (b - a * r) / den)
    } else {
        let r = c / d;
        let den = c * r + d;
        ((a * r + b) / den, (b * r - a) / den)
    }
}

/// The principal square root: `t = √((|z| + |re|) / 2)`, then the other part from it.
pub fn complex_sqrt<R: Real>((re, im): (R, R)) -> (R, R) {
    if re == R::ZERO && im == R::ZERO {
        return (R::ZERO, im);
    }
    let two = R::ONE + R::ONE;
    let t = ((R::magnitude(re, im) + re.abs()) / two).sqrt();
    if re.is_sign_negative() { (im.abs() / (two * t), t.copysign(im)) } else { (t, im / (two * t)) }
}

/// `x^n` by squaring, from the lowest bit of `|n|`, then one division for a negative `n`: a fixed sequence of
/// multiplications, rather than the `pow` of a platform's library (`docs/architecture.md`, 10.3).
pub fn power<R: Real>(x: R, n: i32) -> R {
    let mut base = x;
    let mut exponent = n.unsigned_abs();
    let mut result = R::ONE;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result * base;
        }
        exponent >>= 1;
        if exponent > 0 {
            base = base * base;
        }
    }
    if n < 0 { R::ONE / result } else { result }
}

pub fn complex_power<R: Real>(z: (R, R), n: i32) -> (R, R) {
    let mut base = z;
    let mut exponent = n.unsigned_abs();
    let mut result = (R::ONE, R::ZERO);
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = complex_mul(result, base);
        }
        exponent >>= 1;
        if exponent > 0 {
            base = complex_mul(base, base);
        }
    }
    if n < 0 { complex_div((R::ONE, R::ZERO), result) } else { result }
}

/// IEEE 754-2019 `minimum` and `maximum`: a `nan` gives `nan`, and `-0` is below `0`, so no failed measurement disappears
/// in a `min` and the result does not depend on the order of the inputs.
pub fn minimum<R: Real>(a: R, b: R) -> R {
    if a.is_nan() {
        return a;
    }
    if b.is_nan() {
        return b;
    }
    if a == b {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

pub fn maximum<R: Real>(a: R, b: R) -> R {
    if a.is_nan() {
        return a;
    }
    if b.is_nan() {
        return b;
    }
    if a == b {
        return if a.is_sign_negative() { b } else { a };
    }
    if a > b { a } else { b }
}
