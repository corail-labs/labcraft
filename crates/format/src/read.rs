//! From the bytes of a file to its model and what there is to say about it.

use labcraft_graph::{File, Outline, SourceMap};
use labcraft_types::{Interner, Limits, Report, Span};

use crate::decode::decode;
use crate::parser::parse_file;

/// The stack that a host gives this work, at any `Limits::max_nesting` up to the ceiling
/// (`docs/architecture.md`, section 4). A thread of this size natively; in WASM, whose ceiling is lower,
/// `.cargo/config.toml` links it.
pub const STACK_SIZE: usize = if cfg!(target_arch = "wasm32") { 8 * 1024 * 1024 } else { 32 * 1024 * 1024 };

/// The items of an `outline` section, one per line, read as the section of a file of their own whose names go to `names`
/// (spec 11.4): what `block.set_outline` writes. The message of the first diagnostic when they do not read.
pub fn read_outline(items: &str, names: &mut Interner) -> Result<Outline, String> {
    read_outline_mapped(items, names).map(|(outline, _)| outline).map_err(|(message, _)| message)
}

/// The line on which [`read_outline_mapped`] puts the first line of the items.
pub const OUTLINE_FIRST_LINE: u32 = 8;

/// As [`read_outline`], with where each item is: its first line on [`OUTLINE_FIRST_LINE`], its columns as written; and,
/// when they do not read, where the first diagnostic is, counted in the same way.
pub fn read_outline_mapped(items: &str, names: &mut Interner) -> Result<(Outline, SourceMap), (String, Option<Span>)> {
    let text = format!("labcraft 1\n\nfn f(x: f64) -> (y: f64) {{\n    y = x\n}}\n\noutline {{\n{items}\n}}\n");
    let found = read(text.as_bytes(), names, &Limits::default());
    match found.file {
        // Items that close the section and open another one are not items of it.
        Some(file) if file.panel.is_some() || file.layout.is_some() || file.terminals.is_some() || file.icon.is_some() || !file.end.is_empty() => {
            Err(("the items close the `outline` section".to_owned(), None))
        }
        Some(File { outline: Some(outline), .. }) => Ok((outline, found.source_map)),
        _ => Err(found
            .report
            .diagnostics()
            .first()
            .map_or_else(|| ("the items do not read".to_owned(), None), |diagnostic| (diagnostic.message.clone(), diagnostic.span))),
    }
}

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
    let mut report = Report::for_limits(limits);
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
                report = Report::for_limits(limits);
            }
            report.push(*diagnostic);
            None
        }
    };
    Read { file, source_map, report }
}
