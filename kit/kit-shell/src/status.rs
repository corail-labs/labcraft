//! The status bar (ide.md 2): short segments on the left (the file shown, the last step), and on the right (the agent,
//! the colour by type, the zoom). A segment with a command runs it when clicked.

use egui::RichText;

/// A segment of the bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub hover: Option<String>,
    /// What a click runs, by its id; none, the segment only says.
    pub command: Option<String>,
}

impl Segment {
    pub fn new(text: impl Into<String>) -> Segment {
        Segment { text: text.into(), ..Segment::default() }
    }

    pub fn hover(mut self, hover: impl Into<String>) -> Segment {
        self.hover = Some(hover.into());
        self
    }

    pub fn command(mut self, id: impl Into<String>) -> Segment {
        self.command = Some(id.into());
        self
    }
}

/// Shows the bar in `ui`, a bottom panel; the command of the segment clicked.
pub fn show(ui: &mut egui::Ui, left: &[Segment], right: &[Segment]) -> Option<String> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        for (at, segment) in left.iter().enumerate() {
            if at > 0 {
                ui.separator();
            }
            clicked = clicked.take().or_else(|| one(ui, segment));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (at, segment) in right.iter().rev().enumerate() {
                if at > 0 {
                    ui.separator();
                }
                clicked = clicked.take().or_else(|| one(ui, segment));
            }
        });
    });
    clicked
}

fn one(ui: &mut egui::Ui, segment: &Segment) -> Option<String> {
    let text = RichText::new(&segment.text).small();
    let mut response = match &segment.command {
        Some(_) => ui.add(egui::Button::new(text).frame(false)),
        None => ui.label(text),
    };
    if let Some(hover) = &segment.hover {
        response = response.on_hover_text(hover.as_str());
    }
    if response.clicked() { segment.command.clone() } else { None }
}
