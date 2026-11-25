//! The signature on the canvas (visual notation 2.4; decisions of the owner, 2026-11-19): the terminals of the frame of
//! the function, renamed, typed, given a default or removed; a terminal of a node that holds nothing made an input or an
//! output of the function, by its context menu or by a wire dropped on the edge of the frame. The canvas says what it
//! asks; the commands judge it again (`signature.*`, commands.md 6.7).

use std::collections::BTreeSet;

use egui::{Pos2, Vec2};
use labcraft_diagram::{Drawing, Holds, Port, Shape};

use crate::Intent;
use crate::fields::FieldKind;

/// The mark before the default of an input of the function, on its pill (notation 2.5).
const DEFAULT_MARK: &str = "◂ ";

/// Whether a default as the drawing shows it is cut (a long text ends with `…`): a field would write the cut text, so
/// none opens on it, and the `Text` view edits it.
pub fn default_is_cut(default: &str) -> bool {
    default.ends_with('…')
}

/// What the canvas says of a default too long to edit in a field.
pub(crate) const CUT_DEFAULT: &str = "This default is too long to edit here: edit it in the Text view";

/// A terminal of the frame of the function, as `signature.*` names it: `input x` or `output x`. None for another port.
pub fn frame_terminal(port: &Port) -> Option<String> {
    if !port.frame || port.placed.is_some() || port.caption.is_empty() {
        return None;
    }
    // An input of the function gives a value (no wire reaches it); an output takes one.
    Some(if port.input { format!("output {}", port.caption) } else { format!("input {}", port.caption) })
}

/// The height of a port from the origin of the frame of the function (spec 11), in units: what `input x at y` writes.
pub fn frame_height(drawing: &Drawing, port: &Port) -> Option<i64> {
    let origin = drawing.bodies.first()?.origin.1;
    Some(crate::rerouting::unit(port.y - origin))
}

/// The name of a function without its library: `settle` of `dmm_driver.settle`, as its file and its title name it.
pub fn own_name(function: &str) -> &str {
    function.rsplit('.').next().unwrap_or(function)
}

/// The frame of the function, in units: its left, top, right and bottom.
fn function_frame(drawing: &Drawing) -> Option<(f32, f32, f32, f32)> {
    let body = drawing.bodies.first()?;
    let (x, y, width, height) = body.frame;
    Some((x as f32 / 100.0, y as f32 / 100.0, (x + width) as f32 / 100.0, (y + height) as f32 / 100.0))
}

/// The edge of the frame of the function at `at`, within `reach` units: `Some(true)` for its left edge, where its inputs
/// are, `Some(false)` for its right edge.
pub(crate) fn function_edge(drawing: &Drawing, at: Pos2, reach: f32) -> Option<bool> {
    let (left, top, right, bottom) = function_frame(drawing)?;
    if at.y < top - reach || at.y > bottom + reach {
        return None;
    }
    let (to_left, to_right) = ((at.x - left).abs(), (at.x - right).abs());
    match (to_left <= reach, to_right <= reach) {
        (true, true) => Some(to_left <= to_right),
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    }
}

/// Where the field of a new terminal opens, in units: on the edge of its side, at the height `y`.
pub(crate) fn edge_field_at(drawing: &Drawing, input: bool, y: f32) -> Pos2 {
    match function_frame(drawing) {
        Some((left, _, _, _)) if input => Pos2::new(left + 4.0, y - 8.0),
        Some((_, _, right, _)) => Pos2::new(right - 120.0, y - 8.0),
        None => Pos2::new(0.0, y - 8.0),
    }
}

/// `base`, or `base2`, `base3`… the first that is free (N-14); a base that ends in a digit takes `_` before its number.
/// The rule of `edit`, on the names the drawing shows: the command refuses a name taken that it does not show.
pub fn fresh(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    let gap = if base.ends_with(|c: char| c.is_ascii_digit()) { "_" } else { "" };
    (2..).map(|n| format!("{base}{gap}{n}")).find(|name| !taken.contains(name)).unwrap_or_else(|| base.to_owned())
}

/// The names that the drawing shows: the terminals of the frame of the function, the values on the wires and the nodes.
fn shown_names(drawing: &Drawing) -> BTreeSet<String> {
    let mut taken = BTreeSet::new();
    for port in &drawing.ports {
        if port.frame && port.placed.is_none() && !port.caption.is_empty() {
            taken.insert(port.caption.clone());
        }
        if !port.value.is_empty() && port.value != "_" {
            taken.insert(port.value.clone());
        }
    }
    for placed in &drawing.placed {
        if !placed.name.is_empty() {
            taken.insert(placed.name.trim_start_matches('#').to_owned());
        }
    }
    taken
}

/// What a terminal of a node that holds nothing can become (notation 2.4): an input of the function for an input, an
/// output for an output; its type as the language writes it, when `check` knows it. None for another terminal.
pub(crate) fn makeable(drawing: &Drawing, at: usize) -> Option<(bool, Option<String>)> {
    let port = drawing.ports.get(at)?;
    if port.frame || port.placed.is_none() || port.name.is_empty() || port.order || !port.never.is_empty() {
        return None;
    }
    let free =
        if port.input { matches!(port.holds, Holds::Missing | Holds::Default(_) | Holds::Hole | Holds::Nothing) } else { port.group.is_none() };
    if !free {
        return None;
    }
    let ty = port.ty.and_then(|ty| drawing.types.get(ty)).filter(|ty| !ty.is_empty()).cloned();
    Some((port.input, ty))
}

/// "Make an input of the function", "Make an output of the function": `signature.add` and `wire.connect` in one step,
/// the terminal named after the terminal of the node. None when it cannot be made, or its type is not known.
pub(crate) fn make(drawing: &Drawing, at: usize) -> Option<Intent> {
    let (input, ty) = makeable(drawing, at)?;
    let ty = ty?;
    let port = drawing.ports.get(at)?;
    let base = if port.caption.is_empty() { if input { "input" } else { "output" } } else { port.caption.as_str() };
    // A variadic value or a field of a template is named `parts.2`: its name is what follows the last dot.
    let base = base.rsplit('.').next().filter(|base| !base.is_empty() && !base.chars().all(|c| c.is_ascii_digit())).unwrap_or("value");
    let mut taken = shown_names(drawing);
    // The value of an output is the one the output takes: its own name is free for it.
    if !input {
        taken.remove(&port.value);
    }
    let name = fresh(base, &taken);
    Some(if input {
        Intent::MakeTerminal { terminal: format!("input {name}"), ty, from: name, to: port.name.clone() }
    } else {
        Intent::MakeTerminal { terminal: format!("output {name}"), ty, from: port.name.clone(), to: format!("output {name}") }
    })
}

/// The title of the frame of the function (`fn settle`, notation 2): the name of the function, which a field renames,
/// and its box, in units. None when the drawing cuts the name (`…`): the field would write it cut.
fn title_box(drawing: &Drawing) -> Option<(String, egui::Rect)> {
    drawing.loose.iter().find_map(|shape| {
        let Shape::Text { x, y, size, width, runs, .. } = shape else { return None };
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        let name = own_name(text.strip_prefix("fn ")?);
        if name.is_empty() || name.ends_with('…') {
            return None;
        }
        let size = *size as f32 / 100.0;
        let (left, baseline) = (*x as f32 / 100.0, *y as f32 / 100.0);
        Some((
            name.to_owned(),
            egui::Rect::from_min_max(Pos2::new(left, baseline - size), Pos2::new(left + *width as f32 / 100.0, baseline + size * 0.3)),
        ))
    })
}

/// The title of the frame of the function under `unit`: the name of the function, and where its field opens.
pub(crate) fn title_under(drawing: &Drawing, unit: Pos2) -> Option<(String, Pos2)> {
    title_box(drawing).filter(|(_, rect)| rect.expand(2.0).contains(unit)).map(|(name, rect)| (name, rect.min))
}

/// The name of the function on the title of its frame, and where its field opens: F2 with nothing selected.
pub(crate) fn title(drawing: &Drawing) -> Option<(String, Pos2)> {
    title_box(drawing).map(|(name, rect)| (name, rect.min))
}

/// A text of the terminals of the frame of the function under `unit`: the name of a terminal, or the pill of the default
/// of an input. The field it opens, its text, and where.
pub(crate) fn text_under(drawing: &Drawing, unit: Pos2) -> Option<(FieldKind, String, Pos2)> {
    for group in drawing.groups.iter().filter(|group| group.terminals.is_some() && group.placed.is_none()) {
        for shape in &group.shapes {
            let Shape::Text { x, y, size, width, runs, .. } = shape else { continue };
            let text: String = runs.iter().map(|run| run.text.as_str()).collect();
            let size = *size as f32 / 100.0;
            let (left, baseline) = (*x as f32 / 100.0, *y as f32 / 100.0);
            let rect = egui::Rect::from_min_max(Pos2::new(left, baseline - size), Pos2::new(left + *width as f32 / 100.0, baseline + size * 0.3));
            if !rect.expand(2.0).contains(unit) {
                continue;
            }
            // The terminal of that line: the nearest of the frame of the function.
            // An input and an output may share a row: the nearest on both axes.
            let point = |port: &Port| Pos2::new(port.x as f32, port.y as f32) / 100.0;
            let port = drawing
                .ports
                .iter()
                .filter(|port| frame_terminal(port).is_some())
                .min_by(|a, b| rect.distance_to_pos(point(a)).total_cmp(&rect.distance_to_pos(point(b))))?;
            if (port.y as f32 / 100.0 - rect.center().y).abs() > 12.0 {
                continue;
            }
            let terminal = frame_terminal(port)?;
            if text == port.caption {
                return Some((FieldKind::TerminalName { terminal }, text, rect.min));
            }
            if let Some(default) = text.strip_prefix(DEFAULT_MARK).filter(|default| !port.input && !default_is_cut(default)) {
                return Some((FieldKind::TerminalDefault { terminal }, default.to_owned(), rect.min));
            }
        }
    }
    None
}

/// The default of an input of the function as its pill shows it, the mark before it taken off; empty when it has none.
pub(crate) fn default_at(drawing: &Drawing, port: usize) -> String {
    let Some(found) = drawing.ports.get(port) else { return String::new() };
    let line = found.y as f32 / 100.0;
    for group in drawing.groups.iter().filter(|group| group.terminals.is_some() && group.placed.is_none()) {
        for shape in &group.shapes {
            if let Shape::Text { y, runs, .. } = shape {
                let text: String = runs.iter().map(|run| run.text.as_str()).collect();
                if let Some(default) = text.strip_prefix(DEFAULT_MARK)
                    && (*y as f32 / 100.0 - line).abs() <= 8.0
                {
                    return default.to_owned();
                }
            }
        }
    }
    String::new()
}

/// The default of the input of the function `terminal` (`input low`) as the drawing shows it; empty when it has none.
pub fn frame_default(drawing: &Drawing, terminal: &str) -> String {
    let port = drawing.ports.iter().position(|port| frame_terminal(port).as_deref() == Some(terminal) && !port.input);
    port.map(|port| default_at(drawing, port)).unwrap_or_default()
}

/// The field of a terminal of the frame, from its menu: where it opens, by its terminal.
pub(crate) fn field_by(drawing: &Drawing, at: usize) -> Pos2 {
    drawing.ports.get(at).map_or(Pos2::ZERO, |port| Pos2::new(port.x as f32, port.y as f32) / 100.0 + Vec2::new(8.0, -8.0))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::fresh;

    /// N-14, written by hand: `gain` gives `gain2`, `e2` gives `e2_2`.
    #[test]
    fn a_name_taken_is_numbered() {
        let taken: BTreeSet<String> = ["gain", "gain2", "e2"].iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(fresh("gain", &taken), "gain3");
        assert_eq!(fresh("e2", &taken), "e2_2");
        assert_eq!(fresh("free", &taken), "free");
    }
}
