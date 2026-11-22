//! Reading and editing a JSON file in place, with the comments and the trailing commas that VS Code, Zed and OpenCode
//! accept: only the bytes of what changes are written, so that what else the file holds stays as it was (mcp.md 9).

use std::ops::Range;

use serde::Serialize;
use serde_json::Value;

/// The deepest nesting read *(proposed)*: a configuration is shallow.
const MAX_DEPTH: usize = 128;

/// A member of an object in the text: its key, where its key starts, and the bytes of its value.
#[derive(Debug)]
pub struct Member {
    pub key: String,
    pub key_at: usize,
    pub value: Range<usize>,
}

/// An object in the text: its braces and its members, in order.
#[derive(Debug)]
pub struct Object {
    pub open: usize,
    pub close: usize,
    pub members: Vec<Member>,
}

impl Object {
    /// The member of that key; the last one when the key is written twice, as a reader of JSON takes it.
    pub fn get(&self, key: &str) -> Option<&Member> {
        self.members.iter().rev().find(|member| member.key == key)
    }
}

/// The object at the top of the text, which must hold nothing else. An empty text is not read: the caller writes one.
pub fn root(text: &str) -> Result<Object, String> {
    let bytes = text.as_bytes();
    let start = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let open = skip(bytes, start)?;
    if bytes.get(open) != Some(&b'{') {
        return Err("its top level is not a JSON object".to_owned());
    }
    let object = object_at(text, open)?;
    if skip(bytes, object.close + 1)? != bytes.len() {
        return Err("it holds something after its top-level object".to_owned());
    }
    Ok(object)
}

/// The object whose `{` is at `open`.
pub fn object_at(text: &str, open: usize) -> Result<Object, String> {
    object(text.as_bytes(), open, 0)
}

/// The object that is the value of the member, if it is one (and whether it reads).
pub fn object_of(text: &str, member: &Member) -> Option<Result<Object, String>> {
    (text.as_bytes().get(member.value.start) == Some(&b'{')).then(|| object_at(text, member.value.start))
}

/// The bytes of each item of the array whose `[` is at `open`.
pub fn items_at(text: &str, open: usize) -> Result<Vec<Range<usize>>, String> {
    array(text.as_bytes(), open, 0).map(|(items, _)| items)
}

/// The string at those bytes, when they hold one.
pub fn string(text: &str, range: Range<usize>) -> Option<String> {
    serde_json::from_str(text.get(range)?).ok()
}

/// The strings of the array at those bytes, when it holds only strings.
pub fn strings(text: &str, range: Range<usize>) -> Option<Vec<String>> {
    if text.as_bytes().get(range.start) != Some(&b'[') {
        return None;
    }
    items_at(text, range.start).ok()?.into_iter().map(|item| string(text, item)).collect()
}

/// Adds `"key": value` as the last member of the object: on a line of its own, indented as the members are, when the
/// object holds its members on lines of their own; else on its line. A comma goes after the member before it, unless
/// one is there.
pub fn insert(text: &mut String, object: &Object, key: &str, value: &Value) {
    let newline = newline(text);
    let unit = unit(text);
    let key = serde_json::to_string(key).unwrap_or_default();
    let line_of_close = line_start(text, object.close);
    let close_alone = text.get(line_of_close..object.close).is_some_and(|before| before.trim().is_empty());
    let Some(last) = object.members.last() else {
        let base = indent_of(text, object.open);
        let inner = format!("{base}{unit}");
        let member = format!("{key}: {}", pretty(value, &inner, &unit, newline));
        let empty = text.get(object.open + 1..object.close).is_some_and(|inside| inside.trim().is_empty());
        if empty {
            text.replace_range(object.open + 1..object.close, &format!("{newline}{inner}{member}{newline}{base}"));
        } else {
            // A comment inside: it stays, after the member.
            text.insert_str(object.open + 1, &format!("{newline}{inner}{member}"));
        }
        return;
    };
    let comma = skip(text.as_bytes(), last.value.end).ok().is_some_and(|after| text.as_bytes().get(after) == Some(&b','));
    if close_alone && line_start(text, last.key_at) != line_start(text, object.open) {
        let indent = indent_of(text, last.key_at);
        text.insert_str(line_of_close, &format!("{indent}{key}: {}{newline}", pretty(value, &indent, &unit, newline)));
    } else {
        let compact = serde_json::to_string(value).unwrap_or_default();
        // The comma, when there is none, goes after the member before, below.
        text.insert_str(object.close, &format!(" {key}: {compact}"));
    }
    if !comma {
        text.insert(last.value.end, ',');
    }
}

/// Puts `value` in place of the bytes of `range`.
pub fn replace(text: &mut String, range: Range<usize>, value: &Value) {
    text.replace_range(range, &serde_json::to_string(value).unwrap_or_default());
}

/// The new line the file uses.
fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// One step of indentation, as the members of the top-level object have it; two spaces by default.
fn unit(text: &str) -> String {
    root(text)
        .ok()
        .and_then(|object| object.members.first().map(|member| member.key_at))
        .filter(|&at| line_start(text, at) > 0)
        .map(|at| indent_of(text, at))
        .filter(|indent| !indent.is_empty())
        .unwrap_or_else(|| "  ".to_owned())
}

/// `value` as the file would write it: indented `unit` a level, its lines after the first at `indent`.
fn pretty(value: &Value, indent: &str, unit: &str, newline: &str) -> String {
    let mut written = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut written, serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes()));
    if value.serialize(&mut serializer).is_err() {
        return String::new();
    }
    String::from_utf8(written).unwrap_or_default().replace('\n', &format!("{newline}{indent}"))
}

/// Where the line that holds byte `at` starts.
fn line_start(text: &str, at: usize) -> usize {
    text.get(..at).and_then(|before| before.rfind('\n')).map_or(0, |newline| newline + 1)
}

/// The spaces and tabs that start the line holding byte `at`.
fn indent_of(text: &str, at: usize) -> String {
    text.get(line_start(text, at)..).unwrap_or_default().chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The first byte from `at` that is neither the white space of JSON nor in a comment.
fn skip(bytes: &[u8], mut at: usize) -> Result<usize, String> {
    loop {
        match (bytes.get(at), bytes.get(at + 1)) {
            (Some(b' ' | b'\t' | b'\n' | b'\r'), _) => at += 1,
            (Some(b'/'), Some(b'/')) => {
                while bytes.get(at).is_some_and(|&byte| byte != b'\n') {
                    at += 1;
                }
            }
            (Some(b'/'), Some(b'*')) => {
                at += 2;
                loop {
                    match (bytes.get(at), bytes.get(at + 1)) {
                        (Some(b'*'), Some(b'/')) => break,
                        (None, _) => return Err("a comment is not closed".to_owned()),
                        _ => at += 1,
                    }
                }
                at += 2;
            }
            _ => return Ok(at),
        }
    }
}

/// The byte after the string that starts at `at`.
fn string_end(bytes: &[u8], mut at: usize) -> Result<usize, String> {
    at += 1;
    loop {
        match bytes.get(at) {
            Some(b'\\') => at += 2,
            Some(b'"') => return Ok(at + 1),
            Some(_) => at += 1,
            None => return Err("a string is not closed".to_owned()),
        }
    }
}

/// The byte after the number of JSON that starts at `at`: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
fn number_end(bytes: &[u8], at: usize) -> Option<usize> {
    let digits = |from: usize| {
        let mut end = from;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        (end > from).then_some(end)
    };
    let mut end = if bytes.get(at) == Some(&b'-') { at + 1 } else { at };
    end = if bytes.get(end) == Some(&b'0') { end + 1 } else { digits(end)? };
    if bytes.get(end) == Some(&b'.') {
        end = digits(end + 1)?;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        end += 1;
        if matches!(bytes.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        end = digits(end)?;
    }
    Some(end)
}

/// The byte after the value that starts at `at`.
fn value_end(bytes: &[u8], at: usize, depth: usize) -> Result<usize, String> {
    if depth > MAX_DEPTH {
        return Err(format!("it nests deeper than {MAX_DEPTH} levels"));
    }
    let word = |word: &[u8]| bytes.get(at..at + word.len()) == Some(word);
    let end = match bytes.get(at) {
        Some(b'"') => return string_end(bytes, at),
        Some(b'{') => return Ok(object(bytes, at, depth)?.close + 1),
        Some(b'[') => return Ok(array(bytes, at, depth)?.1),
        Some(b't') if word(b"true") => Some(at + 4),
        Some(b'f') if word(b"false") => Some(at + 5),
        Some(b'n') if word(b"null") => Some(at + 4),
        Some(b'-' | b'0'..=b'9') => number_end(bytes, at),
        _ => None,
    };
    // A value ends where white space, a comment or a delimiter starts: `truex` and `1a` are not values.
    match end {
        Some(end) if bytes.get(end).is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b',' | b'}' | b']' | b'/')) => Ok(end),
        _ => Err(format!("a value is expected at byte {at}")),
    }
}

/// The items of the array whose `[` is at `open`, and the byte after its `]`. A comma separates two items, and one may
/// follow the last.
fn array(bytes: &[u8], open: usize, depth: usize) -> Result<(Vec<Range<usize>>, usize), String> {
    let mut items = Vec::new();
    let mut at = skip(bytes, open + 1)?;
    loop {
        if bytes.get(at) == Some(&b']') {
            return Ok((items, at + 1));
        }
        let end = value_end(bytes, at, depth + 1)?;
        items.push(at..end);
        at = skip(bytes, end)?;
        match bytes.get(at) {
            Some(b']') => return Ok((items, at + 1)),
            Some(b',') => at = skip(bytes, at + 1)?,
            Some(_) => return Err(format!("a `,` or `]` is expected at byte {at}")),
            None => return Err("an array is not closed".to_owned()),
        }
    }
}

/// The object whose `{` is at `open`. A comma separates two members, and one may follow the last.
fn object(bytes: &[u8], open: usize, depth: usize) -> Result<Object, String> {
    if depth > MAX_DEPTH {
        return Err(format!("it nests deeper than {MAX_DEPTH} levels"));
    }
    let mut members = Vec::new();
    let mut at = skip(bytes, open + 1)?;
    loop {
        match bytes.get(at) {
            Some(b'}') => return Ok(Object { open, close: at, members }),
            Some(b'"') => {}
            Some(_) => return Err(format!("a key or `}}` is expected at byte {at}")),
            None => return Err("an object is not closed".to_owned()),
        }
        let end = string_end(bytes, at)?;
        let key = bytes.get(at..end).and_then(|key| serde_json::from_slice(key).ok()).ok_or_else(|| format!("a key at byte {at} does not read"))?;
        let colon = skip(bytes, end)?;
        if bytes.get(colon) != Some(&b':') {
            return Err(format!("a `:` is expected at byte {colon}"));
        }
        let start = skip(bytes, colon + 1)?;
        let end = value_end(bytes, start, depth + 1)?;
        members.push(Member { key, key_at: at, value: start..end });
        at = skip(bytes, end)?;
        match bytes.get(at) {
            Some(b'}') => return Ok(Object { open, close: at, members }),
            Some(b',') => at = skip(bytes, at + 1)?,
            Some(_) => return Err(format!("a `,` or `}}` is expected at byte {at}")),
            None => return Err("an object is not closed".to_owned()),
        }
    }
}
