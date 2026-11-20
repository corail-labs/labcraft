//! The places of the terminals of a compact block (spec 11.2, visual notation 3.1, 3.2), worked out once for `check`,
//! which judges the lines of a `terminals` section at the size that the block takes, and for the drawing, which places
//! the terminals there: the lines of the section first, then each terminal that no line names on its default side, on
//! the first free place from the top, the `err` pair on the last place. The block grows down by two places while its
//! places do not hold its terminals.

use std::collections::{HashMap, HashSet, VecDeque};

use labcraft_format::block::{DEFAULT_TOP, MAX_PLACES, Place, Section, odd_when_middle, place_number, read as read_section};
use labcraft_graph::{BandPlace, BlockSide, Terminals as TerminalsSection};
use labcraft_types::{Interner, LibraryId, Name, Ty, TyId, TypeTable};

use crate::declare::{DeclaredFunction, DeclaredLibrary};
use crate::mask::Mask;
use crate::outline::Read;

/// The places are 8 apart on every block (spec 11.2): a constant of the notation, never a value of the style.
pub const PITCH: i64 = 800;

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

/// The library of an id among those declared.
pub fn library_named(libraries: &[DeclaredLibrary], id: LibraryId) -> Option<&DeclaredLibrary> {
    libraries.iter().find(|library| library.id == id)
}

/// A terminal as its place needs it: its name, its type, which pairs it with another of the same, its family, and whether
/// it anchors a coordinate of an outline (spec 11.4: a value of a variadic parameter does not).
pub trait Slot {
    fn name(&self) -> &str;
    fn ty(&self) -> TyId;
    fn family(&self) -> Family;
    fn anchor(&self) -> bool;
}

/// A terminal of a declared signature at one size.
pub struct Signed<'n> {
    pub name: &'n str,
    pub ty: TyId,
    pub family: Family,
    pub anchor: bool,
}

impl Slot for Signed<'_> {
    fn name(&self) -> &str {
        self.name
    }

    fn ty(&self) -> TyId {
        self.ty
    }

    fn family(&self) -> Family {
        self.family
    }

    fn anchor(&self) -> bool {
        self.anchor
    }
}

/// The terminals of a declared function, its inputs then its outputs, at the size that `values` gives its variadic
/// parameters, in order (one value each without it), and the names of its inputs with a default.
pub fn signed<'n>(
    function: &DeclaredFunction,
    names: &'n Interner,
    types: &TypeTable,
    libraries: &[DeclaredLibrary],
    values: Option<&[u32]>,
) -> (Vec<Signed<'n>>, Vec<Signed<'n>>, HashSet<String>) {
    let slot = |name: Name, ty: TyId, anchor: bool| Signed { name: names.resolve(name), ty, family: family(types, libraries, ty), anchor };
    let mut counts = values.unwrap_or_default().iter();
    let mut inputs = Vec::with_capacity(function.params.len());
    for param in &function.params {
        let count = if param.variadic && values.is_some() { counts.next().copied().unwrap_or(1) } else { 1 };
        for _ in 0..count {
            inputs.push(slot(param.name, param.ty, !param.variadic));
        }
    }
    let outputs = function.outputs.iter().map(|output| slot(output.name, output.ty, true)).collect();
    let optional = function.params.iter().filter(|param| param.has_default).map(|param| names.resolve(param.name).to_owned()).collect();
    (inputs, outputs, optional)
}

/// What places the terminals of a block: the `terminals` section of its function, or the defaults of 11.2 without one,
/// which an outline counts its `w` and `h` from (11.4).
pub fn section_of(terminals: Option<&TerminalsSection>, inputs: &[Name], outputs: &[Name], names: &Interner) -> Section {
    match terminals {
        Some(terminals) => read_section(terminals, inputs, outputs, names).0,
        None => Section { top_places: DEFAULT_TOP, ..Section::default() },
    }
}

/// A line of terminals: at most one input and one output (notation 3.1), by their index.
pub type Line = (Option<usize>, Option<usize>);

/// The lines of a node, from the top, as notation 3.1 makes them: pass-through pairs on one line, the `err` pair on the
/// bottom one, the others side by side in signature order between the pairs.
pub fn lines<T: Slot>(inputs: &[T], outputs: &[T]) -> Vec<Line> {
    // `err` is made last in both lists: the input named `err`, else the last error input; its output by name, else the
    // last error output.
    let error_input = inputs
        .iter()
        .position(|terminal| terminal.name() == "err" && terminal.family().base == Base::Error)
        .or_else(|| inputs.iter().rposition(|terminal| terminal.family() == Family::ERROR));
    let error_output = outputs
        .iter()
        .position(|terminal| terminal.name() == "err" && terminal.family().base == Base::Error)
        .or_else(|| outputs.iter().rposition(|terminal| terminal.family() == Family::ERROR));
    let last = |count: usize, moved: Option<usize>| -> Vec<usize> {
        let mut order: Vec<usize> = (0..count).filter(|&index| Some(index) != moved).collect();
        order.extend(moved);
        order
    };
    let ins = last(inputs.len(), error_input);
    let outs = last(outputs.len(), error_output);
    // Pairs by name first, then in order among what is left, of one type and a family that passes through. Each input
    // takes the first output not taken of its name and type, then of its type: queues of the outputs that pass through,
    // in order, from which a taken one is dropped when it comes first, so that pairing is linear.
    let mut paired_in = vec![None; ins.len()];
    let mut taken = vec![false; outs.len()];
    let mut by_name: HashMap<(&str, TyId), VecDeque<usize>> = HashMap::new();
    let mut by_type: HashMap<TyId, VecDeque<usize>> = HashMap::new();
    // Every error has one type: an error input that no argument wires, which has the family of its table and no type yet
    // (a primitive), pairs as one that has it.
    let key = |terminal: &T| if terminal.family() == Family::ERROR { TyId::ERROR } else { terminal.ty() };
    for (other, &output) in outs.iter().enumerate() {
        let Some(out) = outputs.get(output).filter(|out| out.family().passes_through()) else { continue };
        by_name.entry((out.name(), key(out))).or_default().push_back(other);
        by_type.entry(key(out)).or_default().push_back(other);
    }
    for named in [true, false] {
        for (slot, &input) in ins.iter().enumerate() {
            let Some(terminal) = inputs.get(input).filter(|terminal| terminal.family().passes_through()) else { continue };
            if paired_in.get(slot).copied().flatten().is_some() {
                continue;
            }
            let queue = if named { by_name.get_mut(&(terminal.name(), key(terminal))) } else { by_type.get_mut(&key(terminal)) };
            let Some(queue) = queue else { continue };
            while queue.front().is_some_and(|&other| taken.get(other).copied().unwrap_or(true)) {
                queue.pop_front();
            }
            if let (Some(found), Some(pair)) = (queue.pop_front(), paired_in.get_mut(slot)) {
                *pair = Some(found);
                if let Some(flag) = taken.get_mut(found) {
                    *flag = true;
                }
            }
        }
    }
    // A pair that crosses one before it, in signature order, is not kept.
    let mut kept: Vec<(usize, usize)> = Vec::new();
    for (slot, pair) in paired_in.iter().enumerate() {
        if let Some(output) = *pair
            && kept.last().is_none_or(|&(_, before)| output > before)
        {
            kept.push((slot, output));
        }
    }
    let mut found = Vec::new();
    let (mut next_in, mut next_out) = (0, 0);
    let fill = |found: &mut Vec<Line>, from_in: usize, to_in: usize, from_out: usize, to_out: usize| {
        let (left, right) = (to_in.saturating_sub(from_in), to_out.saturating_sub(from_out));
        for row in 0..left.max(right) {
            let input = (row < left).then(|| ins.get(from_in + row).copied()).flatten();
            let output = (row < right).then(|| outs.get(from_out + row).copied()).flatten();
            found.push((input, output));
        }
    };
    for &(slot, output) in &kept {
        fill(&mut found, next_in, slot, next_out, output);
        found.push((ins.get(slot).copied(), outs.get(output).copied()));
        (next_in, next_out) = (slot + 1, output + 1);
    }
    // An error output with no input is alone on the bottom line (`errors.merge`).
    let alone = error_output.filter(|_| kept.last().is_none_or(|&(_, output)| outs.get(output).copied() != error_output));
    let end_out = if alone.is_some() { outs.len().saturating_sub(1) } else { outs.len() };
    fill(&mut found, next_in, ins.len(), next_out.min(end_out), end_out);
    if let Some(output) = alone {
        found.push((None, Some(output)));
    }
    found
}

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
    /// Its `color`, `fill` and `stroke` (spec 11.2).
    pub colours: labcraft_format::block::Colours,
    pub lines: Vec<Placed>,
    /// The inputs with a default, which go to the top side when the left one is full.
    pub optional: HashSet<String>,
    /// Its outline (spec 11.4), read: none for the ordinary block.
    pub outline: Option<Read>,
}

impl Block {
    /// The block of a function from the sections of its file (spec 11.2, 11.4) and the mask of its library (11.3), whose
    /// fields each section takes when it does not write its own: its outline none for `template block`.
    pub fn read(
        terminals: Option<&TerminalsSection>,
        outline: Option<&labcraft_graph::Outline>,
        mask: Option<&Mask>,
        (inputs, outputs): (&[Name], &[Name]),
        names: &Interner,
        optional: HashSet<String>,
    ) -> Block {
        let section = section_of(terminals, inputs, outputs, names);
        let outline = mask
            .map_or(outline, |mask| mask.outline_of(outline))
            .map(|outline| crate::outline::read(outline, names).0)
            .filter(|read| *read != Read::Block);
        let block = Block { outline, ..Block::of(&section, names, optional) };
        match mask {
            None => block,
            Some(mask) => Block { band: block.band.or(mask.band_place), colours: block.colours.or(mask.colours), ..block },
        }
    }

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
        Block {
            top: section.top_places,
            left: section.left_places,
            middle_sides: section.middle_sides,
            band: section.band,
            colours: section.colours,
            lines,
            optional,
            outline: None,
        }
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

    /// Whether a line of the section holds its terminal on its place at this size: one that another line took, or that
    /// the block cannot hold, leaves its terminal to the defaults.
    pub fn holds<T: Slot>(&self, line: &Placed, inputs: &[T], outputs: &[T]) -> bool {
        let (list, placed) = if line.output { (outputs, &self.outputs) } else { (inputs, &self.inputs) };
        let count = if line.side == BlockSide::Top { self.top } else { self.left };
        let Some(index) = list.iter().position(|terminal| terminal.name() == line.name) else { return false };
        placed.get(index).is_some_and(|&at| at == (line.side, place_number(line.place, count)))
    }
}

/// The places of the terminals of a node, whose function has the section `block`.
pub fn places<T: Slot>(block: &Block, inputs: &[T], outputs: &[T]) -> Places {
    let (top, middle) = (block.top, block.middle_sides);
    let lines = lines(inputs, outputs);
    // The terminals that the lines of the section name, by their index in the node, found once for every attempt.
    let (by_input, by_output) = (by_name(inputs), by_name(outputs));
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
    let most = u32::try_from(inputs.len().saturating_add(outputs.len())).unwrap_or(u32::MAX).saturating_add(MAX_PLACES).saturating_mul(2);
    let at = |steps: u32| left.saturating_add(steps.saturating_mul(2));
    let terminals = Terminals { inputs, outputs, lines: &lines, named: &named, block };
    if let Some(found) = terminals.attempt(top, left) {
        return found;
    }
    let (mut failed, mut steps) = (0_u32, 1_u32);
    let mut found = loop {
        if let Some(found) = terminals.attempt(top, at(steps)) {
            break found;
        }
        if at(steps) >= most {
            // A bound the terminals never reach: what holds is drawn by the defaults of the notation.
            return Places { top, left, inputs: Vec::new(), outputs: Vec::new(), held: Vec::new() };
        }
        failed = steps;
        steps = steps.saturating_mul(2);
    };
    // The first that holds between the last that does not and the first found that does.
    let mut held = steps;
    while held - failed > 1 {
        let middle_steps = failed + (held - failed) / 2;
        match terminals.attempt(top, at(middle_steps)) {
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
    /// The places taken, by side (top, left, right), each from 1: a flag per place, which a large block checks in constant
    /// time without hashing.
    taken: [Vec<bool>; 3],
    /// Below each, no place is free: on the top side, the left side, the right side, and both of the last two.
    from_top: u32,
    from_left: u32,
    from_right: u32,
    from_both: u32,
}

/// The index of a side among the flags of [`Free`].
fn side_index(side: BlockSide) -> usize {
    match side {
        BlockSide::Top => 0,
        BlockSide::Left => 1,
        BlockSide::Right => 2,
    }
}

/// A line of the section, with the index in the node of its terminal and of the other terminal of its pair that follows it.
struct Named<'b> {
    line: &'b Placed,
    terminal: usize,
    partner: Option<usize>,
}

impl Free {
    fn is_taken(&self, side: BlockSide, place: u32) -> bool {
        self.taken.get(side_index(side)).and_then(|flags| flags.get(place as usize)).copied().unwrap_or(false)
    }

    /// The places taken, by side and place.
    fn held(&self) -> Vec<(BlockSide, u32)> {
        let mut held = Vec::new();
        for side in [BlockSide::Top, BlockSide::Left, BlockSide::Right] {
            let flags = self.taken.get(side_index(side)).map_or(&[][..], Vec::as_slice);
            held.extend(
                flags.iter().enumerate().filter(|(_, taken)| **taken).filter_map(|(place, _)| u32::try_from(place).ok()).map(|place| (side, place)),
            );
        }
        held
    }

    fn count(&self, side: BlockSide) -> u32 {
        if side == BlockSide::Top { self.top } else { self.left }
    }

    fn is_free(&self, side: BlockSide, place: u32) -> bool {
        place >= 1 && place <= self.count(side) && !(side == BlockSide::Top && place == 1) && !self.is_taken(side, place)
    }

    fn take(&mut self, side: BlockSide, place: u32) {
        let index = place as usize;
        if let Some(flag) = self.taken.get_mut(side_index(side)).and_then(|flags| flags.get_mut(index)) {
            *flag = true;
        }
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

/// What every attempt at a size places: the terminals of the node, their lines, and those of the section.
struct Terminals<'t, T> {
    inputs: &'t [T],
    outputs: &'t [T],
    lines: &'t [Line],
    named: &'t [Named<'t>],
    block: &'t Block,
}

impl<T: Slot> Terminals<'_, T> {
    fn attempt(&self, top: u32, left: u32) -> Option<Places> {
        let (block, node_inputs, node_outputs, lines) = (self.block, self.inputs, self.outputs, self.lines);
        let flags = |count: u32| vec![false; count as usize + 1];
        let taken = [flags(top), flags(left), flags(left)];
        let mut free = Free { top, left, taken, from_top: 1, from_left: 1, from_right: 1, from_both: 1 };
        let mut inputs: Vec<Option<(BlockSide, u32)>> = vec![None; node_inputs.len()];
        let mut outputs: Vec<Option<(BlockSide, u32)>> = vec![None; node_outputs.len()];
        // The lines of the section, in order: one that the block cannot hold now leaves its terminal to the defaults.
        for &Named { line, terminal, partner } in self.named {
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
        let lines_hold = free.held();
        // The defaults: the `err` line on the last place, then the lines of notation 3.1 in order, a pair on one place.
        let error = |line: &Line| {
            line.0.and_then(|input| node_inputs.get(input)).is_some_and(|terminal| terminal.family().base == Base::Error)
                || line.1.and_then(|output| node_outputs.get(output)).is_some_and(|terminal| terminal.family().base == Base::Error)
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
                && is_pair(node_inputs.get(input), node_outputs.get(output))
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
                    None if node_inputs.get(input).is_some_and(|terminal| block.optional.contains(terminal.name())) => {
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
        let mut held = lines_hold;
        held.sort_by_key(|(side, place)| (side.word(), *place));
        Some(Places {
            top,
            left,
            inputs: inputs.into_iter().collect::<Option<Vec<_>>>()?,
            outputs: outputs.into_iter().collect::<Option<Vec<_>>>()?,
            held,
        })
    }
}

/// Each terminal by its name, the first of a name.
fn by_name<T: Slot>(list: &[T]) -> HashMap<&str, usize> {
    let mut found = HashMap::with_capacity(list.len());
    for (at, terminal) in list.iter().enumerate() {
        found.entry(terminal.name()).or_insert(at);
    }
    found
}

fn set(list: &mut [Option<(BlockSide, u32)>], index: usize, at: (BlockSide, u32)) {
    if let Some(slot) = list.get_mut(index) {
        *slot = Some(at);
    }
}

/// A pass-through pair (notation 3.1): an input and an output of one family that passes through.
fn is_pair<T: Slot>(input: Option<&T>, output: Option<&T>) -> bool {
    match (input, output) {
        (Some(input), Some(output)) => {
            input.family().passes_through()
                && output.family().passes_through()
                && (input.ty() == output.ty() || (input.family() == Family::ERROR && output.family() == Family::ERROR))
        }
        _ => false,
    }
}
