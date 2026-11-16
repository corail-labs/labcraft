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
mod delete;
mod model;
mod names;
mod places;
mod refs;
mod scope;
mod select;
mod wire;

pub use select::selectable;

#[cfg(test)]
mod tests;

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
    NodeAdd { file: String, call: String, at: Option<Point>, name: Option<String> },
    /// `constant.add`: `value` is a constant written as in a file (`240 V`).
    ConstantAdd { file: String, value: String, at: Option<Point>, name: Option<String> },
    /// `node.delete`.
    NodeDelete { file: String, nodes: Vec<String> },
    /// `wire.connect`.
    WireConnect { file: String, from: String, to: String, replace: bool, accept_mismatch: bool },
    /// `wire.disconnect`.
    WireDisconnect { file: String, wire: String },
}

impl Op {
    /// The file it edits.
    pub fn file(&self) -> &str {
        match self {
            Op::NodeAdd { file, .. }
            | Op::ConstantAdd { file, .. }
            | Op::NodeDelete { file, .. }
            | Op::WireConnect { file, .. }
            | Op::WireDisconnect { file, .. } => file,
        }
    }
}

/// What an operation gives: the texts it changed, by path, and what it named and renamed (commands.md 9.3). A text that
/// comes out as it went in is not in `texts`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edited {
    pub texts: BTreeMap<String, String>,
    pub names: BTreeMap<&'static str, String>,
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
}

/// Applies one operation to the texts, under the limits of the session (`--max-file-size`, `--max-nesting`). It never
/// writes a text that does not parse (commands.md 9.1).
pub fn apply(files: &Files, op: &Op, judge: &dyn Judge, limits: &Limits) -> Result<Edited, EditError> {
    let path = op.file();
    let Some(text) = files.get(path) else {
        return Err(EditError::new("path.not_found", format!("`{path}` is not a file of the project")).of("file"));
    };
    let mut edit = model::Edit::read(path, text, limits)?;
    let mut edited = Edited::default();
    match op {
        Op::NodeAdd { call, at, name, .. } => add::node(&mut edit, judge, call, *at, name.as_deref(), &mut edited)?,
        Op::ConstantAdd { value, at, name, .. } => add::constant(&mut edit, value, *at, name.as_deref(), &mut edited)?,
        Op::NodeDelete { nodes, .. } => delete::nodes(&mut edit, judge, nodes)?,
        Op::WireConnect { from, to, replace, accept_mismatch, .. } => {
            wire::connect(&mut edit, judge, from, to, *replace, *accept_mismatch, &mut edited)?;
        }
        Op::WireDisconnect { wire, .. } => wire::disconnect(&mut edit, judge, wire, &mut edited)?,
    }
    let new_text = edit.finish()?;
    if new_text != text {
        edited.texts.insert(path.to_owned(), new_text);
    }
    Ok(edited)
}
