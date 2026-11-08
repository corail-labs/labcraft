//! Scenario 4, point 3: `fmt` of any single file, and `check` of a folder that holds only that file, take at most 2 s and
//! 512 MB; the library of 100,000 files takes at most 60 s. The files are those of the corpus, and others built to cost the
//! most just under the limit of size.
//!
//! In release only, since the crates of the workspace are not optimized in tests: `cargo xtask bounds`. Each case runs in a
//! process of its own, this one started again, which times its own work and, on Linux, reads the peak of its resident memory
//! (`VmHWM`). Elsewhere only the time is measured.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::process::Command;
use std::time::{Duration, Instant};

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_format::{STACK_SIZE, format};
use labcraft_harness::{Embedded, HOSTILE, PROGRAMS, TYPE_ERRORS};
use labcraft_types::{Interner, Limits};

#[path = "../../../crates/format/tests/deep/mod.rs"]
#[allow(dead_code)]
mod deep;

/// The case that a started process runs, as `name:command`.
const CASE: &str = "LABCRAFT_BOUNDS_CASE";
const SECONDS: f64 = 2.0;
const LIBRARY_SECONDS: f64 = 60.0;
const BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Fmt,
    Check,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Fmt => "fmt",
            Tool::Check => "check",
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
    seconds: f64,
    /// The codes that `fmt` and `check` report on it, so that a case that stops early cannot pass for a fast one; none for a
    /// family of folders.
    codes: Option<(&'static [&'static str], &'static [&'static str])>,
}

const CASES: &[Case] = &[
    Case { name: "corpus", folders: corpus, fmt: true, seconds: SECONDS, codes: None },
    Case { name: "nesting", folders: nesting, fmt: true, seconds: SECONDS, codes: None },
    Case { name: "nodes-300000", folders: nodes_300000, fmt: true, seconds: SECONDS, codes: Some((&["LC6003"], &[])) },
    Case { name: "comment-at-the-limit", folders: comment_at_the_limit, fmt: true, seconds: SECONDS, codes: Some((&[], &[])) },
    Case { name: "names-in-random-order", folders: names_in_random_order, fmt: true, seconds: SECONDS, codes: Some((&["LC6001"], &[])) },
    Case { name: "one-value-read-everywhere", folders: one_value_read_everywhere, fmt: true, seconds: SECONDS, codes: Some((&["LC6001"], &[])) },
    Case {
        name: "one-value-read-everywhere-in-layout",
        folders: one_value_read_everywhere_in_layout,
        fmt: true,
        seconds: SECONDS,
        codes: Some((&["LC6001"], &[])),
    },
    Case {
        name: "parameters-that-repeat",
        folders: parameters_that_repeat,
        fmt: true,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &["LC2003", "LC2019"])),
    },
    Case { name: "parameters", folders: parameters, fmt: true, seconds: SECONDS, codes: Some((&["LC6003"], &[])) },
    Case { name: "towers-of-parallel-loops", folders: towers_of_parallel_loops, fmt: true, seconds: SECONDS, codes: Some((&["LC6003"], &[])) },
    Case { name: "targets", folders: targets, fmt: true, seconds: SECONDS, codes: Some((&[], &["LC2103"])) },
    Case { name: "chain-of-calls", folders: chain_of_calls, fmt: true, seconds: SECONDS, codes: Some((&[], &[])) },
    Case { name: "an-entry-per-parameter", folders: an_entry_per_parameter, fmt: true, seconds: SECONDS, codes: Some((&["LC6001"], &[])) },
    Case {
        name: "values-that-cross-250-frames",
        folders: values_that_cross_250_frames,
        fmt: true,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &[])),
    },
    Case {
        name: "a-budget-of-comparisons-spent",
        folders: a_budget_of_comparisons_spent,
        fmt: true,
        seconds: SECONDS,
        codes: Some((&["LC6003"], &["LC5006"])),
    },
    Case { name: "library-of-100000-files", folders: library_of_100000_files, fmt: false, seconds: LIBRARY_SECONDS, codes: Some((&[], &[])) },
];

/// The whole run: each case and command in a process of its own, then the table, then the verdict.
#[test]
#[ignore = "in release only: cargo xtask bounds"]
fn every_case_stays_within_the_bounds() {
    let mut over = Vec::new();
    println!("{:<38} {:<6} {:>8} {:>8}", "case", "", "seconds", "MB");
    for case in CASES {
        for command in [Tool::Fmt, Tool::Check] {
            if command == Tool::Fmt && !case.fmt {
                continue;
            }
            let mut measure = run(case.name, command);
            // A loaded runner can be slow once: a second run that is under the bound settles it.
            if measure.0 > case.seconds {
                measure = run(case.name, command);
            }
            let (seconds, peak, codes) = measure;
            if let Some((fmt, check)) = case.codes {
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
    let case = CASES.iter().find(|case| case.name == name).unwrap();
    let command = if command == "fmt" { Tool::Fmt } else { Tool::Check };
    let folders = (case.folders)();
    let (slowest, codes) = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let mut codes = BTreeSet::new();
            let slowest = folders.into_iter().map(|folder| work(folder, command, &mut codes)).max().unwrap_or_default();
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
fn work(folder: Folder, command: Tool, codes: &mut BTreeSet<&'static str>) -> Duration {
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
    }
    start.elapsed()
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

fn library_of_100000_files() -> Vec<Folder> {
    vec![(0..100_000).map(|index| (format!("f{index}.lcf"), format!("labcraft 1\n\nfn f{index}() -> () {{\n    x = 1\n}}\n").into_bytes())).collect()]
}
