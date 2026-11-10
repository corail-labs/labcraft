//! The bodies of the primitives (`core`, `string`, `array`, `errors`, `time`, `ui`), by the names of `check`'s table of
//! signatures (`docs/architecture.md`, section 10.5): a test requires a body for every signature, and no body without one.
//!
//! A body reads its inputs in the order of `docs/ir.md`, section 4: one per parameter of kind value, in order, a variadic
//! parameter as consecutive inputs, then the free terminals; units, categories, elements and templates are statics. It gives
//! its outputs, or fails with a new error, and the interpreter gives the outputs of a node that fails as of one that skips
//! ([`outputs_on_error`], spec 7.2). What it cannot say on `err` is a fault.

use labcraft_check::PrimitiveLibrary;
use labcraft_ir::{ConstId, ElementId, Skip, Static, Variadic};
use labcraft_types::{LibraryId, TyId};

use crate::convert::{Env, constant};
use crate::fault::Fault;
use crate::value::Value;

mod array;
mod core;
pub use self::core::equal;
mod errors;
#[cfg(test)]
mod lowered;
mod number;
mod string;
#[cfg(test)]
mod tests;
mod time;
mod ui;

/// What a primitive needs from the run that calls it.
pub trait Host {
    /// The clock of the run, in nanoseconds (cli.md 6.2): what `time.now` gives.
    fn now(&mut self) -> i64;
    /// The value of a panel element of the call (spec 10): a `button` is seen pressed by one read.
    fn element(&mut self, element: ElementId) -> Result<Value, Fault>;
    fn set_element(&mut self, element: ElementId, value: Value) -> Result<(), Fault>;
    /// How many bytes of logical size the run can still hold (cli.md 6.2): a body that builds a large value asks first.
    fn room(&self) -> u64;
    /// A new error, made at this node: the host knows where it is, and gives its origin.
    fn error(&self, category: &str, code: i64, message: String) -> Value;
}

/// One call of a primitive.
pub struct Call<'a> {
    pub env: Env<'a>,
    /// The library of the function that holds the node, which names the types its texts show.
    pub viewer: LibraryId,
    /// After the conversions of their wires. A body that fails leaves them, so that an output can pass its input through.
    pub inputs: Vec<Value>,
    /// What each input delivers.
    pub input_types: Vec<TyId>,
    pub output_types: Vec<TyId>,
    pub statics: &'a [Static],
    pub variadic: Option<Variadic>,
}

/// What a body did.
#[derive(Debug)]
pub enum Outcome {
    Done(Vec<Value>),
    /// It failed with this error: its outputs are those of [`outputs_on_error`].
    Fail(Value),
    /// `time.wait`: the node sleeps this many nanoseconds on the clock of the run, then gives `none` on `err`.
    Wait(i64),
    /// `core.call`: the interpreter runs the function with these inputs, in the places of its own.
    Call {
        function: Value,
        inputs: Vec<Value>,
    },
}

type Body = fn(&mut Call<'_>, &mut dyn Host) -> Result<Outcome, Fault>;

/// Every body, by its library and its name.
const BODIES: &[(PrimitiveLibrary, &str, Body)] = &[
    (PrimitiveLibrary::Core, "add", core::add),
    (PrimitiveLibrary::Core, "sub", core::sub),
    (PrimitiveLibrary::Core, "mul", core::mul),
    (PrimitiveLibrary::Core, "div", core::div),
    (PrimitiveLibrary::Core, "abs", core::abs),
    (PrimitiveLibrary::Core, "neg", core::neg),
    (PrimitiveLibrary::Core, "sqrt", core::sqrt),
    (PrimitiveLibrary::Core, "pow", core::pow),
    (PrimitiveLibrary::Core, "min", core::min),
    (PrimitiveLibrary::Core, "max", core::max),
    (PrimitiveLibrary::Core, "lt", core::lt),
    (PrimitiveLibrary::Core, "le", core::le),
    (PrimitiveLibrary::Core, "gt", core::gt),
    (PrimitiveLibrary::Core, "ge", core::ge),
    (PrimitiveLibrary::Core, "eq", core::eq),
    (PrimitiveLibrary::Core, "ne", core::ne),
    (PrimitiveLibrary::Core, "and", core::and),
    (PrimitiveLibrary::Core, "or", core::or),
    (PrimitiveLibrary::Core, "not", core::not),
    (PrimitiveLibrary::Core, "select", core::select),
    (PrimitiveLibrary::Core, "with_unit", core::with_unit),
    (PrimitiveLibrary::Core, "unitless", core::unitless),
    (PrimitiveLibrary::Core, "to_i32", core::to_i32),
    (PrimitiveLibrary::Core, "round_to_i32", core::round_to_i32),
    (PrimitiveLibrary::Core, "to_f32", core::to_f32),
    (PrimitiveLibrary::Core, "call", core::call),
    (PrimitiveLibrary::Array, "len", array::len),
    (PrimitiveLibrary::Array, "get", array::get),
    (PrimitiveLibrary::Array, "set", array::set),
    (PrimitiveLibrary::Array, "push", array::push),
    (PrimitiveLibrary::Array, "concat", array::concat),
    (PrimitiveLibrary::Array, "repeat", array::repeat),
    (PrimitiveLibrary::Array, "slice", array::slice),
    (PrimitiveLibrary::Array, "from_rows", array::from_rows),
    (PrimitiveLibrary::Array, "to_rows", array::to_rows),
    (PrimitiveLibrary::Array, "sum", array::sum),
    (PrimitiveLibrary::String, "concat", string::concat),
    (PrimitiveLibrary::String, "join", string::join),
    (PrimitiveLibrary::String, "format", string::format),
    (PrimitiveLibrary::String, "split", string::split),
    (PrimitiveLibrary::String, "to_f64", string::to_f64),
    (PrimitiveLibrary::String, "to_i32", string::to_i32),
    (PrimitiveLibrary::Time, "now", time::now),
    (PrimitiveLibrary::Time, "elapsed", time::elapsed),
    (PrimitiveLibrary::Time, "wait", time::wait),
    (PrimitiveLibrary::Errors, "make", errors::make),
    (PrimitiveLibrary::Errors, "raise_if", errors::raise_if),
    (PrimitiveLibrary::Errors, "merge", errors::merge),
    (PrimitiveLibrary::Ui, "get", ui::get),
    (PrimitiveLibrary::Ui, "set", ui::set),
];

/// Runs the primitive `library.name` on one call.
pub fn run(library: PrimitiveLibrary, name: &str, call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let body = BODIES
        .iter()
        .find(|(of, named, _)| *of == library && *named == name)
        .map(|(_, _, body)| *body)
        .ok_or_else(|| Fault::bug(format!("no body for the primitive {}.{name}", library.as_str())))?;
    body(call, host)
}

/// The names of the primitives that have a body, for the test against `check`'s table.
pub fn bodies() -> impl Iterator<Item = (PrimitiveLibrary, &'static str)> {
    BODIES.iter().map(|(library, name, _)| (*library, *name))
}

/// The outputs of a node that skips or fails (spec 7.2): an output named like an input passes that input through, the
/// others take their default; the `err` output passes on the error the node received, and when it received none, the
/// one it failed with. A node marked `always` that fails also passes on the error it received in preference to its own.
/// An `[T; 2]` that passes into an `[[T]]` (`array.set`, `ir::passes_through`) goes as its rows, which ask for their room.
pub fn outputs_on_error(
    env: Env<'_>,
    skip: &Skip,
    inputs: &[Value],
    output_types: &[TyId],
    failed: Option<Value>,
    host: &dyn Host,
) -> Result<Vec<Value>, Fault> {
    let Skip::OnError { err, defaults, through, .. } = skip else {
        return Err(Fault::bug("the outputs of a node that skips or fails, for a node that does neither"));
    };
    let received = inputs.get(*err as usize).filter(|value| value.as_error().is_some()).cloned();
    defaults
        .iter()
        .zip(through)
        .enumerate()
        .map(|(output, (default, through))| match through {
            Some(index) if *index == *err => Ok(received.clone().or_else(|| failed.clone()).unwrap_or(Value::NONE)),
            Some(index) => {
                let input = inputs.get(*index as usize).ok_or_else(|| Fault::bug("an output passes through an input that the node has not"))?;
                let into_rows = matches!(output_types.get(output).map(|ty| env.ty(*ty)), Some(labcraft_types::Ty::Array { dimensions: 1, .. }));
                match input {
                    Value::Grid(grid) if into_rows => {
                        let rows = u64::try_from(grid.rows()).unwrap_or(u64::MAX);
                        room_for(
                            host,
                            grid.size().saturating_add(crate::value::VALUE_COST.saturating_mul(rows.saturating_add(1))),
                            "a node that fails",
                        )?;
                        Ok(Value::array(grid.to_rows()))
                    }
                    other => Ok(other.clone()),
                }
            }
            None => default_of(env, *default),
        })
        .collect()
}

fn default_of(env: Env<'_>, id: ConstId) -> Result<Value, Fault> {
    constant(env, id)
}

/// The input at this index.
fn input<'v>(call: &'v Call<'_>, index: usize) -> Result<&'v Value, Fault> {
    call.inputs.get(index).ok_or_else(|| Fault::bug(format!("a primitive without its input {index}")))
}

/// The inputs of the variadic parameter.
fn variadic<'v>(call: &'v Call<'_>) -> Result<&'v [Value], Fault> {
    let Some(variadic) = call.variadic else { return Ok(&[]) };
    let start = variadic.start as usize;
    call.inputs.get(start..start.saturating_add(variadic.count as usize)).ok_or_else(|| Fault::bug("a variadic parameter beyond the inputs"))
}

fn output_type(call: &Call<'_>, index: usize) -> Result<TyId, Fault> {
    call.output_types.get(index).copied().ok_or_else(|| Fault::bug(format!("a primitive without its output {index}")))
}

fn input_type(call: &Call<'_>, index: usize) -> Result<TyId, Fault> {
    call.input_types.get(index).copied().ok_or_else(|| Fault::bug(format!("a primitive without the type of its input {index}")))
}

fn wrong(what: &str) -> Fault {
    Fault::bug(format!("{what}: an input that does not hold its type"))
}

/// A body asks before it builds a value of this logical size: `LC8005` if the run cannot hold it.
fn room_for(host: &dyn Host, size: u64, what: &str) -> Result<(), Fault> {
    if size > host.room() {
        return Err(Fault::memory(format!("{what} would make a value of {size} bytes, beyond the memory left to the run (cli.md 6.2)")));
    }
    Ok(())
}
