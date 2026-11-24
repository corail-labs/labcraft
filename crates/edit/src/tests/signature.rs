//! `function.set_doc`, `signature.*` and the text of `function.create` (commands.md 6.7, "Functions, signatures and
//! types"). Each expected text is written by hand from commands.md 6.7 and 9.1 (a value that loses what fed it is left
//! out, or gets a hole `?`), the spec (4, 5.2, 11) and visual notation 2.4 (an output unwired gives its node back the name
//! that inserting it would have given, N-14), never from the output of this crate.

use super::{FILE, Table, gives, refused, text};
use crate::{EditError, Edited, Files, Judge, Limits, Lookup, Op, Terminal, apply, new_function};

fn input(name: &str) -> Terminal {
    Terminal::Input(name.to_owned())
}

fn output(name: &str) -> Terminal {
    Terminal::Output(name.to_owned())
}

fn doc(text: &str) -> Op {
    Op::FunctionSetDoc { file: FILE.to_owned(), doc: text.to_owned() }
}

fn add(terminal: Terminal, ty: &str, default: Option<&str>, position: Option<u64>) -> Op {
    Op::SignatureAdd { file: FILE.to_owned(), terminal, ty: ty.to_owned(), default: default.map(str::to_owned), position }
}

fn remove(terminal: Terminal) -> Op {
    Op::SignatureRemove { file: FILE.to_owned(), terminal }
}

fn rename(terminal: Terminal, name: &str) -> Op {
    Op::SignatureRename { file: FILE.to_owned(), terminal, name: name.to_owned() }
}

fn set_type(terminal: Terminal, ty: &str) -> Op {
    Op::SignatureSetType { file: FILE.to_owned(), terminal, ty: ty.to_owned() }
}

fn set_default(terminal: Terminal, default: Option<&str>) -> Op {
    Op::SignatureSetDefault { file: FILE.to_owned(), terminal, default: default.map(str::to_owned) }
}

const SIMPLE: &str = "fn f(x: f64) -> (y: f64) {\n    y = x\n}\n";

// The terminal as a command names it.

#[test]
fn a_terminal_is_an_input_or_an_output_and_one_name() {
    assert_eq!(Terminal::parse("input x"), Some(input("x")));
    assert_eq!(Terminal::parse("  output   err "), Some(output("err")));
    assert_eq!(Terminal::parse("x"), None);
    assert_eq!(Terminal::parse("input a b"), None);
    assert_eq!(Terminal::parse("inner x"), None);
}

// function.create

#[test]
fn a_new_function_is_empty_and_named_after_its_file() {
    let limits = Limits::default();
    assert_eq!(new_function("check", None, &limits).as_deref(), Ok("labcraft 1\n\nfn check() -> () {}\n"));
    assert_eq!(
        new_function("check", Some("Checks a channel.\n\nTwice.\n"), &limits).as_deref(),
        Ok("labcraft 1\n\n/// Checks a channel.\n///\n/// Twice.\nfn check() -> () {}\n")
    );
    assert_eq!(new_function("2nd", None, &limits).map_err(|error| error.code), Err("name.invalid"));
    assert_eq!(new_function("fn", None, &limits).map_err(|error| error.code), Err("name.invalid"));
}

// function.set_doc

#[test]
fn the_documentation_is_written_before_the_function() {
    gives(SIMPLE, doc("Copies x."), "/// Copies x.\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n");
    gives(SIMPLE, doc("First.\r\n\r\nThird.  \n\n"), "/// First.\n///\n/// Third.\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n");
}

#[test]
fn a_new_documentation_takes_the_place_of_the_old_and_an_empty_one_removes_it() {
    let documented = "// A plain note.\n/// Old.\n/// Older.\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
    gives(documented, doc("New."), "// A plain note.\n/// New.\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n");
    gives(documented, doc(""), "// A plain note.\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n");
}

// signature.add

#[test]
fn a_new_input_goes_last_before_err() {
    let with_err = "fn f(x: f64, err: error = none) -> (y: f64, err: error) {\n    y = x\n    err = err\n}\n";
    gives(
        with_err,
        add(input("gain"), "f64", Some("2"), None),
        "fn f(x: f64, gain: f64 = 2, err: error = none) -> (y: f64, err: error) {\n    y = x\n    err = err\n}\n",
    );
    gives(
        with_err,
        add(output("count"), "i64", None, None),
        "fn f(x: f64, err: error = none) -> (y: f64, count: i64, err: error) {\n    y = x\n    err = err\n}\n",
    );
    gives(SIMPLE, add(input("gain"), "f64", None, None), "fn f(x: f64, gain: f64) -> (y: f64) {\n    y = x\n}\n");
}

#[test]
fn a_new_terminal_goes_where_its_position_says() {
    gives(SIMPLE, add(input("first"), "bool", None, Some(0)), "fn f(first: bool, x: f64) -> (y: f64) {\n    y = x\n}\n");
    gives(SIMPLE, add(output("z"), "f64", None, Some(1)), "fn f(x: f64) -> (y: f64, z: f64) {\n    y = x\n}\n");
    refused(SIMPLE, add(input("late"), "f64", None, Some(2)), "params.invalid");
}

#[test]
fn a_type_of_a_library_brings_its_use() {
    gives(SIMPLE, add(input("dmm"), "visa.session", None, None), "use visa\n\nfn f(x: f64, dmm: visa.session) -> (y: f64) {\n    y = x\n}\n");
}

#[test]
fn a_new_terminal_needs_a_free_name_a_type_and_no_default_on_an_output() {
    assert_eq!(refused(SIMPLE, add(input("x"), "f64", None, None), "name.taken").parameter, Some("terminal"));
    refused(SIMPLE, add(output("y"), "f64", None, None), "name.taken");
    refused(SIMPLE, add(input("2x"), "f64", None, None), "name.invalid");
    assert_eq!(refused(SIMPLE, add(input("z"), "f64[", None, None), "params.invalid").parameter, Some("type"));
    assert_eq!(refused(SIMPLE, add(output("z"), "f64", Some("1"), None), "params.invalid").parameter, Some("default"));
    assert_eq!(refused(SIMPLE, add(input("z"), "f64", Some("x"), None), "params.invalid").parameter, Some("default"));
}

#[test]
fn an_input_may_bear_the_name_of_an_output_that_no_line_gives() {
    let unwired = "fn f(x: f64) -> (y: f64, err: error) {\n    y = x\n}\n";
    gives(unwired, add(input("err"), "error", Some("none"), None), "fn f(x: f64, err: error = none) -> (y: f64, err: error) {\n    y = x\n}\n");
    // `y` is given by a line whose value `z` reads: an input of that name would take its reader (R-2).
    let read = "fn f(x: f64) -> (y: f64, z: f64) {\n    y = x\n    z = y\n}\n";
    refused(read, add(input("y"), "f64", None, None), "name.taken");
}

/// The error chain (R-2): an input `err` beside the output `err` that a line gives, as long as nothing reads that line.
#[test]
fn an_input_err_goes_beside_the_output_err_that_a_line_gives() {
    let chained = "use core\n\nfn f(x: f64) -> (y: f64, err: error) {\n    y, e2 = core.check(x)\n    err = e2\n}\n";
    gives(
        chained,
        add(input("err"), "error", Some("none"), None),
        "use core\n\nfn f(x: f64, err: error = none) -> (y: f64, err: error) {\n    y, e2 = core.check(x)\n    err = e2\n}\n",
    );
    gives(chained, rename(input("x"), "err"), "use core\n\nfn f(err: f64) -> (y: f64, err: error) {\n    y, e2 = core.check(err)\n    err = e2\n}\n");
}

/// An output `err` beside the input `err`: what reads `err` reads the input (R-2), and keeps reading it.
#[test]
fn an_output_that_shares_the_name_of_an_input_leaves_its_readers_to_the_input() {
    let given = "use visa\n\nfn h(x: string, err: error = none) -> (s: visa.session, err: error) {\n    s1, e2 = visa.open(x, err: err)\n    s, err = visa.open(x, err: e2)\n}\n";
    let edited = gives(
        given,
        remove(output("err")),
        "use visa\n\nfn h(x: string, err: error = none) -> (s: visa.session) {\n    s1, e2 = visa.open(x, err: err)\n    s, err2 = visa.open(x, err: e2)\n}\n",
    );
    assert_eq!(edited.renamed.first().map(|renamed| (renamed.from.as_str(), renamed.to.as_str())), Some(("err", "err2")));
}

/// The `wire` entries of `layout` follow an output, and an input, that share a name (spec 11).
#[test]
fn the_wires_of_a_shared_name_follow_the_terminal_they_belong_to() {
    let placed = "use core\n\nfn f(x: f64, err: error = none) -> (y: f64, err: error) {\n    y, e2 = core.check(x, err: err)\n    err = e2\n}\n\nlayout {\n    wire e2 -> err via (30, 30)\n    wire err -> y via (50, 50)\n}\n";
    gives(
        placed,
        rename(output("err"), "failure"),
        "use core\n\nfn f(x: f64, err: error = none) -> (y: f64, failure: error) {\n    y, e2 = core.check(x, err: err)\n    failure = e2\n}\n\nlayout {\n    wire e2 -> failure via (30, 30)\n    wire err -> y via (50, 50)\n}\n",
    );
    gives(
        placed,
        remove(output("err")),
        "use core\n\nfn f(x: f64, err: error = none) -> (y: f64) {\n    y, e2 = core.check(x, err: err)\n}\n\nlayout {\n    wire err -> y via (50, 50)\n}\n",
    );
    gives(
        placed,
        remove(input("err")),
        "use core\n\nfn f(x: f64) -> (y: f64, err: error) {\n    y, e2 = core.check(x)\n    err = e2\n}\n\nlayout {\n    wire e2 -> err via (30, 30)\n}\n",
    );
}

#[test]
fn a_new_output_named_like_a_value_of_the_body_is_given_by_its_line() {
    let valued = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.abs(x)\n    y = x\n}\n";
    gives(valued, add(output("a"), "f64", None, None), "use core\n\nfn f(x: f64) -> (y: f64, a: f64) {\n    a = core.abs(x)\n    y = x\n}\n");
    // An input of that name would be hidden by the line.
    refused(valued, add(input("a"), "f64", None, None), "name.taken");
}

// signature.remove

#[test]
fn an_input_removed_leaves_its_readers_unwired_or_with_a_hole() {
    let read = "use core\n\nfn f(x: f64, k: f64) -> (y: f64) {\n    a = core.add(x, k)\n    y = a\n}\n";
    gives(read, remove(input("x")), "use core\n\nfn f(k: f64) -> (y: f64) {\n    a = core.add(?, k)\n    y = a\n}\n");
    gives(read, remove(input("k")), "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x)\n    y = a\n}\n");
    gives(SIMPLE, remove(input("x")), "fn f() -> (y: f64) {\n    y = ?\n}\n");
}

#[test]
fn an_input_removed_takes_its_wires_out_of_layout() {
    let placed = "use core\n\nfn f(x: f64, k: f64) -> (y: f64) {\n    a = core.add(x, k)\n    y = a\n}\n\nlayout {\n    wire k -> a.b via (10, 10)\n    wire x -> a.a via (10, 20)\n}\n";
    gives(
        placed,
        remove(input("x")),
        "use core\n\nfn f(k: f64) -> (y: f64) {\n    a = core.add(?, k)\n    y = a\n}\n\nlayout {\n    wire k -> a.b via (10, 10)\n}\n",
    );
}

#[test]
fn an_output_removed_takes_its_wire_with_it() {
    gives(SIMPLE, remove(output("y")), "fn f(x: f64) -> () {}\n");
    // What read the output reads what it was given.
    gives("fn f(x: f64) -> (y: f64, z: f64) {\n    y = x\n    z = y\n}\n", remove(output("y")), "fn f(x: f64) -> (z: f64) {\n    z = x\n}\n");
    // The node that gave it takes back the name that inserting it would have given (N-14).
    let bound = "use core\n\nfn f(x: f64) -> (y: f64, z: f64) {\n    y = core.abs(x)\n    z = y\n}\n";
    let edited = gives(bound, remove(output("y")), "use core\n\nfn f(x: f64) -> (z: f64) {\n    abs = core.abs(x)\n    z = abs\n}\n");
    assert_eq!(edited.renamed.first().map(|renamed| (renamed.from.as_str(), renamed.to.as_str())), Some(("y", "abs")));
}

#[test]
fn a_terminal_that_is_not_there_is_unknown() {
    let error = refused(SIMPLE, remove(input("w")), "edit.unknown");
    assert_eq!(error.parameter, Some("terminal"));
    refused(SIMPLE, remove(output("x")), "edit.unknown");
}

// signature.rename, in its file

#[test]
fn an_input_renamed_is_renamed_where_it_is_read_and_in_layout() {
    let placed = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    y = x\n}\n\nlayout {\n    wire x -> a.a via (10, 10)\n}\n";
    let edited = gives(
        placed,
        rename(input("x"), "volts"),
        "use core\n\nfn f(volts: f64) -> (y: f64) {\n    a = core.add(volts, 1)\n    y = volts\n}\n\nlayout {\n    wire volts -> a.a via (10, 10)\n}\n",
    );
    assert_eq!(edited.names.get("terminal").map(String::as_str), Some("input volts"));
}

#[test]
fn an_output_renamed_takes_its_line_with_it() {
    gives(SIMPLE, rename(output("y"), "out"), "fn f(x: f64) -> (out: f64) {\n    out = x\n}\n");
    let bound = "use core\n\nfn f(x: f64) -> (y: f64, z: f64) {\n    y = core.abs(x)\n    z = y\n}\n";
    gives(bound, rename(output("y"), "size"), "use core\n\nfn f(x: f64) -> (size: f64, z: f64) {\n    size = core.abs(x)\n    z = size\n}\n");
}

#[test]
fn an_output_that_shares_the_name_of_an_input_is_renamed_alone() {
    let shared = "use core\n\nfn f(x: f64, err: error = none) -> (y: f64, err: error) {\n    y, e2 = core.check(x, err: err)\n    err = e2\n}\n";
    gives(
        shared,
        rename(output("err"), "failure"),
        "use core\n\nfn f(x: f64, err: error = none) -> (y: f64, failure: error) {\n    y, e2 = core.check(x, err: err)\n    failure = e2\n}\n",
    );
}

#[test]
fn a_rename_needs_a_free_name() {
    let two = "fn f(x: f64, k: f64) -> (y: f64) {\n    y = x\n}\n";
    assert_eq!(refused(two, rename(input("x"), "k"), "name.taken").parameter, Some("name"));
    // `y` is given by a line that `z` reads.
    let read = "fn f(x: f64, k: f64) -> (y: f64, z: f64) {\n    y = x\n    z = y\n}\n";
    refused(read, rename(input("x"), "y"), "name.taken");
    refused(two, rename(input("x"), "fn"), "name.invalid");
}

// signature.rename, in the callers

const CALLER: &str = "main.lcf";

/// The judge of the table, which knows that `f`, seen from any file of the project, is the function of [`FILE`].
struct Project;

impl Judge for Project {
    fn callable(&self, file: &str, call: &str) -> Lookup {
        Table.callable(file, call)
    }
    fn errors(&self, file: &str, text: &str) -> Vec<labcraft_types::Diagnostic> {
        Table.errors(file, text)
    }
    fn function_file(&self, _: &str, call: &str) -> Option<String> {
        (call == "f").then(|| FILE.to_owned())
    }
}

fn run_project(function: &str, callers: &[(&str, &str)], op: Op) -> Result<Edited, EditError> {
    let function = text(function);
    let callers: Vec<(&str, String)> = callers.iter().map(|(path, rest)| (*path, text(rest))).collect();
    let mut files = Files::new();
    files.insert(FILE, &function);
    for (path, caller) in &callers {
        files.insert(path, caller);
    }
    apply(&files, &op, &Project, &Limits::default())
}

const CALLED: &str = "fn f(a: f64, gain: f64 = 1) -> (y: f64) {\n    y = a\n}\n";

#[test]
fn an_input_renamed_is_renamed_in_the_calls_that_name_it() {
    let caller = "fn main(k: f64) -> (r: f64) {\n    r = f(1, gain: k)\n}\n\nlayout {\n    wire k -> r.gain via (10, 10)\n}\n";
    let edited = run_project(CALLED, &[(CALLER, caller)], rename(input("gain"), "scale")).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(text("fn f(a: f64, scale: f64 = 1) -> (y: f64) {\n    y = a\n}\n").as_str()));
    assert_eq!(
        edited.texts.get(CALLER).map(String::as_str),
        Some(text("fn main(k: f64) -> (r: f64) {\n    r = f(1, scale: k)\n}\n\nlayout {\n    wire k -> r.scale via (10, 10)\n}\n").as_str())
    );
}

#[test]
fn a_call_by_position_and_a_call_of_another_function_stay() {
    let by_position = "fn main() -> (r: f64) {\n    r = f(1, 2)\n}\n";
    let other = "use core\n\nfn other(gain: f64) -> (r: f64) {\n    r = core.abs(gain)\n}\n";
    let edited = run_project(CALLED, &[(CALLER, by_position), ("other.lcf", other)], rename(input("gain"), "scale"))
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.len(), 1, "{edited:?}");
}

#[test]
fn a_caller_that_does_not_parse_stops_the_rename() {
    let broken = "fn main() -> (r: f64) {\n    r = f(1, gain: \n}\n";
    let error = run_project(CALLED, &[(CALLER, broken)], rename(input("gain"), "scale")).err();
    assert_eq!(error.map(|error| error.code), Some("file.unparsed"));
    // An output is never named by a call: the callers are not read.
    let edited = run_project(CALLED, &[(CALLER, broken)], rename(output("y"), "out")).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.len(), 1);
    // A file that does not parse and writes no call of `f` is no caller: it stops nothing.
    let unrelated = "fn other() -> (r: f64) {\n    r = \n}\n";
    let edited = run_project(CALLED, &[(CALLER, unrelated)], rename(input("gain"), "scale")).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.len(), 1);
}

#[test]
fn a_type_of_a_library_renames_a_value_named_after_it() {
    // #317: `use visa` comes with the type, and the value `visa` takes a suffix (N-14), its reader following.
    let before = "fn f(x: f64) -> (y: f64) {\n    visa = x\n    y = visa\n}\n";
    let edited = gives(
        before,
        add(input("dmm"), "visa.session", None, None),
        "use visa\n\nfn f(x: f64, dmm: visa.session) -> (y: f64) {\n    visa2 = x\n    y = visa2\n}\n",
    );
    let renamed: Vec<(&str, &str)> = edited.renamed.iter().map(|renamed| (renamed.from.as_str(), renamed.to.as_str())).collect();
    assert_eq!(renamed, [("visa", "visa2")]);
    // A terminal that would bear the name of the library is refused, new or not: the callers would change.
    refused(SIMPLE, add(input("visa"), "visa.session", None, None), "name.taken");
    refused("fn f(visa: f64) -> (y: f64) {\n    y = visa\n}\n", set_type(input("visa"), "visa.session"), "name.taken");
}

// signature.set_type and signature.set_default

#[test]
fn a_type_changes_where_it_is_written() {
    gives(SIMPLE, set_type(input("x"), "f64[V]"), "fn f(x: f64[V]) -> (y: f64) {\n    y = x\n}\n");
    gives(SIMPLE, set_type(output("y"), "[visa.session]"), "use visa\n\nfn f(x: f64) -> (y: [visa.session]) {\n    y = x\n}\n");
    assert_eq!(refused(SIMPLE, set_type(input("x"), "f64 +"), "params.invalid").parameter, Some("type"));
}

#[test]
fn a_default_is_set_and_removed() {
    gives(SIMPLE, set_default(input("x"), Some("1.5")), "fn f(x: f64 = 1.5) -> (y: f64) {\n    y = x\n}\n");
    gives("fn f(x: f64 = 1.5) -> (y: f64) {\n    y = x\n}\n", set_default(input("x"), None), SIMPLE);
    assert_eq!(refused(SIMPLE, set_default(output("y"), Some("1")), "params.invalid").parameter, Some("terminal"));
    assert_eq!(refused(SIMPLE, set_default(input("x"), Some("y")), "params.invalid").parameter, Some("default"));
    assert_eq!(refused(SIMPLE, set_default(input("x"), Some("(")), "params.invalid").parameter, Some("default"));
}
