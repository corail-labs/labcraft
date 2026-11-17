//! A wire routed by hand (notation 2.4, 2.6): Alt and a drag on a segment moves it across, on the grid of 8, the
//! segments on each side following, and the drop writes `wire … via` (`wire.route`). A segment that leaves a terminal
//! gets a jog 16 from it, so that the wire still leaves the terminal in its direction *(proposed)*. A segment that
//! several wires of the value share moves for all: the one dragged gets the route, and each other one that shared it
//! becomes its branch, its junction where it left the segment (decision of the owner, 2026-11-17). Nothing is written
//! during the drag, and the rest of the drawing stays where it is (decision of the owner, 2026-11-17).

use egui::Pos2;
use labcraft_diagram::Drawing;

/// How far from a terminal a segment that leaves it jogs, in hundredths *(proposed)*.
const JOG: i64 = 1_600;
/// The grid that a segment moved and a junction stand on, in hundredths (notation 2.5).
pub(crate) const GRID: i64 = 800;

/// How the move goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Dragging,
    /// Asked of the host: shown until the new drawing comes, or for a while.
    Asked,
}

/// A segment being moved.
pub(crate) struct Rerouting {
    /// The wire, by its group in [`Drawing::groups`].
    pub group: usize,
    /// The reader whose segment moves, by its rank in the readers of the wire.
    pub reader: usize,
    pub segment: usize,
    /// Whether the segment is vertical: it moves across, to the left or the right.
    pub vertical: bool,
    /// Where the press was, in units.
    pub from: Pos2,
    /// How far the segment goes across, in hundredths, on the grid of the frame.
    pub by: i64,
    pub state: State,
    pub asked_at: Option<f64>,
}

/// A point of a path in hundredths.
type At = (i64, i64);

impl Rerouting {
    /// The segment of the wire `group` nearest `at`, within `reach` units: of the reader whose segment there is the longest.
    /// None for a wire that no `wire` entry can name, or a segment that a terminal reaches from above.
    pub fn grab(drawing: &Drawing, group: usize, at: Pos2, reach: f32) -> Option<Rerouting> {
        let wire = drawing.groups.get(group)?.wire.as_ref()?;
        let mut best: Option<(usize, usize, i64)> = None;
        for (reader, (_, path)) in wire.readers.iter().enumerate() {
            if wire.entries.get(reader).is_none_or(Option::is_none) {
                continue;
            }
            for (segment, pair) in path.windows(2).enumerate() {
                let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
                if !crate::select::near_segment(at, a, b, reach) {
                    continue;
                }
                let length = (b.0 - a.0).abs() + (b.1 - a.1).abs();
                if best.is_none_or(|(_, _, longest)| length > longest) {
                    best = Some((reader, segment, length));
                }
            }
        }
        let (reader, segment, _) = best?;
        let path = &wire.readers.get(reader)?.1;
        let (a, b) = (*path.get(segment)?, *path.get(segment + 1)?);
        let vertical = a.0 == b.0;
        // The last vertical into a terminal on the top edge has nowhere to go, nor the horizontal at the height it is
        // reached from, which the drawing keeps.
        let into_top = matches!((path.len().checked_sub(2).and_then(|at| path.get(at)), path.last()), (Some(a), Some(b)) if a.0 == b.0);
        if into_top && segment + 3 >= path.len() {
            return None;
        }
        Some(Rerouting { group, reader, segment, vertical, from: at, by: 0, state: State::Dragging, asked_at: None })
    }

    /// The same move over a new drawing, while it is dragged and its wire is drawn there along the same path; none
    /// otherwise.
    pub fn again(&self, old: &Drawing, new: &Drawing) -> Option<Rerouting> {
        if self.state != State::Dragging {
            return None;
        }
        let wire = old.groups.get(self.group)?.wire.as_ref()?;
        let (_, path) = wire.readers.get(self.reader)?;
        new.groups.iter().enumerate().find_map(|(group, drawn)| {
            let other = drawn.wire.as_ref()?;
            if other.value != wire.value || other.body != wire.body {
                return None;
            }
            let reader = other.readers.iter().position(|(_, other)| other == path)?;
            Some(Rerouting {
                group,
                reader,
                segment: self.segment,
                vertical: self.vertical,
                from: self.from,
                by: self.by,
                state: self.state,
                asked_at: None,
            })
        })
    }

    /// The pointer at `at`: the segment goes across by as much, on the grid of 8 of the frame of the wire.
    pub fn set_at(&mut self, drawing: &Drawing, at: Pos2) {
        let Some((a, _)) = self.ends(drawing) else { return };
        let origin = origin_of(drawing, self.group);
        let (line, start, delta) = if self.vertical { (a.0, origin.0, at.x - self.from.x) } else { (a.1, origin.1, at.y - self.from.y) };
        let wanted = line + (delta * 100.0).round() as i64;
        self.by = snap(wanted, start) - line;
    }

    fn ends(&self, drawing: &Drawing) -> Option<(At, At)> {
        let path = &drawing.groups.get(self.group)?.wire.as_ref()?.readers.get(self.reader)?.1;
        Some((*path.get(self.segment)?, *path.get(self.segment + 1)?))
    }

    /// The routes that the drop writes: the reader dragged and the readers that share the segment, each by its name as
    /// `wire.route` takes it, with its new path in hundredths and its points in units of its frame.
    pub fn routes(&self, drawing: &Drawing) -> Vec<(String, Vec<At>, Vec<At>)> {
        let Some(wire) = drawing.groups.get(self.group).and_then(|group| group.wire.as_ref()) else { return Vec::new() };
        let Some((_, path)) = wire.readers.get(self.reader) else { return Vec::new() };
        if self.by == 0 {
            return Vec::new();
        }
        let (Some(&a), Some(&b)) = (path.get(self.segment), path.get(self.segment + 1)) else { return Vec::new() };
        let Some(moved) = moved(path, self.segment, self.by, self.vertical) else { return Vec::new() };
        let origin = origin_of(drawing, self.group);
        let units = |points: &[At]| -> Vec<At> { points.iter().map(|&(x, y)| (unit(x - origin.0), unit(y - origin.1))).collect() };
        // The readers that leave it before the segment are the trunk it keeps: it leaves the last of them there, its
        // junction; those that run along the segment follow it.
        let mut junction: Option<(usize, i64, At)> = None;
        let mut followers = Vec::new();
        for (rank, (_, other)) in wire.readers.iter().enumerate() {
            if rank == self.reader {
                continue;
            }
            if shares(other, a, b) {
                followers.push(rank);
                continue;
            }
            let Some((_, point)) = split(path, other) else { continue };
            if let Some((segment, distance)) = along(&moved, point)
                && junction.is_none_or(|(best, far, _)| (segment, distance) > (best, far))
            {
                junction = Some((segment, distance, point));
            }
        }
        let mut out = Vec::new();
        let on_units = |(x, y): At| (x - origin.0) % 100 == 0 && (y - origin.1) % 100 == 0;
        let own = match junction.filter(|&(_, _, point)| on_units(point)).and_then(|(_, _, point)| Some((point, rest_from(&moved, point)?))) {
            Some((point, rest)) => {
                let mut points = vec![point];
                points.extend(vias_of(&rest, 0));
                points
            }
            None => vias_of(&moved, 0),
        };
        if let Some(Some(name)) = wire.entries.get(self.reader) {
            out.push((name.clone(), moved.clone(), units(&own)));
        }
        for rank in followers {
            let (Some(Some(name)), Some((_, other))) = (wire.entries.get(rank), wire.readers.get(rank)) else { continue };
            // Where it leaves the route dragged, moved with the segment when it left on it.
            let Some((_, split)) = split(path, other) else { continue };
            let shifted = if self.vertical { (split.0 + self.by, split.1) } else { (split.0, split.1 + self.by) };
            let Some(point) = [shifted, split].into_iter().find(|&point| on_path(&moved, point) && on_units(point)) else { continue };
            let Some(rest) = rest_from(other, split) else { continue };
            let mut points = vec![point];
            // Across or down from the junction to where the rest of it goes on, which keeps its shape.
            let mut tail = rest.clone();
            if let Some(first) = tail.first_mut() {
                *first = point;
            }
            if let Some(&next) = rest.get(1)
                && point != split
            {
                let corner = if next.0 == split.0 { (split.0, point.1) } else { (point.0, split.1) };
                tail.insert(1, corner);
            }
            let tail = labcraft_diagram::simplify(tail);
            points.extend(vias_of(&tail, 0));
            let mut drawn = labcraft_diagram::prefix_to(&moved, point).unwrap_or_default();
            drawn.extend(tail.iter().skip(1));
            out.push((name.clone(), labcraft_diagram::simplify(drawn), units(&points)));
        }
        out
    }
}

/// Whether `path` runs along part of the segment from `a` to `b`, more than a point of it.
fn shares(path: &[At], a: At, b: At) -> bool {
    path.windows(2).any(|pair| {
        let (Some(&p), Some(&q)) = (pair.first(), pair.get(1)) else { return false };
        let overlap = |p0: i64, p1: i64, q0: i64, q1: i64| p0.max(p1).min(q0.max(q1)) > p0.min(p1).max(q0.min(q1));
        (a.0 == b.0 && p.0 == q.0 && p.0 == a.0 && overlap(a.1, b.1, p.1, q.1))
            || (a.1 == b.1 && p.1 == q.1 && p.1 == a.1 && overlap(a.0, b.0, p.0, q.0))
    })
}

/// The segment of `path` that `at` lies on first, and how far along it.
fn along(path: &[At], at: At) -> Option<(usize, i64)> {
    path.windows(2).enumerate().find_map(|(index, pair)| match pair {
        &[a, b] if on(a, b, at) => Some((index, (at.0 - a.0).abs() + (at.1 - a.1).abs())),
        _ => None,
    })
}

/// The part of `path` from `at` on, `at` first.
fn rest_from(path: &[At], at: At) -> Option<Vec<At>> {
    let (segment, _) = along(path, at)?;
    let mut rest = vec![at];
    rest.extend(path.get(segment + 1..)?.iter().copied().filter(|&point| point != at));
    Some(rest)
}

/// The origin of the coordinates of the frame of a wire, in hundredths (spec 11).
fn origin_of(drawing: &Drawing, group: usize) -> At {
    drawing.groups.get(group).and_then(|group| group.wire.as_ref()).and_then(|wire| drawing.bodies.get(wire.body)).map_or((0, 0), |body| body.origin)
}

/// `value` on the grid of 8 that starts at `start`.
pub(crate) fn snap(value: i64, start: i64) -> i64 {
    start + (value - start + GRID / 2).div_euclid(GRID) * GRID
}

/// Hundredths to units, rounded.
pub(crate) fn unit(hundredths: i64) -> i64 {
    (hundredths + 50).div_euclid(100)
}

fn on(a: At, b: At, at: At) -> bool {
    labcraft_diagram::on_segment(a, b, at)
}

fn on_path(path: &[At], at: At) -> bool {
    path.windows(2).any(|pair| matches!(pair, &[a, b] if on(a, b, at)))
}

/// The path with its segment `segment` moved across by `by`: the segments on each side follow; one that leaves a
/// terminal gets a jog 16 from it. None when the segment is too short for its jogs.
pub(crate) fn moved(path: &[At], segment: usize, by: i64, vertical: bool) -> Option<Vec<At>> {
    let last = path.len().checked_sub(1)?;
    let (a, b) = (*path.get(segment)?, *path.get(segment + 1)?);
    let shift = |point: At| if vertical { (point.0 + by, point.1) } else { (point.0, point.1 + by) };
    // Along the segment, from `a` toward `b`, by `step`.
    let toward = |from: At, to: At, step: i64| ((from.0 + (to.0 - from.0).signum() * step), (from.1 + (to.1 - from.1).signum() * step));
    let length = (b.0 - a.0).abs() + (b.1 - a.1).abs();
    let jogs = i64::from(segment == 0) + i64::from(segment + 1 == last);
    if jogs > 0 && length <= JOG * jogs {
        return None;
    }
    let mut out: Vec<At> = path.get(..segment).unwrap_or_default().to_vec();
    if segment == 0 {
        let stub = toward(a, b, JOG);
        out.extend([a, stub, shift(stub)]);
    } else {
        out.push(shift(a));
    }
    if segment + 1 == last {
        let stub = toward(b, a, JOG);
        out.extend([shift(stub), stub, b]);
    } else {
        out.push(shift(b));
        out.extend(path.get(segment + 2..).unwrap_or_default());
    }
    Some(labcraft_diagram::simplify(out))
}

/// Where `other` leaves `path`, both from one source: the segment of `path` it leaves on, and the point. None when it
/// does not leave it (the same path) or they share nothing.
fn split(path: &[At], other: &[At]) -> Option<(usize, At)> {
    if path.first() != other.first() {
        return None;
    }
    let mut index = 0;
    while index + 1 < path.len() && index + 1 < other.len() {
        let (Some(&a), Some(&b), Some(&c)) = (path.get(index), path.get(index + 1), other.get(index + 1)) else { break };
        if b == c {
            index += 1;
            continue;
        }
        // Along the same line from `a`: they part where the shorter ends.
        let same_line = (a.0 == b.0 && a.0 == c.0 && (b.1 - a.1).signum() == (c.1 - a.1).signum())
            || (a.1 == b.1 && a.1 == c.1 && (b.0 - a.0).signum() == (c.0 - a.0).signum());
        if !same_line {
            return Some((index, a));
        }
        let nearer = if (b.0 - a.0).abs() + (b.1 - a.1).abs() < (c.0 - a.0).abs() + (c.1 - a.1).abs() { b } else { c };
        return Some((if nearer == b { index + 1 } else { index }, nearer));
    }
    if path.len() == other.len() {
        return None;
    }
    Some((index, *path.get(index)?))
}

/// The points of `wire … via` that route `path` from its point `start` as the drawing would (notation 2.2): across,
/// then down to each; the end of each vertical, the last segment, which the reader's own approach draws, left out.
pub(crate) fn vias_of(path: &[At], start: usize) -> Vec<At> {
    let Some(rest) = path.get(start..) else { return Vec::new() };
    let segments = rest.len().saturating_sub(1);
    rest.windows(2)
        .take(segments.saturating_sub(1))
        .filter_map(|pair| match pair {
            &[a, b] if a.0 == b.0 => Some(b),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked by hand: a straight wire from (0, 0) to (100 units, 0), its one segment moved down by 24.
    #[test]
    fn a_straight_wire_moved_jogs_at_each_end() {
        let path = [(0, 0), (10_000, 0)];
        let found = moved(&path, 0, 2_400, false).unwrap();
        assert_eq!(found, [(0, 0), (1_600, 0), (1_600, 2_400), (8_400, 2_400), (8_400, 0), (10_000, 0)]);
        // Across to 16, down to 24; across to 84, down to 0: the last vertical is the reader's own approach.
        assert_eq!(vias_of(&found, 0), [(1_600, 2_400), (8_400, 0)]);
        // Too short for its two jogs.
        assert_eq!(moved(&[(0, 0), (3_000, 0)], 0, 800, false), None);
    }

    /// A vertical in the middle moved right by 16: the horizontals on each side stretch, nothing jogs.
    #[test]
    fn a_middle_segment_moved_stretches_its_neighbours() {
        let path = [(0, 0), (4_000, 0), (4_000, 8_000), (10_000, 8_000)];
        let found = moved(&path, 1, 1_600, true).unwrap();
        assert_eq!(found, [(0, 0), (5_600, 0), (5_600, 8_000), (10_000, 8_000)]);
        assert_eq!(vias_of(&found, 0), [(5_600, 8_000)]);
    }

    /// Two readers of one value: the second leaves the first at (40, 0), going down.
    #[test]
    fn a_reader_leaves_another_where_their_paths_part() {
        let trunk = [(0, 0), (10_000, 0)];
        let branch = [(0, 0), (4_000, 0), (4_000, 8_000), (10_000, 8_000)];
        assert_eq!(split(&trunk, &branch), Some((0, (4_000, 0))));
        assert_eq!(split(&branch, &trunk).map(|(_, point)| point), Some((4_000, 0)));
        assert_eq!(split(&trunk, &trunk), None);
    }

    #[test]
    fn the_grid_counts_from_the_frame() {
        assert_eq!(snap(1_000, 100), 900);
        assert_eq!(snap(1_300, 100), 1_700);
        assert_eq!(snap(-500, 0), -800);
    }
}
