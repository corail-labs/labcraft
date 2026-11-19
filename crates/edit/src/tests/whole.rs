//! `function.rename`, `type.rename` and the text of `type.create` (commands.md 6.7, spec 3.2, 4 and R-22). Each expected
//! text is written by hand from the rule of commands.md: the declaration and every use of it in the project, in one step;
//! a value of the name hides a function (spec 4), a generic variable hides a type.

use crate::{Files, Judge, Limits, Lookup, new_type, rename_function, rename_type};

/// The project of these tests: `f` in `f.lcf`, `Step` (a struct) and `State` (an enum) in their `.lct`, each of the
/// library of the root, also seen as `lib.f`, `lib.Step` and `lib.State` from a library that uses it.
struct Project;

impl Judge for Project {
    fn callable(&self, _: &str, _: &str) -> Lookup {
        Lookup::Unknown(Vec::new())
    }
    fn errors(&self, _: &str, _: &str) -> Vec<labcraft_types::Diagnostic> {
        Vec::new()
    }
    fn function_file(&self, _: &str, call: &str) -> Option<String> {
        match call {
            "f" | "lib.f" => Some("f.lcf".to_owned()),
            "a.b.g" => Some("a/b/g.lcf".to_owned()),
            _ => None,
        }
    }
    fn type_file(&self, _: &str, path: &str) -> Option<String> {
        match path {
            "Step" | "lib.Step" => Some("Step.lct".to_owned()),
            "State" | "lib.State" => Some("State.lct".to_owned()),
            "a.b.Mode" => Some("a/b/Mode.lct".to_owned()),
            _ => None,
        }
    }
}

const F: &str = "labcraft 1\n\n/// Twice.\nfn f(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n";
const MAIN: &str = "labcraft 1\n\nuse lib\n\nfn main(k: f64, measure: fn(f64) -> (f64) = f) -> (a: f64, b: f64, c: f64) {\n    a = f(k)\n    b = lib.f(k)\n    c = core.call(f, k)\n}\n";
/// `f` is a value here: what reads it reads the value.
const HIDDEN: &str = "labcraft 1\n\nfn hidden(f: f64) -> (y: f64) {\n    y = core.neg(f)\n}\n";

fn project<'a>(extra: &[(&'a str, &'a str)]) -> Files<'a> {
    let mut files = Files::new();
    files.insert("f.lcf", F);
    files.insert("main.lcf", MAIN);
    files.insert("hidden.lcf", HIDDEN);
    for (path, text) in extra {
        files.insert(path, text);
    }
    files
}

#[test]
fn a_function_renamed_is_renamed_in_every_call_and_reference() {
    let edited = rename_function(&project(&[]), "f.lcf", "twice", &Project, &Limits::default()).unwrap();
    assert_eq!(edited.names.get("function").map(String::as_str), Some("twice"));
    assert_eq!(
        edited.texts.get("f.lcf").map(String::as_str),
        Some("labcraft 1\n\n/// Twice.\nfn twice(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n")
    );
    assert_eq!(
        edited.texts.get("main.lcf").map(String::as_str),
        Some(
            "labcraft 1\n\nuse lib\n\nfn main(k: f64, measure: fn(f64) -> (f64) = twice) -> (a: f64, b: f64, c: f64) {\n    a = twice(k)\n    b = lib.twice(k)\n    c = core.call(twice, k)\n}\n"
        )
    );
    // The input `f` of `hidden` is no reference to the function.
    assert!(!edited.texts.contains_key("hidden.lcf"));
}

#[test]
fn a_function_that_calls_itself_follows_its_name() {
    let recursive = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = f(x)\n}\n";
    let mut files = Files::new();
    files.insert("f.lcf", recursive);
    let edited = rename_function(&files, "f.lcf", "g", &Project, &Limits::default()).unwrap();
    assert_eq!(edited.texts.get("f.lcf").map(String::as_str), Some("labcraft 1\n\nfn g(x: f64) -> (y: f64) {\n    y = g(x)\n}\n"));
}

#[test]
fn a_rename_takes_a_name_of_the_language_and_stops_at_a_caller_that_does_not_parse() {
    let error = rename_function(&project(&[]), "f.lcf", "for", &Project, &Limits::default()).unwrap_err();
    assert_eq!((error.code, error.parameter), ("name.invalid", Some("name")));
    let error = rename_function(&project(&[]), "f.lcf", "2f", &Project, &Limits::default()).unwrap_err();
    assert_eq!(error.code, "name.invalid");
    let broken = "labcraft 1\n\nfn broken() -> () {\n    y = f(1\n}\n";
    let error = rename_function(&project(&[("broken.lcf", broken)]), "f.lcf", "g", &Project, &Limits::default()).unwrap_err();
    assert_eq!(error.code, "file.unparsed");
    // The same name: nothing to write.
    assert!(rename_function(&project(&[]), "f.lcf", "f", &Project, &Limits::default()).unwrap().texts.is_empty());
    // A type file is no function.
    let error = rename_function(&project(&[("Step.lct", STEP)]), "Step.lct", "g", &Project, &Limits::default()).unwrap_err();
    assert_eq!(error.code, "params.invalid");
}

const STEP: &str = "labcraft 1\n\ntype Step = struct { name: string, limit: f64 }\n";
const STATE: &str = "labcraft 1\n\ntype State = enum { Init, Run, Stop }\n";
/// Holds `Step` in a field.
const PLAN: &str = "labcraft 1\n\ntype Plan = struct { steps: [Step], first: lib.Step }\n";
const RUN: &str = "labcraft 1\n\nuse lib\n\nfn run(s: Step, t: lib.Step, now: State) -> (n: string, later: State) {\n    later = match now {\n        State.Init => yield State.Run\n        lib.State.Run => yield lib.State.Stop\n        _ => yield now\n    }\n    n = s.name\n}\n";
/// `Step` is a generic variable here.
const GENERIC: &str = "labcraft 1\n\nfn same<Step>(x: Step) -> (y: Step) {\n    y = x\n}\n";

fn types<'a>() -> Files<'a> {
    let mut files = Files::new();
    for (path, text) in [("Step.lct", STEP), ("State.lct", STATE), ("Plan.lct", PLAN), ("run.lcf", RUN), ("same.lcf", GENERIC)] {
        files.insert(path, text);
    }
    files
}

#[test]
fn a_type_renamed_is_renamed_in_the_signatures_the_other_types_and_the_variants() {
    let edited = rename_type(&types(), "Step.lct", "Stage", &Project, &Limits::default()).unwrap();
    assert_eq!(edited.names.get("type").map(String::as_str), Some("Stage"));
    assert_eq!(edited.texts.get("Step.lct").map(String::as_str), Some("labcraft 1\n\ntype Stage = struct { name: string, limit: f64 }\n"));
    assert_eq!(edited.texts.get("Plan.lct").map(String::as_str), Some("labcraft 1\n\ntype Plan = struct { steps: [Stage], first: lib.Stage }\n"));
    assert_eq!(edited.texts.get("run.lcf").map(String::as_str), Some(RUN.replace("s: Step, t: lib.Step", "s: Stage, t: lib.Stage").as_str()));
    // A generic variable of the name is no use of the type.
    assert!(!edited.texts.contains_key("same.lcf"));
    let edited = rename_type(&types(), "State.lct", "Phase", &Project, &Limits::default()).unwrap();
    let run = RUN.replace("State", "Phase");
    assert_eq!(edited.texts.get("run.lcf").map(String::as_str), Some(run.as_str()));
    assert_eq!(edited.texts.len(), 2, "{:?}", edited.texts.keys());
}

#[test]
fn a_new_type_is_one_declaration_in_canonical_form() {
    let text = new_type("Reading", "struct {value: f64[V],  ok: bool}", Some("A reading."), &Limits::default()).unwrap();
    assert_eq!(text, "labcraft 1\n\n/// A reading.\ntype Reading = struct { value: f64[V], ok: bool }\n");
    assert_eq!(new_type("Volts", "f64[V]", None, &Limits::default()).unwrap(), "labcraft 1\n\ntype Volts = f64[V]\n");
    for definition in ["", "struct {", "f64[V]\ntype Other = bool", "12"] {
        let error = new_type("Reading", definition, None, &Limits::default()).unwrap_err();
        assert_eq!((error.code, error.parameter), ("params.invalid", Some("definition")), "{definition:?}");
    }
    let error = new_type("match", "bool", None, &Limits::default()).unwrap_err();
    assert_eq!((error.code, error.parameter), ("name.invalid", Some("path")));
}

#[test]
fn a_use_through_a_sub_library_is_renamed() {
    // Spec 4: the contents of the sub-library `b` of `a` are named `a.b.name`.
    let g = "labcraft 1\n\nfn g(x: f64) -> (y: f64) {\n    y = x\n}\n";
    let mode = "labcraft 1\n\ntype Mode = enum { Slow, Fast }\n";
    let top = "labcraft 1\n\nuse a.b\n\nfn top(m: a.b.Mode, x: f64) -> (y: f64, n: a.b.Mode) {\n    y = a.b.g(x)\n    n = a.b.Mode.Fast\n}\n";
    let mut files = Files::new();
    for (path, text) in [("a/b/g.lcf", g), ("a/b/Mode.lct", mode), ("top.lcf", top)] {
        files.insert(path, text);
    }
    let edited = rename_function(&files, "a/b/g.lcf", "h", &Project, &Limits::default()).unwrap();
    assert_eq!(edited.texts.get("top.lcf").map(String::as_str), Some(top.replace("a.b.g(x)", "a.b.h(x)").as_str()));
    let edited = rename_type(&files, "a/b/Mode.lct", "Pace", &Project, &Limits::default()).unwrap();
    assert_eq!(edited.texts.get("top.lcf").map(String::as_str), Some(top.replace("Mode", "Pace").as_str()));
}

#[test]
fn a_new_name_that_a_file_changed_already_gives_is_taken() {
    // `main` has the input `k`, and uses the library `lib`: `f` renamed `k` or `lib` would read as those (decision of
    // the owner, 2026-11-19). `hidden` binds `f` but uses nothing of the function: a name it gives stays free.
    for name in ["k", "lib", "measure"] {
        let error = rename_function(&project(&[]), "f.lcf", name, &Project, &Limits::default()).unwrap_err();
        assert_eq!((error.code, error.parameter), ("name.taken", Some("name")), "{name}");
        assert!(error.message.contains("main.lcf"), "{}", error.message);
    }
    let error = rename_type(&types(), "State.lct", "now", &Project, &Limits::default()).unwrap_err();
    assert_eq!(error.code, "name.taken");
    // A generic variable of a file that uses the type.
    let generic = "labcraft 1\n\nfn pick<T>(x: T, s: Step) -> (y: T) {\n    y = x\n}\n";
    let mut files = types();
    files.insert("pick.lcf", generic);
    let error = rename_type(&files, "Step.lct", "T", &Project, &Limits::default()).unwrap_err();
    assert_eq!(error.code, "name.taken");
}

#[test]
fn a_new_type_has_no_section_after_it() {
    let error = new_type("Volts", "f64[V]\n\nlayout {\n    frame size (1, 1)\n}", None, &Limits::default()).unwrap_err();
    assert_eq!((error.code, error.parameter), ("params.invalid", Some("definition")));
}
