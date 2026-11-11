---
name: labcraft-testing
description: Use when testing a LabCraft program against simulated instruments with the LabCraft MCP server (run_start, simulation_describe) or labcraft-cli run - choosing the benches to run it on, writing a simulation file (.toml), giving arguments, setting panel elements during a run, timeouts, real time, and checking that the result does not depend on the order of parallel branches.
---

# Testing a LabCraft program

A program talks to instruments through `visa`, and `visa` answers the resources `SIM::…` as a **simulation
file** describes them: the program is the same on the bench and in the simulator. A run is deterministic by
default: the same program, bench and arguments give the same report, noise included. So a test is a run whose
report you read against what the bench says it should be.

## 1. The benches you have

- `project_describe` lists the simulation files of the project and those shipped with `visa`
  (`<labcraft>/visa/sim/psu-1W5.toml`), with the resources each defines.
- `simulation_describe {"path": "<labcraft>/visa/sim/psu-1W5.toml"}` says, for each instrument, its resource,
  its `*IDN?`, its latency, each query it answers and what it reads (a quantity, its value and its noise), the
  writes it takes, and the faults it injects. Read it before you write the commands of a program: an instrument
  matches a command exactly, case included.
- The shipped files: multimeters (`dmm-230V`, `dmm-250V`, `dmm-5V`, `dmm-5V-high`, `dmm-settle-300ms`,
  `dmm-settle-3s`, `dmm-timeout`, `dmm-scpi-error`, `two-dmm`, `two-dmm-timeout`), supplies (`psu-1W5`, `psu-2W5`, `psu-timeout`), a bench with a
  relay matrix (`bench`), and `no-instrument`, where every `visa.open` fails.

## 2. What to run

Run every program on more than its good case. For each bench, say first what the report must hold, from the
bench and the program, never from a first run of the program:

| Bench | The report must hold |
|---|---|
| The nominal one | `status: "ok"`, the outputs the bench gives (its values, within a few times its noise), `resources.open: []` |
| Values the program should reject or classify otherwise | The other decision, still `ok` |
| `no-instrument` | `status: "error"`, `error.category: "visa.resource_not_found"`, no decision made on default values |
| An instrument that hangs (`faults.timeout`) | `status: "error"`, `io.timeout`, and every session closed |
| Errors in the instrument's queue | What the program does with them: `scpi.check_errors` gives `scpi.instrument_error` |
| A reading that settles slowly | That the program waits, or tells it apart |

Then read in each report: `status`, `outputs`, `error`, `resources.open`, and `instruments` (each command
sent, in order, and its answer), which shows what the program really asked.

## 3. Arguments

`run_start {"function": "sweep_current", "sim": "benches/psu-load.toml", "args": {...}}`; on the command line,
`--arg name=value` or `--args FILE`. Values are JSON, read against the type of the input:

| Type | Value |
|---|---|
| A number with a unit | A string with its unit: `"230 V"`, `"0.23 kV"`, `"500 mA"` |
| `duration` | `"100 ms"`, `"2 s"` |
| A number with no unit, `bool`, `string` | As JSON: `2`, `0.5`, `true`, `"SIM::PSU0"` |
| An enumeration | `"Mode.Fast"` or `"Fast"`; with values, `{"Reading.Value": ["1.5 W"]}` |
| An array, a struct | `["1 V", "2 V", "5 V"]`; `{"low": "1 V", "high": "2 V"}` |

An input with a default can be left out. A session or a function reference cannot be an argument (`LC8107`):
run a function that opens its sessions itself.

## 4. Writing a simulation file

Write it with `file_write` anywhere in the project, as a `.toml` file, and give its path as `sim`. Each
instrument is a `[[visa.instrument]]`; a misspelt key is refused (`visa:S001`), so a run never goes on with half
a bench.

```toml
# A supply whose voltage `VOLT` sets, and whose current is about 0.25 A.
[[visa.instrument]]
resource = "SIM::PSU0"
latency_ms = 5

[visa.instrument.commands]
"*OPC?" = "1"

[visa.instrument.set]
"VOLT" = "voltage"

[visa.instrument.measure]
"MEAS:VOLT?" = "voltage"
"MEAS:CURR?" = "current"

[visa.instrument.quantity.voltage]
value = 0.0
settle_ms = 20

[visa.instrument.quantity.current]
value = 0.25
noise = 0.001
seed = 7
```

| Key | What it does |
|---|---|
| `resource` | The name a program opens, starting with `SIM::` |
| `idn`, `latency_ms` | The answer to `*IDN?`; the time each command takes on the clock of the run |
| `commands` | A query and its fixed answer |
| `measure` | A query and the quantity it reads, answered with 6 decimals |
| `set` | The header of a write and the quantity it sets: `VOLT 5` sets `voltage` to 5 |
| `accept` | Prefixes of writes that are taken and do nothing: `["ROUT:CLOS", "OUTP"]` |
| `quantity.NAME` | `value`; `initial`, `start_ms` and `settle_ms` for a value that moves; `noise` with its `seed` |
| `quantity.NAME.source` | `"SIM::PSU0.voltage"`: the true value of another quantity, as a meter wired to a source |
| `faults` | `timeout` (queries never answered), `error_on` (a command that adds an error to the queue), `error_queue` (errors there from the start) |

Two instruments wired together, and faults:

```toml
# A supply set by `VOLT`, and a multimeter wired to its output.
[[visa.instrument]]
resource = "SIM::PSU0"

[visa.instrument.set]
"VOLT" = "voltage"

[visa.instrument.quantity.voltage]
value = 0.0
settle_ms = 50

[[visa.instrument]]
resource = "SIM::DMM0"

[visa.instrument.measure]
"MEAS:VOLT:DC?" = "voltage"

[visa.instrument.quantity.voltage]
source = "SIM::PSU0.voltage"
noise = 0.001
seed = 3
```

```toml
# A supply that hangs on its current, and starts with an error in its queue.
[[visa.instrument]]
resource = "SIM::PSU0"
latency_ms = 2

[visa.instrument.measure]
"MEAS:VOLT?" = "voltage"

[visa.instrument.quantity.voltage]
value = 12.0

[visa.instrument.faults]
timeout = ["MEAS:CURR?"]
error_queue = [[-222, "Data out of range"]]
error_on = { "VOLT 99" = [-222, "Data out of range"] }
```

How a simulated instrument answers:

- A query that is in `faults.timeout`, or that it does not know, gets no answer: the read waits for the timeout of
  the session (2 s by default) and fails with `io.timeout`.
- A command it does not know adds `-113,"Undefined header"` to its error queue; that is not an error of the call.
  `SYST:ERR?` reads the queue, oldest first, and `scpi.check_errors` turns it into an error.
- `*IDN?`, `SYST:ERR?`, `*CLS` and `*RST` are answered by the simulator itself (`*RST` does nothing); a file that
  names them is refused (`visa:S011`).
- A quantity with a `noise` draws it from its own sequence, set by its `seed`: two runs read the same values, and
  so do two branches that read two instruments, in any order.

## 5. Panel elements and Stop

A program that reads a live panel element (`ui.get(stop, …)`) gets its default, or what the run sets:

- `set: [{"element": "stop", "value": true, "at": "5 s"}]` in `run_start`, or `--set stop=true@5s`: the element
  changes at that time on the clock of the run, so a deterministic run replays the press exactly.
- In real time, `run_set_element {"element": "stop", "value": true}` sets it while the run goes on.
- `set` names live elements only, not the inputs or outputs of the function (`LC8108`).
- After a planned Stop the program ends its loops and closes its sessions: expect `ok`, the outputs made before
  the stop, and `resources.open: []`.

## 6. Long runs, timeouts and limits

- `run_start` waits 30 s by default (`wait`), then answers `status: "running"` with its `run` id; `run_wait`
  waits again, `run_status` answers at once, `run_abort` stops it.
- `timeout: "10 s"` aborts the run at that time on its clock: `status: "aborted"`, no outputs, where each loop was.
  An abort runs nothing of the diagram: test a clean stop with `set`, not with a timeout.
- A deterministic run stops a `loop` after 10,000 iterations (`max_iterations`), and after 1,000,000 nodes
  (`max_nodes`): a program that never ends gives `fault` `LC8001` or `LC8002`, not a hang.

## 7. Parallel branches and real time

- Branches with no wire between them may run in any order. `schedule_seed: 1`, then `2`, `3`…, draws another
  order among the nodes ready at one time: the outputs, and the commands each instrument receives, must not
  change. If they do, the program depends on an order its wires do not give: order it by the error wire.
- `mode: "real_time"` runs on the real clock: waits and latencies take real time, and `time.now` is the real date.
  The values read are the same as in deterministic mode when the order of reads does not depend on time; the
  times are not. Do not compare real times exactly.
