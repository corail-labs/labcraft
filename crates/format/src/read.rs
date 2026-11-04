//! From the bytes of a file to its model and what there is to say about it.

use labcraft_graph::{File, SourceMap};
use labcraft_types::{Interner, Limits, Report};

use crate::decode::decode;
use crate::parser::parse_file;

/// The stack that a host gives this work, at any `Limits::max_nesting` up to the ceiling
/// (`docs/architecture.md`, section 4). A thread of this size natively; in WASM, whose ceiling is lower,
/// `.cargo/config.toml` links it.
pub const STACK_SIZE: usize = if cfg!(target_arch = "wasm32") { 8 * 1024 * 1024 } else { 32 * 1024 * 1024 };

/// What reading a file gives. `file` is `None` when the report holds an error: a file that does not
/// parse has no model. The report belongs to this file alone: the caller names it with
/// `Report::set_file`, then merges it.
#[derive(Debug)]
pub struct Read {
    pub file: Option<File>,
    pub source_map: SourceMap,
    pub report: Report,
}

/// Reads one file. It never fails otherwise, whatever the bytes. A file that gets a code that stands alone
/// (`docs/cli.md`, section 1) keeps that code only, whatever warnings came before.
///
/// The stack it takes depends on `Limits::max_nesting`, and the caller provides it: see [`STACK_SIZE`].
pub fn read(bytes: &[u8], names: &mut Interner, limits: &Limits) -> Read {
    let mut report = Report::new(limits.max_report);
    let text = match decode(bytes, limits) {
        Ok(text) => text,
        Err(diagnostic) => {
            report.push(*diagnostic);
            return Read { file: None, source_map: SourceMap::default(), report };
        }
    };
    let (result, source_map) = parse_file(text, names, &mut report, limits);
    let file = match result {
        Ok(file) => Some(file),
        Err(diagnostic) => {
            if diagnostic.code().stands_alone() {
                report = Report::new(limits.max_report);
            }
            report.push(*diagnostic);
            None
        }
    };
    Read { file, source_map, report }
}
