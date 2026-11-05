# lcf-poc: Python proof of concept (throwaway)

A dataflow interpreter for the LabCraft v0 subset (`docs/language-spec-v0.md`), with errors,
structures, units, a simulated clock and simulated instruments. It demonstrates the **AI loop**:
an agent writes a `.lcf`, checks it, runs it against a simulated instrument, reads the result,
fixes it. It then serves as the **executable reference** for M0 (same examples, same results).
Throwaway code: no performance work, no Rust.

Requirements: `pip install -r tools/lcf-poc/requirements.txt` (Python 3.11 or later; `mcp` 1.x,
since 2.x renamed the server API). The parser is the one in `tools/lcf-view/lcf_view.py` (same
grammar as the spec, section 14).

## Running

A library is a folder (`examples/`), one function per `.lcf` file, one type per `.lct` file.

```sh
python tools/lcf-poc/lcf.py check examples
python tools/lcf-poc/lcf.py list examples --builtins
python tools/lcf-poc/lcf.py fmt --check --spec docs/language-spec-v0.md examples docs/rfc
python tools/lcf-poc/lcf.py run examples check_voltage --sim instruments/dmm-230V.toml \
    --arg resource=SIM::DMM0 --trace
```

- `check`: syntax, names (value, function, library, type, variant, unit), inputs and outputs of
  calls, outputs bound once, cycles, `match` exhaustiveness, warnings (error never read, `err`
  output not wired). Each diagnostic gives file, line and node. Exit code 1 if there is an error.
  `run` refuses to run until `check` is clean. `--json` prints the report as JSON.
- `fmt --check [--spec FILE] PATH...`: checks `.lcf` and `.lct` files (or folders) against the
  canonical form of spec 5.2. It reports the bodies whose lines are not in canonical order, with the
  expected order (algorithm at the top of `lcfpoc/canon.py`), and the breaks of the layout of the
  text, each as `PATH:LINE` (the path as given) with the rule, and the expected text when it is
  cheap to compute (`lcfpoc/layout.py` lists the rules: sections and blank lines, `use` lines,
  `extern`, indentation, spaces, no alignment, line width in code points and split signatures,
  structs, enums and loop or `for` clauses, comments, line endings). A file that is missing, is
  not UTF-8 or does not parse is a problem (exit code 1), never a traceback. `--spec FILE` applies
  the layout rules to the complete ```` ```text ```` blocks of the spec (not the order of lines):
  whole files as files, other blocks (statements, a signature, types, `panel`, `layout` and
  `icon` sections) completed with a header or a dummy function; blocks with `…`, the reference
  listing of 3.2 and appendix A are skipped. The PoC does not rewrite files.
- `run`: outputs, error, fault, and simulation state (simulated time, sessions still open,
  commands exchanged) as JSON. `--trace` adds each node in execution order, with the wire values
  at its inputs and outputs, the loop iterations, and `skipped: true` for a node skipped because
  of an error. Exit code 1 if an error reaches the `err` output or if there is a fault.
- `--arg name=value`: a number with a unit as text (`"230 V"`, `50ms`), an array or struct as JSON,
  an enumeration by its variant name. **An input of session type** (`visa.session`, `relays.bank`,
  `psu.session`) receives a resource name: the PoC test bench opens the session. That session is
  not counted in `open_sessions`.
- `--max-iterations`: iteration limit per loop (10,000), in addition to a limit of 100,000 nodes
  and 24 nested calls.

Results are reproducible: canonical order (by name, then line) among ready nodes, simulated
clock (`time.wait` does not sleep), instrument noise with a fixed seed.

## MCP server

```sh
claude mcp add labcraft-poc -- python <repo>/tools/lcf-poc/mcp_server.py
```

`<repo>`: absolute path of the repository (the MCP client does not necessarily start the server
from the repository).

Tools: `check(folder)`, `list_functions(folder, builtins)`, `run(folder, function, sim, args, trace,
max_iterations)`, with the same parameters and the same reports as the CLI. Relative paths start
from the client's working directory. `python tools/lcf-poc/tests/mcp_smoke.py` plays an agent's loop
(write, check, read the error, fix, run) through a real stdio MCP client.

## Simulated instruments (`instruments/*.toml`)

The format is documented at the top of `lcfpoc/sim.py`. One `[[instrument]]` table per instrument:
`SIM::…` resource name, `*IDN?` response (`idn`), simulated time per command (`latency_ms`),
writes accepted with no effect (`accept`), fixed SCPI responses (`commands`), measured quantities
(`measure`) with value, noise and seed, **settling time** (`initial` to `value` over `settle_ms`,
linear ramp, starting at `start_ms`),
settable quantities (`set`, for example `VOLT 5`), a quantity that follows another instrument's
(`source`), **injectable faults** (`faults.timeout`, `faults.error_on`, `faults.error_queue` for
`SYST:ERR?`).

| File | Scenario |
|---|---|
| `dmm-230V.toml`, `dmm-250V.toml` | multimeter at 230 V / 250 V |
| `dmm-5V.toml`, `dmm-5V-high.toml` | multimeter at 5.02 V / 5.40 V |
| `psu-1W5.toml`, `psu-2W5.toml` | power supply read back at 5.02 V and 0.30 A / 0.50 A |
| `psu-timeout.toml` | the current readback never answers (`io.timeout`) |
| `dmm-settle-300ms.toml`, `dmm-settle-3s.toml` | source that settles in 300 ms / 3 s |
| `dmm-timeout.toml` | the measurement never answers (`io.timeout`) |
| `dmm-scpi-error.toml` | non-empty SCPI error queue |
| `bench.toml` | multimeter, source, relays and product (for `station`) |

## Tests

```sh
python -m unittest discover tools/lcf-poc/tests
```

They cover PASS/FAIL, a missing resource with `always` close, `settle` at 300 ms and 3 s, hostile
files, determinism, `station`, and the language (`for`/`collect`, `match`, `handle`, units,
limits).

## Limitations

- Units are checked at run time only; no offset temperatures (`°C`, `°F`), no logarithmic
  scales, no unit variables in generic functions (Q-12).
- No static type checking: a type error is a fault reported at run time
  (`lcf.type_error`, `lcf.unit_mismatch`), with file, line and node.
- No generics, function references (`call`), `for parallel` (run sequentially), `panel` or
  `layout` (ignored). Libraries: the primitives (`lcfpoc/stdlib.py`), the sub-folders of the
  library, and `libs/` (of its manifest, only `always` is read). No `--lib-path`, and no sub-library
  other than a direct sub-folder is walked. Variants of another library (`lib.Type.Variant`), types
  that contain themselves and `string.format` units in a field that never runs are not checked.
- `fmt --check` is lexical: it does not check the width of `extern` lines, the spelling of literals,
  nor layout entries that name no node; the order of lines is not checked in the blocks of the spec;
  it only reports and never rewrites. A file larger than the library limit (1,000,000 bytes) is reported, not read.
- Single thread: "parallel branches" run one after the other in canonical order.
  The simulated clock adds up the waits of the branches (two `time.wait(1 s)` with no dependency
  end at 2 s) instead of overlapping them, as spec 8 requires.
- After a fault (for example `lcf.iteration_limit`) the run does not release the sessions it
  opened: they stay in `open_sessions` of the report, unlike spec 8.2.
- `extern` functions (spec 4.1) are read as signatures. `extern rust` runs the native body of
  `lcfpoc/stdlib.py` registered for its crate and function; anything else returns
  `extern.unavailable` on `err`.
- Names and units are flagged by `fmt --check` when they are not in their one spelling (R-20), but
  are not normalized when read: `µV` and `μV` are different to `check` and `run`.
- `core.unitless(230 mV, [V])` returns 0.23 (explicit unit, spec 3.3); `duration` is a quantity in seconds.
- `results_log.append_csv` keeps the results in memory (reported by `run`) instead of writing a file.
