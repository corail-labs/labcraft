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
                    natives: vec![Arc::new(labcraft_visa::Visa)],
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
                    natives: vec![Arc::new(labcraft_visa::Visa)],
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
    assert_eq!(std::fs::read_to_string(folder.join(".labcraft/.gitignore")).unwrap(), "*\n");
    let left: Vec<_> = std::fs::read_dir(folder.join(".labcraft/runs")).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left.len(), 1, "no `.partial` left: {left:?}");
    std::fs::remove_dir_all(&folder).unwrap();
}
