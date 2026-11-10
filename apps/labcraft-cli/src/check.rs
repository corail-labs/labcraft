//! `labcraft-cli check` (`docs/cli.md`, section 3). The logic stays in `labcraft-check`, and the folders, the files and
//! the libraries found through `--lib-path` in `engine` (`folders`); here are the disk and the answer.

use std::process::ExitCode;

use labcraft_engine::folders::{Folders, check_folders};
use labcraft_engine::workspace::Workspace as _;
use labcraft_types::{CheckCounts, Code, Diagnostic, Report, Status};

use crate::args::CheckOptions;
use crate::disk::Disk;
use crate::output;

pub fn run(options: &CheckOptions) -> ExitCode {
    let shipped = crate::shipped();
    let given = Folders { folders: &options.folders, lib_paths: &options.lib_paths, current: Disk.current(), limits: &options.limits };
    let (report, counts) = check_folders(&Disk, &shipped, &given);
    answer(options, report, counts)
}

/// When the work could not start or ended outside any folder, the answer keeps its form: a report, with `LC0099`.
pub fn internal_failure(options: &CheckOptions, what: &str) -> ExitCode {
    let mut report = Report::new(options.limits.max_report);
    report.push(Diagnostic::new(Code::Lc0099, format!("the tool could not finish: {what}"), "labcraft-cli"));
    answer(options, report, CheckCounts::default())
}

fn answer(options: &CheckOptions, mut report: Report, counts: CheckCounts) -> ExitCode {
    report.finish();
    if options.json {
        output::write_json(&report.check_document(counts, options.deny_warnings));
    } else {
        output::write_text(&output::check_text(&report, counts));
    }
    match report.status(options.deny_warnings) {
        Status::Ok => ExitCode::SUCCESS,
        Status::Error => ExitCode::FAILURE,
    }
}
