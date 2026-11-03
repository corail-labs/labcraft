# M0 acceptance scenarios

*Last updated: 2026-11-03. Status: first draft.*

M0 is the first vertical slice in Rust: text format → check → interpreter → result, with a
simulated multimeter, `labcraft-cli run` and `labcraft-cli mcp` ([roadmap](roadmap.md)). It is
done when the five scenarios below pass. Each one gives its setup, what is expected, how it is
verified, what the Python proof of concept already does, and what must be written first.

Values marked *proposed* are starting points, to adjust when the scenarios are automated.

## Common conditions

- **Through the tool, as a user would.** Every scenario goes through `labcraft-cli` (`check`,
  `run`, `fmt`) or its MCP server (`labcraft-cli mcp`). The CLI with `--json` and the MCP server
  return the same reports; their schema belongs to the CLI and MCP specification.
- **Status and exit code.** Every run reports one of the statuses of spec 8.2: `ok`, `error`,
  `aborted`, `fault`. *Proposed* exit codes of the CLI: 0 `ok`, 1 `error` (or a `check` that
  fails), 2 wrong usage (unknown option, malformed `--arg`), 3 `aborted`, 4 `fault`.
- **Deterministic mode by default** (D-12, spec 8): fixed order among ready nodes, a discrete-event
  simulated clock, one random sequence per simulated quantity. Scenario 3 also runs in real time.
- **Never crash.** In no scenario does the tool panic, abort or get killed: every failure is a
  report with a code and a location.
- **Executable reference.** `examples/` and the proof of concept (`tools/lcf-poc`) give the
  expected results. Where M0 and the proof of concept differ, the spec decides, and the difference
  is recorded.
- **Automated** in `tests/acceptance/` and run by the `checks` workflow, except the parts played by
  an agent (scenario 1, second half of scenario 2), which are run by hand before M0 is tagged. The
  agent is Claude Code with the most capable model available at that time; the model and its
  version are recorded with the transcripts, which are attached to the pull request that closes the
  milestone. Another MCP client may run the same scenarios.
- **WASM** (D-22). The parser, `check` and `fmt`, called as library functions, also pass the corpora
  of scenarios 2 and 5 and the families Truncation, Encoding, Nesting, Absurd values and Semantics
  of scenario 4 in a `wasm32-unknown-unknown` build, run by `wasm-bindgen-test` *(proposed)*. The
  CLI, the file system and instruments are not part of this.

## Scenario 1: an agent writes the PASS/FAIL program

**Setup.** An empty library folder outside the repository, the `visa` and `scpi` libraries, the
simulated multimeters `dmm-230V.toml` and `dmm-250V.toml`, and an MCP agent started in that folder
and connected to `labcraft-cli mcp`. The agent has the MCP tools and the skills shipped with
LabCraft (`skills/`), not the repository: `examples/` already contains the answer. The
instructions of the M0 MCP server must not point to it (those of the proof of concept do).

**Prompt**, given once and verbatim:

> Read the voltage of a multimeter over SCPI, check that it is between 220 and 240 V, return PASS
> or FAIL.

**Expected, all required:**

1. `check` reports no error and no warning.
2. A run against `dmm-230V` returns PASS; a run against `dmm-250V`, with the same program, returns
   FAIL. The verdict reads as PASS or FAIL without interpretation (an enumeration or a string; a
   bare `bool` does not count).
3. The program never returns PASS when the measurement did not take place: with a simulation file
   that defines no instrument (`no-instrument.toml`, *new*), the run reports the error and the
   verdict is not PASS (writing rule of 7.2).
4. No session is left open, whatever the outcome (`open_sessions` empty in all three runs).
5. No human message after the prompt; at most 20 calls to the MCP tools *(proposed)*.
6. Five independent sessions; at least four succeed *(proposed)*.

**Recorded, not required:** limits inclusive (two noise-free simulated instruments,
`dmm-220V-exact` and `dmm-240V-exact`, *proposed*), canonical form, number of tool calls, number of
`check` failures before the first clean `check`.

**Verification.** By hand for the agent part. In CI, a scripted client (like
`tools/lcf-poc/tests/mcp_smoke.py`) plays a fixed loop over real stdio MCP: write a draft with one
error, read the diagnostic, fix it, run against both multimeters and against `no-instrument.toml`.

**Proof of concept today.** Points 1 to 4 pass with `examples/check_voltage.lcf` (point 3 with a
resource name that no simulation file defines). An agent has written, checked and fixed a new
program through the MCP server (the demo), but the five-session measure has not been made.

## Scenario 2: a type error names its node and its wire

**Setup.** A corpus `tests/acceptance/type-errors/`: small programs with exactly one error each,
and next to each one the expected diagnostic (code, line, node, and the wires or terminals at
fault). Minimum content:

| Case | Example | At fault |
|---|---|---|
| Wrong type on an input | an `f64[V]` wired to the session input of `scpi.query_f64` | the wire and the input |
| Incompatible dimensions | `add(voltage, current)` | the node and its two input wires |
| Output of the wrong dimension | an output `power: f64[W]` bound to `mul(r, i)` (gives V, section 3.3) | the wire to the output |
| Implicit narrowing | an `f64` wired to an `i32` input (3.4) | the wire and the input |
| `unitless` with a unit of another dimension | `unitless(voltage, [A])` | the node and its unit argument |
| Arms that disagree | a `match` whose arms yield `f64[V]` and `string` | the `yield` of the second arm |
| Carry whose type changes | `carry n: i32 = 0` then `next n = add(n, 0.5)` | the `next n` line (`add` widens `n` to `f64`, 3.4) |
| Function reference of the wrong signature | `call(measure, …)` with a function that has no `err` output | the argument |
| Variant of another enumeration | a `match` on a `Verdict` with an arm `State.Init` | the pattern |

**Expected:**

1. The error is found by `check`, without running anything. `run` refuses the program, and no
   command reaches the instrument.
2. **One error, one diagnostic**: the nodes downstream of the fault produce no further errors.
3. The diagnostic contains: a **stable code**; the severity; file, line and column range; the
   function; the **node** (the name of its line, 5.2, and its text); the **wires or terminals at
   fault**, when there are any (the value name, the source terminal `node.output` and the
   destination terminal `node.input`, with the names of the signatures); the expected type and the
   type found, units included; a fix when one exists (`with_unit`, `unitless`, `round_to_i32`…).
4. Same report from the CLI (`--json`) and from MCP. Without `--json`, a human-readable form that
   starts with `file:line:column`.

**Agent half.** For each case of the corpus, an agent given only the file and the diagnostic fixes
it at the first attempt, for all the cases but one at most *(proposed)*. By hand, before the M0
tag.

**Proof of concept today.** Partial. Unknown names, cycles, missing inputs and a `handle` without
`_` are found by `check`, with file, line and node. Types and units are found only at run time
(faults `lcf.type_error`, `lcf.unit_mismatch`), without a column, a wire or a stable code, and
some not at all: a session input that receives an `f64[V]` passes `check`, and the carry case above
runs and returns 3.5 through an `i32` output. Static type checking is the main new piece of M0.

**Write first:** the stable error codes (not written yet), and the diagnostic schema in the CLI and
MCP specification.

## Scenario 3: two branches in parallel, a clean stop

*Reformulated on 2026-11-03.* The first wording ("two parallel loops communicate through a queue")
needs queues, which come with M2 (spec 6.4, Q-11). It becomes an M2 criterion (below); M0 keeps the
concurrency that the v0 language already has: independent branches (8, 8.1).

**Setup.** A new example, `two_benches`: two independent branches, each with its own multimeter, and
two new simulation files with two multimeters without latency, `two-dmm.toml` and
`two-dmm-timeout.toml` (in the latter, branch A's multimeter never answers). Both loops are in the
body of `two_benches` itself, not in a subfunction (Q-24). Each branch opens its session, runs a
`loop` of N = 10 measurements spaced by `time.wait(100 ms)`, then closes the session (`visa.close`,
`always`). Each loop also ends when the panel element `stop_switch`, a `switch`, is on (`ui.get`,
section 10). Outputs: the mean of each branch and the merged error (`errors.merge`).

**Expected:**

1. **The order between branches does not matter** (8, 8.1). Besides the canonical order, the
   scheduler can pick among ready nodes in an order drawn from a seed (`--schedule-seed`,
   *proposed*). Over 20 seeds the outputs are identical, and so is the sequence of commands seen
   by each instrument. Only the interleaving between the two instruments changes.
2. **The waits of the two branches overlap.** In deterministic mode, with instruments without
   latency, the simulated clock ends at 1 s, not 2 s (spec 8). In real time, two loops of
   10 × 100 ms end in less than 1.5 s, best of three runs *(proposed)*. This checks concurrent
   waiting, not threads.
3. **An error in one branch does not stop the other.** With `two-dmm-timeout.toml`, branch A's
   multimeter never answers: its loop stops
   on the error (6.3) and its session is closed. Branch B makes its N measurements and its mean is
   valid. The function's `err` holds the `io.timeout` error of A, the status is `error`, and
   `open_sessions` is empty.
4. **Stop, the stop the program plans for.** During the loops, a command sets `stop_switch`
   (section 10, syntax Q-20); in deterministic mode it is dated on the simulated clock. Both loops
   end their iteration and exit, and the `visa.close` nodes run through ordinary dataflow. The
   status is `ok`, the outputs hold the measurements made before the stop, and no session is open.
   What runs after Stop is what the diagram says, with no exception.
5. **Abort, the last resort** (8.2). Ctrl-C on the CLI, an MCP `abort` tool, or a `--timeout` on
   `run` (*proposed*). No node starts any more, `always` nodes included; waits end at once, and
   native calls in progress end or time out. Then the runtime releases the resources the run opened
   without running any node of the diagram. The status is `aborted`, with the time of the abort,
   the iteration each loop reached, no output, no open handle except those reported as leaked, and
   a warning that the instruments are in an unknown state. The run ends at most a grace period
   after the request (5 s, *proposed*); a call still running then is abandoned and its session
   reported as leaked. Everything is asserted exactly with a `--timeout` counted on the simulated
   clock; with Ctrl-C or the MCP `abort`, in real time, only the status, the absence of outputs
   and the handles are.
6. **Fault.** With an iteration limit given to the run, smaller than N, the run ends like an
   abort, with the `fault` status, a code, and the loop where the limit was reached.

**Proof of concept today.** Point 3 in part (test `test_resource_absent_close_still_runs`: `always`
close after an error). The proof of concept runs on a single thread and adds up the waits of
parallel branches instead of overlapping them; it has no seed for the scheduler, no `ui.*`, no
abort, and does not have `two_benches`.

**Write first:**

- Done (2026-11-03): spec 8.2 (Stop, Abort, fault, limits, statuses), the `switch` element (10)
  and the discrete-event simulated clock (8).
- How the CLI and MCP set a panel element (Q-20), and `ui.get` in the proof of concept.
- The simulation files `two-dmm.toml` and `two-dmm-timeout.toml`.
- The pre-M0 feasibility prototype (a scheduler with two parallel loops and a clean stop) answers
  points 2, 4 and 5 before M0 starts.

**For M2 (moved from M0):** two loops communicate through a queue; when the producer stops, the
consumer stops cleanly, without losing an element already queued and without waiting forever.

## Scenario 4: a hostile input never crashes the tool

**Scope.** Everything LabCraft reads: `.lcf` and `.lct` files, instrument `.toml` files, `--arg`
values, MCP requests. Each one goes through `check`, `fmt`, `run` and the MCP server.

**Corpus** `tests/acceptance/hostile/`:

| Family | Cases |
|---|---|
| Truncation | every prefix, byte by byte, of every file of `examples/` |
| Encoding | empty file, binary, invalid UTF-8, BOM, CRLF, lone CR, NUL, control characters, bidirectional overrides in names, strings and comments |
| Size | a 100 MB file, a file just over the size limit, a 10 MB line, a function of 300,000 nodes (under the size limit), a library of 100,000 files |
| Nesting | 100,000 nested `match`, `loop` or `handle`; parentheses in a unit; nested array types |
| Absurd values | `1e999`, an integer beyond `i64`, `m^99999999`, `[f64; 4294967296]`, layout coordinates at 1e300, `icon 0`, `labcraft 0`, `labcraft 99999` |
| Semantics | cycle, recursion `f → f` and across files, endless loop, a type that contains itself, two files that define the same function |
| Library | missing folder, symbolic link loop, unreadable file |
| Instruments | malformed TOML, negative `settle_ms`, `nan` as a value, a resource defined twice |
| Arguments | `--arg` of the wrong type, with a malformed unit, for an unknown input, out of range |
| MCP | malformed JSON-RPC, unknown tool, parameters of the wrong type, a 100 MB request |

**Expected:**

1. The process always ends with one of the exit codes of the common conditions: never a panic, an
   abort, a stack overflow, or memory exhaustion. The MCP server answers every request, with an
   error when needed, and keeps running.
2. Every problem gives a diagnostic or a fault with a code and a location. A bad file does not hide
   the diagnostics of the other files of the library.
3. Bounded resources *(proposed)*: `check` of any single file of the corpus takes at most 2 s and
   512 MB on a CI runner, the library of 100,000 files at most 60 s. Limits are part of the tool,
   configurable, and reported with a code when reached. *Proposed* defaults: file size 16 MB,
   nesting depth 256, call depth 1,024; in deterministic mode only (8.2), 10,000 iterations per
   loop and 1,000,000 nodes executed (the proof of concept has 24 calls and 100,000 nodes).
4. **Fuzzing**: `cargo fuzz` targets for the parser, `check`, the `fmt` round trip and the
   instrument files, seeded with `examples/` and the corpus. 60 s per target in each CI run, and
   8 hours per target without a crash before the M0 tag *(proposed)*. Every crash found becomes a
   corpus file.

**Proof of concept today.** Binary, empty, deep and 1.1 MB files, a truncated file, iteration,
node and recursion limits (tests `test_binary_and_empty_and_deep`, `test_truncated`,
`test_endless_*`). No fuzzing, no time or memory bound.

## Scenario 5: saving again changes nothing

In M0, "save" means reading a file into the model (graph, layout, icon, comments) and writing it
back: `labcraft fmt` and every command that writes a file.

**Expected:**

1. **Canonical file, identical file.** For every canonical file (`examples/`, `docs/rfc/`, the
   complete code blocks of the spec, completed into files as the proof of concept does, the files
   of the corpora that are canonical), writing what was
   read gives the same bytes. Comments, documentation comments, `layout` and `icon` included.
2. **Idempotence.** For every file that passes `check`, `fmt(fmt(f)) = fmt(f)`. A body with an
   error at edit time keeps its written order (5.2).
3. **Meaning preserved.** `fmt` changes neither the graph (nodes, wires, types) nor the results:
   the model read before and after is the same, and the examples give the same run results.
4. **Line endings.** A file with CRLF endings is written with LF, ending with a single line feed
   (section 2).

**Proof of concept today.** `fmt --check` checks the order of lines and the layout of the text
(5.2), lexically, on `examples/`, `docs/rfc/` and the complete blocks of the spec (the order of
lines only in the files), but the proof of concept does not rewrite files: points 1 to 4 need the
model and the writer of M0.

**Write first:**

- Done (2026-11-03): the layout of the text, in spec 5.2 next to the order of the lines.
- Still open: the canonical writing of the `panel` and `layout` sections (order of their items,
  how coordinates are written), and the Unicode normalization of names and units (Q-23).

## Not covered by these scenarios

Spec section 9 lists what M0 implements. No scenario reads and checks an `extern` function (4.1)
or uses the `string` library, `ui.set`, or the `button` element and its warning (10): their tests
are ordinary tests, not acceptance scenarios.

## Summary

| Scenario | In CI | By hand, before the M0 tag | Proof of concept today | Write first |
|---|---|---|---|---|
| 1. Agent, PASS/FAIL | Scripted MCP loop | Five agent sessions | Points 1–4 | `no-instrument.toml` |
| 2. Type error | Corpus and expected diagnostics | Agent fixes the corpus | Names only; types at run time, some not at all | Stable error codes, diagnostic schema |
| 3. Parallel, clean stop | Points 1–6 | | Close after an error | Q-20; two-multimeter simulation files; scheduler prototype |
| 4. Hostile inputs | Corpus, short fuzzing | Long fuzzing | Partial | Limits |
| 5. Round trip | All canonical files | | Order and layout checked, no rewriting | `panel` and `layout` writing, Q-23 |
