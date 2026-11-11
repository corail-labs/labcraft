//! The command line (`docs/cli.md`).
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

use labcraft_cli::{args, check, diagram, fmt, mcp, run, walk};

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "\
usage: labcraft-cli <command>

commands:
  fmt [--check] [--json] [options] PATH...    write each file in canonical form
  check [--json] [options] FOLDER...          read each library, find what it uses, and check what it
                                              declares, each body, its types, its dataflow, and its
                                              `panel`, `layout` and `icon` sections
  diagram --svg --out DIR [--json] [options] FOLDER [FUNCTION]...
                                              draw each function of FOLDER, or those named, as an SVG
                                              under DIR, at the path of its file
  run [--json] [options] FOLDER FUNCTION      check FOLDER, then run FUNCTION against simulated
                                              instruments, deterministically or in real time, and
                                              report what it did
  mcp [options]                               serve the commands of LabCraft to an agent over MCP,
                                              on stdio, until stdin closes
  version                                     print the version
  --help                                      print this text

options of fmt:
  --check                nothing is written; a file that would change is an error

options of check:
  --deny-warnings        warnings count for the status and the exit code
  --lib-path DIR         also look for libraries in this folder; can be given several times

options of diagram:
  --lib-path DIR         as for check
  --theme light|dark     one theme only, instead of both under prefers-color-scheme
  --max-diagram-nodes N  a function with more nodes is not drawn, default 20000, at most 100000

options of run:
  --deny-warnings, --lib-path DIR             as for check
  --sim FILE             the simulation file of the bench (TOML)
  --args FILE            the values of inputs, as one JSON object
  --arg NAME=VALUE       the value of an input: 230V, \"0.1 s\", Fast, '[1, 2]'; can be given several times
  --set ELEMENT=VALUE@TIME
                         set a panel element at a time of the run: stop=true@5s; can be given several times
  --real-time            run on the real clock, not a simulated one; not with --schedule-seed or
                         --max-real-time
  --schedule-seed N      draw the order of the nodes ready at one time from N
  --timeout DURATION     abort the run after DURATION on its clock: 30s
  --max-iterations N|none, --max-nodes N|none
                         iterations of each loop (default 10000), nodes run in all (default 1000000);
                         in real time, no limit by default
  --max-call-depth N     depth of calls, default 1024, at most 4096
  --max-memory BYTES     what the values of the run hold at once, default 1 GiB, at least 1 MiB
  --max-made BYTES|none  the bytes of the values the nodes make, in all, default 2 GiB; in real time,
                         no limit by default
  --max-real-time DURATION
                         the real time a deterministic run may take, default 10 min
  --trace FILE           write the trace to FILE instead of .labcraft/runs/
  --no-trace             write no trace
  exit codes: 0 ok, 1 error or refused, 2 wrong usage, 3 aborted (Ctrl-C, --timeout), 4 fault

options of mcp:
  --project DIR          the root of the project, the current folder by default; not the home folder,
                         a folder above it or the root of a drive
  --lib-path DIR         as for check
  --json is not taken: stdout is MCP's

options of all:
  --json                 one JSON document on stdout, nothing else
  --max-report N         diagnostics listed, default 1000; the counts stay complete
  --max-file-size BYTES  limit of a file and of what is written, default 4 MiB, at most 1 GiB
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
        (Some("check"), _) => match args::parse_check(rest).and_then(|options| args::refuse_files(&options).map(|()| options)) {
            Ok(options) => run_check(&options),
            Err(message) => usage_error(&message),
        },
        (Some("diagram"), _) => match args::parse_diagram(rest).and_then(|options| args::refuse_files(&options.check).map(|()| options)) {
            Ok(options) => run_diagram(&options),
            Err(message) => usage_error(&message),
        },
        (Some("run"), _) => match args::parse_run(rest).and_then(|options| args::refuse_a_file_as_folder(&options).map(|()| options)) {
            Ok(options) => run_run(&options),
            Err(message) => usage_error(&message),
        },
        (Some("mcp"), _) => match args::parse_mcp(rest).and_then(|options| mcp::session(&options)) {
            Ok(session) => on_the_stack_of_the_work(|| mcp::serve(session), mcp::internal_failure),
            Err(message) => usage_error(&message),
        },
        _ => usage_error("unknown command"),
    }
}

fn run_fmt(options: &args::FmtOptions) -> ExitCode {
    on_the_stack_of_the_work(|| fmt::run(options), |what| fmt::internal_failure(options, what))
}

fn run_check(options: &args::CheckOptions) -> ExitCode {
    on_the_stack_of_the_work(|| check::run(options), |what| check::internal_failure(options, what))
}

fn run_run(options: &args::RunOptions) -> ExitCode {
    let wrong = std::sync::Mutex::new(None);
    let code = on_the_stack_of_the_work(
        || match run::run(options) {
            Ok(code) => code,
            Err(message) => {
                *wrong.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message);
                ExitCode::from(WRONG_USAGE)
            }
        },
        |what| run::internal_failure(options, what),
    );
    match wrong.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner) {
        Some(message) => usage_error(&message),
        None => code,
    }
}

fn run_diagram(options: &args::DiagramOptions) -> ExitCode {
    on_the_stack_of_the_work(|| diagram::run(options), |what| diagram::internal_failure(options, what))
}

/// Reading takes the stack of the deepest nesting that an option can ask for (`format::STACK_SIZE`). The work has a
/// last resort of its own, so what ends here is a bug outside it.
fn on_the_stack_of_the_work(work: impl FnOnce() -> ExitCode + Send, failed: impl Fn(&str) -> ExitCode) -> ExitCode {
    let outcome = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new().stack_size(labcraft_format::STACK_SIZE).spawn_scoped(scope, work);
        worker.map(std::thread::ScopedJoinHandle::join)
    });
    match outcome {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => failed(&format!("a bug of the tool: {}", fmt::last_panic())),
        Err(error) => failed(&format!("cannot start the work: {}", walk::io_text(&error))),
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
