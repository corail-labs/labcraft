# Language and format specification — v0

*Last updated: 2026-11-09. Status: draft, nothing is frozen.*

This document describes both the language and the file format: the text syntax *is* the format
(D-08). Keywords are in English, since the public repository will be too (D-21). Open questions are
gathered at the end and numbered Q-1, Q-2…

Scope of v0: the language of milestone M0 (types, functions, nodes, errors, loops, `match` and
`handle`; section 9 lists what M0 implements and what comes later) and the place reserved for what
follows (events, panel widgets, timed loops, PLC mode).

## 1. Model

- A **project** contains **libraries**. A library is a folder with a `labcraft.toml` manifest (name,
  version, dependencies: the basis of the package, D-25). In M0 only the libraries shipped with
  LabCraft have one (section 9), with the fields `name`, `version`, `description`, `dependencies`,
  `always` (native functions, 7.2), `resources` (opaque types, 8.1) and `categories` (of errors,
  7.1); see `libs/visa`.
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
| Name | The file bears the name of what it declares, exactly: `fn scale` in `scale.lcf`, `type Verdict` in `Verdict.lct` (R-22) |
| Encoding | UTF-8, LF line endings; the file ends with a single line feed. On reading, CR LF and a byte order mark at the start are accepted, and `fmt` removes them. A CR not followed by a line feed and the other control characters (Unicode category Cc), tab and line feed apart, are refused. A bidirectional control character in a string or a comment gives a warning: the text does not read as it runs |
| Header | `labcraft 1`: format version, on the first line (14.2); older versions are migrated, a newer version is refused with a clear message |
| Section order | header, `use`, documentation comment, `fn` (or `extern` then `fn`, 4.1), `panel` (optional), `layout` (optional), `icon` (optional); a `.lct` file has the header, `use`, documentation comment and `type`, nothing else (R-19) |

Skeleton of a function file:

```text
labcraft 1

use scpi
use visa

/// Documentation of the function (read by the IDE, the CLI and agents).
fn name(inputs) -> (outputs) {
    diagram
}

panel { … } // optional: attached panel (section 10)

layout { … } // optional: layout on screen (section 11)

icon 32 { … } // optional: icon of the compact view (section 11.1)
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
| `timestamp`, `duration` | absolute timestamp; duration: a distinct type, implicitly converted to and from `f64[s]` (Q-2). A `duration` is a whole number of nanoseconds, as the clock of a run is: from an `f64[s]`, the value is multiplied by 10^9 as an `f64`, then rounded to the nearest integer, ties to even (as `libs/visa/simulation.md` does for its times), and a value that is not finite or does not fit is Q-25. To a number, its nanoseconds are taken as an `f64` and divided by 10^9, then converted as a wire is (3.3) |
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
map<K, V>, set<T>     // dictionary and set, M2 (Q-28)
```

`[f64; 2]` is a 2-dimensional array: the number after `;` counts dimensions, not elements (unlike
Rust). The size of an array is a property of the value, not of its type. An `[f64; 2]` is rectangular: its
rows have the same length, and an `[f64; 3]` has one length at each of its levels. An `[[f64]]` is an array of `[f64]`, whose rows are free. They are two types, and
neither converts to the other on a wire: `array.from_rows` and `array.to_rows` do it (appendix A). Only a constant
becomes an `[f64; 2]` where one is expected, and its rows have one length, or it is rejected. An array that holds a
wire keeps the type of its wires (3.4), so `[row, row]` needs `array.from_rows`, whose `err` says when rows differ.

A `waveform<T>` has three fields, `t0: timestamp`, `dt: duration` and `samples: [T]`, which read like those of
a struct (`w.samples`). It is built like one, with all three: `waveform { t0: t, dt: 1 ms, samples: s }`. `T`
comes from `samples`; a literal there, such as `[]`, takes it from what the waveform goes to, and if nothing says
it, it is an error. `T` is a number with any unit, not a `duration`: `waveform<string>` and `samples` of
`[[f64]]` are rejected. As for any array (3.4), nothing converts between `waveform<f64[V]>` and
`waveform<f64[mV]>`. With `type Trace = waveform<f64[V]>`, `Trace { … }` builds that type, so its `samples` are
judged against `[f64[V]]`.

A named type is defined in its own `.lct` file (`type Verdict = enum { Pass, Fail }`). A type cannot
contain itself, directly or through other types: any mention counts, in an array or a function type too.
A type cannot bear a name that the language gives to a type: the base types (3.1), `waveform`, `queue`,
`ref`, `event`, `notifier`, `map` and `set`, kept for M2 (6.4), and `opaque`.

**An alias is another name.** With `type Volts = f64[V]`, `type Readings = [f64[V]]` or `type A = Verdict`,
the two names are one type everywhere, and conversions (3.3, 3.4) apply through the name: an `f64[mV]` is
accepted where a `Volts` is expected. An alias of an enumeration names its variants: `A.Pass` is
`Verdict.Pass`. Only a declared `struct` or `enum` makes a new type, known by its library and its name:
`type Verdict = enum { Pass, Fail }` and `type Grade = enum { Pass, Fail }` are two types, and a declared
struct is not a struct written in place with the same fields. A number that must not mix with other
numbers is a `struct` with one field.

**Opaque types**: a library can declare a type whose content is hidden, handled only by its
functions: `type session = opaque`. This is the case for the session `visa.session` (the equivalent
of LabVIEW's *VISA resource name*): it designates an open connection, not an instrument. Having
neither fields nor variants, its name never appears among operands, so a value can bear it (R-1):
`fn close(session: session, …)` in `libs/visa`.

**Alias of an opaque type.** A driver library can declare its opaque type as an alias of another
library's opaque type: `type bank = visa.session` in the `relays` library. `relays.bank` and
`visa.session` are then two names of the same type, and a resource (8.1) if the type is one: a
`relays.bank` is accepted wherever a
`visa.session` is expected, and the reverse. This is how the functions of a generic library apply
to a driver's session (`scpi.query_f64(bench.relays, …)`, section 13), while the driver's own
signatures still say what the session designates.

### 3.3 Physical units (D-05)

- A number can carry a unit: `f64[V]`, `f64[mA]`, `f64[V/s]`. Without a unit it is dimensionless:
  this is the default.
- The named units of v0 are the base units `m` `kg` `s` `A` `K` `mol` `cd`, then `g`, the derived
  units `Hz` `N` `Pa` `J` `W` `C` `V` `F` `Ω` `S` `Wb` `T` `H`, the times `min` `h`, and the scales
  `°C` `°F`. The prefixes are `p` `n` `µ` `m` `k` `M` `G`, on every name but `kg`, `min`, `h`, `°C`
  and `°F`. `V/A` and `Ω` are the same unit.
- **Same dimension, different units**: implicit conversion, shown as a coercion (`mV` to `V`). It
  is lossless in dimension; the scale factor is applied in floating point, by one operation where it can, so
  that every implementation gives the same bits. A value is held in the unit of its type. It goes to a unit
  whose scale is *n* times larger by one division by *n* (`9 mV` to `V` is 9 ÷ 1000, so 0.009), and to one *n*
  times smaller by one multiplication by *n*, *n* an integer; otherwise it is multiplied by the numerator of
  the ratio of the two scales, then divided by its denominator. Each integer is taken as the nearest `f64`, which
  is exact up to 2^53 and for every power of ten up to 10^22. These operations are in `f64`: an `f32`, or each part of a
  `c64`, is widened to `f64`, which is exact, and the result is rounded to the nearest `f32` once, ties to even. A temperature with an offset (`°C`, `°F`) first loses
  the offset of its unit, then changes scale as above, then gets the offset of the target, each in one operation. The result of an
  operation takes the unit of its first operand: `core.add(1 V, 200 mV)` is `1.2 V`, `core.add(200 mV, 1 V)`
  is `1200 mV`.
- **Different dimensions**: error at edit time (`core.add(voltage, current)` is rejected).
- **Temperatures**: `°C` and `°F` are scales with an offset. A value in `°C` is an absolute
  temperature; the difference of two temperatures is in `K`. In a compound unit a temperature counts
  as a difference, with no offset: `V/°C`, and even `°C^1`. Q-3.
- Literals: `230 V`, `10 ms`, `4.7 kΩ`.
- **Compound units**: `*` (or `·`), `/` and `^`. `^` applies to the unit just before it: `m/s^2`
  is m/(s²). `*`, `·` and `/` have equal precedence and apply from left to right: `kg/m*s` is
  (kg/m)·s. Use parentheses for anything else: `kg/(m*s)`.
- `core.unitless(x, [V])` removes the unit by expressing `x` in the requested unit
  (`core.unitless(230 mV, [V])` is 0.23), `core.with_unit(x, [V])` adds one: both are explicit, like any
  conversion that can hide an error. The unit of `core.unitless` is mandatory and must have the
  dimension of `x`: if the upstream type changes from `V` to `mV`, the resulting number does not
  change silently.

**Mechanism (dimensional analysis).** A unit = a vector of exponents over the 7 base dimensions
(m, kg, s, A, K, mol, cd) + an exact scale factor, so that two units are equal whatever the order of
the operations that made them. Ω = kg·m²·s⁻³·A⁻², A = A¹.

| Operation | Rule | Example |
|---|---|---|
| `core.mul` | adds the exponents | Ω × A = V; V × A = W |
| `core.div` | subtracts the exponents | V / A = Ω |
| `core.add`, `core.sub`, comparisons | identical exponents required | V + A rejected |
| `core.sqrt` | divides the exponents by 2, rejected if impossible, or if the scale is not an exact square (`kV·V`) | √(V²) = V |
| `core.pow` | constant integer exponent | V² |

Everything is computed at compile time; at run time units no longer exist (zero cost), only scale
conversions add an operation (above). A physical error is detected at edit time:
an output `power: f64[W]` bound to `core.mul(r, i)` is rejected, because R × I gives V.

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

- **Implicit widening**, shown in the IDE (coercion point), only where nothing can be lost, and this
  list is closed: a signed integer to a larger signed one; an unsigned integer to a larger unsigned
  one or to a strictly larger signed one (`u16` to `i32`); integers of up to 32 bits to `f64`; `f32`
  to `f64`; `c64` to `c128`.
- **Explicit narrowing** by a node: `core.to_i32(x)`, `core.round_to_i32(x)`, `core.to_f32(x)`.
- Anything else is explicit: `i64` and `u64` to `f64`, which can lose precision without changing size
  (Q-4), a signed integer to an unsigned one, an integer to `f32`, a real number to a complex one.
- **Two number types that meet** in one operation (`core.add(n, 0.5)`) give the smallest type that both
  widen to: `i32` and `f64` give `f64`, and so do `i32` and `f32`. Without one (`i64` and `f64`), the
  second is rejected.
- **A change of scale on an integer is rejected**, since it would lose what it cannot hold: `i32[mV]` to
  `i32[V]`. To a floating-point type it is accepted: `i32[mV]` to `f64[V]`.
- **`core.div`, `core.sqrt` and `core.pow` on integers give an `f64`**: `core.div(3, 2)` is `1.5`, and x/0
  is `inf` or `nan`. `core.add`, `core.sub` and `core.mul` stay integers. An `i64` or a `u64`, which does
  not widen to `f64`, is rejected there.
- **A conversion applies to a number on a wire**, never inside an array or a struct: `[f32]` to `[f64]` is
  rejected. In an array literal, the first element that decides gives the type of the others, as for
  branches (R-7): `[1 V, 200 mV]` is an `[f64[V]]`.
- **An array that holds a wire has the type of its wires**, at any depth, wherever it goes: two `f64[V]` wires
  for an `[f64[mV]]` are rejected, and so is an `i32` wire in `[n]` for an `[f64]`, on the wire that leaves the array.
  So a type never reaches a wire from where its array goes, and an array of wires never becomes an `[f64; 2]` (3.2).
  An array of constants alone takes the type expected.
- **Literals**: a literal takes the number type expected where it goes: an input, an output, a carry, a field, a
  value of a variant, a default value, an element of an array, a value of an arm (R-7), a `collect` whose loop gives
  its array to an output or a carry (6.2). A named constant (5.1) does the same at each terminal it reaches, and an
  error is on that terminal: an array constant that does not fit is refused there as the array it is on its own line.
  On its own line nothing is expected, so `k = []`, `k = 3000000000` and `k = core.abs` are refused there, which
  differs from the same constant written on a terminal. Judging constants again has a budget proportional to their
  size, far beyond any real program; past it, a constant has the type of its own line. So `gain: f32 = 0.5`,
  `v: f32[V] = 1.5 V`, `n: i32[mV] = 5 mV` and `z: c128 = 1.5` are accepted. A decimal does not become an integer:
  `n: i32 = 1.5` and `5.0 mV` for an `i32[mV]` are rejected. A literal keeps its unit and then converts as a wire
  does: `1 V` for an `f32[mV]` is `1000 mV`, and `1 V` for an `i32[mV]` is rejected. Where nothing is expected, an
  integer is an `i32`, a decimal an `f64`, and a literal with a unit an `f64` with that unit (`0 V` is `f64[V]`). Two
  wires that meet expect nothing of each other: `core.add(x_f32, 0.5)` is an `f64`. A type argument or an array that
  fixes a variable (3.5) does: `array.push(floats, 0.5)` with an `[f32]` takes it. A literal
  without a unit has no dimension, whatever its context: `core.ge(voltage, 0)` is rejected, and
  `core.ge(voltage, 0 V)` says what it means. A unit with no dimension but a scale asks the same: `5` for an
  `f64[mV/V]` would read 5000 mV/V, so it is rejected, decimals included, and `5 mV/V` says what it means. A
  wire without a unit still converts, since its type is written where it is declared. Between two operands that do not share a type variable, an
  integer literal takes the number type of the other: `core.div(x_f32, 2)` is an `f32`. An integer literal
  keeps its value in its type or is rejected: `300` for an `i8`, `16777217` for an `f32`,
  `9007199254740993` for an `f64` or with a unit. A decimal rounds to the nearest: `9007199254740993.0` is
  accepted. A literal too large for its type is an error, by its value in the unit it goes to: `1e39` for an `f32` or
  a `c64`, `1e36 V` for an `f32[mV]`, while `1e39 mV` for an `f32[V]` is accepted. One too small to tell from zero reads
  as zero (`1e-999`, or `1e-50` for an `f32`). A literal in the unit it goes to is read once in its type; in another
  unit it is read as an `f64`, converted as a wire is (3.3), then rounded to its type once: `1e39 mV` for an `f32[V]`
  is the `f32` nearest to 1e36.

### 3.5 Generics

```text
fn mean<T: Number>(values: [T]) -> (result: T)
```

Generics replace what LabVIEW does with hand-written polymorphic VIs. A type variable binds the
whole type, unit included: for `mean` of a `[f64[V]]`, `T` is `f64[V]`, so the result is an
`f64[V]`.

- **The v0 bound is `Number`**: integers, floats and complex numbers, with any unit. A `duration`
  counts as an `f64[s]`: `core.add` of two durations is an `f64[s]`. A variable without a bound stands
  for any type (`core.select`, appendix A).
- **Complex numbers have no order**: `core.lt`, `core.le`, `core.gt`, `core.ge`, `core.min` and `core.max`
  reject them.
- **A type argument or an array fixes the variable**, and an input is then judged against it like any
  other: `array.push(ints, x_f64)` with an `[i32]` is rejected, since an `f64` only narrows to an `i32`.
- **A node of constants takes its type from what its line binds**: with no wire to give a variable its type, an
  output or a carry that has a type gives it, as for a constant (3.4). `x, e = array.repeat(0, 100)` with an output
  `x: [f64]` is an `[f64]`, and `y = core.select(c, 0.5, 1.5)` with an output `y: f32` an `f32`. What does not fit
  changes nothing, and the wire that leaves the node says so. A name with no type gives nothing: in
  `r, e = array.repeat(0, 100)`, `r` is an `[i32]`, whatever it goes to. Neither does an output whose type is computed
  (`core.mul`, `core.div`). In a variable of `Number`, a constant keeps its unit when that unit or the one expected is a
  scale with an offset, since adding in another scale gives another value: `core.add(20 °C, 10 °C)` for an `f64[K]` adds in
  °C, then converts, `core.add(20 K, 10 K)` for an `f64[°C]` adds in K, and so does `array.sum` on an array of them.
- **A wire comes before a constant**, whatever their order: `array.concat([1 mV], volts)` and
  `array.concat(volts, [1 mV])` are both arrays of `f64[V]`. Between an array and a value, the array decides, even a
  constant one (`array.push([1 mV], v)` is an `[f64[mV]]`).
- A variable that stands for a unit alone (`f64[U]`) is open question Q-12, which also covers how
  to tell such a variable from a type variable.

## 4. Functions

```text
/// Reads a voltage and tells whether it is within the limits.
fn check_voltage(
    resource: string,
    low: f64[V] = 220 V, // optional input: default value
    high: f64[V] = 240 V,
    err: error = none,
) -> (verdict: Verdict, voltage: f64[V], err: error) {
    …
}
```

- An input without a default value is **required**: a call that does not wire it is rejected.
- **The default value of a parameter is a constant**, written without reading the diagram or calling
  anything (R-21): a literal, a variant with its values, a struct construction, the name of a function.
- Outputs are named. An output left unwired by the caller is allowed; an unwired `err` output
  gives a warning (7.5).
- Call: `name(arguments)`, positional then named (`check_voltage(r, high: 250 V)`).
- A parameter can be **variadic** (`parts: string...`): see 5.4.
- **Scope.** Functions and types of the same library are named without qualification (`settle`,
  `Verdict`), so they cannot share a name. Anything from another library, types included, needs
  `use lib` in the file and the qualified name: after `use visa`, `visa.open` and `visa.session`,
  and for a variant `lib.Type.Variant`. The libraries of section 9 are no exception: after `use core`,
  `core.add`, `core.ge`, `core.select`. So a library can define its own `add`, and a new function in
  `core` breaks no program. The base types (3.1) and the bound `Number` (3.5) belong to the language and need no `use`. `use a.b`
  imports `b`, a sub-library of `a` (a library nested in the folder of `a`); its contents are named
  `a.b.name`.
- **The name of a library** is its path from the root of the project, the folder given to the tool:
  `use relays` in `examples/station.lcf` is `examples/relays`. A library can use another of the
  project and the libraries shipped with LabCraft, but not in a cycle, and nothing can use the root,
  which has no name: a type shared with a sub-library is declared in that sub-library
  (`results_log.StepResult`, section 13). A library shipped with LabCraft sees only the others that
  are shipped: a folder `visa` in a project does not change what `scpi` runs.
- **Function references** (Q-14): a named function, of a library or of the primitives, can be passed as
  an argument, with a signature type checked at edit time, for example
  `measure: fn(visa.session, err: error) -> (f64[V], visa.session, error)`. The callee runs it
  with `core.call(measure, …)`. The name of an argument says how `core.call` wires it and is not part of the
  type: `fn(x: f64[V])` and `fn(f64[V])` are the same type, and so is a function with its own names
  for its parameters. No anonymous functions or closures in v0. This is the typed equivalent
  of LabVIEW's *Call By Reference*, and what makes it possible to write `settle` or `retry` once
  for all measurements.
- **A reference matches a function type exactly**: the same inputs and outputs, in number and in type, with
  no conversion, since no node would hold it. A default value does not shorten a signature, and a
  variadic parameter counts as an array (`parts: string...` is `fn([string])`). A generic primitive takes
  its variables from the type expected (`core.abs` for `fn(f64[V]) -> (f64[V])`), and it is rejected where no
  function type is expected, `core.call(core.abs, x)` included. Beside a wire it takes the type of that wire, in any
  order: `core.select(c, core.lt, f)`. Its result follows the rules of its node:
  `core.mul` for `fn(f64[V], f64[A]) -> (f64[W])`, `core.sub` for `fn(f64[°C], f64[°C]) -> (f64[K])`. `core.pow`
  needs a number without a unit, since the unit of its result depends on a constant exponent. A parameter of kind
  unit, category or element has no function type, and neither have `core.call` and `string.format`, which take
  more than their parameters. `core.call` wires every input of the type, in order or by
  the names that the type gives, since a function type has no default values.

A function with every kind of default value:

```text
labcraft 1

use core

fn sweep(
    mode: Mode = Mode.Fast,
    start: Reading = Reading.Value(0 V),
    limits: Limits = Limits { low: 220 V, high: 240 V },
    compare: fn(f64, f64) -> (bool) = core.lt,
    steps: [i32] = [1, 2, 5],
) -> () {}
```

### 4.1 External functions

A function can be declared without a body and implemented outside LabCraft (D-14). The syntax is
fixed from v0 so that libraries can plan for it. The `rust` interface runs from M0: it is how the
libraries shipped with LabCraft implement their native functions (`visa`, roadmap, `libs/`). The
other interfaces run from M8; until then, an external function is read and checked as a
signature, and calling it returns an `error` of category `extern.unavailable` on its `err`
output.

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
| `rust` | `crate`, `function` | A Rust crate linked into the application, from M0: the native functions of the libraries shipped with LabCraft and of the examples (`results_log`, A.4). How a project adds its own crate is for M8. For example `extern rust(crate: "labcraft-visa", function: "close")` before `fn close(…)` in `libs/visa` |

- **The declaration is the contract.** Types are converted and checked at the boundary, at run
  time. Units cannot be: the declaration asserts them, like `core.with_unit`.
- **An `err` output is required** (R-13): missing library, exception, timeout.
- An external function does not exist in the browser (D-22): error at edit time for the web
  target, except a `rust` function whose crate is built for WebAssembly (the simulator of `visa`).

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
- **No nesting in v0**: `core.add(core.mul(a, b), c)` is rejected; each node is a line with named outputs.
  This is longer but unambiguous, easy to reread, and every node has a name (Q-5).
- Constants are written directly as arguments (`visa.query(dmm, "MEAS:VOLT:DC?")`).
- **A path can be an argument**: a field access (`step.voltage` reads a field of a struct), a
  variant (`State.Init`), an error category (`io.timeout`). For example
  `relays.apply(bench.relays, step.route)`. A path is not a nesting of calls.
- **Naming a value**: `name = operand` gives an existing value a new name (`next v = x`, `err = e3`). It creates
  no node.
- **Naming a constant**: `limit = 240 V` makes a *named constant*, for a value that feeds several terminals or that
  the user wants to place. In the diagram it is a small node without a body: its name, its value and one output
  terminal, called `value` (visual notation, 3.1). It stays a constant: each terminal it reaches gives it a type, as
  for a constant written on that terminal (3.4), so `limit` can go to an `f32[V]` and to an `f64[mV]`. A name given to
  a named constant is the same constant. An array that holds only constants is a constant; an array that holds a wire
  (`k = [s, s]`) is not: it is a node that receives its wires, which the user places too, and it has the type of its
  wires (3.4). A constant on one terminal is simply written there, without a name. Binding an output or a carry to a
  constant makes no node (R-4). A name for an enum value (`k = Verdict.Pass`) is placed the same way, and has one type
  wherever it goes.
- **Struct construction**: `r = StepResult { name: n, value: v, passed: p }`, a node. Every field is given, once.
  A field is an input of the node: a wire converts to it (3.3, 3.4), and a literal takes its type.
- **Variant construction**: `r = Reading.Value(v)`, a node, not a nested call. A variant with
  several values takes one argument per value. Its values have no default: `Reading.Value` and `Reading.Value()`
  are rejected for a variant that holds some. `Reading.Overload` and `Reading.Overload()` are the same.
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

**Canonical form.** It is how the IDE and `labcraft-cli fmt` write a file: the order of the lines,
then the layout of the text. It guarantees the text → graph → text round trip, byte for byte (D-08).
The order applies to each body on its own: the function body, each arm, each loop body.

- **The inputs of a line** are the names it reads, including those read in its sub-bodies and in
  `after:`, that another line of the same body defines. A label (`#close`) counts as a name. This
  is read from the syntax alone: every operand reads its name or the first segment of its path
  (`step` in `step.low`), `[x]` reads `x` whether it turns out to be a unit or an array, and a name
  that a sub-body binds itself (a carry, a pattern, one of its lines) is not read from outside.
- **The name of a line** is its first output that is neither `_` nor `next x`, or else its label.
  A call statement without a label has the empty name.
- **Algorithm**: repeatedly emit, among the lines whose inputs are all emitted, the one that comes
  first in this order:
  1. ordinary lines, by name in code point order. A node that produces a function output is an
     ordinary line (`voltage = core.with_unit(raw, [V])` in section 12);
  2. `next` lines (whose first output that is not `_` is `next x`), by carry name;
  3. `collect` lines, in their written order (it is the order of the outputs, R-6);
  4. the `until` line;
  5. in the function body only, the lines that only bind a function output to a value
     (`err = e3`), in signature order.

  Ties keep the written order. The `yield` of an arm is last, by grammar. A body that cannot be
  ordered (a cycle, a value or a label defined twice) keeps its written order. `fmt` reads no other file
  ([cli.md](cli.md)): a name that the file does not define creates no dependency, and `check`
  reports it if it resolves nowhere.
- **Comments** move with their statement: a comment is attached to the statement that follows it,
  or to the one that ends on its line. A comment with no statement after it in its body stays at
  the end of the body, at the level of the body. A comment outside a declaration belongs to the
  section that follows it, or to the last section at the end of the file. There is none between
  two `use` lines: a comment before or between them goes above all of them, in its written order. A
  comment before the parameters of a declaration (after `fn`, in its generics, in the arguments of
  `extern`) goes on the line before the declaration. An end-of-line comment after an opening `{` (of a function, an arm, a `match`, a
  `handle`, a loop) stays attached to that line, except in the next case. A comment in an arm whose
  body is only a `yield`, even one after its `{`, goes on the line before the arm. A comment inside a
  statement written over several lines goes on the line before the statement, in its written order. Inside a split signature, struct, enum or list of loop clauses
  (below), and inside a `panel`, `layout` or `icon` section, a comment is attached to the item that
  follows it, or to the one that ends on its line; with no item after it, it stays last, at the
  level of the items. A comment inside an item, between its first and its last token, goes on the line
  before that item.
- **No comment is lost.** A line or a section that `fmt` would remove (a repeated `use`, a layout
  entry that names no node, an empty section) stays if a comment is attached to it, or inside it.

**Layout of the text.** The canonical form also fixes how each line is written, so that a program
has one text:

- **Sections**: header, `use` lines, documentation comment and declaration, `panel`, `layout`,
  `icon`, separated by exactly one blank line. An `extern` line and its `fn` line form one
  declaration, with no blank line between them (4.1). The `use` lines are sorted in code point
  order, one per line, each library once. No blank line inside a body or a section.
- **Indentation**: 4 spaces per level, no tab, no space at the end of a line.
- **Spaces**:
  - one after a comma, a `;` and the `:` of a parameter, a field, a named argument or a label, none
    before them (`[f64; 2]`, `after: e2`);
  - one on each side of `=`, `=>` and `->`, and one before an opening `{`;
  - one inside braces written on one line (a struct, an enum, a struct construction:
    `struct { name: string }`); none inside parentheses, square brackets and angle brackets; empty
    ones are written `()`, `[]`, `{}`; no trailing comma in a form written on one line;
  - none between the name of a function, a type, a variant, a category, a library, a panel kind, the
    wildcard `_` or the `fn` of a function type and the `(`, `[` or `<` that follows it
    (`core.add(a, b)`, `f64[V]`, `mean<T: Number>(values: [T])`, `io.timeout(err)`, `_(err)`,
    `numeric(format: "0.000")`, `fn(visa.session)`); one after any other keyword, a number, or a
    word of the `layout` and `icon` sections (`yield (a, b)`, `code -1073807339 (err)`,
    `at (40, 80)`, `rect (2, 4)`);
  - none around `..`, `..=`, `.`, after `#`, before `...` (`0..5`, `visa.*`, `#close`,
    `parts: string...`), after a unary minus (`-1`), nor inside a unit (`m/s^2`, `kg/(m*s)`);
  - one between a number and its unit (`2 s`, `9.81 m/s^2`).
- **No alignment**: `=>`, `=` and comments are never aligned with the neighboring lines
  (`true => yield …`), so that adding a line never changes the others.
- **Braces of a body** (function, arm, `match`, `handle`, `loop`, `for`): `{` ends its line and `}`
  stands alone on its line, at the level of the statement; an empty body is `{}`. An arm whose
  body is only a `yield` is written on one line, `pattern => yield …`; any other arm has a body in
  braces. A `match` or a `handle` is never written on one line.
- **Line width: 100 characters**, counted in Unicode code points, indentation included and
  comments excluded (they are never rewrapped).
  - **A signature** is written on one line if it fits and has no comment between its parentheses.
    Otherwise its parameters go one per line, each followed by a comma, and `) -> (outputs) {` goes
    on its own line (`check_voltage`, section 12). If that line still does not fit, or the outputs
    contain a comment, the outputs are split the same way: `) -> (` ends the line, one output per
    line followed by a comma, then `) {`. Outputs are split only when the parameters are; a
    function without parameters keeps `fn name() -> (` on its first line. A default value is
    never split, whatever its length: a struct construction in it stays on one line.
  - **The fields of a struct and the variants of an enum** follow the same rule: one line if it fits
    and has no comment between its braces, otherwise one item per line, each followed by a comma.
    Inside `[ ]`, `( )` or `< >`, the items are indented one level from the line that holds the `{`,
    the brackets adding no level, and the `}` is at the level of that line (`type T = [struct {`,
    the items, `}]`). A line is judged as a whole: when it does not fit or has such a comment, every
    struct and enum directly on it is split, even one that would fit, and each item is then judged
    on its own line.
  - **A `loop` or a `for`** whose clauses (`in` clauses and carries) do not fit, or contain a
    comment, ends its line with the keyword (with `parallel` or `continue_on_error` if present),
    then puts one clause per line, indented one level and separated by commas, then `{` alone at the
    level of the statement (`station`, section 13).
  - **Every other statement stays on one line**, whatever its length, struct constructions
    included; for a `match`, a `handle`, a `loop` or a `for`, that line runs up to its `{`. One node
    per line (5.1) keeps most lines short; a call with many named arguments can still go past 100
    characters (`run_step`, section 13). The patterns of an arm stay on the line of its `=>` too, in
    their written order.
- **Parentheses around a single value** are dropped: `yield (a)` becomes `yield a`, `after: (e2)`
  becomes `after: e2`.
- **Literals keep their spelling**: `4.7 kΩ` is not rewritten as `4700 Ω`, nor `1e3` as `1000`.
  Only the space before the unit and the spelling of the unit name are normalized: `2s` becomes
  `2 s` (R-20).
- **Names and units** are written in their one spelling (R-20), outside strings and comments.
- **Items of `panel`**: the `control` items, then the `indicator` items. In each group, the
  elements that have the name of an input (for `control`) or of an output (for `indicator`), in
  signature order, then the others by name in code point order. Arguments keep their written order.
- **Items of `layout`**: the `node` items by name, then the `wire` items by source and destination,
  all in code point order on the text of the reference (so `#close` comes before `dmm`). Ties keep
  the written order, and so do the points after `via`. A coordinate that is an integer literal in
  the range (section 11) is written in plain decimal: `1_000` becomes `1000`, `-0` becomes `0`.
  Any other coordinate (`5.0`, `1e3`) is an error for `check` and keeps its spelling.
- **Items of `icon`** keep their written order: a shape is drawn over the ones before it.
- **An empty `panel` or `layout` section**, with neither an item nor a comment, is removed.
- **Comments**: `//` and `///` are followed by at least one space, or by nothing when the comment
  is empty: `fmt` adds a missing space and never removes one, so that indented text in a comment
  is kept. An end-of-line comment is preceded by one space. Comments are never rewrapped. No line
  ends with a space, even in a comment: `//   ` becomes `//`.
- Line endings and the end of the file: section 2.

**The examples are canonical**: the complete code blocks of this document and the files of
`examples/` and `docs/rfc/`. `python tools/lcf-poc/lcf.py fmt --check --spec
docs/language-spec-v0.md examples docs/rfc` checks the layout of all of them, and the order of
lines in the files.
Reference listings, such as the types of 3.2 and the signatures of appendix A, are not files and
keep their alignment.

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

- The template is written on the node: it has no terminal, and only its fields do. In a file it is a literal, so a
  wire or a named constant there is an error. It is checked at edit time: each field has exactly one named argument,
  and each named argument has its field (R-12). Otherwise, an error with a suggestion. A field written twice,
  `{a} and {a:.3}`, is one terminal, and each place keeps its format.
- A field is `{name}` or `{name:format}`, and `{{` and `}}` write braces. Its name is a name of the language (R-20)
  other than `template` and `after`, which the node uses: `{}` and `{0}` have no terminal to name.
- The format is `[fill align][+][#][0][width][.decimals][letter][ unit]`, each part optional:

  | Part | Meaning |
  |---|---|
  | `fill align` | `<`, `^` or `>`, after one optional character: a space, or one that shows by itself, other than a brace (`{s:*^12}`) |
  | `+` | Always a sign (`{x:+.2}`) |
  | `#` | With `x`, `X`, `o` or `b`, the prefix (`{r:#06x}` is `0x00ff`) |
  | `0` | Pads with zeros (`CH{n:02}` is `CH01`); not with an alignment |
  | `width` | Up to 1000 |
  | `.decimals` | Up to 100 |
  | `letter` | `d` integer; `x`, `X`, `o`, `b` hexadecimal, octal, binary; `f` decimals; `e`, `E` scientific (`1.234e+00`); `g` general |
  | ` unit` | The value is converted to this unit first, dimension checked (`{u:.3 mV}`), and the space may go when nothing comes before it (`{u:mV}`) |

- This is the form of C++20, with a name and a unit, so habits from `printf` carry over. **One difference:** `.3`
  with no letter counts decimals (`{x:.3}` is `1.234`, as in Rust), where C++20 and Python count significant digits;
  `.3g` counts those. A letter wins over a unit of the same spelling: `{m:g}` is the general format, `{m: g}` grams.
  **A second difference:** a letter before `^` starts a unit, so `{a:m^2}` is in square metres, and a value cannot be
  centred with a letter (`{s:x^12}` is an unknown unit); `{s:*^12}` centres.
  Refused, since they say nothing: `0` with an alignment, `#` without `x`, `X`, `o` or `b`, `+`, decimals or a unit with
  `x`, `X`, `o` or `b`, decimals or a unit with `d`. So `%.3d` of `printf` has no equivalent. A width from a wire (`{x:w$}`) is refused too.
- A value with a unit is displayed with its unit (`1.234 V`). A field shows any value that has a text: numbers, `string`,
  `bool`, `bytes` (in hexadecimal), `duration`, `timestamp`, enumerations, `error`, and arrays, structs and waveforms of those. A format applies to each value
  inside: `{values:.3 mV}` shows each element in mV, and for a waveform it applies to `samples`. An opaque type, a
  function reference, a `queue` or a `ref` have no text, even inside a struct, and are rejected on the wire. So is a format
  that does not suit the value: decimals, `+`, `0` or a letter on what holds no number, `d`, `x`, `X`, `o` or `b` on what is
  not an integer without a unit, and a unit that does not convert each number inside. `{n:.3}` on an integer is `5.000`. A
  constant on a field keeps its own type (`a: 255` is an `i32`, so `{a:x}` is accepted). The text of each type is fixed with
  `run`, in M0b.
- `string.format` cannot fail and has no `err` input or output. A field may be named `err`, as `"failed: {err}"`: it is
  a field like another, so the node runs and shows the error, which is when that text is useful (7.2). It does not pass the
  error on: that wire goes on separately.
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
    true => yield Verdict.Pass
    false => yield Verdict.Fail
}
```

Selector: boolean, integer without a unit (values and ranges: `1..=5` includes 5, `0..5` excludes it; bounds can
be negative, `-5..=5`), string, enumeration. A variant with data binds one name per value:
`Reading.Value(v) => …`, `Range.Between(low, high) => …`. Each branch ends with exactly one
`yield`, last, with the same number and the same types of values (R-7). `_` stands for "all the
others", wherever it is written. Every value must have a branch (R-8): an enumeration is handled
completely, a boolean has `true` and `false`, and an integer or a string has `_`. Overlapping literal
or range patterns are an error (R-18). A range holds at least one value: `5..=1` and `0..0` are errors.
Every number written in a pattern fits the type of the selector, an excluded bound too: on a `u8`, `0..256`
is refused, and `0..=255` says it.
A `_` that can catch nothing, on a boolean or an enumeration whose values all have their pattern, gives a
warning: the program stays valid, since a library that removes a variant can leave a caller in this state.

A branch can list several patterns, separated by commas, on the line of its `=>`:

```text
voltage = match channel {
    1, _ => yield 5 V
    2 => yield 12 V
    3..=5, 8 => yield 24 V
}
```

`_` can be one of them, so any branch can be the default one and still say which values it was
written for. A `match` has one `_` and names a variant once, in one branch or in two, even by two names
of one type. A pattern that
binds names stands alone in its branch: these names would have no value when another pattern of the
branch matches. A variant that holds data binds it, so it stands alone too. An arm of a `handle` has
one pattern (7.3).

### 6.2 For loop: `for`

```text
total, scaled = for x in readings, carry acc = 0 V {
    s = core.mul(x, 2.0)
    next acc = core.add(acc, x)
    collect s
}
```

- `for x in array`: one iteration per element (LabVIEW's auto-indexing), one level at a time: `x` is an
  `[f64]` for an `[f64; 2]`. `collect` adds a level: a `collect` of `[f64]` gives an `[[f64]]`, which is not
  an `[f64; 2]`. `for i in 0..n`: `i` goes from 0 to n − 1; its bounds are integers without a unit, and `i`
  is the smallest type that both widen to.
- **Several `in` clauses** (`for x in values, i in 0..n`), of any kind, advance together and stop
  at the shortest. This is also how an iteration counter is written. The `in` clauses come before
  the carries, and a `for` has at least one.
- `carry name = initial value`: a carry (LabVIEW's shift register). `next name = …` sets the value
  for the next iteration; the final value is an output of the loop.
- **The type of a carry** is on its line alone: the type written there, or else that of its initial value.
  `carry n = 0` is an `i32`, whatever the body computes. `next` is converted to it as on any wire (3.3,
  3.4), and what cannot be converted is rejected: `next n = core.add(n, 0.5)` asks for
  `carry n: f64 = 0`. `[]` needs a written type.
- **The clauses read from outside the loop**, even a name that a carry then hides (R-3). One clause does
  not see another: `for r in rows, c in r` and `carry b = a` after `carry a = 0` are rejected.
- `collect value`: builds an array of the values of each iteration, as output. A constant takes the element type of the
  output or the carry that the array goes to (3.4); a wire keeps its type, so an `i32` wire is an `[i32]`, which an output
  `[f64]` rejects.
- **Where they go**: `next`, `collect` and `until` appear only directly in a loop body, not in a
  function body and not inside an arm of a `match` or `handle` of the body (R-6). Each carry has
  exactly one `next`. A `loop` has exactly one `until`; a `for` has none.
- Outputs, in order: the `carry` values, then the `collect` values. Several `collect` give their
  outputs in their written order, which the canonical form keeps (5.2).
- **A loop that gives nothing**, with neither a carry nor a `collect`, is written with `_` as its only
  target: `_ = for x in values { … }`. A structure always has a target (14.1).
- A node output can be bound directly to a carry: `next state, next index = match …` avoids one
  `next x = …` line per carry.
- Parallel iterations: only on explicit request (`for parallel x in …`), and rejected if there is a
  `carry`.

### 6.3 While loop: `loop … until`

```text
count, e = loop carry n = 0, carry err = e0 {
    done = core.ge(n, 10)
    _, e5 = instrument.poll(dev, err: err)
    next err = e5
    next n = core.add(n, 1)
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
| `map<K, V>`, `set<T>` | dictionary and set, with their primitives (Q-28) | M2 |
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

Categories are hierarchical. A library declares its own in its manifest. A program does not read these
fields as a path in v0: `e.code` is rejected, and `handle` is how an error is told apart (7.3).

### 7.2 Automatic propagation

- A node that can fail has an `err` input (default `none`) and an `err` output.
- **A native function** (a standard-library primitive, or an `extern` function, 4.1) whose `err`
  input holds an error, which excludes a field named `err` of `string.format` (5.4), is **skipped**: the node does not run, its outputs take their default value
  and the error is passed on unchanged.
- **An output that has the name of an input passes it through** when the node is skipped or fails,
  instead of taking its default value. This is what lets a session pass through (5.3): after a
  failed measurement, `visa.close` still receives the session and can close it. Drivers therefore
  name their session output like their input.
- Exception: a native function marked `always` (in the manifest of its library) runs anyway
  (closing a session, switching off a power supply) and passes on the error it received in
  preference to its own. `always` can only mark native functions.
- **`core.call` skips as the function it runs would**, by the `err` input of that function, whatever names the
  function type gives: `fn(duration, error)` and `fn(duration, err: error)` behave alike.
- **A function written in LabCraft (`.lcf`), from any library, always runs its body**, like a
  LabVIEW subVI: only its native nodes are skipped, each according to its own `err` input, and its
  `always` nodes run. A cleanup function (`cleanup(dmm, err)` that calls `visa.close`) therefore
  closes the session even when called with an error. There is no `always fn`.
- **Default value of a type**: `false`; `0` in the unit of the type (`0 V`, `0 s` for a
  `duration`); `""`; an empty `bytes` or array; the first variant of an enumeration, with its data
  at their default values; for a struct, each field at its default value; `none` for an `error`.
  The default value of an opaque type is an invalid handle, on which the functions of its library
  fail with an error. The default value
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
    ok => yield (value, none)
    dmm.overload => yield (nan V, none) // handled: cleared
    io.timeout(err) => yield (nan V, err) // passed on
    visa.*(err) => yield (nan V, err)
    _(err) => yield (nan V, err)
}
```

- One branch per category (wildcards `visa.*`), per code (`code -1073807339`, the VISA code as is,
  with its sign), plus `ok` and `_`.
- **The most specific arm wins**: `code` > exact category > `a.b.*` > `a.*` > `_`. `a.*` matches
  every category under `a`, at any depth (`visa.*` matches `visa.io.timeout`); a category without
  `.*` matches only itself (R-18).
- **`_` is required**: categories are open, so exhaustiveness can only come from it. Without an
  `ok` branch, `_` also covers the absence of an error (its bound value is then `none`).
- **An arm is written once**: `ok`, `_`, a code or a category, with or without a bound name. `a.b`
  and `a.b.*` are two arms.
- An error that a branch does not return is **cleared**.
- Retrying = a loop around the operation, with `until` on success or on a number of attempts.
  `settle` (section 13) shows the pattern; a generic `retry` is Q-8.

### 7.4 Several errors

Error wires are explicit, so two errors meet only in a merge node: `e = errors.merge(e1, e2)`. The
first argument, in argument order, that holds an error becomes the main error; the other errors go
into its `others`. Nothing is lost. With no error among the arguments, the result is `none`.

### 7.5 An error never handled

- **At edit time**: warning if an `err` output is wired nowhere: `_` on it, a name that nothing reads,
  a name given to it (`e2 = e`) that nothing reads, or a call written alone (5.1). An order wire
  (`after: e`) is a wire. Only an output named `err` of a call counts, not any value of type `error`;
  what a `match`, a `handle` or a loop gives is not judged, since clearing an error there is written on purpose.
- **At run time**: when the top-level function returns an error on its `err` output, the run ends
  with a failure status and the error is reported (CLI, MCP, IDE). Never a dialog box.

## 8. Execution

- A node runs when all its inputs are available.
- A function call runs when all its wired inputs are available, and its outputs are available only
  at the end of the call (like a LabVIEW subVI).
- Independent branches can run in parallel; the program must not depend on an order between them.
- **Deterministic mode** (tests, agents): fixed order, simulated clock, same results at every run
  (D-12).
  - **Discrete-event clock.** Everything that takes simulated time (waits, and the latency and
    timeouts of simulated instruments) overlaps between parallel branches, as in real time: two
    branches that each wait 100 ms advance the clock by 100 ms, not 200 ms. Whether two calls on one
    resource overlap is up to its library (8.1): two calls on one `visa` session do not.
  - **Global order.** The runtime repeatedly runs, among the nodes ready at the current simulated
    time, the first one in this order: the position of a node is the list of the canonical
    positions (5.2) of the statements that contain it, from the top-level function down through
    calls, loops and arms, each loop adding its iteration number, compared element by element. A
    node that becomes ready takes its place
    in this order; it does not run before a ready node that comes first.
  - **A value set at time t**, such as a panel element set by a command (Q-20), is seen by the reads
    made at t.
  - **Noise.** Each simulated quantity draws its noise from its own random sequence: two branches
    that use different instruments get the same values whatever their order. Branches that use the
    same instrument depend on the order of their calls, hence the warning of 8.1.

### 8.1 Concurrency: values and resources

- **No races on values, by construction.** A value is produced once and never changes (5.1);
  parallel branches read it together, and a modification works on a copy (D-11).
- **Races remain possible on resources**, that is, what a value designates without containing it: a
  session (two copies of the handle designate the same connection, and the instrument has a
  state), a `ref<T>` reference, a panel element read or written live (10), a file.
- **What a call guarantees on its resource is up to its library**, which says it; the runtime adds no
  lock. `visa` makes each of its calls atomic on its session, so that `visa.query` writes then reads
  without another call stepping in ([libs/visa/README.md](../libs/visa/README.md)). Two calls that the
  program does not order, on one resource, are the program's concern: the warning below points them out.
- **Warning at edit time** when two nodes receive the same resource with no dependency path between
  them (data, error wire or `after:`), for example two branches that set the same instrument:
  "concurrent access to `dmm`: order by the error wire or `after:`", once per resource. Two nodes in
  different arms of a `match` or `handle` never run together and do not count; `bench.dmm` and
  `bench.source` are different resources, and `bench` overlaps both. A library declares in its
  manifest which of its opaque types are resources, as it marks its `always` functions there; a struct,
  a variant or an array that holds one is a resource too. An opaque type of a project is none before
  the manifests of user libraries (section 9).
- **`check` assumes nothing about what a node does.** It reads signatures, so any node that calls a
  function and receives a resource counts, even one that only seems to read: `query` can change the
  state of an instrument. A primitive or a construction only moves a value, and does not count. So the
  warning says that an order is missing, and not that it matters: two independent reads get it too.
- **What continues a resource.** A new name (`e = d`), a carry (`carry c = d`) and what a structure
  gives of it are the same resource. So is an output of the type of a resource that the node received,
  whatever the name of that output: a session wire that enters a node and leaves it is one connection.
  A node that received two gives both. What a primitive gives continues what it received. A session
  that really is another one only costs a warning, which an order removes. `check` cannot see two sessions
  opened on one instrument, whose address is known at run time, and it says that an order exists, not
  that it is the right one. A construction is a new value (`Bench { dmm: d }`, `Held.One(d)`), and a
  function continues only the type it received: an output of a struct type from a session, or a session
  from a struct or an array (`s = get(bench)`), starts a new resource. A conflict through these is not
  found. What a `next` gives counts for what the loop gives, not for what the body reads of the carry.
- **Same warning for a panel element** written by two unordered nodes. A read is no write.
- **Warnings about dataflow wait for the errors of their function**, signature included, since a wire
  with a mistake in its name reads nothing, and that would look like a wire missing.
- **Same warning for a `for parallel` body** that uses a resource from outside, or writes a panel
  element, once per loop. A resource born in the body is its own, since each iteration has one, and so
  is the element of an `in` clause, for that loop only: it stays what the array holds for the other
  warnings, so two loops on one array have no order between them, and a loop around that one shares the array.
- **Deterministic mode does not show races**, since it fixes an order (D-12). Hence a check at edit
  time rather than at run time.
- **`ref<T>` is not read then written in two nodes**, which would open a race between the two: it
  is modified in an access structure that locks it during its body (syntax for M2, Q-19).

### 8.2 Stopping a run

Besides its normal end, a run stops in one of three ways: Stop, planned by the program; Abort,
requested from outside; a fault, found by the runtime.

- **Stop, planned by the program.** The program reads a panel element, a `switch` when several
  loops must stop (section 10), and its loops end on it: `stop = ui.get(stop_switch, after: …)`,
  then `until stop`, or `halt = core.or(done, stop)` and `until halt`. After the press, what runs is what the diagram
  says: the loops end their iteration and exit, the nodes downstream run, `visa.close` closes the
  sessions, a supply is switched off if the program does it. The run ends normally, with or
  without an error on `err`. A user presses Stop on the panel; an agent, the CLI or a test sets it
  by a command (Q-20). This is the only clean stop: a program that must leave its bench in a safe
  state does it in its diagram, on Stop. A `for` has no `until` (R-6): a loop that must stop on
  Stop is written as a `loop` with an index carry (`next i = core.add(i, 1)`, `until halt`);
  leaving a `for` early is Q-22.
- **Abort, the last resort of the runtime.** Requested from outside: Ctrl-C on the CLI, an MCP
  command, a time limit on the run. No node starts any more, `always` nodes included: an abort has
  no exception and runs nothing of the diagram. Waits (`time.wait`) end at once. A native call in
  progress ends or reaches its own timeout; one that has none, or still runs after a grace period
  of the runtime, is abandoned and its resource is reported as leaked. The grace period counts on the
  clock of the run, simulated in deterministic mode, so that an abort by a time limit replays exactly:
  the run ends when the last call in progress ends, or when the grace period ends if one is abandoned.
  The runtime then releases the other resources the run opened, each through its library (`visa` closes
  its sessions), without running a node. The run
  ends with an `aborted` status, the time of the abort and where each loop was, and no output: the
  function did not finish. The report warns that the instruments are in an unknown state: closing
  a session does not switch a supply off.
- **A fault, found by the runtime.** A fault is not an `error` value that a wire carries: it is a
  failure of the run itself, such as a limit reached (below), a value the runtime cannot represent,
  or a bug of the runtime. It stops the run like an abort: no node starts any more, the resources
  are released, no output is reported. The report gives the `fault` status, a code and the node
  where it happened.

**Limits.** The depth of calls is always limited, to protect the stack: a function may call itself
(with a warning at edit time), and a run that goes too deep ends with a fault. So is the memory that
the values of a run hold at once: a run that would pass it ends with a fault, rather than the process
running out of memory, in real time too. By default, the number of iterations of a `loop` and of nodes
executed is limited only in deterministic mode (a `for` ends with its array): there it protects a test or
an agent from a program that never ends, including one whose simulated clock no longer advances. For the
same reason a deterministic run is limited in real time, since a computation does not advance the
simulated clock; that limit alone depends on the speed of the machine. In real time a loop runs as long as
the program says, for days on a bench, unless the run is given a limit. The clock of a run has an end, far
beyond any run, and a wait that would pass it is a fault. The values of the limits are those of `run`
([cli.md](cli.md), section 6.2). Which other run-time failures are faults rather than `error` values (integer
overflow, `core.to_i32` of `nan`, integer division by zero) is Q-25.

**Status of a run**, in every report (CLI, MCP, IDE): `ok`; `error` when the `err` output of the
top-level function holds an error (7.5); `aborted`; `fault`.

Stop matches the Boolean that a LabVIEW program reads to end its loops, and Abort the Abort
Execution button of the LabVIEW toolbar, which runs no cleanup of the diagram either. In
deterministic mode, a press of Stop (Q-20) and a time limit are dated on the simulated clock, so a
test replays them exactly.

**Panel elements belong to their call** (10): a loop in a subfunction does not see the
`stop_switch` of its caller, and must receive what it needs to stop. How a Stop reaches the loops
of subfunctions is Q-24.

## 9. What M0 implements

This section is the reference for the content of milestone M0; the [roadmap](roadmap.md) points
to it.

**Language.** Everything this document describes, except:

| Part | Milestone |
|---|---|
| Generic functions and types written by the user (3.5); the built-in primitives are generic from M0 | M2 |
| Running external functions (4.1), except `extern rust`, which runs from M0; they are all read and checked from M0 | M8 |
| The `panel` section beyond its elements, their kind and their `default:` (widgets, other properties) | M3 |
| `ref<T>` and its access structure (8.1, Q-19) | M2 |
| Manifests (1, D-25): the libraries shipped with LabCraft declare their categories, `always` functions, resources and dependencies (`scpi` uses `visa`) from M0; manifests of user libraries, versions and their dependencies | M2 |
| Structures planned outside v0: events, queues, notifiers, maps and sets, timed loops, cyclic tasks (6.4) | M2 and later |

**Libraries.**

| Library | Content | Milestone |
|---|---|---|
| `core` | arithmetic, comparisons, logic, `select`, conversions, units, `call` (function references, section 4) | M0 |
| `string` | `concat` (variadic), `join`, `format` (5.4), `split`, number parsing (`to_f64`, `to_i32`) | M0 |
| `array` | size, index (`len`, `get`, `set`), subarray (`slice`), construction (`push`, `concat`, `repeat`), rows (`from_rows`, `to_rows`), `sum` | M0 |
| `errors` | `merge`, error creation (`make`, `raise_if`) | M0 |
| `visa` | `open`, `close`, `write`, `read`, `query` (text). Native (`extern rust`), shipped in `libs/visa`, with its simulator: the `SIM::` resources answer as the instruments described in TOML files (D-17). Real instruments in M4 | M0 |
| `scpi` | `query_f64`, `check_errors` (`SYST:ERR?`). Written in LabCraft over `visa`, shipped in `libs/scpi` | M0 |
| `time` | `now`, `wait`, `elapsed` | M0 |
| `ui` | `get`, `set`: panel elements read and written live (section 10) | M0 |
| `config` | reading a configuration file into a declared type (Q-15) | M2 |

`visa` and `scpi` are not part of the language (section 1): they are libraries shipped with
LabCraft, outside the crates of the tool ([roadmap](roadmap.md)). The signatures of the functions
this document uses are in appendix A.

## 10. Panel (`panel` section)

Widgets are detailed for M3. The model is fixed from v0, because it affects semantics: a loop that
reads a Stop button, a graph updated at each iteration.

The section lists the elements and their kind: `control resource: text_box`,
`indicator verdict: led`, `indicator voltage: numeric(format: "0.000")`. Their placement on the
panel will go in the `layout` section; its syntax comes with M3 (section 9). In M0 the kinds are
`button`, `switch`, `numeric`, `text_box`, `led` and `chart`; another kind gives a warning, and only
`default:` is checked among the arguments. How an element looks (colours, scales) is the choice of the developer: those
arguments come with M3, and `check` never judges them.

**The type of an element.** An element has one type, which the first of these gives:

1. The input or the output of its name; of both, a `control` the input and an `indicator` the output.
2. Its kind, when that kind holds one type: `bool` for a `button` or a `switch`, `string` for a `text_box`.
3. Its `default:`, with its own type (3.4).

A wire written to such an element converts to that type as on a terminal (3.3, 3.4): `default: 1 V` says volts, and wires of
`f64[V]` and `f64[mV]` both go there. With `default: 0.5`, a wire that has a unit is an error on the default, and `0.5 V` says what
is meant; with `default: 0`, so is a wire of `f32` or `f64`, and `0.0` says what is meant. `ui.get` and `ui.set` give the type of the
element.

**An element that the diagram reads says its type** in one of these three ways: a `ui.get` on any other is an error on the element,
since a value is read before anything is written. So an accumulator, which reads an element and writes back what it computes, has a
`default:`.

An element that the diagram never reads takes its type from what its `ui.set` nodes write, and each of them gives what it was given.
The diagram has no order, so none comes first: wires of different types are an error on the element, `f64[V]` and `f64[mV]` included,
since nothing says which unit it shows. The developer converts one of them, or gives a `default:`. A constant takes the type of the
wires, as on a terminal (3.4, 3.5); with no wire, constants have their own type and must agree. With no `ui.set`, it is an error on the
element. A kind that M0 does not know takes any type, and with none of these its type stays unknown, with only the warning about the
kind. A type written on a call, `ui.get<T>`, is checked against a type that the element says, and does not give it one. On an element
that its wires give a type to, it is the type of that value, as on a generic primitive (3.5).

Each kind shows these types. A kind that does not show the type of the input or the output of its name is an error on the
element; a wire that it refuses is an error on that wire, and a `default:` an error on the default.

| Kind | Shows |
|---|---|
| `numeric` | a number, with or without a unit, a complex number, a `duration` |
| `text_box` | a `string` |
| `button`, `switch` | a `bool` |
| `led` | a `bool` or an enumeration, by the name of the variant |
| `chart` | a number, which it adds to its history, an array of numbers (`[f64]`, `[f64; 2]`, not `[[f64]]`), a `waveform` |

**One element, one name.** An element holds its whole value, whatever its type: `ui.set(limits, value)` writes the struct, and
`ui.set(limits.high, x)` is refused. A field is read after `ui.get`, as on any wire. An array or a struct goes through the same rules. Two values that live apart are two
elements, and what arranges them on the panel (tabs, groups) goes in the `layout` section, with no effect on the program.

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
when order matters, it goes through `after:` (5.3).

Two kinds of element hold a boolean that the user presses:

- a **`switch`** keeps its value until it is changed again, by the user, a command or `ui.set`. It
  is the element that stops several loops: each one reads it;
- a **`button`** is latching: a press stays true until its first read, then returns to false, so
  that it is not missed between two iterations. Only one read sees a press: two `ui.get` of the
  same button give a warning at edit time ("only one of these reads sees a press: use a switch"),
  once per button. Two reads in different arms of a `match` or `handle` never run together and do
  not count.

When a press takes effect (on press, on release, while pressed), which LabVIEW's mechanical actions
also distinguish, is left to M3, with a real panel.

```text
labcraft 1

use core
use scpi
use time
use ui
use visa

/// Displays the voltage continuously until the operator presses Stop.
fn monitor(resource: string, period: f64[s] = 200 ms) -> (count: i32, err: error) {
    dmm, e1 = visa.open(resource, timeout: 2 s)
    count, e4 = loop carry n = 0, carry e = e1 {
        raw, _, e2 = scpi.query_f64(dmm, "MEAS:VOLT:DC?", err: e)
        e3 = time.wait(period, err: e2)
        volts = core.with_unit(raw, [V])
        shown = ui.set(live_voltage, volts)
        stop = ui.get(stop_button, after: shown)
        next e = e3
        next n = core.add(n, 1)
        until stop
    }
    #close: e5 = visa.close(dmm, err: e4)
    err = e5
}

panel {
    control resource: text_box
    control period: numeric
    control stop_button: button
    indicator count: numeric
    indicator live_voltage: chart(history: 500)
}
```

`resource` and `period` are read once; `count` is displayed at the end. `stop_button` and
`live_voltage` exist only live. To change the period along the way, it would be read with `ui.get`
in the loop.

**Without a user interface.** The panel state exists as soon as the function runs, displayed or
not: called as a subfunction, by the CLI, by MCP or by an agent. `ui.get` then returns the default
value of the element, or the one a command set during execution (an agent "presses" Stop). The
default value of an element is that of its type (7.2), unless its input has one in the signature, or the panel item gives one:
`control threshold: numeric(default: 0.5)`, a constant that an argument can hold (a number, a text, a boolean, a variant without
values, an array of those), written once. Only an element that exists
only live has a `default:`. An input has its default in its signature, or none: `default:` on one is an error, or it would be
optional on the panel and required for who calls the function. An output has none, since a value shown before it is produced
would read as a result. Writes by `ui.set` are recorded in the run trace. In
deterministic mode, values set by a command are dated on the simulated clock: a test replays
exactly the press of Stop at 5 s (command syntax: Q-20).

**Concurrency.** Reading an element is always safe; writing it from two unordered nodes gives a
warning (8.1).

**Events (M2).** The `events` structure will react to changes of the same elements, without
polling in a loop.

## 11. Layout (`layout` section)

```text
layout {
    node #close at (580, 80)
    node dmm at (40, 80) view standard
    node reading at (220, 80)
    node verdict at (400, 80)
    wire reading -> verdict via (300, 80) (300, 120)
}
```

- **Coordinates** are in canvas units: 1 unit = 1 logical pixel at 100% zoom, origin at the top
  left, y pointing down (visual notation, section 2). `node x at (40, 80)` places the top-left
  corner of the node; the points after `via` are waypoints of the route.
- Nodes are designated by their label (`#close`) if they have one, or by the name of their line
  (5.2): their first output that is neither `_` nor `next x`. A node that has no such output (all
  its outputs are `_` or `next x`, or a call statement) needs a label to be designated here. A named
  constant (5.1) is designated by its name. A line that gives a value a new name (`err = e3`, 5.1, R-4)
  designates nothing. A node or a wire has at most one entry.
- `wire a -> b` designates a wire by **the value it carries**, `a`, the name that the diagram writes on
  the wire, and **the node it goes to**, `b`, designated as above: `wire dmm2 -> #close` and
  `wire e2 -> #close` in `check_voltage` (section 12), `wire count -> mean` for
  `mean = core.div(total, count)`. A value is any name that a body binds: an input, a line, a carry, an
  `in` clause, a pattern; `x = y` gives the wire of `y` a second name. An order wire starts at its value,
  or at the label that `after:` names: `wire #settle -> second`. A node whose output has no name
  starts its wire at its label, which the IDE adds to keep a route: `wire #inc -> n.k` for
  `#inc: next k = core.add(k, 1)`. The start has no terminal: `wire raw.session -> …` names no wire.
- When `a` goes to `b` by several wires, `b.in` names the input: the name of a parameter of its
  signature (`wire x -> sq.b` for `sq = core.mul(x, x)`), a field of a construction, `after` for an order
  wire. On a structure, an input is named by its carry or its `in` clause, or by the name of the outer
  value that crosses its edge: `wire k -> total.sum` and `wire k -> total.k` for
  `total = for v in values, carry sum = k { … }` with `k` read inside. These names never meet, since a
  carry or a clause with the name of an outer value hides it (R-3). Some inputs have no name: the
  selector of a `match` or a `handle`, a value of a variant, an element of an array, and inside,
  `until`, `collect` and `yield`. With several wires, an entry without a terminal designates the one
  that reaches an input without a name. `wire stop -> n` is the
  part inside, from the notch to `until`, and `wire stop -> n.stop` the part outside, for
  `n = loop … { … until stop }`. The outputs of a structure need no name: each is a value of its own
  (`total`, `count`).
- **A value that is also the selector** enters the structure by the selector, which serves as its
  notch: `match c { … }` with `c` read in an arm draws one wire of `c` outside. `wire c -> y.c` gives
  it a notch of its own, as the developer draws it; without that entry, it goes by the selector. An
  agent, which never writes this section, gets the path by the selector.
- **A wire that enters a structure has one entry per frame**, each with its own route: outside, from
  the value to the structure that holds its reader (`wire k -> flight_time` in `projectile`), and
  inside, from the notch on the edge to the node that reads it (`wire k -> ks`). Inside, the value
  keeps its name. Several readers inside share the outer part. From inside, a wire to `next`, `until`,
  `collect` or `yield` ends at the structure itself, `next x` at its input `x`.
- An input or an output of the function is a terminal of its frame, not a node (visual notation,
  section 2). An input is a value, so a `wire` entry can start there, and it can end at an output:
  `wire e3 -> err`. A name that is both a node and an output designates the one that the value goes
  to. A `node` entry cannot name an input or an output, and gets the warning below.
- An entry designates the wires that match it. None is a warning: `LC5004` when a name designates
  nothing (no such value, node or input), `LC5006` when both exist and no wire joins them. Several
  are a warning too (`LC5006`), and naming the input settles it, except in three cases that keep an
  automatic route in v0: a value that is the selector and is also yielded as it is by an arm, one
  value yielded by two arms, and `next s = s0` when `s0` also gives `s` its first value. One value
  that reaches a node twice through inputs without a name (`[x, x]`, a variadic parameter) counts as
  one wire. Two entries for one wire are an error (`LC5007`).
- Wires without a route are routed automatically.
- **View of a node**: compact by default, standard on request (visual notation, section 3).
  `view standard` or `view compact` sets the view of a node, alone or after its position. The IDE
  preference "everything in standard" is not in the file.
- Section missing or incomplete: the IDE places the missing nodes. An agent never writes this
  section.
- Renaming a value updates the section (IDE and CLI command). An entry that names a node that
  does not exist, for example after an agent renamed a value in the text, gives a warning at edit
  time; `fmt` removes it, unless a body of the function cannot be ordered (5.2) or a comment is
  attached to the entry. `fmt` reads one file, so it keeps the name of any line whose value is more than
  one plain name: `k = Verdict.Pass` can be placed while `k = step.low` reads a field and draws no node, and `check`
  tells them apart. `fmt` keeps a `node` entry that names a node, an input or an output of the
  function, and a `wire` entry that starts at a name that the function binds or a label, with no
  terminal, and ends at a node, an input or an output; `check` judges it, whatever its kind or direction. A name defined in
  several bodies (two arms) is ambiguous for a `node` entry, and gives a warning: use a label. For a
  `wire` entry, only the bodies where a wire joins it to `b` count.
- Coordinates are integers between -1,000,000 and 1,000,000 *(range proposed)*; a value outside
  is an error at edit time.

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
  down, like the canvas. It is written `32`, as the header is written `1`.
- **Shapes**: `rect` (two corners, optional radius), `circle` (center, radius), `line`, `path` (SVG
  path syntax: `M L H V C S Q T A Z`, absolute or relative), `text` (`at`: middle of the baseline), set in the font of the visual notation, Hack, at its `size`
  (notation 2.3), 10 when it is not written.
  Fixed stroke width chosen by the IDE; `fill` fills the shape. Numbers are decimals without a unit;
  a radius or a size is above zero. The radius of a `rect` larger than half a side is reduced to it,
  as in SVG.
- **Leaving the grid** is a warning: what a shape draws, stroke aside, goes below 0 or above 32. A
  curve or an arc counts by what it draws, not by its control points; a circle by its center and
  radius; a text by its box: as wide as its cells (notation 2.3) times 1233/2048 of its `size`,
  centered on `at`, from 1493/2048 of its `size` above the baseline (the top of the capitals) to
  483/2048 below it.
- **Color**: monochrome, the ink color of the theme and of the node state (skipped, failed).
  `accent` gives a shape an accent color, designated by a theme token (`accent wire.error`), never
  by a value: a token of the visual notation, section 8, whose parts are names (`frame.fill.1` is not
  one).
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

File `Verdict.lct`:

```text
labcraft 1

/// Result of a check. `Error`: the measurement did not take place (upstream error).
type Verdict = enum { Pass, Fail, Error }
```

File `check_voltage.lcf`:

```text
labcraft 1

use core
use scpi
use visa

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
    voltage = core.with_unit(raw, [V])
    above = core.ge(voltage, low)
    below = core.le(voltage, high)
    in_range = core.and(above, below)
    measured = match in_range {
        true => yield Verdict.Pass
        false => yield Verdict.Fail
    }
    verdict = handle e2 {
        ok => yield measured
        _ => yield Verdict.Error
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
python tools/lcf-poc/lcf.py run examples check_voltage --sim libs/visa/sim/dmm-230V.toml \
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
    route: string, // name of a route in the relay table
    voltage: f64[V],
    low: f64[V],
    high: f64[V],
}

// In the `results_log` library, which writes it: named `results_log.StepResult` elsewhere.
type StepResult = struct { name: string, value: f64[V], passed: bool }

// `relays.bank` and `psu.session` are opaque types that their libraries declare as aliases of
// `visa.session` (`type bank = visa.session` in `relays`): `scpi` functions accept them.
type Bench = struct { dmm: visa.session, relays: relays.bank, source: psu.session }
```

The state machine, `station.lcf` (`bench_selftest` and `wait_dut` are in `examples/`):

```text
labcraft 1

use array
use core
use results_log

/// Self-test, then for each of the `dut_count` products: wait, run the steps until the first failure, report.
fn station(bench: Bench, steps: [Step], dut_count: i32 = 1, err: error = none) -> (err: error) {
    n = array.len(steps)
    _, _, _, _, err_out = loop
        carry state = State.Init,
        carry index = 0,
        carry results: [results_log.StepResult] = [],
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
                s = core.select(present, State.RunStep, State.WaitDut)
                yield (s, 0, [], tested, e2, false)
            }
            State.RunStep => {
                i2 = core.add(index, 1)
                more = core.lt(i2, n)
                step, e5 = array.get(steps, index, err: err)
                r, e3 = run_step(bench, step, err: e5)
                all = array.push(results, r)
                go_on = core.and(more, r.passed)
                s = core.select(go_on, State.RunStep, State.Report)
                yield (s, i2, all, tested, e3, false)
            }
            State.Report => {
                e4 = results_log.append_csv(results, err: err)
                t2 = core.add(tested, 1)
                again = core.lt(t2, dut_count)
                s = core.select(again, State.WaitDut, State.Stop)
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

use core
use psu
use relays
use results_log

/// Switch, set, wait for the reading to settle, measure, compare to the limits.
fn run_step(
    bench: Bench,
    step: Step,
    err: error = none,
) -> (result: results_log.StepResult, err: error) {
    _, e1 = relays.apply(bench.relays, step.route, err: err)
    _, e2 = psu.set_voltage(bench.source, step.voltage, err: e1)
    v, e3 = settle(bench.dmm, target: step.voltage, tolerance: 0.2 V, period: 50 ms, timeout: 1 s, err: e2)
    above = core.ge(v, step.low)
    below = core.le(v, step.high)
    passed = core.and(above, below)
    result = results_log.StepResult { name: step.name, value: v, passed: passed }
    err = e3
}
```

Measure until settled, `settle.lcf`:

```text
labcraft 1

use core
use dmm_driver
use errors
use time
use visa

/// Measures every `period` until within `tolerance` of `target`, or io.timeout error.
fn settle(
    dmm: visa.session,
    target: f64[V],
    tolerance: f64[V],
    period: duration,
    timeout: duration,
    err: error = none,
) -> (value: f64[V], err: error) {
    t0 = time.now(after: err)
    value, err_out = loop carry v = 0 V, carry e = err {
        e1 = time.wait(period, err: e)
        t = time.elapsed(t0)
        late = core.gt(t, timeout)
        x, _, e2 = dmm_driver.read_dc_volts(dmm, err: e1)
        gap = core.sub(x, target)
        dist = core.abs(gap)
        stable = core.le(dist, tolerance)
        done = core.or(stable, late)
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
IDENT      = letter { letter | mark | digit | "_" } ;   (* not a reserved word *)
letter     = a Unicode letter (a-z, A-Z, µ, Ω…) ;       (* never "_" or "°" *)
mark       = a Unicode combining mark ;                 (* é as e + U+0301: one name (R-20) *)
digit      = "0" | … | "9" ;
INT        = digit { digit | "_" } ;
FLOAT      = INT "." INT [ ( "e" | "E" ) [ "-" ] INT ] | INT ( "e" | "E" ) [ "-" ] INT ;
                                                        (* 1_000.5_0 is 1000.50, with no unit *)
STRING     = '"' { char | "\\" escape } '"' ;           (* char: not '"', "\" or a line ending;
                                                           escape: \" \\ \n \t \r *)
UNIT_NAME  = [ "°" ] letter { letter | mark | digit | "_" } ;
                                                        (* checked against the unit table:
                                                           V, mV, A, Ω, kΩ, Hz, s, ms, °C, K… *)
COMMENT    = "//" { line_char } ;                       (* not "///"; ignored; line_char: anything
                                                           but a line ending, up to it or the end
                                                           of the file *)
DOC        = "///" { line_char } ;                      (* documentation, before the declaration *)
```

**Invisible characters.** A code point with the Unicode property `Default_Ignorable_Code_Point` is
neither a `letter` nor a `mark`, whatever its category, so no name or unit name can hold one: U+3164
alone, or `a` followed by U+FE0F or U+034F, is a syntax error (`LC1001`). Such a character shows as
nothing, so a name could otherwise hold text that no reader sees. Strings and comments may hold them.

**Version of Unicode.** Names are read with Unicode 18.0: what a `letter` and a `mark` are, the property
`Default_Ignorable_Code_Point` and the normalization of R-20. The comparison of names without their case
(R-22) follows the standard library of Rust that the tool is built with, Unicode 17.0 today. A character
assigned after these versions can be read otherwise by an older tool; one assigned before cannot, since
Unicode keeps the normalization and the category of an assigned character.

Reserved words, everywhere: `labcraft use fn extern type struct enum match for parallel loop
continue_on_error carry next collect until yield handle events panel layout icon true false none nan inf`.
`events` is reserved for the event structure of M2 (6.4), whose syntax is still open (Q-11).
`panel`, `layout` and `icon` only start a section, but stay reserved.

A reserved word where a name is expected is `LC1005`, including as a target (`next = 1`) or the name of
an argument (`f(true: 1)`). A `yield`, `next`, `collect` or `until` out of place, or a structure without a
target (`for i in a {}` alone), is a syntax error, `LC1001`.

Contextual words, reserved only where the syntax expects them and ordinary identifiers elsewhere
(`e.code` reads a field of an error, `ok` can name a value):

- `ok`, `code`: a `handle` pattern;
- `in`: a `for` clause;
- `after`: an argument list (`after:` is the order input, R-15);
- `opaque`: the whole type of a `type` declaration (3.2);
- `control`, `indicator`: a `panel` item;
- `node`, `wire`, `at`, `via`, `view`, `standard`, `compact`: a `layout` item;
- `rect`, `circle`, `line`, `path`, `text`, `at`, `radius`, `size`, `fill`, `accent`: an `icon` item.

**New words.** From the first release of LabCraft, a version published with its program, a structure or
a declaration that comes later takes a contextual word wherever its place allows one, so that a program
of `labcraft 1` keeps its names: for example `timed loop` or `cyclic task`
(6.4), a test function marked `test fn`, a `break` alone on its line (Q-22), the locked access to a
`ref<T>` (Q-19). A word
reserved everywhere, or a name refused to types (3.2), comes only with a new version of the format
(`labcraft 2`) and its automatic migration, which renames what bears it in the files migrated. A
primitive library added after `core`, `string`, `array`, `errors`, `time` and `ui` (appendix A), such
as the queues of M2, gives way to a folder of the project of that name, as a library shipped with
LabCraft does (`docs/cli.md`, section 3): a project keeps its own `queue` library, and renames it only
to use the new one. Until that release, no program of `labcraft 1` exists outside this repository: a
change that breaks one is allowed, says so in its pull request, and migrates the files of the repository
in the same pull request. The words reserved and the names refused before it are chosen so that later
milestones break as little as they can.

**Line endings** end a statement, an arm, a `use`, and a `panel`, `layout` or `icon` item. They are
ignored inside `( )` and `[ ]`, inside the braces of a `struct` or `enum` type or of a struct
literal, in a structure header up to its `{`, in a declaration up to the `{` of its body or up to its
end, and in the header of a section up to its `{`: `panel` and its `{` can be on two lines, and
`f64` followed on a later line by `[V]` is one type in a `type` declaration. Two statements never share a
line, and a declaration or a section ends its line: nothing but a comment follows it. A `}` may close the
line of a statement, an arm or an item (`{ x = 1 }`). The productions below do not show line endings; this
rule applies on top of them.

**Nesting** has one limit (`--max-nesting`, 256 by default, `LC0007`) and one counter for all its kinds.
These count for 1 each: a `match`, `handle`, `for` or `loop`; a parenthesis of a unit; an array, struct,
enum or function type, or the `<` of a named type; the `[` of an array literal. The brackets around a unit
add nothing (`f64[V]`, `core.with_unit(x, [V])`), and neither do the arms of a `match` or a `handle`: 256 nested
`match` pass, and the 257th gives `LC0007`.

### 14.2 Files and declarations

```ebnf
file          = header { use_decl } { DOC } ( fn_decl | extern_decl | type_decl )
                [ panel_section ] [ layout_section ] [ icon_section ] ;
header        = "labcraft" INT ;
use_decl      = "use" path ;
path          = IDENT { "." IDENT } ;

type_decl     = "type" IDENT [ generics ] "=" type_expr ;
fn_decl       = fn_sig block ;
extern_decl   = "extern" IDENT "(" [ arg { "," arg } ] ")" fn_sig ;   (* 4.1 *)
fn_sig        = "fn" IDENT [ generics ]
                "(" [ param { "," param } [ "," ] ] ")"
                "->" "(" [ output { "," output } [ "," ] ] ")" ;
generics      = "<" generic { "," generic } ">" ;
generic       = IDENT [ ":" path ] ;     (* T: Number; T is a whole type, unit included (3.5).
                                            A variable for a unit alone, f64[U]: Q-12 *)
param         = IDENT ":" type_expr ( [ "=" constant ] | "..." ) ;  (* "..." : variadic (5.4) *)
constant      = call | struct_lit | operand ;           (* a default value; which of them are
                                                            constants is for R-21 *)
output        = IDENT ":" type_expr ;
```

**The header stands alone on the first line.** Only spaces and tabs go before it: a blank line or a comment
there is `LC0101`. Only an end-of-line `//` comment goes after it. Any other text on the line is `LC0101`:
`labcraft 1 x`, a `///` comment, and even what could not be read as a token (`labcraft 1 $`, a string left
open). Extra spaces and tabs are tolerated, and `fmt` removes them.

**The version is the exact text `1`.** Any other integer (`01`, `1_0`, `0`, `99999`) is `LC0102`, whose
message says to write `labcraft 1` for `01`. What is not an integer (`1.5`, `x`, nothing) is `LC0101`.

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
unit_arg      = "[" unit_expr "]" ;                     (* core.with_unit(x, [V]) (R-5) *)
operand       = path | literal ;                        (* path: value, field, variant, category *)
struct_lit    = path "{" field_init { "," field_init } [ "," ] "}" ;
field_init    = IDENT ":" operand ;

literal       = number [ unit_expr ] | STRING | "true" | "false" | "none"
              | "[" [ operand { "," operand } ] "]" ;
number        = [ "-" ] ( INT | FLOAT | "inf" ) | "nan" ;   (* no space after the minus *)
```

### 14.5 Structures

```ebnf
structure     = match_expr | for_expr | loop_expr | handle_expr ;

match_expr    = "match" operand "{" { match_arm } "}" ;
match_arm     = ( bound_pattern | pattern { "," pattern } ) "=>" arm_body ;
bound_pattern = path "(" IDENT { "," IDENT } ")" ;                      (* Reading.Value(v) *)
pattern       = literal | [ "-" ] INT ".." [ "=" ] [ "-" ] INT           (* -5..=5 *)
              | path | "_" ;

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
terminal_ref   = node_ref [ "." IDENT ] ;                (* wire x -> sq.b; a start has no "." (11) *)
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
| R-1 | **A path is resolved by names**: library (`visa.open`), variant (`State.Init`), field (`step.low`), error category (`io.timeout` in `errors.raise_if`) or panel element (`stop_button` in `ui.get`: a parameter of kind *element* names a panel element, even if an input or an output has the same name; section 10). Unit names are resolved only inside `[ ]` and right after a number (R-5). In a function, a value cannot have the name of a library that its file imports (`a` for `use a.b`), of a type of its library (except an opaque type or an alias of one, 3.2; the base types of 3.1 do not count, as in `time.wait(duration: duration)`), or of a panel element that is neither an input nor an output. A function or a type cannot have the name of a library that any file of its library imports (`a` for `use a.b` here too), since a type is visible in all of them: otherwise `core.Pass` in one file would depend on another file. A value can have the name of a function: as an operand the name designates the value, and in a call the function. In a path, the longest library imported comes first: with `use a.T`, `a.T.V` is `V` in that library, even if `a` has a type `T`. |
| R-2 | **An output is bound exactly once and can be read back** (`voltage` in section 12), with the type of its signature. **Exception, a name that is both input and output** (`err`): reading it designates the input; the binding `err = …` designates the output. |
| R-3 | **A sub-body can hide an outer name**: a carry (`carry err = err`), an `in` clause, a pattern or a line of an arm or a loop body. In that body, the name designates what it binds there, and outside it the outer name. None of these can take a name that R-1 refuses. In the body of the function nothing is hidden: a line with the name of an output binds that output (R-2), and a line with the name of an input alone is a value defined twice. |
| R-4 | **Binding an output, `next x = …` or `collect` does not create a node**: they are terminals (function output, carry, output array). |
| R-5 | **Units**: a name right after a number is always read as a unit (`230 V`, `50 ms`); elsewhere unit names are ordinary identifiers. A unit passed as an argument is written in brackets (`core.with_unit(x, [V])`, `core.unitless(v, [mV])`, `core.with_unit(x, [m/s])`); `[x]` is read as a unit for a parameter of kind *unit*, as an array of one value otherwise; so is `[1]`, since `1` is also a unit term. In that array, `x` can name a function of the library when no value has that name, as in `[f, f]`. Elsewhere than as an argument, `[x]` and `[1]` are arrays. |
| R-6 | **`next`, `collect` and `until` appear only directly in a loop body**, never in a function body or in an arm (grammar). Each carry has exactly one `next`; a `loop` has exactly one `until` and a `for` none (grammar); several `collect` give the outputs in their order. An arm ends with exactly one `yield` (grammar). The place of `next`, `collect` and `until` in the body is free; the canonical form puts them last (5.2). |
| R-7 | **The type of the branches** of a `match` or a `handle`: same number of values, and the first branch decides the type of each. The other branches are converted to it as on any wire (3.3, 3.4): to its unit, or widened; what cannot be converted is rejected, so the order of branches can matter. A literal takes the number type of its target, an output or a carry, as on a line of its own (3.4): `gain: f32` with `gain = match c { … yield 0.5 … yield 1.5 }`. After a branch that decides, it takes the type of that branch. A literal that does not decide (an integer without a unit, `[]`, an array of only those) comes after the values that do, so it takes the type of the first branch that gives one; if none does, it takes the type of its target (`x: u8` with `x = match c { … yield 1 … yield 2 }`); without such a target, an integer is an `i32` and `[]` is an error. A wire keeps its type against the target, and so does an array that holds one (3.4), and the wire to the target is then judged as any other. |
| R-8 | **A `match` has a branch for every value.** Without `_`, an enumeration names all its variants and a boolean both `true` and `false`. An integer or a string always has `_`, even when ranges cover the whole type. `_` can share a branch with other patterns (6.1). A `_` that can catch nothing, with every variant named or both `true` and `false`, gives a warning. |
| R-9 | **No nesting of calls** (Q-5): an argument is a name, a path, a literal or a unit in brackets, never a call. A call anywhere a value is expected (an argument, `yield`, `until`, `collect`, a selector, a `carry`, an array element, a field) is `LC1004`. |
| R-10 | **A `handle` always has a `_` branch** (7.3). |
| R-11 | **One variadic parameter** per function; it follows all required parameters and precedes all parameters with a default value, which are wired by name only (5.4). Without a variadic parameter, a positional argument can wire a parameter with a default value. A variadic parameter takes positional arguments only. |
| R-12 | **The `string.format` template is a literal**, written on the node; its fields and the named arguments match exactly, and a field is `{name}` or `{name:format}` (5.4). |
| R-13 | **An external function has no body and has an output `err: error`** (4.1), with that type written as such: an alias of `error` does not do. |
| R-14 | **An icon is valid**: grid written `32` in v0, `path` strings conforming to the SVG path syntax, numbers without a unit, a radius or a size above zero, `accent` token of the visual notation (section 8); a shape that leaves the grid gives a warning (11.1). |
| R-15 | **`after:` is the order input of every call** (5.3): it takes a value, a label (`after: #beep`) or a parenthesized list of values and labels (`after: (#beep, e2)`), never a literal, and only once in a call. It counts as a named argument, so a positional one cannot follow it. A variant construction has it too. No parameter is named `after`, nor an argument of a function type. |
| R-16 | **A call alone is a statement** (`#beep: sound.beep()`, label optional): it binds none of the outputs of the node, and is meant for a function without outputs. If the function has an `err` output, the warning of 7.5 applies. A variant construction can stand alone in the same way. |
| R-17 | **A label is unique in its function** (`#close`): it designates the node in `after:` and in the `layout` section. An `after:` sees a label of its own body or of a body around it, not one inside a sub-body or a neighboring arm, whose node may not run. |
| R-18 | **Overlapping patterns**: in a `match`, overlapping literal or range patterns are an error, in one branch or in two (6.1). Numbers compare by value (`1_000` and `1000`, `-0` and `0`), and strings by their text once escapes are read, with no normalization (R-20). In a `handle`, arms may overlap and the most specific one wins: `code` > exact category > `a.b.*` > `a.*` > `_` (7.3). |
| R-19 | **A type file has no `panel`, `layout` or `icon` section**: they belong to functions (section 2). |
| R-20 | **A name or a unit has one spelling.** Names of every kind and unit names are normalized when read: Unicode NFC, then U+03BC (Greek mu) becomes U+00B5 (micro sign). So `Ω` is one unit whether written U+03A9 or U+2126, `µV` one unit whether written with the micro sign or the mu, `é` one name composed or not, and a value `μ` and a value `µ` are one name. Compatibility forms (NFKC) do not apply. `fmt` writes the normalized form. The text of strings and comments is never changed; the fields and units of a `string.format` template, and the names of files and folders compared with names of the program, are read normalized. |
| R-21 | **The default value of a parameter is a constant** (section 4): a literal; a variant, with its values if it holds some (`Reading.Value(0 V)`); a struct construction with every field given (`Limits { low: 220 V, high: 240 V }`); or the name of a function, of a library or of the primitives (`core.lt`), which matches the type of the parameter as any reference does (section 4). A value inside a variant, a field or an array is an operand (R-9), so it is a literal, a variant without values or the name of a function: a default that would put a struct or a variant with values inside another cannot be written. A default reads nothing of the diagram: a value, a call of a function, `after:` or a type argument is rejected. It has the type of its parameter, with the conversions of a wire (3.3, 3.4): `v: f64[V] = 200 mV`, `t: duration = 5 s`. No constant is a `timestamp`, so a `waveform` cannot be a default. |
| R-22 | **A file bears the name of what it declares, and that name can name a file on every system.** `N.lcf` declares `fn N`, `N.lct` declares `type N`: the two names are compared after normalization (R-20), and case counts. A file of another name is an error (`LC2023`), on that file alone: a call still finds the function by the name it declares, and a copy of a function or a type in a file of another name is no second definition (`LC2013`) of the one of the same kind in its own file. Its fixes rename the declaration as the file, or the file as the declaration, each only when that name is free: a fix never leads to another error on the name (`LC2013`, `LC2014`, `LC2022` or the two below). Two functions, or two types, of one library whose names differ only by their case or their spelling cannot be two files side by side on Windows or macOS (`LC2024`); a function and a type can, since their extensions differ (`fn step`, `type Step`). A name that Windows keeps for a device (`con`, `prn`, `aux`, `nul`, `com1` to `com9`, `lpt1` to `lpt9`, whatever their case) is refused (`LC2025`): Git for Windows refuses a file or a folder of that name, extension or not, so a project that has one cannot be cloned there. The folders given to `check` and their sub-folders follow the last two rules. In the editor, renaming a function or a type renames its file: these errors come from a file copied or renamed outside it (Q-26). |

## 15. Open questions

| # | Question | Direction |
|---|---|---|
| Q-1 | File extensions | **Settled (2026-11-01): `.lcf` / `.lct`.** `.lct` is free in practice. `.lcf` already exists in niches: ArchiCAD libraries, CodeWarrior linker command files (NXP), Anritsu VectorStar network analyzer calibration kits. Conflict accepted: a LabCraft file is recognized by its first line, `labcraft 1` |
| Q-2 | `duration` distinct from `f64[s]`? | Yes for readability, implicit conversion between the two. **Settled in part (2026-11-08):** a whole number of nanoseconds, rounded to the nearest from an `f64[s]`, ties to even (3.1) |
| Q-3 | Temperatures and offset units | Rule 3.3; check the case of averaging temperatures |
| Q-4 | `i64` to `f64` | Explicit, since precision can be lost |
| Q-5 | Allow nesting `core.add(core.mul(a, b), c)`? | **Settled (2026-10-30): no in v0.** One node per line; can be added later without breaking files |
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
| Q-16 | Default values that flow like data | Writing rule in v0 (7.2: a verdict depends on `err`). Seen with the proof of concept (`docs/rfc/check_power.lcf`, run against `libs/visa/sim/psu-timeout.toml`): when the current readback of a power check times out, the verdict is `Error`, as it should be, but the `power` output is `0 W`, a value that looks measured. Options: (a) keep default values and the writing rule; (b) propagate the absence of a value (a node that consumes the output of a skipped node is skipped in turn, and an output reads "no value"); (c) an optional type (`f64[W]?`) for outputs that may be missing. Topic for an RFC |
| Q-17 | Spread an array into a variadic parameter? | No in v0: call the function that takes an array (`string.join`). Spread syntax possible later |
| Q-18 | Mini-language of `string.format` fields | Settled: the form of C++20, with a name and a display unit (5.4). Left open: the inverse reading `string.scan` |
| Q-19 | Locked access to `ref<T>` | Structure that locks the reference during its body (8.1); syntax for M2, with Q-11 |
| Q-20 | Setting a panel element from the CLI or MCP | **Settled (2026-11-08): `--set ELEMENT=VALUE@TIME`** on the clock of the run, simulated in deterministic mode, for the elements of the function that is run; `set` in `run.start` and, in real time, `run.set_element` in MCP ([cli.md](cli.md), section 6.4; [commands.md](commands.md), section 6.4) |
| Q-21 | Default values not covered by 7.2 | `timestamp` (the origin of the clock?), function references (a reference that fails when called?), `ref<T>` and `queue<T>` (an invalid reference, like an opaque type?) |
| Q-22 | Leaving a `for` early | A `for` has no `until` (R-6), so it cannot end on Stop or on a condition; a `loop` with an index carry does it today (8.2). An `until` in `for`, or a `break`? |
| Q-23 | Unicode normalization | **Settled (2026-11-04): one spelling (R-20).** NFC alone does not unify `µ` (U+00B5, U+03BC), hence the explicit rule; NFKC would go too far by rewriting compatibility characters |
| Q-24 | Stopping the loops of a subfunction | Panel elements belong to their call (8.2, 10): a loop in a subfunction (`wait_dut`, `settle`) does not see the caller's Stop. Pass a reference to the element, a stop token, or let a subfunction read its caller's panel? |
| Q-25 | Fault or error at run time | Integer overflow, `core.to_i32` of `nan` or out of range: a fault (8.2), an `error` value on `err`, or a default value? |
| Q-26 | Name of a function and name of its file | **Settled (2026-11-08): the file bears the name of what it declares (R-22).** Option A of this question: the declaration keeps its name in the file and the format does not change; a renamed copy has one error, `LC2023`, with two fixes, since only the user knows which name was meant. Option B, the name of the file alone, was set aside: an excerpt or a diff would no longer say its name, and a file renamed outside the editor would silently break every call. A type is named exactly as a function is (`Verdict.lct`): no rule turns `StepResult` into `step_result`. Names that file systems would confuse or refuse are errors on every system (`LC2024`, `LC2025`) |
| Q-27 | From `[[f64]]` to `[f64; 2]` | Settled: `array.from_rows` and `array.to_rows` (appendix A). A constant takes the type expected if its rows have one length; an array of wires never does, so nothing fails at run time without an `err` (3.1). |
| Q-28 | Dictionary and set | Names settled (2026-11-08): `map<K, V>` and `set<T>`, refused to types until M2 (3.2), the names of LabVIEW, Rust, Java, Go and C++ rather than `dict`. To write for M2, with the generics: the types a key can have (an `f64`? a unit?); an order of iteration that is deterministic (order of insertion, or sorted keys); `get` of a missing key, with an error category as for `array.get`; the cost of a copy on a wire (D-11); how a map is drawn and how a literal is written |
| Q-29 | Constants shared outside a function | Nothing declares a constant outside a function today: `limit = 240 V` lives in one body (5.1), so neither `math.pi` nor a limit common to several functions of a project can be written. **Option A (preferred):** a declaration of its own, one per file as for a type (R-22), named by a path (`math.pi`) and drawn as a constant, with no node; its values are those of R-21, with their units; it needs a file kind or a contextual word at the head of its file (14.1, new words). **Option B:** a function with no input (`math.pi()`), with no change to the grammar, but a node on the diagram for a fixed value. To settle with how the viewer of M0v draws a named constant that feeds several nodes |
| Q-30 | Modes of a `button` | In M0 a `button` is latching: a press stays true until its first read (10). Other modes are likely (true while held, a press seen on release…), and with them what setting a button to `false` from a command does; until then a command can only press it ([cli.md](cli.md), section 6.4) |

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
fn core.add<N: Number>(a: N, b: N) -> (result: N)               // same dimension; unit of `a` (3.3)
fn core.sub<N: Number>(a: N, b: N) -> (result: N)               // same dimension; unit of `a`
fn core.mul<A: Number, B: Number>(a: A, b: B) -> (result: A·B)  // exponents add
fn core.div<A: Number, B: Number>(a: A, b: B) -> (result: A/B)  // exponents subtract; integers give f64 (3.4); x/0: inf or nan
fn core.abs<N: Number>(x: N) -> (result: N)
fn core.neg<N: Number>(x: N) -> (result: N)
fn core.sqrt<N: Number>(x: N) -> (result: √N)                   // exponents halved; rejected if odd; integers give f64
fn core.pow<N: Number>(x: N, exponent: i32) -> (result: N^exponent)   // constant exponent; integers give f64; not in the PoC
fn core.min<N: Number>(a: N, b: N) -> (result: N)               // same dimension
fn core.max<N: Number>(a: N, b: N) -> (result: N)               // same dimension

fn core.lt<N: Number>(a: N, b: N) -> (result: bool)             // same for le, gt, ge; no complex numbers
fn core.eq<T>(a: T, b: T) -> (result: bool)                     // same for ne
fn core.and(a: bool, b: bool) -> (result: bool)                 // same for or
fn core.not(a: bool) -> (result: bool)
fn core.select<T>(condition: bool, if_true: T, if_false: T) -> (result: T)

fn core.with_unit(x: f64, unit: kind unit) -> (result: f64[unit])     // `core.with_unit(raw, [V])`
fn core.unitless<N: Number>(x: N, unit: kind unit) -> (result: f64)   // unit of the dimension of `x`
fn core.to_i32(x: f64) -> (result: i32)                         // narrowing (3.4); not in the PoC
fn core.round_to_i32(x: f64) -> (result: i32)                   // not in the PoC
fn core.to_f32(x: f64) -> (result: f32)                         // not in the PoC
```

`core.call` runs a function reference (section 4); its inputs after the first and its outputs are those
of the referenced function. Not in the PoC.

```text
fn core.call(function: fn(…) -> (…), …) -> (…)
```

### A.2 `array`, `string`, `time`, `errors`, `ui`

```text
fn array.len<T>(array: [T]) -> (length: i32)
fn array.get<T>(array: [T], index: i32, err: error = none) -> (item: T, err: error)
fn array.set<T>(array: [T], index: i32, item: T,
                err: error = none) -> (array: [T], err: error)   // not in the PoC
fn array.push<T>(array: [T], item: T) -> (array: [T])
fn array.concat<T>(parts: [T]...) -> (array: [T])          // 5.4; not in the PoC
fn array.repeat<T>(item: T, count: i32,
                   err: error = none) -> (array: [T], err: error)   // not in the PoC
fn array.slice<T>(array: [T], start: i32 = 0, length: i32 = …,
                  err: error = none) -> (array: [T], err: error)    // not in the PoC
fn array.from_rows<T>(rows: [[T]], err: error = none) -> (array: [T; 2], err: error)   // not in the PoC
fn array.to_rows<T>(array: [T; 2]) -> (rows: [[T]])        // not in the PoC
fn array.sum<N: Number>(values: [N]) -> (sum: N)           // not in the PoC

fn string.concat(parts: string...) -> (text: string)       // 5.4; not in the PoC
fn string.join(parts: [string]) -> (text: string)          // not in the PoC
fn string.format(template: string, …) -> (text: string)    // an input per field (5.4)

fn time.now() -> (timestamp: timestamp)                    // order it with `after:`
fn time.elapsed(since: timestamp) -> (duration: duration)
fn time.wait(duration: duration, err: error = none) -> (err: error)

fn string.split(text: string, separator: string) -> (parts: [string])
fn string.to_f64(text: string, err: error = none) -> (value: f64, err: error)
fn string.to_i32(text: string, err: error = none) -> (value: i32, err: error)

fn errors.make(category: kind category, code: i32 = 0,
               message: string = "") -> (err: error)
fn errors.raise_if(condition: bool, category: kind category, message: string = "",
                   err: error = none) -> (err: error)
fn errors.merge(errs: error...) -> (err: error)            // 7.4; the PoC takes up to six

fn ui.get<T>(element: kind element) -> (value: T)          // T: type of the element, section 10
fn ui.set<T>(element: kind element, value: T) -> (value: T)   // same; never read, T: what this node is given
```

`array.get`, `array.set` and `array.slice` fail with the category `array.index_out_of_range`. An error of
`array.set` gives the array unchanged. `array.slice` goes from `start` for `length` elements: with no wire, `start`
is 0 and `length` reaches the end, and a negative value or what goes past the end is an error, which gives an empty array. It takes
whole rows, so that a slice of an `[f64; 2]` stays an `[f64; 2]`. `array.repeat` gives `count` copies, none at 0, and
fails on a negative `count` with `array.negative_size`, giving an empty array. `array.from_rows` fails with `array.unequal_rows` on rows of
different lengths, giving an empty array, and its message says which row, with both lengths; no row at all gives an
empty array. `array.concat`, `array.push` and `array.set` on rows give an `[[f64]]`, since nothing keeps their
lengths equal, and nothing converts between the arrays of `array.concat` (3.4). With no input, `array.concat` needs its type from
what its line binds (3.5) or a type argument. `string.to_f64` and
`string.to_i32` fail with `string.parse`. Both ignore the spaces around the text: `to_f64` reads an
optional sign, digits with decimals, an exponent, or `nan` and `inf` in any case; `to_i32` reads an
optional sign and digits, in the range of `i32`. `errors.make` produces an error of the given category and
code; `errors.raise_if` produces an error of the given category when `condition` is true, `none`
otherwise.

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

The `visa` functions are declared with `extern rust` in `libs/visa` (4.1), and its manifest marks
`visa.close` as `always`. `scpi` is written in LabCraft over `visa`, in `libs/scpi`:
`query_f64` is a `visa.query` followed by `string.to_f64`. A `scpi` call is therefore atomic only
per `visa` call (8.1). The proof of concept reads these files; its native bodies stand in for the
crate of `visa`.

### A.4 Libraries of the examples

`dmm_driver`, `relays`, `psu` and `results_log` belong to the examples (section 13), not to the
libraries shipped with LabCraft. They are sub-folders of `examples/`: `dmm_driver`, `relays` and
`psu` written in LabCraft over `visa` and `scpi`, like real drivers; `results_log`, which writes a
file, as an `extern rust` function, whose native body in the proof of concept keeps the results in
memory.

```text
fn dmm_driver.read_dc_volts(session: visa.session,
                            err: error = none) -> (volts: f64[V], session: visa.session, err: error)
fn relays.apply(bank: relays.bank, route: string,
                err: error = none) -> (bank: relays.bank, err: error)
fn psu.set_voltage(source: psu.session, voltage: f64[V],
                   err: error = none) -> (source: psu.session, err: error)
fn results_log.append_csv(results: [results_log.StepResult], err: error = none) -> (err: error)
```

The libraries `instrument` (6.3), `sound` and `lamp` (5.3) are illustrative and have no signatures
here. The signatures of `config` (Q-15) and `sim` (section 9) are not written yet.
