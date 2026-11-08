# M0 acceptance scenarios

*Last updated: 2026-11-08. Status: the scenarios of M0a pass, but for the long fuzzing run of
scenario 4; those of M0b and M0v are a first draft.*

M0, the first milestone in Rust, comes in three parts ([roadmap](roadmap.md)): M0a reads, checks
and writes programs; M0b runs them and opens the agent loop; M0a and M0b are a thin vertical
slice. M0v, a read-only viewer of the diagram, comes after M0a, in parallel with M0b. Each part
is done when its scenarios pass. Each scenario gives its setup, what is expected, how it is
verified, what the Python proof of concept already does, and what must be written first.

| Milestone | Scenarios |
|---|---|
| M0a | 2 (type error), 5 (round trip); 4 for `.lcf` and `.lct` files, through `fmt` and `check` |
| M0b | 1 (agent), 3 (parallel, clean stop); 4 for runs, instruments, arguments and MCP; the points of 2, 5 and 6 marked *with M0b* |
| M0v | 6 (viewer); 4 for `.lcf` and `.lct` files, through the viewer |

Values marked *proposed* are starting points, to adjust when the scenarios are automated.

## Common conditions

- **Through the tool, as a user would.** Every scenario goes through `labcraft-cli` (`fmt`,
  `check`, `run`, `diagram`) or its MCP server (`labcraft-cli mcp`); scenario 6 also goes through
  the desktop and web viewers. The CLI with `--json` and the MCP server return the same reports;
  [cli.md](cli.md) gives their schema for `fmt` and `check`; the rest belongs to the specifications
  of M0b and M0v.
- **Status and exit code.** Every run reports one of the statuses of spec 8.2: `ok`, `error`,
  `aborted`, `fault`. Exit codes of the CLI ([cli.md](cli.md)): 0 `ok`; 1 `error`, or an error (not
  a warning) found by `fmt`, `check` or `diagram`; 2 wrong usage (unknown option, malformed
  `--arg`); 3 `aborted`; 4 `fault`. `fmt` exits 1 on a file it cannot read or parse, and leaves
  it unchanged. `diagram` writes the SVG of every drawable function (6.3), type errors marked,
  and exits 1 only for the functions it cannot draw.
- **A point that needs a later milestone** is marked *with M0b* and checked when M0b closes, or
  when its own milestone closes if M0b is already done.
- **Deterministic mode by default** (D-12, spec 8): fixed order among ready nodes, a discrete-event
  simulated clock, one random sequence per simulated quantity. Scenario 3 also runs in real time.
- **Never crash.** In no scenario does the tool panic, abort or get killed: every failure is a
  report with a code and a location. A viewer shows the report and stays responsive.
- **Executable reference.** `examples/` and the proof of concept (`tools/lcf-poc`) give the
  expected results. Where M0 and the proof of concept differ, the spec decides, and the difference
  is recorded.
- **Automated** in `tests/acceptance/` and run by the `checks` workflow, except fuzzing (scenario 4),
  run by the `fuzz` workflow, and the parts played by an agent (scenario 1, second half of scenario 2),
  which are run by hand before the tag of their milestone. The
  agent is Claude Code with the most capable model available at that time; the model and its
  version are recorded with the transcripts, which are attached to the pull request that closes the
  milestone. Another MCP client may run the same scenarios.
- **WASM** (D-22). The parser, `check`, `fmt` and `diagram`, called as library functions, also pass
  the corpora of scenarios 2 and 5, the snapshots of scenario 6, and the families Truncation,
  Encoding, Nesting, Absurd values and Semantics of scenario 4 in a `wasm32-unknown-unknown` build,
  run by `wasm-bindgen-test` *(proposed)*. The CLI, the file system and instruments are not part of
  this.

## Scenario 1: an agent writes the PASS/FAIL program

**Setup.** An empty library folder outside the repository, the `visa` and `scpi` libraries, the
simulated multimeters `dmm-230V.toml` and `dmm-250V.toml`, and an MCP agent started in that folder
and connected to `labcraft-cli mcp`. The agent has the MCP tools and the skills shipped with
LabCraft (`skills/`), not the repository: `examples/` already contains the answer. The
instructions of the M0b MCP server must not point to it (those of the proof of concept do).

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

**Setup.** A corpus [`tests/acceptance/type-errors/`](../tests/acceptance/README.md): small programs
with exactly one error each, and next to each one the expected diagnostic (code, line, node, and the wires or terminals at
fault). Minimum content:

| Case | Example | At fault |
|---|---|---|
| Wrong type on an input | an `f64[V]` wired to the session input of `scpi.query_f64` | the wire and the input |
| Incompatible dimensions | `core.add(voltage, current)` | the node and its two input wires |
| Output of the wrong dimension | an output `power: f64[W]` bound to `core.mul(r, i)` (gives V, section 3.3) | the wire to the output |
| Implicit narrowing | an `f64` wired to an `i32` input (3.4) | the wire and the input |
| `unitless` with a unit of another dimension | `core.unitless(voltage, [A])` | the node and its unit argument |
| Arms that disagree | a `match` whose arms yield `f64[V]` and `string` | the `yield` of the second arm |
| Carry whose type changes | `carry n: i32 = 0` then `next n = core.add(n, 0.5)` | the `next n` line (`core.add` widens `n` to `f64`, 3.4) |
| Function reference of the wrong signature | `core.call(measure, …)` with a function that has no `err` output | the argument |
| Variant of another enumeration | a `match` on a `Verdict` with an arm `State.Init` | the pattern |

**Expected:**

1. The error is found by `check`, without running anything. *With M0b*: `run` refuses the
   program, and no command reaches the instrument.
2. **One error, one diagnostic**: the nodes downstream of the fault produce no further errors.
3. The diagnostic contains: a **stable code**; the severity; file, line and column range; the
   function; the **node** (the name of its line, 5.2, and its text); the **wires or terminals at
   fault**, when there are any (the value name, the source terminal `node.output` and the
   destination terminal `node.input`, with the names of the signatures); the expected type and the
   type found, units included; a fix when one exists (`core.with_unit`, `core.unitless`, `core.round_to_i32`…).
4. Same report from the CLI (`--json`) and, *with M0b*, from MCP. Without `--json`, a
   human-readable form that starts with `file:line:column`.

**Agent half.** For each case of the corpus, an agent given only the file and the diagnostic fixes
it at the first attempt, for all the cases but one at most *(proposed)*. By hand, through the CLI,
before the M0a tag.

**Agent half, run (2026-11-07)** with [`tests/acceptance/agent/run.sh`](../tests/acceptance/agent/run.sh):
one fresh Claude Code session per case (2.1.319, `claude-opus-5-5`), on a copy of the case outside the
repository, given the files and the human-readable diagnostic, with the file tools only: it cannot run
`check`, so its first fix is the one judged. A case is fixed when `check --deny-warnings` then reports
nothing. A first run gave 8 of 9: the agent gave `scale` the `err` input and output of the type
expected without binding the output (`LC4002`), and `LC3008` had no fix. It now has one, which says to
give the function the signature expected and to bind each output. A second run of all the cases gave
9 of 9. Each fix changes what the diagnostic points to; in `incompatible-dimensions`, where nothing
tells what the function is for, the agent multiplied, with an output in `f64[V*A]`. The transcripts of
both runs are attached to the pull request of this run.

**Proof of concept today.** Partial. Unknown names, cycles, missing inputs and a `handle` without
`_` are found by `check`, with file, line and node. Types and units are found only at run time
(faults `lcf.type_error`, `lcf.unit_mismatch`), without a column, a wire or a stable code, and
some not at all: a session input that receives an `f64[V]` passes `check`, and the carry case above
runs and returns 3.5 through an `i32` output. Static type checking is the main new piece of M0a.

**Written (2026-11-04):** the stable error codes and the diagnostic schema, in [cli.md](cli.md);
the codes of this corpus are marked `S2` there. The same schema goes in the MCP specification.

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
- The feasibility prototype (a scheduler with two parallel loops and a clean stop) answers
  points 2, 4 and 5 before M0b starts.

**For M2 (moved from M0):** two loops communicate through a queue; when the producer stops, the
consumer stops cleanly, without losing an element already queued and without waiting forever.

## Scenario 4: a hostile input never crashes the tool

**Scope.** Everything LabCraft reads: `.lcf` and `.lct` files, instrument `.toml` files, `--arg`
values, MCP requests, each through the components that read it, as they arrive:

- M0a: the families Truncation, Encoding, Size, Nesting, Absurd values, Semantics and Library,
  through `fmt` and `check`;
- M0b: all the families, through `run` and the MCP server;
- M0v: the same seven families as M0a, through `labcraft-cli diagram` and the viewers.

**Corpus** [`tests/acceptance/hostile/`](../tests/acceptance/README.md), with what cannot be stored
generated by the test harness:

| Family | Cases |
|---|---|
| Truncation | every prefix, byte by byte, of every file of `examples/` |
| Encoding | empty file, binary, invalid UTF-8, BOM, CRLF, lone CR, NUL, control characters, bidirectional overrides in names, strings and comments, an invisible character in a name |
| Size | a 100 MB file, a file just over the size limit, a 10 MB line, a function of 300,000 nodes (under the size limit, written without indentation), a library of 100,000 files, files built to cost the most under the size limit |
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
3. Bounded resources *(proposed)*: `fmt` of any single file of the corpus, and `check` of a folder
   that holds only that file, take at most 2 s and 512 MB on a CI runner, the library of 100,000 files at most 60 s; so do placing and
   routing any single function in `diagram`. A function of more than 20,000 nodes is not drawn:
   the viewer says why. Coordinates out of the range of spec 11 are an error, and are drawn
   at the edge of the range. Limits are part of the tool, configurable, and reported with a code
   when reached. *Proposed* defaults: file size 4 MiB, under which every file stays within these
   bounds (16 MiB took up to 6 s and 1.2 GB),
   nesting depth 256, call depth 1,024; in deterministic mode only (8.2), 10,000 iterations per
   loop and 1,000,000 nodes executed (the proof of concept has 24 calls and 100,000 nodes).
4. **Fuzzing**: `cargo fuzz` targets for the parser, `check` and the `fmt` round trip (M0a), the
   instrument files (M0b), and placement and routing in `diagram` (M0v), seeded with `examples/`
   and the corpus. Each target runs 60 s in each CI run, and 8 hours without a crash before the tag
   of the milestone that adds it *(proposed)*. Every crash found becomes a corpus file.

**Proof of concept today.** Binary, empty, deep and 1.1 MB files, a truncated file, iteration,
node and recursion limits (tests `test_binary_and_empty_and_deep`, `test_truncated`,
`test_endless_*`). No fuzzing, no time or memory bound.

## Scenario 5: saving again changes nothing

Milestone M0a. Here "save" means reading a file into the model (graph, layout, icon, comments) and
writing it back: `labcraft-cli fmt` and every command that writes a file.

**Expected:**

1. **Canonical file, identical file.** For every canonical file (`examples/`, `docs/rfc/`, the code
   blocks of the spec that are whole files, the files of the corpora that are canonical), writing
   what was read gives the same bytes. Comments, documentation comments, `panel`, `layout` and
   `icon` included (`examples/projectile.lcf` has a `panel` and a `layout`). A block of the spec that
   is only a section is not a file: its nodes exist in no function.
2. **Idempotence.** For every file that parses, `fmt(fmt(f)) = fmt(f)`. A body that cannot be
   ordered (a cycle, a value defined twice) keeps its written order and only its layout is
   normalized (5.2). A file whose canonical form is over the size limit gets `LC6003` and stays
   unchanged ([cli.md](cli.md), section 2).
3. **Meaning preserved.** `fmt` changes neither the graph (nodes, wires, types) nor the results:
   the model read before and after is the same, and, *with M0b*, the examples give the same run
   results.
4. **Line endings.** A file with CRLF endings is written with LF, ending with a single line feed
   (section 2).

**Proof of concept today.** `fmt --check` checks the order of lines and the layout of the text
(5.2), lexically, on `examples/`, `docs/rfc/` and the complete blocks of the spec (the order of
lines only in the files), but the proof of concept does not rewrite files: points 1 to 4 need the
model and the writer of M0a.

**Written:** the layout of the text and the order of the lines (2026-11-03); the canonical writing
of the `panel` and `layout` sections and the Unicode normalization of names and units, R-20
(2026-11-04), all in spec 5.2. The behavior of `fmt` is in [cli.md](cli.md).

## Scenario 6: the viewer draws the program the agent writes

Milestone M0v. Three ways in, read-only, all drawn by the same `diagram` crate following the
[visual notation](visual-notation.md): the `labcraft` desktop app opens a library folder and
watches its files; `labcraft-web` opens the bundled examples or a library folder dropped on the
page, with the libraries of `libs/` bundled so that calls resolve; `labcraft-cli diagram --svg`
writes the SVG of a function, for tests and for agents (*with M0b*, an MCP tool returns the same
SVG, D-18). The canvas paints only the shapes that `diagram` computes, so the canvas and the SVG
agree.

**Expected:**

1. **Every example is drawn.** Each function of `examples/` and `docs/rfc/` is drawn in compact
   view by default, and in standard view on request (notation, section 3), with the families of
   its wires, its terminals in signature order and its badges, which come from `check` (M0a). A
   file without a `layout` section is placed automatically; a file with one is drawn at its
   positions, the nodes it does not place are placed automatically, and its entries that name a
   node that no longer exists (after an agent renamed a value) are ignored with a warning (spec 11).
2. **The same drawing every time.** Placement and routing are deterministic: the SVG of each example
   from `labcraft-cli diagram --svg` is a snapshot checked in CI, and the `diagram` crate built for
   `wasm32-unknown-unknown` gives the same SVG as the native build. A change of the drawing is a
   reviewed change of the snapshot.
3. **Live, in the desktop app.** When an agent rewrites the file, the drawing is updated less than
   200 ms after the file is saved *(proposed)*, measured in CI by a headless harness (file change
   to new drawing); the viewport (zoom and position) is kept. The nodes that the change does not
   touch keep their place as far as the placement rules allow (a goal, to be made precise with
   them). A function is **drawable** when its file parses, its names resolve and it has no
   cycle; everything else, type errors included, is drawn with the marks of the notation. A
   function that is not drawable keeps its last drawing with its diagnostics, or shows only its
   diagnostics if it never was drawable.
4. **Large diagrams.** A generated function of 1,000 nodes and 2,000 wires is placed and routed in
   less than 1 s *(proposed)*, measured in CI, and pans and zooms at 60 frames/s on a reference
   machine, checked by hand before the M0v tag.
5. **Never a crash.** A hostile file gives a diagnostic and the viewer stays responsive
   (scenario 4).
6. **Read-only.** Files are opened read-only: a test opens every example in the viewer and checks
   that no file changed. Editing is M1.
7. **Values on the wires**, *with M0b*, in the desktop app: from the trace file that
   `labcraft-cli run` writes, each wire shows its last value, or its values per iteration in a
   loop, and a skipped node is marked (notation). In the browser, runs come with M10.

**Proof of concept today.** `tools/lcf-figcraft` draws a few functions of `examples/` and of
`docs/rfc/` in standard view, through FigCraft, guessing the types of wires from a table;
`tools/lcf-view` writes a plain graph view of `examples/` as HTML. Neither draws the compact view
(N-12) nor redraws live.

**Write first:**

- In the visual notation: the compact view (short labels and icons, N-10, N-11; primitives,
  section 3.2); the rules of automatic placement (main error chain on a straight axis, N-7) and of
  wire routing, with the stability goal of point 3; how values per iteration show on a wire.
- For the snapshots: the font and its metrics (a fixed font, attributed, named in the SVG), the
  view of `match` and `handle` (tabs or unfolded, N-2, N-9) and the theme they use.
- The architecture of the `diagram` and `editor` crates: what `diagram` computes, what the
  `editor` canvas draws (D-27).

## Not covered by these scenarios

Spec section 9 lists what M0 implements. No scenario reads and checks an `extern` function (4.1)
or uses the `string` library, `ui.set`, or the `button` element and its warning (10): their tests
are ordinary tests, not acceptance scenarios.

## Summary

| Scenario | Milestone | In CI | By hand, before the tag | Proof of concept today | Write first |
|---|---|---|---|---|---|
| 1. Agent, PASS/FAIL | M0b | Scripted MCP loop | Five agent sessions | Points 1–4 | `no-instrument.toml` |
| 2. Type error | M0a | Corpus and expected diagnostics | Agent fixes the corpus | Names only; types at run time, some not at all | |
| 3. Parallel, clean stop | M0b | Points 1–6 | | Close after an error | Q-20; two-multimeter simulation files; scheduler prototype |
| 4. Hostile inputs | M0a, M0b, M0v | Corpus, short fuzzing | Long fuzzing | Partial | Limits of M0b and M0v |
| 5. Round trip | M0a | All canonical files | | Order and layout checked, no rewriting | |
| 6. Viewer | M0v | Points 1, 2, 3, 5, 6, 7; placement time | Frame rate | `lcf-figcraft`, standard view only | Compact view, placement and routing; font, `match` view and theme of the snapshots; `diagram` and `editor` architecture |
