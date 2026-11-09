//! Step 1 (`docs/architecture.md`, 11.3): the graph of a function, from what `check` resolved. Nodes in the canonical
//! order of their body (spec 5.2), their terminals in signature order with their families (notation 4.1). `LC7002` is
//! decided here: the nodes are counted before anything is built, the terminals as the graph is built.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use labcraft_check::primitives::{ParamKind, primitive};
use labcraft_check::{Checked, CheckedFile, DeclaredLibrary, Meaning, PrimitiveLibrary, Resolved, Terminal as Wiring, read_escapes};
use labcraft_format::{BodyOrders, canonical_orders, write_operand};
use labcraft_graph::{
    Arg, Body, Call, Decl, File, Function, Icon, Id, LayoutKind, Numeral, NumeralKind, Operand, Rhs, Signature, Statement, StatementKind, StructLit,
    Target, View,
};
use labcraft_types::{Interner, LibraryId, Name, Ty, TyId, TypeTable, cut_text};

/// How many bytes of a constant are written before it is cut: far more than the 40 cells a pill shows.
const CONSTANT_BYTES: usize = 1024;

mod flow;

/// The family of a terminal, which its shape tells (notation 4.1). An array is the family of its element, with its
/// dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Family {
    pub base: Base,
    pub dimensions: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Number,
    Bool,
    Text,
    Enum,
    Struct,
    Session,
    Error,
    Function,
    /// A type that an error left unknown, or a type variable.
    Unknown,
    /// An order wire (`after:`), which carries no value.
    Order,
}

impl Family {
    pub const ERROR: Family = Family { base: Base::Error, dimensions: 0 };
    pub const ORDER: Family = Family { base: Base::Order, dimensions: 0 };

    /// The families whose input and output share a line when a node passes them through (notation 3.1).
    pub fn passes_through(self) -> bool {
        self.dimensions == 0 && matches!(self.base, Base::Error | Base::Session)
    }
}

/// A terminal of a node, as the drawing needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub name: String,
    /// Two terminals of one opaque type or of `error` make a pass-through pair (notation 3.1).
    pub ty: TyId,
    pub family: Family,
    /// `V` for `f64[V]`.
    pub unit: Option<String>,
    /// The text of an inline constant on an input (spec 5.1), as the text writes it, escapes read.
    pub constant: Option<String>,
    /// An output that the line binds to `_` has no wire (notation 3.1).
    pub ignored: bool,
    /// What an input reads: the paths of its operand that read a value, each the end of a wire, with the name that the
    /// wire bears (`step.low`).
    pub reads: Vec<(Id, String)>,
    /// A wire with a pill reaches it (a field, a conversion): its line keeps its distance as a line with a constant does.
    pub pilled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind<'a> {
    /// A call: `qualified` as the text writes it (`visa.open`), the library of the callee for its band, and its glyph
    /// when it is a primitive drawn as a circle (notation 3.2).
    Call { qualified: String, short: String, library: Option<String>, glyph: Option<Glyph>, icon: Option<&'a Icon> },
    /// A struct construction: the name of its type without its library, which goes to the band.
    Construction { ty: String, library: Option<String> },
    /// A variant constructor with values.
    Variant { ty: String, variant: String, library: Option<String> },
    /// `ui.set` or `ui.get`: the operation and the element on two lines.
    Ui { operation: String, element: String },
    /// A named constant, or a construction whose fields are all constants, folded (notation 3.1).
    Constant { name: String, value: String },
    /// A `loop`, a `for`, a `match` or a `handle`: a frame around its bodies (notation 5).
    Structure(Structure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureKind {
    Loop,
    For,
    Match,
    Handle,
}

/// A structure: its header, and which terminal of its bodies each of its own terminals is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Structure {
    pub kind: StructureKind,
    /// The keyword of the text: `loop`, `for x in readings`, `match state`, `handle e6` (notation 5).
    pub header: String,
    /// `continue_on_error`, a badge in the header (notation 5.1).
    pub badge: Option<&'static str>,
    /// `for parallel`: a double frame (notation 5.1).
    pub parallel: bool,
    /// For each input of the node, its terminals on the left of its bodies (a register, an `in`, a notch), as the
    /// index of the body among the node's bodies and the index of the terminal: only the bodies that have one, so that
    /// a `match` of many arms costs its notches, not its inputs times its arms. Its outputs are the first terminals on
    /// the right of each body, in order.
    pub inputs_at: Vec<Vec<(usize, usize)>>,
    /// The error pair of the structure, by its input and its output (notation 2.1).
    pub pair: Option<(usize, usize)>,
}

/// What a terminal of the frame of a body is (notation 2, 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// An input or an output of the function.
    Input,
    Output,
    /// A `carry`: its box on the left (the current value) and on the right (`next`).
    Register,
    /// `x in values`: `]▸`.
    In,
    /// An outer value read inside, where its wire crosses the edge.
    Notch,
    /// A value that the pattern of an arm binds.
    Bound,
    Collect,
    Until,
    Yield,
}

/// A terminal of the frame of a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameTerminal {
    pub terminal: Terminal,
    pub edge: Edge,
}

/// A place on the edges of a frame, from the top: a register has a box on each side at one height (notation 5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub left: Option<usize>,
    pub right: Option<usize>,
}

/// Where a wire starts or ends, in the body that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum End {
    /// A terminal of a node: an output where a wire starts, an input where it ends.
    Node {
        node: usize,
        terminal: usize,
    },
    /// The order terminal of a node (`after:`), or the node itself where an order wire starts.
    Order(usize),
    /// A terminal of the frame of the body, on its left or its right edge.
    Left(usize),
    Right(usize),
}

/// A wire within one body: a value from outside a structure has one wire per frame it crosses (spec 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    pub from: End,
    pub to: End,
    pub family: Family,
    /// The value it carries, as the diagram writes it on the wire: `dmm`, `step.low`.
    pub name: String,
    /// Where a line reads it, for a wire into a node.
    pub read: Option<Id>,
}

/// The glyph of a primitive drawn as a circle (notation 3.2, closed list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Glyph {
    Sign(&'static str),
    /// `with_unit`, and `unitless` struck through.
    Unit {
        unit: String,
        struck: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node<'a> {
    pub kind: Kind<'a>,
    /// As the terminals of `docs/cli.md`, 4.3 name it: the name of its line, `#label`, or `@12`.
    pub key: String,
    /// What follows it across a change of the file (notation 2.1): the body that holds it, then the name that
    /// designates it in the `layout` section, or its rank among the nodes with no name of its body.
    pub stable: String,
    /// The index of the body that holds it, in `Graph::bodies`.
    pub body: usize,
    /// Its line.
    pub statement: Id,
    /// The bodies of a structure, in `Graph::bodies`: one, or one per arm.
    pub bodies: Vec<usize>,
    /// Where the `layout` section puts it, in hundredths, from the top-left corner of the frame that holds it.
    pub at: Option<(i64, i64)>,
    /// `#close`, drawn above the node.
    pub label: Option<String>,
    pub always: bool,
    pub after: bool,
    /// The `layout` section gives it `view standard`.
    pub standard: bool,
    pub inputs: Vec<Terminal>,
    pub outputs: Vec<Terminal>,
    /// Unwired optional inputs, hidden (notation 3.1, 3.2).
    pub hidden: u32,
}

/// A body: the function's, or one that a structure holds, with its frame and its wires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyOf {
    /// The title of its frame: the name of the function, or the patterns of an arm; none for a loop body, whose frame
    /// is the structure's.
    pub caption: String,
    /// The number of its arm, from 0.
    pub arm: Option<usize>,
    /// Indices in `Graph::nodes`, in canonical order.
    pub nodes: Vec<usize>,
    /// The structure that holds it.
    pub owner: Option<usize>,
    /// What its nodes are followed by (notation 2.1): empty for the function.
    pub key: String,
    pub left: Vec<FrameTerminal>,
    pub right: Vec<FrameTerminal>,
    /// The places of its terminals, from the top, in lists that are placed together: one list for both edges of a loop,
    /// whose registers have a box on each, one per edge otherwise.
    pub slots: Vec<Vec<Slot>>,
    /// Where the error chain enters and leaves the body: a terminal on the left and one on the right (notation 2.1).
    pub pair: (Option<usize>, Option<usize>),
    pub wires: Vec<Wire>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph<'a> {
    /// From the root, as `run` names it: `check_voltage`, `dmm_driver.measure`.
    pub function: String,
    pub bodies: Vec<BodyOf>,
    pub nodes: Vec<Node<'a>>,
    /// The pill that a wire carries just before its reader, by body and wire: the field it reads (`.low`, notation 3.3),
    /// the conversion it goes through (`mV→V`, notation 4.2), or both.
    pub pills: HashMap<(usize, usize), Pill>,
    /// The waypoints that a `wire` entry of the `layout` section gives (spec 11), by body and wire, in hundredths, from the
    /// top-left corner of the frame of the body.
    pub vias: HashMap<(usize, usize), Vec<(i64, i64)>>,
}

/// A pill on a wire: its text, and whether it shows a conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    pub text: String,
    pub conversion: bool,
}

/// What `LC7002` counts (`docs/cli.md`, 7.6): nodes at every level, structures included, and their terminals. The
/// nodes are counted from the text before anything is built; the terminals, which depend on the signatures of what is
/// called (a missing input is still a terminal), as the graph is built, which stops at the first one too many.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub nodes: u64,
    pub terminals: u64,
}

/// What a call needs of its callee, the same for every call to it: worked out once per callee, so that drawing a call
/// costs what its arguments and its terminals cost, never the size of a signature again.
struct Callee<'a> {
    params: Vec<Param>,
    /// The positions of the parameters that are wires without a default: each is a terminal, wired or not.
    required: Vec<usize>,
    /// How many parameters are wires with a default: those that no argument wires are hidden.
    defaults: u32,
    outputs: Vec<String>,
    band: Option<String>,
    always: bool,
    glyph: Option<&'static str>,
    icon: Option<&'a Icon>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum CalleeKey {
    Function(u32, Name),
    Primitive(PrimitiveLibrary, Name),
    Variant(TyId, Name),
}

/// The graph could not be built within the budget of terminals: what was counted when it stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge(pub Counts);

/// Every body of the function, with the order of its lines: the written order when it has no canonical one.
/// A body: the statement of the structure that holds it, the number of its arm, the body, the order of its lines.
type Walked<'a> = (Option<&'a Statement>, Option<usize>, &'a Body, Vec<usize>);

fn bodies<'a>(function: &'a Function, orders: &BodyOrders) -> Vec<Walked<'a>> {
    let order_of = |body: &Body, order: Option<Result<&[usize], labcraft_format::Unorderable>>| match order {
        Some(Ok(order)) => order.to_vec(),
        _ => (0..body.statements.len()).collect(),
    };
    let mut found = Vec::new();
    let mut stack = vec![(None, None, &function.body, order_of(&function.body, orders.of_function()))];
    while let Some((owner, arm, body, order)) = stack.pop() {
        let mut inner = Vec::new();
        for &index in &order {
            let Some(statement) = body.statements.get(index) else { continue };
            if let StatementKind::Binding { value, .. } = &statement.kind {
                match value {
                    Rhs::Loop(structure) => inner.push((Some(statement), None, &structure.body, order_of(&structure.body, orders.of(statement.id)))),
                    Rhs::For(structure) => inner.push((Some(statement), None, &structure.body, order_of(&structure.body, orders.of(statement.id)))),
                    Rhs::Match(branching) | Rhs::Handle(branching) => {
                        for (number, arm) in branching.arms.iter().enumerate() {
                            inner.push((Some(statement), Some(number), &arm.body, order_of(&arm.body, orders.of(arm.id))));
                        }
                    }
                    _ => {}
                }
            }
        }
        found.push((owner, arm, body, order));
        // Depth first, in the order of the lines: the last pushed is walked first.
        stack.extend(inner.into_iter().rev());
    }
    found
}

/// Counts the nodes that `LC7002` counts, without building anything: every line that the graph draws as a node.
pub fn count_nodes(function: &Function, orders: &BodyOrders, resolved: &Resolved) -> u64 {
    let mut nodes: u64 = 0;
    for (_, _, body, _) in bodies(function, orders) {
        for statement in &body.statements {
            if draws_a_node(statement, resolved) {
                nodes = nodes.saturating_add(1);
            }
        }
    }
    nodes
}

/// Whether a line is a node of the drawing: not a new name for a wire (`err = e3`), not `collect` or `until`.
fn draws_a_node(statement: &Statement, resolved: &Resolved) -> bool {
    match &statement.kind {
        StatementKind::Binding { value: Rhs::Operand(operand), .. } => is_constant(operand, resolved),
        StatementKind::Binding { .. } | StatementKind::Call(_) => true,
        StatementKind::Collect(_) | StatementKind::Until(_) => false,
    }
}

/// An operand that gives a constant rather than reading a wire: a literal, a variant without values, a function.
fn is_constant(operand: &Operand, resolved: &Resolved) -> bool {
    !matches!(operand, Operand::Path(_)) || matches!(resolved.of(operand.id()), Meaning::Variant { .. } | Meaning::Function { .. })
}

pub struct Builder<'a> {
    checked: &'a Checked,
    file: &'a CheckedFile,
    names: &'a Interner,
    /// The library of the file, by its name: a call into it has no band.
    own: &'a str,
    own_id: Option<LibraryId>,
    /// Whether a construction whose fields are all constants is folded (notation 3.1): never in an exported image.
    fold: bool,
    callees: HashMap<CalleeKey, Rc<Callee<'a>>>,
}

impl<'a> Builder<'a> {
    pub fn new(checked: &'a Checked, file: &'a CheckedFile, names: &'a Interner) -> Self {
        let declared = checked.libraries.get(file.library as usize);
        let own = declared.map_or("", |library| library.name.as_str());
        let own_id = declared.map(|library| library.id);
        Builder { checked, file, names, own, own_id, fold: false, callees: HashMap::new() }
    }

    /// Folds the constructions whose fields are all constants, as the viewer does by default (notation 3.1).
    pub fn folding(mut self, fold: bool) -> Self {
        self.fold = fold;
        self
    }

    fn resolved(&self) -> &'a Resolved {
        &self.file.resolved
    }

    fn types(&self) -> &'a TypeTable {
        &self.checked.types
    }

    fn text(&self, name: Name) -> String {
        cut_text(self.names.resolve(name)).into_owned()
    }

    /// The graph of a function with no body: an `extern` declaration, a frame with its terminals.
    pub fn empty(&self, name: Name, signature: &Signature) -> Graph<'a> {
        let function = self.qualified(name);
        let mut body = self.function_frame(function.clone(), signature);
        flow::finish_frame(&mut body, None);
        Graph { bodies: vec![body], function, nodes: Vec::new(), pills: HashMap::new(), vias: HashMap::new() }
    }

    /// From the root, as `run` names it.
    fn qualified(&self, name: Name) -> String {
        let qualified = match self.own {
            "" => self.text(name),
            own => format!("{own}.{}", self.names.resolve(name)),
        };
        cut_text(&qualified).into_owned()
    }

    /// The graph, unless its terminals pass `max_terminals`: then what was counted when building stopped.
    pub fn graph(&mut self, function: &'a Function, orders: &BodyOrders, max_terminals: u64) -> Result<Graph<'a>, TooLarge> {
        let mut graph =
            Graph { function: self.qualified(function.name), bodies: Vec::new(), nodes: Vec::new(), pills: HashMap::new(), vias: HashMap::new() };
        let entries = self.layout_entries(&self.file.file);
        let mut counts = Counts::default();
        let mut flow = flow::Flow::default();
        let walked = bodies(function, orders);
        for (owner, arm, body, order) in &walked {
            let index = graph.bodies.len();
            let mut frame = match owner {
                None => self.function_frame(graph.function.clone(), &function.signature),
                Some(statement) => {
                    let Some(&node) = flow.statement_node.get(&statement.id) else { continue };
                    // An arm is part of what follows a node: two arms can each have a line of one name.
                    let key = graph.nodes.get(node).map(|node| node.stable.clone()).unwrap_or_default();
                    let key = match arm {
                        Some(number) => format!("{key}:{}", number + 1),
                        None => key,
                    };
                    let mut frame = self.structure_frame(statement, *arm, body, key);
                    frame.owner = Some(node);
                    if let Some(node) = graph.nodes.get_mut(node) {
                        node.bodies.push(index);
                    }
                    frame
                }
            };
            let mut unnamed = 0usize;
            for &line in order {
                let Some(statement) = body.statements.get(line) else { continue };
                let Some(mut node) = self.node(statement) else { continue };
                node.body = index;
                let designation = match (statement.label, first_name(statement)) {
                    (Some(label), _) => format!("#{}", self.text(label.name)),
                    (None, Some(name)) => self.text(name),
                    (None, None) => {
                        unnamed += 1;
                        format!("~{unnamed}")
                    }
                };
                node.stable = if frame.key.is_empty() { designation } else { format!("{}/{designation}", frame.key) };
                counts.nodes = counts.nodes.saturating_add(1);
                counts.terminals = counts.terminals.saturating_add((node.inputs.len() + node.outputs.len()) as u64);
                if counts.terminals > max_terminals {
                    return Err(TooLarge(counts));
                }
                let at = graph.nodes.len();
                flow.statement_node.insert(statement.id, at);
                if let Some(label) = statement.label {
                    flow.label_node.insert(label.id, at);
                }
                if let Some(designation) = designation_of(statement) {
                    flow.designations.entry(designation).or_default().push(at);
                }
                frame.nodes.push(at);
                graph.nodes.push(node);
            }
            graph.bodies.push(frame);
        }
        // A `node` entry designates a node only when it names one (spec 11): one that names several gives `LC5006`.
        for ((is_label, name), (at, standard)) in entries {
            let Some(&[node]) = flow.designations.get(&(is_label, name)).map(Vec::as_slice) else { continue };
            if let Some(node) = graph.nodes.get_mut(node) {
                node.at = at;
                node.standard = standard;
            }
        }
        self.wires(&mut graph, &walked, &mut flow, &mut counts, max_terminals)?;
        for body in &mut graph.bodies {
            let owner = body.owner.and_then(|owner| graph.nodes.get(owner)).and_then(|node| match &node.kind {
                Kind::Structure(structure) => Some(structure.kind),
                _ => None,
            });
            flow::finish_frame(body, owner);
        }
        flow::structure_pairs(&mut graph, &flow);
        graph.pills = self.pills(&graph);
        graph.vias = self.vias(&graph, &flow);
        for &(body, wire) in graph.pills.keys() {
            if let Some(End::Node { node, terminal }) = graph.bodies.get(body).and_then(|of| of.wires.get(wire)).map(|wire| wire.to)
                && let Some(input) = graph.nodes.get_mut(node).and_then(|node| node.inputs.get_mut(terminal))
            {
                input.pilled = true;
            }
        }
        Ok(graph)
    }

    /// The waypoints of the `wire` entries that name one wire (spec 11): by the value it carries and the node it goes to,
    /// or an output of the function, and the input when one is named. An entry that names none or several gives a
    /// warning of `check` (`LC5004`, `LC5006`) and nothing here; neither does the part inside a structure that names it.
    fn vias(&self, graph: &Graph<'_>, flow: &flow::Flow) -> HashMap<(usize, usize), Vec<(i64, i64)>> {
        let mut vias = HashMap::new();
        let Some(layout) = &self.file.file.layout else { return vias };
        let coordinate = |numeral: &Numeral| -> Option<i64> {
            let NumeralKind::Int(text) = numeral.kind else { return None };
            let value = i64::from(labcraft_format::coordinate_value(self.names.resolve(text))?);
            Some(if numeral.negative { -value } else { value } * 100)
        };
        for item in &layout.items {
            let LayoutKind::Wire { from, to, via } = &item.kind else { continue };
            if via.is_empty() {
                continue;
            }
            let Some(points) = via.iter().map(|point| Some((coordinate(&point.x)?, coordinate(&point.y)?))).collect::<Option<Vec<_>>>() else {
                continue;
            };
            let value = match from.node.is_label {
                true => format!("#{}", self.names.resolve(from.node.name)),
                false => self.names.resolve(from.node.name).to_owned(),
            };
            let input = to.terminal.map(|name| self.names.resolve(name).to_owned());
            let node = match flow.designations.get(&(to.node.is_label, to.node.name)).map(Vec::as_slice) {
                Some(&[node]) => Some(node),
                _ => None,
            };
            let output = (!to.node.is_label).then(|| self.names.resolve(to.node.name).to_owned());
            let mut found = Vec::new();
            for (body, frame) in graph.bodies.iter().enumerate() {
                for (index, wire) in frame.wires.iter().enumerate() {
                    if wire.name.split('.').next() != Some(value.as_str()) {
                        continue;
                    }
                    let fits = match wire.to {
                        End::Node { node: reader, terminal } if Some(reader) == node => input.as_ref().is_none_or(|input| {
                            graph.nodes.get(reader).and_then(|reader| reader.inputs.get(terminal)).is_some_and(|terminal| &terminal.name == input)
                        }),
                        End::Order(reader) if Some(reader) == node => input.as_ref().is_none_or(|input| input == "after"),
                        End::Right(at) if body == 0 && node.is_none() && input.is_none() => {
                            frame.right.get(at).is_some_and(|terminal| Some(&terminal.terminal.name) == output.as_ref())
                        }
                        _ => false,
                    };
                    if fits {
                        found.push((body, index));
                    }
                }
            }
            if let [one] = found.as_slice() {
                vias.insert(*one, points);
            }
        }
        vias
    }

    /// The pills on the wires into nodes: the field that a path reads after its value, the conversion that `check` puts
    /// on what it reads.
    fn pills(&self, graph: &Graph<'_>) -> HashMap<(usize, usize), Pill> {
        let conversions: HashMap<Id, (TyId, TyId)> =
            self.resolved().coercions().iter().map(|coercion| (coercion.at, (coercion.from, coercion.to))).collect();
        let mut pills = HashMap::new();
        for (body, frame) in graph.bodies.iter().enumerate() {
            for (index, wire) in frame.wires.iter().enumerate() {
                let Some(read) = wire.read else { continue };
                let field = wire.name.find('.').and_then(|at| wire.name.get(at..)).map(str::to_owned);
                let conversion = conversions.get(&read).map(|&(from, to)| self.conversion(from, to));
                let text = match (&field, &conversion) {
                    (Some(field), Some(conversion)) => format!("{field} {conversion}"),
                    (Some(text), None) | (None, Some(text)) => text.clone(),
                    (None, None) => continue,
                };
                pills.insert((body, index), Pill { text, conversion: conversion.is_some() });
            }
        }
        pills
    }

    /// A conversion as its pill writes it: the units when only they change (`mV→V`), the types otherwise (`i16→i32`).
    fn conversion(&self, from: TyId, to: TyId) -> String {
        let (a, b) = (unit(self.types(), from), unit(self.types(), to));
        let shape = |ty: TyId| match self.types().get(ty) {
            labcraft_types::Ty::Number { kind, .. } => Some(*kind),
            _ => None,
        };
        if let (Some(a), Some(b)) = (&a, &b)
            && shape(from) == shape(to)
        {
            return format!("{a}\u{2192}{b}");
        }
        let written = |ty: TyId| self.own_id.map_or_else(String::new, |own| self.types().display(ty, own, self.names));
        format!("{}\u{2192}{}", written(from), written(to))
    }

    /// The entries of the `layout` section that place a node or set its view, by how they designate it (spec 11): a
    /// position in hundredths, from the top-left corner of the frame that holds the node.
    fn layout_entries(&self, file: &File) -> Vec<(Designation, Entry)> {
        let Some(layout) = &file.layout else { return Vec::new() };
        let coordinate = |numeral: &Numeral| -> Option<i64> {
            let NumeralKind::Int(text) = numeral.kind else { return None };
            let value = i64::from(labcraft_format::coordinate_value(self.names.resolve(text))?);
            Some(if numeral.negative { -value } else { value } * 100)
        };
        let mut seen = HashSet::new();
        layout
            .items
            .iter()
            .filter_map(|item| match &item.kind {
                // A second entry for one node is `LC5007`: the first one counts.
                LayoutKind::Node { node, at, view } if seen.insert((node.is_label, node.name)) => {
                    let at = at.as_ref().and_then(|point| Some((coordinate(&point.x)?, coordinate(&point.y)?)));
                    Some(((node.is_label, node.name), (at, *view == Some(View::Standard))))
                }
                _ => None,
            })
            .collect()
    }

    /// `docs/cli.md`, 4.3: the name of its line, its label, or `@12`.
    fn key(&self, statement: &Statement) -> String {
        match (first_name(statement), statement.label) {
            (Some(name), _) => self.text(name),
            (None, Some(label)) => format!("#{}", self.text(label.name)),
            (None, None) => format!("@{}", self.file.source_map.span(statement.id).map_or(0, |span| span.start.line)),
        }
    }

    fn node(&mut self, statement: &'a Statement) -> Option<Node<'a>> {
        let (kind, inputs, outputs, hidden, always, after) = match &statement.kind {
            StatementKind::Binding { targets, value } => match value {
                Rhs::Call(call) => {
                    let call = self.call(call, statement, targets);
                    (call.0, call.1, call.2, call.3, call.4, call.5)
                }
                Rhs::Struct(lit) => {
                    let (kind, inputs) = self.construction(lit, targets);
                    let outputs = vec![self.output_of(targets, 0, "value")];
                    (kind, inputs, outputs, 0, false, false)
                }
                // An array of wires is a node with an input per element, as `ir` lowers it (decision of the owner, M0v
                // step 4); an array of constants stays a constant.
                Rhs::Operand(operand @ Operand::Array(_, values)) if !self.reads(operand).is_empty() => {
                    let inputs = values.iter().enumerate().map(|(index, value)| self.input(index.to_string(), value, None)).collect();
                    let kind = Kind::Construction { ty: "[ ]".to_owned(), library: None };
                    (kind, inputs, vec![self.output_of(targets, 0, "value")], 0, false, false)
                }
                Rhs::Operand(operand) if is_constant(operand, self.resolved()) => {
                    let name = targets.first().map_or_else(String::new, |target| self.target_text(target));
                    let kind = Kind::Constant { name, value: self.constant_text(operand) };
                    (kind, Vec::new(), vec![self.output_of(targets, 0, "value")], 0, false, false)
                }
                Rhs::Operand(_) => return None,
                Rhs::Loop(_) | Rhs::For(_) | Rhs::Match(_) | Rhs::Handle(_) => {
                    let (kind, inputs, outputs) = self.structure(value, targets);
                    (kind, inputs, outputs, 0, false, false)
                }
            },
            StatementKind::Call(call) => {
                let call = self.call(call, statement, &[]);
                (call.0, call.1, call.2, call.3, call.4, call.5)
            }
            StatementKind::Collect(_) | StatementKind::Until(_) => return None,
        };
        Some(Node {
            kind,
            key: self.key(statement),
            stable: String::new(),
            body: 0,
            statement: statement.id,
            bodies: Vec::new(),
            at: None,
            label: statement.label.map(|label| format!("#{}", self.text(label.name))),
            always,
            after,
            standard: false,
            inputs,
            outputs,
            hidden,
        })
    }

    fn target_text(&self, target: &Target) -> String {
        match target {
            Target::Name(_, name) => self.text(*name),
            Target::Next(_, name) => format!("next {}", self.text(*name)),
            Target::Skip(_) => "_".to_owned(),
        }
    }

    /// The output of a line that makes one value.
    fn output_of(&self, targets: &[Target], index: usize, name: &str) -> Terminal {
        let (ty, ignored) = match targets.get(index) {
            Some(Target::Name(id, _) | Target::Next(id, _)) => (self.resolved().ty(*id), false),
            Some(Target::Skip(id)) => (self.resolved().ty(*id), true),
            None => (TyId::ERROR, false),
        };
        self.terminal(name.to_owned(), ty, None, ignored)
    }

    fn terminal(&self, name: String, ty: TyId, constant: Option<String>, ignored: bool) -> Terminal {
        Terminal {
            name,
            ty,
            family: family(self.types(), &self.checked.libraries, ty),
            unit: unit(self.types(), ty),
            constant,
            ignored,
            reads: Vec::new(),
            pilled: false,
        }
    }

    /// A call: its kind, its inputs and outputs in signature order, the unwired optional inputs, `always`, `after:`.
    #[allow(clippy::type_complexity)]
    fn call(&mut self, call: &'a Call, statement: &Statement, targets: &[Target]) -> (Kind<'a>, Vec<Terminal>, Vec<Terminal>, u32, bool, bool) {
        let resolved = self.resolved().call(call.id);
        let callee = resolved.map_or(Meaning::None, |resolved| resolved.callee);
        let wiring = |index: usize| resolved.and_then(|resolved| resolved.terminals.get(index)).copied();
        let after = call.args.iter().any(|arg| matches!(arg, Arg::After { .. }));
        let qualified = cut_text(&path_text(&call.callee, self.names)).into_owned();
        let short = cut_text(self.names.resolve(call.callee.rest.last().copied().unwrap_or(call.callee.first))).into_owned();
        let signature = self.callee(callee);
        if let Meaning::Variant { ty, variant } = callee {
            let (type_name, band) = self.type_name(ty);
            let kind = Kind::Variant { ty: type_name, variant: self.text(variant), library: band };
            let inputs = self.inputs(call, &signature, &wiring);
            return (kind, inputs.0, vec![self.output_of(targets, 0, "value")], inputs.1, false, after);
        }
        let (inputs, hidden) = self.inputs(call, &signature, &wiring);
        // Outputs: one per name of the signature, typed by what the line binds, or by what a call written alone gives.
        let alone = self.resolved().outputs_of(statement.id);
        // A call written alone has the outputs that `check` gives it: those of its callee, `core.call` those of the function it calls.
        let count = signature.outputs.len().max(targets.len()).max(if targets.is_empty() { alone.len() } else { 0 });
        let outputs = (0..count)
            .map(|index| {
                let name = signature.outputs.get(index).cloned().unwrap_or_else(|| index.to_string());
                if targets.is_empty() {
                    let ty = alone.get(index).copied().unwrap_or(TyId::ERROR);
                    self.terminal(name, ty, None, true)
                } else {
                    self.output_of(targets, index, &name)
                }
            })
            .collect();
        let kind = match callee {
            Meaning::Primitive { library: PrimitiveLibrary::Ui, .. } => {
                let element = call
                    .args
                    .iter()
                    .find_map(|arg| match arg {
                        Arg::Value { value: Operand::Path(path), .. } if matches!(self.resolved().of(path.id), Meaning::Element(_)) => {
                            Some(cut_text(&path_text(path, self.names)).into_owned())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                Kind::Ui { operation: short.clone(), element }
            }
            _ => {
                let glyph = match (signature.glyph, short.as_str()) {
                    (Some("unit"), name) => self.unit_arg(call).map(|unit| Glyph::Unit { unit, struck: name == "unitless" }),
                    (Some(sign), _) => Some(Glyph::Sign(sign)),
                    (None, _) => None,
                };
                Kind::Call { qualified, short, library: signature.band.clone(), glyph, icon: signature.icon }
            }
        };
        (kind, inputs, outputs, hidden, signature.always, after)
    }

    /// What a call needs of its callee, worked out on the first call to it.
    fn callee(&mut self, callee: Meaning) -> Rc<Callee<'a>> {
        let key = match callee {
            Meaning::Function { library, name } => Some(CalleeKey::Function(library, name)),
            Meaning::Primitive { library, name } => Some(CalleeKey::Primitive(library, name)),
            Meaning::Variant { ty, variant } => Some(CalleeKey::Variant(ty, variant)),
            _ => None,
        };
        if let Some(found) = key.and_then(|key| self.callees.get(&key)) {
            return Rc::clone(found);
        }
        let made = Rc::new(self.make_callee(callee));
        if let Some(key) = key {
            self.callees.insert(key, Rc::clone(&made));
        }
        made
    }

    fn make_callee(&self, callee: Meaning) -> Callee<'a> {
        let mut found =
            Callee { params: Vec::new(), required: Vec::new(), defaults: 0, outputs: Vec::new(), band: None, always: false, glyph: None, icon: None };
        // The parameters: name, whether it has a default, whether it is a wire at all (not a unit, a category, an element).
        match callee {
            Meaning::Function { library, name } => {
                let declared = self.checked.libraries.get(library as usize);
                let function = declared.and_then(|declared| declared.functions.get(&name));
                if let Some(function) = function {
                    found.params = function
                        .params
                        .iter()
                        .map(|param| Param { name: self.text(param.name), has_default: param.has_default, is_wire: true, ty: Some(param.ty) })
                        .collect();
                    found.outputs = function.outputs.iter().map(|output| self.text(output.name)).collect();
                }
                found.band = declared
                    .map(|declared| declared.name.as_str())
                    .filter(|name| !name.is_empty() && *name != self.own)
                    .map(|name| cut_text(name).into_owned());
                found.always = declared.is_some_and(|declared| declared.always.contains(&name));
                found.icon = function
                    .and_then(|function| function.file)
                    .and_then(|file| self.checked.files.get(file as usize))
                    .and_then(|file| file.file.icon.as_ref());
            }
            Meaning::Primitive { library, name } => {
                if let Some(primitive) = primitive(library, self.names.resolve(name)) {
                    found.params = primitive
                        .params
                        .iter()
                        .map(|param| Param {
                            name: param.name.to_owned(),
                            has_default: param.default.is_some(),
                            is_wire: param.kind == ParamKind::Value,
                            ty: None,
                        })
                        .collect();
                    found.outputs = primitive.outputs.map_or_else(Vec::new, |outputs| outputs.iter().map(|&output| output.to_owned()).collect());
                }
                found.glyph = (library == PrimitiveLibrary::Core).then(|| sign_of(self.names.resolve(name))).flatten();
            }
            Meaning::Variant { ty, variant } => {
                found.params = self
                    .variant_values(ty, variant)
                    .iter()
                    .enumerate()
                    .map(|(index, &ty)| Param { name: index.to_string(), has_default: false, is_wire: true, ty: Some(ty) })
                    .collect();
            }
            _ => {}
        }
        for (position, param) in found.params.iter().enumerate() {
            match (param.is_wire, param.has_default) {
                (true, false) => found.required.push(position),
                (true, true) => found.defaults = found.defaults.saturating_add(1),
                (false, _) => {}
            }
        }
        found
    }

    /// The inputs of a call in the order of its parameters: an argument wired, or given as a constant; a variadic parameter
    /// gives one per argument; an argument of no parameter (the fields of `string.format`, what `core.call` passes) comes
    /// last. A parameter with a default and no argument is hidden; a unit, a category or an element is no terminal. In
    /// time, the arguments and the terminals: the parameters with a default are counted, not walked.
    fn inputs(&self, call: &Call, callee: &Callee<'_>, wiring: &dyn Fn(usize) -> Option<Wiring>) -> (Vec<Terminal>, u32) {
        // The arguments that a parameter takes, by its position then their own; the free ones, in their order.
        let mut wired: Vec<(usize, usize, &Operand)> = Vec::new();
        let mut free: Vec<(usize, Option<Name>, &Operand)> = Vec::new();
        for (index, arg) in call.args.iter().enumerate() {
            let Arg::Value { name, value, .. } = arg else { continue };
            match wiring(index) {
                Some(Wiring::Param(position)) => wired.push((position as usize, index, value)),
                Some(Wiring::Free) | None => free.push((index, *name, value)),
                Some(_) => {}
            }
        }
        wired.sort_by_key(|&(position, index, _)| (position, index));
        let mut inputs = Vec::new();
        let mut wired_defaults: u32 = 0;
        let mut required = callee.required.iter().copied().peekable();
        let mut previous = None;
        for &(position, _, operand) in &wired {
            // The parameters without a default that come before, and that no argument wires: missing, which `check`
            // says, and still a terminal, so the drawing shows where.
            while let Some(missing) = required.next_if(|&missing| missing < position) {
                self.missing(callee, missing, &mut inputs);
            }
            let _ = required.next_if_eq(&position);
            let Some(param) = callee.params.get(position) else {
                // A value of no parameter (one too many for a variant, which `check` says): still drawn, by its
                // position, so that the drawing shows where, as an argument too many for a function.
                inputs.push(self.input(position.to_string(), operand, None));
                continue;
            };
            if param.is_wire {
                inputs.push(self.input(param.name.clone(), operand, param.ty));
                if param.has_default && previous != Some(position) {
                    wired_defaults = wired_defaults.saturating_add(1);
                }
            }
            previous = Some(position);
        }
        for missing in required {
            self.missing(callee, missing, &mut inputs);
        }
        for (index, name, operand) in free {
            let name = name.map_or_else(|| index.to_string(), |name| self.text(name));
            inputs.push(self.input(name, operand, None));
        }
        (inputs, callee.defaults.saturating_sub(wired_defaults))
    }

    fn missing(&self, callee: &Callee<'_>, position: usize, inputs: &mut Vec<Terminal>) {
        if let Some(param) = callee.params.get(position) {
            inputs.push(self.terminal(param.name.clone(), param.ty.unwrap_or(TyId::ERROR), None, false));
        }
    }

    /// An input: its type is that of the parameter, when the signature gives one, so that the terminal shows its unit
    /// (notation 4.2), and otherwise that of what the call settles.
    fn input(&self, name: String, operand: &Operand, declared: Option<TyId>) -> Terminal {
        let ty = declared.filter(|ty| !self.types().has_error(*ty)).unwrap_or_else(|| self.resolved().ty(operand.id()));
        let constant = match operand {
            Operand::Path(path) => match self.resolved().of(path.id) {
                Meaning::Value(_) | Meaning::None | Meaning::Element(_) => None,
                _ => Some(self.constant_text(operand)),
            },
            _ => Some(self.constant_text(operand)),
        };
        let mut terminal = self.terminal(name, ty, constant, false);
        terminal.reads = self.reads(operand);
        terminal
    }

    /// The paths of an operand that read a value: a wire each.
    fn reads(&self, operand: &Operand) -> Vec<(Id, String)> {
        let mut found = Vec::new();
        let mut stack = vec![operand];
        while let Some(operand) = stack.pop() {
            match operand {
                Operand::Path(path) if matches!(self.resolved().of(path.id), Meaning::Value(_)) => {
                    found.push((path.id, cut_text(&path_text(path, self.names)).into_owned()));
                }
                Operand::Bracket(id, _) if matches!(self.resolved().of(*id), Meaning::ArrayOfValue(_)) => {
                    let text = write_operand(operand, self.names, CONSTANT_BYTES);
                    found.push((*id, cut_text(text.trim_start_matches('[').trim_end_matches(']')).into_owned()));
                }
                Operand::Array(_, values) => stack.extend(values.iter().rev()),
                _ => {}
            }
        }
        found
    }

    /// An inline constant as the text writes it: a string in quotes with its escapes read, which the drawing shows by
    /// their signs (notation 2.3); the others in their canonical form.
    fn constant_text(&self, operand: &Operand) -> String {
        match operand {
            Operand::Text(_, text) => {
                let raw = text.char_indices().nth(CONSTANT_BYTES / 4).map_or(&**text, |(end, _)| text.get(..end).unwrap_or(text));
                let more = if raw.len() < text.len() { "\u{2026}" } else { "\"" };
                format!("\"{}{more}", read_escapes(raw))
            }
            _ => write_operand(operand, self.names, CONSTANT_BYTES),
        }
    }

    /// The unit that `with_unit` or `unitless` takes, as written.
    fn unit_arg(&self, call: &Call) -> Option<String> {
        call.args.iter().find_map(|arg| match arg {
            Arg::Value { value: operand @ Operand::Bracket(..), .. } => Some(write_operand(operand, self.names, CONSTANT_BYTES)),
            _ => None,
        })
    }

    /// A construction: folded when every field is a constant (notation 3.1), otherwise one input per field, in the order
    /// the type declares them.
    fn construction(&self, lit: &StructLit, targets: &[Target]) -> (Kind<'a>, Vec<Terminal>) {
        let ty = match self.resolved().of(lit.id) {
            Meaning::Type(ty) => ty,
            _ => self.resolved().ty(lit.id),
        };
        let (type_name, band) = match self.resolved().of(lit.id) {
            Meaning::Type(ty) => self.type_name(ty),
            _ => (cut_text(&path_text(&lit.path, self.names)).into_owned(), None),
        };
        if self.fold && lit.fields.iter().all(|field| is_constant(&field.value, self.resolved())) {
            let name = targets.first().map_or_else(String::new, |target| self.target_text(target));
            return (Kind::Constant { name, value: format!("{type_name} {{\u{2026}}}") }, Vec::new());
        }
        let declared: Vec<Name> = match self.types().get(definition(self.types(), &self.checked.libraries, ty)) {
            Ty::Struct(fields) => fields.iter().map(|(name, _)| *name).collect(),
            _ => Vec::new(),
        };
        let mut fields: Vec<&labcraft_graph::FieldInit> = lit.fields.iter().collect();
        if !declared.is_empty() {
            let place: HashMap<Name, usize> = declared.iter().enumerate().rev().map(|(position, name)| (*name, position)).collect();
            fields.sort_by_key(|field| place.get(&field.name).copied().unwrap_or(usize::MAX));
        }
        let inputs = fields.iter().map(|field| self.input(self.text(field.name), &field.value, None)).collect();
        (Kind::Construction { ty: type_name, library: band }, inputs)
    }

    /// The name of a declared type without its library, and that library for the band when it is another one.
    fn type_name(&self, ty: TyId) -> (String, Option<String>) {
        match self.types().get(ty) {
            Ty::Named { library, name } => {
                let band = library_named(&self.checked.libraries, *library)
                    .map(|declared| declared.name.as_str())
                    .filter(|name| !name.is_empty() && *name != self.own)
                    .map(|name| cut_text(name).into_owned());
                (self.text(*name), band)
            }
            _ => (self.own_id.map_or_else(String::new, |own| self.types().display(ty, own, self.names)), None),
        }
    }

    fn variant_values(&self, ty: TyId, variant: Name) -> Vec<TyId> {
        match self.types().get(definition(self.types(), &self.checked.libraries, ty)) {
            Ty::Enum(variants) => variants.iter().find(|candidate| candidate.name == variant).map_or_else(Vec::new, |found| found.values.clone()),
            _ => Vec::new(),
        }
    }
}

/// A parameter of what a call calls: whether it has a default, whether it is a wire at all (not a unit, a category or an
/// element), and its type when the signature gives one that does not depend on the call.
struct Param {
    name: String,
    has_default: bool,
    is_wire: bool,
    ty: Option<TyId>,
}

/// How the `layout` section designates a node: by its label (`true`), or by the name of its line.
type Designation = (bool, Name);
/// What a `node` entry gives: a position, and whether the node is in standard view.
type Entry = (Option<(i64, i64)>, bool);

/// How the `layout` section designates the node of a line (spec 11): by its label, or by the name of its line.
fn designation_of(statement: &Statement) -> Option<(bool, Name)> {
    match statement.label {
        Some(label) => Some((true, label.name)),
        None => first_name(statement).map(|name| (false, name)),
    }
}

/// The first name that a line binds: the name of its line (spec 5.2).
fn first_name(statement: &Statement) -> Option<Name> {
    match &statement.kind {
        StatementKind::Binding { targets, .. } => targets.iter().find_map(|target| match target {
            Target::Name(_, name) => Some(*name),
            _ => None,
        }),
        _ => None,
    }
}

fn path_text(path: &labcraft_graph::Path, names: &Interner) -> String {
    let mut text = names.resolve(path.first).to_owned();
    for segment in &path.rest {
        text.push('.');
        text.push_str(names.resolve(*segment));
    }
    text
}

/// The glyph of a primitive of `core` drawn as a circle (notation 3.2); `unit` for those that show a unit.
fn sign_of(name: &str) -> Option<&'static str> {
    Some(match name {
        "add" => "+",
        "sub" | "neg" => "\u{2212}",
        "mul" => "\u{D7}",
        "div" => "\u{F7}",
        "sqrt" => "\u{221A}",
        "abs" => "|x|",
        "ge" => "\u{2265}",
        "le" => "\u{2264}",
        "gt" => ">",
        "lt" => "<",
        "eq" => "=",
        "ne" => "\u{2260}",
        "and" => "\u{2227}",
        "or" => "\u{2228}",
        "not" => "\u{AC}",
        "select" => "?",
        "with_unit" | "unitless" => "unit",
        _ => return None,
    })
}

fn library_named(libraries: &[DeclaredLibrary], id: LibraryId) -> Option<&DeclaredLibrary> {
    libraries.iter().find(|library| library.id == id)
}

/// What a declared type stands for, followed a few times at most: an alias names a type, never itself.
fn definition(types: &TypeTable, libraries: &[DeclaredLibrary], ty: TyId) -> TyId {
    let mut current = ty;
    for _ in 0..8 {
        let Ty::Named { library, name } = types.get(current) else { return current };
        let Some(declared) = library_named(libraries, *library).and_then(|library| library.types.get(name)) else { return current };
        if declared.opaque || declared.definition == current {
            return current;
        }
        current = declared.definition;
    }
    current
}

/// The family of a type (notation 4.1).
pub fn family(types: &TypeTable, libraries: &[DeclaredLibrary], ty: TyId) -> Family {
    let mut dimensions: u8 = 0;
    let mut current = ty;
    for _ in 0..64 {
        let base = match types.get(current) {
            Ty::Error | Ty::Variable(_) => Base::Unknown,
            Ty::Bool => Base::Bool,
            Ty::Number { .. } | Ty::Duration | Ty::Timestamp => Base::Number,
            Ty::String | Ty::Bytes => Base::Text,
            Ty::ErrorValue => return Family { dimensions, ..Family::ERROR },
            Ty::Array { element, dimensions: more } => {
                dimensions = dimensions.saturating_add(*more);
                current = *element;
                continue;
            }
            Ty::Struct(_) | Ty::Waveform(_) => Base::Struct,
            Ty::Enum(_) => Base::Enum,
            Ty::Queue(_) | Ty::Ref(_) => Base::Session,
            Ty::Function { .. } => Base::Function,
            Ty::Named { library, name } => {
                let declared = library_named(libraries, *library).and_then(|library| library.types.get(name));
                match declared {
                    Some(declared) if declared.opaque => Base::Session,
                    Some(declared) if declared.definition != current => {
                        current = declared.definition;
                        continue;
                    }
                    _ => Base::Unknown,
                }
            }
        };
        return Family { base, dimensions };
    }
    Family { base: Base::Unknown, dimensions }
}

/// The unit of a number, for its badge (notation 4.2): none for a number without one.
pub fn unit(types: &TypeTable, ty: TyId) -> Option<String> {
    match types.get(ty) {
        Ty::Number { unit, .. } => Some(unit.to_string()).filter(|text| !text.is_empty()).map(|text| cut_text(&text).into_owned()),
        _ => None,
    }
}

/// The function of a checked file, if it declares one.
pub fn function_of(file: &CheckedFile) -> Option<&Function> {
    match &file.file.decl {
        Decl::Function(function) => Some(function),
        _ => None,
    }
}

/// The orders of every body of the function, as `fmt` gives them.
pub fn orders_of(file: &CheckedFile, function: &Function, names: &Interner) -> BodyOrders {
    match &file.orders {
        Some(orders) => orders.clone(),
        None => canonical_orders(&function.body, &function.signature, names),
    }
}
