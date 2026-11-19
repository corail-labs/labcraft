//! The `outline` section of a function (spec 11.4): read once for `check`, which judges its items (`LC5016`) and its
//! geometry at every size that the block takes (`LC5017`), and for the drawing, which draws the outline that this gives,
//! or the ordinary block where a guard breaks, so that the two never disagree.
//!
//! All lengths are in hundredths of a unit, as the drawing counts them: the integers of the section are units.

use std::collections::HashMap;

use labcraft_format::coordinate_value;
use labcraft_graph::{Anchor, BlockSide, Coordinate, Id, Outline, OutlineKind, Signed};
use labcraft_types::{Code, Interner, cut_text};

use crate::places::{Places, Slot};

/// Every integer of a corner is between 0 and this *(proposed)*.
const MAX_INTEGER: u32 = 1024;
/// The corners of the body, and the points of a piece, at most *(proposed)*.
const MAX_CORNERS: usize = 64;
const MAX_PIECE_POINTS: usize = 16;
/// The radius of `round`.
const MAX_ROUND: u32 = 16;
/// The cut of `template d`, `node.chamfer` of the default style: what `check` judges it at (11.4).
pub const CHAMFER: i64 = 8 * UNIT;
/// A unit, in hundredths.
const UNIT: i64 = 100;
/// How far a piece reaches out or in, and along its place, in units.
const PIECE_OUT: i64 = 8;
const PIECE_ALONG: i64 = 4;
/// How far inside the box a terminal may start, and how much straight edge a piece needs on each side of its place.
const INSIDE: i64 = 4 * UNIT;
/// The segments of a rounded corner, for a quarter turn.
const ARC_STEPS: f64 = 4.0;

/// A point, in hundredths; a segment between two.
type Point = (i64, i64);
type Segment = (Point, Point);
/// A corner of the body and the radius of its rounding.
type Rounded = (Point, i64);

/// What an item breaks, for `check`: the code, the message, the item.
pub type Problem = (Code, String, Id);

/// What an `outline` section says once read: the ordinary block where it cannot be drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// `template block`, or a section that cannot be read as one.
    Block,
    /// `template d`, by its item.
    D(Id),
    /// A body and its pieces.
    Path { id: Id, corners: Vec<Corner>, pieces: Vec<Piece> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corner {
    pub x: Length,
    pub y: Length,
    /// The radius of its rounding, in hundredths; 0 for none.
    pub round: i64,
}

/// A coordinate of a corner, read: what it counts from and its shift, in hundredths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Length {
    pub base: Base,
    pub shift: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Base {
    Number(i64),
    Width,
    Height,
    Terminal(String),
}

/// A piece of a side: its points in hundredths, outward then along.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub id: Id,
    pub side: BlockSide,
    pub points: Vec<(i64, i64)>,
}

/// Reads a section: its errors (`LC5016`), and the warnings that do not depend on the size, a piece outside its bounds
/// (`LC5017`). Either leaves the ordinary block.
pub fn read(section: &Outline, names: &Interner) -> (Read, Vec<Problem>) {
    let mut problems = Vec::new();
    let mut path: Option<(Id, &[labcraft_graph::Corner])> = None;
    let mut template: Option<(Id, &str)> = None;
    let mut pieces: Vec<Piece> = Vec::new();
    let mut sides: Vec<BlockSide> = Vec::new();
    let integer = |digits| coordinate_value(names.resolve(digits)).filter(|value| *value <= MAX_INTEGER).map(i64::from);
    for item in &section.items {
        match &item.kind {
            OutlineKind::Template(name) => {
                if template.is_some() {
                    problems.push((Code::Lc5016, "a second `template`: an outline has one body (11.4)".to_owned(), item.id));
                    continue;
                }
                template = Some((item.id, names.resolve(*name)));
            }
            OutlineKind::Path(corners) => {
                if path.is_some() {
                    problems.push((Code::Lc5016, "a second `path`: an outline has one body (11.4)".to_owned(), item.id));
                    continue;
                }
                if corners.len() > MAX_CORNERS {
                    let message = format!("{} corners: the body has at most {MAX_CORNERS} (11.4)", corners.len());
                    problems.push((Code::Lc5016, message, item.id));
                }
                // The first integer out of its range, and the first `round`: one problem each for the item.
                let digits = corners.iter().flat_map(|corner| [corner.x, corner.y]).flat_map(|coordinate| {
                    let number = match coordinate.base {
                        Anchor::Number(digits) => Some(digits),
                        Anchor::Name(_) => None,
                    };
                    number.into_iter().chain(coordinate.shift.map(|(_, digits)| digits))
                });
                if let Some(digits) = digits.into_iter().find(|digits| integer(*digits).is_none()) {
                    let shown = cut_text(names.resolve(digits)).into_owned();
                    let message = format!("`{shown}`: an integer of an outline is between 0 and {MAX_INTEGER} (11.4)");
                    problems.push((Code::Lc5016, message, item.id));
                }
                let round_refused =
                    |round: &&labcraft_types::Name| !coordinate_value(names.resolve(**round)).is_some_and(|value| (1..=MAX_ROUND).contains(&value));
                if let Some(round) = corners.iter().filter_map(|corner| corner.round.as_ref()).find(round_refused) {
                    let shown = cut_text(names.resolve(*round)).into_owned();
                    let message = format!("`round {shown}`: a corner is rounded by 1 to {MAX_ROUND} (11.4)");
                    problems.push((Code::Lc5016, message, item.id));
                }
                path = Some((item.id, corners));
            }
            OutlineKind::Piece { side, points } => {
                if sides.contains(side) {
                    let message = format!("a second piece for the `{}` side: a side has one (11.4)", side.word());
                    problems.push((Code::Lc5016, message, item.id));
                    continue;
                }
                sides.push(*side);
                if points.len() > MAX_PIECE_POINTS {
                    let message = format!("{} points: a piece has at most {MAX_PIECE_POINTS} (11.4)", points.len());
                    problems.push((Code::Lc5016, message, item.id));
                    continue;
                }
                let value = |signed: Signed| {
                    let magnitude = coordinate_value(names.resolve(signed.digits)).map_or(i64::MAX, i64::from);
                    if signed.negative { -magnitude } else { magnitude }
                };
                let read: Vec<(i64, i64)> = points.iter().map(|point| (value(point.out), value(point.along))).collect();
                pieces.push(Piece {
                    id: item.id,
                    side: *side,
                    points: read.iter().map(|(out, along)| (out.saturating_mul(UNIT), along.saturating_mul(UNIT))).collect(),
                });
                if let Some(message) = piece_refused(&read) {
                    problems.push((Code::Lc5017, message, item.id));
                }
            }
        }
    }
    match (template, path) {
        (Some((id, _)), Some(_)) => problems.push((Code::Lc5016, "a `template` stands alone in its section (11.4)".to_owned(), id)),
        (Some((id, _)), None) if !pieces.is_empty() => {
            problems.push((Code::Lc5016, "a `template` stands alone in its section: pieces need a `path` (11.4)".to_owned(), id));
        }
        (Some((id, name)), None) if name != "d" && name != "block" => {
            let message = format!("`template {}`: the templates are `d` and `block` (11.4)", cut_text(name));
            problems.push((Code::Lc5016, message, id));
        }
        (None, None) => {
            if let Some(piece) = pieces.first() {
                problems.push((Code::Lc5016, "a piece needs a `path`, the body it sits on (11.4)".to_owned(), piece.id));
            }
        }
        _ => {}
    }
    if !problems.is_empty() {
        return (Read::Block, problems);
    }
    let read = match (template, path) {
        (Some((id, "d")), _) => Read::D(id),
        (Some(_), _) | (None, None) => Read::Block,
        (None, Some((id, corners))) => {
            let coordinate = |coordinate: Coordinate| {
                let base = match coordinate.base {
                    Anchor::Number(digits) => Base::Number(integer(digits).unwrap_or(0) * UNIT),
                    Anchor::Name(name) => match names.resolve(name) {
                        "w" => Base::Width,
                        "h" => Base::Height,
                        other => Base::Terminal(other.to_owned()),
                    },
                };
                let shift = coordinate.shift.map_or(0, |(minus, digits)| integer(digits).unwrap_or(0) * UNIT * if minus { -1 } else { 1 });
                Length { base, shift }
            };
            let corners = corners
                .iter()
                .map(|corner| Corner {
                    x: coordinate(corner.x),
                    y: coordinate(corner.y),
                    round: corner.round.and_then(|round| coordinate_value(names.resolve(round))).map_or(0, |round| i64::from(round) * UNIT),
                })
                .collect();
            Read::Path { id, corners, pieces }
        }
    };
    (read, problems)
}

/// Why a piece is outside its bounds, in units: its first and last points on the edge, outward -8 to 8, along -4 to 4,
/// never back along.
fn piece_refused(points: &[(i64, i64)]) -> Option<String> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return None };
    if first.0 != 0 || last.0 != 0 {
        return Some("the first and the last point of a piece are on the edge, `(0, …)` (11.4)".to_owned());
    }
    if points.iter().any(|(out, along)| out.abs() > PIECE_OUT || along.abs() > PIECE_ALONG) {
        return Some(format!("a point of a piece is from -{PIECE_OUT} to {PIECE_OUT} outward and -{PIECE_ALONG} to {PIECE_ALONG} along (11.4)"));
    }
    if points.windows(2).any(|pair| matches!(pair, [a, b] if b.1 < a.1)) {
        return Some("the second coordinate of a piece never goes back, so that it stays in the row of its place (11.4)".to_owned());
    }
    // Its points as a path, open: no two segments meet but two in a row at their shared end, and none folds back.
    let path: Vec<Segment> = points
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] => Some((*a, *b)),
            _ => None,
        })
        .collect();
    for (index, first) in path.iter().enumerate() {
        if let Some(next) = path.get(index + 1)
            && orientation(first.0, first.1, next.1) == 0
            && dot(first.0, first.1, next.1) > 0
        {
            return Some("a piece crosses itself (11.4)".to_owned());
        }
        if path.iter().skip(index + 2).any(|second| crosses(*first, *second)) {
            return Some("a piece crosses itself (11.4)".to_owned());
        }
    }
    None
}

/// A terminal of the block at the size it takes: its side, its place (a height on the left and the right, a distance
/// from the left on the top), its name, and whether it anchors a coordinate (a variadic parameter does not).
#[derive(Clone, Copy, Debug)]
pub struct At<'t> {
    pub side: BlockSide,
    pub along: i64,
    pub name: &'t str,
    pub output: bool,
    pub anchor: bool,
}

/// The block at one size: its box and its terminals.
#[derive(Clone, Debug)]
pub struct Frame<'t> {
    pub width: i64,
    pub height: i64,
    pub terminals: Vec<At<'t>>,
    /// The cut of the corners of `template d`: `node.chamfer` of the style.
    pub chamfer: i64,
}

impl<'t> Frame<'t> {
    /// The block whose terminals are on `places` (11.2).
    pub fn of<T: Slot>(places: &Places, inputs: &'t [T], outputs: &'t [T], chamfer: i64) -> Frame<'t> {
        let mut terminals = Vec::with_capacity(inputs.len() + outputs.len());
        for (output, list, at) in [(false, inputs, &places.inputs), (true, outputs, &places.outputs)] {
            for (terminal, place) in list.iter().zip(at) {
                let (x, y) = places.at(*place);
                let along = if place.0 == BlockSide::Top { x } else { y };
                terminals.push(At { side: place.0, along, name: terminal.name(), output, anchor: terminal.anchor() });
            }
        }
        Frame { width: places.width(), height: places.height(), terminals, chamfer }
    }
}

/// An outline drawn at one size: its points, closed, where each terminal of the frame starts, and how far it reaches
/// outside the box on the left, the right and the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawn {
    pub points: Vec<(i64, i64)>,
    pub starts: Vec<(i64, i64)>,
    pub left: i64,
    pub right: i64,
    pub top: i64,
}

/// The outline at the size of `frame`: `None` for the ordinary block, the problems of the guards that it breaks there.
pub fn draw(read: &Read, frame: &Frame<'_>) -> Result<Option<Drawn>, Vec<Problem>> {
    let (w, h) = (frame.width, frame.height);
    let (id, body, pieces): (Id, Vec<Rounded>, &[Piece]) = match read {
        Read::Block => return Ok(None),
        Read::D(id) => {
            let cut = frame.chamfer.min(h / 2).min(w / 2);
            let corners = [(0, 0), (w - cut, 0), (w, cut), (w, h - cut), (w - cut, h), (0, h)];
            (*id, corners.into_iter().map(|corner| (corner, 0)).collect(), &[])
        }
        Read::Path { id, corners, pieces } => {
            let mut problems = Vec::new();
            let mut body = Vec::with_capacity(corners.len());
            for corner in corners {
                let x = resolve(&corner.x, frame, BlockSide::Top);
                let y = resolve(&corner.y, frame, BlockSide::Left);
                match (x, y) {
                    (Ok(x), Ok(y)) => {
                        if !(0..=w).contains(&x) || !(0..=h).contains(&y) {
                            let message =
                                format!("the corner ({}, {}) is outside the box {} × {} of the block (11.4)", units(x), units(y), units(w), units(h));
                            problems.push((Code::Lc5017, message, *id));
                        }
                        body.push(((x, y), corner.round));
                    }
                    (Err(message), _) | (_, Err(message)) => problems.push((Code::Lc5017, message, *id)),
                }
            }
            if !problems.is_empty() {
                problems.truncate(1);
                return Err(problems);
            }
            (*id, body, &pieces[..])
        }
    };
    // The body, its corners rounded: each edge keeps the straight part that its roundings leave.
    let (points, edges, arcs) = rounded(&body);
    if points.len() < 3 || area(&points) == 0 || !simple(&points) {
        return Err(vec![(Code::Lc5017, "the body of the outline crosses itself (11.4)".to_owned(), id)]);
    }
    // The pieces, at each terminal of their side, on the straight edge that its place crosses outermost.
    let mut spliced: Vec<Vec<Splice>> = vec![Vec::new(); edges.len()];
    for piece in pieces {
        // The edges that face the side of the piece, found once for all its terminals.
        let facing: Vec<bool> = edges.iter().map(|edge| faces(edge, &points, piece.side)).collect();
        for (index, terminal) in frame.terminals.iter().enumerate().filter(|(_, terminal)| terminal.side == piece.side) {
            let Some(edge) = host(&edges, &facing, piece.side, terminal.along) else {
                let message = format!(
                    "no straight edge of the `{}` side runs 4 on each side of `{}`, where its piece sits (11.4)",
                    piece.side.word(),
                    cut_text(terminal.name)
                );
                return Err(vec![(Code::Lc5017, message, piece.id)]);
            };
            let Some(host_edge) = edges.get(edge) else { continue };
            let level = host_edge.level();
            let at: Vec<(i64, i64)> = piece
                .points
                .iter()
                .map(|&(out, along)| match piece.side {
                    BlockSide::Left => (level - out, terminal.along + along),
                    BlockSide::Right => (level + out, terminal.along + along),
                    BlockSide::Top => (terminal.along + along, level - out),
                })
                .collect();
            if let Some(list) = spliced.get_mut(edge) {
                list.push(Splice { terminal: index, piece: piece.id, side: piece.side, along: terminal.along, points: at });
            }
        }
    }
    // The outline: each edge from its start, its pieces in its direction, to its end.
    let mut outline: Vec<(i64, i64)> = Vec::with_capacity(points.len());
    let mut piece_of_terminal: Vec<Option<(usize, usize)>> = vec![None; frame.terminals.len()];
    let mut order: Vec<(Id, usize, usize, Edge, BlockSide, i64)> = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        // The corner at its start, rounded or not: it ends where the edge starts.
        for point in arcs.get(index).into_iter().flatten() {
            push(&mut outline, *point);
        }
        push(&mut outline, edge.from);
        let Some(list) = spliced.get_mut(index) else { continue };
        let forward = edge.forward();
        list.sort_by_key(|splice| if forward { splice.along } else { -splice.along });
        for pair in list.windows(2) {
            if let [a, b] = pair
                && overlaps(a, b, forward)
            {
                let message = "two pieces of one side overlap: their places are too close for them (11.4)".to_owned();
                return Err(vec![(Code::Lc5017, message, a.piece)]);
            }
        }
        for splice in list.iter() {
            let mut placed = splice.points.clone();
            if !forward {
                placed.reverse();
            }
            // From its first point, which may be the last point of the piece before it.
            let start = if outline.last() == placed.first() { outline.len().saturating_sub(1) } else { outline.len() };
            for point in placed {
                push(&mut outline, point);
            }
            if let Some(slot) = piece_of_terminal.get_mut(splice.terminal) {
                *slot = Some((start, outline.len()));
            }
            order.push((splice.piece, start, outline.len(), *edge, splice.side, splice.along));
        }
        push(&mut outline, edge.to);
    }
    while outline.len() > 1 && outline.first() == outline.last() {
        outline.pop();
    }
    // Each piece against the body and the pieces of the neighbouring places only, so that the cost grows with the
    // terminals and never with their square.
    let body_segments: Vec<Segment> = segments(&points).collect();
    let body_box = bounds(&body_segments);
    // The segments of the body by rows and by columns 8 wide, so that a piece or a terminal looks only at those near its
    // place: the cost grows with the terminals, never with the terminals times the body.
    let rows_of_body = Buckets::new(&body_segments, |(a, b)| (a.1.min(b.1), a.1.max(b.1)), h);
    let columns_of_body = Buckets::new(&body_segments, |(a, b)| (a.0.min(b.0), a.0.max(b.0)), w);
    // The pieces of the left and the right sides by row, for the piece that faces each.
    let rows: HashMap<(BlockSide, i64), usize> =
        order.iter().enumerate().filter(|(_, entry)| entry.4 != BlockSide::Top).map(|(at, entry)| ((entry.4, entry.5), at)).collect();
    let (mut own, mut others): (Vec<Segment>, Vec<Segment>) = (Vec::new(), Vec::new());
    for (at, &(piece, start, end, host, side, along)) in order.iter().enumerate() {
        // Its own segments, from its first point to its last: what joins it to the edge lies on the edge.
        own.clear();
        own.extend((start..end.saturating_sub(1)).filter_map(|index| segment(&outline, index)));
        let (low, high) = bounds(&own);
        let near = |(a, b): &Segment| a.0.max(b.0) >= low.0 && a.0.min(b.0) <= high.0 && a.1.max(b.1) >= low.1 && a.1.min(b.1) <= high.1;
        others.clear();
        // The body near it, when the piece reaches into its box at all.
        if near(&body_box) {
            let buckets = if side == BlockSide::Top { &columns_of_body } else { &rows_of_body };
            let (from, to) = if side == BlockSide::Top { (low.0, high.0) } else { (low.1, high.1) };
            others.extend(
                buckets
                    .near(from, to)
                    .filter_map(|index| body_segments.get(index).copied())
                    .filter(|segment| near(segment) && !on_line(segment.0, segment.1, host)),
            );
        }
        let facing = side.facing().and_then(|facing| rows.get(&(facing, along)).copied());
        let neighbours =
            [at.checked_sub(1), Some(at + 1).filter(|next| *next < order.len()), (at + 1 == order.len() && order.len() > 1).then_some(0), facing];
        for neighbour in neighbours.into_iter().flatten().filter(|neighbour| *neighbour != at) {
            if let Some(&(_, from, to, ..)) = order.get(neighbour) {
                others.extend((from..to.saturating_sub(1)).filter_map(|index| segment(&outline, index)).filter(|segment| near(segment)));
            }
        }
        if own.iter().any(|mine| others.iter().any(|other| crosses(*mine, *other))) {
            return Err(vec![(Code::Lc5017, "a piece crosses the body or another piece (11.4)".to_owned(), piece)]);
        }
    }
    // Where each terminal starts: on its place, at the outermost point of the outline on its side.
    let mut starts = Vec::with_capacity(frame.terminals.len());
    for (index, terminal) in frame.terminals.iter().enumerate() {
        let found = match piece_of_terminal.get(index).copied().flatten() {
            Some((start, end)) => {
                outermost((start.saturating_sub(1)..end).filter_map(|index| segment(&outline, index)), terminal.side, terminal.along)
            }
            // On a side without a piece, the body near its place: a piece sits on its own side.
            None => {
                let buckets = if terminal.side == BlockSide::Top { &columns_of_body } else { &rows_of_body };
                outermost(
                    buckets.near(terminal.along, terminal.along).filter_map(|index| body_segments.get(index).copied()),
                    terminal.side,
                    terminal.along,
                )
            }
        };
        let inside = |found: (i64, i64)| match terminal.side {
            BlockSide::Left => found.0 <= INSIDE,
            BlockSide::Right => found.0 >= w - INSIDE,
            BlockSide::Top => found.1 <= INSIDE,
        };
        match found.filter(|found| inside(*found)) {
            Some(found) => starts.push(found),
            None => {
                let message = format!("the outline does not meet `{}` within 4 of the box of the block (11.4)", cut_text(terminal.name));
                let item = piece_of_terminal
                    .get(index)
                    .copied()
                    .flatten()
                    .and_then(|_| pieces.iter().find(|piece| piece.side == terminal.side))
                    .map_or(id, |piece| piece.id);
                return Err(vec![(Code::Lc5017, message, item)]);
            }
        }
    }
    let (low, high) = outline
        .iter()
        .fold(((0, 0), (w, h)), |(low, high), point| ((low.0.min(point.0), low.1.min(point.1)), (high.0.max(point.0), high.1.max(point.1))));
    Ok(Some(Drawn { left: -low.0, right: high.0 - w, top: -low.1, points: outline, starts }))
}

/// Where the outline crosses the place `along` of `side`, outermost: for the `after:` terminal, which starts there
/// wherever that is, and is never judged.
pub fn crossing(drawn: &Drawn, side: BlockSide, along: i64) -> Option<(i64, i64)> {
    outermost(segments(&drawn.points), side, along)
}

/// Segments by bands 8 wide along one axis, each band with the segments that reach into it.
struct Buckets {
    bands: Vec<Vec<usize>>,
}

impl Buckets {
    const BAND: i64 = 8 * UNIT;

    /// `span` gives the extent of a segment along the axis, which runs from 0 to `length`; a segment past it goes to the
    /// first or the last band.
    fn new(segments: &[Segment], span: impl Fn(&Segment) -> (i64, i64), length: i64) -> Buckets {
        let count = usize::try_from(length.max(0) / Self::BAND).unwrap_or(0).saturating_add(1);
        let mut bands = vec![Vec::new(); count];
        for (index, segment) in segments.iter().enumerate() {
            let (from, to) = span(segment);
            for band in Self::band(from, count)..=Self::band(to, count) {
                if let Some(list) = bands.get_mut(band) {
                    list.push(index);
                }
            }
        }
        Buckets { bands }
    }

    fn band(at: i64, count: usize) -> usize {
        usize::try_from(at.max(0) / Self::BAND).unwrap_or(0).min(count.saturating_sub(1))
    }

    /// The segments that reach into the bands from `from` to `to`, a segment once for each band.
    fn near(&self, from: i64, to: i64) -> impl Iterator<Item = usize> + '_ {
        let count = self.bands.len();
        (Self::band(from, count)..=Self::band(to, count)).filter_map(|band| self.bands.get(band)).flatten().copied()
    }
}

/// A piece at a place, with the terminal that it is at.
#[derive(Clone, Debug)]
struct Splice {
    terminal: usize,
    piece: Id,
    side: BlockSide,
    along: i64,
    points: Vec<(i64, i64)>,
}

/// Two pieces in a row overlap when the first ends past where the second starts, in the direction of their edge.
fn overlaps(a: &Splice, b: &Splice, forward: bool) -> bool {
    let reach = |splice: &Splice, last: bool| {
        let along = splice.points.iter().map(|point| if splice.side == BlockSide::Top { point.0 } else { point.1 });
        if last == forward { along.max() } else { along.min() }
    };
    match (reach(a, true), reach(b, false)) {
        (Some(end), Some(start)) => {
            if forward {
                end > start
            } else {
                end < start
            }
        }
        _ => false,
    }
}

fn push(outline: &mut Vec<(i64, i64)>, point: (i64, i64)) {
    if outline.last() != Some(&point) {
        outline.push(point);
    }
}

/// A coordinate at the size of `frame`: an anchor of the left or the right side gives a height, of the top a distance
/// from the left, so a terminal anchors only the coordinate of its side (`axis`: `Top` for the first, `Left` for the
/// second).
fn resolve(coordinate: &Length, frame: &Frame<'_>, axis: BlockSide) -> Result<i64, String> {
    let base = match &coordinate.base {
        Base::Number(value) => *value,
        Base::Width => frame.width,
        Base::Height => frame.height,
        Base::Terminal(name) => {
            // The input when the name is both an input and an output.
            let found = frame
                .terminals
                .iter()
                .filter(|terminal| terminal.name == name && terminal.anchor && (terminal.side == BlockSide::Top) == (axis == BlockSide::Top))
                .min_by_key(|terminal| terminal.output);
            match found {
                Some(terminal) => terminal.along,
                None => {
                    let which = if axis == BlockSide::Top {
                        "the top side, for a first coordinate"
                    } else {
                        "the left or the right side, for a second coordinate"
                    };
                    return Err(format!("`{}` names no terminal of {which} (11.4)", cut_text(name)));
                }
            }
        }
    };
    Ok(base.saturating_add(coordinate.shift))
}

fn units(value: i64) -> String {
    if value % UNIT == 0 { (value / UNIT).to_string() } else { format!("{:.2}", value as f64 / UNIT as f64) }
}

/// A straight part of an edge of the body, from its start to its end in the order of the outline.
#[derive(Clone, Copy, Debug)]
struct Edge {
    from: (i64, i64),
    to: (i64, i64),
}

impl Edge {
    fn vertical(&self) -> bool {
        self.from.0 == self.to.0 && self.from.1 != self.to.1
    }

    fn horizontal(&self) -> bool {
        self.from.1 == self.to.1 && self.from.0 != self.to.0
    }

    /// Its x for a vertical edge, its y for a horizontal one.
    fn level(&self) -> i64 {
        if self.vertical() { self.from.0 } else { self.from.1 }
    }

    /// Whether it runs down (vertical) or right (horizontal): the direction of the second coordinate of a piece.
    fn forward(&self) -> bool {
        if self.vertical() { self.to.1 > self.from.1 } else { self.to.0 > self.from.0 }
    }

    fn span(&self) -> (i64, i64) {
        if self.vertical() {
            (self.from.1.min(self.to.1), self.from.1.max(self.to.1))
        } else {
            (self.from.0.min(self.to.0), self.from.0.max(self.to.0))
        }
    }
}

/// The straight edge of `side` that the place `along` crosses outermost, if it runs 4 on each side of it: an edge that
/// faces that way, the body outside it, never one that faces the other side or inward.
fn host(edges: &[Edge], facing: &[bool], side: BlockSide, along: i64) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (index, edge) in edges.iter().enumerate() {
        let fits = if side == BlockSide::Top { edge.horizontal() } else { edge.vertical() };
        let (low, high) = edge.span();
        if !fits || along < low || along > high || !facing.get(index).copied().unwrap_or(false) {
            continue;
        }
        let level = edge.level();
        let better = match (best, side) {
            (None, _) => true,
            (Some((_, other)), BlockSide::Left | BlockSide::Top) => level < other,
            (Some((_, other)), BlockSide::Right) => level > other,
        };
        if better {
            best = Some((index, level));
        }
    }
    let (index, _) = best?;
    let (low, high) = edges.get(index)?.span();
    (along - INSIDE >= low && along + INSIDE <= high).then_some(index)
}

/// Whether the body lies on the inner side of an edge for `side`: a point just outside its middle, towards `side`, is
/// outside the body.
fn faces(edge: &Edge, body: &[Point], side: BlockSide) -> bool {
    let fits = if side == BlockSide::Top { edge.horizontal() } else { edge.vertical() };
    if !fits {
        return false;
    }
    let (low, high) = edge.span();
    // Twice the coordinates, so that the middle of the edge and a half step beside it stay integers.
    let middle = low + high;
    let level = 2 * edge.level();
    let beside = match side {
        BlockSide::Left => (level - 1, middle),
        BlockSide::Right => (level + 1, middle),
        BlockSide::Top => (middle, level - 1),
    };
    !inside_doubled(body, beside)
}

/// Whether a point, its coordinates doubled, is inside a polygon (even and odd rule; the point is never on an edge,
/// since its coordinates are odd where they matter).
fn inside_doubled(polygon: &[Point], (x, y): Point) -> bool {
    let mut inside = false;
    for (a, b) in segments(polygon) {
        let (ax, ay, bx, by) = (2 * a.0, 2 * a.1, 2 * b.0, 2 * b.1);
        if (ay > y) != (by > y) {
            let cross = i128::from(bx - ax) * i128::from(y - ay) - i128::from(x - ax) * i128::from(by - ay);
            // The edge crosses the line y right of the point when the cross product has the sign of its direction.
            if (cross > 0) == (by > ay) {
                inside = !inside;
            }
        }
    }
    inside
}

/// The corners of the body with their roundings flattened, for each edge, in order, its straight part, and the points of
/// the corner it starts from.
fn rounded(body: &[Rounded]) -> (Vec<Point>, Vec<Edge>, Vec<Vec<Point>>) {
    let count = body.len();
    let at = |index: usize| body.get(index % count.max(1)).copied().unwrap_or(((0, 0), 0));
    // For each corner, the points that stand for it: itself, or its arc from the edge before to the edge after.
    let mut arcs: Vec<Vec<(i64, i64)>> = Vec::with_capacity(count);
    for index in 0..count {
        let ((x, y), radius) = at(index);
        let (before, _) = at(index + count - 1);
        let (after, _) = at(index + 1);
        arcs.push(if radius > 0 { fillet((x, y), before, after, radius) } else { vec![(x, y)] });
    }
    let mut points = Vec::new();
    let mut edges = Vec::with_capacity(count);
    for index in 0..count {
        let arc = arcs.get(index).cloned().unwrap_or_default();
        let next = arcs.get((index + 1) % count.max(1)).and_then(|arc| arc.first().copied());
        for point in &arc {
            if points.last() != Some(point) {
                points.push(*point);
            }
        }
        if let (Some(&from), Some(to)) = (arc.last(), next) {
            edges.push(Edge { from, to });
        }
    }
    while points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    (points, edges, arcs)
}

/// The arc that rounds the corner `at` between the edges from `before` and to `after`, its radius reduced to half of
/// the shorter of the two edges, as the radius of a `rect` (11.1).
fn fillet(at: (i64, i64), before: (i64, i64), after: (i64, i64), radius: i64) -> Vec<(i64, i64)> {
    let (px, py) = (at.0 as f64, at.1 as f64);
    let (ux, uy) = (before.0 as f64 - px, before.1 as f64 - py);
    let (vx, vy) = (after.0 as f64 - px, after.1 as f64 - py);
    let (lu, lv) = (ux.hypot(uy), vx.hypot(vy));
    if lu == 0.0 || lv == 0.0 {
        return vec![at];
    }
    let (ux, uy, vx, vy) = (ux / lu, uy / lu, vx / lv, vy / lv);
    let angle = (ux * vx + uy * vy).clamp(-1.0, 1.0).acos();
    if angle < 1e-6 || (std::f64::consts::PI - angle) < 1e-6 {
        return vec![at];
    }
    let half = angle / 2.0;
    // The tangent points, at most half of the shorter edge from the corner.
    let mut distance = radius as f64 / half.tan();
    distance = distance.min(lu.min(lv) / 2.0);
    let radius = distance * half.tan();
    let (bx, by) = (ux + vx, uy + vy);
    let lb = bx.hypot(by);
    if lb == 0.0 {
        return vec![at];
    }
    let centre = (px + bx / lb * radius / half.sin(), py + by / lb * radius / half.sin());
    let first = (px + ux * distance, py + uy * distance);
    let last = (px + vx * distance, py + vy * distance);
    let start = (first.1 - centre.1).atan2(first.0 - centre.0);
    let end = (last.1 - centre.1).atan2(last.0 - centre.0);
    let mut sweep = end - start;
    while sweep > std::f64::consts::PI {
        sweep -= 2.0 * std::f64::consts::PI;
    }
    while sweep < -std::f64::consts::PI {
        sweep += 2.0 * std::f64::consts::PI;
    }
    let steps = ((sweep.abs() / std::f64::consts::FRAC_PI_2) * ARC_STEPS).round().max(1.0) as usize;
    (0..=steps)
        .map(|step| {
            let turn = start + sweep * step as f64 / steps as f64;
            ((centre.0 + radius * turn.cos()).round() as i64, (centre.1 + radius * turn.sin()).round() as i64)
        })
        .collect()
}

fn segment(points: &[(i64, i64)], index: usize) -> Option<Segment> {
    let count = points.len();
    if count < 2 {
        return None;
    }
    Some((*points.get(index % count)?, *points.get((index + 1) % count)?))
}

fn segments(points: &[(i64, i64)]) -> impl Iterator<Item = Segment> + '_ {
    (0..points.len()).filter_map(|index| segment(points, index))
}

fn bounds(segments: &[Segment]) -> Segment {
    segments.iter().fold(((i64::MAX, i64::MAX), (i64::MIN, i64::MIN)), |(low, high), (a, b)| {
        ((low.0.min(a.0).min(b.0), low.1.min(a.1).min(b.1)), (high.0.max(a.0).max(b.0), high.1.max(a.1).max(b.1)))
    })
}

/// Whether a segment of the body lies on the line of the straight edge that a piece replaces.
fn on_line(a: (i64, i64), b: (i64, i64), host: Edge) -> bool {
    orientation(host.from, host.to, a) == 0 && orientation(host.from, host.to, b) == 0
}

/// Twice the signed area of a polygon.
fn area(points: &[(i64, i64)]) -> i128 {
    segments(points).map(|(a, b)| i128::from(a.0) * i128::from(b.1) - i128::from(b.0) * i128::from(a.1)).sum()
}

/// Whether a closed polygon never meets itself but at the shared end of two segments in a row.
fn simple(points: &[(i64, i64)]) -> bool {
    let all: Vec<Segment> = segments(points).collect();
    let count = all.len();
    // Two segments in a row meet at their shared end; they must not fold back on each other.
    for (index, before) in all.iter().enumerate() {
        let Some(after) = all.get((index + 1) % count.max(1)) else { continue };
        if orientation(before.0, before.1, after.1) == 0 && dot(before.0, before.1, after.1) > 0 {
            return false;
        }
    }
    // Any other two must not meet: only those whose spans across x overlap can, so they are taken in order of their
    // left end, each against those that start before it ends.
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|index| all.get(*index).map_or(0, |(a, b)| a.0.min(b.0)));
    for (at, &index) in order.iter().enumerate() {
        let Some(first) = all.get(index) else { continue };
        let right = first.0.0.max(first.1.0);
        for &other in order.iter().skip(at + 1) {
            let Some(second) = all.get(other) else { continue };
            if second.0.0.min(second.1.0) > right {
                break;
            }
            let in_row = other.abs_diff(index) == 1 || other.abs_diff(index) + 1 == count;
            if !in_row && crosses(*first, *second) {
                return false;
            }
        }
    }
    true
}

fn orientation(a: (i64, i64), b: (i64, i64), c: (i64, i64)) -> i32 {
    let value = i128::from(b.0 - a.0) * i128::from(c.1 - a.1) - i128::from(b.1 - a.1) * i128::from(c.0 - a.0);
    value.signum() as i32
}

/// Whether `c` lies back towards `a` from `b`, along the line.
fn dot(a: (i64, i64), b: (i64, i64), c: (i64, i64)) -> i128 {
    i128::from(a.0 - b.0) * i128::from(c.0 - b.0) + i128::from(a.1 - b.1) * i128::from(c.1 - b.1)
}

fn on_segment(a: (i64, i64), b: (i64, i64), c: (i64, i64)) -> bool {
    c.0 >= a.0.min(b.0) && c.0 <= a.0.max(b.0) && c.1 >= a.1.min(b.1) && c.1 <= a.1.max(b.1)
}

/// Whether two segments meet anywhere but at an end that they share.
fn crosses(first: Segment, second: Segment) -> bool {
    let ((a, b), (c, d)) = (first, second);
    let shared = [a, b].iter().any(|point| *point == c || *point == d);
    let (o1, o2, o3, o4) = (orientation(a, b, c), orientation(a, b, d), orientation(c, d, a), orientation(c, d, b));
    if o1 != o2 && o3 != o4 && o1 != 0 && o2 != 0 && o3 != 0 && o4 != 0 {
        return true;
    }
    if shared {
        // Meeting at a shared end is no crossing, unless they also run along each other.
        return o1 == 0 && o2 == 0 && overlap_beyond_point(first, second);
    }
    (o1 == 0 && on_segment(a, b, c)) || (o2 == 0 && on_segment(a, b, d)) || (o3 == 0 && on_segment(c, d, a)) || (o4 == 0 && on_segment(c, d, b))
}

/// Two collinear segments that share an end overlap by more than that end.
fn overlap_beyond_point(first: Segment, second: Segment) -> bool {
    let ((a, b), (c, d)) = (first, second);
    let key = |point: (i64, i64)| if a.0 != b.0 { point.0 } else { point.1 };
    let (low1, high1) = (key(a).min(key(b)), key(a).max(key(b)));
    let (low2, high2) = (key(c).min(key(d)), key(c).max(key(d)));
    low1.max(low2) < high1.min(high2)
}

/// The outermost point where segments meet the place `along` of `side`: the least x on a line across the block at that
/// height for the left side, the greatest for the right, the least y on a line down at that distance for the top.
fn outermost(segments: impl Iterator<Item = Segment>, side: BlockSide, along: i64) -> Option<(i64, i64)> {
    let mut best: Option<(i64, i64)> = None;
    for (a, b) in segments {
        // Across: along y for the left and the right sides, along x for the top.
        let (a_along, a_across, b_along, b_across) = match side {
            BlockSide::Top => (a.0, a.1, b.0, b.1),
            BlockSide::Left | BlockSide::Right => (a.1, a.0, b.1, b.0),
        };
        if along < a_along.min(b_along) || along > a_along.max(b_along) {
            continue;
        }
        let candidates = if a_along == b_along {
            [a_across, b_across]
        } else {
            let t = (along - a_along) as f64 / (b_along - a_along) as f64;
            let across = (a_across as f64 + t * (b_across - a_across) as f64).round() as i64;
            [across, across]
        };
        for across in candidates {
            let better = match (best, side) {
                (None, _) => true,
                (Some(found), BlockSide::Left) => across < found.0,
                (Some(found), BlockSide::Right) => across > found.0,
                (Some(found), BlockSide::Top) => across < found.1,
            };
            if better {
                best = Some(if side == BlockSide::Top { (along, across) } else { (across, along) });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests;
