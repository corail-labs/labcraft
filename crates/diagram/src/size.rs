//! Step 2 (`docs/architecture.md`, 11.3): the size of each node in its view, compact or standard (notation 3.1, 3.2),
//! every text cut in cells by `types::text`, and the shapes of the node around its block. All in hundredths of a unit,
//! relative to the top-left corner of the block, which is what `node x at (40, 80)` places (spec 11).

use std::collections::{HashMap, VecDeque};

use labcraft_graph::{IconShape as Written, Numeral, NumeralKind};
use labcraft_types::text::{self, ShownText};
use labcraft_types::{Interner, TyId, cut_text};

use crate::graph::{Base, Family, Glyph, Kind, Node, Terminal};
use crate::shape::{Detail, IconDrawing, IconDrawn, IconItem, IconShape, Paint, Run, Shape, Token, token_of};
use crate::style;

/// The three sizes of text (notation 2.3), in force.
pub fn large() -> u32 {
    u32::try_from(style::large()).unwrap_or(1200)
}

pub fn medium() -> u32 {
    u32::try_from(style::medium()).unwrap_or(1100)
}

pub fn small() -> u32 {
    u32::try_from(style::small()).unwrap_or(900)
}

/// The name on a compact block, the label of an oblong (notation 3.2), in force.
pub fn label_size() -> u32 {
    u32::try_from(style::label()).unwrap_or(1000)
}

/// How many cells of text at `size` hold in `width`, both in hundredths of a unit.
pub fn cells_in(width: i64, size: u32) -> u64 {
    if width <= 0 {
        return 0;
    }
    let per_cell = text::width(1, size).unwrap_or(i64::MAX).max(1);
    let mut cells = u64::try_from(width / per_cell).unwrap_or(0).saturating_add(2);
    while cells > 0 && text::width(cells, size).is_none_or(|taken| taken > width) {
        cells -= 1;
    }
    cells
}

/// The width of a compact block (notation 3.2), whatever its name, its band or its lines.
pub fn block_width() -> i64 {
    up(style::square())
}

/// The cells of a line of the name of a compact block: between its terminals, which straddle its edges (notation 3.2).
pub fn name_cells() -> u64 {
    cells_in(block_width() - style::terminal(), label_size())
}

/// How far a terminal on an edge of a node reaches into it (notation 4.1): half of it, and the brackets of an array.
fn reach_in(family: Family) -> i64 {
    let half = style::terminal() / 2;
    if family.dimensions == 0 { half } else { half + 250 * i64::from(family.dimensions.min(4)) + 60 }
}

/// How far the text of the band of `node` keeps from the left and the right edges of its block (notation 3.2): past the
/// terminal of its first line on that side, which is beside the band, or half a terminal where it has none, and past the
/// chevron of `after:` on the top edge on the left, then the padding of an oblong label.
pub fn band_insets(node: &Node<'_>) -> (i64, i64) {
    let first = lines(&node.inputs, &node.outputs).into_iter().next().unwrap_or((None, None));
    let reach = |terminal: Option<&Terminal>| terminal.map_or(style::terminal() / 2, |terminal| reach_in(terminal.family));
    let mut left = reach(first.0.and_then(|input| node.inputs.get(input)));
    if node.after {
        left = left.max(style::grid() + style::terminal() / 2);
    }
    let right = reach(first.1.and_then(|output| node.outputs.get(output)));
    (left + style::label_padding(), right + style::label_padding())
}

/// The cells of the band of a block `width` wide (notation 3.2): what its insets leave; never less than one, `…`.
pub fn band_cells(width: i64, (left, right): (i64, i64)) -> u64 {
    cells_in(width - left - right, small()).max(1)
}

/// The width of the block of a node in compact view, which its band takes (notation 3.2): an oblong is as wide as its
/// label, every other block `node.square`.
pub fn band_room(node: &Node<'_>) -> i64 {
    match &node.kind {
        Kind::Construction { ty, .. } => oblong_width(&[Text::new(ty, label_line(), label_size())]),
        Kind::Variant { ty, variant, .. } => {
            oblong_width(&[Text::new(ty, label_line(), label_size()), Text::new(variant, label_line(), label_size())])
        }
        _ => block_width(),
    }
}

/// An oblong: as wide as its label with `node.label_padding` on each side, `node.square` at least (notation 3.2).
fn oblong_width(label: &[Text]) -> i64 {
    up(style::square().max(label_width(label) + 2 * style::label_padding()))
}

/// A text that no other rule cuts (notation 2.3), in code points.
pub fn any_text() -> u64 {
    u64::try_from(style::any_text()).unwrap_or(40)
}

/// A line of a label, a band (notation 3.2).
pub fn label_line() -> u64 {
    u64::try_from(style::label_line()).unwrap_or(12)
}

/// An inline constant in compact view (notation 3.2).
pub fn compact_constant() -> u64 {
    u64::try_from(style::compact_constant()).unwrap_or(8)
}

/// A text cut and measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    pub shown: ShownText,
    pub size: u32,
    pub width: i64,
}

impl Text {
    pub fn new(raw: &str, max: u64, size: u32) -> Text {
        let shown = text::show(raw, max);
        Text::measured(shown, size)
    }

    pub fn measured(shown: ShownText, size: u32) -> Text {
        let width = text::width(shown.cells, size).unwrap_or(i64::MAX / 4);
        Text { shown, size, width }
    }

    /// The text at `x`, its capitals centred on `middle` (notation 2.3).
    pub fn at(&self, x: i64, middle: i64, token: Token) -> Shape {
        let cap = text::scale(u64::from(text::CAP_HEIGHT), self.size).unwrap_or(0);
        Shape::Text {
            x,
            y: middle + (cap + 1) / 2,
            size: self.size,
            width: self.width,
            runs: self.shown.runs.iter().map(|(ink, run)| Run { token: token_of(*ink, token), text: run.clone() }).collect(),
            detail: Detail::Label,
            ground: None,
        }
    }

    /// The same, shown from the zoom of `detail` on the canvas (notation 7).
    pub fn shown_from(&self, x: i64, middle: i64, token: Token, detail: Detail) -> Shape {
        let mut shape = self.at(x, middle, token);
        if let Shape::Text { detail: slot, .. } = &mut shape {
            *slot = detail;
        }
        shape
    }

    pub fn centred(&self, centre: i64, middle: i64, token: Token) -> Shape {
        self.at(centre - self.width / 2, middle, token)
    }
}

/// The height of a line of text: 2384/2048 of its size (notation 2.3).
pub fn line_height(size: u32) -> i64 {
    text::scale(u64::from(text::ASCENT + text::DESCENT), size).unwrap_or(0)
}

pub fn up(value: i64) -> i64 {
    let rest = value.rem_euclid(style::grid());
    if rest == 0 { value } else { value.saturating_add(style::grid() - rest) }
}

/// A line of terminals: at most one input and one output (notation 3.1), by their index.
pub type Line = (Option<usize>, Option<usize>);

/// The lines of a node, from the top, as notation 3.1 makes them: pass-through pairs on one line, the `err` pair on the
/// bottom one, the others side by side in signature order between the pairs.
pub fn lines(inputs: &[Terminal], outputs: &[Terminal]) -> Vec<Line> {
    // `err` is made last in both lists: the input named `err`, else the last error input; its output by name, else the
    // last error output.
    let error_input = inputs
        .iter()
        .position(|terminal| terminal.name == "err" && terminal.family.base == Base::Error)
        .or_else(|| inputs.iter().rposition(|terminal| terminal.family == Family { base: Base::Error, dimensions: 0 }));
    let error_output = outputs
        .iter()
        .position(|terminal| terminal.name == "err" && terminal.family.base == Base::Error)
        .or_else(|| outputs.iter().rposition(|terminal| terminal.family == Family { base: Base::Error, dimensions: 0 }));
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
    let key = |terminal: &Terminal| if terminal.family == Family::ERROR { TyId::ERROR } else { terminal.ty };
    for (other, &output) in outs.iter().enumerate() {
        let Some(out) = outputs.get(output).filter(|out| out.family.passes_through()) else { continue };
        by_name.entry((out.name.as_str(), key(out))).or_default().push_back(other);
        by_type.entry(key(out)).or_default().push_back(other);
    }
    for named in [true, false] {
        for (slot, &input) in ins.iter().enumerate() {
            let Some(terminal) = inputs.get(input).filter(|terminal| terminal.family.passes_through()) else { continue };
            if paired_in.get(slot).copied().flatten().is_some() {
                continue;
            }
            let queue = if named { by_name.get_mut(&(terminal.name.as_str(), key(terminal))) } else { by_type.get_mut(&key(terminal)) };
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

/// What the drawing of one node gives: its block and the shapes around it, relative to the top-left of the block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sized {
    pub width: i64,
    pub height: i64,
    /// How far the shapes reach beyond the block: above (its label), to the left (pills), to the right.
    pub above: i64,
    pub left: i64,
    pub right: i64,
    pub shapes: Vec<Shape>,
    /// The centre of each terminal, in the order of the node's inputs and outputs, from the top-left of the block.
    pub inputs: Vec<(i64, i64)>,
    pub outputs: Vec<(i64, i64)>,
    /// The centre of the `after:` terminal, when the node has one.
    pub order: Option<(i64, i64)>,
    /// Where an order wire from its label starts: right after the label, at its middle.
    pub label_end: Option<(i64, i64)>,
    /// A `match` or a `handle` in tabs: the wires in its margins that each arm draws, from the top-left of the block.
    pub arms: Vec<Vec<Shape>>,
    /// For each input with a pill outside (an inline constant, the pill of its wire), how far left of the terminal the
    /// pill starts: further when it steps past the pill of a neighbour (notation 3.2).
    pub reaches: Vec<Option<i64>>,
}

impl Sized {
    pub fn new(width: i64, height: i64, shapes: Vec<Shape>) -> Sized {
        Sized {
            width,
            height,
            above: 0,
            left: 0,
            right: 0,
            shapes,
            inputs: Vec::new(),
            outputs: Vec::new(),
            order: None,
            label_end: None,
            arms: Vec::new(),
            reaches: Vec::new(),
        }
    }
}

/// The paint of a terminal's family (notation 4.1).
pub fn wire_token(family: Family) -> Token {
    match family.base {
        Base::Session => "wire.resource",
        Base::Error => "wire.error",
        _ => "wire.data",
    }
}

/// How a wire of a family is drawn (notation 4.1); `core` is the fill of the frame it runs in.
pub fn wire_style(family: Family, core: Token) -> crate::shape::WireStyle {
    use crate::shape::{Pattern, WireStyle};
    let token = wire_token(family);
    let ticks = family.dimensions.min(3);
    let ground = core;
    let style = match family.base {
        Base::Struct => WireStyle { token, width: style::width_struct(), pattern: Pattern::Solid, core: Some(core), ticks, ground },
        Base::Session => WireStyle { token, width: style::width_session(), pattern: Pattern::Solid, core: None, ticks, ground },
        Base::Error => WireStyle { token, width: style::width_error(), pattern: Pattern::Chevrons, core: None, ticks, ground },
        Base::Order => WireStyle { token: "wire.order", width: style::width_order(), pattern: Pattern::Dotted, core: None, ticks: 0, ground },
        Base::Function => WireStyle { token, width: style::width_function(), pattern: Pattern::DashDot, core: None, ticks, ground },
        Base::Number | Base::Bool | Base::Text | Base::Enum | Base::Unknown => {
            WireStyle { token, width: style::width_data(), pattern: Pattern::Solid, core: None, ticks, ground }
        }
    };
    // An array is as wide as a struct, and 4, at least, so that its ticks are cut into it rather than sticking out.
    if ticks > 0 { WireStyle { width: style.width.max(style::width_struct()).max(400), ..style } } else { style }
}

/// A terminal centred on `(x, y)`, its shape telling its family (notation 4.1), at `scale` percent.
pub fn terminal(family: Family, x: i64, y: i64, scale: i64, ignored: bool) -> Vec<Shape> {
    let half = style::terminal() * scale / 200;
    let token = wire_token(family);
    let paint = if ignored { Paint::both("node.fill", token, 120) } else { Paint::fill(token) };
    let mut shapes = match family.base {
        Base::Number | Base::Unknown => vec![Shape::Circle { x, y, radius: half, paint }],
        Base::Bool => vec![Shape::Poly { points: vec![(x, y - half), (x + half, y), (x, y + half), (x - half, y)], closed: true, paint }],
        Base::Text => vec![Shape::Rect { x: x - half, y: y - half * 7 / 10, width: 2 * half, height: half * 14 / 10, radius: 0, paint }],
        Base::Enum => vec![Shape::Poly { points: vec![(x - half, y - half), (x + half, y), (x - half, y + half)], closed: true, paint }],
        Base::Struct => {
            vec![Shape::Circle { x, y, radius: half, paint: Paint::both("node.fill", token, 150) }, Shape::Circle { x, y, radius: half / 2, paint }]
        }
        Base::Session => vec![Shape::Rect { x: x - half, y: y - half, width: 2 * half, height: 2 * half, radius: 0, paint }],
        Base::Error => {
            vec![Shape::Poly { points: vec![(x - half, y - half), (x + half, y), (x - half, y + half), (x - half / 3, y)], closed: true, paint }]
        }
        Base::Function => vec![Shape::Circle { x, y, radius: half, paint: Paint::both("node.fill", token, 150) }],
        Base::Order => vec![order_terminal(x, y)],
    };
    if family.dimensions > 0 {
        // In brackets, one pair per dimension.
        for dimension in 0..i64::from(family.dimensions.min(4)) {
            let gap = half + 250 + dimension * 250;
            for side in [-1, 1] {
                let edge = x + side * gap;
                shapes.push(Shape::Poly {
                    points: vec![(edge - side * 200, y - half), (edge, y - half), (edge, y + half), (edge - side * 200, y + half)],
                    closed: false,
                    paint: Paint::stroke(token, 120),
                });
            }
        }
    }
    shapes
}

/// The order terminal of `after:`, a hollow chevron (notation 4.1).
pub fn order_terminal(x: i64, y: i64) -> Shape {
    let half = style::terminal() / 2;
    Shape::Poly { points: vec![(x - half, y - half), (x + half, y), (x - half, y + half)], closed: false, paint: Paint::stroke("wire.order", 150) }
}

/// A pill of an inline constant: its text in 11, 4 on each side, 18 high (notation 2.3, 3.2).
pub fn pill(text: &Text, x: i64, middle: i64) -> Vec<Shape> {
    let width = text.width + 2 * style::padding();
    vec![
        Shape::Rect {
            x,
            y: middle - style::pill_height() / 2,
            width,
            height: style::pill_height(),
            radius: style::pill_height() / 2,
            paint: Paint::both("pill.fill", "pill.stroke", style::outline()),
        },
        text.at(x + style::padding(), middle, "pill.text"),
    ]
}

/// The pill of a wire just before its reader: a field it reads (notation 3.3), or a conversion: its arrow, and what it
/// arrives in, in `mark.coercion` on a ground without outline, never a round mark (notation 4.2).
pub fn wire_pill(text: &Text, x: i64, middle: i64, conversion: bool) -> Vec<Shape> {
    if !conversion {
        return pill(text, x, middle);
    }
    vec![
        Shape::Rect {
            x,
            y: middle - style::pill_height() / 2,
            width: pill_width(text),
            height: style::pill_height(),
            radius: style::radius(),
            paint: Paint::fill("pill.fill"),
        },
        text.at(x + style::padding(), middle, "mark.coercion"),
    ]
}

pub fn pill_width(text: &Text) -> i64 {
    text.width + 2 * style::padding()
}

/// The name of a compact block, the last segment of what it calls, on two lines at most of `max` cells (notation 3.2):
/// on one line when it holds; else cut at an underscore, which is not drawn, when both parts hold, the cut whose longer
/// line is the shortest, the first on a tie; else the first line as far as the last underscore that it holds and the
/// rest on the second; else, when the first word does not hold, that word on the first line and the rest on the second,
/// each cut by `…`; else, a single word, two halves as even as the graphemes allow, the first the longer, or, when it
/// needs more than two lines, the first line as far as it holds and the rest cut by `…`. A word is never cut to leave a
/// letter alone (`set / tle`, not `settl / e`).
pub fn label_lines(name: &str, max: u64) -> Vec<Text> {
    // The first 200 code points: no line of more than `max` cells is drawn anyway.
    let name = cut_text(name);
    if max == 0 || name.is_empty() {
        return Vec::new();
    }
    let line = |part: &str| Text::new(part, max, label_size());
    let total = text::cells(&name);
    if total <= max {
        return vec![line(&name)];
    }
    // Each underscore that is a grapheme of its own, with the cells before it: one pass, whatever the underscores.
    let mut best: Option<(u64, usize)> = None;
    let mut first_underscore: Option<usize> = None;
    let (mut at, mut before) = (0usize, 0u64);
    for grapheme in text::graphemes(&name) {
        let cells = text::cells(grapheme);
        if grapheme == "_" && at > 0 && at + 1 < name.len() {
            first_underscore = first_underscore.or(Some(at));
            let longer = before.max(total.saturating_sub(before).saturating_sub(cells));
            if longer <= max && best.is_none_or(|(shortest, _)| longer < shortest) {
                best = Some((longer, at));
            }
        }
        at += grapheme.len();
        before = before.saturating_add(cells);
    }
    if let Some((_, at)) = best {
        return vec![line(name.get(..at).unwrap_or_default()), line(name.get(at + 1..).unwrap_or_default())];
    }
    // As far as the first line holds, between two graphemes.
    let (mut end, mut used) = (0, 0u64);
    for grapheme in text::graphemes(&name) {
        let cells = text::cells(grapheme);
        if used.saturating_add(cells) > max {
            break;
        }
        used += cells;
        end += grapheme.len();
    }
    if end == 0 {
        return vec![line(&name)];
    }
    // The underscore right after what holds, else the last one that it holds; never one that leaves a line empty.
    let underscore =
        if name.get(end..).is_some_and(|rest| rest.starts_with('_')) { Some(end) } else { name.get(..end).and_then(|head| head.rfind('_')) };
    let (first, second) = match (underscore.filter(|&at| at > 0 && at + 1 < name.len()), first_underscore) {
        (Some(at), _) => (name.get(..at), name.get(at + 1..)),
        // The first word does not hold: it keeps its line, cut, and the rest goes on the second (`appe… / csv`).
        (None, Some(at)) => (name.get(..at), name.get(at + 1..)),
        (None, None) if total <= max.saturating_mul(2) => {
            // Two even halves, the first the longer: the shortest first line of half the cells or more, within the line.
            let (mut middle, mut used) = (0, 0u64);
            for grapheme in text::graphemes(&name) {
                let cells = text::cells(grapheme);
                if used.saturating_add(cells) > max {
                    break;
                }
                used = used.saturating_add(cells);
                middle += grapheme.len();
                if used.saturating_mul(2) >= total {
                    break;
                }
            }
            if middle < name.len() { (name.get(..middle), name.get(middle..)) } else { (name.get(..end), name.get(end..)) }
        }
        (None, None) => (name.get(..end), name.get(end..)),
    };
    vec![line(first.unwrap_or_default()), line(second.unwrap_or_default())]
}

fn label_width(lines: &[Text]) -> i64 {
    lines.iter().map(|line| line.width).max().unwrap_or(0)
}

fn label_height(lines: &[Text]) -> i64 {
    line_height(label_size()).saturating_mul(lines.len() as i64)
}

/// Where the terminals of a compact block stand (notation 3.2, the defaults of N-15).
struct Places {
    /// The lines of the left and right sides, from the top, and the height of each.
    lines: Vec<Line>,
    ys: Vec<i64>,
    /// The height of the block: 8 above its first place, 8 below its last.
    height: i64,
}

/// The places of a compact block `width` wide and at least `least` high, a multiple of the grid: its lines from the top,
/// one per place, `node.pitch` apart, the first 8 below the top, the line of the `err` pair on the last place, so that a line has the same height on every
/// block whatever follows it. When they do not hold, the block grows by two places; every input stays on the left (the
/// top side comes with the `terminals` section, M1).
fn places(node: &Node<'_>, least: i64) -> Places {
    let (grid, pitch) = (style::grid(), style::compact_pitch());
    let lines = lines(&node.inputs, &node.outputs);
    let height = grown(lines.len(), least);
    let error = |line: &Line| {
        line.0.and_then(|input| node.inputs.get(input)).is_some_and(|terminal| terminal.family.base == Base::Error)
            || line.1.and_then(|output| node.outputs.get(output)).is_some_and(|terminal| terminal.family.base == Base::Error)
    };
    let count = lines.len();
    let ys =
        lines.iter().enumerate().map(|(row, line)| if row + 1 == count && error(line) { height - grid } else { grid + row as i64 * pitch }).collect();
    Places { lines, ys, height }
}

/// The height of a compact node of `count` places, `least` at least: 8 above the first place and 8 below the last, `node.pitch`
/// between two, growing by two places at a time (notation 3.2).
fn grown(count: usize, least: i64) -> i64 {
    let (grid, pitch) = (style::grid(), style::compact_pitch());
    grown_to(grid + (count.max(1) as i64 - 1).saturating_mul(pitch) + grid, least)
}

/// The height of a compact node that `needed` must hold, `least` at least, growing by two places at a time (notation 3.2).
fn grown_to(needed: i64, least: i64) -> i64 {
    let pitch = style::compact_pitch();
    let mut height = least;
    while height < needed {
        height = up(height.saturating_add(2 * pitch));
    }
    height
}

/// The terminals of a block as drawn: how far its pills reach to the left, where each terminal is, and how far left of
/// its terminal each pill starts.
struct AtPlaces {
    left: i64,
    inputs: Vec<(i64, i64)>,
    outputs: Vec<(i64, i64)>,
    reaches: Vec<Option<i64>>,
}

/// The body of a compact block (notation 3.2): its fill, the band of its library inside, at its top, its outline over
/// both. Where what the band leaves starts: 0 without a band.
fn body(band: Option<&Text>, (left, right): (i64, i64), width: i64, height: i64, shapes: &mut Vec<Shape>) -> i64 {
    let radius = style::radius();
    let outline = |paint| Shape::Rect { x: 0, y: 0, width, height, radius, paint };
    let Some(band) = band else {
        shapes.push(outline(Paint::both("node.fill", "node.stroke", style::outline())));
        return 0;
    };
    let band_height = style::band_height().min(height);
    shapes.push(outline(Paint::fill("node.fill")));
    // Rounded at the top with the block, square at the bottom.
    shapes.push(Shape::Rect { x: 0, y: 0, width, height: band_height, radius, paint: Paint::fill("band.fill") });
    let square = radius.min(band_height / 2);
    if square > 0 {
        shapes.push(Shape::Rect { x: 0, y: band_height - square, width, height: square, radius: 0, paint: Paint::fill("band.fill") });
    }
    shapes.push(Shape::Poly {
        points: vec![(0, band_height), (width, band_height)],
        closed: false,
        paint: Paint::stroke("node.stroke", style::outline()),
    });
    shapes.push(band.centred((left + width - right) / 2, band_height / 2, "band.text"));
    shapes.push(outline(Paint::stroke("node.stroke", style::outline())));
    band_height
}

/// The top of the line of the `▴` of `always`, centred 8 above the bottom of a compact node `height` high (notation 3.2).
fn always_top(height: i64) -> i64 {
    height - style::grid() - line_height(medium()) / 2
}

/// A compact block (a square or an oblong) from its terminals on their places (notation 3.2).
fn compact(width: i64, height: i64, shapes: Vec<Shape>, drawn: AtPlaces, order: Option<(i64, i64)>) -> Sized {
    Sized {
        left: drawn.left,
        right: style::terminal() / 2,
        inputs: drawn.inputs,
        outputs: drawn.outputs,
        order,
        reaches: drawn.reaches,
        ..Sized::new(width, height, shapes)
    }
}

/// The pill outside an input: its inline constant, else the pill of its wire (a field, a conversion), and whether it is
/// a constant, which the block draws (notation 3.2).
fn pill_text(terminal: &Terminal) -> Option<(Text, bool)> {
    match (&terminal.constant, &terminal.pill) {
        (Some(constant), _) => Some((Text::new(constant, compact_constant(), medium()), true)),
        (None, Some(pill)) => Some((Text::new(pill, label_line(), medium()), false)),
        (None, None) => None,
    }
}

/// The wire from a constant that stepped away from its terminal to that terminal (notation 3.2).
fn lead(family: Family, from: (i64, i64), to: (i64, i64)) -> Shape {
    Shape::Poly { points: vec![from, to], closed: false, paint: Paint::stroke(wire_token(family), wire_style(family, "node.fill").width) }
}

pub struct Sizer<'a> {
    pub names: &'a Interner,
    pub standard_everywhere: bool,
}

impl Sizer<'_> {
    /// The node in its view: its band already chosen for the whole drawing (notation 3.2).
    /// `icon`: the index in `Drawing::icons` of the icon of what it calls, when it shows one.
    pub fn size(&self, node: &Node<'_>, band: Option<&ShownText>, icon: Option<usize>) -> Sized {
        let standard = self.standard_everywhere || node.standard;
        let mut sized = match &node.kind {
            Kind::Constant { name, value } => self.constant(name, value),
            Kind::Call { glyph: Some(glyph), .. } if !standard => self.primitive(node, glyph),
            Kind::Call { qualified, short, .. } => {
                if standard {
                    self.standard(node, qualified)
                } else {
                    self.square(node, short, band, icon)
                }
            }
            Kind::Construction { ty, .. } => {
                if standard {
                    self.standard(node, &format!("{ty} {{ }}"))
                } else {
                    self.oblong(node, vec![Text::new(ty, label_line(), label_size())], band)
                }
            }
            Kind::Variant { ty, variant, .. } => {
                if standard {
                    self.standard(node, &format!("{ty}.{variant}"))
                } else {
                    self.oblong(node, vec![Text::new(ty, label_line(), label_size()), Text::new(variant, label_line(), label_size())], band)
                }
            }
            Kind::Ui { operation, element } => {
                if standard {
                    self.standard(node, &format!("ui.{operation} {element}"))
                } else {
                    self.oblong(node, vec![Text::new(operation, label_line(), label_size()), Text::new(element, label_line(), label_size())], None)
                }
            }
            Kind::Structure(_) => Sized::new(0, 0, Vec::new()),
        };
        // A terminal that no line holds stays at the middle of the left or the right edge.
        sized.inputs.resize(node.inputs.len(), (0, sized.height / 2));
        sized.outputs.resize(node.outputs.len(), (sized.width, sized.height / 2));
        if let Some(label) = &node.label {
            // Above the block, whose band is inside it (notation 3.1, 3.2).
            // Right of an `after:` terminal on the top edge, which a wire reaches from above (notation 2.2).
            let text = Text::new(&format!("{label}:"), any_text(), medium());
            let start = match sized.order {
                Some((x, y)) if y <= 0 && x > 0 => x + style::terminal() / 2 + style::padding(),
                _ => 0,
            };
            let middle = -sized.above - 400 - line_height(medium()) / 2;
            sized.shapes.push(text.at(start, middle, "ink.muted"));
            sized.above += 400 + line_height(medium());
            sized.right = sized.right.max(start + text.width - sized.width);
            sized.label_end = Some((start + text.width + style::padding(), middle));
        }
        sized
    }

    /// A named constant, or a construction folded: its name, its value in a pill, one output (notation 3.1).
    fn constant(&self, name: &str, value: &str) -> Sized {
        let name = Text::new(name, any_text(), large());
        let value = Text::new(value, any_text(), medium());
        let height = 2400;
        let middle = height / 2;
        let mut shapes = vec![name.at(0, middle, "node.title")];
        let pill_x = name.width + style::grid();
        shapes.extend(pill(&value, pill_x, middle));
        let width = pill_x + pill_width(&value) + style::grid();
        shapes.extend(terminal(Family { base: Base::Unknown, dimensions: 0 }, width, middle, 100, false));
        Sized { right: style::terminal() / 2, outputs: vec![(width, middle)], ..Sized::new(width, height, shapes) }
    }

    /// The pills left of the inputs of a node, outside (notation 3.2): its inline constants, which it draws, and the pills
    /// of the wires that reach it, which the wires draw. A pill that would overlap a pill above it steps outward, past
    /// it, and a constant so moved draws a wire to its terminal. How far the pills reach left of the block, and for each
    /// input with a pill how far left of its terminal the pill starts.
    fn outside_pills(&self, node: &Node<'_>, at: &[(usize, i64, i64)], shapes: &mut Vec<Shape>) -> (i64, Vec<Option<i64>>) {
        let mut left = style::terminal() / 2;
        let mut reaches = vec![None; node.inputs.len()];
        let mut order: Vec<(usize, i64, i64)> = at.to_vec();
        order.sort_by_key(|&(_, _, y)| y);
        // The pills placed, from the top: the height of each, where it starts and ends.
        let mut placed: Vec<(i64, i64, i64)> = Vec::new();
        for (input, x, y) in order {
            let Some(terminal_of) = node.inputs.get(input) else { continue };
            let Some((text, constant)) = pill_text(terminal_of) else { continue };
            let width = pill_width(&text);
            let edge = x - style::terminal() / 2;
            let mut end = edge - style::grid();
            // Past each pill above that it would overlap: each move leaves one pill behind for good, so that a pill moves
            // at most once past each of the few that are less than a pill above it.
            loop {
                let mut moved = false;
                for &(above, start, stop) in placed.iter().rev() {
                    if y - above >= style::pill_height() {
                        break;
                    }
                    if end + style::grid() > start && end - width < stop + style::grid() {
                        end = start - style::grid();
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
            let start = end - width;
            if constant {
                shapes.extend(pill(&text, start, y));
                if end < edge - style::grid() {
                    shapes.push(lead(terminal_of.family, (end, y), (edge, y)));
                }
            }
            if let Some(slot) = reaches.get_mut(input) {
                *slot = Some(x - start);
            }
            left = left.max(-start);
            placed.push((y, start, end));
        }
        (left, reaches)
    }

    /// A compact block (notation 3.2): `node.square` wide, as tall as its places need and at least as tall as wide, the
    /// icon or the name in its centre. Neither the name nor the band makes it larger.
    fn square(&self, node: &Node<'_>, short: &str, band: Option<&ShownText>, icon: Option<usize>) -> Sized {
        let width = block_width();
        let places = places(node, style::square());
        let side = places.height;
        // The name of a node `always` on one line, between the band and its `▴`, which it never meets.
        let label = match icon {
            Some(_) => Vec::new(),
            None if node.always => vec![Text::new(short, name_cells(), label_size())],
            None => label_lines(short, name_cells()),
        };
        let band = band.map(|band| Text::measured(band.clone(), small()));
        let mut shapes = Vec::new();
        let top = body(band.as_ref(), band_insets(node), width, side, &mut shapes);
        // In the centre of what the band leaves.
        let centre = top + (side - top) / 2;
        match icon {
            Some(icon) => shapes.push(Shape::Icon { x: width / 2 - style::icon() / 2, y: centre - style::icon() / 2, side: style::icon(), icon }),
            None => {
                let height = line_height(label_size());
                let centre = if node.always { (top + always_top(side)) / 2 } else { centre };
                let top = centre - height * label.len() as i64 / 2;
                for (row, line) in label.iter().enumerate() {
                    shapes.push(line.centred(width / 2, top + height * row as i64 + height / 2, "node.title"));
                }
            }
        }
        let drawn = self.terminals(node, &places, width, &mut shapes);
        let order = self.compact_marks(node, width, side, &mut shapes);
        compact(width, side, shapes, drawn, order)
    }

    /// The terminals of a block on its places, and the pills of its constants.
    fn terminals(&self, node: &Node<'_>, places: &Places, width: i64, shapes: &mut Vec<Shape>) -> AtPlaces {
        let mut pills = Vec::new();
        let mut inputs = vec![(0, 0); node.inputs.len()];
        let mut outputs = vec![(width, 0); node.outputs.len()];
        for (line, &y) in places.lines.iter().zip(&places.ys) {
            if let Some(input) = line.0.and_then(|index| node.inputs.get(index).map(|terminal| (index, terminal))) {
                shapes.extend(terminal(input.1.family, 0, y, 100, false));
                pills.push((input.0, 0, y));
                if let Some(at) = inputs.get_mut(input.0) {
                    *at = (0, y);
                }
            }
            if let Some((index, output)) = line.1.and_then(|index| node.outputs.get(index).map(|output| (index, output))) {
                shapes.extend(terminal(output.family, width, y, 100, output.ignored));
                if let Some(at) = outputs.get_mut(index) {
                    *at = (width, y);
                }
            }
        }
        let (left, reaches) = self.outside_pills(node, &pills, shapes);
        AtPlaces { left, inputs, outputs, reaches }
    }

    /// What the compact view never hides (notation 3.2): `always` as `▴` at the bottom, the `after:` terminal, whose
    /// centre it gives.
    fn compact_marks(&self, node: &Node<'_>, width: i64, height: i64, shapes: &mut Vec<Shape>) -> Option<(i64, i64)> {
        if node.always {
            let mark = Text::new("\u{25B4}", 1, medium());
            shapes.push(mark.centred(width / 2, always_top(height) + line_height(medium()) / 2, "node.title"));
        }
        if !node.after {
            return None;
        }
        shapes.push(order_terminal(style::grid(), 0));
        Some((style::grid(), 0))
    }

    /// A construction, a variant constructor or a node of `ui`: as wide as its label, never a square (notation 3.2). Its
    /// band, cut to its width, never makes it wider.
    fn oblong(&self, node: &Node<'_>, label: Vec<Text>, band: Option<&ShownText>) -> Sized {
        let band = band.map(|band| Text::measured(band.clone(), small()));
        let width = oblong_width(&label);
        let band_height = if band.is_some() { style::band_height() } else { 0 };
        let places = places(node, up(style::square().max(band_height + label_height(&label) + 2 * style::grid())));
        let height = places.height;
        let mut shapes = Vec::new();
        let below = body(band.as_ref(), band_insets(node), width, height, &mut shapes);
        let row = line_height(label_size());
        let top = below + (height - below) / 2 - row * label.len() as i64 / 2;
        for (index, line) in label.iter().enumerate() {
            shapes.push(line.centred(width / 2, top + row * index as i64 + row / 2, "node.title"));
        }
        let drawn = self.terminals(node, &places, width, &mut shapes);
        let order = self.compact_marks(node, width, height, &mut shapes);
        compact(width, height, shapes, drawn, order)
    }

    /// A primitive of `core` with a glyph (notation 3.2): a D with its right corners chamfered, 32 × 32, its terminals on
    /// the grid. A unit that does not hold makes it wider; more than three inputs, or more than one output, make it
    /// taller, by two places at a time.
    fn primitive(&self, node: &Node<'_>, glyph: &Glyph) -> Sized {
        let (text, struck) = match glyph {
            Glyph::Sign(sign) => (Text::new(sign, any_text(), large()), false),
            Glyph::Unit { unit, struck } => (Text::new(unit, any_text(), large()), *struck),
        };
        let width = style::primitive().max(up(text.width + 600));
        let (grid, pitch) = (style::grid(), style::compact_pitch());
        let count = node.inputs.len();
        // More than one output (targets too many, an error of `check` that is still drawn) on the straight part of the right
        // edge, one per place from its top, so that no two wires leave from one point and none leaves the outline.
        let many = node.outputs.len() as i64;
        let first = up(style::chamfer().max(grid));
        let mut height = grown(count, style::primitive());
        if many > 1 {
            height = grown_to(first.saturating_add((many - 1).saturating_mul(pitch)).saturating_add(style::chamfer()), height);
        }
        let middle = height / 2;
        // One input at the middle, two on the first and last places, three on the first, the middle and the last; more
        // from the top, one per place (an error of `check` that is still drawn, or a variadic `add`).
        let ys: Vec<i64> = match count {
            0 => Vec::new(),
            1 => vec![middle],
            2 => vec![grid, height - grid],
            3 => vec![grid, middle, height - grid],
            _ => (0..count as i64).map(|index| grid + index * pitch).collect(),
        };
        let chamfer = style::chamfer().min(height / 2).min(width / 2);
        let mut shapes = vec![Shape::Poly {
            points: vec![(0, 0), (width - chamfer, 0), (width, chamfer), (width, height - chamfer), (width - chamfer, height), (0, height)],
            closed: true,
            paint: Paint::both("node.fill", "node.stroke", style::outline()),
        }];
        shapes.push(text.centred(width / 2, middle, "node.title"));
        if struck {
            shapes.push(Shape::Poly {
                points: vec![(width / 2 - text.width / 2, middle), (width / 2 + text.width / 2, middle)],
                closed: false,
                paint: Paint::stroke("node.title", style::outline()),
            });
        }
        let mut pills = Vec::new();
        let mut inputs = Vec::new();
        for (index, (terminal_of, &y)) in node.inputs.iter().zip(&ys).enumerate() {
            shapes.extend(terminal(terminal_of.family, 0, y, 100, false));
            pills.push((index, 0, y));
            inputs.push((0, y));
        }
        let outputs: Vec<(i64, i64)> = if many > 1 { (0..many).map(|index| (width, first + index * pitch)).collect() } else { vec![(width, middle)] };
        for (output, &(x, y)) in node.outputs.iter().zip(&outputs) {
            shapes.extend(terminal(output.family, x, y, 100, output.ignored));
        }
        let (left, reaches) = self.outside_pills(node, &pills, &mut shapes);
        let order = self.compact_marks(node, width, height, &mut shapes);
        Sized { left, right: style::terminal() / 2, inputs, outputs, order, reaches, ..Sized::new(width, height, shapes) }
    }

    /// A block in standard view (notation 3.1, with the numbers decided at step 2 of M0v): the name as a header, the
    /// library in muted ink, named terminals on lines 24 apart.
    fn standard(&self, node: &Node<'_>, title: &str) -> Sized {
        let lines = lines(&node.inputs, &node.outputs);
        let title_text = Text::new(title, any_text(), large());
        // The library, up to the last dot of what is drawn, in `ink.muted`.
        let drawn = title_text.shown.text();
        let library = drawn.rfind('.').filter(|_| matches!(node.kind, Kind::Call { .. })).map(|dot| dot + 1);
        let always = node.always.then(|| Text::new("always", any_text(), medium()));
        let mut header = style::side() + title_text.width + style::side();
        if let Some(always) = &always {
            header += style::grid() + always.width + 2 * style::padding();
        }
        let names = |terminal: &Terminal, constant: bool| -> (Text, Option<Text>, Option<Text>) {
            let name = Text::new(&terminal.name, any_text(), large());
            let unit = terminal.unit.as_ref().map(|unit| Text::new(&format!("[{unit}]"), any_text(), medium()));
            let pill = if constant { terminal.constant.as_ref().map(|constant| Text::new(constant, any_text(), medium())) } else { None };
            (name, unit, pill)
        };
        let side_width = |(name, unit, pill): &(Text, Option<Text>, Option<Text>)| {
            name.width
                + unit.as_ref().map_or(0, |unit| style::padding() + unit.width)
                + pill.as_ref().map_or(0, |pill| style::grid() + pill_width(pill))
        };
        let mut widest = 0;
        let mut rows = Vec::new();
        for line in &lines {
            let input = line.0.and_then(|index| node.inputs.get(index)).map(|terminal| names(terminal, true));
            let output = line.1.and_then(|index| node.outputs.get(index)).map(|terminal| names(terminal, false));
            let width = match (&input, &output) {
                (Some(input), Some(output)) => style::side() + side_width(input) + style::between() + side_width(output) + style::side(),
                (Some(one), None) | (None, Some(one)) => style::side() + side_width(one) + style::side(),
                (None, None) => 0,
            };
            widest = widest.max(width);
            rows.push((input, output));
        }
        let width = up(style::standard_width().max(header).max(widest));
        let count = lines.len().max(1) as i64;
        let height = style::first_line() + style::pitch() * (count - 1) + style::last_line();
        let mut shapes = vec![
            Shape::Rect { x: 0, y: 0, width, height, radius: style::radius(), paint: Paint::both("node.fill", "node.stroke", style::outline()) },
            Shape::Poly {
                points: vec![(0, style::standard_header()), (width, style::standard_header())],
                closed: false,
                paint: Paint::stroke("node.divider", style::outline()),
            },
        ];
        // The header: the library part muted, the name in the ink of titles.
        let mut x = style::side();
        match library {
            Some(split) => {
                let (muted, name) = (drawn.get(..split).unwrap_or_default(), drawn.get(split..).unwrap_or_default());
                let muted = Text::measured(ShownText::plain(muted), large());
                shapes.push(muted.at(x, style::standard_header() / 2, "ink.muted"));
                x += muted.width;
                let name = Text::measured(ShownText::plain(name), large());
                shapes.push(name.at(x, style::standard_header() / 2, "node.title"));
                x += name.width;
            }
            None => {
                shapes.push(title_text.at(x, style::standard_header() / 2, "node.title"));
                x += title_text.width;
            }
        }
        if let Some(always) = &always {
            x += style::grid();
            shapes.push(Shape::Rect {
                x,
                y: style::standard_header() / 2 - style::grid(),
                width: always.width + 2 * style::padding(),
                height: 2 * style::grid(),
                radius: 300,
                paint: Paint::fill("badge.fill"),
            });
            shapes.push(always.at(x + style::padding(), style::standard_header() / 2, "badge.text"));
        }
        if node.after {
            shapes.push(order_terminal(0, style::standard_header() / 2));
        }
        let mut inputs = vec![(0, 0); node.inputs.len()];
        let mut outputs = vec![(width, 0); node.outputs.len()];
        for (index, (line, (input, output))) in lines.iter().zip(&rows).enumerate() {
            let y = style::first_line() + style::pitch() * index as i64;
            if let Some(at) = line.0.and_then(|at| inputs.get_mut(at)) {
                *at = (0, y);
            }
            if let Some(at) = line.1.and_then(|at| outputs.get_mut(at)) {
                *at = (width, y);
            }
            if let (Some(terminal_of), Some((name, unit, pill_text))) = (line.0.and_then(|at| node.inputs.get(at)), input) {
                shapes.extend(terminal(terminal_of.family, 0, y, 100, false));
                shapes.push(name.shown_from(style::side(), y, "node.title", Detail::Name));
                let mut x = style::side() + name.width;
                if let Some(unit) = unit {
                    shapes.push(unit.shown_from(x + style::padding(), y, "ink.muted", Detail::Name));
                    x += style::padding() + unit.width;
                }
                if let Some(pill_text) = pill_text {
                    shapes.extend(pill(pill_text, x + style::grid(), y));
                }
            }
            if let (Some(terminal_of), Some((name, unit, _))) = (line.1.and_then(|at| node.outputs.get(at)), output) {
                shapes.extend(terminal(terminal_of.family, width, y, 100, terminal_of.ignored));
                let mut x = width - style::side();
                if let Some(unit) = unit {
                    x -= unit.width;
                    shapes.push(unit.shown_from(x, y, "ink.muted", Detail::Name));
                    x -= style::padding();
                }
                shapes.push(name.shown_from(x - name.width, y, "node.title", Detail::Name));
            }
        }
        let order = node.after.then_some((0, style::standard_header() / 2));
        Sized { left: style::terminal() / 2, right: style::terminal() / 2, inputs, outputs, order, ..Sized::new(width, height, shapes) }
    }

    /// The icon of the function, in the numbers its file writes (spec 11.1); its texts cut at 40 cells (decided at
    /// step 2 of M0v), the grid cutting the rest when it is drawn.
    pub fn icon(&self, icon: &labcraft_graph::Icon) -> IconDrawing {
        let number = |numeral: &Numeral| -> i64 {
            match numeral.kind {
                NumeralKind::Int(name) | NumeralKind::Float(name) => icon_number(self.names.resolve(name), numeral.negative),
                NumeralKind::Inf | NumeralKind::Nan => 0,
            }
        };
        let text_of = |value: i64| crate::svg::decimal(i128::from(value), ICON_DECIMALS);
        // In hundredths, exact in an f64 within the limit of a coordinate.
        let float = |value: i64| value as f64 / 100.0;
        let items = icon
            .items
            .iter()
            .map(|item| {
                let token = item.accent.as_ref().and_then(|path| accent(path, self.names)).unwrap_or("node.title");
                let (shape, drawn) = match &item.shape {
                    Written::Rect { from, to, radius } => {
                        // A rect is written by its corners, the SVG wants a corner and a size.
                        let (x1, x2, y1, y2) = (number(&from.x), number(&to.x), number(&from.y), number(&to.y));
                        let (width, height) = (x1.abs_diff(x2).try_into().unwrap_or(i64::MAX), y1.abs_diff(y2).try_into().unwrap_or(i64::MAX));
                        let radius = radius.as_ref().map(number);
                        (
                            IconShape::Rect {
                                x: text_of(x1.min(x2)),
                                y: text_of(y1.min(y2)),
                                width: text_of(width),
                                height: text_of(height),
                                radius: radius.map(text_of),
                            },
                            IconDrawn::Rect {
                                x: float(x1.min(x2)),
                                y: float(y1.min(y2)),
                                width: float(width),
                                height: float(height),
                                radius: float(radius.unwrap_or(0)),
                            },
                        )
                    }
                    Written::Circle { center, radius } => {
                        let (x, y, radius) = (number(&center.x), number(&center.y), number(radius));
                        (
                            IconShape::Circle { x: text_of(x), y: text_of(y), radius: text_of(radius) },
                            IconDrawn::Circle { x: float(x), y: float(y), radius: float(radius) },
                        )
                    }
                    Written::Line { from, to } => {
                        let (x1, y1, x2, y2) = (number(&from.x), number(&from.y), number(&to.x), number(&to.y));
                        (
                            IconShape::Line { x1: text_of(x1), y1: text_of(y1), x2: text_of(x2), y2: text_of(y2) },
                            IconDrawn::Line { from: (float(x1), float(y1)), to: (float(x2), float(y2)) },
                        )
                    }
                    // As written: `check` has read its syntax (`LC5008`), and the icon is drawn once per SVG.
                    Written::Path { data } => {
                        (IconShape::Path { data: data.to_string() }, IconDrawn::Path(labcraft_check::path_segments(data).unwrap_or_default()))
                    }
                    Written::Text { text: written, at, size } => {
                        let shown = text::show(&labcraft_check::read_escapes(written), any_text());
                        let (x, y, size) = (number(&at.x), number(&at.y), size.as_ref().map_or(1000, number));
                        (
                            IconShape::Text {
                                x: text_of(x),
                                y: text_of(y),
                                size: text_of(size),
                                runs: shown.runs.iter().map(|(ink, run)| Run { token: token_of(*ink, token), text: run.clone() }).collect(),
                            },
                            IconDrawn::Text { x: float(x), y: float(y), size: float(size) },
                        )
                    }
                };
                IconItem { shape, drawn, fill: item.fill, token }
            })
            .collect();
        let grid = text_of(icon_number(self.names.resolve(icon.grid), false));
        IconDrawing { grid, items }
    }
}

/// An accent token of an icon (spec 11.1): a token of notation 8 whose parts are names; `labcraft_check::icon_refused`
/// has already refused an icon whose accent is none, or a colour of a state of the viewer.
fn accent(path: &labcraft_graph::Path, names: &Interner) -> Option<Token> {
    let mut text = names.resolve(path.first).to_owned();
    for segment in &path.rest {
        text.push('.');
        text.push_str(names.resolve(*segment));
    }
    crate::theme::TOKENS.iter().map(|token| token.name).find(|name| *name == text)
}

/// The decimals of a number of an icon, those of every number of the SVG (cli.md 7.2): a hundredth of a grid of 32
/// drawn 28 wide is less than a hundredth of a unit.
const ICON_DECIMALS: u32 = 2;
/// A million, in hundredths. Beyond it a coordinate is far off a grid of 32 (`LC5009`) and is drawn as a million: the
/// icon is cut to its grid, so nothing that it shows changes, and no number of the SVG needs an exponent (cli.md 7.2).
const ICON_LIMIT: f64 = 1e8;

/// A number of an icon as its file writes it (`1_000`, `3e1`, `2.5`), in hundredths: read as a float, which gives the
/// same on every platform and in WASM, rounded to the nearest, and held within a million.
pub fn icon_number(written: &str, negative: bool) -> i64 {
    let value = written.replace('_', "").parse::<f64>().unwrap_or(0.0);
    let scaled = (value * 100.0).round().clamp(-ICON_LIMIT, ICON_LIMIT);
    // Within the limit, an exact integer of an f64; `as` saturates, and a NaN, which `clamp` keeps, gives 0.
    let magnitude = scaled as i64;
    if negative { -magnitude } else { magnitude }
}
