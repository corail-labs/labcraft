# Roadmap

*Last updated: 2026-11-03*

LabCraft is in its specification phase: the language is described and tried out by a throwaway
Python proof of concept, and there is no product code in Rust yet. This page says where the
project stands, what the milestones are, and how the Rust code will be organized.

## Where we are: phase 0, specification

| Document or tool | Status |
|---|---|
| Vision ([vision.md](vision.md)) | First draft |
| Decisions ([decisions.md](decisions.md)) | D-01 to D-28, all retained |
| Language and file format ([language-spec-v0.md](language-spec-v0.md)) | First draft with grammar; open questions Q-1 to Q-25 |
| Visual notation ([visual-notation.md](visual-notation.md)) | First draft; open questions N-1 to N-12 |
| Glossary ([glossary.md](glossary.md)) | Kept with the spec |
| Alternatives, inspirations, Craft ecosystem | First drafts |
| Python proof of concept ([tools/lcf-poc](../tools/lcf-poc/README.md)) | Checks and runs the examples against simulated instruments; MCP server driven by an AI agent |
| Diagram rendering ([tools/lcf-figcraft](../tools/lcf-figcraft/README.md)) | Throwaway: renders a function following the visual notation |

Still to write before M0: the architecture (from the layers below), the command catalog v0, the MCP
and CLI specification (with the schema of diagnostics and the stable error codes), how the CLI and
MCP set a panel element (Q-20), and what the acceptance scenarios list under "write first"
([acceptance-m0.md](acceptance-m0.md)). Two feasibility prototypes, throwaway as well: an egui
canvas with 1,000 nodes and 2,000 wires at 60 frames/s, and a scheduler with two parallel loops
and a clean stop.

Feedback on the language matters most at this stage: see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Milestones

M0 is a thin vertical slice that crosses all layers, as in SolidCraft.

| # | Milestone | Content |
|---|---|---|
| **M0** | Vertical slice without UI | Text format → check → interpreter → result. The v0 language, as listed in [spec section 9](language-spec-v0.md#9-what-m0-implements): what the proof of concept runs (units, structs and enums, arrays, `loop` and `for` with carries, `match`, `handle`), plus function references, static type checking and live panel elements. A multimeter simulated over SCPI. `labcraft-cli run` and `labcraft-cli mcp`. **Criterion:** an agent writes, runs and fixes the PASS/FAIL multimeter program on its own |
| M1 | Diagram editor | egui canvas, placement, wiring, selection, undo, quick insertion of a node by name, automatic layout; the agent edits the program while the human watches. Target: 1,000 nodes and 2,000 wires at 60 frames/s |
| M2 | Full language v1 | Queues and notifiers, events, `ref<T>` and its access structure, generic functions, manifests of user libraries and their dependencies |
| M3 | Panel | Controls and indicators, graphs, live update, properties |
| M4 | Real instruments | TCP/IP, serial, USBTMC, HiSLIP; binding to an installed VISA; SCPI library |
| M5 | Debugger | Probes, breakpoints, single-stepping, execution animation, value history |
| M6 | Build | Standalone executable, C ABI DLL, .NET facade |
| M7 | Native compilation | Cranelift, equivalence tests with the interpreter |
| M8 | Inbound interop | Calling C DLLs, .NET, Python (`extern` functions) |
| M9 | Plugins | WASM, manifest, permissions |
| M10 | Web | IDE and simulation in the browser |
| M11 | Packages | Manifest, locking, install from git, then registry (D-25) |

**Horizon 2, after v1:** real-time embedded Linux targets (D-24), controller mode and fieldbuses
(D-26), libraries generated from Craft apps and SolidCraft plugins (D-27). The package manifest
(D-25) is designed from M2, when user libraries appear (built-in libraries declare their categories,
`always` functions and resources from M0), even if the registry comes later. The WASM build (D-22)
is maintained from M0, even if the web app ships only in M10.

## M0 acceptance scenarios

Detailed in [acceptance-m0.md](acceptance-m0.md), with what each one needs written first.

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

The proof of concept already runs scenario 1 (`examples/check_voltage.lcf` against the simulated
multimeters `dmm-230V` and `dmm-250V`), and an agent has written, checked and fixed a new program
through its MCP server.

## Layers of the Rust code

As in SolidCraft (L0 to L5): a crate depends only on lower layers, checked by
`cargo xtask layers`. Nothing below L5 depends on egui.

| Layer | Crates | Role |
|---|---|---|
| L0 | `types` | Types, values, physical units |
| L1 | `graph`, `format` | Semantic model of the program, separate layout, reading and writing the format, migrations |
| L2 | `check`, `ir` | Type and connection checking; dataflow IR, analyses, linear IR |
| L3 | `runtime` | Scheduler, interpreter, values, probes and breakpoints |
| L3 | `instruments` | VISA in Rust, SCPI, simulated instruments |
| L3 | `interop` | C ABI, .NET, Python |
| L4 | `engine` | Session, command registry, undo, journal, crash reports |
| L5 | `editor`, `ui-egui`, `mcp`, `codegen` | Reusable headless diagram editor that a Craft app can embed (D-27); IDE (window, panel, debugger); MCP server; Cranelift and building executables and DLLs |
| apps | `labcraft`, `labcraft-cli`, `labcraft-web` | Desktop, CLI, browser |

## Repository layout, once the Rust code starts

Modeled on SolidCraft and GridCraft:

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
├── docs/                language, notation, architecture, MCP, commands
├── examples/            example programs, in the text format
├── instruments/         simulated instrument definitions
├── skills/              skills for agents (write, debug, test a program)
├── tests/               integration tests, file corpus, backend equivalence tests
├── xtask/               ci, layers, assets, wasm, command catalog
└── tools/               the throwaway Python tools, until Rust replaces them
```

## Next

M0 in Rust: the `types`, `format`, `check` and `runtime` crates and `labcraft-cli`, with the
proof of concept and `examples/` as the executable reference.
