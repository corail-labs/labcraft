//! The values of a run on the wires (notation 6.1; `docs/architecture.md`, 11.3): what the reader of a trace keeps for
//! the function drawn ([`expected`]), where each wire takes its value from, found once when the function is drawn
//! ([`TracePlan`]), and the overlay that shows them ([`values`]), drawn over the drawing and never moving it.
//!
//! A value of a node is read from its `ran` event; the others are worked out from those, as the notation says: a carry
//! from its first value and the `next` of the iteration before, the element of a `for x in` from its array, the index of a
//! range from its start, a value that an arm binds from the selector. Each is worked out without recursion, within a
//! budget of steps, so that a carry passed on unchanged over a million iterations costs a bound, never the stack.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use labcraft_trace::json::Scanner;
use labcraft_trace::{Expected, ExpectedNode, ExpectedWire, HeaderProblem, Limit, Trace};
use labcraft_types::{Code, Diagnostic, cut_text};

use crate::graph::{self, Edge, End, Graph, Kind, StructureKind};
use crate::place::{self, Placement};
use crate::shape::{Drawing, Paint, Shape, Token};
use crate::short::{DEPTH, Short, head_of, short};
use crate::size::{self, Text};
use crate::style;

/// The steps that working out one value may take: a carry that its loop passes on unchanged steps back once per
/// iteration *(proposed)*. Past it, the value is not shown.
const STEPS: usize = 10_000;
/// The opacity of the veil over an arm that the run did not take, and over a node it skipped (notation 6.1)
/// *(proposed)*.
const VEIL: u8 = 60;
/// Between two lines of the hatching of a skipped node, along a side, in hundredths: 6 units between the lines, across
/// them, at 45 degrees *(proposed)*.
const HATCH: i64 = 849;

/// What the trace needs of a drawing, found once when it is drawn. Not in the SVG.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TracePlan {
    /// The inputs of the function, in the order of its signature.
    pub inputs: Vec<PlanWire>,
    /// Where the value of each input shows when no wire leaves it, or when it has a default: the start of the pill of its
    /// default, on the edge of the frame, and the middle of its line.
    pub anchors: Vec<(i64, i64)>,
    /// The width of the pill of the default of each input, whose place its value takes (notation 6.1); 0 without one.
    pub defaulted: Vec<i64>,
    /// The nodes, in the order of the graph: those of [`Expected`].
    pub nodes: Vec<PlanNode>,
    /// The bodies, in the order of the graph.
    pub bodies: Vec<PlanBody>,
    /// Where values come from; a wire names one by its index.
    pub sources: Vec<Source>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlanWire {
    pub name: String,
    /// As the trace writes it: `f64[V]`.
    pub ty: String,
    /// An error or a session, which a skipped node passes on (spec 7.2): its value still shows.
    pub passes: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlanBody {
    /// The loops around it, outermost first, by node.
    pub loops: Vec<u32>,
    /// The arms that hold it, outermost first: the structure, by node, and the arm as the trace writes it.
    pub arms: Vec<(u32, Arc<str>)>,
    /// The depth of its frame, whose fill a veil takes.
    pub depth: u32,
    /// The rank of the structure that owns it, which orders the values its frame binds (a carry, an element, a bound value).
    pub order: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlanNode {
    /// As the trace names it (`docs/cli.md`, 4.3).
    pub name: String,
    pub body: u32,
    /// The values it binds, which are its wires.
    pub wires: Vec<PlanWire>,
    /// The wire of each output terminal, by its index in `wires`; none for an output bound to `_`.
    pub outputs: Vec<Option<u32>>,
    /// Its block, by its index in [`Drawing::placed`].
    pub placed: Option<u32>,
    /// Where the values of its error inputs come from, and its error outputs by wire: a node whose error output carries
    /// an error that none of its inputs carried produced it (notation 6).
    pub errors_in: Vec<u32>,
    pub errors_out: Vec<u32>,
    /// Where the value of its `err` input comes from, whose error skips it (spec 7.2): not every input of type `error`,
    /// since `errors.merge` takes errors and runs.
    pub err: Option<u32>,
    /// Its rank in the canonical order of the lines (spec 5.2), which orders the bubbles.
    pub order: u32,
    /// It runs whatever its error input holds (`always`, spec 7.2), as `visa.close`.
    pub always: bool,
    pub kind: PlanKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PlanKind {
    #[default]
    Node,
    /// A `loop` or a `for`: its key, and where its counter goes in its header (`× 6`): from the end of its title, at the
    /// middle of the header, up to its right end.
    Loop { key: String, counter: (i64, i64, i64) },
    /// A `match` or a `handle`: its key, and its arms, each its body and its text as the trace writes it.
    Choice { key: String, arms: Vec<(u32, Arc<str>)> },
}

/// Where a value comes from, at the iterations of the body that reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Source {
    #[default]
    None,
    /// What a node bound: the `out` of its `ran` event.
    Output { node: u32, wire: u32 },
    /// An input of the function: the `in` of the `call` of frame 0.
    Input(u32),
    /// The current value of a carry of a loop: its first value at the first iteration, then its `next` of the one
    /// before.
    Carry { looped: u32, first: u32, next: u32 },
    /// The element of a `for x in` at its iteration.
    Element { looped: u32, array: u32 },
    /// The index of a `for i in a..b`: `a` plus the iteration.
    Index { looped: u32, start: u32 },
    /// A value that an arm binds, from the selector: the values of a variant, by place, or the whole value.
    Bound { selector: u32, variant: Option<String>, place: u32 },
    /// A constant of the text, as JSON.
    Constant(String),
}

/// The source of each start of a wire, by body: what [`crate::WireOf::source`] names.
pub type Sources = HashMap<(usize, End), u32>;

/// The plan of a graph placed: its nodes for the reader, and where each wire takes its value from. `placed` gives the
/// index of each node in [`Drawing::placed`].
pub fn plan(graph: &Graph<'_>, placement: &Placement, placed: &[Option<usize>]) -> (TracePlan, Sources) {
    let mut plan = TracePlan::default();
    let order = canonical(graph);
    // The text of each arm once, which every body and node inside it shares: a pattern may be long.
    let traced: Vec<Arc<str>> = graph.bodies.iter().map(|body| Arc::from(body.traced.as_str())).collect();
    // The loops and the arms around each body, from the outside in: a body comes after the body of its structure.
    for body in &graph.bodies {
        let mut found = PlanBody::default();
        if let Some(owner) = body.owner.and_then(|owner| graph.nodes.get(owner).map(|node| (owner, node))) {
            let (index, node) = owner;
            if let Some(outer) = plan.bodies.get(node.body) {
                found = outer.clone();
            }
            found.order = order.get(index).copied().unwrap_or(u32::MAX);
            if let Kind::Structure(structure) = &node.kind {
                let index = u32::try_from(index).unwrap_or(u32::MAX);
                match structure.kind {
                    StructureKind::Loop | StructureKind::For => found.loops.push(index),
                    StructureKind::Match | StructureKind::Handle => {
                        found.arms.push((index, traced.get(plan.bodies.len()).cloned().unwrap_or_default()));
                    }
                }
            }
        }
        plan.bodies.push(found);
    }
    for (index, frame) in placement.frames.iter().enumerate() {
        if let Some(body) = plan.bodies.get_mut(index) {
            body.depth = frame.depth;
        }
    }
    // The inputs of the function, and where each shows its value: where the pill of its default starts.
    if let (Some(body), Some(frame)) = (graph.bodies.first(), placement.frames.first()) {
        for (terminal, &height) in body.left.iter().zip(&frame.left) {
            plan.inputs.push(PlanWire { name: terminal.terminal.name.clone(), ty: terminal.terminal.ty_text.clone(), passes: false });
            let label = place::TerminalLabel::of(terminal);
            let mut at = frame.x + style::label_start() + label.name.width;
            if let Some(unit) = &label.unit {
                at += style::padding() + unit.width;
            }
            plan.anchors.push((at + style::grid(), frame.y + height));
            plan.defaulted.push(label.pill.as_ref().map_or(0, size::pill_width));
        }
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        let mut wires = Vec::new();
        let outputs: Vec<Option<u32>> = node
            .outputs
            .iter()
            .enumerate()
            .map(|(at, terminal)| {
                let name = node.values.get(at).cloned().flatten()?;
                let ty = terminal.traced.clone().unwrap_or_else(|| terminal.ty_text.clone());
                wires.push(PlanWire { name, ty, passes: terminal.family.passes_through() });
                u32::try_from(wires.len() - 1).ok()
            })
            .collect();
        let errors_out =
            node.outputs.iter().zip(&outputs).filter(|(terminal, _)| terminal.family == graph::Family::ERROR).filter_map(|(_, wire)| *wire).collect();
        let kind = match &node.kind {
            Kind::Structure(structure) => match structure.kind {
                StructureKind::Loop | StructureKind::For => {
                    let counter = node.bodies.first().and_then(|body| placement.frames.get(*body)).map_or((0, 0, 0), |frame| {
                        let title = Text::new(&structure.header, size::any_text(), size::large()).width;
                        let badge = structure
                            .badge
                            .map_or(0, |badge| style::grid() + Text::new(badge, size::any_text(), size::medium()).width + 2 * style::padding());
                        let start = frame.x + style::label_start() + crate::mark_room(Some(crate::Family::of(structure)));
                        (start + title + badge, frame.y + frame.header / 2, frame.x + frame.width - style::label_start())
                    });
                    PlanKind::Loop { key: node.stable.clone(), counter }
                }
                StructureKind::Match | StructureKind::Handle => PlanKind::Choice {
                    key: node.stable.clone(),
                    arms: node.bodies.iter().filter_map(|&body| Some((u32::try_from(body).ok()?, traced.get(body)?.clone()))).collect(),
                },
            },
            _ => PlanKind::Node,
        };
        plan.nodes.push(PlanNode {
            name: node.key.clone(),
            body: u32::try_from(node.body).unwrap_or(u32::MAX),
            wires,
            outputs,
            placed: placed.get(index).copied().flatten().and_then(|at| u32::try_from(at).ok()),
            errors_in: Vec::new(),
            errors_out,
            err: None,
            order: order.get(index).copied().unwrap_or(u32::MAX),
            always: node.always,
            kind,
        });
    }
    let sources = sources(graph, &mut plan);
    (plan, sources)
}

/// The rank of each node in the canonical order of the lines: the lines of a body in order, the lines of a structure's
/// bodies right after it.
fn canonical(graph: &Graph<'_>) -> Vec<u32> {
    let mut order = vec![u32::MAX; graph.nodes.len()];
    let mut next = 0u32;
    let mut stack: Vec<std::slice::Iter<'_, usize>> = graph.bodies.first().map(|body| vec![body.nodes.iter()]).unwrap_or_default();
    while let Some(nodes) = stack.last_mut() {
        let Some(&node) = nodes.next() else {
            stack.pop();
            continue;
        };
        if let Some(slot) = order.get_mut(node) {
            *slot = next;
            next = next.saturating_add(1);
        }
        let inner: Vec<&graph::BodyOf> =
            graph.nodes.get(node).map(|found| found.bodies.iter().filter_map(|&body| graph.bodies.get(body)).collect()).unwrap_or_default();
        for body in inner.into_iter().rev() {
            stack.push(body.nodes.iter());
        }
    }
    order
}

/// Where each start of a wire takes its value from, body by body from the outside in, so that what enters a frame is
/// already known outside it; and the error inputs of each node.
fn sources(graph: &Graph<'_>, plan: &mut TracePlan) -> Sources {
    let mut found: Sources = HashMap::new();
    // What reaches each input of a node, and each terminal on the right of a frame: the start of its wire.
    let mut into: HashMap<(usize, End), End> = HashMap::new();
    for (body, of) in graph.bodies.iter().enumerate() {
        for wire in &of.wires {
            into.entry((body, wire.to)).or_insert(wire.from);
        }
    }
    for (body, of) in graph.bodies.iter().enumerate() {
        // Each start once, its index given before it is worked out: a carry passed on unchanged names itself.
        let starts: Vec<End> = {
            let mut seen = std::collections::HashSet::new();
            let wires = of.wires.iter().map(|wire| wire.from);
            let frame = of.left.iter().enumerate().filter(|(_, terminal)| terminal.edge != Edge::Notch).map(|(index, _)| End::Left(index));
            wires.chain(frame).filter(|start| seen.insert(*start)).collect()
        };
        for start in &starts {
            found.entry((body, *start)).or_insert_with(|| push(plan, Source::None));
        }
        for start in starts {
            let source = source_of(graph, plan, &found, &into, body, start);
            if let Some(slot) = found.get(&(body, start)).and_then(|&at| plan.sources.get_mut(at as usize)) {
                *slot = source;
            }
        }
    }
    // The error inputs of each node.
    for (index, node) in graph.nodes.iter().enumerate() {
        let errors: Vec<u32> = node
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, terminal)| terminal.family == graph::Family::ERROR)
            .filter_map(|(terminal, _)| into.get(&(node.body, End::Node { node: index, terminal })))
            .filter_map(|from| found.get(&(node.body, *from)).copied())
            .collect();
        let err = node
            .inputs
            .iter()
            .position(|terminal| terminal.name == "err" && terminal.family == graph::Family::ERROR)
            .and_then(|terminal| into.get(&(node.body, End::Node { node: index, terminal })))
            .and_then(|from| found.get(&(node.body, *from)).copied());
        if let Some(planned) = plan.nodes.get_mut(index) {
            planned.errors_in = errors;
            planned.err = err;
        }
    }
    found
}

fn push(plan: &mut TracePlan, source: Source) -> u32 {
    plan.sources.push(source);
    u32::try_from(plan.sources.len() - 1).unwrap_or(u32::MAX)
}

/// Where the value that leaves `start` in `body` comes from.
fn source_of(graph: &Graph<'_>, plan: &mut TracePlan, found: &Sources, into: &HashMap<(usize, End), End>, body: usize, start: End) -> Source {
    let index = |at: usize| u32::try_from(at).unwrap_or(u32::MAX);
    match start {
        End::Node { node, terminal } => match plan.nodes.get(node).and_then(|found| found.outputs.get(terminal)).copied().flatten() {
            Some(wire) => Source::Output { node: index(node), wire },
            None => Source::None,
        },
        End::Left(at) => {
            let Some(of) = graph.bodies.get(body) else { return Source::None };
            let Some(terminal) = of.left.get(at) else { return Source::None };
            let Some(owner) = of.owner else {
                return if terminal.edge == Edge::Input { Source::Input(index(at)) } else { Source::None };
            };
            let Some(structure) = graph.nodes.get(owner) else { return Source::None };
            let outer = structure.body;
            // What enters input `input` of the structure, outside: a wire, or a constant of the text.
            let entering = |plan: &mut TracePlan, input: usize| -> u32 {
                if let Some(source) = into.get(&(outer, End::Node { node: owner, terminal: input })).and_then(|from| found.get(&(outer, *from))) {
                    return *source;
                }
                let constant = structure.inputs.get(input).and_then(|terminal| terminal.constant.as_deref()).and_then(literal);
                push(plan, constant.map_or(Source::None, Source::Constant))
            };
            let input_of = |at: usize| -> Option<usize> {
                let Kind::Structure(found) = &structure.kind else { return None };
                let rank = structure.bodies.iter().position(|&inner| inner == body)?;
                found.inputs_at.iter().position(|places| places.contains(&(rank, at)))
            };
            match terminal.edge {
                Edge::Register => {
                    let Some(input) = input_of(at) else { return Source::None };
                    let first = entering(plan, input);
                    // Its `next` box: the registers come first on the right, in the order of the carries.
                    let rank = of.left.iter().take(at).filter(|terminal| terminal.edge == Edge::Register).count();
                    let next = into.get(&(body, End::Right(rank))).and_then(|from| found.get(&(body, *from)).copied());
                    let next = next.unwrap_or_else(|| push(plan, Source::None));
                    Source::Carry { looped: index(owner), first, next }
                }
                Edge::In => match input_of(at) {
                    Some(input) => Source::Element { looped: index(owner), array: entering(plan, input) },
                    None => Source::None,
                },
                Edge::Range => {
                    // A start written in the text is in the box of the range (`0..10`); one that a wire gives enters it.
                    let written = terminal.start.as_deref().and_then(|text| text.split("..").next()).filter(|start| *start != "\u{B7}");
                    let start = match (written.and_then(literal), input_of(at)) {
                        (Some(constant), _) => push(plan, Source::Constant(constant)),
                        (None, Some(input)) => entering(plan, input),
                        (None, None) => push(plan, Source::None),
                    };
                    Source::Index { looped: index(owner), start }
                }
                Edge::Notch => match input_of(at) {
                    Some(input) => {
                        let outer = entering(plan, input);
                        plan.sources.get(outer as usize).cloned().unwrap_or_default()
                    }
                    None => Source::None,
                },
                Edge::Bound => {
                    let selector = entering(plan, 0);
                    let (variant, place) = terminal.bound.clone().unwrap_or((None, 0));
                    Source::Bound { selector, variant, place: index(place) }
                }
                _ => Source::None,
            }
        }
        End::Right(_) | End::Order(_) => Source::None,
    }
}

/// A constant of the text as JSON (`docs/commands.md`, 5), for a first value or the start of a range: a number, with
/// its unit, a boolean, a text, a variant. Another constant shows no value.
fn literal(text: &str) -> Option<String> {
    let text = text.trim();
    if text == "true" || text == "false" {
        return Some(text.to_owned());
    }
    if text.starts_with('"') {
        return Some(text.to_owned());
    }
    let (number, unit) = text.split_once(' ').unwrap_or((text, ""));
    let digits = number.trim_start_matches('-').replace('_', "");
    if digits.starts_with(|c: char| c.is_ascii_digit()) && digits.parse::<f64>().is_ok() {
        let number = if number.starts_with('-') { format!("-{digits}") } else { digits };
        return Some(if unit.is_empty() { number } else { format!("\"{number} {unit}\"") });
    }
    // `Verdict.Pass`.
    if text.contains('.') && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return Some(format!("\"{text}\""));
    }
    None
}

/// What the reader of the trace keeps for this drawing (`docs/architecture.md`, 11.2).
pub fn expected(drawing: &Drawing) -> Expected {
    let plan = &drawing.trace;
    let wire = |wire: &PlanWire| ExpectedWire { name: wire.name.clone(), ty: wire.ty.clone() };
    Expected {
        function: drawing.function.clone(),
        inputs: plan.inputs.iter().map(wire).collect(),
        nodes: plan
            .nodes
            .iter()
            .map(|node| {
                let body = plan.bodies.get(node.body as usize).cloned().unwrap_or_default();
                ExpectedNode {
                    name: node.name.clone(),
                    arms: body.arms.into_iter().map(|(node, arm)| labcraft_trace::ArmOf { node, arm }).collect(),
                    loops: body.loops,
                    wires: node.wires.iter().map(wire).collect(),
                }
            })
            .collect(),
    }
}

/// What the viewer chooses of the values: the iteration of each loop, by its key, the last by default (notation 6.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    pub iterations: HashMap<String, u64>,
}

impl Choice {
    /// The iteration shown of a loop that made `count`, more than none: the one chosen, at most the last, or the last.
    fn shown(&self, key: &str, count: u64) -> u64 {
        let last = count.saturating_sub(1);
        self.iterations.get(key).map_or(last, |chosen| (*chosen).min(last))
    }
}

/// How the trace stands against the files: the files that have changed or are gone since the run, and the time it was
/// written, which the viewer gives and an SVG does not (`docs/cli.md`, 7.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Judged {
    pub stale: Vec<String>,
    pub written: Option<String>,
}

/// Shapes over the drawing, shown with what they stand over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layer {
    pub shapes: Vec<Shape>,
    /// As [`crate::Group::within`].
    pub within: Option<usize>,
    /// As [`crate::Group::placed`]: what it glides with.
    pub placed: Option<usize>,
}

/// The value of a wire: its bubble, its full value for the tooltip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bubble {
    /// `data-value`: the name of its wire.
    pub wire: String,
    /// The `<title>`: the value whole, as the file gave it, cut at 200 code points.
    pub title: String,
    /// Empty when no place holds it: its value is read on hover.
    pub layer: Layer,
}

/// The values of a run over a drawing (notation 6.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Overlay {
    /// The room above the drawing that the banner takes, in hundredths: the image grows by it.
    pub top: i64,
    /// The width of the image, when the banner needs more than the drawing: its text is never cut (notation 6.1).
    pub width: i64,
    pub banner: Vec<Shape>,
    /// The veils over the arms that the run did not take, the hatching of the nodes it skipped, the outlines and badges
    /// of the nodes that produced an error, the counters of the loops: under the bubbles.
    pub under: Vec<Layer>,
    pub bubbles: Vec<Bubble>,
    /// The arm that the run took in each `match` and `handle` it reached, by the key of the structure: the tab shown by
    /// default (notation 6.1).
    pub taken: BTreeMap<String, usize>,
    /// Each loop that the run reached: its key, how many iterations it made, and the one shown.
    pub loops: Vec<(String, u64, u64)>,
}

/// Works out values in a trace.
struct Reading<'a> {
    plan: &'a TracePlan,
    trace: &'a Trace,
}

impl<'a> Reading<'a> {
    fn loops_of(&self, node: u32) -> usize {
        self.plan.nodes.get(node as usize).and_then(|node| self.plan.bodies.get(node.body as usize)).map_or(0, |body| body.loops.len())
    }

    /// The value of `source` at `iters`, the iterations of the body that reads it, as JSON; none when the trace has none.
    fn value(&self, source: u32, iters: &[u64]) -> Option<Cow<'a, str>> {
        let mut source = source;
        let mut iters: Vec<u64> = iters.to_vec();
        for _ in 0..STEPS {
            match self.plan.sources.get(source as usize)? {
                Source::None => return None,
                Source::Output { node, wire } => return self.output(*node, *wire, &iters),
                Source::Input(input) => return self.trace.input(*input as usize).map(Cow::Borrowed),
                Source::Constant(json) => return Some(Cow::Owned(json.clone())),
                Source::Carry { looped, first, next } => {
                    let level = self.loops_of(*looped);
                    let iteration = *iters.get(level)?;
                    if iteration == 0 {
                        iters.truncate(level);
                        source = *first;
                    } else {
                        iters.truncate(level + 1);
                        *iters.get_mut(level)? = iteration - 1;
                        source = *next;
                    }
                }
                Source::Element { looped, array } => {
                    let level = self.loops_of(*looped);
                    let iteration = *iters.get(level)?;
                    let array = self.value(*array, iters.get(..level)?)?;
                    return element(&array, iteration).map(|text| Cow::Owned(text.to_owned()));
                }
                Source::Index { looped, start } => {
                    let level = self.loops_of(*looped);
                    let iteration = *iters.get(level)?;
                    let start = self.value(*start, iters.get(..level)?)?;
                    let start: i128 = start.trim().trim_matches('"').parse().ok()?;
                    return Some(Cow::Owned((start + i128::from(iteration)).to_string()));
                }
                Source::Bound { selector, variant, place } => {
                    let selector = self.value(*selector, &iters)?;
                    return match variant {
                        None => Some(selector),
                        Some(variant) => bound(&selector, variant, *place).map(|text| Cow::Owned(text.to_owned())),
                    };
                }
            }
        }
        None
    }

    /// What a node bound at its iterations; none for an output that a skipped node, or one that produced an error, fills
    /// with its default (notation 6.1).
    fn output(&self, node: u32, wire: u32, iters: &[u64]) -> Option<Cow<'a, str>> {
        let found = self.plan.nodes.get(node as usize)?;
        let iters = iters.get(..self.loops_of(node))?;
        let planned = found.wires.get(wire as usize)?;
        // An error or a session passes whatever the node did, so the chain of errors before it is never walked. A structure
        // that ends on an error gives its last values: a loop keeps the last `next` of its carries (spec 6.3).
        if !planned.passes && (self.skipped(node, iters) || found.kind == PlanKind::Node && self.produced_an_error(node, iters)) {
            return None;
        }
        self.trace.value(node, wire, iters).map(Cow::Borrowed)
    }

    /// Whether a node was skipped (spec 7.2): its `skipped` event, or a call whose `err` input held an error and whose
    /// error output still does, which the trace writes as `ran` when the function called is written in LabCraft, the nodes
    /// of its body being skipped; a body that clears the error ran.
    fn skipped(&self, node: u32, iters: &[u64]) -> bool {
        if self.trace.skipped(node, iters) {
            return true;
        }
        let Some(found) = self.plan.nodes.get(node as usize) else { return false };
        found.kind == PlanKind::Node
            && !found.always
            && found.err.is_some_and(|source| carries(self.value(source, iters).as_deref()))
            && found.errors_out.iter().any(|&wire| carries(self.trace.value(node, wire, iters)))
    }

    /// Whether the error output of a node that ran carries an error that none of its inputs carried.
    fn produced_an_error(&self, node: u32, iters: &[u64]) -> bool {
        let Some(found) = self.plan.nodes.get(node as usize) else { return false };
        let out = found.errors_out.iter().any(|&wire| carries(self.trace.value(node, wire, iters)));
        out && !found.errors_in.iter().any(|&source| carries(self.value(source, iters).as_deref()))
    }
}

/// Whether a value of an error, as JSON, carries an error: `null` is `none`.
fn carries(text: Option<&str>) -> bool {
    text.is_some_and(|text| text.trim() != "null")
}

/// The element `at` of an array of JSON, or of the head that the trace kept of it; none past what it holds.
fn element(array: &str, at: u64) -> Option<&str> {
    let trimmed = array.trim();
    let items = if trimmed.starts_with('{') { head_of(trimmed)?.1 } else { trimmed };
    let mut scanner = Scanner::new(items, DEPTH);
    scanner.begin_array().ok()?;
    let mut first = true;
    let mut index = 0u64;
    while scanner.element(&mut first).ok()? {
        let value = scanner.value().ok()?;
        if index == at {
            return (!crate::short::is_cut_mark(value)).then_some(value);
        }
        index += 1;
    }
    None
}

/// The value at `place` of the variant `variant` of a selector: `{ "Reading.Value": [ … ] }`.
fn bound<'t>(selector: &'t str, variant: &str, place: u32) -> Option<&'t str> {
    let mut scanner = Scanner::new(selector.trim(), DEPTH);
    scanner.begin_object().ok()?;
    let mut first = true;
    while let Some(key) = scanner.key(&mut first).ok()? {
        let value = scanner.value().ok()?;
        if key.rsplit('.').next() == Some(variant) {
            return element(value, u64::from(place));
        }
    }
    None
}

/// The diagnostics of reading a trace (`docs/cli.md`, 7.4 and 7.6), located in its file, `shown`.
pub fn diagnostics(trace: &Trace, shown: &str) -> Vec<Diagnostic> {
    let mut found = Vec::new();
    let at_line = |code: Code, message: String, line: u64| {
        let at = labcraft_types::Position { line: u32::try_from(line).unwrap_or(u32::MAX), column: 1 };
        Diagnostic::new(code, message, shown).with_span(labcraft_types::Span { start: at, end: at })
    };
    match &trace.header {
        Err(problem) => {
            let why = match problem {
                HeaderProblem::Unreadable => "its first line does not read as the first line of a trace",
                HeaderProblem::OtherSchema => "its first line names another schema than `labcraft-trace`",
                HeaderProblem::LaterVersion => "its first line names a later version of `labcraft-trace` than 1",
            };
            found.push(at_line(Code::Lc7101, format!("the trace gives no values: {why}"), 1));
            return found;
        }
        Ok(header) if trace.other_function => {
            let message = format!("the trace is that of `{}`, not of the function drawn: no values", cut_text(&header.function));
            found.push(at_line(Code::Lc7102, message, 1));
            return found;
        }
        Ok(_) => {}
    }
    if let Some(stop) = trace.stop {
        let limit = match stop.limit {
            Limit::Unreadable => "a line that does not read",
            Limit::File => "the size of the file (--max-trace-size)",
            Limit::Line => "the size of a line, 16 MiB",
            Limit::Lines => "the number of lines, 1,000,002 after the first",
            Limit::Kept => "what the viewer keeps of the values, 384 MiB",
        };
        found.push(at_line(
            Code::Lc7103,
            format!("reading the trace stopped at line {}: {limit}; the values up to there are shown", stop.line),
            stop.line,
        ));
    }
    if trace.ignored > 0 {
        let line = trace.first_ignored.unwrap_or(1);
        let events = if trace.ignored == 1 { "1 event".to_owned() } else { format!("{} events", trace.ignored) };
        let message = format!("{events} of the trace fit nothing in the drawing and are ignored, the first at line {line}");
        found.push(at_line(Code::Lc7104, message, line));
    }
    found
}

/// The diagnostic of a trace file that does not exist or cannot be read (`LC7101`).
pub fn unreadable(shown: &str, why: &str) -> Diagnostic {
    Diagnostic::new(Code::Lc7101, format!("the trace cannot be read: {why}; no values"), shown)
}

/// A rectangle: left, top, right, bottom.
type Area = (i64, i64, i64, i64);

fn overlaps(a: Area, b: Area) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// The values of a run on a drawing (notation 6.1): nothing when the trace gives none (`LC7101`, `LC7102`).
pub fn values(drawing: &Drawing, trace: &Trace, judged: &Judged, choice: &Choice) -> Overlay {
    let mut overlay = Overlay::default();
    if trace.header.is_err() || trace.other_function {
        return overlay;
    }
    let reading = Reading { plan: &drawing.trace, trace };
    let plan = &drawing.trace;
    // The iterations of each body, the last by default; none for a body that the run did not reach.
    let mut contexts: Vec<Option<Vec<u64>>> = Vec::with_capacity(plan.bodies.len());
    for body in &plan.bodies {
        let mut iters: Vec<u64> = Vec::with_capacity(body.loops.len());
        let mut live = true;
        for &looped in &body.loops {
            let count = trace.iterations(looped, &iters);
            if count == 0 {
                live = false;
                break;
            }
            let key = match plan.nodes.get(looped as usize).map(|node| &node.kind) {
                Some(PlanKind::Loop { key, .. }) => key.as_str(),
                _ => "",
            };
            iters.push(choice.shown(key, count));
        }
        if live {
            live = body.arms.iter().all(|(structure, arm)| {
                let prefix = iters.get(..reading.loops_of(*structure)).unwrap_or(&[]);
                trace.arm(*structure, prefix) == Some(&**arm)
            });
        }
        contexts.push(live.then_some(iters));
    }
    let context_of = |node: &PlanNode| contexts.get(node.body as usize).cloned().flatten();
    let stale = !judged.stale.is_empty();
    banner(drawing, trace, judged, &mut overlay);
    under(drawing, &reading, &contexts, &mut overlay);
    // The loops reached, for the selectors of the viewer.
    for (index, node) in plan.nodes.iter().enumerate() {
        if let (PlanKind::Loop { key, .. }, Some(iters)) = (&node.kind, context_of(node)) {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            let count = trace.iterations(index, &iters);
            if count > 0 {
                overlay.loops.push((key.clone(), count, choice.shown(key, count)));
            }
        }
    }
    // Each value that starts in a body, with what it shows; the inputs of the function first.
    struct Found {
        key: (u32, u32, u32),
        wire: String,
        json: String,
        short: Short,
        group: Option<usize>,
        /// An input of the function shows its value where the pill of its default starts, at least as wide.
        anchor: Option<((i64, i64), i64)>,
        node: Option<u32>,
    }
    let mut found: Vec<Found> = Vec::new();
    let mut inputs_wired = vec![false; plan.inputs.len()];
    for (index, group) in drawing.groups.iter().enumerate() {
        let Some(wire) = &group.wire else { continue };
        let (Some(source), true) = (wire.source, wire.starts) else { continue };
        let Some(Some(iters)) = contexts.get(wire.body) else { continue };
        let Some(json) = reading.value(source, iters) else { continue };
        let Some(shown) = short(&json, &wire.ty) else { continue };
        let (key, anchor, node) = match plan.sources.get(source as usize) {
            Some(Source::Input(input)) => {
                if let Some(slot) = inputs_wired.get_mut(*input as usize) {
                    *slot = true;
                }
                let pill = plan.defaulted.get(*input as usize).copied().unwrap_or(0);
                let anchor = plan.anchors.get(*input as usize).copied().filter(|_| pill > 0).map(|at| (at, pill));
                ((0, *input, 0), anchor, None)
            }
            Some(Source::Output { node, wire }) => {
                ((1, plan.nodes.get(*node as usize).map_or(u32::MAX, |node| node.order), *wire), None, Some(*node))
            }
            _ => {
                // A value that a frame binds: in the order of its structure.
                let order = plan.bodies.get(wire.body).map_or(u32::MAX, |body| body.order);
                ((1, order, u32::MAX / 2 + u32::try_from(index).unwrap_or(0)), None, None)
            }
        };
        found.push(Found { key, wire: wire.value.clone(), json: json.into_owned(), short: shown, group: Some(index), anchor, node });
    }
    // The inputs that no wire leaves still show their value at their terminal.
    for (input, wired) in inputs_wired.iter().enumerate() {
        if *wired {
            continue;
        }
        let (Some(json), Some(planned)) = (trace.input(input), plan.inputs.get(input)) else { continue };
        let Some(shown) = short(json, &planned.ty) else { continue };
        let input = u32::try_from(input).unwrap_or(u32::MAX);
        found.push(Found {
            key: (0, input, 0),
            wire: planned.name.clone(),
            json: json.to_owned(),
            short: shown,
            group: None,
            anchor: plan.anchors.get(input as usize).copied().map(|at| (at, plan.defaulted.get(input as usize).copied().unwrap_or(0))),
            node: None,
        });
    }
    found.sort_by_key(|found| found.key);
    // Placed in that order, the outputs of one node from the bottom line up.
    let mut obstacles = Obstacles::of(drawing);
    let mut layers: Vec<Option<Layer>> = vec![None; found.len()];
    let mut by_node: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (at, item) in found.iter().enumerate() {
        if let (Some(node), None) = (item.node, item.anchor) {
            by_node.entry(node).or_default().push(at);
        }
    }
    for outputs in by_node.values() {
        let mut outputs: Vec<(usize, i64)> =
            outputs.iter().map(|&at| (at, found.get(at).and_then(|item| start_of(drawing, item.group)).map_or(0, |(_, y, _)| y))).collect();
        outputs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut below: Option<(i64, i64)> = None;
        for (at, y) in outputs {
            let least = below.filter(|(under, _)| under - y < style::probe_apart()).map(|(_, end)| end + style::probe_gap());
            if let Some(item) = found.get(at) {
                let layer = bubble_at_wire(drawing, item.group, &item.short, stale, least, &mut obstacles);
                if let Some(end) = layer.shapes.first().map(|shape| crate::extent(shape).2) {
                    below = Some((y, end));
                }
                if let Some(slot) = layers.get_mut(at) {
                    *slot = Some(layer);
                }
            }
        }
    }
    for (at, item) in found.iter().enumerate() {
        if layers.get(at).is_some_and(Option::is_some) {
            continue;
        }
        let layer = match item.anchor {
            Some((anchor, least)) => bubble_at(drawing, anchor, least, &item.short, stale, &mut obstacles),
            None => bubble_at_wire(drawing, item.group, &item.short, stale, None, &mut obstacles),
        };
        if let Some(slot) = layers.get_mut(at) {
            *slot = Some(layer);
        }
    }
    for (item, layer) in found.into_iter().zip(layers) {
        let mut layer = layer.unwrap_or_default();
        let group = item.group.and_then(|group| drawing.groups.get(group));
        layer.within = group.and_then(|group| group.within);
        layer.placed = group.and_then(|group| group.placed);
        let mut title = cut_text(&item.json).into_owned();
        if item.json.contains("\"$len\"") || item.json.contains("\"$cut\"") {
            title.push_str("\n(cut by the trace)");
        }
        overlay.bubbles.push(Bubble { wire: item.wire, title, layer });
    }
    overlay
}

/// Where a wire starts, and where its first horizontal segment ends: `(x, y, end)`.
fn start_of(drawing: &Drawing, group: Option<usize>) -> Option<(i64, i64, i64)> {
    let wire = drawing.groups.get(group?)?.wire.as_ref()?;
    let points = &wire.readers.first()?.1;
    let &(x, y) = points.first()?;
    let end = points.iter().take_while(|(_, at)| *at == y).map(|(at, _)| *at).max().unwrap_or(x);
    Some((x, y, end.max(x)))
}

/// The side of a cell of [`Obstacles`], in hundredths: 64 units, a few bubbles wide.
const CELL: i64 = 6400;
/// An obstacle over more cells than this is kept apart and looked at for every bubble: a frame of a structure, of which a
/// drawing has few, so that a large one never fills thousands of cells.
const MOST_CELLS: i64 = 64;

/// What a bubble must not cover (notation 6.1), each with the body whose bubbles it stops, or every bubble: the nodes of a
/// body, a structure with its whole frame; the headers of the structures and of the arms, the pills, the terminals of
/// the frames and their names, the tabs, and the bubbles placed before. Kept in cells, so that finding what is in the way
/// of a bubble looks at what is near it, however large the drawing: placing every bubble stays near linear.
#[derive(Default)]
struct Obstacles {
    items: Vec<(Area, Option<usize>)>,
    cells: HashMap<(i64, i64), Vec<usize>>,
    large: Vec<usize>,
}

impl Obstacles {
    fn of(drawing: &Drawing) -> Obstacles {
        let mut found = Obstacles::default();
        for placed in &drawing.placed {
            found.push((placed.x, placed.y, placed.x + placed.width, placed.y + placed.height), Some(placed.body));
            if !placed.holds.is_empty() {
                found.push((placed.x, placed.y, placed.x + placed.width, placed.y + style::frame_header()), None);
                for body in &placed.holds {
                    if let Some(frame) = drawing.bodies.get(*body) {
                        let (x, y, width, _) = frame.frame;
                        if (x, y) != (placed.x, placed.y) {
                            found.push((x, y, x + width, y + style::arm_header()), None);
                        }
                    }
                }
            }
        }
        let pill = |shape: &Shape| matches!(shape, Shape::Rect { paint, .. } if paint.fill == Some("pill.fill"));
        for group in &drawing.groups {
            for shape in &group.shapes {
                if pill(shape) || group.terminals.is_some() && !matches!(shape, Shape::Wire { .. }) {
                    found.push(visible(shape), None);
                }
            }
        }
        for shape in &drawing.loose {
            if matches!(shape, Shape::Text { .. }) {
                found.push(visible(shape), None);
            }
        }
        for tab in &drawing.tabs {
            found.push((tab.x, tab.y, tab.x + tab.width, tab.y + tab.height), None);
        }
        found
    }

    /// The cells an area spans: the first and last on each axis.
    fn span(area: Area) -> (i64, i64, i64, i64) {
        let cell = |at: i64| at.div_euclid(CELL);
        (cell(area.0), cell(area.1), cell(area.2.max(area.0)), cell(area.3.max(area.1)))
    }

    fn push(&mut self, area: Area, body: Option<usize>) {
        let index = self.items.len();
        self.items.push((area, body));
        let (x0, y0, x1, y1) = Self::span(area);
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > MOST_CELLS {
            self.large.push(index);
            return;
        }
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                self.cells.entry((cx, cy)).or_default().push(index);
            }
        }
    }

    /// The right edge of what is in the way of `area` in `body`, the farthest; none when nothing is. With `nodes`, only the
    /// nodes of that body count.
    fn in_the_way(&self, area: Area, body: Option<usize>, nodes: bool) -> Option<i64> {
        let (x0, y0, x1, y1) = Self::span(area);
        let near = (x0..=x1).flat_map(|cx| (y0..=y1).map(move |cy| (cx, cy))).filter_map(|cell| self.cells.get(&cell)).flatten();
        near.chain(self.large.iter())
            .filter_map(|&index| self.items.get(index))
            .filter(|(other, of)| (if nodes { of.is_some() && *of == body } else { of.is_none() || *of == body }) && overlaps(*other, area))
            .map(|(other, _)| other.2)
            .max()
    }
}

/// What a shape shows: a text from the top of its capitals to the bottom of its descenders, which is less than the line
/// it is laid out in.
fn visible(shape: &Shape) -> Area {
    match shape {
        Shape::Text { x, y, size, width, .. } => {
            let cap = labcraft_types::text::scale(u64::from(labcraft_types::text::CAP_HEIGHT), *size).unwrap_or(0);
            let descent = labcraft_types::text::scale(u64::from(labcraft_types::text::DESCENT), *size).unwrap_or(0);
            (*x, y - cap, x + width, y + descent)
        }
        _ => crate::extent(shape),
    }
}

/// The shapes of a bubble at `(x, top)`.
fn bubble_shapes(x: i64, top: i64, width: i64, text: &Text, short: &Short, stale: bool) -> Vec<Shape> {
    let paint = if stale {
        Paint::both("canvas.fill", "probe.stroke", style::outline()).dashed()
    } else {
        Paint::both("probe.fill", "probe.stroke", style::outline())
    };
    let ink: Token = match (stale, short.error) {
        (true, _) => "ink.muted",
        (false, true) => "wire.error",
        (false, false) => "node.title",
    };
    vec![
        Shape::Rect { x, y: top, width, height: style::probe_height(), radius: style::probe_radius(), paint },
        text.at(x + style::padding(), top + style::probe_height() / 2, ink),
    ]
}

/// A bubble at the start of the wire of `group`, 4 units above it, moved along its first horizontal segment to the first
/// place where it covers nothing it must not, from `least` on; none when there is none.
fn bubble_at_wire(drawing: &Drawing, group: Option<usize>, short: &Short, stale: bool, least: Option<i64>, obstacles: &mut Obstacles) -> Layer {
    let Some((x, y, end)) = start_of(drawing, group) else { return Layer::default() };
    let body = group.and_then(|group| drawing.groups.get(group)).and_then(|group| group.wire.as_ref()).map(|wire| wire.body);
    let frame = body.and_then(|body| drawing.bodies.get(body)).map(|body| body.frame);
    let text = Text::new(&short.text, size::any_text(), size::medium());
    let width = text.width + 2 * style::padding();
    let top = y - style::probe_above() - style::probe_height();
    // Its start moves along the segment; the bubble may reach past its end, over the wire that turns there.
    let first = least.unwrap_or(x + style::terminal() / 2 + style::outline()).max(x);
    let last = end.max(first);
    let inside =
        |left: i64| frame.is_none_or(|(fx, fy, fw, fh)| left >= fx && left + width <= fx + fw && top >= fy && top + style::probe_height() <= fy + fh);
    let bottom = top + style::probe_height();
    let mut left = first;
    while left <= last {
        let area = (left, top, left + width, bottom);
        // Past the obstacle in the way, at once: the next place that could hold the bubble starts at its right edge.
        if let Some(blocker) = obstacles.in_the_way(area, body, false) {
            left = blocker.max(left + 1);
            continue;
        }
        if inside(left) {
            obstacles.push(area, None);
            // The name it covers first, so that the fill that hides it never covers the bubble.
            let mut shapes = name_after(drawing, group, area, end, obstacles);
            shapes.extend(bubble_shapes(left, top, width, &text, short, stale));
            return Layer { shapes, ..Layer::default() };
        }
        left += style::grid() / 4;
    }
    Layer::default()
}

/// The name of the wire, when the bubble covers it: hidden under the fill of its frame, and written again after the
/// bubble when the segment has room for it there (notation 6.1); else the tooltip gives it.
fn name_after(drawing: &Drawing, group: Option<usize>, bubble: Area, end: i64, obstacles: &mut Obstacles) -> Vec<Shape> {
    let Some(group) = group.and_then(|group| drawing.groups.get(group)) else { return Vec::new() };
    let Some(wire) = &group.wire else { return Vec::new() };
    let depth = drawing.trace.bodies.get(wire.body).map_or(0, |body| body.depth);
    let name = group.shapes.iter().find(|shape| {
        matches!(shape, Shape::Text { runs, .. } if runs.first().is_some_and(|run| run.token == "ink.muted")) && overlaps(visible(shape), bubble)
    });
    let Some(shape @ Shape::Text { y, size, width, runs, detail, .. }) = name else { return Vec::new() };
    let (cx0, cy0, cx1, cy1) = visible(shape);
    let mut shapes =
        vec![Shape::Rect { x: cx0, y: cy0, width: cx1 - cx0, height: cy1 - cy0, radius: 0, paint: Paint::fill(crate::frame_fill(depth)) }];
    let moved = bubble.2 + style::name_side();
    let area = (moved, cy0, moved + width, cy1);
    if moved + width <= end && obstacles.in_the_way(area, Some(wire.body), false).is_none() {
        obstacles.push(area, None);
        shapes.push(Shape::Text { x: moved, y: *y, size: *size, width: *width, runs: runs.clone(), detail: *detail, ground: None });
    }
    shapes
}

/// A bubble at the terminal of an input of the function, in place of the pill of its default, which it covers; none
/// when it would leave the frame of the function or cover one of its nodes: its value is then read on hover.
fn bubble_at(drawing: &Drawing, (x, y): (i64, i64), least: i64, short: &Short, stale: bool, obstacles: &mut Obstacles) -> Layer {
    let text = Text::new(&short.text, size::any_text(), size::medium());
    let top = y - style::probe_height() / 2;
    let width = (text.width + 2 * style::padding()).max(least);
    let area = (x, top, x + width, top + style::probe_height());
    let inside = drawing.bodies.first().is_none_or(|body| {
        let (fx, _, fw, _) = body.frame;
        x >= fx && x + width <= fx + fw
    });
    if !inside || obstacles.in_the_way(area, Some(0), true).is_some() {
        return Layer::default();
    }
    obstacles.push(area, None);
    Layer { shapes: bubble_shapes(x, top, width, &text, short, stale), ..Layer::default() }
}

/// The banner above the drawing: the files that have changed since the run, then how the run ended when it did not end
/// well (notation 6.1).
fn banner(drawing: &Drawing, trace: &Trace, judged: &Judged, overlay: &mut Overlay) {
    let mut parts: Vec<String> = Vec::new();
    if let Some(first) = judged.stale.first() {
        // A name of the drawing, not of the trace: whole, as any text of a report (200 code points).
        let first = cut_text(first);
        let mut text = match judged.stale.len() {
            1 => format!("`{first}` has changed since this run"),
            2 => format!("`{first}` and 1 other file have changed since this run"),
            more => format!("`{first}` and {} other files have changed since this run", more - 1),
        };
        if let Some(time) = &judged.written {
            text.push_str(&format!(" ({time})"));
        }
        parts.push(text);
    }
    match trace.end.as_deref() {
        Some("aborted") => parts.push("the run was aborted".to_owned()),
        Some("fault") => parts.push("the run stopped on a fault".to_owned()),
        None if !trace.cut() => parts.push("the trace has no end: the run did not finish writing it".to_owned()),
        _ => {}
    }
    if trace.cut() {
        parts.push("the trace was cut: the values end at the last iteration traced".to_owned());
    }
    if parts.is_empty() {
        return;
    }
    let mut text = parts.join("; ");
    if let Some(first) = text.get(..1) {
        text = first.to_uppercase() + text.get(1..).unwrap_or_default();
    }
    let height = style::banner_height();
    let left = style::margin();
    // Its parts are bounded, the names of files cut: the text is whole, and the image as wide as it needs.
    let shown = Text::new(&text, u64::MAX, size::medium());
    let width = (drawing.width - 2 * style::margin()).max(shown.width + 2 * style::padding());
    let top = -height - style::probe_gap();
    overlay.top = height + style::probe_gap() + style::margin();
    overlay.width = width + 2 * style::margin();
    overlay.banner = vec![
        Shape::Rect {
            x: left,
            y: top,
            width,
            height,
            radius: style::probe_radius(),
            paint: Paint::both("probe.fill", "probe.stroke", style::outline()),
        },
        shown.at(left + style::padding(), top + height / 2, "node.title"),
    ];
}

/// The veils over the arms that the run did not take, the hatching of the nodes it skipped, the outline and badge of the
/// nodes that produced an error, the counters of the loops.
fn under(drawing: &Drawing, reading: &Reading<'_>, contexts: &[Option<Vec<u64>>], overlay: &mut Overlay) {
    let plan = reading.plan;
    let trace = reading.trace;
    let mut within_of: HashMap<usize, Option<usize>> = HashMap::new();
    for group in drawing.groups.iter().filter(|group| group.is_node()) {
        if let Some(placed) = group.placed {
            within_of.entry(placed).or_insert(group.within);
        }
    }
    let arm_of: HashMap<(&str, usize), usize> = drawing.arms.iter().enumerate().map(|(index, of)| ((of.structure.as_str(), of.arm), index)).collect();
    for (index, node) in plan.nodes.iter().enumerate() {
        let index32 = u32::try_from(index).unwrap_or(u32::MAX);
        let Some(Some(iters)) = contexts.get(node.body as usize) else { continue };
        let iters = iters.get(..reading.loops_of(index32)).unwrap_or(&[]);
        let within = node.placed.and_then(|placed| within_of.get(&(placed as usize)).copied().flatten());
        let placed = node.placed.map(|placed| placed as usize);
        match &node.kind {
            PlanKind::Choice { key, arms } => {
                let Some(taken) = trace.arm(index32, iters) else { continue };
                for (rank, (body, arm)) in arms.iter().enumerate() {
                    if &**arm == taken {
                        overlay.taken.insert(key.clone(), rank);
                        continue;
                    }
                    let (Some(frame), Some(depth)) = (drawing.bodies.get(*body as usize), plan.bodies.get(*body as usize).map(|body| body.depth))
                    else {
                        continue;
                    };
                    let (x, y, width, height) = frame.frame;
                    let veil =
                        Shape::Rect { x, y, width, height, radius: style::radius(), paint: Paint::fill(crate::frame_fill(depth)).veiled(VEIL) };
                    let arm_within = arm_of.get(&(key.as_str(), rank)).copied().or(within);
                    overlay.under.push(Layer { shapes: vec![veil], within: arm_within, placed });
                }
            }
            PlanKind::Loop { counter: (from, middle, to), .. } => {
                let count = trace.iterations(index32, iters);
                // `× 0` for a loop that ran and made no iteration; nothing for one that the run did not reach.
                let ran = (0..node.wires.len()).any(|wire| trace.value(index32, u32::try_from(wire).unwrap_or(u32::MAX), iters).is_some());
                if count == 0 && !ran {
                    continue;
                }
                let text = Text::new(&format!("\u{D7} {count}"), size::any_text(), size::medium());
                let left = to - text.width;
                if left >= from + style::grid() {
                    overlay.under.push(Layer { shapes: vec![text.at(left, *middle, "ink.muted")], within, placed });
                }
            }
            PlanKind::Node => {
                let Some(block) = node.placed.and_then(|placed| drawing.placed.get(placed as usize)) else { continue };
                let area = (block.x, block.y, block.x + block.width, block.y + block.height);
                if reading.skipped(index32, iters) {
                    let mut shapes = vec![Shape::Rect {
                        x: block.x,
                        y: block.y,
                        width: block.width,
                        height: block.height,
                        radius: style::radius(),
                        paint: Paint::fill("node.fill").veiled(VEIL),
                    }];
                    shapes.extend(hatching(area));
                    overlay.under.push(Layer { shapes, within, placed });
                } else if reading.produced_an_error(index32, iters) {
                    let category = node
                        .errors_out
                        .iter()
                        .filter_map(|&wire| Some((trace.value(index32, wire, iters)?, node.wires.get(wire as usize)?)))
                        .find_map(|(json, wire)| short(json, &wire.ty).filter(|short| short.error));
                    let around = style::mark_around();
                    let mut shapes = vec![Shape::Rect {
                        x: block.x - around,
                        y: block.y - around,
                        width: block.width + 2 * around,
                        height: block.height + 2 * around,
                        radius: style::radius() + around,
                        paint: Paint::stroke("wire.error", style::mark_width()),
                    }];
                    if let Some(category) = category {
                        let text = Text::new(&category.text, size::any_text(), size::small());
                        let width = text.width + 2 * style::padding();
                        let height = style::pill_height();
                        let (x, y) = (block.x + block.width - width, block.y - around - height - style::probe_gap() / 2);
                        shapes.push(Shape::Rect {
                            x,
                            y,
                            width,
                            height,
                            radius: height / 2,
                            paint: Paint::both("probe.fill", "wire.error", style::outline()),
                        });
                        shapes.push(text.at(x + style::padding(), y + height / 2, "wire.error"));
                    }
                    overlay.under.push(Layer { shapes, within, placed });
                }
            }
        }
    }
}

/// Lines at 45 degrees across an area, `HATCH` apart, in `ink.muted`.
fn hatching((x0, y0, x1, y1): Area) -> Vec<Shape> {
    let mut lines = Vec::new();
    // The lines `y = c - x`, from the top-left corner to the bottom-right.
    let mut c = x0 + y0 + HATCH;
    while c < x1 + y1 {
        let from_x = (c - y1).max(x0);
        let to_x = (c - y0).min(x1);
        if from_x < to_x {
            lines.push(Shape::Poly { points: vec![(from_x, c - from_x), (to_x, c - to_x)], closed: false, paint: Paint::stroke("ink.muted", 75) });
        }
        c += HATCH;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_of_the_text_reads_as_json() {
        assert_eq!(literal("0.0").as_deref(), Some("0.0"));
        assert_eq!(literal("230 V").as_deref(), Some("\"230 V\""));
        assert_eq!(literal("-3 mV").as_deref(), Some("\"-3 mV\""));
        assert_eq!(literal("1_000").as_deref(), Some("1000"));
        assert_eq!(literal("\"SIM::DMM0\"").as_deref(), Some("\"SIM::DMM0\""));
        assert_eq!(literal("true").as_deref(), Some("true"));
        assert_eq!(literal("Verdict.Pass").as_deref(), Some("\"Verdict.Pass\""));
        assert_eq!(literal("[1, 2]"), None);
    }

    #[test]
    fn an_element_and_a_bound_value_are_read_from_their_json() {
        assert_eq!(element("[1.0, 2.0, 3.0]", 1), Some("2.0"));
        assert_eq!(element("[1.0]", 3), None);
        assert_eq!(element(r#"{"$len":100,"$head":[5,6]}"#, 1), Some("6"));
        assert_eq!(element(r#"{"$len":100,"$head":[5,6]}"#, 50), None);
        assert_eq!(element(r#"[1, {"$cut":true}]"#, 1), None);
        assert_eq!(bound(r#"{"Reading.Value":["230.0 V", 3]}"#, "Value", 1), Some("3"));
        assert_eq!(bound(r#"{"Reading.Value":["230.0 V"]}"#, "Other", 0), None);
    }

    #[test]
    fn hatching_stays_in_its_area() {
        let lines = hatching((0, 0, 1200, 600));
        assert!(!lines.is_empty());
        for line in lines {
            let Shape::Poly { points, .. } = line else { panic!("a line") };
            for (x, y) in points {
                assert!((0..=1200).contains(&x) && (0..=600).contains(&y), "{x}, {y}");
            }
        }
    }
}
