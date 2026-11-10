//! `labcraft-cli fmt` (`docs/cli.md`, section 2). The logic stays in `labcraft-format`, and the files and the last resort
//! in `engine` (`fmt`); here are the disk and the answer.

use std::process::ExitCode;

use labcraft_engine::fmt::{Formatted, format_paths};
use labcraft_engine::workspace::Workspace as _;
use labcraft_types::{Code, Diagnostic, FileEntry, Report, Status};

pub use labcraft_engine::panics::{keep_panics_and_print_them, keep_panics_for_the_report, last_panic};

use crate::args::FmtOptions;
use crate::disk::Disk;
use crate::output;

pub fn run(options: &FmtOptions) -> ExitCode {
    let Formatted { report, files, changed } = format_paths(&Disk, &options.paths, Disk.current().as_deref(), &options.limits, options.check);
    answer(options, report, &files, changed)
}

/// When the work could not start or ended outside any file, the answer keeps its form: a report, with `LC0099`.
pub fn internal_failure(options: &FmtOptions, what: &str) -> ExitCode {
    answer(options, failure_report(options, what), &[], 0)
}

fn failure_report(options: &FmtOptions, what: &str) -> Report {
    let mut report = Report::new(options.limits.max_report);
    report.push(Diagnostic::new(Code::Lc0099, format!("the tool could not finish: {what}"), "labcraft-cli"));
    report
}

fn answer(options: &FmtOptions, mut report: Report, files: &[FileEntry], changed: u64) -> ExitCode {
    report.finish();
    if options.json {
        output::write_json(&report.fmt_document(files, changed));
    } else {
        output::write_text(&output::fmt_text(&report, files, changed));
    }
    match report.status(false) {
        Status::Ok => ExitCode::SUCCESS,
        Status::Error => ExitCode::FAILURE,
    }
}
