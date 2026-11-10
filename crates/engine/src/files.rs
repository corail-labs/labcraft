//! The commands of files and libraries (`docs/commands.md`, 6.2): read, write, rename, delete, format and check.
//! A command that changes a file then checks the root, so that one call writes and checks.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use labcraft_check::case_key;
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::fmt::{Formatted, format_paths};
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::session::{Session, written};
use crate::walk::io_text;
use crate::workspace::{Kind, Read};

/// At most this much text in one `file.read` *(proposed)*, cut at the end of a line.
pub const MAX_READ: usize = 1 << 20;
/// At most this much text in one `file.write` *(proposed)*, whatever `--max-file-size`.
pub const MAX_WRITE: usize = 4 << 20;

pub(crate) fn is_simulation(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
}

/// Whether the text reads as a simulation file: TOML whose keys at the top level each name a library that simulates
/// (`docs/commands.md`, section 2). An empty file is one. If not, why not, as the diagnostic of the tool would say it.
pub(crate) fn reads_as_simulation(session: &Session, text: &str) -> Result<(), String> {
    let simulating: Vec<&str> = session.config.natives.iter().filter(|library| library.simulates()).map(|library| library.name()).collect();
    let file = labcraft_runtime::simulation::read(text, session.config.limits.max_nesting()).map_err(|diagnostic| {
        let place = diagnostic.span.map(|span| format!(" (line {}, column {})", span.start.line, span.start.column)).unwrap_or_default();
        format!("{}{place}, {}", diagnostic.code, diagnostic.message)
    })?;
    match file.keys().into_iter().find(|key| !simulating.contains(key)) {
        Some(key) => Err(format!(
            "{}, `{}` names no library that simulates ({}){}",
            labcraft_runtime::simulation::NO_LIBRARY,
            labcraft_types::cut_text(key),
            simulating.join(", "),
            crate::error::did_you_mean(key, &simulating)
        )),
        None => Ok(()),
    }
}

pub(crate) fn not_simulation(parameter: &str, shown: &str, what: &str, why: &str) -> CommandError {
    CommandError::of(
        "file.not_simulation",
        parameter,
        format!(
            "`{shown}` {what} ({why}): a `.toml` file is reached only as a simulation file, TOML whose top-level keys each name a library that simulates"
        ),
    )
}

/// The bytes of a file of the project or of a shipped library, bounded by `--max-file-size`.
pub(crate) fn bytes_of(session: &Session, place: &Place, parameter: &str) -> Result<(Cow<'static, [u8]>, String), CommandError> {
    match place {
        Place::Shipped(shown) => {
            let bytes = session
                .config
                .shipped
                .file(shown)
                .ok_or_else(|| CommandError::of("path.not_found", parameter, format!("`{shown}` is no shipped file")))?;
            Ok((Cow::Borrowed(bytes), shown.clone()))
        }
        Place::Project { path, shown, exists } => {
            if !exists {
                return Err(CommandError::of("path.not_found", parameter, format!("`{shown}` does not exist")));
            }
            // A pipe or a device named `x.lcf` would block a read for ever: only a regular file is read.
            if session.ws.metadata(path).map(|metadata| metadata.kind).ok() != Some(Kind::File) {
                return Err(CommandError::of("path.invalid", parameter, format!("`{shown}` is not a regular file")));
            }
            let max = session.config.limits.max_file_size;
            match session.ws.read(path, max) {
                Ok(Read::Bytes(bytes)) if bytes.len() as u64 > max => Err(too_large(parameter, shown, bytes.len() as u64, max)),
                Ok(Read::Bytes(bytes)) => Ok((Cow::Owned(bytes), shown.clone())),
                Ok(Read::TooLarge(size)) => Err(too_large(parameter, shown, size, max)),
                Err(error) => Err(CommandError::of("file.unreadable", parameter, format!("`{shown}` cannot be read: {}", io_text(&error)))),
            }
        }
    }
}

fn too_large(parameter: &str, shown: &str, size: u64, max: u64) -> CommandError {
    CommandError::of("file.too_large", parameter, format!("`{shown}` holds {size} bytes, over the limit of {max} bytes"))
}

pub(crate) fn text_of(bytes: Cow<'static, [u8]>, parameter: &str, shown: &str) -> Result<String, CommandError> {
    String::from_utf8(bytes.into_owned()).map_err(|_| CommandError::of("file.unreadable", parameter, format!("`{shown}` is not UTF-8 text")))
}

/// The path of a file in the project, judged; a shipped file only to read.
fn project_file(session: &Session, params: &Params, parameter: &str, wanted: Wanted) -> Result<(PathBuf, String, bool), CommandError> {
    match paths::judge(&*session.ws, &session.root, params.text(parameter)?, parameter, wanted, false)? {
        Place::Project { path, shown, exists } => Ok((path, shown, exists)),
        Place::Shipped(shown) => {
            Err(CommandError::of("path.outside_project", parameter, format!("`{shown}` is shipped with LabCraft, and never written")))
        }
    }
}

/// `file.read`.
pub fn read(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let place = paths::judge(&*session.ws, &session.root, params.text("path")?, "path", Wanted::File, true)?;
    let (bytes, shown) = bytes_of(session, &place, "path")?;
    let text = text_of(bytes, "path", &shown)?;
    if let Place::Project { path, .. } = &place
        && is_simulation(path)
        && let Err(why) = reads_as_simulation(session, &text)
    {
        return Err(not_simulation("path", &shown, "does not read as a simulation file", &why));
    }
    let from = params.u64("from").unwrap_or(1);
    let wanted = params.u64("lines");
    let lines_total = text.split_inclusive('\n').count() as u64;
    let mut out = String::new();
    // As JSON writes it: a control character takes six bytes there.
    let mut used = 0usize;
    let mut count: u64 = 0;
    let mut truncated = false;
    let first = usize::try_from(from.saturating_sub(1)).unwrap_or(usize::MAX);
    for line in text.split_inclusive('\n').skip(first) {
        if wanted.is_some_and(|wanted| count >= wanted) {
            break;
        }
        let cost: usize = line.chars().map(escaped_len).sum();
        if used + cost > MAX_READ {
            truncated = true;
            if count == 0 {
                // A line longer than the bound is cut, at the end of a character.
                for c in line.chars() {
                    used += escaped_len(c);
                    if used > MAX_READ {
                        break;
                    }
                    out.push(c);
                }
                count = 1;
            }
            break;
        }
        used += cost;
        out.push_str(line);
        count += 1;
    }
    Ok(json!({ "file": shown, "text": out, "from": from, "lines": count, "lines_total": lines_total, "truncated": truncated }))
}

/// What a character takes in a JSON string.
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if u32::from(c) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// A name that differs from another of its folder only by its case or its spelling, which `check` refuses (`LC2024`).
fn refuse_case_twin(session: &Session, path: &Path, shown: &str, parameter: &str, except: Option<&Path>) -> Result<(), CommandError> {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else { return Ok(()) };
    let key = case_key(&name.to_string_lossy());
    let mut twin = None;
    let _ = session.ws.read_dir(folder, &mut |entry| {
        if let Ok(entry) = entry {
            let other = entry.name.to_string_lossy();
            let is_except = except.is_some_and(|except| except.file_name() == Some(entry.name.as_os_str()));
            if entry.name != name && !is_except && case_key(&other) == key && twin.is_none() {
                twin = Some(other.into_owned());
            }
        }
    });
    match twin {
        Some(twin) => Err(CommandError::of(
            "path.invalid",
            parameter,
            format!("`{shown}` differs from `{twin}` of the same folder only by its case or its spelling, which `check` refuses (`LC2024`)"),
        )),
        None => Ok(()),
    }
}

fn unwritable(parameter: &str, shown: &str, error: &std::io::Error) -> CommandError {
    CommandError::of("file.unwritable", parameter, format!("`{shown}` cannot be written: {}", io_text(error)))
}

/// After a change: the report of the root, or what was written.
fn then_check(session: &mut Session, check: bool, shown: &str) -> Result<Value, CommandError> {
    if check { session.root_check_document() } else { Ok(written(shown)) }
}

/// `file.write`.
pub fn write(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let (path, shown, exists) = project_file(session, params, "path", Wanted::File)?;
    let text = params.text("text")?;
    let max = session.config.limits.max_file_size;
    if text.len() > MAX_WRITE || text.len() as u64 > max {
        let limit = (MAX_WRITE as u64).min(max);
        return Err(CommandError::of("file.too_large", "text", format!("the text holds {} bytes, over the limit of {limit} bytes", text.len())));
    }
    let text = text.replace("\r\n", "\n");
    if is_simulation(&path) {
        if exists {
            let (bytes, _) = bytes_of(session, &Place::Project { path: path.clone(), shown: shown.clone(), exists }, "path")?;
            if let Err(why) = reads_as_simulation(session, &String::from_utf8_lossy(&bytes)) {
                return Err(not_simulation("path", &shown, "exists and does not read as a simulation file, so it is not replaced", &why));
            }
        }
        if let Err(why) = reads_as_simulation(session, &text) {
            return Err(not_simulation("text", &shown, "would not read as a simulation file with this text", &why));
        }
    }
    if !exists {
        refuse_case_twin(session, &path, &shown, "path", None)?;
    }
    if exists && session.ws.metadata(&path).is_ok_and(|metadata| metadata.read_only) {
        return Err(CommandError::of("file.unwritable", "path", format!("`{shown}` is read-only")));
    }
    if let Some(folder) = path.parent() {
        session.ws.create_folders(folder).map_err(|error| unwritable("path", &shown, &error))?;
    }
    session.forget();
    session.ws.write(&path, text.as_bytes()).map_err(|error| unwritable("path", &shown, &error))?;
    then_check(session, params.bool("check"), &shown)
}

/// An existing file of the project that a command may change: a simulation file only if it reads as one.
fn existing_file(session: &Session, params: &Params, parameter: &str) -> Result<(PathBuf, String), CommandError> {
    let (path, shown, exists) = project_file(session, params, parameter, Wanted::File)?;
    if !exists {
        return Err(CommandError::of("path.not_found", parameter, format!("`{shown}` does not exist")));
    }
    if session.ws.metadata(&path).map(|metadata| metadata.kind).ok() != Some(Kind::File) {
        return Err(CommandError::of("path.invalid", parameter, format!("`{shown}` is not a regular file")));
    }
    if is_simulation(&path) {
        let (bytes, _) = bytes_of(session, &Place::Project { path: path.clone(), shown: shown.clone(), exists }, parameter)?;
        if let Err(why) = reads_as_simulation(session, &String::from_utf8_lossy(&bytes)) {
            return Err(not_simulation(parameter, &shown, "does not read as a simulation file", &why));
        }
    }
    Ok((path, shown))
}

/// `file.rename`.
pub fn rename(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let (from, from_shown) = existing_file(session, params, "from")?;
    let (to, to_shown, to_exists) = project_file(session, params, "to", Wanted::File)?;
    if is_simulation(&from) != is_simulation(&to) {
        return Err(CommandError::of("path.invalid", "to", "a simulation file stays a `.toml` file, and a program never becomes one"));
    }
    // The name as given: a change of case alone keeps the folder and changes the name.
    // The last name as given, when it names the judged file but for its case: `gain.lcf` for `Gain.lcf`.
    let given = params.text("to")?.replace('\\', "/");
    let given_name = given.split('/').rfind(|name| !name.is_empty() && *name != ".").unwrap_or_default().to_owned();
    let judged_name = to.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let target = match to.parent() {
        Some(folder) if case_key(&given_name) == case_key(&judged_name) => folder.join(&given_name),
        _ => to.clone(),
    };
    let same_file = to_exists && session.ws.canonicalize(&to).ok() == session.ws.canonicalize(&from).ok();
    if to_exists && !same_file {
        return Err(CommandError::of("path.invalid", "to", format!("`{to_shown}` exists already: delete it first, or choose another name")));
    }
    if target == from {
        return Err(CommandError::of("params.invalid", "to", "`to` names the file as it is already"));
    }
    refuse_case_twin(session, &target, &to_shown, "to", Some(&from))?;
    if let Some(folder) = target.parent() {
        session.ws.create_folders(folder).map_err(|error| unwritable("to", &to_shown, &error))?;
    }
    session.forget();
    session.ws.rename(&from, &target).map_err(|error| unwritable("from", &from_shown, &error))?;
    session.root_check_document()
}

/// `file.delete`.
pub fn delete(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let (path, shown) = existing_file(session, params, "path")?;
    session.forget();
    session.ws.remove_file(&path).map_err(|error| unwritable("path", &shown, &error))?;
    session.root_check_document()
}

/// `file.format`.
pub fn format(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let (path, shown, exists) = project_file(session, params, "path", Wanted::FileOrFolder)?;
    if !exists {
        return Err(CommandError::of("path.not_found", "path", format!("`{shown}` does not exist")));
    }
    let Formatted { mut report, files, changed } =
        format_paths(&*session.ws, &[path], Some(&session.root), &session.config.limits, params.bool("check_only"));
    session.forget();
    report.finish();
    serde_json::to_value(report.fmt_document(&files, changed))
        .map_err(|error| CommandError::internal(format!("the report does not write as JSON: {error}")))
}

/// `library.check`.
pub fn library_check(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let folder = params.str("folder").unwrap_or(".");
    let (path, shown, exists) = match paths::judge(&*session.ws, &session.root, folder, "folder", Wanted::Folder, false)? {
        Place::Project { path, shown, exists } => (path, shown, exists),
        Place::Shipped(shown) => {
            return Err(CommandError::of(
                "path.outside_project",
                "folder",
                format!("`{shown}` is shipped with LabCraft: check a folder of the project"),
            ));
        }
    };
    if !exists {
        return Err(CommandError::of("path.not_found", "folder", format!("`{shown}` does not exist")));
    }
    session.check_document(&path, params.bool("deny_warnings"))
}
