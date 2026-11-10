//! Pass 6 (`docs/architecture.md`, section 5): the `panel`, `layout` and `icon` sections of a function (spec 10, 11, 11.1).
//!
//! A `layout` entry is matched against the wires of the function, as the diagram draws them: a wire carries a value to an input of a
//! node, and a value from outside a structure crosses each edge on its way, one wire per frame, each with its own route. The entries wait
//! for the errors of the function, as the warnings of pass 5 do: a name with a mistake reads nothing, and its wire would seem to be missing.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use labcraft_format::coordinate_value;
use labcraft_graph::{
    Arg, Body, Call, Clause, ClauseKind, Function, Id, Layout, LayoutItem, LayoutKind, NodeRef, Numeral, NumeralKind, Operand, OrderRef, Panel,
    PanelDirection, Rhs, Signature, Statement, StatementKind, Target as Line, TerminalRef,
};
use labcraft_types::{Code, Interner, Name, Related, Report, cut_text};

use super::super::{Pass, Scope};
use super::Context;
use super::types::panel::known_kind;
use crate::load::at;
use crate::primitives::PrimitiveLibrary;
use crate::resolved::{Meaning, Resolved, Terminal};

mod icon;

pub use icon::{MAX_ICON_SHAPES, PathSegment, icon_refused, path_segments};
#[cfg(test)]
mod tests;

const NONE: u32 = u32::MAX;

/// The work allowed on matching entries to wires, by entry and wire and at least `MIN_WORK`, as in pass 5.
const WORK_PER_ITEM: u64 = 64;
const MIN_WORK: u64 = 1 << 24;

/// The most wires that the entries of a function can ask for, by item of its file and at least `MIN_WIRES`: a value read deep in nested
/// structures crosses each edge on its way, so a file built for it would ask for its size times its depth.
const WIRES_PER_PLACE: usize = 1;
const MIN_WIRES: usize = 1 << 18;

/// What only this pass says: the programs that test the other passes are written for what those judge.
#[cfg(test)]
pub(in crate::declare) fn only_this_pass(code: Code) -> bool {
    matches!(
        code,
        Code::Lc5001 | Code::Lc5002 | Code::Lc5003 | Code::Lc5004 | Code::Lc5005 | Code::Lc5006 | Code::Lc5007 | Code::Lc5008 | Code::Lc5009
    )
}

/// Judges the sections of one function, or of an `extern`, which has no diagram.
pub(super) fn judge_sections<'a>(
    pass: &mut Pass<'a>,
    context: &Context<'_, 'a>,
    scope: Scope<'a>,
    resolved: &Resolved,
    name: Name,
    signature: &'a Signature,
    function: Option<&'a Function>,
) {
    // Before anything this pass says: its own errors are not the mistakes of the function. Those of its signature were said before
    // its body, so they count apart, as in pass 5.
    let faulty = context.declared.get(scope.library).and_then(|library| library.functions.get(&name)).is_none_or(|declared| declared.faulty);
    let clean = !faulty && pass.reports.get(scope.library).map_or(0, Report::errors) == pass.function_errors;
    let mut sections = Sections { pass, scope };
    let file = &scope.model.file;
    if let Some(panel) = &file.panel {
        sections.panel(panel, signature);
    }
    if let Some(layout) = &file.layout {
        sections.coordinates(layout);
        match function {
            None => sections.no_diagram(layout),
            Some(function) if clean => {
                let mut mentioned = HashSet::new();
                for item in &layout.items {
                    if let LayoutKind::Wire { from, to, .. } = &item.kind {
                        mentioned.insert((from.node.name, from.node.is_label));
                        // An input can bear the name of a value from outside, which crosses there.
                        mentioned.extend(to.terminal.map(|input| (input, false)));
                    }
                }
                let limit = scope.model.source_map.len().saturating_mul(WIRES_PER_PLACE).max(MIN_WIRES);
                let wires = Wires::of(function, signature, resolved, context, &mentioned, limit);
                sections.pass.cut_short |= wires.overflow;
                sections.layout(layout, &wires);
            }
            Some(_) => {}
        }
    }
    if let Some(icon) = &file.icon {
        icon::judge(&mut sections, icon);
    }
}

struct Sections<'p, 'a> {
    pass: &'p mut Pass<'a>,
    scope: Scope<'a>,
}

impl<'a> Sections<'_, 'a> {
    fn names(&self) -> &'a Interner {
        self.pass.names
    }

    fn text(&self, name: Name) -> String {
        cut_text(self.pass.names.resolve(name)).into_owned()
    }

    fn say(&mut self, code: Code, message: String, id: Id, related: Vec<Related>) {
        let mut diagnostic = at(code, message, &self.scope.file.shown, &self.scope.model.source_map, id);
        diagnostic.function = self.scope.function.map(|name| cut_text(name).into_owned());
        diagnostic.related = related;
        self.pass.push(self.scope.library, diagnostic);
    }

    fn here(&self, message: String, id: Id) -> Related {
        Related { message, file: self.scope.file.shown.clone(), span: self.scope.model.source_map.span(id) }
    }

    // The panel (10).

    fn panel(&mut self, panel: &'a Panel, signature: &'a Signature) {
        let inputs: HashSet<Name> = signature.params.iter().map(|param| param.name).collect();
        let outputs: HashSet<Name> = signature.outputs.iter().map(|output| output.name).collect();
        let mut first: HashMap<Name, Id> = HashMap::with_capacity(panel.items.len());
        for item in &panel.items {
            let element = self.text(item.name);
            // Each item is judged on its own as well: which one comes second depends on their order, which `fmt` changes.
            if let Some(earlier) = first.get(&item.name).copied() {
                let related = vec![self.here(format!("`{element}` is first declared here"), earlier)];
                self.say(Code::Lc5001, format!("the panel element `{element}` is declared twice: one element, one name (10)"), item.id, related);
            } else {
                first.insert(item.name, item.id);
            }
            let (input, output) = (inputs.contains(&item.name), outputs.contains(&item.name));
            match item.direction {
                PanelDirection::Control if output && !input => {
                    let message = format!(
                        "`{element}` is an output of the function, so its element is an `indicator`: a `control` gives a value to the function (10)"
                    );
                    self.say(Code::Lc5002, message, item.id, Vec::new());
                }
                PanelDirection::Indicator if input && !output => {
                    let message =
                        format!("`{element}` is an input of the function, so its element is a `control`: an `indicator` shows what it gives (10)");
                    self.say(Code::Lc5002, message, item.id, Vec::new());
                }
                PanelDirection::Control | PanelDirection::Indicator => {}
            }
            if !known_kind(self.names().resolve(item.kind)) {
                let kind = self.text(item.kind);
                let message = format!(
                    "`{kind}` is not a panel kind of M0, so `{element}` has no widget: the kinds are `button`, `switch`, `numeric`, `text_box`, `led` and `chart` (10)"
                );
                self.say(Code::Lc5003, message, item.id, Vec::new());
            }
        }
    }

    // The layout (11).

    /// `LC5005`: whatever names the entries hold, a coordinate is an integer in the range.
    fn coordinates(&mut self, layout: &'a Layout) {
        for item in &layout.items {
            let points = match &item.kind {
                LayoutKind::Node { at, .. } => at.as_slice(),
                LayoutKind::Wire { via, .. } => via.as_slice(),
            };
            for point in points {
                for numeral in [point.x, point.y] {
                    if !self.is_coordinate(numeral) {
                        let text = numeral_text(numeral, self.names());
                        let message =
                            format!("`{text}` is not a coordinate: a coordinate is an integer between -1000000 and 1000000, written in digits (11)");
                        self.say(Code::Lc5005, message, point.id, Vec::new());
                    }
                }
            }
        }
    }

    fn is_coordinate(&self, numeral: Numeral) -> bool {
        match numeral.kind {
            NumeralKind::Int(text) => coordinate_value(self.names().resolve(text)).is_some(),
            NumeralKind::Float(_) | NumeralKind::Inf | NumeralKind::Nan => false,
        }
    }

    /// An `extern` has no diagram, so its entries name nothing.
    fn no_diagram(&mut self, layout: &'a Layout) {
        for item in &layout.items {
            let message = "an `extern` function has no diagram, so this entry names nothing: `fmt` removes it (11)".to_owned();
            self.say(Code::Lc5004, message, item.id, Vec::new());
        }
    }

    fn layout(&mut self, layout: &'a Layout, wires: &Wires) {
        let mut designated: HashMap<Designated, Id> = HashMap::with_capacity(layout.items.len());
        let items = u64::try_from(layout.items.len().saturating_add(wires.list.len())).unwrap_or(u64::MAX);
        let mut work = items.saturating_mul(WORK_PER_ITEM).max(MIN_WORK);
        for item in &layout.items {
            let found = match &item.kind {
                LayoutKind::Node { node, .. } => self.node_entry(item, node, wires),
                LayoutKind::Wire { from, to, .. } => self.wire_entry(item, from, to, wires, &mut work),
            };
            let Some(found) = found else { continue };
            if let Some(first) = designated.get(&found).copied() {
                let what = match found {
                    Designated::Node(_) => "node",
                    Designated::Wire(_) => "wire",
                };
                let related = vec![self.here(format!("the first entry for this {what}"), first)];
                self.say(Code::Lc5007, format!("a second entry for the same {what}: a {what} has at most one entry (11)"), item.id, related);
            } else {
                designated.insert(found, item.id);
            }
        }
    }

    fn reference(&self, node: NodeRef) -> String {
        let name = self.text(node.name);
        if node.is_label { format!("#{name}") } else { name }
    }

    /// What a name that designates no node is, for the message.
    fn frame_terminal(node: NodeRef, wires: &Wires) -> Option<&'static str> {
        if node.is_label {
            return None;
        }
        wires.frame.get(&node.name).copied()
    }

    fn node_entry(&mut self, item: &'a LayoutItem, node: &NodeRef, wires: &Wires) -> Option<Designated> {
        let shown = self.reference(*node);
        match wires.nodes.get(&(node.name, node.is_label)).map(Vec::as_slice) {
            Some([only]) => Some(Designated::Node(*only)),
            Some([_, _, ..]) => {
                let message = format!("`node {shown}` is ambiguous: `{shown}` names a node in several bodies, and a label tells them apart (11)");
                self.say(Code::Lc5006, message, item.id, Vec::new());
                None
            }
            _ => {
                let message = match Self::frame_terminal(*node, wires) {
                    Some(what) => {
                        format!("`node {shown}` names nothing: `{shown}` is {what} of the function, a terminal of its frame, not a node (11)")
                    }
                    None => format!("`node {shown}` names no node: a node is designated by its label, or by the name of its line (11)"),
                };
                self.say(Code::Lc5004, message, item.id, Vec::new());
                None
            }
        }
    }

    fn wire_entry(&mut self, item: &'a LayoutItem, from: &TerminalRef, to: &TerminalRef, wires: &Wires, work: &mut u64) -> Option<Designated> {
        let (start, end) = (self.reference(from.node), self.reference(to.node));
        if let Some(terminal) = from.terminal {
            let terminal = self.text(terminal);
            let message =
                format!("`wire {start}.{terminal} -> …` names no wire: a wire starts at the name of the value it carries, without a terminal (11)");
            self.say(Code::Lc5004, message, item.id, Vec::new());
            return None;
        }
        let Some(starts) = wires.starts.get(&(from.node.name, from.node.is_label)) else {
            let message = format!("`wire {start} -> {end}` names no wire: `{start}` is no value and no label of this function (11)");
            self.say(Code::Lc5004, message, item.id, Vec::new());
            return None;
        };
        let Some(ends) = wires.ends.get(&(to.node.name, to.node.is_label)) else {
            let message = match Self::frame_terminal(to.node, wires) {
                Some(what) => format!("`wire {start} -> {end}` names no wire: `{end}` is {what} of the function, where no wire ends (11)"),
                None => format!("`wire {start} -> {end}` names no wire: `{end}` names no node and no output of this function (11)"),
            };
            self.say(Code::Lc5004, message, item.id, Vec::new());
            return None;
        };
        if wires.overflow {
            return None;
        }
        // `after` is the order input of every call (R-15); a carry, a field or a value can bear that name too.
        let after = to.terminal.is_some_and(|input| self.names().resolve(input) == "after");
        // An input can be the name of a value from outside, under any of its names.
        let crossing = to.terminal.and_then(|input| wires.starts.get(&(input, false)));
        let (mut matched, mut unnamed) = (Matched::default(), Matched::default());
        for start in starts.as_slice() {
            for end in ends {
                // Each list is read once, against a budget: a name defined in many bodies, or a value that reaches one node through many
                // inputs, would make each entry long. Beyond the budget nothing more is said, as in pass 5.
                let list = wires.between.get(&(*start, *end)).map_or(&[][..], Few::as_slice);
                let cost = u64::try_from(list.len()).unwrap_or(u64::MAX).saturating_add(1);
                if cost > *work {
                    *work = 0;
                    self.pass.cut_short = true;
                    return None;
                }
                *work -= cost;
                for index in list {
                    let Some(wire) = wires.list.get(*index as usize) else { continue };
                    let fits = match (to.terminal, wire.input) {
                        (None, _) => true,
                        (Some(input), Input::Named(name)) => name == input,
                        (Some(_), Input::After) => after,
                        (Some(_), Input::Crossing(value)) => crossing.is_some_and(|names| names.contains(&Start::Value(value))),
                        (Some(_), Input::Unnamed) => false,
                    };
                    if fits {
                        matched.push(*index);
                        if wire.input == Input::Unnamed {
                            unnamed.push(*index);
                        }
                    }
                }
            }
        }
        let Some(input) = to.terminal else {
            // Of several, the one that reaches an input without a name: the selector, a value of a variant, an element of an array, and
            // inside `until`, `collect`, `yield` (spec 11).
            if matched.count > 1 && unnamed.count == 1 {
                return self.one_of(item, &unnamed, &start, &end, wires);
            }
            return self.one_of(item, &matched, &start, &end, wires);
        };
        if matched.count == 0 && !ends.iter().any(|candidate| wires.has_input(*candidate, input, after)) {
            let input = self.text(input);
            let message = format!("`wire {start} -> {end}.{input}` names no wire: `{end}` has no input `{input}` (11)");
            self.say(Code::Lc5004, message, item.id, Vec::new());
            return None;
        }
        let end = format!("{end}.{}", self.text(input));
        self.one_of(item, &matched, &start, &end, wires)
    }

    /// The one wire that an entry matches.
    fn one_of(&mut self, item: &'a LayoutItem, matched: &Matched, start: &str, end: &str, wires: &Wires) -> Option<Designated> {
        match (matched.count, matched.sample.as_slice()) {
            (1, [only]) => Some(Designated::Wire(*only)),
            (0, _) => {
                let message = format!("`wire {start} -> {end}` names no wire: `{start}` does not go to `{end}` (11)");
                self.say(Code::Lc5006, message, item.id, Vec::new());
                None
            }
            (count, sample) => {
                let mut inputs: Vec<String> = sample
                    .iter()
                    .filter_map(|index| wires.list.get(*index))
                    .filter_map(|wire| wires.input_text(wire.input, self.names()))
                    .map(|input| format!("`{end}.{}`", cut_text(input)))
                    .collect();
                inputs.sort();
                inputs.dedup();
                // Only when the names tell every wire apart: otherwise naming the input would not settle it.
                let hint = if inputs.len() == count { format!(": name the input, {}", inputs.join(" or ")) } else { String::new() };
                let message = format!("`wire {start} -> {end}` is ambiguous: {count} wires join them{hint} (11)");
                self.say(Code::Lc5006, message, item.id, Vec::new());
                None
            }
        }
    }
}

/// The wires that an entry matches: how many, and the first few, which is all that a message needs.
#[derive(Default)]
struct Matched {
    count: usize,
    sample: Vec<usize>,
}

impl Matched {
    const SAMPLE: usize = 5;

    fn push(&mut self, index: u32) {
        self.count = self.count.saturating_add(1);
        if self.sample.len() < Self::SAMPLE {
            self.sample.push(index as usize);
        }
    }
}

/// The text of a number as written, for a message.
fn numeral_text(numeral: Numeral, names: &Interner) -> String {
    let digits = match numeral.kind {
        NumeralKind::Int(text) | NumeralKind::Float(text) => cut_text(names.resolve(text)).into_owned(),
        NumeralKind::Inf => "inf".to_owned(),
        NumeralKind::Nan => "nan".to_owned(),
    };
    if numeral.negative { format!("-{digits}") } else { digits }
}

/// What a layout entry designates, for `LC5007`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Designated {
    Node(Id),
    Wire(usize),
}

/// Where a wire starts: a value, through the names that a line `x = y` gives it, or the label that an `after:` names.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Start {
    Value(Id),
    Label(Id),
}

/// Where a wire ends.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum End {
    /// A node, by the id of its statement.
    Node(Id),
    /// A structure, seen from inside: `next`, `until`, `collect`, `yield`.
    Inside(Id),
    /// An output of the function.
    Output(Name),
}

/// The input of a node that a wire reaches.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Input {
    Named(Name),
    After,
    /// Where a value from outside crosses the edge of a structure, by that value: once [`Wires`] is made, the value whose second names
    /// `x = y` gives. The input bears each of its names.
    Crossing(Id),
    /// The selector of a `match` or a `handle`, an element of an array, what goes to `until`, `collect` or `yield`.
    Unnamed,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Wire {
    start: Start,
    end: End,
    input: Input,
    /// The frame where it is drawn: two arms that yield one value draw two wires.
    frame: u32,
}

/// A body: the function, a loop or an arm.
#[derive(Clone, Copy)]
struct Frame {
    parent: u32,
    /// The statement of its structure; `None` for the function.
    structure: Option<Id>,
}

/// The parameters of what a call reaches, by its callee.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Callee {
    Function(u32, Name),
    Primitive(PrimitiveLibrary, Name),
}

/// The parameters of a callee, in order and as a set.
type Params = (Rc<[Name]>, Rc<HashSet<Name>>);

/// The wires of a function as the diagram draws them, and what a layout entry can name.
/// A set that most often holds one thing, which then costs no allocation of its own: a function can have a million names.
enum Few<T> {
    One(T),
    /// In order, and as a set: boxed, so that One stays small.
    Many(Box<(Vec<T>, HashSet<T>)>),
}

impl<T: Copy + Eq + std::hash::Hash> Few<T> {
    fn insert(&mut self, value: T) {
        match self {
            Few::One(first) if *first == value => {}
            Few::One(first) => {
                let first = *first;
                *self = Few::Many(Box::new((vec![first, value], HashSet::from([first, value]))));
            }
            Few::Many(many) => {
                if many.1.insert(value) {
                    many.0.push(value);
                }
            }
        }
    }

    fn as_slice(&self) -> &[T] {
        match self {
            Few::One(only) => std::slice::from_ref(only),
            Few::Many(many) => &many.0,
        }
    }

    fn contains(&self, value: &T) -> bool {
        match self {
            Few::One(only) => only == value,
            Few::Many(many) => many.1.contains(value),
        }
    }
}

/// Wire indices by key, most keys having one wire.
fn add_index<K: Eq + std::hash::Hash>(map: &mut HashMap<K, Few<u32>>, key: K, index: u32) {
    match map.get_mut(&key) {
        Some(few) => few.insert(index),
        None => {
            map.insert(key, Few::One(index));
        }
    }
}

struct Wires {
    /// Only the wires of the values that the entries name, as a start or as an input: the others would cost the size of the function
    /// times its depth, and say nothing.
    list: Vec<Wire>,
    /// By where they start and where they end.
    between: HashMap<(Start, End), Few<u32>>,
    /// Where a value from outside crosses into a structure, by that value.
    crossings: HashSet<(Id, End)>,
    /// More wires than the limit: a file built for it, of which nothing more is said.
    overflow: bool,
    /// What a wire can start at, by name and whether it is a label.
    starts: HashMap<(Name, bool), Few<Start>>,
    /// The calls, which all have the order input `after`.
    calls: HashSet<End>,
    /// The inputs and outputs of the function, which are terminals of its frame, not nodes.
    frame: HashMap<Name, &'static str>,
    /// What a wire can end at, and what a `node` entry names.
    ends: HashMap<(Name, bool), Vec<End>>,
    nodes: HashMap<(Name, bool), Vec<Id>>,
    /// The inputs that a node has: those its wires reach, the parameters of a call, the fields of a construction, the clauses of a loop.
    inputs: HashMap<End, HashSet<Input>>,
    /// The parameters of a call, shared by the calls of one function: a function can have a million of them.
    params: HashMap<End, Rc<HashSet<Name>>>,
    /// The name of each value, for a message about where it crosses an edge.
    names: HashMap<Id, Name>,
}

impl Wires {
    /// Whether a node has an input that an entry names `input`, even one that no wire reaches.
    fn has_input(&self, end: End, input: Name, after: bool) -> bool {
        let inputs = self.inputs.get(&end);
        let has = |candidate: Input| inputs.is_some_and(|inputs| inputs.contains(&candidate));
        (after && self.calls.contains(&end))
            || has(Input::Named(input))
            || self.params.get(&end).is_some_and(|params| params.contains(&input))
            || self.starts.get(&(input, false)).is_some_and(|starts| {
                starts.as_slice().iter().any(|start| matches!(start, Start::Value(value) if self.crossings.contains(&(*value, end))))
            })
    }

    fn input_text<'n>(&self, input: Input, names: &'n Interner) -> Option<&'n str> {
        match input {
            Input::Named(name) => Some(names.resolve(name)),
            Input::After => Some("after"),
            Input::Crossing(value) => self.names.get(&value).map(|name| names.resolve(*name)),
            Input::Unnamed => None,
        }
    }
}

/// A start once every line `x = y` is known.
fn lift(aliases: &HashMap<Id, Id>, roots: &mut HashMap<Id, Id>, start: Start) -> Start {
    match start {
        Start::Value(id) => Start::Value(root_of(aliases, roots, id)),
        Start::Label(_) => start,
    }
}

/// What a line `x = y` gives a second name, through a chain of them, each link once: a chain is as long as the file.
fn root_of(aliases: &HashMap<Id, Id>, roots: &mut HashMap<Id, Id>, id: Id) -> Id {
    let mut chain = Vec::new();
    let mut current = id;
    let root = loop {
        if let Some(known) = roots.get(&current) {
            break *known;
        }
        match aliases.get(&current) {
            // A cycle has its `LC4001`, and this pass waits for it to be fixed: the bound only guards a mistake.
            Some(next) if chain.len() <= aliases.len() => {
                chain.push(current);
                current = *next;
            }
            _ => break current,
        }
    };
    for member in chain {
        roots.insert(member, root);
    }
    root
}

/// What builds [`Wires`]: one walk of the function.
struct Builder<'r, 'w, 'a> {
    resolved: &'r Resolved,
    context: &'r Context<'w, 'a>,
    frames: Vec<Frame>,
    /// By the id of what defines it: a value, a label.
    defined: HashMap<Id, (Name, u32)>,
    /// The labels among defined.
    labels: HashSet<Id>,
    aliases: HashMap<Id, Id>,
    list: Vec<Wire>,
    /// A value whose wire already leaves this frame for the one around it, and so goes on to where it is defined.
    crossed: HashSet<(Start, u32)>,
    nodes: HashMap<(Name, bool), Vec<Id>>,
    structures: HashSet<Id>,
    inputs: HashMap<End, HashSet<Input>>,
    params: HashMap<End, Rc<HashSet<Name>>>,
    calls: HashSet<End>,
    callees: HashMap<Callee, Params>,
    /// What the wires of which are made, by the id of what defines it; `None` on the first walk, which makes none.
    wanted: Option<HashSet<Id>>,
    limit: usize,
    overflow: bool,
}

impl Wires {
    /// `mentioned`: the names that the `wire` entries give, as a start or as an input. `limit`: the most wires that it makes.
    fn of<'a>(
        function: &'a Function,
        signature: &'a Signature,
        resolved: &Resolved,
        context: &Context<'_, 'a>,
        mentioned: &HashSet<(Name, bool)>,
        limit: usize,
    ) -> Self {
        // A first walk for the names and the lines `x = y`, which may come after what reads them (5.2), then one for the wires.
        let wanted = {
            let mut first = Builder::new(resolved, context, signature, None, 0);
            first.body(&function.body, 0);
            first.wanted(mentioned)
        };
        let mut builder = Builder::new(resolved, context, signature, Some(wanted), limit);
        builder.body(&function.body, 0);
        builder.finish(signature)
    }
}

impl<'r, 'w, 'a> Builder<'r, 'w, 'a> {
    fn new(resolved: &'r Resolved, context: &'r Context<'w, 'a>, signature: &Signature, wanted: Option<HashSet<Id>>, limit: usize) -> Self {
        let mut defined = HashMap::with_capacity(signature.params.len() + signature.outputs.len());
        for param in &signature.params {
            defined.insert(param.id, (param.name, 0));
        }
        for output in &signature.outputs {
            defined.insert(output.id, (output.name, 0));
        }
        Builder {
            resolved,
            context,
            frames: vec![Frame { parent: NONE, structure: None }],
            defined,
            labels: HashSet::new(),
            aliases: HashMap::new(),
            list: Vec::new(),
            crossed: HashSet::new(),
            nodes: HashMap::new(),
            structures: HashSet::new(),
            inputs: HashMap::new(),
            params: HashMap::new(),
            calls: HashSet::new(),
            callees: HashMap::new(),
            wanted,
            limit,
            overflow: false,
        }
    }

    /// Every value and label that bears a name in `mentioned`, through the lines `x = y` that give one wire several names.
    fn wanted(&self, mentioned: &HashSet<(Name, bool)>) -> HashSet<Id> {
        let mut roots = HashMap::with_capacity(self.aliases.len());
        let mut root = |id: Id| if self.labels.contains(&id) { id } else { root_of(&self.aliases, &mut roots, id) };
        let mut wanted_roots = HashSet::new();
        for (id, (name, _)) in &self.defined {
            if mentioned.contains(&(*name, self.labels.contains(id))) {
                wanted_roots.insert(root(*id));
            }
        }
        let mut wanted = HashSet::new();
        for id in self.defined.keys() {
            if wanted_roots.contains(&root(*id)) {
                wanted.insert(*id);
            }
        }
        wanted
    }
}

impl<'a> Builder<'_, '_, 'a> {
    /// Once every line `x = y` is known, since the order of lines has no meaning (5.2): a wire read as `x` is the wire of `y`.
    fn finish(self, signature: &'a Signature) -> Wires {
        let Builder { defined, labels, aliases, list: read, nodes, structures, inputs, params, calls, overflow, .. } = self;
        let mut roots = HashMap::with_capacity(aliases.len());
        let mut starts: HashMap<(Name, bool), Few<Start>> = HashMap::with_capacity(defined.len());
        for (id, (name, _)) in &defined {
            // A label is defined by its own id, which `after:` names.
            let is_label = labels.contains(id);
            let start = if is_label { Start::Label(*id) } else { lift(&aliases, &mut roots, Start::Value(*id)) };
            match starts.get_mut(&(*name, is_label)) {
                Some(few) => few.insert(start),
                None => {
                    starts.insert((*name, is_label), Few::One(start));
                }
            }
        }
        let mut seen = HashSet::with_capacity(read.len());
        let mut list = Vec::with_capacity(read.len());
        let mut between: HashMap<(Start, End), Few<u32>> = HashMap::new();
        let mut crossings = HashSet::new();
        // Past the limit nothing is said of the wires, so they are not indexed.
        let read = if overflow { Vec::new() } else { read };
        for wire in read {
            let input = match wire.input {
                Input::Crossing(id) => Input::Crossing(root_of(&aliases, &mut roots, id)),
                other => other,
            };
            let wire = Wire { start: lift(&aliases, &mut roots, wire.start), input, ..wire };
            if seen.insert(wire) {
                // The limit keeps the list far below the largest u32.
                let index = u32::try_from(list.len()).unwrap_or(u32::MAX);
                add_index(&mut between, (wire.start, wire.end), index);
                if let Input::Crossing(value) = wire.input {
                    crossings.insert((value, wire.end));
                }
                list.push(wire);
            }
        }
        let mut ends: HashMap<(Name, bool), Vec<End>> = HashMap::new();
        for (key, statements) in &nodes {
            for statement in statements {
                let entry = ends.entry(*key).or_default();
                entry.push(End::Node(*statement));
                if structures.contains(statement) {
                    entry.push(End::Inside(*statement));
                }
            }
        }
        for output in &signature.outputs {
            ends.entry((output.name, false)).or_default().push(End::Output(output.name));
        }
        let names = defined.into_iter().map(|(id, (name, _))| (id, name)).collect();
        let mut frame = HashMap::with_capacity(signature.params.len() + signature.outputs.len());
        for output in &signature.outputs {
            frame.insert(output.name, "an output");
        }
        // Of an input and an output of one name, the input is said first, as before.
        for param in &signature.params {
            frame.insert(param.name, "an input");
        }
        Wires { list, between, crossings, overflow, starts, calls, frame, ends, nodes, inputs, params, names }
    }

    fn new_frame(&mut self, parent: u32, structure: Id) -> u32 {
        let index = u32::try_from(self.frames.len()).unwrap_or(NONE);
        self.frames.push(Frame { parent, structure: Some(structure) });
        index
    }

    fn structure_of(&self, frame: u32) -> Option<Id> {
        self.frames.get(frame as usize).and_then(|frame| frame.structure)
    }

    /// Repeats go when every line `x = y` is known (`finish`).
    fn add(&mut self, wire: Wire) {
        if self.list.len() >= self.limit {
            self.overflow = true;
            return;
        }
        self.list.push(wire);
    }

    /// The names of a body, before anything in it reads them: the order of lines has no meaning (5.2).
    fn body(&mut self, body: &'a Body, frame: u32) {
        for statement in &body.statements {
            if let Some(label) = &statement.label {
                self.defined.insert(label.id, (label.name, frame));
                self.labels.insert(label.id);
            }
            for target in targets(statement) {
                if let Line::Name(id, name) = target {
                    // A line that binds an output defines that output (R-2), which is read by its id.
                    let defined = match self.resolved.of(*id) {
                        Meaning::Value(output) => output,
                        _ => *id,
                    };
                    self.defined.entry(defined).or_insert((*name, frame));
                }
            }
            if let Some(key) = self.designation(statement) {
                self.nodes.entry(key).or_default().push(statement.id);
            }
        }
        for statement in &body.statements {
            self.statement(statement, frame);
        }
    }

    /// How a `layout` entry names the node of this line, if it draws one (spec 11): by its label, or by the name of its line.
    fn designation(&self, statement: &Statement) -> Option<(Name, bool)> {
        let draws = match &statement.kind {
            StatementKind::Call(_) => true,
            StatementKind::Collect(_) | StatementKind::Until(_) => false,
            StatementKind::Binding { targets, value: Rhs::Operand(operand) } => match operand {
                // `x = y` gives a value a second name, and `k = step.low` reads a field: neither draws a node.
                Operand::Path(path) if path.rest.is_empty() => false,
                Operand::Path(path) if matches!(self.resolved.of(path.id), Meaning::Value(_)) => false,
                // A constant that goes to an output, or to nothing, is a terminal (R-4).
                _ => first_name(targets).is_some_and(|(id, _)| !matches!(self.resolved.of(id), Meaning::Value(_))),
            },
            StatementKind::Binding { .. } => true,
        };
        if !draws {
            return None;
        }
        match (&statement.label, first_name(targets(statement))) {
            (Some(label), _) => Some((label.name, true)),
            (None, Some((_, name))) => Some((name, false)),
            (None, None) => None,
        }
    }

    fn statement(&mut self, statement: &'a Statement, frame: u32) {
        let node = End::Node(statement.id);
        match &statement.kind {
            StatementKind::Call(call) => self.call(call, node, frame),
            StatementKind::Collect(value) | StatementKind::Until(value) => {
                if let Some(structure) = self.structure_of(frame) {
                    self.read(value, End::Inside(structure), Input::Unnamed, frame);
                }
            }
            StatementKind::Binding { targets, value } => match value {
                Rhs::Operand(operand) => self.operand_line(statement, targets, operand, frame),
                _ => {
                    // A node whose output is an output of the function: the wire goes from it to the frame (R-2).
                    for target in targets.iter() {
                        match target {
                            Line::Name(id, name) => {
                                if let Meaning::Value(output) = self.resolved.of(*id) {
                                    self.wire(Start::Value(output), output, End::Output(*name), Input::Unnamed, frame);
                                }
                            }
                            // An output with no name goes to `next`: its wire starts at the label of the node, if it has one (spec 11).
                            Line::Next(_, carry) => {
                                if let (Some(label), Some(structure)) = (&statement.label, self.structure_of(frame)) {
                                    self.wire(Start::Label(label.id), label.id, End::Inside(structure), Input::Named(*carry), frame);
                                }
                            }
                            Line::Skip(_) => {}
                        }
                    }
                    self.node_value(statement, value, frame);
                }
            },
        }
    }

    /// What a node reads: a call, a construction or a structure.
    fn node_value(&mut self, statement: &'a Statement, value: &'a Rhs, frame: u32) {
        let node = End::Node(statement.id);
        match value {
            Rhs::Operand(_) => {}
            Rhs::Call(call) => self.call(call, node, frame),
            Rhs::Struct(literal) => {
                self.inputs.entry(node).or_default().extend(literal.fields.iter().map(|field| Input::Named(field.name)));
                for field in &literal.fields {
                    self.read(&field.value, node, Input::Named(field.name), frame);
                }
            }
            Rhs::Match(branching) | Rhs::Handle(branching) => {
                self.structures.insert(statement.id);
                self.read(&branching.selector, node, Input::Unnamed, frame);
                for arm in &branching.arms {
                    let inner = self.new_frame(frame, statement.id);
                    for pattern in arm.patterns() {
                        for binding in pattern.bindings() {
                            self.defined.insert(binding.id, (binding.name, inner));
                        }
                    }
                    self.body(&arm.body, inner);
                    for value in &arm.yields.values {
                        self.read(value, End::Inside(statement.id), Input::Unnamed, inner);
                    }
                }
            }
            Rhs::For(structure) => self.structure(statement, &structure.clauses, &structure.body, frame),
            Rhs::Loop(structure) => self.structure(statement, &structure.clauses, &structure.body, frame),
        }
    }

    fn structure(&mut self, statement: &'a Statement, clauses: &'a [Clause], body: &'a Body, frame: u32) {
        self.structures.insert(statement.id);
        let node = End::Node(statement.id);
        let inner = self.new_frame(frame, statement.id);
        let mut names = Vec::with_capacity(clauses.len());
        for clause in clauses {
            match &clause.kind {
                ClauseKind::In(clause) => {
                    names.push(clause.name);
                    self.defined.insert(clause.id, (clause.name, inner));
                    self.read(&clause.source, node, Input::Named(clause.name), frame);
                    if let Some(end) = &clause.end {
                        self.read(end, node, Input::Named(clause.name), frame);
                    }
                }
                ClauseKind::Carry(carry) => {
                    names.push(carry.name);
                    self.defined.insert(carry.id, (carry.name, inner));
                    self.read(&carry.value, node, Input::Named(carry.name), frame);
                }
            }
        }
        for end in [node, End::Inside(statement.id)] {
            self.inputs.entry(end).or_default().extend(names.iter().map(|name| Input::Named(*name)));
        }
        self.body(body, inner);
    }

    /// A line whose value is an operand: a constant, a second name, a field, or what goes to `next` or to an output.
    fn operand_line(&mut self, statement: &'a Statement, targets: &'a [Line], operand: &'a Operand, frame: u32) {
        let [target] = targets else { return };
        match target {
            Line::Next(_, carry) => {
                if let Some(structure) = self.structure_of(frame) {
                    self.read(operand, End::Inside(structure), Input::Named(*carry), frame);
                }
            }
            Line::Name(id, name) => match (self.resolved.of(*id), operand) {
                (Meaning::Value(output), _) => {
                    // `err = e3` gives the wire of `e3` the name of the output too, which can be read back (5.1, R-2).
                    if let Operand::Path(path) = operand
                        && path.rest.is_empty()
                        && let Meaning::Value(value) = self.resolved.of(path.id)
                    {
                        self.aliases.insert(output, value);
                    }
                    self.read(operand, End::Output(*name), Input::Unnamed, frame);
                }
                // `x = y`: the wire of `y` goes on under a second name (5.1). `k = step.low` reads a field: `k` is a value of its own, whose wire
                // starts here. Either way the wire of `y` or `step` enters this frame.
                (_, Operand::Path(path)) => {
                    if let Meaning::Value(value) = self.resolved.of(path.id) {
                        if path.rest.is_empty() {
                            self.aliases.insert(*id, value);
                        }
                        self.enter(value, frame);
                    }
                }
                // A constant node: an array of wires is built here.
                _ => self.read(operand, End::Node(statement.id), Input::Unnamed, frame),
            },
            Line::Skip(_) => {}
        }
    }

    fn call(&mut self, call: &'a Call, node: End, frame: u32) {
        self.calls.insert(node);
        let resolved = self.resolved.call(call.id);
        let params = resolved.and_then(|resolved| self.params(resolved.callee));
        if let Some((_, known)) = &params {
            self.params.insert(node, Rc::clone(known));
        }
        let terminals = resolved.map_or(&[][..], |resolved| &resolved.terminals[..]);
        for (index, arg) in call.args.iter().enumerate() {
            match arg {
                Arg::Value { name, value, .. } => {
                    if let Some(name) = name
                        && !params.as_ref().is_some_and(|(_, known)| known.contains(name))
                    {
                        // A field of `string.format` is an input that only its argument names; the others are in `params`.
                        self.inputs.entry(node).or_default().insert(Input::Named(*name));
                    }
                    let input = match (name, terminals.get(index)) {
                        (Some(name), _) => Input::Named(*name),
                        (None, Some(Terminal::Param(position))) => {
                            params.as_ref().and_then(|(params, _)| params.get(*position as usize)).map_or(Input::Unnamed, |name| Input::Named(*name))
                        }
                        (None, _) => Input::Unnamed,
                    };
                    self.read(value, node, input, frame);
                }
                Arg::After { refs, .. } => {
                    for order in refs {
                        match order {
                            OrderRef::Value(value) => self.read(value, node, Input::After, frame),
                            OrderRef::Label(label) => {
                                if let Meaning::Label(target) = self.resolved.of(label.id) {
                                    self.wire(Start::Label(target), target, node, Input::After, frame);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The names of the parameters of what a call reaches, in order and as a set, made once for each.
    fn params(&mut self, callee: Meaning) -> Option<Params> {
        let key = match callee {
            Meaning::Function { library, name } => Callee::Function(library, name),
            Meaning::Primitive { library, name } => Callee::Primitive(library, name),
            _ => return None,
        };
        if let Some((list, set)) = self.callees.get(&key) {
            return Some((Rc::clone(list), Rc::clone(set)));
        }
        let params: Rc<[Name]> = match key {
            Callee::Function(library, name) => {
                let function = self.context.declared.get(library as usize)?.functions.get(&name)?;
                function.params.iter().map(|param| param.name).collect()
            }
            Callee::Primitive(library, name) => self.context.primitives.get(library, name)?.params.iter().map(|(name, _)| *name).collect(),
        };
        let set: Rc<HashSet<Name>> = Rc::new(params.iter().copied().collect());
        self.callees.insert(key, (Rc::clone(&params), Rc::clone(&set)));
        Some((params, set))
    }

    fn read(&mut self, operand: &'a Operand, end: End, input: Input, frame: u32) {
        match operand {
            Operand::Path(path) => {
                if let Meaning::Value(value) = self.resolved.of(path.id) {
                    self.wire(Start::Value(value), value, end, input, frame);
                }
            }
            Operand::Array(_, values) => {
                for value in values {
                    self.read(value, end, input, frame);
                }
            }
            Operand::Bracket(id, _) => {
                if let Meaning::ArrayOfValue(value) = self.resolved.of(*id) {
                    self.wire(Start::Value(value), value, end, input, frame);
                }
            }
            Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) => {}
        }
    }

    /// The wire from what `defined` defines to `end` in `frame`, and one more for each edge that it crosses on its way there.
    fn wire(&mut self, start: Start, defined: Id, end: End, input: Input, frame: u32) {
        if self.overflow || !self.records(defined) {
            return;
        }
        self.add(Wire { start, end, input, frame });
        self.cross(start, defined, frame);
    }

    /// A line `x = y` draws nothing, but the wire of `y` still has to reach its frame.
    fn enter(&mut self, value: Id, frame: u32) {
        if !self.overflow && self.records(value) {
            self.cross(Start::Value(value), value, frame);
        }
    }

    /// Whether the wires of what `defined` defines are made: none on the first walk, and on the second only those that an entry names.
    fn records(&self, defined: Id) -> bool {
        self.wanted.as_ref().is_some_and(|wanted| wanted.contains(&defined))
    }

    /// From `frame` out to the frame where the value is defined, one wire per edge: to the structure, by the name of the value.
    fn cross(&mut self, start: Start, defined: Id, mut frame: u32) {
        let Some((_, home)) = self.defined.get(&defined).copied() else { return };
        let input = match start {
            Start::Value(_) => Input::Crossing(defined),
            // The input of a structure is a name, and a label has none to give it.
            Start::Label(_) => Input::Unnamed,
        };
        while frame != home {
            let Some(Frame { parent, structure: Some(structure) }) = self.frames.get(frame as usize).copied() else { return };
            if !self.crossed.insert((start, frame)) {
                return;
            }
            self.add(Wire { start, end: End::Node(structure), input, frame: parent });
            frame = parent;
        }
    }
}

fn targets(statement: &Statement) -> &[Line] {
    match &statement.kind {
        StatementKind::Binding { targets, .. } => targets,
        _ => &[],
    }
}

/// The first output of a line that is neither `_` nor `next x` (spec 11).
fn first_name(targets: &[Line]) -> Option<(Id, Name)> {
    targets.iter().find_map(|target| match target {
        Line::Name(id, name) => Some((*id, *name)),
        Line::Skip(_) | Line::Next(..) => None,
    })
}
