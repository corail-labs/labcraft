//! The command line (`docs/cli.md`). `fmt` and `check` arrive with steps 6 and 7 of
//! `docs/architecture.md`, section 9.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "\
usage: labcraft-cli <command>

commands:
  fmt [--check] [--json] [options] PATH...    not available yet
  check [--json] [options] FOLDER...          not available yet
  version                                     print the version
  --help                                      print this text
";

/// Exit code of wrong usage (`docs/cli.md`, section 1).
const WRONG_USAGE: u8 = 2;

fn main() -> ExitCode {
    // `args_os`: `args` panics on an argument that is not Unicode, and an argument is hostile input.
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["version"] => stdout(&format!("labcraft-cli {}\n", env!("CARGO_PKG_VERSION"))),
        ["--help"] => stdout(USAGE),
        ["fmt" | "check", ..] => usage_error("this command is not available yet"),
        _ => usage_error("unknown command"),
    }
}

/// `print!` panics when stdout is closed, as in `labcraft-cli version | head -0`; this does not.
fn stdout(text: &str) -> ExitCode {
    let _ = std::io::stdout().write_all(text.as_bytes());
    ExitCode::SUCCESS
}

fn usage_error(message: &str) -> ExitCode {
    let _ = write!(std::io::stderr(), "error: {message}\n\n{USAGE}");
    ExitCode::from(WRONG_USAGE)
}
