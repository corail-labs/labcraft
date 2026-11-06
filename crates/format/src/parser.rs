//! The parser: hand-written recursive descent on spec 14, over the lexer. It stops at the first syntax
//! error, as the lexer does: one mistake gives one diagnostic, and a bad file costs nothing more.
//!
//! **Line endings** are significant by context (14.1). `free` counts the enclosing places that ignore
//! them (parentheses, square brackets, the braces of a struct, a structure header); [`Parser::peek`] then
//! steps over them. Elsewhere they are tokens that a production must meet.
//!
//! **Comments** (5.2). Those stepped over wait in `pending` with their line, and the next item takes them.
//! When an item ends, what is pending sorts itself by line: before the end of the item, it goes before
//! the item; on the line of its end, it ends on that line; after it, it waits for the next item.
//! `atomic` turns items off inside a statement, where every comment goes before the statement.
//!
//! **Recursion** is bounded by `depth`: one counter for every kind of nesting (14.1), checked before each
//! recursive step. Tokens are read one at a time, since a file of 16 MiB can hold 8 million of them.

use labcraft_graph::{
    Arg, Arm, ArrayType, Attached, Binding, Body, Branching, Call, Carry, Clause, ClauseKind, Comment, CommentKind, Constant, Decl, EnumType,
    Exponent, Extern, Field, FieldInit, File, FnArg, FnType, For, ForIn, Function, Generic, Icon, IconItem, IconShape, Id, IntBound, Label, Layout,
    LayoutItem, LayoutKind, Loop, NamedType, NodeRef, Numeral, NumeralKind, Operand, OrderRef, Output, Panel, PanelDirection, PanelItem, Param, Path,
    Pattern, Point, Rhs, Signature, SourceMap, Statement, StatementKind, StructLit, StructType, Target, TerminalRef, TypeDecl, TypeExpr, UnitExpr,
    UnitOp, UnitTerm, Use, VariantDecl, View, Yield,
};
use labcraft_types::{Code, Diagnostic, Fix, Interner, Limits, Name, Position, Report, Span};

use crate::lexer::{Keyword as K, Lexer, Token, TokenKind as T};

type Parse<X> = Result<X, Box<Diagnostic>>;

/// A comment that no item has taken yet, with its line.
struct Pending {
    comment: Comment,
    line: u32,
}

/// Which statements a body can hold (R-6).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Function,
    Arm,
    For,
    Loop,
}

/// The items of a split list, which carry their comments.
trait Commented {
    fn comments(&mut self) -> &mut Attached;
}

macro_rules! commented {
    ($($item:ty),*) => {
        $(impl Commented for $item {
            fn comments(&mut self) -> &mut Attached {
                &mut self.comments
            }
        })*
    };
}

commented!(Param, Output, Field, VariantDecl);

struct Parser<'a, 'b> {
    lexer: Lexer<'a, 'b>,
    /// The next token, once read: `Some(None)` at the end of the file.
    ahead: Option<Option<Token<'a>>>,
    /// Where the last token consumed ends; line endings and comments don't count.
    last_end: Position,
    pending: Vec<Pending>,
    free: u32,
    atomic: u32,
    /// In a default value, where a call inside a value has no line before it to move to (R-21).
    in_default: bool,
    depth: u32,
    max_depth: u32,
    map: SourceMap,
}

/// The model of a file, or its first syntax error. Warnings go to `report`.
pub(crate) fn parse_file(text: &str, names: &mut Interner, report: &mut Report, limits: &Limits) -> (Parse<File>, SourceMap) {
    let mut parser = Parser {
        lexer: Lexer::new(text, names, report),
        ahead: None,
        last_end: Position { line: 1, column: 1 },
        pending: Vec::new(),
        free: 0,
        atomic: 0,
        in_default: false,
        depth: 0,
        max_depth: limits.max_nesting(),
        map: SourceMap::default(),
    };
    let file = parser.file();
    (file, parser.map)
}

fn one_column(start: Position) -> Span {
    Span { start, end: Position { line: start.line, column: start.column.saturating_add(1) } }
}

fn failure(code: Code, message: String, span: Span) -> Box<Diagnostic> {
    Box::new(Diagnostic::new(code, message, "").with_span(span))
}

impl<'a> Parser<'a, '_> {
    // Tokens.

    /// The next token as the file has it, line endings and comments included.
    fn raw(&mut self) -> Parse<Option<Token<'a>>> {
        if let Some(token) = self.ahead {
            return Ok(token);
        }
        let token = self.lexer.next().transpose()?;
        self.ahead = Some(token);
        Ok(token)
    }

    fn drop_raw(&mut self) {
        self.ahead = None;
    }

    fn keep_comment(&mut self, kind: CommentKind, text: &str, line: u32) {
        self.pending.push(Pending { comment: comment_of(kind, text), line });
        self.drop_raw();
    }

    /// The next token that counts here: where line endings are ignored, it steps over them and comments.
    fn peek(&mut self) -> Parse<Option<Token<'a>>> {
        loop {
            let token = self.raw()?;
            match token.map(|token| (token.kind, token.span)) {
                Some((T::Newline, _)) if self.free > 0 => self.drop_raw(),
                Some((T::Comment(text), span)) if self.free > 0 => self.keep_comment(CommentKind::Line, text, span.start.line),
                Some((T::Doc(_), span)) if self.free > 0 => return Err(misplaced_documentation(span)),
                _ => return Ok(token),
            }
        }
    }

    fn peek_kind(&mut self) -> Parse<Option<T<'a>>> {
        Ok(self.peek()?.map(|token| token.kind))
    }

    fn at(&mut self, kind: T<'a>) -> Parse<bool> {
        Ok(self.peek_kind()? == Some(kind))
    }

    /// Takes the token that `peek` or `raw` has just seen.
    fn bump(&mut self) {
        if let Some(Some(token)) = self.ahead.take() {
            self.last_end = token.span.end;
        }
    }

    fn eat(&mut self, kind: T<'a>) -> Parse<bool> {
        let found = self.at(kind)?;
        if found {
            self.bump();
        }
        Ok(found)
    }

    fn expect(&mut self, kind: T<'a>, what: &str) -> Parse<Token<'a>> {
        match self.peek()? {
            Some(token) if token.kind == kind => {
                self.bump();
                Ok(token)
            }
            _ => Err(self.unexpected(what)),
        }
    }

    /// Where the next token starts, or the file ends.
    fn start(&mut self) -> Parse<Position> {
        Ok(self.peek()?.map_or_else(|| self.lexer.position(), |token| token.span.start))
    }

    fn resolve(&mut self, name: Name) -> &str {
        self.lexer.names().resolve(name)
    }

    fn intern(&mut self, text: &str) -> Name {
        self.lexer.names().intern(text)
    }

    /// Takes a place in the source map, from `start` to the last token consumed.
    fn place(&mut self, start: Position) -> Id {
        self.map.add(Span { start, end: self.last_end })
    }

    // Errors.

    /// Never shows a string or a comment: only the names, which are known to be visible, and numbers.
    fn describe(&mut self, kind: T<'a>) -> String {
        match kind {
            T::Name(name) | T::DegreeName(name) => format!("`{}`", self.resolve(name)),
            T::Keyword(keyword) => format!("`{}`", keyword.as_str()),
            T::Int(text) | T::Float(text) => format!("`{text}`"),
            T::String(_) => "a string".to_owned(),
            T::Comment(_) | T::Newline => "the end of the line".to_owned(),
            T::Doc(_) => "a documentation comment".to_owned(),
            T::LeftParen => "`(`".to_owned(),
            T::RightParen => "`)`".to_owned(),
            T::LeftBracket => "`[`".to_owned(),
            T::RightBracket => "`]`".to_owned(),
            T::LeftBrace => "`{`".to_owned(),
            T::RightBrace => "`}`".to_owned(),
            T::Comma => "`,`".to_owned(),
            T::Colon => "`:`".to_owned(),
            T::Semicolon => "`;`".to_owned(),
            T::Dot => "`.`".to_owned(),
            T::DotDot => "`..`".to_owned(),
            T::Ellipsis => "`...`".to_owned(),
            T::Equals => "`=`".to_owned(),
            T::Arrow => "`->`".to_owned(),
            T::FatArrow => "`=>`".to_owned(),
            T::Hash => "`#`".to_owned(),
            T::Underscore => "`_`".to_owned(),
            T::Less => "`<`".to_owned(),
            T::Greater => "`>`".to_owned(),
            T::Star => "`*`".to_owned(),
            T::MiddleDot => "`\u{00B7}`".to_owned(),
            T::Slash => "`/`".to_owned(),
            T::Caret => "`^`".to_owned(),
            T::Minus => "`-`".to_owned(),
        }
    }

    /// `LC1002` at the end of the file, otherwise `LC1001`, which names what was expected.
    fn unexpected(&mut self, expected: &str) -> Box<Diagnostic> {
        match self.peek() {
            Err(diagnostic) => diagnostic,
            Ok(None) => {
                let message = format!("file ends where {expected} was expected");
                failure(Code::Lc1002, message, one_column(self.lexer.position()))
            }
            Ok(Some(token)) => {
                let found = self.describe(token.kind);
                failure(Code::Lc1001, format!("expected {expected}, found {found}"), token.span)
            }
        }
    }

    /// For a place where a name is one of the things expected: a reserved word there is `LC1005`.
    fn unexpected_name(&mut self, expected: &str) -> Box<Diagnostic> {
        match self.peek() {
            Ok(Some(Token { kind: T::Keyword(keyword), span })) => reserved_word(keyword, expected, span),
            _ => self.unexpected(expected),
        }
    }

    fn enter(&mut self) -> Parse<()> {
        if self.depth >= self.max_depth {
            let start = self.start()?;
            let message = format!("nested deeper than the limit of {}", self.max_depth);
            return Err(failure(Code::Lc0007, message, one_column(start)));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // Comments and line endings.

    fn take_pending(&mut self) -> Vec<Comment> {
        self.pending.drain(..).map(|pending| pending.comment).collect()
    }

    /// Steps over what lies between two lines. `documentation` says that `///` is allowed here.
    fn skip_lines(&mut self, documentation: bool) -> Parse<()> {
        loop {
            match self.raw()?.map(|token| (token.kind, token.span)) {
                Some((T::Newline, _)) => self.drop_raw(),
                Some((T::Comment(text), span)) => self.keep_comment(CommentKind::Line, text, span.start.line),
                Some((T::Doc(text), span)) if documentation => self.keep_comment(CommentKind::Doc, text, span.start.line),
                Some((T::Doc(_), span)) => return Err(misplaced_documentation(span)),
                _ => return Ok(()),
            }
        }
    }

    /// The comment that ends this line, if it is the next thing in the file.
    fn line_comment(&mut self, line: u32) -> Parse<Option<Comment>> {
        if let Some(Token { kind: T::Comment(text), span }) = self.raw()?
            && span.start.line == line
        {
            self.drop_raw();
            return Ok(Some(comment_of(CommentKind::Line, text)));
        }
        Ok(None)
    }

    fn begin_item(&mut self) -> Vec<Comment> {
        if self.atomic > 0 { Vec::new() } else { self.take_pending() }
    }

    /// What is pending sorts itself by line (see the top of this file).
    fn end_item(&mut self, mut before: Vec<Comment>) -> Parse<Attached> {
        if self.atomic > 0 {
            return Ok(Attached::default());
        }
        let end = self.last_end.line;
        let mut line = None;
        if !self.pending.is_empty() {
            let mut later = Vec::new();
            for pending in std::mem::take(&mut self.pending) {
                if pending.line < end {
                    before.push(pending.comment);
                } else if pending.line == end {
                    line = Some(pending.comment);
                } else {
                    later.push(pending);
                }
            }
            self.pending = later;
        }
        if line.is_none() {
            line = self.line_comment(end)?;
        }
        Ok(Attached { before: before.into_boxed_slice(), line: line.map(Box::new) })
    }

    /// The comma after an item of a list; the comment after that comma, on the line of the item, is the item's.
    fn comma_after(&mut self, comments: &mut Attached, item_line: u32) -> Parse<bool> {
        let more = self.eat(T::Comma)?;
        if more && comments.line.is_none() && self.atomic == 0 {
            comments.line = self.line_comment(item_line)?.map(Box::new);
        }
        Ok(more)
    }

    /// A statement or an item ends here: two of them never share a line. A `}` may close the line, and
    /// the end of a file is for the caller to judge.
    fn end_line(&mut self, what: &str) -> Parse<()> {
        let Some(token) = self.raw()? else {
            return Ok(());
        };
        match token.kind {
            T::Newline => self.drop_raw(),
            T::RightBrace => {}
            T::Doc(_) => return Err(misplaced_documentation(token.span)),
            // Looking for what could continue a type has already stepped over the line ending.
            _ if token.span.start.line > self.last_end.line => {}
            _ => {
                let found = self.describe(token.kind);
                return Err(failure(Code::Lc1001, format!("expected the end of the line after {what}, found {found}"), token.span));
            }
        }
        Ok(())
    }

    /// Opens a place that ignores line endings, whose opening token is next.
    fn open(&mut self, kind: T<'a>, what: &str) -> Parse<()> {
        self.expect(kind, what)?;
        self.free += 1;
        Ok(())
    }

    /// Closes it. Line endings after the closing token count again, as they do where it opened.
    fn close(&mut self, kind: T<'a>, what: &str) -> Parse<()> {
        match self.peek()? {
            Some(token) if token.kind == kind => {
                self.free = self.free.saturating_sub(1);
                self.bump();
                Ok(())
            }
            _ => Err(self.unexpected(what)),
        }
    }

    // Names and paths.

    fn name(&mut self, expected: &str) -> Parse<(Name, Span)> {
        match self.peek()? {
            Some(Token { kind: T::Name(name), span }) => {
                self.bump();
                Ok((name, span))
            }
            _ => Err(self.unexpected_name(expected)),
        }
    }

    fn path(&mut self, expected: &str) -> Parse<Path> {
        let (first, span) = self.name(expected)?;
        self.path_from(first, span.start)
    }

    fn path_from(&mut self, first: Name, start: Position) -> Parse<Path> {
        let mut rest = Vec::new();
        while self.eat(T::Dot)? {
            rest.push(self.name("a name after `.`")?.0);
        }
        Ok(Path { id: self.place(start), first, rest: rest.into_boxed_slice() })
    }

    /// Whether the next token is this contextual word (14.1).
    fn at_word(&mut self, word: &str) -> Parse<bool> {
        Ok(match self.peek_kind()? {
            Some(T::Name(name)) => self.resolve(name) == word,
            _ => false,
        })
    }

    fn eat_word(&mut self, word: &str) -> Parse<bool> {
        let found = self.at_word(word)?;
        if found {
            self.bump();
        }
        Ok(found)
    }

    fn string(&mut self, expected: &str) -> Parse<Box<str>> {
        match self.peek_kind()? {
            Some(T::String(text)) => {
                self.bump();
                Ok(text.into())
            }
            _ => Err(self.unexpected(expected)),
        }
    }

    // The file.

    /// Spec 14.2: nothing before the header but spaces, and nothing after it on its line but a comment. A
    /// lexical error on this line is text that is not a header too.
    fn header(&mut self) -> Parse<(u32, Option<Comment>)> {
        let not_a_header = |span: Span| failure(Code::Lc0101, "the first line must be the header `labcraft 1`".to_owned(), span);
        let first = self.raw().map_err(|diagnostic| not_a_header(diagnostic.span.unwrap_or_else(|| one_column(Position { line: 1, column: 1 }))))?;
        match first {
            Some(Token { kind: T::Keyword(K::Labcraft), .. }) => self.bump(),
            Some(token) => return Err(not_a_header(token.span)),
            None => return Err(not_a_header(one_column(self.lexer.position()))),
        }
        let end_of_labcraft = self.last_end;
        let version = self.raw().map_err(|diagnostic| not_a_header(diagnostic.span.unwrap_or_else(|| one_column(end_of_labcraft))))?;
        let (text, version_span) = match version {
            Some(Token { kind: T::Int(text), span }) => {
                self.bump();
                (text, span)
            }
            Some(token) => return Err(not_a_header(token.span)),
            None => return Err(not_a_header(one_column(self.lexer.position()))),
        };
        let after = self.raw().map_err(|diagnostic| not_a_header(diagnostic.span.unwrap_or(version_span)))?;
        let comment = match after {
            None => None,
            Some(Token { kind: T::Newline, .. }) => {
                self.drop_raw();
                None
            }
            Some(Token { kind: T::Comment(comment), .. }) => {
                self.drop_raw();
                Some(comment_of(CommentKind::Line, comment))
            }
            Some(token) => return Err(not_a_header(token.span)),
        };
        if text != "1" {
            // The advice comes first, since a report cuts a long text.
            let advice = if is_one_written_otherwise(text) { "write" } else { "this tool reads" };
            let message = format!("{advice} `labcraft 1`: format version {text} is not supported");
            return Err(failure(Code::Lc0102, message, version_span));
        }
        Ok((1, comment))
    }

    fn file(&mut self) -> Parse<File> {
        let (version, header_comment) = self.header()?;
        let mut use_comments = Vec::new();
        let mut uses = Vec::new();
        loop {
            self.skip_lines(true)?;
            if !self.at(T::Keyword(K::Use))? {
                break;
            }
            if self.pending.iter().any(|pending| pending.comment.kind == CommentKind::Doc) {
                return Err(self.unexpected("the declaration after a documentation comment"));
            }
            use_comments.append(&mut self.take_pending());
            uses.push(self.use_line()?);
        }
        let decl = self.declaration()?;
        self.end_line("the declaration")?;
        let panel = self.section(K::Panel, Self::panel)?;
        let layout = self.section(K::Layout, Self::layout)?;
        let icon = self.section(K::Icon, Self::icon)?;
        self.skip_lines(false)?;
        if self.raw()?.is_some() {
            return Err(
                self.unexpected("the end of the file, since a file has one declaration and its sections in this order: `panel`, `layout`, `icon`")
            );
        }
        Ok(File { version, header_comment, use_comments, uses, decl, panel, layout, icon, end: self.take_pending() })
    }

    fn use_line(&mut self) -> Parse<Use> {
        let start = self.start()?;
        self.bump();
        let path = self.path("the name of a library")?;
        let id = self.place(start);
        let line = self.line_comment(self.last_end.line)?;
        self.end_line("a `use`")?;
        Ok(Use { id, path, line })
    }

    fn section<S>(&mut self, keyword: K, parse: fn(&mut Self) -> Parse<S>) -> Parse<Option<S>> {
        self.skip_lines(false)?;
        if !self.at(T::Keyword(keyword))? {
            return Ok(None);
        }
        let section = parse(self)?;
        self.end_line("a section")?;
        Ok(Some(section))
    }

    fn declaration(&mut self) -> Parse<Decl> {
        let before = self.take_pending();
        let start = self.start()?;
        // A declaration ignores line endings up to the `{` of its body (14.1 ends only statements).
        self.free += 1;
        match self.peek_kind()? {
            Some(T::Keyword(K::Fn)) => self.function(before, start),
            Some(T::Keyword(K::Extern)) => self.extern_function(before, start),
            Some(T::Keyword(K::Type)) => self.type_declaration(before, start),
            _ => Err(self.unexpected("a declaration: `fn`, `extern` or `type`")),
        }
    }

    fn function(&mut self, mut before: Vec<Comment>, start: Position) -> Parse<Decl> {
        self.bump();
        let (name, _) = self.name("the name of the function")?;
        let mut signature = self.signature(&mut before)?;
        if !self.at(T::LeftBrace)? {
            return Err(self.unexpected("`{`"));
        }
        signature.outputs_end.append(&mut self.take_pending());
        self.free = self.free.saturating_sub(1);
        self.bump();
        let open = self.line_comment(self.last_end.line)?;
        let body = self.body(Place::Function)?;
        self.bump();
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Decl::Function(Function { id, comments, name, signature, open, body }))
    }

    fn extern_function(&mut self, mut before: Vec<Comment>, start: Position) -> Parse<Decl> {
        self.bump();
        let (language, _) = self.name("the language of the external function")?;
        let args = self.args(false)?;
        self.expect(T::Keyword(K::Fn), "`fn`")?;
        let (name, _) = self.name("the name of the function")?;
        let signature = self.signature(&mut before)?;
        self.free = self.free.saturating_sub(1);
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Decl::Extern(Extern { id, comments, language, args, name, signature }))
    }

    fn type_declaration(&mut self, before: Vec<Comment>, start: Position) -> Parse<Decl> {
        self.bump();
        let (name, _) = self.name("the name of the type")?;
        let generics = self.generics()?;
        self.expect(T::Equals, "`=`")?;
        let ty = self.type_expr()?;
        self.free = self.free.saturating_sub(1);
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Decl::Type(TypeDecl { id, comments, name, generics, ty }))
    }

    fn generics(&mut self) -> Parse<Vec<Generic>> {
        let mut generics = Vec::new();
        if !self.at(T::Less)? {
            return Ok(generics);
        }
        self.expect(T::Less, "`<`")?;
        loop {
            let start = self.start()?;
            let (name, _) = self.name("the name of a type variable")?;
            let bound = if self.eat(T::Colon)? { Some(self.path("a bound")?) } else { None };
            let id = self.place(start);
            generics.push(Generic { id, name, bound });
            if !self.eat(T::Comma)? {
                break;
            }
        }
        self.expect(T::Greater, "`,` or `>`")?;
        Ok(generics)
    }

    /// What comes before the parameters is no part of them: its comments go before the declaration.
    fn signature(&mut self, before: &mut Vec<Comment>) -> Parse<Signature> {
        let generics = self.generics()?;
        if !self.at(T::LeftParen)? {
            return Err(self.unexpected("`(`"));
        }
        before.append(&mut self.take_pending());
        self.open(T::LeftParen, "`(`")?;
        let (params, params_end) = self.commented_list(T::RightParen, "`,` or `)`", true, Self::param)?;
        self.expect(T::Arrow, "`->`")?;
        self.open(T::LeftParen, "`(`")?;
        let (outputs, outputs_end) = self.commented_list(T::RightParen, "`,` or `)`", true, Self::output)?;
        Ok(Signature { generics, params, params_end, outputs, outputs_end })
    }

    /// A split list, already opened, up to its closing token: a comma may end it, and each item takes
    /// its comments. What remains at the end stays at the level of the items.
    fn commented_list<I: Commented>(
        &mut self,
        close: T<'a>,
        what: &str,
        may_be_empty: bool,
        parse: fn(&mut Self) -> Parse<I>,
    ) -> Parse<(Vec<I>, Vec<Comment>)> {
        let mut items = Vec::new();
        if !(may_be_empty && self.at(close)?) {
            loop {
                let before = self.begin_item();
                let mut item = parse(self)?;
                let item_line = self.last_end.line;
                let mut comments = self.end_item(before)?;
                let more = self.comma_after(&mut comments, item_line)?;
                *item.comments() = comments;
                items.push(item);
                if !more || self.at(close)? {
                    break;
                }
            }
        }
        if !self.at(close)? {
            return Err(self.unexpected(what));
        }
        let end = if self.atomic > 0 { Vec::new() } else { self.take_pending() };
        self.close(close, what)?;
        Ok((items, end))
    }

    /// The list between the braces of a struct or an enum type. Comments met before its `{` lie inside
    /// the item around it, not in this list: they wait until the list ends.
    fn braced_list<I: Commented>(&mut self, parse: fn(&mut Self) -> Parse<I>) -> Parse<(Vec<I>, Vec<Comment>)> {
        self.open(T::LeftBrace, "`{`")?;
        let around = std::mem::take(&mut self.pending);
        let list = self.commented_list(T::RightBrace, "`,` or `}`", false, parse)?;
        let later = std::mem::replace(&mut self.pending, around);
        self.pending.extend(later);
        Ok(list)
    }

    fn param(&mut self) -> Parse<Param> {
        let start = self.start()?;
        let (name, _) = self.name("the name of a parameter")?;
        self.expect(T::Colon, "`:`")?;
        let ty = self.type_expr()?;
        let mut default = None;
        let mut variadic = false;
        if self.eat(T::Equals)? {
            default = Some(self.constant()?);
        } else if self.eat(T::Ellipsis)? {
            variadic = true;
        }
        let id = self.place(start);
        Ok(Param { id, comments: Attached::default(), name, ty, default, variadic })
    }

    /// `constant`: a path, a call and a struct construction read as in a statement, and `check` says which are constants (R-21).
    fn constant(&mut self) -> Parse<Constant> {
        self.in_default = true;
        let constant = self.constant_inside();
        self.in_default = false;
        constant
    }

    fn constant_inside(&mut self) -> Parse<Constant> {
        let start = self.start()?;
        if !matches!(self.peek_kind()?, Some(T::Name(_))) {
            return Ok(Constant::Operand(self.operand()?));
        }
        let path = self.path("a constant")?;
        match self.peek_kind()? {
            Some(T::LeftParen | T::Less) => Ok(Constant::Call(Box::new(self.call(path, start)?))),
            Some(T::LeftBrace) => Ok(Constant::Struct(Box::new(self.struct_literal(path, start)?))),
            _ => Ok(Constant::Operand(Operand::Path(path))),
        }
    }

    fn output(&mut self) -> Parse<Output> {
        let start = self.start()?;
        let (name, _) = self.name("the name of an output")?;
        self.expect(T::Colon, "`:`")?;
        let ty = self.type_expr()?;
        let id = self.place(start);
        Ok(Output { id, comments: Attached::default(), name, ty })
    }

    // Types.

    fn type_expr(&mut self) -> Parse<TypeExpr> {
        match self.peek_kind()? {
            Some(T::Name(_)) => {
                let start = self.start()?;
                let path = self.path("a type")?;
                Ok(TypeExpr::Named(self.named_type(path, start)?))
            }
            Some(T::LeftBracket) => self.array_type(),
            Some(T::Keyword(K::Struct)) => self.struct_type(),
            Some(T::Keyword(K::Enum)) => self.enum_type(),
            Some(T::Keyword(K::Fn)) => self.fn_type(),
            _ => Err(self.unexpected_name("a type")),
        }
    }

    fn named_type(&mut self, path: Path, start: Position) -> Parse<NamedType> {
        let mut args = Vec::new();
        if self.at(T::Less)? {
            self.enter()?;
            self.bump();
            args = self.types()?;
            self.expect(T::Greater, "`,` or `>`")?;
            self.leave();
        }
        let mut unit = None;
        if self.at(T::LeftBracket)? {
            self.open(T::LeftBracket, "`[`")?;
            unit = Some(self.unit_expr()?);
            self.close(T::RightBracket, "`]`")?;
        }
        let id = self.place(start);
        Ok(NamedType { id, path, args, unit })
    }

    /// One type or more.
    fn types(&mut self) -> Parse<Vec<TypeExpr>> {
        let mut types = vec![self.type_expr()?];
        while self.eat(T::Comma)? {
            types.push(self.type_expr()?);
        }
        Ok(types)
    }

    fn array_type(&mut self) -> Parse<TypeExpr> {
        let start = self.start()?;
        self.enter()?;
        self.open(T::LeftBracket, "`[`")?;
        let element = Box::new(self.type_expr()?);
        let mut dimensions = None;
        if self.eat(T::Semicolon)? {
            let Some(T::Int(text)) = self.peek_kind()? else {
                return Err(self.unexpected("a number of dimensions"));
            };
            self.bump();
            dimensions = Some(self.intern(text));
        }
        self.close(T::RightBracket, "`;` or `]`")?;
        self.leave();
        let id = self.place(start);
        Ok(TypeExpr::Array(ArrayType { id, element, dimensions }))
    }

    fn struct_type(&mut self) -> Parse<TypeExpr> {
        let start = self.start()?;
        self.enter()?;
        self.bump();
        let (fields, end) = self.braced_list(Self::field)?;
        self.leave();
        let id = self.place(start);
        Ok(TypeExpr::Struct(StructType { id, fields, end }))
    }

    fn field(&mut self) -> Parse<Field> {
        let start = self.start()?;
        let (name, _) = self.name("the name of a field")?;
        self.expect(T::Colon, "`:`")?;
        let ty = self.type_expr()?;
        let id = self.place(start);
        Ok(Field { id, comments: Attached::default(), name, ty })
    }

    fn enum_type(&mut self) -> Parse<TypeExpr> {
        let start = self.start()?;
        self.enter()?;
        self.bump();
        let (variants, end) = self.braced_list(Self::variant)?;
        self.leave();
        let id = self.place(start);
        Ok(TypeExpr::Enum(EnumType { id, variants, end }))
    }

    fn variant(&mut self) -> Parse<VariantDecl> {
        let start = self.start()?;
        let (name, _) = self.name("the name of a variant")?;
        let mut payload = Vec::new();
        if self.at(T::LeftParen)? {
            self.open(T::LeftParen, "`(`")?;
            payload = self.types()?;
            self.close(T::RightParen, "`,` or `)`")?;
        }
        let id = self.place(start);
        Ok(VariantDecl { id, comments: Attached::default(), name, payload })
    }

    fn fn_type(&mut self) -> Parse<TypeExpr> {
        let start = self.start()?;
        self.enter()?;
        self.bump();
        self.open(T::LeftParen, "`(`")?;
        let mut params = Vec::new();
        if !self.at(T::RightParen)? {
            loop {
                params.push(self.fn_arg()?);
                if !self.eat(T::Comma)? {
                    break;
                }
            }
        }
        self.close(T::RightParen, "`,` or `)`")?;
        self.expect(T::Arrow, "`->`")?;
        self.open(T::LeftParen, "`(`")?;
        let mut outputs = Vec::new();
        if !self.at(T::RightParen)? {
            outputs = self.types()?;
        }
        self.close(T::RightParen, "`,` or `)`")?;
        self.leave();
        let id = self.place(start);
        Ok(TypeExpr::Fn(FnType { id, params, outputs }))
    }

    /// `visa.session`, or `name: visa.session`.
    fn fn_arg(&mut self) -> Parse<FnArg> {
        let start = self.start()?;
        if !matches!(self.peek_kind()?, Some(T::Name(_))) {
            let ty = self.type_expr()?;
            let id = self.place(start);
            return Ok(FnArg { id, name: None, ty });
        }
        let path = self.path("a type")?;
        if path.rest.is_empty() && self.eat(T::Colon)? {
            let ty = self.type_expr()?;
            let id = self.place(start);
            return Ok(FnArg { id, name: Some(path.first), ty });
        }
        let ty = TypeExpr::Named(self.named_type(path, start)?);
        let id = self.place(start);
        Ok(FnArg { id, name: None, ty })
    }

    // Units.

    fn at_unit_operator(&mut self) -> Parse<Option<UnitOp>> {
        Ok(match self.peek_kind()? {
            Some(T::Star) => Some(UnitOp::Star),
            Some(T::MiddleDot) => Some(UnitOp::MiddleDot),
            Some(T::Slash) => Some(UnitOp::Slash),
            _ => None,
        })
    }

    fn exponent(&mut self) -> Parse<Option<Exponent>> {
        if !self.eat(T::Caret)? {
            return Ok(None);
        }
        let negative = self.eat(T::Minus)?;
        let Some(T::Int(text)) = self.peek_kind()? else {
            return Err(self.unexpected("an integer exponent"));
        };
        self.bump();
        Ok(Some(Exponent { negative, digits: self.intern(text) }))
    }

    fn unit_term(&mut self) -> Parse<UnitTerm> {
        match self.peek_kind()? {
            Some(T::Name(name) | T::DegreeName(name)) => {
                self.bump();
                Ok(UnitTerm::Named { name, exponent: self.exponent()? })
            }
            Some(T::Int("1")) => {
                self.bump();
                Ok(UnitTerm::One { exponent: self.exponent()? })
            }
            Some(T::LeftParen) => {
                self.enter()?;
                self.open(T::LeftParen, "`(`")?;
                let inner = self.unit_expr()?;
                self.close(T::RightParen, "an operator of a unit, or `)`")?;
                self.leave();
                Ok(UnitTerm::Group(Box::new(inner)))
            }
            _ => Err(self.unexpected_name("a unit")),
        }
    }

    fn unit_expr(&mut self) -> Parse<UnitExpr> {
        let start = self.start()?;
        let first = self.unit_term()?;
        self.unit_rest(first, start)
    }

    fn unit_rest(&mut self, first: UnitTerm, start: Position) -> Parse<UnitExpr> {
        let mut rest = Vec::new();
        while let Some(operator) = self.at_unit_operator()? {
            self.bump();
            rest.push((operator, self.unit_term()?));
        }
        let id = self.place(start);
        Ok(UnitExpr { id, first, rest })
    }

    /// Whether a unit starts here, right after a number (R-5).
    fn at_unit(&mut self) -> Parse<bool> {
        Ok(matches!(self.peek_kind()?, Some(T::Name(_) | T::DegreeName(_) | T::LeftParen | T::Int("1"))))
    }

    // Values.

    /// `[ "-" ] ( INT | FLOAT | "inf" ) | "nan"`, with no space after the minus.
    fn numeral(&mut self) -> Parse<Numeral> {
        let mut negative = false;
        if let Some(Token { kind: T::Minus, span }) = self.peek()? {
            self.bump();
            negative = true;
            if let Some(next) = self.raw()?
                && (next.span.start != span.end || matches!(next.kind, T::Newline | T::Comment(_) | T::Doc(_)))
            {
                return Err(failure(Code::Lc1001, "no space after a minus: write `-1`".to_owned(), span));
            }
        }
        let kind = match self.peek_kind()? {
            Some(T::Int(text)) => NumeralKind::Int(self.intern(text)),
            Some(T::Float(text)) => NumeralKind::Float(self.intern(text)),
            Some(T::Keyword(K::Inf)) => NumeralKind::Inf,
            Some(T::Keyword(K::Nan)) if !negative => NumeralKind::Nan,
            _ => return Err(self.unexpected("a number")),
        };
        self.bump();
        Ok(Numeral { negative, kind })
    }

    fn number_with_unit(&mut self, numeral: Numeral, start: Position) -> Parse<Operand> {
        let unit = if self.at_unit()? { Some(Box::new(self.unit_expr()?)) } else { None };
        let id = self.place(start);
        Ok(Operand::Number(id, numeral, unit))
    }

    /// `literal`: a value written directly. A name is not one, and the message says what is expected.
    fn literal(&mut self, expected: &str) -> Parse<Operand> {
        let start = self.start()?;
        match self.peek_kind()? {
            Some(T::Minus | T::Int(_) | T::Float(_) | T::Keyword(K::Inf | K::Nan)) => {
                let numeral = self.numeral()?;
                self.number_with_unit(numeral, start)
            }
            Some(T::String(text)) => {
                self.bump();
                let id = self.place(start);
                Ok(Operand::Text(id, text.into()))
            }
            Some(T::Keyword(K::True)) => self.word_literal(start, |id| Operand::Bool(id, true)),
            Some(T::Keyword(K::False)) => self.word_literal(start, |id| Operand::Bool(id, false)),
            Some(T::Keyword(K::None)) => self.word_literal(start, Operand::None),
            Some(T::LeftBracket) => {
                self.enter()?;
                self.open(T::LeftBracket, "`[`")?;
                let operand = self.array_rest(None, start)?;
                self.leave();
                Ok(operand)
            }
            _ => Err(self.unexpected(expected)),
        }
    }

    fn word_literal(&mut self, start: Position, make: impl FnOnce(Id) -> Operand) -> Parse<Operand> {
        self.bump();
        Ok(make(self.place(start)))
    }

    /// What is left of an array after its `[`, and after its first value if one is read already.
    fn array_rest(&mut self, first: Option<Operand>, start: Position) -> Parse<Operand> {
        let mut values = Vec::new();
        let read_more = match first {
            Some(first) => {
                values.push(first);
                self.eat(T::Comma)?
            }
            None => !self.at(T::RightBracket)?,
        };
        if read_more {
            loop {
                values.push(self.operand()?);
                if !self.eat(T::Comma)? {
                    break;
                }
            }
        }
        self.close(T::RightBracket, "`,` or `]`")?;
        let id = self.place(start);
        Ok(Operand::Array(id, values))
    }

    /// `operand`. A call here is the mistake of R-9, whatever the place.
    fn operand(&mut self) -> Parse<Operand> {
        match self.peek_kind()? {
            Some(T::Name(_)) => {
                let path = self.path("a value")?;
                self.path_operand(path)
            }
            Some(T::Minus | T::Int(_) | T::Float(_) | T::String(_) | T::LeftBracket | T::Keyword(K::Inf | K::Nan | K::True | K::False | K::None)) => {
                self.literal("a value")
            }
            _ => Err(self.unexpected_name("a value")),
        }
    }

    fn path_operand(&mut self, path: Path) -> Parse<Operand> {
        if matches!(self.peek_kind()?, Some(T::LeftParen | T::Less)) {
            return Err(self.nested_call(&path));
        }
        Ok(Operand::Path(path))
    }

    /// `LC1004`, with a fix that names the intermediate value.
    fn nested_call(&mut self, path: &Path) -> Box<Diagnostic> {
        let span = self.map.span(path.id).unwrap_or_else(|| one_column(self.last_end));
        let names: &Interner = self.lexer.names();
        let segments: Vec<&str> = path.segments().map(|name| names.resolve(name)).collect();
        let callee = segments.join(".");
        if self.in_default {
            let message = format!("`{callee}(…)` is a call inside a default value: no struct or variant with values stands inside another (R-21)");
            let mut diagnostic = Diagnostic::new(Code::Lc1004, message, "");
            diagnostic.span = Some(span);
            return Box::new(diagnostic);
        }
        let value = format!("{}_value", segments.last().copied().unwrap_or("result"));
        let mut diagnostic =
            Diagnostic::new(Code::Lc1004, format!("`{callee}(…)` is a call inside a value: each node has a line of its own (R-9)"), "");
        diagnostic.span = Some(span);
        diagnostic
            .fixes
            .push(Fix { title: format!("Write `{value} = {callee}(…)` on the line before, and use `{value}` here"), edits: Vec::new() });
        Box::new(diagnostic)
    }

    /// The `[` of an argument: a unit, or an array (R-5). `[x]` and `[1]` can be both, so they stay a `Bracket`.
    fn bracket_argument(&mut self) -> Parse<Operand> {
        let start = self.start()?;
        self.open(T::LeftBracket, "`[`")?;
        let unit_start = self.start()?;
        let operand = match self.peek_kind()? {
            Some(T::LeftParen | T::DegreeName(_)) => {
                let unit = self.unit_expr()?;
                self.bracket_end(unit, start)?
            }
            Some(T::Name(_)) => {
                let path = self.path("a value")?;
                if path.rest.is_empty() && (self.at(T::RightBracket)? || self.at_unit_operator()?.is_some() || self.at(T::Caret)?) {
                    let first = UnitTerm::Named { name: path.first, exponent: self.exponent()? };
                    let unit = self.unit_rest(first, unit_start)?;
                    self.bracket_end(unit, start)?
                } else {
                    let first = self.path_operand(path)?;
                    self.counted_array_rest(Some(first), start)?
                }
            }
            Some(T::Int("1")) => {
                self.bump();
                if self.at(T::RightBracket)? || self.at_unit_operator()?.is_some() || self.at(T::Caret)? {
                    let first = UnitTerm::One { exponent: self.exponent()? };
                    let unit = self.unit_rest(first, unit_start)?;
                    self.bracket_end(unit, start)?
                } else {
                    let digits = self.intern("1");
                    let first = self.number_with_unit(Numeral { negative: false, kind: NumeralKind::Int(digits) }, unit_start)?;
                    self.counted_array_rest(Some(first), start)?
                }
            }
            _ => self.counted_array_rest(None, start)?,
        };
        Ok(operand)
    }

    /// As `array_rest`, for an array that nothing has counted yet (14.1).
    fn counted_array_rest(&mut self, first: Option<Operand>, start: Position) -> Parse<Operand> {
        self.enter()?;
        let array = self.array_rest(first, start)?;
        self.leave();
        Ok(array)
    }

    fn bracket_end(&mut self, unit: UnitExpr, start: Position) -> Parse<Operand> {
        self.close(T::RightBracket, "an operator of a unit, or `]`")?;
        let id = self.place(start);
        Ok(Operand::Bracket(id, Box::new(unit)))
    }

    /// An argument list, with the option of a final comma, which the grammar gives calls only.
    fn args(&mut self, final_comma: bool) -> Parse<Box<[Arg]>> {
        self.open(T::LeftParen, "`(`")?;
        let mut args = Vec::new();
        if !self.at(T::RightParen)? {
            loop {
                args.push(self.arg()?);
                if !self.eat(T::Comma)? || (final_comma && self.at(T::RightParen)?) {
                    break;
                }
            }
        }
        self.close(T::RightParen, "`,` or `)`")?;
        Ok(args.into_boxed_slice())
    }

    fn arg(&mut self) -> Parse<Arg> {
        let start = self.start()?;
        match self.peek_kind()? {
            Some(T::Name(name)) => {
                self.bump();
                if self.eat(T::Colon)? {
                    if self.resolve(name) == "after" {
                        return self.after_arg(start);
                    }
                    let value = self.arg_value()?;
                    let id = self.place(start);
                    return Ok(Arg::Value { id, name: Some(name), value });
                }
                let path = self.path_from(name, start)?;
                let value = self.path_operand(path)?;
                Ok(Arg::Value { id: value.id(), name: None, value })
            }
            first => {
                let value = self.arg_value()?;
                // `true: 1`: a word that a value can be, used as the name of an argument.
                if let Some(T::Keyword(keyword)) = first
                    && self.at(T::Colon)?
                {
                    return Err(reserved_word(keyword, "the name of an argument", Span { start, end: self.last_end }));
                }
                Ok(Arg::Value { id: value.id(), name: None, value })
            }
        }
    }

    fn arg_value(&mut self) -> Parse<Operand> {
        if self.at(T::LeftBracket)? { self.bracket_argument() } else { self.operand() }
    }

    /// `after: x`, `after: #label`, `after: (x, #label)` (R-15).
    fn after_arg(&mut self, start: Position) -> Parse<Arg> {
        let mut refs = Vec::new();
        if self.at(T::LeftParen)? {
            self.open(T::LeftParen, "`(`")?;
            loop {
                refs.push(self.order_ref()?);
                if !self.eat(T::Comma)? {
                    break;
                }
            }
            self.close(T::RightParen, "`,` or `)`")?;
            // Without its parentheses, which canonical form drops, `[x]` reads as an argument does.
            if let [OrderRef::Value(value)] = refs.as_mut_slice() {
                as_bracket(value, self.lexer.names());
            }
        } else if self.at(T::Hash)? {
            refs.push(OrderRef::Label(self.label_ref()?));
        } else {
            // The grammar reads this one as any named argument, so it may be a unit in brackets; `check` refuses it.
            refs.push(OrderRef::Value(self.arg_value()?));
        }
        let id = self.place(start);
        Ok(Arg::After { id, refs })
    }

    fn order_ref(&mut self) -> Parse<OrderRef> {
        if self.at(T::Hash)? { Ok(OrderRef::Label(self.label_ref()?)) } else { Ok(OrderRef::Value(self.operand()?)) }
    }

    /// `#name`
    fn label_ref(&mut self) -> Parse<Label> {
        let start = self.start()?;
        self.expect(T::Hash, "`#`")?;
        let (name, _) = self.name("the name of a label")?;
        let id = self.place(start);
        Ok(Label { id, name })
    }

    // Statements.

    /// A body, after its `{`, up to its `}`, which is left for the caller.
    fn body(&mut self, place: Place) -> Parse<Body> {
        let mut statements = Vec::new();
        let mut until_seen = false;
        loop {
            self.skip_lines(false)?;
            match self.raw()?.map(|token| token.kind) {
                Some(T::RightBrace) => break,
                None => return Err(self.unexpected("a statement or `}`")),
                _ => {}
            }
            statements.push(self.statement(place, &mut until_seen)?);
            self.end_line("a statement")?;
        }
        if place == Place::Loop && !until_seen {
            return Err(self.unexpected("`until`: a `loop` has exactly one (R-6), before this `}`"));
        }
        Ok(Body { statements, end: self.take_pending() })
    }

    /// `collect`, `until`, `next` and `yield` where a name would be used: `next = 1`.
    fn refuse_as_target(&mut self, keyword: K, span: Span) -> Parse<()> {
        if matches!(self.peek_kind()?, Some(T::Equals | T::Comma)) {
            return Err(reserved_word(keyword, "a name", span));
        }
        Ok(())
    }

    fn statement(&mut self, place: Place, until_seen: &mut bool) -> Parse<Statement> {
        let mut before = self.begin_item();
        let start = self.start()?;
        let label = if self.at(T::Hash)? { Some(self.label()?) } else { None };
        let Some(token) = self.peek()? else {
            return Err(self.unexpected("a statement"));
        };
        let kind = match token.kind {
            T::Keyword(keyword @ (K::Collect | K::Until | K::Yield)) => {
                self.bump();
                self.refuse_as_target(keyword, token.span)?;
                self.reserved_statement(keyword, token.span, place, label.is_some(), until_seen)?
            }
            T::Keyword(keyword @ (K::Match | K::Handle | K::For | K::Loop)) => {
                self.bump();
                self.refuse_as_target(keyword, token.span)?;
                let word = keyword.as_str();
                return Err(failure(Code::Lc1001, format!("`{word}` gives values, so it follows a target: write `x = {word} …`"), token.span));
            }
            T::Underscore | T::Keyword(K::Next) => self.binding(None, place, &mut before)?,
            T::Name(_) => {
                let path_start = token.span.start;
                let path = self.path("a statement")?;
                if path.rest.is_empty() && matches!(self.peek_kind()?, Some(T::Equals | T::Comma)) {
                    let (id, name) = (path.id, path.first);
                    self.binding(Some(Target::Name(id, name)), place, &mut before)?
                } else if matches!(self.peek_kind()?, Some(T::LeftParen | T::Less)) {
                    StatementKind::Call(Box::new(self.call(path, path_start)?))
                } else {
                    return Err(self.unexpected(if path.rest.is_empty() { "`=`, `,` or `(`" } else { "`(`" }));
                }
            }
            _ => return Err(self.unexpected_name("a statement")),
        };
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Statement { id, comments, label, kind })
    }

    /// `#name:`
    fn label(&mut self) -> Parse<Label> {
        let label = self.label_ref()?;
        self.expect(T::Colon, "`:`")?;
        Ok(label)
    }

    fn reserved_statement(&mut self, keyword: K, span: Span, place: Place, labelled: bool, until_seen: &mut bool) -> Parse<StatementKind> {
        let refuse = |message: &str| Err(failure(Code::Lc1001, message.to_owned(), span));
        if labelled {
            return refuse("this statement creates no node, so it takes no label");
        }
        match keyword {
            K::Yield => refuse("`yield` ends an arm of a `match` or a `handle`"),
            K::Collect if !matches!(place, Place::For | Place::Loop) => refuse("`collect` appears only directly in a loop body (R-6)"),
            K::Collect => Ok(StatementKind::Collect(self.operand()?)),
            K::Until if place == Place::For => refuse("a `for` has no `until` (R-6)"),
            K::Until if place != Place::Loop => refuse("`until` appears only directly in a `loop` body (R-6)"),
            K::Until if *until_seen => refuse("a `loop` has exactly one `until` (R-6)"),
            _ => {
                *until_seen = true;
                Ok(StatementKind::Until(self.operand()?))
            }
        }
    }

    fn target(&mut self, place: Place) -> Parse<Target> {
        let Some(token) = self.peek()? else {
            return Err(self.unexpected("a target"));
        };
        match token.kind {
            T::Underscore => {
                self.bump();
                Ok(Target::Skip(self.place(token.span.start)))
            }
            T::Keyword(K::Next) => {
                self.bump();
                self.refuse_as_target(K::Next, token.span)?;
                if !matches!(place, Place::For | Place::Loop) {
                    return Err(failure(Code::Lc1001, "`next` appears only directly in a loop body (R-6)".to_owned(), token.span));
                }
                let (name, _) = self.name("the name of a carry")?;
                Ok(Target::Next(self.place(token.span.start), name))
            }
            _ => {
                let (name, span) = self.name("a target")?;
                Ok(Target::Name(self.map.add(span), name))
            }
        }
    }

    fn binding(&mut self, first: Option<Target>, place: Place, before: &mut Vec<Comment>) -> Parse<StatementKind> {
        let mut targets = Vec::with_capacity(1);
        match first {
            Some(first) => targets.push(first),
            None => targets.push(self.target(place)?),
        }
        while self.eat(T::Comma)? {
            targets.push(self.target(place)?);
        }
        self.expect(T::Equals, "`,` or `=`")?;
        let value = self.rhs(before)?;
        Ok(StatementKind::Binding { targets: targets.into_boxed_slice(), value })
    }

    fn rhs(&mut self, before: &mut Vec<Comment>) -> Parse<Rhs> {
        let start = self.start()?;
        match self.peek_kind()? {
            Some(T::Keyword(K::Match)) => Ok(Rhs::Match(Box::new(self.branching(false, before)?))),
            Some(T::Keyword(K::Handle)) => Ok(Rhs::Handle(Box::new(self.branching(true, before)?))),
            Some(T::Keyword(K::For)) => Ok(Rhs::For(Box::new(self.for_expr(before)?))),
            Some(T::Keyword(K::Loop)) => Ok(Rhs::Loop(Box::new(self.loop_expr(before)?))),
            Some(T::Name(_)) => {
                let path = self.path("a value")?;
                match self.peek_kind()? {
                    Some(T::LeftParen | T::Less) => Ok(Rhs::Call(Box::new(self.call(path, start)?))),
                    Some(T::LeftBrace) => Ok(Rhs::Struct(Box::new(self.struct_literal(path, start)?))),
                    _ => Ok(Rhs::Operand(Operand::Path(path))),
                }
            }
            _ => Ok(Rhs::Operand(self.operand()?)),
        }
    }

    fn call(&mut self, callee: Path, start: Position) -> Parse<Call> {
        let mut type_args = Vec::new();
        if self.at(T::Less)? {
            // A statement stays on one line: its comments go before it, even from a struct in a type.
            self.atomic += 1;
            self.bump();
            type_args = self.types()?;
            self.expect(T::Greater, "`,` or `>`")?;
            self.atomic -= 1;
        }
        let args = self.args(true)?;
        let id = self.place(start);
        Ok(Call { id, callee, type_args: type_args.into_boxed_slice(), args })
    }

    fn struct_literal(&mut self, path: Path, start: Position) -> Parse<StructLit> {
        self.open(T::LeftBrace, "`{`")?;
        let mut fields = Vec::new();
        loop {
            let field_start = self.start()?;
            let (name, _) = self.name("the name of a field")?;
            self.expect(T::Colon, "`:`")?;
            let value = self.operand()?;
            let id = self.place(field_start);
            fields.push(FieldInit { id, name, value });
            if !self.eat(T::Comma)? || self.at(T::RightBrace)? {
                break;
            }
        }
        self.close(T::RightBrace, "`,` or `}`")?;
        let id = self.place(start);
        Ok(StructLit { id, path, fields })
    }

    // Structures.

    /// The `{` of a structure, whose header ignored line endings. A comment left in the header is inside
    /// a statement written over several lines: it goes before that statement.
    fn open_structure(&mut self, before: &mut Vec<Comment>) -> Parse<Option<Comment>> {
        if !self.at(T::LeftBrace)? {
            return Err(self.unexpected("`{`"));
        }
        before.append(&mut self.take_pending());
        self.free = self.free.saturating_sub(1);
        self.bump();
        self.line_comment(self.last_end.line)
    }

    /// A `match` (`handle` false) or a `handle`.
    fn branching(&mut self, handle: bool, before: &mut Vec<Comment>) -> Parse<Branching> {
        let start = self.start()?;
        self.enter()?;
        self.bump();
        self.free += 1;
        let selector = self.operand()?;
        let open = self.open_structure(before)?;
        let mut arms = Vec::new();
        loop {
            self.skip_lines(false)?;
            match self.raw()?.map(|token| token.kind) {
                Some(T::RightBrace) => break,
                None => return Err(self.unexpected("an arm or `}`")),
                _ => {}
            }
            arms.push(self.arm(handle)?);
            self.end_line("an arm")?;
        }
        let end = self.take_pending();
        self.bump();
        self.leave();
        let id = self.place(start);
        Ok(Branching { id, selector, open, arms, end })
    }

    fn arm(&mut self, handle: bool) -> Parse<Arm> {
        let mut before = self.begin_item();
        let start = self.start()?;
        let pattern = if handle { self.handle_pattern()? } else { self.match_pattern()? };
        let others = if handle { Vec::new() } else { self.other_patterns(&pattern, start)? };
        self.expect(T::FatArrow, "`=>`")?;
        before.append(&mut self.take_pending());
        match self.raw()?.map(|token| token.kind) {
            Some(T::Keyword(K::Yield)) => {
                let mut yields = self.yield_statement()?;
                let Attached { before: inner, line } = std::mem::take(&mut yields.comments);
                before.extend(inner.into_vec());
                let id = self.place(start);
                let comments = Attached { before: before.into_boxed_slice(), line };
                Ok(Arm { id, comments, pattern, others, open: None, body: Body::default(), yields })
            }
            Some(T::LeftBrace) => self.braced_arm(before, start, pattern, others),
            _ => Err(self.unexpected("`yield` or `{`: the body of an arm starts on the line of its `=>`")),
        }
    }

    /// The rest of a list, after the first pattern of a `match` arm: `, 3..=5, _`.
    fn other_patterns(&mut self, first: &Pattern, start: Position) -> Parse<Vec<Pattern>> {
        let mut others = Vec::new();
        if !self.at(T::Comma)? {
            return Ok(others);
        }
        self.refuse_binding(first, start)?;
        while self.eat(T::Comma)? {
            let start = self.start()?;
            let pattern = self.match_pattern()?;
            self.refuse_binding(&pattern, start)?;
            others.push(pattern);
        }
        Ok(others)
    }

    /// A name bound by one pattern of a list would have no value when another one matches.
    fn refuse_binding(&self, pattern: &Pattern, start: Position) -> Parse<()> {
        match pattern {
            Pattern::Variant { bindings, .. } if !bindings.is_empty() => {
                let message = "a pattern that binds names stands alone in its arm".to_owned();
                Err(failure(Code::Lc1001, message, Span { start, end: self.last_end }))
            }
            _ => Ok(()),
        }
    }

    fn braced_arm(&mut self, mut before: Vec<Comment>, start: Position, pattern: Pattern, others: Vec<Pattern>) -> Parse<Arm> {
        self.bump();
        let mut open = self.line_comment(self.last_end.line)?;
        let mut statements = Vec::new();
        let mut until_seen = false;
        let mut yields = loop {
            self.skip_lines(false)?;
            match self.raw()?.map(|token| token.kind) {
                Some(T::Keyword(K::Yield)) => {
                    let yields = self.yield_statement()?;
                    self.end_line("a `yield`")?;
                    break yields;
                }
                Some(T::RightBrace) => return Err(self.unexpected("`yield`: an arm ends with exactly one (R-6)")),
                None => return Err(self.unexpected("a statement or `yield`")),
                _ => {}
            }
            statements.push(self.statement(Place::Arm, &mut until_seen)?);
            self.end_line("a statement")?;
        };
        self.skip_lines(false)?;
        if self.raw()?.map(|token| token.kind) != Some(T::RightBrace) {
            return Err(self.unexpected("`}`: the `yield` of an arm comes last"));
        }
        let mut end = self.take_pending();
        self.bump();
        if statements.is_empty() {
            // Only a `yield`: the arm goes on one line, so all its comments go before it (5.2).
            before.extend(open.take());
            let Attached { before: inner, line } = std::mem::take(&mut yields.comments);
            before.extend(inner.into_vec());
            before.extend(line.map(|comment| *comment));
            before.append(&mut end);
        }
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Arm { id, comments, pattern, others, open, body: Body { statements, end }, yields })
    }

    fn yield_statement(&mut self) -> Parse<Yield> {
        let before = self.begin_item();
        let start = self.start()?;
        let keyword = self.expect(T::Keyword(K::Yield), "`yield`")?;
        self.refuse_as_target(K::Yield, keyword.span)?;
        let mut values = Vec::with_capacity(1);
        if self.at(T::LeftParen)? {
            self.open(T::LeftParen, "`(`")?;
            loop {
                values.push(self.operand()?);
                if !self.eat(T::Comma)? {
                    break;
                }
            }
            self.close(T::RightParen, "`,` or `)`")?;
        } else {
            values.push(self.operand()?);
        }
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Yield { id, comments, values })
    }

    fn match_pattern(&mut self) -> Parse<Pattern> {
        let start = self.start()?;
        match self.peek_kind()? {
            Some(T::Underscore) => {
                self.bump();
                Ok(Pattern::Wildcard { id: self.place(start), binding: None })
            }
            Some(T::Name(_)) => {
                let path = self.path("a pattern")?;
                let bindings = if self.at(T::LeftParen)? { self.bindings()? } else { Vec::new() };
                Ok(Pattern::Variant { id: self.place(start), path, bindings })
            }
            Some(T::Minus | T::Int(_) | T::Float(_) | T::Keyword(K::Inf | K::Nan)) => {
                let numeral = self.numeral()?;
                if let NumeralKind::Int(digits) = numeral.kind
                    && self.eat(T::DotDot)?
                {
                    let low = IntBound { negative: numeral.negative, digits };
                    let inclusive = self.eat(T::Equals)?;
                    let high = self.int_bound()?;
                    return Ok(Pattern::Range { id: self.place(start), low, high, inclusive });
                }
                Ok(Pattern::Literal(self.number_with_unit(numeral, start)?))
            }
            Some(T::String(_) | T::LeftBracket | T::Keyword(K::True | K::False | K::None)) => Ok(Pattern::Literal(self.literal("a pattern")?)),
            _ => Err(self.unexpected_name("a pattern")),
        }
    }

    /// `[ "-" ] INT`, with no space after the minus.
    fn int_bound(&mut self) -> Parse<IntBound> {
        let start = self.start()?;
        let numeral = self.numeral()?;
        match numeral.kind {
            NumeralKind::Int(digits) => Ok(IntBound { negative: numeral.negative, digits }),
            _ => Err(failure(Code::Lc1001, "expected an integer here".to_owned(), Span { start, end: self.last_end })),
        }
    }

    /// `(a, b)` after a variant.
    fn bindings(&mut self) -> Parse<Vec<Binding>> {
        self.open(T::LeftParen, "`(`")?;
        let mut bindings = Vec::new();
        loop {
            bindings.push(self.binding_name()?);
            if !self.eat(T::Comma)? {
                break;
            }
        }
        self.close(T::RightParen, "`,` or `)`")?;
        Ok(bindings)
    }

    fn binding_name(&mut self) -> Parse<Binding> {
        let (name, span) = self.name("a name to bind")?;
        Ok(Binding { id: self.map.add(span), name })
    }

    /// `(err)` after a pattern of a `handle`.
    fn optional_binding(&mut self) -> Parse<Option<Binding>> {
        if !self.at(T::LeftParen)? {
            return Ok(None);
        }
        self.open(T::LeftParen, "`(`")?;
        let binding = self.binding_name()?;
        self.close(T::RightParen, "`)`")?;
        Ok(Some(binding))
    }

    fn handle_pattern(&mut self) -> Parse<Pattern> {
        let start = self.start()?;
        match self.peek_kind()? {
            Some(T::Underscore) => {
                self.bump();
                let binding = self.optional_binding()?;
                Ok(Pattern::Wildcard { id: self.place(start), binding })
            }
            Some(T::Name(name)) => {
                self.bump();
                let word = self.resolve(name);
                let (is_ok, is_code) = (word == "ok", word == "code");
                if is_ok && self.at(T::FatArrow)? {
                    return Ok(Pattern::Ok { id: self.place(start) });
                }
                if is_code && matches!(self.peek_kind()?, Some(T::Minus | T::Int(_))) {
                    let code = self.int_bound()?;
                    let binding = self.optional_binding()?;
                    return Ok(Pattern::Code { id: self.place(start), code, binding });
                }
                self.category(name, start)
            }
            _ => Err(self.unexpected_name("a pattern of a `handle`: `ok`, a category, `code` or `_`")),
        }
    }

    /// `IDENT { "." IDENT } [ "." "*" ]`
    fn category(&mut self, first: Name, start: Position) -> Parse<Pattern> {
        let mut rest = Vec::new();
        let mut wildcard = false;
        while self.eat(T::Dot)? {
            if self.eat(T::Star)? {
                wildcard = true;
                break;
            }
            rest.push(self.name("a name or `*` after `.`")?.0);
        }
        let path = Path { id: self.place(start), first, rest: rest.into_boxed_slice() };
        let binding = self.optional_binding()?;
        Ok(Pattern::Category { id: self.place(start), path, wildcard, binding })
    }

    fn for_expr(&mut self, before: &mut Vec<Comment>) -> Parse<For> {
        let start = self.start()?;
        self.enter()?;
        self.bump();
        self.free += 1;
        let parallel = self.eat(T::Keyword(K::Parallel))?;
        let (clauses, clauses_end) = self.clauses(true)?;
        let open = self.open_structure(before)?;
        let body = self.body(Place::For)?;
        self.bump();
        self.leave();
        let id = self.place(start);
        Ok(For { id, parallel, clauses, clauses_end, open, body })
    }

    fn loop_expr(&mut self, before: &mut Vec<Comment>) -> Parse<Loop> {
        let start = self.start()?;
        self.enter()?;
        self.bump();
        self.free += 1;
        let continue_on_error = self.eat(T::Keyword(K::ContinueOnError))?;
        let (clauses, clauses_end) = if self.at(T::Keyword(K::Carry))? { self.clauses(false)? } else { (Vec::new(), Vec::new()) };
        let open = self.open_structure(before)?;
        let body = self.body(Place::Loop)?;
        self.bump();
        self.leave();
        let id = self.place(start);
        Ok(Loop { id, continue_on_error, clauses, clauses_end, open, body })
    }

    /// The clauses of a `for` (`in` clauses, then carries) or of a `loop` (carries). No final comma.
    fn clauses(&mut self, is_for: bool) -> Parse<(Vec<Clause>, Vec<Comment>)> {
        let mut clauses = Vec::new();
        let mut carrying = !is_for;
        loop {
            let before = self.begin_item();
            let at_carry = self.at(T::Keyword(K::Carry))?;
            let kind = if at_carry && (carrying || !clauses.is_empty()) {
                carrying = true;
                ClauseKind::Carry(self.carry()?)
            } else if carrying {
                return Err(self.unexpected("`carry`: the carries of a `for` come after its `in` clauses"));
            } else {
                ClauseKind::In(self.for_in()?)
            };
            let item_line = self.last_end.line;
            let mut comments = self.end_item(before)?;
            let more = self.comma_after(&mut comments, item_line)?;
            clauses.push(Clause { comments, kind });
            if !more {
                break;
            }
        }
        if !self.at(T::LeftBrace)? {
            return Err(self.unexpected("`,` or `{`"));
        }
        Ok((clauses, self.take_pending()))
    }

    fn for_in(&mut self) -> Parse<ForIn> {
        let start = self.start()?;
        let (name, _) = self.name("the name of the value of a `for`")?;
        if !self.eat_word("in")? {
            return Err(self.unexpected("`in`"));
        }
        let source = self.operand()?;
        let end = if self.eat(T::DotDot)? { Some(self.operand()?) } else { None };
        let id = self.place(start);
        Ok(ForIn { id, name, source, end })
    }

    fn carry(&mut self) -> Parse<Carry> {
        let start = self.start()?;
        self.expect(T::Keyword(K::Carry), "`carry`")?;
        let (name, _) = self.name("the name of a carry")?;
        let ty = if self.eat(T::Colon)? { Some(self.type_expr()?) } else { None };
        self.expect(T::Equals, "`:` or `=`")?;
        let value = self.operand()?;
        let id = self.place(start);
        Ok(Carry { id, name, ty, value })
    }

    // Sections.

    /// The opening of a section: a keyword, up to its `{`. Its header ignores line endings.
    fn section_start(&mut self) -> Parse<(Vec<Comment>, Position)> {
        let before = self.take_pending();
        Ok((before, self.start()?))
    }

    fn panel(&mut self) -> Parse<Panel> {
        let (before, start) = self.section_start()?;
        self.bump();
        self.open_section()?;
        let (items, end) = self.section_items("a panel item or `}`", Self::panel_item)?;
        self.bump();
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Panel { id, comments, items, end })
    }

    /// The `{` of a section, from which line endings count.
    fn open_section(&mut self) -> Parse<()> {
        self.free += 1;
        if !self.at(T::LeftBrace)? {
            return Err(self.unexpected("`{`"));
        }
        self.free = self.free.saturating_sub(1);
        self.bump();
        Ok(())
    }

    /// The items of a section, one per line, up to its `}`, which is left for the caller.
    fn section_items<I>(&mut self, expected: &str, parse: fn(&mut Self) -> Parse<I>) -> Parse<(Vec<I>, Vec<Comment>)> {
        let mut items = Vec::new();
        loop {
            self.skip_lines(false)?;
            match self.raw()?.map(|token| token.kind) {
                Some(T::RightBrace) => break,
                None => return Err(self.unexpected(expected)),
                _ => {}
            }
            items.push(parse(self)?);
            self.end_line("an item")?;
        }
        Ok((items, self.take_pending()))
    }

    fn panel_item(&mut self) -> Parse<PanelItem> {
        let before = self.begin_item();
        let start = self.start()?;
        let direction = if self.eat_word("control")? {
            PanelDirection::Control
        } else if self.eat_word("indicator")? {
            PanelDirection::Indicator
        } else {
            return Err(self.unexpected("`control` or `indicator`"));
        };
        let (name, _) = self.name("the name of a panel element")?;
        self.expect(T::Colon, "`:`")?;
        let (kind, _) = self.name("the kind of a panel element")?;
        let args = if self.at(T::LeftParen)? { Some(self.args(false)?) } else { None };
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(PanelItem { id, comments, direction, name, kind, args })
    }

    fn layout(&mut self) -> Parse<Layout> {
        let (before, start) = self.section_start()?;
        self.bump();
        self.open_section()?;
        let (items, end) = self.section_items("a layout item or `}`", Self::layout_item)?;
        self.bump();
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Layout { id, comments, items, end })
    }

    fn layout_item(&mut self) -> Parse<LayoutItem> {
        let before = self.begin_item();
        let start = self.start()?;
        let kind = if self.eat_word("node")? {
            let node = self.node_ref()?;
            if self.eat_word("at")? {
                let at = Some(self.point()?);
                let view = if self.at_word("view")? { Some(self.view()?) } else { None };
                LayoutKind::Node { node, at, view }
            } else {
                LayoutKind::Node { node, at: None, view: Some(self.view()?) }
            }
        } else if self.eat_word("wire")? {
            let from = self.terminal_ref()?;
            self.expect(T::Arrow, "`.` or `->`")?;
            let to = self.terminal_ref()?;
            let mut via = Vec::new();
            if self.eat_word("via")? {
                via.push(self.point()?);
                while self.at(T::LeftParen)? {
                    via.push(self.point()?);
                }
            }
            LayoutKind::Wire { from, to, via }
        } else {
            return Err(self.unexpected("`node` or `wire`"));
        };
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(LayoutItem { id, comments, kind })
    }

    fn node_ref(&mut self) -> Parse<NodeRef> {
        let start = self.start()?;
        let is_label = self.eat(T::Hash)?;
        let (name, _) = self.name("the name of a node")?;
        Ok(NodeRef { id: self.place(start), is_label, name })
    }

    fn terminal_ref(&mut self) -> Parse<TerminalRef> {
        let start = self.start()?;
        let node = self.node_ref()?;
        let terminal = if self.eat(T::Dot)? { Some(self.name("the name of a terminal")?.0) } else { None };
        Ok(TerminalRef { id: self.place(start), node, terminal })
    }

    fn view(&mut self) -> Parse<View> {
        if !self.eat_word("view")? {
            return Err(self.unexpected("`at` or `view`"));
        }
        if self.eat_word("standard")? {
            Ok(View::Standard)
        } else if self.eat_word("compact")? {
            Ok(View::Compact)
        } else {
            Err(self.unexpected("`standard` or `compact`"))
        }
    }

    fn point(&mut self) -> Parse<Point> {
        let start = self.start()?;
        self.open(T::LeftParen, "`(`")?;
        let x = self.numeral()?;
        self.expect(T::Comma, "`,`")?;
        let y = self.numeral()?;
        self.close(T::RightParen, "`)`")?;
        Ok(Point { id: self.place(start), x, y })
    }

    fn icon(&mut self) -> Parse<Icon> {
        let (before, start) = self.section_start()?;
        self.bump();
        self.free += 1;
        let Some(T::Int(text)) = self.peek_kind()? else {
            return Err(self.unexpected("the size of the grid of the icon"));
        };
        self.bump();
        let grid = self.intern(text);
        self.free = self.free.saturating_sub(1);
        self.open_section()?;
        let (items, end) = self.section_items("a shape or `}`", Self::icon_item)?;
        self.bump();
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(Icon { id, comments, grid, items, end })
    }

    fn icon_item(&mut self) -> Parse<IconItem> {
        let before = self.begin_item();
        let start = self.start()?;
        let shape = self.icon_shape()?;
        let fill = self.eat_word("fill")?;
        let accent = if self.eat_word("accent")? { Some(self.path("a token of the theme")?) } else { None };
        let id = self.place(start);
        let comments = self.end_item(before)?;
        Ok(IconItem { id, comments, shape, fill, accent })
    }

    fn icon_shape(&mut self) -> Parse<IconShape> {
        if self.eat_word("rect")? {
            let (from, to) = (self.point()?, self.point()?);
            let radius = if self.eat_word("radius")? { Some(self.numeral()?) } else { None };
            Ok(IconShape::Rect { from, to, radius })
        } else if self.eat_word("circle")? {
            let center = self.point()?;
            Ok(IconShape::Circle { center, radius: self.numeral()? })
        } else if self.eat_word("line")? {
            let (from, to) = (self.point()?, self.point()?);
            Ok(IconShape::Line { from, to })
        } else if self.eat_word("path")? {
            Ok(IconShape::Path { data: self.string("the data of a path, as a string")? })
        } else if self.eat_word("text")? {
            let text = self.string("a string")?;
            if !self.eat_word("at")? {
                return Err(self.unexpected("`at`"));
            }
            let at = self.point()?;
            let size = if self.eat_word("size")? { Some(self.numeral()?) } else { None };
            Ok(IconShape::Text { text, at, size })
        } else {
            Err(self.unexpected("`rect`, `circle`, `line`, `path` or `text`"))
        }
    }
}

/// A comment in its one spelling (5.2): at least one space after `//`, or nothing when it is empty, and no space or
/// tab at the end of the line. Everything else in a comment is its text.
fn comment_of(kind: CommentKind, text: &str) -> Comment {
    let text = text.trim_end_matches([' ', '\t']);
    let text = if text.is_empty() || text.starts_with(' ') { text.into() } else { format!(" {text}").into_boxed_str() };
    Comment { kind, text }
}

/// `[x]` and `[1]`, read as an array, in the form that an argument gives them (R-5).
fn as_bracket(value: &mut Operand, names: &Interner) {
    let Operand::Array(id, values) = value else {
        return;
    };
    let unit = match values.as_slice() {
        [Operand::Path(path)] if path.rest.is_empty() => {
            UnitExpr { id: path.id, first: UnitTerm::Named { name: path.first, exponent: None }, rest: Vec::new() }
        }
        [Operand::Number(number, Numeral { negative: false, kind: NumeralKind::Int(digits) }, None)] if names.resolve(*digits) == "1" => {
            UnitExpr { id: *number, first: UnitTerm::One { exponent: None }, rest: Vec::new() }
        }
        _ => return,
    };
    *value = Operand::Bracket(*id, Box::new(unit));
}

fn misplaced_documentation(span: Span) -> Box<Diagnostic> {
    failure(Code::Lc1001, "`///` documents a declaration and goes right before it: write `//` here".to_owned(), span)
}

fn reserved_word(keyword: K, expected: &str, span: Span) -> Box<Diagnostic> {
    failure(Code::Lc1005, format!("`{}` is a reserved word, so it cannot be a name: expected {expected}", keyword.as_str()), span)
}

/// `01`, `1_`, `0_1`: a way of writing 1.
fn is_one_written_otherwise(text: &str) -> bool {
    text.bytes().filter(|byte| *byte != b'_').skip_while(|byte| *byte == b'0').eq(std::iter::once(b'1'))
}
