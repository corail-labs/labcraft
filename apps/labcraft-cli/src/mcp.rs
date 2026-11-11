//! `labcraft-cli mcp` (`docs/mcp.md`): the MCP server over stdio, on a session of its own (D-18), headless in M0b. Here
//! are the root of the project and what it may not be, the disk, the real clock, the native libraries and the end.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use labcraft_engine::{Config, Session};

use crate::args::McpOptions;
use crate::disk::Disk;

/// Why a root is refused (`docs/mcp.md`, 1): the user's home folder, a folder above it, or the root of a drive, from
/// which an agent would reach every program under it.
fn too_wide(root: &Path, home: Option<&Path>) -> Option<&'static str> {
    if root.parent().is_none() {
        return Some("is the root of a drive");
    }
    match home {
        Some(home) if home == root => Some("is the home folder"),
        Some(home) if home.starts_with(root) => Some("holds the home folder"),
        _ => None,
    }
}

/// The session on the root, checked as it starts; the message of a wrong usage otherwise (exit 2).
pub fn session(options: &McpOptions) -> Result<Session, String> {
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
        natives: vec![Arc::new(labcraft_visa::Visa), Arc::new(labcraft_results_log::ResultsLog)],
        clock: Arc::new(crate::clock::Real::new()),
        process: std::process::id(),
    };
    Session::new(Arc::new(Disk), config, true).map_err(|error| error.message)
}

/// Serves the session until stdin closes, then lets go of the locks of this process (`engine::runs::let_go`): one
/// session lives in it.
pub fn serve(session: Session) -> ExitCode {
    let options = labcraft_mcp::Options { viewer: false, version: env!("CARGO_PKG_VERSION").to_owned(), max_request: labcraft_mcp::MAX_REQUEST };
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
