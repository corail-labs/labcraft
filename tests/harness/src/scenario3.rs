//! Scenario 3 of `docs/acceptance-m0.md` in deterministic mode, on `examples/two_benches.lcf` with the simulation files of
//! `libs/visa/sim/`: points 1 to 6, through `check`, `ir`, `runtime` and `visa`, natively and in WASM. The times are worked out
//! by hand from the program: 10 readings spaced by 100 ms, on multimeters without latency, each loop reading `stop_switch`
//! after its wait. The values of the readings depend on the noise of the simulation file, so they are compared between runs
//! here, and with the proof of concept by the corpus of runs (step 5b).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::borrow::Cow;

use labcraft_check::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_ir::{ElementId, Program, lower_for_run};
use labcraft_runtime::Value;
use labcraft_runtime::native::NativeLibrary;
use labcraft_runtime::run::{AbortBy, Control, Native, PanelSetting, Pool, RealTime, Run, RunEnd, SetBy, Settings, Status, Unobserved, Wake, run};
use labcraft_runtime::{FaultCode, simulation};
use labcraft_types::{Interner, Limits};
use labcraft_visa::Visa;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use super::PROGRAMS;

const MS: i64 = 1_000_000;
const TWO_DMM: &str = include_str!("../../../libs/visa/sim/two-dmm.toml");
const TWO_DMM_TIMEOUT: &str = include_str!("../../../libs/visa/sim/two-dmm-timeout.toml");

struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
    }
}

struct Still;

impl Control for Still {
    fn real_ns(&mut self) -> u64 {
        0
    }
}

fn examples() -> LibrarySource {
    let files = PROGRAMS
        .iter()
        .filter_map(|file| {
            let shown = file.path.strip_prefix("examples/")?;
            (!shown.contains('/')).then(|| SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Borrowed(file.bytes)) })
        })
        .collect();
    LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) }
}

struct Bench {
    checked: Checked,
    program: Program,
    names: Interner,
}

fn two_benches() -> Bench {
    let mut names = Interner::new();
    let checked = check_for_run(examples(), &Shipped, &mut names, &Limits::default());
    let start = checked.files.iter().position(|file| file.shown == "two_benches.lcf").unwrap();
    let program = lower_for_run(&checked, start, &mut names).unwrap();
    Bench { checked, program, names }
}

/// The inputs of `two_benches`, their defaults: `SIM::DMM0`, `SIM::DMM1`, 10 readings, 100 ms.
fn defaults() -> Vec<Value> {
    vec![Value::string("SIM::DMM0"), Value::string("SIM::DMM1"), Value::F64(10.0), Value::Duration(100 * MS)]
}

fn run_on(bench: &Bench, sim: &str, settings: Settings) -> RunEnd {
    run_controlled(bench, sim, settings, &mut Still)
}

fn run_controlled(bench: &Bench, sim: &str, settings: Settings, control: &mut dyn Control) -> RunEnd {
    let file = simulation::read(sim, 64).unwrap();
    let simulations = file.judge(&[&Visa as &dyn NativeLibrary]).unwrap();
    let visa = bench.checked.libraries.iter().find(|library| library.name == "visa").unwrap();
    let natives = [Native { library: visa.id, native: &Visa, simulation: simulations.first().and_then(Option::as_ref) }];
    let given = Run { program: &bench.program, names: &bench.names, files: &bench.checked.files, natives: &natives, inputs: defaults(), settings };
    run(given, control, &mut Unobserved)
}

/// Real time on a clock of the test: a wait of the scheduler jumps to the time it waits for, so that what is asserted does
/// not depend on the speed of the machine. The corpus of runs runs the same on the real clock (`s3-2-real-time`).
struct Jumps(u64);

impl Control for Jumps {
    fn real_ns(&mut self) -> u64 {
        self.0
    }

    fn wait(&mut self, wake: &Wake, seen: u64, until: Option<u64>) {
        if wake.seen() == seen
            && let Some(until) = until
        {
            self.0 = self.0.max(until);
        }
    }
}

fn number(value: &Value) -> f64 {
    match value {
        Value::F64(x) => *x,
        other => panic!("{other:?}"),
    }
}

/// The commands each instrument received, with their times and answers: what point 1 compares.
type Received = Vec<(String, Vec<(i64, String, Option<String>)>)>;

fn commands(end: &RunEnd) -> Received {
    end.instruments
        .iter()
        .map(|log| (log.resource.clone(), log.commands.iter().map(|command| (command.at, command.sent.clone(), command.answer.clone())).collect()))
        .collect()
}

#[test]
fn point_2_the_waits_of_the_two_branches_overlap() {
    let bench = two_benches();
    let end = run_on(&bench, TWO_DMM, Settings::default());
    assert_eq!(end.status, Status::Ok, "{:?}", end.fault);
    // 10 readings spaced by 100 ms in each branch, side by side: the clock ends at 1 s, not 2 s.
    assert_eq!(end.clock, 1000 * MS);
    // Each multimeter got its 10 readings; the means are near the values of the simulation file, 230 V and 12 V, within
    // 5 standard deviations of the noise of a mean of 10 (0.05 / √10 and 0.01 / √10).
    assert_eq!(
        end.instruments.iter().map(|log| (log.resource.as_str(), log.commands.len())).collect::<Vec<_>>(),
        [("SIM::DMM0", 10), ("SIM::DMM1", 10)]
    );
    assert!((number(&end.outputs[0]) - 230.0).abs() < 5.0 * 0.05 / 10f64.sqrt(), "{:?}", end.outputs[0]);
    assert!((number(&end.outputs[1]) - 12.0).abs() < 5.0 * 0.01 / 10f64.sqrt(), "{:?}", end.outputs[1]);
    assert!(end.outputs[2].as_error().is_none());
    // Both sessions were opened, and the program closed them.
    assert_eq!(end.resources.len(), 2);
    assert!(end.resources.iter().all(|resource| !resource.info.as_ref().unwrap().open && !resource.released));
}

#[test]
fn point_2_in_real_time_the_waits_overlap_on_the_real_clock() {
    let bench = two_benches();
    let real_time = RealTime { pool: Pool::default(), wake: Wake::default(), epoch: 0 };
    let settings = Settings { max_iterations: None, max_nodes: None, max_real_time: None, real_time: Some(real_time), ..Settings::default() };
    let end = run_controlled(&bench, TWO_DMM, settings, &mut Jumps(0));
    assert_eq!(end.status, Status::Ok, "{:?}", end.fault);
    // The same waits on the real clock: 1 s, not 2 s. Each wait ends when it is due, so the readings, their times and the
    // means are those of deterministic mode, whose noise depends on how many times a quantity is read (simulation.md 3).
    assert_eq!(end.clock, 1000 * MS);
    let deterministic = run_on(&bench, TWO_DMM, Settings::default());
    assert_eq!(commands(&end), commands(&deterministic));
    assert_eq!(end.outputs.iter().take(2).map(number).collect::<Vec<_>>(), deterministic.outputs.iter().take(2).map(number).collect::<Vec<_>>());
    assert!(end.resources.iter().all(|resource| !resource.info.as_ref().unwrap().open));
}

#[test]
fn point_1_the_order_between_branches_does_not_matter() {
    let bench = two_benches();
    let canonical = run_on(&bench, TWO_DMM, Settings::default());
    for seed in 0..20 {
        let seeded = run_on(&bench, TWO_DMM, Settings { schedule_seed: Some(seed), ..Settings::default() });
        assert_eq!(format!("{:?}", seeded.outputs), format!("{:?}", canonical.outputs), "seed {seed}");
        assert_eq!(commands(&seeded), commands(&canonical), "seed {seed}");
        assert_eq!(seeded.clock, canonical.clock);
    }
}

#[test]
fn point_3_an_error_in_one_branch_does_not_stop_the_other() {
    let bench = two_benches();
    let end = run_on(&bench, TWO_DMM_TIMEOUT, Settings::default());
    // Branch A waits 500 ms, the timeout of its session, for an answer that never comes; its loop stops on the error (Q-7)
    // after one iteration, and `visa.close`, `always`, closes its session. Branch B makes its 10 readings.
    assert_eq!(end.status, Status::Error);
    assert_eq!(end.clock, 1000 * MS);
    let err = end.outputs[2].as_error().unwrap();
    assert_eq!(&*err.category, "io.timeout");
    assert!((number(&end.outputs[1]) - 12.0).abs() < 5.0 * 0.01 / 10f64.sqrt());
    assert!(end.resources.iter().all(|resource| !resource.info.as_ref().unwrap().open), "no session left open");
    assert_eq!(end.instruments[1].commands.len(), 10);
    // Branch A sent its one measure at 0 ms and got no answer.
    let a = &end.instruments[0];
    assert_eq!((a.resource.as_str(), a.commands.len()), ("SIM::DMM0", 1));
    assert_eq!((a.commands[0].at, a.commands[0].answer.as_deref()), (0, None));
}

#[test]
fn point_4_stop_ends_both_loops_through_the_diagram() {
    let bench = two_benches();
    let stop = PanelSetting { element: ElementId(0), value: Value::Bool(true), at: 450 * MS, by: SetBy::Set };
    let end = run_on(&bench, TWO_DMM, Settings { panel: vec![stop], ..Settings::default() });
    // The reads at 100, 200, 300 and 400 ms see the switch off; the one at 500 ms sees it on: five readings in each branch,
    // the clock ends at 500 ms, both sessions closed by the program, status `ok`.
    assert_eq!(end.status, Status::Ok);
    assert_eq!(end.clock, 500 * MS);
    assert_eq!(end.instruments.iter().map(|log| log.commands.len()).collect::<Vec<_>>(), [5, 5]);
    assert!(end.resources.iter().all(|resource| !resource.info.as_ref().unwrap().open && !resource.released));
    assert!(end.panel[0].applied);
}

#[test]
fn point_5_abort_by_timeout_releases_the_sessions_and_runs_nothing() {
    let bench = two_benches();
    let end = run_on(&bench, TWO_DMM, Settings { timeout: Some(350 * MS), ..Settings::default() });
    // At 350 ms both loops are in their iteration 3 (from 0), waiting until 400 ms: the waits end at once, no call is in
    // progress, so the run ends at 350 ms; no output; the runtime closes both sessions, which the program did not.
    assert_eq!(end.status, Status::Aborted);
    let abort = end.abort.as_ref().unwrap();
    assert_eq!((abort.by, abort.at), (AbortBy::Timeout, 350 * MS));
    assert_eq!(abort.loops.iter().map(|at| at.iteration).collect::<Vec<_>>(), [3, 3]);
    assert_eq!(end.clock, 350 * MS);
    assert!(end.outputs.is_empty());
    assert!(end.resources.iter().all(|resource| resource.released && !resource.info.as_ref().unwrap().open));
    assert_eq!(end.instruments.iter().map(|log| log.commands.len()).collect::<Vec<_>>(), [4, 4]);
}

#[test]
fn point_6_a_limit_of_iterations_is_a_fault_at_the_first_loop_to_reach_it() {
    let bench = two_benches();
    let end = run_on(&bench, TWO_DMM, Settings { max_iterations: Some(4), ..Settings::default() });
    // Both loops end their iteration 3 at 400 ms; in the global order `n_a` comes before `n_b` (spec 5.2), so the loop of
    // branch A is the first to start a fifth iteration, past the limit of 4.
    assert_eq!(end.status, Status::Fault);
    let fault = end.fault.as_ref().unwrap();
    assert_eq!(fault.fault.code, FaultCode::Iterations);
    assert_eq!(bench.names.resolve(fault.node.unwrap()), "n_a");
    assert_eq!(end.clock, 400 * MS);
    assert!(end.outputs.is_empty());
    assert!(end.resources.iter().all(|resource| resource.released && !resource.info.as_ref().unwrap().open));
}
