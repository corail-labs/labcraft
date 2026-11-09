# Architecture

*Last updated: 2026-11-08. Status: implemented for M0a (sections 1 to 9); drafts for M0b (section 10)
and M0v (section 11).*

How the Rust code is arranged. Sections 1 to 9: M0a, the crates `types`, `graph`, `format` and `check`, the app
`labcraft-cli`, and `xtask`. Section 10: M0b, the crates `ir`, `runtime`, `engine` and `mcp`, and the native crate of
`visa`. Section 11: M0v, the crates `trace`, `diagram` and `editor`, and the viewers. It is written for whoever
implements them, person or agent. The contract
to meet is elsewhere: [spec](language-spec-v0.md), [command line](cli.md),
[acceptance scenarios 2, 4, 5](acceptance-m0.md), [corpora](../tests/acceptance/README.md). The layers
are in the [roadmap](roadmap.md). The code sketches below show shapes, not final signatures.

The layout is the one shared by the Craft apps (SolidCraft, GridCraft, VectorCraft, FigCraft, DeckCraft,
WordCraft): `crates/`, `apps/<app>`, `apps/<app>-cli`, `apps/<app>-web`, `xtask/`, with a crate
`format`, then `engine`, `mcp` and `ui-egui` above it. GridCraft's `formula` crate (lexer, parser,
printer, by hand) is the nearest thing to our `format`. What the suite already provides is used, not
rewritten (section 7).

## 1. Principles

- **Only an app touches the file system.** `format` and `check` take bytes and return values, and `engine`
  reaches files through what the app gives it (section 10.8). That is what makes them run in WASM (D-22), in the
  viewer and in tests with no setup.
- **A diagnostic is a value, and failure is a diagnostic.** No function of these crates fails with
  anything else on bad input. A hostile file is an ordinary input.
- **Everything is bounded on purpose.** Depth, size, the number of files and the length of the report
  have limits (`Limits`, section 2). Algorithms stay near linear in the size of the input: 300,000
  nodes must take at most 2 s and 512 MB (scenario 4).
- **Recursion only where a limit bounds it.** The parser, and code that walks the syntax tree, may
  recurse: the nesting limit bounds them. Anything whose depth comes from the meaning of a program
  (types built up over lines, chains across files, calls between functions) is iterative or interned.
- **One model.** `fmt`, `check`, and later the runtime and the viewer all read the same `graph`.

```text
apps/labcraft-cli ──> check ──> format ──> graph ──> types
       └──────> libs/embedded (the declarations of libs/)
```

M0b adds `ir`, `runtime`, `engine` and `mcp` above `check` (section 10).

## 2. `types` (L0)

What everything needs, with no knowledge of files.

- `Name`: an identifier or unit name, always normalized (R-20): `Interner::intern` is the only way to
  make one. It is an interned `u32` from an `Interner` the caller owns: a file of 4 MiB can hold
  2 million occurrences of a name, and a pointer per occurrence would threaten the memory bound.
- `Unit`: 7 exponents in `i8`, a scale (exponents of 2, 3 and 5, which every unit of the table is exact
  in, so that equal units are equal whatever the order of the operations, on every target) and an
  offset for `°C` and `°F` (3.3). The table of named units and prefixes; multiply,
  divide and root as **checked** operations, since a chain of `mul` can overflow an exponent (`LC3016`);
  display (`V`, otherwise the expanded form: `m/s^2`, `1000*m^2` when a scale has no name). `parse_unit`
  is public: `check` also needs it for the fields of a `string.format` template. In a compound unit, a
  temperature counts as a difference: `V/°C` has no offset. Q-3 stays open on the rest.
- `Ty` and `TyId`: types are interned, and a value refers to its type by `TyId`. So `a1 = [a0]`,
  `a2 = [a1]`… cannot make a deep structure to drop, compare or print, and depth has a limit (`LC1006`).
  A type with `Ty::Error` anywhere inside matches everything, so that one error gives one diagnostic
  (cli.md, section 3). `TypeTable::matches` takes constant time, whatever the size of the types.
  A named type is identified by a `LibraryId` that `check` makes and by its name, since two libraries
  can bear one name, and it is written from where it is seen: `Verdict`, `visa.session`.
- `Diagnostic`, `Span`, `Code`, `Severity`, `Report`: the schema of cli.md, section 4, with `serde`.
  `Code` is an enumeration; each variant gives its text and its severity. `cargo xtask codes` checks
  that it equals the tables of cli.md, section 5. `Report` holds at most twice `--max-report`
  diagnostics while they arrive, with complete counts, since a file of 4 MiB can hold millions of
  errors; which ones it keeps does not depend on when it prunes. Its counts are complete for each file too
  (`counts_of`), since a run is refused for an error in a file it reaches (10.1). The app calls `finish` last, once all
  folders and `LC0009`, `LC0010` are merged.
- `Limits`: file size, total size of a library (1 GiB *proposed*), nesting, report, with their defaults.
  `max_nesting` has a hard ceiling (cli.md, section 1), lower in WASM: a stack is sized from it (section 4).
- `text`, from M0v: the font of the notation and the counting of cells (section 11.1).

Run-time values are in `runtime` (section 10.3).

## 3. `graph` (L1)

The semantic model of one file. It is **syntactic**: it holds what the file says, and resolving what
that means is the job of `check`.

```rust
pub struct File { version: u32, header_comment: Option<Comment>, use_comments: Vec<Comment>,
                  uses: Vec<Use>, decl: Decl, panel: Option<Panel>, layout: Option<Layout>,
                  icon: Option<Icon>, end: Vec<Comment> }
pub enum Decl { Function(Function), Extern(Extern), Type(TypeDecl) }  // each with its comments
pub struct Signature { generics: Vec<Generic>, params: Vec<Param>, outputs: Vec<Output> }
pub struct Body { statements: Vec<Statement>, end: Vec<Comment> }
pub struct Statement { id: Id, comments: Attached, label: Option<Label>, kind: StatementKind }
pub enum StatementKind { Binding { targets: Vec<Target>, value: Rhs }, Call(Call),
                         Collect(Operand), Until(Operand) }
pub enum Rhs { Call(Box<Call>), Match(Box<Branching>), Handle(Box<Branching>), For(Box<For>),
               Loop(Box<Loop>), Struct(Box<StructLit>), Operand(Operand) }
pub struct Arm { id: Id, comments: Attached, pattern: Pattern, others: Vec<Pattern>,
                 open: Option<Comment>, body: Body, yields: Yield }
pub enum Operand { Path(Path), Number(Id, Numeral, Option<Box<UnitExpr>>), Text(Id, Box<str>),
                   Bool(Id, bool), None(Id), Array(Id, Vec<Operand>), Bracket(Id, Box<UnitExpr>) }
```

Decisions:

- **No concrete syntax tree.** The writer regenerates everything from the model, since canonical form
  leaves nothing else free. What survives from the source is kept in the model:
  - **Numbers keep their text** (`1e3`, `1e999`) and their unit as a `UnitExpr`, with no value: `check`
    evaluates them and reports `LC1006`, while `fmt` never fails on them. The same goes for the
    dimensions of an array and for coordinates.
  - **`TypeExpr` and `UnitExpr` keep the written form** (`kg/(m*s)`, `·`, `V/A`), which `Unit` and `Ty`
    would lose. `check` turns them into `Unit` and `TyId`.
  - **`[x]` as an argument stays a `Bracket`** until `check` knows the kind of the parameter (R-5), and
    so does `[1]`, since `1` is also a unit term. `fmt` does not need to know.
  - **The text of numbers is interned**, like names, and they are ASCII, so normalizing leaves them alone:
    a file can hold millions of them.
- **Every comment has a place:** on each item that can carry one, by `Attached` (the comments before it,
  the one that ends on its line): declarations with their documentation, `use` lines, parameters,
  outputs, fields, variants, loop clauses, statements, arms, `panel`, `layout` and `icon` items, sections.
  Also after an opening `{`, on a closing `}`, and at the end of a list (`end`). The tests of 5.2 are
  written against these slots.
- **Ids instead of positions.** Everything that a diagnostic can point at has an `Id`: statements, and
  also operands, arguments, patterns, targets, yields, items. `SourceMap`, returned beside the `File`,
  maps ids to spans (`arms-that-disagree` points at a whole `yield`, and relates another). Ids are
  renumbered by `canonicalize`, and comparing two models ignores them, so scenario 5, point 3 is
  `canonicalize(read(f))` compared with `canonicalize(read(write(canonicalize(read(f)))))`.
- **Layout is separate data** (D-07): `Layout` refers to nodes by name or label, as the file does.
- The graph as edges (which output feeds which input) is not stored here. Names are the wires, and
  `check` builds what it needs. Bodies stay trees of statements until the `ir` crate of M0b ([ir.md](ir.md)).

## 4. `format` (L2)

```rust
pub fn read(bytes: &[u8], names: &mut Interner, limits: &Limits) -> Read;  // file, source map, diagnostics
// `Name` is an index with no order, and numbers are names too: sorting and printing need the interner.
pub fn canonical_order(body: &Body, signature: Option<&Signature>, names: &Interner) -> Result<Vec<usize>, Unorderable>;  // 5.2, public: M0b needs it (8); the signature, for the body of a function
pub fn canonical_orders(body: &Body, signature: &Signature, names: &Interner) -> BodyOrders;  // every body of a function, in one walk (10.1)
pub fn canonicalize(file: File, map: &SourceMap, names: &mut Interner, report: &mut Report) -> File;  // all the fixes of `fmt`; ids renumbered, so `map` no longer applies
pub fn write(file: &File, names: &Interner, limits: &Limits) -> Result<String, TooLarge>;  // prints what is stored
pub fn write_line(statement: &Statement, names: &Interner) -> String;      // `node.text` of a diagnostic
pub fn format(bytes: &[u8], names: &mut Interner, limits: &Limits) -> Formatted;  // the above, for one file
```

- **Decoding, in this order**, stopping at the first failure (cli.md, section 1): size (`LC0006`),
  UTF-8 (`LC0003`), byte order mark removed, CR LF accepted, forbidden characters (`LC0004`).
- **Lexer:** a hand-written iterator of tokens that ends at the first lexical error. Names and unit
  names become `Name`, numbers keep their text, comments and line endings are tokens. Columns count the
  code points of the source as read. A bidirectional control gives `LC0005` once per string or comment,
  and is an unexpected character elsewhere (`LC1001`, with no `LC0005`, as `encoding-bidi-in-a-name`
  expects). Warnings go straight to the file's own `Report`, which bounds them, since a file can hold
  millions. The caller names the file with `Report::set_file` before merging. A file that gets a code that
  stands alone (cli.md, section 1) gets a new report from `read`, with that code only.
- **Parser:** hand-written recursive descent on spec 14. Line endings are significant by context
  (14.1), and error messages must name what was expected for an agent to read: both are easier by hand
  than with a generator. A depth counter gives `LC0007` before the stack is in danger. It reads one token
  at a time, keeping none, and stops at the first syntax error, as the lexer does: a file that does not
  parse has no model and one error, after the warnings met before it. `Read` holds the model, the source
  map and the report of that file alone, replaced when the code stands alone.
- **The stack.** Depth is bounded by the ceiling of `max_nesting`, and `format::STACK_SIZE` is what each
  host gives. `crates/format/tests/stack.rs` measures the bytes per level at the ceiling, for each kind of
  nesting, covering parsing, `Clone`, `Eq`, `Drop`, `canonicalize` and `write`, and requires at most half of
  `STACK_SIZE`, so that it is raised before a host runs out. `labcraft-cli` and the desktop viewer run the
  work on a thread of that size.
- **The stack in WASM** is two stacks. The linked stack is ours: `.cargo/config.toml` links `STACK_SIZE`, which
  a harness test checks (a debug build traps with the default of 1 MiB, a release build passes). The call
  stack of the engine is not: a browser does not let a page enlarge it, and the web viewer works in a worker
  (section 5), which has half the stack of a page. Measured on a release build, the deepest `match` that reads
  is about 990 levels on a page and 500 in a worker with Chrome and Edge, 2,300 and 1,300 with Firefox, and
  varies with the tier of the engine; Safari was not measured. So `MAX_NESTING` is 256 in WASM, which is also
  the default everywhere: a program within the default limit reads on every host, and no option can ask a WASM
  host for what would overflow it. `cargo xtask wasm` runs the harness on 492 KiB, about what V8 gives a worker.
  That build is a debug one, which takes 300 to 400 KiB at 256 levels: this keeps the release build at about
  twice what it needs, and is not a margin for the debug build.
- **Attaching comments** happens while parsing, following 5.2. A comment inside a statement written
  over several lines goes before the statement. A comment also gets its one spelling there (a space after
  `//`, none at the end of the line), as a name does, so that the model does not change when `fmt` writes the
  file. So does `after: ([x])`: without its parentheses, it is a `Bracket`.
- **`canonicalize`** does everything else that `fmt` changes in the model: sorts `use` lines, orders bodies
  by their canonical order (a body that cannot be ordered keeps its order, with `LC6002`), orders the items
  of `panel` and `layout`, removes what may be removed (never with a comment attached), renumbers ids. Its
  warnings go to a `Report`, which bounds them.
- **`canonical_order`** uses a binary heap of ready lines: the "repeatedly emit" of 5.2 is quadratic if
  written naively. Dependencies are read from syntax alone (5.2), and one walk finds them for every body: a name
  read goes to the innermost scope that binds it, so that a file nested deep does not read its text once per level.
- **`write`** owns all of the layout of 5.2, and prints bodies in their stored order. A form that may go on one line
  is tried in the text and taken back if it does not fit, and each try stops at the width of a line. It stops
  at `max_file_size`, since a text over it could not be read again (`LC6003`).
- **Migration** of older format versions has a place here. There is only version 1 today.

## 5. `check` (L3)

```rust
pub trait Libraries {                       // how `use` finds a library: the caller decides
    fn find(&self, name: &str, by: Origin) -> Found;
    fn last_panic(&self) -> Option<String>;  // where a panic happened, which only a panic hook can see
}
pub enum Found { Library(LibrarySource), NotFound, Unreadable(Box<Diagnostic>) }  // LC2004, or LC0001
pub struct LibrarySource { name: String, shown: String, files: Vec<SourceFile>, manifest: Option<String>,
                           sub_libraries: Vec<LibrarySource>, origin: Origin,
                           report: Report,          // what its host met while gathering the files
                           checked_elsewhere: bool }  // a dependency in a folder that a run checks anyway
pub fn check(root: LibrarySource, libraries: &dyn Libraries, names: &mut Interner,
             limits: &Limits) -> Checked;
pub struct Checked { report: Report, dependencies: Vec<DependencyReport>, counts: CheckCounts,
                      files: Vec<CheckedFile>, types: TypeTable,
                      libraries: Vec<DeclaredLibrary>,             // what each library declares, by name
                      bodies: Report, for_run: bool }              // for a run (10.1)
pub struct CheckedFile { shown: String, file: File, source_map: SourceMap, resolved: Resolved,
                         library: u32, orders: Option<BodyOrders>, counts: FileCounts }
pub struct DependencyReport { shown: String, report: Report }      // merged once per `shown`
```

`Checked` is what the layers above read, so that they never resolve again: the viewer draws wires with
their types, and M0b runs from it. `Resolved` gives, by `Id`:

- what each path is (value, field, variant, category, panel element, function, unit), in a table that holds one
  small `Meaning` per `Id`, since a file can hold millions of operands;
- the kind of each `Bracket` (R-5);
- the callee of each call, and the terminals of that call, which a signature alone does not give for
  `string.format` and variadics;
- the type of each wire, and the coercions to show (3.3, 3.4); the type of what a `_` target binds, and of the outputs of
  a call written alone (`Resolved::outputs_of`), which lowering gives the node.

`root` includes its sub-libraries, which are checked even if nothing uses them (cli.md, section 3).
`Limits` bounds the total size of a library, since the caller hands all its files at once.

Passes, each one over all the files, so that a bad file does not hide the others:

1. **Load:** `format::read` on each file; the set of libraries through `Libraries`, with cycles
   (`LC2021`). A file that does not parse is marked, and what it declares is known as broken.
2. **Declarations:** functions and types per library, duplicates, R-1, R-19, bad types.
3. **Bodies:** names resolved in scopes (inputs, carries, patterns, lines), `use`, calls bound to
   signatures, labels, `after:`, the bounds of each number of a body, a default value or a `default:` (the rest of a `panel`
   comes with pass 6), `LC5012`. Only what can be told without a type: a field of a value and
   the values of a variant wait for pass 4, and so does a `string.format` template, whose fields ask something of their types (`LC2106`). Only for the
   libraries being checked: of a dependency, the others read the signatures, except for a run (10.1).
4. **Types and units:** wire by wire, in dependency order, with `Ty::Error` to stop cascades. That order is one graph for
   the whole function, walked with a queue, since a chain of lines is as long as the file. A structure is two steps, one
   before its body and one after. What is left in a cycle stays unknown, and pass 5 says why. A panel element adds nothing to that
   order (spec 10): one that is read says its type before the walk, by its name, its kind or its `default:`, and the wires of one that
   is never read are compared when the walk ends. So no wire of the diagram depends on the type of an element that its wires decide.
5. **Dataflow:** cycles, outputs bound once, carries, the values passed into structures (`LC4007`, counted in `passed_in.rs`
   as `labcraft-ir` builds them, and stopped a little past the limit), the terminals of the calls (`LC4008`, in `terminals.rs`,
   the same way); then the warnings of 7.5, 8.1 and 10.
6. **Panel, layout, icon:** kinds that M0 does not know, elements declared twice or of the wrong direction, references to nodes, coordinates, the icon (R-14: grid,
   a validator of the SVG path syntax, a table of `accent` tokens). A `wire` entry is matched against the wires of the function, one per frame (spec 11): each
   value read, with the input it reaches, and for a value from outside a structure, one more wire per edge it crosses. The entries of a function wait for its
   errors, as the warnings of pass 5 do: a name with a mistake reads nothing. In `declare/body/sections.rs`, the icon in `sections/icon.rs`.

Types across files (`.lct` chains, `LC2017`) and calls between functions (`LC2016`) are walked with an
explicit stack. A function is checked against the signatures of the others, never by following a call.

**Concurrent access (8.1)** gives one warning per resource, not one per pair. Sort the nodes that receive
the resource in a topological order, and compare each with those that still stand before it. A path between
two nodes can only pass through nodes between them in that order, so in the plain case, where each node takes
the place of the one before, the searches of one resource cover disjoint windows: linear in total. A node takes
the place of what it reaches only if it is the same part and every arm that holds the node also holds the other, since
"does not count" does not carry over: two nodes in different arms of a `match` or `handle` never run together (8.1), and
`bench.dmm` and `bench.source` are apart, while `bench` overlaps both. So what stands in an arm does not hide what stands
before the structure, and a part does not hide the whole. What stays is not searched again: what reaches the last node it
reached reaches this one, when that node reaches this one. A budget caps the total work across resources, so that many resources cannot make it
quadratic again: beyond it, a warning less. A value follows at most sixteen resources, for the same reason, and
beyond that the written order chooses which. Pass 5 has its own graph, in `declare/body/flow.rs`, since an order
wire counts there and not for types. Its warnings wait for the errors of the function, one of which can be their cause.

**Signatures of the primitives** (`core`, `string`, `array`, `errors`, `time`, `ui`) live here as data,
in the form of appendix A, with a rule attached where a signature cannot say it all: the units of `mul`,
the template of `string.format`, the variadic `errors.merge`. A test compares this table with appendix A, types included.
`runtime` (M0b) depends on `check` and tests that each signature has a body. The bodies
stay in `runtime`, as the roadmap says.

**Last resort.** `check` is one call over a library, so it runs each function under `catch_unwind`: a
panic is a bug, reported as `LC0099` on that function while the others go on. Pass 2 does the same for each
declaration, a type included, and pass 3 for each body; what pass 2 walks across declarations has no such net yet. `format` is covered per file
by the app. The release profile keeps `panic = "unwind"`. WASM traps instead, so the web viewer runs
this work in a worker that it can restart (scenario 6, point 5, in M0v).

## 6. `labcraft-cli` and `libs/embedded`

The app holds the arguments, walking folders (32 levels, 100,000 files, no symbolic links), reading
files, atomic writes, and turning a `Report` into text or JSON. Nothing else: the logic stays in the
crates, where WASM reaches it.

- **Libraries:** implements `Libraries` by the rules of cli.md, section 3.
- **`libs/embedded`** (`labcraft-libs`) is a small crate outside `crates/` that embeds the files of
  `libs/` at build time. Apps and test harnesses depend on it, including WASM, where `wrong-input-type`
  needs `scpi`; no crate of `crates/` does.
- **Arguments:** by hand, like `gridcraft-cli` and `solidcraft-cli`, with no parsing crate.
- **Writing a file:** through `craft-atomic` (`craft-libs`), the suite's crash-safe save: a crash leaves
  the old file or the new one. It also keeps a symbolic link a link. A read-only file is refused before it is
  called, since on Windows its rename retries for most of a second. A crash can leave its staging file next to the program, named `.name.pid-n.craft_tmp`, which no
  walk reads.
- **The work** runs on a thread of `format::STACK_SIZE`. A panic on one file is a bug, reported as `LC0099` on
  that file while the others go on; a hook keeps it out of stderr, which is for wrong usage alone.
- `fmt` and `check` become engine commands when `engine` arrives (M0b, D-18, section 10.8), and the last resort
  moves to `Session::execute`, as in the Craft apps. Until then the app calls the library functions directly;
  nothing here should make that move hard.

## 7. `xtask` and the workspace

As in SolidCraft and GridCraft: edition 2024, resolver 3, `rust-version` 1.95, licence
`MIT OR Apache-2.0`, shared lints and profiles at the root, and their `rustfmt.toml`. A folder
`crates/format` holds the package `labcraft-format`; this document and the table of layers use the
short names, as theirs do. As in GridCraft, `clippy.toml` refuses `Instant::now` and
`SystemTime::now`, which trap in WASM.

```toml
[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
dbg_macro = "warn"
todo = "warn"
```

Every production crate root carries the same line as theirs, with two additions, and `clippy.toml`
allows these in tests:

```rust
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo,
        clippy::unreachable, clippy::indexing_slicing, clippy::string_slice)]
#![forbid(unsafe_code)]
```

`indexing_slicing` and `string_slice` make the shared rule "`get()`, not `[i]`, on input-derived values"
a check of the compiler: our input is hostile by scenario 4, a lexer is where a slice panics on a
character boundary, and these crates hold little numerical code.

| `cargo xtask` | Checks |
|---|---|
| `ci` | Everything below but `unicode` and `fuzz-licences`, plus `cargo fmt --check`, `clippy -D warnings`, tests |
| `layers` | A crate depends only on lower layers, and directly on no UI toolkit below L7, from `cargo metadata` and a table, as in GridCraft. A crate missing from the table is an error |
| `licences` | No GPL, LGPL or AGPL dependency (D-23), on any platform, tests and tools included; a licence that cannot be read is an error. Refusing is ours: SolidCraft's command of that name lists the licences for a release |
| `libs` | No crate of `crates/` depends on `libs/` (`libs/embedded`, and the native crates of libraries such as `libs/visa`), nor, from M0b, on a native crate of `examples/` (`examples/results_log`) |
| `codes` | `Code` equals the tables of cli.md, section 5; from M0b, the codes of each native library equal the table of its document (section 10.6) |
| `commands` | From M0b: the registry of `engine` equals the tables of commands.md, section 6, and no two ids give one MCP tool name (section 10.8) |
| `bounds` | `fmt` of each file of scenario 4, and `check` of a folder that holds only that file, take at most 2 s and 512 MB, the library of 100,000 files at most 60 s: the corpora, and files built to cost the most just under the limit of size (section 8). In release, each case in a process of its own; the memory on Linux only |
| `wasm` | `format`, `check`, `trace` and `ir` build for `wasm32-unknown-unknown`, and pass the corpora there, on the call stack of V8 in a worker (section 4); some test must have run. From M0b, the deterministic part of `runtime`, and `engine` without runs build there too (section 10) |
| `assets` | Every file of `assets/`, and every file that `unicode` writes, has its line in `ATTRIBUTION.md`, and every file that it names exists |
| `figures` | Not a check: the size of each function of `examples/`, the zoom at which it fits in the canvas of the window, and the figure of stability (section 11.6), in Markdown on standard output, which the workflow `checks` adds to its summary. Not part of `ci`, and without a threshold |
| `unicode` | Not a check: writes the tables of `types::text` and the test file of UAX #29 from the Unicode Character Database of spec 14.1, downloaded once into `target/` with `curl` and refused when a SHA-256 is not the one it pins (section 11.1). Not part of `ci`, which needs no network |
| `fuzz-licences` | The same as `licences`, for `fuzz/`, which is outside the workspace, and refuses a stale `fuzz/Cargo.lock`. Not part of `ci`: the job `fuzz` runs it (section 8) |

**Dependencies** of the crates, kept few and permissive: `unicode-normalization` (R-20),
`unicode-properties` (the Unicode categories of 14.1), `serde` and `serde_json` (reports), `toml` (manifests of shipped libraries),
`thiserror` for the error type of a crate, where a function can fail on something other than the program,
such as a file that cannot be written (the versions the Craft apps use). In tests: `wasm-bindgen-test`,
`cargo-fuzz`. `assets` joined `xtask ci` with the first asset, the font of the notation (section 11.1).

**From the suite.** `craft-atomic`, from [`craft-libs`](https://github.com/storytold/craft-libs)
(MIT or Apache-2.0), pinned to a revision as the apps do, is the only shared crate M0a needs, and only in
`labcraft-cli`. The others wait for what they serve: `craft-ui`, `craft-window` and `craft-settings`
for the desktop viewer (section 11), `craft-keymap` for editing, `craft-i18n` for a translated interface. Messages of the tool stay in
English (D-21). The reusable workflows of
[`craft-common-actions`](https://github.com/storytold/craft-common-actions) (`gate`, `supply-chain`,
`release`) wait for the first release; `release` is what a signed build for the launcher's catalog
(D-20) would go through. `gate` was evaluated when the Rust CI started, and not adopted for now: it
follows the `main` branch of another organization, and its job name would replace `checks`, the
check required on `main`. Our single job takes the same steps, with its actions pinned by commit.

## 8. Tests

A crate is tested on its own first: the corpora need no CLI to reach `format` and `check`, and the
harness runs natively and in WASM. It lives in `tests/harness`, outside `crates/`, and embeds the corpora
and `libs/embedded`. `forbid(unsafe_code)` holds there too: what `wasm_bindgen_test` expands to needs no
exception (verified at step 1).

| What | Where | Scenario |
|---|---|---|
| Unit tests: units, lexer, each rule and comment slot of 5.2 | in each crate | |
| Maximum nesting: parse, `Drop`, `Clone`, `Eq`, `canonicalize`, `write`: bytes per level, with the stack of each host | `format` tests, `tests/harness` | 4 |
| `type-errors` and stored `hostile`, compared with their reports; Truncation and Nesting | `tests/harness`, native and WASM | 2, 4 |
| Every canonical file written back byte for byte; `fmt(fmt(f)) = fmt(f)`; model unchanged | `format` tests, `tests/harness` | 5 |
| Exit codes, paths, walking, JSON, atomic writes, the generated families | `apps/labcraft-cli/tests` | 4 |
| `examples/`, `docs/rfc/`, `libs/` clean under `check` and `fmt --check` | `apps/labcraft-cli/tests` | |
| Time and memory of `fmt`, `check` and the drawing of `diagram` (11.6), in release: `cargo xtask bounds` | `tests/harness/tests/bounds.rs` | 4 |
| Fuzzing the parser, `check`, the round trip and `diagram` | `fuzz/` | 4 |

`cargo fuzz` needs Rust nightly and works poorly on Windows, so `fuzz/` stays out of the workspace: the
product builds and ships on stable, and a separate CI job fuzzes on Linux with a pinned nightly. That job is
the workflow `fuzz`, next to `checks`, which knows nothing of it and stays the only check GitHub requires on
`main`; a red `fuzz` still stops the merge unless the owner decides otherwise for that pull request.
`fuzz/README.md` says how to run a target and what becomes of a crash.

The workflow `checks` also has a job `windows`, which runs the tests of `labcraft-cli` and `labcraft-libs`: paths,
walking folders and atomic writes differ there, and the crates are the same. GitHub does not require it either, and
a red one stops the merge in the same way.

The proof of concept remains the reference for what the examples do until the Rust `run` passes the corpus of runs
(section 10.11); it does not judge types, where the corpus decides.

## 9. Order of work

Each step ends with something testable, and can be one or a few pull requests. All ten are merged
(2026-11-07), and M0a is tagged `v0.1.0-m0a` (2026-11-08).

1. **Workspace:** crates, lints, `xtask` (`ci`, `layers`, `licences`, `libs`, `wasm`), CI.
2. **`types`:** `Name`, `Unit`, `Ty`, `Code` and `Report`, `xtask codes`.
3. **Lexer**, tested on the decoding part of the Encoding family, at crate level.
4. **`graph` and parser:** every file of `examples/`, `libs/` and `docs/rfc/` reads; Encoding, Truncation
   and Nesting at crate level, with the stack test; fuzzing the parser starts here.
5. **`write` and `canonicalize`:** scenario 5; fuzzing the round trip.
6. **`labcraft-cli fmt`:** arguments, walking, reading, JSON and human reports, atomic writes,
   `LC0001`, `LC0002`, `LC0008`, `LC0009`, `LC0010`, the Size and Library families. `libs/embedded`.
7. **`check`, passes 1 to 3:** name errors with their codes. `labcraft-cli check` exists from here.
8. **`check`, pass 4:** the corpus of scenario 2. The main new piece of M0a.
9. **`check`, passes 5 and 6:** warnings, panel, layout, icon, and the rest of scenario 4; fuzzing `check`.
10. **The agent half of scenario 2,** by hand before the tag.

Rust contributions have been open since the tag of M0a for its crates, and for those of M0b and M0v once their
sections are written ([CONTRIBUTING.md](../CONTRIBUTING.md)).

## 10. M0b: running and agents

*Status: draft; steps 0 to 3 of 10.11 implemented, and the first part of step 4 (4a).* How the code of M0b is arranged: the crates `ir`, `runtime`, `engine` and
`mcp`, the native crates of `visa` and of the example `results_log`, and what `check` and `labcraft-cli` gain. The
contract is elsewhere: [spec](language-spec-v0.md) 8 and 10, [`run`](cli.md) (section 6), the
[command catalog](commands.md), the [MCP server](mcp.md), [`visa`](../libs/visa/README.md) and its
[simulation](../libs/visa/simulation.md), [acceptance scenarios 1, 3, 4](acceptance-m0.md). The scheduler was tried
first in the proof of concept (scenario 3): what it taught is written here.

```text
apps/labcraft-cli ──> mcp ──> engine ──> runtime ──> ir ──> check ──> format ──> graph ──> types
       │                                    ├──> trace
       ├──> libs/visa (labcraft-visa) ──────┤
       ├──> examples/results_log ───────────┘
       └──> libs/embedded
```

| Layer | Crate | In M0b |
|---|---|---|
| L1 | `trace` | The events of `labcraft-trace/1`, the JSON form of values with its bounded writer (commands.md 5), the bounded reader of the trace; shared with M0v (section 11) |
| L4 | `ir` | The dataflow IR: [ir.md](ir.md) |
| L5 | `runtime` | Values, the scheduler, the interpreter, the primitives, the interface of a native library, the report of a run |
| L6 | `engine` | The session, the registry of commands, runs as tasks |
| L6 | `labcraft-visa`, `labcraft-results-log` | The native crates of the libraries `visa` and `results_log` |
| L7 | `mcp` | The MCP server over stdio, made from the registry |

**A native crate lives beside the declarations it gives bodies to**: `labcraft-visa` in `libs/visa/`,
`labcraft-results-log` in `examples/results_log/`. A library is its `.lcf` files and, when it has native functions,
its crate (spec 4.1: a Rust crate linked into the application). Each depends on `runtime` and below, and no crate of
`crates/` depends on it (`cargo xtask libs`): the apps and the test harness link them and give them to the session.
`xtask layers` places them at L6, beside `engine`, which does not see them.

### 10.1 What `check` gives a run

*Status: implemented (step 0).* `check` was built to judge files; a run also needs what it found, in a form that
lowering can read. So `check` has an interface for runs:

```rust
pub fn check_for_run(root: LibrarySource, libraries: &dyn Libraries, names: &mut Interner, limits: &Limits) -> Checked;
impl Checked {
    pub fn function_named(&self, path: &str, names: &Interner) -> Option<usize>;  // `dmm_driver.measure`, from the root
    pub fn reach(&self, function: usize) -> Reach;                                 // by index in `files`
    pub fn counts_of(&self, file: usize) -> FileCounts;                            // its errors and warnings, complete
}
pub struct Reach { functions: Vec<u32>, types: Vec<u32>, unresolved: Vec<Unresolved> }  // `refuses(&checked, deny_warnings)`
```

- **The bodies of every library the folder uses**, those of `scpi` and of a `--lib-path` folder included, are resolved
  and typed as the folder given is; `check` alone reads them for their signatures. What such a body says goes to
  `Checked::bodies`, one report for all of them, bounded as one, so the report is the one `check` gives, and a run says
  it only of what it reaches (cli.md 6.1). `Checked::files` then holds the files of every library, each with its
  `library`, and `for_run` says so.
- **What a run reaches**, by a work list: the functions it calls and names as references, in every library, and the
  `.lct` files of the types they use, down through fields, items and variants. An error in one of them refuses the run,
  and so does a warning in one of the folder given under `--deny-warnings`; a warning of a dependency never does.
  **One error gives one diagnostic**, so what a reached file leans on can be broken with no word of its own: a name that
  a file that did not read may declare (a call, a function reference, a construction, a variant), or a value whose type
  an error left unknown, such as a struct with a field of an unknown type. Wherever passes 3 and 4 stay silent for that
  reason, they mark the file (`Resolved::leans_elsewhere`); `Reach::unresolved` names it, with the calls that bind to
  nothing, the values left without a type, and as a net under all of these the types that hold the error type (`[Place]`
  with a `Place` that has an error); the run is refused, and the cause has its diagnostic elsewhere. The counts
  of each file (`CheckedFile::counts`) come from `Report::counts_of`, complete whatever the lists keep, and are added up
  once, when `check` ends.
- **Limits:** a run of the CLI reaches a file of the folder given through a `--lib-path` copy of it as that dependency,
  whose warnings do not count under `--deny-warnings`; `run` (step 7) maps the copy back to the folder. When
  `Checked::bodies` lists fewer diagnostics than it counts, those of a reached file may be among the ones it left out:
  the run is refused all the same, by the counts.
- **What the manifests say**, normalized and in their order, in each `DeclaredLibrary`: `always`, `resources`,
  `categories`; and `file`
  for each function, `type_files` for each type, by index in `Checked::files`. The **defaults** of inputs are the
  operands of the signatures, typed in `Resolved`; those of outputs are the defaults of their types (spec 7.2).
- **The table of primitive signatures** is public (`check::primitives`), with the value of each default: `none`, `0`,
  `""`, and `…` for the `length` of `array.slice`, which reaches the end of the array and is no constant. `runtime`
  tests its bodies against it.
- **The canonical order of every body**, in one walk: `format::canonical_order` called body by body is not linear
  in a function nested deep, so `format::canonical_orders` gives them all from the walk that finds them, by the id of
  the statement or arm that holds each body, in `CheckedFile::orders`.

### 10.2 `ir` (L4)

*Status: implemented (step 2).* [ir.md](ir.md) defines it: a program as regions of nodes, the nodes of the diagram, each
region reading only its arguments, every node with its origin and its canonical position, every value typed. It is
written to last: the interpreter, the trace, the debugger (M5) and the linear IR (M7) build on it.

```rust
pub fn lower(checked: &Checked, start: usize, names: &mut Interner) -> Result<Program, LowerError>;  // LowerError: LC8004
pub fn verify(program: &Program) -> Vec<Violation>;
pub fn print(program: &Program, names: &Interner) -> String;     // print_function: one of them, for a snapshot
pub fn apply_scale(value: f64, scale: &Scale, single: bool) -> f64;  // the conversions of 3.3, for the runtime too
```

- **Lowering** takes the `Checked` of a run that is not refused (10.1) and the file of the function to run, and
  lowers it, then each function it names, through a work list; the signature of a callee, with its defaults, is made
  when it is first named. A function is walked once to find its shape (its bodies, where each value is defined, the
  lines that make no node, the values each structure passes in, the orders of `after:`), then its bodies are lowered
  innermost first. Nothing recurses on the nesting of bodies; a constant and a type recurse, as `check` does, on the
  stack of the host. `crates/ir/tests/stack.rs` lowers loops, `match` and an array constant nested to `MAX_NESTING`.
- **What it adds to `check` and `types`**, for it: `Resolved::outputs_of` and the type of `_` targets (section 5);
  `check::unit_of`, `check::read_escapes` and `check::template_fields`, so that a unit, a text and the order of the
  fields of a template are read as `check` reads them; `Unit::scale_ratio` and `Unit::offset`, the integers and the
  offsets of spec 3.3.
- **Tests:** a test per construct and per kind of node in `crates/ir/src/tests.rs`, their expected text written by hand;
  the snapshots of every function of `examples/`, `libs/` and `docs/rfc/` in `tests/harness/snapshots/ir/`, natively
  and in WASM, with `ir::verify` on each.

### 10.3 Values

*Status: implemented (step 3).* A run-time value is one enum, `runtime::Value`: booleans, the integers, floats and complex numbers of 3.1, `string`,
`bytes`, arrays, structs, variants, `error`, a `duration` in whole nanoseconds (spec 3.1, Q-2), a `timestamp`, a
function reference, and a **handle** for an opaque type (the library that made it and its number there). Arrays,
strings, structs and variants are behind an `Arc`, and a change works on a copy only when the value is shared
(`Arc::make_mut`): values never change once made (5.1, D-11), and a wire read by ten nodes is not copied ten times.

- **A number is kept in the unit of its type** (3.3), and converted on a wire as spec 3.3 says, one rounding where the
  ratio of the two units allows it, so that the interpreter, the WASM build and Cranelift give the same bits (D-10).
- **The logical size** of cli.md 6.2 is counted as values are made, not by asking the allocator, so a limit gives the
  same fault on every platform. A value counts once while some wire holds it. Each node that runs or waits, and each
  frame, counts too (cli.md 6.2), so that a `for parallel` over millions of elements reaches the limit as a fault
  rather than exhausting the memory of the process.
- **Operations are defined bit for bit**: the primitives of `core` use operations that IEEE 754 rounds correctly, and
  those it does not (`core.pow` with an integer exponent, for one) are written as a fixed sequence of such operations,
  rather than a function of the platform's library: `pow` by squaring, a complex magnitude as the larger part times
  `√(1 + r²)`, Smith's complex division (spec, appendix A.1). For `1.1^10`, CPython's `pow` gives `2.5937424601000023`
  and the squaring `2.593742460100002`, which is what every platform gives.
- **What it is in Rust:** an integer of each kind, `f32`, `f64`, a complex number as two of them; `String` and `Bytes`
  as `Arc<str>` and `Arc<[u8]>`; `Array(Arc<Items>)` for one dimension, whose items keep their logical size as they
  change, so that a push in a loop does not count the array again; `Grid` for an `[T; 2]` or more, its shape and its
  items row after row; `Struct`, `Variant`; `Error(Option<Arc<ErrorData>>)`, `none` being `None`; `Function(ConstId)`,
  since a reference is always a constant of the program; `Handle`; and `ToTheEnd`, the `length` of `array.slice` with no
  wire. `runtime::constant` gives each constant of the program its value, `runtime::read` what a wire delivers (its
  fields, then its conversion, by the steps that `ir` evaluates constants with: `ir::apply_scale`, `ir::nanoseconds`), and
  `runtime::Writer` its text (spec 5.4), with the number writers of `trace`, so that a report and `string.format` agree.
  A default that Q-21 has not settled has no value yet: the interpreter decides it (step 5).

### 10.4 The scheduler

What the prototype showed (scenario 3, the proof of concept): every point of spec 8 falls out of one design.

- **Each node that runs is a task**, and a structure is a task that starts the nodes of its region as their inputs
  arrive and waits for them. A task keeps its own context: the frames of its calls, the iterations of its loops, its
  **position** (spec 8: the positions of what contains it, each loop adding its iteration), kept as a link to the
  position of what contains it plus one index, so that a task costs the same at any depth. **Readiness is counted,
  not searched**: a node knows how many of its inputs are still missing, and the producer of a value decrements the
  count of each node that reads it. The prototype searched the pending lines of a body each time a node ended,
  which is quadratic in the size of a body.
- **Ready tasks** are kept by position in a binary heap: the first runs. With a schedule seed, they are kept in a plain
  list, one is drawn, then swapped with the last, so a draw costs one step; the first version of the prototype rebuilt a
  heap on each draw, which was quadratic.
- **Time.** A task that takes time sleeps until a time on the clock of the run, in a heap of wake-ups. When no task is
  ready, the clock jumps to the next wake-up in deterministic mode, or the scheduler waits until then in real time. At
  one time, the settings of `--set` apply first, then the timeout, then the tasks (cli.md 6.2).
- **One thread runs the diagram.** The scheduler and the nodes it runs share one thread: the interpreter is cheap, and
  a single thread keeps the order of spec 8 a fact of the code rather than of locks. What can block is a **native
  call** (a VISA read, later a file): in real time it runs on a thread of a pool of the runtime, and its end comes back
  to the scheduler as an event on a channel, so a call that waits for an instrument does not stop the other loops
  (D-12). In deterministic mode a simulated call does not block: it does its work at once and says how long it took,
  and its task sleeps that long, so the waits of parallel branches overlap and the run replays exactly. The two modes
  share all the code but the clock and where a native call runs.
- **Running compute nodes on several threads**, D-12's blocks on a thread pool, comes when a program needs it (M2 or
  later). Nothing above stops it: the order of spec 8 binds deterministic mode, and a real-time run may run ready
  nodes side by side.
- **Calls are iterative.** A call is a frame on the heap, not a call of the interpreter on the stack: the depth limit
  (`LC8003`) is a count, and a deep recursion of the program cannot overflow the process. Every thread that runs a
  command or a run (the thread of a run, the thread of the session, the threads of the MCP server) has the stack of
  `format::STACK_SIZE`, like the other work of the app.
- **Stopping** (spec 8.2). Stop is the program's own: the scheduler does nothing special. An abort or a fault winds down
  the same way: no task starts; waits end at once; a native call in progress ends within the grace period, on the clock
  of the run, or is abandoned when the period ends and its resource reported as leaked; then each resource the run
  opened is released by its library. The prototype first had faults release resources while a call still used them:
  both paths now go through one function, and so will these.
- **A panic** of a primitive or of a native body, on the thread of the run or on a thread of the pool, is caught there
  and ends the run with the fault `LC8004`, released as any fault is; the session goes on.
- **Two clocks.** The app gives the runtime a `runtime::Clock`: the clock of the run, simulated or real, and the real
  time that `--max-real-time` bounds in deterministic mode. The runtime never calls `Instant::now` itself, which
  `clippy.toml` refuses and which traps in WASM.

### 10.5 The interpreter and the primitives

*Status: the primitives are implemented (step 3); the interpreter is step 5.* A frame holds the values of a region instance and the state of the panel of its call (spec 10: a `switch` holds, a
`button` is seen by one read). A node runs by its kind (ir.md, section 4), skips by its `Skip` (ir.md, section 5), and
checks the limits of cli.md 6.2 as it goes: iterations of a loop, nodes run, depth, memory, the real time of a
deterministic run. A limit reached is a fault at that node.

The primitives (`core`, `string`, `array`, `errors`, `time`, `ui`) have their bodies here, by the names of `check`'s
table of signatures; a test requires a body for every signature, and no body without one. A primitive that takes
simulated time (`time.wait`) is a native call of the runtime's own.

```rust
pub fn primitives::run(library: PrimitiveLibrary, name: &str, call: &mut Call, host: &mut dyn Host) -> Result<Outcome, Fault>;
pub enum Outcome { Done(Vec<Value>), Fail(Value), Wait(i64), Call { function: Value, inputs: Vec<Value> } }
pub trait Host { fn now(&mut self) -> i64; fn element(..); fn set_element(..); fn room(&self) -> u64; fn error(..) -> Value; }
pub fn outputs_on_error(env: Env, skip: &Skip, inputs: &[Value], failed: Option<Value>) -> Result<Vec<Value>, Fault>;
```

- **A body** reads its inputs in the order of ir.md, section 4, and its statics (units, categories, elements, the
  template). It gives its outputs; or fails with a new error, which the host makes, so that the error has the origin of its
  node; or, for `time.wait`, a wait on the clock of the run; or, for `core.call`, the function to run.
- **A node that fails gives the outputs of a node that skips** (spec 7.2): `outputs_on_error` passes through each input
  named like an output, gives the others their default, and the `err` output the error received, else the new one. So
  `array.slice` and `array.set` that fail give their array unchanged.
- **What `err` cannot say is a fault** (`runtime::Fault`): `LC8008` for a value that its operation cannot take, a number
  that its type cannot hold first (spec 8.2, Q-25), `LC8005` when a body would build a value beyond the memory the host
  has left (`Host::room`, asked before `array.repeat`, `concat`, `push`, `set`, `errors.merge` and the texts of `string`),
  `LC8004` for a bug. A change asks for what it adds when nothing else holds what it changes, which then changes in
  place, and for a whole copy when it is shared (D-11): the interpreter gives a body the last hold of a value it can.
- **An `[T; 2]`** is a `Grid`: `array.get` gives a row, and `array.set`, `push` and `concat` work on its rows and give an
  `[[T]]` (appendix A.2); a node of them that fails passes through the rows, in the type of its output.
- **The template of `string.format`** is read by `check::template_pieces`, so a field and its format are read as `check`
  judged them.
- **Tests:** in `crates/runtime/src/primitives/tests.rs`, each body against appendix A and the spec, the expected values
  worked out by hand or with Python's doubles; in `lowered.rs`, programs that go through `check` and `ir`, whose nodes a
  small walk runs through the bodies with the `Skip` that lowering built; in the harness, every constant of `examples/`,
  `libs/` and `docs/rfc/` is a value of its type that writes as text, and every primitive their nodes call has a body.
- **Limits:** the logical size is below what the process holds: `string.split` of 4 million commas counts 61 MB and
  takes 153 MB, `array.repeat(true, 50000000)` counts 810 MB and takes 1.1 GB (measured on Windows, by the review of
  step 3); a run under the default limit of 1 GiB may hold about 2.5 times that. `cargo xtask codes` checks section 5 of
  cli.md, not the codes of `run` (6.6) that `FaultCode` gives.

### 10.6 Native libraries

*Status: the interface and `labcraft-visa` in deterministic mode are implemented (step 4a); `Blocking` comes with real time
(step 8).* The runtime defines what a library with native functions gives it (`runtime::native`):

```rust
pub trait NativeLibrary: Send + Sync {
    fn name(&self) -> &str;                                              // "visa": as its manifest says
    fn codes(&self) -> &'static [LibraryCode];                           // `visa:S001`…, for `xtask codes`
    fn simulates(&self) -> bool;                                         // it reads a section of a simulation file
    fn simulate(&self, section: &Section) -> Result<Simulation, Vec<LibraryDiagnostic>>;
    fn start_run(&self, library: u32, simulation: Option<&Simulation>) -> Box<dyn LibraryRun>;   // what one run holds
}
pub trait LibraryRun: Send {                                             // its sessions, its instruments, for one run
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId>;   // starts an `extern rust` body
    fn step(&mut self, call: CallId, cx: CallContext) -> Step;           // asked again after each wait
    fn cancel(&mut self, call: CallId);                                  // a call the run abandons
    fn describe(&self, handle: Handle) -> Option<HandleInfo>;            // what it designates, whether it is open
    fn release(&mut self, handle: Handle);                               // after an abort or a fault
    fn instruments(&self) -> Vec<InstrumentLog>;                         // the commands each instrument saw, for the report
}
pub enum Step { Done(NativeOutcome), Sleep(i64), Exclusive(Key), Release(Key) }   // `Blocking(…)` with real time
pub enum NativeOutcome { Outputs(Vec<Value>), Fail { category, code, message } }
```

- **The library keeps its calls.** A call is a state machine, but the run of its library holds it and steps it
  (`LibraryRun::step`), rather than a `NativeCall` object of its own: a call reads and changes the sessions and the
  instruments of its run, which an object apart from the run could only borrow from it while the run lends itself to
  another call. A call that fails gives its error, without origin: the host makes it, at its node, and the outputs are
  those of a node that fails (`outputs_on_error`, 10.5).
- **A library judges its section** of the simulation file and returns its diagnostics with its own codes
  (`LibraryDiagnostic`, cli.md 4.4), each at its key or its value; they join the report of the run with `labcraft-cli run`
  (step 7). What it judged (`Simulation`, its own type) serves every run that uses the file.
- **The file as a whole** is read by `runtime::simulation`: its nesting is counted first, on the tokens of the lexer of
  `toml` and without recursion, along the path from its root to its deepest value (cli.md 6.2): 80 levels at most, or
  `--max-nesting` when lower, since the parser recurses and stops at 80, and since the tree it builds is dropped by
  recursion, so that the levels of keys and of containers multiply there: 80 inline tables each led by a key of 80
  segments make 6,400 levels, which overflow a stack of 1 MiB. A parse error of the reader's own limit is `LC0007` too. Then it is parsed, its integers checked to fit in 64 bits, and each library given its table
  (`SimulationFile::judge`). A position is a binary search in `simulation::Places` (the start of each line, and the code
  points before every 1,024 bytes), so a file of many problems is placed in linear time. The codes of the tool that this
  gives, `LC0007`, `LC8109` and `LC8110`, join the tables of cli.md, section 5, when `labcraft-cli run` gives them;
  `cargo xtask codes` checks that each has its row there. A library says its problems in the order of the keys of its
  section, which `toml` sorts: the tool sorts the diagnostics of a run by place (cli.md 4.2).

- **What a run holds is the library's own** (`LibraryRun`): its sessions and simulated instruments live and end with
  the run, and `describe` and `release` read them there.
- **A call is a small state machine.** It returns `Sleep` to wait on the clock of the run (the latency of a simulated
  instrument, a read that waits for its timeout), `Exclusive` to wait for a key and `Release` to give one back, in
  the middle of a call if it must (an instrument is free once its latency has passed, while the session waits for
  the answer: simulation.md 4), `Blocking` for work that blocks in real time and runs on the pool, and `Done` with its
  outputs; the keys it still holds are given back then. The scheduler calls `step` again after each wait, so a
  call can wait in its middle, as an instrument that is busy makes it (simulation.md 4).
- **A library owns what it guarantees** (spec 8.1): `visa` keeps each of its calls atomic on its session, and each
  instrument handling one command at a time, through keys that the runtime lends (`Exclusive`). A key is a lock in
  real time, and in deterministic mode a queue of the scheduler that gives it in the global order of spec 8, so that a
  replay is exact. The runtime locks nothing on its own.
- **The simulation file** is split by the tool (cli.md 6.2): each library gets its table, with the spans of its keys,
  so that a diagnostic points at its key (simulation.md 7), and judges it with its own codes. `cargo xtask codes` also
  checks that the codes of each library equal the table of its document.
- **Resources** are what the manifest declares (`resources = ["session"]`). The runtime counts what was opened, what
  stays open and what leaked, for the report (`resources`, commands.md 7), which shows each by what `describe` gives;
  and it calls `release` on what a run leaves open after an abort or a fault.

`labcraft-visa` holds the sessions, the `SIM::` backend of [simulation.md](../libs/visa/simulation.md) (the clock in
whole nanoseconds, the noise generator, the error queue, the faults), and its fuzz target for simulation files (step 4b).
Real instruments come with M4. `labcraft-results-log` keeps the results it is given, as the proof of concept does, and the
report shows them (`logs`, decision of the owner, 2026-11-09; step 4b).

- **The keys of `visa`:** a call takes the key of its session for all of its length, so it is atomic on its session; a
  command takes the key of its instrument for its latency only, so an instrument handles one command at a time, whichever
  session sends it, and is free while a session waits for its answer or its timeout (simulation.md 4). A session is the
  handle `n` of its run, from 1, so the default of a `session`, numbered 0, is no session.
- **What it keeps for the report:** the first 1,000 commands of each instrument *(proposed)*, then a count of the others;
  of each command and answer, its first 1,000 code points and its length (commands.md 5); and for all the instruments of a
  run, 16 MiB *(proposed)*, past which commands are counted, not kept, so that what a program sends cannot grow the memory of
  a run outside `--max-memory`. A session holds the index of its instrument, not its name.
- **Tests:** `libs/visa/src/tests.rs`, with a small driver that gives the keys and the clock as the scheduler will; the
  expected values worked out by hand, or by a Python script written from simulation.md 3 alone, which gives the factor that
  the document states. `cargo xtask codes` compares `labcraft_visa::CODES` with simulation.md 7. `labcraft-visa` builds for
  WASM.

### 10.7 The trace and the report

The runtime writes the trace (`labcraft-trace/1`, commands.md 8) as it runs, to a sink the caller gives, and the values
of its report (`labcraft-run/1`, commands.md 7), both through the crate `trace` (L1): the events, and the JSON form of
values with its bounds (commands.md 5), so that the report, the trace and the reader of M0v cut a value the same way.
The runtime turns its `Value` into that form; `trace` never sees `runtime`. Neither the trace nor the report knows
where it is written: `engine` and the app decide.

### 10.8 `engine` (L6)

As in the Craft apps: a **session** and a **registry** of commands (D-18).

- **The registry** holds each command as the data of commands.md 1: id, label, summary, description, schema of its
  parameters, result, errors, effects, `long`, `planned`. `Session::execute(id, params)` is the only way in: the CLI,
  the MCP server and later the IDE call it. A panic in a command is caught there and returned as `internal`, the last
  resort that the app holds in M0a (section 6). `cargo xtask commands` checks the registry against the tables of
  commands.md 6.
- **The session** holds the root of the project, the interner, the libraries (shipped, embedded, and those of
  `--lib-path`), the native libraries the app gave it, its last `check` (checked at start, then again only for the
  library a change touched: commands.md 6.2), and its runs.
- **Runs** are long commands (commands.md 4): `run.start` starts one on a thread of its own and returns its report, or
  `running` after `wait`; one run at a time per session; the last 16 reports are kept. **A caller waits on the run, not
  in the session**: `run.wait`, and `run.start` until its `wait`, block the thread of their caller on the run's own
  channel, so the session goes on answering, and `run.abort` reaches the run while another caller waits for it. `run.abort`, `run.set_element`
  (real time only) and Ctrl-C reach the scheduler through a channel.
- **Files** are reached through `engine::Workspace`, which the app implements: the disk with atomic writes for the CLI,
  memory for tests. So principle 1 holds: only an app touches the file system.
- **The state of the viewer** (the folder open, the function shown, the view state that the file does not hold, the
  preference of the standard view, the trace shown) is held by the session too, and the `view.*` commands change it;
  the app reads it each frame, as in the Craft apps. M0v writes that part (`engine::view`, section 11).
- **`engine` builds for `wasm32-unknown-unknown`** without runs, whose commands are not enabled there: the web viewer
  runs a session in a worker and sends it commands as an id and parameters in JSON, so the boundary of the worker is
  that of the commands, and a trap restarts the worker with a new session. On the desktop, the session runs on a
  thread of `format::STACK_SIZE`, and the interface sends it commands through a channel.
- `fmt` and `check` become the commands `file.format` and `library.check`, and `labcraft-cli` calls them like the
  others. `diagram.svg` and the `view.*` commands are registered as `planned` until M0v brings them (commands.md
  6.5), and the MCP server exposes each one once it is enabled.

### 10.9 `mcp` (L7)

The MCP server of [mcp.md](mcp.md): JSON-RPC over stdio, one tool per enabled command, named from its id (`run.start`
gives `run_start`), its schema from the registry, a command error returned with `isError`. Written by hand over
`serde_json`, without an async runtime: a thread reads requests, bounded as mcp.md 5 says (size, nesting, a queue of 16);
requests run in order on another thread, while `ping`, `run_status`, `run_set_element` and `run_abort` answer at once
from the reading thread, so an agent can always stop what it started; progress and cancellation are notifications
on the same stdout, written one whole line at a time. `labcraft-cli mcp` starts it on a session of its own (D-18),
headless in M0b. The instructions it gives point to `guide.read`.

### 10.10 `labcraft-cli`

It gains `run` (cli.md 6) and `mcp`, both through `engine`. It implements `Workspace` and the real `Clock`, gives the
session the native crates it links (`labcraft-visa`, `labcraft-results-log`), and turns Ctrl-C into an abort, a second
one into the end of the grace period (cli.md 6.5), through the `ctrlc` crate. `run` is not in the WASM build of M0
(D-22); the deterministic part of `runtime` still builds for `wasm32-unknown-unknown`, which `cargo xtask wasm`
checks, for the web simulation of M10.

### 10.11 Tests and the order of work

| What | Where | Scenario |
|---|---|---|
| The IR of every file of `examples/`, `libs/`, `docs/rfc/`, against snapshots; `ir::verify` after every lowering | `tests/harness` (`snapshots/ir/`) | |
| One lowering test per construct of the language, and per row of ir.md, section 4 | `ir` tests | |
| A body for every primitive signature, and no other | `runtime` tests | |
| **The corpus of runs**: each case a program, a simulation file, arguments and settings, and the expected report | `tests/acceptance/runs/` | 1, 3, 4 |
| The MCP loop of scenario 1, scripted, through a real stdio client | `apps/labcraft-cli/tests` | 1 |
| Limits of a run: each fault of cli.md 6.6 reached, hostile simulation files, programs built to cost the most | `tests/acceptance/runs/`, `xtask bounds` | 4 |
| Fuzzing simulation files, and runs of fuzzed programs under small limits: no crash, every outcome a report | `fuzz/` | 4 |

**The corpus of runs is the reference of `run`**, as the corpus of `type-errors` is for `check`. A case holds what a run
needs and the parts of `labcraft-run/1` it asserts: status, outputs, clock, resources, abort, panel, the commands each
instrument saw. Its expected values are worked out by hand, as the tests of the proof of concept are, and each says how.
The proof of concept runs the corpus too, through a small runner, until the Rust `run` passes all of it: then
`tools/lcf-poc` is retired, and the corpus stays.

Each step of work ends with something testable:

0. **`check` for runs:** the interface of 10.1. *Done.*
1. **`trace`**, with M0v: the events, the JSON form of values and its bounds, the writer and the reader. *Done*
   (section 11.2).
2. **`ir`:** the model, lowering of every construct, the verifier, the printer, the snapshots of `examples/`. *Done*
   (section 10.2).
3. **Values and primitives** in `runtime`, with their test of signatures. *Done* (sections 10.3, 10.5).
4. **The interface of native libraries and `labcraft-visa`**, in deterministic mode: sessions, the `SIM::` backend, its
   codes, `release`, its keys; the fuzz target of simulation files. `labcraft-results-log`. *4a done* (the interface,
   `labcraft-visa`, `runtime::simulation`); 4b: the fuzz target and `labcraft-results-log`.
5. **The scheduler and the interpreter, in deterministic mode:** limits, faults, Stop, Abort by timeout, the trace, the
   report. The corpus of runs starts here, with the cases of scenario 3, and the proof of concept's runner.
6. **`engine`:** the session, the registry and the commands of commands.md 6, the kept `check`, runs as tasks; `cargo
   xtask commands`. `fmt` and `check` move behind it.
7. **`labcraft-cli run`**, through `engine`: options, arguments, settings, exit codes, the human form, Ctrl-C.
8. **Real time:** native calls on the pool, the real clock, `--real-time`; scenario 3, point 2.
9. **`mcp`:** the server, the guide, the scripted loop of scenario 1.
10. **Bounds of M0b and scenario 1 by hand** (five agent sessions), then the tag.

Rust contributions to these crates open when this section is merged (CONTRIBUTING.md).

## 11. M0v: the viewer

*Status: draft; `text` (11.1) and `trace` (11.2) implemented, `diagram` (11.3) up to placement.* How the code of M0v is arranged: the crates `trace`, `diagram` and `editor`,
the module `text` of `types`, the apps `labcraft` and `labcraft-web`, and what `labcraft-cli` gains. The contract
is elsewhere: the [visual notation](visual-notation.md), [`diagram`](cli.md) (section 7), the commands of the
viewer ([commands.md](commands.md), section 6.5, *with M0b*), the trace (commands.md, section 8),
[acceptance scenarios 4 and 6](acceptance-m0.md).

```text
apps/labcraft ──> engine ──> diagram ──> check ──> format ──> graph ──> types
      │                         └──────> trace
      └──> editor ──> diagram, egui (the fork of the suite)
apps/labcraft-web: the same, with the session in a worker
apps/labcraft-cli ──> diagram (diagram --svg), and mcp ──> engine (diagram.svg)
```

| Layer | Crate | In M0v |
|---|---|---|
| L0 | `types` | The module `text`: the metrics of the font and the counting of cells (notation 2.3) |
| L1 | `trace` | The trace of a run (`labcraft-trace/1`): its events, the JSON form of values with its bounded writer, and the bounded reader of cli.md 7.4 |
| L5 | `diagram` | From what `check` gives to shapes: sizes, placement, routing, values, the SVG. Headless, no egui |
| L7 | `editor` | The egui canvas that paints the shapes of `diagram`, read-only in M0v |

The window around the canvas is the app itself in M0v: a list of functions, the canvas, the diagnostics. It is
throwaway, and `ui-egui` starts with the IDE. What lasts is `diagram`, `editor` and the commands.

### 11.1 `text` in `types` (L0)

*Status: implemented (step 1 of 11.7).* The numbers of notation 2.3, low enough for `check` (L3), which judges the
text of an icon by its box (`LC5009`, spec 11.1), and for `diagram` (L5), which places:

- the metrics of Hack 3.003 Regular in units of 2048: advance 1233, ascent 1901, descent 483, capital height
  1493, and the SHA-256 of the file they come from;
- `graphemes(text)` by UAX #29, extended grapheme clusters, and `drawn(grapheme)`, its first 32 code points, all
  that a drawing keeps (a grapheme cut short ends the text that `cut` gives, with `…`); `cells(text)`: two cells for a grapheme one of whose code points is East Asian Wide or
  Fullwidth or `Emoji_Presentation`, or is U+FE0F (notation 2.3), one otherwise; `cut(text, cells)`, in logical
  order, with `…`, which reads no further than the grapheme that passes its limit, and `cut_middle` for the bands
  that would read the same (notation 3.2); `shown(text)`, each control replaced by its sign as a drawing shows it
  (`↵`, `⇥`, or its code, `⟨U+202E⟩`), and `show(text, cells)`, the same cut on those pieces, in runs of one ink,
  a bidirectional control of `LC0005` in `mark.invalid`;
- `width(cells, size)` and `scale`: a length from the font, in hundredths of a unit from a size in hundredths,
  rounded to the nearest hundredth, a half up, the one rounding of every such length (722 for a cell at 12).
  `check` judges the box of a text of an icon with the same constants in floating point, as the other shapes, since
  the size of an icon is a decimal of any precision.

The tables (the grapheme break property, `Indic_Conjunct_Break`, East Asian Width, `Extended_Pictographic` and
`Emoji_Presentation`) are generated by `cargo xtask unicode` from the Unicode Character Database of the version that
names are read with (spec 14.1), rather than taken from a crate, which can lag behind that version: Unicode 18.0
changed rule GB9c. They are committed, with the notice of the Unicode License V3 and their line in `ATTRIBUTION.md`,
and a test checks their version against the spec. The test file of UAX #29 that Unicode publishes with them is
committed too, and every one of its cases passes. A test of `types` reads the font file in `assets/fonts/`, and
checks its SHA-256 and its metrics from its tables; the app, which installs that file, does the same: another
version of the font is a reviewed change of the drawing.

### 11.2 `trace` (L1)

*Status: implemented (step 1 of M0b, section 10.11).* The trace belongs to M0b (commands.md, sections 5 and 8), which
wrote this crate first; it is where the writer and the readers meet, so that what a run writes and what the viewer
reads cannot drift apart, and a test reads back what the writer writes. It depends on no other crate of LabCraft, and on
`sha2` (MIT or Apache-2.0).

```rust
pub trait WriteValue { fn write_value(&self, writer: &mut ValueWriter<'_>); }    // `runtime` implements it
pub fn write_value(out: &mut String, value: &dyn WriteValue) -> Written;          // a value of a report, 64 KiB
impl<W: io::Write> TraceWriter<W> {
    pub fn new(sink: W, header: &Header) -> Self;                                  // the first line
    pub fn event(&mut self, t_ns: u64, event: &Event);                             // `call`, `ran`, `skipped`, `arm`, `set`
    pub fn end(&mut self, t_ns: u64, status: &str);
}
impl Reader {
    pub fn new(expected: Expected, limits: ReadLimits) -> Self;
    pub fn feed(&mut self, bytes: &[u8]) -> bool;                                  // false: the rest is not needed
    pub fn finish(self) -> Trace;
}
pub fn stale<'a>(header: &TraceHeader, files: &[(&'a str, Option<&[u8]>)]) -> Vec<&'a str>;
```

- **Values in JSON** (commands.md 5), with the bounded writer: `$len`, `$head`, `$cut`, 64 KiB and 64 levels. The
  caller drives it, as it knows its values: it gives scalars, and begins and ends arrays and objects; the writer
  decides as it goes, so an array of a million elements costs the 16 it writes, and a caller can stop when it is
  `spent`. `runtime` implements `WriteValue` for its `Value` with its type, and the report of a run uses the same
  writer; `trace` never sees `runtime`. Numbers are written by `trace::number`, the same for the report and the trace:
  the fewest digits that read back the same, decimal from 10^-5 up to below 10^16, `.0` on a float with no
  fraction; durations and the clock exactly, from their nanoseconds.
- **Events** as plain data (`Event`), written one per line to an `io::Write` that the caller gives, with the bounds
  of commands.md 8 (`WriteLimits`): a line of 16 MiB, its values cut, then its entries dropped, then only what
  places it; 1,000,000 events or 256 MiB, then `truncated` and `end`. A sink that fails stops the trace and says why
  (`error`), never the run (section 10.7); `flush` gives the sink what it holds, so that a run can be followed, and
  `end` flushes. The reader reads every line that the writer shortens: such a line cuts the trace, and a value
  whose type it left out is dropped without counting under `LC7104`.
- **The reader** of cli.md 7.4. It takes the bytes of the file in parts, so that it never holds the whole file, and
  cuts at the limits of a file, a line, the number of lines and nesting (`ReadLimits`), with a scanner of JSON that
  does not recurse. The caller gives it, as plain data, what the function drawn has (`Expected`, from `diagram`,
  section 11.3): its inputs; its nodes by name, each with the arms that hold it (`ArmOf`: the `match` or `handle`
  and its arm as the trace writes its pattern) and the loops around it, by index; and its wires by name, each with
  its type as the trace writes it. So the reader decides what fits nothing (`LC7104`) as it reads, and keeps nothing
  for it: a name that no node has, a value of another type than its wire (by the `types` of the `call` of frame 0,
  or of the event), a name that the `arm` events before it do not tell apart from another, an iteration whose
  levels are not the loops around the node or that reaches 2^63, a value given twice, an event of a frame that no
  `call` opened, a value whose text passes the 64 KiB that the writer never passes (commands.md 5). A name shared
  by several nodes is resolved in a few lookups, however many share it: a node becomes a candidate once the `arm`
  of its innermost structure is taken, that structure having been resolved the same way. A line is decoded only as
  far as its frame, whatever the order of its fields: the events of frame 0 are kept, the other frames are skipped. What it keeps is
  an index sorted by node, wire and iterations, a node and a wire as their numbers in what the caller gave
  (`Trace::value`, `Trace::values`); the `arm` and `skipped` events by node and iteration, which choose the tab
  shown and keep the defaults of a skipped node out of sight (notation 6.1); the iterations of each loop within
  those of the loops around it (`Trace::iterations`, and `Trace::values_within` for the values of one outer
  iteration); and the text of the values, as the file gave them: `diagram`
  decodes only the values it shows, one at a time, so that decoding never holds more than one value of at most
  64 KiB. **What it keeps, the index included, is bounded**: 384 MiB *(proposed)*, each entry counted as its
  text, its iterations and 112 bytes, its share of the index and of what an allocator adds, the texts kept in one
  buffer; past it, reading stops as at the other limits (`LC7103`, cli.md 7.4). Measured with a counting allocator,
  the memory really taken at the bound is about 0.65 of what is counted, and 1,000,000 events of three small outputs
  each, the most a run writes, fit; values of 60 bytes each would stop near line 930,000. So its
  memory follows the values of frame 0 that fit the drawing, never the size of the file, the depth of an iteration
  or a hostile number.
- **Its findings are plain data**, and `diagram` makes the diagnostics `LC7101` to `LC7104` from them, with the file
  of the trace, when it adds them to cli.md 5: `Trace::header` (or why the first line gives no values),
  `other_function`, `stop` (the line and the limit), `ignored` and the line of the first. The time and memory of
  cli.md 7.4 are measured by `cargo xtask bounds` once `diagram --trace` reads a trace.
- `stale(header, files)` compares the SHA-256 that the first line gives with the bytes of the files that `diagram`
  resolved to draw the function (notation 6.1); a file gone, or that the first line does not list, is stale too.
  The session gives the bytes and the time of the trace file, which it reads through the `Workspace` of the app
  (section 10.8).

### 11.3 `diagram` (L5)

*Status: steps 1 to 4 below implemented (step 4 of 11.7): the graph with its wires, the sizes, placement, routing,
with the SVG of the frames, the nodes and the wires, their names and their pills, checked by
`tests/harness/src/routing.rs` on every drawing of the repository, on cases built to break the rules
(`tests/harness/routing-cases/`) and, by the fuzz target `diagram`, on the drawings it makes; their time and memory
are bounded by `cargo xtask bounds`. Structures are unfolded, as an export shows them; tabs come with the canvas that
clicks them (step 6).* `draw` takes the index of the file of the
function in `Checked::files`, as `ir::lower` does, and the `Interner`.

What the notation says, computed once, for the canvas and the SVG alike: the canvas paints only what this crate
gives (scenario 6). It reads `check::Checked` and never resolves again: names, callees, the terminals of each call
and the type of each wire are in `Resolved` (section 5). It does not build on `ir`: a function with a type error is
still drawn, with its marks (cli.md 7.1), and `ir` lowers only a run that `check` accepts. A node is named as `check`
names its terminals (cli.md 4.3); the trace names it by its first output (commands.md 8), which `expected` follows.

```rust
pub struct ViewState { standard_everywhere: bool,   // the preference of the IDE (notation 3)
                       unfolded: ..., constructions: ..., optional_inputs: ... }   // what placement depends on (2.1)
pub struct Shown { tabs: ..., iterations: ..., values_hidden: bool }  // what it does not depend on (5.2, 6.1)
pub fn draw(checked: &Checked, function: &FunctionRef, view: &ViewState, limits: &DiagramLimits) -> Drawn;
pub enum Drawn { Drawing(Drawing), NotDrawn(Vec<Diagnostic>) }      // LC7002, or a function that is not drawable
pub fn expected(drawing: &Drawing) -> trace::Expected;              // what the reader keeps (11.2)
pub fn values(drawing: &Drawing, checked: &Checked, trace: &trace::Trace, shown: &Shown) -> Overlay;  // 6.1
pub fn svg(drawing: &Drawing, overlay: Option<&Overlay>, shown: &Shown, theme: Theme) -> String;  // cli.md 7.2
pub fn svg_not_drawn(function: &FunctionRef, diagnostics: &[Diagnostic], theme: Theme) -> String;
```

- **A `Drawing`** is a list of shapes: boxes, paths with their rounded corners, texts in logical order with their
  cells, hatching, marks. Each carries the name of its theme token (notation 8), the densities it shows at
  (notation 7), and what it stands for, in the forms of the reports: the node, the wire, the arm, the code
  (cli.md 7.2). With it come a stable key for each node, from the body that holds it and the name that designates
  it (notation 2.1), so that the canvas can make it glide, and an index of where each node, wire and terminal is,
  for hovering.
- **Every branch is placed.** A structure shown in tabs is as large as its largest branch (2.1, 5.2), so the
  `Drawing` holds the shapes of every branch, and the tab shown is chosen when painting, from `Shown`: picking a
  tab, choosing an iteration (the tab follows the branch the run took, 6.1) or hiding the values places nothing.
- **Coordinates are integers**, in hundredths of a unit, in `i64`: every text is cut before it is measured, and
  `LC7002` bounds the nodes and the terminals, so a text of millions of cells cannot wrap a position. No floating point decides a position, and no map is walked in an order that a
  hash gives: the native and the WASM builds give the same bytes of SVG (scenario 6, point 2).
- **Steps**, each in a module of its own:
  1. **The graph of the function**, from `Resolved`: nodes, terminals in signature order, wires with their
     families, structures and their frames. A value from outside a structure enters by a notch and has one wire
     per frame it crosses, its readers inside sharing the part outside (spec 11). `LC7002` is decided here, before
     anything is placed: the nodes are counted from the text, the terminals as the graph is built, notches
     included, which stops at the first one too many, so that a signature of thousands of inputs called thousands
     of times costs no more than the limit. What a call needs of its callee is worked out once per callee.
  2. **Sizes**: the view of each node (its file, then the preference), compact and standard blocks (3.1, 3.2),
     every text cut in cells by `types::text` (3.2, 6.1, and 40 cells for the others, 2.3), icons, bands,
     badges.
  3. **Placement** (2.1): each body on its own, the innermost first, so that a structure is a node of the size of
     its frame in the body around it; columns, the error chain on one line through every level, stacks ordered by
     four sweeps of barycentres each way, the terminals of each frame, the positions of a `layout` section and the
     unplaced nodes around them. The `Drawing` keeps where each node is, by the key that follows it across a change
     of the file (notation 2.1), with its column and its rank: for the glide, and for the figure of stability.
  4. **Routing** (2.2), `route.rs`, in two passes: a plan on the heights alone, before anything is placed across,
     so that each channel is as wide as its verticals need (their slots, from a tree over their intervals, after
     the constraints of the wires that enter and leave at one height); then, once the columns are placed, a
     polyline per wire and the junction points of each value. Tracks stack on a skyline over channels and columns.
     The names and the pills of the wires, `names.rs`, are placed where they cover nothing, tested against the
     edges of what the body draws, kept merged by line.
  5. **Shapes**, with their tokens, marks and badges.
- **The values of a run** are an `Overlay`, drawn over the `Drawing` and never moving it (6.1): opening a trace
  computes no placement. `values` makes the short forms of 6.1 from the
  text that the reader kept and the type of each wire.
- **The theme**: the values of the tokens, light and dark, are data here, and the test of the contrasts of
  notation 8 reads them. The canvas maps each token to a colour; the SVG writes them as CSS variables.
- **Bounded on purpose.** Every step is near linear: a fixed number of sweeps to lower crossings, intervals
  sorted once for channels and tracks, no search whose cost grows with the square of the nodes. A function of
  20,000 nodes is placed and routed within 2 s and 512 MB, one of 1,000 nodes and 2,000 wires within 1 s
  (scenarios 4 and 6). Above `--max-diagram-nodes`, `LC7002` (cli.md 7.6).
- **Text.** `diagram` cuts texts and counts their cells; it does not reorder them. The canvas and the SVG both
  give a text in logical order, isolated, with a left-to-right base, to an engine that applies UAX #9 (notation
  2.3): epaint of the fork for the canvas, the browser for the SVG. A bidirectional control is replaced here by its
  code, so neither engine sees it.

### 11.4 `editor` (L7)

The canvas that a Craft app can embed (D-27). It depends on `diagram` and on egui only: no engine, no file, no
window. It embeds Hack, and `editor::install_fonts` adds the family of the notation to the fonts of its host.
epaint panics on a family with no face, and fonts that a host sets take effect only at the next pass: so `show`
looks, at each frame, whether the family is there, and when it is not (a host that never called
`install_fonts`, or set its fonts again without it), installs it and paints that frame without text.

```rust
pub struct Canvas { /* the drawing shown, its overlay, the viewport, caches, the glide */ }
impl Canvas {
    pub fn set_drawing(&mut self, drawing: Arc<Drawing>, overlay: Option<Arc<Overlay>>);
    pub fn show(&mut self, ui: &mut egui::Ui, theme: &Theme, shown: &Shown) -> CanvasResponse;  // hover, a click on a tab, a fold, a selector
}
```

- **It paints, and decides nothing of the notation.** Every position, text and colour comes from the `Drawing`.
  What a click asks for (a tab, a fold, an iteration) comes back in `CanvasResponse`, and the host makes it a
  command (`view.set_state`). A fold comes back as a new drawing, like any other; a tab or an iteration only
  changes the `Shown` that the host passes at the next frame, and the overlay that `values` gives for it, so
  nothing is placed again. The tab that a run took is resolved into `Shown` by `values`, from the `arm` events.
- **egui is the fork of the suite**, `storytold/egui` at the revision of `craft-libs`, by `[patch.crates-io]` in
  the workspace. A text is a `LayoutJob` in the family of the notation (Hack, then its fallbacks), at its size
  times the zoom, with `direction: LeftToRight`, in logical order; epaint applies UAX #9, joins Arabic, and sets
  fallback glyphs on the baseline. The canvas never gives it a text already reordered, which would be reordered
  twice. A text that a fallback font makes wider than its cells is drawn smaller, as a whole, to fit them.
- **60 frames/s with 1,000 nodes and 2,000 wires** (notation 7). The shapes are tessellated once per drawing
  and per step of zoom, and kept; panning and zooming within a step is a transform of the layer. egui still copies
  and transforms each mesh it paints, every frame, so only what the viewport shows is painted, found through a
  grid; if that does not reach 60 frames/s, the meshes go to the GPU once, through a paint callback of wgpu, and
  only the transform changes. The galleys of the texts are kept by text and size, and laid out again when a zoom
  ends, so that a text is never shown scaled for long.
- **The glide** (2.1): when a new drawing comes, each node goes from its old place to its new one in about 150 ms,
  found by its key; the new wires show at the end. No glide when the system asks for reduced motion:
  `prefers-reduced-motion` in the browser; on the desktop, where egui does not report it, a preference of the
  viewer, until the platform gives it.
- **Tooltips** come from the index of the `Drawing`: the full name of what a node calls, the value of a wire,
  the values over the iterations of a loop (6.1), each cut at 200 code points (notation 2.3).

### 11.5 The apps

**`apps/labcraft`**, the desktop viewer. eframe with wgpu (DX12, Vulkan or Metal), `craft-ui` for the window
around the canvas, `craft-window`, `craft-settings` for the preferences (standard view everywhere, theme).

- **The session** of `engine` runs on a thread of `format::STACK_SIZE`. It holds the state of the viewer: the
  folder open, the function shown, the view state that the file does not hold, the preference, the trace. The
  window sends it commands through a channel (`view.open`, `view.show_function`, `view.set_standard_view`,
  `view.set_state`, `view.open_trace`, commands.md 6.5, *with M0b*), and reads its state; the drawings come back as
  `Arc<Drawing>`. M0v writes these commands, in a module of `engine`. While a drawing is computed, the one before
  stays on screen; a function that is no longer drawable keeps its last drawing with its diagnostics
  (scenario 6, point 3).
- **The window.** A tree of the files of the folder, which marks a file whose function has a trace open and
  gives the number of errors of a `.lcf` file whose function is not drawn; a tab per function opened; a bar with the trace and
  its status; the canvas; `Problems` with the diagnostics. There is no text panel: the text stays in the
  developer's editor, and the IDE of M1 adds it.
- **Read-only.** Its `Workspace` refuses every write (scenario 6, point 6).
- **Watching.** `notify` (CC0-1.0) watches the folder, the folders of `--lib-path` and the trace shown, without
  following symbolic links (cli.md, section 1); the changes of 50 ms are taken together, then the session checks
  again what they touch and draws the function shown, or reads the trace again when a run has replaced it. File
  change to new drawing within 200 ms (scenario 6, point 3). When the system refuses to watch more (a limit of
  watches), the viewer says so and looks at the files every second.
- **Fonts.** `craft-ui`'s `UiFonts` installs them: Hack first in the family of the notation, Inter Regular for
  the panels, which the developer can change (notation 2.3), then the faces of `craft-fonts` when the build embeds
  them, the fonts of the system and the emoji. egui and eframe are built without `default_fonts`, which
  `cargo xtask layers` checks in the features of the graph. Hack (MIT and the Bitstream Vera licence) and Inter
  (OFL-1.1) are in `assets/fonts/` with their licences, and every face that a build embeds, those of
  `craft-fonts` and the emoji face included, has its line in `ATTRIBUTION.md`, which `cargo xtask assets` checks in
  `ci`.

**`apps/labcraft-web`**, the same viewer in the browser: eframe on WebGPU or WebGL2. The session runs in a Web
Worker, and the page sends it the same commands as JSON. The page keeps the files it gave the worker, so that a
trap of WASM (section 5) restarts the worker with a new session and the same files. The page knows the command
in flight: when a drawing trapped, that function is not drawn again and shows `LC0099` with its last drawing;
when another command trapped (`view.open`, or checking again after a change), the worker restarts once for
that set of files, and a second trap shows `LC0099` for the folder until the files change. A file that traps
cannot restart the worker forever. The drawing crosses to the page in a compact binary form (`postcard`). It opens the
bundled examples and libraries (`libs/embedded`) or a folder dropped on the page, walked with the limits of
section 6 and read once; there is no trace in the browser in M0 (scenario 6, point 7). The fallback faces are the served faces of
`craft-ui` when the build has them; otherwise a character that Hack lacks shows in its cells as a missing glyph.

**`labcraft-cli diagram`** (cli.md 7) calls `diagram` itself and writes the files, with `craft-atomic`: an SVG of
20,000 nodes can pass the 4 MiB of a command result (commands.md, section 2), which `diagram.svg`, the way in of
MCP, keeps. Both give the same SVG, from the same function of `diagram`.

### 11.6 Tests

| What | Where | Scenario |
|---|---|---|
| The metrics, the cells, the Unicode version of the tables, the SHA-256 of Hack | `types` tests, `apps/labcraft` tests | 6 |
| The SVG of each function of `examples/`, `docs/rfc/` and `libs/`, as its file gives it and with every node in standard view, compared byte for byte with a snapshot, native and WASM | `tests/harness` | 6.2 |
| The rules of 2.2, checked on every drawing by a checker of their own, not by comparing with an output: no wire crosses a node, a label, a pill, a band or a wire name, no bubble covers a node, a pill or another bubble, parallel wires 8 units apart, nothing but error wires below the main chain, which no other wire crosses | `tests/harness` | 6.1 |
| The figure of stability: a neutral node on each data wire between two nodes, the nodes that move, and those whose column holds other nodes or another order (2.1) | `tests/harness/tests/figures.rs`, `cargo xtask figures`, in the summary of the job `checks` | 6.3 |
| The size of each example and the zoom at which it fits whole in the canvas of the window (notation 7), a figure without a threshold | the same | — |
| The SVG of `diagram --trace` for the runs of `examples/`, as a snapshot, *with M0b* | `tests/harness` | 6.7 |
| The contrasts of the tokens (notation 8) | `diagram` tests | |
| The seven families of M0a through `diagram`, native and WASM, and Traces through the reader | `tests/harness`, `apps/labcraft-cli/tests` | 4 |
| Maximum nesting through `diagram`: 256 levels on the stack of a worker, as for `format` (section 4) | `tests/harness` | 4 |
| Placing and routing within 2 s and 512 MB, 1,000 nodes within 1 s, reading a trace within 5 s and 512 MB, and `diagram --trace` as a whole within 7 s and 1 GB *(proposed)*, on the corpora and on cases built to cost the most (wires over many columns, deep chains of structures, values kept just under 384 MiB): `cargo xtask bounds` | `tests/harness/tests/bounds.rs` | 4, 6.4 |
| File change to new drawing within 200 ms, through the session and the watcher, without a window | `apps/labcraft/tests` | 6.3 |
| Every example opened, no file changed | `apps/labcraft/tests` | 6.6 |
| Fuzzing placement and routing (target `diagram`, which also checks the rules of 2.2), and the reader of a trace | `fuzz/` | 4 |
| 60 frames/s with 1,000 nodes and 2,000 wires, on a reference machine | by hand, before the tag | 6.4 |

`cargo xtask layers` gets `labcraft-trace` at L1; `wasm` also builds `trace`, `diagram` and `editor`, and passes
the snapshots there.

### 11.7 Order of work

Each step ends with something testable.

1. **`types::text`**, Hack in `assets/fonts`, `cargo xtask assets` and `unicode`; `LC5009` by the box of the text
   (spec 11.1) in `check`.
2. **`diagram`: the graph and the sizes**, compact and standard, with the SVG of the boxes; the snapshots start.
3. **Placement**, with the `layout` section, the frames of the structures, unfolded; the figures of stability and
   of size.
4. **Routing**: the snapshots of scenario 6, point 2, native and WASM; bounds; fuzzing.
5. **`labcraft-cli diagram --svg`** (cli.md 7), and the diagnostics SVG.
6. **`editor` and `apps/labcraft`**: the canvas, the fonts, watching, the glide, read-only; the session of `engine`
   once M0b has merged it.
7. **`apps/labcraft-web`**.
8. **The values of a run** (6.1), once M0b has written `trace` (section 10.11): `diagram::values`, `--trace`, the
   family Traces, the fuzzing of the reader.
9. **Before the tag**: 60 frames/s by hand, 8 hours of fuzzing for each new target.
