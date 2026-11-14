<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/logo-dark.svg">
    <img src="docs/images/logo-light.svg" width="96" height="96" alt="LabCraft">
  </picture>
</p>

<h1 align="center">LabCraft</h1>

<p align="center">
  <strong>A graphical dataflow programming language and IDE, built in pure Rust.</strong><br />
  Designed from the start to be written, run and fixed by people and AI agents alike. Ambition:
  to do at least what LabVIEW does, open and cross-platform.
</p>

<p align="center">
  <a href="docs/language-spec-v0.md">Specification</a> ·
  <a href="docs/visual-notation.md">Visual notation</a> ·
  <a href="examples/">Examples</a> ·
  <a href="#try-it">Try it</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

**Status (2026-11-14): M0, the first milestone, is done in Rust (tag `v0.3.0-m0`, with binaries
on the [releases page](https://github.com/corail-labs/labcraft/releases)): `labcraft-cli` reads, checks, writes and
runs programs against simulated instruments (`fmt`, `check`, `run`), an AI agent drives it over MCP
(`labcraft-cli mcp`), and a read-only viewer draws each function as its file changes, with the values of a
run on the wires (`labcraft`, `labcraft-web`, `labcraft-cli diagram`).**
The language and its text format are described (spec v0), with example programs. Editing the diagram comes
next (M1). The throwaway Python proof of concept that ran the examples before Rust remains in `tools/`. The
milestones are described in [docs/roadmap.md](docs/roadmap.md).

## One program, two views

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/read_voltage-dark.svg">
    <img width="720" alt="Diagram of read_voltage: visa.open, scpi.query_f64 and visa.close in a row on a red error wire, the VISA session as a blue wire, the raw reading converted to volts" src="docs/images/read_voltage-light.svg">
  </picture>
</p>

The error wire (red) orders the actions and carries the error: if the measurement fails, the
nodes after it are skipped, except `visa.close`, marked `always`, which still closes the session
(blue). Values carry their physical unit (`[V]`).

The diagram is drawn from a text file, which is the format itself,
[examples/read_voltage.lcf](examples/read_voltage.lcf):

```text
labcraft 1

use core
use scpi
use visa

/// Reads the DC voltage of a multimeter.
fn read_voltage(resource: string, err: error = none) -> (voltage: f64[V], err: error) {
    dmm, e1 = visa.open(resource, timeout: 2 s, err: err)
    raw, dmm2, e2 = scpi.query_f64(dmm, "MEAS:VOLT:DC?", err: e1)
    #close: e3 = visa.close(dmm2, err: e2)
    voltage = core.with_unit(raw, [V])
    err = e3
}
```

The diagram is drawn from this file by `labcraft-cli diagram --svg`, following the
[visual notation](docs/visual-notation.md); the desktop viewer draws it the same way.

## An AI agent at work

An agent (Claude Code) on `labcraft-cli mcp` writes a new function from a request: it reads the
guide and the functions it can call, writes `check_power`, which `check` accepts, and runs it against a simulated
power supply. The desktop viewer, open on the folder beside it, draws each file as it is written, and the values of
the run appear on the wires. Then a change of the function, redrawn and run again (46 seconds):

https://github.com/user-attachments/assets/38106f7d-c1aa-41ad-a4d6-761a483cec2e

In five sessions of five, an agent wrote, checked, ran and fixed a test program on its own
([acceptance-m0.md](docs/acceptance-m0.md), scenario 1).

## Try it

**From a release.** Each [release](https://github.com/corail-labs/labcraft/releases) has an archive for Windows,
Linux and macOS with `labcraft-cli`, the viewer `labcraft`, the examples and the simulated instruments of `visa`
in `sim/`. In a terminal, in the folder of the archive (on Windows, in PowerShell):

```sh
./labcraft-cli check examples
./labcraft-cli run --sim sim/dmm-230V.toml --arg resource=SIM::DMM0 examples check_voltage
./labcraft examples
```

The run writes its trace in `examples/.labcraft/runs/`, and the viewer shows its values on the wires of
`check_voltage`. The binaries are not signed yet: the note of each release says how to open them on Windows and
macOS. The release also has the web viewer, `labcraft-web-<version>.zip`, a folder to serve with any static
server.

**From the sources.** The Rust command line checks the examples, that they are in canonical form, and runs one
against a simulated multimeter (Rust 1.95 or later):

```sh
cargo run -p labcraft-cli -- check examples
cargo run -p labcraft-cli -- fmt --check examples
cargo run -p labcraft-cli -- run --sim libs/visa/sim/dmm-230V.toml --arg resource=SIM::DMM0 examples check_voltage
cargo run --release -p labcraft -- examples
```

`check` reports errors with their line and node; `run` executes a function against a simulated
instrument and prints its outputs (here `Verdict.Pass`, 230.071217 V). The reports and diagnostic codes are
described in [docs/cli.md](docs/cli.md), the viewer in its section 8.

To let an AI agent write, check and run programs on its own, give it the MCP server of the command line on a
folder of its own, here in Claude Code, with the `labcraft-cli` of a release or one built from the sources
(`cargo build --release -p labcraft-cli`, in `target/release/`):

```sh
claude mcp add labcraft -- <path>/labcraft-cli mcp --project <folder>
```

Open the viewer on the same folder to watch the agent's functions drawn as they are written.

The server is described in [docs/mcp.md](docs/mcp.md), and the skills an agent can install with it in
[skills/README.md](skills/README.md).

The throwaway proof of concept, which ran programs before Rust did, is in Python (3.11 or later), with its own
MCP server ([tools/lcf-poc/README.md](tools/lcf-poc/README.md)):

```sh
pip install -r tools/lcf-poc/requirements.txt
python tools/lcf-poc/lcf.py check examples
python tools/lcf-poc/lcf.py run examples check_voltage --sim libs/visa/sim/dmm-230V.toml --arg resource=SIM::DMM0
```

## Documents

**The language**

| Document | Content |
|---|---|
| [docs/language-spec-v0.md](docs/language-spec-v0.md) | Language and file format, v0 (first draft): types and units, dataflow, errors, structures, grammar, open questions |
| [docs/visual-notation.md](docs/visual-notation.md) | How a diagram is drawn: nodes, terminals, wire families, structures (first draft) |
| [docs/cli.md](docs/cli.md) | The command line: `fmt` and `check`, their reports and diagnostic codes (implemented for M0a), `run` and `mcp` (implemented for M0b), `diagram` and the viewer (M0v) |
| [docs/commands.md](docs/commands.md) | The commands of LabCraft: how they are named, called and described, the catalog of M0b, values in JSON, the report and the trace of a run (implemented for M0b) |
| [docs/mcp.md](docs/mcp.md) | The MCP server: tools, results and errors, what the agent is told, bounds (implemented for M0b) |
| [skills/](skills/README.md) | Skills for agents: writing, debugging and testing a program, which the MCP server serves through `guide_read` |
| [libs/visa/README.md](libs/visa/README.md) | The `visa` library: its functions, its sessions, and what it guarantees on them (atomic calls, one command at a time, release after an abort; implemented for M0b) |
| [libs/visa/simulation.md](libs/visa/simulation.md) | Simulated instruments of `visa`: the format of simulation files, how an instrument answers, the files shipped in `libs/visa/sim/` (implemented for M0b) |
| [examples/](examples/) | Example programs (`.lcf` functions, `.lct` types), from a multimeter check to a test station state machine |
| [docs/glossary.md](docs/glossary.md) | Terms used across the documents |

**The project**

| Document | Content |
|---|---|
| [docs/vision.md](docs/vision.md) | Why LabCraft, for whom, principles, differentiators, non-goals |
| [docs/roadmap.md](docs/roadmap.md) | Where the project stands, milestones from a first vertical slice in Rust (M0), planned code layout |
| [docs/acceptance-m0.md](docs/acceptance-m0.md) | The scenarios that say when each part of M0 is done (M0a, M0b, M0v), and what they need written first |
| [docs/architecture.md](docs/architecture.md) | How the Rust code is arranged, and the order of work: M0a, M0b and M0v (implemented) |
| [docs/ir.md](docs/ir.md) | The dataflow IR that a checked program is lowered to and the runtime runs: regions, one node per line, origins, invariants (implemented for M0b) |
| [docs/decisions.md](docs/decisions.md) | The project decisions: options, recommendation, status |
| [docs/alternatives.md](docs/alternatives.md) | Existing products and what they teach us |
| [docs/inspirations.md](docs/inspirations.md) | Ideas to borrow (languages, compilers, tools, UX) |
| [docs/craft-ecosystem.md](docs/craft-ecosystem.md) | The Craft apps (storytold, SolidCraft) and what LabCraft takes from them |

**Throwaway tools and material**

| Folder | Content |
|---|---|
| [tools/lcf-poc](tools/lcf-poc/README.md) | Python proof of concept: check, run against simulated instruments, MCP server |
| [tools/lcf-figcraft](tools/lcf-figcraft/README.md) | Renders a function as a diagram, following the visual notation |
| [tools/lcf-view](tools/lcf-view/lcf_view.py) | Syntax check of the examples and of the code blocks of the spec and the skills |
| [design/](design/) | The board of the visual notation, drawn by hand, and the component library the renderer reads ([FigCraft](https://github.com/storytold/figcraft) files) |
| [docs/rfc/](docs/rfc/) | Small programs discussed in the RFCs |

## Contributing

The language is not frozen: feedback from users of LabVIEW and other test and measurement tools is
what matters most right now. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Your choice of [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE). LabCraft is an independent
project, not affiliated with NI; see [NOTICE](NOTICE).
