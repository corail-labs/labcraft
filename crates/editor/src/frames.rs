//! The frame under a point (notation 2.6): where quick insertion, a drop from the panel `Palette` and a paste put what
//! they make, the body of the function or the arm shown of a structure, as `inside` and `arm` name it (commands.md 6.7).

use egui::Pos2;
use labcraft_diagram::Drawing;

use crate::select::{Selection, body_rect};
use crate::{Canvas, Shown, arms_shown};

/// The grid of a gesture (notation 2.4).
const GRID: f32 = 8.0;

/// The structures that surrounding makes and quick insertion lists, in the order of the menu (notation 2.6).
pub const STRUCTURES: [&str; 4] = ["match", "handle", "for", "loop"];

/// A value that a frame sees, by its name, with its type as the language writes it (empty when `check` does not know it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub name: String,
    pub ty: String,
}

/// A frame where something goes: the structure, as a command names it, and the arm of it by its patterns; neither for the
/// body of the function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InFrame {
    pub inside: Option<String>,
    pub arm: Option<String>,
}

impl Canvas {
    /// The frame shown under `unit`, a point of the canvas in units, and that point in units of that frame on the grid of
    /// 8: the deepest body that shows there, the arm shown of a structure in tabs. The body of the function when the point
    /// is in no structure, or in one that a command cannot name.
    pub(crate) fn frame_under(&self, drawing: &Drawing, visible: &dyn Fn(Option<usize>) -> bool, unit: Pos2) -> (InFrame, (i64, i64)) {
        let (frame, at, _) = self.frame_body(drawing, visible, unit);
        (frame, at)
    }

    /// [`Canvas::frame_under`], and the body of that frame by its index in the drawing.
    pub(crate) fn frame_body(&self, drawing: &Drawing, visible: &dyn Fn(Option<usize>) -> bool, unit: Pos2) -> (InFrame, (i64, i64), usize) {
        let deepest = (1..drawing.bodies.len())
            .filter(|&body| self.index.body_shown(body, visible))
            .filter_map(|body| body_rect(drawing, body).filter(|area| area.contains(unit)).map(|area| (body, area.area())))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(body, _)| body);
        let named = deepest.and_then(|body| {
            let found = drawing.bodies.get(body)?;
            let structure = drawing.placed.get(found.owner?)?;
            (!structure.name.is_empty()).then(|| (body, structure.name.clone(), (!found.arm.is_empty()).then(|| found.arm.clone())))
        });
        let (body, frame) = match named {
            Some((body, inside, arm)) => (body, InFrame { inside: Some(inside), arm }),
            None => (0, InFrame::default()),
        };
        let origin = drawing.bodies.get(body).map_or((0, 0), |found| found.origin);
        let snap = |value: f32| ((value / GRID).round() * GRID) as i64;
        (frame, (snap(unit.x - origin.0 as f32 / 100.0), snap(unit.y - origin.1 as f32 / 100.0)), body)
    }

    /// The values that a structure could look at or go over, with their types (notation 2.6): those that the frame sees,
    /// of the nodes selected when `selection` names some, else of the frame under `screen`; the values that those nodes
    /// read come first, then the others in the order of the drawing; none that the nodes make. Each value once.
    pub fn values_seen(&self, shown: &Shown, selection: &Selection, screen: Option<Pos2>) -> Vec<Seen> {
        let Some(drawing) = self.drawing.as_ref() else { return Vec::new() };
        let found = self.index.find(selection);
        let selected = &found.nodes;
        let body = match selected.first().and_then(|placed| drawing.placed.get(*placed)) {
            Some(node) => node.body,
            None => {
                let arms = arms_shown(drawing, shown);
                let visible = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
                let unit = screen.map(|at| self.unit_at(at));
                unit.map_or(0, |unit| self.frame_body(drawing, &visible, unit).2)
            }
        };
        // The body and those around it, which it sees; not the outputs of the structures that hold it, which it would
        // read from inside.
        let mut seen_bodies = vec![body];
        let mut owners = Vec::new();
        let mut at = body;
        while let Some(owner) = drawing.bodies.get(at).and_then(|found| found.owner)
            && let Some(node) = drawing.placed.get(owner)
            && !seen_bodies.contains(&node.body)
        {
            owners.push(owner);
            seen_bodies.push(node.body);
            at = node.body;
        }
        let inside = |placed: Option<usize>| placed.is_some_and(|placed| selected.contains(&placed));
        let mut first: Vec<Seen> = Vec::new();
        let mut rest: Vec<Seen> = Vec::new();
        let read: std::collections::HashSet<&str> =
            drawing.ports.iter().filter(|port| port.input && inside(port.placed)).map(|port| port.value.as_str()).collect();
        let mut listed: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for port in &drawing.ports {
            let plain = !port.name.is_empty() && !port.name.contains(['.', ' ']) && port.name != "_";
            if port.input
                || !plain
                || !seen_bodies.contains(&port.body)
                || inside(port.placed)
                || port.placed.is_some_and(|placed| owners.contains(&placed) && drawing.placed.get(placed).is_some_and(|node| node.body == port.body))
            {
                continue;
            }
            if !listed.insert(port.name.as_str()) {
                continue;
            }
            let ty = port.ty.and_then(|ty| drawing.types.get(ty)).cloned().unwrap_or_default();
            let value = Seen { name: port.name.clone(), ty };
            if read.contains(&port.name.as_str()) { first.push(value) } else { rest.push(value) }
        }
        first.extend(rest);
        first
    }

    /// A point of the screen as a point of the canvas, in units.
    pub(crate) fn unit_at(&self, screen: Pos2) -> Pos2 {
        (self.viewport.origin + (screen - self.rect.min) / self.viewport.zoom).to_pos2()
    }

    /// Where on the screen a list about the nodes selected opens: below the box around them. None when none shows.
    pub fn selection_screen(&self, selection: &Selection) -> Option<Pos2> {
        let drawing = self.drawing.as_ref()?;
        let area = crate::select::focus_rect(drawing, &self.index.find(selection))?;
        let unit = area.left_bottom();
        Some(self.rect.min + (unit.to_vec2() - self.viewport.origin) * self.viewport.zoom)
    }

    /// [`Canvas::frame_under`] a point of the screen, when the canvas holds a drawing.
    pub fn frame_at(&self, shown: &Shown, screen: Pos2) -> Option<(InFrame, (i64, i64))> {
        let drawing = self.drawing.as_ref()?;
        let unit = self.unit_at(screen);
        let arms = arms_shown(drawing, shown);
        let visible = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
        Some(self.frame_under(drawing, &visible, unit))
    }
}
