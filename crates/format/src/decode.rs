//! From bytes to text, in the order of `docs/cli.md`, section 1, stopping at the first failure: size,
//! UTF-8, byte order mark removed, CR LF accepted, forbidden characters.

use labcraft_types::{Code, Diagnostic, Limits, Position, Span};

const BYTE_ORDER_MARK: char = '\u{FEFF}';

/// The text of a file, or the diagnostic that says why it has none. CR LF stays in the text: the lexer
/// reads it as a line ending, and columns count what the file holds.
pub fn decode<'a>(bytes: &'a [u8], limits: &Limits) -> Result<&'a str, Box<Diagnostic>> {
    if bytes.len() as u64 > limits.max_file_size {
        let message = format!("file of {} bytes, over the limit of {} bytes", bytes.len(), limits.max_file_size);
        return Err(Box::new(Diagnostic::new(Code::Lc0006, message, "")));
    }
    let text = std::str::from_utf8(bytes).map_err(|error| {
        let valid = bytes.get(..error.valid_up_to()).and_then(|valid| std::str::from_utf8(valid).ok()).unwrap_or_default();
        located(Code::Lc0003, "not valid UTF-8".to_owned(), position_after(valid.strip_prefix(BYTE_ORDER_MARK).unwrap_or(valid)))
    })?;
    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text);
    check_characters(text)?;
    Ok(text)
}

fn check_characters(text: &str) -> Result<(), Box<Diagnostic>> {
    let mut position = Position { line: 1, column: 1 };
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let refused = match c {
            '\n' | '\t' => false,
            '\r' => chars.peek() != Some(&'\n'),
            _ => c.is_control(),
        };
        if refused {
            let what =
                if c == '\r' { "a CR not followed by a line feed".to_owned() } else { format!("the control character U+{:04X}", u32::from(c)) };
            return Err(located(Code::Lc0004, format!("forbidden character: {what}"), position));
        }
        position = advance(position, c);
    }
    Ok(())
}

fn located(code: Code, message: String, start: Position) -> Box<Diagnostic> {
    let end = Position { line: start.line, column: start.column.saturating_add(1) };
    Box::new(Diagnostic::new(code, message, "").with_span(Span { start, end }))
}

pub(crate) fn advance(position: Position, c: char) -> Position {
    if c == '\n' {
        Position { line: position.line.saturating_add(1), column: 1 }
    } else {
        Position { line: position.line, column: position.column.saturating_add(1) }
    }
}

fn position_after(text: &str) -> Position {
    text.chars().fold(Position { line: 1, column: 1 }, advance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(bytes: &[u8]) -> Option<Code> {
        decode(bytes, &Limits::default()).err().map(|diagnostic| diagnostic.code())
    }

    fn start_of(bytes: &[u8]) -> (u32, u32) {
        let start = decode(bytes, &Limits::default()).unwrap_err().span.unwrap().start;
        (start.line, start.column)
    }

    #[test]
    fn good_text_stays_as_it_is() {
        for text in ["", "a", "a\nb\n", "a\tb", "a\r\nb\r\n", "\u{00E9}\u{202E}\n"] {
            assert_eq!(decode(text.as_bytes(), &Limits::default()), Ok(text));
        }
    }

    #[test]
    fn a_byte_order_mark_at_the_start_is_removed() {
        assert_eq!(decode("\u{FEFF}a\n".as_bytes(), &Limits::default()), Ok("a\n"));
        assert_eq!(decode("\u{FEFF}".as_bytes(), &Limits::default()), Ok(""));
        assert_eq!(decode("a\u{FEFF}".as_bytes(), &Limits::default()), Ok("a\u{FEFF}"), "elsewhere it stays, for the lexer to refuse");
    }

    #[test]
    fn invalid_utf8_is_refused_where_it_is() {
        assert_eq!(code_of(b"\xFF"), Some(Code::Lc0003));
        assert_eq!(code_of(b"ab\xC3"), Some(Code::Lc0003), "cut inside a character");
        assert_eq!(start_of(b"ab\n\xC3\xA9c\xFF"), (2, 3));
        assert_eq!(start_of(b"\xEF\xBB\xBFab\xFF"), (1, 3), "a byte order mark takes no column");
    }

    #[test]
    fn control_characters_are_refused_where_they_are() {
        for c in ['\0', '\u{7}', '\u{B}', '\u{C}', '\u{1B}', '\u{7F}', '\u{85}'] {
            assert_eq!(code_of(format!("a{c}").as_bytes()), Some(Code::Lc0004), "{c:?}");
        }
        assert_eq!(start_of(b"ab\ncd\x00"), (2, 3));
        assert_eq!(start_of("\u{00E9}\u{00E9}\0".as_bytes()), (1, 3), "columns count code points");
    }

    #[test]
    fn a_lone_cr_is_refused() {
        for text in ["a\r", "a\rb", "\r", "\r\r\n"] {
            assert_eq!(code_of(text.as_bytes()), Some(Code::Lc0004), "{text:?}");
        }
        assert_eq!(start_of(b"a\r\nb\rc"), (2, 2));
    }

    #[test]
    fn the_size_comes_first() {
        let mut limits = Limits::default();
        limits.max_file_size = 3;
        assert_eq!(decode(b"abc", &limits), Ok("abc"));
        let diagnostic = decode(b"\xFF\xFF\xFF\xFF", &limits).unwrap_err();
        assert_eq!(diagnostic.code(), Code::Lc0006);
        assert_eq!(diagnostic.span, None, "it concerns the file as a whole");
    }

    #[test]
    fn utf8_comes_before_characters() {
        assert_eq!(code_of(b"\x00\xFF"), Some(Code::Lc0003));
    }
}
