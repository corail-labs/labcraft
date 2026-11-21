//! The gesture lock (architecture 12.5, commands.md 9.1): while the user is in a gesture, an editing command of an agent
//! waits for its end, at most [`WAIT`], then is `gesture.busy`. The window says when a gesture starts and ends
//! ([`crate::Session::set_gesture`]); the app holds the commands that wait, and runs them when it ends.

use std::time::Duration;

use serde_json::Value;

use crate::registry::{self, Effect};

/// The longest an editing command of an agent waits for the end of a gesture *(proposed)*, from its arrival.
pub const WAIT: Duration = Duration::from_secs(30);

/// A gesture of the user in progress, as `view.get_state` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// A rectangle of selection is being drawn.
    Rectangle,
    /// Nodes are being moved: a drag, or a run of presses of Ctrl and an arrow.
    Move,
    /// A wire is being drawn.
    Wire,
    /// A segment of a wire is being moved.
    Route,
    /// A frame is being resized by its edge.
    Resize,
    /// A field holds a text typed and not written yet: a name, a constant, the inspector, the `Text` panel.
    Field,
}

impl Gesture {
    pub fn as_str(self) -> &'static str {
        match self {
            Gesture::Rectangle => "rectangle",
            Gesture::Move => "move",
            Gesture::Wire => "wire",
            Gesture::Route => "route",
            Gesture::Resize => "resize",
            Gesture::Field => "field",
        }
    }
}

/// Whether a call waits for the end of a gesture: a command that can change a file, `commands.execute` by the command
/// it runs. A command with no effect, a read, does not wait (architecture 12.5).
pub fn waits(id: &str, params: &Value) -> bool {
    let id = if id == "commands.execute" { params.get("id").and_then(Value::as_str).unwrap_or_default() } else { id };
    registry::command(id).is_some_and(|command| command.undoable || command.effects.contains(&Effect::Files))
}

/// What an editing command gets that comes during a gesture that already made one `gesture.busy`: at once, so that an
/// agent waits at most one [`WAIT`] for one gesture (decision of the owner, 2026-11-21).
pub fn still_busy(id: &str) -> crate::CommandError {
    crate::CommandError::new(
        "gesture.busy",
        format!("`{id}` came during a gesture of the user that already gave up an edit, and changed nothing: call it again once it ends"),
    )
}

/// What an editing command gets that waited `waited` ([`WAIT`] but in the tests): it changed nothing.
pub fn busy(id: &str, waited: Duration) -> crate::CommandError {
    crate::CommandError::new(
        "gesture.busy",
        format!("`{id}` waited {} s for the end of a gesture of the user, and changed nothing: call it again", waited.as_secs_f32()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What changes a file waits, through `commands.execute` too; what reads does not (architecture 12.5).
    #[test]
    fn an_editing_command_waits_and_a_read_does_not() {
        for id in ["node.add", "wire.connect", "file.write", "history.undo", "commands.batch", "text.save"] {
            assert!(waits(id, &json!({})), "{id} waits");
        }
        for id in ["view.get_state", "history.list", "functions.list", "run.start", "run.abort", "check", "no.such"] {
            assert!(!waits(id, &json!({})), "{id} does not wait");
        }
        assert!(waits("commands.execute", &json!({ "id": "file.write", "params": {} })));
        assert!(!waits("commands.execute", &json!({ "id": "settings.get", "params": {} })));
    }
}
