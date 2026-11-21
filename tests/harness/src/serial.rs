//! `serial` on the host it runs on (`libs/serial/README.md`, `list` and `open`; decision 35 of the owner): a host without
//! serial ports, the browser, gives `io.unsupported` to `list` and to `open`, with `--sim` too, and before the arguments of an
//! open are judged (io.md 3). Natively, `list` under `--sim` gives the serial endpoints of the file, here none, and an open
//! with a baud rate of 0 is `io.invalid_argument`. This module runs in WASM too (`cargo xtask wasm`).
#![allow(clippy::unwrap_used, clippy::panic)]

use labcraft_runtime::Value;
use labcraft_runtime::native::{CallContext, Context, LOG_BUDGET, LogBudget, Mode, NativeOutcome, Start, Step};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// The category of a call that ends at its first step, or none for one that gives outputs.
fn first(function: &str, inputs: Vec<Value>) -> Option<&'static str> {
    let serial = labcraft_libs::natives().into_iter().find(|native| native.name() == "serial").unwrap();
    let context = Context { simulation_file: true, ..Context::default() };
    let start = Start { library: 0, simulation: None, budget: LogBudget::new(LOG_BUDGET), mode: Mode::Deterministic, context: &context };
    let mut run = serial.start_run(start);
    let call = run.call(function, inputs).unwrap();
    match run.step(call, CallContext { now: 0 }) {
        Step::Done(NativeOutcome::Fail { category, code: 0, .. }) => Some(category),
        Step::Done(NativeOutcome::Outputs(_)) => None,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_host_without_serial_ports_gives_io_unsupported_first() {
    let list = first("list", vec![Value::NONE]);
    let open = first("open", vec![Value::string("COM3"), Value::U32(0), Value::U8(8), Value::NONE]);
    if cfg!(target_arch = "wasm32") {
        assert_eq!((list, open), (Some("io.unsupported"), Some("io.unsupported")));
    } else {
        assert_eq!((list, open), (None, Some("io.invalid_argument")));
    }
}
