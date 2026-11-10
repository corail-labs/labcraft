//! The file of `--style` (`docs/cli.md`, 8; notation 10): the values of the notation that the viewer draws with, written
//! with the defaults when it does not exist yet, and read again each time it is saved. A file that cannot be read, or
//! whose values are refused, leaves the values before, and the viewer says why.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use labcraft_diagram::style::{self, Style};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::session::Message;

/// How many colours below notation 8 the notice names.
const NAMED: usize = 4;

/// Writes the defaults into `path` when nothing is there; says why it could not.
pub fn prepare(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::fs::write(path, style::file_of(&Style::default()))
            .map_err(|error| format!("cannot write the defaults into `{}`: {error}", path.display())),
        Err(error) => Err(format!("cannot read `{}`: {error}", path.display())),
    }
}

/// The values of the file, if it can be read and they are taken, and what the viewer says of them: why they were not
/// taken, or which colours fall below the contrasts of notation 8.
pub fn read(path: &Path) -> (Option<Style>, Option<String>) {
    let shown = path.display();
    let text = match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > style::MAX_FILE as u64 => Err(format!("more than {} bytes", style::MAX_FILE)),
        Ok(_) => {
            std::fs::read(path).map_err(|error| error.to_string()).and_then(|bytes| String::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned()))
        }
        Err(error) => Err(error.to_string()),
    };
    let read = text.and_then(|text| style::read(&text));
    match read {
        Err(problem) => (None, Some(format!("`{shown}`: {problem}; the values before stay"))),
        Ok(style) => {
            let problems = style::contrast_problems(&style);
            let notice = (!problems.is_empty()).then(|| {
                let mut said = problems.iter().take(NAMED).cloned().collect::<Vec<_>>().join("; ");
                if problems.len() > NAMED {
                    said.push_str(&format!("; and {} more", problems.len() - NAMED));
                }
                format!("`{shown}`: colours below the contrasts of notation 8: {said}")
            });
            (Some(style), notice)
        }
    }
}

/// Watches the folder of `path`, not its subfolders, and sends [`Message::StyleChanged`] when the file is written,
/// replaced or removed: an editor that saves by renaming a new file over it is seen too.
pub fn watch(path: &Path, changes: Sender<Message>) -> Result<RecommendedWatcher, String> {
    let name = path.file_name().map(ToOwned::to_owned);
    let folder: PathBuf = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let handler = move |event: notify::Result<Event>| {
        if let Ok(event) = event
            && !matches!(event.kind, EventKind::Access(_))
            && event.paths.iter().any(|changed| changed.file_name() == name.as_deref())
        {
            let _ = changes.send(Message::StyleChanged);
        }
    };
    let mut watcher = RecommendedWatcher::new(handler, Config::default().with_follow_symlinks(false)).map_err(|error| error.to_string())?;
    watcher.watch(&folder, RecursiveMode::NonRecursive).map_err(|error| format!("cannot watch `{}`: {error}", folder.display()))?;
    Ok(watcher)
}
