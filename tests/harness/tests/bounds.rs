//! Scenario 4, point 3: `fmt` of any single file, and `check` of a folder that holds only that file, take at most 2 s and
//! 512 MB; the library of 100,000 files takes at most 60 s. The files are those of the corpus, and others built to cost the
//! most just under the limit of size.
//!
//! Scenarios 4 and 6 (`docs/architecture.md`, 11.3, 11.6): drawing a function, placed, routed and written as SVG, takes at
//! most 2 s and 512 MB for 20,000 nodes, the default of `--max-diagram-nodes`, and 1 s for 1,000 nodes and 2,000 wires.
//! `diagram` times the drawing of each function of the folder after `check`, which is not counted, on the corpus and on
//! cases built to cost the most: wires over many columns, a value or an error read everywhere, a chain of structures,
//! crossings in one channel, a waypoint for every wire.
//!
//! Scenario 4, point 3, for runs (decisions of the owner, 2026-11-11): `run.start` of a program built to cost the most, up
//! to a default limit of the run, takes at most 2 s and 512 MB, and a run that fills its memory at most `--max-memory` and 30 %.
//! Each case must end with the fault of its limit, so that a run that stops early cannot pass for a fast one.
//!
//! Scenario 4 and `docs/cli.md`, 7.4: reading a trace at the limits of reading, and working out its values over the
//! drawing with their SVG, takes at most 5 s and 512 MB, whatever the numbers it holds: a million events of a loop, values
//! kept up to the bound of what the reader keeps, a file of 512 MiB of events of other frames. The trace is made as it is
//! read, a part at a time, so that the memory measured is that of the reader. Each case must end with the code it reaches.
//!
//! In release only, since the crates of the workspace are not optimized in tests: `cargo xtask bounds`. Each case runs in a
//! process of its own, this one started again, which times its own work and, on Linux, reads the peak of its resident memory
//! (`VmHWM`). Elsewhere only the time is measured.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_diagram::{DiagramLimits, Drawn, Theme, ViewState, draw, svg};
use labcraft_format::{STACK_SIZE, format};
use labcraft_graph::Decl;
use labcraft_harness::{Embedded, HOSTILE, PROGRAMS, TYPE_ERRORS};
use labcraft_types::{Interner, Limits};

#[path = "../../../crates/format/tests/deep/mod.rs"]
#[allow(dead_code)]
mod deep;

/// The case that a started process runs, as `name:command`.
const CASE: &str = "LABCRAFT_BOUNDS_CASE";
const SECONDS: f64 = 2.0;
/// 1,000 nodes and 2,000 wires drawn (scenario 6).
const SMALL_DRAWING_SECONDS: f64 = 1.0;
const LIBRARY_SECONDS: f64 = 60.0;
const BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Fmt,
    Check,
    Diagram,
    Run,
    Trace,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Fmt => "fmt",
            Tool::Check => "check",
            Tool::Diagram => "diagram",
            Tool::Run => "run",
            Tool::Trace => "trace",
        }
    }
}

/// A folder to check, as its files.
type Folder = Vec<(String, Vec<u8>)>;

struct Case {
    name: &'static str,
    /// Several folders for a family, each one measured alone: the slowest one counts.
    folders: fn() -> Vec<Folder>,
    /// Whether `fmt` reads it: one file at a time, so not a library.
    fmt: bool,
    /// Whether its functions are drawn.
    diagram: bool,
    seconds: f64,
    /// The codes that `fmt` and `check` report on it, so that a case that stops early cannot pass for a fast one; none for a
    /// family of folders.
    codes: Option<(&'static [&'static str], &'static [&'static str])>,
}

const CASES: &[Case] = &[
    Case { name: "corpus", folders: corpus, fmt: true, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "nesting", folders: nesting, fmt: true, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "nodes-300000", folders: nodes_300000, fmt: true, diagram: false, seconds: SECONDS, codes: Some((&["LC6003"], &[])) },
    Case { name: "comment-at-the-limit", folders: comment_at_the_limit, fmt: true, diagram: false, seconds: SECONDS, codes: Some((&[], &[])) },
    Case {
        name: "names-in-random-order",
        folders: names_in_random_order,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &[])),
    },
    Case {
        name: "one-value-read-everywhere",
        folders: one_value_read_everywhere,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &[])),
    },
    Case {
        name: "one-value-read-everywhere-in-layout",
        folders: one_value_read_everywhere_in_layout,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &[])),
    },
    Case {
        name: "parameters-that-repeat",
        folders: parameters_that_repeat,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &["LC2003", "LC2019"])),
    },
    Case { name: "parameters", folders: parameters, fmt: true, diagram: false, seconds: SECONDS, codes: Some((&["LC6003"], &[])) },
    Case {
        name: "towers-of-parallel-loops",
        folders: towers_of_parallel_loops,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &[])),
    },
    Case { name: "targets", folders: targets, fmt: true, diagram: false, seconds: SECONDS, codes: Some((&[], &["LC2103"])) },
    Case { name: "chain-of-calls", folders: chain_of_calls, fmt: true, diagram: false, seconds: SECONDS, codes: Some((&[], &[])) },
    Case {
        name: "an-entry-per-parameter",
        folders: an_entry_per_parameter,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &[])),
    },
    Case {
        name: "values-that-cross-250-frames",
        folders: values_that_cross_250_frames,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &[])),
    },
    Case {
        name: "a-budget-of-comparisons-spent",
        folders: a_budget_of_comparisons_spent,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &["LC5006"])),
    },
    Case {
        name: "values-passed-into-250-frames",
        folders: values_passed_into_250_frames,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &["LC4007"])),
    },
    Case {
        name: "calls-alone-of-many-outputs",
        folders: calls_alone_of_many_outputs,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &["LC4008"])),
    },
    Case {
        name: "calls-of-many-defaults",
        folders: calls_of_many_defaults,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &["LC4008"])),
    },
    Case {
        name: "calls-of-a-faulty-signature",
        folders: calls_of_a_faulty_signature,
        fmt: true,
        diagram: false,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &["LC2107"])),
    },
    Case {
        name: "library-of-100000-files",
        folders: library_of_100000_files,
        fmt: false,
        diagram: false,
        seconds: LIBRARY_SECONDS,
        codes: Some((&[], &[])),
    },
    Case { name: "drawn-chain-of-20000-nodes", folders: drawn_chain, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "drawn-value-in-every-column", folders: drawn_value_in_every_column, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "drawn-wires-over-100-columns", folders: drawn_wires_over_100_columns, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "drawn-error-read-everywhere", folders: drawn_error_read_everywhere, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case {
        name: "drawn-crossings-in-one-channel",
        folders: drawn_crossings_in_one_channel,
        fmt: false,
        diagram: true,
        seconds: SECONDS,
        codes: None,
    },
    Case { name: "drawn-chain-of-structures", folders: drawn_chain_of_structures, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "drawn-waypoints-for-every-wire", folders: drawn_waypoints, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "drawn-match-of-many-arms", folders: drawn_match_of_many_arms, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case { name: "drawn-calls-of-huge-icons", folders: drawn_calls_of_huge_icons, fmt: false, diagram: true, seconds: SECONDS, codes: None },
    Case {
        name: "drawn-1000-nodes-2000-wires",
        folders: drawn_1000_nodes_2000_wires,
        fmt: false,
        diagram: true,
        seconds: SMALL_DRAWING_SECONDS,
        codes: None,
    },
];

/// A run measured: the files of its folder, the function, what `run.start` is given besides, the code of the fault it ends
/// with, and the most memory it may take.
struct RunBound {
    name: &'static str,
    files: fn() -> Folder,
    function: &'static str,
    /// A JSON object, the parameters of `run.start` besides `function` and `folder`.
    params: &'static str,
    code: &'static str,
    seconds: f64,
    bytes: u64,
}

/// A run whose trace reaches its bound of 256 MiB (commands.md 8): the writing adds at most 2 s (decision of the owner,
/// 2026-11-11).
const TRACED_SECONDS: f64 = 4.0;

/// 1 GiB, the default of `--max-memory`, and 30 % (decision of the owner, 2026-11-11).
const FILLED_BYTES: u64 = (1 << 30) / 10 * 13;

const RUN_BOUNDS: &[RunBound] = &[
    RunBound {
        name: "run-loops-of-10000-in-loops",
        files: loops_in_loops,
        function: "nested",
        params: "{}",
        code: "LC8002",
        seconds: SECONDS,
        bytes: BYTES,
    },
    RunBound {
        name: "run-10000-iterations-of-90-nodes",
        files: iterations_of_90_nodes,
        function: "body",
        params: "{}",
        code: "LC8001",
        seconds: SECONDS,
        bytes: BYTES,
    },
    RunBound {
        name: "run-for-over-10000000-items",
        files: for_over_many_items,
        function: "wide",
        params: "{}",
        code: "LC8002",
        seconds: SECONDS,
        bytes: BYTES,
    },
    RunBound { name: "run-calls-1024-deep", files: calls_deep, function: "deep", params: "{}", code: "LC8003", seconds: SECONDS, bytes: BYTES },
    RunBound {
        name: "run-text-that-doubles",
        files: text_that_doubles,
        function: "double",
        // Without the limit of the bytes made, which a text that doubles reaches first at 1 GiB: the memory of the run fills.
        params: r#"{"max_made": null}"#,
        code: "LC8005",
        seconds: SECONDS,
        bytes: FILLED_BYTES,
    },
    RunBound {
        name: "run-copies-of-a-large-array",
        files: copies_of_a_large_array,
        function: "copies",
        params: "{}",
        code: "LC8009",
        seconds: SECONDS,
        bytes: BYTES,
    },
    RunBound {
        name: "run-10000-queries",
        files: queries,
        function: "queries",
        params: r#"{"sim": "<labcraft>/visa/sim/dmm-230V.toml", "args": {"resource": "SIM::DMM0"}}"#,
        code: "LC8001",
        seconds: SECONDS,
        bytes: BYTES,
    },
    RunBound {
        name: "run-trace-to-its-bound",
        files: trace_to_its_bound,
        function: "arrays",
        params: "{}",
        code: "LC8002",
        seconds: TRACED_SECONDS,
        bytes: BYTES,
    },
];

/// Reading a trace, its values worked out and drawn (`docs/cli.md`, 7.4) *(proposed)*.
const TRACE_SECONDS: f64 = 5.0;

/// What takes each line of a trace as it is made.
type Line<'a> = dyn FnMut(&str) + 'a;

/// A trace read at a limit: its lines, made as they are read, and the code that reading it ends with, if any.
struct TraceBound {
    name: &'static str,
    /// The function the trace is read for.
    drawing: fn() -> labcraft_diagram::Drawing,
    /// Its lines, from what the reader keeps of that function.
    lines: fn(&labcraft_trace::Expected, &mut Line<'_>),
    code: &'static str,
}

const TRACE_BOUNDS: &[TraceBound] = &[
    TraceBound { name: "trace-of-1000000-events", drawing: walk_drawing, lines: million_events, code: "" },
    TraceBound { name: "trace-kept-to-its-bound", drawing: walk_drawing, lines: kept_to_its_bound, code: "LC7103" },
    TraceBound { name: "trace-of-512-mib", drawing: walk_drawing, lines: file_of_512_mib, code: "LC7103" },
    // A value on every wire of the chain of 20,000 nodes: every bubble placed among the others.
    TraceBound { name: "trace-on-a-chain-of-20000-nodes", drawing: chain_drawing, lines: value_of_every_node, code: "" },
];

/// The first line and the call of frame 0 of a trace of `walk` (`fuzz/trace/walk.lcf`).
fn start_of_walk(line: &mut Line<'_>) {
    line(r#"{"schema":"labcraft-trace/1","run":"r1","function":"walk","mode":"deterministic","files":[]}"#);
    line(
        r#"{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"walk","in":[{"name":"values","type":"[f64]","value":[1.0]},{"name":"start","type":"Reading","value":"Reading.Overload"}],"types":{"got":"f64","v":"f64","total":"f64","same":"f64","last":"i32","x":"f64","i":"i32","t":"f64","k":"f64","j":"i32","next.t":"f64"}}"#,
    );
}

/// A value of every wire of every node, `1.0`, as a run of a function without loops writes them.
fn value_of_every_node(expected: &labcraft_trace::Expected, line: &mut Line<'_>) {
    line(&format!(r#"{{"schema":"labcraft-trace/1","run":"r1","function":"{}","mode":"deterministic","files":[]}}"#, expected.function));
    let types: Vec<String> =
        expected.nodes.iter().flat_map(|node| node.wires.iter()).map(|wire| format!(r#""{}":"{}""#, wire.name, wire.ty)).collect();
    line(&format!(r#"{{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"{}","in":[],"types":{{{}}}}}"#, expected.function, types.join(",")));
    for (seq, node) in expected.nodes.iter().enumerate() {
        let out: Vec<String> = node.wires.iter().map(|wire| format!(r#""{}":1.0"#, wire.name)).collect();
        line(&format!(r#"{{"seq":{},"t_ms":0.0,"event":"ran","frame":0,"node":"{}","out":{{{}}}}}"#, seq + 2, node.name, out.join(",")));
    }
    line(&format!(r#"{{"seq":{},"t_ms":0.0,"event":"end","status":"ok"}}"#, expected.nodes.len() + 2));
}

/// The drawing of the chain of 20,000 nodes.
fn chain_drawing() -> labcraft_diagram::Drawing {
    let folder = drawn_chain().into_iter().next().unwrap();
    let files = folder.into_iter().map(|(shown, bytes)| SourceFile { shown, content: Content::Bytes(Cow::Owned(bytes)) }).collect();
    let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut names = Interner::new();
    let checked = check(root, &Shipped, &mut names, &Limits::default());
    match draw(&checked, 0, &names, &ViewState::default(), &DiagramLimits::default()) {
        Drawn::Drawing(drawing) => drawing,
        Drawn::NotDrawn(diagnostics) => panic!("the chain is not drawn: {diagnostics:?}"),
    }
}

/// The 1,000,000 events of a run, each a value of the loop at its own iteration, then `end`: every one is kept.
fn million_events(_: &labcraft_trace::Expected, line: &mut Line<'_>) {
    start_of_walk(line);
    for iteration in 0..999_999u64 {
        line(&format!(
            r#"{{"seq":{},"t_ms":0.0,"event":"ran","frame":0,"node":"@16","loops":["total"],"iter":[{iteration}],"out":{{"next.t":{iteration}.5}}}}"#,
            iteration + 2
        ));
    }
    line(r#"{"seq":1000001,"t_ms":0.0,"event":"end","status":"ok"}"#);
}

/// Values of 2 KiB, each kept, until what the reader keeps passes its bound of 384 MiB.
fn kept_to_its_bound(_: &labcraft_trace::Expected, line: &mut Line<'_>) {
    start_of_walk(line);
    let value = "7".repeat(2048);
    for iteration in 0..400_000u64 {
        line(&format!(
            r#"{{"seq":{},"t_ms":0.0,"event":"ran","frame":0,"node":"@16","loops":["total"],"iter":[{iteration}],"out":{{"next.t":{value}}}}}"#,
            iteration + 2
        ));
    }
}

/// Events of a frame called by frame 0, each of 600 bytes, until the file passes 512 MiB: read only as far as their frame.
fn file_of_512_mib(_: &labcraft_trace::Expected, line: &mut Line<'_>) {
    start_of_walk(line);
    line(r#"{"seq":2,"t_ms":0.0,"event":"call","frame":1,"parent":0,"node":"got","function":"core.add","in":[],"types":{}}"#);
    let padding = "x".repeat(520);
    for seq in 3..1_000_000u64 {
        line(&format!(r#"{{"seq":{seq},"t_ms":0.0,"event":"ran","frame":1,"node":"n","out":{{"p":"{padding}"}}}}"#));
    }
}

/// The drawing of `walk`, as `labcraft-cli diagram` draws it.
fn walk_drawing() -> labcraft_diagram::Drawing {
    let files =
        [("walk.lcf", &include_bytes!("../../../fuzz/trace/walk.lcf")[..]), ("Reading.lct", &include_bytes!("../../../fuzz/trace/Reading.lct")[..])];
    let files = files.iter().map(|(shown, bytes)| SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Borrowed(bytes)) }).collect();
    let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut names = Interner::new();
    let checked = check(root, &Shipped, &mut names, &Limits::default());
    let index = checked.files.iter().position(|file| file.shown == "walk.lcf").unwrap();
    match draw(&checked, index, &names, &ViewState::default(), &DiagramLimits::default()) {
        Drawn::Drawing(drawing) => drawing,
        Drawn::NotDrawn(diagnostics) => panic!("walk is not drawn: {diagnostics:?}"),
    }
}

/// A trace read a part of 64 KiB at a time, as `engine::drawings::read_trace` reads a file, then its values and their SVG:
/// the time it takes, and the codes of reading it.
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn trace_bound(case: &TraceBound) -> (Duration, String) {
    let drawing = (case.drawing)();
    let expected = labcraft_diagram::expected(&drawing);
    let start = Instant::now();
    let mut reader = labcraft_trace::Reader::new(expected.clone(), labcraft_trace::ReadLimits::default());
    let mut part: Vec<u8> = Vec::with_capacity(1 << 17);
    let mut going = true;
    (case.lines)(&expected, &mut |line: &str| {
        if !going {
            return;
        }
        part.extend_from_slice(line.as_bytes());
        part.push(b'\n');
        if part.len() >= 1 << 16 {
            going = reader.feed(&part);
            part.clear();
        }
    });
    if going {
        reader.feed(&part);
    }
    let trace = reader.finish();
    let overlay = labcraft_diagram::values(&drawing, &trace, &labcraft_diagram::Judged::default(), &labcraft_diagram::Choice::default());
    std::hint::black_box(labcraft_diagram::svg_with(&drawing, Some(&overlay), Theme::Both));
    let took = start.elapsed();
    let codes: Vec<&str> = labcraft_diagram::values::diagnostics(&trace, "t.jsonl").iter().map(|diagnostic| diagnostic.code().as_str()).collect();
    (took, codes.join(","))
}

/// The whole run: each case and command in a process of its own, then the table, then the verdict.
#[test]
#[ignore = "in release only: cargo xtask bounds"]
fn every_case_stays_within_the_bounds() {
    let mut over = Vec::new();
    println!("{:<38} {:<6} {:>8} {:>8}", "case", "", "seconds", "MB");
    for case in CASES {
        for command in [Tool::Fmt, Tool::Check, Tool::Diagram] {
            if (command == Tool::Fmt && !case.fmt) || (command == Tool::Diagram && !case.diagram) {
                continue;
            }
            let mut measure = run(case.name, command);
            // A loaded machine can be slow for a while (another build beside it): up to two more runs, the shortest kept.
            // Work well over the bound is over it in every run; work near it gets three tries instead of two.
            for _ in 0..2 {
                if measure.0 <= case.seconds {
                    break;
                }
                let again = run(case.name, command);
                if again.0 < measure.0 {
                    measure = again;
                }
            }
            let (seconds, peak, codes) = measure;
            if let (Some((fmt, check)), false) = (case.codes, command == Tool::Diagram) {
                let expected = if command == Tool::Fmt { fmt } else { check };
                let expected: BTreeSet<String> = expected.iter().map(|code| (*code).to_owned()).collect();
                if codes != expected {
                    over.push(format!("{} {}: codes {codes:?}, expected {expected:?}", case.name, command.name()));
                }
            }
            let shown = peak.map_or_else(|| "-".to_owned(), |bytes| (bytes / (1024 * 1024)).to_string());
            let listed: Vec<&str> = codes.iter().map(String::as_str).collect();
            println!("{:<38} {:<6} {seconds:>8.2} {shown:>8}  {}", case.name, command.name(), listed.join(" "));
            if seconds > case.seconds || peak.is_some_and(|bytes| bytes > BYTES) {
                over.push(format!("{} {}: {seconds:.2} s, {shown} MB", case.name, command.name()));
            }
        }
    }
    for case in RUN_BOUNDS {
        let mut measure = run(case.name, Tool::Run);
        for _ in 0..2 {
            if measure.0 <= case.seconds {
                break;
            }
            let again = run(case.name, Tool::Run);
            if again.0 < measure.0 {
                measure = again;
            }
        }
        let (seconds, peak, codes) = measure;
        if codes != BTreeSet::from([case.code.to_owned()]) {
            over.push(format!("{} run: ended with {codes:?}, expected the fault {}", case.name, case.code));
        }
        let shown = peak.map_or_else(|| "-".to_owned(), |bytes| (bytes / (1024 * 1024)).to_string());
        let listed: Vec<&str> = codes.iter().map(String::as_str).collect();
        println!("{:<38} {:<6} {seconds:>8.2} {shown:>8}  {}", case.name, "run", listed.join(" "));
        if seconds > case.seconds || peak.is_some_and(|bytes| bytes > case.bytes) {
            over.push(format!("{} run: {seconds:.2} s, {shown} MB", case.name));
        }
    }
    for case in TRACE_BOUNDS {
        let mut measure = run(case.name, Tool::Trace);
        for _ in 0..2 {
            if measure.0 <= TRACE_SECONDS {
                break;
            }
            let again = run(case.name, Tool::Trace);
            if again.0 < measure.0 {
                measure = again;
            }
        }
        let (seconds, peak, codes) = measure;
        let expected: BTreeSet<String> = case.code.split(',').filter(|code| !code.is_empty()).map(str::to_owned).collect();
        if codes != expected {
            over.push(format!("{} trace: ended with {codes:?}, expected {expected:?}", case.name));
        }
        let shown = peak.map_or_else(|| "-".to_owned(), |bytes| (bytes / (1024 * 1024)).to_string());
        let listed: Vec<&str> = codes.iter().map(String::as_str).collect();
        println!("{:<38} {:<6} {seconds:>8.2} {shown:>8}  {}", case.name, "trace", listed.join(" "));
        if seconds > TRACE_SECONDS || peak.is_some_and(|bytes| bytes > BYTES) {
            over.push(format!("{} trace: {seconds:.2} s, {shown} MB", case.name));
        }
    }
    if peak_memory().is_none() {
        println!("memory is measured on Linux only");
    }
    assert!(over.is_empty(), "over the bounds of scenario 4 ({SECONDS} s, {} MB):\n{}", BYTES / (1024 * 1024), over.join("\n"));
}

/// Starts this program again on one case; it gives back the slowest time of its folders, its peak of memory and the codes
/// reported.
fn run(name: &str, command: Tool) -> (f64, Option<u64>, BTreeSet<String>) {
    let program = std::env::current_exe().unwrap();
    let output = Command::new(program)
        .args(["one_case", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env(CASE, format!("{name}:{}", command.name()))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{name} {}: {}\n{stdout}{}", command.name(), output.status, String::from_utf8_lossy(&output.stderr));
    // The test harness writes the name of the test on the same line.
    let line = stdout
        .lines()
        .find_map(|line| line.split_once("bounds: ").map(|(_, measure)| measure))
        .unwrap_or_else(|| panic!("{name}: no measure in\n{stdout}"));
    let mut fields = line.split(' ');
    let seconds: f64 = fields.next().and_then(|field| field.parse().ok()).unwrap();
    let peak = fields.next().and_then(|field| field.parse().ok());
    let codes = fields.next().unwrap_or_default().split(',').filter(|code| !code.is_empty()).map(str::to_owned).collect();
    (seconds, peak, codes)
}

/// What a started process runs; nothing without the variable.
#[test]
#[ignore = "started by every_case_stays_within_the_bounds"]
fn one_case() {
    let Ok(which) = std::env::var(CASE) else { return };
    let (name, command) = which.split_once(':').unwrap();
    if command == "trace" {
        let case = TRACE_BOUNDS.iter().find(|case| case.name == name).unwrap();
        let (took, codes) = trace_bound(case);
        let peak = peak_memory().map_or_else(|| "-".to_owned(), |bytes| bytes.to_string());
        println!("bounds: {} {peak} {codes}", took.as_secs_f64());
        return;
    }
    if command == "run" {
        let case = RUN_BOUNDS.iter().find(|case| case.name == name).unwrap();
        let (took, code) = std::thread::Builder::new().stack_size(STACK_SIZE).spawn(|| run_bound(case)).unwrap().join().unwrap();
        let peak = peak_memory().map_or_else(|| "-".to_owned(), |bytes| bytes.to_string());
        println!("bounds: {} {peak} {code}", took.as_secs_f64());
        return;
    }
    let case = CASES.iter().find(|case| case.name == name).unwrap();
    let command = match command {
        "fmt" => Tool::Fmt,
        "diagram" => Tool::Diagram,
        _ => Tool::Check,
    };
    let folders = (case.folders)();
    // A case built for the drawing must draw each of its functions, or it would pass for a fast one.
    let drawn = name.starts_with("drawn-");
    let (slowest, codes) = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let mut codes = BTreeSet::new();
            let slowest = folders.into_iter().map(|folder| work(folder, command, drawn, &mut codes)).max().unwrap_or_default();
            (slowest, codes)
        })
        .unwrap()
        .join()
        .unwrap();
    let peak = peak_memory().map_or_else(|| "-".to_owned(), |bytes| bytes.to_string());
    println!("bounds: {} {peak} {}", slowest.as_secs_f64(), codes.into_iter().collect::<Vec<_>>().join(","));
}

/// What the program does on one folder, as far as the crates go.
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn work(folder: Folder, command: Tool, drawn: bool, codes: &mut BTreeSet<&'static str>) -> Duration {
    let limits = Limits::default();
    let start = Instant::now();
    match command {
        Tool::Fmt => {
            for (_, bytes) in &folder {
                let formatted = format(bytes, &mut Interner::new(), &limits);
                // `--check`: a canonical form that differs is `LC6001`.
                if formatted.text.as_deref().is_some_and(|text| text.as_bytes() != bytes.as_slice()) {
                    codes.insert("LC6001");
                }
                codes.extend(formatted.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()));
            }
        }
        Tool::Check => {
            let files = folder.into_iter().map(|(shown, bytes)| SourceFile { shown, content: Content::Bytes(Cow::Owned(bytes)) }).collect();
            let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
            let mut checked = check(root, &Shipped, &mut Interner::new(), &limits);
            checked.report.finish();
            std::hint::black_box(serde_json::to_string(&checked.report.check_document(checked.counts, false)).unwrap());
            codes.extend(checked.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()));
        }
        Tool::Diagram => {
            let files = folder.into_iter().map(|(shown, bytes)| SourceFile { shown, content: Content::Bytes(Cow::Owned(bytes)) }).collect();
            let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
            let mut names = Interner::new();
            let checked = check(root, &Shipped, &mut names, &limits);
            codes.extend(checked.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()));
            // Only the drawing counts: placed, routed and written; as an export draws it, and in the tabs of the viewer,
            // each within the bound.
            let mut longest = Duration::ZERO;
            for view in [ViewState::default(), ViewState { tabs: true, ..ViewState::default() }] {
                let start = Instant::now();
                for (index, file) in checked.files.iter().enumerate() {
                    match draw(&checked, index, &names, &view, &DiagramLimits::default()) {
                        Drawn::Drawing(drawing) => {
                            std::hint::black_box(svg(&drawing, Theme::Both));
                        }
                        Drawn::NotDrawn(_) => assert!(!drawn || !matches!(file.file.decl, Decl::Function(_)), "{} is not drawn", file.shown),
                    }
                }
                longest = longest.max(start.elapsed());
            }
            return longest;
        }
        // A run is measured by `run_bound`, on a session of `engine`, and a trace by `trace_bound`.
        Tool::Run | Tool::Trace => {}
    }
    start.elapsed()
}

/// A workspace in memory whose files written as they go, the trace of a run, go nowhere, as they go to the disk for the
/// command line: the memory of the process is that of the run, not that of a file of 256 MiB held in memory. Each file
/// created is still there, empty, so that the trace ends as it does on disk (renamed from its `.partial`).
struct TraceToTheDisk(Arc<labcraft_engine::Memory>);

impl labcraft_engine::Workspace for TraceToTheDisk {
    fn canonicalize(&self, path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        self.0.canonicalize(path)
    }
    fn absolute(&self, path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        self.0.absolute(path)
    }
    fn current(&self) -> Option<std::path::PathBuf> {
        self.0.current()
    }
    fn metadata(&self, path: &std::path::Path) -> std::io::Result<labcraft_engine::workspace::Metadata> {
        self.0.metadata(path)
    }
    fn link_metadata(&self, path: &std::path::Path) -> std::io::Result<labcraft_engine::workspace::Metadata> {
        self.0.link_metadata(path)
    }
    fn read_dir(&self, path: &std::path::Path, each: &mut dyn FnMut(std::io::Result<labcraft_engine::workspace::Entry>)) -> std::io::Result<()> {
        self.0.read_dir(path, each)
    }
    fn read(&self, path: &std::path::Path, max: u64) -> std::io::Result<labcraft_engine::workspace::Read> {
        self.0.read(path, max)
    }
    fn write(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        self.0.write(path, bytes)
    }
    fn create_folders(&self, path: &std::path::Path) -> std::io::Result<()> {
        self.0.create_folders(path)
    }
    fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        self.0.rename(from, to)
    }
    fn remove_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        self.0.remove_file(path)
    }
    fn create(&self, path: &std::path::Path) -> std::io::Result<Box<dyn std::io::Write + Send>> {
        self.0.write(path, b"")?;
        Ok(Box::new(std::io::sink()))
    }
    fn open(&self, path: &std::path::Path, offset: u64) -> std::io::Result<Box<dyn std::io::Read + Send>> {
        self.0.open(path, offset)
    }
    fn try_lock(&self, path: &std::path::Path) -> std::io::Result<Option<labcraft_engine::workspace::Held>> {
        self.0.try_lock(path)
    }
}

/// `run.start` of a case, on a session of `engine` whose root holds its folder in memory, its trace written to nowhere as
/// to a disk: the time it takes, judged and run, and the code of its fault.
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn run_bound(case: &RunBound) -> (Duration, String) {
    let memory = labcraft_engine::Memory::new();
    for (name, bytes) in (case.files)() {
        memory.put(format!("/bounds/{name}"), bytes);
    }
    let config = labcraft_engine::Config {
        root: "/".into(),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: labcraft_libs::shipped(),
        natives: vec![Arc::new(labcraft_visa::Visa)],
        clock: Arc::new(labcraft_engine::Still),
        process: 0,
    };
    let workspace = TraceToTheDisk(Arc::new(memory));
    let mut session = labcraft_engine::Session::new(Arc::new(workspace) as Arc<dyn labcraft_engine::Workspace>, config, false).unwrap();
    let mut params: serde_json::Value = serde_json::from_str(case.params).unwrap();
    params["function"] = case.function.into();
    params["folder"] = "bounds".into();
    let start = Instant::now();
    let report = session.execute("run.start", &params).unwrap();
    let took = start.elapsed();
    std::hint::black_box(serde_json::to_string(&report).unwrap());
    let code = report["fault"]["code"].as_str().map_or_else(|| format!("status-{}", report["status"]), str::to_owned);
    (took, code)
}

/// The peak of the resident memory of this process, in bytes.
fn peak_memory() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find_map(|line| line.strip_prefix("VmHWM:"))?;
    let kilobytes: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kilobytes * 1024)
}

/// The libraries shipped with LabCraft.
struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
    }
}

// The cases.

fn program(name: &str, text: &str) -> Folder {
    vec![(name.to_owned(), text.as_bytes().to_vec())]
}

/// Two loops of 10,000 iterations, one in the other: the limit of nodes, 1,000,000, comes first.
fn loops_in_loops() -> Folder {
    program(
        "nested.lcf",
        "labcraft 1\n\nuse core\n\n/// Two loops of 10,000 iterations, one in the other.\nfn nested() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        k = loop carry j = 0.0 {\n            m = core.add(j, 1.0)\n            done = core.ge(m, 9999.0)\n            next j = m\n            until done\n        }\n        a = core.add(i, k)\n        next i = a\n        until false\n    }\n}\n",
    )
}

/// A loop of 90 nodes in each iteration: 900,000 nodes, under their limit, when the loop reaches its 10,000 iterations.
fn iterations_of_90_nodes() -> Folder {
    let mut text = String::from(
        "labcraft 1\n\nuse core\n\n/// A loop of 10,000 iterations, each of 90 nodes.\nfn body() -> (n: f64) {\n    n = loop carry i = 0.0 {\n",
    );
    let mut previous = "i".to_owned();
    for index in 1..90 {
        text.push_str(&format!("        a{index} = core.add({previous}, 1.0)\n"));
        previous = format!("a{index}");
    }
    text.push_str(&format!("        next i = {previous}\n        until false\n    }}\n}}\n"));
    program("body.lcf", &text)
}

/// A `for` over an array of 10,000,000 items, 240 MB, which collects a value for each: the limit of nodes comes first.
fn for_over_many_items() -> Folder {
    program(
        "wide.lcf",
        "labcraft 1\n\nuse array\nuse core\n\n/// Adds one to each item of a large array.\nfn wide() -> (ys: [f64], err: error) {\n    xs, err = array.repeat(0.0, 10000000)\n    ys = for x in xs {\n        y = core.add(x, 1.0)\n        collect y\n    }\n}\n",
    )
}

/// A function that calls itself with an array one item longer, which each level copies, since it reads the array again after
/// the push (D-11): 1,024 copies of about 120 kB, until the limit of call depth.
fn calls_deep() -> Folder {
    vec![
        (
            "deep.lcf".to_owned(),
            b"labcraft 1\n\nuse array\n\n/// Starts the calls.\nfn deep() -> (n: [f64], err: error) {\n    big, err = array.repeat(0.0, 5000)\n    n, k = down(big)\n}\n".to_vec(),
        ),
        (
            "down.lcf".to_owned(),
            b"labcraft 1\n\nuse array\n\n/// Calls itself, with an array one item longer, and reads it again.\nfn down(a: [f64]) -> (n: [f64], l: i32) {\n    b = array.push(a, 1.0)\n    l = array.len(a)\n    n, m = down(b)\n}\n".to_vec(),
        ),
    ]
}

/// A text that doubles at each iteration, until the memory of the run is full.
fn text_that_doubles() -> Folder {
    program(
        "double.lcf",
        "labcraft 1\n\nuse string\n\n/// Doubles a text until the run stops it.\nfn double() -> (n: string) {\n    n = loop carry s = \"x\" {\n        t = string.concat(s, s)\n        next s = t\n        until false\n    }\n}\n",
    )
}

/// An array of 10,000,000 items, read twice in each iteration, so that `array.set` copies it each time: the limit of the
/// bytes made comes first.
fn copies_of_a_large_array() -> Folder {
    program(
        "copies.lcf",
        "labcraft 1\n\nuse array\n\n/// Sets an item of a large array at each iteration, and reads the array before the change too.\nfn copies() -> (a: [f64], err: error, k: i32) {\n    big, e = array.repeat(0.0, 10000000)\n    a, err, k = loop carry xs = big, carry g = e, carry n = 0 {\n        b, h = array.set(xs, 0, 1.0, err: g)\n        l = array.len(xs)\n        next g = h\n        next n = l\n        next xs = b\n        until false\n    }\n}\n",
    )
}

/// An array of 40 floats made by each node, until the limit of nodes: the trace writes 40,000,000 floats, and reaches its
/// bound of 256 MiB before the end.
fn trace_to_its_bound() -> Folder {
    program(
        "arrays.lcf",
        "labcraft 1

use array

/// Makes an array of 40 items at each iteration, which the trace writes.
fn arrays() -> (a: [f64], err: error) {
    xs, e = array.repeat(1.5, 2000000)
    a, err = for x in xs, carry k = [0.0], carry f = e {
        y, g = array.repeat(x, 40, err: f)
        next f = g
        next k = y
    }
}
",
    )
}

/// A multimeter read at each iteration, until the limit of iterations: what the instrument and the log keep of 10,000 queries.
fn queries() -> Folder {
    program(
        "queries.lcf",
        "labcraft 1\n\nuse core\nuse scpi\nuse visa\n\n/// Reads a multimeter until the run stops it.\nfn queries(resource: string) -> (n: f64, err: error) {\n    dmm, e1 = visa.open(resource)\n    n, s, e = loop carry total = 0.0, carry session = dmm, carry f = e1 {\n        v, s2, f2 = scpi.query_f64(session, \"MEAS:VOLT:DC?\", err: f)\n        t = core.add(total, v)\n        next f = f2\n        next session = s2\n        next total = t\n        until false\n    }\n    #close: err = visa.close(s, err: e)\n}\n",
    )
}

fn alone(name: &str, text: String) -> Vec<Folder> {
    vec![vec![(name.to_owned(), text.into_bytes())]]
}

/// Every file of the corpora and of the repository, each in a folder of its own.
fn corpus() -> Vec<Folder> {
    let cases = HOSTILE.iter().chain(TYPE_ERRORS).flat_map(|case| case.files);
    cases.chain(PROGRAMS).map(|file: &Embedded| vec![(file.path.rsplit('/').next().unwrap().to_owned(), file.bytes.to_vec())]).collect()
}

/// Each kind of nesting at the default limit, and far over it.
fn nesting() -> Vec<Folder> {
    let depth = Limits::default().max_nesting() as usize;
    let mut folders = Vec::new();
    for (_, generate) in deep::DEEP {
        for depth in [depth, 100_000] {
            let text = generate(depth);
            let name = if text.contains("\ntype ") { "deep.lct" } else { "deep.lcf" };
            folders.push(vec![(name.to_owned(), text.into_bytes())]);
        }
    }
    folders
}

const HEAD: &str = "labcraft 1\n\nuse core\n\n";

/// Room for the end of a file.
const ROOM: usize = 4096;

/// `head`, then as many lines as fit under the limit of size, then `tail`.
fn filled(head: &str, mut line: impl FnMut(usize) -> String, tail: &str) -> String {
    let limit = usize::try_from(Limits::default().max_file_size).unwrap() - ROOM;
    let mut text = String::from(head);
    for index in 0.. {
        let next = line(index);
        if text.len() + next.len() + tail.len() > limit {
            break;
        }
        text.push_str(&next);
    }
    text + tail
}

/// The Size family: under the limit only without indentation.
fn nodes_300000() -> Vec<Folder> {
    let lines: String = (0..300_000).map(|index| format!("a{index}=1\n")).collect();
    alone("f.lcf", format!("labcraft 1\nfn f()->(){{\n{lines}}}\n"))
}

fn comment_at_the_limit() -> Vec<Folder> {
    let (head, tail) = ("labcraft 1\n\n// ", "\nfn f() -> () {}\n");
    let size = usize::try_from(Limits::default().max_file_size).unwrap();
    alone("f.lcf", format!("{head}{}{tail}", "x".repeat(size - head.len() - tail.len())))
}

/// A pseudo-random sequence, the same on every run.
struct Random(u64);

impl Random {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((self.0 >> 33) % bound as u64).unwrap()
    }
}

/// Each line reads one before it, and the lines are in no order: `canonicalize` sorts them all.
fn names_in_random_order() -> Vec<Folder> {
    let mut random = Random(7);
    let text = filled(
        "",
        |index| if index == 0 { "    a0 = 1\n".to_owned() } else { format!("    a{index} = core.add(a{}, 1)\n", random.below(index)) },
        "",
    );
    let mut lines: Vec<&str> = text.lines().collect();
    for index in (1..lines.len()).rev() {
        lines.swap(index, random.below(index + 1));
    }
    alone("f.lcf", format!("{HEAD}fn f() -> () {{\n{}\n}}\n", lines.join("\n")))
}

fn one_value_read_everywhere() -> Vec<Folder> {
    alone("f.lcf", filled(&format!("{HEAD}fn f() -> () {{\n    x = 1\n"), |index| format!("    b{index} = core.add(x, 1)\n"), "}\n"))
}

/// The entry asks for the wires of a value read on every line.
fn one_value_read_everywhere_in_layout() -> Vec<Folder> {
    let tail = "}\n\nlayout {\n    wire x -> b1 via (10, 10)\n}\n";
    alone("f.lcf", filled(&format!("{HEAD}fn f() -> () {{\n    x = 1\n"), |index| format!("    b{index} = core.add(x, 1)\n"), tail))
}

/// Four bytes a parameter, each one with the same name and an unknown type.
fn parameters_that_repeat() -> Vec<Folder> {
    alone("f.lcf", filled(&format!("{HEAD}fn f("), |_| "a:b,".to_owned(), ") -> () {}\n"))
}

fn parameters() -> Vec<Folder> {
    alone("f.lcf", filled(&format!("{HEAD}fn f("), |index| format!("p{index}: f64, "), ") -> () {}\n"))
}

/// Loops 250 deep, again and again, without indentation.
fn towers_of_parallel_loops() -> Vec<Folder> {
    let tower: String = (0..250).map(|depth| format!("_=for parallel x{depth} in [1]{{\n")).collect::<String>() + "y=1\n" + &"}\n".repeat(250);
    alone("f.lcf", filled(&format!("{HEAD}fn f() -> () {{\n"), |_| tower.clone(), "}\n"))
}

/// One node with millions of targets, which `core.add` does not have (`LC2103`).
fn targets() -> Vec<Folder> {
    alone("f.lcf", filled(&format!("{HEAD}fn f() -> () {{\n    "), |index| format!("c{index}, "), "z = core.add(1, 1)\n}\n"))
}

fn chain_of_calls() -> Vec<Folder> {
    alone("f.lcf", filled(&format!("{HEAD}fn f() -> () {{\n    c0 = 1\n"), |index| format!("    c{} = core.add(c{index}, 2)\n", index + 1), "}\n"))
}

/// A wire entry on each parameter.
fn an_entry_per_parameter() -> Vec<Folder> {
    let size = usize::try_from(Limits::default().max_file_size).unwrap() - ROOM;
    let text = |count: usize| {
        let params: Vec<String> = (0..count).map(|index| format!("p{index}: f64")).collect();
        let body: String = (0..count).map(|index| format!("    q{index} = core.add(p{index}, 1)\n")).collect();
        let entries: String = (0..count).map(|index| format!("    wire p{index} -> q{index}\n")).collect();
        format!("{HEAD}fn f({}) -> () {{\n{body}}}\n\nlayout {{\n{entries}}}\n", params.join(", "))
    };
    let mut count = size / 77;
    while text(count).len() > size {
        count -= count / 20;
    }
    alone("f.lcf", text(count))
}

/// Each value is read under 250 loops and has an entry: far more wires than the file has places, so past the limit of wires,
/// where pass 6 says nothing of them.
fn values_that_cross_250_frames() -> Vec<Folder> {
    let size = usize::try_from(Limits::default().max_file_size).unwrap() - ROOM;
    let open: String = (0..250).map(|depth| format!("_=for i{depth} in [1]{{\n")).collect();
    let close = "}\n".repeat(250);
    let text = |count: usize| {
        let values: String = (0..count).map(|index| format!("v{index}=1\n")).collect();
        let reads: String = (0..count).map(|index| format!("b{index}=core.add(v{index},1)\n")).collect();
        let entries: String = (0..count).map(|index| format!("    wire v{index} -> b{index}\n")).collect();
        // Under the limit, the last entry is `LC5006`: `v0` does not go to `b1`.
        format!("{HEAD}fn f()->(){{\n{values}{open}{reads}{close}}}\n\nlayout {{\n{entries}    wire v0 -> b1\n}}\n")
    };
    let mut count = size / 60;
    while text(count).len() > size {
        count -= count / 20;
    }
    alone("f.lcf", text(count))
}

/// `x` and `y` in 100 arms, and an entry from one to the other again and again: each entry compares 10,000 pairs, until the
/// budget of pass 6 runs out. The rest of the file is an array that reads one value 2 million times.
fn a_budget_of_comparisons_spent() -> Vec<Folder> {
    let arms: String = (0..100).map(|arm| format!("{arm}=>{{\nx=1\ny=core.add(x,1)\nyield y\n}}\n")).collect();
    let head = format!("labcraft 1\nuse core\nfn f(n: i32) -> (r: i32) {{\nq=1\nr=match n {{\n{arms}_=>yield 0\n}}\na=[q");
    let tail = format!("]\n}}\nlayout {{\n{}}}\n", "wire x->y\n".repeat(15_000));
    alone("f.lcf", filled(&head, |_| ",q".to_owned(), &tail))
}

/// Inputs read under 250 loops: each passes into every loop, on its node and in its region, so the count goes past the limit of
/// `LC4007` long before the last of them, and nothing is lowered.
fn values_passed_into_250_frames() -> Vec<Folder> {
    let size = usize::try_from(Limits::default().max_file_size).unwrap() - ROOM;
    let open: String = (0..250).map(|depth| format!("_=for i{depth} in [1]{{\n")).collect();
    let close = "}\n".repeat(250);
    let text = |count: usize| {
        let inputs: Vec<String> = (0..count).map(|index| format!("a{index}:f64")).collect();
        let reads: Vec<String> = (0..count).map(|index| format!("a{index}")).collect();
        format!("{HEAD}fn f({})->(){{\n{open}x=[{}]\n{close}}}\n", inputs.join(","), reads.join(","))
    };
    let mut count = size / 10;
    while text(count).len() > size {
        count -= count / 20;
    }
    alone("f.lcf", text(count))
}

/// A function of 100,000 outputs, and 300,000 calls of it written alone, each of nine bytes: the tenth call takes the terminals
/// past the limit of `LC4008`, and what a call gives is shared by all of them, or the types alone would take 120 GB.
fn calls_alone_of_many_outputs() -> Vec<Folder> {
    let outputs: Vec<String> = (0..100_000).map(|index| format!("y{index}: f64")).collect();
    let binds: String = (0..100_000).map(|index| format!("    y{index} = x\n")).collect();
    let f = format!("labcraft 1\n\nfn f(x: f64) -> ({}) {{\n{binds}}}\n", outputs.join(", "));
    let calls = "    f(x)\n".repeat(300_000);
    let main = format!("labcraft 1\n\nfn main(x: f64) -> (z: f64) {{\n{calls}    z = x\n}}\n");
    vec![vec![("f.lcf".to_owned(), f.into_bytes()), ("main.lcf".to_owned(), main.into_bytes())]]
}

/// A function of 100,000 parameters with a default, and 300,000 calls of it with no argument, each of eight bytes: the tenth
/// call takes the terminals past the limit of `LC4008`.
fn calls_of_many_defaults() -> Vec<Folder> {
    let params: Vec<String> = (0..100_000).map(|index| format!("p{index}: f64 = 1.0")).collect();
    let f = format!("labcraft 1\n\nfn f({}) -> (y: f64) {{\n    y = p0\n}}\n", params.join(", "));
    let main = format!("labcraft 1\n\nfn main(x: f64) -> (z: f64) {{\n{}    z = x\n}}\n", "    f()\n".repeat(300_000));
    vec![vec![("f.lcf".to_owned(), f.into_bytes()), ("main.lcf".to_owned(), main.into_bytes())]]
}

/// A signature of 20,000 variadic parameters, which `LC2107` refuses, and 100,000 calls of it with no argument: its calls are
/// not counted for `LC4008`, or each would walk the 20,000.
fn calls_of_a_faulty_signature() -> Vec<Folder> {
    let params: Vec<String> = (0..20_000).map(|index| format!("a{index}: f64...")).collect();
    let f = format!("labcraft 1\n\nfn f({}) -> (y: f64) {{\n    y = 1.0\n}}\n", params.join(", "));
    let main = format!("labcraft 1\n\nfn main(x: f64) -> (z: f64) {{\n{}    z = x\n}}\n", "    f()\n".repeat(100_000));
    vec![vec![("f.lcf".to_owned(), f.into_bytes()), ("main.lcf".to_owned(), main.into_bytes())]]
}

fn library_of_100000_files() -> Vec<Folder> {
    vec![(0..100_000).map(|index| (format!("f{index}.lcf"), format!("labcraft 1\n\nfn f{index}() -> () {{\n    x = 1\n}}\n").into_bytes())).collect()]
}

// The drawings that cost the most, each under 20,000 nodes.

const DRAWN: usize = 19_990;

/// A chain of nodes, one per column.
fn drawn_chain() -> Vec<Folder> {
    let lines: String = (1..DRAWN).map(|k| format!("    v{k} = core.add(v{}, 1.0)\n", k - 1)).collect();
    alone("f.lcf", format!("{HEAD}fn f() -> () {{\n    v0 = core.neg(1.0)\n{lines}}}\n"))
}

/// An input read in every column: one net, one drop per column.
fn drawn_value_in_every_column() -> Vec<Folder> {
    let lines: String = (1..DRAWN).map(|k| format!("    v{k} = core.add(v{}, x)\n", k - 1)).collect();
    alone("f.lcf", format!("{HEAD}fn f(x: f64) -> () {{\n    v0 = core.neg(x)\n{lines}}}\n"))
}

/// Each value read again 100 columns later: 20,000 tracks, a hundred over each column.
fn drawn_wires_over_100_columns() -> Vec<Folder> {
    let lines: String = (1..DRAWN)
        .map(|k| match k {
            0..100 => format!("    v{k} = core.add(v{}, x)\n", k - 1),
            _ => format!("    v{k} = core.add(v{}, v{})\n", k - 1, k - 100),
        })
        .collect();
    alone("f.lcf", format!("{HEAD}fn f(x: f64) -> () {{\n    v0 = core.neg(x)\n{lines}}}\n"))
}

/// The function that the cases call with an error: it passes one through.
const PASS: &str = "labcraft 1\n\nuse core\n\nfn s(x: f64, err: error = none) -> (y: f64, err: error) {\n    y = core.neg(x)\n    err = err\n}\n";
const SWAP: &str = "labcraft 1\n\nuse core\n\nfn t(x: f64, z: f64) -> (y: f64, w: f64) {\n    y = core.neg(x)\n    w = core.abs(z)\n}\n";

/// One error read by every node of a chain: a track below the chain, a drop in every column.
fn drawn_error_read_everywhere() -> Vec<Folder> {
    let lines: String = (1..DRAWN).map(|k| format!("    x{k}, e{k} = s(x{}, err: err)\n", k - 1)).collect();
    let main = format!("labcraft 1\n\nuse core\n\nfn f(err: error = none) -> () {{\n    x0 = core.neg(1.0)\n{lines}}}\n");
    vec![vec![("f.lcf".to_owned(), main.into_bytes()), ("s.lcf".to_owned(), PASS.as_bytes().to_vec())]]
}

/// Four functions whose files, each under the limit of 4 MiB, hold an icon of 160 000 shapes, and one call to each: past
/// 256 shapes an icon is not drawn and its shapes are not judged one by one (spec 11.1), so neither costs its size.
fn drawn_calls_of_huge_icons() -> Vec<Folder> {
    let shapes: String = (0..160_000).map(|k| format!("    rect ({}, 1) (2, 2)\n", k % 30)).collect();
    let function = |k: usize| format!("labcraft 1\n\nfn f{k}(x: f64) -> (y: f64) {{\n    y = x\n}}\n\nicon 32 {{\n{shapes}}}\n");
    let mut files: Folder = (0..4).map(|k| (format!("f{k}.lcf"), function(k).into_bytes())).collect();
    let calls: String = (0..4).map(|k| format!("    y{k} = f{k}(x)\n")).collect();
    files.push(("main.lcf".to_owned(), format!("labcraft 1\n\nfn main(x: f64) -> () {{\n{calls}}}\n").into_bytes()));
    vec![files]
}

/// Pairs of values swapped between two calls, all in two columns: crossings in one channel, each a cycle that jogs.
/// `t` has no `err`: an optional input is drawn (notation 3.2), and four terminals a node keep the whole case under the
/// bound of terminals, five a node.
fn drawn_crossings_in_one_channel() -> Vec<Folder> {
    let lines: String = (0..DRAWN / 2).map(|k| format!("    p{k}, q{k} = t(a, b)\n    y{k}, z{k} = t(q{k}, p{k})\n")).collect();
    let main = format!("labcraft 1\n\nfn f(a: f64, b: f64) -> () {{\n{lines}}}\n");
    vec![vec![("f.lcf".to_owned(), main.into_bytes()), ("t.lcf".to_owned(), SWAP.as_bytes().to_vec())]]
}

/// A chain of loops, each holding a little body: a frame per structure, a value through each.
fn drawn_chain_of_structures() -> Vec<Folder> {
    let lines: String = (1..DRAWN / 4)
        .map(|k| {
            format!(
                "    v{k} = loop carry c = v{} {{\n        d = core.add(c, 1.0)\n        stop = core.gt(d, 0.0)\n        next c = d\n        until stop\n    }}\n",
                k - 1
            )
        })
        .collect();
    alone("f.lcf", format!("{HEAD}fn f(x: f64) -> () {{\n    v0 = core.neg(x)\n{lines}}}\n"))
}

/// A `match` whose arms all read two values from outside: unfolded, and in the tabs of the viewer, where each arm
/// draws its own margins.
fn drawn_match_of_many_arms() -> Vec<Folder> {
    let arms: String = (0..DRAWN / 2)
        .map(|arm| {
            format!(
                "        {arm} => {{
            a = core.add(x, y)
            yield a
        }}
"
            )
        })
        .collect();
    let body = format!(
        "    r = match n {{
{arms}        _ => yield x
    }}
"
    );
    alone(
        "f.lcf",
        format!(
            "{HEAD}fn f(x: f64, y: f64, n: i64) -> (r: f64) {{
{body}}}
"
        ),
    )
}

/// A waypoint for every wire, from the `layout` section.
fn drawn_waypoints() -> Vec<Folder> {
    let count = DRAWN / 2;
    let lines: String = (0..count).map(|k| format!("    y{k} = core.neg(x)\n")).collect();
    let entries: String = (0..count).map(|k| format!("    wire x -> y{k} via (100, {})\n", 40 + k % 1000)).collect();
    alone("f.lcf", format!("{HEAD}fn f(x: f64) -> () {{\n{lines}}}\n\nlayout {{\n{entries}}}\n"))
}

/// 1,000 nodes, each reading the two before it: 2,000 wires.
fn drawn_1000_nodes_2000_wires() -> Vec<Folder> {
    let lines: String = (2..1000).map(|k| format!("    v{k} = core.add(v{}, v{})\n", k - 1, k - 2)).collect();
    alone("f.lcf", format!("{HEAD}fn f(x: f64) -> () {{\n    v0 = core.neg(x)\n    v1 = core.add(v0, x)\n{lines}}}\n"))
}
