//! Copy and paste of M1 step 4c (commands.md 6.7, visual notation 2.6). Each expected text is written by hand from
//! commands.md 6.7 and 9.1, notation 2.6 and spec 5.2 (the order of lines: by name among those ready, a line that only binds an output last) and 11.

use labcraft_types::Limits;

use super::{FILE, TWO, Table, gives, refused, text};
use crate::{EditError, Files, Op, Point, Renamed};

fn copy(before: &str, nodes: &[&str]) -> Result<String, EditError> {
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    let nodes: Vec<String> = nodes.iter().map(|node| (*node).to_owned()).collect();
    crate::copy(&files, FILE, &nodes, &Table, &Limits::default())
}

fn paste(fragment: &str, x: i64, y: i64) -> Op {
    Op::FragmentPaste { file: FILE.to_owned(), text: fragment.to_owned(), at: Point { x, y }, within: Default::default() }
}

fn renamed(pairs: &[(&str, &str)]) -> Vec<Renamed> {
    pairs.iter().map(|(from, to)| Renamed { file: FILE.to_owned(), from: (*from).to_owned(), to: (*to).to_owned() }).collect()
}

/// Three nodes in a row, fixed, with routed wires; `visa` used by nothing of them.
const ROW: &str = "use core\nuse visa\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    c = core.abs(b)\n    y = c\n}\n\nlayout {\n    node a at (40, 80)\n    node b at (200, 80)\n    node c at (300, 120)\n    wire a -> b via (120, 80)\n    wire b -> c via (250, 100)\n}\n";

/// What copying `b` and `c` of `ROW` gives: `a` is read from outside, a positional input before another, so a hole; the
/// entries counted from the top-left corner of the two, (200, 80); only the `use` that the text needs.
const COPIED: &str = "labcraft 1\n\nuse core\n\nfn clipboard() -> () {\n    b = core.mul(?, 2)\n    c = core.abs(b)\n}\n\nlayout {\n    node b at (0, 0)\n    node c at (100, 40)\n    wire b -> c via (50, 20)\n}\n";

const STRUCTURE: &str = "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {\n    m = match c {\n        true => {\n            d = core.add(x, 1)\n            yield d\n        }\n        false => yield x\n    }\n    y = core.abs(m)\n}\n\nlayout {\n    node d at (30, 40)\n    node m at (100, 50)\n}\n";

#[test]
fn a_copy_is_a_function_clipboard_counted_from_its_corner() {
    assert_eq!(copy(ROW, &["c", "b"]).as_deref(), Ok(COPIED));
}

#[test]
fn a_copy_of_a_structure_holds_what_it_holds() {
    // The selector and the `yield x` read from outside, and a yield cannot leave a value out: holes. `x` in `core.add`
    // is a positional input before another: a hole. `d` keeps the coordinates of the frame of its arm.
    let copied = "labcraft 1\n\nuse core\n\nfn clipboard() -> () {\n    m = match ? {\n        true => {\n            d = core.add(?, 1)\n            yield d\n        }\n        false => yield ?\n    }\n}\n\nlayout {\n    node d at (30, 40)\n    node m at (0, 0)\n}\n";
    assert_eq!(copy(STRUCTURE, &["m"]).as_deref(), Ok(copied));
}

#[test]
fn a_copy_takes_the_nodes_of_one_frame() {
    let refused = copy(STRUCTURE, &["m", "d"]);
    assert_eq!(refused.map_err(|error| (error.code, error.parameter)), Err(("params.invalid", Some("nodes"))));
    assert_eq!(copy(STRUCTURE, &[]).map_err(|error| error.code), Err("params.invalid"));
    assert_eq!(copy(STRUCTURE, &["nothing"]).map_err(|error| error.code), Err("edit.unknown"));
}

#[test]
fn a_paste_gives_a_suffix_to_a_name_taken_and_moves_to_its_place() {
    // `b` is taken: `b2`, and `c` reads it. The entries go from the corner (0, 0) to (400, 200); the output line last.
    let edited = gives(
        TWO,
        paste(COPIED, 400, 200),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    b2 = core.mul(?, 2)\n    c = core.abs(b2)\n    y = b\n}\n\nlayout {\n    node b2 at (400, 200)\n    node c at (500, 240)\n    wire b2 -> c via (450, 220)\n}\n",
    );
    assert_eq!(edited.renamed, renamed(&[("b", "b2")]));
    assert_eq!(edited.lists.get("nodes"), Some(&vec!["b2".to_owned(), "c".to_owned()]));
}

#[test]
fn a_paste_takes_any_function_and_leaves_what_it_does_not_bind() {
    // The input `r` of `g` is not pasted: the last positional input, left out. The `after:` of a label that the fragment
    // lacks goes. `e` and `#close` are taken: suffixes. `use visa` comes; the `panel` does not.
    let fragment = "labcraft 1\n\nuse visa\n\nfn g(r: string) -> (s: session) {\n    s, e = visa.open(r)\n    #close: visa.close(s, after: #wait)\n}\n\npanel {\n    control r: text_box\n}\n";
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    e = core.abs(x)\n    #close: y = core.add(e, 1)\n}\n";
    let edited = gives(
        before,
        paste(fragment, 0, 0),
        "use core\nuse visa\n\nfn f(x: f64) -> (y: f64) {\n    e = core.abs(x)\n    s, e2 = visa.open()\n    #close2: visa.close(s)\n    #close: y = core.add(e, 1)\n}\n",
    );
    assert_eq!(edited.renamed, renamed(&[("e", "e2"), ("#close", "#close2")]));
    assert_eq!(edited.lists.get("nodes"), Some(&vec!["s".to_owned(), "#close2".to_owned()]));
}

#[test]
fn a_copy_pastes_back_into_its_own_function() {
    let edited = gives(
        ROW,
        paste(COPIED, 200, 300),
        "use core\nuse visa\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    b2 = core.mul(?, 2)\n    c = core.abs(b)\n    c2 = core.abs(b2)\n    y = c\n}\n\nlayout {\n    node a at (40, 80)\n    node b at (200, 80)\n    node b2 at (200, 300)\n    node c at (300, 120)\n    node c2 at (300, 340)\n    wire a -> b via (120, 80)\n    wire b -> c via (250, 100)\n    wire b2 -> c2 via (250, 320)\n}\n",
    );
    assert_eq!(edited.renamed, renamed(&[("b", "b2"), ("c", "c2")]));
}

#[test]
fn a_paste_refuses_a_text_without_a_function() {
    let error = refused(TWO, paste("labcraft 1\n\nfn g( {\n", 0, 0), "params.invalid");
    assert_eq!(error.parameter, Some("text"));
    let error = refused(TWO, paste("labcraft 1\n\ntype T = struct {\n    x: f64,\n}\n", 0, 0), "params.invalid");
    assert_eq!(error.parameter, Some("text"));
    let error = refused(TWO, paste(COPIED, 2_000_000, 0), "params.invalid");
    assert_eq!(error.parameter, Some("at"));
    let large = format!("{COPIED}{}", "\n".repeat(crate::MAX_FRAGMENT));
    assert_eq!(refused(TWO, paste(&large, 0, 0), "file.too_large").parameter, Some("text"));
}

#[test]
fn a_suffix_is_free_of_the_names_that_the_structures_of_the_fragment_bind() {
    // `v` is taken; `v2` is the carry of the loop, so the line becomes `v3`, and the loop reads it still (R-3). The
    // clauses fit on the line of the loop (spec 5.2, 100 characters).
    let fragment = "labcraft 1\n\nuse core\n\nfn g() -> () {\n    v = core.abs(1)\n    s = loop\n        carry v2 = 0\n    {\n        w = core.add(v, v2)\n        next v2 = w\n        until true\n    }\n}\n";
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    v = core.add(x, 1)\n    y = v\n}\n";
    let edited = gives(
        before,
        paste(fragment, 0, 0),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    v = core.add(x, 1)\n    v3 = core.abs(1)\n    s = loop carry v2 = 0 {\n        w = core.add(v3, v2)\n        next v2 = w\n        until true\n    }\n    y = v\n}\n",
    );
    assert_eq!(edited.renamed, renamed(&[("v", "v3")]));
}

#[test]
fn a_name_that_only_a_structure_of_the_fragment_binds_is_read_from_outside_after_it() {
    // `w` reads a `v` that only the arm binds: from outside, so a hole, never the `v` of the function. The input `t`
    // is not pasted: the selector gets a hole. The arm's `v` is taken: `v2`. `visa` is named by nothing: no `use`.
    let fragment = "labcraft 1\n\nuse core\nuse visa\n\nfn g(t: bool) -> () {\n    m = match t {\n        true => {\n            v = core.abs(1)\n            yield v\n        }\n        false => yield 0\n    }\n    w = core.neg(v)\n}\n";
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    v = core.add(x, 1)\n    y = v\n}\n";
    gives(
        before,
        paste(fragment, 0, 0),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    m = match ? {\n        true => {\n            v2 = core.abs(1)\n            yield v2\n        }\n        false => yield 0\n    }\n    v = core.add(x, 1)\n    w = core.neg(?)\n    y = v\n}\n",
    );
}

#[test]
fn a_copy_counts_within_the_bounds_of_a_coordinate() {
    // From the corner (-1000000, 0), `b` would be at 2000000, past the bound of spec 11.
    let wide = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.abs(x)\n    y = b\n}\n\nlayout {\n    node a at (-1000000, 0)\n    node b at (1000000, 0)\n}\n";
    assert_eq!(copy(wide, &["a", "b"]).map_err(|error| (error.code, error.parameter)), Err(("params.invalid", Some("nodes"))));
}

#[test]
fn a_copy_keeps_the_route_of_an_order_wire() {
    // `x` comes from outside: the last positional input, left out; the `after:` stays, `#beep` is copied.
    let before = "use core\nuse sound\n\nfn f(x: f64) -> (y: f64) {\n    #beep: sound.beep()\n    a = core.abs(x, after: #beep)\n    y = a\n}\n\nlayout {\n    node #beep at (10, 10)\n    node a at (110, 30)\n    wire #beep -> a via (60, 20)\n}\n";
    let copied = copy(before, &["#beep", "a"]).unwrap_or_else(|error| panic!("{error:?}"));
    for line in [
        "use core\nuse sound\n",
        "    a = core.abs(after: #beep)\n",
        "    node #beep at (0, 0)\n",
        "    node a at (100, 20)\n",
        "    wire #beep -> a via (50, 10)\n",
    ] {
        assert!(copied.contains(line), "{line:?} in {copied}");
    }
}

#[test]
fn a_line_of_a_loop_goes_with_its_loop() {
    // `next`, `collect` and `until` belong in a loop; a paste goes into the body of the function until step 5.
    let fragment = "labcraft 1\n\nuse core\n\nfn g(a: f64) -> () {\n    next a = core.abs(a)\n}\n";
    assert_eq!(refused(TWO, paste(fragment, 0, 0), "params.invalid").parameter, Some("text"));
}

#[test]
fn a_paste_does_not_name_a_value_after_a_library_that_it_imports() {
    // The fragment brings `use visa` and a value `visa`: in the file that now imports `visa`, the value cannot bear its
    // name (R-1, `LC2014`), so it takes a suffix (N-14), as a name already taken does; a hole stays a hole, as `b2` above.
    let fragment = "labcraft 1\n\nuse core\nuse visa\n\nfn clipboard() -> () {\n    visa = core.abs(?)\n    s, _ = visa.open(?)\n}\n";
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
    let edited = gives(
        before,
        paste(fragment, 0, 0),
        "use core\nuse visa\n\nfn f(x: f64) -> (y: f64) {\n    s, _ = visa.open(?)\n    visa2 = core.abs(?)\n    y = x\n}\n",
    );
    assert_eq!(edited.renamed, renamed(&[("visa", "visa2")]));
}
