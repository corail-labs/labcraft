//! `core` (appendix A.1). A number arrives in the unit of its type, which the conversions of its wire gave it (3.3), so the
//! bodies compute on numbers alone: the unit of a result is in its type. An integer that overflows is `LC8008` (Q-25).

use labcraft_ir::{ConstValue, Static, apply_scale, scale_between};
use labcraft_types::{NumberKind, Ty, Unit};

use super::number::{
    Real, as_f64, complex_div, complex_mul, complex_power, complex_sqrt, kind_of, magnitude, magnitude_f32, maximum, meet, minimum, power, to_kind,
};
use super::{Call, Host, Outcome, input, input_type, output_type, wrong};
use crate::convert::Env;
use crate::fault::Fault;
use crate::value::{ErrorData, Value};

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Add => "core.add",
            Op::Sub => "core.sub",
            Op::Mul => "core.mul",
            Op::Div => "core.div",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
        }
    }

    fn real<R: Real>(self, a: R, b: R) -> R {
        match self {
            Op::Add => a + b,
            Op::Sub => a - b,
            Op::Mul => a * b,
            Op::Div => a / b,
        }
    }

    fn complex<R: Real>(self, a: (R, R), b: (R, R)) -> (R, R) {
        match self {
            Op::Add => (a.0 + b.0, a.1 + b.1),
            Op::Sub => (a.0 - b.0, a.1 - b.1),
            Op::Mul => complex_mul(a, b),
            Op::Div => complex_div(a, b),
        }
    }
}

fn overflow(op: &str, detail: String, kind: NumberKind) -> Fault {
    Fault::out_of_range(format!("{op}: {detail} is out of the range of `{}` (Q-25)", kind.as_str()))
}

macro_rules! checked {
    ($op:expr, $a:expr, $b:expr, $variant:ident, $kind:expr) => {{
        let (a, b) = ($a, $b);
        let result = match $op {
            Op::Add => a.checked_add(b),
            Op::Sub => a.checked_sub(b),
            Op::Mul => a.checked_mul(b),
            Op::Div => None,
        };
        result.map(Value::$variant).ok_or_else(|| overflow($op.name(), format!("{a} {} {b}", $op.symbol()), $kind))
    }};
}

/// `a op b` in the kind of the output: both operands go to it first (3.4), so `core.div` of two integers is a real division.
fn arithmetic(op: Op, call: &mut Call<'_>) -> Result<Outcome, Fault> {
    let kind = call.env.number_kind(output_type(call, 0)?).ok_or_else(|| wrong(op.name()))?;
    let a = to_kind(input(call, 0)?, kind).ok_or_else(|| wrong(op.name()))?;
    let b = to_kind(input(call, 1)?, kind).ok_or_else(|| wrong(op.name()))?;
    let result = match (a, b) {
        (Value::I8(a), Value::I8(b)) => checked!(op, a, b, I8, kind)?,
        (Value::I16(a), Value::I16(b)) => checked!(op, a, b, I16, kind)?,
        (Value::I32(a), Value::I32(b)) => checked!(op, a, b, I32, kind)?,
        (Value::I64(a), Value::I64(b)) => checked!(op, a, b, I64, kind)?,
        (Value::U8(a), Value::U8(b)) => checked!(op, a, b, U8, kind)?,
        (Value::U16(a), Value::U16(b)) => checked!(op, a, b, U16, kind)?,
        (Value::U32(a), Value::U32(b)) => checked!(op, a, b, U32, kind)?,
        (Value::U64(a), Value::U64(b)) => checked!(op, a, b, U64, kind)?,
        (Value::F32(a), Value::F32(b)) => Value::F32(op.real(a, b)),
        (Value::F64(a), Value::F64(b)) => Value::F64(op.real(a, b)),
        (Value::C64(a, b), Value::C64(c, d)) => {
            let (re, im) = op.complex((a, b), (c, d));
            Value::C64(re, im)
        }
        (Value::C128(a, b), Value::C128(c, d)) => {
            let (re, im) = op.complex((a, b), (c, d));
            Value::C128(re, im)
        }
        _ => return Err(wrong(op.name())),
    };
    Ok(Outcome::Done(vec![result]))
}

pub fn add(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    arithmetic(Op::Add, call)
}

pub fn sub(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    arithmetic(Op::Sub, call)
}

pub fn mul(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    arithmetic(Op::Mul, call)
}

/// Integers give an `f64` (3.4), and `x / 0` is `inf` or `nan`, as IEEE 754 divides.
pub fn div(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    arithmetic(Op::Div, call)
}

/// `|x|`; that of a complex number is its magnitude, with no imaginary part.
pub fn abs(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let x = input(call, 0)?;
    let kind = kind_of(x).ok_or_else(|| wrong("core.abs"))?;
    let range = |text: String| overflow("core.abs", text, kind);
    let result = match x {
        Value::I8(x) => Value::I8(x.checked_abs().ok_or_else(|| range(format!("|{x}|")))?),
        Value::I16(x) => Value::I16(x.checked_abs().ok_or_else(|| range(format!("|{x}|")))?),
        Value::I32(x) => Value::I32(x.checked_abs().ok_or_else(|| range(format!("|{x}|")))?),
        Value::I64(x) => Value::I64(x.checked_abs().ok_or_else(|| range(format!("|{x}|")))?),
        Value::U8(_) | Value::U16(_) | Value::U32(_) | Value::U64(_) => x.clone(),
        Value::F32(x) => Value::F32(x.abs()),
        Value::F64(x) => Value::F64(x.abs()),
        Value::C64(re, im) => Value::C64(magnitude_f32(*re, *im), 0.0),
        Value::C128(re, im) => Value::C128(magnitude(*re, *im), 0.0),
        _ => return Err(wrong("core.abs")),
    };
    Ok(Outcome::Done(vec![result]))
}

pub fn neg(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let x = input(call, 0)?;
    let kind = kind_of(x).ok_or_else(|| wrong("core.neg"))?;
    let range = |text: String| overflow("core.neg", text, kind);
    let result = match x {
        Value::I8(x) => Value::I8(x.checked_neg().ok_or_else(|| range(format!("-{x}")))?),
        Value::I16(x) => Value::I16(x.checked_neg().ok_or_else(|| range(format!("-{x}")))?),
        Value::I32(x) => Value::I32(x.checked_neg().ok_or_else(|| range(format!("-({x})")))?),
        Value::I64(x) => Value::I64(x.checked_neg().ok_or_else(|| range(format!("-({x})")))?),
        Value::U8(x) => Value::U8(x.checked_neg().ok_or_else(|| range(format!("-{x}")))?),
        Value::U16(x) => Value::U16(x.checked_neg().ok_or_else(|| range(format!("-{x}")))?),
        Value::U32(x) => Value::U32(x.checked_neg().ok_or_else(|| range(format!("-{x}")))?),
        Value::U64(x) => Value::U64(x.checked_neg().ok_or_else(|| range(format!("-{x}")))?),
        Value::F32(x) => Value::F32(-*x),
        Value::F64(x) => Value::F64(-*x),
        Value::C64(re, im) => Value::C64(-*re, -*im),
        Value::C128(re, im) => Value::C128(-*re, -*im),
        _ => return Err(wrong("core.neg")),
    };
    Ok(Outcome::Done(vec![result]))
}

/// IEEE 754 rounds the square root correctly; integers give an `f64`, and a negative real number `nan`.
pub fn sqrt(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let kind = call.env.number_kind(output_type(call, 0)?).ok_or_else(|| wrong("core.sqrt"))?;
    let x = to_kind(input(call, 0)?, kind).ok_or_else(|| wrong("core.sqrt"))?;
    let result = match x {
        Value::F32(x) => Value::F32(x.sqrt()),
        Value::F64(x) => Value::F64(x.sqrt()),
        Value::C64(re, im) => {
            let (re, im) = complex_sqrt((re, im));
            Value::C64(re, im)
        }
        Value::C128(re, im) => {
            let (re, im) = complex_sqrt((re, im));
            Value::C128(re, im)
        }
        _ => return Err(wrong("core.sqrt")),
    };
    Ok(Outcome::Done(vec![result]))
}

/// `x^exponent` by squaring (see [`power`]); integers give an `f64`.
pub fn pow(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let kind = call.env.number_kind(output_type(call, 0)?).ok_or_else(|| wrong("core.pow"))?;
    let x = to_kind(input(call, 0)?, kind).ok_or_else(|| wrong("core.pow"))?;
    let Value::I32(n) = *input(call, 1)? else { return Err(wrong("core.pow")) };
    let result = match x {
        Value::F32(x) => Value::F32(power(x, n)),
        Value::F64(x) => Value::F64(power(x, n)),
        Value::C64(re, im) => {
            let (re, im) = complex_power((re, im), n);
            Value::C64(re, im)
        }
        Value::C128(re, im) => {
            let (re, im) = complex_power((re, im), n);
            Value::C128(re, im)
        }
        _ => return Err(wrong("core.pow")),
    };
    Ok(Outcome::Done(vec![result]))
}

fn extreme(call: &mut Call<'_>, name: &str, smaller: bool) -> Result<Outcome, Fault> {
    let (a, b) = meet(input(call, 0)?, input(call, 1)?).ok_or_else(|| wrong(name))?;
    let result = match (a, b) {
        (Value::F32(a), Value::F32(b)) => Value::F32(if smaller { minimum(a, b) } else { maximum(a, b) }),
        (Value::F64(a), Value::F64(b)) => Value::F64(if smaller { minimum(a, b) } else { maximum(a, b) }),
        (a, b) => {
            let (x, y) = (super::number::as_i128(&a).ok_or_else(|| wrong(name))?, super::number::as_i128(&b).ok_or_else(|| wrong(name))?);
            if (x <= y) == smaller { a } else { b }
        }
    };
    Ok(Outcome::Done(vec![result]))
}

/// IEEE 754-2019 `minimum`: a `nan` among the inputs gives `nan`, and `-0` is below `0`.
pub fn min(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    extreme(call, "core.min", true)
}

pub fn max(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    extreme(call, "core.max", false)
}

#[derive(Clone, Copy)]
enum Order {
    Lt,
    Le,
    Gt,
    Ge,
}

/// IEEE 754 comparisons: `nan` compares false with everything.
fn compare(call: &mut Call<'_>, order: Order) -> Result<Outcome, Fault> {
    let (a, b) = meet(input(call, 0)?, input(call, 1)?).ok_or_else(|| wrong("a comparison"))?;
    let ordering = match (&a, &b) {
        (Value::F32(a), Value::F32(b)) => a.partial_cmp(b),
        (Value::F64(a), Value::F64(b)) => a.partial_cmp(b),
        (a, b) => {
            let x = super::number::as_i128(a).ok_or_else(|| wrong("a comparison"))?;
            let y = super::number::as_i128(b).ok_or_else(|| wrong("a comparison"))?;
            Some(x.cmp(&y))
        }
    };
    let result = ordering.is_some_and(|ordering| match order {
        Order::Lt => ordering.is_lt(),
        Order::Le => ordering.is_le(),
        Order::Gt => ordering.is_gt(),
        Order::Ge => ordering.is_ge(),
    });
    Ok(Outcome::Done(vec![Value::Bool(result)]))
}

pub fn lt(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    compare(call, Order::Lt)
}

pub fn le(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    compare(call, Order::Le)
}

pub fn gt(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    compare(call, Order::Gt)
}

pub fn ge(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    compare(call, Order::Ge)
}

pub fn eq(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let same = equal(call.env, input(call, 0)?, input(call, 1)?)?;
    Ok(Outcome::Done(vec![Value::Bool(same)]))
}

pub fn ne(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let same = equal(call.env, input(call, 0)?, input(call, 1)?)?;
    Ok(Outcome::Done(vec![Value::Bool(!same)]))
}

/// What a function reference designates: a function of the program, or a primitive.
fn function_of(env: Env<'_>, value: &Value) -> Option<(u32, String)> {
    let Value::Function(id) = value else { return None };
    match &env.program.constants.get(id.0 as usize)?.value {
        ConstValue::Function(function) => Some((function.0, String::new())),
        ConstValue::Primitive { library, name, .. } => Some((u32::MAX, format!("{}.{}", library.as_str(), env.names.resolve(*name)))),
        _ => None,
    }
}

fn same_error(a: &ErrorData, b: &ErrorData) -> bool {
    a.category == b.category
        && a.code == b.code
        && a.message == b.message
        && a.others.len() == b.others.len()
        && a.others.iter().zip(b.others.iter()).all(|(a, b)| a.category == b.category && a.code == b.code && a.message == b.message)
}

/// Two values of one type are equal when they hold the same: numbers as IEEE 754 compares them (`nan` equals nothing, `-0`
/// equals `0`), texts and bytes by their content, arrays by their shape and their items, structs and variants field by
/// field, errors by category, code, message and the errors merged into them (not where they were made), handles by what
/// they designate, references by the function. An explicit stack: values nest as deep as their types.
pub fn equal(env: Env<'_>, a: &Value, b: &Value) -> Result<bool, Fault> {
    let mut pending = vec![(a, b)];
    while let Some((a, b)) = pending.pop() {
        let same = match (a, b) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Duration(a), Value::Duration(b)) | (Value::Timestamp(a), Value::Timestamp(b)) => a == b,
            (Value::Error(None), Value::Error(None)) => true,
            (Value::Error(Some(a)), Value::Error(Some(b))) => same_error(a, b),
            (Value::Error(_), Value::Error(_)) => false,
            (Value::Handle(a), Value::Handle(b)) => a == b,
            (Value::Function(_), Value::Function(_)) => {
                let (x, y) = (function_of(env, a), function_of(env, b));
                x.is_some() && x == y
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    false
                } else {
                    pending.extend(x.items().iter().zip(y.items()));
                    true
                }
            }
            // With no item, two grids are equal when they write the same: their lengths down to the first that is 0, below
            // which nothing shows (`[2, 0, 0]` and `[2, 0, 5]` both write `[[], []]`).
            (Value::Grid(x), Value::Grid(y)) if x.items().is_empty() || y.items().is_empty() => {
                x.items().is_empty() && y.items().is_empty() && shown_shape(x.shape()) == shown_shape(y.shape())
            }
            (Value::Grid(x), Value::Grid(y)) => {
                if x.shape() != y.shape() {
                    false
                } else {
                    pending.extend(x.items().iter().zip(y.items()));
                    true
                }
            }
            (Value::Struct(x), Value::Struct(y)) => {
                if x.len() != y.len() {
                    false
                } else {
                    pending.extend(x.iter().zip(y.iter()));
                    true
                }
            }
            (Value::Variant(x), Value::Variant(y)) => {
                if x.index != y.index || x.values.len() != y.values.len() {
                    false
                } else {
                    pending.extend(x.values.iter().zip(y.values.iter()));
                    true
                }
            }
            (Value::C64(a, b), Value::C64(c, d)) => a == c && b == d,
            (Value::C128(a, b), Value::C128(c, d)) => a == c && b == d,
            (a, b) => match meet(a, b) {
                Some((Value::F32(x), Value::F32(y))) => x == y,
                Some((Value::F64(x), Value::F64(y))) => x == y,
                Some((Value::C64(a, b), Value::C64(c, d))) => a == c && b == d,
                Some((Value::C128(a, b), Value::C128(c, d))) => a == c && b == d,
                Some((x, y)) => super::number::as_i128(&x).zip(super::number::as_i128(&y)).is_some_and(|(x, y)| x == y),
                None => return Err(Fault::bug("core.eq of two values of different types")),
            },
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The lengths of a grid that its text shows: down to the first that is 0.
fn shown_shape(shape: &[usize]) -> &[usize] {
    match shape.iter().position(|length| *length == 0) {
        Some(zero) => shape.get(..=zero).unwrap_or(shape),
        None => shape,
    }
}

fn boolean(call: &Call<'_>, index: usize, name: &str) -> Result<bool, Fault> {
    match input(call, index)? {
        Value::Bool(value) => Ok(*value),
        _ => Err(wrong(name)),
    }
}

pub fn and(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let result = boolean(call, 0, "core.and")? && boolean(call, 1, "core.and")?;
    Ok(Outcome::Done(vec![Value::Bool(result)]))
}

pub fn or(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let result = boolean(call, 0, "core.or")? || boolean(call, 1, "core.or")?;
    Ok(Outcome::Done(vec![Value::Bool(result)]))
}

pub fn not(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let result = !boolean(call, 0, "core.not")?;
    Ok(Outcome::Done(vec![Value::Bool(result)]))
}

pub fn select(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let condition = boolean(call, 0, "core.select")?;
    let chosen = if condition { 1 } else { 2 };
    let value = call.inputs.get_mut(chosen).map(|slot| std::mem::replace(slot, Value::NONE)).ok_or_else(|| wrong("core.select"))?;
    Ok(Outcome::Done(vec![value]))
}

/// The number, now in the unit of its static: the same number, which its type now says is in that unit.
pub fn with_unit(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    match input(call, 0)? {
        Value::F64(x) => Ok(Outcome::Done(vec![Value::F64(*x)])),
        _ => Err(wrong("core.with_unit")),
    }
}

/// The number in the unit of its static, as an `f64`: converted as a wire converts it (3.3).
pub fn unitless(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let Some(Static::Unit(to)) = call.statics.first() else { return Err(wrong("core.unitless")) };
    let from = match call.env.ty(input_type(call, 0)?) {
        Ty::Number { unit, .. } => *unit,
        Ty::Duration => Unit::SECOND,
        _ => return Err(wrong("core.unitless")),
    };
    let x = as_f64(input(call, 0)?).ok_or_else(|| wrong("core.unitless"))?;
    Ok(Outcome::Done(vec![Value::F64(apply_scale(x, &scale_between(&from, to), false))]))
}

fn real_input(call: &Call<'_>, name: &str) -> Result<f64, Fault> {
    match input(call, 0)? {
        Value::F64(x) => Ok(*x),
        _ => Err(wrong(name)),
    }
}

/// An integer of the `i32`, or `LC8008` for `nan` and what is out of its range (Q-25).
fn narrow(x: f64, whole: f64, name: &str) -> Result<Outcome, Fault> {
    // -2^31 and 2^31 are `f64`s exactly.
    if whole.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&whole) {
        return Err(Fault::out_of_range(format!("{name}: {x} is not an i32 (Q-25)")));
    }
    Ok(Outcome::Done(vec![Value::I32(whole as i32)]))
}

/// Toward zero: `2.7` gives `2`, `-2.7` gives `-2`.
pub fn to_i32(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let x = real_input(call, "core.to_i32")?;
    narrow(x, x.trunc(), "core.to_i32")
}

/// To the nearest, a half to even (decision of the owner, 2026-11-09): `2.5` gives `2`, `3.5` gives `4`, as a `duration` is
/// rounded (3.1).
pub fn round_to_i32(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let x = real_input(call, "core.round_to_i32")?;
    narrow(x, x.round_ties_even(), "core.round_to_i32")
}

/// To the nearest `f32`, a half to even; beyond its range, `inf`.
pub fn to_f32(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let x = real_input(call, "core.to_f32")?;
    Ok(Outcome::Done(vec![Value::F32(x as f32)]))
}

/// The interpreter runs the function: its inputs follow the reference, in the places of its own.
pub fn call(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let mut inputs = std::mem::take(&mut call.inputs);
    if inputs.is_empty() {
        return Err(wrong("core.call"));
    }
    let function = inputs.remove(0);
    if !matches!(function, Value::Function(_)) {
        return Err(wrong("core.call"));
    }
    Ok(Outcome::Call { function, inputs })
}
