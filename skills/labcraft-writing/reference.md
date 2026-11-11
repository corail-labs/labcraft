# The LabCraft language, v0

The reference of the skill `labcraft-writing`: what a program can say, and the rules `check` applies. Read
`SKILL.md` first; it has the loop and a complete program. Each section here stands alone.

## 1. Files, libraries, `use`

- A **library** is a folder; a **project** is the folder given to the tool, its root. A library is named by
  its path from the root (`drivers/psu` is `use drivers.psu`), and nothing can `use` the root, which has no
  name: a type shared with a sub-folder is declared in that sub-folder.
- The **shipped libraries** need no folder: `core`, `string`, `array`, `errors`, `time`, `ui` (the
  primitives), and `visa` and `scpi` (instruments). `functions_list` gives all their signatures.
- A file starts with `labcraft 1`, then its `use` lines, then a `///` comment, then its one declaration. A
  `.lct` file holds a `type` and nothing else; a `.lcf` file holds a `fn`, then an optional `panel` section.
- Names: a letter or `_`, then letters, digits or `_`. A value cannot take the name of a library its file
  imports or of a type of its library. The functions of one library share one name space with its types.

## 2. Types

| Type | Notes |
|---|---|
| `bool` | `true`, `false` |
| `i8` `i16` `i32` `i64`, `u8` `u16` `u32` `u64` | An integer literal with nothing to say otherwise is an `i32` |
| `f32`, `f64` | A decimal literal is an `f64` |
| `c64`, `c128` | Complex numbers; they have no order |
| `string`, `bytes` | UTF-8 text; binary data |
| `duration`, `timestamp` | `2 s`, `100 ms`; a `duration` goes to and from an `f64[s]` |
| `error` | `none`, or an error with a category, a code, a message, where it came from |
| `[T]` | An array: `[f64[V]]`; `[f64; 2]` is a 2-dimensional array (the number counts dimensions) |
| `struct { a: T, b: U }` | Fields, built all at once and read as `x.a` |
| `enum { A, B(T) }` | Variants, with values or without |
| `fn(T, U) -> (V, error)` | A function reference (section 4) |

- A **declared type** lives in its own `.lct` file: `type Limits = struct { low: f64[V], high: f64[V] }`.
  Two declared structs or enums are two types, even with the same fields.
- An **alias** is another name for a type: `type Volts = f64[V]`, `type Readings = [f64[V]]`.
- **Opaque types** hide what they hold: `visa.session` is an open connection, used only through the functions
  of `visa`. A driver library can name it again (`type bank = visa.session`): the two names are one type.
- **Default values**, which a skipped node gives (section 8): `false`, `0` in the unit of the type, `""`, an
  empty array, the first variant of an enumeration, each field of a struct at its default, `none`. The default
  of an opaque type is an invalid handle, on which its library fails with an error.

## 3. Units and conversions

- A number can carry a unit: `f64[V]`, `f64[mA]`, `f64[V/s]`, `f64[m/s^2]`. Without one it is dimensionless.
- Units of v0: `m` `kg` `s` `A` `K` `mol` `cd`, `g`, `Hz` `N` `Pa` `J` `W` `C` `V` `F` `Ω` `S` `Wb` `T` `H`,
  `min` `h`, `°C` `°F`, with the prefixes `p` `n` `µ` `m` `k` `M` `G` (not on `kg`, `min`, `h`, `°C`, `°F`).
  `*` (or `·`), `/` and `^` combine them, from left to right; `^` applies to the unit just before it.
- A literal is a number, a space, a unit: `230 V`, `4.7 kΩ`, `10 ms`, `9.81 m/s^2`.
- **Same dimension, another unit: converted for you** (`200 mV` where `f64[V]` is expected). **Another
  dimension: refused** (`LC3002`). Multiplying and dividing compute the unit (`V × A` is `W`, `V / A` is
  `Ω`); `core.add`, `core.sub` and the comparisons need one dimension; the result is in the unit of the first
  operand: `core.add(1 V, 200 mV)` is `1.2 V`.
- **A bare number has no dimension**: `core.ge(voltage, 0)` is refused, `core.ge(voltage, 0 V)` is not.
- `core.with_unit(x, [V])` gives a bare `f64` its unit; `core.unitless(v, [mV])` takes it off, expressed in
  the unit asked for (`core.unitless(230 mV, [V])` is `0.23`). A unit given as an argument is in brackets.
- `°C` and `°F` are scales with an offset: the difference of two `°C` is in `K`.
- **Widening is implicit** only where nothing can be lost: an integer to a larger one of the same sign, an
  unsigned one to a strictly larger signed one, an integer of up to 32 bits to `f64`, `f32` to `f64`, `c64` to
  `c128`. **Everything else is a node**: `core.to_i32(x)` (toward zero), `core.round_to_i32(x)`,
  `core.to_f32(x)`. An `i64` does not widen to `f64`.
- `core.div`, `core.sqrt` and `core.pow` of integers give an `f64`: `core.div(3, 2)` is `1.5`.
- A conversion applies to a number on a wire, never inside an array: an `[f32]` wire is not an `[f64]`.
- **A literal takes the type it goes to**: `gain: f32 = 0.5` is an `f32`, `1 V` for an `f64[mV]` is
  `1000 mV`. A decimal never becomes an integer: `n: i32 = 1.5` is refused.
- A number that overflows its type (`core.add` of two `i32` past 2^31) stops the run with a fault: count in
  an `i64` when a count can grow that far.

## 4. Functions

```text
labcraft 1

use core

/// Scales a raw reading into volts, with an offset.
fn to_volts(raw: f64, gain: f64[V] = 1 V, offset: f64[V] = 0 V) -> (volts: f64[V]) {
    product = core.mul(raw, gain)
    volts = core.add(product, offset)
}
```

- An input with no default is **required**; an input with one is optional. **A default is a constant**: a
  literal, a variant, a struct written in full, the name of a function. It reads nothing of the diagram.
- **Every output is bound once** (`LC4002` otherwise) and can be read back like a wire. `err` is both an input
  and an output: reading `err` reads the input, `err = …` binds the output.
- **A call** gives its arguments by position, then by name: `to_volts(r, offset: 5 mV)`. A positional argument
  after a named one is refused.
- **A variadic parameter** (`parts: string...`) takes as many positional arguments as given, and is an array
  in the body: `string.concat(a, "-", b)`, `errors.merge(e1, e2, e3)`. An array is not spread into one: call
  the function that takes an array (`string.join`).
- **Function references**: a parameter can take a function, `measure: fn(visa.session, err: error) ->
  (f64[V], visa.session, error)`, and the body runs it with `core.call(measure, s, err: e)`. Give it by
  name: `retry(measure: read_dc)`. A reference matches its type exactly, with no conversion. No closures.
- A function may call itself (with a warning, `LC2016`): the run stops at the limit of call depth.

## 5. Statements

One statement per line, each one node at most.

| Statement | Example | Notes |
|---|---|---|
| Binding | `raw, s2, e2 = scpi.query_f64(s, "MEAS:CURR?", err: e1)` | Every output, in the order of the signature; `_` for one left out |
| A new name | `err = e4` | No node; how an output is bound to a wire |
| A named constant | `limit = 2.5 A` | A small node with one output; it takes the type of each place it goes |
| Building a struct | `p = Pair { low: a, high: b }` | Every field, once |
| Building a variant | `r = Reading.Value(x)` | One argument per value; `Reading.Missing` alone is a value, no node |
| A label | `#close: e9 = visa.close(s, err: e8)` | Names the node, for `after:` |
| A call alone | `#beep: sound.beep()` | For a function with no outputs; one with an `err` output gets `LC4101` |

- **No nesting**: an argument is a name, a field (`p.low`), a variant (`Mode.Fast`), a literal, or a unit in
  brackets, never a call (`LC1004`).
- **A value is defined once** and never changes (`LC2012` for a second definition). To change it, make a new
  value: `next` in a loop is the one way to feed a value back (section 7).
- **The order of the lines has no meaning.** A node runs when its inputs are available. `file_format` writes
  the canonical order: a line comes after the lines it reads, and among those that are ready, by name, then the
  `next`, `collect` and `until` lines, then the lines that bind an output.
- **`after:` orders two nodes with no data between them**: `t0 = time.now(after: #close)`,
  `after: (e2, #beep)`. When the node has an `err` input, prefer the error wire: it orders and carries the
  error.

### Text

```text
line = string.format("U = {u:.3 mV}, I = {i:.2}", u: voltage, i: current)
```

- The template is a literal. Each `{name}` is an input of the node, given by name; each named argument has
  its field. `{{` and `}}` write braces.
- The format: `[fill align][+][#][0][width][.decimals][letter][ unit]`, as in C++20 and Rust: `{x:.3}` has
  three decimals, `{n:02}` pads with zeros, `{r:#06x}` writes `0x00ff`, `{u:.3 mV}` converts to millivolts
  first. A value with a unit is written with it (`1.234 V`); to send a bare number to an instrument, take the
  unit off first with `core.unitless`.
- `string.concat(a, b, …)` joins texts, `string.join(parts)` an array of them, `string.split(text, ",")`
  cuts one, `string.to_f64` and `string.to_i32` read a number and fail with `string.parse`.

## 6. Order, sessions, side effects

- **Order comes from data**, the error wire and `after:`. There is no sequence structure.
- **A session passes through** the calls of a driver: each takes the session and gives it back, so wiring the
  output to the next call orders them on the instrument.
- **To order actions on two instruments**, which share no data ("set the source, *then* measure"), chain
  their error wires: `e2 = psu.set_voltage(…, err: e1)`, then `scpi.query_f64(dmm, …, err: e2)`.
- **Two calls on one session with no order between them** get `LC4102`: one branch could run before the
  other. Order them by the error wire, or by `after:`.
- **Independent branches may run in parallel.** A deterministic run fixes one order (section 10); a program
  must not depend on an order that its wires do not say.

## 7. Structures

A structure is a node with a body. A value from outside is read inside directly; the outputs of a structure
are named on its line like those of any node.

### `match`

```text
command = match mode {
    Mode.Fast => yield "SENS:NPLC 0.1"
    Mode.Precise => yield "SENS:NPLC 10"
}
```

- The selector is a `bool`, an integer with no unit, a `string` or an enumeration.
- **Every value has an arm** (`LC3010`): every variant, `true` and `false`, and always `_` on an integer or a
  string. `_` catches the rest, wherever it is written.
- An arm can list patterns: `1, 2 => …`, ranges `3..=5` (5 included), `0..5` (5 excluded).
- **A variant with values binds them**: `Reading.Value(p) => yield p`. Such a pattern stands alone in its arm.
- **Each arm ends with one `yield`**, of the same number and types of values; the first arm decides the types.
  An arm whose body is more than its `yield` has a body in braces, with its own lines.
- A `yield` gives values, not calls: build a variant with values on a line before the `match`, and yield its
  name.

### `for`

```text
labcraft 1

use core
use scpi
use string
use visa

/// Sets the supply to each voltage in turn, and reads the current it then delivers.
fn sweep_current(
    resource: string,
    setpoints: [f64[V]],
    err: error = none,
) -> (currents: [f64[A]], err: error) {
    psu, e1 = visa.open(resource, err: err)
    last, e4, currents = for v in setpoints, carry s = psu, carry e = e1 {
        volts = core.unitless(v, [V])
        command = string.format("VOLT {volts:.3}", volts: volts)
        s2, e2 = visa.write(s, command, err: e)
        raw, s3, e3 = scpi.query_f64(s2, "MEAS:CURR?", err: e2)
        current = core.with_unit(raw, [A])
        next e = e3
        next s = s3
        collect current
    }
    #close: e5 = visa.close(last, err: e4)
    err = e5
}
```

- `for x in array` runs once per element; `for i in 0..n` from 0 to n − 1. Several `in` clauses advance
  together and stop at the shortest: `for x in values, i in 0..n` is how an index is counted.
- **`carry name = initial`** is a value fed from one iteration to the next. **`next name = …`** gives it, once
  per carry. The type of a carry is that of its line: `carry n = 0` is an `i32`, and `carry x: f64 = 0`
  says otherwise.
- **`collect value`** builds an array, one element per iteration.
- **The outputs of the loop**: the carries in their order, then the collects in theirs (`last`, `e4`,
  `currents` above). A loop that gives nothing is written `_ = for x in values { … }`.
- `next`, `collect` and `until` stand directly in the loop body, never inside an arm.
- **A session and an error are carried** through a loop like any value: the next iteration takes what the
  last one gave.
- `for parallel x in …` runs its iterations in parallel, and takes no carry.

### `loop … until`

```text
labcraft 1

use core
use errors
use time
use visa

/// Asks the instrument whether its last operation is complete, every 100 ms, until it is or `attempts` run out.
fn wait_ready(
    session: visa.session,
    attempts: i32 = 50,
    err: error = none,
) -> (session: visa.session, err: error) {
    last, e_out, ready, _ = loop carry s = session, carry e = err, carry ok = false, carry n = 0 {
        answer, s2, e2 = visa.query(s, "*OPC?", err: e)
        complete = core.eq(answer, "1")
        e3 = time.wait(100 ms, err: e2)
        tried = core.add(n, 1)
        out_of_tries = core.ge(tried, attempts)
        stop = core.or(complete, out_of_tries)
        next e = e3
        next n = tried
        next ok = complete
        next s = s2
        until stop
    }
    not_ready = core.not(ready)
    e4 = errors.raise_if(not_ready, bench.not_ready, "no `1` to `*OPC?`", err: e_out)
    session = last
    err = e4
}
```

- The body runs at least once; `until` takes a `bool`, read at the end of each iteration.
- **A `loop` also stops when a carry of type `error` holds an error**, after that iteration: otherwise the
  nodes of the body would skip and the loop would never end. `loop continue_on_error` keeps going (rare: log
  and carry on).
- There is no `break` and no early exit from a `for`: a loop that may end early is a `loop` with an index
  carry.
- A deterministic run stops a `loop` after 10,000 iterations by default (fault `LC8001`): a loop must end on
  its own.

## 8. Errors

- **A node that can fail has `err: error = none` in, and `err: error` out.** A native function whose `err`
  input holds an error **is skipped**: it does not run, its outputs take their defaults, and the error goes on
  unchanged. An output named like an input passes that input through instead (the session, so that a close
  still gets it).
- **`always`** functions run anyway (closing a session, switching a supply off), and pass on the error they
  received before their own. `functions_list` says which.
- **A function written in LabCraft always runs its body**: only its native nodes skip, each by its own `err`.
  So a `cleanup(session, err)` that calls `visa.close` closes the session even when called with an error.
- **Values after an error are defaults** (`0 V`); only `err` says whether they were measured. Decide from a
  measurement through a `handle` on its error.

```text
reading = handle e3 {
    ok => yield value
    io.timeout(err) => yield Reading.Missing
    visa.* => yield Reading.Missing
    _ => yield Reading.Missing
}
```

- `handle` chooses by the category of an error: `ok` for none, an exact category (`io.timeout`), a family
  (`visa.*`, any depth), a VISA code (`code -1073807339`), and `_`, which is required (`LC3012`). The most
  specific arm wins: code, then exact category, then the longest family, then `_`.
- `io.timeout(err) =>` binds the error to a name, to yield it on. **An error that no arm yields is cleared.**
- `errors.merge(e1, e2, …)` joins errors: the first one that holds an error leads, the others go with it.
  Two error wires meet only there.
- `errors.raise_if(condition, my.category, "message", err: e)` makes an error when `condition` is true;
  `errors.make(my.category, code: 3, message: "…")` makes one. A category is a dotted name of your choice.
- `LC4101` warns when an `err` output of a call is wired nowhere: an error would be lost. A `match`, a `handle`
  or a loop that drops an error does it on purpose, and gets no warning.
- When the top-level function ends with an error on `err`, the run's status is `error` and the report gives
  the error, its category, its message and where it came from.

Categories that the shipped libraries give: `visa.resource_not_found`, `visa.invalid_session`, `io.timeout`
(VISA codes), `scpi.instrument_error` (from `scpi.check_errors`), `string.parse`,
`array.index_out_of_range`, `array.negative_size`, `array.unequal_rows`, `core.invalid_function`.

## 9. The panel

```text
labcraft 1

use core
use scpi
use time
use ui
use visa

/// Shows the current of a supply on a chart until the operator stops it.
fn watch_current(resource: string, period: duration = 200 ms) -> (count: i32, err: error) {
    psu, e1 = visa.open(resource, err: none)
    count, last, e4 = loop carry n = 0, carry s = psu, carry e = e1 {
        raw, s2, e2 = scpi.query_f64(s, "MEAS:CURR?", err: e)
        amps = core.with_unit(raw, [A])
        e3 = time.wait(period, err: e2)
        shown = ui.set(live_current, amps)
        stop_now = ui.get(stop, after: shown)
        next e = e3
        next n = core.add(n, 1)
        next s = s2
        until stop_now
    }
    #close: e5 = visa.close(last, err: e4)
    err = e5
}

panel {
    control resource: text_box
    control period: numeric
    control stop: switch
    indicator count: numeric
    indicator live_current: chart(history: 500)
}
```

- The `panel` section lists elements and their kind: `button`, `switch`, `numeric`, `text_box`, `led`,
  `chart`. An element named like an input is a `control`, read once at the start; one named like an output is
  an `indicator`, shown when the output is produced. Others exist only live.
- **Live**: `v = ui.get(element, after: x)` reads it now, `shown = ui.set(element, value)` writes it and gives
  the value back, to order what follows. In a loop they run each iteration.
- **A `switch` keeps its value**: it is how a program plans its Stop, and how several loops stop together. A
  `button` is true until its first read only.
- **An element that a `ui.get` reads says its type**: by its kind (`switch` and `button` are `bool`,
  `text_box` is `string`), by the input or output of its name, or by a `default:`
  (`control threshold: numeric(default: 0.5 A)`).
- **Without a screen** (MCP, the command line, a test) the panel still exists: `ui.get` gives the default, or
  what `run_start` set with `set` at a time of the run (`labcraft-testing`).
- A Stop is planned in the diagram: the loops read the switch and end their iteration, then the nodes after
  them run (`visa.close`, switching a supply off). Aborting a run runs nothing of the diagram.

## 10. Running

- **Deterministic mode**, the default: a fixed order, a simulated clock that starts at
  `1970-01-01T00:00:00Z`, the same results on every run. Waits and the latencies of simulated instruments
  advance the clock; two branches that each wait 100 ms advance it by 100 ms, not 200.
- **Real time** (`mode: "real_time"`): the same program on the real clock.
- **Limits** of a deterministic run: 10,000 iterations per `loop`, 1,000,000 nodes, a call depth of 1,024, and
  memory; past one, the run ends with a fault (`LC80…`). In real time a loop runs as long as the program says.
- **Statuses**: `ok`; `error` (the `err` output holds an error); `aborted` (stopped from outside); `fault`
  (a limit, or a value the run cannot represent); `refused` (`check` found an error, or an argument did not
  convert, and nothing ran).

## 11. What v0 does not have

No nesting of calls; no sequence structure; no `break`; no generic functions or types of your own (the
primitives are generic); no `ref`, `queue`, `map` or `set` (`LC4006`); no closures; no reading the fields of an
error (`handle` tells errors apart); no spreading an array into a variadic parameter. Write the long form: one
node per line, a `loop` with an index carry, a `handle`.
