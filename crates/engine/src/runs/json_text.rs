//! JSON given as text by the command line (`docs/cli.md`, 6.3): the file of `--args` and the value of an `--arg` that needs
//! JSON. Read without recursion, so that the bound is `--max-nesting` itself, counted as a value counts it (a container
//! is a level); a key given twice is refused, at any depth, since JSON leaves it open and `serde_json` keeps the last.
//! Strings and numbers are decoded by `serde_json`, one at a time.

use std::ops::Range;

use serde_json::{Map, Value as Json};

/// What a text of JSON holds, and where the keys of its top object are.
#[derive(Debug)]
pub(crate) struct Parsed {
    pub value: Json,
    /// The keys of the top object, if it is one, each with the bytes of its quoted name.
    pub keys: Vec<(String, Range<usize>)>,
}

/// Why a text is not the JSON it should be, and at which byte.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Bad {
    Syntax(usize),
    TooDeep,
    Twice(String, Range<usize>),
    /// A number of JSON, `1e999`, that no `f64` holds.
    OutOfRange(Range<usize>),
}

enum Open {
    Array(Vec<Json>),
    /// The object, and the key whose value is being read.
    Object(Map<String, Json>, Option<String>),
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, Bad> {
        if self.text.get(self.at..).is_some_and(|rest| rest.starts_with(word)) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(Bad::Syntax(self.at))
        }
    }

    /// A string, its quotes included in the range.
    fn string(&mut self) -> Result<(String, Range<usize>), Bad> {
        let start = self.at;
        let bytes = self.text.as_bytes();
        let mut at = start + 1;
        loop {
            match bytes.get(at) {
                None => return Err(Bad::Syntax(start)),
                Some(b'\\') => at += 2,
                Some(b'"') => break,
                Some(_) => at += 1,
            }
        }
        let end = at + 1;
        let slice = self.text.get(start..end).ok_or(Bad::Syntax(start))?;
        let decoded: String = serde_json::from_str(slice).map_err(|_| Bad::Syntax(start))?;
        self.at = end;
        Ok((decoded, start..end))
    }

    fn number(&mut self) -> Result<Json, Bad> {
        let start = self.at;
        while matches!(self.peek(), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
            self.at += 1;
        }
        let slice = self.text.get(start..self.at).ok_or(Bad::Syntax(start))?;
        match serde_json::from_str::<Json>(slice) {
            Ok(number @ Json::Number(_)) => Ok(number),
            _ if is_json_number(slice) => Err(Bad::OutOfRange(start..self.at)),
            _ => Err(Bad::Syntax(start)),
        }
    }
}

/// The grammar of a JSON number (RFC 8259, 6): `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
fn is_json_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = usize::from(bytes.first() == Some(&b'-'));
    let digits = |at: &mut usize| {
        let from = *at;
        while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        }
        *at > from
    };
    if bytes.get(at) == Some(&b'0') {
        at += 1;
    } else if !digits(&mut at) {
        return false;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        if !digits(&mut at) {
            return false;
        }
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if !digits(&mut at) {
            return false;
        }
    }
    at == bytes.len()
}

/// Reads `text` as one JSON value, nested at most `max_nesting` levels.
pub(crate) fn parse(text: &str, max_nesting: u32) -> Result<Parsed, Bad> {
    let mut reader = Reader { text, at: 0 };
    // A byte order mark is no part of JSON, but an editor may put one at the start of a file.
    if text.starts_with('\u{feff}') {
        reader.at = '\u{feff}'.len_utf8();
    }
    let mut stack: Vec<Open> = Vec::new();
    let mut keys = Vec::new();
    let mut done: Option<Json> = None;
    loop {
        reader.space();
        // A value is wanted: at the start, after `[`, `,` in an array, or `:` in an object.
        let value = match reader.peek() {
            Some(b'{') | Some(b'[') => {
                if stack.len() >= max_nesting as usize {
                    return Err(Bad::TooDeep);
                }
                let object = reader.peek() == Some(b'{');
                reader.at += 1;
                reader.space();
                if object {
                    if reader.peek() == Some(b'}') {
                        reader.at += 1;
                        Some(Json::Object(Map::new()))
                    } else {
                        let key = key(&mut reader, &Map::new(), stack.is_empty(), &mut keys)?;
                        stack.push(Open::Object(Map::new(), Some(key)));
                        None
                    }
                } else if reader.peek() == Some(b']') {
                    reader.at += 1;
                    Some(Json::Array(Vec::new()))
                } else {
                    stack.push(Open::Array(Vec::new()));
                    None
                }
            }
            Some(b'"') => Some(Json::String(reader.string()?.0)),
            Some(b't') => Some(reader.word("true", Json::Bool(true))?),
            Some(b'f') => Some(reader.word("false", Json::Bool(false))?),
            Some(b'n') => Some(reader.word("null", Json::Null)?),
            Some(b'-' | b'0'..=b'9') => Some(reader.number()?),
            _ => return Err(Bad::Syntax(reader.at)),
        };
        let Some(mut value) = value else { continue };
        // The value goes into what is open, and closes what it ends.
        loop {
            match stack.last_mut() {
                None => {
                    done = Some(value);
                    break;
                }
                Some(Open::Array(items)) => {
                    items.push(value);
                    reader.space();
                    match reader.peek() {
                        Some(b',') => {
                            reader.at += 1;
                            break;
                        }
                        Some(b']') => {
                            reader.at += 1;
                            let Some(Open::Array(items)) = stack.pop() else { return Err(Bad::Syntax(reader.at)) };
                            value = Json::Array(items);
                        }
                        _ => return Err(Bad::Syntax(reader.at)),
                    }
                }
                Some(Open::Object(fields, pending)) => {
                    if let Some(key) = pending.take() {
                        fields.insert(key, value);
                    }
                    reader.space();
                    match reader.peek() {
                        Some(b',') => {
                            reader.at += 1;
                            reader.space();
                            let top = stack.len() == 1;
                            let Some(Open::Object(fields, pending)) = stack.last_mut() else { return Err(Bad::Syntax(reader.at)) };
                            *pending = Some(key(&mut reader, fields, top, &mut keys)?);
                            break;
                        }
                        Some(b'}') => {
                            reader.at += 1;
                            let Some(Open::Object(fields, _)) = stack.pop() else { return Err(Bad::Syntax(reader.at)) };
                            value = Json::Object(fields);
                        }
                        _ => return Err(Bad::Syntax(reader.at)),
                    }
                }
            }
        }
        if let Some(value) = done.take() {
            reader.space();
            if reader.at < text.len() {
                return Err(Bad::Syntax(reader.at));
            }
            return Ok(Parsed { value, keys });
        }
    }
}

/// A key and its `:`, refused if the object has it already.
fn key(reader: &mut Reader<'_>, fields: &Map<String, Json>, top: bool, keys: &mut Vec<(String, Range<usize>)>) -> Result<String, Bad> {
    if reader.peek() != Some(b'"') {
        return Err(Bad::Syntax(reader.at));
    }
    let (key, range) = reader.string()?;
    if fields.contains_key(&key) {
        return Err(Bad::Twice(key, range));
    }
    reader.space();
    if reader.peek() != Some(b':') {
        return Err(Bad::Syntax(reader.at));
    }
    reader.at += 1;
    if top {
        keys.push((key.clone(), range));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_reads_what_serde_json_reads() {
        for text in [r#"{"a": [1, 2.5, -3e2], "b": {"c": null, "d": true}, "e": "x\"yé"}"#, "[]", "{}", "0", r#""s""#, "[[[]]]"] {
            assert_eq!(parse(text, 64).unwrap().value, serde_json::from_str::<Json>(text).unwrap(), "{text}");
        }
    }

    #[test]
    fn the_keys_of_the_top_object_are_placed() {
        let text = "{\"resource\": \"SIM::DMM0\",\n \"limits\": {\"low\": \"220 V\"}}";
        let parsed = parse(text, 64).unwrap();
        // By hand: `"resource"` is bytes 1 to 11, `"limits"` starts after `,\n ` at byte 27.
        assert_eq!(parsed.keys, [("resource".to_owned(), 1..11), ("limits".to_owned(), 27..35)]);
        assert_eq!(parsed.value["limits"], json!({"low": "220 V"}));
    }

    #[test]
    fn a_key_given_twice_is_refused_at_any_depth() {
        assert_eq!(parse(r#"{"a": 1, "a": 2}"#, 64).unwrap_err(), Bad::Twice("a".to_owned(), 9..12));
        assert!(matches!(parse(r#"{"x": {"b": 1, "b": 1}}"#, 64), Err(Bad::Twice(..))));
    }

    #[test]
    fn the_bound_is_the_nesting_given_beyond_what_serde_json_takes() {
        let deep = |levels: usize| format!("{}{}", "[".repeat(levels), "]".repeat(levels));
        assert!(parse(&deep(200), 200).is_ok());
        assert_eq!(parse(&deep(201), 200).unwrap_err(), Bad::TooDeep);
        assert_eq!(parse("[1]", 0).unwrap_err(), Bad::TooDeep);
        assert!(parse("1", 0).is_ok());
    }

    #[test]
    fn a_number_that_no_f64_holds_is_out_of_range_not_a_syntax_error() {
        assert_eq!(parse("1e999", 64).unwrap_err(), Bad::OutOfRange(0..5));
        assert_eq!(parse("{\"x\": -1e999}", 64).unwrap_err(), Bad::OutOfRange(6..12));
        assert!(matches!(parse("1e", 64), Err(Bad::Syntax(_))));
    }

    #[test]
    fn what_is_not_json_is_refused() {
        for text in ["", "{", "[1,]", "{\"a\" 1}", "{a: 1}", "tru", "1 2", "[1] x", "'a'", "01x", "\"\u{1}\""] {
            assert!(matches!(parse(text, 64), Err(Bad::Syntax(_))), "{text:?}");
        }
    }
}
