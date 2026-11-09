# The dataflow IR

*Last updated: 2026-11-09. Status: implemented for milestone M0b (the crate `ir`, step 2 of architecture 10.11). Version 1 of the IR.*

The `ir` crate (L4, [roadmap](roadmap.md)) holds a program as a dataflow graph, after `check` has resolved and
typed it and before anything runs it: the second of the three levels of D-09. It is the form that the
interpreter of M0b runs, that the trace and the viewer refer to, that the debugger of M5 puts its probes on, and
that the linear IR of M7 is lowered from, before Cranelift. It will outlive most of the code around it. This
document defines it, for whoever writes or reads the crate. [architecture.md](architecture.md), section 10, says
how it fits with the other crates.

The design follows the regionalized value state dependence graph (RVSDG) of the literature (Reissmann et al.,
*RVSDG: An Intermediate Representation for Optimizing Compilers*, 2020): nodes compute, edges carry values, and
structures hold **regions** of their own. It keeps two things of the language that a compiler IR would lose:
**the nodes of the diagram**, one per line that the diagram draws, and **the origin** of everything in the source,
since a user debugs the diagram, not the IR.

## 1. What it is, and is not

- **Versioned and documented** (D-09): this document is its definition, and `ir::VERSION` its version, raised by
  any change of section 9. Tools and agents will read it through its text form (section 8): a command that prints
  it is left for later (section 10).
- **Run directly in M0.** D-10 has the interpreter run the linear IR, which comes with Cranelift in M7. Until then
  the interpreter runs this IR, where the trace and the debugger speak in nodes and wires; the corpus of runs
  checks that a later interpreter gives the same results ([decisions](decisions.md), D-10).
- **Not a file format.** Nothing writes it to disk to read it back: a program is lowered again from its files.
- **Not a second checker.** It is made only from a program that `check` accepted: every type is known, every name
  resolved. Lowering that meets what `check` should have refused is a bug of LabCraft (`LC8004`), not a diagnostic
  for the user.
- **Not the layout.** Positions on the canvas, comments, `layout` and `icon` stay in `graph` (D-07).

## 2. A program

`ir::lower` takes the `Checked` of a run and the function to run, and gives a `Program`: that function and every
function it can reach (what it calls, what those call, the functions named as references), each lowered once. It
walks the calls with an explicit work list, never by recursion (architecture, section 1).

```rust
pub struct Program {
    pub version: u32,                      // `ir::VERSION`
    pub functions: Vec<Function>,          // by FunctionId, dense; [0] is the one the run starts
    pub constants: Vec<Constant>,          // by ConstId, shared by all functions, each one once
    pub types: TypeTable,                  // from `check`: every TyId below is in it
    pub definitions: HashMap<TyId, Option<TyId>>,  // what each type of a `.lct` file stands for; None: opaque
}
pub struct Function {
    pub key: FunctionKey,                  // library (as `check` numbers it) and name
    pub library: LibraryId,                // its library in the table of types
    pub shown: Box<str>,                   // as reports and the trace name it: `scpi.query_f64`
    pub file: u32,                         // the file it was read from, by its index in `Checked::files`
    pub inputs: Vec<Port>,                 // the signature, in order: name, type, default (ConstId) if any
    pub outputs: Vec<Port>,                // an output's default is that of its type (spec 7.2)
    pub elements: Vec<Element>,            // its panel (spec 10), by ElementId
    pub body: Body,                        // its region, or the native function an `extern` declaration names
}
pub enum Body { Region(Region), Native { library: LibraryId, function: Name, runs: bool, skip: Skip } }
```

- **An `extern` function** (spec 4.1) has a native body: `function` is what its `function:` argument names, and `runs`
  is true for an `extern rust` function, whose library gives the body, and false for the others, whose call gives
  `extern.unavailable` on `err` in M0; `skip` is what a call of it does with an error (section 5), in the places of its
  own inputs, which a `core.call` of it applies.
- **An element** has its name, its kind (`switch`, `button`, `numeric`…), its type as `check` settled it, its
  default value, and the input or the output it is the element of, if any. An element that no node reads or writes, and
  that nothing else gives a type, has none, and no default. It belongs to its call: each call of the
  function has its own state of it (spec 8.2, 10), which the runtime keeps, not the IR.

**What lowering reads from `check`**: the bodies of every function the run reaches, those of `scpi` and of a
`--lib-path` library included; what each path, call and bracket means, the type of each value and the coercions on
each wire (`Resolved`); the manifests (`always`, `resources`); the defaults of inputs and outputs (spec 7.2); the
table of primitive signatures; the canonical order of every body, found in one walk (architecture, section 4).
`check` gives all of this through what section 10.1 of the architecture calls its interface for runs.

## 3. Regions, nodes, values

```rust
pub struct Region {
    pub origin: StatementId,               // the declaration of its function, the statement of its structure, or its arm
    pub args: Vec<Value>,                  // the values that enter it, typed
    pub nodes: Vec<Node>,                  // in canonical order
    pub results: Vec<Use>,                 // the values that leave it
}
pub struct Node {
    pub kind: NodeKind,
    pub inputs: Vec<Use>,                  // in the order of section 4
    pub outputs: Vec<Value>,               // one per output of what it computes
    pub after: Vec<Order>,                 // what it waits for without reading it
    pub position: u32,                     // the canonical index of its line in its body (spec 5.2)
    pub origin: StatementId,               // its line
    pub name: Name,                        // as the trace names it: first output, label, or `@line` (commands.md, 8)
    pub skip: Skip,                        // what an error on its `err` input does (section 5)
}
pub struct Value { pub ty: TyId, pub names: Vec<Name> }        // the names it bears: the first, then each new name
pub struct Use {
    pub source: Source,
    pub fields: Vec<u32>,                  // a path of fields read on the way, by index in each struct
    pub convert: Option<Conversion>,       // the conversion of the wire, if any (spec 3.3, 3.4)
    pub ty: TyId,                          // what it delivers, after its fields and its conversion
    pub origin: OperandId,                 // the operand it was written as, or the line that binds a result
}
pub enum Source { Arg(u32), Out { node: u32, output: u32 }, Const(ConstId), Array(Vec<Use>) }
pub enum Order { Value(Use), Node(u32) }   // a value of this region, or a node of it by its label
```

- **A region holds one body**: the body of a function, of a loop, of a `for`, or of one arm of a `match` or a
  `handle`.
- **Its nodes are the nodes of the diagram.** A line is a node when its right side is a call, a structure, a
  construction, an array written of wires (`r = [x, y]`, a node that receives its wires: spec 5.1), or a constant
  (a named constant, `limit = 240 V`, drawn as a small node): visual notation, 3. A call
  written alone (`time.wait(50 ms)`) is a node whose outputs no target names. **A line makes no node** when it only
  names what exists (R-4, notation 3.3): a new name for a wire (`e = d`), a field read (`k = step.low`), the binding
  of an output (`err = e3`) or of a carry (`next n = m`), `collect` and `until`. Such a line adds a name to a
  value, or a use with its fields, or a result of the region (section 4). Its line keeps its canonical position,
  so the positions of the nodes of a region increase but may skip.
- **A value is defined once** (spec 5.1): by an argument of its region or by an output of a node of that region.
  A `Use` names it by `Arg(i)` or `Out { node, output }`, both within the same region. **A region reads nothing from
  outside except through its arguments**: a structure passes in, as inputs of its node, the values of the
  enclosing region that its body reads (the "context variables" of RVSDG). So a region can be run, copied per
  iteration, shown, or lowered on its own.
- **A constant** is `Source::Const`, evaluated once at lowering by the rules of spec 3.4: a number in the type and
  unit of the use it reaches, read once in its kind (an `f32` is not read through an `f64`), then converted as a wire
  would convert it; a text, its escapes read; a boolean, `none`, a variant, an array of constants, a struct (a default,
  R-21), a function reference: a `FunctionId`, or a primitive with the `Skip` of a call of it, by its function type
  (section 5). Two more have no value of their own: **the `length` of `array.slice` with no wire is the
  constant `…`, which reaches the end of the array** (decision of the owner, 2026-11-09: the diagram shows `…` on the
  terminal, as appendix A writes it), and **a default that Q-21 has not settled** (a `timestamp`, a function reference)
  is `Unsettled`. The default of an opaque type is `InvalidHandle` (7.2). **A literal takes the type of each use** (3.4): the line `k = 0.5` is a node whose output holds the
  literal, and each use of `k` is the constant evaluated for the type it reaches, so `k` can feed an `f32` and an
  `f64`.
- **An array written of wires as an argument** (`string.join([a, b])`, and `[x]` for a parameter of kind value,
  R-5) is `Source::Array`: a use of uses, which builds the array on the wire, as the diagram draws it. On its own line
  (`r = [x, y]`) it is a node, `BuildArray`.
- **A use carries what the wire does on the way**: a path of fields (`bench.dmm` reads field `dmm`), then the
  conversion `check` recorded for it (`mV` to `V`, `i32` to `f64`), done as spec 3.3 says. They stay on the use,
  where the viewer shows them. A value bound to an output of another unit (`voltage = raw_mv` for an `f64[V]`)
  keeps its own type on its node, and what reads it under the name of the output reads it converted.
- **A conversion** is data, so that the interpreter, the WASM build and Cranelift do the same operations (D-10):

  ```rust
  pub enum Conversion {
      Number { to: NumberKind, scale: Scale },        // to the kind first, then the scale, in that kind
      ToDuration { scale: Scale },                    // to f64[s], then × 10^9, nearest, ties to even (3.1)
      FromDuration { to: NumberKind, scale: Scale },  // nanoseconds as an f64, ÷ 10^9, then the scale
  }
  pub struct Scale { pub add: Option<f64>, pub multiply: Option<f64>, pub divide: Option<f64>, pub subtract: Option<f64> }
  ```

  A scale is the steps of spec 3.3, each one operation: the offset of a temperature it leaves, the numerator and the
  denominator of the ratio of the two scales (`types::Unit::scale_ratio`, each the nearest `f64` of its integer), then
  the offset of the temperature it goes to. A step that does nothing is `None`, since adding zero would change `-0.0`;
  the same unit takes none. The steps run in `f64`, and a value of `f32` is rounded once at the end (spec 3.3).
  `ir::apply_scale` does them, for lowering and for the runtime.
- **`after:`** is an `Order`: a value whose definition the node waits for without reading it, or a node by its
  label (`#close`). It adds no input. **An order on something outside the region** becomes an order of the
  structure node that contains the node, region by region up to the region of what it waits for, as a value read
  from outside becomes an input of that structure.

## 4. Kinds of node

The inputs of a node that calls follow the signature: one per parameter, in order, a default made explicit as a
constant; **a variadic parameter** takes its arguments as consecutive inputs, and the node records where they start
and how many there are; **free terminals** come last: the fields of a `string.format` template, in the order of the
fields of the template, and the arguments of `core.call` after the function, in the order of the signature of the
function it calls.

| Kind | Inputs | Outputs | Regions |
|---|---|---|---|
| `Constant` | one constant | one | |
| `Primitive { library, name, statics, variadic }` | as above | its outputs | |
| `Native { function: FunctionId, variadic }` | as above, for an `extern` function (section 2) | its outputs | |
| `Call { function: FunctionId, variadic }` | as above, for a function written in LabCraft | its outputs | |
| `Construct { ty, variant }` | the fields of a struct or a `waveform` (`t0`, `dt`, `samples`), in declaration order, or the values of a variant | one | |
| `BuildArray` | one per item, in order | one | |
| `Loop { continue_on_error, carries, collects, region }` | *c* initial carries, then *k* values from outside | *c* final carries, then one array per `collect` | one |
| `For { parallel, clauses, carries, collects, region }` | one array per `in` clause (`Each`), or two bounds for a range (`Range`); then *c* initial carries; then *k* values from outside | as a `Loop` | one |
| `Match { arms }` | the selector, then *k* values from outside | the values of the `yield`, in order | one per arm |
| `Handle { arms }` | the error, then *k* values from outside | as a `Match` | one per arm |

- **`statics`** are the arguments of kind *unit*, *category* and *element* (R-1, R-5): `[V]`, `io.timeout`,
  `stop_switch` as an `ElementId`, and the template of `string.format`, its escapes read. They are data of the node,
  never wires: `ui.get(stop_switch)` reads no wire named `stop_switch`, and the inputs of `string.format` are the fields
  of its template only.
- **A line that makes a node of a value**: a named constant (`limit = 240 V`) or a function as a value (`g = scale`) is
  a `Constant` node; a variant without values on its own line (`s = State.Init`) is a `Construct`.
- **The region of a `Loop`** has *c* + *k* arguments: the current value of each carry, then the values passed in.
  Its results are the next value of each carry (what its one `next x = …` names: spec 6.2), the condition of its
  one `until`, then each `collect`. The node runs its region until the
  condition is true, or a carry of type `error` holds an error (Q-7) unless `continue_on_error`.
- **The region of a `For`** has one argument per `in` clause (its element, or the index of a range), then *c*
  carries, then *k* values passed in; its results are the next carries, then the collects. `parallel` means each
  iteration may run at its own pace (spec 6.2, 8); a parallel `for` has no carry.
- **An arm** has its patterns, as data: a literal (a constant in the type of the selector), a range, a variant by
  its index with the number of values it binds, a category (`visa.*` with `wildcard`) or a code (`handle`), `ok`, or
  `_`; several in a list (`1, 3..=5`), where none binds. The arms are stored in the order they are tried, an arm that has
  `_` last wherever it was written (spec 6.1). The region of an arm has as arguments the *b* values its pattern binds,
  then the *k* values passed in, all *k* of the structure in every arm; its results are the values of its `yield`.
- **The values passed in** (*k*) are what the regions of a structure read from outside, at any depth, in the order a
  walk of the file first reads them: a name read three structures down is an input of each of the three. A new name
  (`e = d`) and a field read (`k = step.low`) are followed to what they name first, so `step` is passed in, not `k`.

## 5. Errors on the wire

What spec 7.2 says a node does with an error is decided at lowering, so that the runtime never reads a name:

```rust
pub enum Skip {
    Never,                                  // no `err` input, a structure, a construction, a constant
    Runs,                                   // a call to a LabCraft function: it runs its body, whose own nodes skip (7.2)
    OnError { err: u32, always: bool,       // which input is `err`; `always` from the manifest (7.2)
              defaults: Vec<ConstId>,       // the default of each output (7.2)
              through: Vec<Option<u32>> },  // the output named like an input passes that input through (5.3)
    ByCallee,                               // `core.call`: the skip of the function it runs, which that function holds
}
```

`OnError` is for a primitive or an `extern` function with an input named `err` of type `error`; `always` comes from the
manifest of its library, so a primitive is never `always`. `ByCallee` is for every `core.call`: the function it runs
decides, by its own input named `err`, whatever names the function type gives (`fn(duration, error)` and
`fn(duration, err: error)` skip alike: decision of the owner, 2026-11-09).

A node `OnError` whose `err` input holds an error, and that is not `always`, does not run: its outputs take their
defaults, those in `through` take the input they pass, and its `err` output takes the error it got. An `always`
node runs, and its `err` output passes on the error it got if it had one. `ByCallee` applies, when the call
happens, the `Skip` of the function it runs, which lowering made from that function's signature: `Runs` for a function
written in LabCraft; for an `extern` function, the `skip` of its `Body::Native`; for a primitive, the `skip` of its
`ConstValue::Primitive`, made from the names of its parameters and the types its function type gives them. Those are in
the places of the function's own inputs, a variadic parameter as one array (spec 4), so on the node, whose first input is
the function, each input is one place further on. `verify` keeps `ByCallee` to a primitive whose first input is a
function and whose other inputs and outputs are those of its type; it reads no names, so it cannot tell that such a node is
`core.call` itself, nor see a `core.call` given another skip, since a generic primitive such as `array.repeat` can take a
function first too. This is all the runtime needs to know of 7.2.

## 6. Origins and positions

- **Every node knows its line** (`origin`: the `Id` of its statement in the file of its function), every use the
  operand it was written as, every region the structure or arm it is. So a fault, a trace event or a probe is
  reported at a node of the program, and the viewer finds the wire of a value.
- **The position of a node** is the index of its line in the canonical order of its body (spec 5.2). The global
  order of spec 8 is the list of positions of what contains a node, from the top-level function down through calls,
  loops and arms, each loop adding its iteration number: the runtime builds it as it runs, the IR does not store it.
- **Names** are those of the program, interned: a value bears the name of its line and each new name a line gives it
  (`err = e3`), so the trace and the debugger speak the user's words.

## 7. Invariants

`ir::verify` checks a `Program` and returns every violation. It runs after each lowering in the tests, on every function
of `examples/`, `libs/` and `docs/rfc/`, and on every program the fuzzing of `run` lowers. A violation is a bug of
lowering, and so is what `ir::lower` returns as a `LowerError`: what `check` should have refused.

1. Every `FunctionId`, `ConstId`, `ElementId`, `TyId` and index named exists, and `version` is `ir::VERSION`.
2. A `Use` names an argument of its own region, an output of a node of its own region, or a constant; the items of
   a `Source::Array` follow the same rule. An `Order` names a value or a node of its own region.
3. Within a region, a node uses only outputs of nodes before it in a topological order: a region is acyclic. The
   only cycles of a program go through the carries of a loop, from the results of its region back to its arguments.
4. A node has the inputs, outputs, regions, arguments and results that section 4 gives its kind, with the same *c*,
   *k* and *b* on its two sides: a `Loop` of *c* carries and *k* values passed in has *c* + *k* inputs, *c* + *k*
   arguments, and *c* + 1 + (number of `collect`) results; a `For` has as many inputs per clause as its clause takes;
   an arm has *b* + *k* arguments, *b* being what its pattern binds.
5. The type of each use equals the type of the input it reaches; the type of each result of a region equals what the
   structure or the function expects there. No type is `Ty::Error`. The inputs of a primitive have the types its call
   settles, which only `check` knows: there the verifier checks only that they have one.
6. In each region, positions strictly increase in the order the nodes are stored, and no two nodes have one line.
7. A `Skip::OnError` names an input and an output of type `error`; each `through` names an input of the type of its
   output.
8. A parallel `for` has no carry.
9. A constant holds only constants stored before it, so the table has no cycle.

## 8. Its text form

`ir::print` writes a `Program` as text, for people, diffs and tests, and `ir::print_function` one function of it: the IR
of every function of `examples/`, `libs/` and `docs/rfc/` is printed and compared with its snapshot in
`tests/harness/snapshots/ir/`, natively and in WASM, so that a change of lowering shows in review. Uses are written by the
name of what they read, fields by their name, a conversion after `as`, constants as the language writes them (a duration
in nanoseconds), and a native body with its skip. A constant is written up to 80 bytes, the names of its types and
variants and its brackets included, and then `…` if it goes on: a text is cut between its quotes, so a `…` inside them is
a character of the text and one after them the cut, and the constant `…` of `array.slice` is the only one that is `…`
alone. Constants share what they hold, and a name can be as long as its file, so without the cut one use of a
constant could be written larger than the file; with it, the text form stays within a constant factor of the file.
`examples/check_voltage.lcf`:

```text
fn check_voltage(resource: string, low: f64[V] = 220.0 V, high: f64[V] = 240.0 V, err: error = none) -> (verdict: Verdict, voltage: f64[V], err: error)
  region (resource, low, high, err) -> (verdict, voltage, e3)
    0 dmm, e1 = native visa.open(resource, 2000000000 ns, err)  skip(err: 2)
    1 raw, dmm2, e2 = call scpi.query_f64(dmm, "MEAS:VOLT:DC?", e1)  runs
    2 e3 = native visa.close(dmm2, e2)  skip(err: 1, always)
    3 voltage = primitive core.with_unit(raw, [V])
    4 above = primitive core.ge(voltage, low)
    5 below = primitive core.le(voltage, high)
    6 in_range = primitive core.and(above, below)
    7 measured = match (in_range)
      arm true () -> (Verdict.Pass)
      arm false () -> (Verdict.Fail)
    8 verdict = handle (e2, measured)
      arm ok (measured) -> (measured)
      arm _ (measured) -> (Verdict.Error)
```

A node is named by its first output, as the trace names it; a node with none shows its label or `@12`. The form may
change with the IR: `LABCRAFT_BLESS=1 cargo test -p labcraft-harness` writes the snapshots again, and the pull request
that changes them is read with them.

## 9. Changing it

The IR will grow: generic functions (M2), queues, events and timed loops (M2 and later), `ref<T>`, probes (M5),
the linear IR beside it (M7). A change to it is one pull request that changes together:

- this document, and `ir::VERSION`;
- the verifier, so that every new rule is checked;
- the printer, and the snapshots it changes;
- the lowering tests, with one case per new construct;
- the runtime, and the corpus of runs (`tests/acceptance/runs/`, architecture, section 10.11), when what a program
  does changes.

What this document defines (the nodes of the diagram, regions that read only their arguments, origins, positions,
section 5) does not change without the owner's agreement, since the trace, the viewer and the debugger build on it.

## 10. Left for later

- **Generic functions** (M2): lowered once per instantiation, or with type parameters kept. To decide with them.
- **The linear IR** (M7): a fixed order of execution, regions flattened into blocks, values in slots, which `codegen`
  turns into Cranelift's own IR and the interpreter may then run (D-10). It is lowered from this one.
- **Analyses** (D-09: buffers, operations in place, blocks to schedule; dead branches, constant folding): none in
  M0, where the interpreter runs the program as written.
- **A command that prints the IR** of a function, for an agent or a tool (D-09).
