//! What the page and the worker of its session say to each other (`docs/architecture.md`, 11.5), in `postcard`: the
//! page sends the files it read and the commands of the viewer, their parameters as JSON, as any caller of `engine`
//! does; the worker says when it listens, names each function just before it draws it, and answers each message with
//! what the window shows, and the drawing when it is a new one.

use std::sync::Arc;

use labcraft_diagram::Drawing;
use serde::{Deserialize, Serialize};

/// A folder as the page read it: its name, as the window shows it, and its program files.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub files: Vec<File>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    /// From the folder, with `/`.
    pub path: String,
    pub bytes: Vec<u8>,
}

/// From the page to the worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToWorker {
    /// Opens a folder in a new session, with the preference of the viewer. `skipped`: the functions whose drawing
    /// trapped a worker before, which this one never draws. `show`: the function to show once it is open, the one shown
    /// before a restart.
    Open { folder: Arc<Folder>, standard_everywhere: bool, skipped: Vec<String>, show: Option<String> },
    /// A command of the session (`docs/commands.md`, 6.5), its parameters as JSON.
    Command { id: String, params: String },
    /// Traps the worker, as a bug would: what the smoke test of the page restarts from.
    Trap,
}

/// From the worker to the page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FromWorker {
    /// It listens: what the page sent before was lost.
    Ready,
    /// It is about to draw this function: if it traps now, the drawing did.
    Drawing(String),
    /// The answer to a message.
    Answer(Answer),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    /// What the command returned, as JSON, or why it failed.
    pub result: Result<String, Failure>,
    pub state: State,
    /// The drawing of [`State::function`], when it is not the one the worker sent last.
    pub drawing: Option<Arc<Drawing>>,
}

/// A command that failed (`docs/commands.md`, 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

/// What the window shows of the session: `engine::view::Published`, with the diagnostics as text, without the drawing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub folder: Option<String>,
    pub functions: Vec<Listed>,
    /// The diagnostics, as `labcraft-cli check` prints them.
    pub problems: String,
    pub errors: u64,
    pub warnings: u64,
    pub function: Option<String>,
    pub opened: Vec<String>,
    pub glide: bool,
    pub kept: bool,
    /// The arm shown of each structure in tabs, by its key.
    pub arms: Vec<(String, usize)>,
    pub standard_everywhere: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub function: String,
    pub file: String,
    pub drawable: bool,
    pub errors: u64,
}

/// A message in `postcard`.
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, String> {
    postcard::to_allocvec(message).map_err(|error| format!("a message of the viewer could not be written: {error}"))
}

/// A message from `postcard`, and nothing after it.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, String> {
    match postcard::take_from_bytes(bytes) {
        Ok((message, [])) => Ok(message),
        Ok((_, rest)) => Err(format!("a message of the viewer has {} bytes too many", rest.len())),
        Err(error) => Err(format!("a message of the viewer could not be read: {error}")),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    #[test]
    fn a_message_is_read_whole_or_not_at_all() {
        let message = ToWorker::Command { id: "view.show_function".to_owned(), params: "{}".to_owned() };
        let mut bytes = encode(&message).unwrap();
        assert_eq!(decode::<ToWorker>(&bytes), Ok(message));
        assert!(decode::<ToWorker>(bytes.get(..bytes.len() - 1).unwrap()).is_err());
        bytes.push(0);
        assert!(decode::<ToWorker>(&bytes).unwrap_err().contains("1 bytes too many"));
    }
}
