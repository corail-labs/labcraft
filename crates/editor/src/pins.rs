//! The pin of a fixed node (notation 2.4, 2.5): the icon `pin` of Lucide (`assets/icons/lucide/pin.svg`, ISC, attributed
//! in `ATTRIBUTION.md`; decision of the owner, 2026-11-17) in `ink.muted`, 16 units across, on the top-left corner of the
//! body of a node that `layout` fixes. It shows an entry of the file, but only the editor draws it, under the setting
//! "mark the fixed nodes"; an export never does.

use std::sync::OnceLock;

use egui::{Pos2, Rect, Stroke, Vec2};
use labcraft_diagram::{Drawing, path_segments};

use crate::view::Grid;
use crate::{Frame, Theme};

/// The icon, as Lucide gives it.
const SVG: &str = include_str!("../../../assets/icons/lucide/pin.svg");
/// Its size across, in units (notation 2.5), and the size of the grid it is drawn on.
const SIDE: f32 = 16.0;
const GRID: f32 = 24.0;
/// The width of its lines on its grid, as the file says (`stroke-width="2"`).
const STROKE: f32 = 2.0;

/// The lines of the icon on its grid of 24, its curves flattened: read once from the file.
fn lines() -> &'static [Vec<Pos2>] {
    static LINES: OnceLock<Vec<Vec<Pos2>>> = OnceLock::new();
    LINES.get_or_init(|| {
        SVG.split(" d=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .filter_map(|data| path_segments(data).ok())
            .flat_map(|segments| crate::paint::subpaths(&segments))
            .map(|(points, closed)| {
                let mut line: Vec<Pos2> = points.iter().map(|&(x, y)| Pos2::new(x as f32, y as f32)).collect();
                if closed && let Some(&first) = line.first() {
                    line.push(first);
                }
                line
            })
            .collect()
    })
}

/// Where the pin of a node is, in units: centred on the top-left corner of its body.
pub(crate) fn area(drawing: &Drawing, placed: usize, offset: Vec2) -> Option<Rect> {
    let node = drawing.placed.get(placed)?;
    node.at?;
    let corner = Pos2::new(node.x as f32, node.y as f32) / 100.0 + offset;
    Some(Rect::from_center_size(corner, Vec2::splat(SIDE)))
}

/// Paints the pin in `area`, a rectangle of the screen.
pub(crate) fn paint(painter: &egui::Painter, area: Rect, theme: Theme, zoom: f32) {
    let scale = area.width() / GRID;
    let stroke = Stroke::new((STROKE * SIDE / GRID * zoom).max(1.0), theme.color("ink.muted"));
    for line in lines() {
        let points: Vec<Pos2> = line.iter().map(|point| area.min + point.to_vec2() * scale).collect();
        painter.add(egui::Shape::line(points, stroke));
    }
}

/// The fixed node whose pin is under `unit`, among those shown.
pub(crate) fn under(drawing: &Drawing, frame: &Frame, grid: &Grid, unit: Pos2) -> Option<usize> {
    (0..drawing.placed.len())
        .filter(|&placed| grid.shown(placed, &|within| frame.visible(within)))
        .find(|&placed| area(drawing, placed, frame.offset_of(Some(placed))).is_some_and(|area| area.contains(unit)))
}

/// What the pin says on hover (notation 2.4).
pub(crate) fn tooltip(drawing: &Drawing, placed: usize) -> String {
    let Some(node) = drawing.placed.get(placed) else { return String::new() };
    let (x, y) = node.at.unwrap_or_default();
    format!("Position fixed (`node {} at ({}, {})` in `layout`). Right-click › Remove the fixed position", node.name, x / 100, y / 100)
}
