//! What `Connect an AI agent` says (ide.md 8): the app writes the entry of the MCP server in the configuration of each
//! MCP client found on the computer, as `labcraft-cli mcp setup` does, and the window shows its lines, one for each
//! client, until the user closes it.

use egui::RichText;

/// The dialog.
#[derive(Debug, Default)]
pub enum Connect {
    #[default]
    Closed,
    /// The app is writing.
    Writing,
    /// What the app said, a line for each client.
    Done(Vec<String>),
}

impl Connect {
    pub fn is_open(&self) -> bool {
        !matches!(self, Connect::Closed)
    }

    /// The dialog, while it is open.
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.is_open() {
            return;
        }
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("ide-connect-agent")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new("Connect an AI agent").strong().size(16.0));
            ui.label(RichText::new("The MCP clients of this computer get the server `labcraft`; nothing else changes in their files.").weak());
            ui.label(
                RichText::new(
                    "Then open the same folder in the client and here: its agent works on the folder it starts in, in the window that shows it.",
                )
                .weak(),
            );
            ui.add_space(6.0);
            match self {
                Connect::Done(lines) => {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for line in lines.iter() {
                            ui.label(line);
                        }
                    });
                }
                _ => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Writing…");
                    });
                }
            }
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button("Close").clicked();
            });
        });
        if close || modal.should_close() {
            *self = Connect::Closed;
        }
    }
}
