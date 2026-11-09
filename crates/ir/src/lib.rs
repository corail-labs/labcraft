//! L4: the dataflow IR of `docs/ir.md`. A program, once `check` has accepted it, as regions of nodes: the nodes of the
//! diagram, each region reading only its arguments, every node with its origin and its canonical position, every value
//! typed. [`lower`] makes it from the `Checked` of a run, [`verify`] checks its invariants, [`print`] gives its text form.
//!
//! Lowering never recurses on what a file can nest: calls and bodies are walked with explicit work lists.
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

use std::collections::HashMap;
use std::sync::Arc;

use labcraft_check::PrimitiveLibrary;
use labcraft_graph::Id;
use labcraft_types::{LibraryId, Name, NumberKind, TyId, TypeTable, Unit};

mod constant;
mod lower;
mod print;
#[cfg(test)]
mod tests;
mod verify;

pub use constant::{ConversionError, apply_scale, scale_between};
pub use lower::{LowerError, lower};
pub use print::{print, print_function};
pub use verify::{Violation, verify};

/// The version of the IR that this crate makes (`docs/ir.md`, section 9).
pub const VERSION: u32 = 1;

/// A function of a [`Program`], by its index in `functions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(pub u32);

/// A constant of a [`Program`], by its index in `constants`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstId(pub u32);

/// A panel element of a [`Function`], by its index in `elements`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(pub u32);

#[derive(Clone, Debug)]
pub struct Program {
    /// [`VERSION`].
    pub version: u32,
    /// By [`FunctionId`]: `functions[0]` is the one the run starts.
    pub functions: Vec<Function>,
    /// By [`ConstId`], shared by all functions, each one once.
    pub constants: Vec<Constant>,
    /// From `check`: every [`TyId`] of the program is in it.
    pub types: TypeTable,
    /// What each type declared in a `.lct` file of the run stands for, through its aliases: `None` for an opaque type.
    pub definitions: HashMap<TyId, Option<TyId>>,
}

/// A function by its library, as `check` numbers them (`Checked::libraries`), and its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FunctionKey {
    pub library: u32,
    pub name: Name,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub key: FunctionKey,
    /// Its library in the table of types, from which its types are shown.
    pub library: LibraryId,
    /// As reports and the trace name it: `scpi.query_f64`, or `check_voltage` in the folder given.
    pub shown: Box<str>,
    /// The file it was read from, by its index in `Checked::files`: origins are ids of that file.
    pub file: u32,
    /// The signature, in order.
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    /// Its panel (spec 10), by [`ElementId`], in the order of the section.
    pub elements: Vec<Element>,
    pub body: Body,
}

/// An input or an output of a signature.
#[derive(Clone, Debug)]
pub struct Port {
    pub name: Name,
    /// The type of a variadic input is the array it takes.
    pub ty: TyId,
    /// An input: its default, if the signature gives one. An output: the default of its type (spec 7.2).
    pub default: Option<ConstId>,
    pub variadic: bool,
    pub origin: Id,
}

/// A panel element (spec 10). Its state belongs to each call of its function, which the runtime keeps.
#[derive(Clone, Debug)]
pub struct Element {
    pub name: Name,
    /// `switch`, `button`, `numeric`…
    pub kind: Name,
    /// As `check` settled it: `None` for an element that no node reads or writes, and that nothing else gives a type.
    pub ty: Option<TyId>,
    /// That of its input, which a required input does not have; its `default:`; or the default of its type (spec 7.2, 10).
    /// `None` without a type.
    pub default: Option<ConstId>,
    /// The input or the output it is the element of.
    pub port: Option<PortOf>,
    pub origin: Id,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortOf {
    Input(u32),
    Output(u32),
}

#[derive(Clone, Debug)]
pub enum Body {
    Region(Region),
    /// What an `extern` declaration names (spec 4.1). `runs` is true for `extern rust`, whose library gives the body;
    /// a call of another gives `extern.unavailable` on `err` in M0. `skip` is what a call of it does with an error (7.2), in
    /// the places of its own inputs: what a `core.call` of it applies, one place further on.
    Native {
        library: LibraryId,
        function: Name,
        runs: bool,
        skip: Skip,
    },
}

/// One body: of a function, of a loop, of a `for`, or of one arm of a `match` or a `handle`.
#[derive(Clone, Debug)]
pub struct Region {
    /// The declaration of its function, the statement of its structure, or its arm.
    pub origin: Id,
    /// The values that enter it, typed.
    pub args: Vec<Value>,
    /// In canonical order.
    pub nodes: Vec<Node>,
    /// The values that leave it.
    pub results: Vec<Use>,
}

impl Region {
    /// The values that the structures inside it pass in: on each structure node, its inputs from outside, once on the node and
    /// once among the arguments of each of its regions (`docs/ir.md`, section 4). What `check::passed_in` counts on the source,
    /// and bounds with `LC4007`. Regions nest as deep as `check` lets them: an explicit stack.
    pub fn passed_in(&self) -> u64 {
        let mut total = 0_u64;
        let mut pending = vec![self];
        while let Some(region) = pending.pop() {
            for node in &region.nodes {
                let (passed, regions): (usize, Vec<&Region>) = match &node.kind {
                    NodeKind::Loop { carries, region, .. } => (node.inputs.len().saturating_sub(*carries as usize), vec![region]),
                    NodeKind::For { clauses, carries, region, .. } => {
                        (region.args.len().saturating_sub(clauses.len()).saturating_sub(*carries as usize), vec![region])
                    }
                    NodeKind::Match { arms } | NodeKind::Handle { arms } => {
                        (node.inputs.len().saturating_sub(1), arms.iter().map(|arm| &arm.region).collect())
                    }
                    _ => continue,
                };
                let passed = u64::try_from(passed).unwrap_or(u64::MAX);
                let frames = u64::try_from(regions.len()).unwrap_or(u64::MAX).saturating_add(1);
                total = total.saturating_add(passed.saturating_mul(frames));
                pending.extend(regions);
            }
        }
        total
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    /// In the order of `docs/ir.md`, section 4.
    pub inputs: Vec<Use>,
    /// One per output of what it computes.
    pub outputs: Vec<Value>,
    /// What it waits for without reading it.
    pub after: Vec<Order>,
    /// The canonical index of its line in its body (spec 5.2).
    pub position: u32,
    /// Its statement.
    pub origin: Id,
    /// As the trace names it: its first output, its label (`#close`), or `@12` (`docs/commands.md`, 8).
    pub name: Name,
    /// What an error on its `err` input does (`docs/ir.md`, section 5).
    pub skip: Skip,
}

/// A value, defined once: by an argument of its region, or by an output of one of its nodes.
#[derive(Clone, Debug)]
pub struct Value {
    pub ty: TyId,
    /// The names it bears: that of its line, then each new name a line gives it (`err = e3`). Shared by every argument that
    /// passes the value into a region, so that a value of many names passed into many regions costs its names once.
    pub names: Arc<[Name]>,
}

/// A wire as one input reads it.
#[derive(Clone, Debug)]
pub struct Use {
    pub source: Source,
    /// The fields read on the way, by their index in each struct (`bench.dmm`).
    pub fields: Vec<u32>,
    /// What `check` recorded for this wire (spec 3.3, 3.4), done after the fields.
    pub convert: Option<Conversion>,
    /// What it delivers, after its fields and its conversion.
    pub ty: TyId,
    /// The operand it was written as, or the line that binds it.
    pub origin: Id,
}

#[derive(Clone, Debug)]
pub enum Source {
    Arg(u32),
    Out {
        node: u32,
        output: u32,
    },
    Const(ConstId),
    /// An array written of wires as an argument (`string.join([a, b])`): a use of uses.
    Array(Vec<Use>),
}

#[derive(Clone, Debug)]
pub enum Order {
    /// A value of this region whose definition the node waits for.
    Value(Use),
    /// A node of this region, by its index, named by its label (`#close`).
    Node(u32),
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    /// A named constant (`limit = 240 V`): its input is the literal in its own type.
    Constant,
    Primitive {
        library: PrimitiveLibrary,
        name: Name,
        /// The arguments that are data of the node, never wires (R-1, R-5).
        statics: Vec<Static>,
        variadic: Option<Variadic>,
    },
    /// An `extern` function: its [`Function`] has a native body.
    Native {
        function: FunctionId,
        variadic: Option<Variadic>,
    },
    /// A function written in LabCraft.
    Call {
        function: FunctionId,
        variadic: Option<Variadic>,
    },
    /// A struct, its fields in declaration order; or a variant, by its index, with its values.
    Construct {
        ty: TyId,
        variant: Option<u32>,
    },
    BuildArray,
    Loop {
        continue_on_error: bool,
        carries: u32,
        collects: u32,
        region: Region,
    },
    For {
        parallel: bool,
        clauses: Vec<Clause>,
        carries: u32,
        collects: u32,
        region: Region,
    },
    /// The arms in the order they are tried, `_` last.
    Match {
        arms: Vec<Arm>,
    },
    Handle {
        arms: Vec<Arm>,
    },
}

/// Where the arguments of a variadic parameter sit among the inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variadic {
    pub start: u32,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Static {
    /// `[V]`, for a parameter of kind unit.
    Unit(Unit),
    /// `io.timeout`, for a parameter of kind category.
    Category(Box<str>),
    /// `stop_switch`, for a parameter of kind element.
    Element(ElementId),
    /// The template of `string.format`, its escapes read.
    Template(Box<str>),
}

/// An `in` clause of a `for`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clause {
    /// `x in values`: one input, an array; its argument is an element.
    Each,
    /// `i in 0..n`: two inputs, the bounds; its argument is the index.
    Range,
}

#[derive(Clone, Debug)]
pub struct Arm {
    /// One or more (`1, 3..=5`); only a single pattern binds names.
    pub patterns: Vec<Pattern>,
    pub origin: Id,
    /// Its arguments: what its pattern binds, then the values passed in.
    pub region: Region,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    /// A number, a text or a boolean, in the type of the selector.
    Literal(ConstId),
    /// Integers, in the type of the selector.
    Range {
        low: i128,
        high: i128,
        inclusive: bool,
    },
    /// A variant by its index in its enumeration, binding as many values as it holds.
    Variant {
        variant: u32,
        binds: u32,
    },
    /// `io.timeout`, or `visa.*` with `wildcard`.
    Category {
        category: Box<str>,
        wildcard: bool,
        binds: bool,
    },
    /// `code -1073807339`
    Code {
        code: i128,
        binds: bool,
    },
    Ok,
    /// `_`, or `_(err)` in a `handle`.
    Wildcard {
        binds: bool,
    },
}

/// What a node does with an error on its `err` input (spec 7.2), decided at lowering.
#[derive(Clone, Debug, PartialEq)]
pub enum Skip {
    /// No `err` input, a structure, a construction, a constant.
    Never,
    /// A call of a LabCraft function: it runs its body, whose own nodes skip.
    Runs,
    OnError {
        /// Which input is `err`.
        err: u32,
        /// From the manifest of its library.
        always: bool,
        /// The default of each output.
        defaults: Vec<ConstId>,
        /// The output named like an input passes that input through (5.3).
        through: Vec<Option<u32>>,
    },
    /// `core.call`: the skip of the function it runs, whatever names the function type gives. A LabCraft function `Runs`; an
    /// `extern` function has its own in its `Body::Native`, and a primitive in its `ConstValue::Primitive`. Their inputs and
    /// outputs are in the places of the function, so the node's inputs, which start with the function, are one place on.
    ByCallee,
}

/// A conversion on a wire (spec 3.1, 3.3, 3.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Conversion {
    /// To the kind `to`, then the scale, in that kind.
    Number { to: NumberKind, scale: Scale },
    /// To `f64[s]` by the scale, then times 10^9, rounded to the nearest integer, ties to even.
    ToDuration { scale: Scale },
    /// The nanoseconds as an `f64`, divided by 10^9, then the scale, then to the kind `to`.
    FromDuration { to: NumberKind, scale: Scale },
}

/// A change of unit, each step one operation (spec 3.3): the offset of the unit it leaves is added, then the value is
/// multiplied by the numerator of the ratio of the scales and divided by its denominator, then the offset of the target
/// is taken away. A step that does nothing is `None`, since adding zero would change `-0.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scale {
    pub add: Option<f64>,
    pub multiply: Option<f64>,
    pub divide: Option<f64>,
    pub subtract: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Constant {
    pub ty: TyId,
    pub value: ConstValue,
}

#[derive(Clone, Debug)]
pub enum ConstValue {
    Bool(bool),
    /// Any integer kind.
    Int(i128),
    /// `f64`, or an `f32` held exactly.
    Float(f64),
    Complex {
        re: f64,
        im: f64,
    },
    Text(Box<str>),
    Bytes(Box<[u8]>),
    /// Whole nanoseconds.
    Duration(i64),
    /// `none`
    NoError,
    Array(Vec<ConstId>),
    /// The fields in declaration order; for a `waveform`, `t0`, `dt`, `samples`.
    Struct(Vec<ConstId>),
    Variant {
        variant: u32,
        values: Vec<ConstId>,
    },
    Function(FunctionId),
    /// A primitive as a value, with what a call of it does with an error (7.2), in the places of its own inputs, as its
    /// function type gives them.
    Primitive {
        library: PrimitiveLibrary,
        name: Name,
        skip: Skip,
    },
    /// The default of an opaque type, on which the functions of its library fail (7.2).
    InvalidHandle,
    /// The `length` of `array.slice` with no wire: it reaches the end of the array.
    ToTheEnd,
    /// A default that Q-21 has not settled: a `timestamp`, a function reference.
    Unsettled,
}
