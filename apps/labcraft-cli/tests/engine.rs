//! The session of `engine` on the disk, with the libraries shipped with LabCraft and `labcraft-visa`: what its commands
//! give is what the command line gives, and what the documents say. Expected values are written by hand from
//! `libs/visa/sim/*.toml`, `libs/visa/simulation.md` (sections 2, 3, 9) and appendix A of the spec.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use labcraft_cli::disk::Disk;
use labcraft_engine::{Config, Session};
use labcraft_types::Limits;
use serde_json::{Value, json};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").canonicalize().unwrap()
}

/// A session on `examples/`, on a thread as large as the work needs, as the apps run it.
fn on_examples<T: Send>(work: impl FnOnce(&mut Session) -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || {
                let config = Config {
                    root: repository().join("examples"),
                    lib_paths: Vec::new(),
                    limits: Limits::default(),
                    shipped: labcraft_cli::shipped(),
                    natives: labcraft_libs::natives(),
                    clock: Arc::new(labcraft_engine::Still),
                    process: 0,
                };
                let mut session = Session::new(Arc::new(Disk), config, true).unwrap();
                work(&mut session)
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

fn ok(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {}", error.to_json()))
}

#[test]
fn library_check_gives_what_labcraft_cli_check_json_gives() {
    let output =
        Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).args(["check", "--json", "."]).current_dir(repository().join("examples")).output().unwrap();
    let from_the_command_line: Value = serde_json::from_slice(&output.stdout).unwrap();
    let from_the_session = on_examples(|session| ok(session, "library.check", json!({})));
    assert_eq!(from_the_session, from_the_command_line);
    assert_eq!(from_the_session["status"], "ok", "{from_the_session}");
}

#[test]
fn functions_list_writes_visa_and_scpi_as_appendix_a_does() {
    let listed = on_examples(|session| ok(session, "functions.list", json!({ "library": "visa" })));
    let open = listed["entries"].as_array().unwrap().iter().find(|entry| entry["name"] == "visa.open").unwrap();
    assert_eq!(
        open["signature"],
        "fn visa.open(resource: string, timeout: duration = 2 s, err: error = none) -> (session: visa.session, err: error)"
    );
    assert_eq!(open["doc"], "Opens a session on a resource (`SIM::…`: a simulated instrument).");
    assert_eq!(open["categories"], json!(["io.timeout", "visa.invalid_session", "visa.resource_not_found"]), "the categories of its manifest");
    let close = listed["entries"].as_array().unwrap().iter().find(|entry| entry["name"] == "visa.close").unwrap();
    assert_eq!(close["always"], true, "`always` in its manifest");
    let session = listed["entries"].as_array().unwrap().iter().find(|entry| entry["name"] == "visa.session").unwrap();
    assert_eq!((session["kind"].as_str(), session["signature"].as_str()), (Some("type"), Some("type visa.session = opaque")));
    let scpi = on_examples(|session| ok(session, "functions.list", json!({ "library": "scpi", "filter": "query_f64" })));
    assert_eq!(
        scpi["entries"][0]["signature"],
        "fn scpi.query_f64(session: visa.session, command: string, err: error = none) -> (value: f64, session: visa.session, err: error)"
    );
    let own = on_examples(|session| ok(session, "functions.list", json!({ "library": "dmm_driver" })));
    assert_eq!(
        own["entries"][0]["signature"],
        "fn dmm_driver.read_dc_volts(session: visa.session, err: error = none) -> (volts: f64[V], session: visa.session, err: error)"
    );
}

#[test]
fn simulation_describe_says_what_a_shipped_file_defines() {
    let described = on_examples(|session| ok(session, "simulation.describe", json!({ "path": "<labcraft>/visa/sim/dmm-230V.toml" })));
    // `idn` as given, `latency_ms` 0 by default (simulation.md 2); the value at 0 ms is `value`, since `start_ms` and
    // `settle_ms` are 0 (section 3).
    let instrument = json!({
        "resource": "SIM::DMM0",
        "idn": "SIM,DMM,0,1.0",
        "latency_ms": 0.0,
        "queries": [{ "query": "MEAS:VOLT:DC?", "reads": { "quantity": "voltage", "value_at_0_ms": 230.0, "noise": 0.05 } }],
        "set": [],
        "accept": [],
        "faults": { "timeout": [], "error_on": [], "error_queue": [] }
    });
    assert_eq!(
        described,
        json!({ "file": "<labcraft>/visa/sim/dmm-230V.toml", "parts": [{ "library": "visa", "description": { "instruments": [instrument] } }] })
    );
    let faulty = on_examples(|session| ok(session, "simulation.describe", json!({ "path": "<labcraft>/visa/sim/dmm-scpi-error.toml" })));
    let instrument = &faulty["parts"][0]["description"]["instruments"][0];
    assert_eq!(instrument["idn"], "SIM,SIM::DMM0,0,1.0", "the default of `idn`");
    assert_eq!(
        instrument["faults"]["error_queue"],
        json!([{ "code": -221, "message": "Settings conflict" }, { "code": -113, "message": "Undefined header" }])
    );
}

#[test]
fn project_describe_lists_the_shipped_simulation_files_with_their_resources() {
    // Once the first check has ended: until then, the summary says `pending` and no file says what it declares.
    let described = on_examples(|session| {
        ok(session, "library.check", json!({}));
        ok(session, "project.describe", json!({}))
    });
    assert_eq!(described["summary"]["errors"], 0, "{}", described["summary"]);
    let simulations = described["simulations"].as_array().unwrap();
    let bench = simulations.iter().find(|file| file["file"] == "<labcraft>/visa/sim/bench.toml").unwrap();
    assert_eq!(bench["resources"], json!(["SIM::DMM0", "SIM::PSU0", "SIM::RELAY0"]));
    assert_eq!(simulations.len(), 15, "the files of libs/visa/sim");
    let root = described["libraries"].as_array().unwrap().iter().find(|library| library["folder"] == ".").unwrap();
    let check_voltage = root["files"].as_array().unwrap().iter().find(|file| file["file"] == "check_voltage.lcf").unwrap();
    assert_eq!(check_voltage["declares"], json!({ "function": "check_voltage" }));
}

/// A run on the disk (commands.md 7 and 8), in a folder of its own, out of the repository: the report of scenario 1 on
/// `check_voltage` against `dmm-230V.toml`, whose reading `tests/acceptance/runs/noise.py` gives (`230.071217`), and a
/// second run whose trace replaces the first while a reader holds it open, as the viewer does.
#[test]
fn a_run_on_the_disk_writes_its_trace_and_replaces_one_being_read() {
    let folder = std::env::temp_dir().join(format!("labcraft-cli-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    for file in ["check_voltage.lcf", "Verdict.lct"] {
        std::fs::copy(repository().join("examples").join(file), folder.join(file)).unwrap();
    }
    let root = folder.clone();
    let reports = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || {
                let config = Config {
                    root,
                    lib_paths: Vec::new(),
                    limits: Limits::default(),
                    shipped: labcraft_cli::shipped(),
                    natives: labcraft_libs::natives(),
                    clock: Arc::new(labcraft_engine::Still),
                    process: std::process::id(),
                };
                let mut session = Session::new(Arc::new(Disk), config, false).unwrap();
                let params = json!({ "function": "check_voltage", "sim": "<labcraft>/visa/sim/dmm-230V.toml", "args": { "resource": "SIM::DMM0" } });
                let first = ok(&mut session, "run.start", params.clone());
                let reader = labcraft_engine::Workspace::open(&Disk, &session.root().join(".labcraft/runs/check_voltage.trace.jsonl"), 0).unwrap();
                let second = ok(&mut session, "run.start", params);
                drop(reader);
                (first, second)
            })
            .unwrap()
            .join()
            .unwrap()
    });
    let (first, second) = reports;
    assert_eq!(first["status"], "ok", "{first}");
    assert_eq!(first["outputs"][1], json!({ "name": "voltage", "type": "f64[V]", "value": "230.071217 V" }));
    assert_eq!(second["trace"]["file"], ".labcraft/runs/check_voltage.trace.jsonl");
    assert!(second["trace"].get("error").is_none(), "renamed over the trace held open: {second}");
    let trace = std::fs::read_to_string(folder.join(".labcraft/runs/check_voltage.trace.jsonl")).unwrap();
    assert!(trace.starts_with("{\"schema\":\"labcraft-trace/1\",\"run\":\"r2\""), "the last run's trace");
    assert_eq!(std::fs::read_to_string(folder.join(".labcraft/.gitignore")).unwrap(), "*\n!.gitignore\n!config.toml\n");
    // The process holds its lock beside the trace until it lets it go, as the command line does as it ends.
    let lock = format!(".{}.lock", std::process::id());
    assert!(folder.join(".labcraft/runs").join(&lock).is_file(), "{lock}");
    labcraft_engine::runs::let_go();
    let left: Vec<_> = std::fs::read_dir(folder.join(".labcraft/runs")).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left.len(), 1, "no `.partial` and no lock left: {left:?}");
    std::fs::remove_dir_all(&folder).unwrap();
}

/// `diagram.svg` gives, in `image`, the SVG that `labcraft-cli diagram --svg` writes, and its report less the paths of
/// the files (commands.md 6.5; decision of the owner, 2026-11-11). The expected SVGs are those that the command line
/// wrote, which other tests compare to the snapshots.
#[test]
fn diagram_svg_gives_the_svgs_that_labcraft_cli_diagram_writes() {
    let out = std::env::temp_dir().join(format!("labcraft-diagram-svg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let output = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .args(["diagram", "--svg", "--json", "--out"])
        .arg(&out)
        .arg(".")
        .current_dir(repository().join("examples"))
        .output()
        .unwrap();
    let mut from_the_command_line: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut from_the_session = on_examples(|session| ok(session, "diagram.svg", json!({ "folder": "." })));
    let written = from_the_command_line["diagrams"].as_array_mut().unwrap();
    let given = from_the_session["diagrams"].as_array_mut().unwrap();
    assert_eq!(written.len(), given.len());
    assert!(given.len() > 10, "{}", given.len());
    for (written, given) in written.iter_mut().zip(given.iter_mut()) {
        let file = out.join(written["file"].as_str().unwrap()).with_extension("svg");
        let svg = std::fs::read_to_string(&file).unwrap();
        assert_eq!(given["image"].as_str(), Some(svg.as_str()), "{}", written["function"]);
        written.as_object_mut().unwrap().remove("svg");
        given.as_object_mut().unwrap().remove("image");
    }
    assert_eq!(from_the_session, from_the_command_line);
    let _ = std::fs::remove_dir_all(&out);
}

/// What the canvas selects on the drawing of each function of `examples/` is a name that `view.select` takes
/// (commands.md 9.2): each node that has a name, each wire that the drawing names, each terminal of the frame of the
/// function. A name of the drawing that the command refused would make a click select nothing.
#[test]
fn view_select_takes_every_name_that_the_drawings_of_the_examples_give() {
    on_examples(|session| {
        session.with_window();
        ok(session, "view.open", json!({ "folder": "." }));
        let functions: Vec<String> = session.view().functions.iter().map(|listed| listed.function.clone()).collect();
        assert!(functions.len() > 10, "{functions:?}");
        let mut counted = (0, 0, 0);
        let mut lines = 0;
        for function in functions {
            ok(session, "view.show_function", json!({ "function": function }));
            let Some(drawing) = session.view().main.drawing.clone() else { continue };
            let nodes: Vec<String> = drawing.placed.iter().map(|placed| placed.name.clone()).filter(|name| !name.is_empty()).collect();
            let wires: Vec<String> = drawing.groups.iter().filter_map(|group| group.wire.as_ref()).flat_map(|wire| wire.named.clone()).collect();
            let terminals: Vec<String> = drawing.hints.iter().filter_map(|hint| hint.selects.clone()).collect();
            counted = (counted.0 + nodes.len(), counted.1 + wires.len(), counted.2 + terminals.len());
            lines += nodes.iter().filter(|name| name.starts_with('@')).count();
            for (part, names) in [("nodes", &nodes), ("wires", &wires), ("terminals", &terminals)] {
                for name in names {
                    if let Err(error) = session.execute("view.select", &json!({ "function": function, part: [name] })) {
                        panic!("{function}: `{name}` in `{part}`: {}", error.to_json());
                    }
                }
            }
        }
        // Each kind is met: the test would pass on drawings that name nothing; and the nodes named by their line, the
        // three `s` of the arms of `station` among them (commands.md 9.2).
        assert!(counted.0 > 50 && counted.1 > 50 && counted.2 > 20, "{counted:?}");
        assert!(lines >= 3, "{lines} nodes named by their line");
    });
}

/// The program files of `examples/`, in memory under `/p`, so that a test edits them without touching the repository.
fn examples_in_memory() -> Arc<labcraft_engine::Memory> {
    let memory = Arc::new(labcraft_engine::Memory::new());
    let root = repository().join("examples");
    let mut pending = vec![root.clone()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "lcf" || extension == "lct" || extension == "toml") {
                let relative = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                memory.put(format!("/p/{relative}"), std::fs::read(&path).unwrap());
            }
        }
    }
    memory
}

/// What a diff of lines shows between `before` and `after`, as git counts it: the lines added and the lines removed,
/// outside a longest sequence of lines that both keep in their order.
fn changed_lines(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
    let (old, new): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    // The length of the longest common sequence of each pair of suffixes.
    let mut longest = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            longest[i][j] = if old[i] == new[j] { longest[i + 1][j + 1] + 1 } else { longest[i + 1][j].max(longest[i][j + 1]) };
        }
    }
    let (mut added, mut removed) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            (i, j) = (i + 1, j + 1);
        } else if j < new.len() && (i == old.len() || longest[i][j + 1] >= longest[i + 1][j]) {
            added.push(new[j].to_owned());
            j += 1;
        } else {
            removed.push(old[i].to_owned());
            i += 1;
        }
    }
    (added, removed)
}

/// Scenario 5 of `docs/acceptance-m1.md`, point 1: moving one node of a function of `examples/` changes its entry of
/// `layout` alone, or adds it, with the section when the file had none; a node that the section cannot name gets its
/// label first (commands.md 6.7, `node.move`). And where it goes is where it was in its frame, plus the move (spec 11):
/// the place in its frame of the new drawing is the old one, plus `by`. Each move is undone before the next.
#[test]
fn scenario_5_moving_a_node_changes_one_entry_of_layout() {
    let memory = examples_in_memory();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || {
                let config = Config {
                    root: PathBuf::from("/p"),
                    lib_paths: Vec::new(),
                    limits: Limits::default(),
                    shipped: labcraft_cli::shipped(),
                    natives: labcraft_libs::natives(),
                    clock: Arc::new(labcraft_engine::Still),
                    process: 0,
                };
                let workspace: Arc<dyn labcraft_engine::Workspace> = memory.clone();
                let mut session = Session::new(workspace, config, true).unwrap();
                session.with_window();
                ok(&mut session, "view.open", json!({ "folder": "." }));
                let listed: Vec<(String, String)> = session
                    .view()
                    .functions
                    .iter()
                    .filter(|listed| listed.drawable)
                    .map(|listed| (listed.function.clone(), listed.file.clone()))
                    .collect();
                let (mut moved, mut labels, mut frames_moved) = (0, 0, 0);
                // Down and to the right, then up and to the left, past the top of a small frame (the move that brought a
                // node back to its place in an arm, before the decision of the owner of 2026-11-17).
                for ((function, file), by) in listed.iter().flat_map(|listed| [(listed.clone(), (8, 16)), (listed.clone(), (-16, -40))]) {
                    ok(&mut session, "view.show_function", json!({ "function": function }));
                    let Some(before) = session.view().main.drawing.clone() else { continue };
                    let path = format!("/p/{file}");
                    let text = String::from_utf8(memory.get(&path).unwrap()).unwrap();
                    for (at, placed) in before.placed.iter().enumerate().filter(|(_, placed)| !placed.name.is_empty()) {
                        let result = ok(&mut session, "node.move", json!({ "file": file, "nodes": [placed.name], "by": [by.0, by.1] }));
                        let after_text = String::from_utf8(memory.get(&path).unwrap()).unwrap();
                        let (added, removed) = changed_lines(&text, &after_text);
                        let entry = |line: &String| line.trim_start().starts_with("node ") && line.contains(" at (");
                        let labelled = result["renamed"].as_array().is_some_and(|renamed| !renamed.is_empty());
                        let section = !text.contains("\nlayout {");
                        // The entry, and its label on the line of the node; the section, its end, and a blank line before it.
                        let room = 1 + usize::from(labelled) + if section { 3 } else { 0 };
                        assert!(
                            added.iter().filter(|line| entry(line)).count() == 1 && added.len() <= room,
                            "{function}, {}: added {added:?}",
                            placed.name
                        );
                        assert!(removed.len() <= 1 + usize::from(labelled), "{function}, {}: removed {removed:?}", placed.name);
                        assert!(removed.iter().all(|line| entry(line) || labelled), "{function}, {}: removed {removed:?}", placed.name);
                        // Where it went: in its frame, where it was plus the move. The app tells the session of what it wrote.
                        session.files_changed();
                        let after = session.view().main.drawing.clone().unwrap();
                        let now = after.placed.iter().position(|found| found.line == placed.line && found.body == placed.body).unwrap();
                        let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
                        let (x, y) = before.in_frame(at).unwrap();
                        let (x1, y1) = after.in_frame(now).unwrap();
                        assert_eq!((unit(x1), unit(y1)), (unit(x) + by.0, unit(y) + by.1), "{function}, {}", placed.name);
                        assert_eq!(after.placed[now].at, Some(((unit(x) + by.0) * 100, (unit(y) + by.1) * 100)));
                        // And on the screen: its frame stays where it was from the frame of the function, which the canvas
                        // keeps in place, so the node goes where it is dropped (a frame that its structure moved back over
                        // it would bring it back: decision of the owner, 2026-11-17).
                        let from_function = |drawing: &labcraft_diagram::Drawing| {
                            let (frame, function) = (drawing.bodies[placed.body].origin, drawing.bodies[0].origin);
                            (frame.0 - function.0, frame.1 - function.1)
                        };
                        // A node that got its label shows it above itself, which can move what is around it by the height
                        // of a line: counted, not held.
                        if labelled {
                            frames_moved += usize::from(from_function(&after) != from_function(&before));
                        } else {
                            assert_eq!(from_function(&after), from_function(&before), "{function}, {}: its frame moved", placed.name);
                        }
                        ok(&mut session, "history.undo", json!({}));
                        session.files_changed();
                        assert_eq!(String::from_utf8(memory.get(&path).unwrap()).unwrap(), text, "undo gives the bytes back");
                        moved += 1;
                        labels += usize::from(labelled);
                    }
                }
                // The three `s` of the arms of `station` are named by their line: they get a label (commands.md 9.2).
                assert!(moved > 50 && labels >= 3, "{moved} nodes moved, {labels} labelled");
                eprintln!("scenario 5: {moved} nodes moved, {labels} labelled, of which {frames_moved} moved their frame");
            })
            .unwrap()
            .join()
            .unwrap();
    });
}

/// What the canvas says of a wire while it is drawn is what `wire.connect` does at the drop (notation 2.5, 2.6: the
/// filter is the predicate of the command): on each function of `examples/`, a wire between two ports, judged on the
/// drawing by `labcraft_diagram::wiring` from its start, an output, and from its end, an input, which must agree, then
/// given to the command with `replace`, which a drop on a wired input passes, and without it when the canvas says the
/// wire replaces something, which the command then refuses (`wire.occupied`). Every pair of a small function, and of a
/// large one a pair in so many, the same each run. A wire that the canvas takes the command writes, and one that it
/// refuses the command refuses with the same code; each wire written is undone before the next. Three cases are left to
/// the command, which judges again at the drop (architecture 12.4), and counted: an input that takes a family whatever
/// its unit or a type that the call settles, the canvas taking it, where `check` alone knows what the new wire changes
/// downstream; a value whose name the reader sees in another body, which the canvas refuses for the port drawn and the
/// command takes for the value that the name gives; and a field of a template that the call does not give yet,
/// which the canvas refuses.
#[test]
fn the_canvas_judges_a_wire_as_wire_connect_does() {
    let memory = examples_in_memory();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || {
                let config = Config {
                    root: PathBuf::from("/p"),
                    lib_paths: Vec::new(),
                    limits: Limits::default(),
                    shipped: labcraft_cli::shipped(),
                    natives: labcraft_libs::natives(),
                    clock: Arc::new(labcraft_engine::Still),
                    process: 0,
                };
                let workspace: Arc<dyn labcraft_engine::Workspace> = memory.clone();
                let mut session = Session::new(workspace, config, true).unwrap();
                session.with_window();
                ok(&mut session, "view.open", json!({ "folder": "." }));
                let listed: Vec<(String, String)> = session
                    .view()
                    .functions
                    .iter()
                    .filter(|listed| listed.drawable)
                    .map(|listed| (listed.function.clone(), listed.file.clone()))
                    .collect();
                let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
                let mut wrong = Vec::new();
                for (function, file) in listed {
                    ok(&mut session, "view.show_function", json!({ "function": function }));
                    let Some(drawing) = session.view().main.drawing.clone() else { continue };
                    let path = format!("/p/{file}");
                    let text = String::from_utf8(memory.get(&path).unwrap()).unwrap();
                    let mut pairs = Vec::new();
                    let backward: Vec<Vec<Option<labcraft_diagram::Verdict>>> = (0..drawing.ports.len())
                        .map(|at| if drawing.ports[at].input { labcraft_diagram::wiring(&drawing, at) } else { Vec::new() })
                        .collect();
                    for from in (0..drawing.ports.len()).filter(|&at| !drawing.ports[at].input && !drawing.ports[at].name.is_empty()) {
                        for (to, verdict) in labcraft_diagram::wiring(&drawing, from).into_iter().enumerate() {
                            if let Some(verdict) = verdict {
                                // From the input, the same wire is judged the same.
                                let back = backward[to].get(from).cloned().flatten();
                                assert_eq!(
                                    back.as_ref(),
                                    Some(&verdict),
                                    "{function}: from {} back to {}",
                                    drawing.ports[to].name,
                                    drawing.ports[from].name
                                );
                                pairs.push((from, to, verdict));
                            }
                        }
                    }
                    let named_twice = |name: &str| drawing.ports.iter().filter(|port| !port.input && port.name == name).count() > 1;
                    // Every pair of a small function; of a large one, a pair in so many, the same each run.
                    let step = pairs.len().div_ceil(400).max(1);
                    for (rank, (from_port, to_port, verdict)) in pairs.into_iter().step_by(step).enumerate() {
                        let (source, target) = (&drawing.ports[from_port], &drawing.ports[to_port]);
                        let (from, to) = (source.name.clone(), target.name.clone());
                        // A `yield` is named with its arm (commands.md 9.2).
                        let mut params = json!({ "file": file, "from": from, "to": to, "replace": true });
                        if !target.arm.is_empty() {
                            params["arm"] = json!(target.arm);
                        }
                        // Out of loops, each takes the value out: a carry of its last value or a `collect`, by turns, and a
                        // `collect` out of a `for parallel`, which takes no carry.
                        let passages: Vec<&str> =
                            match &verdict {
                                labcraft_diagram::Verdict::Takes { exits, .. } => exits
                                    .iter()
                                    .map(|&body| {
                                        if rank % 2 == 1 || drawing.bodies.get(body).is_some_and(|found| found.parallel) {
                                            "collect"
                                        } else {
                                            "carry"
                                        }
                                    })
                                    .collect(),
                                labcraft_diagram::Verdict::Refused { .. } => Vec::new(),
                            };
                        let exits = passages.len();
                        if exits > 0 {
                            params["through"] = json!(passages);
                        }
                        let mut given = session.execute("wire.connect", &params);
                        // A wire out of a loop that the canvas refuses for what comes after the passages (a cycle) is
                        // judged with them.
                        if exits == 0
                            && given.as_ref().is_err_and(|error| error.code == "wire.out_of_scope" && error.message.contains("inside a loop"))
                        {
                            params["through"] = json!("collect");
                            given = session.execute("wire.connect", &params);
                        }
                        // Out of loops, the canvas judges the type of the passages once they are chosen.
                        let collects = passages.iter().filter(|passage| **passage == "collect").count();
                        let said = match &verdict {
                            labcraft_diagram::Verdict::Takes { .. } if exits > 0 => {
                                match labcraft_diagram::passage_refusal(&drawing, source, target, collects) {
                                    Some(refusal) => refusal.code.to_owned(),
                                    None => "takes".to_owned(),
                                }
                            }
                            labcraft_diagram::Verdict::Takes { .. } => "takes".to_owned(),
                            labcraft_diagram::Verdict::Refused { refusal, .. } => refusal.code.to_owned(),
                        };
                        let did = match &given {
                            Ok(_) => "takes".to_owned(),
                            Err(error) => error.code.to_owned(),
                        };
                        *counts.entry(format!("{said} / {did}")).or_insert(0) += 1;
                        if exits > 0 && did == "takes" {
                            *counts.entry("through".to_owned()).or_insert(0) += 1;
                        }
                        // An input of the function to the output of its name (R-2) is left to the command, which
                        // changes nothing.
                        if to == format!("output {from}") {
                            *counts.entry("left to the command".to_owned()).or_insert(0) += 1;
                            if given.as_ref().is_ok_and(|result| !result["step"].is_null()) {
                                ok(&mut session, "history.undo", json!({}));
                            }
                            continue;
                        }
                        // Without `replace`, a wire that replaces something is refused, and one that does not is written.
                        if let (labcraft_diagram::Verdict::Takes { replaces: true, exits }, Ok(_)) = (&verdict, &given)
                            && exits.is_empty()
                        {
                            if !given.as_ref().is_ok_and(|result| result["step"].is_null()) {
                                ok(&mut session, "history.undo", json!({}));
                            }
                            let mut params = json!({ "file": file, "from": from, "to": to });
                            if !target.arm.is_empty() {
                                params["arm"] = json!(target.arm);
                            }
                            let plain = session.execute("wire.connect", &params);
                            assert_eq!(
                                plain.as_ref().err().map(|error| error.code),
                                Some("wire.occupied"),
                                "{function}: {from} -> {to} without replace"
                            );
                            *counts.entry("replaces".to_owned()).or_insert(0) += 1;
                            assert_eq!(String::from_utf8(memory.get(&path).unwrap()).unwrap(), text, "nothing written");
                            continue;
                        }
                        let left = (said == "takes" && did == "wire.type_mismatch" && target.ty.is_none())
                            || (said == "wire.out_of_scope" && named_twice(&from))
                            // A name of several bodies names none of them out of them (9.2): the editor sends `node.output`.
                            || (did == "edit.ambiguous" && named_twice(&from))
                            // A name of several arms is the one that the reader sees, for the command (9.2).
                            || (said == "wire.cycle" && named_twice(&from))
                            // The template of a `string.format` (`node.template`, a name that no field takes: spec 5.4) is refused by the
                            // command too; the field of a template that the call does not give yet is wired from M1 step 4.
                            || (!target.never.is_empty() && !target.name.ends_with(".template"));
                        if left {
                            *counts.entry("left to the command".to_owned()).or_insert(0) += 1;
                        } else if said != did {
                            let message = given.as_ref().err().map(|error| error.message.clone()).unwrap_or_default();
                            wrong.push(format!("{function}: {from} -> {to}: the canvas says {said}, the command {did} ({message})"));
                        }
                        // A wire that is there already changes nothing, and makes no step.
                        if given.as_ref().is_ok_and(|result| !result["step"].is_null()) {
                            ok(&mut session, "history.undo", json!({}));
                            assert_eq!(String::from_utf8(memory.get(&path).unwrap()).unwrap(), text, "undo gives the bytes back");
                        }
                    }
                }
                eprintln!("{counts:?}");
                assert!(wrong.is_empty(), "{} of {}:\n{}", wrong.len(), counts.values().sum::<usize>(), wrong.join("\n"));
                // Each verdict is met, agreed on.
                // A wire out of a loop is no longer refused (commands.md 6.7, `through`): the examples hold few other
                // values out of scope.
                for code in ["takes", "wire.cycle", "wire.type_mismatch"] {
                    assert!(counts.get(&format!("{code} / {code}")).is_some_and(|&count| count > 10), "{code}: {counts:?}");
                }
                // Most wires out of a loop of the sample are of another type than their input: a few are written.
                assert!(counts.get("through").is_some_and(|&count| count > 5), "{counts:?}");
                assert!(counts.get("replaces").is_some_and(|&count| count > 10), "{counts:?}");
            })
            .unwrap()
            .join()
            .unwrap();
    });
}
