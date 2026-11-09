//! What a wire does to a value on the way (spec 3.1, 3.3, 3.4), and the value of each constant of the program. The steps are
//! those of `ir`, which evaluates constants with them at lowering: the same steps give the same bits (D-10).

use std::sync::Arc;

use labcraft_ir::{ConstId, ConstValue, Conversion, Program, Scale, Use, apply_scale, nanoseconds};
use labcraft_types::{Interner, NumberKind, Ty, TyId};

use crate::fault::Fault;
use crate::value::{Grid, Handle, Value, VariantValue};

/// What a run reads its types and names from.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    pub program: &'a Program,
    pub names: &'a Interner,
}

impl<'a> Env<'a> {
    /// The type, through what a type declared in a `.lct` file stands for; an opaque type stays itself.
    pub fn ty(&self, id: TyId) -> &'a Ty {
        let types = &self.program.types;
        match types.get(id) {
            Ty::Named { .. } => match self.program.definitions.get(&id) {
                Some(Some(definition)) => types.get(*definition),
                _ => types.get(id),
            },
            other => other,
        }
    }

    /// The type itself, through what a type declared in a `.lct` file stands for.
    pub fn defined(&self, id: TyId) -> TyId {
        match self.program.definitions.get(&id) {
            Some(Some(definition)) => *definition,
            _ => id,
        }
    }

    pub fn number_kind(&self, id: TyId) -> Option<NumberKind> {
        match self.ty(id) {
            Ty::Number { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

/// A value as one input reads it: its fields, then its conversion.
pub fn read(value: &Value, used: &Use) -> Result<Value, Fault> {
    let mut value = value.clone();
    for field in &used.fields {
        value = match &value {
            Value::Struct(fields) => fields.get(*field as usize).cloned().ok_or_else(|| Fault::bug(format!("no field {field} on this wire")))?,
            _ => return Err(Fault::bug(format!("field {field} read on a value that is no struct"))),
        };
    }
    match &used.convert {
        Some(conversion) => convert(value, conversion),
        None => Ok(value),
    }
}

/// A number as an `f64`, as a conversion goes through one: an integer of up to 32 bits is exact (3.4), and so is an `f32`.
fn as_f64(value: &Value) -> Option<f64> {
    Some(match value {
        Value::I8(x) => f64::from(*x),
        Value::I16(x) => f64::from(*x),
        Value::I32(x) => f64::from(*x),
        Value::U8(x) => f64::from(*x),
        Value::U16(x) => f64::from(*x),
        Value::U32(x) => f64::from(*x),
        // Explicit only (Q-4): the nearest `f64`.
        Value::I64(x) => *x as f64,
        Value::U64(x) => *x as f64,
        Value::F32(x) => f64::from(*x),
        Value::F64(x) => *x,
        _ => return None,
    })
}

fn as_i128(value: &Value) -> Option<i128> {
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

/// An integer of this kind, if it is in its range.
pub fn integer(kind: NumberKind, value: i128) -> Option<Value> {
    Some(match kind {
        NumberKind::I8 => Value::I8(i8::try_from(value).ok()?),
        NumberKind::I16 => Value::I16(i16::try_from(value).ok()?),
        NumberKind::I32 => Value::I32(i32::try_from(value).ok()?),
        NumberKind::I64 => Value::I64(i64::try_from(value).ok()?),
        NumberKind::U8 => Value::U8(u8::try_from(value).ok()?),
        NumberKind::U16 => Value::U16(u16::try_from(value).ok()?),
        NumberKind::U32 => Value::U32(u32::try_from(value).ok()?),
        NumberKind::U64 => Value::U64(u64::try_from(value).ok()?),
        _ => return None,
    })
}

/// A real number in a floating-point or complex kind: an `f32` is rounded once.
fn float(kind: NumberKind, x: f64) -> Option<Value> {
    Some(match kind {
        NumberKind::F32 => Value::F32(x as f32),
        NumberKind::F64 => Value::F64(x),
        NumberKind::C64 => Value::C64(x as f32, 0.0),
        NumberKind::C128 => Value::C128(x, 0.0),
        _ => return None,
    })
}

/// A duration from an `f64[s]`: not finite, or beyond `i64` nanoseconds, is `LC8008` (Q-25).
pub fn duration_of_seconds(seconds: f64) -> Result<i64, Fault> {
    nanoseconds(seconds)
        .map_err(|_| Fault::out_of_range(format!("{seconds} s is no duration: a duration is a whole number of nanoseconds that an i64 holds (3.1)")))
}

/// A conversion on a wire, as `check` recorded it (`docs/ir.md`, section 4).
pub fn convert(value: Value, conversion: &Conversion) -> Result<Value, Fault> {
    let wrong = || Fault::bug("a conversion on a wire that does not hold a number of its kind");
    match conversion {
        Conversion::Number { to, scale } => {
            if to.is_complex() {
                let single = *to == NumberKind::C64;
                let (re, im) = match value {
                    Value::C64(re, im) => (f64::from(re), f64::from(im)),
                    Value::C128(re, im) => (re, im),
                    other => (as_f64(&other).ok_or_else(wrong)?, 0.0),
                };
                let (re, im) = (apply_scale(re, scale, single), apply_scale(im, scale, single));
                return Ok(if single { Value::C64(re as f32, im as f32) } else { Value::C128(re, im) });
            }
            if to.is_integer() {
                // Integers only widen; a change of scale goes through a floating-point number (3.4), which `check` asked for.
                if *scale != Scale::default() {
                    return Err(wrong());
                }
                return as_i128(&value).and_then(|x| integer(*to, x)).ok_or_else(wrong);
            }
            let x = as_f64(&value).ok_or_else(wrong)?;
            float(*to, apply_scale(x, scale, *to == NumberKind::F32)).ok_or_else(wrong)
        }
        Conversion::ToDuration { scale } => {
            let x = as_f64(&value).ok_or_else(wrong)?;
            Ok(Value::Duration(duration_of_seconds(apply_scale(x, scale, false))?))
        }
        Conversion::FromDuration { to, scale } => {
            let Value::Duration(ns) = value else { return Err(wrong()) };
            if to.is_integer() {
                return Err(wrong());
            }
            let x = apply_scale(ns as f64 / 1e9, scale, false);
            float(*to, x).ok_or_else(wrong)
        }
    }
}

/// The value of a constant of the program, in its own type (`docs/ir.md`, section 4).
pub fn constant(env: Env<'_>, id: ConstId) -> Result<Value, Fault> {
    let constants = &env.program.constants;
    let found = constants.get(id.0 as usize).ok_or_else(|| Fault::bug(format!("no constant {}", id.0)))?;
    let wrong = || Fault::bug(format!("constant {} does not hold a value of its type", id.0));
    Ok(match &found.value {
        ConstValue::Bool(value) => Value::Bool(*value),
        ConstValue::Int(value) => {
            let kind = env.number_kind(found.ty).ok_or_else(wrong)?;
            if kind.is_integer() { integer(kind, *value) } else { float(kind, *value as f64) }.ok_or_else(wrong)?
        }
        ConstValue::Float(value) => float(env.number_kind(found.ty).ok_or_else(wrong)?, *value).ok_or_else(wrong)?,
        ConstValue::Complex { re, im } => match env.number_kind(found.ty) {
            Some(NumberKind::C64) => Value::C64(*re as f32, *im as f32),
            Some(NumberKind::C128) => Value::C128(*re, *im),
            _ => return Err(wrong()),
        },
        ConstValue::Text(text) => Value::String(Arc::from(&**text)),
        ConstValue::Bytes(bytes) => Value::Bytes(Arc::from(&**bytes)),
        ConstValue::Duration(ns) => Value::Duration(*ns),
        ConstValue::NoError => Value::NONE,
        ConstValue::Array(items) => {
            let values = items.iter().map(|item| constant(env, *item)).collect::<Result<Vec<_>, _>>()?;
            match env.ty(found.ty) {
                Ty::Array { dimensions, .. } if *dimensions >= 2 => grid(values, usize::from(*dimensions)).ok_or_else(wrong)?,
                Ty::Array { .. } => Value::array(values),
                _ => return Err(wrong()),
            }
        }
        ConstValue::Struct(fields) => Value::Struct(fields.iter().map(|field| constant(env, *field)).collect::<Result<Vec<_>, _>>()?.into()),
        ConstValue::Variant { variant, values } => Value::Variant(Arc::new(VariantValue {
            index: *variant,
            values: values.iter().map(|value| constant(env, *value)).collect::<Result<Vec<_>, _>>()?.into(),
        })),
        ConstValue::Function(_) | ConstValue::Primitive { .. } => Value::Function(id),
        ConstValue::InvalidHandle => Value::Handle(Handle::INVALID),
        ConstValue::ToTheEnd => Value::ToTheEnd,
        ConstValue::Unsettled => return Err(Fault::bug(format!("constant {} is a default that Q-21 has not settled", id.0))),
    })
}

/// A rectangular array from its rows, each an array of one level less (`[T]` or a grid): `None` when they differ in shape.
pub fn grid(rows: Vec<Value>, dimensions: usize) -> Option<Value> {
    if rows.is_empty() {
        return Some(Value::Grid(Arc::new(Grid::empty(dimensions))));
    }
    let count = rows.len();
    let mut inner: Option<Vec<usize>> = None;
    let mut items = Vec::new();
    for row in rows {
        let (shape, row_items): (Vec<usize>, Vec<Value>) = match row {
            Value::Array(row) => (vec![row.len()], Arc::unwrap_or_clone(row).into_items()),
            Value::Grid(row) => (row.shape().to_vec(), row.items().to_vec()),
            _ => return None,
        };
        match &inner {
            Some(expected) if *expected != shape => return None,
            Some(_) => {}
            None => inner = Some(shape),
        }
        items.extend(row_items);
    }
    let mut shape = vec![count];
    shape.extend(inner.unwrap_or_default());
    (shape.len() == dimensions).then_some(())?;
    Grid::new(shape.into_boxed_slice(), items).map(|grid| Value::Grid(Arc::new(grid)))
}
