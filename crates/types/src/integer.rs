//! The integers as the language writes them (spec 14.1): `INT`, digits and `_`, and `BASED` (0.5.0), `0x` then a
//! hexadecimal digit or `0b` then a binary one, then those and `_`. The lexer, `check`, the lowering and the editor read
//! them here, so that they agree on what is one.

/// The digits after the prefix, and their base.
fn radix_of(text: &str) -> (&str, u32) {
    match (text.strip_prefix("0x"), text.strip_prefix("0b")) {
        (Some(hex), _) => (hex, 16),
        (_, Some(binary)) => (binary, 2),
        _ => (text, 10),
    }
}

/// Whether the text is `INT` or `BASED`, without a sign: `0x_1`, `0X10`, `0b2` and `1.5` are not.
pub fn is_integer(text: &str) -> bool {
    let (digits, radix) = radix_of(text);
    digits.chars().next().is_some_and(|c| c.is_digit(radix)) && digits.chars().all(|c| c == '_' || c.is_digit(radix))
}

/// Whether the digits of an integer are `INT`, and not `BASED`: what stays decimal (the `code` of a `handle`, the numbers
/// of the sections, spec 3.4) refuses the other.
pub fn is_decimal(text: &str) -> bool {
    radix_of(text).1 == 10
}

/// The value of an integer without its sign; `None` beyond `u128`, or for a text that is not [`is_integer`]. One pass,
/// which stops at the first digit too many: a literal can be 4 MiB long.
pub fn magnitude(text: &str) -> Option<u128> {
    let (digits, radix) = radix_of(text);
    if !digits.chars().next().is_some_and(|c| c.is_digit(radix)) {
        return None;
    }
    let mut value: u128 = 0;
    for c in digits.chars().filter(|c| *c != '_') {
        value = value.checked_mul(u128::from(radix))?.checked_add(u128::from(c.to_digit(radix)?))?;
    }
    Some(value)
}

/// The value of an integer with its sign; `None` beyond `i128`, or for a text that is not [`is_integer`].
pub fn value(negative: bool, text: &str) -> Option<i128> {
    let magnitude = i128::try_from(magnitude(text)?).ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Place values by hand: 0xFF00 = 15·16³ + 15·16² = 65,280; 0b1010_0001 = 128 + 32 + 1 = 161.
    #[test]
    fn an_integer_reads_in_its_base() {
        assert_eq!(magnitude("0xFF00"), Some(65_280));
        assert_eq!(magnitude("0xff00"), Some(65_280));
        assert_eq!(magnitude("0b1010_0001"), Some(161));
        assert_eq!(magnitude("1_000"), Some(1000));
        assert_eq!(magnitude("007"), Some(7));
        assert_eq!(value(true, "0x80"), Some(-128));
        assert_eq!(magnitude(&format!("0x{}", "F".repeat(32))), Some(u128::MAX));
        assert_eq!(magnitude(&format!("0x1{}", "0".repeat(32))), None, "beyond `u128`");
        assert_eq!(value(false, &format!("0x{}", "F".repeat(32))), None, "beyond `i128`");
    }

    /// Spec 14.1: a digit of the base right after the prefix; the prefix in lower case.
    #[test]
    fn what_the_grammar_does_not_give_is_no_integer() {
        for text in ["", "_1", "0x", "0b", "0x_1", "0b__1", "0X10", "0B1", "0b102", "0xFFg", "1.5", "1e3", "-1"] {
            assert!(!is_integer(text), "{text}");
            assert_eq!(magnitude(text), None, "{text}");
        }
        for text in ["0", "1_", "0x0", "0xA_b", "0b1_", "0b0"] {
            assert!(is_integer(text), "{text}");
        }
        assert!(is_decimal("1_000") && !is_decimal("0x10") && !is_decimal("0b1"));
    }
}
