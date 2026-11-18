//! Where quick insertion opens and from what (notation 2.6), and the drop of a function dragged from the panel `Palette`
//! (2.5): the canvas says where the node falls and what wire it comes with; the host shows the list and makes the
//! commands (`node.add`, `constant.add`, `wire.connect` in one `commands.batch`).

use std::sync::Arc;

use egui::Pos2;
use labcraft_diagram::{Drawing, Holds};

use crate::insertion::{Catalog, Wanted};
use crate::{Canvas, Shown, select, wiring};

/// What quick insertion is opened from (notation 2.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Space on the empty canvas, or with several nodes selected: the whole list.
    Nothing,
    /// A wire from this terminal, as a command names it: dropped on the empty canvas, or Space on the terminal in focus
    /// or while it is drawn from the keyboard. From an output, the node takes it at an input; drawn back from an input,
    /// the node gives it at an output, which replaces what the input holds.
    /// `arm`: the arm of a `yield`, by its patterns, which `wire.connect` takes with its name (commands.md 9.2).
    Terminal { name: String, arm: Option<String>, wanted: Wanted, replace: bool },
    /// Space with one node selected (autopatch): its first output, which the first input of the new node that takes it
    /// is wired to.
    Node { name: String, wanted: Wanted },
    /// Space with a wire selected: the node goes into it (`node.add` with `into`).
    Wire { name: String, wanted: Wanted },
}

/// Quick insertion, as the canvas opens it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quick {
    /// Where the node falls, in units of its frame (its `node … at`), on the grid of 8; none to place it automatically, as
    /// a node inserted into a wire is.
    pub at: Option<(i64, i64)>,
    /// The frame it falls in, under the point where the list opens: an arm or a loop body, or the body of the function.
    pub within: crate::InFrame,
    /// Where the list opens, on the screen.
    pub screen: Pos2,
    pub source: Source,
}

/// A function dragged from the panel `Palette`: the catalog and its entry, by its index, for its ghost and the wire it
/// can go into (notation 2.5).
#[derive(Clone, Debug)]
pub struct Dragged {
    pub catalog: Arc<Catalog>,
    pub entry: usize,
}

/// The grid of a gesture (notation 2.4).
const GRID: f32 = 8.0;
/// How far to the right of what it comes from a node falls, in units.
const BESIDE: f32 = 48.0;

/// A point of the canvas, in units, as a `node … at` of the frame of the function, on the grid.
pub(crate) fn frame_point(drawing: &Drawing, unit: Pos2) -> (i64, i64) {
    let origin = drawing.bodies.first().map_or((0, 0), |body| body.origin);
    let snap = |value: f32| ((value / GRID).round() * GRID) as i64;
    (snap(unit.x - origin.0 as f32 / 100.0), snap(unit.y - origin.1 as f32 / 100.0))
}

impl Canvas {
    /// Quick insertion from what the canvas holds (notation 2.6): the wire drawn from the keyboard, the terminal in
    /// focus, the one node selected (autopatch, when `autopatch` is on), the wire selected, else nothing; the node falls
    /// under `pointer` when it is on the canvas, else at the focus. None without a drawing.
    pub fn quick_insert(&mut self, shown: &Shown, autopatch: bool, pointer: Option<Pos2>) -> Option<Quick> {
        let drawing = self.drawing.clone()?;
        let rect = self.rect;
        let arms = crate::arms_shown(&drawing, shown);
        let visible = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
        let to_screen = |unit: Pos2| rect.min + (unit.to_vec2() - self.viewport.origin) * self.viewport.zoom;
        let unit_of = |at: Pos2| (self.viewport.origin + (at - rect.min) / self.viewport.zoom).to_pos2();
        // A wire drawn from the keyboard, or the terminal in focus.
        let from = self.wiring.as_ref().map(|wiring| wiring.from).or(self.port_focus);
        if let Some(port) = from.and_then(|at| drawing.ports.get(at).map(|port| (at, port))) {
            let (index, port) = port;
            // A terminal whose type is not known keeps the wire being drawn.
            let wanted = Wanted::of_port(&drawing, index)?;
            self.wiring = None;
            let centre = wiring::centre(port);
            let unit = if port.input { centre - egui::vec2(BESIDE * 3.0, 0.0) } else { centre + egui::vec2(BESIDE, 0.0) };
            let replace = port.input && matches!(port.holds, Holds::Wire | Holds::Constant(_));
            let arm = (!port.arm.is_empty()).then(|| port.arm.clone());
            let source = Source::Terminal { name: port.name.clone(), arm, wanted, replace };
            let (within, at) = self.frame_under(&drawing, &visible, unit);
            return Some(Quick { at: Some(at), within, screen: to_screen(unit), source });
        }
        let found = self.index.find(&shown.selection);
        let under = pointer.filter(|at| rect.contains(*at)).map(unit_of);
        // One wire selected: into it, the reader nearest to where it was clicked.
        if found.nodes.is_empty() && found.groups.len() == 1 {
            let group = found.groups.first().copied()?;
            let wire = drawing.groups.get(group)?.wire.as_ref()?;
            let near = self.pinned.filter(|(pinned, _)| *pinned == group).map(|(_, at)| at).or(under);
            let name = nearest_entry(wire, near)?;
            let wanted = Wanted::of_wire(&drawing, group)?;
            let screen = near.map_or(rect.center(), to_screen);
            return Some(Quick { at: None, within: crate::InFrame::default(), screen, source: Source::Wire { name, wanted } });
        }
        // One node selected: autopatch from its first output, the node to its right.
        if autopatch
            && found.groups.is_empty()
            && found.nodes.len() == 1
            && let Some(placed) = found.nodes.first()
            && let Some(node) = drawing.placed.get(*placed)
        {
            let unit = Pos2::new((node.x + node.width) as f32 / 100.0 + BESIDE, node.y as f32 / 100.0);
            let (within, at) = self.frame_under(&drawing, &visible, unit);
            let at = Some(at);
            let output = drawing.ports.iter().position(|port| port.placed == Some(*placed) && !port.input && !port.order && !port.name.is_empty());
            let source = output
                .and_then(|output| Some(Source::Node { name: drawing.ports.get(output)?.name.clone(), wanted: Wanted::of_port(&drawing, output)? }))
                .unwrap_or(Source::Nothing);
            return Some(Quick { at, within, screen: to_screen(unit), source });
        }
        // The whole list, under the pointer, else at the focus, else in the middle of the view.
        let focus = select::focus_rect(&drawing, &found).map(|area| area.center());
        let unit = under.or(focus).unwrap_or_else(|| unit_of(rect.center()));
        let (within, at) = self.frame_under(&drawing, &visible, unit);
        Some(Quick { at: Some(at), within, screen: to_screen(unit), source: Source::Nothing })
    }

    /// The wires that Delete on a terminal removes (notation 2.6): of a terminal of the frame (`input x`, `output y`) or
    /// of the terminal in focus, as `wire.disconnect` names them. An input of a node, or an output of the function, has
    /// its one wire; an output of a node, or an input of the function, every wire of its value.
    /// The input in focus when it holds a constant, as `input.clear_constant` names it: Delete takes the constant out
    /// (notation 2.6).
    pub fn constant_in_focus(&self) -> Option<String> {
        let drawing = self.drawing.as_ref()?;
        let port = self.port_focus.and_then(|at| drawing.ports.get(at))?;
        (port.input && matches!(port.holds, Holds::Constant(_)) && port.never.is_empty() && !port.name.is_empty()).then(|| port.name.clone())
    }

    pub fn wires_of_terminal(&self, terminal: Option<&str>) -> Vec<String> {
        let Some(drawing) = self.drawing.as_ref() else { return Vec::new() };
        let port = match terminal {
            Some(name) => drawing.ports.iter().position(|port| port.frame && port.placed.is_none() && port.name == name),
            None => self.port_focus,
        };
        let Some(port) = port.and_then(|at| drawing.ports.get(at)) else { return Vec::new() };
        let Some(wire) = port.group.and_then(|group| drawing.groups.get(group)).and_then(|group| group.wire.as_ref()) else { return Vec::new() };
        if !port.input {
            return wire.named.clone();
        }
        let end = port.name.strip_prefix("output ").unwrap_or(&port.name);
        wire.named.iter().filter(|named| named.rsplit_once("->").is_some_and(|(_, to)| to.trim() == end || to.trim() == port.name)).cloned().collect()
    }

    /// While a function from the panel `Palette` is dragged over the canvas: where it would fall, in units of the canvas,
    /// and the wire under the pointer that it would go into, by its group and its name (notation 2.5). `into` off, it goes
    /// into none (the setting "inserting into a wire", 2.6).
    pub(crate) fn dragged_over(&self, dragged: &Dragged, unit: Pos2, wire_under: Option<usize>, into: bool) -> Option<(usize, String)> {
        let drawing = self.drawing.as_ref()?;
        let entry = dragged.catalog.entries.get(dragged.entry)?;
        let group = wire_under.filter(|_| into)?;
        let wanted = Wanted::of_wire(drawing, group)?;
        if !dragged.catalog.goes_into(entry, &wanted) {
            return None;
        }
        let wire = drawing.groups.get(group)?.wire.as_ref()?;
        let name = nearest_entry(wire, Some(unit))?;
        Some((group, name))
    }
}

/// The name of the wire, as `wire.*` takes it, to the reader nearest to `near` (in units); without a point, or when no
/// reader has a name of its own, the first name of the wire.
fn nearest_entry(wire: &labcraft_diagram::WireOf, near: Option<Pos2>) -> Option<String> {
    near.and_then(|near| {
        wire.readers
            .iter()
            .zip(&wire.entries)
            .filter_map(|((_, path), entry)| Some((distance(path, near), entry.clone()?)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, entry)| entry)
    })
    .or_else(|| wire.named.first().cloned())
}

/// The distance from a point, in units, to a path in hundredths.
fn distance(path: &[(i64, i64)], at: Pos2) -> f32 {
    let point = |(x, y): (i64, i64)| Pos2::new(x as f32 / 100.0, y as f32 / 100.0);
    path.windows(2)
        .filter_map(|pair| match pair {
            [a, b] => Some(segment_distance(point(*a), point(*b), at)),
            _ => None,
        })
        .fold(f32::INFINITY, f32::min)
}

fn segment_distance(a: Pos2, b: Pos2, at: Pos2) -> f32 {
    let along = b - a;
    let length = along.length_sq();
    let t = if length > 0.0 { ((at - a).dot(along) / length).clamp(0.0, 1.0) } else { 0.0 };
    (a + along * t).distance(at)
}
