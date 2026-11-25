//! `labcraft-cli mcp` (`docs/mcp.md`): the MCP server over stdio, on a session of its own (D-18), headless in M0b. Here
//! are the root of the project and what it may not be, the disk, the real clock, the native libraries and the end.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use labcraft_engine::{Config, Session};
use labcraft_runtime::io::Allowed;

use crate::args::McpOptions;
use crate::disk::Disk;

/// Why a root is refused (`docs/mcp.md`, 1): the user's home folder, a folder above it, or the root of a drive, from
/// which an agent would reach every program under it.
pub fn too_wide(root: &Path, home: Option<&Path>) -> Option<&'static str> {
    if root.parent().is_none() {
        return Some("is the root of a drive");
    }
    match home {
        Some(home) if home == root => Some("is the home folder"),
        Some(home) if home.starts_with(root) => Some("holds the home folder"),
        _ => None,
    }
}

/// Why `folder` is refused as the root of a project (`docs/mcp.md`, 1), as it is on the disk: for the server and for
/// the window of the IDE (commands.md 6.7, "The window").
pub fn too_wide_folder(folder: &Path) -> Option<&'static str> {
    let root = std::fs::canonicalize(folder).ok()?;
    let home: Option<PathBuf> = std::env::home_dir().and_then(|home| std::fs::canonicalize(home).ok());
    too_wide(&root, home.as_deref())
}

/// What `mcp` serves: the window of its project, when one shows it (ide.md 8), else a session of its own.
pub enum Started {
    Attached(std::net::TcpStream),
    /// What took the connection refused it, or did not prove it holds the token: why (exit 2), not a wrong usage.
    Refused(String),
    /// A session of its own, and what it tells the agent of the windows that show other projects.
    Headless(Box<Session>, Option<String>),
}

/// Attaches to the window of the project of `--project` or the current folder (ide.md 8; mcp.md 1), else starts a
/// session of its own on it, checked as it starts; the message of a wrong usage otherwise (exit 2), and of a window
/// that refuses the connection.
pub fn start(options: &McpOptions) -> Result<Started, String> {
    let root = root_of(options)?;
    // A right that does not read is wrong usage, attached or not.
    let allowed = allowed(options)?;
    if let Some(entry) = crate::attach::find(&root) {
        match crate::attach::connect(&entry) {
            Ok(stream) => {
                if !options.allow_hardware.is_empty() || !options.allow_paths.is_empty() {
                    let _ = writeln!(std::io::stderr(), "{ASKED_IN_THE_WINDOW}");
                }
                return Ok(Started::Attached(stream));
            }
            Err(crate::attach::NotAttached::Refused(why)) => {
                let file = crate::attach::folder().map(|folder| crate::attach::file_of(&folder, &root));
                let file = file.map_or_else(String::new, |file| format!(" (if no window of LabCraft shows it, remove `{}`)", file.display()));
                return Ok(Started::Refused(format!("the window of LabCraft that shows `{}` refused the connection: {why}{file}", root.display())));
            }
            // A file left over from a window that did not close: no process takes its port.
            Err(crate::attach::NotAttached::NoWindow) => {}
        }
    }
    let note = elsewhere(&root, &crate::attach::other_projects(&root));
    let mut session = session_at(options, root, true)?;
    session.allow_agents(allowed);
    Ok(Started::Headless(Box::new(session), note))
}

/// What the human allows the runs of the agent (io.md 4): the endpoints of `--allow-hardware`, and the folders of
/// `--allow-path` with their `..` and links resolved, each a folder that is not too wide for a project either: not the root
/// of a drive, the home folder or a folder above it (decisions of the owner, 2026-11-24); no question, since no window
/// asks. The message of a wrong usage otherwise (exit 2).
fn allowed(options: &McpOptions) -> Result<Allowed, String> {
    let home: Option<PathBuf> = std::env::home_dir().and_then(|home| std::fs::canonicalize(home).ok());
    let mut paths = Vec::new();
    for given in &options.allow_paths {
        let folder = std::fs::canonicalize(given)
            .map_err(|error| format!("--allow-path `{}` cannot be read: {}", given.display(), crate::walk::io_text(&error)))?;
        if !folder.is_dir() {
            return Err(format!("--allow-path takes a folder, and `{}` is a file", given.display()));
        }
        if let Some(why) = too_wide(&folder, home.as_deref()) {
            return Err(format!("--allow-path `{}` {why}: an agent would reach every file under it; allow a folder of the bench", given.display()));
        }
        paths.push(folder);
    }
    Ok(Allowed { endpoints: options.allow_hardware.clone(), paths, consent: None, server: true })
}

/// What the server says on stderr when it attaches to a window although the human gave rights (decision of the owner,
/// 2026-11-24): the window allows, endpoint by endpoint and folder by folder (io.md 4), the options are not used, and until
/// the window asks (#239) an attached agent reaches none.
const ASKED_IN_THE_WINDOW: &str = "labcraft-cli mcp: attached to the window of LabCraft, which allows real hardware and outside folders: \
     --allow-hardware and --allow-path are not used, and until the window asks (issue #239) the runs of the agent reach none";

/// What an agent is told when it works on a project that no window shows while windows show others (decision of the
/// owner, 2026-11-22): its edits are not seen there, and how the user gives it the project they look at.
fn elsewhere(root: &Path, others: &[String]) -> Option<String> {
    let shown: Vec<String> = others.iter().map(|project| format!("`{}`", crate::setup::plain(Path::new(project)))).collect();
    let (first, _) = others.split_first()?;
    Some(format!(
        "This server works on `{}`, which no window of LabCraft shows, so the user does not see your edits. A window of \
         LabCraft shows {}. To work on one of those, the user starts this MCP client in that folder, or gives \
         `--project {}` to `labcraft-cli mcp` in its configuration: tell them so, rather than driving the window of \
         LabCraft through its interface.",
        crate::setup::plain(root),
        shown.join(", "),
        crate::setup::plain(Path::new(first)),
    ))
}

/// The session on the root, checked as it starts; the message of a wrong usage otherwise (exit 2).
pub fn session(options: &McpOptions) -> Result<Session, String> {
    session_on(options, true)
}

/// The session on the root of `--project` or the current folder, checked in the background as it starts when
/// `background` is: that of `mcp`, and of `exec`, which runs one command.
pub fn session_on(options: &McpOptions, background: bool) -> Result<Session, String> {
    let root = root_of(options)?;
    session_at(options, root, background)
}

/// The canonical root of `--project` or the current folder, which may not be too wide.
fn root_of(options: &McpOptions) -> Result<PathBuf, String> {
    let given = match &options.project {
        Some(project) => project.clone(),
        None => std::env::current_dir().map_err(|error| format!("the current folder cannot be read: {}", crate::walk::io_text(&error)))?,
    };
    let root = std::fs::canonicalize(&given).map_err(|error| format!("`{}` cannot be read: {}", given.display(), crate::walk::io_text(&error)))?;
    if !root.is_dir() {
        return Err(format!("`{}` is not a folder", given.display()));
    }
    let home: Option<PathBuf> = std::env::home_dir().and_then(|home| std::fs::canonicalize(home).ok());
    if let Some(why) = too_wide(&root, home.as_deref()) {
        return Err(format!(
            "`{}` {why}: an agent would reach every program under it; start the server in the folder of a project, or name it with --project",
            given.display()
        ));
    }
    Ok(root)
}

fn session_at(options: &McpOptions, root: PathBuf, background: bool) -> Result<Session, String> {
    for folder in &options.lib_paths {
        if std::fs::metadata(folder).is_ok_and(|metadata| !metadata.is_dir()) {
            return Err(format!("`{}` is a file, and --lib-path takes a folder", folder.display()));
        }
    }
    let config = Config {
        root,
        lib_paths: options.lib_paths.clone(),
        limits: options.limits.clone(),
        shipped: crate::shipped(),
        natives: labcraft_libs::natives(),
        clock: Arc::new(crate::clock::Real::new()),
        process: std::process::id(),
    };
    let mut session = Session::new(Arc::new(Disk), config, background).map_err(|error| error.message)?;
    if let Some(problem) = crate::user_settings::give(&mut session) {
        let _ = writeln!(std::io::stderr(), "warning: {problem}");
    }
    Ok(session)
}

/// Serves what `mcp` started until stdin closes: relayed to the window, or a session of its own.
pub fn serve_started(started: Started) -> ExitCode {
    match started {
        Started::Headless(session, note) => {
            if let Some(note) = &note {
                let _ = writeln!(std::io::stderr(), "labcraft-cli mcp: {note}");
            }
            serve(*session, note)
        }
        Started::Refused(why) => {
            let _ = writeln!(std::io::stderr(), "error: {why}");
            ExitCode::from(2)
        }
        Started::Attached(stream) => {
            let mut output = std::io::stdout();
            match crate::attach::relay(stream, Box::new(std::io::stdin()), &mut output) {
                Ok(()) => ExitCode::SUCCESS,
                // The window closed while the agent was attached: the client starts the server again.
                Err(error) => {
                    let _ = writeln!(std::io::stderr(), "error: the window of LabCraft closed the connection: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Serves the session until stdin closes, then lets go of the locks of this process (`engine::runs::let_go`): one
/// session lives in it.
pub fn serve(session: Session, note: Option<String>) -> ExitCode {
    let options =
        labcraft_mcp::Options { viewer: false, version: env!("CARGO_PKG_VERSION").to_owned(), max_request: labcraft_mcp::MAX_REQUEST, note };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    labcraft_mcp::serve(session, &mut input, Box::new(std::io::stdout()), &options);
    labcraft_engine::runs::let_go();
    ExitCode::SUCCESS
}

/// A bug outside what the session catches: nothing can be said on stdout, which is MCP's, so it goes to stderr.
pub fn internal_failure(what: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "error: LC0099: the server could not go on: {what}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_on_a_project_no_window_shows_is_told_which_ones_are_shown() {
        assert_eq!(elsewhere(Path::new("/w/thread"), &[]), None);
        let note = elsewhere(Path::new("/w/thread"), &["/p/bench".to_owned(), "/p/other".to_owned()]).unwrap_or_default();
        assert!(note.starts_with("This server works on `/w/thread`, which no window of LabCraft shows"), "{note}");
        assert!(note.contains("A window of LabCraft shows `/p/bench`, `/p/other`."), "{note}");
        assert!(note.contains("`--project /p/bench`"), "{note}");
    }

    #[test]
    fn a_root_that_holds_the_home_folder_is_too_wide() {
        let home = Path::new("/home/ada");
        assert_eq!(too_wide(Path::new("/"), Some(home)), Some("is the root of a drive"));
        assert_eq!(too_wide(Path::new("/home"), Some(home)), Some("holds the home folder"));
        assert_eq!(too_wide(Path::new("/home/ada"), Some(home)), Some("is the home folder"));
        assert_eq!(too_wide(Path::new("/home/ada/bench"), Some(home)), None);
        assert_eq!(too_wide(Path::new("/home/adam"), Some(home)), None, "a neighbour whose name starts alike");
        assert_eq!(too_wide(Path::new("/srv/bench"), None), None);
    }
}
