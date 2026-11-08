//! On any input, `check` ends without an internal error, says the same thing whatever the order of the files, with
//! diagnostics placed as `docs/cli.md` says, and `fmt` does not change its verdict: the same codes before and after, but for
//! what `fmt` removes, and unless a budget or a limit made `check` say less (`Checked::cut_short`). A file named after what it
//! declares is never misnamed (`LC2023`).
//!
//! The input is a library of up to 32 files, separated by two NUL bytes, which no file can hold since `LC0005` stops at the
//! first one; each file is named as [`files_of`] says. The shipped libraries are there, as for the program. The work runs on the stack of a host
//! (`format::STACK_SIZE`), as the program does.
#![no_main]

use std::borrow::Cow;
use std::collections::BTreeMap;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_format::{STACK_SIZE, format, read};
use labcraft_graph::Decl;
use labcraft_types::{Code, Interner, Limits, Report};
use libfuzzer_sys::fuzz_target;

const SEPARATOR: &[u8] = b"\0\0";
const FILES: usize = 32;

struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
    }
}

/// The files of a library, as a host that walks a folder would find them, and the names of those named after what they declare.
/// The length of a file chooses its name, so that the fuzzer reaches each case of R-22 by adding or removing a byte:
///
/// - two lengths in four, the name of what it declares, with `.lct` for a type;
/// - one length in four, `f<N>`: a misnamed file, or one of several files of the same kind that declare the same name;
/// - one length in four, the name of what it declares with the other extension (`LC0103`).
///
/// A file that does not read, or whose name a file before it took, is `f<N>` (`_f<N>` if that is taken too).
fn files_of(bytes: &[u8]) -> (Vec<(String, Vec<u8>)>, Vec<String>) {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut named = Vec::new();
    let mut rest = bytes;
    while files.len() < FILES {
        let (file, next) = match rest.windows(SEPARATOR.len()).position(|window| window == SEPARATOR) {
            Some(at) if files.len() + 1 < FILES => (&rest[..at], Some(&rest[at + SEPARATOR.len()..])),
            _ => (rest, None),
        };
        let mut names = Interner::new();
        let (stem, kind) = match read(file, &mut names, &Limits::default()).file.map(|file| file.decl) {
            Some(Decl::Type(ty)) => (names.resolve(ty.name).to_owned(), "lct"),
            Some(Decl::Function(function)) => (names.resolve(function.name).to_owned(), "lcf"),
            Some(Decl::Extern(function)) => (names.resolve(function.name).to_owned(), "lcf"),
            None if file.windows(6).any(|window| window == b"\ntype ") => (String::new(), "lct"),
            None => (String::new(), "lcf"),
        };
        let (stem, kind, own) = match file.len() % 4 {
            2 => (String::new(), kind, false),
            3 => (stem, if kind == "lct" { "lcf" } else { "lct" }, false),
            _ => (stem, kind, true),
        };
        let taken = |shown: &str| files.iter().any(|(other, _)| other == shown);
        let mut shown = format!("{stem}.{kind}");
        if stem.is_empty() || taken(&shown) {
            shown = format!("f{}.{kind}", files.len());
            while taken(&shown) {
                shown.insert(0, '_');
            }
        } else if own {
            named.push(shown.clone());
        }
        files.push((shown, file.to_vec()));
        match next {
            Some(next) => rest = next,
            None => break,
        }
    }
    (files, named)
}

/// The report, and whether a budget or a limit made `check` say less.
fn checked(files: &[(String, Vec<u8>)], limits: &Limits) -> (Report, bool) {
    let files = files.iter().map(|(shown, bytes)| SourceFile { shown: shown.clone(), content: Content::Bytes(Cow::Owned(bytes.clone())) }).collect();
    let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let checked = check(root, &Shipped, &mut Interner::new(), limits);
    let mut report = checked.report;
    report.finish();
    (report, checked.cut_short)
}

/// What `fmt` can make disappear without a word: a duplicate `use` (spec 5.2).
fn codes(report: &Report) -> BTreeMap<Code, usize> {
    let mut codes = BTreeMap::new();
    for diagnostic in report.diagnostics().iter().filter(|diagnostic| diagnostic.code() != Code::Lc2020) {
        *codes.entry(diagnostic.code()).or_insert(0) += 1;
    }
    codes
}

fn judge(bytes: &[u8]) {
    // A report that cuts nothing, since the codes are compared.
    let mut limits = Limits::default();
    limits.max_report = usize::MAX;
    let (files, named) = files_of(bytes);
    let (report, cut_short) = checked(&files, &limits);
    let diagnostics = report.diagnostics();
    assert!(!diagnostics.iter().any(|diagnostic| diagnostic.code() == Code::Lc0099), "{diagnostics:?}");
    for diagnostic in diagnostics {
        // A file of the library, a shipped one, or the folder itself.
        let file = &diagnostic.file;
        assert!(diagnostic.code() != Code::Lc2023 || !named.contains(file), "{diagnostic:?}");
        assert!(file.is_empty() || file.starts_with("<labcraft>/") || files.iter().any(|(shown, _)| shown == file), "{diagnostic:?}");
        if let Some(span) = diagnostic.span {
            assert!(span.start.line >= 1 && span.start.column >= 1 && span.start <= span.end, "{diagnostic:?}");
        }
    }
    assert_eq!(report.errors() + report.warnings(), diagnostics.len() as u64);
    // Past a budget, what is said depends on the order of the lines and entries, which `fmt` changes. A budget counts by function,
    // so the order of the files changes nothing there, and a spent budget is slow under the sanitizer: one run is enough.
    if cut_short {
        return;
    }
    // The order in which a host hands the files changes nothing.
    let reversed: Vec<_> = files.iter().rev().cloned().collect();
    let (again, _) = checked(&reversed, &limits);
    assert!(again == report, "in order: {:?}\nreversed: {:?}", report.diagnostics(), again.diagnostics());
    let mut formatted = Vec::with_capacity(files.len());
    for (shown, bytes) in &files {
        let result = format(bytes, &mut Interner::new(), &limits);
        if result.report.diagnostics().iter().any(|diagnostic| diagnostic.code() == Code::Lc5004) {
            return;
        }
        match result.text {
            Some(text) => formatted.push((shown.clone(), text.into_bytes())),
            None => return,
        }
    }
    let (after, after_cut_short) = checked(&formatted, &limits);
    if after_cut_short {
        return;
    }
    assert_eq!(codes(&report), codes(&after), "before fmt: {:?}\nafter fmt: {:?}", report.diagnostics(), after.diagnostics());
}

fuzz_target!(|bytes: &[u8]| {
    let bytes = bytes.to_vec();
    // A panic in the thread aborts the process, through the hook of libFuzzer.
    let _ = std::thread::Builder::new().stack_size(STACK_SIZE).spawn(move || judge(&bytes)).map(|thread| thread.join());
});
