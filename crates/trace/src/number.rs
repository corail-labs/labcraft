//! Numbers, durations and timestamps as text (`docs/commands.md`, 5). Every text here is also a constant of the language
//! (spec 14.3), so an agent can give it back as an argument.

use std::fmt::Write as _;

/// The decimal notation is kept from 10^-5 up to below 10^16; beyond, an exponent keeps the text short.
const PLAIN_FROM: i32 = -5;
const PLAIN_BELOW: i32 = 16;

/// A float as the fewest digits that read back the same, in decimal notation within [`PLAIN_FROM`, `PLAIN_BELOW`) and
/// with an exponent outside; a float with no fraction keeps `.0`, so that `12.0` is not taken for an integer. `nan`,
/// `inf` and `-inf` as the language writes them.
pub fn f64_text(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    // `{:e}` gives the shortest digits that read back the same, with one digit before the point.
    layout(&format!("{:e}", value.abs()), value.is_sign_negative())
}

/// As [`f64_text`], with the digits that read back the same `f32`.
pub fn f32_text(value: f32) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    layout(&format!("{:e}", value.abs()), value.is_sign_negative())
}

fn layout(scientific: &str, negative: bool) -> String {
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let mut text = String::with_capacity(digits.len() + 8);
    if negative {
        text.push('-');
    }
    if digits.bytes().all(|digit| digit == b'0') {
        text.push_str("0.0");
        return text;
    }
    if (PLAIN_FROM..PLAIN_BELOW).contains(&exponent) {
        if let Ok(integer) = usize::try_from(exponent + 1)
            && integer > 0
        {
            let (whole, fraction) = digits.split_at(integer.min(digits.len()));
            text.push_str(whole);
            text.extend(std::iter::repeat_n('0', integer.saturating_sub(digits.len())));
            text.push('.');
            text.push_str(if fraction.is_empty() { "0" } else { fraction });
        } else {
            text.push_str("0.");
            text.extend(std::iter::repeat_n('0', usize::try_from(-exponent - 1).unwrap_or(0)));
            text.push_str(&digits);
        }
    } else {
        let mut chars = digits.chars();
        text.extend(chars.next());
        let rest = chars.as_str();
        if !rest.is_empty() {
            text.push('.');
            text.push_str(rest);
        }
        let _ = write!(text, "e{exponent}");
    }
    text
}

/// The largest integer that a JSON reader holds exactly in a double: beyond, an integer is written as a string.
const EXACT: u64 = (1 << 53) - 1;

pub fn i64_is_exact(value: i64) -> bool {
    value.unsigned_abs() <= EXACT
}

pub fn u64_is_exact(value: u64) -> bool {
    value <= EXACT
}

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// A `duration` in seconds, exactly, from its whole nanoseconds (spec 3.1): `0.1 s`, `-2 s`, `0.00000025 s`.
pub fn duration_text(nanos: i64) -> String {
    let mut text = String::new();
    if nanos < 0 {
        text.push('-');
    }
    let magnitude = nanos.unsigned_abs();
    let _ = write!(text, "{}", magnitude / NANOS_PER_SECOND);
    push_fraction(&mut text, magnitude % NANOS_PER_SECOND, 9);
    text.push_str(" s");
    text
}

/// The clock of a run in milliseconds, exactly, from its whole nanoseconds: `0.0`, `1.5`, `0.00025`. A JSON number.
pub fn millis_text(nanos: u64) -> String {
    let mut text = format!("{}", nanos / 1_000_000);
    let fraction = nanos % 1_000_000;
    if fraction == 0 {
        text.push_str(".0");
    } else {
        push_fraction(&mut text, fraction, 6);
    }
    text
}

/// `.` and the digits of a fraction of `10^width`, without the zeros that end it; nothing for 0.
fn push_fraction(text: &mut String, fraction: u64, width: usize) {
    if fraction == 0 {
        return;
    }
    let digits = format!("{fraction:0width$}");
    text.push('.');
    text.push_str(digits.trim_end_matches('0'));
}

const SECONDS_PER_DAY: i64 = 86_400;

/// A `timestamp`, from nanoseconds since 1970-01-01T00:00:00Z, as RFC 3339 in UTC, to the nanosecond:
/// `2026-11-08T12:00:00.000000250Z`. An `i64` of nanoseconds reaches from 1677 to 2262, always four digits of year.
pub fn timestamp_text(nanos: i64) -> String {
    let seconds = nanos.div_euclid(NANOS_PER_SECOND as i64);
    let fraction = nanos.rem_euclid(NANOS_PER_SECOND as i64);
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let second_of_day = seconds.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{fraction:09}Z", second_of_day / 3600, second_of_day / 60 % 60, second_of_day % 60)
}

/// The date of a day counted from 1970-01-01, in the proleptic Gregorian calendar (H. Hinnant, `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 with padding (RFC 4648, section 4).
pub fn base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let symbol = |index: u32| char::from(BASE64.get((index & 63) as usize).copied().unwrap_or(b'A'));
    for chunk in bytes.chunks(3) {
        let mut group = [0u8; 3];
        for (slot, byte) in group.iter_mut().zip(chunk) {
            *slot = *byte;
        }
        let [first, second, third] = group;
        let bits = u32::from(first) << 16 | u32::from(second) << 8 | u32::from(third);
        text.push(symbol(bits >> 18));
        text.push(symbol(bits >> 12));
        text.push(if chunk.len() > 1 { symbol(bits >> 6) } else { '=' });
        text.push(if chunk.len() > 2 { symbol(bits) } else { '=' });
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked out by hand from the rule: the fewest digits, decimal from 1e-5 to below 1e16, `.0` on a whole float.
    #[test]
    fn floats_read_as_the_language_writes_them() {
        let cases: &[(f64, &str)] = &[
            (230.071217, "230.071217"),
            (12.0, "12.0"),
            (0.0015, "0.0015"),
            (-0.0015, "-0.0015"),
            (1.5e-7, "1.5e-7"),
            (1e16, "1e16"),
            (1e15, "1000000000000000.0"),
            (1e-5, "0.00001"),
            (9.999e-6, "9.999e-6"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (0.5, "0.5"),
            (123_456_789_012_345_680.0, "1.2345678901234568e17"),
            (f64::MAX, "1.7976931348623157e308"),
            (5e-324, "5e-324"),
            (f64::NAN, "nan"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ];
        for (value, text) in cases {
            assert_eq!(f64_text(*value), *text, "{value:e}");
        }
        // 0.1 as an f32 is 0.100000001490116…, which only the digits of an f32 write as 0.1.
        assert_eq!(f32_text(0.1), "0.1");
        assert_eq!(f64_text(f64::from(0.1f32)), "0.10000000149011612");
        assert_eq!(f32_text(f32::MAX), "3.4028235e38");
    }

    #[test]
    fn integers_beyond_two_to_the_53_are_not_exact() {
        assert!(i64_is_exact(9_007_199_254_740_991));
        assert!(!i64_is_exact(9_007_199_254_740_992));
        assert!(i64_is_exact(-9_007_199_254_740_991));
        assert!(!i64_is_exact(i64::MIN));
        assert!(!u64_is_exact(u64::MAX));
    }

    #[test]
    fn durations_are_exact_seconds() {
        assert_eq!(duration_text(100_000_000), "0.1 s");
        assert_eq!(duration_text(2_000_000_000), "2 s");
        assert_eq!(duration_text(0), "0 s");
        assert_eq!(duration_text(250), "0.00000025 s");
        assert_eq!(duration_text(-1_500_000_000), "-1.5 s");
        assert_eq!(duration_text(i64::MIN), "-9223372036.854775808 s");
        assert_eq!(millis_text(0), "0.0");
        assert_eq!(millis_text(1_500_000), "1.5");
        assert_eq!(millis_text(250), "0.00025");
        assert_eq!(millis_text(2_000_000_000), "2000.0");
    }

    /// The nanoseconds come from Python: `datetime(2026, 11, 8, 12, tzinfo=timezone.utc).timestamp()` is 1794139200.
    #[test]
    fn timestamps_are_rfc_3339_in_utc() {
        assert_eq!(timestamp_text(0), "1970-01-01T00:00:00.000000000Z");
        assert_eq!(timestamp_text(1_794_139_200_000_000_250), "2026-11-08T12:00:00.000000250Z");
        assert_eq!(timestamp_text(-1), "1969-12-31T23:59:59.999999999Z");
        // 2000-02-29, a leap day of a year divisible by 400: 951782400 s.
        assert_eq!(timestamp_text(951_782_400_000_000_000), "2000-02-29T00:00:00.000000000Z");
        // The ends of an i64, from Python: 1677-09-21T00:12:43.145224192Z and 2262-04-11T23:47:16.854775807Z.
        assert_eq!(timestamp_text(i64::MIN), "1677-09-21T00:12:43.145224192Z");
        assert_eq!(timestamp_text(i64::MAX), "2262-04-11T23:47:16.854775807Z");
    }

    /// RFC 4648, section 10.
    #[test]
    fn base64_follows_rfc_4648() {
        for (bytes, text) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(base64(bytes.as_bytes()), text);
        }
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }
}
