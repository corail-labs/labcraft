//! The tokens of spec 14.1, from text that [`crate::decode`] has accepted. Hand-written: line endings
//! are significant by context (14.1), so they are tokens, and comments are tokens too, since `fmt`
//! must keep them all. Numbers, strings and comments keep their text, borrowed from the file.

use labcraft_types::{Code, Diagnostic, Interner, Name, Position, Report, Span, normalize};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

use crate::decode::advance;

macro_rules! keywords {
    ($($variant:ident = $text:literal,)*) => {
        /// The reserved words (spec 14.1). Contextual words are ordinary names: the parser reads them
        /// where the syntax expects them.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Keyword {
            $($variant,)*
        }

        impl Keyword {
            pub const ALL: &[Keyword] = &[$(Keyword::$variant,)*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Keyword::$variant => $text,)*
                }
            }

            fn from_text(text: &str) -> Option<Keyword> {
                match text {
                    $($text => Some(Keyword::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

keywords! {
    Labcraft = "labcraft",
    Use = "use",
    Fn = "fn",
    Extern = "extern",
    Type = "type",
    Struct = "struct",
    Enum = "enum",
    Match = "match",
    For = "for",
    Parallel = "parallel",
    Loop = "loop",
    ContinueOnError = "continue_on_error",
    Carry = "carry",
    Next = "next",
    Collect = "collect",
    Until = "until",
    Yield = "yield",
    Handle = "handle",
    Events = "events",
    Panel = "panel",
    Layout = "layout",
    Icon = "icon",
    True = "true",
    False = "false",
    None = "none",
    Nan = "nan",
    Inf = "inf",
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind<'a> {
    /// `IDENT`, in its one spelling (R-20). It may also turn out to be a unit, right after a number or
    /// in brackets (R-5), which the parser knows and this does not.
    Name(Name),
    /// `UNIT_NAME` starting with `°`, which no identifier can.
    DegreeName(Name),
    Keyword(Keyword),
    /// The text as written: `1_000`.
    Int(&'a str),
    /// The text as written: `1_000.5_0`, `1e3`, `1e-3`.
    Float(&'a str),
    /// What lies between the quotes, escapes unread; they are known to be valid.
    String(&'a str),
    /// What follows `//`, up to the line ending.
    Comment(&'a str),
    /// What follows `///`, up to the line ending.
    Doc(&'a str),
    Newline,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Comma,
    Colon,
    Semicolon,
    Dot,
    DotDot,
    Ellipsis,
    Equals,
    Arrow,
    FatArrow,
    Hash,
    Underscore,
    Less,
    Greater,
    Star,
    MiddleDot,
    Slash,
    Caret,
    Minus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind<'a>,
    pub span: Span,
}

/// The bidirectional controls of `LC0005`: the text does not read as it runs.
fn is_bidirectional_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `Default_Ignorable_Code_Point` of Unicode (`DerivedCoreProperties.txt`, the same in 17 and 18). These
/// show as nothing, and spec 14.1 keeps them out of names. The whole property is here, not only what
/// is a letter or a mark today: part of it is reserved, and could be assigned as one later.
fn is_default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

fn is_letter(c: char) -> bool {
    c.general_category_group() == GeneralCategoryGroup::Letter && !is_default_ignorable(c)
}

fn is_mark(c: char) -> bool {
    c.general_category_group() == GeneralCategoryGroup::Mark && !is_default_ignorable(c)
}

fn is_name_char(c: char) -> bool {
    is_letter(c) || c.is_ascii_digit() || c == '_' || is_mark(c)
}

/// Whether this text is one name of the language, as the lexer would read it: not a reserved word (14.1, R-20).
pub fn is_name(text: &str) -> bool {
    let text = normalize(text);
    let mut chars = text.chars();
    chars.next().is_some_and(is_letter) && chars.all(is_name_char) && Keyword::from_text(&text).is_none()
}

/// Whether this character shows nothing: a control, or what spec 14.1 keeps out of names.
pub fn is_invisible(c: char) -> bool {
    c.is_control() || is_default_ignorable(c)
}

/// Whether this character can pad a value: a space, or what shows by itself, so no mark, separator, control or private use.
pub fn is_fill(c: char) -> bool {
    c == ' '
        || (!is_default_ignorable(c)
            && !matches!(c.general_category_group(), GeneralCategoryGroup::Mark | GeneralCategoryGroup::Separator | GeneralCategoryGroup::Other))
}

/// A character as a message can safely show it. Beyond ASCII it is a number: an invisible character,
/// whose category can even be a letter (U+3164), or a control of the reading direction, would make the
/// message itself lie.
fn describe(c: char) -> String {
    if c.is_ascii_graphic() { format!("`{c}`") } else { format!("U+{:04X}", u32::from(c)) }
}

/// An iterator of tokens that ends at the first lexical error, which is a diagnostic: the file is then
/// not read further. Warnings go to the report, which bounds them.
pub struct Lexer<'a, 'b> {
    rest: &'a str,
    position: Position,
    names: &'b mut Interner,
    report: &'b mut Report,
    failed: bool,
}

impl<'a, 'b> Lexer<'a, 'b> {
    pub fn new(text: &'a str, names: &'b mut Interner, report: &'b mut Report) -> Self {
        Lexer { rest: text, position: Position { line: 1, column: 1 }, names, report, failed: false }
    }

    /// How far the lexer has read: once the tokens end, where the file ends, spaces included.
    pub fn position(&self) -> Position {
        self.position
    }

    /// The interner it was given, which its reader needs too while it holds the lexer.
    pub fn names(&mut self) -> &mut Interner {
        self.names
    }

    fn peek(&self) -> Option<char> {
        self.rest.chars().next()
    }

    /// The character after the next one.
    fn peek_second(&self) -> Option<char> {
        self.rest.chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let mut chars = self.rest.chars();
        let c = chars.next()?;
        self.rest = chars.as_str();
        self.position = advance(self.position, c);
        Some(c)
    }

    /// Takes these characters if the text goes on with them.
    fn eat(&mut self, expected: &str) -> bool {
        if !self.rest.starts_with(expected) {
            return false;
        }
        for _ in expected.chars() {
            self.bump();
        }
        true
    }

    fn take_while(&mut self, accepted: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&accepted) {
            self.bump();
        }
    }

    /// The text taken since `start`, an earlier value of `rest`.
    fn since(&self, start: &'a str) -> &'a str {
        start.get(..start.len().saturating_sub(self.rest.len())).unwrap_or_default()
    }

    fn fail(&mut self, code: Code, message: String, start: Position) -> Box<Diagnostic> {
        self.failed = true;
        let end = if self.position == start { Position { line: start.line, column: start.column.saturating_add(1) } } else { self.position };
        Box::new(Diagnostic::new(code, message, "").with_span(Span { start, end }))
    }

    fn warn_about_bidirectional_control(&mut self, c: char) {
        let start = self.position;
        let end = Position { line: start.line, column: start.column.saturating_add(1) };
        let message = format!("bidirectional control character U+{:04X}: the text does not read as it runs", u32::from(c));
        self.report.push(Diagnostic::new(Code::Lc0005, message, "").with_span(Span { start, end }));
    }

    /// One warning per string or comment, at its first control.
    fn watch_for_bidirectional_control(&mut self, c: char, warned: &mut bool) {
        if !*warned && is_bidirectional_control(c) {
            self.warn_about_bidirectional_control(c);
            *warned = true;
        }
    }

    fn name_or_keyword(&mut self) -> TokenKind<'a> {
        let start = self.rest;
        self.take_while(is_name_char);
        let text = normalize(self.since(start));
        match Keyword::from_text(&text) {
            Some(keyword) => TokenKind::Keyword(keyword),
            None => TokenKind::Name(self.names.intern(&text)),
        }
    }

    fn degree_name(&mut self, start_position: Position) -> Result<TokenKind<'a>, Box<Diagnostic>> {
        let start = self.rest;
        self.bump();
        match self.peek() {
            None => return Err(self.fail(Code::Lc1002, "file ends after `\u{00B0}`, where a unit name was expected".to_owned(), start_position)),
            Some(c) if !is_letter(c) => return Err(self.fail(Code::Lc1001, "expected a unit name after `\u{00B0}`".to_owned(), start_position)),
            Some(_) => {}
        }
        self.take_while(is_name_char);
        let text = self.since(start);
        Ok(TokenKind::DegreeName(self.names.intern(text)))
    }

    fn digits(&mut self) {
        self.take_while(|c| c.is_ascii_digit() || c == '_');
    }

    /// `1..5` is two integers, `1.5` one float, and `5e` an integer and a name.
    fn number(&mut self) -> TokenKind<'a> {
        let start = self.rest;
        self.digits();
        let mut float = false;
        if self.peek() == Some('.') && self.peek_second().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            self.digits();
            float = true;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let mut after = self.rest.chars().skip(1);
            let first = after.next();
            let negative = first == Some('-');
            let digit = if negative { after.next() } else { first };
            if digit.is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
                if negative {
                    self.bump();
                }
                self.digits();
                float = true;
            }
        }
        let text = self.since(start);
        if float { TokenKind::Float(text) } else { TokenKind::Int(text) }
    }

    fn string(&mut self, start_position: Position) -> Result<TokenKind<'a>, Box<Diagnostic>> {
        self.bump();
        let start = self.rest;
        let mut warned = false;
        loop {
            match self.peek() {
                None | Some('\n' | '\r') => return Err(self.fail(Code::Lc1003, "unterminated string".to_owned(), start_position)),
                Some('"') => {
                    let text = self.since(start);
                    self.bump();
                    return Ok(TokenKind::String(text));
                }
                Some('\\') => {
                    let escape = self.position;
                    self.bump();
                    match self.peek() {
                        Some('"' | '\\' | 'n' | 't' | 'r') => {
                            self.bump();
                        }
                        None | Some('\n' | '\r') => {
                            return Err(self.fail(Code::Lc1003, "unterminated string".to_owned(), start_position));
                        }
                        Some(c) => {
                            let what = describe(c);
                            return Err(self.fail(Code::Lc1003, format!("unknown escape before {what}: use \\\" \\\\ \\n \\t \\r"), escape));
                        }
                    }
                }
                Some(c) => {
                    self.watch_for_bidirectional_control(c, &mut warned);
                    self.bump();
                }
            }
        }
    }

    /// The text of a comment, up to its line ending.
    fn comment_text(&mut self) -> &'a str {
        let start = self.rest;
        let mut warned = false;
        while let Some(c) = self.peek().filter(|c| !matches!(c, '\n' | '\r')) {
            self.watch_for_bidirectional_control(c, &mut warned);
            self.bump();
        }
        self.since(start)
    }

    fn token(&mut self, c: char, start: Position) -> Result<TokenKind<'a>, Box<Diagnostic>> {
        let single = |kind: TokenKind<'a>, lexer: &mut Self| {
            lexer.bump();
            Ok(kind)
        };
        match c {
            '\n' => single(TokenKind::Newline, self),
            '\r' if self.peek_second() == Some('\n') => {
                self.bump();
                single(TokenKind::Newline, self)
            }
            '"' => self.string(start),
            '\u{00B0}' => self.degree_name(start),
            '/' if self.eat("///") => Ok(TokenKind::Doc(self.comment_text())),
            '/' if self.eat("//") => Ok(TokenKind::Comment(self.comment_text())),
            '/' => single(TokenKind::Slash, self),
            '.' if self.eat("...") => Ok(TokenKind::Ellipsis),
            '.' if self.eat("..") => Ok(TokenKind::DotDot),
            '.' => single(TokenKind::Dot, self),
            '-' if self.eat("->") => Ok(TokenKind::Arrow),
            '-' => single(TokenKind::Minus, self),
            '=' if self.eat("=>") => Ok(TokenKind::FatArrow),
            '=' => single(TokenKind::Equals, self),
            '(' => single(TokenKind::LeftParen, self),
            ')' => single(TokenKind::RightParen, self),
            '[' => single(TokenKind::LeftBracket, self),
            ']' => single(TokenKind::RightBracket, self),
            '{' => single(TokenKind::LeftBrace, self),
            '}' => single(TokenKind::RightBrace, self),
            ',' => single(TokenKind::Comma, self),
            ':' => single(TokenKind::Colon, self),
            ';' => single(TokenKind::Semicolon, self),
            '#' => single(TokenKind::Hash, self),
            '_' => single(TokenKind::Underscore, self),
            '<' => single(TokenKind::Less, self),
            '>' => single(TokenKind::Greater, self),
            '*' => single(TokenKind::Star, self),
            '\u{00B7}' => single(TokenKind::MiddleDot, self),
            '^' => single(TokenKind::Caret, self),
            '0'..='9' => Ok(self.number()),
            c if is_letter(c) => Ok(self.name_or_keyword()),
            c => {
                let message = format!("unexpected character {}", describe(c));
                Err(self.fail(Code::Lc1001, message, start))
            }
        }
    }
}

impl<'a> Iterator for Lexer<'a, '_> {
    /// The error is boxed: it comes once, and tokens come by the million.
    type Item = Result<Token<'a>, Box<Diagnostic>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        self.take_while(|c| matches!(c, ' ' | '\t'));
        let c = self.peek()?;
        let start = self.position;
        Some(self.token(c, start).map(|kind| Token { kind, span: Span { start, end: self.position } }))
    }
}

#[cfg(test)]
mod tests {
    use super::TokenKind as T;
    use super::*;

    struct Lexed {
        report: Report,
        tokens: Vec<String>,
        error: Option<Diagnostic>,
    }

    /// Tokens as short text, so that a test reads like the file.
    fn lex(text: &str) -> Lexed {
        let (mut names, mut report) = (Interner::new(), Report::new(1000));
        let mut kinds = Vec::new();
        let mut error = None;
        for item in Lexer::new(text, &mut names, &mut report) {
            match item {
                Ok(token) => kinds.push(token.kind),
                Err(diagnostic) => error = Some(*diagnostic),
            }
        }
        let tokens = kinds
            .iter()
            .map(|kind| match kind {
                T::Name(name) => format!("name:{}", names.resolve(*name)),
                T::DegreeName(name) => format!("degree:{}", names.resolve(*name)),
                T::Keyword(keyword) => format!("kw:{}", keyword.as_str()),
                T::Int(text) => format!("int:{text}"),
                T::Float(text) => format!("float:{text}"),
                T::String(text) => format!("str:{text}"),
                T::Comment(text) => format!("comment:{text}"),
                T::Doc(text) => format!("doc:{text}"),
                other => format!("{other:?}"),
            })
            .collect();
        Lexed { report, tokens, error }
    }

    fn tokens(text: &str) -> Vec<String> {
        let lexed = lex(text);
        assert_eq!(lexed.error, None, "{text}");
        lexed.tokens
    }

    fn error_code(text: &str) -> Code {
        lex(text).error.unwrap_or_else(|| panic!("no error in {text:?}")).code()
    }

    #[test]
    fn nothing_gives_nothing() {
        assert!(tokens("").is_empty());
        assert!(tokens("  \t ").is_empty());
    }

    #[test]
    fn a_statement() {
        assert_eq!(
            tokens("#a: p = add(x, 1 V)\n"),
            ["Hash", "name:a", "Colon", "name:p", "Equals", "name:add", "LeftParen", "name:x", "Comma", "int:1", "name:V", "RightParen", "Newline",]
        );
    }

    #[test]
    fn every_reserved_word_is_a_keyword() {
        for keyword in Keyword::ALL {
            assert_eq!(tokens(keyword.as_str()), [format!("kw:{}", keyword.as_str())]);
        }
        assert_eq!(Keyword::ALL.len(), 27);
    }

    /// The list of spec 14.1, which `tools/lcf-view/lcf_view.py` copies too.
    #[test]
    fn the_reserved_words_are_those_of_the_spec() {
        const SPEC: &str = include_str!("../../../docs/language-spec-v0.md");
        let start = SPEC.find("Reserved words, everywhere: `").map(|at| at + "Reserved words, everywhere: `".len());
        let list = start.and_then(|start| SPEC.get(start..)).and_then(|rest| rest.split_once('`')).map(|(list, _)| list);
        let mut spec: Vec<&str> = list.unwrap_or_default().split_whitespace().collect();
        let mut ours: Vec<&str> = Keyword::ALL.iter().map(|keyword| keyword.as_str()).collect();
        spec.sort_unstable();
        ours.sort_unstable();
        assert_eq!(spec, ours);
        const VIEW: &str = include_str!("../../../tools/lcf-view/lcf_view.py");
        for keyword in ours {
            let written = [format!(" {keyword} "), format!("\"{keyword} "), format!(" {keyword}\"")];
            assert!(written.iter().any(|text| VIEW.contains(text.as_str())), "{keyword} in lcf_view.py");
        }
    }

    #[test]
    fn contextual_words_are_names() {
        for word in ["ok", "code", "in", "after", "opaque", "control", "indicator", "node", "wire", "at", "via", "view", "rect", "fill", "accent"] {
            assert_eq!(tokens(word), [format!("name:{word}")]);
        }
    }

    #[test]
    fn a_word_that_starts_like_a_keyword_is_a_name() {
        assert_eq!(tokens("fnord for_ use2"), ["name:fnord", "name:for_", "name:use2"]);
    }

    #[test]
    fn names_are_read_in_their_one_spelling() {
        assert_eq!(tokens("e\u{0301}t\u{00E9}"), ["name:\u{00E9}t\u{00E9}"]);
        assert_eq!(tokens("\u{03BC}V"), ["name:\u{00B5}V"]);
        assert_eq!(tokens("k\u{2126}"), ["name:k\u{03A9}"]);
    }

    #[test]
    fn the_same_name_is_the_same_token() {
        let (mut names, mut report) = (Interner::new(), Report::new(10));
        let kinds: Vec<TokenKind> = Lexer::new("\u{00E9} e\u{0301} a", &mut names, &mut report).map(|item| item.unwrap().kind).collect();
        assert_eq!(kinds[0], kinds[1]);
        assert_ne!(kinds[0], kinds[2]);
    }

    #[test]
    fn a_name_starts_with_a_letter() {
        assert_eq!(tokens("_ _x a_b"), ["Underscore", "Underscore", "name:x", "name:a_b"]);
        assert_eq!(tokens("9a"), ["int:9", "name:a"]);
        for bad in ["\u{0301}a", "\u{0663}", "$", "@", "~", "\u{00A0}", "\u{200B}", "\u{FEFF}"] {
            assert_eq!(error_code(bad), Code::Lc1001, "{bad:?}");
        }
    }

    /// `DerivedCoreProperties.txt`, written out again: a slip in the table of the lexer shows here.
    const DEFAULT_IGNORABLE: [(u32, u32); 17] = [
        (0x00AD, 0x00AD),
        (0x034F, 0x034F),
        (0x061C, 0x061C),
        (0x115F, 0x1160),
        (0x17B4, 0x17B5),
        (0x180B, 0x180F),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x206F),
        (0x3164, 0x3164),
        (0xFE00, 0xFE0F),
        (0xFEFF, 0xFEFF),
        (0xFFA0, 0xFFA0),
        (0xFFF0, 0xFFF8),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0000, 0xE0FFF),
    ];

    #[test]
    fn no_name_holds_an_invisible_character() {
        for c in DEFAULT_IGNORABLE.iter().flat_map(|&(first, last)| first..=last).filter_map(char::from_u32) {
            for text in [format!("{c}"), format!("{c}a"), format!("a{c}"), format!("a{c}b"), format!("\u{00B0}{c}"), format!("\u{00B0}C{c}")] {
                let lexed = lex(&text);
                let error = lexed.error.unwrap_or_else(|| panic!("no error for U+{:04X} in {text:?}", u32::from(c)));
                assert_eq!(error.code(), Code::Lc1001, "{text:?}");
                assert_eq!(lexed.report.warnings(), 0, "{text:?}");
            }
        }
    }

    /// Two copies typed by hand can lose the same line, so the count comes from the data file too: 4,174
    /// code points in Unicode 17 and 18. The ranges were compared with that file when they were written.
    #[test]
    fn the_two_tables_of_invisible_characters_agree_and_have_the_count_of_unicode() {
        let listed = |c: char| DEFAULT_IGNORABLE.iter().any(|&(first, last)| (first..=last).contains(&u32::from(c)));
        for c in '\0'..=char::MAX {
            assert_eq!(is_default_ignorable(c), listed(c), "U+{:04X}", u32::from(c));
        }
        assert_eq!(('\0'..=char::MAX).filter(|c| is_default_ignorable(*c)).count(), 4174);
    }

    #[test]
    fn an_invisible_character_is_reported_where_it_stands() {
        let lexed = lex("ab\u{FE0F}c");
        assert_eq!(lexed.tokens, ["name:ab"]);
        assert_eq!(error_span("ab\u{FE0F}c"), (1, 3, 1, 4));
        assert_eq!(error_span("  \u{3164}"), (1, 3, 1, 4));
    }

    #[test]
    fn the_neighbours_of_invisible_characters_stay_in_names() {
        for c in ['\u{034E}', '\u{0350}', '\u{115E}', '\u{1161}', '\u{17B3}', '\u{17B6}', '\u{3163}', '\u{3165}', '\u{FE20}', '\u{FFA1}'] {
            assert_eq!(tokens(&format!("a{c}")), [format!("name:a{c}")], "U+{:04X}", u32::from(c));
        }
        for c in ['\u{115E}', '\u{1161}', '\u{3163}', '\u{3165}', '\u{FFA1}'] {
            assert_eq!(tokens(&format!("{c}")), [format!("name:{c}")], "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn strings_and_comments_may_hold_invisible_characters() {
        assert_eq!(tokens("\"\u{3164}a\u{FE0F}\""), ["str:\u{3164}a\u{FE0F}"]);
        assert_eq!(tokens("// a\u{FE0F}\u{034F}"), ["comment: a\u{FE0F}\u{034F}"]);
        assert_eq!(tokens("/// \u{200B}"), ["doc: \u{200B}"]);
    }

    #[test]
    fn digits_of_a_name_are_ascii() {
        assert_eq!(error_code("a\u{0663}"), Code::Lc1001);
    }

    #[test]
    fn a_unit_may_start_with_a_degree_sign() {
        assert_eq!(tokens("20 \u{00B0}C"), ["int:20", "degree:\u{00B0}C"]);
        assert_eq!(tokens("\u{00B0}F/s"), ["degree:\u{00B0}F", "Slash", "name:s"]);
        for bad in ["\u{00B0} C", "\u{00B0}1", "\u{00B0}\n"] {
            assert_eq!(error_code(bad), Code::Lc1001, "{bad:?}");
        }
    }

    #[test]
    fn a_file_cut_after_a_degree_sign_is_truncated() {
        assert_eq!(error_code("\u{00B0}"), Code::Lc1002);
        assert_eq!(error_code("20 \u{00B0}"), Code::Lc1002);
    }

    #[test]
    fn numbers_keep_their_text() {
        assert_eq!(tokens("1 1_000 007"), ["int:1", "int:1_000", "int:007"]);
        assert_eq!(
            tokens("1.5 1_000.5_0 1e3 1E3 1e-3 1.5e3 1.5E-3 1e999"),
            ["float:1.5", "float:1_000.5_0", "float:1e3", "float:1E3", "float:1e-3", "float:1.5e3", "float:1.5E-3", "float:1e999",]
        );
    }

    #[test]
    fn what_is_not_quite_a_number() {
        assert_eq!(tokens("1..5"), ["int:1", "DotDot", "int:5"]);
        assert_eq!(tokens("1..=5"), ["int:1", "DotDot", "Equals", "int:5"]);
        assert_eq!(tokens("1."), ["int:1", "Dot"]);
        assert_eq!(tokens("1.a"), ["int:1", "Dot", "name:a"]);
        assert_eq!(tokens("5e"), ["int:5", "name:e"]);
        assert_eq!(tokens("5e-"), ["int:5", "name:e", "Minus"]);
        assert_eq!(tokens("5ms"), ["int:5", "name:ms"]);
        assert_eq!(tokens("1.5e"), ["float:1.5", "name:e"]);
        assert_eq!(tokens("-5"), ["Minus", "int:5"]);
    }

    #[test]
    fn strings_keep_their_text() {
        assert_eq!(tokens(r#""" "a b" "\" \\ \n \t \r""#), ["str:", "str:a b", r#"str:\" \\ \n \t \r"#]);
        assert_eq!(tokens("\"// no comment\""), ["str:// no comment"]);
    }

    #[test]
    fn a_bad_string_is_lc1003() {
        for bad in ["\"", "\"abc", "\"abc\n\"", "\"abc\r\n\"", "\"\\", "\"\\\n", "\"\\q\"", "\"\\\u{00E9}\""] {
            assert_eq!(error_code(bad), Code::Lc1003, "{bad:?}");
        }
    }

    #[test]
    fn comments_and_documentation_keep_their_text() {
        assert_eq!(tokens("// a\n"), ["comment: a", "Newline"]);
        assert_eq!(tokens("//"), ["comment:"]);
        assert_eq!(tokens("/// a"), ["doc: a"]);
        assert_eq!(tokens("////a"), ["doc:/a"]);
        assert_eq!(tokens("a // b \" c\nd"), ["name:a", "comment: b \" c", "Newline", "name:d"]);
        assert_eq!(tokens("// a  \r\n"), ["comment: a  ", "Newline"]);
    }

    #[test]
    fn every_symbol_of_the_grammar() {
        assert_eq!(
            tokens("( ) [ ] { } , : ; . .. ... = -> => # _ < > * \u{00B7} / ^ -"),
            [
                "LeftParen",
                "RightParen",
                "LeftBracket",
                "RightBracket",
                "LeftBrace",
                "RightBrace",
                "Comma",
                "Colon",
                "Semicolon",
                "Dot",
                "DotDot",
                "Ellipsis",
                "Equals",
                "Arrow",
                "FatArrow",
                "Hash",
                "Underscore",
                "Less",
                "Greater",
                "Star",
                "MiddleDot",
                "Slash",
                "Caret",
                "Minus",
            ]
        );
    }

    #[test]
    fn line_endings_are_tokens_in_both_forms() {
        assert_eq!(tokens("a\nb\r\nc"), ["name:a", "Newline", "name:b", "Newline", "name:c"]);
        assert_eq!(error_code("a\rb"), Code::Lc1001);
    }

    #[test]
    fn spans_count_lines_and_code_points() {
        let (mut names, mut report) = (Interner::new(), Report::new(10));
        let spans: Vec<(u32, u32, u32, u32)> = Lexer::new("\u{00E9}\u{00E9} ab\n\tcd", &mut names, &mut report)
            .map(|item| {
                let span = item.unwrap().span;
                (span.start.line, span.start.column, span.end.line, span.end.column)
            })
            .collect();
        assert_eq!(spans, [(1, 1, 1, 3), (1, 4, 1, 6), (1, 6, 2, 1), (2, 2, 2, 4)]);
    }

    #[test]
    fn position_says_where_the_file_ends() {
        let (mut names, mut report) = (Interner::new(), Report::new(10));
        let mut lexer = Lexer::new("a\nbc", &mut names, &mut report);
        while lexer.next().is_some() {}
        assert_eq!(lexer.position(), Position { line: 2, column: 3 });
    }

    fn spans_of(text: &str) -> Vec<(u32, u32, u32, u32)> {
        let (mut names, mut report) = (Interner::new(), Report::new(10));
        Lexer::new(text, &mut names, &mut report)
            .map(|item| {
                let span = item.unwrap().span;
                (span.start.line, span.start.column, span.end.line, span.end.column)
            })
            .collect()
    }

    fn error_span(text: &str) -> (u32, u32, u32, u32) {
        let span = lex(text).error.unwrap().span.unwrap();
        (span.start.line, span.start.column, span.end.line, span.end.column)
    }

    #[test]
    fn spans_around_line_endings_and_comments() {
        assert_eq!(spans_of("a\r\nb"), [(1, 1, 1, 2), (1, 2, 2, 1), (2, 1, 2, 2)]);
        assert_eq!(spans_of("// ab\nc"), [(1, 1, 1, 6), (1, 6, 2, 1), (2, 1, 2, 2)]);
        assert_eq!(spans_of("/// ab\r\nc"), [(1, 1, 1, 7), (1, 7, 2, 1), (2, 1, 2, 2)]);
        assert_eq!(spans_of("\"ab\" c"), [(1, 1, 1, 5), (1, 6, 1, 7)]);
    }

    #[test]
    fn spans_of_errors() {
        assert_eq!(error_span("x \"ab"), (1, 3, 1, 6), "an unterminated string, from its quote");
        assert_eq!(error_span("\"ab\n"), (1, 1, 1, 4));
        assert_eq!(error_span("\"\\"), (1, 1, 1, 3));
        assert_eq!(error_span("\"a\\q\""), (1, 3, 1, 4), "an unknown escape, at its backslash");
        assert_eq!(error_span("x \u{00B0}"), (1, 3, 1, 4));
        assert_eq!(error_span("x \u{00B0}1"), (1, 3, 1, 4));
        assert_eq!(error_span("a\n  $"), (2, 3, 2, 4));
    }

    #[test]
    fn the_first_error_ends_the_tokens() {
        let lexed = lex("a $ b");
        assert_eq!(lexed.tokens, ["name:a"]);
        let span = lexed.error.unwrap().span.unwrap();
        assert_eq!((span.start.column, span.end.column), (3, 4));
    }

    #[test]
    fn a_bidirectional_control_in_a_string_or_a_comment_warns_once() {
        for text in ["\"a\u{202E}b\u{202E}\"", "// a\u{2066}b\u{2069}", "/// a\u{202A}"] {
            let lexed = lex(text);
            assert_eq!(lexed.error, None, "{text:?}");
            assert_eq!(lexed.report.warnings(), 1, "{text:?}");
            assert_eq!(lexed.report.diagnostics()[0].code(), Code::Lc0005);
        }
        let lexed = lex("\"ab\u{202E}\"");
        assert_eq!(lexed.report.diagnostics()[0].span.unwrap().start.column, 4);
        assert_eq!(lex("// \u{202E}\n// \u{202E}\n").report.warnings(), 2);
    }

    #[test]
    fn a_bidirectional_control_elsewhere_is_an_unexpected_character_and_no_warning() {
        for text in ["a\u{202E}b", "\u{202E}", "a \u{2066} b"] {
            let lexed = lex(text);
            assert_eq!(lexed.error.as_ref().map(Diagnostic::code), Some(Code::Lc1001), "{text:?}");
            assert_eq!(lexed.report.warnings(), 0, "{text:?}");
        }
    }

    #[test]
    fn a_message_never_shows_an_invisible_character() {
        for c in ['\u{202E}', '\u{200B}', '\u{00A0}', '\u{0301}'] {
            let text = format!("{c}");
            let message = lex(&text).error.unwrap().message;
            assert!(!message.contains(c), "{message}");
            assert!(message.contains("U+"), "{message}");
        }
        for c in ['\u{3164}', '\u{2800}', '\u{00E9}'] {
            let text = format!("\"\\{c}\"");
            let message = lex(&text).error.unwrap().message;
            assert!(!message.contains(c), "{message}");
            assert!(message.contains("U+"), "{message}");
        }
        assert!(lex("$").error.unwrap().message.contains("`$`"));
    }

    #[test]
    fn many_warnings_stay_bounded() {
        let text = "// \u{202E}\n".repeat(10_000);
        let (mut names, mut report) = (Interner::new(), Report::new(4));
        for item in Lexer::new(&text, &mut names, &mut report) {
            item.unwrap();
        }
        assert_eq!(report.warnings(), 10_000);
        assert!(report.diagnostics().len() <= 8);
    }

    #[test]
    fn a_long_line_is_linear() {
        // No clock: quadratic work here would take hours, and end at the time limit of the CI.
        let line = "a ".repeat(2_000_000);
        let (mut names, mut report) = (Interner::new(), Report::new(4));
        assert_eq!(Lexer::new(&line, &mut names, &mut report).count(), 2_000_000);
        let comment = format!("//{}", "x".repeat(8_000_000));
        assert_eq!(Lexer::new(&comment, &mut names, &mut report).count(), 1);
    }
}
