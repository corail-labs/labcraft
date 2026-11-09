//! Step 2 (`docs/architecture.md`, 11.3): the size of each node in its view, compact or standard (notation 3.1, 3.2),
//! every text cut in cells by `types::text`, and the shapes of the node around its block. All in hundredths of a unit,
//! relative to the top-left corner of the block, which is what `node x at (40, 80)` places (spec 11).

use std::collections::{HashMap, VecDeque};

use labcraft_graph::{IconShape as Written, Numeral, NumeralKind};
use labcraft_types::text::{self, ShownText};
use labcraft_types::{Interner, TyId, cut_text};

use crate::graph::{Base, Family, Glyph, Kind, Node, Terminal};
use crate::shape::{IconDrawing, IconItem, IconShape, Paint, Run, Shape, Token, token_of};

/// The grid of the canvas, 8 units (notation 2).
pub const GRID: i64 = 800;
/// The three sizes of text (notation 2.3).
pub const LARGE: u32 = 1200;
pub const MEDIUM: u32 = 1100;
pub const SMALL: u32 = 900;
/// A text that no other rule cuts (notation 2.3).
pub const ANY_TEXT: u64 = 40;
/// A line of a label, a band (notation 3.2).
pub const LABEL_LINE: u64 = 12;
/// An inline constant in compact view (notation 3.2).
pub const COMPACT_CONSTANT: u64 = 8;

pub const TERMINAL: i64 = 1000;
pub const PILL_HEIGHT: i64 = 1800;
const BAND_HEIGHT: i64 = 1200;
pub const PADDING: i64 = 400;
const LABEL_PADDING: i64 = 900;
const SQUARE: i64 = 4000;
const CIRCLE: i64 = 2800;
const ICON: i64 = 2800;
pub const OUTLINE: i64 = 100;
pub const RADIUS: i64 = 400;

/// Notation 3.1, decided at step 2 of M0v: the numbers of `design/notation.figcraft` on the grid.
const HEADER: i64 = 2400;
const FIRST_LINE: i64 = 4000;
const PITCH: i64 = 2400;
const LAST_LINE: i64 = 1600;
const SIDE: i64 = 1200;
const BETWEEN: i64 = 2400;
const STANDARD_WIDTH: i64 = 11200;

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
        }
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
    let rest = value.rem_euclid(GRID);
    if rest == 0 { value } else { value.saturating_add(GRID - rest) }
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
    for (other, &output) in outs.iter().enumerate() {
        let Some(out) = outputs.get(output).filter(|out| out.family.passes_through()) else { continue };
        by_name.entry((out.name.as_str(), out.ty)).or_default().push_back(other);
        by_type.entry(out.ty).or_default().push_back(other);
    }
    for named in [true, false] {
        for (slot, &input) in ins.iter().enumerate() {
            let Some(terminal) = inputs.get(input).filter(|terminal| terminal.family.passes_through()) else { continue };
            if paired_in.get(slot).copied().flatten().is_some() {
                continue;
            }
            let queue = if named { by_name.get_mut(&(terminal.name.as_str(), terminal.ty)) } else { by_type.get_mut(&terminal.ty) };
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
    /// How far the shapes reach beyond the block: above (band, label), to the left (pills), to the right.
    pub above: i64,
    pub left: i64,
    pub right: i64,
    pub shapes: Vec<Shape>,
    /// The centre of each terminal, in the order of the node's inputs and outputs, from the top-left of the block.
    pub inputs: Vec<(i64, i64)>,
    pub outputs: Vec<(i64, i64)>,
}

impl Sized {
    pub fn new(width: i64, height: i64, shapes: Vec<Shape>) -> Sized {
        Sized { width, height, above: 0, left: 0, right: 0, shapes, inputs: Vec::new(), outputs: Vec::new() }
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

/// A terminal centred on `(x, y)`, its shape telling its family (notation 4.1), at `scale` percent.
pub fn terminal(family: Family, x: i64, y: i64, scale: i64, ignored: bool) -> Vec<Shape> {
    let half = TERMINAL * scale / 200;
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
    let half = TERMINAL / 2;
    Shape::Poly { points: vec![(x - half, y - half), (x + half, y), (x - half, y + half)], closed: false, paint: Paint::stroke("wire.order", 150) }
}

/// A pill of an inline constant: its text in 11, 4 on each side, 18 high (notation 2.3, 3.2).
pub fn pill(text: &Text, x: i64, middle: i64) -> Vec<Shape> {
    let width = text.width + 2 * PADDING;
    vec![
        Shape::Rect {
            x,
            y: middle - PILL_HEIGHT / 2,
            width,
            height: PILL_HEIGHT,
            radius: PILL_HEIGHT / 2,
            paint: Paint::both("pill.fill", "pill.stroke", OUTLINE),
        },
        text.at(x + PADDING, middle, "pill.text"),
    ]
}

pub fn pill_width(text: &Text) -> i64 {
    text.width + 2 * PADDING
}

/// The label of a compact block: the last segment of the name, on two lines at an underscore when that makes the square
/// smaller (notation 3.2), each line cut at 12 cells.
fn label_lines(name: &str) -> Vec<Vec<Text>> {
    let one = vec![Text::new(name, LABEL_LINE, LARGE)];
    let mut best: Option<(u64, Vec<Text>)> = None;
    // The underscores of the first 200 code points: no line of more than 12 cells is drawn anyway.
    let name = cut_text(name);
    for (at, _) in name.match_indices('_') {
        let (first, second) = (name.get(..at).unwrap_or_default(), name.get(at + 1..).unwrap_or_default());
        if first.is_empty() || second.is_empty() {
            continue;
        }
        let longer = text::cells(first).max(text::cells(second));
        if best.as_ref().is_none_or(|(cells, _)| longer < *cells) {
            best = Some((longer, vec![Text::new(first, LABEL_LINE, LARGE), Text::new(second, LABEL_LINE, LARGE)]));
        }
    }
    let mut candidates = vec![one];
    candidates.extend(best.map(|(_, lines)| lines));
    candidates
}

fn label_width(lines: &[Text]) -> i64 {
    lines.iter().map(|line| line.width).max().unwrap_or(0)
}

fn label_height(lines: &[Text]) -> i64 {
    line_height(LARGE).saturating_mul(lines.len() as i64)
}

/// Whether a line carries an inline constant: it keeps 16 from its neighbours, 24 from one that carries one too.
fn carries(line: &Line, inputs: &[Terminal]) -> bool {
    line.0.and_then(|input| inputs.get(input)).is_some_and(|terminal| terminal.constant.is_some())
}

/// The gaps between the lines, from the top: as the notation spaces them, and at the closest (8 between wired lines).
fn gaps(lines: &[Line], inputs: &[Terminal]) -> (Vec<i64>, Vec<i64>) {
    let mut normal = Vec::new();
    let mut closest = Vec::new();
    for pair in lines.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else { continue };
        let (a, b) = (carries(a, inputs), carries(b, inputs));
        let gap = if a && b { 2400 } else { 1600 };
        normal.push(gap);
        closest.push(if a || b { gap } else { GRID });
    }
    (normal, closest)
}

/// The heights of the lines, from the top, the last one `bottom` above the lower edge of a block `height` high.
fn heights(gaps: &[i64], height: i64, bottom: i64) -> Vec<i64> {
    let mut y = height - bottom;
    let mut found = vec![y];
    for gap in gaps.iter().rev() {
        y -= gap;
        found.push(y);
    }
    found.reverse();
    found
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
            Kind::Call { glyph: Some(glyph), .. } if !standard => self.circle(node, glyph),
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
                    self.oblong(node, vec![Text::new(ty, LABEL_LINE, LARGE)], band)
                }
            }
            Kind::Variant { ty, variant, .. } => {
                if standard {
                    self.standard(node, &format!("{ty}.{variant}"))
                } else {
                    self.oblong(node, vec![Text::new(ty, LABEL_LINE, LARGE), Text::new(variant, LABEL_LINE, LARGE)], band)
                }
            }
            Kind::Ui { operation, element } => {
                if standard {
                    self.standard(node, &format!("ui.{operation} {element}"))
                } else {
                    self.oblong(node, vec![Text::new(operation, LABEL_LINE, LARGE), Text::new(element, LABEL_LINE, LARGE)], None)
                }
            }
            Kind::Structure(_) => Sized::new(0, 0, Vec::new()),
        };
        // A terminal that no line holds stays at the middle of the left or the right edge.
        sized.inputs.resize(node.inputs.len(), (0, sized.height / 2));
        sized.outputs.resize(node.outputs.len(), (sized.width, sized.height / 2));
        if let Some(label) = &node.label {
            // Above the band, or above the block (notation 3.1, 3.2).
            let text = Text::new(&format!("{label}:"), ANY_TEXT, MEDIUM);
            let middle = -sized.above - 400 - line_height(MEDIUM) / 2;
            sized.shapes.push(text.at(0, middle, "ink.muted"));
            sized.above += 400 + line_height(MEDIUM);
            sized.right = sized.right.max(text.width - sized.width);
        }
        sized
    }

    /// A named constant, or a construction folded: its name, its value in a pill, one output (notation 3.1).
    fn constant(&self, name: &str, value: &str) -> Sized {
        let name = Text::new(name, ANY_TEXT, LARGE);
        let value = Text::new(value, ANY_TEXT, MEDIUM);
        let height = 2400;
        let middle = height / 2;
        let mut shapes = vec![name.at(0, middle, "node.title")];
        let pill_x = name.width + GRID;
        shapes.extend(pill(&value, pill_x, middle));
        let width = pill_x + pill_width(&value) + GRID;
        shapes.extend(terminal(Family { base: Base::Unknown, dimensions: 0 }, width, middle, 100, false));
        Sized { right: TERMINAL / 2, outputs: vec![(width, middle)], ..Sized::new(width, height, shapes) }
    }

    /// The pills of the inline constants of a compact node, left of its inputs, outside (notation 3.2).
    fn outside_pills(&self, node: &Node<'_>, at: &[(usize, i64, i64)], shapes: &mut Vec<Shape>) -> i64 {
        let mut left = TERMINAL / 2;
        for &(input, x, y) in at {
            let Some(constant) = node.inputs.get(input).and_then(|terminal| terminal.constant.as_deref()) else { continue };
            let text = Text::new(constant, COMPACT_CONSTANT, MEDIUM);
            let start = x - TERMINAL / 2 - GRID - pill_width(&text);
            shapes.extend(pill(&text, start, y));
            left = left.max(-start);
        }
        left
    }

    /// A compact block: a square of 40 at least, the label or the icon in its centre (notation 3.2).
    fn square(&self, node: &Node<'_>, short: &str, band: Option<&ShownText>, icon: Option<usize>) -> Sized {
        let lines = lines(&node.inputs, &node.outputs);
        let (normal, closest) = gaps(&lines, &node.inputs);
        let need = |gaps: &[i64]| GRID + gaps.iter().sum::<i64>() + GRID;
        let side_of = |label: &[Text]| {
            let mut side = SQUARE.max(need(&closest));
            if icon.is_none() {
                side = side.max(label_width(label) + 2 * LABEL_PADDING).max(label_height(label) + 2 * GRID);
            }
            up(side)
        };
        let mut label = Vec::new();
        let mut side = i64::MAX;
        for candidate in label_lines(short) {
            let candidate_side = side_of(&candidate);
            if candidate_side < side {
                (side, label) = (candidate_side, candidate);
            }
        }
        let band = band.map(|band| Text::measured(band.clone(), SMALL));
        let width = band.as_ref().map_or(side, |band| side.max(up(band.width + 2 * PADDING)));
        let shrunk = need(&normal) > side;
        let ys = heights(if shrunk { &closest } else { &normal }, side, GRID);
        let mut shapes =
            vec![Shape::Rect { x: 0, y: 0, width, height: side, radius: RADIUS, paint: Paint::both("node.fill", "node.stroke", OUTLINE) }];
        let mut above = 0;
        if let Some(band) = &band {
            shapes.push(Shape::Rect {
                x: 0,
                y: -BAND_HEIGHT,
                width,
                height: BAND_HEIGHT,
                radius: 0,
                paint: Paint::both("band.fill", "node.stroke", OUTLINE),
            });
            shapes.push(band.centred(width / 2, -BAND_HEIGHT / 2, "band.text"));
            above = BAND_HEIGHT;
        }
        // In the centre of the block, which a band can make wider than the square.
        match icon {
            Some(icon) => shapes.push(Shape::Icon { x: width / 2 - ICON / 2, y: side / 2 - ICON / 2, side: ICON, icon }),
            None => {
                let height = line_height(LARGE);
                let top = side / 2 - height * label.len() as i64 / 2;
                for (row, line) in label.iter().enumerate() {
                    shapes.push(line.centred(width / 2, top + height * row as i64 + height / 2, "node.title"));
                }
            }
        }
        let (left, inputs, outputs) = self.terminals(node, &lines, &ys, width, if shrunk { 70 } else { 100 }, &mut shapes);
        self.compact_marks(node, width, side, band.is_some(), &mut shapes);
        Sized { width, height: side, above, left, right: TERMINAL / 2, shapes, inputs, outputs }
    }

    /// The terminals of a block on its lines, and the pills of its constants; how far the pills reach to the left, and
    /// where each terminal is.
    #[allow(clippy::type_complexity)]
    fn terminals(
        &self,
        node: &Node<'_>,
        lines: &[Line],
        ys: &[i64],
        width: i64,
        scale: i64,
        shapes: &mut Vec<Shape>,
    ) -> (i64, Vec<(i64, i64)>, Vec<(i64, i64)>) {
        let mut pills = Vec::new();
        let mut inputs = vec![(0, 0); node.inputs.len()];
        let mut outputs = vec![(width, 0); node.outputs.len()];
        for (line, &y) in lines.iter().zip(ys) {
            if let Some(input) = line.0.and_then(|index| node.inputs.get(index).map(|terminal| (index, terminal))) {
                shapes.extend(terminal(input.1.family, 0, y, scale, false));
                pills.push((input.0, 0, y));
                if let Some(at) = inputs.get_mut(input.0) {
                    *at = (0, y);
                }
            }
            if let Some((index, output)) = line.1.and_then(|index| node.outputs.get(index).map(|output| (index, output))) {
                shapes.extend(terminal(output.family, width, y, scale, output.ignored));
                if let Some(at) = outputs.get_mut(index) {
                    *at = (width, y);
                }
            }
        }
        (self.outside_pills(node, &pills, shapes), inputs, outputs)
    }

    /// What the compact view never hides (notation 3.2): `always` as `▴` at the bottom, the `after:` terminal.
    fn compact_marks(&self, node: &Node<'_>, width: i64, height: i64, band: bool, shapes: &mut Vec<Shape>) {
        if node.always {
            let mark = Text::new("\u{25B4}", 1, MEDIUM);
            shapes.push(mark.centred(width / 2, height - GRID, "node.title"));
        }
        if node.after {
            shapes.push(if band { order_terminal(0, -BAND_HEIGHT / 2) } else { order_terminal(GRID, 0) });
        }
    }

    /// A construction, a variant constructor or a node of `ui`: as wide as its label, never a square (notation 3.2).
    fn oblong(&self, node: &Node<'_>, label: Vec<Text>, band: Option<&ShownText>) -> Sized {
        let lines = lines(&node.inputs, &node.outputs);
        let (normal, _) = gaps(&lines, &node.inputs);
        let need = GRID + normal.iter().sum::<i64>() + GRID;
        let band = band.map(|band| Text::measured(band.clone(), SMALL));
        let mut width = up(SQUARE.max(label_width(&label) + 2 * LABEL_PADDING));
        if let Some(band) = &band {
            width = width.max(up(band.width + 2 * PADDING));
        }
        let height = up(SQUARE.max(need).max(label_height(&label) + 2 * GRID));
        let ys = heights(&normal, height, GRID);
        let mut shapes = vec![Shape::Rect { x: 0, y: 0, width, height, radius: RADIUS, paint: Paint::both("node.fill", "node.stroke", OUTLINE) }];
        let mut above = 0;
        if let Some(band) = &band {
            shapes.push(Shape::Rect {
                x: 0,
                y: -BAND_HEIGHT,
                width,
                height: BAND_HEIGHT,
                radius: 0,
                paint: Paint::both("band.fill", "node.stroke", OUTLINE),
            });
            shapes.push(band.centred(width / 2, -BAND_HEIGHT / 2, "band.text"));
            above = BAND_HEIGHT;
        }
        let row = line_height(LARGE);
        let top = height / 2 - row * label.len() as i64 / 2;
        for (index, line) in label.iter().enumerate() {
            shapes.push(line.centred(width / 2, top + row * index as i64 + row / 2, "node.title"));
        }
        let (left, inputs, outputs) = self.terminals(node, &lines, &ys, width, 100, &mut shapes);
        self.compact_marks(node, width, height, band.is_some(), &mut shapes);
        Sized { width, height, above, left, right: TERMINAL / 2, shapes, inputs, outputs }
    }

    /// A primitive of `core` with a glyph: a circle of 28, or a capsule (notation 3.2).
    fn circle(&self, node: &Node<'_>, glyph: &Glyph) -> Sized {
        let (text, struck) = match glyph {
            Glyph::Sign(sign) => (Text::new(sign, ANY_TEXT, LARGE), false),
            Glyph::Unit { unit, struck } => (Text::new(unit, ANY_TEXT, LARGE), *struck),
        };
        let width = if text.width + 600 > CIRCLE { text.width + 600 } else { CIRCLE };
        // The inputs from the middle: one at it, two at 8 above and below, three at 10; 24 apart where both neighbours
        // carry a constant, since a pill is 18 high.
        let count = node.inputs.len();
        let mut offsets: Vec<i64> = match count {
            0 => Vec::new(),
            1 => vec![0],
            2 => vec![-800, 800],
            3 => vec![-1000, 0, 1000],
            _ => (0..count as i64).map(|index| (index * 2 - (count as i64 - 1)) * 400).collect(),
        };
        // What the inputs before have moved, carried to those after: one pass.
        let mut stretched = false;
        let mut shift = 0;
        for index in 1..count {
            let both = [index - 1, index].iter().all(|&input| node.inputs.get(input).is_some_and(|terminal| terminal.constant.is_some()));
            let before = offsets.get(index - 1).copied();
            let Some(at) = offsets.get_mut(index) else { continue };
            *at += shift;
            if let Some(before) = before
                && both
                && *at - before < 2400
            {
                let more = 2400 - (*at - before);
                *at += more;
                shift += more;
                stretched = true;
            }
        }
        if stretched {
            let middle = (offsets.first().copied().unwrap_or(0) + offsets.last().copied().unwrap_or(0)) / 2;
            offsets.iter_mut().for_each(|offset| *offset -= middle);
        }
        let span = offsets.last().copied().unwrap_or(0) - offsets.first().copied().unwrap_or(0);
        // More than three inputs (an error that is still drawn) stretch it too, so that none is outside.
        let height = if stretched || count > 3 { CIRCLE.max(span + PILL_HEIGHT) } else { CIRCLE };
        let radius = width.min(height) / 2;
        let middle = height / 2;
        let mut shapes = vec![if width == height {
            Shape::Circle { x: radius, y: radius, radius, paint: Paint::both("node.fill", "node.stroke", OUTLINE) }
        } else {
            Shape::Rect { x: 0, y: 0, width, height, radius, paint: Paint::both("node.fill", "node.stroke", OUTLINE) }
        }];
        shapes.push(text.centred(width / 2, middle, "node.title"));
        if struck {
            shapes.push(Shape::Poly {
                points: vec![(width / 2 - text.width / 2, middle), (width / 2 + text.width / 2, middle)],
                closed: false,
                paint: Paint::stroke("node.title", OUTLINE),
            });
        }
        // On the edge: where the outline is round, the depth of the arc at that height, by an integer square root.
        let edge = |dy: i64| -> i64 {
            let straight = (height - width).max(0) / 2;
            let beyond = (dy.abs() - straight).max(0);
            if beyond >= radius { radius } else { radius - (radius * radius - beyond * beyond).isqrt() }
        };
        let mut pills = Vec::new();
        let mut inputs = Vec::new();
        for (index, (terminal_of, &offset)) in node.inputs.iter().zip(&offsets).enumerate() {
            let x = edge(offset);
            shapes.extend(terminal(terminal_of.family, x, middle + offset, 100, false));
            pills.push((index, x, middle + offset));
            inputs.push((x, middle + offset));
        }
        if let Some(output) = node.outputs.first() {
            shapes.extend(terminal(output.family, width, middle, 100, output.ignored));
        }
        let left = self.outside_pills(node, &pills, &mut shapes);
        self.compact_marks(node, width, height, false, &mut shapes);
        Sized { left, right: TERMINAL / 2, inputs, outputs: vec![(width, middle)], ..Sized::new(width, height, shapes) }
    }

    /// A block in standard view (notation 3.1, with the numbers decided at step 2 of M0v): the name as a header, the
    /// library in muted ink, named terminals on lines 24 apart.
    fn standard(&self, node: &Node<'_>, title: &str) -> Sized {
        let lines = lines(&node.inputs, &node.outputs);
        let title_text = Text::new(title, ANY_TEXT, LARGE);
        // The library, up to the last dot of what is drawn, in `ink.muted`.
        let drawn = title_text.shown.text();
        let library = drawn.rfind('.').filter(|_| matches!(node.kind, Kind::Call { .. })).map(|dot| dot + 1);
        let always = node.always.then(|| Text::new("always", ANY_TEXT, MEDIUM));
        let hidden = (node.hidden > 0).then(|| Text::new(&format!("+{}", node.hidden), ANY_TEXT, MEDIUM));
        let mut header = SIDE + title_text.width + SIDE;
        if let Some(always) = &always {
            header += GRID + always.width + 2 * PADDING;
        }
        if let Some(hidden) = &hidden {
            header += GRID + hidden.width;
        }
        let names = |terminal: &Terminal, constant: bool| -> (Text, Option<Text>, Option<Text>) {
            let name = Text::new(&terminal.name, ANY_TEXT, LARGE);
            let unit = terminal.unit.as_ref().map(|unit| Text::new(&format!("[{unit}]"), ANY_TEXT, MEDIUM));
            let pill = if constant { terminal.constant.as_ref().map(|constant| Text::new(constant, ANY_TEXT, MEDIUM)) } else { None };
            (name, unit, pill)
        };
        let side_width = |(name, unit, pill): &(Text, Option<Text>, Option<Text>)| {
            name.width + unit.as_ref().map_or(0, |unit| PADDING + unit.width) + pill.as_ref().map_or(0, |pill| GRID + pill_width(pill))
        };
        let mut widest = 0;
        let mut rows = Vec::new();
        for line in &lines {
            let input = line.0.and_then(|index| node.inputs.get(index)).map(|terminal| names(terminal, true));
            let output = line.1.and_then(|index| node.outputs.get(index)).map(|terminal| names(terminal, false));
            let width = match (&input, &output) {
                (Some(input), Some(output)) => SIDE + side_width(input) + BETWEEN + side_width(output) + SIDE,
                (Some(one), None) | (None, Some(one)) => SIDE + side_width(one) + SIDE,
                (None, None) => 0,
            };
            widest = widest.max(width);
            rows.push((input, output));
        }
        let width = up(STANDARD_WIDTH.max(header).max(widest));
        let count = lines.len().max(1) as i64;
        let height = FIRST_LINE + PITCH * (count - 1) + LAST_LINE;
        let mut shapes = vec![
            Shape::Rect { x: 0, y: 0, width, height, radius: RADIUS, paint: Paint::both("node.fill", "node.stroke", OUTLINE) },
            Shape::Poly { points: vec![(0, HEADER), (width, HEADER)], closed: false, paint: Paint::stroke("node.divider", OUTLINE) },
        ];
        // The header: the library part muted, the name in the ink of titles.
        let mut x = SIDE;
        match library {
            Some(split) => {
                let (muted, name) = (drawn.get(..split).unwrap_or_default(), drawn.get(split..).unwrap_or_default());
                let muted = Text::measured(ShownText::plain(muted), LARGE);
                shapes.push(muted.at(x, HEADER / 2, "ink.muted"));
                x += muted.width;
                let name = Text::measured(ShownText::plain(name), LARGE);
                shapes.push(name.at(x, HEADER / 2, "node.title"));
                x += name.width;
            }
            None => {
                shapes.push(title_text.at(x, HEADER / 2, "node.title"));
                x += title_text.width;
            }
        }
        if let Some(always) = &always {
            x += GRID;
            shapes.push(Shape::Rect {
                x,
                y: HEADER / 2 - GRID,
                width: always.width + 2 * PADDING,
                height: 2 * GRID,
                radius: 300,
                paint: Paint::fill("badge.fill"),
            });
            shapes.push(always.at(x + PADDING, HEADER / 2, "badge.text"));
            x += always.width + 2 * PADDING;
        }
        if let Some(hidden) = &hidden {
            x += GRID;
            shapes.push(hidden.at(x, HEADER / 2, "ink.muted"));
        }
        if node.after {
            shapes.push(order_terminal(0, HEADER / 2));
        }
        let mut inputs = vec![(0, 0); node.inputs.len()];
        let mut outputs = vec![(width, 0); node.outputs.len()];
        for (index, (line, (input, output))) in lines.iter().zip(&rows).enumerate() {
            let y = FIRST_LINE + PITCH * index as i64;
            if let Some(at) = line.0.and_then(|at| inputs.get_mut(at)) {
                *at = (0, y);
            }
            if let Some(at) = line.1.and_then(|at| outputs.get_mut(at)) {
                *at = (width, y);
            }
            if let (Some(terminal_of), Some((name, unit, pill_text))) = (line.0.and_then(|at| node.inputs.get(at)), input) {
                shapes.extend(terminal(terminal_of.family, 0, y, 100, false));
                shapes.push(name.at(SIDE, y, "node.title"));
                let mut x = SIDE + name.width;
                if let Some(unit) = unit {
                    shapes.push(unit.at(x + PADDING, y, "ink.muted"));
                    x += PADDING + unit.width;
                }
                if let Some(pill_text) = pill_text {
                    shapes.extend(pill(pill_text, x + GRID, y));
                }
            }
            if let (Some(terminal_of), Some((name, unit, _))) = (line.1.and_then(|at| node.outputs.get(at)), output) {
                shapes.extend(terminal(terminal_of.family, width, y, 100, terminal_of.ignored));
                let mut x = width - SIDE;
                if let Some(unit) = unit {
                    x -= unit.width;
                    shapes.push(unit.at(x, y, "ink.muted"));
                    x -= PADDING;
                }
                shapes.push(name.at(x - name.width, y, "node.title"));
            }
        }
        Sized { left: TERMINAL / 2, right: TERMINAL / 2, inputs, outputs, ..Sized::new(width, height, shapes) }
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
        let items = icon
            .items
            .iter()
            .map(|item| {
                let token = item.accent.as_ref().and_then(|path| accent(path, self.names)).unwrap_or("node.title");
                let shape = match &item.shape {
                    Written::Rect { from, to, radius } => {
                        // A rect is written by its corners, the SVG wants a corner and a size.
                        let (x1, x2, y1, y2) = (number(&from.x), number(&to.x), number(&from.y), number(&to.y));
                        IconShape::Rect {
                            x: text_of(x1.min(x2)),
                            y: text_of(y1.min(y2)),
                            width: text_of(x1.abs_diff(x2).try_into().unwrap_or(i64::MAX)),
                            height: text_of(y1.abs_diff(y2).try_into().unwrap_or(i64::MAX)),
                            radius: radius.as_ref().map(|radius| text_of(number(radius))),
                        }
                    }
                    Written::Circle { center, radius } => {
                        IconShape::Circle { x: text_of(number(&center.x)), y: text_of(number(&center.y)), radius: text_of(number(radius)) }
                    }
                    Written::Line { from, to } => IconShape::Line {
                        x1: text_of(number(&from.x)),
                        y1: text_of(number(&from.y)),
                        x2: text_of(number(&to.x)),
                        y2: text_of(number(&to.y)),
                    },
                    // As written: `check` has read its syntax (`LC5008`), and the icon is drawn once per SVG.
                    Written::Path { data } => IconShape::Path { data: data.to_string() },
                    Written::Text { text: written, at, size } => {
                        let shown = text::show(&labcraft_check::read_escapes(written), ANY_TEXT);
                        IconShape::Text {
                            x: text_of(number(&at.x)),
                            y: text_of(number(&at.y)),
                            size: size.as_ref().map_or_else(|| "10".to_owned(), |size| text_of(number(size))),
                            runs: shown.runs.iter().map(|(ink, run)| Run { token: token_of(*ink, token), text: run.clone() }).collect(),
                        }
                    }
                };
                IconItem { shape, fill: item.fill, token }
            })
            .collect();
        let grid = text_of(icon_number(self.names.resolve(icon.grid), false));
        IconDrawing { grid, items }
    }
}

/// An accent token of an icon (spec 11.1): a token of notation 8 whose parts are names.
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
