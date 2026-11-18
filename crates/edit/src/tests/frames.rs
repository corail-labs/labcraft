//! `frame.resize` and the `frame … size` entries of M1 (commands.md `frame.resize` and `layout.arrange`, spec 5.2 and 11).
//! Each expected text is written by hand from the canonical order of spec 5.2: the `frame size` of the function, then the
//! `frame … size` of the structures by name, then `node`, then `wire`; a body goes by name among the lines that are ready,
//! a label sorting by its name.

use super::{FILE, Table, gives, text};
use crate::{EditError, Edited, Files, FrameDrawn, Judge, Limits, Lookup, Op, Point, apply};

/// The judge of the table, and the least sizes that a drawing would give: 10 by 6 for the structure on line 7 (`ys` of
/// [`HEAD`], after `labcraft 1` and a blank line), named `ys` as the drawing names it, and for one on line 6 named `@6`;
/// 20 by 12 for the function.
struct Framed;

impl Judge for Framed {
    fn callable(&self, file: &str, call: &str) -> Lookup {
        Table.callable(file, call)
    }
    fn errors(&self, file: &str, text: &str) -> Vec<labcraft_types::Diagnostic> {
        Table.errors(file, text)
    }
    fn frames(&self, _: &str, _: &str) -> Vec<FrameDrawn> {
        vec![
            FrameDrawn { structure: None, line: 0, least: Point { x: 20, y: 12 } },
            FrameDrawn { structure: Some("ys".to_owned()), line: 7, least: Point { x: 10, y: 6 } },
            FrameDrawn { structure: Some("@6".to_owned()), line: 6, least: Point { x: 10, y: 6 } },
        ]
    }
}

fn run_framed(before: &str, op: Op) -> Result<Edited, EditError> {
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    apply(&files, &op, &Framed, &Limits::default())
}

#[track_caller]
fn gives_framed(before: &str, op: Op, after: &str) {
    let edited = run_framed(before, op).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(text(after).as_str()));
}

fn resize(inside: Option<&str>, w: i64, h: i64) -> Op {
    Op::FrameResize { file: FILE.to_owned(), inside: inside.map(str::to_owned), size: Some(Point { x: w, y: h }) }
}

const HEAD: &str = "use core\n\nfn f(x: f64, v: [f64]) -> (w: f64) {\n    w = core.abs(x)\n    ys = for e in v {\n        collect e\n    }\n}\n";

fn with_layout(entries: &str) -> String {
    format!("{HEAD}\nlayout {{\n{entries}}}\n")
}

#[test]
fn a_resize_writes_the_entry_of_the_structure_or_of_the_function() {
    gives_framed(HEAD, resize(Some("ys"), 320, 200), &with_layout("    frame ys size (320, 200)\n"));
    // Without `inside`: the frame of the function, first.
    gives_framed(
        &with_layout("    frame ys size (320, 200)\n"),
        resize(None, 600, 300),
        &with_layout("    frame size (600, 300)\n    frame ys size (320, 200)\n"),
    );
}

#[test]
fn a_second_resize_replaces_the_entry() {
    gives_framed(&with_layout("    frame ys size (320, 200)\n"), resize(Some("ys"), 400, 250), &with_layout("    frame ys size (400, 250)\n"));
    gives_framed(&with_layout("    frame size (600, 300)\n"), resize(None, 700, 400), &with_layout("    frame size (700, 400)\n"));
}

#[test]
fn a_size_under_the_least_is_refused_and_the_least_is_allowed() {
    let error = run_framed(HEAD, resize(Some("ys"), 9, 6)).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("size"))));
    let error = run_framed(HEAD, resize(None, 20, 11)).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("size"))));
    // Exactly the least is allowed.
    gives_framed(HEAD, resize(Some("ys"), 10, 6), &with_layout("    frame ys size (10, 6)\n"));
}

#[test]
fn the_least_size_is_found_for_a_structure_named_by_its_label_or_by_its_line() {
    // The drawing names the structure of line 7 `ys`; a command names it `#scan`, or `@7` when it has no name: the same
    // frame, found by its line, so the least holds.
    let labelled = HEAD.replace("    ys = for", "    #scan: ys = for");
    let error = run_framed(&labelled, resize(Some("#scan"), 9, 6)).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("size"))));
    // In canonical order, so that a line names the same statement: the `for` first, on line 6.
    let unnamed = "use core

fn f(x: f64, v: [f64]) -> (w: f64) {
    _ = for e in v {
        collect e
    }
    w = core.abs(x)
}
";
    let error = run_framed(unnamed, resize(Some("@6"), 1, 1)).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("size"))));
}

#[test]
fn inside_names_a_structure() {
    let error = run_framed(HEAD, resize(Some("w"), 320, 200)).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("inside"))));
}

#[test]
fn the_entry_of_a_frame_follows_a_rename_and_goes_with_its_structure() {
    let before = with_layout("    frame size (600, 300)\n    frame ys size (320, 200)\n    node w at (1, 2)\n");
    // `zz` sorts after `w`, so the lines stay in order.
    let renamed = Op::ValueRename { file: FILE.to_owned(), value: "ys".to_owned(), name: "zz".to_owned() };
    gives(&before, renamed, &before.replace("ys = for", "zz = for").replace("frame ys", "frame zz"));
    // A label designates the structure; the line still goes by its name `ys`, after `w`.
    let labelled = Op::NodeSetLabel { file: FILE.to_owned(), line: 7, label: "scan".to_owned() };
    gives(
        &before,
        labelled,
        "use core\n\nfn f(x: f64, v: [f64]) -> (w: f64) {\n    w = core.abs(x)\n    #scan: ys = for e in v {\n        collect e\n    }\n}\n\nlayout {\n    frame size (600, 300)\n    frame #scan size (320, 200)\n    node w at (1, 2)\n}\n",
    );
    // Deleted with the structure; the frame of the function stays.
    let delete = Op::NodeDelete { file: FILE.to_owned(), nodes: vec!["ys".to_owned()] };
    gives(
        &before,
        delete,
        "use core\n\nfn f(x: f64, v: [f64]) -> (w: f64) {\n    w = core.abs(x)\n}\n\nlayout {\n    frame size (600, 300)\n    node w at (1, 2)\n}\n",
    );
}

#[test]
fn arranging_removes_the_frame_entries_it_arranges() {
    let before = with_layout("    frame size (600, 300)\n    frame ys size (320, 200)\n    node w at (1, 2) view compact\n");
    let arrange = |nodes: Option<Vec<String>>| Op::LayoutArrange {
        file: FILE.to_owned(),
        nodes,
        wires: Vec::new(),
        terminals: Vec::new(),
        within: Default::default(),
    };
    // The structure alone: its frame, not the frame of the function.
    gives_framed(&before, arrange(Some(vec!["ys".to_owned()])), &with_layout("    frame size (600, 300)\n    node w at (1, 2) view compact\n"));
    // The whole function: every frame and every place; a view stays.
    gives_framed(&before, arrange(None), &with_layout("    node w view compact\n"));
}

#[test]
fn no_size_removes_the_entry_and_keeps_the_place() {
    // "Remove the fixed size" (notation 2.4): `frame.resize` with no size; the `node … at` entry stays, and a section
    // left empty goes.
    let op = |inside: Option<&str>| Op::FrameResize { file: FILE.to_owned(), inside: inside.map(str::to_owned), size: None };
    gives_framed(
        &with_layout(
            "    frame ys size (320, 200)
    node ys at (40, 80)
",
        ),
        op(Some("ys")),
        &with_layout(
            "    node ys at (40, 80)
",
        ),
    );
    gives_framed(
        &with_layout(
            "    frame size (600, 300)
",
        ),
        op(None),
        HEAD,
    );
}

#[test]
fn the_entry_replaced_is_the_one_of_the_structure_whatever_its_spelling() {
    // `frame ys size` names the structure `#scan: ys = for …` by its line; a resize writes the entry by its label, the
    // one that a command prefers (spec 11), in place of the other: one entry, never two (`LC5007`).
    let labelled = HEAD.replace("    ys = for", "    #scan: ys = for");
    let with = |entries: &str| format!("{labelled}\nlayout {{\n{entries}}}\n");
    gives_framed(&with("    frame ys size (320, 200)\n"), resize(Some("ys"), 400, 250), &with("    frame #scan size (400, 250)\n"));
    // And no size removes it, spelled either way.
    let op = Op::FrameResize { file: FILE.to_owned(), inside: Some("#scan".to_owned()), size: None };
    gives_framed(&with("    frame ys size (320, 200)\n"), op, &labelled);
}

#[test]
fn a_size_is_at_least_one_by_one_and_removing_none_changes_nothing() {
    let error = run_framed(HEAD, resize(None, 0, 40)).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("size"))));
    let error = run_framed(HEAD, resize(Some("ys"), 40, -1)).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("size"))));
    // A structure that the section cannot name, with no entry: no size gives it no label, and the text stays.
    let unnamed = "use core\n\nfn f(x: f64, v: [f64]) -> (w: f64) {\n    _ = for e in v {\n        collect e\n    }\n    w = core.abs(x)\n}\n";
    let op = Op::FrameResize { file: FILE.to_owned(), inside: Some("@6".to_owned()), size: None };
    let edited = run_framed(unnamed, op).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(edited.texts.get(FILE).is_none_or(|after| *after == text(unnamed)), "{:?}", edited.texts);
}
