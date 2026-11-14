//! The figures of the viewer that CI reports, without a threshold (`docs/visual-notation.md`, 2.1 and 7;
//! `docs/architecture.md`, 11.6), for each function of `examples/`:
//!
//! - its size, and the zoom at which it fits whole in the canvas of the window of M0v, 1340 × 560 units;
//! - the figure of stability: on each data wire between two nodes, a neutral node `core.select(true, v, v)` is put, the
//!   drawing is compared with the one before, and two counts are kept: the nodes whose place changes, which glide on
//!   screen, and the nodes whose column holds other nodes, or in another order, than before (a column that only moves
//!   across as a whole does not count).
//!
//! `cargo xtask figures` runs it and writes its table in Markdown, which the workflow `checks` adds to its summary.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use labcraft_check::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_diagram::{DiagramLimits, Drawn, Placed, ViewState, data_wires, draw};
use labcraft_graph::Decl;
use labcraft_harness::PROGRAMS;
use labcraft_types::{Interner, Limits};

/// Where `cargo xtask figures` wants the table.
const OUTPUT: &str = "LABCRAFT_FIGURES";
/// The canvas of the window of M0v (notation 7).
const CANVAS: (i64, i64) = (134_000, 56_000);
/// The name of the neutral node.
const PROBE: &str = "stability_probe";

struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
    }
}

/// The files of `examples/`, one of them replaced.
fn examples(replaced: Option<(&str, &str)>) -> LibrarySource {
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    let mut folders: BTreeMap<String, Vec<SourceFile>> = BTreeMap::new();
    for program in PROGRAMS.iter().filter(|program| program.path.starts_with("examples/")) {
        let relative = program.path.trim_start_matches("examples/");
        let content = match replaced {
            Some((path, text)) if path == relative => Content::Bytes(Cow::Owned(text.as_bytes().to_vec())),
            _ => Content::Bytes(Cow::Borrowed(program.bytes)),
        };
        let file = SourceFile { shown: relative.to_owned(), content };
        match relative.rsplit_once('/') {
            Some((folder, _)) => folders.entry(folder.to_owned()).or_default().push(file),
            None => root.files.push(file),
        }
    }
    for (name, files) in folders {
        root.sub_libraries.push(LibrarySource { files, ..LibrarySource::new(name.clone(), name, Origin::Project) });
    }
    root
}

fn placed(checked: &Checked, file: usize, names: &Interner) -> Option<HashMap<String, Placed>> {
    let Drawn::Drawing(drawing) = draw(checked, file, names, &ViewState::default(), &DiagramLimits::default()) else { return None };
    Some(drawing.placed.into_iter().map(|placed| (placed.key.clone(), placed)).collect())
}

/// The text with a neutral node on the wire that `line:column` to `end` reads, put on the line before its reader.
fn probed(text: &str, read: (u32, u32, u32), reader_line: u32) -> Option<String> {
    let (line, start, end) = read;
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    let reading = lines.get_mut(line as usize - 1)?;
    let chars: Vec<char> = reading.chars().collect();
    let value: String = chars.get(start as usize - 1..end as usize - 1)?.iter().collect();
    let before: String = chars.get(..start as usize - 1)?.iter().collect();
    let after: String = chars.get(end as usize - 1..)?.iter().collect();
    *reading = format!("{before}{PROBE}{after}");
    let reader = lines.get(reader_line as usize - 1)?;
    let indent: String = reader.chars().take_while(|c| *c == ' ').collect();
    lines.insert(reader_line as usize - 1, format!("{indent}{PROBE} = core.select(true, {value}, {value})"));
    if !lines.iter().any(|line| line == "use core") {
        let at = lines.iter().position(|line| line.starts_with("use ")).unwrap_or(2);
        lines.insert(at, "use core".to_owned());
    }
    Some(lines.join("\n"))
}

/// The nodes that moved, and those whose column changed, between two drawings of a function.
fn changes(before: &HashMap<String, Placed>, after: &HashMap<String, Placed>) -> (usize, usize) {
    let kept = |placed: &Placed| !placed.key.rsplit('/').next().is_some_and(|last| last == PROBE);
    let columns = |drawing: &HashMap<String, Placed>| -> HashMap<(usize, u32), Vec<(u32, String)>> {
        let mut columns: HashMap<(usize, u32), Vec<(u32, String)>> = HashMap::new();
        for placed in drawing.values().filter(|placed| kept(placed)) {
            columns.entry((placed.body, placed.column)).or_default().push((placed.rank, placed.key.clone()));
        }
        for column in columns.values_mut() {
            column.sort();
        }
        columns
    };
    let (old, new) = (columns(before), columns(after));
    let mates = |columns: &HashMap<(usize, u32), Vec<(u32, String)>>, placed: &Placed| -> Vec<String> {
        columns.get(&(placed.body, placed.column)).map_or_else(Vec::new, |column| column.iter().map(|(_, key)| key.clone()).collect())
    };
    let (mut moved, mut reordered) = (0, 0);
    for (key, was) in before {
        let Some(now) = after.get(key) else { continue };
        if (was.x, was.y) != (now.x, now.y) {
            moved += 1;
        }
        // The body of a node can shift when the probe is a line of its own body: compare by the keys.
        if mates(&old, was) != mates(&new, now) {
            reordered += 1;
        }
    }
    (moved, reordered)
}

#[test]
#[ignore = "run by `cargo xtask figures`"]
fn the_figures_of_the_examples() {
    let limits = Limits::default();
    let mut names = Interner::new();
    let checked = check(examples(None), &Shipped, &mut names, &limits);
    let mut rows = Vec::new();
    for (index, file) in checked.files.iter().enumerate() {
        if !matches!(file.file.decl, Decl::Function(_)) {
            continue;
        }
        let Drawn::Drawing(drawing) = draw(&checked, index, &names, &ViewState::default(), &DiagramLimits::default()) else { continue };
        let before: HashMap<String, Placed> = drawing.placed.iter().map(|placed| (placed.key.clone(), placed.clone())).collect();
        let zoom = (CANVAS.0 * 100 / drawing.width.max(1)).min(CANVAS.1 * 100 / drawing.height.max(1)).min(100);
        let text = PROGRAMS
            .iter()
            .find(|program| program.path == format!("examples/{}", file.shown.replace('\\', "/")))
            .map(|program| String::from_utf8_lossy(program.bytes).into_owned());
        let (mut probes, mut moved, mut reordered, mut most_moved, mut most_reordered, mut failed) = (0, 0, 0, 0, 0, 0);
        for wire in data_wires(&checked, index, &names, &DiagramLimits::default()) {
            let (Some(text), Some(read), Some(reader)) = (&text, file.source_map.span(wire.read), file.source_map.span(wire.statement)) else {
                continue;
            };
            if read.start.line != read.end.line {
                failed += 1;
                continue;
            }
            let Some(changed) = probed(text, (read.start.line, read.start.column, read.end.column), reader.start.line) else {
                failed += 1;
                continue;
            };
            let mut probe_names = Interner::new();
            let probed_checked = check(examples(Some((&file.shown.replace('\\', "/"), &changed))), &Shipped, &mut probe_names, &limits);
            let probed_file = probed_checked.files.iter().position(|other| other.shown == file.shown);
            let after = probed_file.filter(|&at| probed_checked.counts_of(at).errors == 0).and_then(|at| placed(&probed_checked, at, &probe_names));
            let Some(after) = after else {
                failed += 1;
                continue;
            };
            let (m, r) = changes(&before, &after);
            probes += 1;
            moved += m;
            reordered += r;
            most_moved = most_moved.max(m);
            most_reordered = most_reordered.max(r);
        }
        let mean = |sum: usize| if probes == 0 { "–".to_owned() } else { format!("{:.1}", sum as f64 / probes as f64) };
        rows.push(format!(
            "| `{}` | {} | {} × {} | {zoom} % | {probes}{} | {} / {most_moved} | {} / {most_reordered} |",
            drawing.function,
            drawing.placed.len(),
            drawing.width / 100,
            drawing.height / 100,
            if failed > 0 { format!(" ({failed} not probed)") } else { String::new() },
            mean(moved),
            mean(reordered),
        ));
    }
    let table = format!(
        "## Figures of the viewer\n\nFor each function of `examples/`: its size in units, the zoom at which it fits in 1340 × 560 \
         (notation 7), and the figure of stability (notation 2.1): over the data wires probed, the nodes that move (mean / most) \
         and the nodes whose column changes (mean / most). No threshold in M0 (notation 2.1).\n\n\
         | Function | Nodes | Size | Zoom to fit | Wires probed | Moved | Column changed |\n|---|---|---|---|---|---|---|\n{}\n",
        rows.join("\n")
    );
    match std::env::var_os(OUTPUT) {
        Some(path) => std::fs::write(path, table).unwrap(),
        None => print!("{table}"),
    }
}
