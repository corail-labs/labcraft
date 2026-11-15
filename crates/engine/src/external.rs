//! External writes (commands.md 9.1, architecture 12.5): a file of the project whose bytes are no longer those the session
//! last saw was changed by another program; the change becomes a step signed `external`, which undo goes back over as over
//! any other. The session follows every program and simulation file of the project from its first command, and those that
//! appear; it looks before each command and when the app says that files changed ([`crate::Session::files_changed`]).
//! Writes whose files were modified less than 1 s apart are one step (decisions of the owner, 2026-11-15).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::history::Change;
use crate::session::Session;
use crate::workspace::{Kind, Read};

/// At most this many files followed *(proposed)*: past it, the others are not.
pub const MAX_FOLLOWED: usize = 10_000;
/// At most this many bytes of the files followed are kept *(proposed)*, as the history keeps 64 MiB of texts (decision of
/// the owner, 2026-11-15): past it, a file is followed by its size and time only, and a change of it from outside is no
/// step that undo can take back.
pub const MAX_KEPT: usize = 64 << 20;
/// At most this deep below the root.
const MAX_DEPTH: usize = 32;
/// A file modified at least this long before the session read it is settled: a later write gives it a new time, so its
/// size and time tell whether it changed. A file modified more recently is read again, since a file system whose clock
/// ticks slowly gives two writes of one tick the same time.
const SETTLED_NS: i128 = 2_000_000_000;

/// What the session keeps of a file it follows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Content {
    Bytes(Arc<[u8]>),
    Missing,
    /// Past [`MAX_KEPT`]: its size and time only.
    Unheld,
}

impl Content {
    fn of(bytes: Option<Arc<[u8]>>) -> Content {
        bytes.map_or(Content::Missing, Content::Bytes)
    }

    fn len(&self) -> usize {
        match self {
            Content::Bytes(bytes) => bytes.len(),
            _ => 0,
        }
    }
}

/// A file as the session last saw it: its path as shown, what it keeps of it, and its size and time of modification, when
/// the file was settled (or is not kept), so that a file that did not change is not read again.
#[derive(Clone, Debug)]
struct Seen {
    shown: String,
    content: Content,
    stamp: Option<(u64, u128)>,
}

/// The files of the project as the session last saw them.
#[derive(Debug, Default)]
pub(crate) struct Known {
    files: BTreeMap<PathBuf, Seen>,
    /// The bytes kept, at most [`MAX_KEPT`] but for the growth of a file kept.
    kept: usize,
    /// Whether the files of the project were read once: until then, a file found is not an external write.
    started: bool,
}

impl Known {
    fn put(&mut self, path: PathBuf, seen: Seen) {
        self.kept = self.kept.saturating_add(seen.content.len());
        if let Some(old) = self.files.insert(path, seen) {
            self.kept = self.kept.saturating_sub(old.content.len());
        }
    }

    /// Whether `len` more bytes fit, in place of those kept of `path`.
    fn fits(&self, path: &Path, len: u64) -> bool {
        let held = self.files.get(path).map_or(0, |seen| seen.content.len());
        usize::try_from(len).is_ok_and(|len| self.kept.saturating_sub(held).saturating_add(len) <= MAX_KEPT)
    }

    /// The session wrote `bytes` to the file, or removed it.
    pub(crate) fn wrote(&mut self, path: &Path, shown: &str, bytes: Option<Arc<[u8]>>) {
        let len = bytes.as_ref().map_or(0, |bytes| bytes.len() as u64);
        let content = if self.fits(path, len) { Content::of(bytes) } else { Content::Unheld };
        self.put(path.to_path_buf(), Seen { shown: shown.to_owned(), content, stamp: None });
    }

    /// The bytes it last saw of the file, when they are `bytes`: shared with the history rather than copied.
    pub(crate) fn shared(&self, path: &Path, bytes: &[u8]) -> Option<Arc<[u8]>> {
        match self.files.get(path).map(|seen| &seen.content) {
            Some(Content::Bytes(known)) if **known == *bytes => Some(Arc::clone(known)),
            _ => None,
        }
    }
}

/// Whether a file is followed: a program, or a simulation file (and the manifest, `labcraft.toml`).
fn followed(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".lcf") || lower.ends_with(".lct") || lower.ends_with(".toml")
}

/// The files of the project that are followed, by their paths, with their names as shown: a link is not followed, nor a
/// folder whose name starts with `.` (`.labcraft`, `.git`).
fn files_of(session: &Session) -> BTreeMap<PathBuf, String> {
    let mut found = BTreeMap::new();
    let mut folders = vec![(session.root.clone(), String::new(), 0)];
    while let Some((folder, shown, depth)) = folders.pop() {
        let _ = session.ws.read_dir(&folder, &mut |entry| {
            let Ok(entry) = entry else { return };
            let Some(name) = entry.name.to_str() else { return };
            let path = folder.join(name);
            let named = if shown.is_empty() { name.to_owned() } else { format!("{shown}/{name}") };
            match entry.kind {
                Ok(Kind::Folder) if !name.starts_with('.') && depth < MAX_DEPTH => folders.push((path, named, depth + 1)),
                Ok(Kind::File) if followed(name) && found.len() < MAX_FOLLOWED => {
                    found.insert(path, named);
                }
                _ => {}
            }
        });
    }
    found
}

/// What the file holds now: `Some(None)` when it is missing, `None` when it cannot be read.
fn bytes_now(session: &Session, path: &Path) -> Option<Option<Vec<u8>>> {
    match session.ws.read(path, session.config.limits.max_file_size) {
        Ok(Read::Bytes(bytes)) => Some(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(None),
        _ => None,
    }
}

/// A file of a step, as the journal names it: its path as shown, and its revision (`None` once removed).
pub(crate) type Revised = (String, Option<String>);

/// Records the external writes since the last look as a step, merged into the last one when they are of one burst: the
/// number of the step and the files it changed, if one changed. A file that cannot be read now is looked at next time.
pub(crate) fn notice(session: &mut Session) -> Option<(u64, Vec<Revised>)> {
    if !session.keeps_history {
        return None;
    }
    let found = files_of(session);
    let started = session.known.started;
    let mut paths: BTreeSet<PathBuf> = found.keys().cloned().collect();
    paths.extend(session.known.files.keys().cloned());
    let mut changes = Vec::new();
    let mut latest: Option<u128> = None;
    let looked_at = i128::from(session.config.clock.utc_ns());
    for path in paths {
        let metadata = session.ws.metadata(&path).ok().filter(|metadata| metadata.kind == Kind::File);
        let stamp = metadata.as_ref().and_then(|metadata| metadata.modified.map(|modified| (metadata.len, modified)));
        let known = session.known.files.get(&path).cloned();
        if stamp.is_some() && known.as_ref().is_some_and(|seen| seen.stamp == stamp) {
            continue;
        }
        let shown = found.get(&path).cloned().or_else(|| known.as_ref().map(|seen| seen.shown.clone())).unwrap_or_default();
        // Not kept, or past the budget: followed by its size and time, with no step.
        let kept = known.as_ref().is_some_and(|seen| matches!(seen.content, Content::Bytes(_)));
        let unheld = known.as_ref().is_some_and(|seen| seen.content == Content::Unheld);
        if let Some(metadata) = &metadata
            && (unheld || (!kept && !session.known.fits(&path, metadata.len)))
        {
            session.known.put(path, Seen { shown, content: Content::Unheld, stamp });
            continue;
        }
        let Some(now) = bytes_now(session, &path) else { continue };
        if unheld {
            // Gone: what it held was not kept, so its removal is no step either.
            session.known.put(path, Seen { shown, content: Content::Missing, stamp: None });
            continue;
        }
        let before = known.as_ref().and_then(|seen| match &seen.content {
            Content::Bytes(bytes) => Some(Arc::clone(bytes)),
            _ => None,
        });
        // A file removed that another name reaches, as its case does on Windows, is still removed: that other name is
        // followed of its own (`file.rename` of `Gain.lcf` to `gain.lcf`).
        if now.is_some() && known.as_ref().is_some_and(|seen| seen.content == Content::Missing) && !same_name(session, &path) {
            continue;
        }
        let after: Option<Arc<[u8]>> = match (&before, now) {
            (Some(before), Some(now)) if **before == *now => Some(Arc::clone(before)),
            (_, now) => now.map(Arc::from),
        };
        if started && before != after {
            latest = latest.max(stamp.map(|(_, modified)| modified));
            changes.push(Change { path: path.clone(), shown: shown.clone(), before, after: after.clone() });
        }
        let settled = stamp.filter(|(_, modified)| i128::try_from(*modified).is_ok_and(|modified| modified + SETTLED_NS <= looked_at));
        session.known.put(path, Seen { shown, content: Content::of(after), stamp: settled });
    }
    session.known.started = true;
    if changes.is_empty() {
        return None;
    }
    session.forget();
    // The step has the time of the session. Its burst has the latest modification of its files, as the system says it, or
    // the clock of the session when it says none (a removal).
    let time = session.config.clock.utc_ns();
    let burst = latest.and_then(|modified| i64::try_from(modified).ok()).unwrap_or(time);
    let files = changes.iter().map(|change| (change.shown.clone(), change.after.as_ref().map(|bytes| crate::revision(bytes)))).collect();
    session.history.record_external(time, burst, changes).map(|step| (step, files))
}

/// Whether the path names the file under its own name, not through another spelling that the system takes for it.
pub(crate) fn same_name(session: &Session, path: &Path) -> bool {
    session.ws.canonicalize(path).ok().as_deref() == Some(path)
}
