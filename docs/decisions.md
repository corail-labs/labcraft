# Decisions

*Last updated: 2026-11-08*

Each decision has a status. D-01 to D-28 are all accepted; the other two statuses are for future
entries:

- **Accepted**: accepted by the project lead (date given). It is reopened only with a new fact.
- **Proposed**: a reasoned recommendation, to be validated.
- **Open**: to be settled by the project lead; the recommendation is only an opinion.

This document is public: it is the project's decision register. A new decision, or a change to an
accepted decision, goes through a public RFC (a GitHub issue with the `rfc` label), then through an
entry here, in the pull request that closes the issue.
LabCraft publishes its decisions, because the language is designed with its users.

## Summary

| # | Decision | Status | Recommendation |
|---|---|---|---|
| D-01 | Ambition and compatibility | Accepted | Equal to LabVIEW, no `.vi` compatibility |
| D-02 | Implementation language | Accepted | Rust |
| D-03 | Graphical interface | Accepted | egui / eframe + craft-libs |
| D-04 | Execution model | Accepted | Dataflow by value + structures + events; native error type |
| D-05 | Type system | Accepted | Static, inferred, physical units from v0 |
| D-06 | Code unit (the equivalent of the VI) | Accepted | Function; optional attached panel |
| D-07 | File format | Accepted | Text, one file per function, layout in a separate section |
| D-08 | Text syntax | Accepted | The format is a real, readable text language |
| D-09 | Intermediate representation | Accepted | Own dataflow IR, then a linear IR |
| D-10 | Execution strategy | Accepted | Interpreter first, Cranelift next, LLVM as an option |
| D-11 | Memory management of values | Accepted | Reference counting + copy on write + in-place analysis |
| D-12 | Scheduler | Accepted | Own scheduler on a thread pool |
| D-13 | Panel at run time | Accepted | Same egui engine in the IDE and in executables |
| D-14 | Calling external code | Accepted | C ABI, .NET, Python; out-of-process as an option |
| D-15 | What LabCraft builds | Accepted | Exe, standalone C ABI DLL, .NET facade, WASM |
| D-16 | Plugins | Accepted | WebAssembly (component model, WIT) |
| D-17 | Instruments | Accepted | VISA layer in Rust + simulators + binding to an installed VISA |
| D-18 | AI integration | Accepted | Commands, CLI, MCP, control channel |
| D-19 | Licence | Accepted | MIT OR Apache-2.0; contributions "inbound = outbound", no DCO or CLA |
| D-20 | Place in the ecosystem | Accepted | Independent project for now; storytold or tangency-labs not excluded |
| D-21 | Language | Accepted | Public repository in English |
| D-22 | Platforms | Accepted | Windows, Linux, macOS, web |
| D-23 | Clean room rules | Accepted | Take those of SolidCraft, adapted to NI |
| D-24 | Embedded, real time, FPGA | Accepted | Planned after v1; Cranelift for 64-bit Linux, LLVM or C for the rest |
| D-25 | Package manager | Accepted | Cargo model: manifest, lock file, git registry |
| D-26 | Alternative to PLCs (IEC 61131-3) | Accepted | Cyclic execution mode, fieldbuses, PLCopen XML; no SIL |
| D-27 | Libraries from the Craft apps, SolidCraft plugins | Accepted | Nodes generated from a command registry; plugin = WASM component; editor embeddable in an app (2026-11-02) |
| D-28 | Test executives | Accepted | LabCraft is not a sequencer; test executives can call it (DLL, .NET, native adapter) |

---

## D-01 — Ambition and compatibility

**Status:** accepted (ambition on 2026-10-30, no official converter on 2026-10-31).

- **Ambition:** do at least as much as LabVIEW. The project is not a reduced tool.
- **`.vi` compatibility:** no. The format is closed and documented nowhere; reverse engineering it
  raises a licence problem (NI EULA) and would make the project carry the choices of someone else.
- **Migration path: no official tool.** The NI licence (January 2025 version, section B) forbids
  discovering the "non-literal aspects" of the software by reverse engineering, file formats
  included (clause v), and using LabVIEW's scripting functions to create an application that acts
  as the editor of a graphical programming environment (clause xi). A `.vi` to LabCraft converter
  based on VI Scripting would be, at best, in a grey area. None is built without legal advice.
  Migration will be done by rewriting, helped by AI from what the user reads in their own
  programs.

## D-02 — Implementation language

**Status:** accepted.

| Option | Pros | Cons |
|---|---|---|
| **Rust** | Memory safety without a garbage collector, native, WASM, an ownership model close to the by-value semantics of dataflow, Cranelift written in Rust, Craft ecosystem | Learning curve, dynamic plugins are trickier |
| C# / .NET | Mature industrial ecosystem, trivial .NET interop | Garbage collector in the runtime of programs, .NET runtime to embed, WASM less natural |
| C++ | Native LLVM, the reference in the field | Memory safety, maintenance cost |

Rust is accepted for the engine, the compiler, the runtime and the IDE.

## D-03 — Graphical interface

**Status:** accepted (egui), points of attention noted.

| Option | Opinion |
|---|---|
| **egui / eframe** | Accepted. Immediate mode suited to an interactive canvas, wgpu rendering, WASM, used by the whole Craft ecosystem (`craft-ui` already provides docking, tabs, icons, themes) |
| Avalonia (.NET) | Serious for a business app; imposes a Rust/.NET boundary to maintain |
| Slint | Declarative, good for embedded; less suited to a free-form graph editor |
| Iced, Xilem | Rust, but less mature for an IDE of this size |
| Tauri + web | Great UI richness, but two languages and canvas rendering in JS |

**Points of attention:**

- With thousands of nodes and wires, cache the canvas geometry (wire tessellation) instead of
  recomputing everything on every frame. Measured early: it is a target of the M0v viewer (see
  D-12 and [roadmap.md](roadmap.md)).
- egui changes version often: versions are pinned, as in craft-libs.
- The graph editor has to be written. `egui-snarl` and `egui_node_graph` serve as references, not
  as default dependencies.
- Accessibility: AccessKit has been built into egui with no option since 0.34 (checked on
  2026-11-01). The painter emits no node: each accessible element of the canvas (node, terminal,
  selected wire) allocates its own response with a stable identifier and a `Role`. With thousands
  of nodes: expose only the visible or selected elements, plus a list or tree view of the program
  for screen reader navigation. Support on the web target remains to be checked.

## D-04 — Execution model

**Status:** accepted on 2026-10-30.

- **Dataflow by value.** A node runs when all its inputs are available; a wire carries a value,
  not a reference. This is what makes parallelism safe and natural.
- **Structures:** `loop`, `for` (with automatic indexing), `match`, carries, events (planned, spec
  6.4). No sequence structure or feedback node: order comes from data, the error wire or `after:`.
- **Communication between loops:** queues, notifiers, typed shared variables.
- **Explicit references** (the equivalent of DVRs) for the rare cases where sharing is wanted:
  visible in the type, never implicit.
- **Error handling: native error type** (accepted, preferred over the LabVIEW error cluster and
  over exceptions).
  - **Automatic propagation**: a node that receives an error on its input is skipped and passes
    it on, with no `match` to write in every function. Some nodes are marked `always` (closing a
    session, switching off a power supply).
  - **The error wire stays visible** and also orders side effects (sending SCPI commands in the
    right order).
  - **An error is a structured value**: a hierarchical category (`io.timeout`,
    `visa.resource_not_found`, `scpi.instrument_error`, or a category declared by a library, for
    example `dmm.overload`), a numeric code that keeps the original codes (VISA, `SYST:ERR?`), a
    message, and the chain of origin (node, function, callers), readable by an agent.
  - **Handling by error type**: `handle` (spec 7.3), with branches by category (with wildcards,
    `visa.*`) or by code, plus an "others" branch. A handled error is cleared; an unhandled error
    continues. Typical cases: retry on `io.timeout`, change range on `dmm.overload`, pass the rest
    on.
  - **Error never handled**: a warning at edit time if an error output is wired nowhere; at run
    time, the function stops and the error is reported (never a dialog box).
  - Several simultaneous errors (parallel branches): a merge node, `errors.merge`; the first error
    is the main one and the others are kept (spec 7.4).

## D-05 — Type system

**Status:** accepted on 2026-10-30 (physical units from v0; implicit widening, explicit
narrowing).

- Static typing, checked during editing, with inference on wires.
- Types: booleans, signed and unsigned integers of all sizes, floats, complex numbers, strings,
  N-dimensional arrays, clusters (structures), enumerations, tagged unions, waveforms, timestamps,
  references, defined types (the equivalent of *typedefs*).
- Polymorphism: generic functions rather than hand-written "polymorphic VIs".
- **Physical units in the types, from v0** (V, A, Hz, s, °C…): adding volts and amperes is an
  error detected during editing. A value without a unit remains the default case: those who do
  not use units are not bothered. Adding them afterwards would be very hard, hence their presence
  from the specification. SolidCraft already has expressions with units. To be detailed in the
  specification: prefixes (mV, kHz), derived units (V/A = Ω), temperatures (°C and K do not
  convert by a simple factor), display on wires.
- **Numeric conversions**: widening is implicit and shown visually (16-bit integer to 32-bit or to
  float, like the LabVIEW coercion dot); narrowing is explicit (float to integer, 64 to 32 bits: a
  conversion node is mandatory, because information can be lost).
- Classes and interfaces (the equivalent of LVOOP): planned in the model, delivered later.

## D-06 — Code unit (the equivalent of the VI)

**Status:** accepted on 2026-10-30: option B.

In LabVIEW, every VI necessarily has a front panel, a diagram and a connector.

| Option | Pros | Cons |
|---|---|---|
| A. Like LabVIEW: every VI has its panel | Familiar to LabVIEW users | Thousands of useless panels in libraries |
| **B. Function (signature + diagram), panel attached as an option** | Simpler for libraries and for AI, the panel remains available | Departs from the LabVIEW habit |

**Decision: B.** The code unit is the function (signature + diagram). A panel can be attached to
it; it then behaves like a VI. Libraries have no useless panels, and an agent writes a function
without worrying about an interface.

## D-07 — File format

**Status:** accepted on 2026-10-30 (one file per function, layout in a separate section of the
same file).

- **Text**, UTF-8 encoding, deterministic order so that differences in git are readable.
- **Semantics and layout separated, in the same file**: the text program at the top (D-08), a
  layout section at the end (node positions, wire routes). Moving a node changes only this section
  in a diff. An agent can write the file without a layout section; the IDE then computes an
  automatic placement. Rejected option: a sidecar file, which would require keeping two files in
  sync (renaming, moving, merging).
- **Stable identifiers** for every node and every wire, for merging and for references (their form
  is open question Q-10 of the spec; corrected 2026-11-02).
- **Format version** in the file (`labcraft 1`; corrected 2026-11-02), with successive migrations
  and a clean refusal of newer formats.
- **One file per function** (simpler to merge); a library = a folder with a manifest, which also
  serves as the basis of the package (D-25).

## D-08 — Text syntax

**Status:** accepted on 2026-10-30: option B. This is the most structuring decision for AI.

| Option | Pros | Cons |
|---|---|---|
| A. Raw JSON of the graph | Trivial to write and to validate | Verbose; a model writes graphs badly with identifiers everywhere |
| **B. A real text language, equivalent to the graph in both directions** | An agent writes compact, readable code; readable diffs and reviews | The syntax must be designed and the graph ↔ text round trip guaranteed |
| C. No text: AI goes only through commands | A single API surface | The agent cannot "read" a program at a glance |

**Decision: B.** The file format is a real text language, equivalent to the graph in both
directions. Commands (option C) remain the fine-grained editing surface, since they exist anyway
(D-18). Enso is the reference for this dual nature (see [inspirations.md](inspirations.md)).

Consequences:

- The language specification and the format specification become one: the text syntax *is* the
  format (the graphical layout stays in a section at the end of the file, D-07).
- Test requirement: text → graph → text gives back the same text, byte for byte; graph → text →
  graph gives back the same graph.
- Parallelism must stay visible in the text: two independent branches must not look like a
  sequence of statements (see the LabVIEW / Blueprints difference in
  [inspirations.md](inspirations.md)).

## D-09 — Intermediate representation (IR)

**Status:** accepted on 2026-10-31.

Three levels, a common shape for dataflow compilers:

1. **Source graph** (the file).
2. **Dataflow IR**: typed graph, explicit structures, visible parallelism. The analyses specific
   to dataflow are done here: buffer allocation, in-place operations, grouping of nodes into
   schedulable blocks.
3. **Linear IR** (SSA form): what the interpreter and the code generators consume.

The dataflow IR is versioned and documented: third-party tools and agents can inspect it.

## D-10 — Execution strategy

**Status:** accepted on 2026-10-31.

| Step | Backend | Role |
|---|---|---|
| 1 | **Interpreter** of the linear IR | Execution in the IDE, debugging (probes, breakpoints, highlighted execution), web |
| 2 | **Cranelift** (JIT and AOT) | Native code that is fast to produce: it can be recompiled continuously during editing, like LabVIEW |
| 3 | LLVM (optional) | Maximum optimization for release builds, if measurements justify it; needed for 32-bit ARM and microcontrollers (D-24) |
| — | Rust or C source generation | Rejected as the main backend: requires a compiler on the user's machine and slow builds. A serious option for microcontrollers (D-24) |
| — | C# source generation | Rejected: it is a second full compiler, with a risk of semantic divergence |

Most test programs spend their time waiting for instruments; heavy computation is in native
primitives (arrays, signal, analysis). A good interpreter with native primitives therefore covers
most needs before the native compiler.

**Rule:** the same set of equivalence tests runs on all backends. A difference is a bug.

## D-11 — Memory management of values

**Status:** accepted on 2026-10-31.

By-value semantics requires copies; avoiding them is what makes performance.

- Reference counting with copy on write: an array is copied only if it is shared at the moment it
  is modified.
- Static analysis on the dataflow IR to reuse buffers in place (what LabVIEW calls
  *inplaceness*).
- Reference: Lean 4, "Counting Immutable Beans" (see [inspirations.md](inspirations.md)).
- Rust's ownership model helps to write a correct runtime on this point.

## D-12 — Scheduler

**Status:** accepted on 2026-10-31.

- Nodes are grouped into blocks (LabVIEW's *clumps*) run on a thread pool.
- Own scheduler rather than `tokio` or `rayon` as they are: priorities, debugger stepping,
  breakpoints and clean program shutdown must be under control.
- Blocking I/O (instruments, files) must not block the other loops: to be designed (dedicated
  threads or async I/O under the scheduler).
- Determinism: a reproducible execution mode (fixed order, simulated clock) for tests and agents.
- Plan from the design stage for two other modes, even if they come later: timed real-time loops
  (D-24) and PLC-style cyclic execution (D-26).

## D-13 — Panel at run time

**Status:** accepted on 2026-10-31.

- The same egui rendering engine is used in the IDE and in the built executables.
- Controls and indicators: numeric, boolean, string, graphs, strip charts, tables, tabs, lists,
  gauges; decorations; resizable layout.
- Update from the running code: a channel between execution threads and the interface thread, with
  rate limiting for fast graphs.
- Properties and methods of controls reachable from the diagram (the equivalent of *property
  nodes*), as typed commands.

## D-14 — Calling external code

**Status:** accepted on 2026-10-31.

| Target | Means | Note |
|---|---|---|
| C / C++ DLL | C ABI, dynamic loading, declaration of signatures | Equivalent of the *Call Library Function Node* |
| .NET (modern) | Hosting the CLR from Rust (`hostfxr`, `netcorehost` crate), type discovery by reflection | Constructor, method, property nodes |
| .NET Framework | Windows only, later | Many instrument SDKs are still in .NET Framework |
| Python | In a separate process: LabCraft does not embed Python (corrected 2026-11-02) | Access to numpy, scipy, vendor libraries |
| Separate process | Option for everything else | An unstable DLL does not bring the IDE down |

## D-15 — What LabCraft builds

**Status:** accepted on 2026-10-31.

Recommended order:

1. **Standalone executable** with panel, embedded runtime.
2. **Native DLL with a C interface**, runtime statically linked: usable from C, C++, C#, Python,
   Rust, with nothing to install, unlike LabVIEW-built DLLs, which require the Run-Time Engine.
3. **Automatically generated .NET assembly**: a thin P/Invoke layer on top of the C ABI DLL. Little
   work, cross-platform with modern .NET.
4. **WebAssembly module.**

Rejected: generating real, independent C# code (see D-10).

## D-16 — Plugins

**Status:** accepted on 2026-10-31.

- Plugins in WebAssembly (component model, versioned WIT interface), as SolidCraft plans
  (`docs/plugins.md` in the SolidCraft repository).
- Plugins act only through commands: undoable, logged.
- Permissions declared in a manifest; memory and compute limits.
- Drivers that require native access (USB, vendor DLL) go through D-14, or through a library
  shipped with LabCraft whose native functions are `extern rust` (spec 4.1, roadmap `libs/`), not
  through WASM plugins.

## D-17 — Instruments

**Status:** accepted on 2026-10-31.

- **VISA layer in Rust**: raw TCP/IP (port 5025), VXI-11, HiSLIP, USBTMC, serial, GPIB through an
  adapter. NI-VISA is not needed for most modern instruments.
- **Optional binding to an installed VISA** (NI-VISA, Keysight, R&S) through its public C API.
- **SCPI**: helper library (commands, response parsing, `SYST:ERR?` errors).
- **Simulated instruments** described in files (responses, delays, noise, faults), in the spirit
  of `pyvisa-sim`. Essential for the AI loop (see [vision.md](vision.md)).
- **NI-DAQmx acquisition**: through its public C API, later. Check the redistribution terms of the
  NI runtime.
- Industrial buses later: CAN, Modbus, OPC UA, MQTT.

## D-18 — AI integration

**Status:** accepted.

Taken directly from the SolidCraft model:

- Command registry: each action has an identifier, a description, a parameter schema.
- `labcraft-cli`: `run`, `check`, `eval`, `mcp`. `run` runs a function of a program, as `cargo run`
  does; a script of commands, which `solidcraft-cli run` runs, is `labcraft-cli script`
  (*proposed*). A precision, not a change: the list above never said what `run` runs. Owner's
  decision, 2026-11-08.
- MCP server: headless (internal session) or connected to the open IDE through a local control
  channel with a token, so that the human can watch the agent work.
- MCP tools: create and edit the graph, validate, run, read probes and traces, drive the simulated
  instruments, capture the panel or the diagram.
- No dialog box during a programmatic call: default parameters or an error.
- Later: an assistant in the IDE, through `craft-ai-integrations`.

## D-19 — Licence

**Status:** accepted on 2026-10-31.

MIT OR Apache-2.0, at the user's choice, like SolidCraft and the storytold apps. Programs written
by users belong to them, without restriction.

**Contributions: "inbound = outbound"**, like SolidCraft (neither DCO nor CLA in the Craft
ecosystem). `CONTRIBUTING.md` states it: *"By contributing you agree that
your contribution is licensed as MIT OR Apache-2.0, like the rest of LabCraft."* Each contributor
keeps their rights: the licence of contributions cannot be changed. No CLA. The DCO
(`Signed-off-by:` on each commit) remains possible later, if a host organization requires it.

**What users ship** (added on 2026-11-08). The runtime goes into everything a user builds (D-15): an
executable, a library with a C interface, a .NET assembly or a WASM module. With it go the MIT and
Apache-2.0 licence texts, and those of its dependencies. No runtime exception is added to the licence:
`labcraft build` will generate these notices and join them to what it builds, so that the user has
nothing to do.

## D-20 — Place in the Craft ecosystem

**Status:** accepted on 2026-10-30: option A, for now.

| Option | Consequence |
|---|---|
| **A. Independent project, Craft conventions** | Like SolidCraft (tangency-labs): total freedom, then a request to join the launcher catalog as a "community app" |
| B. Offer LabCraft to storytold | Visibility and shared infrastructure; governance no longer belongs to the project lead |
| C. Independent without Craft conventions | No reason to give up proven conventions |

**Decision: A for now.** LabCraft starts as an independent project that follows the Craft
conventions. Joining storytold or tangency-labs later is not excluded; to keep this door open: same
licence, same conventions, dependencies on the `craft-libs` rather than home-made equivalents.
Details in [craft-ecosystem.md](craft-ecosystem.md).

## D-21 — Language

**Status:** accepted on 2026-10-30.

**Decision:** the public repository (code, `AGENTS.md`, docs, issues) is in English, like the whole
Craft ecosystem, to attract contributors. Interface translated through `craft-i18n`, French
included from the start.

## D-22 — Platforms

**Status:** accepted on 2026-10-31.

- Windows, Linux, macOS (x86-64 and ARM64).
- Web (WASM): editor, simulation, simulated instruments. Real hardware is reachable there only
  through WebSerial and WebUSB. "Never break the WASM build" from day one, like GridCraft.

## D-23 — Clean room rules

**Status:** accepted on 2026-10-31.

Adapted from the rules of SolidCraft and GridCraft:

- Behavior designed from public documentation, general conventions of the field and user
  feedback.
- Never copy NI's code, icons, images, help texts, examples or palettes. No LabVIEW screenshot in
  the repository.
- Do not reverse engineer the `.vi` format or the LabVIEW runtime.
- **No observation of LabVIEW to design LabCraft.** Unlike GridCraft, which observes Excel as a
  black box, the NI licence (January 2025, section B) rules it out in practice: clause v forbids
  any attempt to discover the "non-literal aspects" of the software (structure, organization,
  file formats, ideas, algorithms). LabCraft is designed from public documentation, general
  knowledge of the field and user feedback.
- **No published LabVIEW performance measurements** (clause x: no publication of performance data
  without NI's written agreement). No numeric comparison of LabCraft against LabVIEW.
- **No public display of LabVIEW** (clause iv): no screenshot in the repository, the docs or the
  communication.
- **No VI Scripting in the service of LabCraft** (clause xi, see D-01).
- LabVIEW Community Edition is reserved for personal, non-commercial and non-industrial use: do
  not use it for the project.
- Own vocabulary: do not reuse NI's registered or proprietary names (VI, G, LabVIEW, DAQmx other
  than to describe compatibility). See [NOTICE](../NOTICE).
- Do not read or copy GPL / LGPL / AGPL code.

## D-24 — Embedded, real time, FPGA

**Status:** embedded and real-time targets accepted as a goal on 2026-10-30, after v1; choice of
backends accepted on 2026-10-31; FPGA out of scope.

**Backends by target:**

| Target | Examples | Backend |
|---|---|---|
| 64-bit embedded Linux | Recent cRIO equivalent, industrial PC, Raspberry Pi 4/5, RISC-V 64 | Cranelift (x86-64, ARM64, RISC-V 64) |
| 32-bit ARM embedded Linux | Older cRIOs (Zynq ARMv7), many industrial boards | LLVM: Cranelift has no 32-bit backend, only the Pulley interpreter (`pulley32`), which is slower (checked on 2026-11-01) |
| Microcontrollers | Cortex-M, ESP32 | LLVM, or generation of C / Rust compiled by the manufacturer's toolchain (like Simulink Embedded Coder) |

LLVM is therefore not needed as long as the target is 64-bit Linux. Nothing to settle now if the
IR stays independent of the backend (D-09).

**The real work is in the runtime, not in the code generator:**

- deterministic scheduling, priorities, timed loops (the equivalent of *timed loops*);
- no memory allocation in a real-time loop (checkable at compile time);
- real-time OS: Linux with PREEMPT_RT (merged into the mainline kernel since version 6.12);
- deployment to the target from the IDE, remote panel, remote debugging.

From v1, the IR and the runtime must forbid none of this: explicit allocations in the IR,
replaceable scheduler, primitives with no hidden state.

FPGA: out of scope (it would be a high-level synthesis compiler, another project).

## D-25 — Package manager

**Status:** accepted on 2026-10-31. The equivalent of VI Package Manager (VIPM) is planned.

Recommended model: **cargo** rather than VIPM. VIPM installs packages in LabVIEW itself, for the
whole workstation, which makes projects hard to reproduce.

- **Per-project dependencies**, declared in the manifest, pinned in a lock file: a project is
  rebuilt identically on another machine or in CI.
- **Manifest**: name, version (semver), description, licence, dependencies, minimum LabCraft
  version, supported targets, requested permissions.
- **Content of a package**: function libraries, instrument drivers, simulated instruments, project
  templates, WASM plugins, and possibly native binaries per platform.
- **Registry**: at first, installation from a git URL; then a public index hosted in a git
  repository (the crates.io index model), and private registries for companies.
- **Security**: signed packages, permissions shown at install time, native binaries flagged.
- **Commands**: `labcraft add`, `labcraft update`, `labcraft publish`, also available through MCP
  so that an agent finds and adds an instrument driver.

References: cargo, npm, the Node-RED palette, VIPM, NIPM.

## D-26 — Alternative to PLCs (IEC 61131-3)

**Status:** accepted on 2026-10-31, horizon 2 goal (see [vision.md](vision.md)).

LabCraft is already close to PLCs: the FBD (Function Block Diagram) of IEC 61131-3 is a graphical
dataflow language, and IEC 61499 (event-driven function blocks, distributed over several devices)
is even closer.

To add for automation engineers:

- **Cyclic execution**: read inputs, execute, write outputs, at a fixed period, with detection of
  cycle overruns. It is an execution mode in its own right, next to free dataflow (to be taken
  into account from D-12).
- **Retentive variables**, kept after a power cut.
- **Online change** of the program without stopping the machine.
- **Fieldbuses**: EtherCAT (the `ethercrab` crate is a pure-Rust EtherCAT master), Modbus TCP,
  PROFINET, EtherNet/IP, CANopen; OPC UA for supervision.
- **Exchange**: PLCopen XML import and export; possibly the ST language (structured text).
- **Libraries**: timers, counters, edges, PID controllers; later PLCopen Motion.

Limit: certified functional safety (IEC 61508, SIL) is out of reach for a long time. LabCraft will
aim at control and monitoring, not at safety functions.

Reference competitors: CODESYS, TwinCAT (Beckhoff, PC-based PLC). The existing free projects
(OpenPLC, Beremiz) are under the GPL licence: do not read their code (D-23).

## D-27 — Libraries from the Craft apps, SolidCraft plugins

**Status:** accepted on 2026-10-31, long term.

Precedent: **Grasshopper**, Rhino's graphical dataflow language, which became indispensable in
parametric design.

- **Nodes generated from a command registry.** Each Craft app publishes its commands with a
  parameter schema (SolidCraft: `list_commands`, `describe_command`). LabCraft can generate a
  library of typed nodes from them. Apps driven this way: SolidCraft for CAD, GridCraft or
  PdfCraft for test reports.
- **Two modes:**
  - LabCraft drives the app from outside (MCP or control channel): possible as soon as LabCraft
    can call a service;
  - a LabCraft program compiled to a WebAssembly component becomes a **plugin** of the app
    (SolidCraft plans WASM plugins that act through `run(command, args)`, including parametric
    functions of the history).
- **Third mode, added on 2026-11-02: the embedded editor.** The diagram editor is a reusable egui
  crate that a Craft app can open in a panel (a parametric function in SolidCraft, a data
  processing step in GridCraft). The program remains an ordinary LabCraft file. On the agent side,
  **one MCP server per app**: the host app's server exposes the LabCraft commands under their own
  prefix (`labcraft.*`), rather than a second server to configure. LabCraft alone keeps its own
  MCP server (D-18).
- **State of the hosts (checked on 2026-11-02):** VectorCraft and PhotoCraft have a WebAssembly
  plugin API in service (ABI v1, modules run by wasmi, manifest and parameter schema); that of
  SolidCraft (WIT components, `run(command, args)`) is being designed.
- **Reservations:** plugins depend on the WASM backend (D-15), and each app has its own ABI; the
  embedded editor requires an editor crate with no dependency on the IDE window: the `editor` crate,
  from the M0v viewer on (roadmap). This is a community entry point (vision, Positioning), but not
  what sets the order of milestones.

## D-28 — Test executives

**Status:** accepted on 2026-10-31.

In the NI ecosystem, TestStand is a separate test executive: sequences, limits, Passed/Failed
statuses, reports, a results database. LabVIEW is only one **adapter** among others (C/C++ DLL,
.NET, Python, ActiveX). LabCraft follows the same split: it is a language, not a sequencer.

What LabCraft must offer to executives:

1. **The existing adapters**: a LabCraft function built as a C ABI DLL or a .NET assembly (D-15)
   is called from TestStand's DLL or .NET adapter, with nothing specific.
2. **A native adapter** for executives that want better: a runtime library (C API and Rust crate)
   that loads a LabCraft library, lists its functions with their types (units included), calls
   them, and returns the structured errors (category, code, origin) that the executive translates
   into a step status.
3. **The CLI and MCP** for scripted executives and agents.

Consequence for the language: signatures (types, units, default values, documentation) must be
readable by program without running the code, so that an executive builds its steps
automatically.
