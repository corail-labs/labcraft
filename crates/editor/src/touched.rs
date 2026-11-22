//! What an edit of an agent touched, and its highlight (ide.md 8): the nodes and wires of a drawing that differ from the
//! drawing before it, highlighted for a moment on the canvas.
//!
//! **Touched.** A node, found by its key (notation 2.1), is touched when it is new, when its shapes differ once taken
//! to its corner (a constant set, a terminal wired, a label), or when the place that the file fixes for it changed: a
//! free node that the layout only placed elsewhere is not.
//! A wire, found by its value and the terminals it joins, is touched when it is new, or when it keeps waypoints that
//! moved; a wire that the routing only drew again around a new node is not.

use std::collections::{HashMap, HashSet};

use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use labcraft_diagram::{Drawing, Group, Shape, moved};

/// How long the highlight lasts *(proposed)*, the last part of it fading unless motion is reduced.
pub const HIGHLIGHT_SECONDS: f64 = 2.0;
const FADE_SECONDS: f64 = 0.6;

/// A wire, as the next drawing finds it again: its value, the terminal it leaves and those it reaches.
type WireKey = (String, Option<String>, Vec<String>);

/// What an edit touched, by what follows it from one drawing to the next.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Touched {
    /// The keys of the nodes.
    pub nodes: Vec<String>,
    pub wires: Vec<WireKey>,
}

impl Touched {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.wires.is_empty()
    }
}

fn wire_key(wire: &labcraft_diagram::WireOf) -> WireKey {
    let mut to = wire.to.clone();
    to.sort();
    (wire.value.clone(), wire.from.clone(), to)
}

/// The groups that draw each node, by its index in [`Drawing::placed`]: one pass over the groups.
fn groups_by_node(drawing: &Drawing) -> Vec<Vec<&Group>> {
    let mut groups = vec![Vec::new(); drawing.placed.len()];
    for group in drawing.groups.iter().filter(|group| group.wire.is_none()) {
        if let Some(of) = group.placed.and_then(|placed| groups.get_mut(placed)) {
            of.push(group);
        }
    }
    groups
}

/// Whether two nodes are drawn the same, those of `old` moved by `(dx, dy)`: an outline or an icon by what it draws, not
/// by its place in the tables of its drawing, which another drawing numbers its own way.
fn drawn_alike((old, before): (&Drawing, &[&Group]), (new, after): (&Drawing, &[&Group]), dx: i64, dy: i64) -> bool {
    let (mut was, mut is) = (before.iter().flat_map(|group| &group.shapes), after.iter().flat_map(|group| &group.shapes));
    loop {
        let alike = match (was.next(), is.next()) {
            (None, None) => return true,
            (Some(Shape::Outline { outline: a, .. }), Some(Shape::Outline { outline: b, .. })) if old.outlines.get(*a) != new.outlines.get(*b) => {
                false
            }
            (Some(Shape::Icon { icon: a, .. }), Some(Shape::Icon { icon: b, .. })) if old.icons.get(*a) != new.icons.get(*b) => false,
            // The same tables: the rest of the shape decides, its index taken as the other's.
            (Some(Shape::Outline { x, y, extent, paint, .. }), Some(Shape::Outline { x: x2, y: y2, extent: e2, paint: p2, .. })) => {
                (x + dx, y + dy, extent, paint) == (*x2, *y2, e2, p2)
            }
            (Some(Shape::Icon { x, y, side, .. }), Some(Shape::Icon { x: x2, y: y2, side: s2, .. })) => (x + dx, y + dy, side) == (*x2, *y2, s2),
            (Some(a), Some(b)) if (dx, dy) == (0, 0) => a == b,
            (Some(a), Some(b)) => moved(a.clone(), dx, dy) == *b,
            _ => false,
        };
        if !alike {
            return false;
        }
    }
}

fn wire_points(group: &Group) -> impl Iterator<Item = (i64, i64)> + '_ {
    group
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Wire { points, .. } => Some(points.iter().copied()),
            _ => None,
        })
        .flatten()
}

/// What differs from `old` to `new`, the same function drawn again after an edit.
pub fn touched(old: &Drawing, new: &Drawing) -> Touched {
    let (dx, dy) = crate::view::shift_of_frame(old, new);
    let (old_groups, new_groups) = (groups_by_node(old), groups_by_node(new));
    let by_key: HashMap<&str, usize> = old.placed.iter().enumerate().map(|(at, placed)| (placed.key.as_str(), at)).collect();
    let mut touched = Touched::default();
    for (at, placed) in new.placed.iter().enumerate() {
        if placed.key.is_empty() {
            continue;
        }
        // Where the file fixes it, the same; and drawn the same from its own corner, wherever the layout put it.
        let same = by_key.get(placed.key.as_str()).and_then(|&before| Some((before, old.placed.get(before)?))).is_some_and(|(before, was)| {
            was.at == placed.at
                && drawn_alike(
                    (old, old_groups.get(before).map(Vec::as_slice).unwrap_or_default()),
                    (new, new_groups.get(at).map(Vec::as_slice).unwrap_or_default()),
                    placed.x - was.x,
                    placed.y - was.y,
                )
        });
        if !same {
            touched.nodes.push(placed.key.clone());
        }
    }
    let before: HashMap<WireKey, &Group> = old.groups.iter().filter_map(|group| group.wire.as_ref().map(|wire| (wire_key(wire), group))).collect();
    let mut seen = HashSet::new();
    for group in &new.groups {
        let Some(wire) = &group.wire else { continue };
        let key = wire_key(wire);
        let same = before.get(&key).is_some_and(|was| !wire.via || wire_points(was).map(|(x, y)| (x + dx, y + dy)).eq(wire_points(group)));
        if !same && seen.insert(key.clone()) {
            touched.wires.push(key);
        }
    }
    touched
}

/// A highlight on the canvas, from `since` on the clock of egui: what it touched by its keys, and found on the drawing
/// shown, by index, once for each drawing.
#[derive(Clone, Debug)]
pub(crate) struct Highlight {
    pub touched: Touched,
    pub since: f64,
    /// The nodes, by their index in [`Drawing::placed`], and the groups of the wires, on the drawing shown.
    nodes: Vec<usize>,
    wires: Vec<usize>,
}

impl Highlight {
    pub fn new(touched: Touched, since: f64) -> Highlight {
        Highlight { touched, since, nodes: Vec::new(), wires: Vec::new() }
    }

    /// Finds what it touched on `drawing`.
    pub fn find_on(&mut self, drawing: &Drawing) {
        let nodes: HashSet<&str> = self.touched.nodes.iter().map(String::as_str).collect();
        let wires: HashSet<&WireKey> = self.touched.wires.iter().collect();
        self.nodes = drawing.placed.iter().enumerate().filter(|(_, placed)| nodes.contains(placed.key.as_str())).map(|(at, _)| at).collect();
        self.wires = drawing
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.wire.as_ref().is_some_and(|wire| wires.contains(&wire_key(wire))))
            .map(|(at, _)| at)
            .collect();
    }

    /// When it next looks different, from `now`: at once while it glides or fades, else when it starts to fade.
    pub fn next_change(&self, now: f64, gliding: bool, reduced_motion: bool) -> std::time::Duration {
        let fade = self.since + HIGHLIGHT_SECONDS - if reduced_motion { 0.0 } else { FADE_SECONDS };
        if gliding || now >= fade { std::time::Duration::ZERO } else { std::time::Duration::from_secs_f64(fade - now) }
    }

    /// How strong it is at `now`, from 1 to 0; none once it is over.
    pub fn strength(&self, now: f64, reduced_motion: bool) -> Option<f32> {
        let age = now - self.since;
        if !(0.0..HIGHLIGHT_SECONDS).contains(&age) {
            return None;
        }
        if reduced_motion {
            return Some(1.0);
        }
        let left = HIGHLIGHT_SECONDS - age;
        Some(if left < FADE_SECONDS { (left / FADE_SECONDS) as f32 } else { 1.0 })
    }

    /// Paints it over `drawing` in `colour`, faded to its strength: a halo around each node, where it glides to, and a
    /// broad stroke along each wire, once the nodes no longer glide (the new wires show at the end of the glide).
    pub fn paint(
        &self,
        painter: &egui::Painter,
        drawing: &Drawing,
        colour: Color32,
        zoom: f32,
        gliding: Option<&dyn Fn(usize) -> Vec2>,
        to_screen: &dyn Fn(Vec2) -> Pos2,
    ) {
        let faded = |alpha: f32| colour.gamma_multiply(alpha);
        for (at, placed) in self.nodes.iter().filter_map(|&at| Some((at, drawing.placed.get(at)?))) {
            let offset = gliding.map_or(Vec2::ZERO, |offset_of| offset_of(at));
            let corner = offset + Vec2::new(placed.x as f32, placed.y as f32) / 100.0;
            let size = Vec2::new(placed.width as f32, placed.height as f32) / 100.0;
            let area = Rect::from_min_max(to_screen(corner), to_screen(corner + size)).expand(4.0);
            painter.rect(area, 4.0, faded(0.14), Stroke::new(2.0, faded(0.9)), egui::StrokeKind::Outside);
        }
        if gliding.is_some() {
            return;
        }
        for group in self.wires.iter().filter_map(|&at| drawing.groups.get(at)) {
            for shape in &group.shapes {
                if let Shape::Wire { points, .. } = shape {
                    let line: Vec<Pos2> = points.iter().map(|&(x, y)| to_screen(Vec2::new(x as f32, y as f32) / 100.0)).collect();
                    painter.add(egui::Shape::line(line, Stroke::new((6.0 * zoom).clamp(4.0, 12.0), faded(0.35))));
                }
            }
        }
    }
}
