//! The fields in place of the canvas (visual notation 2.6): a double-click on the name of a node, on a pill of a
//! constant, on a dot of a default or on a hole, F2, "Label…" of the context menu, and the template of a `string.format`
//! inserted. Enter, or a click elsewhere, writes what was typed (decision of the owner, 2026-11-17); Escape writes
//! nothing. A command that refuses leaves the field open, its text kept, a red border and the message under it.

use egui::{Pos2, Rect, Vec2};
use labcraft_diagram::{Drawing, Holds, Shape};

use crate::{Canvas, Frame, Input, Intent};

/// What a field writes, and the command that the host sends for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// `value.rename` of a value, or of a label (`#close`).
    Rename { value: String },
    /// `node.set_label` on the node that starts on `line`, which has no label yet.
    Label { line: u32 },
    /// `input.set_constant` on the input `to`, with `replace`; an empty text clears the constant `held`
    /// (`input.clear_constant`), and does nothing on a dot or a hole.
    Constant { to: String, held: bool },
    /// `node.set_template` on a `string.format`.
    Template { node: String },
    /// `structure.add_arm`: the patterns of a new arm (notation 2.6).
    AddArm { structure: String },
    /// `structure.set_patterns` of the arm `arm`, by its patterns.
    Patterns { structure: String, arm: String },
    /// `structure.add_carry`: `name = initial`, or `name: type = initial`.
    AddCarry { structure: String },
    /// `signature.rename` of a terminal of the frame of the function, `input x` or `output x`.
    TerminalName { terminal: String },
    /// `signature.set_default` of an input of the function; an empty text makes it required.
    TerminalDefault { terminal: String },
    /// `signature.add`: `name: type`, or `name: type = default` for an input; with no type, the host opens the list of
    /// types (decision of the owner, 2026-11-19).
    AddTerminal { input: bool },
    /// `function.rename` of the function shown, from the title of its frame (decision of the owner, 2026-11-19).
    FunctionName,
}

/// A field open on the canvas.
#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub kind: FieldKind,
    pub text: String,
    /// What it opened with: Enter on it unchanged writes nothing.
    original: String,
    /// Its top-left corner, in units, and the node it follows.
    at: Pos2,
    placed: Option<usize>,
    /// Why the last command refused, shown under it.
    error: Option<String>,
    /// Written, and the answer not back yet.
    pending: bool,
    focused: bool,
}

impl Field {
    fn new(kind: FieldKind, text: String, at: Pos2, placed: Option<usize>) -> Field {
        Field { kind, original: text.clone(), text, at, placed, error: None, pending: false, focused: false }
    }
}

/// A text of a node, where it is in units.
pub(crate) struct Written {
    pub rect: Rect,
    pub text: String,
}

/// The texts of the group of a node.
pub(crate) fn texts_of(drawing: &Drawing, placed: usize) -> Vec<Written> {
    let mut found = Vec::new();
    for group in drawing.groups.iter().filter(|group| group.placed == Some(placed) && group.is_node()) {
        for shape in &group.shapes {
            if let Shape::Text { x, y, size, width, runs, .. } = shape {
                let text: String = runs.iter().map(|run| run.text.as_str()).collect();
                let size = *size as f32 / 100.0;
                let (left, baseline) = (*x as f32 / 100.0, *y as f32 / 100.0);
                let rect = Rect::from_min_max(Pos2::new(left, baseline - size), Pos2::new(left + *width as f32 / 100.0, baseline + size * 0.3));
                found.push(Written { rect, text });
            }
        }
    }
    found
}

/// The values that a node gives, by its outputs.
fn values_of(drawing: &Drawing, placed: usize) -> Vec<String> {
    drawing
        .ports
        .iter()
        .filter(|port| port.placed == Some(placed) && !port.input && !port.frame && !port.value.is_empty() && port.value != "_")
        .map(|port| port.value.clone())
        .collect()
}

/// The text of a template as the field shows it: between its quotes, which the host writes back (decision of the owner,
/// 2026-11-17), so that what one types over it is the template.
pub fn unquoted(literal: &str) -> String {
    let inner = literal.strip_prefix('"').and_then(|rest| rest.strip_suffix('"'));
    inner.unwrap_or(literal).to_owned()
}

/// An input of a node under `unit` that a field edits, with the text it opens with and where (notation 2.6): a pill of
/// a constant, by the pill of that input (two pills may show the same text), else the dot of a default or a hole near
/// its terminal; the template of a `string.format`.
pub(crate) fn input_under(drawing: &Drawing, placed: usize, unit: Pos2, offset: Vec2) -> Option<(FieldKind, String, Pos2)> {
    let texts = texts_of(drawing, placed);
    let mut near: Option<(f32, FieldKind, String, Pos2)> = None;
    for port in drawing.ports.iter().filter(|port| port.placed == Some(placed) && port.input && !port.order && !port.name.is_empty()) {
        let point = Pos2::new(port.x as f32, port.y as f32) / 100.0 + offset;
        // The pill shows the constant with its escapes read, and cut; the field edits it as the file writes it
        // (`Port::literal`), and does not open on one too long for it.
        let (text, held, opens_with) = match &port.holds {
            Holds::Constant(_) if port.literal.is_empty() => continue,
            Holds::Constant(text) => (text.clone(), true, port.literal.clone()),
            Holds::Default(text) => (text.clone(), false, text.clone()),
            Holds::Hole => (String::new(), false, String::new()),
            Holds::Wire | Holds::Nothing | Holds::Missing => continue,
        };
        // The pill of this input: of the texts that read so, the one nearest its terminal.
        let pill = texts
            .iter()
            .filter(|written| held && written.text == text)
            .map(|written| written.rect.translate(offset))
            .min_by(|a, b| a.right_center().distance(point).total_cmp(&b.right_center().distance(point)));
        let distance = match pill {
            Some(rect) if rect.expand(2.0).contains(unit) => 0.0,
            _ => point.distance(unit),
        };
        if distance > 6.0 || near.as_ref().is_some_and(|(best, ..)| *best <= distance) {
            continue;
        }
        let at = pill.map_or(point - Vec2::new(24.0, 6.0), |rect| rect.min) - offset;
        let (kind, text) = if port.never.is_empty() {
            (FieldKind::Constant { to: port.name.clone(), held }, opens_with)
        } else {
            match drawing.placed.get(placed) {
                Some(found) if !found.name.is_empty() => (FieldKind::Template { node: found.name.clone() }, unquoted(&opens_with)),
                _ => continue,
            }
        };
        near = Some((distance, kind, text, at));
    }
    near.map(|(_, kind, text, at)| (kind, text, at))
}

/// The label of a node, `#close`, when it has one.
fn label_of(drawing: &Drawing, placed: usize) -> Option<String> {
    drawing.placed.get(placed).map(|found| found.name.clone()).filter(|name| name.starts_with('#'))
}

impl Canvas {
    /// Whether a field is open: the keys of the canvas leave it the keyboard.
    pub fn field_open(&self) -> bool {
        self.field.is_some()
    }

    /// The command of the field refused: it stays open, with the message (decision of the owner, 2026-11-17).
    pub fn field_refused(&mut self, message: &str) {
        if let Some(field) = &mut self.field {
            field.pending = false;
            field.focused = false;
            field.error = Some(message.to_owned());
        }
    }

    /// The command of the field is done.
    pub fn close_field(&mut self) {
        self.refocus |= self.field.is_some();
        self.field = None;
    }

    /// A field written whose answer has not come: no other opens before it, so that the answer is its own.
    fn waiting(&self) -> bool {
        self.field.as_ref().is_some_and(|field| field.pending)
    }

    /// F2 (notation 2.6): the name of the node selected, or of the one whose terminal is in focus; on a node with no
    /// name, its label (decision of the owner, 2026-11-17); the name of the one terminal of the frame selected (2.4).
    /// False when there is no one node or terminal.
    pub fn edit_name(&mut self) -> bool {
        if self.waiting() {
            return false;
        }
        let Some(drawing) = self.drawing.clone() else { return false };
        // One terminal of the frame of the function selected: its name (notation 2.4).
        if self.found.nodes.is_empty()
            && let [terminal] = self.found.terminals.iter().filter_map(|&at| drawing.hints.get(at)?.selects.clone()).collect::<Vec<_>>().as_slice()
            && let Some(port) = drawing.ports.iter().position(|port| crate::signature::frame_terminal(port).as_deref() == Some(terminal.as_str()))
        {
            let caption = drawing.ports.get(port).map(|port| port.caption.clone()).unwrap_or_default();
            let at = crate::signature::field_by(&drawing, port);
            self.field = Some(Field::new(FieldKind::TerminalName { terminal: terminal.clone() }, caption, at, None));
            return true;
        }
        // Nothing selected: the name of the function, on the title of its frame.
        if self.found.is_empty()
            && self.port_focus.is_none()
            && let Some((name, at)) = crate::signature::title(&drawing)
        {
            self.field = Some(Field::new(FieldKind::FunctionName, name, at, None));
            return true;
        }
        let focused = self.port_focus.and_then(|at| drawing.ports.get(at)).and_then(|port| port.placed);
        let placed = match (focused, self.found.nodes.len()) {
            (Some(placed), _) => placed,
            (None, 1) => match self.found.nodes.first() {
                Some(&placed) => placed,
                None => return false,
            },
            _ => return false,
        };
        let texts = texts_of(&drawing, placed);
        if let Some(value) = values_of(&drawing, placed).into_iter().next() {
            let at = texts.iter().find(|written| written.text == value).map(|written| written.rect.min);
            let at = at.unwrap_or_else(|| self.corner_of(&drawing, placed));
            self.field = Some(Field::new(FieldKind::Rename { value: value.clone() }, value, at, Some(placed)));
            return true;
        }
        self.edit_label(placed)
    }

    /// "Label…" (notation 2.6): the label of a node, to change, or a new one.
    pub(crate) fn edit_label(&mut self, placed: usize) -> bool {
        if self.waiting() {
            return false;
        }
        let Some(drawing) = self.drawing.clone() else { return false };
        let Some(found) = drawing.placed.get(placed) else { return false };
        let corner = self.corner_of(&drawing, placed);
        let above = corner - Vec2::new(0.0, 14.0);
        match label_of(&drawing, placed) {
            Some(label) => {
                let at = texts_of(&drawing, placed)
                    .into_iter()
                    .find(|written| written.text.trim_end_matches(':') == label)
                    .map_or(above, |written| written.rect.min);
                self.field = Some(Field::new(FieldKind::Rename { value: label.clone() }, label, at, Some(placed)));
            }
            None if found.line > 0 => {
                self.field = Some(Field::new(FieldKind::Label { line: found.line }, "#".to_owned(), above, Some(placed)));
            }
            None => return false,
        }
        true
    }

    /// A field of a terminal of the frame of the function (notation 2.4): to rename it, type it, give it a default, or a
    /// new one on an edge. A new one writes what Enter has, even the text it opened with.
    pub(crate) fn edit_terminal(&mut self, kind: FieldKind, text: String, at: Pos2) -> bool {
        if self.waiting() {
            return false;
        }
        let adds = matches!(kind, FieldKind::AddTerminal { .. });
        let mut field = Field::new(kind, text, at, None);
        if adds {
            field.original = String::new();
        }
        self.field = Some(field);
        true
    }

    /// A field of a part of the structure `placed` (notation 2.6): what Enter writes is new, even when it is the text it
    /// opened with.
    pub(crate) fn edit_part(&mut self, placed: usize, kind: FieldKind, text: String) -> bool {
        if self.waiting() {
            return false;
        }
        let Some(drawing) = self.drawing.clone() else { return false };
        let at = crate::parts::field_at(&drawing, placed, &kind);
        let adds = matches!(kind, FieldKind::AddArm { .. } | FieldKind::AddCarry { .. });
        let mut field = Field::new(kind, text, at, Some(placed));
        if adds {
            field.original = String::new();
        }
        self.field = Some(field);
        true
    }

    /// The template field of a `string.format` that quick insertion added, open as it arrives (notation 2.6). False
    /// while the drawing does not hold it yet.
    pub fn edit_template(&mut self, node: &str) -> bool {
        if self.waiting() {
            return false;
        }
        let Some(drawing) = self.drawing.clone() else { return false };
        let Some(placed) = drawing.placed.iter().position(|found| found.name == node) else { return false };
        let template = drawing.ports.iter().find(|port| port.placed == Some(placed) && port.input && !port.never.is_empty());
        let Some(port) = template else { return false };
        let (shown, literal) = match &port.holds {
            Holds::Constant(_) if port.literal.is_empty() => return false,
            Holds::Constant(shown) => (shown.clone(), port.literal.clone()),
            _ => (String::new(), "\"\"".to_owned()),
        };
        let at = self.pill_of(&drawing, placed, &shown).unwrap_or_else(|| self.corner_of(&drawing, placed));
        self.field = Some(Field::new(FieldKind::Template { node: node.to_owned() }, unquoted(&literal), at, Some(placed)));
        true
    }

    fn corner_of(&self, drawing: &Drawing, placed: usize) -> Pos2 {
        drawing.placed.get(placed).map_or(Pos2::ZERO, |found| Pos2::new(found.x as f32, found.y as f32) / 100.0)
    }

    /// Where the pill of a node that shows `text` is.
    fn pill_of(&self, drawing: &Drawing, placed: usize, text: &str) -> Option<Pos2> {
        texts_of(drawing, placed).into_iter().find(|written| written.text == text).map(|written| written.rect.min)
    }

    /// A double-click at `unit`: a field when it is on what one edits (notation 2.6). True when it opened one.
    pub(crate) fn field_at(&mut self, frame: &Frame, unit: Pos2) -> bool {
        if self.waiting() {
            return false;
        }
        let drawing = frame.drawing.clone();
        // The title of the frame of the function: its name.
        if frame.hovered.is_none()
            && let Some((name, at)) = crate::signature::title_under(&drawing, unit)
        {
            self.field = Some(Field::new(FieldKind::FunctionName, name, at, None));
            return true;
        }
        // A terminal of the frame of the function: its name, the pill of its default (notation 2.4).
        if frame.hovered.is_none()
            && let Some((kind, text, at)) = crate::signature::text_under(&drawing, unit)
        {
            self.field = Some(Field::new(kind, text, at, None));
            return true;
        }
        let Some(placed) = frame.hovered.or_else(|| {
            drawing.ports.iter().filter(|port| port.input && frame.visible(port.within)).find_map(|port| {
                let at = Pos2::new(port.x as f32, port.y as f32) / 100.0 + frame.offset_of(port.placed);
                (at.distance(unit) <= 8.0).then_some(port.placed).flatten()
            })
        }) else {
            return false;
        };
        let offset = frame.offset_of(Some(placed));
        let texts = texts_of(&drawing, placed);
        let under = |text: &str| texts.iter().find(|written| written.text == text && written.rect.translate(offset).expand(2.0).contains(unit));
        // An input: a pill of a constant, a dot of a default, a hole; the template of a `string.format`.
        if let Some((kind, text, at)) = input_under(&drawing, placed, unit, offset) {
            self.field = Some(Field::new(kind, text, at, Some(placed)));
            return true;
        }
        // The label of the node, and the name of a value it gives.
        if let Some(label) = label_of(&drawing, placed)
            && let Some(written) =
                texts.iter().find(|written| written.text.trim_end_matches(':') == label && written.rect.translate(offset).expand(2.0).contains(unit))
        {
            self.field = Some(Field::new(FieldKind::Rename { value: label.clone() }, label, written.rect.min, Some(placed)));
            return true;
        }
        for value in values_of(&drawing, placed) {
            if let Some(written) = under(&value) {
                self.field = Some(Field::new(FieldKind::Rename { value: value.clone() }, value, written.rect.min, Some(placed)));
                return true;
            }
        }
        false
    }

    /// The field, over the canvas: Enter or a click elsewhere writes, Escape closes.
    pub(crate) fn show_field(&mut self, frame: &Frame, input: &mut Input) {
        let Some(field) = &mut self.field else { return };
        let zoom = self.viewport.zoom;
        let origin = self.viewport.origin;
        let unit = field.at.to_vec2() + frame.offset_of(field.placed);
        let corner = frame.rect.min + (unit - origin) * zoom;
        let red = frame.theme.color("mark.invalid");
        let mut written = None;
        let mut cancelled = false;
        egui::Area::new(egui::Id::unique("labcraft-field-in-place")).order(egui::Order::Foreground).fixed_pos(corner).show(&frame.ctx, |ui| {
            let mut popup = egui::Frame::popup(ui.style());
            if field.error.is_some() {
                popup = popup.stroke(egui::Stroke::new(1.5, red));
            }
            popup.show(ui, |ui| {
                let width = (field.text.chars().count() as f32 * 8.0 + 24.0).clamp(64.0, 360.0);
                let edit = egui::TextEdit::singleline(&mut field.text).desired_width(width).interactive(!field.pending);
                let output = edit.show(ui);
                let response = output.response;
                if !field.focused {
                    response.request_focus();
                    // All of it selected: one types over it (decision of the owner, 2026-11-17).
                    let mut state = output.state;
                    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(field.text.chars().count()));
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), response.id);
                    field.focused = true;
                }
                if let Some(error) = &field.error {
                    ui.label(egui::RichText::new(error.as_str()).color(red).small());
                }
                if response.lost_focus() && !field.pending {
                    if ui.input(|keys| keys.key_pressed(egui::Key::Escape)) {
                        cancelled = true;
                    } else {
                        written = Some(field.text.trim().to_owned());
                    }
                }
            });
        });
        if cancelled {
            self.close_field();
            return;
        }
        let Some(text) = written else { return };
        let unchanged = text == field.original.trim();
        let nothing = text.is_empty() && matches!(field.kind, FieldKind::Constant { held: false, .. });
        if unchanged || nothing {
            self.close_field();
            return;
        }
        field.pending = true;
        field.error = None;
        input.intents.push(Intent::Field { kind: field.kind.clone(), text });
    }
}
