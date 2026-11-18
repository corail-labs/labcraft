# Outline of the milestones after M1 — not a specification

*Last updated: 2026-11-18*

This page outlines what the milestones after M1 hold (M4, M3, M2a, M2b, M5, in the order of work), so that M1 blocks
none of them, and what must be decided early for the milestones beyond. It is not a specification and decides no rule of
the language: each milestone gets its specification when its work starts, and its page here then shrinks to its goal and
a link to that specification. It is a living document, changed by pull requests like the other documents: each part
gives its date and its state, and "Beyond M5" gains a line at each decision that touches a later milestone. The
decisions D-01 to D-28 (decisions.md) stay as they are.

What this page gives as an owner's decision, a decision of decisions.md or a settled question of the spec is decided;
everything else is *proposed*, and the specification of its milestone settles it.

## 1. What M1 must not block

*Written 2026-11-14, checked against `main` on 2026-11-18 (after #216). State: constraints on the work of M1.*

Seven constraints on M1, drawn from the milestones below.

1. **The editing commands and the undo are not specific to the diagram.** M3 brings a second editor (placing the panel
   elements), which must reuse the same commands, the same selection, the same undo and the same `batch`: a command
   targets a *section* of a file (`body`, `layout`, `panel`, later the `layout` of the panel), not "the canvas".
2. **"Surround with a structure": the crossing of the frame is written once** (commands.md 9.6,
   `crates/edit/src/surround.rs`): what goes in, what comes out, the `layout` counted again in the new frame, the error
   chain. Each structure adds only its head (`check_params`, `head`, `fill`: `match`, `handle`, `for`, `loop` today).
   M2a adds `events` and the access to `ref<T>` by a variant and its head, without rewriting the gesture.
3. **Identity of the nodes (Q-10)**: settled on 2026-11-14 (spec 11): by name, with no hidden identifier; `value.rename`
   moves the `layout` line (and, in M5, breakpoints and probes); a rename made in the text, with the IDE open, is
   recognized (same function, same wires) and keeps the position. To revisit if M1 shows that a name is not enough
   (Q-10). **Gap:** the recognition of a rename made in the text is in no step of the Rust of M1 (architecture 12, end)
   nor in `acceptance-m1.md`; M5 depends on it (#212).
4. **The IDE window has panels to add** (docking of `craft-ui`): `Panel` (M3) and `Debug` (M5) are "Planned" workspaces
   (ide.md 6), `Panel` is also a view of a tab, and `Pause`, `Breakpoint` and `Watch` are "Planned" entries of the tab
   bar and of the `Run` menu. The bar has a `Stop` (Shift+F5) which is `run.abort` (ide.md 3): an abort. The Stop
   element of the front panel (spec 8.2) stays the clean end of a loop; the two coexist.
5. **The selection is read, the agent does not set it**: `view.get_state` gives it to the agent, `view.select` is
   refused to it (ide.md 8, commands.md 6.7); it designates by names (commands.md 9.2). In M5, the agent sets a probe or
   a breakpoint on what it names, the human on what they select, by the same commands.
6. **The attached MCP has a place for consent**: the policies Ask, Edits and All by the `effects` of the commands
   (ide.md 8); none reaches a real instrument, and M4 says how the human allows it (mcp.md 6).
7. **The settings** (`.labcraft/config.toml` of the project, versioned, and those of the user; commands.md 9.1) receive
   no breakpoints and no probes: these are view state of the user (spec 11, ide.md 3, like the view of a tab), never in
   the `.lcf` (as in VS Code). "Workspace" means something else in ide.md 6 (`Edit`, `Panel`, `Debug`, `Build`): do not
   use it for this state.

## 2. The milestones

### M4: real I/O

*Outlined 2026-11-14, recast 2026-11-18 (owner's decision). State: outline; its specification is the next work of M4.*

M4 does the real I/O in general, of which `visa` is the first user, rather than every transport of VISA: LabCraft is
used for more than SCPI benches (owner's decision, 2026-11-18). It blocks nothing, and it goes on in parallel with M1.

**Goal.** The same program runs on the simulator and on the hardware, without changing a line; an agent touches real
hardware only if the human has allowed it. The hardware: a SCPI instrument, but also an Arduino board, a serial or
network device with a protocol of its own.

**In**

- **Two transports in pure Rust, TCP and serial**, behind `NativeLibrary` (a host can provide its own, part 3), exposed
  twice: as raw bytes, by libraries `net` and `serial`, for devices without VISA; and under `visa`, by the resource
  names `TCPIP0::10.0.0.5::5025::SOCKET` and `ASRL3::INSTR`, with `SIM::` kept.
- A `--sim` file can **answer for a real name**: the program of the bench, with its real resource name, is tested
  without hardware. Without `--sim`, a real name goes to the hardware. `libs/visa/simulation.md`, where a
  resource starts with `SIM::`, changes with it; a real name that the file given to `--sim` does not describe (the
  hardware, or `resource_not_found`) is a question for the specification of M4.
- **Consent, for any real resource** (not only VISA: a serial port, a host and a TCP port): `labcraft-cli run` started
  by the human has the right; the MCP server alone, only for the resources given at start (`--allow-hardware RESOURCE`,
  mcp.md 6); attached to the IDE, a question in the window, resource by resource (ide.md 8 points here). No tool grants
  it. Without the right: a `not_allowed` error from the library, not a crash. The effect `instruments` of
  the commands (commands.md 1), which the policies of ide.md 8 read, then covers every real resource, not only an
  instrument; its name is reviewed with the specification of M4.
- **In the IDE**: the choice of the simulation file in the `Run` panel (ide.md 4) gains an entry "Real hardware" (no
  `--sim`: real names go to the hardware), and the consent question in the window. The planned entry `Run > Real
  hardware` of ide.md 6 is where M4 appears. **An instrument is not a target** (owner's decision, 2026-11-18): a target
  is where the program runs or what it becomes (`.exe`, Raspberry Pi, a microcontroller, plugin, web: M6, M9, M10, horizon 2 and after);
  real hardware is a resource that the program touches, whatever the target.
- **The binary minimum**: `read_bytes`, `write_bytes` (N bytes, with timeout) and, so that a protocol can be written in
  pure LabCraft on top of `net` and `serial` (as `scpi` on top of `visa`):
  - in `core`, the bitwise operations on integers (`and`, `or`, `xor`, `not`, shifts; today `core.and` and `core.or`
    take only `bool`), the integer division and the modulo (today `core.div` of two integers gives an `f64`); new
    primitives, so the spec (appendix A);
  - in `bytes`: from and to `[u8]`, slicing, concatenation, length; read and write `u16`, `u32`, `i16`, `i32`, `f32` in
    big and little endian at a position; SCPI binary blocks `#…`.
  Without them, no oscilloscope traces, no Modbus, no custom serial protocol.
- **Modbus TCP in pure LabCraft, the example that validates M4** (in `examples/`): MBAP header, PDU, big endian
  registers, on top of `net` and `bytes`. It proves that the primitives are enough and serves as a model for
  contributors for the other protocols. It also avoids `tokio-modbus`, which relies on tokio.
- `scpi` extended: `*OPC?`, `*ESR?`, status registers; `faults.hang` in the simulator, to test a lost device.
- The fix of #138 (from M0).

**Out, as separate libraries** (one issue each, open to contributors, according to demand and the hardware available):
VXI-11, HiSLIP, USBTMC, the binding to an installed VISA (its C API, loaded at run time: `unsafe`, which would be an
exception that the repository names; how `unsafe` is allowed is not decided, craft-ecosystem.md), GPIB through an
adapter (D-17); Modbus RTU (CRC-16 by the bitwise operations; frames cut by the length that the
function code announces, not by the silence of 3.5 characters, which is hard to hold from a diagram), MQTT, CAN, DAQmx,
`i2c`/`spi`, an Arduino board as a slave (the serial transport opens the way). SRQ as events (waits for M2a). A
dedicated interface for the simulation files: the `Text` view is enough in M4.

**When.** In parallel with M1: it lives in `libs/visa`, the runtime and the CLI, not in the crates of the editor
(`edit`, `ide`, `engine/edits`). **First tasks**: the trial of `serial2` for the serial transport (below), then #138. The
hardware serves for the final trial by hand, not for development: a software TCP server, a virtual serial port or an
Arduino board are enough for the tests.

**Needs.** From M1: point 6 of part 1. From M0: #138.

**Risks.**
- **Licence of the serial transport**: `serialport` 4.10 is MPL-2.0, which `cargo xtask licences` accepts (it refuses
  only GPL, LGPL and AGPL); `serial2` 0.2.39 is `BSD-2-Clause OR Apache-2.0`, a permissive licence as the rest of the
  repository, taken under Apache-2.0, maintained, with `libc`, `cfg-if` and `windows-sys` as its only dependencies, where
  `serialport` also needs `libudev` on Linux. Owner's decision (2026-11-18): `serial2`; the first task of M4 is a short
  trial that confirms it (read with a timeout, list the ports, on Windows, Linux and macOS, with a virtual port or an
  Arduino board). It may give less USB information (VID, PID) when listing ports than `serialport`, which matters only
  to a future discovery of devices.
- The runtime: blocking real-time I/O touches the scheduler; a PR of M1 on `engine` may cross those of M4.
- USBTMC on Windows (an instrument installed with the driver of its maker cannot be seen through WinUSB): taken out of
  M4, it stays the risk of its library.

### M3: front panel

*Outlined 2026-11-14, revised 2026-11-18. State: outline.*

**Goal.** An operator starts a test from a front panel: buttons, values, graphs, Stop; the front panel is edited with
the mouse in the IDE.

**In**

- **The front panel in the IDE**: the `Panel` view of a tab and the `Panel` workspace, both "Planned" in ide.md 3 and 6
  (place decided by the specs of M1), live during the run, editable outside the run with the gestures, the commands and
  the undo of M1 (part 1, point 1). The detached windows of M1 (1b) give an operator front panel in its own window, with
  no extra work.
- **List of the v1 elements**: the six of M0 (`button`, `switch`, `numeric`, `text_box`, `led`, `chart`) plus `graph`
  (XY), `table`, `enum` (drop-down list), an `error` indicator, `tabs` and a container (group frame). Gauges, user-made
  controls and rich decorations: later.
- **Writing**: the place of the elements in the `layout` of the panel, their look in `panel` (arguments that `check`
  does not judge, spec 10); the look changes during the run by `ui.set` of a property (the colour of a `led`).
- **Modes of `button`** (Q-30): latching until read (M0), true while held, a press seen on release.
- **Runtime → interface channel** bounded in rate (D-13): a `ui.set` at 10 kHz does not freeze the IDE; the trace keeps
  everything when `Record values` is on (off by default, ide.md 3: without it, a run writes no trace).
- **No events**: the elements are read in a loop, as in M0. The channel is designed so that the events of M2a plug into
  it (the runtime already receives "value changed").
- **Stop of a subfunction** (Q-24): a parameter of kind *element* (`stop: element bool`), which the caller wires with
  its element; the subfunction reads it by `ui.get`.
- Accessibility: an AccessKit `Role` and a name per element, keyboard navigation.

**Out.** Standalone executable with its front panel (M6), web HMI served by an executable (M10 or later), remote panel
(horizon 2).

**Needs.** From M1: points 1, 4 and 5. From M0: `ui.get`, `ui.set`, `--set`. The RFC of the front panel model (#12)
follows this outline: M3 has no events.

**Risks.** The list of elements that grows; the front panel is a compatibility surface (a change of egui moves what an
operator sees): snapshots of the front panel from the first element.

### M2a and M2b: the full language

*Outlined 2026-11-14, revised 2026-11-18. State: outline.*

**Goal.** Write a real program with several loops (acquisition, HMI, log: a bench, a supervision, a control system) and
the analysis libraries (`stats`, `signal`, `fit`).

Too large for one milestone: **two halves**, M2a then M2b (owner's decision, 2026-11-14).

**M2a, communication**: `queue<T>` (bounded, with timeout, never by name), notifier, `events` structure (front panel
elements, timeouts, user events; one handler per loop), `ref<T>` and its access structure (Q-19, `check` refuses a
nested access that it sees), `for` that stops early (Q-22). The scenario put off from M0: two loops by a queue, clean
stop without loss. In deterministic mode, the events of a test are scripted on the simulated clock.

**M2b, types and libraries**: user generics, with the units in the signatures (Q-12); `map<K, V>` and `set<T>` (Q-28);
shared constants (Q-29); manifests of the user libraries (D-25: dependencies, `always`, `resources`), with their page in
the IDE (at the head of the library in the tree: name, version, dependencies, and the text, to switch like `Diagram` and
`Text`), storage folders without namespace, private functions; the recommended entry (Q-31, "not in M1"), with the
signatures.

**Out.** `timed loop` and `cyclic task` (horizon 2, D-24 and D-26), actors (a library in LabCraft, validated first on an
example by hand in M2a), `config` (Q-15, with M2b or later).

**Needs.** From M1: points 2 and 3 of part 1. From the specs: D-04's "typed shared variables" are `ref<T>` within a
program (M2a); between programs, they wait for a tag engine, horizon 2 (D-04, staging). **From the editing rules of M1**
(commands.md 9.1 and 9.2): each new construct (queue, notifier, `events`, access to `ref<T>`) is written incomplete,
with a hole `?` at each place the text cannot omit, and each of its places has a name for the commands; otherwise the
editor cannot place it step by step. Its grammar (spec 14) says so from its spec.

**Risks.** The time the design of the language takes; deadlocks; the determinism of events.

### M5: debugger

*Outlined 2026-11-14, revised 2026-11-18. State: outline.*

**Goal.** See the program run and stop it where one wants, the human as well as the agent.

**In**

- **Probes**: values on the wires during the run, history of a probe (the curve of M0v). Since M1, the trace is a file,
  written only when `Record values` is on (off by default, ide.md 3): M5 keeps the values **in memory** during the
  session, and the file when `Record values` is on (to compare two runs, give them to an agent, keep them after a crash,
  read them on the web).
- **Breakpoints** on a node or a wire, conditional; **pause and step** (next node, end of iteration, exit of the call);
  **animation** of the execution; open a call to see its values.
- Everything goes through commands (`debug.*`): the agent debugs by MCP like the human (D-18), designating by names
  (part 1, point 5). **Their `effects`** (commands.md 1), which the policies of the attached agent read (ide.md 8):
  setting or removing a breakpoint or a probe has none (it is view state); pause, step and resume are `run`.
- Their place in the window is decided: `Pause`, `Breakpoint` and `Watch` in the tab bar and the `Run` menu, the `Debug`
  workspace (ide.md 3 and 6).
- **Semantics**:
  - in deterministic mode, a pause stops the simulated clock: nothing changes, everything can be replayed;
  - in real time, a pause stops the diagram, not the world: the instruments stay in their state, a power supply stays
    on, and a banner says so; a wait resumes with the time it has left;
  - an Abort is allowed during a pause.
- The breakpoints live in the view state of the user, not in the `.lcf` (part 1, point 7).

**Out.** Editing during a pause (no hot reload), remote debug (horizon 2), probes on queues and events before M2a.

**Needs.** From M1: points 3, 4 and 5; of point 3, the recognition of a rename made in the text, which M1 has not yet
put in a step (#212; without it, a breakpoint is lost when an agent renames a value). From the runtime: the thread of
the diagram already hands back control between two nodes (`Observer`): the pause fits there.

**Risks.** Low on the code side (the IR and the trace are made for it); the semantics of real time is the real question.

## 3. Beyond M5

*Written 2026-11-14, widened 2026-11-18. State: outline.*

No design of M6 and later, nor of the plugins: only what must be decided before them, and in which milestone.

| What comes (target, host or capability) | What would block it | Where it is decided | Decisions |
|---|---|---|---|
| Standalone `.exe` with its front panel (M6) | A front panel drawn in the code of the IDE | M3: the rendering of the front panel in a separate crate, shared by the IDE and the executable | D-13, D-15 |
| Library: C ABI DLL, .NET facade (M6) | Types with no translation to C; an exported generic function | M2b: an exported function has concrete types (units, enums, `error`, arrays have a C shape); a generic is exported by its instances; `map`, `ref`, `queue` do not cross the boundary | D-15 |
| LabCraft in **any Craft app** (SolidCraft, ArtCraft, VectorCraft, PhotoCraft, GridCraft…): a program as a WASM plugin of the app, the editor embedded in one of its panels, nodes generated from its command registry (horizon 2) | A runtime that imposes threads and a real clock; hard-wired I/O; an editor tied to the window of the IDE; a single plugin ABI assumed | M4: the transports behind `NativeLibrary`, a host can provide its own; the runtime stays free of imposed threads and clock (as a microcontroller needs, row below); WASM build held. M1: `editor` does not depend on `ide` (true on 2026-11-18: it depends on `egui`, `diagram` and `types`), a host takes `editor` and `engine` without the window. Each app has its ABI (VectorCraft and PhotoCraft: ABI v1 under wasmi; SolidCraft: WIT, in design): one adapter per app, no imposed ABI | D-16, D-22, D-27 |
| **LabCraft plugins** (M9): nodes, transports, widgets, value views provided by a WASM component, and a LabCraft program itself compiled as a component | A plugin that acts outside the commands; permissions invented apart; hard-coded extension points | M1: everything is a command, with its `effects` (commands.md 1); the policies of the attached agent (ide.md 8) are the model for the permissions of a plugin (`files`, `run`, real hardware of M4). M3: the widget kinds in a registry (row below). M4: the transports behind `NativeLibrary`. The same component model (WIT) as the Craft apps, so that a component serves both sides when the ABIs allow | D-16, D-27 |
| Runtime on a microcontroller, Cortex-M (after v1, D-24) | A scheduler or an interpreter that assumes threads, a real clock or loading from the text | M4 (blocking real-time I/O): threads of D-12 behind a trait, values and primitives separable from loading | D-12, D-24 |
| Binding to an installed VISA, C calls (M8) | — | Out of M4 (2026-11-18): a separate library, its `unsafe` an exception that the repository would name (not decided); the rest of the `extern` in M8 (spec 4.1) | D-14, D-17 |
| New widgets by plugin (M9) | Element kinds hard-coded in a `match` | M3: kinds in a registry, qualified names `lib.kind` reserved for libraries and plugins | D-16 |
| Web HMI served by an executable (M10 or later), remote panel and debug (horizon 2) | A runtime → interface channel in shared memory, specific to the process of the IDE | M3: the channel carries serializable messages (like the trace), bounded in rate, that another process or a page can receive; M5: the debug already goes through commands, so through the control channel | D-13, D-18 |
| `timed loop`, `cyclic task` (horizon 2) | — | Out of M2 (spec 6.4) | D-24, D-26 |
| Tags shared between programs (horizon 2) | — | M2a: `ref<T>` in a program; between programs, the tag engine (D-04, staging) | D-04 |

For all of them: the runtime depends neither on the IDE nor on egui (layers of `xtask layers`), which is true today.

The targets and milestones to come are visible in the IDE from M1, greyed and marked Planned, each linking to its
milestone or its issue (ide.md 6).
