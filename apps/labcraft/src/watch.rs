//! Watching the folder and the folders of `--lib-path` (`docs/architecture.md`, 11.5): `notify` says when a program
//! file changes (`.lcf`, `.lct`, or a folder that holds them), and the session checks again. A path that the system
//! refuses to watch (a limit of watches, a path that does not exist) is looked at every second, and the viewer says
//! so; the others are still watched. Symbolic links are not followed (`docs/cli.md`, section 1).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::session::Message;

/// How often the files are looked at when they cannot be watched.
pub const POLL: Duration = Duration::from_secs(1);

/// A watch under way; dropping it stops it.
pub struct Watching {
    _watchers: Vec<RecommendedWatcher>,
    stop: Arc<AtomicBool>,
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Whether a change of this path can change a drawing: a program file, or a folder (it has no extension), whose
/// renaming or removal moves program files. A log that a bench writes in its folder changes nothing.
pub fn concerns(path: &Path) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => extension.eq_ignore_ascii_case("lcf") || extension.eq_ignore_ascii_case("lct"),
        None => true,
    }
}

/// Whether an event can change a drawing: a program file written, created, removed or renamed; a folder created,
/// removed or renamed, whatever its name (`v1.0`); a path that is gone, which may have been either.
pub fn changes_a_drawing(kind: &EventKind, path: &Path) -> bool {
    let program = |path: &Path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lcf") || extension.eq_ignore_ascii_case("lct"))
    };
    match kind {
        EventKind::Access(_) => false,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
            program(path) || path.is_dir() || !path.exists()
        }
        _ => program(path),
    }
}

/// Watches each of `paths`, sending [`Message::Changed`] for each change of a program file; the notice says which
/// paths are looked at every second instead, and why.
pub fn watch(paths: &[PathBuf], changes: Sender<Message>) -> (Watching, Option<String>) {
    let stop = Arc::new(AtomicBool::new(false));
    let mut watchers = Vec::new();
    let mut refused: Vec<(PathBuf, String)> = Vec::new();
    for path in paths {
        let sender = changes.clone();
        let handler = move |event: notify::Result<Event>| {
            if let Ok(event) = event
                && event.paths.iter().any(|path| changes_a_drawing(&event.kind, path))
            {
                let _ = sender.send(Message::Changed);
            }
        };
        let made = RecommendedWatcher::new(handler, Config::default().with_follow_symlinks(false));
        match made {
            Ok(mut watcher) => match watcher.watch(path, RecursiveMode::Recursive) {
                Ok(()) => watchers.push(watcher),
                Err(error) => refused.push((path.clone(), error.to_string())),
            },
            Err(error) => refused.push((path.clone(), error.to_string())),
        }
    }
    if refused.is_empty() {
        return (Watching { _watchers: watchers, stop }, None);
    }
    let said: Vec<String> = refused.iter().map(|(path, error)| format!("`{}` ({error})", path.display())).collect();
    let polled: Vec<PathBuf> = refused.into_iter().map(|(path, _)| path).collect();
    let flag = stop.clone();
    let started = std::thread::Builder::new().name("labcraft-poll".to_owned()).spawn(move || poll(&polled, &changes, &flag));
    let notice = match started {
        Ok(_) => format!("cannot watch {}: looking at its files every second", said.join(", ")),
        Err(error) => format!("cannot watch {}, nor look at its files: {error}", said.join(", ")),
    };
    (Watching { _watchers: watchers, stop }, Some(notice))
}

/// Every second, what the program files look like; a change sends [`Message::Changed`].
fn poll(paths: &[PathBuf], changes: &Sender<Message>, stop: &AtomicBool) {
    let mut before = fingerprint(paths);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(POLL);
        let now = fingerprint(paths);
        if now != before {
            before = now;
            if changes.send(Message::Changed).is_err() {
                return;
            }
        }
    }
}

/// The path, size and time of each program file under `paths`, without following links.
pub fn fingerprint(paths: &[PathBuf]) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    let mut found = Vec::new();
    for path in paths {
        collect(path, &mut found, 0);
    }
    found.sort();
    found
}

fn collect(path: &Path, found: &mut Vec<(PathBuf, u64, Option<SystemTime>)>, depth: usize) {
    // As deep as a walk goes (cli.md, section 1).
    if depth > 256 {
        return;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else { return };
    if metadata.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            collect(&entry.path(), found, depth + 1);
        }
    } else if metadata.is_file() && concerns(path) {
        found.push((path.to_path_buf(), metadata.len(), metadata.modified().ok()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_renamed_counts_whatever_its_name_and_a_log_written_does_not() {
        use notify::event::{DataChange, ModifyKind, RenameMode};
        let renamed = EventKind::Modify(ModifyKind::Name(RenameMode::Any));
        let written = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        let here = std::env::temp_dir();
        assert!(changes_a_drawing(&renamed, &here.join("labcraft-gone-v1.0")));
        assert!(changes_a_drawing(&renamed, &here));
        assert!(changes_a_drawing(&written, Path::new("a/main.lcf")));
        assert!(!changes_a_drawing(&written, &here.join("results.log")));
        assert!(!changes_a_drawing(&EventKind::Access(notify::event::AccessKind::Any), Path::new("a/main.lcf")));
    }

    #[test]
    fn only_program_files_and_folders_concern_a_drawing() {
        for path in ["a/main.lcf", "a/Verdict.LCT", "a/drivers"] {
            assert!(concerns(Path::new(path)), "{path}");
        }
        for path in ["a/results.log", "a/main.lcf.swp", "a/notes.md"] {
            assert!(!concerns(Path::new(path)), "{path}");
        }
    }
}
