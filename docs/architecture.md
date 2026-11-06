# Architecture: M0a

*Last updated: 2026-11-04. Status: first draft. M0b and M0v add their sections when their milestones
are prepared.*

How the Rust code of M0a is arranged: the crates `types`, `graph`, `format` and `check`, the app
`labcraft-cli`, and `xtask`. It is written for whoever implements them, person or agent. The contract
to meet is elsewhere: [spec](language-spec-v0.md), [command line](cli.md),
[acceptance scenarios 2, 4, 5](acceptance-m0.md), [corpora](../tests/acceptance/README.md). The layers
are in the [roadmap](roadmap.md). The code sketches below show shapes, not final signatures.

The layout is the one shared by the Craft apps (SolidCraft, GridCraft, VectorCraft, FigCraft, DeckCraft,
WordCraft): `crates/`, `apps/<app>`, `apps/<app>-cli`, `apps/<app>-web`, `xtask/`, with a crate
`format`, then `engine`, `mcp` and `ui-egui` above it. GridCraft's `formula` crate (lexer, parser,
printer, by hand) is the nearest thing to our `format`. What the suite already provides is used, not
rewritten (section 7).

## 1. Principles

- **Only `labcraft-cli` touches the file system.** `format` and `check` take bytes and return values.
  That is what makes them run in WASM (D-22), in the viewer and in tests with no setup.
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

## 2. `types` (L0)

What everything needs, with no knowledge of files.

- `Name`: an identifier or unit name, always normalized (R-20): `Interner::intern` is the only way to
  make one. It is an interned `u32` from an `Interner` the caller owns: a file of 16 MiB can hold
  8 million occurrences of a name, and a pointer per occurrence would threaten the memory bound.
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
  diagnostics while they arrive, with complete counts, since a file of 16 MiB can hold millions of
  errors; which ones it keeps does not depend on when it prunes. The app calls `finish` last, once all
  folders and `LC0009`, `LC0010` are merged.
- `Limits`: file size, total size of a library (1 GiB *proposed*), nesting, report, with their defaults.
  `max_nesting` has a hard ceiling (cli.md, section 1), lower in WASM: a stack is sized from it (section 4).

Run-time values come with M0b.

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
pub struct Arm { id: Id, comments: Attached, pattern: Pattern, open: Option<Comment>, body: Body,
                 yields: Yield }
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
  `check` builds what it needs. Bodies stay trees of statements until the `ir` crate of M0b.

## 4. `format` (L2)

```rust
pub fn read(bytes: &[u8], names: &mut Interner, limits: &Limits) -> Read;  // file, source map, diagnostics
// `Name` is an index with no order, and numbers are names too: sorting and printing need the interner.
pub fn canonical_order(body: &Body, signature: Option<&Signature>, names: &Interner) -> Result<Vec<usize>, Unorderable>;  // 5.2, public: M0b needs it (8); the signature, for the body of a function
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
                      libraries: Vec<DeclaredLibrary> }            // what each library declares, by name
pub struct CheckedFile { shown: String, file: File, source_map: SourceMap, resolved: Resolved }
pub struct DependencyReport { shown: String, report: Report }      // merged once per `shown`
```

`Checked` is what the layers above read, so that they never resolve again: the viewer draws wires with
their types, and M0b runs from it. `Resolved` gives, by `Id`:

- what each path is (value, field, variant, category, panel element, function, unit), in a table that holds one
  small `Meaning` per `Id`, since a file can hold millions of operands;
- the kind of each `Bracket` (R-5);
- the callee of each call, and the terminals of that call, which a signature alone does not give for
  `string.format` and variadics;
- the type of each wire, and the coercions to show (3.3, 3.4), with pass 4.

`root` includes its sub-libraries, which are checked even if nothing uses them (cli.md, section 3).
`Limits` bounds the total size of a library, since the caller hands all its files at once.

Passes, each one over all the files, so that a bad file does not hide the others:

1. **Load:** `format::read` on each file; the set of libraries through `Libraries`, with cycles
   (`LC2021`). A file that does not parse is marked, and what it declares is known as broken.
2. **Declarations:** functions and types per library, duplicates, R-1, R-19, bad types.
3. **Bodies:** names resolved in scopes (inputs, carries, patterns, lines), `use`, calls bound to
   signatures, labels, `after:`, the bounds of each number of a body or a default value (those of a `panel` come with
   pass 6). Only what can be told without a type: a field of a value and
   the values of a variant wait for pass 4, and so do the fields of a `string.format` template (`LC2106`). Only for the
   libraries being checked: of a dependency, the others read the signatures.
4. **Types and units:** wire by wire, in dependency order, with `Ty::Error` to stop cascades.
5. **Dataflow:** cycles, outputs bound once, carries; then the warnings of 7.5, 8.1 and 10.
6. **Panel, layout, icon:** kinds and `default:`, references to nodes, coordinates, the icon (R-14: grid,
   a validator of the SVG path syntax, a table of `accent` tokens).

Types across files (`.lct` chains, `LC2017`) and calls between functions (`LC2016`) are walked with an
explicit stack. A function is checked against the signatures of the others, never by following a call.

**Concurrent access (8.1)** gives one warning per resource, not one per pair. Sort the nodes that receive
the resource in a topological order; all of them are ordered if and only if each one reaches the next.
Only consecutive pairs are tested, and a path between them can only pass through nodes between them in
that order, so the searches of one resource cover disjoint windows: linear in total. Two nodes in
different arms of a `match` or `handle` never run together and do not count (8.1). A budget caps the
total work across resources, so that many resources cannot make it quadratic again.

**Signatures of the primitives** (`core`, `string`, `array`, `errors`, `time`, `ui`) live here as data,
in the form of appendix A, with a rule attached where a signature cannot say it all: the units of `mul`,
the template of `string.format`, the variadic `errors.merge`. A test compares this table with appendix A. Their types
and rules join it with pass 4. `runtime` (M0b) depends on `check` and tests that each signature has a body. The bodies
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
- `fmt` and `check` become engine commands when `engine` arrives (M0b, D-18), and the last resort moves
  to `Session::execute`, as in the Craft apps. Until then the app calls the library functions directly;
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
| `ci` | Everything below but `fuzz-licences`, plus `cargo fmt --check`, `clippy -D warnings`, tests |
| `layers` | A crate depends only on lower layers, and directly on no UI toolkit below L7, from `cargo metadata` and a table, as in GridCraft. A crate missing from the table is an error |
| `licences` | No GPL, LGPL or AGPL dependency (D-23), on any platform, tests and tools included; a licence that cannot be read is an error. Refusing is ours: SolidCraft's command of that name lists the licences for a release |
| `libs` | No crate of `crates/` depends on `libs/` |
| `codes` | `Code` equals the tables of cli.md, section 5 |
| `wasm` | `format` and `check` build for `wasm32-unknown-unknown`, and pass the corpora there, on the call stack of V8 in a worker (section 4); some test must have run |
| `fuzz-licences` | The same as `licences`, for `fuzz/`, which is outside the workspace, and refuses a stale `fuzz/Cargo.lock`. Not part of `ci`: the job `fuzz` runs it (section 8) |

**Dependencies** of the crates, kept few and permissive: `unicode-normalization` (R-20),
`unicode-properties` (the Unicode categories of 14.1), `serde` and `serde_json` (reports), `toml` (manifests of shipped libraries),
`thiserror` for the error type of a crate, where a function can fail on something other than the program,
such as a file that cannot be written (the versions the Craft apps use). In tests: `wasm-bindgen-test`,
`cargo-fuzz`. `assets` joins `xtask ci` with the first asset.

**From the suite.** `craft-atomic`, from [`craft-libs`](https://github.com/storytold/craft-libs)
(MIT or Apache-2.0), pinned to a revision as the apps do, is the only shared crate M0a needs, and only in
`labcraft-cli`. The others wait for what they serve: `craft-ui`, `craft-window` and `craft-keymap`
for the desktop viewer (M0v), `craft-i18n` for a translated interface. Messages of the tool stay in
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
| Fuzzing the parser, `check` and the round trip | `fuzz/` | 4 |

`cargo fuzz` needs Rust nightly and works poorly on Windows, so `fuzz/` stays out of the workspace: the
product builds and ships on stable, and a separate CI job fuzzes on Linux with a pinned nightly. That job is
the workflow `fuzz`, next to `checks`, which knows nothing of it and stays the only check GitHub requires on
`main`; a red `fuzz` still stops the merge unless the owner decides otherwise for that pull request.
`fuzz/README.md` says how to run a target and what becomes of a crash.

The workflow `checks` also has a job `windows`, which runs the tests of `labcraft-cli` and `labcraft-libs`: paths,
walking folders and atomic writes differ there, and the crates are the same. GitHub does not require it either, and
a red one stops the merge in the same way.

The proof of concept remains the reference for what the examples do until M0b replaces it; it does not
judge types, where the corpus decides.

## 9. Order of work

Each step ends with something testable, and can be one or a few pull requests.

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

Rust contributions open at the end of M0a, but steps 7 to 9 divide well along the codes of cli.md if
someone offers earlier.
