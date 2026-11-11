---
name: labcraft-debugging
description: Use when a LabCraft check report has a diagnostic you do not understand, or a run ends with the status refused, error, fault or aborted, or gives outputs you did not expect. Reads labcraft-check/1, labcraft-run/1 and the trace (run_trace) of the LabCraft MCP server or labcraft-cli.
---

# Debugging a LabCraft program

Two reports say what went wrong: the report of `check` (`labcraft-check/1`), which `file_write`,
`library_check` and every run give, and the report of a run (`labcraft-run/1`), which `run_start` gives. The
trace of a run (`run_trace`) says what each node did. Read them in that order, and decide on codes and fields,
never on the text of a message.

## 1. A diagnostic of `check`

```json
{
  "code": "LC3002",
  "severity": "error",
  "message": "`f64[V]` and `f64[A]` have different dimensions",
  "file": "read_power.lcf",
  "span": { "start": { "line": 18, "column": 31 }, "end": { "line": 18, "column": 38 } },
  "function": "read_power",
  "node": { "name": "power", "text": "power = core.add(voltage, current)" },
  "wires": [
    { "value": "voltage", "from": "voltage.result", "to": "power.a" },
    { "value": "current", "from": "current.result", "to": "power.b" }
  ],
  "expected": "f64[V]",
  "found": "f64[A]"
}
```

- **`node`** is the line at fault, by its name and its text. **`wires`** are the wires at fault: `value` is the
  name of the wire, `from` and `to` its terminals. **`expected`** and **`found`** are types, written as the
  language writes them.
- **A terminal** is `node.port`: `power.a` is the input `a` of the node `power`, `current.result` the output
  `result` of the node `current`. `fn.voltage` is an input or an output of the function itself; `next.n` the carry `n`;
  `until`, `collect.0`, `yield.1` the parts of a loop or an arm; `limit.value` a named constant; `@12.err` a
  node with no name on line 12; `panel.gain` a panel element.
- **`fixes`** give a title and, when the fix is mechanical, the exact `edits` (a span and its new text): apply
  them as they are. A fix that adds `core.with_unit` asserts something only you know: judge it.
- **`related`** is another place: where a name is already defined, the other arm of a conflict.
- **One error, one diagnostic.** A value whose type an error made unknown matches everything, so the
  diagnostics that follow are not echoes: fix the first one of a chain, and check again.
- **Syntax first.** A file that does not parse (`LC0…`, `LC1…`) is not checked further, and what it declares is
  unknown, which can quiet a diagnostic in another file. Fix those, then read the report again.
- **Warnings matter.** `LC4101` (an `err` wired nowhere), `LC4102` (two calls on one session with no order) and
  `LC4105` (two reads of one `button`) point to programs that run, and run wrong.

| Codes | Family | First thing to look at |
|---|---|---|
| `LC00…`, `LC01…` | Reading a file: encoding, header, kind of file | The first line is `labcraft 1`; a `fn` goes in `.lcf`, a `type` in `.lct` |
| `LC10…` | Syntax | `LC1004`: a call inside an argument; `LC1001` lists what was expected |
| `LC20…` | Names | Spelling, the `use` line, the file name (`LC2023`), a value defined twice (`LC2012`) |
| `LC21…` | Calls | Targets (`LC2103`), a required input missing (`LC2101`), arguments in the wrong order (`LC2104`) |
| `LC30…` | Types and units | `expected` and `found`; a bare number beside a unit; a narrowing that needs a node |
| `LC40…`, `LC41…` | Dataflow and structures | A cycle (`LC4001`, feed back through a carry), an output never bound (`LC4002`), a carry with no `next` (`LC4003`) |
| `LC50…` | Panel, layout, icon | The kind of an element and its type (`LC5010`, `LC5011`, `LC5013`) |

## 2. The report of a run

Read `status` first; each one says where to look next.

| `status` | Means | Look at |
|---|---|---|
| `ok` | The function ended, its `err` holds no error | `outputs`; then `resources.open`, which must be `[]` |
| `error` | The function ended, its `err` holds an error | `error`: `category`, `code`, `message`, `origin` |
| `refused` | Nothing ran | `check.diagnostics`: an error of the program, or of an argument (`LC81…`) |
| `fault` | The run itself failed: a limit, or a value it cannot represent | `fault`: `code` (`LC80…`), `node`, `loops` |
| `aborted` | Stopped from outside: a `timeout`, `run_abort`, Ctrl-C | `abort`: `by`, `at_ms`, `loops`; the instruments are in an unknown state |

**`error`.** `origin` names the node where the error was made, its function and line, and `callers` the chain
up to your function. Here `sweep_current` (in the skill `labcraft-writing`) ran on a supply that never answers
`MEAS:CURR?`:

```json
{
  "category": "io.timeout",
  "code": -1073807339,
  "message": "SIM::PSU0: no answer to `MEAS:CURR?` within 2 s",
  "origin": {
    "callers": [{ "file": "sweep_current.lcf", "function": "sweep_current", "line": 19, "node": "raw" }],
    "file": "<labcraft>/scpi/query_f64.lcf",
    "function": "scpi.query_f64",
    "line": 12,
    "node": "text"
  },
  "others": []
}
```

**`refused` by an argument** (`args` of `run_start`, `--arg` of the command line):

| Code | Cause | Fix |
|---|---|---|
| `LC8101` | No function of that name in the folder | The name as `project_describe` lists it, `driver.measure` for a sub-folder |
| `LC8102` | An argument names no input | The names of the signature |
| `LC8103` | A required input has no argument | Give it, or give the input a default |
| `LC8104` | A value does not read as its type | `"Mode.Fast"` or `"Fast"` for an enumeration, a JSON array for an array |
| `LC8105` | A number with no unit, for a type with one | `"230 V"`, `["1 V", "2 V"]`, `"100 ms"`: a string with its unit |
| `LC8106` | Out of the range of its type | `300` is not a `u8` |
| `LC8107` | An input that holds a session or a function | Run a function that opens the session itself |
| `LC8108` | A `set` that names no live panel element | The elements of the `panel`, not its inputs or outputs |
| `LC8109`, `LC8110` | The simulation file is not TOML, or has a key of no library | The skill `labcraft-testing` |

**`fault`.** `LC8001`: a `loop` reached its iteration limit, 10,000 by default: its `until` never became true
(often because the nodes that move it are skipped by an error that no carry holds). `LC8002`: too many nodes.
`LC8003`: calls too deep, a function that calls itself. `LC8005`: memory. `LC8008`: an integer that overflows,
or `core.to_i32` of `nan`. `LC8004` is a bug of LabCraft (section 5).

**What the instruments saw.** `instruments` lists, per resource, each command sent, its time and its answer;
`answer: null` is a write, or a query that got no answer. This is how you see that a command was misspelt:
an instrument matches commands exactly, case included, and one it does not know is not an error of the call,
only an entry in its own error queue (`-113,"Undefined header"`), which `scpi.check_errors` reads.

**`resources.open`** lists the sessions still open at the end, with where each was opened: a `visa.close`
that never got the session (an `_` on a session output, a branch that drops it).

## 3. The trace

Every run with `trace: "file"` (the default) keeps its trace; `run_trace` reads it in pages, filtered:

- `run_trace {"node": "raw"}`: what the node `raw` did, in every iteration (`iter`, outermost loop first).
- `run_trace {"event": "skipped"}`: every node that did not run because its `err` held an error, with the
  error it passed on. The first one is where the failure started to spread.
- `run_trace {"event": "arm"}`: which arm each `match` and `handle` took.

Each event has `seq`, `t_ms` (the clock of the run), `frame` (one call of a function; `call` events say which)
and `node`. `ran` gives `out`, the value of each wire the node bound, by its name; `skipped` the defaults and
the error. Values are written as the report writes them: `"230.071217 V"`, `"Mode.Fast"`, `null` for no error.
`next` in the answer is the `from` of the next page.

## 4. Symptoms

| What you see | Usual cause | What to do |
|---|---|---|
| `visa.resource_not_found`, 2000 ms on the clock | The resource is not in the simulation file, or no `sim` was given | `simulation_describe` the file; give `sim` to `run_start` |
| `io.timeout` on a query | The instrument does not answer that exact text (`MEAS:VOLT?` is not `MEAS:VOLT:DC?`), or it is set to hang | `simulation_describe`: the queries it answers; `instruments` in the report |
| Outputs at `0 V`, `""`, the first variant, and status `error` | A failure upstream: the nodes after it skipped | `error.origin`; `run_trace {"event": "skipped"}` |
| Outputs at defaults and status `ok` | An error was dropped: `_` on an `err`, a `handle` that clears it, a merge left out | The `LC4101` warnings; the `handle` arms |
| A decision that looks right after a failure | It was computed on default values | Decide through `handle` on the error of the measurement |
| `resources.open` not empty | A session that no `visa.close` received | Pass the session through every call to the close |
| The same values whatever you change on the bench | The program reads another quantity, or a fixed answer of `commands` | `instruments`: the commands and their answers |
| `fault` `LC8001` | A `loop` whose `until` never comes | Make `until` depend on a count too; carry the error so the loop stops on it |
| Values that differ between two runs | Real time (`mode: "real_time"`), or `schedule_seed` with an order the wires do not fix | Deterministic mode replays exactly; order with the error wire |

## 5. When it is LabCraft's fault

A command error `internal`, a diagnostic `LC0099` or a fault `LC8004` is a bug of LabCraft, caught: the session
goes on. Reproduce it in the smallest program you can, with its simulation file and arguments, and give those to
the user with the message; do not work around it in silence.
