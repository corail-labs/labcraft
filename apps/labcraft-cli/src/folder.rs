//! What `diagram` and the viewer share (`docs/cli.md`, 7.1 and 7.2): a folder read and checked as `check` does, its
//! functions named from its root, the drawing of each, and the picture that shows it, with its marks, or its
//! diagnostics when it is not drawable.

use std::collections::{HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use labcraft_check::{Checked, check};
use labcraft_diagram::{DiagramLimits, Drawing, Drawn, MAX_LINES, ViewState, draw, mark, undrawn};
use labcraft_types::{CheckCounts, Code, Diagnostic, Interner, Report, cut_text};

use crate::args::CheckOptions;
use crate::check::{Host, library_of, readable_lib_paths, roots};
use crate::fmt::last_panic;
use crate::walk::{self, slashed};

/// A function: named from the root, as `run` names it, by the path of its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// Its file, as reports show it.
    pub shown: String,
    /// The path of its file under the folder, as the SVG shows it.
    pub source: String,
    /// The path of its SVG under `DIR`: that of its file, with `.svg`.
    pub relative: PathBuf,
}

/// A folder read and checked.
pub struct Loaded {
    pub report: Report,
    pub counts: CheckCounts,
    pub checked: Option<Checked>,
    pub names: Interner,
    pub functions: Vec<Function>,
    /// The folder, as reports show it; none when it could not be read.
    pub root: Option<String>,
}

/// Reads and checks the first folder of `options` with its `--lib-path`, keeping the first diagnostics of each file
/// for its picture, whatever `--max-report` lists (`docs/cli.md`, 7.2).
pub fn load(options: &CheckOptions, current: Option<&Path>) -> Loaded {
    let mut limits = options.limits.clone();
    limits.keep_per_file = MAX_LINES;
    let limits = &limits;
    let mut walked = walk::walk(&options.folders, current, limits);
    let mut report = std::mem::replace(&mut walked.report, Report::for_limits(limits));
    let lib_paths = readable_lib_paths(options, current, &mut report);
    let in_the_walk: HashSet<PathBuf> =
        walked.files.iter().map(|found| found.path.clone()).chain(walked.refused.iter().map(|refused| refused.path.clone())).collect();
    let host = Host { lib_paths: &lib_paths, in_the_walk: &in_the_walk, current, limits };
    let mut names = Interner::new();
    let Some(root) = roots(&options.folders, current).into_iter().next() else {
        return Loaded { report, counts: CheckCounts::default(), checked: None, names, functions: Vec::new(), root: None };
    };
    let functions = functions_of(&root.real, &walked);
    let has_types = walked.files.iter().any(|found| found.path.extension().is_some_and(|extension| extension == "lct"));
    let mut counts = CheckCounts::default();
    let checked = match library_of(&root.real, &root.shown, &mut walked, limits) {
        Some(source) => {
            let work = || check(source, &host, &mut names, limits);
            match panic::catch_unwind(AssertUnwindSafe(work)) {
                Ok(mut checked) => {
                    report.merge(std::mem::replace(&mut checked.report, Report::for_limits(limits)));
                    for dependency in std::mem::take(&mut checked.dependencies) {
                        report.merge(dependency.report);
                    }
                    counts = checked.counts;
                    Some(checked)
                }
                Err(_) => {
                    let message = format!("internal error, which is a bug of the tool: {}", last_panic());
                    report.push(Diagnostic::new(Code::Lc0099, message, root.shown.as_str()));
                    None
                }
            }
        }
        None => None,
    };
    if functions.is_empty() && has_types {
        report.push(Diagnostic::new(Code::Lc7003, "no function to draw here, only types", root.shown.as_str()));
    }
    Loaded { report, counts, checked, names, functions, root: Some(root.shown) }
}

/// Every `.lcf` file under the folder, read or not, named from the root by its path: `dmm_driver/measure.lcf` is
/// `dmm_driver.measure`. A file whose name is not that of its function gets `LC2023`, which keeps it from being drawn.
fn functions_of(root: &Path, walked: &walk::Walked) -> Vec<Function> {
    let found =
        walked.files.iter().map(|found| (&found.path, &found.shown)).chain(walked.refused.iter().map(|refused| (&refused.path, &refused.shown)));
    let mut functions: Vec<Function> = found
        .filter(|(path, _)| path.extension().is_some_and(|extension| extension == "lcf"))
        .filter_map(|(path, shown)| {
            let relative = path.strip_prefix(root).ok()?;
            let parts: Vec<String> = relative.with_extension("").components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
            Some(Function { name: parts.join("."), shown: shown.clone(), source: slashed(relative), relative: relative.with_extension("svg") })
        })
        .collect();
    functions.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.shown.cmp(&b.shown)));
    functions
}

/// The index of each file in `Checked::files`, by its name in reports.
pub fn files_of(checked: Option<&Checked>) -> HashMap<&str, usize> {
    checked.map(|checked| checked.files.iter().enumerate().map(|(index, file)| (file.shown.as_str(), index)).collect()).unwrap_or_default()
}

/// A drawing or none, the nodes counted, the codes that keep it from being drawn.
pub type Drawn3 = (Option<Drawing>, Option<u64>, Vec<Code>);

/// The drawing of a function, or none when it is not drawable (`docs/cli.md`, 7.1); `LC7002` and a bug of the tool
/// join the report.
pub fn drawing_of(
    function: &Function,
    checked: Option<&Checked>,
    files: &HashMap<&str, usize>,
    names: &Interner,
    view: &ViewState,
    limits: &DiagramLimits,
    report: &mut Report,
) -> Drawn3 {
    let mut codes = report.keeping_from_drawing(&function.shown);
    let (Some(checked), Some(&index)) = (checked, files.get(function.shown.as_str())) else { return (None, None, codes) };
    let nodes = labcraft_diagram::count_nodes(checked, index, names);
    if !codes.is_empty() {
        return (None, nodes, codes);
    }
    let work = || draw(checked, index, names, view, limits);
    match panic::catch_unwind(AssertUnwindSafe(work)) {
        Ok(Drawn::Drawing(drawing)) => (Some(drawing), nodes, codes),
        Ok(Drawn::NotDrawn(diagnostics)) => {
            for diagnostic in diagnostics {
                codes.push(diagnostic.code());
                report.push(diagnostic);
            }
            codes.sort();
            codes.dedup();
            (None, nodes, codes)
        }
        Err(_) => {
            let message = format!("internal error, which is a bug of the tool: {}", last_panic());
            report.push(Diagnostic::new(Code::Lc0099, message, function.shown.as_str()));
            (None, nodes, vec![Code::Lc0099])
        }
    }
}

/// What shows a function: its drawing with the marks of the first diagnostics of its file, or, when it is not drawn,
/// its frame and those diagnostics (`docs/cli.md`, 7.2).
pub fn picture(function: &Function, drawing: Option<Drawing>, report: &Report) -> Drawing {
    let of_file = report.first_of(&function.shown);
    match drawing {
        Some(mut drawing) => {
            mark(&mut drawing, &of_file);
            drawing
        }
        None => {
            let shown: Vec<Diagnostic> = of_file
                .into_iter()
                .map(|mut diagnostic| {
                    diagnostic.file.clone_from(&function.source);
                    diagnostic
                })
                .collect();
            let counts = report.counts_of(&function.shown);
            undrawn(&cut_text(&function.name), &shown, counts.errors + counts.warnings)
        }
    }
}
