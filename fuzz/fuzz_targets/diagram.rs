//! On any input, `diagram` draws every function that `check` lets it draw without an internal error, gives the same SVG
//! when it draws it again, and keeps the rules of routing of the notation (2.2), as the checker of the harness reads them
//! (`tests/harness/src/routing.rs`): no wire crosses a node, a label, a pill or a wire name of its body, parallel wires
//! keep 8 apart, only error wires go below the main chain. Both views are drawn, and once more under a low limit of nodes,
//! which `LC7002` must refuse without drawing.
//!
//! The input is a library as for the target `check`: up to 32 files separated by two NUL bytes, each named after what it
//! declares when it reads, `f<N>` otherwise. The shipped libraries are there. The work runs on the stack of a host
//! (`format::STACK_SIZE`), as the program does.
#![no_main]

use std::borrow::Cow;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_diagram::{DiagramLimits, Drawn, Theme, ViewState, draw, svg};
use labcraft_format::{STACK_SIZE, read};
use labcraft_graph::Decl;
use labcraft_types::{Interner, Limits};
use libfuzzer_sys::fuzz_target;

#[path = "../../tests/harness/src/routing.rs"]
mod routing;

const SEPARATOR: &[u8] = b"\0\0";
const FILES: usize = 32;

struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
    }
}

/// The files of the library, each named after what it declares, so that calls between them resolve.
fn files_of(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
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
            None => (String::new(), "lcf"),
        };
        let mut shown = format!("{stem}.{kind}");
        if stem.is_empty() || files.iter().any(|(other, _)| *other == shown) {
            shown = format!("f{}.{kind}", files.len());
            while files.iter().any(|(other, _)| *other == shown) {
                shown.insert(0, '_');
            }
        }
        files.push((shown, file.to_vec()));
        match next {
            Some(next) => rest = next,
            None => break,
        }
    }
    files
}

fn judge(bytes: &[u8]) {
    let files = files_of(bytes).into_iter().map(|(shown, bytes)| SourceFile { shown, content: Content::Bytes(Cow::Owned(bytes)) }).collect();
    let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut names = Interner::new();
    let checked = check(root, &Shipped, &mut names, &Limits::default());
    let few = DiagramLimits { max_nodes: 3 };
    for index in 0..checked.files.len() {
        for standard_everywhere in [false, true] {
            let view = ViewState { standard_everywhere, ..ViewState::default() };
            let Drawn::Drawing(drawing) = draw(&checked, index, &names, &view, &DiagramLimits::default()) else { continue };
            let first = svg(&drawing, Theme::Both);
            let Drawn::Drawing(again) = draw(&checked, index, &names, &view, &DiagramLimits::default()) else { panic!("drawn once, then not") };
            assert!(svg(&again, Theme::Both) == first, "two drawings of one function differ");
            let faults = routing::faults(&drawing);
            assert!(faults.is_empty(), "the rules of routing (notation 2.2):\n{}", faults.join("\n"));
            if drawing.placed.len() > 3 {
                assert!(matches!(draw(&checked, index, &names, &view, &few), Drawn::NotDrawn(_)), "more than 3 nodes drawn under a limit of 3");
            }
        }
    }
}

fuzz_target!(|bytes: &[u8]| {
    let bytes = bytes.to_vec();
    // A panic in the thread aborts the process, through the hook of libFuzzer.
    let _ = std::thread::Builder::new().stack_size(STACK_SIZE).spawn(move || judge(&bytes)).map(|thread| thread.join());
});
