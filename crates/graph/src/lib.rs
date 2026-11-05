//! L1: the semantic model of one file, as the file says it (`docs/architecture.md`, section 3). It is
//! syntactic: resolving what a name or a number means is the job of `check`.
//!
//! What must survive from the source is kept here: the text of numbers, the written form of types and
//! units, and every comment, in a place of its own (spec 5.2). The types that a hostile file can repeat by
//! the million (`Operand`, `Statement`, `Arg`) stay small, since what is kept per item decides the memory
//! of a large file.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use labcraft_types::{Name, Span};

mod renumber;

/// What a diagnostic can point at. [`SourceMap`] gives its place in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(u32);

impl Id {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// From ids to spans, returned beside a [`File`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMap {
    spans: Vec<Span>,
}

impl SourceMap {
    pub fn add(&mut self, span: Span) -> Id {
        // More than 2^32 places cannot fit under `Limits`; the last one stands for them.
        let id = Id(u32::try_from(self.spans.len()).unwrap_or(u32::MAX));
        self.spans.push(span);
        id
    }

    /// `None` past the end of the map. An id means something only in the map that made it, which this
    /// cannot check.
    pub fn span(&self, id: Id) -> Option<Span> {
        self.spans.get(id.index()).copied()
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    /// `//`
    Line,
    /// `///`, which the grammar allows only before the declaration.
    Doc,
}

/// A comment, with the text that follows its `//` or `///`, up to the line ending, in its one spelling (5.2):
/// empty or starting with a space, and with no space at its end. That makes a comment a name of sorts: `read`
/// gives this form, so that the model does not change when `fmt` writes the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub kind: CommentKind,
    pub text: Box<str>,
}

/// The comments of an item that moves as one (spec 5.2): those before it, and the one that ends on its line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attached {
    pub before: Box<[Comment]>,
    pub line: Option<Box<Comment>>,
}

/// `a.b.c`. Most paths have one segment, so `rest` usually holds nothing and allocates nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    pub id: Id,
    pub first: Name,
    pub rest: Box<[Name]>,
}

impl Path {
    pub fn segments(&self) -> impl Iterator<Item = Name> + '_ {
        std::iter::once(self.first).chain(self.rest.iter().copied())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Always 1 for now: the header refuses anything else (`LC0102`).
    pub version: u32,
    /// The comment that ends on the header line.
    pub header_comment: Option<Comment>,
    /// The comments of the `use` section, which go before it: there is none between two `use` lines (5.2).
    pub use_comments: Vec<Comment>,
    pub uses: Vec<Use>,
    pub decl: Decl,
    pub panel: Option<Panel>,
    pub layout: Option<Layout>,
    pub icon: Option<Icon>,
    /// Comments after the last section.
    pub end: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    pub id: Id,
    pub path: Path,
    /// The comment that ends on its line.
    pub line: Option<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    Function(Function),
    Extern(Extern),
    Type(TypeDecl),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub id: Id,
    /// `before` holds the documentation, in written order among the plain comments.
    pub comments: Attached,
    pub name: Name,
    pub signature: Signature,
    /// The comment that ends the line of the `{`.
    pub open: Option<Comment>,
    pub body: Body,
}

/// `extern rust(crate: "…", function: "…")` and its `fn` line, which has no body (4.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extern {
    pub id: Id,
    pub comments: Attached,
    pub language: Name,
    pub args: Box<[Arg]>,
    pub name: Name,
    pub signature: Signature,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    pub id: Id,
    pub comments: Attached,
    pub name: Name,
    pub generics: Vec<Generic>,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signature {
    pub generics: Vec<Generic>,
    pub params: Vec<Param>,
    /// Comments after the last parameter, and those of a list without parameters.
    pub params_end: Vec<Comment>,
    pub outputs: Vec<Output>,
    pub outputs_end: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generic {
    pub id: Id,
    pub name: Name,
    pub bound: Option<Path>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub id: Id,
    pub comments: Attached,
    pub name: Name,
    pub ty: TypeExpr,
    pub default: Option<Operand>,
    pub variadic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub id: Id,
    pub comments: Attached,
    pub name: Name,
    pub ty: TypeExpr,
}

/// The written form of a type: `check` turns it into a `TyId`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeExpr {
    Named(NamedType),
    Array(ArrayType),
    Struct(StructType),
    Enum(EnumType),
    Fn(FnType),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedType {
    pub id: Id,
    pub path: Path,
    pub args: Vec<TypeExpr>,
    pub unit: Option<UnitExpr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrayType {
    pub id: Id,
    pub element: Box<TypeExpr>,
    /// The text of the number of dimensions: `check` evaluates it (`LC1006`).
    pub dimensions: Option<Name>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructType {
    pub id: Id,
    pub fields: Vec<Field>,
    pub end: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: Id,
    pub comments: Attached,
    pub name: Name,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumType {
    pub id: Id,
    pub variants: Vec<VariantDecl>,
    pub end: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDecl {
    pub id: Id,
    pub comments: Attached,
    pub name: Name,
    pub payload: Vec<TypeExpr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnType {
    pub id: Id,
    pub params: Vec<FnArg>,
    pub outputs: Vec<TypeExpr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnArg {
    pub id: Id,
    pub name: Option<Name>,
    pub ty: TypeExpr,
}

/// The written form of a unit (`kg/(m*s)`, `·`, `V/A`), which `Unit` would lose. Equal precedence, left to right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitExpr {
    pub id: Id,
    pub first: UnitTerm,
    pub rest: Vec<(UnitOp, UnitTerm)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitOp {
    /// `*`
    Star,
    /// `·`
    MiddleDot,
    /// `/`
    Slash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitTerm {
    Named {
        name: Name,
        exponent: Option<Exponent>,
    },
    /// The `1` of `1/s`.
    One {
        exponent: Option<Exponent>,
    },
    Group(Box<UnitExpr>),
}

/// `^-2`, with the text of its digits: `check` evaluates it (`LC1006`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exponent {
    pub negative: bool,
    pub digits: Name,
}

/// A function body, an arm or a loop body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body {
    pub statements: Vec<Statement>,
    /// Comments with no statement after them, at the level of the body.
    pub end: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub id: Id,
    pub comments: Attached,
    pub label: Option<Label>,
    pub kind: StatementKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    pub id: Id,
    pub name: Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatementKind {
    Binding { targets: Box<[Target]>, value: Rhs },
    Call(Box<Call>),
    Collect(Operand),
    Until(Operand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Name(Id, Name),
    /// `_`
    Skip(Id),
    /// `next x`
    Next(Id, Name),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rhs {
    Call(Box<Call>),
    Match(Box<Branching>),
    Handle(Box<Branching>),
    For(Box<For>),
    Loop(Box<Loop>),
    Struct(Box<StructLit>),
    Operand(Operand),
}

/// A variant construction is a call too: `Reading.Value(v)`. `check` tells them apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub id: Id,
    pub callee: Path,
    pub type_args: Box<[TypeExpr]>,
    pub args: Box<[Arg]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// A positional argument has the id of its value, whose place it shares.
    Value { id: Id, name: Option<Name>, value: Operand },
    /// `after: x`, `after: #label` or `after: (x, #label)` (R-15).
    After { id: Id, refs: Vec<OrderRef> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderRef {
    Value(Operand),
    Label(Label),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructLit {
    pub id: Id,
    pub path: Path,
    pub fields: Vec<FieldInit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldInit {
    pub id: Id,
    pub name: Name,
    pub value: Operand,
}

/// A value, written directly. `[x]` as an argument stays a `Bracket` until `check` knows the kind of the
/// parameter (R-5); `[1]` too, since `1` is also a unit term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    Path(Path),
    Number(Id, Numeral, Option<Box<UnitExpr>>),
    /// What lies between the quotes, escapes unread.
    Text(Id, Box<str>),
    Bool(Id, bool),
    None(Id),
    Array(Id, Vec<Operand>),
    Bracket(Id, Box<UnitExpr>),
}

impl Operand {
    pub fn id(&self) -> Id {
        match self {
            Operand::Path(path) => path.id,
            Operand::Number(id, ..)
            | Operand::Text(id, _)
            | Operand::Bool(id, _)
            | Operand::None(id)
            | Operand::Array(id, _)
            | Operand::Bracket(id, _) => *id,
        }
    }
}

/// A number with its text and no value: `1e999` reads, `check` reports it (`LC1006`), and `fmt` never
/// fails on it. Texts are interned, as names are, so that a file of millions of numbers stays small; they
/// are ASCII, which normalizing leaves alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Numeral {
    pub negative: bool,
    pub kind: NumeralKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumeralKind {
    Int(Name),
    Float(Name),
    Inf,
    Nan,
}

/// A `match` or a `handle`, which `Rhs` tells apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branching {
    pub id: Id,
    pub selector: Operand,
    pub open: Option<Comment>,
    pub arms: Vec<Arm>,
    pub end: Vec<Comment>,
}

/// An arm whose body is only a `yield` keeps its comments in `comments` (5.2): it goes on one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub id: Id,
    pub comments: Attached,
    pub pattern: Pattern,
    pub open: Option<Comment>,
    pub body: Body,
    pub yields: Yield,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Yield {
    pub id: Id,
    pub comments: Attached,
    pub values: Vec<Operand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// A literal: `Operand` is never a `Path` or a `Bracket` here.
    Literal(Operand),
    Range {
        id: Id,
        low: IntBound,
        high: IntBound,
        inclusive: bool,
    },
    /// `State.Init`, `Reading.Value(v)`.
    Variant {
        id: Id,
        path: Path,
        bindings: Vec<Binding>,
    },
    /// `_`, and in a `handle`, `_(err)`.
    Wildcard {
        id: Id,
        binding: Option<Binding>,
    },
    /// `ok`
    Ok {
        id: Id,
    },
    /// `io.timeout(err)`, `visa.*`.
    Category {
        id: Id,
        path: Path,
        wildcard: bool,
        binding: Option<Binding>,
    },
    /// `code -1073807339 (err)`
    Code {
        id: Id,
        code: IntBound,
        binding: Option<Binding>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntBound {
    pub negative: bool,
    pub digits: Name,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub id: Id,
    pub name: Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct For {
    pub id: Id,
    pub parallel: bool,
    pub clauses: Vec<Clause>,
    /// Comments after the last clause, at the level of the clauses.
    pub clauses_end: Vec<Comment>,
    pub open: Option<Comment>,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loop {
    pub id: Id,
    pub continue_on_error: bool,
    /// Only `ClauseKind::Carry` here.
    pub clauses: Vec<Clause>,
    pub clauses_end: Vec<Comment>,
    pub open: Option<Comment>,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    pub comments: Attached,
    pub kind: ClauseKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClauseKind {
    In(ForIn),
    Carry(Carry),
}

/// `x in values`, or `i in 0..n` with `end`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForIn {
    pub id: Id,
    pub name: Name,
    pub source: Operand,
    pub end: Option<Operand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Carry {
    pub id: Id,
    pub name: Name,
    pub ty: Option<TypeExpr>,
    pub value: Operand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub id: Id,
    pub comments: Attached,
    pub items: Vec<PanelItem>,
    pub end: Vec<Comment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelDirection {
    Control,
    Indicator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelItem {
    pub id: Id,
    pub comments: Attached,
    pub direction: PanelDirection,
    pub name: Name,
    pub kind: Name,
    /// `numeric` has `None`, `numeric()` an empty list.
    pub args: Option<Box<[Arg]>>,
}

/// Separate data (D-07): it refers to nodes by name or label, as the file does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub id: Id,
    pub comments: Attached,
    pub items: Vec<LayoutItem>,
    pub end: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutItem {
    pub id: Id,
    pub comments: Attached,
    pub kind: LayoutKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    Node { node: NodeRef, at: Option<Point>, view: Option<View> },
    Wire { from: TerminalRef, to: TerminalRef, via: Vec<Point> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Standard,
    Compact,
}

/// `dmm` or `#close`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeRef {
    pub id: Id,
    pub is_label: bool,
    pub name: Name,
}

/// `raw` or `raw.session`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalRef {
    pub id: Id,
    pub node: NodeRef,
    pub terminal: Option<Name>,
}

/// Coordinates keep their text: `check` refuses `5.0` or `1e3` (`LC5005`), and `fmt` keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub id: Id,
    pub x: Numeral,
    pub y: Numeral,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub id: Id,
    pub comments: Attached,
    /// The text of the size of the grid: `check` evaluates it (`LC5008`).
    pub grid: Name,
    pub items: Vec<IconItem>,
    pub end: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconItem {
    pub id: Id,
    pub comments: Attached,
    pub shape: IconShape,
    pub fill: bool,
    pub accent: Option<Path>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconShape {
    Rect { from: Point, to: Point, radius: Option<Numeral> },
    Circle { center: Point, radius: Numeral },
    Line { from: Point, to: Point },
    Path { data: Box<str> },
    Text { text: Box<str>, at: Point, size: Option<Numeral> },
}

#[cfg(test)]
mod tests {
    use labcraft_types::{Interner, Position};

    use super::*;

    fn span(line: u32) -> Span {
        Span { start: Position { line, column: 1 }, end: Position { line, column: 2 } }
    }

    #[test]
    fn a_source_map_gives_back_what_it_was_given() {
        let mut map = SourceMap::default();
        assert!(map.is_empty());
        let (a, b) = (map.add(span(1)), map.add(span(2)));
        assert_ne!(a, b);
        assert_eq!((map.span(a), map.span(b), map.len()), (Some(span(1)), Some(span(2)), 2));
    }

    #[test]
    fn an_id_past_the_end_gives_no_span() {
        let mut other = SourceMap::default();
        other.add(span(1));
        let foreign = other.add(span(2));
        assert_eq!(SourceMap::default().span(foreign), None);
    }

    #[test]
    fn a_path_lists_its_segments_in_order() {
        let mut names = Interner::new();
        let (a, b, c) = (names.intern("a"), names.intern("b"), names.intern("c"));
        let id = SourceMap::default().add(span(1));
        let path = Path { id, first: a, rest: Box::new([b, c]) };
        assert_eq!(path.segments().collect::<Vec<_>>(), [a, b, c]);
    }

    /// A file of 16 MiB can hold 8 million operands (`[a,a,a…`): each byte counts here.
    #[test]
    fn what_a_file_can_repeat_stays_small() {
        assert!(size_of::<Operand>() <= 32, "{}", size_of::<Operand>());
        assert!(size_of::<Statement>() <= 88, "{}", size_of::<Statement>());
    }
}
