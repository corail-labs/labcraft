//! `labcraft-cli exec` (`docs/cli.md`, section 9): one command of the registry, as an MCP tool would run it, on a
//! session of its own with no history (commands.md 9.5). Its writes reach an open IDE as external writes.

use std::io::Write as _;
use std::process::ExitCode;

use crate::args::ExecOptions;

/// What the command returned, or its error, as one JSON document on stdout: 0, or 1 for an error of the command.
pub fn run(options: &ExecOptions) -> Result<ExitCode, String> {
    let mut session = crate::mcp::session_on(&options.session, false)?;
    session.without_history();
    let (document, code) = match session.execute(&options.id, &options.params) {
        Ok(value) => (value, ExitCode::SUCCESS),
        Err(error) => (error.to_json(), ExitCode::FAILURE),
    };
    let mut text = serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_owned());
    text.push('\n');
    let _ = std::io::stdout().write_all(text.as_bytes());
    labcraft_engine::runs::let_go();
    Ok(code)
}

/// A bug outside what the session catches.
pub fn internal_failure(what: &str) -> ExitCode {
    let document = serde_json::json!({ "error": { "code": "internal", "message": format!("LC0099: the command could not go on: {what}") } });
    let _ = writeln!(std::io::stdout(), "{document}");
    ExitCode::FAILURE
}
