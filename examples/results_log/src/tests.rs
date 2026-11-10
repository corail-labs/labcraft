//! `results_log` against spec A.4 and commands.md 7 (`logs`). Logical sizes are worked out by hand from cli.md 6.2: 16 bytes
//! for each value, plus the bytes of a `string`, and 1 for a `bool`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use labcraft_runtime::native::{CallContext, LOG_BUDGET, LibraryRun, LogBudget, Logs, NativeLibrary, NativeOutcome, Step};
use labcraft_runtime::{Handle, Value};

use crate::{KEPT_ENTRIES, ResultsLog};

fn fresh() -> Box<dyn LibraryRun> {
    ResultsLog.start_run(0, None, LogBudget::new(LOG_BUDGET))
}

/// The log as `(function, time, value)`, the value as `Debug` writes it: `Value` has no equality, since `NaN` has none.
fn entries(logs: &Logs) -> Vec<(String, i64, String)> {
    assert!(logs.entries.iter().all(|entry| entry.input == 0), "the input `results`");
    logs.entries.iter().map(|entry| (entry.function.clone(), entry.at, format!("{:?}", entry.value))).collect()
}

/// One call of `append_csv`, stepped at `now`: it ends at its first step.
fn append(run: &mut Box<dyn LibraryRun>, results: Value, now: i64) {
    let call = run.call("append_csv", vec![results, Value::NONE]).unwrap();
    match run.step(call, CallContext { now }) {
        Step::Done(NativeOutcome::Outputs(outputs)) => assert!(matches!(outputs.as_slice(), [Value::Error(None)]), "{outputs:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn each_call_is_kept_in_order_with_its_time() {
    let mut run = fresh();
    let first = Value::array(vec![Value::string("R1"), Value::Bool(true)]);
    let second = Value::array(vec![Value::Bool(false)]);
    append(&mut run, first.clone(), 5);
    append(&mut run, second.clone(), 7);
    let logs = run.logs();
    assert_eq!(entries(&logs), vec![("append_csv".to_owned(), 5, format!("{first:?}")), ("append_csv".to_owned(), 7, format!("{second:?}"))]);
    assert_eq!(logs.more, 0);
}

#[test]
fn it_has_only_append_csv_and_no_handle() {
    let mut run = fresh();
    assert!(run.call("open", vec![]).is_none());
    assert!(run.describe(Handle { library: 0, number: 1 }).is_none());
    assert!(ResultsLog.codes().is_empty());
    assert!(!ResultsLog.simulates());
}

#[test]
fn the_first_1000_entries_are_kept_and_the_others_counted() {
    let mut run = fresh();
    for i in 0..1002 {
        append(&mut run, Value::I64(i), i);
    }
    let logs = run.logs();
    assert_eq!(logs.entries.len(), KEPT_ENTRIES);
    assert_eq!(entries(&logs)[999], ("append_csv".to_owned(), 999, "I64(999)".to_owned()));
    assert_eq!(logs.more, 2);
}

#[test]
fn the_budget_counts_the_logical_size_and_the_log_stays_a_prefix() {
    // 16 + (LOG_BUDGET - 16) bytes: exactly the budget, so it is kept, and a `bool` after it, 17 bytes, is counted.
    let mut run = fresh();
    append(&mut run, Value::string("a".repeat(usize::try_from(LOG_BUDGET).unwrap() - 16)), 0);
    append(&mut run, Value::Bool(true), 1);
    let logs = run.logs();
    assert_eq!(logs.entries.len(), 1);
    assert_eq!(logs.more, 1);

    // One byte over: counted, and the `bool` after it too, though it would fit.
    let mut run = fresh();
    append(&mut run, Value::string("a".repeat(usize::try_from(LOG_BUDGET).unwrap() - 15)), 0);
    append(&mut run, Value::Bool(true), 1);
    let logs = run.logs();
    assert!(logs.entries.is_empty());
    assert_eq!(logs.more, 2);
}

#[test]
fn a_cancelled_call_logs_nothing() {
    let mut run = fresh();
    let call = run.call("append_csv", vec![Value::Bool(true), Value::NONE]).unwrap();
    run.cancel(call);
    let step = run.step(call, CallContext { now: 0 });
    assert!(matches!(step, Step::Done(NativeOutcome::Fail { category: "results_log.not_in_progress", .. })), "{step:?}");
    assert!(run.logs().entries.is_empty());
    assert_eq!(run.logs().more, 0);
}

#[test]
fn the_budget_is_the_run_s_and_follows_max_memory() {
    // 16 MiB, or `--max-memory` when lower; 1 MiB is the lowest that cli.md 6.2 accepts.
    assert_eq!(LogBudget::of_run(u64::MAX).left(), 16 * 1024 * 1024);
    assert_eq!(LogBudget::of_run(1024 * 1024).left(), 1024 * 1024);
    // Two libraries of a run share it: what one keeps, the other no longer can. By hand: a `bool` costs 17 bytes.
    let budget = LogBudget::new(40);
    let mut first = ResultsLog.start_run(0, None, budget.clone());
    let mut second = ResultsLog.start_run(1, None, budget.clone());
    append(&mut first, Value::Bool(true), 0);
    append(&mut second, Value::Bool(true), 1);
    assert_eq!(budget.left(), 6);
    append(&mut first, Value::Bool(false), 2);
    assert_eq!((first.logs().entries.len(), first.logs().more, second.logs().entries.len()), (1, 1, 1));
}
