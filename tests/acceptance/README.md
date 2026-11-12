# Acceptance corpora

Files for the [M0 acceptance scenarios](../../docs/acceptance-m0.md), and what the tool must report on
them, in the terms of [docs/cli.md](../../docs/cli.md). `tests/harness` runs them against the Rust crates,
natively and in WASM: the families Truncation and Nesting, through `read`, the stored cases, through `format`
and `check`, and the corpus of scenario 2. `apps/labcraft-cli/tests/fmt.rs` and `check.rs` run what needs
files through the program: the stored cases, the families Size and Library, and Truncation and Nesting again.
[`hostile-later.txt`](hostile-later.txt) lists the stored cases whose codes come with a later step of
[the architecture](../../docs/architecture.md); none is left for M0a. The proof of concept has no static
type checking and no stable codes: `tools/lcf-poc/tests/test_corpus.py` checks that the corpora themselves are sound, and
that the proof of concept does not crash on them.

An `expected.json` is worked out from the documents, never taken from the output of the program: the agent
[`labcraft-oracle`](../../.claude/agents/labcraft-oracle.md) does it from the case folder alone ([AGENTS.md](../../AGENTS.md)).

## `type-errors/`: scenario 2

One folder per case, one error in each. From inside the folder, `labcraft-cli check --json .` must
give `expected.json`:

- `schema`, `status`, `summary.errors` and `summary.warnings` are compared; the other counts are not;
- each diagnostic is compared field by field, except the text of `message` and `fixes`;
- `fix_mentions` is not a field of the report: some fix of the diagnostic must mention that text, or each of a list of texts;
- `related` is compared by `file` and `span`.

Every program is in canonical form and has exactly one error, so a second diagnostic is a failure
("one error, one diagnostic"). Each code marked `S2` in `docs/cli.md` has one case. `apps/labcraft-cli/tests/check.rs`
runs them, through the program, and `tests/harness` with no command line, natively and in WASM.

[`agent/run.sh`](agent/run.sh) plays the agent half of scenario 2 on these cases, by hand: it needs Claude Code
and is not part of the `checks` workflow.

## `runs/`: the corpus of runs

The reference of `run` ([architecture](../../docs/architecture.md), section 10.11), scenarios 1, 3 and 4. One folder
per case:

- `run.json` gives the parameters of `run.start` ([commands.md](../../docs/commands.md), section 6.4), as `run.start`
  takes them: its paths (`folder`, `sim`) from the root of the project, which is the repository (decision of the owner,
  2026-11-10): `examples` for the examples, `tests/acceptance/runs/<case>` for the programs of a case, and
  `<labcraft>/visa/sim/two-dmm.toml` for a simulation file shipped with `visa`;
- `expected.json` gives the parts of `labcraft-run/1` (commands.md, section 7) that the case asserts, and `how`, which
  says how its values were found. An object compares the keys it gives, an array every element in order, so an output
  given by its name alone is not compared; numbers compare as numbers. A number of a run in real time, which depends on
  the machine, is given as a range, `{"at_least": 1000, "below": 1500}`.

**A run in real time** (`"mode": "real_time"`, decision of the owner, 2026-11-11) asserts what does not depend on the
speed of the machine: the status, the outputs, the resources, the commands and answers of each instrument in order but
not their times, and `clock.real_ms` within a range. The lower bound is a fact (a wait waits at least its time); the upper
one is given three tries, and only it: everything else must hold on each run. WASM, which has no threads, skips these
cases; the scheduler in real time is tested without the real clock in `crates/runtime` and `tests/harness`.

The values of noisy quantities are exact: [`noise.py`](runs/noise.py), written from
[simulation.md](../../libs/visa/simulation.md), sections 3 and 4, alone, gives the answers of an instrument and the means
that `two_benches` makes (`python tests/acceptance/runs/noise.py 1 230 0.05 10`).

`tests/harness` gives every `run.json` to `run.start` unchanged, on a session of `engine` whose root is the repository
held in memory: natively through the command, with the real clock for a case in real time, in WASM, which has no
threads, through `engine::run_to_end`, the same judging, run and report. It reads back the trace that the run wrote to `.labcraft/runs/`, and checks that each line reads
and that its last says the status of the report. [`tools/lcf-poc/tests/test_runs.py`](../../tools/lcf-poc/tests/test_runs.py) runs them
through the proof of concept, which has a report of its own: it compares what that report gives, and runs the cases of
[`poc-later.txt`](runs/poc-later.txt), rules the proof of concept does not follow, only to see that each gives a report.

## `hostile/`: scenario 4

One folder per case; these files are stored byte for byte (`.gitattributes`). `expected.json` gives,
for `labcraft-cli check .` and for `labcraft-cli fmt .` on a copy of the folder, the exit code and the
exact set of codes reported. `changes` says whether `fmt` rewrites a file. Whatever the file, the
process ends normally: never a panic, an abort, a stack overflow or memory exhaustion.

These families are too large or cannot be stored, so the test harness generates them:

| Family | Cases | Expected |
|---|---|---|
| Truncation | Every prefix, byte by byte, of every file of `examples/` and `libs/` | Exit 0 or 1; `fmt` leaves a file it cannot parse unchanged |
| Size | A file of 100 MB, one just over the limit, a line of 10 MB | `LC0006` when over the limit; the line reads under `--max-file-size` 16 MiB |
| Size | A function of 300,000 nodes under the limit, written without indentation, a library of 100,000 files | Ends normally; `fmt` gives `LC6003`, since its canonical form is over the limit |
| Size | Files built to cost the most just under the limit, the stored cases, the family Nesting | Within the bounds of scenario 4, measured by `cargo xtask bounds` |
| Size | A library of 100,001 files | `LC0010` |
| Size | Short lines nested deep, whose canonical form is over the limit | `LC6003` |
| Nesting | 100,000 nested `match`, `loop` or `handle`, parentheses in a unit, nested array types | `LC0007` |
| Library | A path that does not exist, an unreadable file | `LC0001` |
| Library | A folder with nothing to check | `LC0002` |
| Library | A symbolic link loop, next to a clean file | `LC0009`, exit 0 |

The families Instruments, Arguments and MCP of scenario 4 come with M0b, and the family Traces with M0v, once M0b writes traces.
