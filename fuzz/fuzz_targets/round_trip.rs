//! Scenario 5 on any input that parses: what `fmt` writes holds every line of the declaration, and reads again, to
//! the same model and the same text, with every comment still there, and with nothing left to remove or to warn
//! about but a body that cannot be ordered.
//!
//! The default limit of nesting, like the target `read` (`fuzz/README.md`).
#![no_main]

use std::collections::BTreeMap;

use labcraft_format::{Lexer, TokenKind, canonicalize, read, write};
use labcraft_types::{Code, Interner, Limits, Report};
use libfuzzer_sys::fuzz_target;

fn comments_of(text: &str) -> BTreeMap<String, usize> {
    let mut names = Interner::new();
    let mut report = Report::new(1);
    let mut comments = BTreeMap::new();
    for token in Lexer::new(text, &mut names, &mut report).flatten() {
        if let TokenKind::Comment(comment) | TokenKind::Doc(comment) = token.kind {
            *comments.entry(comment.trim_matches([' ', '\t']).to_owned()).or_insert(0) += 1;
        }
    }
    comments
}

fn count(report: &Report, code: Code) -> usize {
    report.diagnostics().iter().filter(|diagnostic| diagnostic.code() == code).count()
}

fuzz_target!(|bytes: &[u8]| {
    // A report that cuts nothing, since the counts below compare what is listed.
    let mut limits = Limits::default();
    limits.max_report = usize::MAX;
    let mut names = Interner::new();
    let first = read(bytes, &mut names, &limits);
    let Some(file) = first.file else { return };
    let mut report = first.report;
    // Ordering its lines is all that `canonicalize` may do to a declaration: none is lost, and none is invented.
    let mut as_read = file.clone();
    as_read.uses.clear();
    as_read.panel = None;
    as_read.layout = None;
    let once = canonicalize(file, &first.source_map, &mut names, &mut report);
    let mut canonical = once.clone();
    canonical.uses.clear();
    canonical.panel = None;
    canonical.layout = None;
    if let (Ok(as_read), Ok(canonical)) = (write(&as_read, &names, &limits), write(&canonical, &names, &limits)) {
        let mut as_read: Vec<&str> = as_read.lines().collect();
        let mut canonical: Vec<&str> = canonical.lines().collect();
        as_read.sort_unstable();
        canonical.sort_unstable();
        assert_eq!(as_read, canonical);
    }
    // `LC6003`: nothing is written. The deep and short lines that this takes are in `crates/format/tests`.
    let Ok(text) = write(&once, &names, &limits) else { return };

    let second = read(text.as_bytes(), &mut names, &limits);
    let again = second.file.unwrap_or_else(|| panic!("{:?}\n{text}", second.report.diagnostics()));
    let mut report_again = second.report;
    let again = canonicalize(again, &second.source_map, &mut names, &mut report_again);
    assert!(once == again, "the model changed:\n{text}");
    assert_eq!(write(&again, &names, &limits).as_deref(), Ok(text.as_str()));

    assert_eq!(count(&report_again, Code::Lc5004), 0, "{text}");
    assert_eq!(count(&report_again, Code::Lc6002), count(&report, Code::Lc6002), "{text}");

    assert!(text.ends_with('\n') && !text.ends_with("\n\n") && !text.contains('\r') && !text.contains(" \n") && !text.contains("\t\n"), "{text}");
    // A prefix of the input that is not UTF-8 does not parse, so these are the bytes that `read` took.
    if let Ok(input) = std::str::from_utf8(bytes) {
        assert_eq!(comments_of(input.strip_prefix('\u{FEFF}').unwrap_or(input)), comments_of(&text));
    }
});
