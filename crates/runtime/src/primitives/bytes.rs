//! `bytes` (appendix A.2): the binary data of frames and of instrument responses. A body that fails gives its error, and its
//! outputs are those of a node that fails (spec 7.2): `bytes.slice` passes its input `data` through, a `read_…` gives 0 and
//! a `from_…` empty bytes. A body that builds bytes or an array asks the run for its room first (cli.md 6.2).

use std::sync::Arc;

use super::{Call, Host, Outcome, input, room_for, variadic, wrong};
use crate::fault::Fault;
use crate::value::{VALUE_COST, Value, logical_size, text_cost};

const OUT_OF_RANGE: &str = "bytes.out_of_range";

fn data_of<'v>(value: &'v Value, name: &str) -> Result<&'v Arc<[u8]>, Fault> {
    match value {
        Value::Bytes(data) => Ok(data),
        _ => Err(wrong(name)),
    }
}

fn i32_of(call: &Call<'_>, at: usize, name: &str) -> Result<i32, Fault> {
    match input(call, at)? {
        Value::I32(value) => Ok(*value),
        _ => Err(wrong(name)),
    }
}

fn bool_of(call: &Call<'_>, at: usize, name: &str) -> Result<bool, Fault> {
    match input(call, at)? {
        Value::Bool(value) => Ok(*value),
        _ => Err(wrong(name)),
    }
}

/// New bytes, once the run has room for them.
fn made(data: Vec<u8>, host: &dyn Host, name: &str) -> Result<Value, Fault> {
    room_for(host, text_cost(data.len()), name)?;
    Ok(Value::Bytes(Arc::from(data)))
}

pub fn len(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let data = data_of(input(call, 0)?, "bytes.len")?;
    let length = i32::try_from(data.len()).map_err(|_| Fault::out_of_range(format!("bytes.len: a length of {} is not an i32 (Q-25)", data.len())))?;
    Ok(Outcome::Done(vec![Value::I32(length)]))
}

/// The parts one after the other; none gives empty bytes.
pub fn concat(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let parts = variadic(call)?;
    let mut length = 0_usize;
    for part in parts {
        length = length.saturating_add(data_of(part, "bytes.concat")?.len());
    }
    room_for(host, text_cost(length), "bytes.concat")?;
    let mut all = Vec::with_capacity(length);
    for part in parts {
        all.extend_from_slice(data_of(part, "bytes.concat")?);
    }
    Ok(Outcome::Done(vec![Value::Bytes(Arc::from(all))]))
}

/// From `start` for `length` bytes, as `array.slice` takes items: with no wire, `start` is 0 and `length` reaches the end;
/// a negative value or what goes past the end fails, and the output `data` passes the input through.
pub fn slice(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let data = data_of(input(call, 0)?, "bytes.slice")?;
    let start = i32_of(call, 1, "bytes.slice")?;
    let total = data.len();
    let fail = |message: String| Outcome::Fail(host.error(OUT_OF_RANGE, 0, message));
    let Ok(first) = usize::try_from(start) else {
        return Ok(fail(format!("start {start} is negative")));
    };
    let count = match input(call, 2)? {
        Value::ToTheEnd => match total.checked_sub(first) {
            Some(count) => count,
            None => return Ok(fail(format!("start {start} is past the end of the bytes (length {total})"))),
        },
        Value::I32(length) => match usize::try_from(*length) {
            Ok(count) => count,
            Err(_) => return Ok(fail(format!("length {length} is negative"))),
        },
        _ => return Err(wrong("bytes.slice")),
    };
    let Some(part) = first.checked_add(count).and_then(|end| data.get(first..end)) else {
        return Ok(fail(format!("{count} bytes from position {start} go past the end of the bytes (length {total})")));
    };
    Ok(Outcome::Done(vec![made(part.to_vec(), host, "bytes.slice")?, Value::NONE]))
}

pub fn from_array(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::Array(values) = input(call, 0)? else { return Err(wrong("bytes.from_array")) };
    let data = values
        .items()
        .iter()
        .map(|value| match value {
            Value::U8(byte) => Ok(*byte),
            _ => Err(wrong("bytes.from_array")),
        })
        .collect::<Result<Vec<u8>, Fault>>()?;
    Ok(Outcome::Done(vec![made(data, host, "bytes.from_array")?]))
}

pub fn to_array(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let data = data_of(input(call, 0)?, "bytes.to_array")?;
    let count = u64::try_from(data.len()).unwrap_or(u64::MAX);
    room_for(host, VALUE_COST.saturating_add(count.saturating_mul(logical_size(&Value::U8(0)))), "bytes.to_array")?;
    Ok(Outcome::Done(vec![Value::array(data.iter().map(|byte| Value::U8(*byte)).collect())]))
}

/// Its UTF-8 bytes.
pub fn from_string(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::String(text) = input(call, 0)? else { return Err(wrong("bytes.from_string")) };
    Ok(Outcome::Done(vec![made(text.as_bytes().to_vec(), host, "bytes.from_string")?]))
}

/// The text the bytes hold in UTF-8; bytes that are not UTF-8 fail, and the text is `""`.
pub fn to_string(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let data = data_of(input(call, 0)?, "bytes.to_string")?;
    match std::str::from_utf8(data) {
        Ok(text) => {
            room_for(host, text_cost(text.len()), "bytes.to_string")?;
            Ok(Outcome::Done(vec![Value::string(text), Value::NONE]))
        }
        Err(error) => {
            let at = error.valid_up_to();
            let message = match error.error_len() {
                Some(_) => format!("not UTF-8: no character at position {at}"),
                None => format!("not UTF-8: the character at position {at} is cut short by the end"),
            };
            Ok(Outcome::Fail(host.error("bytes.invalid_text", 0, message)))
        }
    }
}

/// The `N` bytes at `position`, as they are written (big endian) or turned around (`little_endian`); the message of
/// `bytes.out_of_range` when they are not all there.
fn bytes_at<const N: usize>(call: &Call<'_>, name: &str, with_order: bool) -> Result<Result<[u8; N], String>, Fault> {
    let data = data_of(input(call, 0)?, name)?;
    let position = i32_of(call, 1, name)?;
    let little_endian = with_order && bool_of(call, 2, name)?;
    let Ok(first) = usize::try_from(position) else {
        return Ok(Err(format!("position {position} is negative")));
    };
    let Some(mut read) = first.checked_add(N).and_then(|end| data.get(first..end)).and_then(|part| <[u8; N]>::try_from(part).ok()) else {
        let length = data.len();
        return Ok(Err(format!("{N} bytes at position {position} go past the end of the bytes (length {length})")));
    };
    if little_endian {
        read.reverse();
    }
    Ok(Ok(read))
}

/// A `read_…`: the value its bytes hold, read in big endian.
fn read<const N: usize>(call: &Call<'_>, host: &dyn Host, name: &str, with_order: bool, value: fn([u8; N]) -> Value) -> Result<Outcome, Fault> {
    Ok(match bytes_at::<N>(call, name, with_order)? {
        Ok(read) => Outcome::Done(vec![value(read), Value::NONE]),
        Err(message) => Outcome::Fail(host.error(OUT_OF_RANGE, 0, message)),
    })
}

pub fn read_u8(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<1>(call, host, "bytes.read_u8", false, |read| Value::U8(read[0]))
}

pub fn read_u16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<2>(call, host, "bytes.read_u16", true, |read| Value::U16(u16::from_be_bytes(read)))
}

pub fn read_i16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<2>(call, host, "bytes.read_i16", true, |read| Value::I16(i16::from_be_bytes(read)))
}

pub fn read_u32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<4>(call, host, "bytes.read_u32", true, |read| Value::U32(u32::from_be_bytes(read)))
}

pub fn read_i32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<4>(call, host, "bytes.read_i32", true, |read| Value::I32(i32::from_be_bytes(read)))
}

/// The `f32` its four bytes hold, a `nan` included.
pub fn read_f32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<4>(call, host, "bytes.read_f32", true, |read| Value::F32(f32::from_be_bytes(read)))
}

/// A `from_…` of an integer type: its bytes in big endian, from the `i64` it takes; a value outside the type fails, and
/// gives empty bytes.
fn from_integer(
    call: &Call<'_>,
    host: &dyn Host,
    name: &str,
    type_name: &str,
    with_order: bool,
    bytes: fn(i64) -> Option<Vec<u8>>,
) -> Result<Outcome, Fault> {
    let Value::I64(value) = input(call, 0)? else { return Err(wrong(name)) };
    let little_endian = with_order && bool_of(call, 1, name)?;
    let Some(mut data) = bytes(*value) else {
        return Ok(Outcome::Fail(host.error(OUT_OF_RANGE, 0, format!("{value} is outside the range of {type_name}"))));
    };
    if little_endian {
        data.reverse();
    }
    Ok(Outcome::Done(vec![made(data, host, name)?, Value::NONE]))
}

pub fn from_u8(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_u8", "u8", false, |value| u8::try_from(value).ok().map(|value| vec![value]))
}

pub fn from_u16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_u16", "u16", true, |value| u16::try_from(value).ok().map(|value| value.to_be_bytes().to_vec()))
}

pub fn from_i16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_i16", "i16", true, |value| i16::try_from(value).ok().map(|value| value.to_be_bytes().to_vec()))
}

pub fn from_u32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_u32", "u32", true, |value| u32::try_from(value).ok().map(|value| value.to_be_bytes().to_vec()))
}

pub fn from_i32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_i32", "i32", true, |value| i32::try_from(value).ok().map(|value| value.to_be_bytes().to_vec()))
}

/// The four bytes of the `f32` nearest the value, ties to even, an infinity beyond the `f32` (3.4); any `nan` is `7fc00000`,
/// whose bits do not depend on the platform (spec A.2, decision of the owner, 2026-11-19).
pub fn from_f32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::F64(value) = input(call, 0)? else { return Err(wrong("bytes.from_f32")) };
    let little_endian = bool_of(call, 1, "bytes.from_f32")?;
    // `as` rounds to the nearest `f32`, ties to even, and gives an infinity beyond it, as `core.to_f32` does.
    let single = if value.is_nan() { f32::from_bits(0x7fc0_0000) } else { *value as f32 };
    let data = if little_endian { single.to_le_bytes() } else { single.to_be_bytes() };
    Ok(Outcome::Done(vec![made(data.to_vec(), host, "bytes.from_f32")?]))
}
