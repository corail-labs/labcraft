//! The journal (architecture 12.5): every command that a session runs, from any author, is a line of JSON in
//! `.labcraft/journal/<process>-<session>.jsonl`, or, for the command line, whose processes run one command each, in
//! `.labcraft/journal/cli.jsonl`. It is for replaying a bug; git does not see it (`.labcraft/.gitignore`). It never fails a
//! command: a journal that cannot be written is given up, and the command goes on. In WASM it is kept in memory.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::error::CommandError;
use crate::history::Author;
use crate::runs::{hold, in_place, lock_of};
use crate::workspace::{Kind, Workspace};

/// The parameters of a line are cut beyond this many bytes of JSON *(proposed)*, the cut said.
pub const MAX_PARAMS: usize = 64 << 10;
/// A file of the journal holds at most this many bytes *(proposed)*.
pub const MAX_FILE: u64 = 16 << 20;
/// The newest files of sessions kept *(proposed)*, `cli.jsonl` apart.
pub const KEPT_FILES: usize = 20;
/// In WASM, the last lines kept in memory.
pub const MEMORY_LINES: usize = 1_000;
/// The file of the command line.
pub const CLI_FILE: &str = "cli.jsonl";
/// The lock that a process holds while it cuts `cli.jsonl`, and while it appends to it.
const CLI_LOCK: &str = "cli.lock";
/// A line names at most this many files *(proposed)*, the others counted.
pub const MAX_FILES: usize = 1_000;

/// Where the lines go.
enum Sink {
    /// Not opened yet: the first line makes the folder.
    Closed,
    /// The file, and how many bytes this session wrote to it.
    Open(PathBuf, u64),
    /// Given up: it could not be written, or it is full.
    Off,
    /// In memory, the last [`MEMORY_LINES`].
    Memory(VecDeque<String>),
}

pub(crate) struct Journal {
    /// The name of its file in `.labcraft/journal/`.
    name: String,
    /// The command line: `cli.jsonl`, shared by its processes and cut to its newest half when it passes [`MAX_FILE`].
    cli: bool,
    process: u32,
    sink: Sink,
    /// The lines written, to know whether a command wrote its own.
    pub(crate) lines: u64,
}

/// What a line says of a command.
pub(crate) struct Entry<'a> {
    pub time_ns: i64,
    pub author: &'a Author,
    /// `None` for external writes, which are no command.
    pub id: Option<&'a str>,
    pub params: &'a Value,
    pub outcome: Result<(), &'a CommandError>,
    /// Each file written, by its path from the root, with its revision (`None` for a file removed).
    pub files: &'a [(String, Option<String>)],
}

impl Journal {
    pub(crate) fn new(process: u32, session: u64) -> Journal {
        let sink = if cfg!(target_arch = "wasm32") { Sink::Memory(VecDeque::new()) } else { Sink::Closed };
        Journal { name: format!("{process}-{session}.jsonl"), cli: false, process, sink, lines: 0 }
    }

    /// The command line writes `cli.jsonl`.
    pub(crate) fn for_command_line(&mut self) {
        self.cli = true;
        self.name = CLI_FILE.to_owned();
    }

    /// The lines kept in memory, oldest first: in WASM only.
    pub(crate) fn in_memory(&self) -> Vec<String> {
        match &self.sink {
            Sink::Memory(lines) => lines.iter().cloned().collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn write(&mut self, ws: &Arc<dyn Workspace>, root: &Path, entry: &Entry<'_>) {
        let mut line = line_of(entry);
        line.push('\n');
        self.lines += 1;
        if let Sink::Memory(lines) = &mut self.sink {
            if lines.len() >= MEMORY_LINES {
                lines.pop_front();
            }
            lines.push_back(line);
            return;
        }
        if let Sink::Closed = self.sink {
            self.sink = match open(ws, root, &self.name, self.cli, self.process) {
                Some(path) => Sink::Open(path, 0),
                None => Sink::Off,
            };
        }
        let Sink::Open(path, written) = &mut self.sink else { return };
        // A link put there, by a clone of the project or by another program, never leads a line out of it.
        if !matches!(ws.link_metadata(path).map(|metadata| metadata.kind), Ok(Kind::File) | Err(_)) {
            self.sink = Sink::Off;
            return;
        }
        let size = line.len() as u64;
        // Held while this process cuts `cli.jsonl` or appends to it, so that a cut never drops a line of another.
        let _held = if self.cli { cli_lock(&**ws, path) } else { None };
        if self.cli {
            cut_if_full(&**ws, path);
        } else if written.saturating_add(size) > MAX_FILE {
            let full = json!({ "time": time_text(entry.time_ns), "full": format!("the journal holds at most {MAX_FILE} bytes: the commands after this one are not written") });
            let _ = ws.append(path, format!("{full}\n").as_bytes());
            self.sink = Sink::Off;
            return;
        }
        match ws.append(path, line.as_bytes()) {
            Ok(()) => *written = written.saturating_add(size),
            Err(_) => self.sink = Sink::Off,
        }
    }
}

fn time_text(time_ns: i64) -> String {
    labcraft_trace::number::timestamp_text(time_ns)
}

/// The line of a command, without its end.
pub(crate) fn line_of(entry: &Entry<'_>) -> String {
    let mut line = Map::new();
    line.insert("time".to_owned(), json!(time_text(entry.time_ns)));
    line.insert("author".to_owned(), json!(entry.author.name()));
    line.insert("id".to_owned(), json!(entry.id));
    if entry.id.is_none() {
        // External writes, which are no command, have no parameters.
        line.insert("params".to_owned(), Value::Null);
        line.insert("result".to_owned(), json!("ok"));
        let files = files_json(entry.files, &mut line);
        line.insert("files".to_owned(), files);
        return Value::Object(line).to_string();
    }
    let params = serde_json::to_string(entry.params).unwrap_or_default();
    if params.len() > MAX_PARAMS {
        let mut end = MAX_PARAMS;
        while !params.is_char_boundary(end) {
            end -= 1;
        }
        line.insert("params".to_owned(), json!(params.get(..end).unwrap_or_default()));
        line.insert("params_cut".to_owned(), json!(params.len()));
    } else {
        line.insert("params".to_owned(), entry.params.clone());
    }
    let result = match entry.outcome {
        Ok(()) => "ok",
        Err(error) => error.code,
    };
    line.insert("result".to_owned(), json!(result));
    let files = files_json(entry.files, &mut line);
    line.insert("files".to_owned(), files);
    Value::Object(line).to_string()
}

/// The files of a line, at most [`MAX_FILES`]; past them, `files_cut` counts them all.
fn files_json(files: &[(String, Option<String>)], line: &mut Map<String, Value>) -> Value {
    if files.len() > MAX_FILES {
        line.insert("files_cut".to_owned(), json!(files.len()));
    }
    Value::Array(files.iter().take(MAX_FILES).map(|(file, revision)| json!({ "file": file, "revision": revision })).collect())
}

/// The lock of `cli.jsonl`, waited for a little: another process holds it only while it appends a line or cuts the file.
/// Without it, a line is still appended in one write, and the cut waits for the next command.
fn cli_lock(ws: &dyn Workspace, path: &Path) -> Option<crate::workspace::Held> {
    let lock = path.with_file_name(CLI_LOCK);
    for _ in 0..50 {
        match ws.try_lock(&lock) {
            Ok(Some(held)) => return Some(held),
            Ok(None) => {
                #[cfg(not(target_arch = "wasm32"))]
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
    None
}

/// Makes `.labcraft/journal/` (and `.labcraft/.gitignore` holding `*` when it makes `.labcraft`), never through a link
/// that leads out of the project, holds the lock of the process there, and keeps the newest files. The path of the file.
fn open(ws: &Arc<dyn Workspace>, root: &Path, name: &str, cli: bool, process: u32) -> Option<PathBuf> {
    let labcraft = root.join(".labcraft");
    let new = ws.link_metadata(&labcraft).is_err();
    for (path, shown) in [(labcraft.clone(), ".labcraft"), (labcraft.join("journal"), ".labcraft/journal")] {
        if ws.link_metadata(&path).is_ok() {
            if !in_place(&**ws, root, shown) {
                return None;
            }
        } else {
            ws.create_folders(&path).ok()?;
        }
    }
    if new {
        ws.write(&labcraft.join(".gitignore"), crate::runs::LABCRAFT_GITIGNORE).ok()?;
    }
    let folder = labcraft.join("journal");
    let locks = hold(ws, &folder, process);
    if !cli {
        sweep(&**ws, &folder, name, process, locks);
    }
    Some(folder.join(name))
}

/// The process of a file of a session: `<process>-<session>.jsonl`.
fn owner(file: &str) -> Option<u32> {
    let (process, session) = file.strip_suffix(".jsonl")?.split_once('-')?;
    session.parse::<u64>().ok()?;
    process.parse().ok()
}

/// Keeps the [`KEPT_FILES`] newest files of sessions, `name` among them, and those of a process that still runs; removes
/// the lock of a process that no longer runs once no file of it is left.
fn sweep(ws: &dyn Workspace, folder: &Path, name: &str, process: u32, locks: bool) {
    let mut files: Vec<(Option<u128>, String, u32)> = Vec::new();
    let mut lock_owners: Vec<u32> = Vec::new();
    let _ = ws.read_dir(folder, &mut |entry| {
        if let Ok(entry) = entry
            && matches!(entry.kind, Ok(Kind::File))
            && let Some(file) = entry.name.to_str()
        {
            if let Some(owner) = owner(file) {
                if file != name {
                    let modified = ws.metadata(&folder.join(file)).ok().and_then(|metadata| metadata.modified);
                    files.push((modified, file.to_owned(), owner));
                }
            } else if let Some(owner) = file.strip_prefix('.').and_then(|rest| rest.strip_suffix(".lock")).and_then(|pid| pid.parse().ok()) {
                lock_owners.push(owner);
            }
        }
    });
    // Newest first; the file of this session is the newest of all.
    files.sort_by(|a, b| b.cmp(a));
    let mut left: HashSet<u32> = HashSet::new();
    for (index, (_, file, owner)) in files.into_iter().enumerate() {
        let old = index + 1 >= KEPT_FILES;
        if old && locks && owner != process && runs_no_more(ws, folder, owner) {
            let _ = ws.remove_file(&folder.join(file));
        } else {
            left.insert(owner);
        }
    }
    if locks {
        for owner in lock_owners {
            if owner != process && !left.contains(&owner) && runs_no_more(ws, folder, owner) {
                let _ = ws.remove_file(&lock_of(folder, owner));
            }
        }
    }
}

/// Whether a process no longer runs: its lock is missing, or free.
fn runs_no_more(ws: &dyn Workspace, folder: &Path, process: u32) -> bool {
    let lock = lock_of(folder, process);
    if ws.link_metadata(&lock).is_err() {
        return true;
    }
    // Let go at once: Windows does not remove a file that a lock holds.
    matches!(ws.try_lock(&lock), Ok(Some(_)))
}

/// `cli.jsonl` past [`MAX_FILE`] is cut to its newest half, from the start of a line: only that half is read.
fn cut_if_full(ws: &dyn Workspace, path: &Path) {
    let Ok(metadata) = ws.metadata(path) else { return };
    if metadata.len <= MAX_FILE {
        return;
    }
    let Ok(reader) = ws.open(path, metadata.len / 2) else { return };
    let mut half = Vec::new();
    // At most the file as it was, and a little that another process appended meanwhile.
    if std::io::Read::read_to_end(&mut std::io::Read::take(reader, MAX_FILE), &mut half).is_err() {
        return;
    }
    let start = half.iter().position(|byte| *byte == b'\n').map_or(half.len(), |at| at + 1);
    let _ = ws.write(path, half.get(start..).unwrap_or_default());
}

#[cfg(test)]
mod tests;
