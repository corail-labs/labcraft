//! `kit-text-egui`: paints the text of `kit-text` in egui and takes its keys (`docs/ide.md`, sections 7 and 11). Only
//! the lines in view are laid out, each from the spans that [`Colours`] keeps, so that a file of megabytes scrolls and
//! types as a small one does. One cursor; the keys of a text field (notation 2.6): the field keeps them while it has the
//! focus, Tab included.
//!
//! It depends on no crate of LabCraft (`cargo xtask layers`).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use std::hash::Hash;

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{Color32, Event, EventFilter, FontId, Galley, ImeEvent, Key, Modifiers, Pos2, Rect, Response, Sense, Ui, Vec2};
use kit_text::{Action, Colours, Field, Highlighter, Motion, Selection};

#[cfg(test)]
mod tests;

/// How the field looks.
#[derive(Clone, Debug)]
pub struct Style {
    pub font: FontId,
    /// The colour of what no span covers.
    pub text: Color32,
    /// The colour of each class of the highlighter; a class past the end is plain.
    pub classes: Vec<Color32>,
    pub selection: Color32,
    pub cursor: Color32,
    /// Under the line of the cursor, while the field has the focus.
    pub current_line: Color32,
    /// The numbers of the lines.
    pub gutter: Color32,
}

/// What the host adds to the text for one frame.
#[derive(Clone, Debug, Default)]
pub struct Shown<'a> {
    /// The text cannot be changed: the cursor moves, and the text can be selected and copied.
    pub read_only: bool,
    /// Lines tinted under their text (from 0), with their colour.
    pub tints: &'a [(usize, Color32)],
    /// Show the cursor at this position, scrolled into view, once.
    pub reveal: Option<usize>,
    /// What the field is, for a screen reader.
    pub label: &'a str,
}

/// The field keeps Tab and the arrows while it has the focus: they move no focus away.
const KEYS: EventFilter = EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false };

/// What the field keeps from one frame to the next.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    /// The text is being selected with the mouse.
    dragging: bool,
    /// The context of the input method: changes each time the field takes the focus.
    ime_generation: u64,
    had_focus: bool,
}

/// The field, in the room left in `ui`, scrolled in both directions.
pub fn show<H: Highlighter>(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug,
    field: &mut Field,
    colours: &mut Colours<H>,
    style: &Style,
    shown: &Shown<'_>,
) -> Response {
    colours.splice(&field.take_splices());
    let row = ui.fonts_mut(|fonts| fonts.row_height(&style.font)).max(1.0);
    let glyph = ui.fonts_mut(|fonts| fonts.glyph_width(&style.font, '0')).max(1.0);
    let digits = field.len_lines().max(1).to_string().len().max(3);
    let gutter = glyph * (digits as f32 + 2.0);
    let size = Vec2::new(gutter + glyph * (field.widest() as f32 + 4.0), row * (field.len_lines() as f32 + 1.0));
    egui::ScrollArea::both()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let width = size.x.max(ui.available_width());
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, size.y.max(ui.available_height())), Sense::click_and_drag());
            let id = response.id;
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, ui.is_enabled(), shown.label));
            let mut state: State = ui.data(|data| data.get_temp(id)).unwrap_or_default();
            let text_left = rect.min.x + gutter;
            let layout = Layout { top: rect.min.y, left: text_left, row };
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            // The mouse: a press places the cursor (Shift extends), a drag selects, a double click takes a word, a triple
            // click a line.
            if let Some(pointer) = response.interact_pointer_pos() {
                let at = position_at(ui, field, colours, style, &layout, pointer);
                if response.triple_clicked() {
                    let (line, _) = field.line_column(at);
                    let start = field.position(line, 0);
                    let end = if line + 1 < field.len_lines() { field.position(line + 1, 0) } else { field.position(line, usize::MAX) };
                    field.apply(Action::Select(Selection { anchor: start, head: end }));
                } else if response.double_clicked() {
                    field.apply(Action::Select(field.word_at(at)));
                } else if response.drag_started() || response.is_pointer_button_down_on() && !state.dragging {
                    let extend = ui.input(|input| input.modifiers.shift);
                    let anchor = if extend { field.selection().anchor } else { at };
                    field.apply(Action::Select(Selection { anchor, head: at }));
                    state.dragging = true;
                } else if state.dragging && response.dragged() {
                    let anchor = field.selection().anchor;
                    field.apply(Action::Select(Selection { anchor, head: at }));
                }
                response.request_focus();
            }
            if !ui.input(|input| input.pointer.any_down()) {
                state.dragging = false;
            }
            let focused = response.has_focus();
            if focused && !state.had_focus {
                state.ime_generation += 1;
            }
            state.had_focus = focused;
            let mut moved = false;
            if focused {
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(id, KEYS);
                });
                let page = ((viewport.height() / row).floor() as usize).saturating_sub(1).max(1);
                let events = ui.input(|input| input.filtered_events(&KEYS));
                for event in events {
                    let Some(action) = action_of(&event, page) else { continue };
                    if shown.read_only && changes(&action) {
                        continue;
                    }
                    moved = true;
                    if let Some(text) = field.apply(action) {
                        ui.ctx().copy_text(text);
                    }
                }
            }
            colours.splice(&field.take_splices());
            // The lines in view, and one more above and below.
            let first = ((viewport.min.y / row).floor().max(0.0) as usize).saturating_sub(1);
            let last = ((viewport.max.y / row).ceil().max(0.0) as usize + 1).min(field.len_lines());
            let painter = ui.painter_at(rect.intersect(ui.clip_rect()));
            let selection = field.selection();
            let range = selection.range();
            let (cursor_line, _) = field.line_column(selection.head);
            let mut cursor_rect = None;
            for line in first..last {
                let top = layout.top + row * line as f32;
                let band = Rect::from_min_max(Pos2::new(rect.min.x, top), Pos2::new(rect.max.x, top + row));
                for (tinted, colour) in shown.tints {
                    if *tinted == line {
                        painter.rect_filled(band, 0.0, *colour);
                    }
                }
                if focused && line == cursor_line && selection.is_empty() {
                    painter.rect_filled(band, 0.0, style.current_line);
                }
                let galley = galley_of(ui, field, colours, style, line);
                let start = field.position(line, 0);
                let len = field.line_len(line);
                // The selection on this line, its line ending as a space.
                if !range.is_empty() && range.start <= start + len && range.end > start {
                    let from = range.start.saturating_sub(start).min(len);
                    let to = range.end.saturating_sub(start);
                    let x0 = layout.left + galley.pos_from_cursor(CCursor::new(from)).min.x;
                    let x1 =
                        if to > len { layout.left + galley.size().x + glyph } else { layout.left + galley.pos_from_cursor(CCursor::new(to)).min.x };
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, top + row)), 0.0, style.selection);
                }
                painter.galley(Pos2::new(layout.left, top), galley.clone(), style.text);
                let number = (line + 1).to_string();
                painter.text(Pos2::new(text_left - glyph, top), egui::Align2::RIGHT_TOP, number, style.font.clone(), style.gutter);
                if line == cursor_line {
                    let column = selection.head - start;
                    let x = layout.left + galley.pos_from_cursor(CCursor::new(column)).min.x;
                    cursor_rect = Some(Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + 1.5, top + row)));
                }
            }
            if let (true, Some(cursor)) = (focused, cursor_rect) {
                painter.rect_filled(cursor, 0.0, style.cursor);
                let mut ime = egui::output::IMEOutput::new(id, state.ime_generation, egui::IMEPurpose::Normal, rect, cursor);
                ime.should_interrupt_composition = false;
                ui.output_mut(|output| output.ime = Some(ime));
            }
            // The size of the text was measured before the keys: the next frame lays out what they changed.
            if moved {
                ui.ctx().request_repaint();
            }
            // A cursor moved by a key, or shown by the host, comes into view.
            let reveal = shown.reveal.or(moved.then_some(selection.head));
            if let Some(at) = reveal {
                if shown.reveal.is_some() {
                    field.apply(Action::Select(Selection::cursor(at)));
                }
                let (line, column) = field.line_column(at);
                let galley = galley_of(ui, field, colours, style, line);
                let x = layout.left + galley.pos_from_cursor(CCursor::new(column)).min.x;
                let top = layout.top + row * line as f32;
                ui.scroll_to_rect(Rect::from_min_max(Pos2::new(x - gutter, top), Pos2::new(x + glyph * 2.0, top + row)), None);
            }
            ui.data_mut(|data| data.insert_temp(id, state));
            response
        })
        .inner
}

/// Where the lines are painted.
struct Layout {
    top: f32,
    left: f32,
    row: f32,
}

/// A line laid out in its colours: egui keeps the galley of a job it laid out already, so a line that did not change
/// costs no layout.
fn galley_of<H: Highlighter>(ui: &Ui, field: &Field, colours: &mut Colours<H>, style: &Style, line: usize) -> std::sync::Arc<Galley> {
    let text = field.line(line);
    let spans = colours.spans(line, field.len_lines(), |index| field.line(index)).to_vec();
    let mut job = LayoutJob::default();
    let plain = TextFormat::simple(style.font.clone(), style.text);
    let mut at = 0;
    for span in spans {
        if span.range.start < at || span.range.end < span.range.start {
            continue;
        }
        if let Some(part) = text.get(at..span.range.start) {
            job.append(part, 0.0, plain.clone());
        }
        let colour = style.classes.get(usize::from(span.class)).copied().unwrap_or(style.text);
        match text.get(span.range.clone()) {
            Some(part) => job.append(part, 0.0, TextFormat::simple(style.font.clone(), colour)),
            None => break,
        }
        at = span.range.end;
    }
    if let Some(rest) = text.get(at..) {
        job.append(rest, 0.0, plain);
    }
    job.wrap.max_width = f32::INFINITY;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// The position of the text under a point: the nearest character boundary of the line under it.
fn position_at<H: Highlighter>(ui: &Ui, field: &Field, colours: &mut Colours<H>, style: &Style, layout: &Layout, point: Pos2) -> usize {
    let line = ((point.y - layout.top) / layout.row).floor().max(0.0) as usize;
    let line = line.min(field.len_lines().saturating_sub(1));
    let galley = galley_of(ui, field, colours, style, line);
    let column = galley.cursor_from_pos(Vec2::new(point.x - layout.left, layout.row * 0.5)).index.0;
    field.position(line, column)
}

/// Whether an action changes the text.
fn changes(action: &Action) -> bool {
    !matches!(action, Action::Move { .. } | Action::SelectAll | Action::Select(_) | Action::Copy)
}

/// The action of an event of egui, while the field has the focus.
fn action_of(event: &Event, page: usize) -> Option<Action> {
    match event {
        // A line ending or a tab in a text that the system gives at once, as the keys give them.
        Event::Text(text) if !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') => Some(Action::Insert(text.clone())),
        Event::Paste(text) => Some(Action::Insert(text.clone())),
        Event::Copy => Some(Action::Copy),
        Event::Cut => Some(Action::Cut),
        Event::Ime(ImeEvent::Commit(text)) if !text.is_empty() => Some(Action::Insert(text.clone())),
        Event::Key { key, pressed: true, modifiers, .. } => key_action(*key, *modifiers, page),
        _ => None,
    }
}

/// The keys of a text field (notation 2.6).
fn key_action(key: Key, modifiers: Modifiers, page: usize) -> Option<Action> {
    let extend = modifiers.shift;
    // By word: Ctrl, or Option on a Mac; Cmd and an arrow go to the end of the line.
    let word = if cfg!(target_os = "macos") { modifiers.alt } else { modifiers.ctrl };
    let line_end = cfg!(target_os = "macos") && modifiers.mac_cmd;
    let go = |motion| Some(Action::Move { motion, extend });
    match key {
        Key::ArrowLeft if line_end => go(Motion::Home),
        Key::ArrowRight if line_end => go(Motion::End),
        Key::ArrowUp if line_end => go(Motion::TextStart),
        Key::ArrowDown if line_end => go(Motion::TextEnd),
        Key::ArrowLeft => go(if word { Motion::WordLeft } else { Motion::Left }),
        Key::ArrowRight => go(if word { Motion::WordRight } else { Motion::Right }),
        Key::ArrowUp => go(Motion::Up),
        Key::ArrowDown => go(Motion::Down),
        Key::Home if modifiers.command => go(Motion::TextStart),
        Key::End if modifiers.command => go(Motion::TextEnd),
        Key::Home => go(Motion::Home),
        Key::End => go(Motion::End),
        Key::PageUp => go(Motion::PageUp(page)),
        Key::PageDown => go(Motion::PageDown(page)),
        Key::Enter if !modifiers.command => Some(Action::Newline),
        Key::Tab if modifiers.shift => Some(Action::Outdent),
        Key::Tab if !modifiers.command => Some(Action::Indent),
        Key::Backspace if word => Some(Action::BackspaceWord),
        Key::Backspace => Some(Action::Backspace),
        Key::Delete if word => Some(Action::DeleteWord),
        Key::Delete => Some(Action::Delete),
        Key::A if modifiers.command => Some(Action::SelectAll),
        Key::Z if modifiers.command && modifiers.shift => Some(Action::Redo),
        Key::Z if modifiers.command => Some(Action::Undo),
        Key::Y if modifiers.command && !cfg!(target_os = "macos") => Some(Action::Redo),
        _ => None,
    }
}
