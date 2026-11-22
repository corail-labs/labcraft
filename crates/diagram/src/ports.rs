//! The terminals where a wire starts or ends, and the judgement of a wire on them (notation 2.5, 2.6): while a wire is
//! drawn, the canvas lights the terminals that would take it and says why the others refuse it, from the drawing alone,
//! as `wire.connect` would (commands.md 6.7); the command judges it again at the drop (architecture 12.6).

use std::collections::{HashMap, HashSet, VecDeque};

use labcraft_check::primitives::PrimitiveTy;
use labcraft_types::{Fit, FitKey, Ty, TyId, TypeTable, Unit};

use crate::graph::{self, Edge, Graph, Kind, StructureKind};
use crate::place::Placement;
use crate::shape::{Drawing, Holds, Port};
use crate::size;

/// The centres of the terminals of a node in the drawing, in hundredths: its inputs, its outputs, its `after:`.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeTerminals {
    pub inputs: Vec<(i64, i64)>,
    pub outputs: Vec<(i64, i64)>,
    pub order: Option<(i64, i64)>,
}

/// What `shapes` knows of a drawing that the ports need.
pub(crate) struct Context<'a, 'g> {
    pub graph: &'a Graph<'g>,
    pub placement: &'a Placement,
    /// By node of the graph.
    pub terminals: &'a [Option<NodeTerminals>],
    pub placed_of: &'a [Option<usize>],
    pub command_names: &'a [Option<String>],
    pub within: &'a dyn Fn(usize) -> Option<usize>,
    pub types: Option<&'a TypeTable>,
}

/// The ports of the drawing, the types they carry and which go to which, and the owner of each body.
pub(crate) fn build(context: &Context<'_, '_>, drawing: &mut Drawing) {
    let graph = context.graph;
    let mut ports: Vec<(Port, Option<(Key, String)>)> = Vec::new();
    // The notches, and the body of the value each stands for.
    let mut notches: Vec<(usize, usize)> = Vec::new();
    let style = |family: graph::Family, body: usize| {
        let depth = context.placement.frames.get(body).map_or(0, |frame| frame.depth);
        size::wire_style(family, crate::frame_fill(depth))
    };
    // The outputs of the function, which a bare name names (commands.md 9.2).
    let function_outputs: Vec<&str> = graph
        .bodies
        .first()
        .map(|function| {
            function.right.iter().filter(|terminal| terminal.edge == Edge::Output).map(|terminal| terminal.terminal.name.as_str()).collect()
        })
        .unwrap_or_default();
    let outputs = &function_outputs;
    for (index, node) in graph.nodes.iter().enumerate() {
        let (Some(Some(at)), Some(&Some(placed))) = (context.terminals.get(index), context.placed_of.get(index)) else { continue };
        let structure = matches!(node.kind, Kind::Structure(_));
        let name = context.command_names.get(index).cloned().flatten().unwrap_or_default();
        let unnamed = node.inputs.iter().filter(|input| input.name.is_empty()).count();
        let mut rank = 0;
        for (input, (terminal, &(x, y))) in node.inputs.iter().zip(&at.inputs).enumerate() {
            // A structure: its selector by the node alone, a carry or an `in` by its name (commands.md 9.2); an outer value
            // read inside is no place to wire.
            let to = if let Kind::Structure(found) = &node.kind {
                found.input_name(&name, input, &terminal.name, function_outputs.contains(&name.as_str())).unwrap_or_default()
            } else if name.is_empty() {
                String::new()
            } else if terminal.name.is_empty() {
                rank += 1;
                if unnamed == 1 { name.clone() } else { format!("{name}.{}", rank - 1) }
            } else {
                match terminal.place {
                    Some(place) => format!("{name}.{}.{place}", terminal.name),
                    None => format!("{name}.{}", terminal.name),
                }
            };
            let value = terminal.reads.first().map(|(_, read)| value_of(read)).unwrap_or_default();
            let holds = if !terminal.reads.is_empty() {
                Holds::Wire
            } else {
                match (&terminal.constant, &terminal.default) {
                    (Some(constant), _) if constant == "?" => Holds::Hole,
                    (Some(constant), _) => Holds::Constant(constant.clone()),
                    (None, Some(default)) => Holds::Default(default.clone()),
                    (None, None) => Holds::Missing,
                }
            };
            let mut never = String::new();
            let (mut meets, mut bound) = (false, None);
            let (key, family) = match terminal.expects {
                graph::Expects::Type => (Some(Key::Id(terminal.ty)), String::new()),
                graph::Expects::Fixed(ty) => (fixed(ty).map(Key::Fixed), String::new()),
                graph::Expects::Family(family) => (None, family_key(family)),
                graph::Expects::Variable { variable, bound: of_variable } => {
                    // Another input of the same variable that a wire reaches settles it; a literal takes the type
                    // that the call settles (`core.add(x, 1)` with `x` an `f64`).
                    let settled = node.inputs.iter().find(|other| {
                        !std::ptr::eq(*other, terminal)
                            && other.expects == graph::Expects::Variable { variable, bound: of_variable }
                            && !other.reads.is_empty()
                    });
                    match settled {
                        Some(other) => {
                            (meets, bound) = (true, of_variable);
                            (Some(Key::Id(other.ty)), String::new())
                        }
                        // A variable that only numbers can be: `Number` or `Integer`, not `Bits`, which holds `bool` too.
                        None if of_variable.is_some_and(|bound| !bound.holds(&labcraft_types::Ty::Bool)) => {
                            (None, family_key(graph::Family { base: graph::Base::Number, dimensions: 0 }))
                        }
                        None => (None, String::new()),
                    }
                }
                graph::Expects::Any => (None, String::new()),
                graph::Expects::Never(why) => {
                    never = why.to_owned();
                    (None, String::new())
                }
            };
            let port = Port {
                x,
                y,
                placed: Some(placed),
                within: (context.within)(node.body),
                body: node.body,
                input: true,
                order: false,
                frame: false,
                notch: false,
                family,
                never,
                name: to,
                caption: terminal.name.clone(),
                ty: None,
                meets,
                bound,
                literal: terminal.literal.clone().unwrap_or_default(),
                holds,
                value,
                group: None,
                style: style(terminal.family, node.body),
                arm: String::new(),
                part: String::new(),
                read: false,
                fixed: false,
            };
            ports.push((port, key.map(|key| (key, terminal.ty_text.clone()))));
        }
        for (rank, (terminal, &(x, y))) in node.outputs.iter().zip(&at.outputs).enumerate() {
            let value = node.values.get(rank).cloned().flatten();
            // The `next x` of a loop is no value that a wire leaves (step 5 of M1), nor an output with no name.
            let carried = value.as_deref().is_some_and(|value| value.starts_with("next."));
            let bound = value.filter(|value| is_plain(value));
            let from = match &bound {
                _ if carried => String::new(),
                Some(value) => value.clone(),
                None if !name.is_empty() && !terminal.name.is_empty() && terminal.name != "_" => format!("{name}.{}", terminal.name),
                None => String::new(),
            };
            let port = Port {
                x,
                y,
                placed: Some(placed),
                within: (context.within)(node.body),
                body: node.body,
                input: false,
                order: false,
                frame: false,
                notch: false,
                family: family_key(terminal.family),
                never: String::new(),
                name: from,
                caption: terminal.name.clone(),
                ty: None,
                meets: false,
                bound: None,
                literal: String::new(),
                holds: Holds::Nothing,
                value: bound.unwrap_or_default(),
                group: None,
                style: style(terminal.family, node.body),
                arm: String::new(),
                part: String::new(),
                read: false,
                fixed: false,
            };
            ports.push((port, Some((Key::Id(terminal.ty), terminal.ty_text.clone()))));
        }
        if let Some((x, y)) = at.order
            && !structure
        {
            let port = Port {
                x,
                y,
                placed: Some(placed),
                within: (context.within)(node.body),
                body: node.body,
                input: true,
                order: true,
                frame: false,
                notch: false,
                family: String::new(),
                never: String::new(),
                name: if name.is_empty() { String::new() } else { format!("{name}.after") },
                caption: "after".to_owned(),
                ty: None,
                meets: false,
                bound: None,
                literal: String::new(),
                holds: Holds::Nothing,
                value: String::new(),
                group: None,
                style: style(graph::Family::ORDER, node.body),
                arm: String::new(),
                part: String::new(),
                read: false,
                fixed: false,
            };
            ports.push((port, None));
        }
    }
    for (body, of) in graph.bodies.iter().enumerate() {
        let Some(frame) = context.placement.frames.get(body) else { continue };
        let placed = of.owner.and_then(|owner| context.placed_of.get(owner).copied().flatten());
        // The structure that holds the body, as a command names it.
        let owner_name = of.owner.and_then(|owner| context.command_names.get(owner).cloned().flatten()).unwrap_or_default();
        for (right, terminals, heights) in [(false, &of.left, &frame.left), (true, &of.right, &frame.right)] {
            let x = if right { frame.x + frame.width } else { frame.x };
            for (index, (terminal, &height)) in terminals.iter().zip(heights).enumerate() {
                let (input, name) = match (terminal.edge, right) {
                    (Edge::Output, true) if body == 0 => (true, format!("output {}", terminal.terminal.name)),
                    // What a body is given: an input of the function, a carry, an `in` clause, a value bound by a pattern,
                    // an outer value read inside.
                    (Edge::Input | Edge::Register | Edge::In | Edge::Range | Edge::Bound | Edge::Notch, false) => {
                        (false, terminal.terminal.name.clone())
                    }
                    // What a body gives its structure: `next x` of a carry, a `collect`, a `last` and a `yield` by the output they
                    // give, the `until` by the loop alone (commands.md 9.2).
                    (Edge::Register, true) if !owner_name.is_empty() => {
                        let carry = terminal.terminal.name.strip_prefix("next ").unwrap_or(&terminal.terminal.name);
                        (true, format!("{owner_name}.next.{carry}"))
                    }
                    (Edge::Until, true) if !owner_name.is_empty() => (true, format!("{owner_name}.until")),
                    // A name that is also an output of the function names that output (commands.md 9.2), and a `next x` that
                    // a structure binds is no value to name it by: the `yield` keeps its `arm`, the `collect` has no name.
                    (Edge::Yield, true) if is_plain(&terminal.terminal.name) && !terminal.terminal.name.contains(' ') => {
                        (true, terminal.terminal.name.clone())
                    }
                    (Edge::Collect | Edge::Last, true)
                        if is_plain(&terminal.terminal.name)
                            && !terminal.terminal.name.contains(' ')
                            && !outputs.contains(&terminal.terminal.name.as_str()) =>
                    {
                        (true, terminal.terminal.name.clone())
                    }
                    _ => (right, String::new()),
                };
                let arm = if right && terminal.edge == Edge::Yield && !name.is_empty() { of.traced.clone() } else { String::new() };
                // The part it stands for, as `structure.remove_part` names it.
                let caption = &terminal.terminal.name;
                let part = match (terminal.edge, right) {
                    (Edge::Register, false) if is_plain(caption) => format!("carry {caption}"),
                    (Edge::In | Edge::Range, false) if is_plain(caption) => format!("in {caption}"),
                    (Edge::Collect, true) if is_plain(caption) && !caption.contains(' ') => format!("collect {caption}"),
                    (Edge::Last, true) if is_plain(caption) && !caption.contains(' ') => format!("last {caption}"),
                    _ => String::new(),
                };
                let wire =
                    of.wires.iter().find(|wire| if right { wire.to == graph::End::Right(index) } else { wire.from == graph::End::Left(index) });
                let value = match (input, wire) {
                    (true, Some(wire)) => value_of(&wire.name),
                    (false, _) => terminal.terminal.name.clone(),
                    (true, None) => String::new(),
                };
                let holds = match (input, wire) {
                    (true, Some(_)) => Holds::Wire,
                    (true, None) => Holds::Missing,
                    (false, _) => Holds::Nothing,
                };
                // An outer value read inside is the value of the body around (its notch shows where it crosses).
                let bound_in = match terminal.edge {
                    Edge::Notch => of.owner.and_then(|owner| graph.nodes.get(owner)).map_or(body, |owner| owner.body),
                    _ => body,
                };
                let port = Port {
                    x,
                    y: frame.y + height,
                    placed,
                    within: (context.within)(body),
                    body,
                    input,
                    order: false,
                    frame: true,
                    notch: terminal.edge == Edge::Notch,
                    family: family_key(terminal.terminal.family),
                    never: String::new(),
                    name,
                    caption: terminal.terminal.name.clone(),
                    ty: None,
                    meets: false,
                    bound: None,
                    literal: String::new(),
                    holds,
                    value,
                    group: None,
                    style: style(terminal.terminal.family, body),
                    arm,
                    part,
                    read: !input && wire.is_some(),
                    fixed: body == 0
                        && if right { graph.terminals_at.right.contains_key(&index) } else { graph.terminals_at.left.contains_key(&index) },
                };
                if bound_in != body {
                    notches.push((ports.len(), bound_in));
                }
                ports.push((port, Some((Key::Id(terminal.terminal.ty), terminal.terminal.ty_text.clone()))));
            }
        }
    }
    // A name that two values of one body bear names neither (`edit.ambiguous`): an output of a node is then named after its
    // node, and a terminal of the frame of a structure not at all. Reading a name that is an input of the function and an
    // output reads the input (R-2): the input keeps it, and a node that binds the output is named after its node.
    let mut counts: HashMap<(usize, String), usize> = HashMap::new();
    for (port, _) in ports.iter().filter(|(port, _)| !port.input && !port.name.is_empty()) {
        *counts.entry((port.body, port.name.clone())).or_insert(0) += 1;
    }
    let inputs: HashSet<String> =
        ports.iter().filter(|(port, _)| port.frame && !port.input && port.placed.is_none()).map(|(port, _)| port.name.clone()).collect();
    for (port, _) in ports.iter_mut().filter(|(port, _)| !port.input && !port.name.is_empty()) {
        if port.frame && port.placed.is_none() {
            continue;
        }
        if port.body == 0 && inputs.contains(&port.name) {
            counts.insert((port.body, port.name.clone()), 2);
        }
        if counts.get(&(port.body, port.name.clone())).is_some_and(|&count| count > 1) {
            let node = port.placed.and_then(|placed| drawing.placed.get(placed)).filter(|node| node.body == port.body);
            port.name = match node {
                Some(node) if !node.name.is_empty() && !port.caption.is_empty() && port.caption != "_" => format!("{}.{}", node.name, port.caption),
                _ => String::new(),
            };
        }
    }
    // A notch of a notch reads a value from further out: its body is the one that binds it.
    let notched: HashSet<(usize, String)> =
        notches.iter().filter_map(|&(index, _)| ports.get(index)).map(|(port, _)| (port.body, port.name.clone())).collect();
    for (index, mut body) in notches {
        let Some(name) = ports.get(index).map(|(port, _)| port.name.clone()) else { continue };
        let mut steps = 0;
        while notched.contains(&(body, name.clone())) && steps <= graph.bodies.len() {
            let owner = graph.bodies.get(body).and_then(|of| of.owner).and_then(|owner| graph.nodes.get(owner));
            let Some(owner) = owner else { break };
            body = owner.body;
            steps += 1;
        }
        if let Some((port, _)) = ports.get_mut(index) {
            port.body = body;
        }
    }
    // The wire at each port: that of its value in its body.
    let mut groups: HashMap<(usize, &str), usize> = HashMap::new();
    for (index, group) in drawing.groups.iter().enumerate() {
        if let Some(wire) = &group.wire {
            groups.entry((wire.body, wire.value.as_str())).or_insert(index);
        }
    }
    let found: Vec<Option<usize>> =
        ports.iter().map(|(port, _)| (!port.value.is_empty()).then(|| groups.get(&(port.body, port.value.as_str())).copied()).flatten()).collect();
    // Each type once; what `check` does not know has none, and takes anything.
    let mut keys: Vec<FitKey> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut class_of: HashMap<(FitKey, String), usize> = HashMap::new();
    let mut out = Vec::with_capacity(ports.len());
    for ((mut port, ty), group) in ports.into_iter().zip(found) {
        port.group = group;
        let key = match (ty, context.types) {
            (Some((Key::Id(ty), text)), Some(types)) if !types.has_error(ty) => Some((types.fit_key(ty), text)),
            (Some((Key::Fixed(ty), text)), Some(_)) => FitKey::of(&ty).map(|key| (key, text)),
            _ => None,
        };
        port.ty = key.map(|(key, text)| {
            *class_of.entry((key, text.clone())).or_insert_with(|| {
                keys.push(key);
                texts.push(text);
                keys.len() - 1
            })
        });
        out.push(port);
    }
    drawing.type_keys = keys;
    drawing.types = texts;
    drawing.ports = out;
}

/// A type that a port carries or takes: one of the table, or a parameter of a primitive.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Id(TyId),
    Fixed(Ty),
}

/// The type of a parameter of a primitive of a fixed type (appendix A): a number without a unit for `f64`.
fn fixed(ty: PrimitiveTy) -> Option<Ty> {
    if let Some(kind) = ty.number_kind() {
        return Some(Ty::Number { kind, unit: Unit::ONE });
    }
    Some(match ty {
        PrimitiveTy::Bool => Ty::Bool,
        PrimitiveTy::Text => Ty::String,
        PrimitiveTy::Bytes => Ty::Bytes,
        PrimitiveTy::Timestamp => Ty::Timestamp,
        PrimitiveTy::Duration => Ty::Duration,
        PrimitiveTy::Error => Ty::ErrorValue,
        _ => return None,
    })
}

/// A family as the judgement compares it; empty for one that a type variable or an error leaves unknown.
fn family_key(family: graph::Family) -> String {
    match family.base {
        graph::Base::Unknown if family.dimensions == 0 => String::new(),
        base => format!("{base:?}/{}", family.dimensions),
    }
}

/// Whether a value of the family `found` goes to an input that takes the family `expected`: the same family, or for an
/// array of a type variable (`Unknown/1`), an array of as many dimensions.
fn family_fits(found: &str, expected: &str) -> bool {
    match expected.strip_prefix("Unknown/") {
        Some(dimensions) => found.rsplit_once('/').is_some_and(|(_, found)| found == dimensions),
        None => found == expected,
    }
}

/// A family as a message says what an input takes: `a number`, `bool`.
fn family_text(key: &str) -> String {
    let (base, dimensions) = key.split_once('/').unwrap_or((key, "0"));
    if dimensions != "0" {
        return "an array".to_owned();
    }
    match base {
        "Number" => "a number",
        "Bool" => "bool",
        "Text" => "string",
        "Error" => "error",
        "Session" => "a session",
        "Function" => "a function",
        _ => "a value of another family",
    }
    .to_owned()
}

/// The value that a wire bears, without the field it reads: `step` for `step.low`.
fn value_of(name: &str) -> String {
    name.split('.').next().unwrap_or_default().to_owned()
}

/// A name that a body binds, which `from` takes as it is: not `_`, not `next.x`.
fn is_plain(value: &str) -> bool {
    !value.is_empty() && value != "_" && !value.contains('.') && !value.starts_with('@')
}

/// Why a wire is refused, as `wire.connect` would say it (commands.md 3, 6.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// `wire.type_mismatch`, `wire.cycle`, `wire.out_of_scope`, `wire.occupied`, or `params.invalid` for what a later
    /// step of M1 edits.
    pub code: &'static str,
    pub message: String,
}

/// What a terminal would do with a wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It takes it; `replaces` when it holds a wire or a constant that the wire replaces (notation 2.4). `exits`: the
    /// bodies of the loops that the wire leaves, from the inside out, each of which takes the value out (commands.md 6.7,
    /// `through`); the type is then judged by `wire.connect`, after the passages.
    Takes { replaces: bool, exits: Vec<usize> },
    /// It refuses it. A wire of the wrong type alone is `wrong_type`: the setting "accept it" writes it anyway.
    Refused { refusal: Refusal, wrong_type: bool },
}

/// What each port would do with a wire drawn from `from`, by its index in [`Drawing::ports`]: none for a port that is
/// no end of such a wire (the same direction, or one that no command wires yet). From an input, the wire goes back to an
/// output: each output is judged as the start of a wire to `from`.
pub fn wiring(drawing: &Drawing, from: usize) -> Vec<Option<Verdict>> {
    let Some(start) = drawing.ports.get(from) else { return vec![None; drawing.ports.len()] };
    let graph = Dependencies::of(drawing);
    let values = Values::of(drawing);
    let made_by = |source: &Port| values.made_by(source);
    let ahead = if start.input { None } else { made_by(start).map(|placed| graph.reaching(placed)) };
    let behind = if start.input { start.placed.map(|placed| graph.reached(drawing, placed)) } else { None };
    // What each structure reads, deep, once each.
    let mut upstream: HashMap<usize, HashSet<usize>> = HashMap::new();
    drawing
        .ports
        .iter()
        .enumerate()
        .map(|(index, port)| {
            if index == from || port.input == start.input || port.name.is_empty() || start.name.is_empty() {
                return None;
            }
            let (source, target) = if start.input { (port, start) } else { (start, port) };
            let cycle = match (made_by(source), target.placed) {
                (Some(made_by), Some(reader)) => {
                    let holder = lifted(drawing, reader, drawing.placed.get(made_by).map_or(source.body, |node| node.body));
                    match (&ahead, &behind, holder) {
                        (_, _, Some(holder)) if holder == made_by => true,
                        (Some(reaching), _, Some(holder)) => reaching.contains(&holder),
                        (_, Some(reached), Some(_)) => reached.contains(&made_by),
                        // A part inside a structure (`next`, `until`, `collect`, `yield`) is read by its structure: no reader
                        // outside for a value inside it, but a value from outside, out of a structure beside it, closes a
                        // cycle when that structure reads, deep, what the structure of the part makes (commands.md 6.7).
                        _ if target.frame => target.placed.is_some_and(|structure| {
                            lifted(drawing, made_by, target.body).is_none() && lifted_cycle(drawing, &graph, &mut upstream, made_by, structure)
                        }),
                        // A value taken out of an arm (commands.md 6.7): the structures around it give it, in the body
                        // that both see, so the reader there must not feed the outermost of them.
                        _ => lifted_cycle(drawing, &graph, &mut upstream, made_by, reader),
                    }
                }
                // A carry or a clause taken out of its loop (commands.md 6.7, `through`): the loop gives it, so a reader
                // outside it that feeds the loop closes a cycle.
                (None, Some(reader)) if source.frame && !source.input => match drawing.bodies.get(source.body).and_then(|body| body.owner) {
                    // A part of a structure (a `yield`) is read in the body that holds it: there, the loop is no reader.
                    Some(owner) if target.frame && lifted(drawing, owner, target.body).is_some() => false,
                    Some(owner) if lifted(drawing, reader, drawing.placed.get(owner).map_or(source.body, |node| node.body)) != Some(owner) => {
                        lifted_cycle(drawing, &graph, &mut upstream, owner, reader)
                    }
                    _ => false,
                },
                _ => false,
            };
            Some(judge(drawing, &values, source, target, cycle))
        })
        .collect()
}

/// The values of a drawing that a name gives in each body: the ports where a wire starts, notches left out, by body and
/// name.
struct Values<'a> {
    by_name: HashMap<(usize, &'a str), Vec<&'a Port>>,
}

impl<'a> Values<'a> {
    fn of(drawing: &'a Drawing) -> Values<'a> {
        let mut by_name: HashMap<(usize, &str), Vec<&Port>> = HashMap::new();
        for port in drawing.ports.iter().filter(|port| !port.input && !port.notch && !port.name.is_empty()) {
            by_name.entry((port.body, port.name.as_str())).or_default().push(port);
        }
        Values { by_name }
    }

    /// The node that makes the value a wire from `source` carries, by its index in [`Drawing::placed`]: its own node,
    /// or for an outer value read inside a structure, the node of that value; none for an input of the function, a
    /// carry, an `in` clause or a pattern, from which no wire closes a cycle.
    fn made_by(&self, source: &Port) -> Option<usize> {
        if !source.frame {
            return source.placed;
        }
        self.by_name.get(&(source.body, source.name.as_str()))?.iter().find(|port| !port.frame).and_then(|port| port.placed)
    }

    /// Whether `body` binds the name for a value other than `source` (R-3).
    fn hides(&self, body: usize, name: &str, source: &Port) -> bool {
        self.by_name.get(&(body, name)).is_some_and(|ports| ports.iter().any(|port| !std::ptr::eq(*port, source)))
    }
}

/// A wire from `source` to `target`, judged in the order of `wire.connect`: the scope, the cycle, what the input holds,
/// the type.
fn judge(drawing: &Drawing, values: &Values<'_>, source: &Port, target: &Port, cycle: bool) -> Verdict {
    let refused = |code: &'static str, message: String| Verdict::Refused { refusal: Refusal { code, message }, wrong_type: false };
    if !target.never.is_empty() {
        return refused("params.invalid", target.never.clone());
    }
    let exits = match out_of_scope(drawing, values, source, target) {
        Ok(exits) => exits,
        Err(refusal) => return Verdict::Refused { refusal, wrong_type: false },
    };
    // An order wire does not leave a structure (R-17).
    if target.order && !exits.is_empty() {
        return refused("wire.out_of_scope", format!("`{}` is inside a loop: an order wire does not leave a structure (R-17)", source.name));
    }
    if cycle {
        return refused("wire.cycle", "it would close a cycle: a value comes back only through a carry (spec 6.2)".to_owned());
    }
    let replaces = matches!(target.holds, Holds::Wire | Holds::Constant(_)) && !target.order;
    // Out of loops, the type is that of the passages, which the editor chooses: `passage_refusal` judges it then.
    if !exits.is_empty() {
        return Verdict::Takes { replaces, exits };
    }
    if let Some(refusal) = passage_refusal(drawing, source, target, 0) {
        return Verdict::Refused { refusal, wrong_type: true };
    }
    Verdict::Takes { replaces, exits }
}

/// Whether a value of `source`, put in an array `collects` times by the loops it leaves (a carry keeps its type, spec
/// 6.2), is of a type that `target` does not take: the refusal `wire.connect` gives, `wire.type_mismatch`.
pub fn passage_refusal(drawing: &Drawing, source: &Port, target: &Port, collects: usize) -> Option<Refusal> {
    let text = |ty: Option<usize>| ty.and_then(|ty| drawing.types.get(ty)).cloned().unwrap_or_default();
    let found = text(source.ty);
    let arrayed = |inner: &str| format!("{}{inner}{}", "[".repeat(collects), "]".repeat(collects));
    let wrong = match (source.ty, target.ty) {
        _ if target.order => None,
        // In an array, a wire keeps its type (spec 6.2): an `[i32]` is no `[f64]`.
        (Some(_), Some(expected)) if collects > 0 => (arrayed(&found) != text(Some(expected))).then(|| text(Some(expected))),
        (Some(found), Some(expected)) => {
            let fits = match (drawing.type_keys.get(found), drawing.type_keys.get(expected)) {
                // Two inputs of one type variable meet (spec 3.4).
                (Some(found), Some(expected)) if target.meets => found.meets(expected, target.bound),
                (Some(found), Some(expected)) => matches!(found.fit(expected), Fit::Same | Fit::Coerce),
                _ => true,
            };
            (!fits).then(|| text(Some(expected)))
        }
        // A parameter of a primitive takes one family, whatever its unit.
        (_, None) if !target.family.is_empty() && !source.family.is_empty() => {
            let family = match source.family.rsplit_once('/') {
                Some((base, dimensions)) if collects > 0 => {
                    format!("{base}/{}", dimensions.parse::<usize>().unwrap_or_default().saturating_add(collects))
                }
                _ => source.family.clone(),
            };
            (!family_fits(&family, &target.family)).then(|| family_text(&target.family))
        }
        _ => None,
    };
    let expected = wrong?;
    let found = if collects > 0 && !found.is_empty() { arrayed(&found) } else { found };
    let message =
        if found.is_empty() { format!("the value does not go to `{expected}`") } else { format!("`{found}` where `{expected}` is expected") };
    Some(Refusal { code: "wire.type_mismatch", message })
}

/// What the input holds, for the refusal of a drop that would replace it without the modifier (notation 2.4).
pub fn occupied(port: &Port) -> Refusal {
    let held = match &port.holds {
        Holds::Constant(constant) => constant.clone(),
        _ => port.value.clone(),
    };
    Refusal { code: "wire.occupied", message: format!("`{}` already holds `{held}`: hold Alt to replace it", port.caption_or_name()) }
}

impl Port {
    /// How a message names it: its name in its node, else the name a command gives it.
    pub fn caption_or_name(&self) -> &str {
        if self.caption.is_empty() { &self.name } else { &self.caption }
    }
}

/// A value the reader cannot see: of another arm or structure (commands.md 6.7). Otherwise the loops that the wire leaves,
/// from the inside out, each of which takes the value out (`through`); none for a value that the reader sees.
fn out_of_scope(drawing: &Drawing, values: &Values<'_>, source: &Port, target: &Port) -> Result<Vec<usize>, Refusal> {
    let chain = |body: usize| {
        let mut bodies = vec![body];
        let mut at = body;
        while let Some(owner) = drawing.bodies.get(at).and_then(|found| found.owner)
            && let Some(node) = drawing.placed.get(owner)
            && !bodies.contains(&node.body)
        {
            bodies.push(node.body);
            at = node.body;
        }
        bodies
    };
    let value = &source.name;
    let of_target = chain(target.body);
    if let Some(at) = of_target.iter().position(|&body| body == source.body) {
        // A body between the two that binds the name for itself hides the value (R-3): a carry, a clause, a pattern, a
        // line.
        let hidden = of_target.iter().take(at).any(|&body| values.hides(body, value, source));
        if hidden {
            return Err(Refusal {
                code: "wire.out_of_scope",
                message: format!("`{value}` cannot be seen from there: a value of the same name hides it (R-3)"),
            });
        }
        return Ok(Vec::new());
    }
    let of_source = chain(source.body);
    // Deeper than the reader, or in a structure beside the one that holds it, through arms only: each structure between
    // the value and the body that both see gives it as a new output (commands.md 6.7).
    if let Some(at) = of_source.iter().position(|body| of_target.contains(body))
        && at > 0
    {
        // Out of a loop, each loop takes the value out (decision of the owner, 2026-11-18).
        let exits: Vec<usize> =
            of_source.iter().take(at).copied().filter(|&body| drawing.bodies.get(body).is_some_and(|found| found.looping)).collect();
        // Out to the structure that holds it, or into one of its other arms: the structure would read its own output.
        let lifted: Vec<Option<usize>> = of_source.iter().take(at).map(|&body| drawing.bodies.get(body).and_then(|found| found.owner)).collect();
        let reads_it = |body: &usize| lifted.contains(&drawing.bodies.get(*body).and_then(|found| found.owner));
        let own = (lifted.contains(&target.placed) && !target.frame) || of_target.iter().any(reads_it);
        if own {
            return Err(Refusal {
                code: "wire.cycle",
                message: format!(
                    "a wire from `{value}` to `{}` would close a cycle: a value comes back only through a carry (spec 6.2)",
                    target.name
                ),
            });
        }
        return Ok(exits);
    }
    Err(Refusal { code: "wire.out_of_scope", message: format!("`{value}` cannot be seen from there: it is a value of another arm or structure") })
}

/// Whether a wire from `made_by`, deeper than `reader`, closes a cycle once the structures around `made_by` give its value
/// in the body where `reader`, or the structure that holds it, is: that structure reads, deep, what `reader` makes.
fn lifted_cycle(drawing: &Drawing, graph: &Dependencies, upstream: &mut HashMap<usize, HashSet<usize>>, made_by: usize, reader: usize) -> bool {
    let mut body = drawing.placed.get(reader).map(|node| node.body);
    let mut seen = 0;
    while let Some(at) = body {
        if let Some(outer) = lifted(drawing, made_by, at) {
            let Some(holder) = lifted(drawing, reader, at) else { return false };
            return holder == outer || upstream.entry(outer).or_insert_with(|| graph.reaching(outer)).contains(&holder);
        }
        body = drawing.bodies.get(at).and_then(|found| found.owner).and_then(|owner| drawing.placed.get(owner)).map(|node| node.body);
        seen += 1;
        if seen > drawing.bodies.len() {
            return false;
        }
    }
    false
}

/// The node of the body `body` that holds `placed`: itself, or the structure around it there.
fn lifted(drawing: &Drawing, placed: usize, body: usize) -> Option<usize> {
    let mut at = placed;
    let mut seen = 0;
    loop {
        let node = drawing.placed.get(at)?;
        if node.body == body {
            return Some(at);
        }
        at = drawing.bodies.get(node.body)?.owner?;
        seen += 1;
        if seen > drawing.placed.len() {
            return None;
        }
    }
}

/// What depends on what, between the nodes of one body, a structure standing for what it holds: the wires of the
/// drawing, order wires included.
struct Dependencies {
    /// By node, the nodes of its body that read it.
    readers: Vec<Vec<usize>>,
    /// By node, the nodes of its body that it reads.
    sources: Vec<Vec<usize>>,
}

impl Dependencies {
    fn of(drawing: &Drawing) -> Dependencies {
        let count = drawing.placed.len();
        let keys: HashMap<&str, usize> = drawing.placed.iter().enumerate().map(|(at, placed)| (placed.key.as_str(), at)).collect();
        let mut readers = vec![Vec::new(); count];
        let mut sources = vec![Vec::new(); count];
        for wire in drawing.groups.iter().filter_map(|group| group.wire.as_ref()) {
            let Some(&from) = wire.from_node.as_deref().and_then(|key| keys.get(key)) else { continue };
            let Some(body) = drawing.placed.get(from).map(|placed| placed.body) else { continue };
            for to in wire.to_nodes.iter().filter_map(|key| keys.get(key.as_str())) {
                if let Some(to) = lifted(drawing, *to, body).filter(|&to| to != from) {
                    if let Some(list) = readers.get_mut(from) {
                        list.push(to);
                    }
                    if let Some(list) = sources.get_mut(to) {
                        list.push(from);
                    }
                }
            }
        }
        Dependencies { readers, sources }
    }

    /// The nodes from which `node` is reached, itself included: a wire from `node` into one of them closes a cycle.
    fn reaching(&self, node: usize) -> HashSet<usize> {
        walk(&self.sources, [node])
    }

    /// The nodes that `node` reaches, and those that each structure around it reaches, themselves included: a wire into
    /// `node` from one of them closes a cycle.
    fn reached(&self, drawing: &Drawing, node: usize) -> HashSet<usize> {
        let mut starts = vec![node];
        let mut at = node;
        while let Some(owner) = drawing.placed.get(at).and_then(|placed| drawing.bodies.get(placed.body)).and_then(|body| body.owner)
            && !starts.contains(&owner)
        {
            starts.push(owner);
            at = owner;
        }
        walk(&self.readers, starts)
    }
}

fn walk(next: &[Vec<usize>], starts: impl IntoIterator<Item = usize>) -> HashSet<usize> {
    let mut seen = HashSet::new();
    let mut pending: VecDeque<usize> = starts.into_iter().collect();
    while let Some(at) = pending.pop_front() {
        if seen.insert(at) {
            pending.extend(next.get(at).into_iter().flatten().copied());
        }
    }
    seen
}

/// The owner of each body, whether it loops, and the patterns of an arm, for [`crate::BodyDrawn`].
pub(crate) fn owners(graph: &Graph<'_>, placed_of: &[Option<usize>]) -> Vec<Owner> {
    graph
        .bodies
        .iter()
        .map(|body| {
            let owner = body.owner.and_then(|owner| placed_of.get(owner).copied().flatten());
            let structure = body.owner.and_then(|owner| graph.nodes.get(owner)).and_then(|node| match &node.kind {
                Kind::Structure(structure) => Some(structure),
                _ => None,
            });
            let looping = structure.is_some_and(|structure| matches!(structure.kind, StructureKind::Loop | StructureKind::For));
            let each = structure.is_some_and(|structure| structure.kind == StructureKind::For);
            let parallel = structure.is_some_and(|structure| structure.parallel);
            let continue_on_error = structure.is_some_and(|structure| structure.kind == StructureKind::Loop && structure.badge.is_some());
            let arm = if body.arm.is_some() { body.traced.clone() } else { String::new() };
            Owner { owner, looping, each, parallel, continue_on_error, arm }
        })
        .collect()
}

/// What holds a body, for [`crate::BodyDrawn`].
pub(crate) struct Owner {
    pub owner: Option<usize>,
    pub looping: bool,
    pub each: bool,
    pub parallel: bool,
    pub continue_on_error: bool,
    pub arm: String,
}
