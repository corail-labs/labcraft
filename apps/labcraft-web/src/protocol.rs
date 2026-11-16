//! What the page and the worker of its session say to each other (`docs/architecture.md`, 12.9), in `postcard`: the
//! page sends the files it read and the commands of the window, their parameters as JSON, as any caller of `engine`
//! does; the worker says when it listens, names each function just before it draws it, and answers each message with
//! what the session publishes ([`Seen`]), the listing and the drawing when they are new ones, and the files that
//! changed, of which the page keeps a copy.

use std::sync::Arc;

use labcraft_diagram::Drawing;
use serde::{Deserialize, Serialize};

/// A folder as the page read it: its name, as the window shows it, its program files, and the folders that no file
/// makes (`folder::plan`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub files: Vec<File>,
    /// From the folder, with `/`.
    pub folders: Vec<String>,
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
    /// Opens a folder in a new session, with the preference of the window. `skipped`: the functions whose drawing
    /// trapped a worker before, which this one never draws. `show`: the function to show once it is open, the one shown
    /// before a restart.
    Open { folder: Arc<Folder>, standard_everywhere: bool, skipped: Vec<String>, show: Option<String> },
    /// A command of the session (`docs/commands.md`), its parameters as JSON; `request` names its answer.
    Command { request: u64, id: String, params: String },
    /// The zoom of the canvas shown, which `view.get_state` returns: no command.
    Zoom(f32),
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
    /// The files that the message in flight changed, said before the session draws again after them: a trap of that
    /// drawing does not lose them.
    Files(Changed),
    /// The answer to a message.
    Answer(Box<Answer>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    /// The [`ToWorker::Command`] answered; none for the other messages.
    pub request: Option<u64>,
    /// What the command returned, as JSON, or why it failed.
    pub result: Result<String, Failure>,
    pub seen: Seen,
    /// The functions and the diagnostics, when they are not those the worker sent last: they change with a check, not
    /// with a click.
    pub listing: Option<Listing>,
    /// The drawing of [`Seen::function`], when it is not the one the worker sent last.
    pub drawing: Option<Arc<Drawing>>,
    /// The files that the message changed, when it changed any.
    pub files: Option<Changed>,
}

impl Answer {
    /// A message that could not be taken: it answers no command, and changes nothing.
    pub fn failed(code: &str, message: String) -> Answer {
        Answer {
            request: None,
            result: Err(Failure { code: code.to_owned(), message }),
            seen: Seen::default(),
            listing: None,
            drawing: None,
            files: None,
        }
    }
}

/// A command that failed (`docs/commands.md`, 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

/// What the window shows of the session: `engine::view::Published`, without the drawing and the [`Listing`]. No trace
/// and no values: a run needs threads, which the WASM build does not have (D-22).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    /// The generation of the publication of the session: what the page counts as a change.
    pub generation: u64,
    /// The checks of the folder that the session made: what the window reads the files again on.
    pub checks: u64,
    pub folder: Option<String>,
    pub function: Option<String>,
    pub opened: Vec<String>,
    pub glide: bool,
    pub kept: bool,
    /// The arm shown of each structure in tabs, by its key.
    pub arms: Vec<(String, usize)>,
    pub standard_everywhere: bool,
    /// The view of each tab whose view is not the diagram alone: the function, its view, the second view of a split.
    pub tabs: Vec<(String, String, Option<String>)>,
    pub preview: Option<String>,
    /// The selection: the function, its nodes, wires and terminals.
    pub selection: (String, Vec<String>, Vec<String>, Vec<String>),
    pub workspace: String,
}

/// The functions of the folder and its diagnostics, from one report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub functions: Vec<Listed>,
    pub problems: Vec<Problem>,
    pub errors: u64,
    pub warnings: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub function: String,
    pub file: String,
    pub drawable: bool,
    pub errors: u64,
    pub warnings: u64,
}

/// A diagnostic, as `Problems` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// `LC0001`.
    pub code: String,
    pub message: String,
    pub file: String,
    /// Its start and end, as lines and columns.
    pub span: Option<[u32; 4]>,
    pub function: Option<String>,
}

/// The files that changed since the worker last said: their new bytes, and those that are gone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changed {
    pub written: Vec<File>,
    /// From the folder, with `/`.
    pub removed: Vec<String>,
}

impl Folder {
    /// The folder with the changes the worker said: what a new worker opens after a trap.
    pub fn changed(&mut self, changed: Changed) {
        self.files.retain(|file| !changed.removed.contains(&file.path) && !changed.written.iter().any(|written| written.path == file.path));
        self.files.extend(changed.written);
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
    }
}

/// A message in `postcard`.
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, String> {
    postcard::to_allocvec(message).map_err(|error| format!("a message of the window could not be written: {error}"))
}

/// A message from `postcard`, and nothing after it.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, String> {
    match postcard::take_from_bytes(bytes) {
        Ok((message, [])) => Ok(message),
        Ok((_, rest)) => Err(format!("a message of the window has {} bytes too many", rest.len())),
        Err(error) => Err(format!("a message of the window could not be read: {error}")),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    #[test]
    fn a_message_is_read_whole_or_not_at_all() {
        let message = ToWorker::Command { request: 7, id: "view.show_function".to_owned(), params: "{}".to_owned() };
        let mut bytes = encode(&message).unwrap();
        assert_eq!(decode::<ToWorker>(&bytes), Ok(message));
        assert!(decode::<ToWorker>(bytes.get(..bytes.len() - 1).unwrap()).is_err());
        bytes.push(0);
        assert!(decode::<ToWorker>(&bytes).unwrap_err().contains("1 bytes too many"));
    }

    #[test]
    fn a_folder_takes_the_changes_of_the_worker() {
        let file = |path: &str, text: &str| File { path: path.to_owned(), bytes: text.as_bytes().to_vec() };
        let mut folder = Folder { name: "f".to_owned(), files: vec![file("a.lcf", "a"), file("b.lcf", "b")], folders: Vec::new() };
        folder.changed(Changed { written: vec![file("c.lcf", "c"), file("a.lcf", "a2")], removed: vec!["b.lcf".to_owned()] });
        assert_eq!(folder.files, [file("a.lcf", "a2"), file("c.lcf", "c")]);
    }
}
