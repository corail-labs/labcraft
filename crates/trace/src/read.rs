//! The reader of a trace (`docs/cli.md`, 7.4; `docs/architecture.md`, 11.2): it takes the bytes of the file in parts,
//! never holds the whole file, and keeps only the values of frame 0 that fit the function drawn, within a bound.
//!
//! What it finds is plain data: `diagram` makes the diagnostics `LC7101` to `LC7104` from it, with their file.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::json::{Bad, Scanner};
use crate::value::VALUE_BUDGET;
use crate::write::SCHEMA;

/// What the function drawn has, as the caller gives it (`diagram`, from what `check` found).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expected {
    /// Named from the root of the project, as the first line of the trace names it.
    pub function: String,
    /// Its inputs, in the order of its signature: the `in` of the `call` of frame 0.
    pub inputs: Vec<ExpectedWire>,
    pub nodes: Vec<ExpectedNode>,
}

/// A node of the function drawn, at any level of its bodies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpectedNode {
    /// As the trace names it (cli.md, 4.3): its first output, its label (`#close`), or `@12`.
    pub name: String,
    /// The arms that hold it, outermost first. Two nodes of one name are told apart by the `arm` events of these.
    pub arms: Vec<ArmOf>,
    /// The loops around it, outermost first, by index in `nodes`: an `iter` has one level for each.
    pub loops: Vec<u32>,
    /// The values it binds, which are its wires.
    pub wires: Vec<ExpectedWire>,
}

/// An arm of a `match` or a `handle`: the node of the structure, by index in `nodes`, and the arm as the trace writes
/// its pattern.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArmOf {
    pub node: u32,
    pub arm: String,
}

/// A wire, by the name of its value, with its type as the trace writes it (`f64[V]`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpectedWire {
    pub name: String,
    pub ty: String,
}

/// The limits of reading *(proposed figures)*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    /// `--max-trace-size`: 512 MiB by default.
    pub file: u64,
    /// A line, without its end: 16 MiB, the bound of a line that a run writes.
    pub line: usize,
    /// The lines after the first: the 1,000,000 events of a run, then `truncated` and `end`.
    pub lines: u64,
    /// The levels of a line: a value nests at most 64.
    pub nesting: usize,
    /// What the reader keeps, its index included: 384 MiB, so that the values of 1,000,000 events of three outputs
    /// each fit, while the memory really taken stays well within the 512 MB of cli.md 7.4.
    pub kept: usize,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self { file: 512 << 20, line: 16 << 20, lines: 1_000_002, nesting: 256, kept: 384 << 20 }
    }
}

/// The first line of a trace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceHeader {
    pub run: String,
    pub function: String,
    pub mode: String,
    /// `(file, sha256)`, in the order of the line.
    pub files: Vec<(String, String)>,
    /// The line dropped some of its files, to stay within its bound.
    pub truncated: bool,
}

/// Why the first line gives no values (`LC7101`). A file that cannot be read is the caller's to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderProblem {
    /// Empty, not JSON, not an object, or a field missing or of the wrong kind.
    Unreadable,
    /// Another schema than `labcraft-trace`.
    OtherSchema,
    /// A version of `labcraft-trace` after 1.
    LaterVersion,
}

/// Where reading stopped, and why (`LC7103`); what was read before stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    /// From 1, the first line of the file.
    pub line: u64,
    pub limit: Limit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// A line that does not read: not JSON, too deep, a field missing or of the wrong kind, a negative count.
    Unreadable,
    File,
    Line,
    Lines,
    /// What the reader keeps.
    Kept,
}

/// An iteration from 2^63 on fits nothing.
const ITERATION_END: u64 = 1 << 63;
/// What an entry of the index costs beyond its text and its iterations: its share of the nodes of the map, the box of
/// its iterations and what an allocator adds to a small block (up to 24 bytes for glibc). Texts share one buffer. A
/// counting allocator measured about 82 bytes requested for an entry of one loop and a few bytes of text, in 1.17 blocks.
const ENTRY_COST: usize = 112;

type Iter = Box<[u64]>;
/// Where a text is in [`Trace::text`]: one buffer for every value and arm, rather than an allocation each.
type Span = (u32, u32);

/// What a trace gave for the function drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    /// The first line, or why it gives no values (`LC7101`).
    pub header: Result<TraceHeader, HeaderProblem>,
    /// The trace is that of another function (`LC7102`): no values.
    pub other_function: bool,
    /// The lines read after the first.
    pub events: u64,
    /// The status of the `end` event; `None` when the trace has none.
    pub end: Option<String>,
    /// The run cut its trace (a `truncated` event).
    pub truncated: bool,
    /// Where reading stopped at a line or a limit (`LC7103`).
    pub stop: Option<Stop>,
    /// The events that fit nothing in the drawing (`LC7104`), and the line of the first.
    pub ignored: u64,
    pub first_ignored: Option<u64>,
    inputs: Vec<Option<Box<str>>>,
    text: String,
    values: BTreeMap<(u32, u32, Iter), Span>,
    arms: BTreeMap<(u32, Iter), Span>,
    skipped: BTreeSet<(u32, Iter)>,
    highest: BTreeMap<(u32, Iter), u64>,
}

impl Trace {
    fn new(inputs: usize) -> Self {
        Self {
            header: Err(HeaderProblem::Unreadable),
            other_function: false,
            events: 0,
            end: None,
            truncated: false,
            stop: None,
            ignored: 0,
            first_ignored: None,
            inputs: vec![None; inputs],
            text: String::new(),
            values: BTreeMap::new(),
            arms: BTreeMap::new(),
            skipped: BTreeSet::new(),
            highest: BTreeMap::new(),
        }
    }

    fn text_of(&self, (start, len): Span) -> &str {
        self.text.get(start as usize..start as usize + len as usize).unwrap_or_default()
    }

    /// Keeps a text in the buffer; `None` past 4 GiB, which no bound of reading allows.
    fn store(&mut self, text: &str) -> Option<Span> {
        let start = u32::try_from(self.text.len()).ok()?;
        let len = u32::try_from(text.len()).ok()?;
        start.checked_add(len)?;
        self.text.push_str(text);
        Some((start, len))
    }

    /// The run or a limit of reading cut the trace: the banner says so.
    pub fn cut(&self) -> bool {
        self.truncated || self.stop.is_some()
    }

    /// The value of an input of the function, by its index in [`Expected::inputs`], as the file wrote it.
    pub fn input(&self, input: usize) -> Option<&str> {
        self.inputs.get(input)?.as_deref()
    }

    /// The value of a wire at an iteration, as the file wrote it: a node and a wire by their indices in [`Expected`].
    pub fn value(&self, node: u32, wire: u32, iter: &[u64]) -> Option<&str> {
        self.values.get(&(node, wire, Iter::from(iter))).map(|span| self.text_of(*span))
    }

    /// Every value of a wire, by iteration in order.
    pub fn values(&self, node: u32, wire: u32) -> impl Iterator<Item = (&[u64], &str)> {
        self.values
            .range((node, wire, Iter::default())..)
            .take_while(move |((n, w, _), _)| *n == node && *w == wire)
            .map(|((_, _, iter), span)| (iter.as_ref(), self.text_of(*span)))
    }

    /// The values of a wire within one iteration of the loops around it, `outer` being the outermost levels: what an
    /// iteration selector shows when an outer loop moves (notation 6.1).
    pub fn values_within<'a>(&'a self, node: u32, wire: u32, outer: &'a [u64]) -> impl Iterator<Item = (&'a [u64], &'a str)> {
        self.values
            .range((node, wire, Iter::from(outer))..)
            .take_while(move |((n, w, iter), _)| *n == node && *w == wire && iter.starts_with(outer))
            .map(|((_, _, iter), span)| (iter.as_ref(), self.text_of(*span)))
    }

    /// The arm that a `match` or a `handle` took at an iteration.
    pub fn arm(&self, node: u32, iter: &[u64]) -> Option<&str> {
        self.arms.get(&(node, Iter::from(iter))).map(|span| self.text_of(*span))
    }

    /// Whether the node was skipped at an iteration: its outputs hold defaults, kept out of sight (notation 6.1).
    pub fn skipped(&self, node: u32, iter: &[u64]) -> bool {
        self.skipped.contains(&(node, Iter::from(iter)))
    }

    /// The iterations of a loop that the trace gives, within an iteration of the loops around it: one past the highest.
    pub fn iterations(&self, loop_node: u32, outer: &[u64]) -> u64 {
        self.highest.get(&(loop_node, Iter::from(outer))).map_or(0, |highest| highest + 1)
    }
}

/// Reads a trace from its bytes, given in parts: [`Reader::feed`], then [`Reader::finish`].
pub struct Reader {
    expected: Expected,
    limits: ReadLimits,
    /// Each name of a node, as a number, and the name of each node.
    name_ids: HashMap<Box<str>, u32>,
    node_names: Vec<u32>,
    /// The node of each name that one node alone has: checked against its arms at once.
    single: HashMap<u32, u32>,
    /// The nodes of a shared name in no arm, by name and number of loops.
    plain: HashMap<(u32, usize), Vec<u32>>,
    /// The nodes of a shared name whose innermost arm is this arm of this structure, two at most for each name and
    /// number of loops, since a third could only make it ambiguous again.
    inside: HashMap<(u32, Box<str>), Vec<u32>>,
    /// The nodes whose arms the trace took, by name, the iteration of their innermost structure and their number of
    /// loops: at most two each, since two already tell nothing apart. So a name is resolved in a few lookups, however
    /// many nodes share it.
    active: HashMap<(u32, Iter, usize), Vec<u32>>,
    wires: Vec<HashMap<Box<str>, u32>>,
    inputs: HashMap<Box<str>, u32>,
    /// Every name of a wire: an entry of `types` for another name is not kept.
    wire_names: HashSet<Box<str>>,
    deepest: usize,
    pending: Vec<u8>,
    fed: u64,
    line: u64,
    done: bool,
    kept: usize,
    frames: HashSet<u64>,
    frame0: bool,
    types: HashMap<Box<str>, Box<str>>,
    /// The `call` of frame 0 was shortened by the run: a value whose type it left out is cut, not foreign.
    types_cut: bool,
    trace: Trace,
}

/// What an event of frame 0 gives, before it is placed.
#[derive(Default)]
struct Fields<'a> {
    seq: bool,
    t_ms: bool,
    event: Option<std::borrow::Cow<'a, str>>,
    frame: Option<u64>,
    node: Option<std::borrow::Cow<'a, str>>,
    iter: Option<&'a str>,
    out: Option<&'a str>,
    types: Option<&'a str>,
    inputs: Option<&'a str>,
    arm: Option<std::borrow::Cow<'a, str>>,
    status: Option<std::borrow::Cow<'a, str>>,
    /// The run shortened the line to stay within its bound (commands.md 8).
    truncated: bool,
}

impl Reader {
    pub fn new(expected: Expected, limits: ReadLimits) -> Self {
        let mut name_ids: HashMap<Box<str>, u32> = HashMap::new();
        let mut node_names = Vec::with_capacity(expected.nodes.len());
        let mut plain: HashMap<(u32, usize), Vec<u32>> = HashMap::new();
        let mut inside: HashMap<(u32, Box<str>), Vec<u32>> = HashMap::new();
        let mut wires = Vec::with_capacity(expected.nodes.len());
        let mut wire_names = HashSet::new();
        let mut deepest = 0;
        let mut sharing: HashMap<u32, Vec<u32>> = HashMap::new();
        for (index, node) in expected.nodes.iter().enumerate() {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            let next = u32::try_from(name_ids.len()).unwrap_or(u32::MAX);
            let name = *name_ids.entry(node.name.as_str().into()).or_insert(next);
            node_names.push(name);
            sharing.entry(name).or_default().push(index);
            let mut of_node = HashMap::new();
            for (wire, expected_wire) in node.wires.iter().enumerate() {
                of_node.entry(expected_wire.name.as_str().into()).or_insert(u32::try_from(wire).unwrap_or(u32::MAX));
                wire_names.insert(expected_wire.name.as_str().into());
            }
            wires.push(of_node);
            deepest = deepest.max(node.loops.len());
        }
        let mut single = HashMap::new();
        // How many nodes of a name and a number of loops each group holds already.
        type Group<'a> = (Option<(u32, &'a str)>, u32, usize);
        let mut alike: HashMap<Group<'_>, u8> = HashMap::new();
        for (name, nodes) in &sharing {
            if let [node] = nodes.as_slice() {
                single.insert(*name, *node);
                continue;
            }
            for &index in nodes {
                let Some(node) = expected.nodes.get(index as usize) else { continue };
                let innermost = node.arms.last().map(|arm| (arm.node, arm.arm.as_str()));
                let count = alike.entry((innermost, *name, node.loops.len())).or_default();
                if *count >= 2 {
                    continue;
                }
                *count += 1;
                match innermost {
                    None => plain.entry((*name, node.loops.len())).or_default().push(index),
                    Some((structure, arm)) => inside.entry((structure, arm.into())).or_default().push(index),
                }
            }
        }
        let mut inputs = HashMap::new();
        for (index, input) in expected.inputs.iter().enumerate() {
            inputs.entry(input.name.as_str().into()).or_insert(u32::try_from(index).unwrap_or(u32::MAX));
        }
        let trace = Trace::new(expected.inputs.len());
        Self {
            expected,
            limits,
            name_ids,
            node_names,
            single,
            plain,
            inside,
            active: HashMap::new(),
            wires,
            inputs,
            wire_names,
            deepest,
            pending: Vec::new(),
            fed: 0,
            line: 0,
            done: false,
            kept: 0,
            frames: HashSet::new(),
            frame0: false,
            types: HashMap::new(),
            types_cut: false,
            trace,
        }
    }

    /// Reads the next bytes of the file, and says whether reading goes on: after `false`, the rest of the file is not
    /// needed.
    pub fn feed(&mut self, mut bytes: &[u8]) -> bool {
        if self.done {
            return false;
        }
        let allowed = self.limits.file.saturating_sub(self.fed);
        let over = bytes.len() as u64 > allowed;
        if over {
            bytes = bytes.get(..usize::try_from(allowed).unwrap_or(usize::MAX)).unwrap_or_default();
        }
        self.fed += bytes.len() as u64;
        while !self.done {
            let Some(end) = bytes.iter().position(|byte| *byte == b'\n') else { break };
            let (line, rest) = bytes.split_at(end);
            bytes = rest.get(1..).unwrap_or_default();
            if self.pending.is_empty() {
                self.next_line(line);
            } else {
                let mut whole = std::mem::take(&mut self.pending);
                if whole.len() + line.len() > self.limits.line {
                    self.stop(self.line + 1, Limit::Line);
                    break;
                }
                whole.extend_from_slice(line);
                self.next_line(&whole);
                whole.clear();
                self.pending = whole;
            }
        }
        if !self.done {
            if self.pending.len() + bytes.len() > self.limits.line {
                self.stop(self.line + 1, Limit::Line);
            } else {
                self.pending.extend_from_slice(bytes);
            }
        }
        if over && !self.done {
            self.stop(self.line + 1, Limit::File);
        }
        !self.done
    }

    /// The end of the file: a last line without its end is read too.
    pub fn finish(mut self) -> Trace {
        if !self.done && !self.pending.is_empty() {
            let last = std::mem::take(&mut self.pending);
            self.next_line(&last);
        }
        self.trace
    }

    fn stop(&mut self, line: u64, limit: Limit) {
        if self.trace.stop.is_none() && self.trace.header.is_ok() {
            self.trace.stop = Some(Stop { line, limit });
        }
        self.done = true;
    }

    fn next_line(&mut self, bytes: &[u8]) {
        self.line += 1;
        if bytes.len() > self.limits.line {
            self.stop(self.line, Limit::Line);
            return;
        }
        if self.line == 1 {
            self.header(bytes);
            return;
        }
        if self.line - 1 > self.limits.lines {
            self.stop(self.line, Limit::Lines);
            return;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            self.stop(self.line, Limit::Unreadable);
            return;
        };
        if text.trim().is_empty() {
            self.stop(self.line, Limit::Unreadable);
            return;
        }
        self.trace.events += 1;
        if self.event(text).is_err() {
            self.trace.events -= 1;
            self.stop(self.line, Limit::Unreadable);
        }
    }

    fn header(&mut self, bytes: &[u8]) {
        self.done = true;
        let Ok(text) = std::str::from_utf8(bytes) else { return };
        // A byte order mark, which an editor may add, is not part of the line.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let header = match read_header(text, self.limits.nesting) {
            Ok(Ok(header)) => header,
            Ok(Err(problem)) => {
                self.trace.header = Err(problem);
                return;
            }
            Err(Bad) => return,
        };
        let cost = header.files.iter().map(|(file, sha)| file.len() + sha.len() + ENTRY_COST).sum::<usize>();
        let other = header.function != self.expected.function;
        self.trace.header = Ok(header);
        if other {
            self.trace.other_function = true;
            return;
        }
        self.done = false;
        if !self.keep(cost) {
            self.stop(1, Limit::Kept);
        }
    }

    /// Counts what is kept; `false` when it passes the bound.
    fn keep(&mut self, cost: usize) -> bool {
        self.kept = self.kept.saturating_add(cost);
        self.kept <= self.limits.kept
    }

    fn ignore(&mut self) {
        self.trace.ignored += 1;
        self.trace.first_ignored.get_or_insert(self.line);
    }

    fn event(&mut self, text: &str) -> Result<(), Bad> {
        let mut scanner = Scanner::new(text, self.limits.nesting);
        scanner.begin_object()?;
        let mut fields = Fields::default();
        let mut first = true;
        let mut whole = true;
        while let Some(key) = scanner.key(&mut first)? {
            match key.as_ref() {
                "seq" => {
                    once(fields.seq)?;
                    scanner.uint()?;
                    fields.seq = true;
                }
                "t_ms" => {
                    once(fields.t_ms)?;
                    scanner.number()?;
                    fields.t_ms = true;
                }
                "event" => set(&mut fields.event, scanner.string()?)?,
                "frame" => set(&mut fields.frame, scanner.uint()?)?,
                "node" => set(&mut fields.node, scanner.string()?)?,
                "arm" => set(&mut fields.arm, scanner.string()?)?,
                "status" => set(&mut fields.status, scanner.string()?)?,
                "iter" => set(&mut fields.iter, scanner.value()?)?,
                "out" => set(&mut fields.out, scanner.value()?)?,
                "types" => set(&mut fields.types, scanner.value()?)?,
                "in" => set(&mut fields.inputs, scanner.value()?)?,
                "truncated" => fields.truncated = scanner.bool()?,
                _ => {
                    scanner.value()?;
                }
            }
            // A line is decoded only as far as its frame: the events of another frame say no more than that.
            if let (Some(frame), Some(_)) = (fields.frame, &fields.event)
                && frame != 0
            {
                whole = false;
                break;
            }
        }
        // What the line holds after its frame is not read, its `seq` and `t_ms` included, whatever their order.
        if whole {
            scanner.end()?;
            if !fields.seq || !fields.t_ms {
                return Err(Bad);
            }
        }
        let event = fields.event.clone().ok_or(Bad)?;
        match event.as_ref() {
            "end" => {
                self.trace.end = Some(fields.status.ok_or(Bad)?.into_owned());
                return Ok(());
            }
            "truncated" => {
                self.trace.truncated = true;
                return Ok(());
            }
            _ => {}
        }
        let frame = fields.frame.ok_or(Bad)?;
        if frame == 0 && fields.truncated {
            // The run cut values of this line, or shortened it to what places it: the trace is cut.
            self.trace.truncated = true;
        }
        if frame != 0 {
            if event == "call" {
                if !self.frames.insert(frame) {
                    self.ignore();
                } else if !self.keep(ENTRY_COST) {
                    self.stop(self.line, Limit::Kept);
                }
            } else if !self.frames.contains(&frame) {
                self.ignore();
            }
            return Ok(());
        }
        match event.as_ref() {
            "call" => self.call(&fields),
            "ran" | "skipped" => self.ran(&fields, event == "skipped"),
            "arm" => self.arm_event(&fields),
            // A panel element: not on a wire.
            "set" if self.frame0 => Ok(()),
            _ => {
                self.ignore();
                Ok(())
            }
        }
    }

    fn call(&mut self, fields: &Fields<'_>) -> Result<(), Bad> {
        // A line that the run shortened may have lost its inputs and its types, never its place.
        let inputs = match fields.inputs {
            Some(inputs) => inputs,
            None if fields.truncated => "[]",
            None => return Err(Bad),
        };
        if self.frame0 {
            self.ignore();
            return Ok(());
        }
        self.frame0 = true;
        let mut ignored = false;
        let mut scanner = Scanner::new(inputs, self.limits.nesting);
        scanner.begin_array()?;
        let mut first = true;
        while scanner.element(&mut first)? {
            scanner.begin_object()?;
            let (mut name, mut ty, mut value) = (None, None, None);
            let mut first_key = true;
            while let Some(key) = scanner.key(&mut first_key)? {
                match key.as_ref() {
                    "name" => set(&mut name, scanner.string()?)?,
                    "type" => set(&mut ty, scanner.string()?)?,
                    "value" => set(&mut value, scanner.value()?)?,
                    _ => {
                        scanner.value()?;
                    }
                }
            }
            let (Some(name), Some(ty), Some(value)) = (name, ty, value) else { return Err(Bad) };
            let index = self.inputs.get(name.as_ref()).copied();
            let fits =
                index.and_then(|index| self.expected.inputs.get(index as usize)).is_some_and(|input| input.ty == ty) && value.len() <= VALUE_BUDGET;
            match index.and_then(|index| self.trace.inputs.get_mut(index as usize)) {
                Some(slot @ None) if fits => {
                    *slot = Some(value.into());
                    self.kept = self.kept.saturating_add(value.len() + ENTRY_COST);
                }
                _ => ignored = true,
            }
        }
        scanner.end()?;
        match fields.types {
            Some(types) => self.read_types(types, None)?,
            None if fields.truncated => {}
            None => return Err(Bad),
        }
        self.types_cut = fields.truncated;
        if self.kept > self.limits.kept {
            self.stop(self.line, Limit::Kept);
        }
        if ignored {
            self.ignore();
        }
        Ok(())
    }

    /// `types`, of the call of frame 0 (`own` is `None`), or of one event: only the names of wires are kept.
    fn read_types(&mut self, text: &str, mut own: Option<&mut HashMap<Box<str>, Box<str>>>) -> Result<(), Bad> {
        let mut scanner = Scanner::new(text, self.limits.nesting);
        scanner.begin_object()?;
        let mut first = true;
        while let Some(name) = scanner.key(&mut first)? {
            let ty = scanner.string()?;
            if !self.wire_names.contains(name.as_ref()) {
                continue;
            }
            match own.as_deref_mut() {
                Some(own) => {
                    own.insert(name.as_ref().into(), ty.as_ref().into());
                }
                None => {
                    self.kept = self.kept.saturating_add(name.len() + ty.len() + ENTRY_COST);
                    self.types.insert(name.as_ref().into(), ty.as_ref().into());
                }
            }
        }
        scanner.end()
    }

    /// The `iter` of an event: `None` when it fits no node, an iteration from 2^63 on or more levels than any node has.
    fn read_iter(&self, text: Option<&str>) -> Result<Option<Vec<u64>>, Bad> {
        let Some(text) = text else { return Ok(Some(Vec::new())) };
        let mut scanner = Scanner::new(text, self.limits.nesting);
        scanner.begin_array()?;
        let mut first = true;
        let mut iter = Vec::new();
        let mut fits = true;
        while scanner.element(&mut first)? {
            let iteration = scanner.uint()?;
            fits &= iteration < ITERATION_END;
            if iter.len() < self.deepest {
                iter.push(iteration);
            } else {
                fits = false;
            }
        }
        scanner.end()?;
        Ok(fits.then_some(iter))
    }

    /// The one node of this name whose loops have these iterations and whose arms were taken; `None` when there are
    /// none or several. A node is in `active` once the `arm` of its innermost structure was taken, and that structure
    /// was itself resolved so: so each of its arms was taken.
    fn resolve(&self, name: &str, iter: &[u64]) -> Option<u32> {
        let name = *self.name_ids.get(name)?;
        if let Some(&node) = self.single.get(&name) {
            let expected = self.expected.nodes.get(node as usize)?;
            let taken = expected.arms.iter().all(|arm| {
                let depth = self.expected.nodes.get(arm.node as usize).map_or(usize::MAX, |structure| structure.loops.len());
                iter.get(..depth)
                    .and_then(|outer| self.trace.arms.get(&(arm.node, Iter::from(outer))))
                    .is_some_and(|span| self.trace.text_of(*span) == arm.arm)
            });
            return (expected.loops.len() == iter.len() && taken).then_some(node);
        }
        let mut found: Vec<u32> = Vec::new();
        found.extend(self.plain.get(&(name, iter.len())).into_iter().flatten().take(2));
        for level in 0..=iter.len() {
            if found.len() > 1 {
                return None;
            }
            let outer: Iter = iter.get(..level).unwrap_or_default().into();
            found.extend(self.active.get(&(name, outer, iter.len())).into_iter().flatten().take(2));
        }
        match found.as_slice() {
            [node] => Some(*node),
            _ => None,
        }
    }

    /// An arm was taken: the nodes it holds directly can now be resolved at this iteration.
    fn activate(&mut self, structure: u32, arm: &str, iter: &[u64]) {
        let Some(nodes) = self.inside.get(&(structure, Box::from(arm))) else { return };
        for node in nodes {
            let (Some(name), Some(expected)) = (self.node_names.get(*node as usize), self.expected.nodes.get(*node as usize)) else { continue };
            let entry = self.active.entry((*name, Iter::from(iter), expected.loops.len())).or_default();
            if entry.len() < 2 {
                entry.push(*node);
                self.kept = self.kept.saturating_add(iter.len() * 8 + ENTRY_COST);
            }
        }
    }

    /// The loops around a node have reached these iterations.
    fn count_iterations(&mut self, node: u32, iter: &[u64]) {
        let Some(loops) = self.expected.nodes.get(node as usize).map(|node| node.loops.clone()) else { return };
        for (level, (loop_node, iteration)) in loops.iter().zip(iter).enumerate() {
            let outer: Iter = iter.get(..level).unwrap_or_default().into();
            let cost = outer.len() * 8 + ENTRY_COST;
            let highest = self.trace.highest.entry((*loop_node, outer)).or_insert_with(|| {
                self.kept = self.kept.saturating_add(cost);
                *iteration
            });
            *highest = (*highest).max(*iteration);
        }
    }

    fn ran(&mut self, fields: &Fields<'_>, skipped: bool) -> Result<(), Bad> {
        let name = fields.node.as_deref().ok_or(Bad)?;
        let out = match fields.out {
            Some(out) => out,
            // Shortened to what places it: no values, so nothing to place or to ignore.
            None if fields.truncated => return Ok(()),
            None => return Err(Bad),
        };
        let iter = self.read_iter(fields.iter)?;
        // A value whose type a shortened line left out is cut, not foreign: it is dropped without a word.
        let types_cut = self.types_cut || fields.truncated;
        let mut own = HashMap::new();
        if let Some(types) = fields.types {
            self.read_types(types, Some(&mut own))?;
        }
        let node = match (&iter, self.frame0) {
            (Some(iter), true) => self.resolve(name, iter),
            _ => None,
        };
        let mut scanner = Scanner::new(out, self.limits.nesting);
        scanner.begin_object()?;
        let (Some(node), Some(iter)) = (node, iter) else {
            // Still checked: a line that does not read stops the values, whether or not it fits.
            let mut first = true;
            while scanner.key(&mut first)?.is_some() {
                scanner.value()?;
            }
            scanner.end()?;
            self.ignore();
            return Ok(());
        };
        let mut ignored = false;
        if skipped {
            let key = (node, Iter::from(iter.as_slice()));
            if self.trace.skipped.contains(&key) {
                ignored = true;
            } else {
                self.kept = self.kept.saturating_add(iter.len() * 8 + ENTRY_COST);
                self.trace.skipped.insert(key);
            }
        }
        let mut first = true;
        while let Some(name) = scanner.key(&mut first)? {
            let value = scanner.value()?;
            let wire = self.wires.get(node as usize).and_then(|wires| wires.get(name.as_ref())).copied();
            let expected = wire.and_then(|wire| self.expected.nodes.get(node as usize)?.wires.get(wire as usize));
            let ty = own.get(name.as_ref()).or_else(|| self.types.get(name.as_ref()));
            if ty.is_none() && types_cut && wire.is_some() {
                continue;
            }
            let fits = matches!((expected, ty), (Some(expected), Some(ty)) if *expected.ty == **ty) && value.len() <= VALUE_BUDGET;
            let Some(wire) = wire.filter(|_| fits) else {
                ignored = true;
                continue;
            };
            let key = (node, wire, Iter::from(iter.as_slice()));
            if self.trace.values.contains_key(&key) {
                ignored = true;
                continue;
            }
            let Some(span) = self.trace.store(value).filter(|_| self.keep(value.len() + iter.len() * 8 + ENTRY_COST)) else {
                self.stop(self.line, Limit::Kept);
                return Ok(());
            };
            self.trace.values.insert(key, span);
        }
        scanner.end()?;
        self.count_iterations(node, &iter);
        if self.kept > self.limits.kept {
            self.stop(self.line, Limit::Kept);
        }
        if ignored {
            self.ignore();
        }
        Ok(())
    }

    fn arm_event(&mut self, fields: &Fields<'_>) -> Result<(), Bad> {
        let name = fields.node.as_deref().ok_or(Bad)?;
        let arm = fields.arm.as_deref().ok_or(Bad)?;
        let iter = self.read_iter(fields.iter)?;
        let node = match (&iter, self.frame0) {
            (Some(iter), true) => self.resolve(name, iter),
            _ => None,
        };
        let (Some(node), Some(iter)) = (node, iter) else {
            self.ignore();
            return Ok(());
        };
        let key = (node, Iter::from(iter.as_slice()));
        if self.trace.arms.contains_key(&key) {
            self.ignore();
            return Ok(());
        }
        let Some(span) = self.trace.store(arm).filter(|_| self.keep(arm.len() + iter.len() * 8 + ENTRY_COST)) else {
            self.stop(self.line, Limit::Kept);
            return Ok(());
        };
        self.trace.arms.insert(key, span);
        self.activate(node, arm, &iter);
        self.count_iterations(node, &iter);
        if self.kept > self.limits.kept {
            self.stop(self.line, Limit::Kept);
        }
        Ok(())
    }
}

fn once(seen: bool) -> Result<(), Bad> {
    if seen { Err(Bad) } else { Ok(()) }
}

/// A field given twice does not read.
fn set<T>(slot: &mut Option<T>, value: T) -> Result<(), Bad> {
    if slot.is_some() {
        return Err(Bad);
    }
    *slot = Some(value);
    Ok(())
}

/// The first line: `Err(Bad)` when it does not read, a problem when it names another schema or version. The schema is
/// judged first, so that a later version of another shape is said to be one.
fn read_header(text: &str, nesting: usize) -> Result<Result<TraceHeader, HeaderProblem>, Bad> {
    let mut scanner = Scanner::new(text, nesting);
    scanner.begin_object()?;
    let mut fields: HashMap<std::borrow::Cow<'_, str>, &str> = HashMap::new();
    let mut first = true;
    while let Some(key) = scanner.key(&mut first)? {
        let value = scanner.value()?;
        if fields.insert(key, value).is_some() {
            return Err(Bad);
        }
    }
    scanner.end()?;
    let string = |key: &str| -> Result<String, Bad> {
        let mut scanner = Scanner::new(fields.get(key).ok_or(Bad)?, nesting);
        let value = scanner.string()?.into_owned();
        scanner.end()?;
        Ok(value)
    };
    let schema = string("schema")?;
    if schema != SCHEMA {
        let later = schema.strip_prefix("labcraft-trace/").is_some_and(|version| {
            !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit()) && version.parse::<u64>().map_or(true, |version| version > 1)
        });
        return Ok(Err(if later { HeaderProblem::LaterVersion } else { HeaderProblem::OtherSchema }));
    }
    let truncated = match fields.get("truncated") {
        Some(value) => Scanner::new(value, nesting).bool()?,
        None => false,
    };
    let mut files = Vec::new();
    let mut scanner = Scanner::new(fields.get("files").ok_or(Bad)?, nesting);
    scanner.begin_array()?;
    let mut first_file = true;
    while scanner.element(&mut first_file)? {
        scanner.begin_object()?;
        let (mut file, mut sha) = (None, None);
        let mut first_key = true;
        while let Some(key) = scanner.key(&mut first_key)? {
            match key.as_ref() {
                "file" => set(&mut file, scanner.string()?.into_owned())?,
                "sha256" => set(&mut sha, scanner.string()?.into_owned())?,
                _ => {
                    scanner.value()?;
                }
            }
        }
        files.push((file.ok_or(Bad)?, sha.ok_or(Bad)?));
    }
    scanner.end()?;
    Ok(Ok(TraceHeader { run: string("run")?, function: string("function")?, mode: string("mode")?, files, truncated }))
}

#[cfg(test)]
mod tests;
