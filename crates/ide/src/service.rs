//! The language service of the `Text` view (ide.md 7; decisions of the owner, 2026-11-21): what `kit-text` asks of the
//! language, made of the lexer of `format` and of what the session knows (architecture 12.7). The diagnostics are those
//! of the check of the session, and the error of a text that does not parse; completion, hover and definitions read the
//! tokens of the text the field holds, so that they follow what is typed between two checks, and take the functions and
//! types from `functions.list` and the types of the values from the drawing of the function.
//!
//! The tokens give the shape of the text, not its meaning: a value is seen where a line binds it, in the braces around
//! it (R-3), which is what a body sees but for a few cases that `check` judges (a name bound twice, an arm that binds a
//! name its structure also binds).

use std::ops::Range;
use std::str::FromStr;
use std::sync::Arc;

use kit_text::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, GotoDefinitionResponse, Hover, HoverContents, Location, MarkupContent,
    MarkupKind, NumberOrString, Position, Uri,
};
use kit_text::{Field, LanguageService, Published, offset_of, position_of};
use labcraft_diagram::Drawing;
use labcraft_engine::view::{Listed, ListedType};
use labcraft_format::{Keyword, Lexer, TokenKind};
use labcraft_types::{Interner, Report, Severity};

use crate::panels::Callable;

/// A path of more segments than this names nothing the service knows *(proposed)*.
const MAX_SEGMENTS: usize = 64;

/// The source of the diagnostics of the check of the session.
pub const CHECKED: u8 = 0;
/// The source of the error of a text that does not parse, which the file does not have.
pub const UNPARSED: u8 = 1;

/// The base types of the language (spec 3.1), offered where a type is written.
const BASE_TYPES: [&str; 19] = [
    "bool",
    "i8",
    "i16",
    "i32",
    "i64",
    "u8",
    "u16",
    "u32",
    "u64",
    "f32",
    "f64",
    "c64",
    "c128",
    "string",
    "bytes",
    "timestamp",
    "duration",
    "error",
    "waveform",
];

/// The words of each place (spec 14.1): reserved words and contextual ones.
const TOP_WORDS: [&str; 9] = ["use", "fn", "extern", "type", "panel", "layout", "terminals", "outline", "icon"];
const BODY_WORDS: [&str; 17] = [
    "match",
    "for",
    "loop",
    "handle",
    "parallel",
    "continue_on_error",
    "carry",
    "next",
    "collect",
    "last",
    "until",
    "yield",
    "true",
    "false",
    "none",
    "nan",
    "inf",
];
const PANEL_WORDS: [&str; 2] = ["control", "indicator"];
const LAYOUT_WORDS: [&str; 11] = ["node", "wire", "at", "via", "view", "standard", "compact", "frame", "size", "input", "output"];
const TERMINALS_WORDS: [&str; 14] =
    ["top", "left", "right", "last", "middle", "input", "output", "band", "hidden", "inside", "above", "color", "fill", "stroke"];
const OUTLINE_WORDS: [&str; 7] = ["path", "round", "template", "terminal", "top", "left", "right"];
const ICON_WORDS: [&str; 11] = ["rect", "circle", "line", "path", "text", "at", "radius", "size", "width", "fill", "accent"];

/// What a token is, as far as the service needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Name,
    Keyword(Keyword),
    /// A string, a comment or a documentation comment: no completion in it.
    Prose,
    Number,
    Newline,
    Dot,
    Comma,
    Colon,
    Equals,
    Arrow,
    FatArrow,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    Hash,
    Less,
    Other,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    /// Characters of the text.
    range: Range<usize>,
    /// A name, normalized (R-20).
    name: String,
}

/// Where a part of the text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Outside every declaration: `use`, `fn`, a section.
    Top,
    /// The signature of a function, from `fn` to its body.
    Signature,
    Body,
    /// A `type` declaration, or a struct or enum type.
    Type,
    Panel,
    Layout,
    Terminals,
    Outline,
    Icon,
}

/// The text between a pair of braces.
#[derive(Clone, Debug)]
struct Region {
    /// From its `{` to its `}`, or to the end of the text when it is not closed.
    range: Range<usize>,
    place: Place,
    /// It ends with its `}`.
    closed: bool,
}

impl Region {
    /// Whether a position is inside its braces.
    fn holds(&self, at: usize) -> bool {
        self.range.start < at && (at < self.range.end || !self.closed && at == self.range.end)
    }
}

/// A name that the text binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    /// Where the name is written.
    pub range: Range<usize>,
    /// The region where it is seen, by its index; none for an input or an output, seen in the whole file.
    region: Option<usize>,
}

/// The shape of a text: its tokens, its regions, the names it binds.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    tokens: Vec<Token>,
    regions: Vec<Region>,
    bindings: Vec<Binding>,
    /// Where the signature of the function is, from `fn` to its body.
    signature: Option<Range<usize>>,
}

impl Scan {
    /// Reads the tokens of `text` with the lexer of the language; what follows a lexical error is not read.
    pub fn new(text: &str) -> Scan {
        let (bom, text) = match text.strip_prefix('\u{FEFF}') {
            Some(rest) => (1, rest),
            None => (0, text),
        };
        let mut starts = vec![bom];
        let mut count = 0;
        for c in text.chars() {
            count += 1;
            if c == '\n' {
                starts.push(count + bom);
            }
        }
        let offset = |line: u32, column: u32| -> Option<usize> {
            let start = *starts.get(usize::try_from(line).ok()?.checked_sub(1)?)?;
            Some(start + usize::try_from(column).ok()?.checked_sub(1)?)
        };
        let mut names = Interner::new();
        let mut report = Report::new(1);
        let mut found = Vec::new();
        for token in Lexer::new(text, &mut names, &mut report) {
            let Ok(token) = token else { break };
            let (Some(start), Some(end)) =
                (offset(token.span.start.line, token.span.start.column), offset(token.span.end.line, token.span.end.column))
            else {
                break;
            };
            let (kind, name) = match token.kind {
                TokenKind::Name(name) | TokenKind::DegreeName(name) => (Kind::Name, Some(name)),
                TokenKind::Keyword(keyword) => (Kind::Keyword(keyword), None),
                _ => (kind_of(&token.kind), None),
            };
            found.push((kind, start..end.max(start), name));
        }
        let tokens: Vec<Token> = found
            .into_iter()
            .map(|(kind, range, name)| Token { kind, range, name: name.map(|name| names.resolve(name).to_owned()).unwrap_or_default() })
            .collect();
        let mut scan = Scan { tokens, ..Scan::default() };
        scan.shape(count + bom);
        scan
    }

    /// The regions and the bindings, in one walk of the tokens.
    fn shape(&mut self, end_of_text: usize) {
        let tokens = &self.tokens;
        let mut stack: Vec<usize> = Vec::new();
        let mut regions: Vec<Region> = Vec::new();
        let mut bindings = Vec::new();
        // Names that the next `{` will see: a carry, a `for` clause, the values of a pattern before `=> {`.
        let mut waiting: Vec<(String, Range<usize>)> = Vec::new();
        // The place the next `{` opens, at the top.
        let mut next_place = Place::Body;
        let mut signature: Option<(usize, u32)> = None;
        let mut signature_range = None;
        // An external function has no body (spec 4.1): its `fn` opens no signature that a `{` would end.
        let mut external = false;
        let mut line_start = true;
        let mut index = 0;
        while let Some(token) = tokens.get(index) {
            let place = stack.last().and_then(|&region| regions.get(region)).map_or(Place::Top, |region: &Region| region.place);
            match token.kind {
                Kind::Keyword(Keyword::Extern) if stack.is_empty() => external = true,
                Kind::Keyword(Keyword::Fn) if stack.is_empty() && !external && signature.is_none() && signature_range.is_none() => {
                    signature = Some((token.range.start, 0));
                }
                Kind::LeftParen if signature.is_some() && stack.is_empty() => {
                    if let Some((_, depth)) = signature.as_mut() {
                        *depth += 1;
                    }
                }
                Kind::RightParen if signature.is_some() && stack.is_empty() => {
                    if let Some((_, depth)) = signature.as_mut() {
                        *depth = depth.saturating_sub(1);
                    }
                }
                // An input or an output: a name at the first level of the parentheses of the signature, before `:`.
                Kind::Name
                    if signature.is_some_and(|(_, depth)| depth == 1)
                        && stack.is_empty()
                        && tokens.get(index + 1).is_some_and(|next| next.kind == Kind::Colon) =>
                {
                    bindings.push(Binding { name: token.name.clone(), range: token.range.clone(), region: None });
                }
                Kind::Keyword(Keyword::Type) if stack.is_empty() => next_place = Place::Type,
                Kind::Keyword(Keyword::Panel) if stack.is_empty() => next_place = Place::Panel,
                Kind::Keyword(Keyword::Layout) if stack.is_empty() => next_place = Place::Layout,
                Kind::Keyword(Keyword::Icon) if stack.is_empty() => next_place = Place::Icon,
                Kind::Name if stack.is_empty() && line_start && token.name == "terminals" => next_place = Place::Terminals,
                Kind::Name if stack.is_empty() && line_start && token.name == "outline" => next_place = Place::Outline,
                Kind::Keyword(Keyword::Struct | Keyword::Enum) => next_place = Place::Type,
                Kind::LeftBrace => {
                    let opened = if stack.is_empty() {
                        match signature {
                            // The body of the function.
                            Some((start, 0)) => {
                                signature = None;
                                signature_range = Some(start..token.range.start);
                                Place::Body
                            }
                            // A struct or an enum type in the signature.
                            Some(_) => Place::Type,
                            None => next_place,
                        }
                    } else if next_place == Place::Type || place == Place::Type {
                        Place::Type
                    } else {
                        place
                    };
                    next_place = Place::Body;
                    let region = regions.len();
                    regions.push(Region { range: token.range.start..end_of_text, place: opened, closed: false });
                    stack.push(region);
                    for (name, range) in waiting.drain(..) {
                        bindings.push(Binding { name, range, region: Some(region) });
                    }
                    line_start = true;
                    index += 1;
                    continue;
                }
                Kind::RightBrace => {
                    if let Some(region) = stack.pop().and_then(|region| regions.get_mut(region)) {
                        region.range.end = token.range.end;
                        region.closed = true;
                    }
                    waiting.clear();
                }
                Kind::Keyword(Keyword::Carry) if place == Place::Body => {
                    if let Some(name) = tokens.get(index + 1).filter(|next| next.kind == Kind::Name) {
                        waiting.push((name.name.clone(), name.range.clone()));
                    }
                }
                // `for x in`, and each `, y in` of its header.
                Kind::Name
                    if place == Place::Body
                        && tokens.get(index + 1).is_some_and(|next| next.kind == Kind::Name && next.name == "in")
                        && index
                            .checked_sub(1)
                            .and_then(|before| tokens.get(before))
                            .is_some_and(|before| matches!(before.kind, Kind::Comma | Kind::Keyword(Keyword::For | Keyword::Parallel))) =>
                {
                    waiting.push((token.name.clone(), token.range.clone()));
                }
                _ => {}
            }
            // A line of a body: its targets, or the values a pattern binds.
            if line_start && place == Place::Body {
                let mut at = index;
                // A label, `#name:`.
                if tokens.get(at).is_some_and(|token| token.kind == Kind::Hash) {
                    at += 3;
                }
                let mut targets = Vec::new();
                while let Some(target) = tokens.get(at).filter(|token| token.kind == Kind::Name) {
                    targets.push(target);
                    match tokens.get(at + 1).map(|next| next.kind) {
                        Some(Kind::Comma) => at += 2,
                        _ => {
                            at += 1;
                            break;
                        }
                    }
                }
                match tokens.get(at).map(|token| token.kind) {
                    Some(Kind::Equals) if !targets.is_empty() => {
                        let region = stack.last().copied();
                        bindings.extend(targets.iter().map(|target| Binding { name: target.name.clone(), range: target.range.clone(), region }));
                    }
                    // A pattern that binds: `Reading.Value(v) =>`, `io.timeout(e) =>`.
                    _ => {
                        let mut at = index;
                        while tokens.get(at).is_some_and(|token| matches!(token.kind, Kind::Name | Kind::Dot)) {
                            at += 1;
                        }
                        if at > index && tokens.get(at).is_some_and(|token| token.kind == Kind::LeftParen) {
                            let mut bound = Vec::new();
                            at += 1;
                            while let Some(name) = tokens.get(at).filter(|token| token.kind == Kind::Name) {
                                bound.push((name.name.clone(), name.range.clone()));
                                at += if tokens.get(at + 1).is_some_and(|next| next.kind == Kind::Comma) { 2 } else { 1 };
                            }
                            let closed = tokens.get(at).is_some_and(|token| token.kind == Kind::RightParen)
                                && tokens.get(at + 1).is_some_and(|token| token.kind == Kind::FatArrow);
                            if closed {
                                if tokens.get(at + 2).is_some_and(|token| token.kind == Kind::LeftBrace) {
                                    waiting.extend(bound);
                                } else {
                                    let region = stack.last().copied();
                                    bindings.extend(bound.into_iter().map(|(name, range)| Binding { name, range, region }));
                                }
                            }
                        }
                    }
                }
            }
            line_start = token.kind == Kind::Newline;
            index += 1;
        }
        if let (None, Some((start, _))) = (&signature_range, signature) {
            signature_range = Some(start..end_of_text);
        }
        self.regions = regions;
        self.bindings = bindings;
        self.signature = signature_range;
    }

    /// The token at a position: the one that holds it, or that ends there.
    fn token_at(&self, at: usize) -> Option<usize> {
        let after = self.tokens.partition_point(|token| token.range.end < at);
        let found = self.tokens.get(after)?;
        (found.range.start <= at).then_some(after)
    }

    /// The token under a character: the one that holds it, else a name that ends just before it (the cursor after a
    /// name).
    fn token_under(&self, at: usize) -> Option<usize> {
        let after = self.tokens.partition_point(|token| token.range.end <= at);
        if self.tokens.get(after).is_some_and(|token| token.range.start <= at) {
            return Some(after);
        }
        let before = after.checked_sub(1)?;
        self.tokens.get(before).filter(|token| token.kind == Kind::Name && token.range.end == at).map(|_| before)
    }

    /// The last token that ends at or before a position.
    fn token_before(&self, at: usize) -> Option<usize> {
        self.tokens.partition_point(|token| token.range.end <= at).checked_sub(1)
    }

    /// Where a position is.
    pub fn place_at(&self, at: usize) -> Place {
        if self.signature.as_ref().is_some_and(|signature| signature.contains(&at)) {
            return Place::Signature;
        }
        self.innermost(at).and_then(|region| self.regions.get(region)).map_or(Place::Top, |region| region.place)
    }

    /// The innermost region that holds a position, inside its braces.
    fn innermost(&self, at: usize) -> Option<usize> {
        self.regions.iter().enumerate().filter(|(_, region)| region.holds(at)).min_by_key(|(_, region)| region.range.len()).map(|(index, _)| index)
    }

    /// Whether a binding is seen at a position: in its region, or anywhere for one of the signature.
    fn sees(&self, region: Option<usize>, at: usize) -> bool {
        region.and_then(|region| self.regions.get(region)).is_none_or(|region| region.holds(at))
    }

    /// The bindings seen at a position, the innermost first: what a name there designates (R-3).
    pub fn seen_at(&self, at: usize) -> Vec<&Binding> {
        let mut seen: Vec<&Binding> = self.bindings.iter().filter(|binding| self.sees(binding.region, at)).collect();
        let size = |binding: &Binding| binding.region.and_then(|region| self.regions.get(region)).map_or(usize::MAX, |region| region.range.len());
        seen.sort_by_key(|binding| size(binding));
        seen
    }

    /// Every binding of the text, in its order.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The path written around a name token, up to it: `core.add` for `add`, `core` for `core`. Its segments, and the
    /// range of the token.
    fn path_to(&self, index: usize) -> Option<(Vec<String>, Range<usize>)> {
        let token = self.tokens.get(index).filter(|token| token.kind == Kind::Name)?;
        let mut segments = vec![token.name.clone()];
        let mut at = index;
        // At most so many segments are read back *(proposed)*: no name of the language is longer.
        let mut left = MAX_SEGMENTS;
        while at >= 2
            && self
                .tokens
                .get(at - 1)
                .is_some_and(|dot| dot.kind == Kind::Dot && dot.range.start == self.tokens.get(at - 2).map_or(0, |name| name.range.end))
            && let Some(name) = self.tokens.get(at - 2).filter(|name| name.kind == Kind::Name)
        {
            if left == 0 {
                return None;
            }
            left -= 1;
            segments.push(name.name.clone());
            at -= 2;
        }
        segments.reverse();
        Some((segments, token.range.clone()))
    }

    /// The path before a dot that ends at `at`: `core` for `core.|`.
    fn path_before_dot(&self, at: usize) -> Option<Vec<String>> {
        let dot = self.token_before(at)?;
        let token = self.tokens.get(dot).filter(|token| token.kind == Kind::Dot && token.range.end == at)?;
        let name = dot.checked_sub(1)?;
        self.tokens.get(name).filter(|name| name.kind == Kind::Name && name.range.end == token.range.start)?;
        self.path_to(name).map(|(segments, _)| segments)
    }
}

fn kind_of(kind: &TokenKind<'_>) -> Kind {
    match kind {
        TokenKind::String(_) | TokenKind::Comment(_) | TokenKind::Doc(_) => Kind::Prose,
        TokenKind::Int(_) | TokenKind::Float(_) => Kind::Number,
        TokenKind::Newline => Kind::Newline,
        TokenKind::Dot => Kind::Dot,
        TokenKind::Comma => Kind::Comma,
        TokenKind::Colon => Kind::Colon,
        TokenKind::Equals => Kind::Equals,
        TokenKind::Arrow => Kind::Arrow,
        TokenKind::FatArrow => Kind::FatArrow,
        TokenKind::LeftParen => Kind::LeftParen,
        TokenKind::RightParen => Kind::RightParen,
        TokenKind::LeftBrace => Kind::LeftBrace,
        TokenKind::RightBrace => Kind::RightBrace,
        TokenKind::Hash => Kind::Hash,
        TokenKind::Less => Kind::Less,
        _ => Kind::Other,
    }
}

/// The diagnostics of a file in a report, as LSP gives them: lines and characters from 0, from the lines and columns
/// from 1 of `labcraft-check/1`. Those with no place in the file are in `Problems` only.
pub fn diagnostics_of(report: &Report, file: &str) -> Vec<Diagnostic> {
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.file == file)
        .filter_map(|diagnostic| {
            let span = diagnostic.span?;
            let severity = match diagnostic.severity() {
                Severity::Error => DiagnosticSeverity::ERROR,
                _ => DiagnosticSeverity::WARNING,
            };
            Some(lsp_diagnostic(
                (span.start.line, span.start.column),
                (span.end.line, span.end.column),
                severity,
                diagnostic.code().as_str(),
                &diagnostic.message,
            ))
        })
        .collect()
}

/// The diagnostics of `check` on a text that starts with a byte order mark, which `check` does not read (spec 2): on
/// the first line, one character later in the text the field holds.
pub fn after_bom(text: &str, mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    if text.starts_with('\u{FEFF}') {
        for diagnostic in &mut diagnostics {
            for position in [&mut diagnostic.range.start, &mut diagnostic.range.end] {
                if position.line == 0 {
                    position.character = position.character.saturating_add(1);
                }
            }
        }
    }
    diagnostics
}

/// The diagnostic of a text that does not parse, from the `error` of `text.save` (`labcraft-check/1`).
pub fn diagnostic_of_error(error: &serde_json::Value) -> Option<Diagnostic> {
    let number = |pointer: &str| error.pointer(pointer).and_then(serde_json::Value::as_u64).and_then(|value| u32::try_from(value).ok());
    let start = (number("/span/start/line")?, number("/span/start/column")?);
    let end = (number("/span/end/line").unwrap_or(start.0), number("/span/end/column").unwrap_or(start.1));
    let code = error.get("code").and_then(serde_json::Value::as_str).unwrap_or_default();
    let message = error.get("message").and_then(serde_json::Value::as_str).unwrap_or("the text does not parse");
    Some(lsp_diagnostic(start, end, DiagnosticSeverity::ERROR, code, message))
}

fn lsp_diagnostic(start: (u32, u32), end: (u32, u32), severity: DiagnosticSeverity, code: &str, message: &str) -> Diagnostic {
    let position = |(line, column): (u32, u32)| Position { line: line.saturating_sub(1), character: column.saturating_sub(1) };
    Diagnostic {
        range: kit_text::lsp_types::Range { start: position(start), end: position(end) },
        severity: Some(severity),
        code: (!code.is_empty()).then(|| NumberOrString::String(code.to_owned())),
        source: Some("labcraft".to_owned()),
        message: message.to_owned(),
        ..Diagnostic::default()
    }
}

/// The URI of a file of the project in a [`Location`]: `labcraft:` and its path, as the session names it, encoded.
pub fn uri_of(path: &str) -> Option<Uri> {
    let mut encoded = String::from("labcraft:");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    Uri::from_str(&encoded).ok()
}

/// The path of a file of the project from its URI.
pub fn path_of(uri: &Uri) -> Option<String> {
    let text = uri.as_str().strip_prefix("labcraft:")?;
    let mut bytes = Vec::new();
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let high = char::from(rest.next()?).to_digit(16)?;
            let low = char::from(rest.next()?).to_digit(16)?;
            bytes.push(u8::try_from(high * 16 + low).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

/// The scan of a field, kept while its text does not change.
#[derive(Clone, Debug, Default)]
pub struct Scanned {
    revision: Option<u64>,
    scan: Arc<Scan>,
}

impl Scanned {
    pub fn of(&mut self, field: &Field) -> Arc<Scan> {
        if self.revision != Some(field.revision()) {
            self.scan = Arc::new(Scan::new(&field.text()));
            self.revision = Some(field.revision());
        }
        self.scan.clone()
    }
}

/// What the session knows of the project, for the language service.
pub struct Known<'a> {
    pub callables: &'a [Callable],
    pub functions: &'a [Listed],
    pub types: &'a [ListedType],
    /// The drawing of the function: the types of its values.
    pub drawing: Option<&'a Drawing>,
}

/// The service, for one frame: what the session knows of the project, and the text of the view.
pub struct Service<'a> {
    pub file: &'a str,
    pub known: &'a Known<'a>,
    /// The diagnostics that the view published since the last frame.
    pub published: &'a mut Vec<Published>,
    pub scanned: &'a mut Scanned,
}

impl Service<'_> {
    /// The type of a value, from the drawing of the function: that of the terminal that gives it.
    fn type_of(&self, name: &str) -> Option<String> {
        let drawing = self.known.drawing?;
        let port = drawing.ports.iter().find(|port| !port.input && port.name == name && port.ty.is_some())?;
        drawing.types.get(port.ty?).cloned().filter(|ty| !ty.is_empty())
    }

    /// The libraries that functions or types come from, `core` and the others; not the root of the project.
    fn libraries(&self) -> Vec<String> {
        let mut libraries: Vec<String> =
            self.known.callables.iter().filter_map(|callable| callable.name.split_once('.').map(|(library, _)| library.to_owned())).collect();
        libraries.sort();
        libraries.dedup();
        libraries
    }

    /// What follows `path.` in the names of the functions and types: their next segment, once each; the variants of a
    /// type of that name.
    fn members(&self, path: &[String]) -> Vec<CompletionItem> {
        let prefix = format!("{}.", path.join("."));
        let mut items: Vec<CompletionItem> = Vec::new();
        for callable in self.known.callables {
            let Some(rest) = callable.name.strip_prefix(&prefix) else { continue };
            match rest.split_once('.') {
                Some((library, _)) => {
                    if !items.iter().any(|item| item.label == library) {
                        items.push(library_item(library, "2"));
                    }
                }
                None => items.push(callable_item(callable, rest)),
            }
        }
        let whole = path.join(".");
        if let Some(ty) = self.known.callables.iter().find(|callable| callable.kind == "type" && callable.name == whole) {
            for variant in &ty.typed.variants {
                items.push(item(&variant.name, CompletionItemKind::ENUM_MEMBER, Some(whole.clone()), "0"));
            }
        }
        items
    }

    fn type_items(&self) -> Vec<CompletionItem> {
        let mut items: Vec<CompletionItem> =
            BASE_TYPES.iter().map(|ty| item(ty, CompletionItemKind::TYPE_PARAMETER, Some("base type".to_owned()), "1")).collect();
        items.extend(
            self.known
                .callables
                .iter()
                .filter(|callable| callable.kind == "type" && !callable.name.contains('.'))
                .map(|callable| callable_item(callable, &callable.name)),
        );
        items.extend(self.libraries().into_iter().map(|library| library_item(&library, "2")));
        items
    }

    fn words(words: &[&str]) -> impl Iterator<Item = CompletionItem> {
        words.iter().map(|word| item(word, CompletionItemKind::KEYWORD, None, "3"))
    }

    /// The values, each name once, with their types: linear in the bindings and the terminals of the drawing.
    fn value_items(&self, bindings: Vec<&Binding>) -> Vec<CompletionItem> {
        let mut types: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        if let Some(drawing) = self.known.drawing {
            for port in drawing.ports.iter().filter(|port| !port.input) {
                if let Some(ty) = port.ty.and_then(|ty| drawing.types.get(ty)).filter(|ty| !ty.is_empty()) {
                    types.entry(port.name.as_str()).or_insert(ty.as_str());
                }
            }
        }
        let mut given = std::collections::HashSet::new();
        bindings
            .into_iter()
            .filter(|binding| given.insert(binding.name.as_str()))
            .map(|binding| item(&binding.name, CompletionItemKind::VARIABLE, types.get(binding.name.as_str()).map(|ty| (*ty).to_owned()), "0"))
            .collect()
    }
}

fn item(label: &str, kind: CompletionItemKind, detail: Option<String>, rank: &str) -> CompletionItem {
    CompletionItem { label: label.to_owned(), kind: Some(kind), detail, sort_text: Some(format!("{rank}{label}")), ..CompletionItem::default() }
}

fn library_item(library: &str, rank: &str) -> CompletionItem {
    item(library, CompletionItemKind::MODULE, Some("library".to_owned()), rank)
}

fn callable_item(callable: &Callable, label: &str) -> CompletionItem {
    let kind = if callable.kind == "type" { CompletionItemKind::STRUCT } else { CompletionItemKind::FUNCTION };
    let mut found = item(label, kind, Some(callable.signature.clone()), "1");
    found.documentation = callable.doc.clone().map(kit_text::lsp_types::Documentation::String);
    found
}

impl LanguageService for Service<'_> {
    fn diagnostics(&mut self) -> Vec<Published> {
        std::mem::take(self.published)
    }

    fn completion(&mut self, field: &Field, at: Position) -> Vec<CompletionItem> {
        let scan = self.scanned.of(field);
        let at = offset_of(field, at);
        let start = kit_text::complete::name_start(field, at);
        // Nothing in a string or a comment.
        if scan.token_at(start).and_then(|index| scan.tokens.get(index)).is_some_and(|token| token.kind == Kind::Prose && token.range.start < start) {
            return Vec::new();
        }
        if let Some(path) = scan.path_before_dot(start) {
            return self.members(&path);
        }
        let before = scan.token_before(start).and_then(|index| scan.tokens.get(index)).map(|token| token.kind);
        let place = scan.place_at(start);
        match place {
            Place::Top if before == Some(Kind::Keyword(Keyword::Use)) => {
                self.libraries().into_iter().map(|library| library_item(&library, "0")).collect()
            }
            Place::Top => Self::words(&TOP_WORDS).collect(),
            Place::Signature | Place::Panel if before == Some(Kind::Colon) => self.type_items(),
            Place::Type => self.type_items(),
            Place::Signature => Vec::new(),
            Place::Panel => Self::words(&PANEL_WORDS).collect(),
            Place::Body if before == Some(Kind::Colon) && is_carry_type(&scan, start) => self.type_items(),
            Place::Body => {
                let mut items = self.value_items(scan.seen_at(start));
                items.extend(
                    self.known
                        .callables
                        .iter()
                        .filter(|callable| !callable.name.contains('.'))
                        .map(|callable| callable_item(callable, &callable.name)),
                );
                items.extend(self.libraries().into_iter().map(|library| library_item(&library, "2")));
                items.extend(Self::words(&BODY_WORDS));
                items
            }
            Place::Layout => {
                let mut items = self.value_items(scan.bindings().iter().collect());
                items.extend(Self::words(&LAYOUT_WORDS));
                items
            }
            Place::Terminals => {
                let mut items = self.value_items(scan.bindings().iter().filter(|binding| binding.region.is_none()).collect());
                items.extend(Self::words(&TERMINALS_WORDS));
                items
            }
            Place::Outline => Self::words(&OUTLINE_WORDS).collect(),
            Place::Icon => Self::words(&ICON_WORDS).collect(),
        }
    }

    fn hover(&mut self, field: &Field, at: Position) -> Option<Hover> {
        let named = Named::at(self.scanned.of(field), offset_of(field, at))?;
        let path = &named.path;
        let said = if let Some(callable) = self.known.callables.iter().find(|callable| callable.name == *path) {
            match &callable.doc {
                Some(doc) if !doc.is_empty() => format!("{}\n\n{doc}", callable.signature),
                _ => callable.signature.clone(),
            }
        } else if let Some(binding) = named.binding() {
            match self.type_of(&binding.name) {
                Some(ty) => format!("{}: {ty}", binding.name),
                None => binding.name.clone(),
            }
        } else if self.libraries().contains(path) {
            format!("library {path}")
        } else {
            return None;
        };
        let range = range_of(field, &named.range);
        Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::PlainText, value: said }), range: Some(range) })
    }

    fn definition(&mut self, field: &Field, at: Position) -> Option<GotoDefinitionResponse> {
        let named = Named::at(self.scanned.of(field), offset_of(field, at))?;
        if let Some(binding) = named.binding() {
            return Some(GotoDefinitionResponse::Scalar(Location { uri: uri_of(self.file)?, range: range_of(field, &binding.range) }));
        }
        let path = &named.path;
        let file = self
            .known
            .functions
            .iter()
            .find(|listed| listed.function == *path)
            .map(|listed| listed.file.as_str())
            .or_else(|| self.known.types.iter().find(|listed| listed.name == *path).map(|listed| listed.file.as_str()))?;
        Some(GotoDefinitionResponse::Scalar(Location { uri: uri_of(file)?, range: kit_text::lsp_types::Range::default() }))
    }
}

/// The range of LSP of characters of the field.
fn range_of(field: &Field, range: &Range<usize>) -> kit_text::lsp_types::Range {
    kit_text::lsp_types::Range { start: position_of(field, range.start), end: position_of(field, range.end) }
}

/// The name under a position: the path written up to it, and what it designates when a line binds it.
struct Named {
    scan: Arc<Scan>,
    offset: usize,
    /// `core.add` for `add`, `x` for `x`.
    path: String,
    segments: usize,
    /// The characters of its last segment.
    range: Range<usize>,
}

impl Named {
    fn at(scan: Arc<Scan>, offset: usize) -> Option<Named> {
        let index = scan.token_under(offset)?;
        let (segments, range) = scan.path_to(index)?;
        Some(Named { path: segments.join("."), segments: segments.len(), range, scan, offset })
    }

    /// The binding it designates, the innermost seen there (R-3): a name alone only. In a `layout` or `terminals`
    /// section, which names the values of the whole function, the first that binds it.
    fn binding(&self) -> Option<&Binding> {
        if self.segments != 1 {
            return None;
        }
        let seen = self.scan.seen_at(self.offset).into_iter().find(|binding| binding.name == self.path);
        match self.scan.place_at(self.offset) {
            Place::Layout | Place::Terminals => seen.or_else(|| self.scan.bindings().iter().find(|binding| binding.name == self.path)),
            _ => seen,
        }
    }
}

/// `carry name:` before a position: the type of a carry is written there (spec 14.5).
fn is_carry_type(scan: &Scan, at: usize) -> bool {
    let Some(colon) = scan.token_before(at) else { return false };
    colon >= 2
        && scan.tokens.get(colon - 1).is_some_and(|name| name.kind == Kind::Name)
        && scan.tokens.get(colon - 2).is_some_and(|carry| carry.kind == Kind::Keyword(Keyword::Carry))
}

#[cfg(test)]
mod tests;
