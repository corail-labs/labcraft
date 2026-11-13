//! On any input, read as the trace of a run of a fixed function (`fuzz/trace/walk.lcf`: a `for` over an array with a
//! range and three carries, and a `match` that binds the value of a variant), the reader of a trace ends within its
//! limits (`docs/cli.md`, 7.4), and gives the same whether it is fed the file whole or in parts; its diagnostics are
//! `LC7101` to `LC7104`, each at a line of the file; a first line that does not read, or the trace of another function,
//! gives no values; and the values over the drawing (notation 6.1) and the SVG that shows them are the same worked out
//! twice. The seed is the trace of a run of that function (`deep_seeds`).
#![no_main]

use std::borrow::Cow;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_diagram::values::diagnostics;
use labcraft_diagram::{Choice, DiagramLimits, Drawing, Drawn, Judged, Theme, ViewState, draw, expected, svg_with, values};
use labcraft_trace::{ReadLimits, Reader};
use labcraft_types::{Interner, Limits};
use libfuzzer_sys::fuzz_target;

struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
    }
}

/// The drawing of `walk`, as `labcraft-cli diagram` draws it.
fn drawing() -> Drawing {
    let files = [("lib/walk.lcf", &include_bytes!("../trace/walk.lcf")[..]), ("lib/Reading.lct", &include_bytes!("../trace/Reading.lct")[..])];
    let files = files.iter().map(|(shown, bytes)| SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Borrowed(bytes)) }).collect();
    let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut names = Interner::new();
    let checked = check(root, &Shipped, &mut names, &Limits::default());
    let index = checked.files.iter().position(|file| file.shown == "lib/walk.lcf").expect("walk.lcf is checked");
    match draw(&checked, index, &names, &ViewState::default(), &DiagramLimits::default()) {
        Drawn::Drawing(drawing) => drawing,
        Drawn::NotDrawn(diagnostics) => panic!("walk is not drawn: {diagnostics:?}"),
    }
}

thread_local! {
    static DRAWING: Drawing = drawing();
}

fuzz_target!(|bytes: &[u8]| {
    DRAWING.with(|drawing| {
        let mut whole = Reader::new(expected(drawing), ReadLimits::default());
        whole.feed(bytes);
        let trace = whole.finish();
        // In three parts, as a file is read a part at a time: what one part ends, the next goes on with.
        let mut parts = Reader::new(expected(drawing), ReadLimits::default());
        for part in bytes.chunks(bytes.len().div_ceil(3).max(1)) {
            if !parts.feed(part) {
                break;
            }
        }
        assert!(parts.finish() == trace, "read whole and in parts, a trace gives two answers");
        let lines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64 + 1;
        assert!(trace.events < lines, "{} events in {lines} lines", trace.events);
        for diagnostic in diagnostics(&trace, "t.jsonl") {
            let code = diagnostic.code().as_str();
            assert!(["LC7101", "LC7102", "LC7103", "LC7104"].contains(&code), "{code}");
            let line = diagnostic.span.map_or(0, |span| u64::from(span.start.line));
            assert!((1..=lines).contains(&line), "{code} at line {line} of {lines}");
        }
        let overlay = values(drawing, &trace, &Judged::default(), &Choice::default());
        if trace.header.is_err() || trace.other_function {
            assert!(overlay.bubbles.is_empty() && overlay.banner.is_empty(), "a trace that gives no values shows some");
        }
        assert!(values(drawing, &trace, &Judged::default(), &Choice::default()) == overlay, "two overlays of one trace differ");
        let svg = svg_with(drawing, Some(&overlay), Theme::Both);
        assert!(svg.ends_with("</svg>\n"));
    });
});
