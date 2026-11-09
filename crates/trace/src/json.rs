//! A small JSON scanner for the lines of a trace: it reads what the reader asks for, and skips the rest while checking
//! it, without recursion and within a bound of nesting, so that a hostile line costs its length and no stack.

use std::borrow::Cow;

/// A line that does not read: not JSON, too deep, or a field of the wrong kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bad;

pub struct Scanner<'a> {
    text: &'a str,
    pos: usize,
    depth: usize,
    max_depth: usize,
}

/// An unsigned integer; one past `u64::MAX` reads as `u64::MAX`, which is beyond every bound it is checked against.
pub type Uint = u64;

impl<'a> Scanner<'a> {
    pub fn new(text: &'a str, max_depth: usize) -> Self {
        Self { text, pos: 0, depth: 0, max_depth }
    }

    fn bytes(&self) -> &'a [u8] {
        self.text.as_bytes()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes().get(self.pos).copied()
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.ws();
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), Bad> {
        if self.eat(byte) { Ok(()) } else { Err(Bad) }
    }

    fn deeper(&mut self) -> Result<(), Bad> {
        self.depth += 1;
        if self.depth > self.max_depth { Err(Bad) } else { Ok(()) }
    }

    /// Only white space is left.
    pub fn end(&mut self) -> Result<(), Bad> {
        self.ws();
        if self.pos == self.text.len() { Ok(()) } else { Err(Bad) }
    }

    /// The next value is an object: its `{`.
    pub fn begin_object(&mut self) -> Result<(), Bad> {
        self.expect(b'{')?;
        self.deeper()
    }

    /// The next key of the object begun last, or `None` at its end.
    pub fn key(&mut self, first: &mut bool) -> Result<Option<Cow<'a, str>>, Bad> {
        if self.eat(b'}') {
            self.depth = self.depth.saturating_sub(1);
            return Ok(None);
        }
        if !std::mem::replace(first, false) {
            self.expect(b',')?;
        }
        self.ws();
        let key = self.string()?;
        self.expect(b':')?;
        Ok(Some(key))
    }

    pub fn begin_array(&mut self) -> Result<(), Bad> {
        self.expect(b'[')?;
        self.deeper()
    }

    /// Whether the array begun last has another element; after `false`, it is closed.
    pub fn element(&mut self, first: &mut bool) -> Result<bool, Bad> {
        if self.eat(b']') {
            self.depth = self.depth.saturating_sub(1);
            return Ok(false);
        }
        if !std::mem::replace(first, false) {
            self.expect(b',')?;
        }
        Ok(true)
    }

    /// A string, with its escapes read.
    pub fn string(&mut self) -> Result<Cow<'a, str>, Bad> {
        self.ws();
        let (start, end, escaped) = self.string_span()?;
        let raw = self.text.get(start..end).ok_or(Bad)?;
        if !escaped {
            return Ok(Cow::Borrowed(raw));
        }
        unescape(raw).map(Cow::Owned)
    }

    /// The span of the contents of a string, between its quotes, and whether it holds escapes.
    fn string_span(&mut self) -> Result<(usize, usize, bool), Bad> {
        if self.peek() != Some(b'"') {
            return Err(Bad);
        }
        self.pos += 1;
        let start = self.pos;
        let mut escaped = false;
        loop {
            match self.peek().ok_or(Bad)? {
                b'"' => {
                    let end = self.pos;
                    self.pos += 1;
                    return Ok((start, end, escaped));
                }
                b'\\' => {
                    escaped = true;
                    self.pos += 1;
                    match self.peek().ok_or(Bad)? {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.pos += 1,
                        b'u' => {
                            let hex = self.bytes().get(self.pos + 1..self.pos + 5).ok_or(Bad)?;
                            if !hex.iter().all(u8::is_ascii_hexdigit) {
                                return Err(Bad);
                            }
                            self.pos += 5;
                        }
                        _ => return Err(Bad),
                    }
                }
                byte if byte < 0x20 => return Err(Bad),
                _ => self.pos += 1,
            }
        }
    }

    /// A number that counts: digits only, no sign, no fraction, no exponent.
    pub fn uint(&mut self) -> Result<Uint, Bad> {
        self.ws();
        let start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
        let digits = self.text.get(start..self.pos).ok_or(Bad)?;
        if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
            return Err(Bad);
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(Bad);
        }
        Ok(digits.parse().unwrap_or(u64::MAX))
    }

    pub fn bool(&mut self) -> Result<bool, Bad> {
        self.ws();
        if self.literal("true") {
            Ok(true)
        } else if self.literal("false") {
            Ok(false)
        } else {
            Err(Bad)
        }
    }

    fn literal(&mut self, word: &str) -> bool {
        let found = self.bytes().get(self.pos..self.pos + word.len()) == Some(word.as_bytes());
        if found {
            self.pos += word.len();
        }
        found
    }

    /// A number of JSON (RFC 8259, 6).
    pub fn number(&mut self) -> Result<(), Bad> {
        self.ws();
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(Bad),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(Bad);
            }
            self.digits();
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(Bad);
            }
            self.digits();
        }
        Ok(())
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    /// Skips one value, checking it, and returns its text as the line gives it.
    pub fn value(&mut self) -> Result<&'a str, Bad> {
        self.ws();
        let start = self.pos;
        // The open containers inside this value: `true` for an object.
        let mut open: Vec<bool> = Vec::new();
        'value: loop {
            self.ws();
            match self.peek().ok_or(Bad)? {
                b'{' => {
                    self.pos += 1;
                    self.deeper()?;
                    if self.eat(b'}') {
                        self.depth = self.depth.saturating_sub(1);
                    } else {
                        open.push(true);
                        self.ws();
                        self.string_span()?;
                        self.expect(b':')?;
                        continue 'value;
                    }
                }
                b'[' => {
                    self.pos += 1;
                    self.deeper()?;
                    if self.eat(b']') {
                        self.depth = self.depth.saturating_sub(1);
                    } else {
                        open.push(false);
                        continue 'value;
                    }
                }
                b'"' => {
                    self.string_span()?;
                }
                b't' | b'f' => {
                    self.bool()?;
                }
                b'n' => {
                    if !self.literal("null") {
                        return Err(Bad);
                    }
                }
                _ => self.number()?,
            }
            // After a value: the next element or member, or the ends of what holds it.
            loop {
                let Some(&object) = open.last() else {
                    let end = self.pos;
                    return self.text.get(start..end).ok_or(Bad);
                };
                if self.eat(b',') {
                    if object {
                        self.ws();
                        self.string_span()?;
                        self.expect(b':')?;
                    }
                    continue 'value;
                }
                self.expect(if object { b'}' } else { b']' })?;
                open.pop();
                self.depth = self.depth.saturating_sub(1);
            }
        }
    }
}

/// The text of a string with escapes: a lone surrogate does not read, since no text holds it.
fn unescape(raw: &str) -> Result<String, Bad> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let escaped = match chars.next().ok_or(Bad)? {
            '"' => '"',
            '\\' => '\\',
            '/' => '/',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'u' => {
                let high = hex4(&mut chars)?;
                if (0xD800..0xDC00).contains(&high) {
                    if chars.next() != Some('\\') || chars.next() != Some('u') {
                        return Err(Bad);
                    }
                    let low = hex4(&mut chars)?;
                    if !(0xDC00..0xE000).contains(&low) {
                        return Err(Bad);
                    }
                    char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)).ok_or(Bad)?
                } else {
                    char::from_u32(high).ok_or(Bad)?
                }
            }
            _ => return Err(Bad),
        };
        out.push(escaped);
    }
    Ok(out)
}

fn hex4(chars: &mut std::str::Chars<'_>) -> Result<u32, Bad> {
    let mut value = 0;
    for _ in 0..4 {
        value = value * 16 + chars.next().and_then(|c| c.to_digit(16)).ok_or(Bad)?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip(text: &str, depth: usize) -> Result<&str, Bad> {
        let mut scanner = Scanner::new(text, depth);
        let value = scanner.value()?;
        scanner.end()?;
        Ok(value)
    }

    #[test]
    fn values_are_skipped_whole_and_checked() {
        for text in [r#"{"a":[1,-2.5e-3,true,null,"x\n"],"b":{}}"#, "[]", "0", r#""\ud83d\ude00""#, " [ [ ] , { } ] "] {
            assert_eq!(skip(text, 256).map(str::trim), Ok(text.trim()), "{text}");
        }
        for text in ["[1,]", "{\"a\"}", "01", "1.", "-", "[1 2]", "{\"a\":1,}", "\"\u{1}\"", "nul", "\"\\x\"", "[", "1 1"] {
            assert_eq!(skip(text, 256), Err(Bad), "{text}");
        }
    }

    #[test]
    fn nesting_is_bounded_without_recursion() {
        let deep = |levels: usize| format!("{}{}", "[".repeat(levels), "]".repeat(levels));
        assert!(skip(&deep(256), 256).is_ok());
        assert_eq!(skip(&deep(257), 256), Err(Bad));
        // Far deeper than any stack would take, if this recursed.
        assert_eq!(skip(&deep(1_000_000), 256), Err(Bad));
    }

    #[test]
    fn strings_read_their_escapes() {
        let mut scanner = Scanner::new(r#""a\"b\\c\u00e9\ud83d\ude00\n""#, 4);
        assert_eq!(scanner.string().as_deref(), Ok("a\"b\\c\u{e9}\u{1F600}\n"));
        assert_eq!(Scanner::new(r#""\ud83d""#, 4).string(), Err(Bad), "a lone surrogate");
        assert_eq!(Scanner::new(r#""\ude00x""#, 4).string(), Err(Bad));
    }

    #[test]
    fn counts_are_digits_only() {
        assert_eq!(Scanner::new("12", 1).uint(), Ok(12));
        assert_eq!(Scanner::new("99999999999999999999999", 1).uint(), Ok(u64::MAX));
        for text in ["-1", "1.5", "1e3", "01", "x"] {
            assert_eq!(Scanner::new(text, 1).uint(), Err(Bad), "{text}");
        }
    }
}
