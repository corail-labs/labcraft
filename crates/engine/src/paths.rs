//! The paths that commands take (`docs/commands.md`, section 2): relative to the root of the project and written with
//! `/`; `..` never leaves the root, an absolute path must be inside it, a link counts by its target; hidden names are out
//! of reach but `.labcraft/`; and the forms that Windows reads as devices, streams, other machines or short names are
//! refused before anything touches the files, then checked again once the path is resolved.
//!
//! These forms are refused on every system, not on Windows alone, so that a project behaves the same wherever it is
//! opened and a test of the rules runs anywhere.

use std::path::{Path, PathBuf};

use labcraft_check::kept_by_windows;

use crate::error::CommandError;
use crate::shipped::PREFIX;
use crate::workspace::Workspace;

/// A path holds at most this many names, as a walk goes 32 levels deep below the root it is given, which can itself be
/// deep on the disk *(proposed)*.
const MAX_LEVELS: usize = 64;
/// And at most this many bytes *(proposed)*.
const MAX_PATH: usize = 4096;

/// A path of a command, once judged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// In the project: where it is, and how reports show it, from the root, with `/`.
    Project { path: PathBuf, shown: String, exists: bool },
    /// A file of a shipped library, `<labcraft>/visa/open.lcf`, read only.
    Shipped(String),
}

/// What a file of a command may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// A `.lcf`, `.lct` or simulation file.
    File,
    /// A simulation file only.
    Simulation,
    /// A folder, or a program file, as `fmt` takes them.
    FileOrFolder,
    /// A library folder.
    Folder,
    /// The trace of a run, which is only read: any file.
    Trace,
}

fn outside(parameter: &str, message: impl Into<String>) -> CommandError {
    CommandError::of("path.outside_project", parameter, message)
}

fn invalid(parameter: &str, message: impl Into<String>) -> CommandError {
    CommandError::of("path.invalid", parameter, message)
}

/// The forms that Windows reads as something other than a file of the folder: refused as written, before any call.
fn refuse_windows_forms(given: &str, parameter: &str) -> Result<(), CommandError> {
    let slashed = given.replace('\\', "/");
    if slashed.starts_with("//") || slashed.starts_with("/??/") {
        return Err(invalid(parameter, format!("`{given}` names another machine or a device, not a file of the project")));
    }
    let bytes = slashed.as_bytes();
    let drive = bytes.len() >= 2 && bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':');
    if drive && bytes.get(2) != Some(&b'/') {
        return Err(invalid(parameter, format!("`{given}` is relative to the current folder of a drive: give a path from the root of the project")));
    }
    let names = slashed.split('/').skip(usize::from(drive));
    for name in names {
        if name.contains(':') {
            return Err(invalid(parameter, format!("`{name}` holds `:`, which Windows reads as a stream of a file")));
        }
        let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
        if !stem.is_empty() && kept_by_windows(stem) {
            return Err(invalid(parameter, format!("Windows keeps `{stem}` for a device, whatever its extension (`LC2025`)")));
        }
        if is_short_name(name) {
            return Err(invalid(parameter, format!("`{name}` looks like a short 8.3 name, which may name another file: give its full name")));
        }
    }
    Ok(())
}

/// `PROGRA~1`, `MYFILE~12.LCF`: a `~` followed by digits that end the name before its extension.
fn is_short_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    stem.rsplit_once('~').is_some_and(|(before, digits)| !before.is_empty() && !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

/// As the walk reads it: `.LCF` is no program, and `check` would never see the file.
fn extension_of(path: &Path) -> Option<String> {
    path.extension().map(|extension| extension.to_string_lossy().into_owned())
}

/// A name that starts with `.` is hidden, but `.labcraft`.
fn hidden(name: &str) -> bool {
    name.starts_with('.') && name != ".labcraft" && name != "." && name != ".."
}

/// Judges the path of a parameter against the root of the project, `root` being without links.
pub fn judge(ws: &dyn Workspace, root: &Path, given: &str, parameter: &str, wanted: Wanted, shipped_allowed: bool) -> Result<Place, CommandError> {
    if given.is_empty() {
        return Err(invalid(parameter, "the path is empty"));
    }
    if let Some(rest) = given.strip_prefix(PREFIX) {
        if !shipped_allowed {
            return Err(outside(parameter, "a shipped library is never written"));
        }
        if rest.split('/').any(|name| name.is_empty() || name == "." || name == "..") {
            return Err(invalid(parameter, format!("`{given}` is not the name of a shipped file")));
        }
        judge_name(Path::new(rest), given, parameter, wanted)?;
        return Ok(Place::Shipped(given.to_owned()));
    }
    if given.contains('\0') {
        return Err(invalid(parameter, "a path holds no NUL character"));
    }
    // Each missing level costs a call to the file system: a path is bounded first, as a walk is (`docs/cli.md`, 1).
    let levels = given.split(['/', '\\']).filter(|name| !name.is_empty() && *name != ".").count();
    if given.len() > MAX_PATH || levels > MAX_LEVELS {
        return Err(invalid(parameter, format!("a path holds at most {MAX_LEVELS} names and {MAX_PATH} bytes")));
    }
    refuse_windows_forms(given, parameter)?;
    let slashed = given.replace('\\', "/");
    let absolute = slashed.starts_with('/') || slashed.as_bytes().get(1) == Some(&b':');
    let relative: String = if absolute {
        let real = ws.canonicalize(Path::new(given)).ok();
        let inside = |path: &Path| path.strip_prefix(root).ok().map(|rest| rest.to_string_lossy().replace('\\', "/"));
        // An absolute path that does not exist yet is judged by the deepest folder of it that does.
        let found = real.as_deref().and_then(inside).or_else(|| {
            let mut existing = Path::new(given);
            let mut names: Vec<String> = Vec::new();
            let folder = loop {
                names.push(existing.file_name()?.to_string_lossy().into_owned());
                existing = existing.parent()?;
                if let Ok(folder) = ws.canonicalize(existing) {
                    break folder;
                }
            };
            let rest = inside(&folder)?;
            names.reverse();
            Some(if rest.is_empty() { names.join("/") } else { format!("{rest}/{}", names.join("/")) })
        });
        found.ok_or_else(|| outside(parameter, format!("`{given}` is not inside the project")))?
    } else {
        slashed
    };
    let mut parts: Vec<&str> = Vec::new();
    for name in relative.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(outside(parameter, format!("`{given}` leaves the root of the project")));
                }
            }
            name if hidden(name) => {
                return Err(outside(parameter, format!("`{name}` is hidden: files and folders whose name starts with `.` are out of reach")));
            }
            name => parts.push(name),
        }
    }
    let shown_lexical = parts.join("/");
    let mut path = root.to_path_buf();
    path.extend(&parts);
    // A link counts by its target: the deepest part that exists, resolved, must stay inside the root.
    let mut existing = path.as_path();
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    let resolved = loop {
        match ws.canonicalize(existing) {
            Ok(real) => break Some(real),
            Err(_) => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else { break None };
                rest.push(name);
                existing = parent;
            }
        }
    };
    let Some(mut real) = resolved else { return Err(outside(parameter, format!("`{given}` is not inside the project"))) };
    if !real.starts_with(root) {
        return Err(outside(parameter, format!("`{given}` leads out of the project through a link")));
    }
    let exists = rest.is_empty();
    for name in rest.iter().rev() {
        real.push(name);
    }
    let shown = match real.strip_prefix(root) {
        Ok(inside) if exists => inside.to_string_lossy().replace('\\', "/"),
        _ => shown_lexical,
    };
    // The name as resolved: a link to a device, a stream or a hidden folder is refused as its spelling would be.
    refuse_windows_forms(&shown, parameter)?;
    let shown = if shown.is_empty() { ".".to_owned() } else { shown };
    let resolved: Vec<String> = real
        .strip_prefix(root)
        .map(|inside| inside.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    if let Some(name) = resolved.iter().find(|name| hidden(name)) {
        return Err(outside(parameter, format!("`{given}` leads through a link to `{name}`, which is hidden")));
    }
    judge_kind(ws, &real, &shown, exists, parameter, wanted)?;
    Ok(Place::Project { path: real, shown, exists })
}

fn judge_kind(ws: &dyn Workspace, path: &Path, shown: &str, exists: bool, parameter: &str, wanted: Wanted) -> Result<(), CommandError> {
    let is_folder = exists && ws.metadata(path).is_ok_and(|metadata| metadata.kind == crate::workspace::Kind::Folder);
    judge_extension(path, shown, exists, is_folder, parameter, wanted)
}

/// A shipped file, judged by its name alone: it is a file.
fn judge_name(path: &Path, shown: &str, parameter: &str, wanted: Wanted) -> Result<(), CommandError> {
    judge_extension(path, shown, true, false, parameter, wanted)
}

fn judge_extension(path: &Path, shown: &str, exists: bool, is_folder: bool, parameter: &str, wanted: Wanted) -> Result<(), CommandError> {
    let extension = extension_of(path);
    let program = matches!(extension.as_deref(), Some("lcf" | "lct"));
    let simulation = extension.as_deref() == Some("toml");
    let manifest = path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("labcraft.toml"));
    if manifest {
        return Err(outside(parameter, "`labcraft.toml` is the manifest of a library, which no command writes or reads"));
    }
    match wanted {
        Wanted::Folder if exists && !is_folder => Err(invalid(parameter, format!("`{shown}` is a file, not a library folder"))),
        Wanted::Folder => Ok(()),
        Wanted::FileOrFolder if is_folder || program => Ok(()),
        Wanted::FileOrFolder => Err(invalid(parameter, format!("`{shown}` is neither a folder nor a `.lcf` or `.lct` file"))),
        _ if is_folder => Err(invalid(parameter, format!("`{shown}` is a folder, not a file"))),
        Wanted::Trace => Ok(()),
        Wanted::File if program || simulation => Ok(()),
        Wanted::Simulation if simulation => Ok(()),
        Wanted::Simulation => Err(invalid(parameter, format!("`{shown}` is not a simulation file: the extension is `.toml`"))),
        Wanted::File => Err(invalid(parameter, format!("`{shown}` is not a `.lcf`, `.lct` or simulation (`.toml`) file"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Memory;

    fn project() -> Memory {
        let memory = Memory::new();
        memory.put("/p/main.lcf", "fn main()\n");
        memory.put("/p/lib/a.lcf", "fn a()\n");
        memory.put("/p/bench.toml", "");
        memory.put("/outside/x.lcf", "fn x()\n");
        memory.put_link("/p/out", "/outside");
        memory.put_link("/p/inner", "/p/lib");
        memory.put("/p/.secret/s.lcf", "fn s()\n");
        memory.put_link("/p/plain", "/p/.secret");
        memory
    }

    fn judged(given: &str, wanted: Wanted) -> Result<Place, CommandError> {
        judge(&project(), Path::new("/p"), given, "path", wanted, true)
    }

    fn code(given: &str) -> &'static str {
        judged(given, Wanted::File).unwrap_err().code
    }

    #[test]
    fn a_path_inside_is_shown_from_the_root() {
        assert_eq!(
            judged("lib/./a.lcf", Wanted::File).unwrap(),
            Place::Project { path: PathBuf::from("/p/lib/a.lcf"), shown: "lib/a.lcf".to_owned(), exists: true }
        );
        assert_eq!(
            judged("lib\\new.lcf", Wanted::File).unwrap(),
            Place::Project { path: PathBuf::from("/p/lib/new.lcf"), shown: "lib/new.lcf".to_owned(), exists: false }
        );
        assert_eq!(
            judged("/p/main.lcf", Wanted::File).unwrap(),
            Place::Project { path: PathBuf::from("/p/main.lcf"), shown: "main.lcf".to_owned(), exists: true }
        );
    }

    #[test]
    fn a_path_that_leaves_the_root_is_outside_by_dots_by_its_spelling_or_through_a_link() {
        assert_eq!(code("../outside/x.lcf"), "path.outside_project");
        assert_eq!(code("lib/../../outside/x.lcf"), "path.outside_project");
        assert_eq!(code("/outside/x.lcf"), "path.outside_project");
        assert_eq!(code("out/x.lcf"), "path.outside_project");
        assert_eq!(code("out/new.lcf"), "path.outside_project", "a new file through a link out");
        assert!(judged("inner/a.lcf", Wanted::File).is_ok(), "a link that stays inside is followed");
    }

    #[test]
    fn hidden_names_are_out_of_reach_but_labcraft() {
        assert_eq!(code(".git/config.toml"), "path.outside_project");
        assert_eq!(code("lib/.env.lcf"), "path.outside_project");
        assert!(judged(".labcraft/runs/a.toml", Wanted::File).is_ok());
        assert_eq!(code("plain/s.lcf"), "path.outside_project", "a hidden folder through a link");
        assert_eq!(code("plain/new.lcf"), "path.outside_project");
    }

    #[test]
    fn a_path_too_deep_or_too_long_is_refused_before_anything_resolves_it() {
        assert_eq!(code(&"a/".repeat(65)), "path.invalid");
        assert_eq!(code(&format!("{}.lcf", "a".repeat(4096))), "path.invalid");
        assert!(judged(&format!("{}x.lcf", "a/".repeat(63)), Wanted::File).is_ok(), "64 names");
    }

    #[test]
    fn an_absolute_path_into_a_folder_to_make_is_inside() {
        assert_eq!(
            judged("/p/new/deeper/a.lcf", Wanted::File).unwrap(),
            Place::Project { path: PathBuf::from("/p/new/deeper/a.lcf"), shown: "new/deeper/a.lcf".to_owned(), exists: false }
        );
    }

    #[test]
    fn the_forms_windows_reads_otherwise_are_invalid_on_every_system() {
        for given in ["\\\\server\\share\\a.lcf", "//server/share/a.lcf", "\\\\?\\UNC\\s\\a.lcf", "\\\\.\\COM1", "\\??\\C:\\a.lcf", "C:a.lcf"] {
            assert_eq!(code(given), "path.invalid", "{given}");
        }
        for given in ["notes.txt:a.lcf", "nul", "NUL.lcf", "lib/com1.lcf", "con.tar.lcf", "PROGRA~1/a.lcf", "MAIN~12.LCF"] {
            assert_eq!(code(given), "path.invalid", "{given}");
        }
        assert!(judged("tilde~a.lcf", Wanted::File).is_ok(), "a tilde alone is a name");
        assert!(judged("console.lcf", Wanted::File).is_ok(), "only the names themselves are kept");
    }

    #[test]
    fn an_extension_the_command_does_not_take_is_invalid_and_the_manifest_is_out_of_reach() {
        assert_eq!(code("notes.txt"), "path.invalid");
        assert_eq!(code("B.LCF"), "path.invalid", "the walk reads `.lcf` only, so `check` would never see it");
        assert_eq!(code("b.Toml"), "path.invalid");
        assert_eq!(code("lib"), "path.invalid", "a folder is not a file");
        assert_eq!(code("labcraft.toml"), "path.outside_project");
        assert_eq!(judged("main.lcf", Wanted::Simulation).unwrap_err().code, "path.invalid");
        assert!(judged("lib", Wanted::Folder).is_ok());
        assert!(judged("lib", Wanted::FileOrFolder).is_ok());
        assert_eq!(judged("main.lcf", Wanted::Folder).unwrap_err().code, "path.invalid");
    }

    #[test]
    fn a_shipped_file_is_read_only() {
        assert_eq!(judged("<labcraft>/visa/open.lcf", Wanted::File).unwrap(), Place::Shipped("<labcraft>/visa/open.lcf".to_owned()));
        let error = judge(&project(), Path::new("/p"), "<labcraft>/visa/open.lcf", "path", Wanted::File, false).unwrap_err();
        assert_eq!(error.code, "path.outside_project");
        assert_eq!(code("<labcraft>/visa/../x.lcf"), "path.invalid");
        assert_eq!(judged("<labcraft>/visa/open.lcf", Wanted::Simulation).unwrap_err().code, "path.invalid");
    }
}
