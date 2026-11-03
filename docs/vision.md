# Vision

*Last updated: 2026-11-03*

## In one sentence

LabCraft is a graphical dataflow language and its development environment, on a par with LabVIEW.
Every program is an open text file that humans edit graphically and that AI agents read, write, run
and fix directly.

## The problem

LabVIEW has been a reference in test, measurement and laboratory automation since 1986. Its core
ideas are excellent: dataflow makes parallelism natural, the front panel gives every program an
interface, and the link to instruments is direct. But:

- **The format is closed and binary.** `.vi` files cannot be diffed or merged as text; teamwork
  needs LabVIEW's own compare and merge tools.
- **An AI assistant cannot write a LabVIEW program directly.** There is no documented text
  representation that a model can read or produce. AI stays outside the tool.
- **Cost and dependency.** Proprietary licence (subscription or perpetual), a runtime to install on
  every target machine, an ecosystem tied to a single vendor (NI, now a subsidiary of Emerson).
- **Cross-platform support is secondary.** Windows first; Linux and macOS with limits.
- **A rewrite was discontinued.** LabVIEW NXG, shipped in 2017, was discontinued in 2020 (see
  [alternatives.md](alternatives.md)).

## Positioning

**A general-purpose language, two entry points.** Nothing in the language is specific to testing:
it is a real programming language, in which one must be able to write complete applications. It
enters through two doors:

- **Test and measurement** (test benches, SCPI instruments, PASS/FAIL, reports): this is where it
  can be complete earliest, and where the LabVIEW users are.
- **The Craft ecosystem**: be the graphical language of the Craft apps, to automate them, write
  plugins for them and, later, be embedded in their interface. These apps already have
  contributors who work with AI agents.

Horizons:

1. **Test, measurement, laboratory** (first release): desktop on Windows, Linux, macOS, and web.
2. **Control systems on target** (planned, not now): embedded real-time Linux targets, in the spirit of
   LabVIEW with cRIO, and an **alternative to the IEC 61131-3 PLC languages** (D-26 in
   [decisions.md](decisions.md)).
3. **Microcontrollers** (later still): code generation for Cortex-M and equivalents.

In parallel, **the language of the Craft ecosystem** (D-27), in three steps:

1. **Drive a Craft app** from a LabCraft program, with nodes generated from the app's command
   registry (MCP or control channel). This is possible as soon as LabCraft can call a service.
2. **Write plugins**: a program compiled to WebAssembly becomes a plugin of the app. VectorCraft
   and PhotoCraft already have a WebAssembly plugin API; SolidCraft's is being designed. Depends on
   the WebAssembly backend (D-15).
3. **Embed the diagram editor in an app**: a LabCraft panel in SolidCraft or GridCraft, for a
   parametric function or a data-processing task. The architecture therefore keeps the editor in a
   reusable crate, with no dependency on the IDE window.

The examples are test and measurement for now, plus a computation (`projectile`); examples that
drive a Craft app will follow.

## Who it is for

1. **Test and validation engineers (ATE)**: test benches, production control, PASS/FAIL, reports.
   The priority audience.
2. **Laboratories and R&D**: acquisition, instrument control, signal processing.
3. **Automation engineers**: process control, HMI, fieldbuses, a soft PLC on a PC or an embedded
   target.
4. **Developers**: a general-purpose language, parallel by nature, for complete applications.
5. **Education**: a free, open tool that also runs in the browser.
6. **Users and contributors of the Craft apps**: automate an app without writing Rust, write
   plugins (parametric generators for SolidCraft, filters for VectorCraft), chain several apps (a
   measurement, a GridCraft table, a PdfCraft report) (D-27).
7. **AI agents**: full users, through the CLI and the MCP server.

## Principles

1. **No less than LabVIEW.** The target scope is that of LabVIEW: dataflow, structures, front
   panel, debugger, libraries, instruments, building executables and libraries. We are not
   compatible with its files.
2. **The semantic model is the truth.** A program is a typed, documented, versioned graph. The
   layout on screen (positions, wire routing) is separate, secondary information. An agent never
   handles coordinates.
3. **Programs are text.** An open, stable format that can be compared and merged in git.
4. **One semantics.** A program gives the same result interpreted, compiled, in the IDE, as an
   executable or in the browser.
5. **Everything is a command.** Every action of the interface is a named command with documented
   parameters, which the CLI, scripts, the MCP server and plugins also call (Craft convention, see
   [craft-ecosystem.md](craft-ecosystem.md)).
6. **Never crash.** An IDE that drives hardware and holds hours of work must not go down. No panic
   in production code, and every input is treated as hostile.
7. **No runtime to install.** An executable or a DLL produced by LabCraft embeds what it needs.
8. **Open.** Permissive licence, documented format, sandboxed plugins, no dependency on a hardware
   vendor.
9. **Cross-platform and web.** Windows, Linux, macOS; the editor and the simulation also in the
   browser.

## How we differ from LabVIEW

| | LabVIEW | LabCraft |
|---|---|---|
| Program format | Binary, closed | Text, open, versioned, diffable |
| AI | Outside the tool | Native: CLI, MCP, commands, simulation |
| Simulated instruments | Simulated devices for some drivers | At the core: an agent tests without hardware |
| Built programs | Require the Run-Time Engine | Standalone, runtime embedded |
| Platforms | Windows first | Windows, Linux, macOS, web |
| Extensions | DLL, .NET, ActiveX, Python | C ABI, .NET, Python, sandboxed WASM plugins |
| Physical units | Optional | In the type system from v0 (D-05, spec 3.3) |
| Licence and cost | Proprietary and paid (subscription or perpetual) | Free software (MIT or Apache 2.0) and free of charge |

## The loop that makes the difference

An AI agent must be able to do the following alone, without a screen:

1. write a LabCraft program from a natural-language request;
2. validate it (types, connections, structures) and read precise errors;
3. run it against a simulated instrument;
4. read the results, probes and traces;
5. fix its program and start again;
6. show it to the human in the IDE, who sees it appear live.

Example of a criterion: "Read the voltage of a multimeter over SCPI, check that it is between 220
and 240 V and return PASS or FAIL" must end in a valid program, tested against a simulated
multimeter, without human intervention.

## Non-goals (for the first release)

- Reading or importing LabVIEW `.vi` files.
- Embedded targets, real time and PLCs: planned, not in v1. The architecture must not, however, do
  anything that prevents them (D-24).
- FPGA.
- Certified functional safety (IEC 61508, SIL): LabCraft targets control systems, not safety functions.
- Rewriting the NI drivers: we go through their public C APIs when they exist.
- A built-in AI that replaces the language: the AI writes LabCraft, it does not hide it.

## Long term

- A package manager and library registry, equivalent to VI Package Manager (D-25), with shared
  instrument libraries.
- Be callable by test executives (TestStand and others): LabCraft is a language, not a sequencer.
  Like LabVIEW in TestStand, it is one adapter among others (D-28).
- Embedded real-time Linux targets, in the spirit of cRIO (D-24).
- An alternative to the PLC languages: cyclic execution, fieldbuses, PLCopen exchange (D-26).
- The language of the Craft ecosystem: driving apps, plugins, embedded editor (D-27, see
  Positioning), and a place in the launcher catalog (D-20).
