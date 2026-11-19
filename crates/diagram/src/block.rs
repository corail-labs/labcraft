//! The terminals of a compact block that the `terminals` section of its function places (spec 11.2, visual notation
//! 3.2): the lines of the section first, then each terminal that no line names on its default side, on the first free
//! place from the top, the `err` pair on the last place. The block grows down by two places while its places do not hold
//! its terminals. A block without a section is drawn as notation 3.2 says, by `size`.

use std::collections::{HashMap, HashSet};

use labcraft_format::block::{DEFAULT_TOP, MAX_PLACES, Place, Section, odd_when_middle, place_number};
use labcraft_graph::{BandPlace, BlockSide};
use labcraft_types::Interner;

use crate::graph::{Base, Family, Node, Terminal};
use crate::size::{Line, lines};

/// The places are 8 apart on every block (spec 11.2): a constant of the notation, never a value of the style.
pub const PITCH: i64 = 800;

/// A line of a section that applies, by the name of its terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub side: BlockSide,
    pub place: Place,
    pub output: bool,
    pub name: String,
    /// A pass-through pair named alone on the left or the right: its other terminal goes to the same place of the facing side.
    pub partner: bool,
}

/// What the section of a function says, for every call to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The places of the sides as the section counts them (`labcraft_format::block::Section`).
    pub top: u32,
    pub left: Option<u32>,
    pub middle_sides: bool,
    pub band: Option<BandPlace>,
    pub lines: Vec<Placed>,
    /// The inputs with a default, which go to the top side when the left one is full.
    pub optional: HashSet<String>,
}

impl Block {
    pub fn of(section: &Section, names: &Interner, optional: HashSet<String>) -> Block {
        let lines = section
            .lines
            .iter()
            .map(|line| Placed {
                side: line.side,
                place: line.place,
                output: line.terminal.output,
                name: names.resolve(line.terminal.name).to_owned(),
                partner: line.partner,
            })
            .collect();
        Block { top: section.top_places, left: section.left_places, middle_sides: section.middle_sides, band: section.band, lines, optional }
    }
}

/// Where the terminals of one node go: the places of its sides, and for each input and output its side and its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    pub top: u32,
    pub left: u32,
    pub inputs: Vec<(BlockSide, u32)>,
    pub outputs: Vec<(BlockSide, u32)>,
    /// The places that the lines of the section hold, which a terminal moved by hand cannot take (commands.md 9.4).
    pub held: Vec<(BlockSide, u32)>,
}

impl Places {
    pub fn width(&self) -> i64 {
        (i64::from(self.top) + 1) * PITCH
    }

    pub fn height(&self) -> i64 {
        (i64::from(self.left) + 1) * PITCH
    }

    /// The centre of a terminal on a place, from the top-left corner of the block.
    pub fn at(&self, (side, place): (BlockSide, u32)) -> (i64, i64) {
        let along = i64::from(place) * PITCH;
        match side {
            BlockSide::Top => (along, 0),
            BlockSide::Left => (0, along),
            BlockSide::Right => (self.width(), along),
        }
    }

    /// Whether a terminal is on the top side: the band then stays inside, and the label goes below (notation 3.2).
    pub fn has_top_side(&self) -> bool {
        self.inputs.iter().chain(&self.outputs).any(|(side, _)| *side == BlockSide::Top)
    }
}

/// The places of the terminals of `node`, whose function has the section `block`.
pub fn places(block: &Block, node: &Node<'_>) -> Places {
    let (top, middle) = (block.top, block.middle_sides);
    let lines = lines(&node.inputs, &node.outputs);
    // The terminals that the lines of the section name, by their index in the node, found once for every attempt.
    let (by_input, by_output) = (by_name(&node.inputs), by_name(&node.outputs));
    let named: Vec<Named> = block
        .lines
        .iter()
        .filter_map(|line| {
            let (own, other) = if line.output { (&by_output, &by_input) } else { (&by_input, &by_output) };
            let terminal = *own.get(line.name.as_str())?;
            let partner = if line.partner { other.get(line.name.as_str()).copied() } else { None };
            Some(Named { line, terminal, partner })
        })
        .collect();
    let numbered = block
        .lines
        .iter()
        .filter(|line| line.side != BlockSide::Top)
        .filter_map(|line| match line.place {
            Place::Number(number) => Some(number),
            Place::Last | Place::Middle => None,
        })
        .max()
        .unwrap_or(0);
    // Without `left`, the places that the terminals need, as notation 3.2 gives them, and at least the highest a line names.
    let needed = u32::try_from(lines.len()).unwrap_or(u32::MAX).max(numbered);
    let start = block.left.unwrap_or_else(|| {
        // A block of 40 has four places, and grows by two (notation 3.2).
        let mut places = DEFAULT_TOP;
        while places < needed {
            places = places.saturating_add(2);
        }
        places
    });
    let left = odd_when_middle(start, middle);
    // Two places at a time, until they hold: the first size that holds, found by doubling the steps then halving them, so
    // that a signature of many terminals costs a few attempts, not one per step. A side never needs more places than the
    // terminals and the highest line.
    let most = u32::try_from(node.inputs.len().saturating_add(node.outputs.len())).unwrap_or(u32::MAX).saturating_add(MAX_PLACES).saturating_mul(2);
    let at = |steps: u32| left.saturating_add(steps.saturating_mul(2));
    if let Some(found) = attempt(block, &named, node, &lines, top, left) {
        return found;
    }
    let (mut failed, mut steps) = (0_u32, 1_u32);
    let mut found = loop {
        if let Some(found) = attempt(block, &named, node, &lines, top, at(steps)) {
            break found;
        }
        if at(steps) >= most {
            // A bound the terminals never reach: what holds is drawn by the defaults of `size`.
            return Places { top, left, inputs: Vec::new(), outputs: Vec::new(), held: Vec::new() };
        }
        failed = steps;
        steps = steps.saturating_mul(2);
    };
    // The first that holds between the last that does not and the first found that does.
    let mut held = steps;
    while held - failed > 1 {
        let middle_steps = failed + (held - failed) / 2;
        match attempt(block, &named, node, &lines, top, at(middle_steps)) {
            Some(places) => {
                held = middle_steps;
                found = places;
            }
            None => failed = middle_steps,
        }
    }
    found
}

/// Free places by side, from 1. The defaults take the first free place from the top, and a place once taken stays so:
/// a cursor by side, and one for both the left and the right, keep each search linear in all.
struct Free {
    top: u32,
    left: u32,
    taken: HashSet<(BlockSide, u32)>,
    /// Below each, no place is free: on the top side, the left side, the right side, and both of the last two.
    from_top: u32,
    from_left: u32,
    from_right: u32,
    from_both: u32,
}

/// A line of the section, with the index in the node of its terminal and of the other terminal of its pair that follows it.
struct Named<'b> {
    line: &'b Placed,
    terminal: usize,
    partner: Option<usize>,
}

impl Free {
    fn count(&self, side: BlockSide) -> u32 {
        if side == BlockSide::Top { self.top } else { self.left }
    }

    fn is_free(&self, side: BlockSide, place: u32) -> bool {
        place >= 1 && place <= self.count(side) && !(side == BlockSide::Top && place == 1) && !self.taken.contains(&(side, place))
    }

    fn take(&mut self, side: BlockSide, place: u32) {
        self.taken.insert((side, place));
    }

    fn first(&mut self, side: BlockSide) -> Option<u32> {
        let mut place = match side {
            BlockSide::Top => self.from_top,
            BlockSide::Left => self.from_left,
            BlockSide::Right => self.from_right,
        };
        while place <= self.count(side) && !self.is_free(side, place) {
            place += 1;
        }
        match side {
            BlockSide::Top => self.from_top = place,
            BlockSide::Left => self.from_left = place,
            BlockSide::Right => self.from_right = place,
        }
        (place <= self.count(side)).then_some(place)
    }

    fn both_free(&self, place: u32) -> bool {
        self.is_free(BlockSide::Left, place) && self.is_free(BlockSide::Right, place)
    }

    fn first_of_both(&mut self) -> Option<u32> {
        let mut place = self.from_both;
        while place <= self.left && !self.both_free(place) {
            place += 1;
        }
        self.from_both = place;
        (place <= self.left).then_some(place)
    }

    /// The last free place of a side, or of both the left and the right with none.
    fn last(&self, side: Option<BlockSide>) -> Option<u32> {
        (1..=self.left).rev().find(|&place| match side {
            Some(side) => self.is_free(side, place),
            None => self.both_free(place),
        })
    }
}

fn attempt(block: &Block, named: &[Named<'_>], node: &Node<'_>, lines: &[Line], top: u32, left: u32) -> Option<Places> {
    let mut free = Free { top, left, taken: HashSet::new(), from_top: 1, from_left: 1, from_right: 1, from_both: 1 };
    let mut inputs: Vec<Option<(BlockSide, u32)>> = vec![None; node.inputs.len()];
    let mut outputs: Vec<Option<(BlockSide, u32)>> = vec![None; node.outputs.len()];
    // The lines of the section, in order: one that the block cannot hold now leaves its terminal to the defaults.
    for &Named { line, terminal, partner } in named {
        let place = place_number(line.place, free.count(line.side));
        if !free.is_free(line.side, place) {
            continue;
        }
        free.take(line.side, place);
        let (own, other) = if line.output { (&mut outputs, &mut inputs) } else { (&mut inputs, &mut outputs) };
        set(own, terminal, (line.side, place));
        if let (Some(partner), Some(facing)) = (partner, line.side.facing())
            && other.get(partner).is_some_and(Option::is_none)
            && free.is_free(facing, place)
        {
            free.take(facing, place);
            set(other, partner, (facing, place));
        }
    }
    // What the lines hold, before the defaults take theirs.
    let lines_hold: HashSet<(BlockSide, u32)> = free.taken.clone();
    // The defaults: the `err` line on the last place, then the lines of notation 3.1 in order, a pair on one place.
    let error = |line: &Line| {
        line.0.and_then(|input| node.inputs.get(input)).is_some_and(|terminal| terminal.family.base == Base::Error)
            || line.1.and_then(|output| node.outputs.get(output)).is_some_and(|terminal| terminal.family.base == Base::Error)
    };
    let free_in = |line: &Line, inputs: &[Option<(BlockSide, u32)>]| line.0.filter(|index| inputs.get(*index).is_some_and(Option::is_none));
    let free_out = |line: &Line, outputs: &[Option<(BlockSide, u32)>]| line.1.filter(|index| outputs.get(*index).is_some_and(Option::is_none));
    let mut order: Vec<&Line> = lines.iter().collect();
    let last_error = lines.last().filter(|line| error(line));
    if let Some(last) = last_error {
        order.pop();
        match (free_in(last, &inputs), free_out(last, &outputs)) {
            (Some(input), Some(output)) => {
                let place = free.last(None)?;
                free.take(BlockSide::Left, place);
                free.take(BlockSide::Right, place);
                set(&mut inputs, input, (BlockSide::Left, place));
                set(&mut outputs, output, (BlockSide::Right, place));
            }
            (Some(input), None) => {
                let place = free.last(Some(BlockSide::Left))?;
                free.take(BlockSide::Left, place);
                set(&mut inputs, input, (BlockSide::Left, place));
            }
            (None, Some(output)) => {
                let place = free.last(Some(BlockSide::Right))?;
                free.take(BlockSide::Right, place);
                set(&mut outputs, output, (BlockSide::Right, place));
            }
            (None, None) => {}
        }
    }
    for line in order {
        let (input, output) = (free_in(line, &inputs), free_out(line, &outputs));
        if let (Some(input), Some(output)) = (input, output)
            && is_pair(node.inputs.get(input), node.outputs.get(output))
        {
            let place = free.first_of_both()?;
            free.take(BlockSide::Left, place);
            free.take(BlockSide::Right, place);
            set(&mut inputs, input, (BlockSide::Left, place));
            set(&mut outputs, output, (BlockSide::Right, place));
            continue;
        }
        if let Some(input) = input {
            let at = match free.first(BlockSide::Left) {
                Some(place) => (BlockSide::Left, place),
                // An optional input goes to the top side when the left one is full (11.2).
                None if node.inputs.get(input).is_some_and(|terminal| block.optional.contains(&terminal.name)) => {
                    (BlockSide::Top, free.first(BlockSide::Top)?)
                }
                None => return None,
            };
            free.take(at.0, at.1);
            set(&mut inputs, input, at);
        }
        if let Some(output) = output {
            let place = free.first(BlockSide::Right)?;
            free.take(BlockSide::Right, place);
            set(&mut outputs, output, (BlockSide::Right, place));
        }
    }
    let mut held: Vec<(BlockSide, u32)> = lines_hold.into_iter().collect();
    held.sort_by_key(|(side, place)| (side.word(), *place));
    Some(Places {
        top,
        left,
        inputs: inputs.into_iter().collect::<Option<Vec<_>>>()?,
        outputs: outputs.into_iter().collect::<Option<Vec<_>>>()?,
        held,
    })
}

/// Each terminal by its name, the first of a name.
fn by_name(list: &[Terminal]) -> HashMap<&str, usize> {
    let mut found = HashMap::with_capacity(list.len());
    for (at, terminal) in list.iter().enumerate() {
        found.entry(terminal.name.as_str()).or_insert(at);
    }
    found
}

fn set(list: &mut [Option<(BlockSide, u32)>], index: usize, at: (BlockSide, u32)) {
    if let Some(slot) = list.get_mut(index) {
        *slot = Some(at);
    }
}

/// A pass-through pair (notation 3.1): an input and an output of one family that passes through.
fn is_pair(input: Option<&Terminal>, output: Option<&Terminal>) -> bool {
    match (input, output) {
        (Some(input), Some(output)) => {
            input.family.passes_through()
                && output.family.passes_through()
                && (input.ty == output.ty || (input.family == Family::ERROR && output.family == Family::ERROR))
        }
        _ => false,
    }
}
