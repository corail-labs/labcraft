//! `kit-text-egui`: paints the text of `kit-text` in egui and takes its keys (`docs/ide.md`, sections 7 and 11). Only
//! the lines in view are laid out, each from the spans that [`Colours`] keeps, so that a file of megabytes scrolls and
//! types as a small one does. One cursor; the keys of a text field (notation 2.6): the field keeps them while it has the
//! focus, Tab included. The diagnostics of the language service are underlined with a wave, their messages and its
//! hover shown under the pointer at rest; its completion opens as a name is typed, or with Ctrl+Space, and Enter or Tab
//! takes the entry chosen.
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
use kit_text::lsp_types::{DiagnosticSeverity, Hover, HoverContents, MarkedString, NumberOrString};
use kit_text::{Action, Code, Colours, Completion, Field, Highlighter, LanguageService, Marks, Motion, Selection, position_of};

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
    /// The waves under the diagnostics: an error, a warning, and the others.
    pub error: Color32,
    pub warning: Color32,
    pub info: Color32,
}

impl Style {
    /// The colour of the wave of a diagnostic.
    pub fn mark(&self, severity: DiagnosticSeverity) -> Color32 {
        match severity {
            DiagnosticSeverity::WARNING => self.warning,
            DiagnosticSeverity::INFORMATION | DiagnosticSeverity::HINT => self.info,
            _ => self.error,
        }
    }
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
    /// An entry of the list was clicked: the field takes the keyboard back, which the click took.
    refocus: bool,
}

/// What the field says after a frame.
pub struct Output {
    pub response: Response,
    /// Ctrl+click (Cmd+click on a Mac) on the text, at this position: the host goes to the definition there.
    pub definition: Option<usize>,
}

/// The field, in the room left in `ui`, scrolled in both directions, with the diagnostics of `code` underlined, their
/// messages and the hover of the service under the pointer, and the list of completion.
pub fn show<H: Highlighter>(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug,
    code: &mut Code<H>,
    service: &mut dyn LanguageService,
    style: &Style,
    shown: &Shown<'_>,
) -> Output {
    code.take_published(service);
    let Code { field, colours, marks, completion } = code;
    colours.splice(&field.take_splices());
    let row = ui.fonts_mut(|fonts| fonts.row_height(&style.font)).max(1.0);
    let glyph = ui.fonts_mut(|fonts| fonts.glyph_width(&style.font, '0')).max(1.0);
    let digits = field.len_lines().max(1).to_string().len().max(3);
    let gutter = glyph * (digits as f32 + 2.0);
    let size = Vec2::new(gutter + glyph * (field.widest() as f32 + 4.0), row * (field.len_lines() as f32 + 1.0));
    let mut definition = None;
    // An id of its own, not the next one of `ui`: a banner that comes or goes above the field changes that one, and
    // the field would lose the focus and its state.
    let base = ui.make_persistent_id(id_salt);
    let id = base.with("field");
    let response = egui::ScrollArea::both()
        .id_salt(base.with("scroll"))
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let width = size.x.max(ui.available_width());
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, size.y.max(ui.available_height())), Sense::hover());
            let response = ui.interact(rect, id, Sense::click_and_drag());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, ui.is_enabled(), shown.label));
            let mut state: State = ui.data(|data| data.get_temp(id)).unwrap_or_default();
            let text_left = rect.min.x + gutter;
            let layout = Layout { top: rect.min.y, left: text_left, row };
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            // The mouse: a press places the cursor (Shift extends), a drag selects, a double click takes a word, a triple
            // click a line; Ctrl+click (Cmd on a Mac) goes to the definition.
            if let Some(pointer) = response.interact_pointer_pos() {
                let at = position_at(ui, field, colours, style, &layout, pointer);
                let command = ui.input(|input| input.modifiers.command);
                if response.clicked() && command {
                    field.apply(Action::Select(Selection::cursor(at)));
                    definition = Some(at);
                } else if response.triple_clicked() {
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
                completion.close();
                response.request_focus();
            }
            if !ui.input(|input| input.pointer.any_down()) {
                state.dragging = false;
            }
            if std::mem::take(&mut state.refocus) {
                response.request_focus();
            }
            let focused = response.has_focus();
            if focused && !state.had_focus {
                state.ime_generation += 1;
            }
            if !focused {
                completion.close();
            }
            state.had_focus = focused;
            let mut moved = false;
            if focused {
                // Escape closes the list of completion, and keeps the focus while it is open.
                let keys = EventFilter { escape: completion.is_open(), ..KEYS };
                ui.memory_mut(|memory| memory.set_focus_lock_filter(id, keys));
                let page = ((viewport.height() / row).floor() as usize).saturating_sub(1).max(1);
                let events = ui.input(|input| input.filtered_events(&keys));
                for event in events {
                    if completion.is_open()
                        && let Some(key) = list_key(&event)
                    {
                        match key {
                            ListKey::Take => {
                                completion.take(field);
                            }
                            ListKey::Close => completion.close(),
                            ListKey::Choose(by) => completion.choose(by),
                        }
                        moved = true;
                        continue;
                    }
                    // A field that cannot be changed lists nothing to write.
                    if is_ask(&event) && !shown.read_only {
                        completion.open(field, service, true);
                        moved = true;
                        continue;
                    }
                    let Some(action) = action_of(&event, page) else { continue };
                    if shown.read_only && changes(&action) {
                        continue;
                    }
                    moved = true;
                    let typed = matches!(&action, Action::Insert(text) if text.chars().last().is_some_and(kit_text::complete::is_name_char));
                    if let Some(text) = field.apply(action) {
                        ui.ctx().copy_text(text);
                    }
                    if completion.is_open() {
                        completion.follow(field);
                    } else if typed {
                        completion.open(field, service, false);
                    } else {
                        completion.close();
                    }
                }
            }
            colours.splice(&field.take_splices());
            marks.follow(field);
            // The lines in view, and one more above and below.
            let first = ((viewport.min.y / row).floor().max(0.0) as usize).saturating_sub(1);
            let last = ((viewport.max.y / row).ceil().max(0.0) as usize + 1).min(field.len_lines());
            let painter = ui.painter_at(rect.intersect(ui.clip_rect()));
            let selection = field.selection();
            let range = selection.range();
            let (cursor_line, _) = field.line_column(selection.head);
            let mut cursor_rect = None;
            // The marks on the lines in view, in the order of the text.
            let first_char = field.position(first, 0);
            let last_char = field.position(last, 0).max(field.position(last.saturating_sub(1), usize::MAX));
            let in_view: Vec<&kit_text::Mark> =
                marks.marks().iter().filter(|mark| mark.range.end >= first_char && mark.range.start <= last_char).collect();
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
                // The diagnostics on this line, a wave under their text; a point, or the end of a line, under one character.
                for mark in &in_view {
                    if mark.range.start > start + len || mark.range.end < start || mark.range.end == start && !mark.range.is_empty() {
                        continue;
                    }
                    let from = mark.range.start.saturating_sub(start).min(len);
                    let to = mark.range.end.saturating_sub(start).min(len);
                    let x0 = layout.left + galley.pos_from_cursor(CCursor::new(from)).min.x;
                    let x1 = if to > from { layout.left + galley.pos_from_cursor(CCursor::new(to)).min.x } else { x0 + glyph };
                    wave(&painter, x0, x1, top + row - 2.0, style.mark(mark.severity()));
                }
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
            hover(ui, &response, field, colours, marks, service, style, &layout);
            if let Some(taken) = popup(ui, id, field, colours, completion, style, &layout) {
                completion.set_chosen(taken);
                completion.take(field);
                state.refocus = true;
                moved = true;
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
        .inner;
    Output { response, definition }
}

/// The pointer rests this long, in seconds, before the hover shows *(proposed)*.
pub const HOVER_DELAY: f32 = 0.4;
/// The list of completion shows so many rows at most, and scrolls *(proposed)*.
const LIST_ROWS: usize = 10;

/// What the hover said last, kept while the pointer stays on the same character of the same text.
#[derive(Clone, Debug, Default)]
struct HoverCache {
    key: (usize, u64),
    said: Option<String>,
}

/// The messages of the diagnostics under the pointer, then what the service says there, once the pointer rests on a
/// character.
#[allow(clippy::too_many_arguments)]
fn hover<H: Highlighter>(
    ui: &Ui,
    response: &Response,
    field: &Field,
    colours: &mut Colours<H>,
    marks: &Marks,
    service: &mut dyn LanguageService,
    style: &Style,
    layout: &Layout,
) {
    let Some(pointer) = response.hover_pos() else { return };
    if ui.input(|input| input.pointer.any_down()) {
        return;
    }
    let still = ui.input(|input| input.pointer.time_since_last_movement());
    if still < HOVER_DELAY {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32(HOVER_DELAY - still));
        return;
    }
    let Some(at) = character_at(ui, field, colours, style, layout, pointer) else { return };
    let cache_id = response.id.with("hover");
    let key = (at, field.revision());
    let cached = match ui.data(|data| data.get_temp::<HoverCache>(cache_id)) {
        Some(cached) if cached.key == key => cached,
        _ => {
            let said = service.hover(field, position_of(field, at)).map(|hover| hover_text(&hover)).filter(|text| !text.is_empty());
            let cached = HoverCache { key, said };
            ui.data_mut(|data| data.insert_temp(cache_id, cached.clone()));
            cached
        }
    };
    let found = marks.at(at);
    if found.is_empty() && cached.said.is_none() {
        return;
    }
    egui::Tooltip::always_open(ui.ctx().clone(), response.layer_id, cache_id, egui::PopupAnchor::Position(pointer)).show(|ui| {
        for mark in &found {
            let code = mark.diagnostic.code.as_ref().map(|code| format!("[{}]", code_text(code))).unwrap_or_default();
            let severity = severity_word(mark.severity());
            ui.label(egui::RichText::new(format!("{severity}{code}: {}", mark.diagnostic.message)).color(style.mark(mark.severity())));
        }
        if let Some(said) = &cached.said {
            if !found.is_empty() {
                ui.separator();
            }
            ui.label(egui::RichText::new(said).font(style.font.clone()));
        }
    });
}

fn code_text(code: &NumberOrString) -> String {
    match code {
        NumberOrString::String(code) => code.clone(),
        NumberOrString::Number(code) => code.to_string(),
    }
}

fn severity_word(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::WARNING => "warning",
        DiagnosticSeverity::INFORMATION => "info",
        DiagnosticSeverity::HINT => "hint",
        _ => "error",
    }
}

/// The text of a hover of LSP, its parts one per paragraph.
pub fn hover_text(hover: &Hover) -> String {
    let marked = |marked: &MarkedString| match marked {
        MarkedString::String(text) => text.clone(),
        MarkedString::LanguageString(code) => code.value.clone(),
    };
    match &hover.contents {
        HoverContents::Scalar(one) => marked(one),
        HoverContents::Array(all) => all.iter().map(marked).collect::<Vec<_>>().join("\n\n"),
        HoverContents::Markup(markup) => markup.value.clone(),
    }
}

/// The list of completion, under the start of the name being completed. The entry clicked, by its place in the list.
fn popup<H: Highlighter>(
    ui: &Ui,
    id: egui::Id,
    field: &Field,
    colours: &mut Colours<H>,
    completion: &Completion,
    style: &Style,
    layout: &Layout,
) -> Option<usize> {
    let start = completion.start()?;
    let (entries, chosen) = completion.shown();
    if entries.is_empty() {
        return None;
    }
    let (line, column) = field.line_column(start);
    let galley = galley_of(ui, field, colours, style, line);
    let x = layout.left + galley.pos_from_cursor(CCursor::new(column)).min.x;
    let below = Pos2::new(x, layout.top + layout.row * (line + 1) as f32);
    let mut clicked = None;
    egui::Area::new(id.with("completion")).order(egui::Order::Foreground).fixed_pos(below).constrain(true).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_min_width(240.0);
            egui::ScrollArea::vertical().max_height(layout.row * LIST_ROWS as f32 + 8.0).show(ui, |ui| {
                for (index, entry) in entries.iter().enumerate() {
                    let mut text = egui::text::LayoutJob::default();
                    text.append(&entry.label, 0.0, TextFormat::simple(style.font.clone(), style.text));
                    if let Some(detail) = &entry.detail {
                        text.append(&format!("  {detail}"), 0.0, TextFormat::simple(style.font.clone(), style.gutter));
                    }
                    let row = ui.selectable_label(index == chosen, text);
                    if index == chosen {
                        row.scroll_to_me(None);
                    }
                    // Taken on the press: the field keeps the keyboard, and the list is still open.
                    if row.clicked() || row.hovered() && ui.input(|input| input.pointer.primary_pressed()) {
                        clicked = Some(index);
                    }
                }
            });
        });
    });
    clicked
}

/// A wavy line from `x0` to `x1` at `y`: a diagnostic underlined.
fn wave(painter: &egui::Painter, x0: f32, x1: f32, y: f32, colour: Color32) {
    let step = 2.0;
    // Only what shows: a diagnostic over a line of a megabyte is not drawn whole each frame.
    let clip = painter.clip_rect();
    let x1 = x1.min(clip.max.x + step);
    let skipped = ((clip.min.x - step - x0) / step).floor().max(0.0);
    let mut points = Vec::new();
    let mut x = x0 + skipped * step;
    let mut up = skipped as u64 % 2 == 1;
    if x >= x1 {
        return;
    }
    while x < x1 {
        points.push(Pos2::new(x, if up { y - 1.5 } else { y + 0.5 }));
        x += step;
        up = !up;
    }
    points.push(Pos2::new(x1, if up { y - 1.5 } else { y + 0.5 }));
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, colour)));
}

/// What a key does to the list of completion while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListKey {
    Take,
    Close,
    Choose(isize),
}

fn list_key(event: &Event) -> Option<ListKey> {
    let Event::Key { key, pressed: true, modifiers, .. } = event else { return None };
    if modifiers.command || modifiers.alt || modifiers.shift {
        return None;
    }
    Some(match key {
        Key::Enter | Key::Tab => ListKey::Take,
        Key::Escape => ListKey::Close,
        Key::ArrowDown => ListKey::Choose(1),
        Key::ArrowUp => ListKey::Choose(-1),
        Key::PageDown => ListKey::Choose(LIST_ROWS as isize),
        Key::PageUp => ListKey::Choose(-(LIST_ROWS as isize)),
        _ => return None,
    })
}

/// Ctrl+Space asks for the list of completion (ide.md 7), on a Mac too: Cmd+Space opens the search of the system there.
fn is_ask(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Space, pressed: true, modifiers, .. } if modifiers.ctrl && !modifiers.alt && !modifiers.shift)
}

/// The character under a point, when the point is on the text of a line: not past its end.
fn character_at<H: Highlighter>(ui: &Ui, field: &Field, colours: &mut Colours<H>, style: &Style, layout: &Layout, point: Pos2) -> Option<usize> {
    if point.x < layout.left || point.y < layout.top {
        return None;
    }
    let line = ((point.y - layout.top) / layout.row).floor() as usize;
    if line >= field.len_lines() {
        return None;
    }
    let galley = galley_of(ui, field, colours, style, line);
    if point.x - layout.left >= galley.size().x {
        return None;
    }
    let column = galley.cursor_from_pos(Vec2::new(point.x - layout.left, layout.row * 0.5)).index.0;
    // The cursor nearest a point falls between two characters: the one under the point is before it when the point is
    // left of that boundary.
    let boundary = layout.left + galley.pos_from_cursor(CCursor::new(column)).min.x;
    let column = if point.x < boundary { column.saturating_sub(1) } else { column };
    Some(field.position(line, column))
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
