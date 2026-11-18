//! Copy, cut, paste and duplicate on the canvas (notation 2.6). The clipboard holds text: the selection as a function
//! `clipboard` of the language (`fragment.copy`), which the window puts on the clipboard of the system as plain text; a
//! paste puts the text of the clipboard into the frame (`fragment.paste`) under the pointer, or in the middle of what the
//! view shows of the frame (decision of the owner, 2026-11-17).

use labcraft_engine::view::Published;
use serde_json::{Value, json};

use crate::host::{Host, Reply};
use crate::{Ide, Selection, canvas_shown, file_of};

/// What a copy asked of the session is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clip {
    /// The text goes on the clipboard.
    Copy,
    /// The text goes on the clipboard, then the nodes go, one step.
    Cut,
    /// The text is pasted beside the nodes, without the clipboard.
    Duplicate,
}

/// A copy sent: what it is for, the function, its file and its nodes, the corner of a duplicate, and its answer.
pub(crate) struct Copying {
    clip: Clip,
    function: String,
    file: String,
    nodes: Vec<String>,
    corner: (i64, i64),
    /// The frame of the nodes, where a duplicate goes.
    within: labcraft_editor::InFrame,
    reply: Reply,
}

/// How long a paste asked by a menu waits for the text of the clipboard, in seconds.
const PASTE_WAIT: f64 = 1.0;

/// How far Duplicate puts the copy, to the right and below (notation 2.6).
const DUPLICATE_OFFSET: i64 = 16;

impl Ide {
    /// Copy, cut or duplicate the nodes selected on the canvas of `function`: `fragment.copy` first, whose answer the
    /// window follows (`take_clips`).
    pub(crate) fn copy(&mut self, host: &dyn Host, view: &Published, function: &str, clip: Clip) {
        let Some(file) = file_of(view, function) else { return };
        let Some(window) = view.window_of(function) else { return };
        let selection = canvas_shown(window, &self.prefs).selection;
        if selection.nodes.is_empty() {
            self.say("Copy, cut and duplicate take the nodes selected: select one or more nodes");
            return;
        }
        // Duplicate counts from the corner of the selection, as the copy does, and pastes into the frame of the nodes.
        let canvas = self.drawn.get(function).map(|drawn| &drawn.canvas);
        let corner = canvas.and_then(|canvas| canvas.selection_corner(&selection));
        let frame = canvas.and_then(|canvas| canvas.selection_frame(&selection));
        let corner = match (clip, corner) {
            (Clip::Duplicate, _) if frame.is_none() => {
                self.say("Duplicate takes nodes of one frame: select nodes of one arm, one loop or the function");
                return;
            }
            (Clip::Duplicate, None) => {
                self.say("Duplicate: the nodes selected are not drawn yet");
                return;
            }
            (_, corner) => corner.unwrap_or_default(),
        };
        let reply = host.send("fragment.copy", json!({ "file": file, "nodes": selection.nodes }));
        let within = frame.unwrap_or_default();
        self.copying.push(Copying { clip, function: function.to_owned(), file, nodes: selection.nodes, corner, within, reply });
    }

    /// Paste `text` into the frame of `function`, its top-left corner where `Canvas::paste_at` says for `pointer`.
    pub(crate) fn paste(&mut self, host: &dyn Host, view: &Published, function: &str, text: &str, pointer: Option<egui::Pos2>) {
        let Some(file) = file_of(view, function) else { return };
        let Some(window) = view.window_of(function) else { return };
        let shown = canvas_shown(window, &self.prefs);
        let Some((within, (x, y))) = self.drawn.get(function).and_then(|drawn| drawn.canvas.paste_at(&shown, pointer)) else {
            self.say("Paste: the function is not drawn yet");
            return;
        };
        let reply = host.send("fragment.paste", crate::insert::within(json!({ "file": file, "text": text, "at": [x, y] }), &within));
        self.pasting.push((function.to_owned(), reply));
    }

    /// The answers of the copies and of the pastes: the text on the clipboard of the system, the nodes of a cut deleted,
    /// a duplicate pasted, and the nodes pasted selected.
    pub(crate) fn take_clips(&mut self, ctx: &egui::Context, host: &dyn Host) {
        for copying in std::mem::take(&mut self.copying) {
            let Some(answer) = copying.reply.take() else {
                self.copying.push(copying);
                continue;
            };
            let Copying { clip, function, file, nodes, corner: (x, y), within, .. } = copying;
            let text = match answer.map(|result| result.get("text").and_then(Value::as_str).map(str::to_owned)) {
                Ok(Some(text)) => text,
                // Nothing goes on the clipboard, and nothing is cut, without the text of the copy.
                Ok(None) => {
                    self.say(format!("{function}: the copy gave no text"));
                    continue;
                }
                Err(error) => {
                    self.say(format!("{function}: {}", error.message));
                    continue;
                }
            };
            match clip {
                Clip::Copy => ctx.copy_text(text),
                Clip::Cut => {
                    ctx.copy_text(text);
                    let commands = json!([{ "id": "node.delete", "params": { "file": file, "nodes": nodes } }]);
                    let reply = host.send("commands.batch", json!({ "commands": commands, "label": "Cut" }));
                    self.edits.push((function, reply, None));
                }
                Clip::Duplicate => {
                    let at = [x.saturating_add(DUPLICATE_OFFSET), y.saturating_add(DUPLICATE_OFFSET)];
                    let params = crate::insert::within(json!({ "file": file, "text": text, "at": at }), &within);
                    let commands = json!([{ "id": "fragment.paste", "params": params }]);
                    let reply = host.send("commands.batch", json!({ "commands": commands, "label": "Duplicate" }));
                    self.pasting.push((function, reply));
                }
            }
        }
        for (function, reply) in std::mem::take(&mut self.pasting) {
            match reply.take() {
                None => self.pasting.push((function, reply)),
                Some(Err(error)) => self.say(format!("{function}: {}", error.message)),
                // What was pasted is the selection, as the nodes moved by a drag are (notation 2.6).
                Some(Ok(result)) => {
                    let nodes = pasted_nodes(&result);
                    if !nodes.is_empty() {
                        self.ask_select(host, &function, Selection { nodes, wires: Vec::new(), terminals: Vec::new() });
                    }
                }
            }
        }
    }

    /// A paste asked by a menu or the palette: the Paste event that egui sends back once asked for one
    /// (`ViewportCommand::RequestPaste`), taken out of the input before the keymap reads it. egui sends none when the
    /// clipboard holds no text: after a second the ask goes, so that a later Ctrl+V in a field stays the field's.
    pub(crate) fn asked_paste(&mut self, ctx: &egui::Context) -> Option<(String, String)> {
        let (function, _, since) = self.paste_asked.as_ref()?;
        let (function, since) = (function.clone(), *since);
        // Checked first: a frame can come long after the ask, with a Ctrl+V meant for a field.
        if self.now - since > PASTE_WAIT {
            self.paste_asked = None;
            self.say("Paste: the clipboard holds no text");
            return None;
        }
        let text = ctx.input_mut(|input| {
            let at = input.events.iter().position(|event| matches!(event, egui::Event::Paste(_)))?;
            match input.events.remove(at) {
                egui::Event::Paste(text) => Some(text),
                _ => None,
            }
        });
        let text = text?;
        self.paste_asked = None;
        Some((function, text))
    }
}

/// The nodes that a paste named, in its result or in the one command of a batch (`names.nodes`).
fn pasted_nodes(result: &Value) -> Vec<String> {
    let names = result.get("names").or_else(|| result.get("commands")?.as_array()?.first()?.get("names"));
    names
        .and_then(|names| names.get("nodes"))
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}
