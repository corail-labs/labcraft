//! The view `Block` of a tab (ide.md 3; decisions of the owner, 2026-11-20): the block of the function as its callers see
//! it, enlarged, where a drag moves a terminal or an edge; under it, the block at its real size in the light and the dark
//! theme, and the warnings of `check` on the block; beside it, what the commands of the block change (commands.md 9.4),
//! each value said to come from the mask of the library when the function does not write its own (spec 11.3). No command
//! of its own: each gesture sends one of `block.*`, one step of undo.

use std::sync::Arc;

use egui::{Color32, RichText};
use labcraft_diagram::{BlockFields, Drawing, MAX_PLACES, OutlineChoice, OwnBlock, style};
use labcraft_editor::{BlockIntent, BlockView, Theme, swatch};
use labcraft_types::{Diagnostic, Severity};
use serde_json::{Value, json};

/// The codes of `check` that speak of the block of a function (spec 11.1 to 11.4).
pub const BLOCK_CODES: [&str; 9] = ["LC5008", "LC5009", "LC5014", "LC5015", "LC5016", "LC5017", "LC5018", "LC5019", "LC0107"];

/// At most this many warnings under the block; the others are in `Problems`.
const MAX_SHOWN: usize = 8;

/// The width of the fields beside the block, and the least width of the view that holds both side by side.
const FIELDS_WIDTH: f32 = 250.0;
const SIDE_BY_SIDE: f32 = 560.0;

/// A colour of the block (spec 11.2), as its command names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Color,
    Fill,
    Stroke,
}

impl Role {
    const ALL: [Role; 3] = [Role::Color, Role::Fill, Role::Stroke];

    fn word(self) -> &'static str {
        match self {
            Role::Color => "color",
            Role::Fill => "fill",
            Role::Stroke => "stroke",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Role::Color => "Colour",
            Role::Fill => "Fill",
            Role::Stroke => "Stroke",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Role::Color => "block.set_color",
            Role::Fill => "block.set_fill",
            Role::Stroke => "block.set_stroke",
        }
    }

    fn of(self, fields: &BlockFields) -> Option<&String> {
        match self {
            Role::Color => fields.color.as_ref(),
            Role::Fill => fields.fill.as_ref(),
            Role::Stroke => fields.stroke.as_ref(),
        }
    }
}

/// A colour being chosen: written when its picker closes, so that a drag in the picker is one step of undo.
struct Picking {
    role: Role,
    colour: Color32,
    changed: bool,
}

/// What the view of a function keeps between two frames.
#[derive(Default)]
pub struct BlockPane {
    view: BlockView,
    picking: Option<Picking>,
    /// A size being typed or dragged in its field, sent when the field lets go.
    top: Option<u32>,
    left: Option<u32>,
}

/// What the view asks for: a command of the block, without its `file`, which the host adds; or what to say.
pub enum Asked {
    Command(&'static str, Value),
    Say(String),
}

fn hex(colour: Color32) -> String {
    style::written(u32::from_be_bytes([0, colour.r(), colour.g(), colour.b()]))
}

/// The warnings of the block, of its file and of the manifest of its library.
pub fn diagnostics_of<'a>(all: &'a [Diagnostic], file: &str, manifest: &str) -> Vec<&'a Diagnostic> {
    all.iter()
        .filter(|diagnostic| BLOCK_CODES.contains(&diagnostic.code().as_str()))
        .filter(|diagnostic| diagnostic.file == file || diagnostic.file == manifest)
        .collect()
}

impl BlockPane {
    /// The view, in the room of `ui`: the block of `drawing` ([`Drawing::block`]) and its sizes, its colours and its
    /// warnings.
    pub fn show(&mut self, ui: &mut egui::Ui, drawing: &Arc<Drawing>, theme: Theme, diagnostics: &[&Diagnostic]) -> Vec<Asked> {
        let mut asked = Vec::new();
        let Some(own) = drawing.block.as_deref() else { return asked };
        if ui.available_width() >= SIDE_BY_SIDE {
            ui.horizontal_top(|ui| {
                let height = ui.available_height();
                let width = ui.available_width() - FIELDS_WIDTH - 16.0;
                ui.allocate_ui(egui::vec2(width, height), |ui| {
                    ui.vertical(|ui| self.preview(ui, drawing, own, theme, diagnostics, &mut asked));
                });
                ui.separator();
                ui.allocate_ui(egui::vec2(FIELDS_WIDTH, height), |ui| {
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical().id_salt("ide-block-fields").show(ui, |ui| self.fields(ui, own, &mut asked));
                    });
                });
            });
        } else {
            egui::ScrollArea::vertical().id_salt("ide-block-narrow").auto_shrink([false, false]).show(ui, |ui| {
                let room = (ui.available_height() * 0.5).max(160.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), room + 120.0), |ui| {
                    ui.vertical(|ui| self.preview(ui, drawing, own, theme, diagnostics, &mut asked));
                });
                ui.separator();
                self.fields(ui, own, &mut asked);
            });
        }
        asked
    }

    /// The block enlarged, then at its real size in both themes, then its warnings.
    fn preview(
        &mut self,
        ui: &mut egui::Ui,
        drawing: &Arc<Drawing>,
        own: &OwnBlock,
        theme: Theme,
        diagnostics: &[&Diagnostic],
        asked: &mut Vec<Asked>,
    ) {
        let real = own.drawing.height as f32 / 100.0 + 28.0;
        let warnings = (diagnostics.len().min(MAX_SHOWN) as f32 + 1.0) * 18.0;
        let room = (ui.available_height() - real - warnings).max(80.0);
        ui.allocate_ui(egui::vec2(ui.available_width(), room), |ui| {
            for intent in self.view.show(ui, drawing, theme, own.fixed.is_empty()) {
                asked.push(match intent {
                    BlockIntent::Terminal { terminal, side, place } => {
                        Asked::Command("block.set_terminal", json!({ "terminal": terminal, "side": side, "place": place }))
                    }
                    BlockIntent::Size { top, left } => {
                        let mut params = serde_json::Map::new();
                        if let Some(top) = top {
                            params.insert("top".to_owned(), json!(top));
                        }
                        if let Some(left) = left {
                            params.insert("left".to_owned(), json!(left));
                        }
                        Asked::Command("block.set_size", Value::Object(params))
                    }
                    BlockIntent::Say(text) => Asked::Say(text),
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Real size").weak());
            for (theme, name) in [(Theme::Light, "Light"), (Theme::Dark, "Dark")] {
                ui.add_space(12.0);
                if let Some(response) = self.view.real_size(ui, drawing, theme) {
                    response.on_hover_text(format!("{name} theme"));
                }
            }
        });
        for diagnostic in diagnostics.iter().take(MAX_SHOWN) {
            let colour = if diagnostic.severity() == Severity::Error { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color };
            ui.label(
                RichText::new(format!("{} {}  {}", diagnostic.severity().as_str(), diagnostic.code().as_str(), diagnostic.message)).color(colour),
            );
        }
        if diagnostics.len() > MAX_SHOWN {
            ui.label(RichText::new(format!("{} more in Problems", diagnostics.len() - MAX_SHOWN)).weak());
        }
    }

    /// What the commands of the block change, each value with where it comes from.
    fn fields(&mut self, ui: &mut egui::Ui, own: &OwnBlock, asked: &mut Vec<Asked>) {
        let (written, mask) = (&own.written, &own.mask);
        let from_mask = if own.library.is_empty() { "from the mask of its library".to_owned() } else { format!("from the mask of {}", own.library) };
        if !own.fixed.is_empty() {
            ui.label(RichText::new(&own.fixed).color(ui.visuals().warn_fg_color));
            ui.add_space(6.0);
        }
        ui.add_enabled_ui(own.fixed.is_empty(), |ui| {
            egui::Grid::new("ide-block-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                self.outline_row(ui, written, mask, asked);
                ui.end_row();
                band_row(ui, own, asked);
                ui.end_row();
                if let Some(top) = size_row(ui, ("Top", &mut self.top), own.top, written.top, own.least.0.max(1)) {
                    asked.push(Asked::Command("block.set_size", json!({ "top": top })));
                }
                ui.end_row();
                if let Some(left) = size_row(ui, ("Left", &mut self.left), own.left, written.left, own.least.1) {
                    asked.push(Asked::Command("block.set_size", json!({ "left": left })));
                }
                ui.end_row();
                for role in Role::ALL {
                    self.colour_row(ui, role, (written, mask), asked);
                    ui.end_row();
                }
            });
            // Where an inherited value comes from, once for the fields that say "mask".
            let inherits = (written.band.is_none() && mask.band.is_some())
                || (written.outline.is_none() && mask.outline.is_some())
                || Role::ALL.iter().any(|role| role.of(written).is_none() && role.of(mask).is_some());
            if inherits {
                ui.label(RichText::new(format!("\u{2022} {from_mask}")).weak());
            }
            ui.add_space(8.0);
            if ui.button("Reset the block").on_hover_text("Removes the `terminals` and `outline` sections (block.reset)").clicked() {
                asked.push(Asked::Command("block.reset", json!({})));
            }
        });
    }

    fn outline_row(&mut self, ui: &mut egui::Ui, written: &BlockFields, mask: &BlockFields, asked: &mut Vec<Asked>) {
        ui.label("Outline");
        let follow = if mask.outline.is_some() { "\u{2022} Mask" } else { "Ordinary" };
        let shown = match &written.outline {
            None => follow.to_owned(),
            Some(OutlineChoice::Template(name)) if name == "d" => "D".to_owned(),
            Some(OutlineChoice::Template(name)) if name == "block" => "Ordinary".to_owned(),
            Some(OutlineChoice::Template(name)) => format!("template {name}"),
            Some(OutlineChoice::Path) => "Drawn in the text".to_owned(),
        };
        let mut chosen: Option<Option<&str>> = None;
        egui::ComboBox::from_id_salt("ide-block-outline").selected_text(shown).width(150.0).show_ui(ui, |ui| {
            if ui.selectable_label(written.outline.is_none(), follow).clicked() {
                chosen = Some(None);
            }
            // Under a mask with an outline, the ordinary block is written as `template block` (11.4).
            if mask.outline.is_some()
                && ui.selectable_label(written.outline == Some(OutlineChoice::Template("block".to_owned())), "Ordinary").clicked()
            {
                chosen = Some(Some("template block"));
            }
            if ui.selectable_label(written.outline == Some(OutlineChoice::Template("d".to_owned())), "D").clicked() {
                chosen = Some(Some("template d"));
            }
            if written.outline == Some(OutlineChoice::Path) {
                ui.add_enabled(false, egui::Button::selectable(true, "Drawn in the text"));
            }
        });
        if let Some(outline) = chosen {
            asked.push(Asked::Command("block.set_outline", json!({ "outline": outline })));
        }
    }

    /// A colour: a swatch of the one in force, which opens a picker; "None" removes the line.
    fn colour_row(&mut self, ui: &mut egui::Ui, role: Role, (written, mask): (&BlockFields, &BlockFields), asked: &mut Vec<Asked>) {
        ui.label(role.label());
        let own = role.of(written);
        let inherited = role.of(mask).filter(|_| own.is_none());
        let shown = own.or(inherited);
        let picking = self.picking.as_ref().filter(|picking| picking.role == role);
        let colour = picking.map(|picking| picking.colour).or_else(|| shown.and_then(|hex| swatch(hex)));
        let response = ui
            .horizontal(|ui| {
                let (area, response) = ui.allocate_exact_size(egui::vec2(36.0, 18.0), egui::Sense::click());
                let stroke = ui.visuals().widgets.inactive.fg_stroke;
                match colour {
                    Some(colour) => {
                        ui.painter().rect_filled(area, 3.0, colour);
                    }
                    None => {
                        ui.painter().line_segment([area.left_bottom(), area.right_top()], stroke);
                    }
                }
                ui.painter().rect_stroke(area, 3.0, stroke, egui::StrokeKind::Inside);
                let text = match (own, inherited) {
                    (Some(hex), _) => RichText::new(hex),
                    (None, Some(hex)) => RichText::new(format!("{hex} \u{2022}")).weak(),
                    (None, None) => RichText::new("none").weak(),
                };
                ui.label(text);
                response
            })
            .inner
            .on_hover_text(format!("`{}` of the `terminals` section (spec 11.2)", role.word()));
        let open = self.picking.as_ref().is_some_and(|picking| picking.role == role);
        if response.clicked() && !open {
            if let Some(before) = self.picking.take().filter(|before| before.changed) {
                asked.push(Asked::Command(before.role.command(), json!({ before.role.word(): hex(before.colour) })));
            }
            self.picking = Some(Picking { role, colour: colour.unwrap_or(Color32::from_rgb(0x64, 0x74, 0x8B)), changed: false });
        }
        let Some(picking) = self.picking.as_mut().filter(|picking| picking.role == role) else { return };
        let mut close = false;
        let mut remove = false;
        let area = egui::Area::new(ui.make_persistent_id(("ide-block-picker", role.word())))
            .order(egui::Order::Foreground)
            .fixed_pos(response.rect.left_bottom() + egui::vec2(0.0, 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if egui::color_picker::color_picker_color32(ui, &mut picking.colour, egui::color_picker::Alpha::Opaque) {
                        picking.changed = true;
                    }
                    ui.horizontal(|ui| {
                        ui.label(hex(picking.colour));
                        if ui.button("Done").clicked() {
                            close = true;
                        }
                        if own.is_some() && ui.button(if role.of(mask).is_some() { "Follow the mask" } else { "None" }).clicked() {
                            remove = true;
                        }
                    });
                });
            });
        let swatch_again = open && response.clicked();
        let clicked_elsewhere = ui.ctx().input(|input| {
            input.pointer.any_click()
                && input.pointer.interact_pos().is_some_and(|at| !area.response.rect.contains(at) && !response.rect.contains(at))
        });
        let escape = ui.ctx().input(|input| input.key_pressed(egui::Key::Escape));
        if remove {
            asked.push(Asked::Command(role.command(), json!({ role.word(): Value::Null })));
            self.picking = None;
        } else if close || swatch_again || clicked_elsewhere || escape {
            if picking.changed && !escape {
                asked.push(Asked::Command(role.command(), json!({ role.word(): hex(picking.colour) })));
            }
            self.picking = None;
        }
    }
}

/// The band: its place, or the mask's; none for the root library, whose functions have no band (11.2).
fn band_row(ui: &mut egui::Ui, own: &OwnBlock, asked: &mut Vec<Asked>) {
    ui.label("Band");
    let (written, mask) = (&own.written, &own.mask);
    let follow = match &mask.band {
        Some(place) => format!("\u{2022} {}", capital(place)),
        None => "Default (inside)".to_owned(),
    };
    let shown = written.band.as_deref().map_or_else(|| follow.clone(), capital);
    let mut chosen: Option<Option<&str>> = None;
    ui.add_enabled_ui(!own.library.is_empty(), |ui| {
        egui::ComboBox::from_id_salt("ide-block-band").selected_text(shown).width(150.0).show_ui(ui, |ui| {
            if ui.selectable_label(written.band.is_none(), &follow).clicked() {
                chosen = Some(None);
            }
            for place in ["hidden", "inside", "above"] {
                if ui.selectable_label(written.band.as_deref() == Some(place), capital(place)).clicked() {
                    chosen = Some(Some(place));
                }
            }
        })
    })
    .response
    .on_disabled_hover_text("A function of the root library, or of a library of primitives, has no band (spec 11.2)");
    if let Some(band) = chosen {
        asked.push(Asked::Command("block.set_band", json!({ "band": band })));
    }
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// A size: its value as drawn, written or not, sent when its field lets go; "Auto" removes it.
fn size_row(ui: &mut egui::Ui, (label, editing): (&str, &mut Option<u32>), shown: u32, written: Option<u32>, least: u32) -> Option<Option<u32>> {
    ui.label(label);
    let mut sent = None;
    ui.horizontal(|ui| {
        let mut value = editing.unwrap_or(shown);
        let response = ui.add(egui::DragValue::new(&mut value).range(least..=MAX_PLACES).suffix(" places"));
        if response.changed() {
            *editing = Some(value);
        }
        let let_go = response.drag_stopped() || response.lost_focus() || (response.changed() && !response.dragged() && !response.has_focus());
        if let_go && let Some(value) = editing.take().filter(|value| *value != shown || written.is_none()) {
            sent = Some(Some(value));
        }
        match written {
            Some(_) => {
                if ui.small_button("Auto").on_hover_text("Removes the size: the block takes the size of notation 3.2").clicked() {
                    sent = Some(None);
                }
            }
            None => {
                ui.label(RichText::new("auto").weak());
            }
        }
    });
    sent
}
