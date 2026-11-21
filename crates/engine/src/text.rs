//! The commands of the `Text` panel (commands.md 9.1; decision of the owner, 2026-11-21): `text.save` writes the text
//! of a function or a type when it parses, and keeps one that does not in the recovery folder, `.labcraft/recovery/`,
//! under the path of its file (architecture 12.6); `text.recovered` gives back the texts kept there that are newer than
//! their file, and `text.discard` removes one. They are commands of the window: an agent writes with `file.write`.

use std::path::{Path, PathBuf};

use labcraft_types::{Interner, Severity};
use serde_json::{Value, json};

use crate::edits::{bytes_now, revision};
use crate::error::CommandError;
use crate::files::{self, MAX_WRITE};
use crate::history::Author;
use crate::params::Params;
use crate::paths::{self, Wanted};
use crate::registry::Command;
use crate::runs::{in_place, labcraft_folder};
use crate::session::Session;
use crate::walk::{has_program_extension, io_text};
use crate::workspace::{Kind, Read};

#[cfg(test)]
mod tests;

/// The recovery folder, from the root of the project.
pub const RECOVERY: &str = ".labcraft/recovery";
/// At most this much text in all in one `text.recovered` *(proposed)*.
pub const MAX_RECOVERED: usize = 16 << 20;
/// At most this much text in one `text.save`, as `file.write`.
pub const MAX_SAVE: usize = MAX_WRITE;
/// At most this many entries of the recovery folder are looked at *(proposed)*.
const MAX_ENTRIES: usize = 1_000;

/// A program or a type of the project, judged as every command judges a path; the `Text` panel edits no simulation file.
fn program(session: &Session, params: &Params, existing: bool) -> Result<(PathBuf, String), CommandError> {
    let given = params.text("path")?;
    let (path, shown, exists) = paths::in_project(&*session.ws, &session.root, given, "path", Wanted::File)?;
    if files::is_simulation(&path) || !has_program_extension(&path) {
        return Err(CommandError::of(
            "path.invalid",
            "path",
            format!("`{shown}` is not a `.lcf` or `.lct` file: the `Text` panel edits programs and types"),
        ));
    }
    if existing && !exists {
        return Err(CommandError::of("path.not_found", "path", format!("`{shown}` does not exist")));
    }
    if exists && session.ws.metadata(&path).map(|metadata| metadata.kind).ok() != Some(Kind::File) {
        return Err(CommandError::of("path.invalid", "path", format!("`{shown}` is not a regular file")));
    }
    Ok((path, shown))
}

/// The copy of a file in the recovery folder.
fn copy_of(root: &Path, shown: &str) -> (PathBuf, String) {
    let shown = format!("{RECOVERY}/{shown}");
    let mut path = root.to_path_buf();
    path.extend(shown.split('/'));
    (path, shown)
}

/// `text.save`: written when it parses, one step of history; kept in the recovery folder when it does not, when the
/// file changed since `expected_revision`, or with `keep`.
pub fn save(session: &mut Session, author: &Author, command: &Command, params: &Params) -> Result<Value, CommandError> {
    let (path, shown) = program(session, params, true)?;
    let text = files::writable_text(session, params)?;
    if params.bool("keep") {
        return Ok(json!({ "written": false, "kept": keep(session, &shown, &text) }));
    }
    let mut names = Interner::new();
    let read = labcraft_format::read(text.as_bytes(), &mut names, &session.config.limits);
    if read.file.is_none() {
        let error = read.report.diagnostics().iter().find(|diagnostic| diagnostic.severity() == Severity::Error).cloned().map(|mut diagnostic| {
            diagnostic.file.clone_from(&shown);
            diagnostic
        });
        let kept = keep(session, &shown, &text);
        return Ok(json!({ "written": false, "kept": kept, "error": error }));
    }
    if let Some(expected) = params.get("expected_revision").and_then(Value::as_str) {
        let now = bytes_now(session, &path)?.map(|bytes| revision(&bytes)).unwrap_or_default();
        if now != expected {
            let kept = keep(session, &shown, &text);
            return Ok(json!({ "written": false, "kept": kept, "stale": true, "revision": now }));
        }
    }
    let written = revision(text.as_bytes());
    let copy = text.clone();
    if let Err(error) = files::write_text(session, author, command, path, &shown, true, text) {
        keep(session, &shown, &copy);
        return Err(error);
    }
    remove_copy(session, &shown);
    Ok(json!({ "written": true, "revision": written }))
}

/// Keeps the text in the recovery folder, never through a link that leads out of the project; whether it is kept.
fn keep(session: &Session, shown: &str, text: &str) -> bool {
    let ws = &*session.ws;
    let root = &session.root;
    if labcraft_folder(ws, root).is_err() {
        return false;
    }
    let (path, copy) = copy_of(root, shown);
    // Each folder made one at a time, and judged before the next: a link among them is never followed.
    let mut folder = root.clone();
    let mut folder_shown = String::new();
    let folders: Vec<&str> = copy.split('/').collect();
    for name in folders.iter().take(folders.len().saturating_sub(1)) {
        folder.push(name);
        if !folder_shown.is_empty() {
            folder_shown.push('/');
        }
        folder_shown.push_str(name);
        if ws.link_metadata(&folder).is_err() && ws.create_folders(&folder).is_err() {
            return false;
        }
        if !in_place(ws, root, &folder_shown) {
            return false;
        }
    }
    // A link where the copy goes is not written through.
    if ws.link_metadata(&path).is_ok_and(|metadata| metadata.kind != Kind::File) {
        return false;
    }
    ws.write(&path, text.as_bytes()).is_ok()
}

/// Removes the copy of a file, if the recovery folder holds one in place; whether it did.
fn remove_copy(session: &Session, shown: &str) -> bool {
    let ws = &*session.ws;
    let (path, copy) = copy_of(&session.root, shown);
    let Some((folder_shown, _)) = copy.rsplit_once('/') else { return false };
    if ws.link_metadata(&path).map(|metadata| metadata.kind).ok() != Some(Kind::File) || !in_place(ws, &session.root, folder_shown) {
        return false;
    }
    ws.remove_file(&path).is_ok()
}

/// `text.discard`.
pub fn discard(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let (_, shown) = program(session, params, false)?;
    Ok(json!({ "discarded": remove_copy(session, &shown) }))
}

/// `text.recovered`: the texts of the recovery folder whose file exists and changed before them.
pub fn recovered(session: &mut Session) -> Result<Value, CommandError> {
    let ws = &*session.ws;
    let root = &session.root;
    let mut texts = Vec::new();
    let mut truncated = false;
    let mut entries = 0usize;
    let mut used = 0usize;
    let mut folder = root.clone();
    folder.extend(RECOVERY.split('/'));
    if ws.link_metadata(&folder).is_err() || !in_place(ws, root, RECOVERY) {
        return Ok(json!({ "texts": texts, "truncated": false }));
    }
    // The folders still to read, each with its path from the recovery folder.
    let mut folders = vec![(folder, String::new())];
    'walk: while let Some((folder, prefix)) = folders.pop() {
        // At most the entries left to look at, from one folder too.
        let mut found = Vec::new();
        let left = MAX_ENTRIES.saturating_sub(entries) + 1;
        let listed = ws.read_dir(&folder, &mut |entry| {
            if let Ok(entry) = entry
                && found.len() < left
            {
                found.push(entry);
            }
        });
        if let Err(error) = listed {
            return Err(CommandError::of("file.unreadable", "path", format!("`{RECOVERY}/{prefix}` cannot be read: {}", io_text(&error))));
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in found {
            entries += 1;
            if entries > MAX_ENTRIES {
                truncated = true;
                break 'walk;
            }
            let name = entry.name.to_string_lossy().into_owned();
            let shown = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            match entry.kind {
                Ok(Kind::Folder) => folders.push((folder.join(&name), shown)),
                Ok(Kind::File) if has_program_extension(Path::new(&shown)) => {
                    // What is left of the budget bounds the read itself.
                    let left = MAX_RECOVERED.saturating_sub(used);
                    let Some((text, modified)) = newer_copy(session, &folder.join(&name), &shown, left, &mut truncated) else { continue };
                    used += text.len();
                    texts.push(json!({ "path": shown, "text": text, "modified": modified }));
                }
                _ => {}
            }
        }
    }
    texts.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(json!({ "texts": texts, "truncated": truncated }))
}

/// The text of a copy and its time, when its file exists and changed before it; a time the system does not give counts
/// as newer.
fn newer_copy(session: &Session, copy: &Path, shown: &str, left: usize, truncated: &mut bool) -> Option<(String, u64)> {
    let ws = &*session.ws;
    let mut file = session.root.clone();
    file.extend(shown.split('/'));
    let original = ws.link_metadata(&file).ok().filter(|metadata| metadata.kind == Kind::File)?;
    let kept = ws.metadata(copy).ok()?;
    if let (Some(original), Some(kept)) = (original.modified, kept.modified)
        && kept <= original
    {
        return None;
    }
    if usize::try_from(kept.len).unwrap_or(usize::MAX) > MAX_WRITE {
        return None;
    }
    if usize::try_from(kept.len).unwrap_or(usize::MAX) > left {
        *truncated = true;
        return None;
    }
    let max = MAX_WRITE.min(left);
    let Ok(Read::Bytes(bytes)) = ws.read(copy, max as u64) else { return None };
    if bytes.len() > max {
        *truncated = true;
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let modified = kept.modified.map_or(0, |modified| u64::try_from(modified).unwrap_or(u64::MAX));
    Some((text, modified))
}
