# Inspirations

*Last updated: 2026-10-30*

Ideas to borrow, by domain. Borrowing an idea does not mean copying: see the clean-room rules (D-23
in [decisions.md](decisions.md)). Check the licence of any project before reusing its code.

## Languages and execution models

| Source | Idea to borrow | For |
|---|---|---|
| LabVIEW | Dataflow, structures, front panel, error wire, Quick Drop | The language as a whole |
| Simulink | Hierarchical subsystems, time-stepped simulation, code generation | Simulation, embedded in the long term |
| Enso | One program seen as text and as a graph, lossless round trip | D-08 (text syntax) |
| Unreal Blueprints | Ergonomics only: reroute nodes, group comments, large-scale graphs. **Not its execution model**: Blueprints is control flow (execution wires that set the order, a single thread), whereas LabCraft is parallel dataflow. The order of side effects is forced by a data dependency (error wire), not by an execution wire | Editor UX |
| Houdini, Blender geometry nodes | Reusable node groups, "viewer" node, spreadsheet for inspecting data | Debugging and inspection |
| Max/MSP, Pure Data | Modifying the program while it runs | "Live" mode in the long term |
| Node-RED | Simple JSON format, installable community palette | Library ecosystem |
| Lustre, Esterel | Synchronous languages: logical time, determinism | Deterministic execution mode (D-12) |
| Synchronous dataflow (Lee and Messerschmitt, 1987) | Static scheduling when rates are known | Scheduler optimization |
| Kahn process networks | Semantics of processes communicating through queues | Communication between loops (D-04) |
| Unison | Content-addressed code, stable identifiers | Stable identifiers, merging (D-07) |
| IEC 61131-3 (FBD, ST, SFC) | Cyclic execution, function blocks with state, retained variables | PLC mode (D-26) |
| IEC 61499 | Event-driven function blocks, applications distributed over several devices | PLC mode, multiple targets (D-26) |
| Grasshopper (Rhino) | Dataflow for parametric CAD, data lists and trees | SolidCraft plugins (D-27) |

## Packages and distribution

| Source | Idea to borrow | For |
|---|---|---|
| cargo, crates.io | Manifest, lock file, per-project dependencies, index in a git repository | D-25 |
| npm, Node-RED palette | Installation from the editor, search, popularity | D-25 |
| VI Package Manager, NIPM | What LabVIEW users expect; avoid global installation | D-25 |

## Real time and PLCs

| Source | Idea to borrow | For |
|---|---|---|
| CompactRIO, LabVIEW Real-Time | Deployment from the IDE, timed loops, remote front panel | D-24 |
| TwinCAT, CODESYS | Soft PLC, online change, cycle diagnostics | D-26 |
| PREEMPT_RT (Linux kernel) | Standard real-time Linux | D-24 |
| `ethercrab` | EtherCAT master in pure Rust | D-26 |
| PLCopen XML, PLCopen Motion | Program exchange, standard motion library | D-26 |

## Compilers and runtimes

| Source | Idea to borrow | For |
|---|---|---|
| "LabVIEW Compiler: Under the Hood" (NI, public) | G → DFIR → LLVM; analyses on the dataflow IR | D-09 |
| Cranelift (Bytecode Alliance) | Fast code generator written in Rust, JIT and AOT | D-10 |
| MLIR | Several IR levels (dialects) lowered progressively | IR structure (D-09) |
| Lean 4, "Counting Immutable Beans" (Ullrich and de Moura, 2019) | Reference counting with in-place reuse of immutable values | D-11 |
| Perceus (Koka) | Precise reference counting, reuse without a garbage collector | D-11 |
| wasmtime, component model, WIT | Sandboxed plugins with a typed, versioned interface | D-16 |

## Instruments and test

| Source | Idea to borrow | For |
|---|---|---|
| PyVISA | Simple VISA API, resource discovery | D-17 |
| pyvisa-sim | Simulated instruments described in files | AI loop, tests without hardware |
| SCPI, IEEE 488.2 | Standard commands, error queue | SCPI library |
| HiSLIP, VXI-11, USBTMC | Open protocols implementable in Rust | Native VISA |
| IVI | Interchangeable instrument classes (DMM, power supply, oscilloscope) | Instrument libraries |
| OpenHTF (Google) | Structure of a hardware test: phases, measurements, limits, reports | Interface with test executives (D-28) |
| TestStand | Adapter model: an executive calls modules written in several languages | Interface with test executives (D-28) |

## Craft ecosystem

| Source | Idea to borrow | For |
|---|---|---|
| SolidCraft | Command registry, CLI, MCP headless or attached to the app, token-protected control channel, JSON scripts, replayable crash reports, versioned format with migrations, "oracle" tests | D-18, robustness |
| GridCraft | Local `plan/`, layers checked by `cargo xtask layers`, "never break WASM" | Project start |
| craft-libs | Shared building blocks (docking, shortcuts, settings, atomic writes, i18n) | Avoid rewriting what exists |
| craft-ai-integrations | Connections to AI providers, consent, egui widgets | Built-in assistant (later) |

Details: [craft-ecosystem.md](craft-ecosystem.md).

## Graph editors in egui (references)

- `egui-snarl` and `egui_node_graph`: ways of structuring a node editor in immediate mode. Read
  them as references; check licence and maintenance before any dependency.

## UX

| Source | Idea to borrow |
|---|---|
| Quick Drop (LabVIEW), command palette (VS Code) | Do everything from the keyboard, fuzzy search |
| Automatic wiring, diagram cleanup | Automatic placement when an agent produces a graph without a layout |
| SolidCraft (manipulators, live previews) | Immediate preview of every change |
| Browser debuggers | Conditional breakpoints, history of a probe's values |
