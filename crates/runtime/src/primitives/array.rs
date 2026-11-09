//! `array` (appendix A.2). A body that fails gives its error, and its outputs are those of a node that fails (spec 7.2): the
//! output `array`, named like the input, passes it through unchanged, for `array.set` as for `array.slice` (decision of the
//! owner, 2026-11-09). A body that builds an array asks the run for its room first, so that `array.repeat("", 2147483647)`
//! is the fault `LC8005` rather than the memory of the process (cli.md 6.2).

use std::sync::Arc;

use labcraft_types::{NumberKind, Ty};

use super::number::{as_i128, to_kind};
use super::{Call, Host, Outcome, input, output_type, room_for, variadic, wrong};
use crate::convert::{grid, integer};
use crate::fault::Fault;
use crate::value::{Grid, Items, VALUE_COST, Value, logical_size};

fn index_of(call: &Call<'_>, at: usize, name: &str) -> Result<i32, Fault> {
    match input(call, at)? {
        Value::I32(index) => Ok(*index),
        _ => Err(wrong(name)),
    }
}

fn length_value(length: usize, name: &str) -> Result<Value, Fault> {
    i32::try_from(length).map(Value::I32).map_err(|_| Fault::out_of_range(format!("{name}: a length of {length} is not an i32 (Q-25)")))
}

fn out_of_range(host: &dyn Host, index: i64, length: usize) -> Outcome {
    Outcome::Fail(host.error("array.index_out_of_range", 0, format!("index {index} outside the array (size {length})")))
}

/// The array input as one level of items: an `[T; 2]` is its rows, each an `[T]`, so that `array.set`, `push` and `concat`
/// give an `[[T]]` (appendix A.2). The input of the call becomes that array, so that a node that fails passes it through
/// in the type of its output.
fn as_rows(call: &mut Call<'_>, host: &dyn Host, at: usize, name: &str) -> Result<(), Fault> {
    let slot = call.inputs.get_mut(at).ok_or_else(|| wrong(name))?;
    if let Value::Grid(grid) = slot {
        // A copy, row by row: one value more for each row, and the array.
        room_for(host, rows_size(grid), name)?;
        *slot = Value::array(grid.to_rows());
    }
    Ok(())
}

/// The logical size of the rows of a grid as an `[[T]]`.
fn rows_size(grid: &Grid) -> u64 {
    array_size(grid.size().saturating_add(VALUE_COST.saturating_mul(u64::try_from(grid.rows()).unwrap_or(u64::MAX))))
}

fn items_of<'v>(call: &'v Call<'_>, at: usize, name: &str) -> Result<&'v Arc<Items>, Fault> {
    match input(call, at)? {
        Value::Array(items) => Ok(items),
        _ => Err(wrong(name)),
    }
}

/// Takes the array input out of the call, to change it in place when nothing else holds it.
fn take_items(call: &mut Call<'_>, at: usize, name: &str) -> Result<Arc<Items>, Fault> {
    match call.inputs.get_mut(at).map(|slot| std::mem::replace(slot, Value::NONE)) {
        Some(Value::Array(items)) => Ok(items),
        _ => Err(wrong(name)),
    }
}

/// The logical size of an array of items of this size.
fn array_size(items_size: u64) -> u64 {
    VALUE_COST.saturating_add(items_size)
}

/// What a change makes the run hold: only what it adds when nothing else holds the array, which then changes in place, and
/// a whole new array when it is shared, since the change works on a copy (D-11).
fn room_to_change(host: &dyn Host, items: &mut Arc<Items>, added: u64, removed: u64, name: &str) -> Result<(), Fault> {
    let alone = Arc::get_mut(items).is_some();
    let needed = if alone { added.saturating_sub(removed) } else { array_size(items.size().saturating_sub(removed).saturating_add(added)) };
    room_for(host, needed, name)
}

pub fn len(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let length = match input(call, 0)? {
        Value::Array(items) => items.len(),
        Value::Grid(grid) => grid.rows(),
        _ => return Err(wrong("array.len")),
    };
    Ok(Outcome::Done(vec![length_value(length, "array.len")?]))
}

/// The item at `index`; of an `[T; 2]`, the row.
pub fn get(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let index = index_of(call, 1, "array.get")?;
    let (item, length) = match input(call, 0)? {
        Value::Array(items) => (usize::try_from(index).ok().and_then(|at| items.items().get(at)).cloned(), items.len()),
        Value::Grid(grid) => {
            // A row is a copy of its items.
            let row = usize::try_from(index).ok().filter(|at| *at < grid.rows());
            if row.is_some() {
                let rows = u64::try_from(grid.rows()).unwrap_or(u64::MAX).max(1);
                room_for(host, array_size(grid.size() / rows), "array.get")?;
            }
            (row.and_then(|at| grid.row(at)), grid.rows())
        }
        _ => return Err(wrong("array.get")),
    };
    match item {
        Some(item) => Ok(Outcome::Done(vec![item, Value::NONE])),
        None => Ok(out_of_range(host, i64::from(index), length)),
    }
}

pub fn set(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    as_rows(call, host, 0, "array.set")?;
    let index = index_of(call, 1, "array.set")?;
    let length = items_of(call, 0, "array.set")?.len();
    let Some(at) = usize::try_from(index).ok().filter(|at| *at < length) else {
        return Ok(out_of_range(host, i64::from(index), length));
    };
    let item = call.inputs.get_mut(2).map(|slot| std::mem::replace(slot, Value::NONE)).ok_or_else(|| wrong("array.set"))?;
    let mut items = take_items(call, 0, "array.set")?;
    let old = items.items().get(at).map_or(0, logical_size);
    room_to_change(host, &mut items, logical_size(&item), old, "array.set")?;
    Arc::make_mut(&mut items).set(at, item);
    Ok(Outcome::Done(vec![Value::Array(items), Value::NONE]))
}

pub fn push(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    as_rows(call, host, 0, "array.push")?;
    let item = call.inputs.get_mut(1).map(|slot| std::mem::replace(slot, Value::NONE)).ok_or_else(|| wrong("array.push"))?;
    let mut items = take_items(call, 0, "array.push")?;
    room_to_change(host, &mut items, logical_size(&item), 0, "array.push")?;
    Arc::make_mut(&mut items).push(item);
    Ok(Outcome::Done(vec![Value::Array(items)]))
}

/// The items of each part, in order; the rows of an `[T; 2]`.
pub fn concat(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let parts = variadic(call)?;
    let mut size = 0_u64;
    let mut count = 0_usize;
    for part in parts {
        let (part_size, part_count) = match part {
            Value::Array(items) => (items.size(), items.len()),
            // Each row becomes an array of its own: one value more per row.
            Value::Grid(grid) => (rows_size(grid).saturating_sub(VALUE_COST), grid.rows()),
            _ => return Err(wrong("array.concat")),
        };
        size = size.saturating_add(part_size);
        count = count.saturating_add(part_count);
    }
    room_for(host, array_size(size), "array.concat")?;
    let mut all = Vec::with_capacity(count);
    for part in parts {
        match part {
            Value::Array(items) => all.extend(items.items().iter().cloned()),
            Value::Grid(grid) => all.extend(grid.to_rows()),
            _ => {}
        }
    }
    Ok(Outcome::Done(vec![Value::array(all)]))
}

pub fn repeat(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let count = index_of(call, 1, "array.repeat")?;
    let Ok(count) = usize::try_from(count) else {
        return Ok(Outcome::Fail(host.error("array.negative_size", 0, format!("count {count} is negative"))));
    };
    let item = input(call, 0)?;
    let size = array_size(logical_size(item).saturating_mul(u64::try_from(count).unwrap_or(u64::MAX)));
    room_for(host, size, "array.repeat")?;
    Ok(Outcome::Done(vec![Value::array(vec![item.clone(); count]), Value::NONE]))
}

/// From `start` for `length` items, or rows of an `[T; 2]`: with no wire, `start` is 0 and `length` reaches the end; a
/// negative value or what goes past the end fails.
pub fn slice(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let start = index_of(call, 1, "array.slice")?;
    let length = input(call, 2)?;
    let total = match input(call, 0)? {
        Value::Array(items) => items.len(),
        Value::Grid(grid) => grid.rows(),
        _ => return Err(wrong("array.slice")),
    };
    let fail = |message: String| Outcome::Fail(host.error("array.index_out_of_range", 0, message));
    let Ok(first) = usize::try_from(start) else {
        return Ok(fail(format!("start {start} is negative")));
    };
    let count = match length {
        Value::ToTheEnd => match total.checked_sub(first) {
            Some(count) => count,
            None => return Ok(fail(format!("start {start} is past the end of the array (size {total})"))),
        },
        Value::I32(length) => match usize::try_from(*length) {
            Ok(count) => count,
            Err(_) => return Ok(fail(format!("length {length} is negative"))),
        },
        _ => return Err(wrong("array.slice")),
    };
    let Some(end) = first.checked_add(count).filter(|end| *end <= total) else {
        return Ok(fail(format!("{count} items from index {start} go past the end of the array (size {total})")));
    };
    let result = match input(call, 0)? {
        Value::Array(items) => Value::array(items.items().get(first..end).unwrap_or_default().to_vec()),
        Value::Grid(rows) => {
            let row = rows.row_len();
            let mut shape = rows.shape().to_vec();
            if let Some(first_level) = shape.first_mut() {
                *first_level = count;
            }
            let part = rows.items().get(first.saturating_mul(row)..end.saturating_mul(row)).unwrap_or_default().to_vec();
            Value::Grid(Arc::new(Grid::new(shape.into_boxed_slice(), part).ok_or_else(|| wrong("array.slice"))?))
        }
        _ => return Err(wrong("array.slice")),
    };
    Ok(Outcome::Done(vec![result, Value::NONE]))
}

/// Rows of one length make an `[T; 2]`; rows of different lengths fail, and the message says which row, with both lengths.
pub fn from_rows(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let rows = items_of(call, 0, "array.from_rows")?;
    let mut first: Option<usize> = None;
    for (index, row) in rows.items().iter().enumerate() {
        let Value::Array(row) = row else { return Err(wrong("array.from_rows")) };
        match first {
            None => first = Some(row.len()),
            Some(length) if length != row.len() => {
                let message = format!("row {index} has a length of {}, and row 0 of {length}", row.len());
                return Ok(Outcome::Fail(host.error("array.unequal_rows", 0, message)));
            }
            Some(_) => {}
        }
    }
    let dimensions = match call.env.ty(output_type(call, 0)?) {
        Ty::Array { dimensions, .. } => usize::from(*dimensions),
        _ => return Err(wrong("array.from_rows")),
    };
    let result = grid(rows.items().to_vec(), dimensions).ok_or_else(|| wrong("array.from_rows"))?;
    Ok(Outcome::Done(vec![result, Value::NONE]))
}

pub fn to_rows(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::Grid(rows) = input(call, 0)? else { return Err(wrong("array.to_rows")) };
    let row = rows.row_len();
    let result = if row == 0 {
        Value::array(vec![Value::array(Vec::new()); rows.rows()])
    } else {
        Value::array(rows.items().chunks(row).map(|chunk| Value::array(chunk.to_vec())).collect())
    };
    Ok(Outcome::Done(vec![result]))
}

/// From the first item to the last, in the kind of the output; `0` for no item. An integer sum that overflows is `LC8008`.
pub fn sum(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let kind = call.env.number_kind(output_type(call, 0)?).ok_or_else(|| wrong("array.sum"))?;
    let items = items_of(call, 0, "array.sum")?;
    let result = match kind {
        NumberKind::F32 => Value::F32(fold(items.items(), kind, 0.0_f32, |total, item| match item {
            Value::F32(x) => Some(total + x),
            _ => None,
        })?),
        NumberKind::F64 => Value::F64(fold(items.items(), kind, 0.0_f64, |total, item| match item {
            Value::F64(x) => Some(total + x),
            _ => None,
        })?),
        NumberKind::C64 => {
            let (re, im) = fold(items.items(), kind, (0.0_f32, 0.0_f32), |(re, im), item| match item {
                Value::C64(a, b) => Some((re + a, im + b)),
                _ => None,
            })?;
            Value::C64(re, im)
        }
        NumberKind::C128 => {
            let (re, im) = fold(items.items(), kind, (0.0_f64, 0.0_f64), |(re, im), item| match item {
                Value::C128(a, b) => Some((re + a, im + b)),
                _ => None,
            })?;
            Value::C128(re, im)
        }
        integer_kind => {
            // Exact in an `i128`, which no array that memory holds can pass: only the sum has to fit its kind, so
            // `[i32::MAX, 1, -1]` sums to `i32::MAX`.
            let mut total = 0_i128;
            for item in items.items() {
                let x = to_kind(item, integer_kind).as_ref().and_then(as_i128).ok_or_else(|| wrong("array.sum"))?;
                total = total.checked_add(x).ok_or_else(|| Fault::out_of_range("array.sum: the sum passes the range of an i128 (Q-25)"))?;
            }
            integer(integer_kind, total)
                .ok_or_else(|| Fault::out_of_range(format!("array.sum: the sum {total} is out of the range of `{}` (Q-25)", integer_kind.as_str())))?
        }
    };
    Ok(Outcome::Done(vec![result]))
}

fn fold<T>(items: &[Value], kind: NumberKind, zero: T, add: impl Fn(T, Value) -> Option<T>) -> Result<T, Fault> {
    items.iter().try_fold(zero, |total, item| to_kind(item, kind).and_then(|item| add(total, item)).ok_or_else(|| wrong("array.sum")))
}
