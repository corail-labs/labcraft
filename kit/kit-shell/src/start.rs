//! The start page (ide.md 10): what the IDE shows with no project open. Its actions (`New project`, `Open folder`, the
//! settings…), the recent projects, a project no longer there marked "not found", and the examples, each opened as a
//! copy. The page knows no command: the host gives the actions as palette entries, and runs what comes back.

use egui::RichText;

use crate::palette::{Entry, State};

/// A recent project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    /// Its name, shown first.
    pub name: String,
    /// Where it is, shown under it.
    pub path: String,
    /// Whether it is still there: none until the host has looked, off the thread of the window.
    pub found: Option<bool>,
}

/// An example shipped with the IDE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Example {
    pub name: String,
    pub line: String,
}

/// What the page shows.
pub struct Page<'a> {
    pub title: &'a str,
    pub line: &'a str,
    pub actions: &'a [Entry],
    /// The recent projects; none where projects are not kept, as on the web.
    pub recent: Option<&'a [Recent]>,
    pub examples: &'a [Example],
    /// How an example opens, said under them.
    pub examples_note: &'a str,
    /// A problem to say, such as a project that did not open.
    pub problem: Option<&'a str>,
}

/// What the user chose on the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Chosen {
    /// The action of this index.
    Action(usize),
    Recent(usize),
    Example(usize),
}

/// Shows the page in `ui`.
pub fn show(ui: &mut egui::Ui, page: &Page) -> Option<Chosen> {
    let mut chosen = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(32.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(760.0);
            ui.label(RichText::new(page.title).size(28.0).strong());
            ui.label(RichText::new(page.line).weak());
            if let Some(problem) = page.problem {
                ui.add_space(8.0);
                ui.label(RichText::new(problem).color(ui.visuals().warn_fg_color));
            }
            ui.add_space(24.0);
        });
        // The columns under the title, as wide as it at most, in the middle: never against the edges of the window.
        let width = ui.available_width().min(760.0);
        let margin = ((ui.available_width() - width) / 2.0).max(16.0);
        ui.horizontal_top(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width((width - 16.0).max(0.0));
                ui.columns(2, |columns| {
                    let [left, right] = columns else { return };
                    left.label(RichText::new("Start").strong());
                    for (index, action) in page.actions.iter().enumerate() {
                        let (text, hover) = match &action.state {
                            State::Enabled => (RichText::new(&action.label), None),
                            State::Disabled(why) => (RichText::new(&action.label).weak(), Some(why.clone())),
                            State::Planned(planned) => (RichText::new(format!("{}  ·  Planned, {}", action.label, planned.milestone)).weak(), None),
                        };
                        let mut response = left.link(text);
                        if let Some(key) = &action.key {
                            response = response.on_hover_text(key.as_str());
                        }
                        if let Some(hover) = hover {
                            response = response.on_hover_text(hover);
                        }
                        if response.clicked() && !matches!(action.state, State::Disabled(_)) {
                            chosen = Some(Chosen::Action(index));
                        }
                    }
                    left.add_space(16.0);
                    left.label(RichText::new("Examples").strong());
                    left.label(RichText::new(page.examples_note).small().weak());
                    for (index, example) in page.examples.iter().enumerate() {
                        if left.link(&example.name).on_hover_text(&example.line).clicked() {
                            chosen = Some(Chosen::Example(index));
                        }
                    }
                    let Some(recent) = page.recent else { return };
                    right.label(RichText::new("Recent").strong());
                    if recent.is_empty() {
                        right.label(RichText::new("No project opened yet").weak());
                    }
                    for (index, recent) in recent.iter().enumerate() {
                        right.horizontal(|ui| {
                            let gone = recent.found == Some(false);
                            let name = if gone { RichText::new(&recent.name).weak().strikethrough() } else { RichText::new(&recent.name) };
                            let response = ui.add_enabled(!gone, egui::Link::new(name)).on_hover_text(&recent.path);
                            if response.clicked() {
                                chosen = Some(Chosen::Recent(index));
                            }
                            if gone {
                                ui.label(RichText::new("not found").small().italics().weak());
                            }
                        });
                        right.label(RichText::new(&recent.path).small().weak());
                    }
                });
            });
        });
    });
    chosen
}
