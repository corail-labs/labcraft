//! Nodes dragged over another frame (notation 2.4, "Moving nodes into another frame"; commands.md 6.7, `node.reframe`):
//! while the drag lasts, the canvas outlines the frame under the pointer and says why it refuses the nodes, from the
//! drawing alone, as `node.reframe` would; the command judges it again at the drop (architecture 12.6). What crosses an
//! edge is judged as a wire is (`crate::wiring`): each value that the nodes read or make, from where it is made to where
//! it is read once they moved.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::ports::{Refusal, Values, chain, is_plain, out_of_scope};
use crate::shape::{Drawing, Holds, Port};

/// What a frame would do with nodes dropped into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reframing {
    /// It takes them. `leaving`: each value that would leave a loop, which `through` of `node.reframe` takes out.
    Takes { leaving: Vec<Leaving> },
    /// It refuses them, as `node.reframe` would.
    Refused(Refusal),
}

/// A value that the move takes out of loops (commands.md 6.7, `through`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaving {
    /// Its name, as `through` names it.
    pub value: String,
    /// The port that gives it and one that reads it outside the loops, by their index in [`Drawing::ports`]: their types
    /// choose the passages (notation 2.5).
    pub source: usize,
    pub reader: usize,
    /// The bodies of the loops it leaves, from the inside out.
    pub exits: Vec<usize>,
}

/// Where a value is made or read once the nodes moved: a body, and the node of it, none for a terminal of its frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spot {
    body: usize,
    node: Option<usize>,
}

/// What a read takes: a value by the port that gives it, or the node that an order wire waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Port(usize),
    Node(usize),
}

/// A read: the port that reads (an input, the `after:` of a node, a part of a frame), and what it reads.
struct Read {
    reader: usize,
    source: Source,
    order: bool,
}

/// What the frame `body` would do with `nodes`, by their index in [`Drawing::placed`], all of one frame, moved into it.
pub fn reframing(drawing: &Drawing, nodes: &[usize], body: usize) -> Reframing {
    let refused = |code: &'static str, message: String| Reframing::Refused(Refusal { code, message });
    let moved: HashSet<usize> = nodes.iter().copied().collect();
    let Some(from) = nodes.first().and_then(|&node| drawing.placed.get(node)).map(|placed| placed.body) else {
        return refused("params.invalid", "no node to move".to_owned());
    };
    let Some(into) = drawing.bodies.get(body).filter(|_| body != from) else {
        return refused("params.invalid", "the nodes are in that frame already".to_owned());
    };
    // A command names a frame by its structure (commands.md 9.2).
    if into.owner.and_then(|owner| drawing.placed.get(owner)).is_some_and(|structure| structure.name.is_empty()) {
        return refused("params.invalid", "this structure has no name that a command gives it: give it a label first".to_owned());
    }
    // What `node.reframe` does not move (commands.md 6.7, R-6).
    for &node in nodes {
        let Some(placed) = drawing.placed.get(node) else { continue };
        let named = if placed.name.is_empty() { "a node" } else { placed.name.as_str() };
        if !placed.holds.is_empty() {
            return refused(
                "params.invalid",
                format!("`{named}` is a structure: moving a structure into another frame is not done yet; move the nodes it holds"),
            );
        }
        // A `next x = …`: its wire goes to the `next` of the loop, and bears no value of its own (a node that `next x = y`
        // reads gives `y`).
        let carried = drawing.groups.iter().filter_map(|group| group.wire.as_ref()).any(|wire| {
            wire.from_node.as_deref() == Some(placed.key.as_str())
                && wire.to.iter().any(|to| to.starts_with("next."))
                && !drawing.ports.iter().any(|port| port.placed == Some(node) && !port.input && !port.frame && port.name == wire.value)
        });
        if carried {
            return refused("params.invalid", "a `next`, a `collect`, a `last` or an `until` stays in its loop body (R-6)".to_owned());
        }
    }
    let of_moved = |port: &Port| !port.frame && port.placed.is_some_and(|placed| moved.contains(&placed));
    let at = |placed: usize| if moved.contains(&placed) { body } else { drawing.placed.get(placed).map_or(0, |found| found.body) };
    // A value of the same name in the new frame.
    let made: Vec<&Port> = drawing.ports.iter().filter(|port| of_moved(port) && !port.input && is_plain(&port.name)).collect();
    let bound = Bound::of(drawing);
    if let Some(port) = made.iter().find(|port| bound.binds(body, &port.name)) {
        return refused("name.taken", format!("`{}` is a value of that frame already: rename one of them, then move the node", port.name));
    }
    // In the body of the function, a value named as an output gives that output (R-2).
    let output = |name: &str| drawing.ports.iter().any(|port| port.frame && port.input && port.placed.is_none() && port.caption == name);
    if let Some(port) = made.iter().find(|port| body == 0 && output(&port.name)) {
        return refused(
            "name.taken",
            format!("`{}` is an output of the function: in its body the node would give it (R-2); rename the value first", port.name),
        );
    }
    let reads = reads(drawing, &bound);
    let spot_of_port = |port: &Port| match port.placed {
        Some(placed) if !port.frame => Spot { body: at(placed), node: Some(placed) },
        _ => Spot { body: port.body, node: None },
    };
    let spot_of_source = |source: Source| match source {
        Source::Node(node) => Some(Spot { body: at(node), node: Some(node) }),
        Source::Port(port) => drawing.ports.get(port).map(spot_of_port),
    };
    let moves = |spot: Spot| spot.node.is_some_and(|node| moved.contains(&node));
    let others = Values::except(drawing, &of_moved);
    let made_names: HashSet<&str> = made.iter().map(|port| port.name.as_str()).collect();
    let mut leaving: BTreeMap<String, Leaving> = BTreeMap::new();
    let mut edges: Vec<(usize, usize, bool)> = Vec::new();
    // `node.reframe` judges the order wires last, after the cycles.
    let mut order_refused = None;
    for read in &reads {
        let Some(reader) = drawing.ports.get(read.reader) else { continue };
        let Some(source) = spot_of_source(read.source) else { continue };
        let target = spot_of_port(reader);
        let crossing = moves(source) != moves(target);
        if crossing && read.order {
            // An order wire does not leave a structure (R-17): the node waited for is in a body around the waiter, not
            // around it.
            let seen = chain(drawing, target.body).contains(&source.body)
                && source.node.is_none_or(|node| lifted(drawing, &at, target, source.body) != Some(node));
            if !seen && order_refused.is_none() {
                order_refused = Some(refused(
                    "wire.out_of_scope",
                    "an order wire (`after:`) would cross the edge of a structure, which it does not (R-17)".to_owned(),
                ));
            }
        } else if crossing && let Source::Port(given) = read.source {
            let Some(found) = drawing.ports.get(given) else { continue };
            let (mut from_port, mut to_port) = (found.clone(), reader.clone());
            (from_port.body, to_port.body) = (source.body, target.body);
            match out_of_scope(drawing, &others, &from_port, &to_port) {
                Err(refusal) => return Reframing::Refused(refusal),
                Ok(exits) if !exits.is_empty() => {
                    let longer = leaving.get(&found.name).is_none_or(|known| known.exits.len() < exits.len());
                    if longer {
                        leaving.insert(found.name.clone(), Leaving { value: found.name.clone(), source: given, reader: read.reader, exits });
                    }
                }
                Ok(_) => {}
            }
        } else if !crossing
            && !moves(source)
            && let Source::Port(given) = read.source
            && let Some(found) = drawing.ports.get(given)
            && made_names.contains(found.name.as_str())
        {
            // A value that the nodes moved would hide from what reads it (R-3).
            let bodies = chain(drawing, target.body);
            let (new, old) = (bodies.iter().position(|&found| found == body), bodies.iter().position(|&found| found == source.body));
            if let (Some(new), Some(old)) = (new, old)
                && new < old
            {
                return refused(
                    "wire.out_of_scope",
                    format!("in that frame, the node moved would hide `{}` from what reads it: rename one of them first", found.name),
                );
            }
        }
        // What depends on what once the nodes moved, in the body that both ends see: by the values, as `node.reframe`
        // judges the cycles (an order wire that crosses an edge is refused after them).
        let around = chain(drawing, target.body);
        let common = chain(drawing, source.body).into_iter().find(|found| around.contains(found));
        if !read.order
            && let Some(common) = common
            && let (Some(from_node), Some(to_node)) = (lifted(drawing, &at, source, common), lifted(drawing, &at, target, common))
        {
            edges.push((from_node, to_node, moves(source) || moves(target)));
        }
    }
    // A move that makes a structure read what it gives, or closes a cycle otherwise.
    let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(from_node, to_node, _) in &edges {
        next.entry(from_node).or_default().push(to_node);
    }
    if edges.iter().any(|&(from_node, to_node, changed)| changed && (from_node == to_node || reaches(&next, to_node, from_node))) {
        return refused("wire.cycle", "the move would close a cycle: a structure would read what it gives (spec 6.2)".to_owned());
    }
    if let Some(refusal) = order_refused {
        return refusal;
    }
    Reframing::Takes { leaving: leaving.into_values().collect() }
}

/// The node of `body` that holds `spot` once the nodes moved: its node, or the structure around it there; none for a
/// terminal of the frame of `body` itself.
fn lifted(drawing: &Drawing, at: &dyn Fn(usize) -> usize, spot: Spot, body: usize) -> Option<usize> {
    if spot.body == body {
        return spot.node;
    }
    let mut inner = spot.body;
    for _ in 0..=drawing.bodies.len() {
        let owner = drawing.bodies.get(inner)?.owner?;
        let outer = at(owner);
        if outer == body {
            return Some(owner);
        }
        inner = outer;
    }
    None
}

/// Whether `to` is reached from `from` by `next`.
fn reaches(next: &HashMap<usize, Vec<usize>>, from: usize, to: usize) -> bool {
    let mut seen = HashSet::new();
    let mut pending = VecDeque::from([from]);
    while let Some(node) = pending.pop_front() {
        if node == to {
            return true;
        }
        if seen.insert(node) {
            pending.extend(next.get(&node).into_iter().flatten().copied());
        }
    }
    false
}

/// The values that each body binds, by their ports: those where a wire starts, notches left out (they stand for a value
/// of a body around). As `crate::ports::Values`, by the index of the port, which a read keeps.
struct Bound<'a> {
    by_name: HashMap<(usize, &'a str), usize>,
}

impl<'a> Bound<'a> {
    fn of(drawing: &'a Drawing) -> Bound<'a> {
        let mut by_name = HashMap::new();
        for (index, port) in drawing.ports.iter().enumerate().filter(|(_, port)| !port.input && !port.notch && !port.name.is_empty()) {
            by_name.entry((port.body, port.name.as_str())).or_insert(index);
        }
        Bound { by_name }
    }

    fn binds(&self, body: usize, name: &str) -> bool {
        self.by_name.contains_key(&(body, name))
    }

    /// The port of the value that `name` names in `body`: the nearest body around that binds it.
    fn resolve(&self, drawing: &Drawing, body: usize, name: &str) -> Option<usize> {
        chain(drawing, body).into_iter().find_map(|found| self.by_name.get(&(found, name)).copied())
    }
}

/// Every read of the drawing: each input that a wire reaches, by the value it reads; each order wire, by the node it
/// waits for, or the value when no node makes it.
fn reads(drawing: &Drawing, bound: &Bound<'_>) -> Vec<Read> {
    let mut found: Vec<Read> = drawing
        .ports
        .iter()
        .enumerate()
        .filter(|(_, port)| port.input && !port.order && port.holds == Holds::Wire && !port.value.is_empty())
        .filter_map(|(reader, port)| {
            let source = bound.resolve(drawing, port.body, &port.value)?;
            Some(Read { reader, source: Source::Port(source), order: false })
        })
        .collect();
    let keys: HashMap<&str, usize> = drawing.placed.iter().enumerate().map(|(at, placed)| (placed.key.as_str(), at)).collect();
    // The inputs of each node, by their ports.
    let mut inputs: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, port) in drawing.ports.iter().enumerate().filter(|(_, port)| port.input && !port.frame) {
        if let Some(placed) = port.placed {
            inputs.entry(placed).or_default().push(index);
        }
    }
    for wire in drawing.groups.iter().filter_map(|group| group.wire.as_ref()) {
        for waiter in wire.to_nodes.iter().filter_map(|key| keys.get(key.as_str()).copied()) {
            let of_waiter = inputs.get(&waiter).map(Vec::as_slice).unwrap_or_default();
            let port = |index: &usize| drawing.ports.get(*index);
            // A node that reads the value is wired by it: only an order wire reaches an `after:` alone.
            let reads_it = of_waiter.iter().filter_map(port).any(|port| !port.order && port.value == wire.value);
            let Some(&reader) = of_waiter.iter().find(|index| port(index).is_some_and(|port| port.order)) else { continue };
            if reads_it {
                continue;
            }
            let source = match wire.from_node.as_deref().and_then(|key| keys.get(key).copied()) {
                Some(node) => Source::Node(node),
                None => match bound.resolve(drawing, wire.body, &wire.value) {
                    Some(port) => Source::Port(port),
                    None => continue,
                },
            };
            found.push(Read { reader, source, order: true });
        }
    }
    found
}
