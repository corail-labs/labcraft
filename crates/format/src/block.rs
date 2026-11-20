//! The `terminals` section of a function (spec 11.2): what its lines say, read once for `check`, which judges them
//! (`LC5014`, `LC5015`), for the drawing, which places the terminals that they name and then the others, and for the
//! commands of the block, which refuse a line that would not apply. A line that cannot apply is left out here, so that
//! the three never disagree on which lines count.

use std::collections::{HashMap, HashSet};

use labcraft_graph::{BandPlace, BlockPlace, BlockSide, ColourRole, Id, Rgb, Terminals, TerminalsKind};
pub use labcraft_graph::{DEFAULT_TOP, MAX_PLACES, Place, odd_when_middle, place_number, places_range};

use crate::canonicalize::coordinate_value;
use labcraft_types::{Code, Interner, Name, cut_text};

/// A terminal of the signature: an input or an output, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Which {
    pub output: bool,
    pub name: Name,
}

/// A line that applies: its terminal on a place, and whether the other terminal of its pair goes to the same place of the
/// facing side (a pass-through pair on the left or the right whose other terminal no line names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub side: BlockSide,
    pub place: Place,
    pub terminal: Which,
    pub partner: bool,
    /// The line, which a diagnostic points at.
    pub id: Id,
}

/// What a `terminals` section says, once the lines that cannot apply are left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    /// The places of the top side: its size, or 4, odd when a line puts a terminal on its `middle`.
    pub top_places: u32,
    /// The places of the left and right sides when the section gives them, odd likewise; otherwise they grow to hold
    /// their terminals.
    pub left_places: Option<u32>,
    /// A line puts a terminal on the `middle` of the left or the right side, which keeps them odd as they grow.
    pub middle_sides: bool,
    pub band: Option<BandPlace>,
    pub colours: Colours,
    pub lines: Vec<Placed>,
}

/// The colours that a section writes (11.2), each optional and independent of the others.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Colours {
    /// The band, and the tint of the body without `fill`.
    pub color: Option<Rgb>,
    /// The body, as written.
    pub fill: Option<Rgb>,
    /// The outline, as written.
    pub stroke: Option<Rgb>,
}

impl Colours {
    /// Each colour of `self`, else that of `other`.
    pub fn or(self, other: Colours) -> Colours {
        Colours { color: self.color.or(other.color), fill: self.fill.or(other.fill), stroke: self.stroke.or(other.stroke) }
    }

    pub fn slot(&mut self, role: ColourRole) -> &mut Option<Rgb> {
        match role {
            ColourRole::Color => &mut self.color,
            ColourRole::Fill => &mut self.fill,
            ColourRole::Stroke => &mut self.stroke,
        }
    }
}

/// The terminal that a line of the section names, if it names one of the signature (11.2): `input x`, `output x`, or the
/// name alone, which means the input on the left or the top and the output on the right when it is both.
pub fn named(kind: &TerminalsKind, inputs: &[Name], outputs: &[Name]) -> Option<Which> {
    let TerminalsKind::Place { side, output, name, .. } = *kind else { return None };
    which(side, output, name, inputs.contains(&name), outputs.contains(&name))
}

/// [`named`], once whether the name is an input and an output is known.
fn which(side: BlockSide, output: Option<bool>, name: Name, is_input: bool, is_output: bool) -> Option<Which> {
    match output {
        Some(true) if is_output => Some(Which { output: true, name }),
        Some(false) if is_input => Some(Which { output: false, name }),
        Some(_) => None,
        None if is_input && is_output => Some(Which { output: side == BlockSide::Right, name }),
        None if is_input || is_output => Some(Which { output: is_output, name }),
        None => None,
    }
}

/// What a line of the section breaks, for `check`: the code, the message, the line.
pub type Problem = (Code, String, Id);

/// Reads a section against the inputs and the outputs of a signature, by name.
pub fn read(section: &Terminals, inputs: &[Name], outputs: &[Name], names: &Interner) -> (Section, Vec<Problem>) {
    read_sized(section, inputs, outputs, names, (None, None))
}

/// The smallest `top` and `left` at which every line of the section that applies still applies (11.2), as `block.set_size`
/// counts them: where a drag of an edge of the block stops in the view `Block` (ide.md 3).
pub fn least_sizes(section: &Terminals, inputs: &[Name], outputs: &[Name], names: &Interner) -> (u32, u32) {
    let (read_now, problems) = read(section, inputs, outputs, names);
    let before: HashSet<(Id, Code)> = problems.into_iter().map(|problem| (problem.2, problem.0)).collect();
    let least = |side: BlockSide| {
        let range = places_range(side);
        let sized = |places: u32| if side == BlockSide::Top { (Some(places), None) } else { (None, Some(places)) };
        // No size below the highest place that a line of this side gives holds it: the search starts there, so that a
        // section of many lines is read a few times, not once per size.
        let on_side = |line: &&Placed| if side == BlockSide::Top { line.side == BlockSide::Top } else { line.side != BlockSide::Top };
        let highest = read_now
            .lines
            .iter()
            .filter(on_side)
            .filter_map(|line| match line.place {
                Place::Number(number) => Some(number),
                Place::Last | Place::Middle => None,
            })
            .max()
            .unwrap_or(0);
        (highest.max(*range.start())..=*range.end())
            .find(|&places| {
                read_sized(section, inputs, outputs, names, sized(places)).1.iter().all(|problem| before.contains(&(problem.2, problem.0)))
            })
            .unwrap_or(*range.end())
    };
    (least(BlockSide::Top), least(BlockSide::Left))
}

/// [`read`], with the size of the top side or of the left one, when given, in place of the one that the section writes.
fn read_sized(
    section: &Terminals,
    inputs: &[Name],
    outputs: &[Name],
    names: &Interner,
    given: (Option<u32>, Option<u32>),
) -> (Section, Vec<Problem>) {
    // The place of each terminal in the signature, for the order in which lines take their places.
    let rank = |list: &[Name]| -> HashMap<Name, usize> {
        let mut found = HashMap::with_capacity(list.len());
        for (at, name) in list.iter().enumerate() {
            found.entry(*name).or_insert(at);
        }
        found
    };
    let (input_rank, output_rank) = (rank(inputs), rank(outputs));
    let inputs: HashSet<Name> = inputs.iter().copied().collect();
    let outputs: HashSet<Name> = outputs.iter().copied().collect();
    let text = |name: Name| cut_text(names.resolve(name)).into_owned();
    let mut problems = Vec::new();
    let mut read = Section::default();
    let (mut top, mut left) = (None, None);
    let (mut sizes, mut band): (HashMap<BlockSide, Id>, Option<Id>) = (HashMap::new(), None);
    let mut colours: HashSet<ColourRole> = HashSet::new();
    // First the sizes and the band, which the places are judged against, whatever the order of the lines.
    for item in &section.items {
        match item.kind {
            TerminalsKind::Size { side, places } => {
                let word = if side == BlockSide::Top { "top" } else { "left" };
                if sizes.insert(side, item.id).is_some() {
                    problems.push((Code::Lc5015, format!("a second size for the `{word}` side: a side has one size (11.2)"), item.id));
                    continue;
                }
                let digits = text(places);
                // A size out of the range is drawn at the nearest bound (11.2): digits too large for a coordinate are past
                // the most.
                // Read whole, as `fmt` reads them (leading zeros past the cut of the message included); only the message is cut.
                let (range, written) = (places_range(side), coordinate_value(names.resolve(places)));
                let value = written.map_or(*range.end(), |value| value.clamp(*range.start(), *range.end()));
                if written != Some(value) {
                    let (least, most) = range.into_inner();
                    let message = format!("`{word} {digits}`: this side has {least} to {most} places, so it is drawn with {value} (11.2)");
                    problems.push((Code::Lc5015, message, item.id));
                }
                *(if side == BlockSide::Top { &mut top } else { &mut left }) = Some(value);
            }
            TerminalsKind::Band(place) => {
                if band.is_some() {
                    problems.push((Code::Lc5015, "a second `band` line: a block has one band (11.2)".to_owned(), item.id));
                } else {
                    band = Some(item.id);
                    read.band = Some(place);
                }
            }
            TerminalsKind::Colour { role, text } => {
                let word = role.word();
                if !colours.insert(role) {
                    problems.push((Code::Lc5015, format!("a second `{word}` line: a block has one (11.2)"), item.id));
                    continue;
                }
                match Rgb::parse(names.resolve(text)) {
                    Some(rgb) => *read.colours.slot(role) = Some(rgb),
                    None => {
                        let shown = cut_text(names.resolve(text)).into_owned();
                        let message = format!("`{word} \"{shown}\"` is no colour `\"#RRGGBB\"`, so this line does not apply (11.2)");
                        problems.push((Code::Lc5018, message, item.id));
                    }
                }
            }
            TerminalsKind::Place { .. } => {}
        }
    }
    // The terminals that a line names on its own: a pair named alone moves together only when no line names its other one.
    let mut named: HashMap<Which, Id> = HashMap::new();
    let mut lines = Vec::new();
    for item in &section.items {
        let TerminalsKind::Place { side, place, output, name } = item.kind else { continue };
        let shown = text(name);
        let (is_input, is_output) = (inputs.contains(&name), outputs.contains(&name));
        let Some(terminal) = which(side, output, name, is_input, is_output) else {
            let message = match output {
                Some(output) => {
                    let kind = if output { "output" } else { "input" };
                    format!("`{kind} {shown}` names no {kind} of the function, so this line does not apply (11.2)")
                }
                None => format!("`{shown}` is no input and no output of the function, so this line does not apply (11.2)"),
            };
            problems.push((Code::Lc5014, message, item.id));
            continue;
        };
        if named.contains_key(&terminal) {
            let message = format!("a second line for the terminal `{shown}`: a terminal has one line (11.2)");
            problems.push((Code::Lc5015, message, item.id));
            continue;
        }
        named.insert(terminal, item.id);
        let place = match place {
            BlockPlace::Number(digits) => match coordinate_value(names.resolve(digits)) {
                Some(number) => Place::Number(number),
                None => Place::Number(0),
            },
            BlockPlace::Last => Place::Last,
            BlockPlace::Middle => Place::Middle,
        };
        let pair = is_input && is_output;
        lines.push((item.id, side, place, terminal, pair));
    }
    // A `middle` makes its side odd when its line names a terminal: one that names none does not count, as for the drawing.
    let middle_on = |sides: &[BlockSide]| lines.iter().any(|(_, side, place, ..)| *place == Place::Middle && sides.contains(side));
    let (top, left) = (given.0.or(top), given.1.or(left));
    let top = odd_when_middle(top.unwrap_or(DEFAULT_TOP), middle_on(&[BlockSide::Top]));
    read.middle_sides = middle_on(&[BlockSide::Left, BlockSide::Right]);
    let left = left.map(|left| odd_when_middle(left, read.middle_sides));
    (read.top_places, read.left_places) = (top, left);
    // Then the places, in the order of the signature, the inputs then the outputs, which is the order `fmt` writes (5.2):
    // of two lines on one place, the second does not apply, and `fmt` never changes which (11.2).
    lines.sort_by_key(|(_, _, _, terminal, _)| {
        let ranks = if terminal.output { &output_rank } else { &input_rank };
        (terminal.output, ranks.get(&terminal.name).copied().unwrap_or(usize::MAX))
    });
    let mut taken: HashMap<(BlockSide, Place), Id> = HashMap::new();
    for (id, side, place, terminal, pair) in lines {
        let count = match side {
            BlockSide::Top => Some(top),
            BlockSide::Left | BlockSide::Right => left,
        };
        let shown = text(terminal.name);
        // A place counted on a side of a known size; on a side that grows to hold its terminals, `last` and `middle` stay as written.
        let resolved = match (place, count) {
            (Place::Number(number), _) => Place::Number(number),
            (_, Some(count)) => Place::Number(place_number(place, count)),
            (other, None) => other,
        };
        let outside = match resolved {
            Place::Number(number) => number == 0 || number > count.unwrap_or(MAX_PLACES),
            Place::Last | Place::Middle => false,
        };
        if outside {
            let room = count.map_or_else(|| format!("at most {MAX_PLACES} places"), |count| format!("{count} places"));
            let message = format!("this place is outside its side, which has {room}, so `{shown}` keeps its default place (11.2)");
            problems.push((Code::Lc5014, message, id));
            continue;
        }
        if side == BlockSide::Top && resolved == Place::Number(1) {
            let message = format!("the first place of the top side is kept for `after:`, so `{shown}` keeps its default place (11.2)");
            problems.push((Code::Lc5014, message, id));
            continue;
        }
        // A pair named alone on the left or the right takes the same place of the facing side, unless a line names its other terminal.
        let other = Which { output: !terminal.output, name: terminal.name };
        let facing = side.facing().filter(|_| pair && !named.contains_key(&other));
        let partner = facing.is_some();
        let mut wanted = vec![(side, resolved)];
        wanted.extend(facing.map(|facing| (facing, resolved)));
        if wanted.iter().any(|key| taken.contains_key(key)) {
            let message = format!("another line took this place, so `{shown}` keeps its default place (11.2)");
            problems.push((Code::Lc5014, message, id));
            continue;
        }
        for key in wanted {
            taken.insert(key, id);
        }
        read.lines.push(Placed { side, place, terminal, partner, id });
    }
    (read, problems)
}
