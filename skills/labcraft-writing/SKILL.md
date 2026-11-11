---
name: labcraft-writing
description: Use when writing or changing a LabCraft program (a function in a .lcf file, a type in a .lct file) through the LabCraft MCP server (tools such as guide_read, functions_list, file_write, run_start) or labcraft-cli. Read it before the first file_write.
---

# Writing a LabCraft program

LabCraft is a graphical dataflow language for test benches. Its programs are text files, and the text is the
diagram that the user sees: each line is a node, each name is a wire. Write it so that the drawing reads well:
one step per line, names that say what a wire carries.

## The loop

1. `functions_list`, once, with no filter: every function you can call, written as signatures, with its
   defaults, its `err` and whether it is `always`. Do not guess a name or an argument.
2. `project_describe`: the folders and files of the project, and the simulation files with the resources
   they define (`SIM::PSU0`). `simulation_describe` says what one instrument answers.
3. Write the types first, then the functions: `file_write` with `Name.lct` or `name.lcf`. Each `file_write`
   checks the whole project and returns its report: fix every error, and every warning unless you mean it.
   Fix syntax errors (`LC0…`, `LC1…`) first: while a file does not parse, some checks of the other files wait.
4. `run_start` with `function`, `sim` (a simulation file) and `args` (`{"resource": "SIM::PSU0"}`), then read
   `status`, `outputs` and `error`. Run more than one bench: the skill `labcraft-testing` says which.
5. When a report surprises you, the skill `labcraft-debugging` reads it with you.

The whole language is in `reference.md`, next to this file: `guide_read` with `{"skill": "labcraft-writing",
"file": "reference.md"}`. Read it when this page does not answer.

Without MCP, the command line does the same: `labcraft-cli check --json FOLDER`, then
`labcraft-cli run --json --sim FILE --arg resource=SIM::PSU0 FOLDER FUNCTION`.

## Files

- **One function per `.lcf` file, one type per `.lct` file**, named exactly as what it declares, case
  included: `read_power.lcf` holds `fn read_power`, `Reading.lct` holds `type Reading`.
- **First line `labcraft 1`**, then the `use` lines, one per library, `core` included, then a `///` comment
  that says what the function does, then the declaration.
- **Everything from another library is qualified**: `core.mul`, `visa.open`, `visa.session`, after its `use`.
  The functions and types of your own library are named alone (`Reading`).
- **Never write a `layout` or an `icon` section**: the editor places the nodes, and draws an icon only when the
  user asks for one.

## A complete program

`Reading.lct`:

```text
labcraft 1

/// A power that was measured, or the sign that it could not be.
type Reading = enum { Missing, Value(f64[W]) }
```

`read_power.lcf`:

```text
labcraft 1

use core
use scpi
use visa

/// Reads the voltage and the current of a power supply, and gives the power it delivers.
fn read_power(
    resource: string,
    err: error = none,
) -> (power: f64[W], reading: Reading, err: error) {
    psu, e1 = visa.open(resource, timeout: 2 s, err: err)
    raw_v, psu2, e2 = scpi.query_f64(psu, "MEAS:VOLT?", err: e1)
    raw_i, psu3, e3 = scpi.query_f64(psu2, "MEAS:CURR?", err: e2)
    current = core.with_unit(raw_i, [A])
    #close: e4 = visa.close(psu3, err: e3)
    voltage = core.with_unit(raw_v, [V])
    power = core.mul(voltage, current)
    value = Reading.Value(power)
    reading = handle e3 {
        ok => yield value
        _ => yield Reading.Missing
    }
    err = e4
}
```

What it shows, rule by rule:

- **The signature** names every input and every output. An input with a default (`err: error = none`) is
  optional. A function that can fail takes `err: error = none` and gives `err: error`.
- **A line lists every output of its node, in the order of the signature.** `scpi.query_f64` gives
  `(value, session, err)`, so its line has three targets. `_` leaves an output unwired, but never an `err`.
- **One node per line: no nesting.** `core.mul(core.with_unit(raw_v, [V]), current)` is refused (`LC1004`):
  give the inner value a name on a line of its own. Constants go straight in the arguments (`"MEAS:VOLT?"`,
  `2 s`).
- **A name is a wire.** It is bound once and read as often as needed. The order of the lines has no meaning:
  a node runs when its inputs are there. `file_format` writes the lines in their canonical order, which is
  why `current` comes before `#close` above.
- **The session passes through.** Each call on an instrument takes the session that the call before it gave
  (`psu`, then `psu2`, then `psu3`), and **the error wire goes from each call to the next** (`e1`, `e2`,
  `e3`). The error wire is what orders the actions on a bench, and what carries a failure to the end.
- **`visa.close` is `always`**: it runs even when its `err` holds an error, and passes that error on. Close
  every session you open. `#close:` is a label: it names a node whose outputs say little.
- **Units are checked.** `scpi.query_f64` gives a bare `f64`; `core.with_unit(raw_v, [V])` says once what it
  is. Then volts times amperes is watts, and an output `power: f64[W]` agrees; adding volts to amperes is
  refused (`LC3002`). A constant carries its unit too: `220 V`, `100 ms`, `0.5 A`. A comparison with a bare
  number is refused: `core.ge(voltage, 0)` is `LC3002`, `core.ge(voltage, 0 V)` says what it means.
- **After an error, measurements are defaults.** A primitive or a native function (`visa.query`,
  `string.to_f64`) whose `err` input holds an error does not run: its outputs take default values (`0.0`, `""`,
  the first variant of an enumeration) and the error goes on. A function written in LabCraft (`scpi.query_f64`,
  yours) runs its body, where the same happens node by node: its outputs are what its body gives, defaults where
  they come from skipped nodes (`scpi.query_f64` gives `0.0` and the session it received). Nodes without an
  `err` still run on those: with no instrument, `power` is `0 W`. Only `err` says whether a value
  was measured, so **anything that decides from a measurement goes through `handle` on its error**:
  `reading` is `Reading.Missing` when the measurement did not happen, not a reading of 0 W.
- **The last error of the chain is the `err` output** (`err = e4`). A run whose top-level `err` holds an
  error ends with the status `error`.

## The language at a glance

| What | How |
|---|---|
| Base types | `bool`, `i32` (and `i8` to `u64`), `f64`, `f32`, `string`, `bytes`, `duration`, `timestamp`, `error` |
| Units | `f64[V]`, `f64[mA]`, `f64[V/s]`; `230 V`, `10 ms`, `4.7 kΩ`; `core.with_unit(x, [V])`, `core.unitless(v, [mV])` |
| Arrays | `[f64[V]]`, a constant `[1 V, 2 V]`; `array.len`, `array.get`, `array.push`, `array.sum` |
| A type of your own | `type Pair = struct { low: f64[V], high: f64[V] }`, `type Mode = enum { Fast, Precise }`, one per `.lct` |
| Building | `p = Pair { low: a, high: b }`, read back as `p.low`; `r = Reading.Value(x)`; `Mode.Fast` is a value |
| A named constant | `limit = 2.5 A`, read like any wire |
| Choosing | `match` on a `bool`, an integer, a `string` or an enumeration; every value has an arm, `_` for the rest |
| Errors | `handle e { ok => … io.timeout(err) => … _ => … }`; `errors.merge(e1, e2)`; `errors.raise_if(c, my.category)` |
| Loops | `for x in values, carry acc = 0 V { … next acc = … collect y }`; `loop carry n = 0 { … until done }` |
| Order with no data | `after: e2`, `after: #close` on any call |
| Text | `string.format("VOLT {v:.3}", v: volts)`, `string.concat(a, "-", b)` |
| A panel | `panel { control stop: switch }`, then `stop_now = ui.get(stop, after: e)` in a loop |

Each row is explained, with its rules, in `reference.md`.

## Mistakes that cost a round

| Code | What happened | Fix |
|---|---|---|
| `LC1004` | A call inside an argument | One node per line: name the inner value |
| `LC2002` | Unknown function | Read `functions_list`; a function of `core` needs its prefix: `core.add` |
| `LC2005` | A library used without `use` | Add `use string` (the fix says it) |
| `LC2103` | Not as many targets as the node gives values | List every output, `_` for one you do not use |
| `LC3002` | Two dimensions that cannot meet, or a bare number beside a unit | `0 V`, not `0`; `core.with_unit` on a raw reading |
| `LC3001`, `LC3004` | The wrong type on an input, or a conversion that could lose something | Convert on a line of its own: `core.to_i32`, `core.unitless` |
| `LC3010` | A `match` with no arm for some value | Add the arm, or `_` |
| `LC3012` | A `handle` without `_` | `_` is required: categories are open |
| `LC4002` | An output never bound | Bind it once: `err = e4` |
| `LC4101` | An `err` output wired nowhere (a warning) | Wire it to the next call or to the `err` output |
| `LC4102` | Two calls on one session with no order between them (a warning) | Chain them by the error wire |
| `LC2023` | A file whose name is not what it declares | Rename one of the two, as the fix says |

A diagnostic gives the node and the wires at fault, and often a fix with the exact edit: read `fixes`
before changing anything else.
