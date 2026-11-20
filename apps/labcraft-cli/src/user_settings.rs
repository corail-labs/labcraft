//! The store of the user's settings on this computer (`docs/commands.md`, 9.7): `settings.toml`, in the folder that
//! `craft-settings` gives LabCraft, beside the `config.toml` of the window. The window, the command line and the MCP
//! server read and write it through here, so that each writes only the keys it changes and keeps the rest of the file.

use std::path::PathBuf;

use craft_settings::paths::{Location, Portable, Probe, resolve};
use labcraft_engine::CommandError;
use serde_json::{Map, Value};

/// The name of the file.
pub const FILE: &str = "settings.toml";

/// The most it holds *(proposed)*.
const MAX: u64 = 1 << 20;

/// Where it is; none when the folder is isolated (`LABCRAFT_CONFIG_DIR` empty) or in memory.
pub fn path() -> Option<PathBuf> {
    resolve("labcraft", &Portable::DEFAULT, &Location::Auto, &Probe::default()).dir.map(|dir| dir.join(FILE))
}

/// The keys it holds, and what is wrong with it, if anything: a file that does not read gives no key.
pub fn read() -> (Map<String, Value>, Option<String>) {
    let Some(path) = path() else { return (Map::new(), None) };
    let text = match std::fs::metadata(&path) {
        Err(_) => return (Map::new(), None),
        Ok(metadata) if metadata.len() > MAX => return (Map::new(), Some(format!("`{}` holds over {MAX} bytes", path.display()))),
        Ok(_) => match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => return (Map::new(), Some(format!("`{}` cannot be read: {error}", path.display()))),
        },
    };
    match labcraft_engine::settings::read_keys(&text) {
        Ok(keys) => (keys, None),
        Err(why) => (Map::new(), Some(format!("`{}` does not read as TOML ({why}): its settings are not used", path.display()))),
    }
}

/// Writes the keys that went from `before` to `after`, the rest of the file as it is now: what another process wrote
/// meanwhile stays. Why it could not: `file.unparsed` for a file that does not read, or holds over 1 MiB;
/// `file.unwritable` otherwise.
pub fn write(before: &Map<String, Value>, after: &Map<String, Value>) -> Result<(), CommandError> {
    let Some(path) = path() else { return Ok(()) };
    let unparsed = |why: String| CommandError::new("file.unparsed", format!("`{}` {why}: mend it, then set the key again", path.display()));
    let unwritable = |why: String| CommandError::new("file.unwritable", why);
    let text = match std::fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Ok(metadata) if metadata.len() > MAX => return Err(unparsed(format!("holds over {MAX} bytes"))),
        _ => std::fs::read_to_string(&path).map_err(|error| unwritable(format!("`{}` cannot be read: {error}", path.display())))?,
    };
    let written = labcraft_engine::settings::write_keys(&text, before, after).map_err(|why| unparsed(format!("does not read as TOML ({why})")))?;
    if written == text {
        return Ok(());
    }
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| unwritable(format!("`{}` cannot be made: {error}", folder.display())))?;
    }
    craft_atomic::atomic_write(&path, written.as_bytes()).map_err(|error| unwritable(format!("`{}` cannot be written: {error}", path.display())))
}

/// The session takes the keys of the user, and writes them each time a command changes them; it does not draw with
/// them (commands.md 9.7). What is wrong with the file, if anything.
pub fn give(session: &mut labcraft_engine::Session) -> Option<String> {
    let (keys, problem) = read();
    session.set_user_settings(&keys);
    session.draw_without_user_settings();
    session.on_user_settings(Box::new(write));
    problem
}
