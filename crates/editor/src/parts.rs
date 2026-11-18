//! The parts of a structure in its context menu (notation 2.6, "The parts of a structure"): its arms, its carries, its
//! clauses, its `collect` lines and its options. The menu says what the structure has; the commands judge it again.

use egui::{Pos2, Ui};
use labcraft_diagram::Drawing;

use crate::fields::FieldKind;

/// What the context menu of a structure offers, read from the drawing.
pub(crate) struct PartsMenu {
    /// The structure, as a command names it.
    structure: String,
    shape: Shape,
    /// The part that the terminal right-clicked stands for (`carry x`, `in x`, `collect y`).
    part: Option<String>,
    /// Whether the body reads the value of that terminal: a carry that it reads becomes no `collect` or `last`.
    read_inside: bool,
    /// Whether that terminal carries an `error`: a carry of one stops a `loop` on the first error (spec 6.3).
    error: bool,
}

enum Shape {
    /// A `match` or a `handle`: the patterns of the arm right-clicked or shown, and how many arms it has.
    Arms { arm: Option<String>, count: usize },
    /// A `for` or a `loop`.
    Loop { each: bool, parallel: bool, continue_on_error: bool, carries: bool },
}

/// What the menu chose.
pub(crate) enum Chosen {
    /// A field in place to open, with its text.
    Field(FieldKind, String),
    Intent(crate::Intent),
}

impl PartsMenu {
    /// The menu of the structure `placed`; `arm`, the rank of the arm whose tab was right-clicked; `port`, the terminal
    /// right-clicked. None for a node that is no structure, or that no command names.
    pub fn of(drawing: &Drawing, placed: usize, arm: Option<usize>, port: Option<usize>, shown: &dyn Fn(usize) -> bool) -> Option<PartsMenu> {
        let node = drawing.placed.get(placed)?;
        let first = drawing.bodies.get(*node.holds.first()?)?;
        if node.name.is_empty() {
            return None;
        }
        let terminal = port.and_then(|at| drawing.ports.get(at)).filter(|found| found.placed == Some(placed) && !found.part.is_empty());
        let part = terminal.map(|found| found.part.clone());
        let read_inside = terminal.is_some_and(|found| found.read);
        let error = terminal
            .and_then(|found| found.ty)
            .and_then(|ty| drawing.types.get(ty))
            .is_some_and(|text| text.trim_start_matches('[').trim_end_matches(']') == "error");
        let shape = if first.looping {
            let carries = node.holds.iter().any(|&body| drawing.ports.iter().any(|port| port.body == body && port.part.starts_with("carry ")));
            Shape::Loop { each: first.each, parallel: first.parallel, continue_on_error: first.continue_on_error, carries }
        } else {
            // The arm right-clicked, else the one shown.
            let rank = arm.or_else(|| {
                drawing.arms.iter().enumerate().find(|(index, found)| found.structure == node.key && shown(*index)).map(|(_, found)| found.arm)
            });
            let arm = rank.and_then(|rank| node.holds.get(rank)).and_then(|&body| drawing.bodies.get(body)).map(|body| body.arm.clone());
            Shape::Arms { arm: arm.filter(|arm| !arm.is_empty()), count: node.holds.len() }
        };
        Some(PartsMenu { structure: node.name.clone(), shape, part, read_inside, error })
    }

    /// Its entries, after those of any node; what one chose.
    pub fn ui(&self, ui: &mut Ui) -> Option<Chosen> {
        let structure = self.structure.clone();
        let mut chosen = None;
        ui.separator();
        match &self.shape {
            Shape::Arms { arm, count } => {
                if ui.button("Add arm…").clicked() {
                    chosen = Some(Chosen::Field(FieldKind::AddArm { structure: structure.clone() }, String::new()));
                }
                if let Some(arm) = arm {
                    if ui.button(format!("Edit patterns of `{}`…", shown_arm(arm))).clicked() {
                        chosen = Some(Chosen::Field(FieldKind::Patterns { structure: structure.clone(), arm: arm.clone() }, arm.clone()));
                    }
                    if *count > 1 && ui.button(format!("Remove arm `{}`", shown_arm(arm))).clicked() {
                        chosen = Some(Chosen::Intent(crate::Intent::RemovePart { structure: structure.clone(), part: arm.clone() }));
                    }
                }
            }
            Shape::Loop { each, parallel, continue_on_error, carries } => {
                if !parallel && ui.button("Add carry…").clicked() {
                    chosen = Some(Chosen::Field(FieldKind::AddCarry { structure: structure.clone() }, "acc = 0".to_owned()));
                }
                if *each && ui.button("Add clause…").clicked() {
                    chosen = Some(Chosen::Intent(crate::Intent::AddClause { structure: structure.clone(), over: None }));
                }
                if let Some(part) = &self.part
                    && ui.button(format!("Remove {}", part_label(part))).clicked()
                {
                    chosen = Some(Chosen::Intent(crate::Intent::RemovePart { structure: structure.clone(), part: part.clone() }));
                }
                if let Some(part) = &self.part
                    && let Some(to) = passage_menu(ui, part, *parallel, self.read_inside, self.error && !each && !continue_on_error)
                {
                    chosen = Some(Chosen::Intent(crate::Intent::SetPassage { structure: structure.clone(), part: part.clone(), to }));
                }
                if *each {
                    let mut on = *parallel;
                    let entry = ui.add_enabled(!carries || *parallel, egui::Checkbox::new(&mut on, "Parallel"));
                    let entry = entry.on_disabled_hover_text("A `for parallel` takes no carry (spec 6.2): remove its carries first");
                    if entry.clicked() {
                        chosen = Some(Chosen::Intent(crate::Intent::SetOption { structure: structure.clone(), option: "parallel", on }));
                    }
                } else {
                    let mut on = *continue_on_error;
                    if ui.checkbox(&mut on, "Continue on error").clicked() {
                        chosen = Some(Chosen::Intent(crate::Intent::SetOption { structure: structure.clone(), option: "continue_on_error", on }));
                    }
                }
            }
        }
        if chosen.is_some() {
            ui.close();
        }
        chosen
    }
}

/// "Make it a carry", "Make it a collect", "Make it a last" for the terminal of a part of a loop (notation 2.6): the other
/// two kinds, none for an `in` clause. A `for parallel` takes no carry (spec 6.2); a carry that the body reads becomes
/// neither of the others, and the tooltip says why. `stops`: the part carries an `error` in a `loop` that stops on one
/// (spec 6.3), and the tooltip says what the change does to that. What one chose, as `structure.set_passage` takes `to`.
fn passage_menu(ui: &mut Ui, part: &str, parallel: bool, read_inside: bool, stops: bool) -> Option<&'static str> {
    let (kind, name) = part.split_once(' ').unwrap_or(("", ""));
    let mut chosen = None;
    for (to, enabled) in passages_offered(part, parallel, read_inside) {
        let entry = ui.add_enabled(enabled, egui::Button::new(format!("Make it a {to}")));
        let entry = entry.on_disabled_hover_text(format!("The body reads the carry `{name}`: a `{to}` has no value of the iteration before"));
        let entry = match passage_warning(kind, to, stops) {
            Some(warning) => entry.on_hover_text(warning),
            None => entry,
        };
        if entry.clicked() {
            chosen = Some(to);
        }
    }
    chosen
}

/// The entries of [`passage_menu`] for a part: each kind it can become, and whether its entry is enabled.
pub(crate) fn passages_offered(part: &str, parallel: bool, read_inside: bool) -> Vec<(&'static str, bool)> {
    let Some((kind, _)) = part.split_once(' ') else { return Vec::new() };
    if !matches!(kind, "carry" | "collect" | "last") {
        return Vec::new();
    }
    ["carry", "collect", "last"]
        .into_iter()
        .filter(|to| *to != kind && !(*to == "carry" && parallel))
        .map(|to| (to, !(kind == "carry" && read_inside)))
        .collect()
}

/// What changing a part of kind `kind` into `to` does to a `loop` that stops on the first error that a carry holds
/// (spec 6.3, decision of the owner, 2026-11-18): said, not refused.
pub(crate) fn passage_warning(kind: &str, to: &str, stops: bool) -> Option<&'static str> {
    match (stops, kind == "carry", to == "carry") {
        (true, true, false) => Some("This loop no longer stops on the first error: only a carry of an `error` stops it (spec 6.3)"),
        (true, false, true) => Some("This loop will stop on the first error that the carry holds (spec 6.3)"),
        _ => None,
    }
}

/// An arm as a tab writes it: `_` is `other` (notation 5.2).
fn shown_arm(patterns: &str) -> String {
    patterns.split(", ").map(|pattern| if pattern == "_" { "other" } else { pattern }).collect::<Vec<_>>().join(", ")
}

/// A part as its entry names it: `carry x`, `clause x`, `collect y`.
fn part_label(part: &str) -> String {
    match part.strip_prefix("in ") {
        Some(name) => format!("clause `{name}`"),
        None => match part.split_once(' ') {
            Some((kind, name)) => format!("{kind} `{name}`"),
            None => part.to_owned(),
        },
    }
}

/// Where the field of a part opens, in units: on the tab of its arm, else at the corner of the structure; for a carry, on
/// the left edge of its frame.
pub(crate) fn field_at(drawing: &Drawing, placed: usize, kind: &FieldKind) -> Pos2 {
    let node = drawing.placed.get(placed);
    let corner = node.map_or(Pos2::ZERO, |node| Pos2::new(node.x as f32, node.y as f32) / 100.0);
    match kind {
        FieldKind::AddCarry { .. } => node
            .and_then(|node| node.holds.first())
            .and_then(|&body| drawing.bodies.get(body))
            .map_or(corner, |body| Pos2::new(body.frame.0 as f32, (body.frame.1 + body.frame.3 / 2) as f32) / 100.0 - egui::Vec2::new(24.0, 0.0)),
        FieldKind::Patterns { arm, .. } => drawing
            .tabs
            .iter()
            .find(|tab| {
                tab.structure == placed
                    && drawing
                        .arms
                        .get(tab.arm)
                        .and_then(|found| node?.holds.get(found.arm))
                        .and_then(|&body| drawing.bodies.get(body))
                        .is_some_and(|body| &body.arm == arm)
            })
            .map_or(corner, |tab| Pos2::new(tab.x as f32, tab.y as f32) / 100.0),
        _ => corner - egui::Vec2::new(0.0, 14.0),
    }
}

/// A part asked for from the command palette, on the one structure selected (notation 2.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartAsked {
    AddArm,
    AddCarry,
    AddClause,
    ContinueOnError,
    Parallel,
}

impl crate::Canvas {
    /// A part of the one structure selected, as its context menu gives it: a field opens, or what it asks for comes
    /// with the next frame. Why not, for the status bar, when the selection is not a structure of that kind.
    pub fn ask_part(&mut self, shown: &crate::Shown, asked: PartAsked) -> Result<(), String> {
        let Some(drawing) = self.drawing.clone() else { return Err("no function is shown".to_owned()) };
        let found = self.index.find(&shown.selection);
        let nodes: Vec<usize> = found.nodes.iter().copied().collect();
        let (Some(&placed), 1) = (nodes.first(), nodes.len()) else { return Err("select one structure".to_owned()) };
        let Some(node) = drawing.placed.get(placed).filter(|node| !node.holds.is_empty() && !node.name.is_empty()) else {
            return Err("select one structure".to_owned());
        };
        let Some(first) = node.holds.first().and_then(|&body| drawing.bodies.get(body)) else { return Err("select one structure".to_owned()) };
        let structure = node.name.clone();
        let carries = node.holds.iter().any(|&body| drawing.ports.iter().any(|port| port.body == body && port.part.starts_with("carry ")));
        match asked {
            PartAsked::AddArm if !first.looping => {
                self.edit_part(placed, FieldKind::AddArm { structure }, String::new());
            }
            PartAsked::AddArm => return Err("a loop has one body: arms are for a `match` or a `handle`".to_owned()),
            PartAsked::AddCarry if first.parallel => return Err("a `for parallel` takes no carry (spec 6.2)".to_owned()),
            PartAsked::AddCarry if first.looping => {
                self.edit_part(placed, FieldKind::AddCarry { structure }, "acc = 0".to_owned());
            }
            PartAsked::AddClause if first.each => self.queued.push(crate::Intent::AddClause { structure, over: None }),
            PartAsked::ContinueOnError if first.looping && !first.each => {
                self.queued.push(crate::Intent::SetOption { structure, option: "continue_on_error", on: !first.continue_on_error });
            }
            PartAsked::Parallel if first.each && carries && !first.parallel => {
                return Err("a `for parallel` takes no carry (spec 6.2): remove its carries first".to_owned());
            }
            PartAsked::Parallel if first.each => {
                self.queued.push(crate::Intent::SetOption { structure, option: "parallel", on: !first.parallel });
            }
            PartAsked::AddCarry => return Err("a carry is for a `for` or a `loop`".to_owned()),
            PartAsked::AddClause | PartAsked::Parallel => return Err("an `in` clause and `parallel` are for a `for`".to_owned()),
            PartAsked::ContinueOnError => return Err("`continue_on_error` is for a `loop`".to_owned()),
        }
        Ok(())
    }
}

/// The `+` after the last tab of each `match` or `handle` shown in tabs (notation 2.6), in units: what a click on it
/// opens, "Add arm…". It stands right after the last tab, or the last part of the selector, as high as a tab.
pub(crate) fn plus_tabs(
    drawing: &Drawing,
    offset_of: &dyn Fn(usize) -> egui::Vec2,
    shown: &dyn Fn(&labcraft_diagram::Tab) -> bool,
) -> Vec<(usize, egui::Rect)> {
    let mut last: std::collections::BTreeMap<usize, egui::Rect> = std::collections::BTreeMap::new();
    for tab in drawing.tabs.iter().filter(|tab| shown(tab)) {
        let corner = Pos2::new(tab.x as f32, tab.y as f32) / 100.0 + offset_of(tab.structure);
        let area = egui::Rect::from_min_size(corner, egui::Vec2::new(tab.width as f32, tab.height as f32) / 100.0);
        let entry = last.entry(tab.structure).or_insert(area);
        if area.max.x > entry.max.x {
            *entry = area;
        }
    }
    last.into_iter()
        .filter(|(structure, _)| drawing.placed.get(*structure).is_some_and(|node| !node.name.is_empty()))
        .map(|(structure, area)| {
            let side = area.height();
            (structure, egui::Rect::from_min_size(Pos2::new(area.max.x + 4.0, area.min.y), egui::Vec2::splat(side)))
        })
        .collect()
}

/// Where a wire from `source` dropped at `at`, on no terminal, adds a part (notation 2.6): on the inner right edge of a
/// loop that holds the value, a `collect`, or a `last` with `alt`; on the left edge of a `for` that the value comes into from outside, a clause.
pub(crate) fn edge_drop(
    drawing: &Drawing,
    source: &labcraft_diagram::Port,
    at: Pos2,
    reach: f32,
    alt: bool,
    visible: &dyn Fn(Option<usize>) -> bool,
) -> Option<crate::Intent> {
    if source.input || source.name.is_empty() {
        return None;
    }
    let inside = |mut body: usize, outer: usize| {
        for _ in 0..=drawing.bodies.len() {
            if body == outer {
                return true;
            }
            let Some(next) = drawing.bodies.get(body).and_then(|found| found.owner).and_then(|owner| drawing.placed.get(owner)) else {
                return false;
            };
            body = next.body;
        }
        false
    };
    let mut best: Option<(f32, crate::Intent)> = None;
    for (index, body) in drawing.bodies.iter().enumerate() {
        let Some(owner) = body.owner.filter(|_| body.looping) else { continue };
        let Some(node) = drawing.placed.get(owner).filter(|node| !node.name.is_empty()) else { continue };
        let within = drawing.ports.iter().find(|port| port.placed == Some(owner)).and_then(|port| port.within);
        if !visible(within) {
            continue;
        }
        let (x, y, width, height) = body.frame;
        let (left, right) = (x as f32 / 100.0, (x + width) as f32 / 100.0);
        let (top, bottom) = (y as f32 / 100.0, (y + height) as f32 / 100.0);
        if at.y < top || at.y > bottom {
            continue;
        }
        // A `collect` reads a value of the loop body itself: one from an arm or an inner loop is taken out by a wire.
        let candidate = if (at.x - right).abs() <= reach && source.body == index {
            Some(((at.x - right).abs(), crate::Intent::AddCollect { structure: node.name.clone(), value: source.name.clone(), last: alt }))
        } else if body.each && (at.x - left).abs() <= reach && !inside(source.body, index) {
            Some(((at.x - left).abs(), crate::Intent::AddClause { structure: node.name.clone(), over: Some(source.name.clone()) }))
        } else {
            None
        };
        if let Some((distance, intent)) = candidate
            && best.as_ref().is_none_or(|(nearest, _)| distance < *nearest)
        {
            best = Some((distance, intent));
        }
    }
    best.map(|(_, intent)| intent)
}
