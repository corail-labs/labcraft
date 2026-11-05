//! The command line (`docs/cli.md`). `check` arrives with step 7 of `docs/architecture.md`, section 9.
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

mod args;
mod fmt;
mod output;
mod walk;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "\
usage: labcraft-cli <command>

commands:
  fmt [--check] [--json] [options] PATH...    write each file in canonical form
  check [--json] [options] FOLDER...          not available yet
  version                                     print the version
  --help                                      print this text

options of fmt:
  --check                nothing is written; a file that would change is an error
  --json                 one JSON document on stdout, nothing else
  --max-report N         diagnostics listed, default 1000; the counts stay complete
  --max-file-size BYTES  limit of a file and of what is written, default 16 MiB, at most 1 GiB
  --max-nesting N        depth of nested structures, default 256, at most 1024
  --                     what follows is a path, even if it starts with -
";

/// Exit code of wrong usage (`docs/cli.md`, section 1).
const WRONG_USAGE: u8 = 2;

fn main() -> ExitCode {
    fmt::keep_panics_for_the_report();
    // `args_os`: `args` panics on an argument that is not Unicode, and a path need not be.
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        return usage_error("no command");
    };
    match (command.to_str(), rest) {
        (Some("version"), []) => stdout(&format!("labcraft-cli {}\n", env!("CARGO_PKG_VERSION"))),
        (Some("--help"), []) => stdout(USAGE),
        (Some("fmt"), _) => match args::parse_fmt(rest) {
            Ok(options) => run_fmt(&options),
            Err(message) => usage_error(&message),
        },
        (Some("check"), _) => usage_error("this command is not available yet"),
        _ => usage_error("unknown command"),
    }
}

/// Reading takes the stack of the deepest nesting that an option can ask for (`format::STACK_SIZE`). Each file has a
/// last resort of its own, so what ends here is a bug outside the work on a file.
fn run_fmt(options: &args::FmtOptions) -> ExitCode {
    let outcome = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new().stack_size(labcraft_format::STACK_SIZE).spawn_scoped(scope, || fmt::run(options));
        worker.map(std::thread::ScopedJoinHandle::join)
    });
    match outcome {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => fmt::internal_failure(options, &format!("a bug of the tool: {}", fmt::last_panic())),
        Err(error) => fmt::internal_failure(options, &format!("cannot start the work: {}", walk::io_text(&error))),
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
