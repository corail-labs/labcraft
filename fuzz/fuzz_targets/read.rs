//! On any input, `read` ends, with a model or with an error and never both, it says what `docs/cli.md` and the
//! architecture promise, and what it builds can be cloned, compared and dropped.
//!
//! The default limit of nesting, so the main thread of libFuzzer is enough, sanitizer included (`fuzz/README.md`).
//! `crates/format/tests/stack.rs` holds the ceiling.
#![no_main]

use labcraft_format::read;
use labcraft_types::{Interner, Limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let mut names = Interner::new();
    let result = read(bytes, &mut names, &Limits::default());
    let diagnostics = result.report.diagnostics();
    assert_eq!(result.file.is_some(), result.report.errors() == 0, "{diagnostics:?}");
    // The parser stops at the first error (architecture, section 4).
    assert!(result.report.errors() <= 1, "{diagnostics:?}");
    // A file that gets such a code gets no other diagnostic (cli.md, section 1).
    assert!(diagnostics.len() == 1 || !diagnostics.iter().any(|diagnostic| diagnostic.code().stands_alone()), "{diagnostics:?}");
    let line_endings = bytes.iter().filter(|byte| matches!(byte, b'\n' | b'\r')).count();
    for diagnostic in diagnostics {
        if let Some(span) = diagnostic.span {
            assert!(span.start.line >= 1 && span.start.column >= 1 && span.start <= span.end, "{diagnostic:?}");
            // The last line, and an end that is exclusive.
            assert!(usize::try_from(span.end.line).is_ok_and(|line| line <= line_endings + 2), "{diagnostic:?}");
        }
    }
    if let Some(file) = result.file {
        assert!(file == file.clone());
    }
});
