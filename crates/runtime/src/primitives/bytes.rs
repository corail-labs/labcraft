//! `bytes` (appendix A.2): the binary data of frames and of instrument responses. A body that fails gives its error, and its
//! outputs are those of a node that fails (spec 7.2): `bytes.slice` passes its input `data` through, a `read_…` gives 0 and
//! a `from_…` empty bytes. A body that builds bytes or an array asks the run for its room first (cli.md 6.2).

use std::sync::Arc;

use super::{Call, Host, Outcome, bool_input, i32_input, input, length_value, room_for, variadic, wrong};
use crate::fault::Fault;
use crate::value::{VALUE_COST, Value, logical_size, text_cost};

const OUT_OF_RANGE: &str = "bytes.out_of_range";

fn data_of<'v>(value: &'v Value, name: &str) -> Result<&'v Arc<[u8]>, Fault> {
    match value {
        Value::Bytes(data) => Ok(data),
        _ => Err(wrong(name)),
    }
}

/// New bytes, copied once the run has room for them.
fn made(data: &[u8], host: &dyn Host, name: &str) -> Result<Value, Fault> {
    room_for(host, text_cost(data.len()), name)?;
    Ok(Value::Bytes(Arc::from(data)))
}

pub fn len(call: &mut Call<'_>, _: &mut dyn Host) -> Result<Outcome, Fault> {
    let data = data_of(input(call, 0)?, "bytes.len")?;
    Ok(Outcome::Done(vec![length_value(data.len(), "bytes.len")?]))
}

/// The parts one after the other; none gives empty bytes.
pub fn concat(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let parts = variadic(call)?.iter().map(|part| data_of(part, "bytes.concat")).collect::<Result<Vec<_>, Fault>>()?;
    let length = parts.iter().fold(0_usize, |length, part| length.saturating_add(part.len()));
    room_for(host, text_cost(length), "bytes.concat")?;
    let all: Arc<[u8]> = parts.iter().flat_map(|part| part.iter().copied()).collect();
    Ok(Outcome::Done(vec![Value::Bytes(all)]))
}

/// From `start` for `length` bytes, as `array.slice` takes items: with no wire, `start` is 0 and `length` reaches the end;
/// a negative value or what goes past the end fails, and the output `data` passes the input through.
pub fn slice(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let data = data_of(input(call, 0)?, "bytes.slice")?;
    let start = i32_input(call, 1, "bytes.slice")?;
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
    Ok(Outcome::Done(vec![made(part, host, "bytes.slice")?, Value::NONE]))
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
    Ok(Outcome::Done(vec![made(&data, host, "bytes.from_array")?]))
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
    Ok(Outcome::Done(vec![made(text.as_bytes(), host, "bytes.from_string")?]))
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

/// A `read_…`: the value of the `N` bytes at `position`, read in big endian, or turned around first with `little_endian`
/// (an input only when `N` is above 1); bytes that are not all there fail, and the value is 0.
fn read<const N: usize>(call: &Call<'_>, host: &dyn Host, name: &str, value: fn([u8; N]) -> Value) -> Result<Outcome, Fault> {
    let data = data_of(input(call, 0)?, name)?;
    let position = i32_input(call, 1, name)?;
    let little_endian = N > 1 && bool_input(call, 2, name)?;
    let read = usize::try_from(position)
        .ok()
        .and_then(|first| first.checked_add(N).and_then(|end| data.get(first..end)))
        .and_then(|part| <[u8; N]>::try_from(part).ok());
    let Some(mut read) = read else {
        let message = if position < 0 {
            format!("position {position} is negative")
        } else {
            format!("{N} bytes at position {position} go past the end of the bytes (length {})", data.len())
        };
        return Ok(Outcome::Fail(host.error(OUT_OF_RANGE, 0, message)));
    };
    if little_endian {
        read.reverse();
    }
    Ok(Outcome::Done(vec![value(read), Value::NONE]))
}

pub fn read_u8(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<1>(call, host, "bytes.read_u8", |read| Value::U8(read[0]))
}

pub fn read_u16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<2>(call, host, "bytes.read_u16", |read| Value::U16(u16::from_be_bytes(read)))
}

pub fn read_i16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<2>(call, host, "bytes.read_i16", |read| Value::I16(i16::from_be_bytes(read)))
}

pub fn read_u32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<4>(call, host, "bytes.read_u32", |read| Value::U32(u32::from_be_bytes(read)))
}

pub fn read_i32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<4>(call, host, "bytes.read_i32", |read| Value::I32(i32::from_be_bytes(read)))
}

/// The `f32` its four bytes hold, a `nan` included.
pub fn read_f32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    read::<4>(call, host, "bytes.read_f32", |read| Value::F32(f32::from_be_bytes(read)))
}

/// A `from_…` of an integer type: its bytes in big endian, from the `i64` it takes, turned around with `little_endian` (an
/// input only when `N` is above 1); a value outside the type fails, and gives empty bytes.
fn from_integer<const N: usize>(
    call: &Call<'_>,
    host: &dyn Host,
    name: &str,
    type_name: &str,
    bytes: fn(i64) -> Option<[u8; N]>,
) -> Result<Outcome, Fault> {
    let Value::I64(value) = input(call, 0)? else { return Err(wrong(name)) };
    let little_endian = N > 1 && bool_input(call, 1, name)?;
    let Some(mut data) = bytes(*value) else {
        return Ok(Outcome::Fail(host.error(OUT_OF_RANGE, 0, format!("{value} is outside the range of {type_name}"))));
    };
    if little_endian {
        data.reverse();
    }
    Ok(Outcome::Done(vec![made(&data, host, name)?, Value::NONE]))
}

pub fn from_u8(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_u8", "u8", |value| u8::try_from(value).ok().map(u8::to_be_bytes))
}

pub fn from_u16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_u16", "u16", |value| u16::try_from(value).ok().map(u16::to_be_bytes))
}

pub fn from_i16(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_i16", "i16", |value| i16::try_from(value).ok().map(i16::to_be_bytes))
}

pub fn from_u32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_u32", "u32", |value| u32::try_from(value).ok().map(u32::to_be_bytes))
}

pub fn from_i32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    from_integer(call, host, "bytes.from_i32", "i32", |value| i32::try_from(value).ok().map(i32::to_be_bytes))
}

/// The four bytes of the `f32` nearest the value, ties to even, an infinity beyond the `f32` (3.4); any `nan` is `7fc00000`,
/// whose bits do not depend on the platform (spec A.2, decision of the owner, 2026-11-19).
pub fn from_f32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::F64(value) = input(call, 0)? else { return Err(wrong("bytes.from_f32")) };
    let little_endian = bool_input(call, 1, "bytes.from_f32")?;
    // `as` rounds to the nearest `f32`, ties to even, and gives an infinity beyond it, as `core.to_f32` does.
    let single = if value.is_nan() { f32::from_bits(0x7fc0_0000) } else { *value as f32 };
    let data = if little_endian { single.to_le_bytes() } else { single.to_be_bytes() };
    Ok(Outcome::Done(vec![made(&data, host, "bytes.from_f32")?]))
}
