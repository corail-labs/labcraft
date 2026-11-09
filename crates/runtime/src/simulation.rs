//! The simulation file of a run (cli.md 6.2): TOML, one table per library that simulates, by its name. The tool reads it,
//! judges it as a file, and gives each library its table, with the place of each key, so that a diagnostic of the library
//! points at its key (simulation.md 7). What the file is as a whole has a code of the tool: nested too deep (`LC0007`), not
//! TOML (`LC8109`), a key at its top level that no library claims (`LC8110`).
//!
//! The parser of `toml` recurses on what a file nests, and stops at 80 levels: a simulation file nests at most that, or
//! `--max-nesting` when it is lower (decision of the owner, 2026-11-09), measured first on the tokens of its lexer, without
//! recursion; then each integer is checked to fit in 64 bits, with an explicit stack. A position is found in logarithmic time,
//! so that a file of many problems is placed in linear time.

use std::ops::Range;

use labcraft_types::{Position, Severity, Span};
use toml::Spanned;
use toml::de::{DeTable, DeValue};
use toml_parser::Source;
use toml_parser::lexer::TokenKind;

use crate::native::{LibraryDiagnostic, NativeLibrary, Section, Simulation};

/// The codes of the tool that reading a simulation file gives (cli.md 5 and 6.6).
pub const NESTED_TOO_DEEP: &str = "LC0007";
pub const NOT_TOML: &str = "LC8109";
pub const NO_LIBRARY: &str = "LC8110";
pub const CODES: &[&str] = &[NESTED_TOO_DEEP, NOT_TOML, NO_LIBRARY];

/// How deep the reader of `toml` goes: past it, it stops, so no `--max-nesting` can take a simulation file further.
pub const TOML_NESTING: u32 = 80;

/// A simulation file read as TOML: its text, its tables, and where each byte of it is.
pub struct SimulationFile<'a> {
    text: &'a str,
    root: Spanned<DeTable<'a>>,
    places: Places<'a>,
}

fn tool(code: &'static str, message: String, span: Option<Span>) -> LibraryDiagnostic {
    LibraryDiagnostic { code, severity: Severity::Error, message, span, fix: None }
}

/// How many bytes apart the marks of [`Places`] are.
const MARK: usize = 1024;

/// Where each byte of a text is, in lines and columns from 1, columns in code points of the file as read, a byte order mark
/// excluded (cli.md 4.2): the start of each line, and every 1,024 bytes the code points before that place, so that a position
/// is a binary search and a count of at most 1,024 bytes, never a scan of the text.
pub struct Places<'a> {
    text: &'a str,
    lines: Vec<usize>,
    /// A byte that starts a character, and how many characters come before it.
    marks: Vec<(usize, usize)>,
}

impl<'a> Places<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut lines = vec![0];
        let mut marks = vec![(0, 0)];
        let mut next = MARK;
        for (count, (byte, c)) in text.char_indices().enumerate() {
            if byte >= next {
                marks.push((byte, count));
                next = byte.saturating_add(MARK);
            }
            if c == '\n' {
                lines.push(byte + 1);
            }
        }
        Places { text, lines, marks }
    }

    /// The characters before `byte`, a byte order mark included.
    fn chars_before(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        let mark = self.marks.partition_point(|(at, _)| *at <= byte).saturating_sub(1);
        let (at, count) = self.marks.get(mark).copied().unwrap_or((0, 0));
        count + self.text.get(at..byte).map_or(0, |part| part.chars().count())
    }

    pub fn position(&self, byte: usize) -> Position {
        let line = self.lines.partition_point(|start| *start <= byte).max(1);
        let start = self.lines.get(line - 1).copied().unwrap_or(0);
        let mut column = self.chars_before(byte).saturating_sub(self.chars_before(start));
        if start == 0 && self.text.starts_with('\u{feff}') && byte > 0 {
            column = column.saturating_sub(1);
        }
        Position { line: u32::try_from(line).unwrap_or(u32::MAX), column: u32::try_from(column + 1).unwrap_or(u32::MAX) }
    }

    pub fn span(&self, range: Range<usize>) -> Span {
        Span { start: self.position(range.start), end: self.position(range.end) }
    }
}

/// The span of a range of a text, placed once: for a few positions; a reader of many uses [`Places`].
pub fn span_of(text: &str, range: Range<usize>) -> Span {
    Places::new(text).span(range)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Array,
    Table,
}

/// How deep the file nests at most: the tables and arrays from its root to its deepest value, counted on its tokens along
/// the path (decision of the owner, 2026-11-09: 80 levels at most). A table header of n segments is n tables; a key of n
/// segments is n - 1 tables around its value; an array or an inline table is one more, after the tables of the key that leads
/// to it. Along the path the levels add up, as they do in the tree that the reader builds, and that the drop of the tree walks:
/// 80 inline tables each led by a key of 80 segments are 6,400 levels, refused.
pub fn nesting(text: &str) -> usize {
    let source = Source::new(text);
    let mut header = 0_usize;
    let mut in_header: Option<usize> = None;
    // The levels of each container open, and their sum.
    let mut open: Vec<(Open, usize)> = Vec::new();
    let mut levels = 0_usize;
    // The segments of the key being written, and of the key that leads to the value being written.
    let mut segments = 0_usize;
    let mut lead = 0_usize;
    let mut in_key = true;
    let mut deepest = 0_usize;
    for token in source.lex() {
        match token.kind() {
            TokenKind::Whitespace | TokenKind::Comment => continue,
            TokenKind::Newline => {
                if open.is_empty() {
                    in_key = true;
                    segments = 0;
                    lead = 0;
                }
            }
            TokenKind::LeftSquareBracket => {
                if in_header.is_some() {
                    // The second bracket of `[[`.
                } else if open.is_empty() && in_key && segments == 0 {
                    in_header = Some(0);
                } else {
                    let weight = lead.saturating_sub(1) + 1;
                    levels += weight;
                    open.push((Open::Array, weight));
                    lead = 0;
                    in_key = false;
                }
            }
            TokenKind::RightSquareBracket => {
                match in_header.take() {
                    Some(parts) => header = parts,
                    None => {
                        if let Some((_, weight)) = open.pop() {
                            levels -= weight;
                        }
                    }
                }
                in_key = false;
            }
            TokenKind::LeftCurlyBracket => {
                let weight = lead.saturating_sub(1) + 1;
                levels += weight;
                open.push((Open::Table, weight));
                lead = 0;
                segments = 0;
                in_key = true;
            }
            TokenKind::RightCurlyBracket => {
                if let Some((_, weight)) = open.pop() {
                    levels -= weight;
                }
                lead = 0;
                in_key = false;
            }
            TokenKind::Comma => {
                segments = 0;
                lead = 0;
                in_key = matches!(open.last(), Some((Open::Table, _)));
            }
            TokenKind::Equals => {
                lead = segments;
                segments = 0;
                in_key = false;
            }
            TokenKind::Dot => {}
            // A part of a key or a header: a bare key, or a quoted one.
            _ if in_header.is_some() || in_key => match in_header.as_mut() {
                Some(parts) => *parts += 1,
                None => segments += 1,
            },
            _ => {}
        }
        let key = lead.max(segments).saturating_sub(1);
        deepest = deepest.max(header + levels + key + in_header.unwrap_or(0));
    }
    deepest
}

/// Each integer of the file fits in 64 bits, as TOML requires (simulation.md 3): one that does not makes the file not TOML.
fn integers_fit<'a>(root: &'a Spanned<DeTable<'a>>, places: &Places<'_>) -> Result<(), LibraryDiagnostic> {
    let mut pending: Vec<&Spanned<DeValue<'a>>> = root.get_ref().values().collect();
    while let Some(value) = pending.pop() {
        match value.get_ref() {
            DeValue::Integer(integer) => {
                if integer_of(integer).is_none() {
                    let message = "an integer that does not fit in 64 bits, as TOML requires".to_owned();
                    return Err(tool(NOT_TOML, message, Some(places.span(value.span()))));
                }
            }
            DeValue::Array(items) => pending.extend(items.iter()),
            DeValue::Table(table) => pending.extend(table.values()),
            _ => {}
        }
    }
    Ok(())
}

/// The value of a TOML integer, in any of its forms (`1_000`, `0x1F`, `0o17`, `0b101`), if it fits in 64 bits.
pub fn integer_of(integer: &toml::de::DeInteger<'_>) -> Option<i64> {
    i64::from_str_radix(integer.as_str(), integer.radix()).ok()
}

/// The value of a TOML float, `inf` and `nan` included; one too large for an `f64` is infinite, which a library refuses.
pub fn float_of(float: &toml::de::DeFloat<'_>) -> Option<f64> {
    float.as_str().parse().ok()
}

/// Reads a simulation file as TOML: `LC0007` past `max_nesting`, `LC8109` when it is not TOML (syntax, a key given twice, an
/// integer that does not fit in 64 bits). Its size and its encoding are judged when it is read from disk (cli.md 6.2).
pub fn read(text: &str, max_nesting: u32) -> Result<SimulationFile<'_>, LibraryDiagnostic> {
    let limit = max_nesting.min(TOML_NESTING);
    let too_deep = |depth: String| {
        let message = format!("the simulation file nests {depth} levels deep, beyond {limit} (--max-nesting, and at most {TOML_NESTING} for TOML)");
        tool(NESTED_TOO_DEEP, message, None)
    };
    let depth = nesting(text);
    if depth > limit as usize {
        return Err(too_deep(depth.to_string()));
    }
    let places = Places::new(text);
    let root = DeTable::parse(text).map_err(|error| {
        // Should the count above differ from the reader's somewhere, its own limit is still said as a nesting.
        if error.message().contains("recurs") {
            return too_deep(format!("more than {TOML_NESTING}"));
        }
        let span = error.span().map(|range| places.span(range));
        tool(NOT_TOML, format!("the simulation file is not TOML: {}", error.message()), span)
    })?;
    integers_fit(&root, &places)?;
    Ok(SimulationFile { text, root, places })
}

impl<'a> SimulationFile<'a> {
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The table of the library of this name, if the file has one.
    pub fn section(&self, library: &str) -> Option<Section<'_>> {
        self.root.get_ref().iter().find(|(key, _)| key.get_ref().as_ref() == library).map(|(_, value)| Section { value, places: &self.places })
    }

    /// Gives each library that simulates its table, and judges the keys at the top level: one that names no library that
    /// simulates is `LC8110`, with the nearest name. What each library judged is in the order of `libraries`.
    pub fn judge(&self, libraries: &[&dyn NativeLibrary]) -> Result<Vec<Option<Simulation>>, Vec<LibraryDiagnostic>> {
        let mut problems = Vec::new();
        let names: Vec<&str> = libraries.iter().filter(|library| library.simulates()).map(|library| library.name()).collect();
        for (key, _) in self.root.get_ref().iter() {
            let name = key.get_ref().as_ref();
            if !names.contains(&name) {
                let nearest = nearest(name, &names);
                let mut diagnostic = tool(
                    NO_LIBRARY,
                    format!("`{}` names no library that simulates ({})", labcraft_types::cut_text(name), names.join(", ")),
                    Some(self.places.span(key.span())),
                );
                diagnostic.fix = nearest.map(str::to_owned);
                problems.push(diagnostic);
            }
        }
        let mut simulations = Vec::with_capacity(libraries.len());
        for library in libraries {
            let simulation = match self.section(library.name()) {
                Some(section) if library.simulates() => match library.simulate(&section) {
                    Ok(simulation) => Some(simulation),
                    Err(found) => {
                        problems.extend(found);
                        None
                    }
                },
                _ => None,
            };
            simulations.push(simulation);
        }
        if problems.is_empty() { Ok(simulations) } else { Err(problems) }
    }
}

/// The name at the fewest edits, if it is near: at most a third of the longer one.
pub fn nearest<'n>(name: &str, names: &[&'n str]) -> Option<&'n str> {
    let found = names.iter().map(|candidate| (edits(name, candidate), *candidate)).min()?;
    let longest = name.chars().count().max(found.1.chars().count());
    (found.0 * 3 <= longest).then_some(found.1)
}

/// Levenshtein distance, in code points, of two short names (a name longer than 64 code points is far from any).
fn edits(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len() > 64 || b.len() > 64 {
        return usize::MAX / 4;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            let substitute = previous.get(j).copied().unwrap_or(0) + usize::from(x != y);
            let delete = previous.get(j + 1).copied().unwrap_or(0) + 1;
            let insert = current.get(j).copied().unwrap_or(0) + 1;
            current.push(substitute.min(delete).min(insert));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}
