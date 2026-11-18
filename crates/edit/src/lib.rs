//! L4: the editing operations of `docs/commands.md`, section 6.7, as pure functions over the text of the files
//! (`docs/architecture.md`, 12.3). Each reads the texts it needs, applies one operation, and returns the new texts with
//! what it named and renamed (commands.md 9.3), or an error of section 3 of commands.md. The rules of the gestures that
//! change the text are here, once, for the IDE and the agent: the name that an insertion gives (N-14), a value bound to an
//! output that takes the name of the output and gives it back when unwired, the hole written where the text cannot leave a
//! place out, what a delete removes with a node (commands.md 9.1, visual notation 2.4).
//!
//! No history, no file, no thread. What an operation must know of the checked program (the signature of a call, the type
//! errors that a wire would make) comes through [`Judge`], which `engine` implements from the kept check of the session,
//! so that this crate reads no library itself. Every text it gives is in canonical form (spec 5.2) and reads again: a text
//! that does not is `internal`, a bug of this crate, which the fuzzing looks for (12.10).
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

use std::collections::BTreeMap;

use labcraft_types::{Diagnostic, Limits};

mod add;
mod constant;
mod delete;
mod fragment;
mod frame;
mod into;
mod layout;
mod model;
mod names;
mod parts;
mod places;
mod refs;
mod rename;
mod route;
mod scope;
mod select;
mod surround;
mod wire;

pub use fragment::MAX_FRAGMENT;
pub use frame::Within;
pub use names::of_type;
pub use route::MAX_VIA;
pub use select::selectable;

#[cfg(test)]
mod tests;

/// At most this many characters of a text that a caller gave go into a message, then `…` (as `wire.occupied` shows what
/// an input holds).
const SHOWN: usize = 60;

/// A text that a caller gave, cut for a message.
pub(crate) fn cut(text: &str) -> String {
    if text.chars().count() <= SHOWN {
        return text.to_owned();
    }
    let mut shown: String = text.chars().take(SHOWN - 1).collect();
    shown.push('…');
    shown
}

/// The texts an operation may read, by path in the project (commands.md 2).
#[derive(Clone, Debug, Default)]
pub struct Files<'a> {
    texts: BTreeMap<&'a str, &'a str>,
}

impl<'a> Files<'a> {
    pub fn new() -> Self {
        Files::default()
    }

    pub fn insert(&mut self, path: &'a str, text: &'a str) {
        self.texts.insert(path, text);
    }

    pub fn get(&self, path: &str) -> Option<&'a str> {
        self.texts.get(path).copied()
    }
}

/// A position of `layout`, in canvas units (spec 11, commands.md 9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

/// The bound of a coordinate (spec 11, *proposed*).
pub const MAX_COORDINATE: i64 = 1_000_000;

/// One operation of the catalog, read from the parameters of its command by `engine`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// `node.add`: `call` is a function as a call names it, a struct type, or a variant that holds values.
    NodeAdd {
        file: String,
        call: String,
        at: Option<Point>,
        name: Option<String>,
        /// A wire to insert the node into.
        into: Option<String>,
        within: Within,
    },
    /// `constant.add`: `value` is a constant written as in a file (`240 V`).
    ConstantAdd {
        file: String,
        value: String,
        at: Option<Point>,
        name: Option<String>,
        within: Within,
    },
    /// `node.delete`.
    NodeDelete {
        file: String,
        nodes: Vec<String>,
    },
    /// `wire.connect`.
    WireConnect {
        file: String,
        from: String,
        to: String,
        /// The arm of a `yield` that `to` names.
        arm: Option<String>,
        replace: bool,
        accept_mismatch: bool,
        /// How each loop that the wire leaves takes the value out (commands.md 6.7).
        through: Option<Through>,
    },
    /// `wire.disconnect`.
    WireDisconnect {
        file: String,
        wire: String,
        /// The arm of a `yield` that the wire reaches.
        arm: Option<String>,
    },
    /// `node.move`: the nodes of one frame by `by`, each fixed.
    NodeMove {
        file: String,
        nodes: Vec<String>,
        by: Point,
    },
    /// `frame.resize`: `frame size (w, h)`, or `frame ys size (w, h)` for the structure `inside`; with no size, its
    /// entry goes.
    FrameResize {
        file: String,
        inside: Option<String>,
        size: Option<Point>,
    },
    /// `node.place`: one node at `at` in its frame, fixed.
    NodePlace {
        file: String,
        node: String,
        at: Point,
    },
    /// `node.set_view`: `None` is `default`, which removes the view.
    NodeSetView {
        file: String,
        nodes: Vec<String>,
        view: Option<labcraft_graph::View>,
    },
    /// `layout.arrange`: `nodes` is `None` when the command names no node.
    LayoutArrange {
        file: String,
        nodes: Option<Vec<String>>,
        wires: Vec<String>,
        terminals: Vec<String>,
        /// A branch to arrange.
        within: Within,
    },
    WireRoute {
        file: String,
        wire: String,
        via: Vec<Point>,
    },
    /// `input.set_constant`: `value` is a constant written as in a file.
    InputSetConstant {
        file: String,
        to: String,
        /// The arm of a `yield` that `to` names.
        arm: Option<String>,
        value: String,
        replace: bool,
        accept_mismatch: bool,
    },
    /// `input.clear_constant`.
    InputClearConstant {
        file: String,
        to: String,
        /// The arm of a `yield` that `to` names.
        arm: Option<String>,
    },
    /// `input.remove_value`: a value of a variadic parameter or an element of an array.
    InputRemoveValue {
        file: String,
        to: String,
    },
    /// `value.rename`: a value, or a label (`#close`).
    ValueRename {
        file: String,
        value: String,
        name: String,
    },
    /// `node.set_label`: the node that starts on `line`, from 1.
    NodeSetLabel {
        file: String,
        line: u32,
        label: String,
    },
    /// `node.set_template`: `template` is a string literal as the text writes it.
    NodeSetTemplate {
        file: String,
        node: String,
        template: String,
    },
    /// `fragment.paste`: `text` holds a function, whose body goes into the frame, its top-left corner at `at`.
    FragmentPaste {
        file: String,
        text: String,
        at: Point,
        within: Within,
    },
    /// `structure.surround`: the nodes of one frame in a new structure, or an empty one in the frame of `within` when
    /// `nodes` is empty (commands.md 9.6).
    StructureSurround {
        file: String,
        kind: StructureKind,
        nodes: Vec<String>,
        selector: Option<String>,
        over: Option<String>,
        /// The arm that takes the nodes.
        arm: Option<String>,
        within: Within,
        at: Option<Point>,
    },
    /// `structure.add_arm`.
    StructureAddArm {
        file: String,
        structure: String,
        patterns: String,
    },
    /// `structure.set_patterns`.
    StructureSetPatterns {
        file: String,
        structure: String,
        arm: String,
        patterns: String,
    },
    /// `structure.add_carry`.
    StructureAddCarry {
        file: String,
        structure: String,
        name: String,
        initial: Option<String>,
        ty: Option<String>,
    },
    /// `structure.add_clause`.
    StructureAddClause {
        file: String,
        structure: String,
        over: String,
        name: Option<String>,
    },
    /// `structure.add_collect`.
    StructureAddCollect {
        file: String,
        structure: String,
        value: String,
        name: Option<String>,
    },
    /// `structure.add_last`.
    StructureAddLast {
        file: String,
        structure: String,
        value: String,
        name: Option<String>,
    },
    /// `structure.remove_part`: an arm by its patterns, `carry x`, `in x`, `collect y` or `last y`.
    StructureRemovePart {
        file: String,
        structure: String,
        part: String,
    },
    /// `structure.set_passage`: a part `carry x`, `collect y` or `last y` of a loop becomes another of the three.
    StructureSetPassage {
        file: String,
        structure: String,
        part: String,
        to: Passage,
    },
    /// `structure.set_option`.
    StructureSetOption {
        file: String,
        structure: String,
        option: StructureOption,
        on: bool,
    },
}

/// An option of a loop (spec 6.2, 6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureOption {
    ContinueOnError,
    Parallel,
}

/// How a loop takes a value out (commands.md 6.7, `wire.connect`): a carry that holds its last value, a `collect`, or a
/// `last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Passage {
    Carry,
    Collect,
    Last,
}

/// `through`: one passage for every loop that a wire leaves, or one per loop, from the inside out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Through {
    All(Passage),
    Each(Vec<Passage>),
}

/// The four structures of spec 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureKind {
    Match,
    Handle,
    For,
    Loop,
}

impl StructureKind {
    pub fn parse(text: &str) -> Option<StructureKind> {
        match text {
            "match" => Some(StructureKind::Match),
            "handle" => Some(StructureKind::Handle),
            "for" => Some(StructureKind::For),
            "loop" => Some(StructureKind::Loop),
            _ => None,
        }
    }

    pub fn keyword(self) -> &'static str {
        match self {
            StructureKind::Match => "match",
            StructureKind::Handle => "handle",
            StructureKind::For => "for",
            StructureKind::Loop => "loop",
        }
    }
}

impl Op {
    /// The file it edits.
    pub fn file(&self) -> &str {
        match self {
            Op::NodeAdd { file, .. }
            | Op::ConstantAdd { file, .. }
            | Op::NodeDelete { file, .. }
            | Op::WireConnect { file, .. }
            | Op::WireDisconnect { file, .. }
            | Op::NodeMove { file, .. }
            | Op::FrameResize { file, .. }
            | Op::NodePlace { file, .. }
            | Op::NodeSetView { file, .. }
            | Op::LayoutArrange { file, .. }
            | Op::WireRoute { file, .. }
            | Op::InputSetConstant { file, .. }
            | Op::InputClearConstant { file, .. }
            | Op::InputRemoveValue { file, .. }
            | Op::ValueRename { file, .. }
            | Op::NodeSetLabel { file, .. }
            | Op::NodeSetTemplate { file, .. }
            | Op::FragmentPaste { file, .. }
            | Op::StructureSurround { file, .. }
            | Op::StructureAddArm { file, .. }
            | Op::StructureSetPatterns { file, .. }
            | Op::StructureAddCarry { file, .. }
            | Op::StructureAddClause { file, .. }
            | Op::StructureAddCollect { file, .. }
            | Op::StructureAddLast { file, .. }
            | Op::StructureRemovePart { file, .. }
            | Op::StructureSetPassage { file, .. }
            | Op::StructureSetOption { file, .. } => file,
        }
    }
}

/// What an operation gives: the texts it changed, by path, and what it named and renamed (commands.md 9.3). A text that
/// comes out as it went in is not in `texts`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edited {
    pub texts: BTreeMap<String, String>,
    pub names: BTreeMap<&'static str, String>,
    /// What it named, when that is several names: `nodes` of `node.move`, as a command names them after it.
    pub lists: BTreeMap<&'static str, Vec<String>>,
    pub renamed: Vec<Renamed>,
}

/// A value that the rules of the gestures renamed as well (visual notation 2.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renamed {
    pub file: String,
    pub from: String,
    pub to: String,
}

/// An error of section 3 of commands.md.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditError {
    pub code: &'static str,
    pub message: String,
    /// The parameter it is about, when it is about one.
    pub parameter: Option<&'static str>,
    /// For `edit.unknown`, the nearest names; for `edit.ambiguous`, what the name names.
    pub expected: Vec<String>,
}

impl EditError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> EditError {
        EditError { code, message: message.into(), parameter: None, expected: Vec::new() }
    }

    pub(crate) fn of(mut self, parameter: &'static str) -> EditError {
        self.parameter = Some(parameter);
        self
    }

    pub(crate) fn expecting(mut self, expected: Vec<String>) -> EditError {
        self.expected = expected;
        self
    }

    pub(crate) fn invalid(parameter: &'static str, message: impl Into<String>) -> EditError {
        EditError::new("params.invalid", message).of(parameter)
    }

    pub(crate) fn internal(message: impl Into<String>) -> EditError {
        EditError::new("internal", message)
    }
}

/// What a name of a call names, as the check of the session sees it from a file (commands.md 6.7, `node.add`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Callable {
    pub kind: Kind,
    /// The library that the file must `use` for it, when the call names one (`visa` for `visa.open`).
    pub library: Option<String>,
    /// Its inputs, in order: the parameters of a function, the fields of a struct, the values of a variant (no name).
    pub inputs: Vec<Input>,
    /// The names of its outputs, in order; a struct or a variant has one, the name of its type.
    pub outputs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Function,
    Struct,
    Variant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// Empty for a value of a variant, which has no name.
    pub name: String,
    pub required: bool,
    /// `parts: string...` (spec 5.4).
    pub variadic: bool,
    /// After a variadic parameter, a parameter is wired by name only (R-11).
    pub named_only: bool,
}

/// What `Judge::callable` finds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(Callable),
    /// Nothing of that name: the nearest names, for `edit.unknown`.
    Unknown(Vec<String>),
}

/// What an operation must know of the checked program, which `engine` gives from the kept check of the session.
pub trait Judge {
    /// What `call` names, seen from `file`: `visa.open`, `settle` (a function of the same library), `Point`, `Reading.Value`.
    fn callable(&self, file: &str, call: &str) -> Lookup;

    /// The errors of `file` when its text is `text`, the rest of the project as it is: what a wire changes in them tells
    /// a wire of the wrong type.
    fn errors(&self, file: &str, text: &str) -> Vec<Diagnostic>;

    /// Where the drawing of `file`, when its text is `text`, places each node, in its frame: what a move by `[dx, dy]`
    /// of a node placed automatically starts from (architecture 12.3). None when the function is not drawn.
    fn places(&self, file: &str, text: &str) -> Vec<Place> {
        let _ = (file, text);
        Vec::new()
    }

    /// The wires of the function of `file` as the drawing of `text` routes them, one per reader that a `wire` entry can
    /// name: what `wire.route` names, and the trunk that a junction follows (notation 2.5). Empty when it is not drawn.
    fn wires(&self, file: &str, text: &str) -> Vec<WireDrawn> {
        let _ = (file, text);
        Vec::new()
    }

    /// The least size of each frame of the function of `file` as the drawing of `text` draws it, from its origin: what
    /// its content needs, whatever its `frame … size` entry (spec 11). Empty when it is not drawn.
    fn frames(&self, file: &str, text: &str) -> Vec<FrameDrawn> {
        let _ = (file, text);
        Vec::new()
    }

    /// The type of a value of `file` when its text is `text`, as the check sees it: the value `name` bound on `line`, from
    /// 1, by a line, a pattern, a clause or a carry, or the input of the function of that name when `line` is 0. What a
    /// structure needs to know of its selector and of what crosses it (commands.md 9.6).
    fn value_type(&self, file: &str, text: &str, line: u32, name: &str) -> ValueType {
        let _ = (file, text, line, name);
        ValueType::default()
    }

    /// The fields of a `string.format` template, `literal` as the text writes it between its quotes, each once in the
    /// order they first appear (spec 5.4); or the message of `check` for a template whose fields do not read (R-12).
    fn template(&self, literal: &str) -> Result<Vec<String>, String> {
        let _ = literal;
        Err("this judge reads no template".to_owned())
    }
}

/// The type of a value, as far as a structure needs it (commands.md 9.6), and as the check writes it (`f64[V]`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValueType {
    pub kind: TypeKind,
    pub shown: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TypeKind {
    /// No type yet: an error upstream, or a value that the check could not find.
    #[default]
    Unknown,
    Bool,
    /// An integer without a unit.
    Integer,
    Text,
    /// `error`.
    Error,
    Array,
    /// An enumeration: the library that the file must `use` for its variants, when it is another one, and its variants.
    Enum {
        library: Option<String>,
        variants: Vec<VariantShape>,
    },
    /// Any other type: a float, a number with a unit, a struct.
    Other,
}

/// A variant of an enumeration, as a pattern writes it from the file (`Verdict.Pass`, `stations.State.Init`), and the
/// base of the name of each value it holds (N-14): `step` for a `Step`, `value` for a type of spec 3.1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantShape {
    pub path: String,
    pub values: Vec<String>,
}

/// A wire drawn, to one reader: its name as `wire.route` takes it (`reading -> verdict.a`), its value, the body that holds
/// it, and its path from its source to its reader, in hundredths, in the frame of its body (spec 11), the trunk that it
/// shares with the other wires of its value included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireDrawn {
    pub name: String,
    pub value: String,
    pub body: usize,
    pub path: Vec<(i64, i64)>,
}

/// A frame of a drawing and its size (spec 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDrawn {
    /// The structure, as the drawing names it (the name of its line, else its label, else `@N`); none for the frame of
    /// the function.
    pub structure: Option<String>,
    /// The line where the structure starts, from 1; 0 for the frame of the function.
    pub line: u32,
    /// The width and the height that its content needs from its origin, in units, rounded up: the least `frame.resize`
    /// writes.
    pub least: Point,
}

/// A node of a drawing, where it is in its frame, in units (spec 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The line where it starts in the text drawn, from 1.
    pub line: u32,
    /// As a command names it, `#close` or `dmm`; empty for a node that has no such name.
    pub name: String,
    pub at: Point,
    /// Where it is on the canvas, from the origin of the frame of the function: what tells where a frame starts inside
    /// another (`structure.surround`).
    pub absolute: Point,
}

/// Gives a label to each node of `path` that `lines` names by its line and that the `layout` section cannot name, in one
/// pass (commands.md 6.7, `commands.batch`): the text, if it changed, and each line that names one node with how a
/// command names that node now (`@41`, `#s`), in `renamed`; `lists["labelled"]` holds the lines whose node got its
/// label now.
pub fn label_lines(files: &Files, path: &str, lines: &[u32], limits: &Limits) -> Result<Edited, EditError> {
    let Some(text) = files.get(path) else {
        return Err(EditError::new("path.not_found", format!("`{path}` is not a file of the project")).of("file"));
    };
    let mut edit = model::Edit::read(path, text, limits)?;
    let found = layout::label_lines(&mut edit, lines)?;
    let labelled: Vec<String> = found.iter().filter(|(_, _, new)| *new).map(|(from, _, _)| from.clone()).collect();
    let renamed = found.into_iter().map(|(from, to, _)| Renamed { file: path.to_owned(), from, to }).collect();
    let mut edited = Edited { renamed, ..Edited::default() };
    edited.lists.insert("labelled", labelled);
    let new_text = edit.finish()?;
    if new_text != text {
        edited.texts.insert(path.to_owned(), new_text);
    }
    Ok(edited)
}

/// `fragment.copy` (commands.md 6.7): the nodes of `path` that `nodes` names, in one frame, as the text of a function
/// `clipboard` in canonical form, with the `use` lines they need and their `layout` entries counted from the top-left
/// corner of the selection (notation 2.6). It writes nothing.
pub fn copy(files: &Files, path: &str, nodes: &[String], judge: &dyn Judge, limits: &Limits) -> Result<String, EditError> {
    let Some(text) = files.get(path) else {
        return Err(EditError::new("path.not_found", format!("`{path}` is not a file of the project")).of("file"));
    };
    let edit = model::Edit::read(path, text, limits)?;
    fragment::copy(&edit, judge, nodes)
}

/// Applies one operation to the texts, under the limits of the session (`--max-file-size`, `--max-nesting`). It never
/// writes a text that does not parse (commands.md 9.1).
pub fn apply(files: &Files, op: &Op, judge: &dyn Judge, limits: &Limits) -> Result<Edited, EditError> {
    let path = op.file();
    let Some(text) = files.get(path) else {
        return Err(EditError::new("path.not_found", format!("`{path}` is not a file of the project")).of("file"));
    };
    let mut edited = Edited::default();
    if let Op::NodeAdd { call, at, name, into: Some(into), within, .. } = op {
        if within.inside.is_some() || within.arm.is_some() {
            return Err(EditError::invalid(
                "inside",
                "a node added `into` a wire goes in the frame of that wire: `inside` and `arm` do not go with it",
            ));
        }
        let inserted = into::node(path, text, judge, limits, call, *at, name.as_deref(), into, &mut edited)?;
        let new_text = route::follow(path, text, inserted, judge, limits)?;
        if new_text != text {
            edited.texts.insert(path.to_owned(), new_text);
        }
        return Ok(edited);
    }
    let mut edit = model::Edit::read(path, text, limits)?;
    match op {
        Op::NodeAdd { call, at, name, within, .. } => {
            let body = frame::target(&edit, within, "inside", "arm")?;
            add::node(&mut edit, judge, call, *at, name.as_deref(), &body, &mut edited)?;
        }
        Op::ConstantAdd { value, at, name, within, .. } => {
            let body = frame::target(&edit, within, "inside", "arm")?;
            add::constant(&mut edit, value, *at, name.as_deref(), &body, &mut edited)?;
        }
        Op::NodeDelete { nodes, .. } => delete::nodes(&mut edit, judge, nodes)?,
        Op::WireConnect { from, to, arm, replace, accept_mismatch, through, .. } => {
            let how = wire::How { replace: *replace, accept_mismatch: *accept_mismatch, through: through.as_ref() };
            wire::connect(&mut edit, judge, from, to, arm.as_deref(), &how, &mut edited)?;
        }
        Op::WireDisconnect { wire, arm, .. } => wire::disconnect(&mut edit, judge, wire, arm.as_deref(), &mut edited)?,
        Op::NodeMove { nodes, by, .. } => layout::move_nodes(&mut edit, judge, nodes, *by, &mut edited)?,
        Op::NodePlace { node, at, .. } => layout::place_node(&mut edit, node, *at, &mut edited)?,
        Op::FrameResize { inside, size, .. } => layout::resize(&mut edit, judge, inside.as_deref(), size.as_ref().copied(), &mut edited)?,
        Op::NodeSetView { nodes, view, .. } => layout::set_view(&mut edit, nodes, *view, &mut edited)?,
        Op::LayoutArrange { nodes, wires, terminals, within, .. } => {
            let branch = if *within == Within::default() { None } else { Some(frame::target(&edit, within, "inside", "arm")?) };
            layout::arrange(&mut edit, judge, nodes.as_deref(), wires, terminals, branch.as_deref())?;
        }
        Op::WireRoute { wire, via, .. } => route::route(&mut edit, judge, wire, via, &mut edited)?,
        Op::InputSetConstant { to, arm, value, replace, accept_mismatch, .. } => {
            constant::set(&mut edit, judge, to, arm.as_deref(), value, *replace, *accept_mismatch)?;
        }
        Op::InputClearConstant { to, arm, .. } => constant::clear(&mut edit, judge, to, arm.as_deref())?,
        Op::InputRemoveValue { to, .. } => constant::remove_value(&mut edit, judge, to)?,
        Op::ValueRename { value, name, .. } => rename::value(&mut edit, value, name, &mut edited)?,
        Op::NodeSetLabel { line, label, .. } => rename::set_label(&mut edit, *line, label, &mut edited)?,
        Op::NodeSetTemplate { node, template, .. } => rename::set_template(&mut edit, judge, node, template)?,
        Op::FragmentPaste { text, at, within, .. } => {
            let body = frame::target(&edit, within, "inside", "arm")?;
            fragment::paste(&mut edit, judge, text, *at, &body, &mut edited)?;
        }
        Op::StructureSurround { kind, nodes, selector, over, arm, within, at, .. } => {
            let given = surround::Given { kind: *kind, selector: selector.as_deref(), over: over.as_deref(), arm: arm.as_deref(), at: *at };
            surround::surround(&mut edit, judge, nodes, within, &given, &mut edited)?;
        }
        Op::StructureAddArm { structure, patterns, .. } => parts::add_arm(&mut edit, structure, patterns, &mut edited)?,
        Op::StructureSetPatterns { structure, arm, patterns, .. } => parts::set_patterns(&mut edit, structure, arm, patterns, &mut edited)?,
        Op::StructureAddCarry { structure, name, initial, ty, .. } => {
            parts::add_carry(&mut edit, structure, name, initial.as_deref(), ty.as_deref(), &mut edited)?;
        }
        Op::StructureAddClause { structure, over, name, .. } => parts::add_clause(&mut edit, judge, structure, over, name.as_deref(), &mut edited)?,
        Op::StructureAddCollect { structure, value, name, .. } => {
            parts::add_gather(&mut edit, labcraft_graph::Gather::Collect, structure, value, name.as_deref(), &mut edited)?;
        }
        Op::StructureAddLast { structure, value, name, .. } => {
            parts::add_gather(&mut edit, labcraft_graph::Gather::Last, structure, value, name.as_deref(), &mut edited)?;
        }
        Op::StructureRemovePart { structure, part, .. } => parts::remove_part(&mut edit, judge, structure, part)?,
        Op::StructureSetPassage { structure, part, to, .. } => parts::set_passage(&mut edit, judge, structure, part, *to, &mut edited)?,
        Op::StructureSetOption { structure, option, on, .. } => parts::set_option(&mut edit, structure, *option, *on)?,
    }
    let new_text = route::follow(path, text, edit.finish()?, judge, limits)?;
    if new_text != text {
        edited.texts.insert(path.to_owned(), new_text);
    }
    Ok(edited)
}
