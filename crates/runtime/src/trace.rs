//! The trace of a run (`labcraft-trace/1`, `docs/commands.md`, 8), written as it runs through the writer of `trace`: an
//! [`Observer`] of the run that turns what it hears into events (`docs/architecture.md`, 10.7). Where the trace goes is the
//! caller's: a file under `.labcraft/runs/`, a buffer, nothing.
//!
//! What a frame binds, by name and type, and the text of the patterns of its arms are found once per function, by walks
//! that keep their own stack, however deep its bodies nest.

use std::collections::{HashMap, HashSet};
use std::io;
use std::rc::Rc;

use labcraft_check::CheckedFile;
use labcraft_graph::{Body as Source, Id, Rhs, StatementKind};
use labcraft_ir::{Body, ElementId, FunctionId, Node, NodeKind, Region};
use labcraft_trace::{Caller as TraceCaller, Event, Header, Input, TraceWriter, WriteLimits, WriteValue};
use labcraft_types::{Name, TyId};

use crate::convert::Env;
use crate::json::{Describe, Json};
use crate::run::{Caller, Observer, Outputs, RunEnd, SetBy, Status};
use crate::value::Value;

/// The text of a pattern is never cut, so that two arms never write the same: a line of the trace has its own bound.
const PATTERN_TEXT: usize = usize::MAX;

/// What the trace says of one function, found the first time one of its frames begins.
struct Bound {
    /// The `types` of its `call` events, in the order of the lines: each value its body binds, by name.
    types: Vec<(String, String)>,
    /// The names bound with two types, which `types` leaves out: each event that binds one gives its type.
    several: HashSet<Name>,
    /// The text of each arm of its `match` and `handle`, by the id of the arm.
    arms: HashMap<Id, String>,
    /// The loops around each node in a loop, by its statement: the name of each, outermost first, one for each level of
    /// its `iter`, so that two nodes of one name in two loops are told apart (commands.md 8).
    loops: HashMap<Id, Rc<[String]>>,
}

/// An [`Observer`] that writes the trace of the run to `sink`. A sink that fails stops the trace, never the run.
pub struct TraceObserver<'a, W: io::Write> {
    writer: TraceWriter<W>,
    env: Env<'a>,
    files: &'a [CheckedFile],
    /// The function of each frame, by its number.
    frames: Vec<FunctionId>,
    functions: HashMap<FunctionId, Bound>,
}

/// What a trace became when its run ended: what the report says of it (commands.md 7, `trace`).
pub struct Traced<W> {
    pub sink: W,
    pub events: u64,
    pub truncated: bool,
    pub error: Option<io::Error>,
}

impl<'a, W: io::Write> TraceObserver<'a, W> {
    /// Writes the first line of the trace. `files` are those of the `Checked` the program was lowered from.
    pub fn new(sink: W, header: &Header<'_>, env: Env<'a>, files: &'a [CheckedFile], limits: WriteLimits) -> Self {
        TraceObserver { writer: TraceWriter::with_limits(sink, header, limits), env, files, frames: Vec::new(), functions: HashMap::new() }
    }

    /// Writes the last line, `end`, with the status and the clock of the run.
    pub fn finish(mut self, end: &RunEnd) -> Traced<W> {
        self.writer.end(nanos(end.clock), status_text(end.status));
        self.writer.flush();
        let events = self.writer.events();
        let truncated = self.writer.truncated();
        let error = self.writer.error().map(|error| io::Error::new(error.kind(), error.to_string()));
        Traced { sink: self.writer.into_inner(), events, truncated, error }
    }

    fn function_of(&self, frame: u64) -> FunctionId {
        usize::try_from(frame).ok().and_then(|frame| self.frames.get(frame)).copied().unwrap_or(FunctionId(0))
    }

    /// What the trace says of the function, found the first time it is asked for.
    fn bound(functions: &mut HashMap<FunctionId, Bound>, env: Env<'_>, files: &[CheckedFile], function: FunctionId) {
        functions.entry(function).or_insert_with(|| bound_of(env, files, function));
    }

    /// The values a node bound, by the name of each, and the types of those whose name the frame binds with two types.
    fn node_event(&mut self, at: i64, frame: u64, node: &Node, iter: &[u64], ended: Outputs<'_>, handles: &dyn Describe) {
        let (outputs, skipped) = (ended.values, ended.skipped);
        let function = self.function_of(frame);
        Self::bound(&mut self.functions, self.env, self.files, function);
        let empty = HashSet::new();
        let several = self.functions.get(&function).map_or(&empty, |bound| &bound.several);
        let Some(json) = json_of(self.env, function, handles) else { return };
        let names = self.env.names;
        let mut named: Vec<(&str, TyId, &Value)> = Vec::new();
        for (port, value) in node.outputs.iter().zip(outputs) {
            if let Some(name) = port.names.first() {
                named.push((names.resolve(*name), port.ty, value));
            }
        }
        let typed: Vec<_> = named.iter().map(|(name, ty, value)| (*name, json.typed(value, *ty))).collect();
        let out: Vec<(&str, &dyn WriteValue)> = typed.iter().map(|(name, value)| (*name, value as &dyn WriteValue)).collect();
        let own: Vec<(String, String)> = node
            .outputs
            .iter()
            .filter_map(|port| {
                port.names.first().filter(|name| several.contains(name)).map(|name| (names.resolve(*name).to_owned(), json.type_text(port.ty)))
            })
            .collect();
        let own: Vec<(&str, &str)> = own.iter().map(|(name, ty)| (name.as_str(), ty.as_str())).collect();
        let loops: Vec<&str> = loops_of(&self.functions, function, node);
        let node = names.resolve(node.name);
        let event = if skipped {
            Event::Skipped { frame, node, loops: &loops, iter, out: &out, types: &own }
        } else {
            Event::Ran { frame, node, loops: &loops, iter, out: &out, types: &own }
        };
        self.writer.event(nanos(at), &event);
    }
}

impl<W: io::Write> Observer for TraceObserver<'_, W> {
    fn call(&mut self, at: i64, frame: u64, caller: Option<Caller<'_>>, function: FunctionId, inputs: &[Value], handles: &dyn Describe) {
        if let Ok(index) = usize::try_from(frame) {
            if self.frames.len() <= index {
                self.frames.resize(index + 1, FunctionId(0));
            }
            if let Some(slot) = self.frames.get_mut(index) {
                *slot = function;
            }
        }
        let Some(declared) = self.env.program.functions.get(function.0 as usize) else { return };
        Self::bound(&mut self.functions, self.env, self.files, function);
        let types: Vec<(&str, &str)> = self
            .functions
            .get(&function)
            .map(|bound| bound.types.iter().map(|(name, ty)| (name.as_str(), ty.as_str())).collect())
            .unwrap_or_default();
        let Some(json) = json_of(self.env, function, handles) else { return };
        let names = self.env.names;
        let shown: Vec<(&str, String, Option<_>)> = declared
            .inputs
            .iter()
            .zip(inputs)
            .map(|(port, value)| (names.resolve(port.name), json.type_text(port.ty), Some(json.typed(value, port.ty))))
            .collect();
        let inputs: Vec<Input<'_>> =
            shown.iter().filter_map(|(name, ty, value)| value.as_ref().map(|value| Input { name, ty, value: value as &dyn WriteValue })).collect();
        let caller_node = caller.as_ref().map(|caller| names.resolve(caller.node.name));
        let caller_loops: Vec<&str> = match &caller {
            Some(caller) => {
                let function = self.function_of(caller.frame);
                loops_of(&self.functions, function, caller.node)
            }
            None => Vec::new(),
        };
        let caller = match (caller, caller_node) {
            (Some(caller), Some(node)) => Some(TraceCaller { frame: caller.frame, node, loops: &caller_loops, iter: caller.iter }),
            _ => None,
        };
        self.writer.event(nanos(at), &Event::Call { frame, caller, function: &declared.shown, inputs: &inputs, types: &types });
    }

    fn ran(&mut self, at: i64, frame: u64, node: &Node, iter: &[u64], outputs: Outputs<'_>, handles: &dyn Describe) {
        self.node_event(at, frame, node, iter, outputs, handles);
    }

    fn arm(&mut self, at: i64, frame: u64, node: &Node, iter: &[u64], arm: usize) {
        let origin = match &node.kind {
            NodeKind::Match { arms } | NodeKind::Handle { arms } => arms.get(arm).map(|arm| arm.origin),
            _ => None,
        };
        let function = self.function_of(frame);
        Self::bound(&mut self.functions, self.env, self.files, function);
        let text = origin.and_then(|origin| self.functions.get(&function).and_then(|bound| bound.arms.get(&origin))).map_or("_", String::as_str);
        let loops: Vec<&str> = loops_of(&self.functions, function, node);
        let node = self.env.names.resolve(node.name);
        self.writer.event(nanos(at), &Event::Arm { frame, node, loops: &loops, iter, arm: text });
    }

    fn set(&mut self, at: i64, frame: u64, element: ElementId, value: &Value, by: SetBy, handles: &dyn Describe) {
        let function = self.function_of(frame);
        let Some(declared) = self.env.program.functions.get(function.0 as usize).and_then(|function| function.elements.get(element.0 as usize))
        else {
            return;
        };
        let Some(json) = json_of(self.env, function, handles) else { return };
        let typed = json.typed(value, declared.ty.unwrap_or(TyId::ERROR));
        let by = match by {
            SetBy::Program => "ui.set",
            SetBy::Set => "set",
            SetBy::RunSetElement => "run.set_element",
        };
        let element = self.env.names.resolve(declared.name);
        self.writer.event(nanos(at), &Event::Set { frame, element, value: &typed, by });
    }
}

/// The status of a run, as the report and the `end` of a trace write it.
pub fn status_text(status: Status) -> &'static str {
    match status {
        Status::Ok => "ok",
        Status::Error => "error",
        Status::Aborted => "aborted",
        Status::Fault => "fault",
    }
}

/// What writes the values of a frame of `function`: its types are shown from its library.
fn json_of<'b>(env: Env<'b>, function: FunctionId, handles: &'b dyn Describe) -> Option<Json<'b>> {
    let declared = env.program.functions.get(function.0 as usize).or_else(|| env.program.functions.first())?;
    Some(Json { env, viewer: declared.library, handles })
}

/// The loops around a node, by name, outermost first: none when it is in no loop.
fn loops_of<'b>(functions: &'b HashMap<FunctionId, Bound>, function: FunctionId, node: &Node) -> Vec<&'b str> {
    functions
        .get(&function)
        .and_then(|bound| bound.loops.get(&node.origin))
        .map(|loops| loops.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

fn nanos(at: i64) -> u64 {
    u64::try_from(at).unwrap_or(0)
}

fn bound_of(env: Env<'_>, files: &[CheckedFile], function: FunctionId) -> Bound {
    let mut bound = Bound { types: Vec::new(), several: HashSet::new(), arms: HashMap::new(), loops: HashMap::new() };
    let Some(declared) = env.program.functions.get(function.0 as usize) else { return bound };
    let Body::Region(region) = &declared.body else { return bound };
    let viewer = declared.library;
    let mut seen: HashMap<Name, TyId> = HashMap::new();
    let mut bind = |name: Name, ty: TyId, bound: &mut Bound| match seen.get(&name) {
        None => {
            seen.insert(name, ty);
            bound.types.push((env.names.resolve(name).to_owned(), env.program.types.display(ty, viewer, env.names)));
        }
        Some(first) if *first == ty => {}
        Some(_) => {
            bound.several.insert(name);
        }
    };
    // In the order of the lines: a structure, then the lines of its bodies, then what follows it.
    // Each body with the loops around it.
    let mut pending: Vec<(std::slice::Iter<'_, Node>, Rc<[String]>)> = vec![(region.nodes.iter(), Rc::from([]))];
    let enter = |region: &Region, bound: &mut Bound, bind: &mut dyn FnMut(Name, TyId, &mut Bound)| {
        for value in &region.args {
            for name in value.names.iter() {
                bind(*name, value.ty, bound);
            }
        }
    };
    // The inputs of the function are not bound by its body: the `in` of the `call` gives their types.
    while let Some((nodes, around)) = pending.last_mut() {
        let around = Rc::clone(around);
        let Some(node) = nodes.next() else {
            pending.pop();
            continue;
        };
        if !around.is_empty() {
            bound.loops.insert(node.origin, Rc::clone(&around));
        }
        for value in node.outputs.iter() {
            for name in value.names.iter() {
                bind(*name, value.ty, &mut bound);
            }
        }
        match &node.kind {
            NodeKind::Loop { region, .. } | NodeKind::For { region, .. } => {
                enter(region, &mut bound, &mut bind);
                let inner: Rc<[String]> = around.iter().cloned().chain(std::iter::once(env.names.resolve(node.name).to_owned())).collect();
                pending.push((region.nodes.iter(), inner));
            }
            NodeKind::Match { arms } | NodeKind::Handle { arms } => {
                // The arms are entered last first, so that the first is walked first.
                for arm in arms.iter().rev() {
                    pending.push((arm.region.nodes.iter(), Rc::clone(&around)));
                }
                for arm in arms {
                    enter(&arm.region, &mut bound, &mut bind);
                }
            }
            _ => {}
        }
    }
    let several: HashSet<&str> = bound.several.iter().map(|name| env.names.resolve(*name)).collect();
    bound.types.retain(|(name, _)| !several.contains(name.as_str()));
    if let Some(file) = files.get(declared.file as usize) {
        arm_texts(file, env, &mut bound.arms);
    }
    bound
}

/// The text of each arm of the file, its patterns as the text writes them (`format::write_pattern`), joined as a list is
/// (`1, 3..=5`): `_` stays `_`.
fn arm_texts(file: &CheckedFile, env: Env<'_>, arms: &mut HashMap<Id, String>) {
    let labcraft_graph::Decl::Function(function) = &file.file.decl else { return };
    let mut pending: Vec<&Source> = vec![&function.body];
    while let Some(body) = pending.pop() {
        for statement in &body.statements {
            let StatementKind::Binding { value, .. } = &statement.kind else { continue };
            match value {
                Rhs::Match(branching) | Rhs::Handle(branching) => {
                    for arm in &branching.arms {
                        let text: Vec<String> =
                            arm.patterns().map(|pattern| labcraft_format::write_pattern(pattern, env.names, PATTERN_TEXT)).collect();
                        arms.insert(arm.id, text.join(", "));
                        pending.push(&arm.body);
                    }
                }
                Rhs::For(structure) => pending.push(&structure.body),
                Rhs::Loop(structure) => pending.push(&structure.body),
                Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
            }
        }
    }
}
