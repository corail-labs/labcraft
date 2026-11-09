//! What a run reaches, and what refuses it (`docs/cli.md`, 6.1), on small libraries written for each rule.

use std::borrow::Cow;
use std::collections::BTreeSet;

use labcraft_types::{Interner, Limits};

use crate::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, Unresolved, check, check_for_run};

fn file(shown: &str, text: &str) -> SourceFile {
    SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) }
}

fn function(name: &str, uses: &[&str], signature: &str, lines: &[&str]) -> String {
    let uses: String = uses.iter().map(|library| format!("use {library}\n")).collect();
    let body: String = lines.iter().map(|line| format!("    {line}\n")).collect();
    let uses = if uses.is_empty() { String::new() } else { format!("{uses}\n") };
    format!("labcraft 1\n\n{uses}fn {name}{signature} {{\n{body}}}\n")
}

fn folder(name: &str, files: Vec<SourceFile>) -> LibrarySource {
    LibrarySource { files, ..LibrarySource::new(name.to_owned(), name.to_owned(), Origin::Project) }
}

/// A host with one dependency, `dep`, found as a `--lib-path` folder would be.
struct Dependency(Vec<(&'static str, String)>);

impl Libraries for Dependency {
    fn find(&self, name: &str, _: Origin) -> Found {
        if name != "dep" {
            return Found::NotFound;
        }
        Found::Library(folder("dep", self.0.iter().map(|(shown, text)| file(shown, text)).collect()))
    }
}

/// `dep.g`, `dep.h`, `dep.k`: `g` calls `h`, `k` stands alone.
fn dependency(g: &[&str], k: &[&str]) -> Dependency {
    Dependency(vec![
        ("dep/g.lcf", function("g", &[], "(x: i32) -> (r: i32)", g)),
        ("dep/h.lcf", function("h", &[], "(x: i32) -> (r: i32)", &["r = x"])),
        ("dep/k.lcf", function("k", &[], "(x: i32) -> (r: i32)", k)),
    ])
}

fn for_run(root: LibrarySource, libraries: &dyn Libraries) -> (Checked, Interner) {
    let mut names = Interner::new();
    let checked = check_for_run(root, libraries, &mut names, &Limits::default());
    (checked, names)
}

fn shown(checked: &Checked, files: &[u32]) -> BTreeSet<String> {
    files.iter().map(|file| checked.files[*file as usize].shown.clone()).collect()
}

fn set(files: &[&str]) -> BTreeSet<String> {
    files.iter().map(|file| (*file).to_owned()).collect()
}

const CALLS_G: &[&str] = &["r = dep.g(1)"];

#[test]
fn a_run_reaches_what_its_function_calls_and_names_in_every_library() {
    let root = folder(
        "",
        vec![
            file("f.lcf", &function("f", &["dep"], "() -> (r: i32, h: fn(i32) -> (i32))", &["r = dep.g(1)", "h = dep.h"])),
            file("other.lcf", &function("other", &["dep"], "() -> (r: i32)", &["r = dep.k(1)"])),
        ],
    );
    // `h` only by its reference.
    let (checked, names) = for_run(root, &dependency(&["r = x"], &["r = x"]));
    assert_eq!(checked.report.errors() + checked.report.warnings(), 0, "{:?}", checked.report.diagnostics());
    let f = checked.function_named("f", &names).unwrap();
    let reach = checked.reach(f);
    assert_eq!(shown(&checked, &reach.functions), set(&["f.lcf", "dep/g.lcf", "dep/h.lcf"]));
    assert_eq!(reach.functions.first().copied(), u32::try_from(f).ok(), "the function run comes first");
    assert!(reach.types.is_empty() && reach.unresolved.is_empty());
    assert!(!reach.refuses(&checked, true));
    // And through a call of a dependency to its own library.
    let root = folder("", vec![file("f.lcf", &function("f", &["dep"], "() -> (r: i32)", CALLS_G))]);
    let (checked, names) = for_run(root, &dependency(&["r = h(x)"], &["r = x"]));
    let reach = checked.reach(checked.function_named("f", &names).unwrap());
    assert_eq!(shown(&checked, &reach.functions), set(&["f.lcf", "dep/g.lcf", "dep/h.lcf"]));
    // The dependency is read for a run: every body has its order, and says what it has to say apart.
    let g = checked.files.iter().find(|file| file.shown == "dep/g.lcf").unwrap();
    assert!(g.orders.is_some());
    assert!(!checked.libraries[g.library as usize].checked);
    assert!(checked.for_run);
}

#[test]
fn check_alone_reads_no_body_of_a_dependency() {
    let root = folder("", vec![file("f.lcf", &function("f", &["dep"], "() -> (r: i32)", CALLS_G))]);
    let mut names = Interner::new();
    let checked = check(root, &dependency(&["r = nowhere(x)"], &["r = x"]), &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0);
    assert!(checked.dependencies.is_empty(), "a body of a dependency is not judged by `check`");
    assert_eq!(checked.files.len(), 1);
    assert!(checked.files[0].orders.is_none() && !checked.for_run);
    assert_eq!(checked.bodies.errors(), 0);
}

#[test]
fn an_error_in_a_body_that_the_run_reaches_refuses_it() {
    let root = folder("", vec![file("f.lcf", &function("f", &["dep"], "() -> (r: i32)", CALLS_G))]);
    let (checked, names) = for_run(root, &dependency(&["r = nowhere(x)"], &["r = x"]));
    // The report says what `check` says: nothing of a body of a dependency.
    assert_eq!(checked.report.errors(), 0);
    assert!(checked.dependencies.is_empty());
    let reach = checked.reach(checked.function_named("f", &names).unwrap());
    assert!(reach.refuses(&checked, false));
    let g = checked.files.iter().position(|file| file.shown == "dep/g.lcf").unwrap();
    assert_eq!(checked.counts_of(g).errors, 1);
    let said: Vec<&str> = checked.bodies.diagnostics().iter().map(|diagnostic| diagnostic.file.as_str()).collect();
    assert_eq!(said, ["dep/g.lcf"]);
    assert_eq!(checked.bodies.diagnostics()[0].code().as_str(), "LC2002", "unknown function (`docs/cli.md`, section 5)");
}

#[test]
fn an_error_in_a_body_that_the_run_does_not_reach_does_not_refuse_it() {
    let root = folder("", vec![file("f.lcf", &function("f", &["dep"], "() -> (r: i32)", CALLS_G))]);
    let (checked, names) = for_run(root, &dependency(&["r = h(x)"], &["r = nowhere(x)"]));
    let reach = checked.reach(checked.function_named("f", &names).unwrap());
    assert!(!reach.refuses(&checked, false));
    // Nor in another function of the folder given, which `check` counts.
    let root = folder(
        "",
        vec![
            file("f.lcf", &function("f", &[], "() -> (r: i32)", &["r = 1"])),
            file("other.lcf", &function("other", &[], "() -> (r: i32)", &["r = nowhere(1)"])),
        ],
    );
    let (checked, names) = for_run(root, &Dependency(Vec::new()));
    assert_eq!(checked.report.errors(), 1);
    assert!(!checked.reach(checked.function_named("f", &names).unwrap()).refuses(&checked, true));
    assert!(checked.reach(checked.function_named("other", &names).unwrap()).refuses(&checked, false));
}

#[test]
fn a_warning_refuses_only_under_deny_warnings_and_only_in_the_folder_given() {
    let unwired = ["r, _ = string.to_f64(\"1\")"];
    let root = folder("", vec![file("f.lcf", &function("f", &["string"], "() -> (r: f64)", &unwired))]);
    let (checked, names) = for_run(root, &Dependency(Vec::new()));
    assert_eq!((checked.report.errors(), checked.report.warnings()), (0, 1));
    let reach = checked.reach(checked.function_named("f", &names).unwrap());
    assert!(!reach.refuses(&checked, false));
    assert!(reach.refuses(&checked, true));
    // The same warning in a dependency: the user cannot fix it there.
    let host = Dependency(vec![("dep/g.lcf", function("g", &["string"], "(x: i32) -> (r: f64)", &unwired))]);
    let root = folder("", vec![file("f.lcf", &function("f", &["dep"], "() -> (r: f64)", CALLS_G))]);
    let (checked, names) = for_run(root, &host);
    let reach = checked.reach(checked.function_named("f", &names).unwrap());
    let g = checked.files.iter().position(|file| file.shown == "dep/g.lcf").unwrap();
    assert_eq!(checked.counts_of(g).warnings, 1);
    assert!(!reach.refuses(&checked, true));
}

#[test]
fn a_run_reaches_the_types_that_it_uses_and_an_error_there_refuses_it() {
    let point = "labcraft 1\n\ntype Point = struct { at: Place, label: string }\n";
    let place = "labcraft 1\n\ntype Place = struct { x: f64, y: f64 }\n";
    let unused = "labcraft 1\n\ntype Unused = struct { x: nowhere }\n";
    let uses_point = function("f", &[], "(p: Point) -> (x: f64)", &["x = p.at.x"]);
    let root = folder("", vec![file("f.lcf", &uses_point), file("Point.lct", point), file("Place.lct", place), file("Unused.lct", unused)]);
    let (checked, names) = for_run(root, &Dependency(Vec::new()));
    assert_eq!(checked.report.errors(), 1, "`Unused` has its error");
    let reach = checked.reach(checked.function_named("f", &names).unwrap());
    assert_eq!(shown(&checked, &reach.types), set(&["Point.lct", "Place.lct"]));
    assert!(!reach.refuses(&checked, false), "an error in a type that the run does not use");
    // `Place` has an error: `Point` is the error type by its name, and the run cannot be lowered.
    let broken = "labcraft 1\n\ntype Place = struct { x: nowhere, y: f64 }\n";
    let root = folder("", vec![file("f.lcf", &uses_point), file("Point.lct", point), file("Place.lct", broken)]);
    let (checked, names) = for_run(root, &Dependency(Vec::new()));
    let f = checked.function_named("f", &names).unwrap();
    assert_eq!(checked.counts_of(f).errors, 0, "one error, one diagnostic: the function says nothing");
    let reach = checked.reach(f);
    assert!(reach.refuses(&checked, false));
    assert!(reach.unresolved.contains(&Unresolved::Type { file: u32::try_from(f).unwrap() }), "{:?}", reach.unresolved);
}

#[test]
fn a_call_to_what_did_not_read_refuses_the_run() {
    // `dep` has a file that does not read, which may declare `g`: the call says nothing, and binds to nothing.
    let host = Dependency(vec![("dep/g.lcf", "labcraft 1\n\nfn g( {\n".to_owned()), ("dep/k.lcf", function("k", &[], "() -> ()", &[]))]);
    let root = folder(
        "",
        vec![
            file("f.lcf", &function("f", &["dep"], "() -> (r: i32)", CALLS_G)),
            file("other.lcf", &function("other", &[], "() -> (r: i32)", &["r = 1"])),
        ],
    );
    let (checked, names) = for_run(root, &host);
    let f = checked.function_named("f", &names).unwrap();
    assert_eq!(checked.counts_of(f).errors, 0);
    let reach = checked.reach(f);
    assert!(reach.unresolved.contains(&Unresolved::Call { file: u32::try_from(f).unwrap() }), "{:?}", reach.unresolved);
    assert!(reach.refuses(&checked, false));
    assert!(!checked.reach(checked.function_named("other", &names).unwrap()).refuses(&checked, false));
}

#[test]
fn a_run_names_its_function_from_the_root_of_the_project() {
    let root = LibrarySource {
        sub_libraries: vec![folder("driver", vec![file("driver/measure.lcf", &function("measure", &[], "() -> (r: i32)", &["r = 1"]))])],
        ..folder("", vec![file("f.lcf", &function("f", &[], "() -> (r: i32)", &["r = 1"]))])
    };
    let (checked, names) = for_run(root, &dependency(&["r = x"], &["r = x"]));
    let at = |path: &str| checked.function_named(path, &names).map(|file| checked.files[file].shown.as_str());
    assert_eq!(at("f"), Some("f.lcf"));
    assert_eq!(at("driver.measure"), Some("driver/measure.lcf"));
    assert_eq!(at("measure"), None);
    assert_eq!(at("nowhere"), None);
    assert_eq!(at("dep.g"), None, "a dependency is not part of the project");
    for wrong in ["", ".", ".f", "f.", "driver..measure", ".driver.measure"] {
        assert_eq!(at(wrong), None, "{wrong:?}");
    }
}

/// One error gives one diagnostic: each of these files says nothing, and leans on a file that did not read.
#[test]
fn what_leans_on_a_file_that_did_not_read_refuses_the_run() {
    let unread_function = ("g.lcf", "labcraft 1\n\nfn g( {\n");
    let unread_type = ("T.lct", "labcraft 1\n\ntype T = struct {\n");
    let cases: [(&str, (&str, &str), &[&str]); 4] = [
        ("a function reference", unread_function, &["h = g", "r = 1"]),
        ("a construction that nothing reads", unread_type, &["p = T { x: 1 }", "r = 1"]),
        ("a variant that nothing reads", unread_type, &["_ = T.a", "r = 1"]),
        ("a call", unread_function, &["r = g(1)"]),
    ];
    for (what, (shown, text), lines) in cases {
        let root = folder("", vec![file("main.lcf", &function("main", &[], "() -> (r: i32)", lines)), file(shown, text)]);
        let (checked, names) = for_run(root, &Dependency(Vec::new()));
        let main = checked.function_named("main", &names).unwrap();
        assert_eq!(checked.counts_of(main).errors, 0, "{what}: {:?}", checked.report.diagnostics());
        assert!(checked.reach(main).refuses(&checked, false), "{what}");
    }
    // The same through a dependency: `dep/T.lct` does not read.
    let host = Dependency(vec![("dep/T.lct", "labcraft 1\n\ntype T = struct {\n".to_owned()), ("dep/k.lcf", function("k", &[], "() -> ()", &[]))]);
    let root = folder("", vec![file("main.lcf", &function("main", &["dep"], "() -> (r: i32)", &["p = dep.T { x: 1 }", "r = 1"]))]);
    let (checked, names) = for_run(root, &host);
    let main = checked.function_named("main", &names).unwrap();
    assert_eq!(checked.counts_of(main).errors, 0, "{:?}", checked.report.diagnostics());
    let reach = checked.reach(main);
    assert!(reach.unresolved.contains(&Unresolved::Elsewhere { file: u32::try_from(main).unwrap() }), "{:?}", reach.unresolved);
    assert!(reach.refuses(&checked, false));
    // A function of the same folder that leans on nothing is not refused for it.
    let root = folder("", vec![file("main.lcf", &function("main", &[], "() -> (r: i32)", &["r = 1"])), file("g.lcf", "labcraft 1\n\nfn g( {\n")]);
    let (checked, names) = for_run(root, &Dependency(Vec::new()));
    assert!(!checked.reach(checked.function_named("main", &names).unwrap()).refuses(&checked, false));
}

/// What the bodies of the dependencies say is one report, bounded as one, whatever their number; the counts stay complete.
#[test]
fn what_the_bodies_of_dependencies_say_is_bounded() {
    let errors: Vec<String> = (0..10).map(|n| format!("r{n} = nowhere(x)")).collect();
    let lines: Vec<&str> = errors.iter().map(String::as_str).chain(["r = x"]).collect();
    let files: Vec<(&'static str, String)> = (0..50)
        .map(|n| {
            let name: &'static str = Box::leak(format!("dep/g{n}.lcf").into_boxed_str());
            (name, function(&format!("g{n}"), &[], "(x: i32) -> (r: i32)", &lines))
        })
        .collect();
    let root = folder("", vec![file("f.lcf", &function("f", &["dep"], "() -> (r: i32)", &["r = dep.g7(1)"]))]);
    let mut limits = Limits::default();
    limits.max_report = 5;
    let mut names = Interner::new();
    let checked = check_for_run(root, &Dependency(files), &mut names, &limits);
    assert_eq!(checked.bodies.errors(), 500, "10 unknown functions in each of 50 files");
    assert!(checked.bodies.diagnostics().len() <= 10, "{}", checked.bodies.diagnostics().len());
    let g7 = checked.files.iter().position(|file| file.shown == "dep/g7.lcf").unwrap();
    assert_eq!(checked.counts_of(g7).errors, 10);
    assert!(checked.reach(checked.function_named("f", &names).unwrap()).refuses(&checked, false));
}

/// A value whose type holds the error type without being it, `[Place]` with a `Place` that has an error: the run cannot
/// give it a value, whatever passes 3 and 4 marked.
#[test]
fn a_type_that_holds_an_error_refuses_the_run() {
    let place = "labcraft 1\n\ntype Place = struct { x: nowhere }\n";
    let lines = [
        "_, n = loop carry places: [Place] = [], carry k = 0 {",
        "    k2 = core.add(k, 1)",
        "    stop = core.ge(k2, 3)",
        "    next places = places",
        "    next k = k2",
        "    until stop",
        "}",
        "r = 1",
    ];
    let root = folder("", vec![file("main.lcf", &function("main", &["core"], "() -> (r: i32)", &lines)), file("Place.lct", place)]);
    let (checked, names) = for_run(root, &Dependency(Vec::new()));
    let main = checked.function_named("main", &names).unwrap();
    assert_eq!(checked.counts_of(main).errors, 0, "{:?}", checked.report.diagnostics());
    let reach = checked.reach(main);
    assert!(reach.unresolved.contains(&Unresolved::Type { file: u32::try_from(main).unwrap() }), "{:?}", reach.unresolved);
    assert!(reach.refuses(&checked, false));
}
