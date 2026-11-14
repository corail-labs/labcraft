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
| PCBCraft | One session in the window with the MCP server attached to it: one undo history for the human and the agent, the status bar naming who made an edit, a revision on every edit, window commands (selection, zoom) outside undo | M1, the agent and the human on one program |
| MaterialCraft | Gestures of its node editor: Space opens the search of nodes, one wire per input, a wire that would make a cycle refused with a message, a gesture that becomes a command only when it ends | M1, gestures |
| SolidCraft | A `batch` of commands rolled back to a checkpoint when one fails, with an outcome for each command | M1, editing commands |

Details: [craft-ecosystem.md](craft-ecosystem.md).

## Graph editors in egui (references)

- `egui-snarl` (MIT OR Apache-2.0), `egui_node_editor` (MIT), `egui-graph-edit` (MIT, the successor of
  `egui_node_graph`): ways of structuring a node editor in immediate mode, read as references for the hit-testing
  of terminals, dragging a wire, the selection rectangle and context menus. None is a dependency: each has its own
  model of nodes, its own drawing and its own placement, where the canvas of LabCraft paints what `diagram`
  computes (seen on 2026-11-13, when none followed the version of egui that LabCraft uses).

## UX

| Source | Idea to borrow |
|---|---|
| Quick Drop (LabVIEW), command palette (VS Code) | Do everything from the keyboard, fuzzy search |
| Automatic wiring, diagram cleanup | Automatic placement when an agent produces a graph without a layout |
| SolidCraft (manipulators, live previews) | Immediate preview of every change |
| Browser debuggers | Conditional breakpoints, history of a probe's values |
| Blueprints, Blender, Houdini, vvvv, Node-RED, Enso, Simulink | A wire dropped on empty canvas opens the search, filtered on what fits there; the node chosen lands at that point, already wired, in one undo step |
| Pure Data (autopatch) | From the keyboard, a new node is placed under the selection and wired to it |
| Simulink, Dynamo, KNIME, Blender | The interface of a selection is computed from the wires that cross its border: the same computation extracts a function and wraps a selection in a structure |
| Simulink, Houdini | Tidy in parts (a selection, a branch, everything), in one undo step, applied only if it improves the drawing; wires routed apart from the placement of nodes |
| Simulink, Blueprints | While a wire is dragged, show where it can go and, at a refused terminal, why; the filter must be the rule of `check`, or the editor offers what it then refuses |
| Hazel | An incomplete program is a normal state: a hole is shown where it is, and the rest still checks |
| ELK (interactive mode) | A node added without a position goes to the free place nearest its source, and nothing else moves |
| n8n, Blueprints | A visual diff of a change (nodes added, changed, removed; moves apart), to review what an agent did |
| Enso, PCBCraft | Every edit names the revision it applies to; a stale one is refused |
| Grasshopper, Dynamo, KNIME, Houdini | Problems shown on the node by severity, still readable when zoomed out, with a list that goes to them |
