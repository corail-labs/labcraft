# Roadmap

*Last updated: 2026-11-14*

The language is described and tried out by a throwaway Python proof of concept. M0, the first
milestone in Rust, is done in its three parts, M0a, M0b and M0v ([order of work](architecture.md#9-order-of-work); for M0b, [10.11](architecture.md#1011-tests-and-the-order-of-work); for M0v, [11.7](architecture.md#117-order-of-work)),
with the tags `v0.1.0-m0a`, `v0.2.0-m0b` and `v0.3.0-m0`, the first release with binaries. This page says where the project stands, what the milestones
are, and how the Rust code is organized.

## Where we are: M0 done

| Document or tool | Status |
|---|---|
| Vision ([vision.md](vision.md)) | First draft |
| Decisions ([decisions.md](decisions.md)) | D-01 to D-28, all retained |
| Language and file format ([language-spec-v0.md](language-spec-v0.md)) | First draft with grammar; open questions Q-1 to Q-29 |
| Command line ([cli.md](cli.md)) | `fmt` and `check`, reports, diagnostic codes: implemented for M0a; `run` and `mcp`: implemented for M0b; `diagram` and the viewer: implemented for M0v |
| Architecture ([architecture.md](architecture.md)) | The crates of M0a, M0b and M0v as implemented, and the order of work |
| Rust code (`crates/`, `apps/`, `libs/embedded`, `libs/visa`, `xtask`, `tests/`, `fuzz/`) | M0a: crates `types`, `graph`, `format`, `check`; the declarations of the shipped libraries; `labcraft-cli fmt` and `check`; acceptance corpora, bounds, fuzzing, WASM build. M0b: crates `trace`, `ir`, `runtime`, `engine`, `mcp`, `labcraft-visa`, and that of `examples/results_log`; `labcraft-cli run` and `mcp`; the corpus of runs, the skills of `skills/`. M0v: crates `diagram`, `editor` (read-only), the module `text` of `types`; `labcraft-cli diagram`; the apps `labcraft` and `labcraft-web`; the snapshots of the drawings |
| Visual notation ([visual-notation.md](visual-notation.md)) | First draft; open questions N-1 to N-14 |
| Glossary ([glossary.md](glossary.md)) | Kept with the spec |
| Alternatives, inspirations, Craft ecosystem | First drafts |
| Python proof of concept ([tools/lcf-poc](../tools/lcf-poc/README.md)) | Checks and runs the examples against simulated instruments; MCP server driven by an AI agent |
| Diagram rendering ([tools/lcf-figcraft](../tools/lcf-figcraft/README.md)) | Throwaway: renders a function following the visual notation |

Still to write, besides what the acceptance scenarios list under "write first"
([acceptance-m0.md](acceptance-m0.md)):

- **before M1**: the model of editing (when the file is written, what the editor keeps in `layout`), the
  commands of editing, the notation of what editing shows (selection, a wire being drawn), the gestures and the
  keyboard, then the architecture of an editable `editor` and the acceptance scenarios of M1. The threshold of
  the figure of stability (visual notation, 2.1) is set there too: M0 reports the figure only.

Feedback on the language matters most at this stage: see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Milestones

M0 crosses all layers, as in SolidCraft: M0a and M0b are a thin vertical slice, and M0v adds the
viewer. M0 comes in three parts so that the diagram can be seen early, before the interpreter
exists: a graphical language is judged first on its pictures. The viewer is read-only, so it freezes
no editing choice; editing waits for M1. It also completes the agent loop: the agent writes the
text, the human watches the diagram change. M0a comes first, since drawing a diagram needs the types
of its wires and the signatures of its nodes; M0b and M0v then run in parallel.

| # | Milestone | Content |
|---|---|---|
| **M0a** | Read, check, write (done, tag `v0.1.0-m0a`) | The whole v0 syntax read into the model (graph, layout, icon, comments) and written back in canonical form; names resolved, the signatures of the built-in libraries, type and unit checking with diagnostics, for the v0 language as listed in [spec section 9](language-spec-v0.md#9-what-m0-implements). The declarations of the libraries shipped with LabCraft (`libs/visa`, `libs/scpi`, below) and of the libraries of the examples. Crates `types`, `graph`, `format`, `check`; `labcraft-cli fmt` and `check`. **Criterion:** every canonical file is written back byte for byte, a type error names its node and its wire, and no hostile file crashes the tool |
| **M0b** | Run, agents (done, tag `v0.2.0-m0b`) | The interpreter with its scheduler: what the proof of concept runs (units, structs and enums, arrays, `loop` and `for` with carries, `match`, `handle`), plus function references and live panel elements. The native functions of `libs/visa` and its simulator, so that `scpi`, written in LabCraft, runs. Crates `runtime`, `engine`, `mcp`; `labcraft-cli run` and `mcp`. **Criterion:** an agent writes, runs and fixes the PASS/FAIL multimeter program on its own |
| **M0v** | Viewer (done, tag `v0.3.0-m0`, which closes M0) | A read-only egui canvas that draws a function following the [visual notation](visual-notation.md), compact view by default, with automatic placement when the file has no `layout` section, and redraws when the file changes: the agent writes the text, the human watches the diagram. `labcraft-cli diagram --svg` gives the same drawing as SVG, for tests and agents: the canvas paints only the shapes that `diagram` computes. The same viewer in the browser (WASM). Once M0b is there, the values of a run on the wires, read from the trace that `labcraft-cli run` writes, through the crate `trace` of M0b. Crates `diagram`, `editor` (read-only); apps `labcraft` and `labcraft-web`. `check` judges the text of an icon by its box (`LC5009`, spec 11.1), with the metrics of the font. Target: 1,000 nodes and 2,000 wires, panned and zoomed at 60 frames/s |
| M1 | Diagram editor | Editing on the `editor` canvas of M0v: placement, wiring, selection, undo, quick insertion of a node by name; the agent and the human edit the same program. Target: 60 frames/s while editing 1,000 nodes |
| M2 | Full language v1 | Queues and notifiers, events, `ref<T>` and its access structure, `map<K, V>` and `set<T>` (Q-28), generic functions, manifests of user libraries and their dependencies |
| M3 | Panel | Controls and indicators, graphs, live update, properties |
| M4 | Real instruments | Transports of the `visa` library: TCP/IP, serial, USBTMC, HiSLIP; binding to an installed VISA; more of the `scpi` library |
| M5 | Debugger | Probes, breakpoints, single-stepping, execution animation, value history |
| M6 | Build | Standalone executable, C ABI DLL, .NET facade |
| M7 | Native compilation | Cranelift, equivalence tests with the interpreter |
| M8 | Inbound interop | Calling C DLLs, .NET, Python (`extern` functions) |
| M9 | Plugins | WASM, manifest, permissions |
| M10 | Web | The IDE and simulation in the browser |
| M11 | Packages | Manifest, locking, install from git, then registry (D-25) |

**Horizon 2, after v1:** real-time embedded Linux targets (D-24), controller mode and fieldbuses
(D-26), libraries generated from Craft apps and SolidCraft plugins (D-27). The package manifest
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
| L5 | `runtime` | Scheduler, interpreter, values, probes and breakpoints; the native primitives of the language (`core`, `string`, `array`, `errors`, `time`, `ui`) |
| L5 | `diagram` | Automatic placement, wire routing and the visual notation as shapes, headless, with SVG export; run values come in as plain data |
| L5 | `interop` | C ABI, .NET, Python |
| L6 | `engine` | Session, command registry, undo, journal, crash reports |
| L6 | native crates of libraries | The bodies of `extern rust` functions, beside their library: `libs/visa`, `examples/results_log` ([architecture](architecture.md), section 10) |
| L7 | `editor`, `ui-egui`, `mcp`, `codegen` | Reusable egui diagram canvas that a Craft app can embed (D-27), read-only in M0v and editable from M1; IDE (window, panel, debugger); MCP server; Cranelift and building executables and DLLs |
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

Modeled on SolidCraft and GridCraft; folders appear with the milestones that need them:

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
├── libs/                libraries shipped with LabCraft: visa, scpi; embedded, the crate that embeds them
├── docs/                language, notation, architecture, MCP, commands
├── examples/            example programs, in the text format
├── skills/              skills for agents (write, debug, test a program)
├── tests/               acceptance corpora and their harness, backend equivalence tests
├── fuzz/                cargo fuzz targets, on a pinned nightly, outside the workspace
├── xtask/               ci, layers, libs boundary, assets, wasm, command catalog
└── tools/               the throwaway Python tools, until Rust replaces them
```

## Next

M1, the diagram editor: the specifications listed above come first, then the Rust, on the `editor` canvas of M0v.
