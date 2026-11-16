//! What is planned is visible (ide.md 1 and 6): an entry of a milestone to come shows where it will be, greyed, marked
//! Planned, and can never be chosen; a click opens a short card that says what it will be and links to its milestone
//! or its issue, in the browser. No date is promised.

use egui::RichText;

/// An entry to come, as data: the IDE ships the list (ide.md 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub label: String,
    /// One line on what it will be.
    pub line: String,
    /// The milestone that brings it: `M3`.
    pub milestone: String,
    /// Its milestone or its issue on the web.
    pub link: Option<String>,
}

/// The card of a planned entry, open or not.
#[derive(Debug, Default)]
pub struct Card {
    shown: Option<Planned>,
}

impl Card {
    pub fn open(&mut self, planned: Planned) {
        self.shown = Some(planned);
    }

    pub fn is_open(&self) -> bool {
        self.shown.is_some()
    }

    /// Shows the card when one is open; the link to open in the browser, when its button was clicked.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<String> {
        let planned = self.shown.as_ref()?;
        let mut follow = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("kit-shell-planned")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&planned.label).strong());
                ui.label(RichText::new(format!("Planned, {}", planned.milestone)).weak().italics());
            });
            ui.label(&planned.line);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if let Some(link) = &planned.link
                    && ui.button(format!("See {} on the web", planned.milestone)).on_hover_text(link.as_str()).clicked()
                {
                    follow = Some(link.clone());
                }
                close = ui.button("Close").clicked();
            });
        });
        if close || follow.is_some() || modal.should_close() {
            self.shown = None;
        }
        follow
    }
}
