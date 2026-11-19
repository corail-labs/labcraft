# Roadmap

*Last updated: 2026-11-19*

The language was described and tried out by a throwaway Python proof of concept, retired after M0 (2026-11-15). M0, the first
milestone in Rust, is done in its three parts, M0a, M0b and M0v ([order of work](architecture.md#9-order-of-work); for M0b, [10.11](architecture.md#1011-tests-and-the-order-of-work); for M0v, [11.7](architecture.md#117-order-of-work)),
with the tags `v0.1.0-m0a`, `v0.2.0-m0b` and `v0.3.0-m0`, the first release with binaries. This page says where the project stands, what the themes and the
releases are, and how the Rust code is organized.

## Where we are: M0 done

| Document or tool | Status |
|---|---|
| Vision ([vision.md](vision.md)) | First draft |
| Decisions ([decisions.md](decisions.md)) | D-01 to D-28, all retained |
| Language and file format ([language-spec-v0.md](language-spec-v0.md)) | First draft with grammar; open questions Q-1 to Q-29 |
| Command line ([cli.md](cli.md)) | `fmt` and `check`, reports, diagnostic codes: implemented for M0a; `run` and `mcp`: implemented for M0b; `diagram` and the viewer: implemented for M0v |
| Architecture ([architecture.md](architecture.md)) | The crates of M0a, M0b and M0v as implemented, and the order of work |
| Rust code (`crates/`, `apps/`, `libs/embedded`, `libs/visa`, `xtask`, `tests/`, `fuzz/`) | M0a: crates `types`, `graph`, `format`, `check`; the declarations of the shipped libraries; `labcraft-cli fmt` and `check`; acceptance corpora, bounds, fuzzing, WASM build. M0b: crates `trace`, `ir`, `runtime`, `engine`, `mcp`, `labcraft-visa`, and that of `examples/results_log`; `labcraft-cli run` and `mcp`; the corpus of runs, the skills of `skills/`. M0v: crates `diagram`, `editor` (read-only), the module `text` of `types`; `labcraft-cli diagram`; the apps `labcraft` and `labcraft-web`; the snapshots of the drawings |
| Visual notation ([visual-notation.md](visual-notation.md)) | First draft; open questions N-1 to N-15 |
| Glossary ([glossary.md](glossary.md)) | Kept with the spec |
| Alternatives, inspirations, Craft ecosystem | First drafts |
| Python proof of concept and diagram rendering (`tools/lcf-poc`, `tools/lcf-figcraft`) | Throwaway, retired after M0 (2026-11-15): Rust replaces them |

Still to write, besides what the acceptance scenarios list under "write first"
([acceptance-m0.md](acceptance-m0.md)):

- **before M1**: written. The model of editing and its commands ([commands.md](commands.md), 9), the notation of
  what editing shows and the gestures ([visual-notation.md](visual-notation.md), 2.4 and 2.6), the window
  ([ide.md](ide.md)), the architecture of editing ([architecture.md](architecture.md), 12) and the acceptance
  scenarios of M1 ([acceptance-m1.md](acceptance-m1.md)), which set the threshold of the figure of stability
  (visual notation, 2.1). The size of a structure is written by dragging its edge (`frame.resize`), step 5 of the
  Rust of M1 ([architecture.md](architecture.md), 12.11).
- **after M1**: outlined. What the themes M4, M3, M2 and M5 hold, what M1 must not block and what must be
  decided before the themes beyond M5 ([outline.md](outline.md)); a theme gets its specification when its work starts.

Feedback on the language matters most at this stage: see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Themes and releases

The roadmap has two kinds of entries (owner's decision, 2026-11-18):

- a **theme** is an area of LabCraft that keeps growing: M1 the editor, M4 real I/O, M3 the panel… The numbers stay
  the names of the themes, which the documents and the issues already use; the documents written before this page
  call them milestones. A theme has no end: what is not needed now stays in it, often as an issue for a contributor;
- a **release** is what a user can do once it ships: a number `0.x.0` (the tag `v0.x.0`, the milestone on GitHub and the
  note of [releases/](releases/)), an **exit criterion** said in what the user does, and a slice of one theme or of
  several. A fix is `0.x.1`. The tags before `v0.4.0` carry the name of a theme (`v0.3.0-m0`); the next ones do not.

An issue has the label of its theme; it gets the milestone of a release only when it is planned in it. The issues with
a theme and no release are the backlog.

A library shipped with LabCraft (in `libs/`) belongs to the theme that gives it its capability: `visa`, `scpi`, `net`,
`serial`, `file` and `bytes` to M4, which brings real I/O; one that needs no new capability
(`math`, `stats`, `csv`, `log`…) to the theme Libraries. A package (D-25), distributed by the package manager and not
shipped with LabCraft, has no theme and no release here: until the package manager of M11, its idea is an issue labelled
`package`, and what it needs from LabCraft (calling C, publishing a package) is an issue of the theme concerned.

**Former names.** M0a, M0b and M0v, done, keep their rows as history; they built the theme Core; M1r is release 0.4.0; M2a and M2b are the theme M2; M7, native compilation, is part of M6. Dated
texts, such as the staging of a decision in [decisions.md](decisions.md), keep the names of their date.

### Themes

M0 crosses all layers, as in SolidCraft: M0a and M0b are a thin vertical slice, and M0v adds the
viewer. M0 comes in three parts so that the diagram can be seen early, before the interpreter
exists: a graphical language is judged first on its pictures. The viewer is read-only, so it freezes
no editing choice; editing waits for M1. It also completes the agent loop: the agent writes the
text, the human watches the diagram change. M0a comes first, since drawing a diagram needs the types
of its wires and the signatures of its nodes; M0b and M0v then run in parallel.

| # | Theme | Content |
|---|---|---|
| **M0a** | Read, check, write (done, tag `v0.1.0-m0a`) | The whole v0 syntax read into the model (graph, layout, icon, comments) and written back in canonical form; names resolved, the signatures of the built-in libraries, type and unit checking with diagnostics, for the v0 language as listed in [spec section 9](language-spec-v0.md#9-what-m0-implements). The declarations of the libraries shipped with LabCraft (`libs/visa`, `libs/scpi`, below) and of the libraries of the examples. Crates `types`, `graph`, `format`, `check`; `labcraft-cli fmt` and `check`. **Criterion:** every canonical file is written back byte for byte, a type error names its node and its wire, and no hostile file crashes the tool |
| **M0b** | Run, agents (done, tag `v0.2.0-m0b`) | The interpreter with its scheduler: what the proof of concept ran (units, structs and enums, arrays, `loop` and `for` with carries, `match`, `handle`), plus function references and live panel elements. The native functions of `libs/visa` and its simulator, so that `scpi`, written in LabCraft, runs. Crates `runtime`, `engine`, `mcp`; `labcraft-cli run` and `mcp`. **Criterion:** an agent writes, runs and fixes the PASS/FAIL multimeter program on its own |
| **M0v** | Viewer (done, tag `v0.3.0-m0`, which closes M0) | A read-only egui canvas that draws a function following the [visual notation](visual-notation.md), compact view by default, with automatic placement when the file has no `layout` section, and redraws when the file changes: the agent writes the text, the human watches the diagram. `labcraft-cli diagram --svg` gives the same drawing as SVG, for tests and agents: the canvas paints only the shapes that `diagram` computes. The same viewer in the browser (WASM). Once M0b is there, the values of a run on the wires, read from the trace that `labcraft-cli run` writes, through the crate `trace` of M0b. Crates `diagram`, `editor` (read-only); apps `labcraft` and `labcraft-web`. `check` judges the text of an icon by its box (`LC5009`, spec 11.1), with the metrics of the font. Target: 1,000 nodes and 2,000 wires, panned and zoomed at 60 frames/s |
| — | Core | Reading, checking, writing, running and drawing a program, which M0a, M0b and M0v built: the parser, `fmt`, `check`, the interpreter and its scheduler, the trace, `diagram`, the command line and the MCP server |
| M1 | Diagram editor | Editing on the `editor` canvas of M0v: placement, wiring, selection, undo, quick insertion of a node by name; tidying a wire, a selection or a whole function again, and a wire routed by hand (`via`); the agent and the human edit the same program, through the MCP server attached to the IDE, which takes the folders an agent may read and write as the other Craft apps do (RFC-0015, [craft-ecosystem.md](craft-ecosystem.md)). Target: 60 frames/s while editing 1,000 nodes |
| M4 | Real I/O | Two transports in pure Rust, TCP and serial, under the `visa` library and as raw bytes (`net`, `serial`) for the devices without VISA; the `file` library (text and bytes); consent for every real resource; bitwise operations, integer division and the `bytes` library, so that a protocol is written in LabCraft, with Modbus TCP as the example; more of the `scpi` library. The other transports of D-17 (VXI-11, HiSLIP, USBTMC, GPIB through an adapter), the binding to an installed VISA and the other buses come after, each a library of its own |
| M3 | Panel | Controls and indicators, graphs, live update, properties; the elements are read in loops, without events (M2) |
| M2 | Full language v1 | Communication: queues and notifiers, the `events` structure, `ref<T>` and its access structure, leaving a `for` early (Q-22). Types and libraries: generic functions, with units (Q-12), `map<K, V>` and `set<T>` (Q-28), shared constants (Q-29), manifests of user libraries and their dependencies |
| M5 | Debugger | Probes, breakpoints, single-stepping, execution animation, value history |
| M6 | Build and targets | What the program becomes and where it runs: a standalone executable (the interpreter shipped with the program), a C ABI DLL, a .NET facade; native compilation by Cranelift, with equivalence tests with the interpreter, when speed needs it, and for the 64-bit embedded Linux targets of D-24; those and the other targets of D-24 come after v1 (horizon 2) |
| M8 | Inbound interop | Calling C DLLs, .NET, Python (`extern` functions) |
| M9 | Plugins | WASM, manifest, permissions |
| M10 | Web | The IDE and simulation in the browser |
| M11 | Packages | Manifest, locking, install from git, then registry (D-25) |
| — | Libraries | The libraries shipped with LabCraft that need no new capability of the runtime: computing, analysis, files read into a type, logging, tests (`math`, `stats`, `signal`, `fit`, `csv`, `json`, `log`, `test`…); many wait for the generics and the units in the signatures of M2 |
| — | Distribution | An installer signed for Windows, an archive or a package for Linux; the macOS archives built and tried by the CI, signed and notarized later (an issue open to contributors). Until the first signed installer, the archives of a release are not signed for the platforms, and a release after `v0.3.0-m0` is planned to add a signed provenance to their checksums |
| — | Tooling | What the project is built and checked with: the CI, `xtask`, the fuzzing, the hooks for agents, the welcome of contributors |

The table is in the order of work. After the editor come real I/O and the panel, before the full
language: a user then drives a real bench or device, first through the agent and the command line, then
through a panel, with the live elements of M0. LabCraft is for more than SCPI benches: a serial or
network device with a protocol of its own, a supervision, a control system, a Raspberry Pi. M4 goes on
in parallel with M1, since it lives in `libs/visa`, the runtime and the CLI.
What M4 to M5 hold is outlined in [outline.md](outline.md), so that the editor blocks none of them.

### Releases

The next two releases are cut in detail; the later ones are one line each, proposed, and their order and numbers may change.

| Release | Exit criterion | Slice |
|---|---|---|
| **0.4.0**, the editor | The scenarios of [acceptance-m1.md](acceptance-m1.md); and someone outside the project installs LabCraft from an installer signed on Windows, which the system does not refuse as coming from an unidentified developer, or from the Linux archive, opens an example, changes it in the diagram and runs it on the simulator | M1 ([acceptance-m1.md](acceptance-m1.md)) and #218, which adds its scenario there; Distribution: the signed installer for Windows, the archives for Linux and macOS. The first release for people outside the project |
| **0.5.0**, the first real bench | A program drawn in the IDE talks to a real device over TCP (Modbus TCP, for example) and to another over a serial port; the same program runs on a `--sim` file without changing a line; an attached agent touches the hardware only after the human has said yes | M4 ([io.md](io.md)): TCP, UDP and serial, `net`, `serial`, `file`, `console`, `bytes`, the bitwise operations and hexadecimal integers, consent, `visa` on TCP and serial, the extended `scpi`, Modbus TCP, #138. Its work goes on in parallel with M1; it is released after 0.4.0 |
| 0.6.0, the bench has its panel | An operator starts a test from a front panel: buttons, values, graphs, Stop | M3 |
| 0.7.0, several loops | Acquisition, HMI and log in loops that talk to each other | M2: communication |
| 0.8.0, libraries and generics | A generic function and a user library with its manifest | M2: types and libraries |
| 0.9.0, the debugger | See the program run and stop it where one wants, the human as well as the agent | M5 |

What a release does not need stays in its theme: for M4, USB and USBTMC, the other buses and transports, Modbus RTU
([io.md](io.md), section 11).

**Horizon 2, after v1:** real-time embedded Linux targets (D-24), controller mode and fieldbuses
(D-26), with timed loops and cyclic tasks (spec 6.4), libraries generated from the Craft apps, LabCraft
as a plugin of a Craft app (D-27), variables shared between programs (D-04). The package manifest
(D-25) is designed from M2, when user libraries appear (the libraries shipped with LabCraft declare
their categories, `always` functions, resources and dependencies from M0), even if the registry
comes later. The WASM build (D-22) is maintained from M0, even if the whole IDE reaches the browser
only in M10.

## M0 acceptance scenarios

Detailed in [acceptance-m0.md](acceptance-m0.md), with the milestone of each one and what it needs
written first. M0a: scenarios 2 and 5, and 4 for `.lcf` and `.lct` files through `fmt` and `check`.
M0b: scenarios 1 and 3, and 4 for runs, instruments, arguments and MCP. M0v: scenario 6, and 4
through the viewer.

1. An agent is given: "Read the voltage of a multimeter over SCPI, check that it is between 220
   and 240 V, return PASS or FAIL." It produces a valid program, runs it against the simulated
   multimeter, and gets PASS. The simulated voltage is changed to 250 V, and it gets FAIL.
2. A program with a type error is rejected with a message that names the node and the wire at
   fault, readable by an agent.
3. Two independent branches run in parallel: their order does not change the result, an error in one
   does not stop the other, a Stop switch ends both loops cleanly, and an abort leaves no session
   open except those it reports as leaked. (Two loops that communicate through a queue: M2, with
   queues.)
4. A hostile file (truncated, absurd values, deep nesting) never crashes the tool.
5. The same program, saved again, produces a byte-for-byte identical file.
6. The viewer draws every example following the notation, the same way every time, and redraws
   within a fraction of a second when an agent rewrites the file.

The Rust `labcraft-cli mcp` passes scenario 1: an agent writes, checks, runs and fixes a new program
through it, five sessions of five ([acceptance-m0.md](acceptance-m0.md), scenario 1).

## Layers of the Rust code

As in the Craft apps: a crate depends only on strictly lower layers, checked by
`cargo xtask layers`. Nothing below L7 depends on egui.

| Layer | Crates | Role |
|---|---|---|
| L0 | `types` | Types, values, physical units, diagnostics |
| L1 | `graph` | Semantic model of the program, separate layout |
| L1 | `trace` | The trace of a run and the JSON form of values: events, a bounded writer and reader (commands.md 5, 8) |
| L2 | `format` | Reading and writing the format, migrations |
| L3 | `check` | Name resolution, signatures (those of the primitives too), type and connection checking |
| L4 | `ir` | Dataflow IR, analyses, linear IR |
| L4 | `edit` | The editing operations on the text of a file, from M1: text in, text out ([architecture](architecture.md), 12.3) |
| L5 | `runtime` | Scheduler, interpreter, values, probes and breakpoints; the native primitives of the language (`core`, `string`, `array`, `errors`, `time`, `ui`) |
| L5 | `diagram` | Automatic placement, wire routing and the visual notation as shapes, headless, with SVG export; run values come in as plain data |
| L5 | `interop` | C ABI, .NET, Python |
| L6 | `engine` | Session, command registry, undo, journal, crash reports |
| L6 | native crates of libraries | The bodies of `extern rust` functions, beside their library: `libs/visa`, `examples/results_log` ([architecture](architecture.md), section 10) |
| L7 | `editor`, `mcp`, `codegen` | Reusable egui diagram canvas that a Craft app can embed (D-27), read-only in M0v and editable from M1; MCP server; Cranelift and building executables and DLLs |
| L8 | `ide` | The window of the IDE, once for the desktop and the web (panels, inspector, later the debugger), on the canvas of `editor` |
| kit | `kit-text`, `kit-text-egui`, `kit-shell` | The pieces of the window that are not about LabCraft, for other IDEs: they depend on no crate of LabCraft ([ide.md](ide.md), 11) |
| apps | `labcraft`, `labcraft-cli`, `labcraft-web` | Desktop, CLI, browser |

### Libraries shipped with LabCraft (`libs/`)

`visa`, `scpi` and later instrument drivers are not part of the language (spec, section 1): they
are libraries, shipped with LabCraft in `libs/`, apart from the crates of the tool. Each is an
ordinary library folder, with its `labcraft.toml` and `.lcf` files, plus a Rust crate when it has
native functions. A native function is declared in a `.lcf` file with `extern rust` (spec 4.1):
the signature is LabCraft, read by `check` and the viewer from M0a; the body is the crate,
registered with the runtime. The rule, checked by `cargo xtask`: **no crate of `crates/` depends
on `libs/`**; only the apps and the test harness link them. The boundary that packages (M11) will use is tried from M0.
When the registry exists, these libraries can be published as packages.

| Library | Content |
|---|---|
| `libs/visa` | Declarations (`extern rust`) and manifest, with `visa.close` marked `always` (written, read by `check` from M0a); the native body: the simulator of `SIM::` resources ([libs/visa/simulation.md](../libs/visa/simulation.md)) and its instrument definitions, in `libs/visa/sim/`; the real transports (M4) |
| `libs/scpi` | Written in LabCraft over `visa` (written, read from M0a): `query_f64`, `check_errors`… The first library shipped in the language itself, and the model for driver authors |

The primitives of the language (`core`, `string`, `array`, `errors`, `time`, `ui`) stay native in
`runtime`: they are the standard library.

## Repository layout

Modeled on SolidCraft and GridCraft; folders appear with the themes that need them:

```text
LabCraft/
├── AGENTS.md            instructions for agents (rules, session start)
├── CLAUDE.md            points to AGENTS.md
├── README.md
├── CONTRIBUTING.md
├── ATTRIBUTION.md       every asset and adapted code
├── NOTICE               trademarks cited (NI, LabVIEW…) only to describe compatibility
├── LICENSE-MIT, LICENSE-APACHE
├── Cargo.toml           workspace, edition 2024, resolver 3, shared lints
├── crates/              the layers above
├── apps/                labcraft, labcraft-cli, labcraft-web
├── kit/                 pieces of the window for any IDE: kit-text, kit-text-egui, kit-shell (M1)
├── libs/                libraries shipped with LabCraft: visa, scpi; embedded, the crate that embeds them
├── docs/                language, notation, architecture, MCP, commands
├── examples/            example programs, in the text format
├── skills/              skills for agents (write, debug, test a program)
├── tests/               acceptance corpora and their harness, backend equivalence tests
├── fuzz/                cargo fuzz targets, on a pinned nightly, outside the workspace
├── xtask/               ci, layers, libs boundary, assets, wasm, command catalog
└── tools/               independent Python calculations of expected values (oracles)
```

## Next

M1, the diagram editor: the specifications listed above come first, then the Rust, on the `editor` canvas of M0v.
