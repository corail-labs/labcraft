# Language and format specification — v0

*Last updated: 2026-11-03. Status: draft, nothing is frozen.*

This document describes both the language and the file format: the text syntax *is* the format
(D-08). Keywords are in English, since the public repository will be too (D-21). Open questions are
gathered at the end and numbered Q-1, Q-2…

Scope of v0: what milestone M0 needs (types, functions, nodes, errors, loops, `match` and
`handle`) and the place reserved for what follows (events, panel, timed loops, PLC mode).

## 1. Model

- A **project** contains **libraries**. A library is a folder with a `labcraft.toml` manifest (name,
  version, dependencies: the basis of the package, D-25).
- A library contains **functions** and **types**, **one per file** (D-07).
- A **function** = a signature (inputs, outputs) + a **diagram**. A **panel** can be attached to it
  (D-06).
- A diagram is a graph: **nodes** connected by **wires**. In the text, a wire is a **named value**,
  defined once and usable several times.
- **The language knows nothing about instruments or tests.** As in LabVIEW, where an instrument
  driver is a library of VIs on top of VISA, instruments, VISA, SCPI, simulation and the PASS/FAIL
  verdict are **libraries**. The language provides only what is needed to write them: opaque types,
  references, errors, units.

## 2. Files

| Element | Proposed choice |
|---|---|
| Extension | `.lcf` for a function, `.lct` for a type (Q-1) |
| Encoding | UTF-8, LF line endings; the file ends with a single line feed |
| Header | `labcraft 1`: format version; older versions are migrated, a newer version is refused with a clear message |
| Section order | header, `use`, documentation comment, `fn`, `panel` (optional), `layout` (optional), `icon` (optional) |

Skeleton of a function file:

```text
labcraft 1

use visa
use scpi

/// Documentation of the function (read by the IDE, the CLI and agents).
fn name(inputs) -> (outputs) {
    diagram
}

panel { … }      // optional: attached panel (section 10)
layout { … }     // optional: layout on screen (section 11)
icon 32 { … }    // optional: icon of the compact view (section 11.1)
```

## 3. Types

### 3.1 Base types

| Type | Notes |
|---|---|
| `bool` | |
| `i8` `i16` `i32` `i64`, `u8` `u16` `u32` `u64` | |
| `f32` `f64` | `f64` by default for decimal literals |
| `c64` `c128` | complex |
| `string` | UTF-8 |
| `bytes` | binary data (instrument responses, frames) |
| `timestamp`, `duration` | absolute timestamp; duration: a distinct type, implicitly converted to and from `f64[s]` (Q-2) |
| `error` | see section 7 |

### 3.2 Composite types

```text
[f64]                 // 1-dimensional array
[f64; 2]              // 2-dimensional array
struct { voltage: f64[V], ok: bool }           // struct (LabVIEW's cluster)
enum { Pass, Fail }                            // enumeration
enum { Value(f64[V]), Overload }               // tagged union
waveform<f64[V]>      // t0, dt, samples
queue<T>, ref<T>      // explicit references (D-04), never implicit
```

`[f64; 2]` is a 2-dimensional array: the number after `;` counts dimensions, not elements (unlike
Rust). The size of an array is a property of the value, not of its type.

A named type is defined in its own `.lct` file (`type Verdict = enum { Pass, Fail }`).

**Opaque types**: a library can declare a type whose content is hidden, handled only by its
functions. This is the case for the session `visa.session` (the equivalent of LabVIEW's *VISA
resource name*): it designates an open connection, not an instrument.

**Alias of an opaque type.** A driver library can declare its opaque type as an alias of another
library's opaque type: `type bank = visa.session` in the `relays` library. `relays.bank` and
`visa.session` are then two names of the same type: a `relays.bank` is accepted wherever a
`visa.session` is expected, and the reverse. This is how the functions of a generic library apply
to a driver's session (`scpi.query_f64(bench.relays, …)`, section 13), while the driver's own
signatures still say what the session designates.

### 3.3 Physical units (D-05)

- A number can carry a unit: `f64[V]`, `f64[mA]`, `f64[V/s]`. Without a unit it is dimensionless:
  this is the default.
- The 7 SI base dimensions, the prefixes (`n` `µ` `m` `k` `M` `G`) and the common derived units
  (`Hz`, `Ω`, `W`, `F`…) are recognized. `V/A` and `Ω` are the same dimension.
- **Same dimension, different units**: implicit conversion, shown as a coercion (`mV` to `V`). It
  is lossless in dimension; the scale factor is applied in floating point. The result of an
  operation takes the unit of its first operand: `add(1 V, 200 mV)` is `1.2 V`, `add(200 mV, 1 V)`
  is `1200 mV`.
- **Different dimensions**: error at edit time (`add(voltage, current)` is rejected).
- **Temperatures**: `°C` and `°F` are scales with an offset. A value in `°C` is an absolute
  temperature; the difference of two temperatures is in `K`. Q-3.
- Literals: `230 V`, `10 ms`, `4.7 kΩ`.
- **Compound units**: `*` (or `·`), `/` and `^`. `^` applies to the unit just before it: `m/s^2`
  is m/(s²). `*`, `·` and `/` have equal precedence and apply from left to right: `kg/m*s` is
  (kg/m)·s. Use parentheses for anything else: `kg/(m*s)`.
- `unitless(x, [V])` removes the unit by expressing `x` in the requested unit
  (`unitless(230 mV, [V])` is 0.23), `with_unit(x, [V])` adds one: both are explicit, like any
  conversion that can hide an error. The unit of `unitless` is mandatory and must have the
  dimension of `x`: if the upstream type changes from `V` to `mV`, the resulting number does not
  change silently.

**Mechanism (dimensional analysis).** A unit = a vector of exponents over the 7 base dimensions
(m, kg, s, A, K, mol, cd) + a scale factor. Ω = kg·m²·s⁻³·A⁻², A = A¹.

| Operation | Rule | Example |
|---|---|---|
| `mul` | adds the exponents | Ω × A = V; V × A = W |
| `div` | subtracts the exponents | V / A = Ω |
| `add`, `sub`, comparisons | identical exponents required | V + A rejected |
| `sqrt` | divides the exponents by 2, rejected if impossible | √(V²) = V |
| `pow` | constant integer exponent | V² |

Everything is computed at compile time; at run time units no longer exist (zero cost), only scale
conversions add a multiplication. A physical error is detected at edit time:
`power: f64[W] = mul(r, i)` is rejected, because R × I gives V.

**Limits in v0:**

- **Name of the result**: a table of named units (V, W, Ω, Hz…); otherwise, the expanded form.
  Identical dimensions with different meanings (N·m and J, Hz and 1/s) are equivalent.
- **Logarithmic scales** (dB, dBm, dBV): outside the type system; explicit conversion nodes
  (`to_dbm`, `from_dbv`).
- **Primitives**: built-in unit rules. **Generic user functions over units**: a type variable
  already covers a function that works in V as well as in A (`T: Number`, 3.5); a variable that
  stands for a unit alone, to relate the units of several inputs and outputs, is to be specified
  (Q-12).

### 3.4 Numeric conversions (D-05)

- **Implicit widening**, shown in the IDE (coercion point): `i16` to `i32`, integers of up to 32
  bits to `f64`, `f32` to `f64`.
- **Explicit narrowing** by a node: `to_i32(x)`, `round_to_i32(x)`, `to_f32(x)`.
- Cases that lose precision without changing size (`i64` and `u64` to `f64`) are explicit (Q-4).
- **Literals**: an integer literal takes the type its context requires (`i32` if none); a decimal
  literal is `f64`; a literal with a unit is an `f64` with that unit (`0 V` is `f64[V]`).

### 3.5 Generics

```text
fn mean<T: Number>(values: [T]) -> (result: T)
```

Generics replace what LabVIEW does with hand-written polymorphic VIs. A type variable binds the
whole type, unit included: for `mean` of a `[f64[V]]`, `T` is `f64[V]`, so the result is an
`f64[V]`.

- **The v0 bound is `Number`**: integers, floats and complex numbers, with any unit. A variable
  without a bound stands for any type (`select`, appendix A).
- A variable that stands for a unit alone (`f64[U]`) is open question Q-12, which also covers how
  to tell such a variable from a type variable.

## 4. Functions

```text
/// Reads a voltage and tells whether it is within the limits.
fn check_voltage(
    resource: string,
    low: f64[V] = 220 V,        // optional input: default value
    high: f64[V] = 240 V,
    err: error = none,
) -> (verdict: Verdict, voltage: f64[V], err: error) {
    …
}
```

- An input without a default value is **required**: a call that does not wire it is rejected.
- Outputs are named. An output left unwired by the caller is allowed; an unwired `err` output
  gives a warning (7.5).
- Call: `name(arguments)`, positional then named (`check_voltage(r, high: 250 V)`).
- A parameter can be **variadic** (`parts: string...`): see 5.4.
- **Scope.** `core` is always in scope, unqualified (`add`, `ge`, `select`). Functions and types
  of the same library are named without qualification (`settle`, `Verdict`). Anything from another
  library, types included, needs `use lib` in the file and the qualified name: after `use visa`,
  `visa.open` and `visa.session`. `use a.b` imports `b`, a sub-library of `a` (a library nested in
  the folder of `a`); its contents are named `a.b.name`.
- **Function references** (Q-14): a named function can be passed as an argument, with a signature
  type checked at edit time, for example
  `measure: fn(visa.session, err: error) -> (f64[V], visa.session, error)`. The callee runs it
  with `call(measure, …)`. No anonymous functions or closures in v0. This is the typed equivalent
  of LabVIEW's *Call By Reference*, and what makes it possible to write `settle` or `retry` once
  for all measurements.

### 4.1 External functions (reserved, M8)

A function can be declared without a body and implemented outside LabCraft (D-14). The syntax is
fixed from v0 so that libraries can plan for it; execution comes in M8. In v0, an external function
is read and checked as a signature; calling it returns an `error` of category
`extern.unavailable` on its `err` output.

```text
labcraft 1

/// Amplitude spectrum, computed by numpy.
extern python(module: "analysis", function: "spectrum")
fn spectrum(samples: [f64[V]], rate: f64[Hz]) -> (magnitude: [f64[V]], err: error)
```

| Interface | Arguments | Execution |
|---|---|---|
| `c` | `library`, `symbol` | C ABI, dynamic loading; in-process, or isolated in a separate process |
| `dotnet` | `assembly`, `type`, `method` | Hosted CLR (D-14) |
| `python` | `module`, `function` | The user's Python interpreter, in a separate process: LabCraft does not embed Python |

- **The declaration is the contract.** Types are converted and checked at the boundary, at run
  time. Units cannot be: the declaration asserts them, like `with_unit`.
- **An `err` output is required** (R-13): missing library, exception, timeout.
- An external function does not exist in the browser (D-22): error at edit time for the web target.

## 5. Diagram

### 5.1 Statements

A binding creates a node and names its outputs:

```text
dmm, e1 = visa.open(resource)
```

- **The targets list every output of the node, in signature order**; `_` leaves one unwired
  (`_, e2 = …`).
- **One value = one wire.** It is defined once (no reassignment) and can feed several inputs: the
  wire splits. Semantics are by value; copies are made only when necessary (D-11).
- **No nesting in v0**: `add(mul(a, b), c)` is rejected; each node is a line with named outputs.
  This is longer but unambiguous, easy to reread, and every node has a name (Q-5).
- Constants are written directly as arguments (`scpi.query(dmm, "MEAS:VOLT:DC?")`).
- **A path can be an argument**: a field access (`step.voltage` reads a field of a struct), a
  variant (`State.Init`), an error category (`io.timeout`). For example
  `relays.apply(bench.relays, step.route)`. A path is not a nesting of calls.
- **Naming a value**: `name = operand` gives a constant or an existing value a new name
  (`next v = x`, `err = e3`). It creates no node.
- **Struct construction**: `r = StepResult { name: n, value: v, passed: p }`, a node.
- **Variant construction**: `r = Reading.Value(v)`, a node, not a nested call. A variant with
  several values takes one argument per value.
- **Label**: a node can be given a label (`#close:`), useful when its outputs carry nothing of
  interest: `#close: e9 = visa.close(dmm, err: e8)`. A label is unique in its function (R-17);
  `after:` (5.3) and the `layout` section (11) designate the node by it.
- **Call statement**: a call whose outputs are all unused is written alone, with an optional
  label: `#beep: sound.beep()`. It binds no output; it is meant for a function without outputs
  (R-16). If the function has an `err` output, the warning of 7.5 applies: write
  `#close: e9 = visa.close(…)` and wire `e9`.

### 5.2 The order of lines has no meaning

This is the central rule of dataflow. A node runs when its inputs are available, wherever it is
written. Two lines with no dependency between them can run in parallel.

- A reference to a value that does not exist, or a cycle (a value can only be fed back through a
  carry, 6.2), is an error at edit time.

**Canonical form.** It is the order in which the IDE and `labcraft fmt` write the lines, and it
guarantees the text → graph → text round trip, byte for byte (D-08). It applies to each body on its
own: the function body, each arm, each loop body.

- **The inputs of a line** are the names it reads, including those read in its sub-bodies and in
  `after:`, that another line of the same body defines. A label (`#close`) counts as a name.
- **The name of a line** is its first output that is neither `_` nor `next x`, or else its label.
  A call statement without a label has the empty name.
- **Algorithm**: repeatedly emit, among the lines whose inputs are all emitted, the one that comes
  first in this order:
  1. ordinary lines, by name in Unicode order. A node that produces a function output is an
     ordinary line (`voltage = with_unit(raw, [V])` in section 12);
  2. `next` lines (whose first output that is not `_` is `next x`), by carry name;
  3. `collect` lines, in their written order (it is the order of the outputs, R-6);
  4. the `until` line;
  5. in the function body only, the lines that only bind a function output to a value
     (`err = e3`), in signature order.

  Ties keep the written order. The `yield` of an arm is last, by grammar.
- **Comments** move with their statement: a comment is attached to the statement that follows it,
  or to the one that ends on its line.
- **The examples are canonical**: those of this document and those of `examples/`, which
  `python tools/lcf-poc/lcf.py fmt --check examples` checks.

### 5.3 Order of side effects

It comes only from dependencies between nodes: data, the error wire, order wires.

- **A session is a value like any other**, of opaque type: a handle. Copying it does not copy the
  connection.
- **Driver convention: the session passes through** (input and output, like the *dup refnum* of
  LabVIEW drivers). This is the general case: wiring the session output to the next operation
  orders the two operations, like any data dependency.
- **Wiring the same session to several nodes** is also allowed: the order then comes only from the
  error wire. An unused session output is ignored with `_` (a state machine does not have to pass
  its sessions around). Two such nodes with no dependency between them give a concurrent access
  warning (8.1).
- **The error wire** links operations in the intended order. It is the way to order two actions
  on different equipment ("set the source, *then* measure"), since no data links them.
- **Misuse is flagged**: chaining the session from node to node without also chaining their error
  wires. The error of the first operation is then lost. The warning "`err` output wired nowhere"
  (7.5) detects it.
- **`after:`, the order input** (R-15). Every call has it, to impose a dependency without data:
  the node waits for a value (`after: e2`), for a labeled node (`after: #beep`), or for all of a
  list (`after: (#beep, e2)`). It is drawn as an order wire. Advice: when the node has an `err`
  input, use the error wire instead; it orders the node and carries the error.
- **There is no sequence structure**: order comes from data, the error wire or order wires. Three
  actions with no data between them:

  ```text
  fn signal() -> (t0: timestamp) {
      #beep: sound.beep()
      #flash: lamp.flash(after: #beep)
      t0 = time.now(after: #flash)
  }
  ```

### 5.4 Nodes with a variable number of inputs

Without nesting (Q-5), concatenating five strings with a two-input node would take four lines. Two
mechanisms avoid this.

**Variadic parameter.** A parameter followed by `...` receives as many positional arguments as are
wired, all of the same type. In the body of the function, it is an array.

```text
/// Sum of measurements of the same dimension.
fn total<T: Number>(values: T...) -> (sum: T) {
    sum = array.sum(values)
}
```

```text
e = errors.merge(e1, e2, e3)
label = string.concat(prefix, "-", serial, "-", suffix)
```

- One variadic parameter per function. It follows all required parameters and precedes all
  parameters with a default value (including `err`); those are wired by name only (R-11).
- Zero arguments is allowed: the array is empty.
- In the diagram, one terminal line per argument, and a handle to add one.
- An existing array is not spread into a variadic parameter in v0 (Q-17): call the function that
  takes an array (`string.join`).
- `errors.merge` (7.4) is variadic.

**Inputs deduced from a template.** `string.format` takes a constant template; each `{name}` field
of the template becomes a named input of the node.

```text
line = string.format("U = {u:.3 mV}, I = {i:.2}", u: voltage, i: current)
```

- The template is a literal, checked at edit time: each field has exactly one named argument, and
  each named argument has its field (R-12). Otherwise, an error with a suggestion.
- `{name}`: default format of the type. `{name:.3}`: three decimals. A value with a unit is
  displayed with its unit (`1.234 V`); `{name:.3 mV}` converts it first, dimension checked. `{{`
  and `}}` write braces. The rest of the mini-language is to be specified (Q-18).
- Changing the template changes the terminals of the node; the IDE keeps wires by name.
- In v0, this mechanism is reserved for standard library functions that declare it
  (`string.format`, later the inverse `string.scan`). A user function cannot declare it.

## 6. Structures

A structure is a node that contains a sub-diagram. It returns outputs like an ordinary node. Outer
values can be used directly inside (these are LabVIEW's *tunnels*).

What LabVIEW users will look for and not find:

- **There is no sequence structure**: order comes from data, the error wire or order wires
  (`after:`, 5.3).
- **There is no feedback node**: a value fed back from one iteration to the next is a carry (6.2).
- **There is no implicit iteration counter**: write it as a clause, `for x in values, i in 0..n`
  (6.2), or use a carry in a `loop`.

### 6.1 Branching: `match`

The equivalent of LabVIEW's Case structure.

```text
verdict = match in_range {
    true  => yield Verdict.Pass
    false => yield Verdict.Fail
}
```

Selector: boolean, integer (values and ranges: `1..=5` includes 5, `0..5` excludes it; bounds can
be negative, `-5..=5`), string, enumeration. A variant with data binds one name per value:
`Reading.Value(v) => …`, `Range.Between(low, high) => …`. Each branch ends with exactly one
`yield`, last, with the same number and the same types of values (R-7). `_` stands for "all the
others". An enumeration must be handled completely (R-8). Overlapping literal or range patterns are
an error (R-18).

### 6.2 For loop: `for`

```text
total, scaled = for x in readings, carry acc = 0 V {
    s = mul(x, 2.0)
    next acc = add(acc, x)
    collect s
}
```

- `for x in array`: one iteration per element (LabVIEW's auto-indexing). `for i in 0..n`: `i`
  goes from 0 to n − 1.
- **Several `in` clauses** (`for x in values, i in 0..n`), of any kind, advance together and stop
  at the shortest. This is also how an iteration counter is written. The `in` clauses come before
  the carries, and a `for` has at least one.
- `carry name = initial value`: a carry (LabVIEW's shift register). `next name = …` sets the value
  for the next iteration; the final value is an output of the loop.
- `collect value`: builds an array of the values of each iteration, as output.
- **Where they go**: `next`, `collect` and `until` appear only directly in a loop body, not in a
  function body and not inside an arm of a `match` or `handle` of the body (R-6). Each carry has
  exactly one `next`. A `loop` has exactly one `until`; a `for` has none.
- Outputs, in order: the `carry` values, then the `collect` values. Several `collect` give their
  outputs in their written order, which the canonical form keeps (5.2).
- A node output can be bound directly to a carry: `next state, next index = match …` avoids one
  `next x = …` line per carry.
- Parallel iterations: only on explicit request (`for parallel x in …`), and rejected if there is a
  `carry`.

### 6.3 While loop: `loop … until`

```text
count, e = loop carry n = 0, carry err = e0 {
    done = ge(n, 10)
    _, e5 = instrument.poll(dev, err: err)
    next err = e5
    next n = add(n, 1)
    until done
}
```

The body runs at least once; `until` is evaluated at each iteration. Same `carry`, `next` and
`collect` as `for`.

**Stop on error, by default (Q-7).** The loop also stops when a carry of type `error` holds an
error. The test is made after the complete iteration, at the same time as `until`; carries keep
their last `next`, and the error leaves through the carry. Without this rule, an error would make
the nodes of the body skip (7.2), including the one that advances the stop condition (`time.wait`),
and the loop would run forever. For a loop that must continue despite an error (rare: log and
carry on), `loop continue_on_error` disables the rule. Only `loop` is concerned: a `for` ends by
itself.

### 6.4 Planned, outside v0

| Structure | Role | Milestone |
|---|---|---|
| `events` | event structure (panel, user, timeouts) | M2 |
| queues, notifiers | communication between loops | M2 |
| `timed loop` | real-time timed loop | D-24 |
| `cyclic task` | PLC-style cyclic execution | D-26 |

## 7. Errors (D-04)

### 7.1 The `error` type

An `error` value is either `none`, or an error with:

| Field | Example |
|---|---|
| `category` | `io.timeout`, `visa.resource_not_found`, `scpi.instrument_error`, `dmm.overload` |
| `code` | original code (VISA, `SYST:ERR?`), or 0 |
| `message` | readable text |
| `origin` | node, function and chain of callers |
| `others` | merged errors (7.4) |

Categories are hierarchical. A library declares its own in its manifest.

### 7.2 Automatic propagation

- A node that can fail has an `err` input (default `none`) and an `err` output.
- **A native function** (a standard-library primitive, or an `extern` function, 4.1) whose `err`
  input holds an error is **skipped**: the node does not run, its outputs take their default value
  and the error is passed on unchanged.
- **An output that has the name of an input passes it through** when the node is skipped or fails,
  instead of taking its default value. This is what lets a session pass through (5.3): after a
  failed measurement, `visa.close` still receives the session and can close it. Drivers therefore
  name their session output like their input.
- Exception: a native function marked `always` (in the manifest of its library) runs anyway
  (closing a session, switching off a power supply) and passes on the error it received in
  preference to its own. `always` can only mark native functions.
- **A function written in LabCraft (`.lcf`), from any library, always runs its body**, like a
  LabVIEW subVI: only its native nodes are skipped, each according to its own `err` input, and its
  `always` nodes run. A cleanup function (`cleanup(dmm, err)` that calls `visa.close`) therefore
  closes the session even when called with an error. There is no `always fn`.
- **Default value of a type**: `false`; `0` in the unit of the type (`0 V`, `0 s` for a
  `duration`); `""`; an empty `bytes` or array; the first variant of an enumeration, with its data
  at their default values; for a struct, each field at its default value; `none` for an `error`.
  An opaque type declares its default value in its library (an invalid handle). The default value
  of a panel element is that of its type, unless the panel item gives `default: …` (section 10).
  What `timestamp`, function references, `ref<T>` and `queue<T>` default to is Q-21.
- **Default values flow like data.** A node without an `err` input (arithmetic, `match`) runs on
  the default values of a skipped node: after a missing resource, `voltage` is `0 V`. Only the
  `err` output says whether a value was measured. **Writing rule: a verdict always depends on
  `err`**, through a `handle` (section 12, `Verdict.Error`). Propagating the absence of a value in
  the language remains an open question (Q-16).

### 7.3 Handling by category: `handle`

```text
reading, e7 = handle e6 {
    ok              => yield (value, none)
    dmm.overload    => yield (nan V, none)            // handled: cleared
    io.timeout(err) => yield (nan V, err)             // passed on
    visa.*(err)     => yield (nan V, err)
    _(err)          => yield (nan V, err)
}
```

- One branch per category (wildcards `visa.*`), per code (`code -1073807339`, the VISA code as is,
  with its sign), plus `ok` and `_`.
- **The most specific arm wins**: `code` > exact category > `a.b.*` > `a.*` > `_`. `a.*` matches
  every category under `a`, at any depth (`visa.*` matches `visa.io.timeout`); a category without
  `.*` matches only itself (R-18).
- **`_` is required**: categories are open, so exhaustiveness can only come from it. Without an
  `ok` branch, `_` also covers the absence of an error (its bound value is then `none`).
- An error that a branch does not return is **cleared**.
- Retrying = a loop around the operation, with `until` on success or on a number of attempts.
  `settle` (section 13) shows the pattern; a generic `retry` is Q-8.

### 7.4 Several errors

Error wires are explicit, so two errors meet only in a merge node: `e = errors.merge(e1, e2)`. The
first argument, in argument order, that holds an error becomes the main error; the other errors go
into its `others`. Nothing is lost. With no error among the arguments, the result is `none`.

### 7.5 An error never handled

- **At edit time**: warning if an `err` output is wired nowhere.
- **At run time**: when the top-level function returns an error on its `err` output, the run ends
  with a failure status and the error is reported (CLI, MCP, IDE). Never a dialog box.

## 8. Execution

- A node runs when all its inputs are available.
- A function call runs when all its wired inputs are available, and its outputs are available only
  at the end of the call (like a LabVIEW subVI).
- Independent branches can run in parallel; the program must not depend on an order between them.
- **Deterministic mode** (tests, agents): fixed order, simulated clock, same results at every run
  (D-12).

### 8.1 Concurrency: values and resources

- **No races on values, by construction.** A value is produced once and never changes (5.1);
  parallel branches read it together, and a modification works on a copy (D-11).
- **Races remain possible on resources**, that is, what a value designates without containing it: a
  session (two copies of the handle designate the same connection, and the instrument has a
  state), a `ref<T>` reference, a panel element read or written live (10), a file.
- **Each call is atomic on its resource.** The runtime locks a session for the duration of a
  native call: `scpi.query` writes then reads without another branch stepping in. A race therefore
  never corrupts a message; it leaves the order between calls undetermined.
- **Warning at edit time** when two nodes receive the same resource with no dependency path between
  them (data, error wire or `after:`), for example two branches that set the same instrument:
  "concurrent access to `dmm`: order by the error wire or `after:`". A library declares in its
  manifest which of its opaque types are resources, as it marks its `always` functions there. Same
  warning for a panel element written by two unordered nodes, and for a `for parallel` body that
  uses a resource.
- **Deterministic mode does not show races**, since it fixes an order (D-12). Hence a check at edit
  time rather than at run time.
- **`ref<T>` is not read then written in two nodes**, which would open a race between the two: it
  is modified in an access structure that locks it during its body (syntax for M2, Q-19).

## 9. Minimal library for M0

| Library | Content |
|---|---|
| `core` | arithmetic, comparisons, logic, `select`, conversions, units, `call` (function references, section 4) |
| `string` | `concat` (variadic), `join`, `format` (5.4), number parsing |
| `array` | size, index, subarray, construction |
| `errors` | `merge`, error creation |
| `visa` | `open`, `close`, `write`, `read`, `query` (text) |
| `scpi` | `query_f64`, `check_errors` (`SYST:ERR?`) |
| `sim` | simulated instruments described in `instruments/` (D-17) |
| `time` | `now`, `wait`, `elapsed` |
| `config` | reading a configuration file into a declared type (Q-15) |
| `ui` | `get`, `set`: panel elements read and written live (section 10) |

The signatures of the functions this document uses are in appendix A.

## 10. Panel (`panel` section)

Widgets are detailed for M3. The model is fixed from v0, because it affects semantics: a loop that
reads a Stop button, a graph updated at each iteration.

The section lists the elements and their kind: `control resource: text_box`,
`indicator verdict: led`, `indicator voltage: numeric(format: "0.000")`. Their layout goes in the
`layout` section.

**At the call terminals.** An element that has the name of an input of the function is a control:
its value is read once, at the start of the call, like any input. An element that has the name of
an output is an indicator: it displays the output when it is produced.

**Live.** During execution, the diagram reads and writes elements through two nodes of the `ui`
library:

- `value = ui.get(element, after: x)` reads the current value;
- `shown = ui.set(element, value)` writes it and returns the value, to order what follows
  (`after: shown`).

The first parameter of `ui.get` and `ui.set` is of kind *element*: it names a panel element, even
if an input or an output of the function has the same name (R-1).

In a loop, these nodes run at each iteration: this is how a Stop button is read or a graph is fed.
An element that is neither an input nor an output exists only live. `ui.get` has no `err` input:
when order matters, it goes through `after:` (5.3). A button is latching: a press stays true until
its first read, so that it is not missed between two iterations.

```text
labcraft 1

use visa
use scpi
use time
use ui

/// Displays the voltage continuously until the operator presses Stop.
fn monitor(resource: string, period: f64[s] = 200 ms) -> (count: i32, err: error) {
    dmm, e1 = visa.open(resource, timeout: 2 s)
    count, e4 = loop carry n = 0, carry e = e1 {
        raw, _, e2 = scpi.query_f64(dmm, "MEAS:VOLT:DC?", err: e)
        e3 = time.wait(period, err: e2)
        volts = with_unit(raw, [V])
        shown = ui.set(live_voltage, volts)
        stop = ui.get(stop_button, after: shown)
        next e = e3
        next n = add(n, 1)
        until stop
    }
    #close: e5 = visa.close(dmm, err: e4)
    err = e5
}

panel {
    control resource: text_box
    control period: numeric
    control stop_button: button
    indicator live_voltage: chart(history: 500)
    indicator count: numeric
}
```

`resource` and `period` are read once; `count` is displayed at the end. `stop_button` and
`live_voltage` exist only live. To change the period along the way, it would be read with `ui.get`
in the loop.

**Without a user interface.** The panel state exists as soon as the function runs, displayed or
not: called as a subfunction, by the CLI, by MCP or by an agent. `ui.get` then returns the default
value of the element, or the one a command set during execution (an agent "presses" Stop). The
default value of an element is that of its type (7.2), unless the panel item gives one:
`control threshold: numeric(default: 0.5)`. Writes by `ui.set` are recorded in the run trace. In
deterministic mode, values set by a command are dated on the simulated clock: a test replays
exactly the press of Stop at 5 s (command syntax: Q-20).

**Concurrency.** Reading an element is always safe; writing it from two unordered nodes gives a
warning (8.1).

**Events (M2).** The `events` structure will react to changes of the same elements, without
polling in a loop.

## 11. Layout (`layout` section)

```text
layout {
    node dmm     at (40, 80) view standard
    node reading at (220, 80)
    node verdict at (400, 80)
    node #close  at (580, 80)
    wire reading -> verdict via (300, 80) (300, 120)
}
```

- **Coordinates** are in canvas units: 1 unit = 1 logical pixel at 100% zoom, origin at the top
  left, y pointing down (visual notation, section 2). `node x at (40, 80)` places the top-left
  corner of the node; the points after `via` are waypoints of the route.
- Nodes are designated by the name of their line (5.2): their first output that is neither `_` nor
  `next x`, or else their label (`#close`). A node that has no such output (all its outputs are
  `_` or `next x`, or a call statement) needs a label to be designated here.
- `wire a -> b` designates the wire from node `a` to node `b`. When it is not the only one,
  `wire a.out -> b.in` names the terminals, with the output and input names of the signatures:
  `wire raw.session -> #close.session` in `check_voltage` (section 12).
- Wires without a route are routed automatically.
- **View of a node**: compact by default, standard on request (visual notation, section 3).
  `view standard` or `view compact` sets the view of a node, alone or after its position. The IDE
  preference "everything in standard" is not in the file.
- Section missing or incomplete: the IDE places the missing nodes. An agent never writes this
  section.
- Renaming a value updates the section (IDE and CLI command).

### 11.1 Icon (`icon` section)

The icon of a function is displayed in the compact view of each of its calls. It lives in the file
of the function, after `layout`: copying, renaming or moving the function carries its icon along.
File `check_voltage.lcf`, at the end:

```text
icon 32 {
    rect (2, 4) (30, 28) radius 4
    path "M7 21 A9 9 0 0 1 25 21"
    line (16, 21) (21, 13)
    text "V" at (16, 27) size 6
}
```

- **Grid**: `icon 32` gives the size of the grid, 32 × 32 in v0, origin at the top left, y pointing
  down, like the canvas.
- **Shapes**: `rect` (two corners, optional radius), `circle` (center, radius), `line`, `path` (SVG
  path syntax: `M L H V C S Q T A Z`, absolute or relative), `text` (`at`: middle of the baseline).
  Fixed stroke width chosen by the IDE; `fill` fills the shape.
- **Color**: monochrome, the ink color of the theme and of the node state (skipped, failed).
  `accent` gives a shape an accent color, designated by a theme token (`accent wire.error`), never
  by a value.
- **No script, no link, no image**: the notation can neither run nor load anything. An icon coming
  from a third-party package can be drawn safely. The IDE imports and exports SVG (commands
  `icon.import`, `icon.export`) for drawing in a drawing program.
- **Without an `icon` section**, the compact view displays a short label derived from the name.
- **Reading by an agent**: by default the CLI and the MCP server render a function without its
  `layout` and `icon` sections, which say nothing about execution; an agent asks for them if it
  needs them. An agent writes an icon only on request.

## 12. Complete example: PASS/FAIL multimeter

`Verdict` is not a type of the language: it is defined in the project. LabCraft has no built-in
verdict or limit type; a project can define one, as here. Sequencing, step statuses and reports
stay with the test executive that calls LabCraft (D-28).

File `verdict.lct`:

```text
labcraft 1

/// Result of a check. `Error`: the measurement did not take place (upstream error).
type Verdict = enum { Pass, Fail, Error }
```

File `check_voltage.lcf`:

```text
labcraft 1

use visa
use scpi

/// Reads the DC voltage of a multimeter and tells whether it is within the limits.
fn check_voltage(
    resource: string,
    low: f64[V] = 220 V,
    high: f64[V] = 240 V,
    err: error = none,
) -> (verdict: Verdict, voltage: f64[V], err: error) {
    dmm, e1 = visa.open(resource, timeout: 2 s, err: err)
    raw, dmm2, e2 = scpi.query_f64(dmm, "MEAS:VOLT:DC?", err: e1)
    #close: e3 = visa.close(dmm2, err: e2)
    voltage = with_unit(raw, [V])
    above = ge(voltage, low)
    below = le(voltage, high)
    in_range = and(above, below)
    measured = match in_range {
        true  => yield Verdict.Pass
        false => yield Verdict.Fail
    }
    verdict = handle e2 {
        ok => yield measured
        _  => yield Verdict.Error
    }
    err = e3
}
```

`dmm` and `dmm2` are of type `visa.session`: the session passes through `query_f64`, as in a
LabVIEW driver, and the same connection is closed. `visa.close` is marked `always` in the library:
the session is closed even if the measurement failed. The verdict goes through `handle e2`: without
it, a missing resource would give `voltage = 0 V` and a verdict computed on a measurement that did
not take place (7.2). Simulation plugs in at the VISA level (resource `SIM::…`): the program is the
same with a real instrument. Running against a simulated instrument, with the proof of concept (the
future `labcraft run` will be equivalent):

```sh
python tools/lcf-poc/lcf.py run examples check_voltage --sim instruments/dmm-230V.toml \
    --arg resource=SIM::DMM0
```

## 13. Example: state machine of a test station

The most common sequencer on a test bench without a test executive: self-test, wait for the
product, run the steps until the first failure, report. Each step switches relays, sets a source,
**waits for the reading to settle**, measures and compares to the limits. The steps and their limits come from a
configuration file per product reference.

Types, in their `.lct` files:

```text
type State = enum { Init, WaitDut, RunStep, Report, Stop }

type Step = struct {
    name: string,
    route: string,          // name of a route in the relay table
    voltage: f64[V],
    low: f64[V],
    high: f64[V],
}

type StepResult = struct { name: string, value: f64[V], passed: bool }

// `relays.bank` and `psu.session` are opaque types that their libraries declare as aliases of
// `visa.session` (`type bank = visa.session` in `relays`): `scpi` functions accept them.
type Bench = struct { dmm: visa.session, relays: relays.bank, source: psu.session }
```

The state machine, `station.lcf` (`bench_selftest` and `wait_dut` are in `examples/`):

```text
labcraft 1

use array
use results_log

/// Self-test, then for each of the `dut_count` products: wait, run the steps until the first failure, report.
fn station(bench: Bench, steps: [Step], dut_count: i32 = 1, err: error = none) -> (err: error) {
    n = array.len(steps)
    _, _, _, _, err_out = loop
        carry state = State.Init,
        carry index = 0,
        carry results: [StepResult] = [],
        carry tested = 0,
        carry err = err
    {
        next state, next index, next results, next tested, next err, stop = match state {
            State.Init => {
                e1 = bench_selftest(bench, err: err)
                yield (State.WaitDut, 0, results, tested, e1, false)
            }
            State.WaitDut => {
                present, e2 = wait_dut(bench, timeout: 30 s, err: err)
                s = select(present, State.RunStep, State.WaitDut)
                yield (s, 0, [], tested, e2, false)
            }
            State.RunStep => {
                i2 = add(index, 1)
                more = lt(i2, n)
                step, e5 = array.get(steps, index, err: err)
                r, e3 = run_step(bench, step, err: e5)
                all = array.push(results, r)
                go_on = and(more, r.passed)
                s = select(go_on, State.RunStep, State.Report)
                yield (s, i2, all, tested, e3, false)
            }
            State.Report => {
                e4 = results_log.append_csv(results, err: err)
                t2 = add(tested, 1)
                again = lt(t2, dut_count)
                s = select(again, State.WaitDut, State.Stop)
                yield (s, 0, [], t2, e4, false)
            }
            State.Stop => yield (State.Stop, index, results, tested, err, true)
        }
        until stop
    }
    err = err_out
}
```

One step, `run_step.lcf`:

```text
labcraft 1

use relays
use psu

/// Switch, set, wait for the reading to settle, measure, compare to the limits.
fn run_step(bench: Bench, step: Step, err: error = none) -> (result: StepResult, err: error) {
    _, e1 = relays.apply(bench.relays, step.route, err: err)
    _, e2 = psu.set_voltage(bench.source, step.voltage, err: e1)
    v, e3 = settle(bench.dmm, target: step.voltage, tolerance: 0.2 V,
                   period: 50 ms, timeout: 1 s, err: e2)
    above = ge(v, step.low)
    below = le(v, step.high)
    passed = and(above, below)
    result = StepResult { name: step.name, value: v, passed: passed }
    err = e3
}
```

Measure until settled, `settle.lcf`:

```text
labcraft 1

use visa
use time
use errors
use dmm_driver

/// Measures every `period` until within `tolerance` of `target`, or io.timeout error.
fn settle(dmm: visa.session, target: f64[V], tolerance: f64[V], period: duration,
          timeout: duration, err: error = none) -> (value: f64[V], err: error) {
    t0 = time.now(after: err)
    value, err_out = loop
        carry v = 0 V,
        carry e = err
    {
        e1 = time.wait(period, err: e)
        t = time.elapsed(t0)
        late = gt(t, timeout)
        x, _, e2 = dmm_driver.read_dc_volts(dmm, err: e1)
        gap = sub(x, target)
        dist = abs(gap)
        stable = le(dist, tolerance)
        done = or(stable, late)
        next e = errors.raise_if(late, io.timeout, "reading not settled", err: e2)
        next v = x
        until done
    }
    err = err_out
}
```

**Lessons from this example:**

- **The "set, then measure" order goes through the error wire** (`e1` → `e2` → `e3`). Without it,
  the source and the multimeter would be started in parallel and the measurement could precede the
  voltage change: a classic defect of test benches.
- **`settle` replaces fixed waits**, which are always too short or too long.
- **Session outputs are ignored here** (`_`): the state machine keeps `bench` as is, and the order
  goes through the error wire since the actions touch different equipment. In a chain on a single
  instrument (section 12), the session passes through (section 5.3).
- `next x = …` directly on a node output, field access (`step.low`) and struct construction are
  needed (sections 5.1, 6.2).
- Trap: within an iteration, `v` always designates the *current* value of the carry. To use the
  new measurement in the same iteration, name it (`x`), then write `next v = x`. The diagram labels
  the two sides of the carry `v` and `next v` (visual notation 5.1).
- A loop that carries an error must stop on it: otherwise an error would make all the nodes skip
  and the loop would run forever. Hence the default stop (6.3, Q-7).
- The error wire on each call remains readable here (Q-9); it is what carries the order.
- **A state machine must have an exit.** Here `dut_count` products, then `State.Stop`; a stop
  requested by the operator is read with `ui.get` (section 10). `array.get` has an `err` output (index
  out of the array): it is wired to `run_step`, like any error output (7.5).
- A generic `settle` (that measures anything) takes a function reference (section 4).

## 14. Grammar (EBNF)

Notation: `{ x }` = zero or more, `[ x ]` = optional, `|` = choice, `"…"` = literal text. This
grammar describes the syntax; name resolution and types belong to checking (sections 3 to 8).

### 14.1 Lexicon

```ebnf
IDENT      = letter { letter | digit | "_" } ;          (* not a reserved word *)
letter     = a Unicode letter (a-z, A-Z, µ, Ω…) ;       (* never "_" or "°" *)
INT        = digit { digit | "_" } ;
FLOAT      = INT "." INT [ ( "e" | "E" ) [ "-" ] INT ] | INT ( "e" | "E" ) [ "-" ] INT ;
                                                        (* 1_000.5_0 is 1000.50, with no unit *)
STRING     = '"' { char | "\\" escape } '"' ;           (* char: not '"', "\" or a line ending;
                                                           escape: \" \\ \n \t \r *)
UNIT_NAME  = [ "°" ] letter { letter | digit | "_" } ;  (* checked against the unit table:
                                                           V, mV, A, Ω, kΩ, Hz, s, ms, °C, K… *)
COMMENT    = "//" { char } end_of_line ;                (* not "///"; ignored *)
DOC        = "///" { char } end_of_line ;               (* documentation, before the declaration *)
```

Reserved words, everywhere: `labcraft use fn extern type struct enum match for parallel loop
continue_on_error carry next collect until yield handle panel layout icon true false none nan inf`.
`panel`, `layout` and `icon` only start a section, but stay reserved.

Contextual words, reserved only where the syntax expects them and ordinary identifiers elsewhere
(`e.code` reads a field of an error, `ok` can name a value):

- `ok`, `code`: a `handle` pattern;
- `in`: a `for` clause;
- `after`: an argument list (`after:` is the order input, R-15);
- `control`, `indicator`: a `panel` item;
- `node`, `wire`, `at`, `via`, `view`, `standard`, `compact`: a `layout` item;
- `rect`, `circle`, `line`, `path`, `text`, `at`, `radius`, `size`, `fill`, `accent`: an `icon` item.

**Line endings** end a statement, an arm, a `use`, and a `panel`, `layout` or `icon` item. They are
ignored inside `( )` and `[ ]`, inside the braces of a `struct` or `enum` type or of a struct
literal, and in a structure header up to its `{`. Two statements never share a line. The
productions below do not show line endings; this rule applies on top of them.

### 14.2 Files and declarations

```ebnf
file          = header { use_decl } { DOC } ( fn_decl | extern_decl | type_decl )
                [ panel_section ] [ layout_section ] [ icon_section ] ;
header        = "labcraft" INT ;
use_decl      = "use" path ;
path          = IDENT { "." IDENT } ;

type_decl     = "type" IDENT [ generics ] "=" type_expr ;
fn_decl       = fn_sig block ;
extern_decl   = "extern" IDENT "(" [ arg { "," arg } ] ")" fn_sig ;   (* reserved, M8 (4.1) *)
fn_sig        = "fn" IDENT [ generics ]
                "(" [ param { "," param } [ "," ] ] ")"
                "->" "(" [ output { "," output } [ "," ] ] ")" ;
generics      = "<" generic { "," generic } ">" ;
generic       = IDENT [ ":" path ] ;     (* T: Number; T is a whole type, unit included (3.5).
                                            A variable for a unit alone, f64[U]: Q-12 *)
param         = IDENT ":" type_expr ( [ "=" literal ] | "..." ) ;   (* "..." : variadic (5.4) *)
output        = IDENT ":" type_expr ;
```

### 14.3 Types

```ebnf
type_expr     = named_type | array_type | struct_type | enum_type | fn_type ;
named_type    = path [ "<" type_expr { "," type_expr } ">" ] [ unit_annot ] ;
array_type    = "[" type_expr [ ";" INT ] "]" ;          (* [f64]; [f64; 2]: 2 dimensions *)
struct_type   = "struct" "{" field { "," field } [ "," ] "}" ;
field         = IDENT ":" type_expr ;
enum_type     = "enum" "{" variant { "," variant } [ "," ] "}" ;
variant       = IDENT [ "(" type_expr { "," type_expr } ")" ] ;
fn_type       = "fn" "(" [ fn_arg { "," fn_arg } ] ")" "->" "(" [ type_expr { "," type_expr } ] ")" ;
fn_arg        = [ IDENT ":" ] type_expr ;

unit_annot    = "[" unit_expr "]" ;                      (* f64[V], f64[V/s]; f64[U]: Q-12 *)
unit_expr     = unit_term { ( "*" | "·" | "/" ) unit_term } ;   (* equal precedence, left to
                                                                   right: kg/m*s is (kg/m)·s *)
unit_term     = ( UNIT_NAME | IDENT | "1" ) [ "^" [ "-" ] INT ] | "(" unit_expr ")" ;
```

### 14.4 Diagram

```ebnf
block         = "{" { statement } "}" ;                 (* function body *)
statement     = binding | call_stmt ;
binding       = [ label ] target { "," target } "=" rhs ;
call_stmt     = [ label ] call ;                        (* a call alone (R-16) *)
label         = "#" IDENT ":" ;
target        = IDENT | "_" ;
rhs           = call | structure | struct_lit | operand ;

call          = path [ "<" type_expr { "," type_expr } ">" ] "(" [ arg { "," arg } [ "," ] ] ")" ;
arg           = [ IDENT ":" ] ( operand | unit_arg )
              | "after" ":" ( "#" IDENT | "(" order_ref { "," order_ref } ")" ) ;   (* R-15 *)
order_ref     = operand | "#" IDENT ;
unit_arg      = "[" unit_expr "]" ;                     (* with_unit(x, [V]) (R-5) *)
operand       = path | literal ;                        (* path: value, field, variant, category *)
struct_lit    = path "{" field_init { "," field_init } [ "," ] "}" ;
field_init    = IDENT ":" operand ;

literal       = number [ unit_expr ] | STRING | "true" | "false" | "none"
              | "[" [ operand { "," operand } ] "]" ;
number        = [ "-" ] ( INT | FLOAT | "inf" ) | "nan" ;
```

### 14.5 Structures

```ebnf
structure     = match_expr | for_expr | loop_expr | handle_expr ;

match_expr    = "match" operand "{" { match_arm } "}" ;
match_arm     = pattern "=>" arm_body ;
pattern       = literal | [ "-" ] INT ".." [ "=" ] [ "-" ] INT           (* -5..=5 *)
              | path [ "(" IDENT { "," IDENT } ")" ] | "_" ;            (* Reading.Value(v) *)

for_expr      = "for" [ "parallel" ] for_in { "," for_in } { "," carry } for_body ;
for_in        = IDENT "in" ( operand | operand ".." operand ) ;
loop_expr     = "loop" [ "continue_on_error" ] [ carry { "," carry } ] loop_body ;
carry         = "carry" IDENT [ ":" type_expr ] "=" operand ;
for_body      = "{" { loop_stmt } "}" ;
loop_body     = "{" { loop_stmt } until_stmt { loop_stmt } "}" ;      (* exactly one until *)
loop_stmt     = loop_binding | call_stmt | collect_stmt ;
loop_binding  = [ label ] loop_target { "," loop_target } "=" rhs ;
loop_target   = target | "next" IDENT ;
collect_stmt  = "collect" operand ;
until_stmt    = "until" operand ;

handle_expr   = "handle" operand "{" { handle_arm } "}" ;
handle_arm    = handle_pat "=>" arm_body ;
handle_pat    = "ok" | ( category | "code" [ "-" ] INT | "_" ) [ "(" IDENT ")" ] ;
category      = IDENT { "." IDENT } [ "." "*" ] ;

arm_body      = yield_stmt | "{" { statement } yield_stmt "}" ;       (* exactly one yield, last *)
yield_stmt    = "yield" ( operand | "(" operand { "," operand } ")" ) ;
```

### 14.6 `panel`, `layout` and `icon` sections

```ebnf
panel_section  = "panel" "{" { panel_item } "}" ;
panel_item     = ( "control" | "indicator" ) IDENT ":" IDENT [ "(" [ arg { "," arg } ] ")" ] ;

layout_section = "layout" "{" { layout_item } "}" ;
layout_item    = "node" node_ref ( "at" point [ view ] | view )
               | "wire" terminal_ref "->" terminal_ref [ "via" point { point } ] ;
view           = "view" ( "standard" | "compact" ) ;
terminal_ref   = node_ref [ "." IDENT ] ;                (* wire raw.session -> #close.session *)
node_ref       = IDENT | "#" IDENT ;
point          = "(" number "," number ")" ;

icon_section   = "icon" INT "{" { icon_item } "}" ;
icon_item      = ( "rect" point point [ "radius" number ]
                 | "circle" point number
                 | "line" point point
                 | "path" STRING
                 | "text" STRING "at" point [ "size" number ] ) icon_style ;
icon_style     = [ "fill" ] [ "accent" path ] ;
```

### 14.7 Rules outside the grammar

What the grammar alone does not say:

| # | Rule |
|---|---|
| R-1 | **A path is resolved by names**: library (`visa.open`), variant (`State.Init`), field (`step.low`), error category (`io.timeout` in `errors.raise_if`) or panel element (`stop_button` in `ui.get`: a parameter of kind *element* names a panel element, even if an input or an output has the same name; section 10). Unit names are resolved only inside `[ ]` and right after a number (R-5). In a function, a value cannot have the name of an imported library, a visible type, or a panel element that is neither an input nor an output. |
| R-2 | **An output is bound exactly once and can be read back** (`voltage` in section 12). **Exception, a name that is both input and output** (`err`): reading it designates the input; the binding `err = …` designates the output. |
| R-3 | **A `carry` can shadow an outer name** (`carry err = err`): in the body, the name designates the carry. |
| R-4 | **Binding an output, `next x = …` or `collect` does not create a node**: they are terminals (function output, carry, output array). |
| R-5 | **Units**: a name right after a number is always read as a unit (`230 V`, `50 ms`); elsewhere unit names are ordinary identifiers. A unit passed as an argument is written in brackets (`with_unit(x, [V])`, `unitless(v, [mV])`, `with_unit(x, [m/s])`); `[x]` is read as a unit for a parameter of kind *unit*, as an array of one value otherwise. |
| R-6 | **`next`, `collect` and `until` appear only directly in a loop body**, never in a function body or in an arm (grammar). Each carry has exactly one `next`; a `loop` has exactly one `until` and a `for` none (grammar); several `collect` give the outputs in their order. An arm ends with exactly one `yield` (grammar). The place of `next`, `collect` and `until` in the body is free; the canonical form puts them last (5.2). |
| R-7 | **The type of the branches** of a `match` or a `handle`: same number of values, same types; a value with a unit needs the same dimension, and the other branches are converted to the unit of the first branch. |
| R-8 | **A `match` on an enumeration is exhaustive**, otherwise `_` is required. |
| R-9 | **No nesting of calls** (Q-5): an argument is a name, a path, a literal or a unit in brackets, never a call. |
| R-10 | **A `handle` always has a `_` branch** (7.3). |
| R-11 | **One variadic parameter** per function; it follows all required parameters and precedes all parameters with a default value, which are wired by name only (5.4). |
| R-12 | **The `string.format` template is a literal**; its fields and the named arguments match exactly (5.4). |
| R-13 | **An external function has no body and has an `err` output** (4.1). |
| R-14 | **An icon is valid**: grid of 32 in v0, `path` strings conforming to the SVG path syntax, `accent` token known to the theme; a shape that leaves the grid gives a warning (11.1). |
| R-15 | **`after:` is the order input of every call** (5.3): it takes a value, a label (`after: #beep`) or a parenthesized list of values and labels (`after: (#beep, e2)`). No parameter is named `after`. |
| R-16 | **A call alone is a statement** (`#beep: sound.beep()`, label optional): it binds none of the outputs of the node, and is meant for a function without outputs. If the function has an `err` output, the warning of 7.5 applies. |
| R-17 | **A label is unique in its function** (`#close`): it designates the node in `after:` and in the `layout` section. |
| R-18 | **Overlapping patterns**: in a `match`, overlapping literal or range patterns are an error (6.1). In a `handle`, arms may overlap and the most specific one wins: `code` > exact category > `a.b.*` > `a.*` > `_` (7.3). |

## 15. Open questions

| # | Question | Direction |
|---|---|---|
| Q-1 | File extensions | **Settled (2026-11-01): `.lcf` / `.lct`.** `.lct` is free in practice. `.lcf` already exists in niches: ArchiCAD libraries, CodeWarrior linker command files (NXP), Anritsu VectorStar network analyzer calibration kits. Conflict accepted: a LabCraft file is recognized by its first line, `labcraft 1` |
| Q-2 | `duration` distinct from `f64[s]`? | Yes for readability, implicit conversion between the two |
| Q-3 | Temperatures and offset units | Rule 3.3; check the case of averaging temperatures |
| Q-4 | `i64` to `f64` | Explicit, since precision can be lost |
| Q-5 | Allow nesting `add(mul(a, b), c)`? | **Settled (2026-10-30): no in v0.** One node per line; can be added later without breaking files |
| Q-6 | Make parallelism visible in the text | Parallelism is visible in the diagram, but D-08 also asks for it in the text. The canonical form (5.2) does not mark independent branches yet, because "independent branch" needs a precise definition. To revisit on real examples |
| Q-7 | Stopping a loop on error | **Settled (2026-10-31): stop by default** when an `error` carry holds an error, `continue_on_error` to avoid it (6.3) |
| Q-8 | Retry | Generic `retry` function or documented loop pattern |
| Q-9 | Error wire on each call: too verbose? | **Settled (2026-10-30): explicit everywhere.** It carries the order of actions; the text matches the diagram exactly |
| Q-10 | Node identity for git merging | Is the name of the first output enough, or is a hidden identifier needed? |
| Q-11 | Syntax of queues and events | To be written for M2 |
| Q-12 | Unit variables in user functions | A type variable already binds a whole type with its unit (3.5); this question covers a variable for a unit alone. F# model: `fn scale<U>(x: f64[U], k: f64) -> (y: f64[U])`, inference. Also to decide: how `U` is told apart from a type variable (a bound, or its position in `[ ]`) |
| Q-13 | State machine | Reference example written (section 13): readable without dedicated syntax. To be confirmed on a larger machine |
| Q-14 | Functions passed as arguments | **Settled (2026-10-30): typed function references in v0**, without anonymous functions or closures (section 4) |
| Q-15 | Typed configuration loading | `config.load(path) -> (steps: [Step], err: error)`: file (TOML) to declared type, precise field errors |
| Q-16 | Default values that flow like data | Writing rule in v0 (7.2: a verdict depends on `err`). Seen with the proof of concept (`docs/rfc/check_power.lcf`, run against `instruments/psu-timeout.toml`): when the current readback of a power check times out, the verdict is `Error`, as it should be, but the `power` output is `0 W`, a value that looks measured. Options: (a) keep default values and the writing rule; (b) propagate the absence of a value (a node that consumes the output of a skipped node is skipped in turn, and an output reads "no value"); (c) an optional type (`f64[W]?`) for outputs that may be missing. Topic for an RFC |
| Q-17 | Spread an array into a variadic parameter? | No in v0: call the function that takes an array (`string.join`). Spread syntax possible later |
| Q-18 | Mini-language of `string.format` fields | Close to Rust (precision, width, hexadecimal), plus the display unit; inverse reading `string.scan` |
| Q-19 | Locked access to `ref<T>` | Structure that locks the reference during its body (8.1); syntax for M2, with Q-11 |
| Q-20 | Setting a panel element from the CLI or MCP | For example `--set stop_button=true@5s` in deterministic mode, and the equivalent MCP command |
| Q-21 | Default values not covered by 7.2 | `timestamp` (the origin of the clock?), function references (a reference that fails when called?), `ref<T>` and `queue<T>` (an invalid reference, like an opaque type?) |

## Appendix A. Library signatures (v0)

The signatures of the library functions that this document and `examples/` use, written as
declarations without a body. Conventions:

- `N` is a type variable bound by `Number` and `T` a variable without a bound (3.5); each stands
  for a whole type, unit included. When the unit of a result follows a rule of 3.3, the result type
  says it informally (`A·B`, `√N`) and a comment gives the rule.
- A parameter of kind *unit*, *category* or *element* (R-1, R-5) shows its kind in place of a type:
  `unit: kind unit`.
- `after:` (R-15) exists on every call and is not repeated.
- The proof of concept (`tools/lcf-poc/lcfpoc/stdlib.py`) implements these functions, except those
  marked "not in the PoC".

### A.1 `core`

```text
fn add<N: Number>(a: N, b: N) -> (result: N)               // same dimension; unit of `a` (3.3)
fn sub<N: Number>(a: N, b: N) -> (result: N)               // same dimension; unit of `a`
fn mul<A: Number, B: Number>(a: A, b: B) -> (result: A·B)  // exponents add
fn div<A: Number, B: Number>(a: A, b: B) -> (result: A/B)  // exponents subtract; x/0: inf or nan
fn abs<N: Number>(x: N) -> (result: N)
fn neg<N: Number>(x: N) -> (result: N)
fn sqrt<N: Number>(x: N) -> (result: √N)                   // exponents halved; rejected if odd
fn pow<N: Number>(x: N, exponent: i32) -> (result: N^exponent)   // constant exponent; not in the PoC
fn min<N: Number>(a: N, b: N) -> (result: N)               // same dimension
fn max<N: Number>(a: N, b: N) -> (result: N)               // same dimension

fn lt<N: Number>(a: N, b: N) -> (result: bool)             // same for le, gt, ge
fn eq<T>(a: T, b: T) -> (result: bool)                     // same for ne
fn and(a: bool, b: bool) -> (result: bool)                 // same for or
fn not(a: bool) -> (result: bool)
fn select<T>(condition: bool, if_true: T, if_false: T) -> (result: T)

fn with_unit(x: f64, unit: kind unit) -> (result: f64[unit])     // `with_unit(raw, [V])`
fn unitless<N: Number>(x: N, unit: kind unit) -> (result: f64)   // unit of the dimension of `x`
fn to_i32(x: f64) -> (result: i32)                         // narrowing (3.4); not in the PoC
fn round_to_i32(x: f64) -> (result: i32)                   // not in the PoC
fn to_f32(x: f64) -> (result: f32)                         // not in the PoC
```

`call` runs a function reference (section 4); its inputs after the first and its outputs are those
of the referenced function. Not in the PoC.

```text
fn call(function: fn(…) -> (…), …) -> (…)
```

### A.2 `array`, `string`, `time`, `errors`, `ui`

```text
fn array.len<T>(array: [T]) -> (length: i32)
fn array.get<T>(array: [T], index: i32, err: error = none) -> (item: T, err: error)
fn array.push<T>(array: [T], item: T) -> (array: [T])
fn array.sum<N: Number>(values: [N]) -> (sum: N)           // not in the PoC

fn string.concat(parts: string...) -> (text: string)       // 5.4; not in the PoC
fn string.join(parts: [string]) -> (text: string)          // not in the PoC
fn string.format(template: string, …) -> (text: string)    // an input per field (5.4); not in the PoC

fn time.now() -> (timestamp: timestamp)                    // order it with `after:`
fn time.elapsed(since: timestamp) -> (duration: duration)
fn time.wait(duration: duration, err: error = none) -> (err: error)

fn errors.raise_if(condition: bool, category: kind category, message: string = "",
                   err: error = none) -> (err: error)
fn errors.merge(errs: error...) -> (err: error)            // 7.4; the PoC takes up to six

fn ui.get<T>(element: kind element) -> (value: T)          // T: type of the element; not in the PoC
fn ui.set<T>(element: kind element, value: T) -> (value: T)   // section 10; not in the PoC
```

`array.get` fails with the category `array.index_out_of_range`; `errors.raise_if` produces an
error of the given category when `condition` is true, `none` otherwise.

### A.3 `visa` and `scpi`

```text
fn visa.open(resource: string, timeout: duration = 2 s,
             err: error = none) -> (session: visa.session, err: error)
fn visa.close(session: visa.session, err: error = none) -> (err: error)   // always (7.2)
fn visa.write(session: visa.session, command: string,
              err: error = none) -> (session: visa.session, err: error)
fn visa.read(session: visa.session,
             err: error = none) -> (text: string, session: visa.session, err: error)
fn visa.query(session: visa.session, command: string,
              err: error = none) -> (text: string, session: visa.session, err: error)

fn scpi.query_f64(session: visa.session, command: string,
                  err: error = none) -> (value: f64, session: visa.session, err: error)
fn scpi.check_errors(session: visa.session,
                     err: error = none) -> (session: visa.session, err: error)   // SYST:ERR?
```

### A.4 Libraries of the examples

`dmm_driver`, `relays`, `psu` and `results_log` belong to the examples (section 13), not to the
minimal library; the proof of concept implements them natively.

```text
fn dmm_driver.read_dc_volts(session: visa.session,
                            err: error = none) -> (volts: f64[V], session: visa.session, err: error)
fn relays.apply(bank: relays.bank, route: string,
                err: error = none) -> (bank: relays.bank, err: error)
fn psu.set_voltage(source: psu.session, voltage: f64[V],
                   err: error = none) -> (source: psu.session, err: error)
fn results_log.append_csv(results: [StepResult], err: error = none) -> (err: error)
```

The libraries `instrument` (6.3), `sound` and `lamp` (5.3) are illustrative and have no signatures
here. The signatures of `config` (Q-15) and `sim` (section 9) are not written yet.
