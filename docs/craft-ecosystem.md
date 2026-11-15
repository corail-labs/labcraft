# The Craft ecosystem

*Last updated: 2026-11-14. Sources: public storytold repositories on GitHub (the documents of
GridCraft, SolidCraft and MaterialCraft were read again on 2026-11-06; the repositories of storytold and
tangency-labs were surveyed on 2026-11-13 and 14), the public repositories tangency-labs/solidcraft and
tangency-labs/pcbcraft, ArtCraft launcher catalog (v0.10.0, 2026-10-29).*

## Who does what

### storytold (ArtCraft)

GitHub organization `storytold`: "Open Source Tools and Models for Makers, Builders, and
Creatives". A family of pure-Rust applications, each modelled on a well-known commercial tool
(their descriptions say "in the style of Excel / AutoCAD / PowerPoint…"), under MIT OR Apache-2.0.

| App | Domain |
|---|---|
| ArtCraft | AI image and video creation ("The IDE for artists") |
| CADCraft | 2D CAD, in the style of AutoCAD |
| DeckCraft | Presentations |
| DesignCraft, FigCraft | Interface design and prototyping |
| EffectCraft | Visual effects |
| FilmCraft | Non-linear video editing |
| GridCraft | Spreadsheet, aiming at parity with Excel |
| LightCraft | Non-destructive photo editing |
| PdfCraft | PDF |
| PhotoCraft | Photo editing |
| SoundCraft | Audio |
| VectorCraft | Vector drawing |
| WordCraft | Word processing |

Shared repositories:

- **craft-libs**: common `craft-*` crates (see below), consumed by git URL pinned to a commit.
- **craft-ai-integrations**: common AI layer (connections, keys, providers, consent, tasks, egui
  widgets).
- **craft-common-actions**: shared CI. **craft-fonts**: fonts kept out of the app repositories.
- **craft-launcher** (ArtCraft Launcher): installs and updates the apps; distinguishes "Official"
  and "Community" apps.
- **craft-contrib**: the house rules, a block of 15 rules copied into the `AGENTS.md` of each storytold
  repository, and the RFCs of the suite.
- **craft-extensions**: a provisional contract for themes, plugins and resources, and a store that hosts
  nothing (the assets stay in the releases of their authors, pinned by digest). No app uses it yet.

### tangency-labs (SolidCraft)

- **SolidCraft**: 3D parametric CAD in pure Rust, desktop, browser, CLI and MCP server. A project
  independent of storytold but following its conventions (its `AGENTS.md` cites the storytold
  projects as a reference); entered the launcher catalog as a "community app" on 2026-10-29.
- **SolidCore**: geometry kernel. **PCBCraft**: printed circuit board design, "people and agents" on one
  document: the MCP server attached to the open window shares its session and its undo history.

LabCraft would be in the same situation as SolidCraft: independent, Craft conventions, candidate
for the catalog (D-20 in [decisions.md](decisions.md)).

## The common conventions

Collected from the `AGENTS.md` files of SolidCraft and GridCraft and from craft-libs.

| Convention | What it means | For LabCraft |
|---|---|---|
| **Everything is a command** | Every visible action is a command (`id`, label, documented parameters, `enabled`, `run`, tests). The UI, CLI, scripts, MCP and plugins call the same ones | Adopted (D-18). Also the basis of undo and the journal |
| **No dialog for a programmatic call** | Empty parameters → default values or an error | Adopted |
| **Never crash** | No `unwrap`, `expect`, `panic!` or `unsafe` outside tests, except where the repository names an exception; errors as `Result`; hostile inputs; `Session::execute` under `catch_unwind`; replayable crash reports; regression test for every fix | Adopted, and strengthened: a user program that crashes must never bring down the IDE. Loading a native library (D-14) and running native code (D-10) will need `unsafe`; how it is allowed is not decided yet |
| **Checked layers** | Crates arranged in levels L0…L7, dependencies checked by `cargo xtask layers`; nothing below the UI depends on egui | Adopted (see [roadmap.md](roadmap.md)) |
| **Replaceable, thin UI** | Panels read the engine state and act through `app.run(id, params)`; colors through theme tokens | Adopted |
| **Quality gates** | `cargo xtask ci`: fmt, clippy `-D warnings`, tests, asset attribution, layers, WASM build | Adopted |
| **Never break WASM** | The web build passes on every commit | Adopted (D-22) |
| **Clean room** | Black-box observation of the target product; never its code, icons or texts; no GPL code | Adapted to NI (D-23): LabVIEW is not even observed, so there is no target to compare with (see below) |
| **Attributed assets** | Every asset has a line in `ATTRIBUTION.md` | Adopted |
| **Versioned format** | `solidcraft/N`, step-by-step migrations, a corpus of old files that must still open; SolidCraft opens a newer file with a warning and keeps what it does not read | Adopted (`labcraft N`, D-07), but a newer format is refused: a program that drives hardware must not run half understood |
| **CLI and MCP** | `<app>-cli run / eval / mcp`; MCP headless or attached to the app through a local token-protected control channel. `gridcraft-cli check`: exit 0, 1, 2; `--json` prints one object with a named schema (`gridcraft-check/1`); `docs/cli.md` | Adopted (D-18, [cli.md](cli.md)), except that `labcraft-cli run` runs a program and a script of commands is `labcraft-cli script` |
| **Local `plan/`** | A local, git-ignored `plan/` folder for working notes | Adopted, except the ADRs: public in [decisions.md](decisions.md) (see [roadmap.md](roadmap.md)) |
| **Progress docs** | One-page `ROADMAP.md`; detail in `docs/` (parity, ranked gaps, architecture) | Adopted |
| **One task identifier per commit** | For example `M2.1: borders gallery` | Adopted |
| **Licence** | MIT OR Apache-2.0, Rust edition 2024 | Adopted (D-19) |
| **House rules** (craft-contrib) | 15 common rules: pull requests on `main`, a local `plan/`, no secrets… Rule 15: a pull request does not say that an agent wrote it, and no rule of a repository may require it | Most are followed, the block itself not copied. Rule 15 in part: no rule requires saying that an agent helped, but unlike rule 15 an agent's `Co-Authored-By:` is accepted ([AGENTS.md](../AGENTS.md), [CONTRIBUTING.md](../CONTRIBUTING.md)) |
| **Shared CI** (house rule 11) | Apps call the workflows of `craft-common-actions` (signed release for each platform) instead of copying CI | Not adopted: `checks` and `release` are LabCraft's. To revisit with the installable release (M1r) |
| **Confined file access** (RFC-0015, a proposal) | `<app>-cli mcp --automation-read-root DIR --automation-write-root DIR`: the folders an agent may read and write, refused without them | In M1, with the MCP server attached to the IDE; `labcraft-cli mcp --project DIR` stays and sets both |

## What LabCraft can reuse from craft-libs

| Crate | Use in LabCraft |
|---|---|
| `craft-ui` | Docking, tabs, SVG icon buttons, themes: what the shell of the IDE is built on ([ide.md](ide.md), 11) |
| `craft-keymap` | Shortcuts, conflicts, customization |
| `craft-settings` | Preferences, portable mode, migrations |
| `craft-atomic` | Safe saving of programs (never a half-written file) |
| `craft-dialogs`, `craft-menu`, `craft-quit` | Native dialogs, macOS menu bar, clean shutdown |
| `craft-i18n` | Translations (French from the start) |
| `craft-docpath` | Checking the file paths referenced by a program |
| `craft-pdf`, `craft-print` | Test reports in PDF, printing |
| `craft-ai-*` | Built-in AI assistant, later |

What stays specific to LabCraft: the language, the compiler, the runtime, the graph editor, the
front panel, the instruments.

No crate of the ecosystem offers a canvas of nodes, a graph or an undo history: MaterialCraft's node
editor lives in its app crate. The `editor` canvas of LabCraft could be the first one shared (D-27). For
plugins (D-16, M9), two designs exist and are not aligned: `craft-extensions` (wasmi, core modules) and
SolidCraft's (wasmtime, components and WIT, the choice of D-16). Neither has permissions for hardware
(instruments, ports, buses), which LabCraft needs.

## What LabCraft brings that is new to the ecosystem

The Craft apps edit documents (drawings, tables, parts), and some already evaluate them: a GridCraft
macro is a typed program, and MaterialCraft evaluates a graph of nodes. What is new is how far a
LabCraft document goes: a macro is bounded in time and steps, fits in one undo and is cancelled by an
error, while a LabCraft program runs for days, in parallel, on real hardware that no undo brings back.
Points to design without an existing model in the ecosystem:

- A runtime isolated from the IDE: a program that hangs or crashes does not freeze the interface.
- Long-running commands (run, acquisition): start, follow, stop. SolidCraft's render task model
  (`render.start`, `render.status`, `render.cancel`) is the starting point.
- Hardware access and the associated permissions, including for an AI agent: an agent must not be
  able to drive a real power supply without explicit consent.

## What a language changes

A Craft app usually stands on two things: a target that it observes, and a document that commands
change within a session. LabCraft has neither: its target (D-01) is never observed (D-23), and its
document is a program that runs. The engineering of the Craft apps still holds; what changes is what the
project promises, and how it knows that a result is right.

- **No oracle.** Every behaviour is a design decision, and nothing outside says that a result is wrong.
  An expected value in a test is worked out by hand or by another tool (Python, NumPy, C++20
  `std::format`), never copied from the output of the program under test ([AGENTS.md](../AGENTS.md)).
- **Every rule is a promise.** An unused command costs an app nothing; a rule of the language is paid by
  every reader and every agent. Changes that look harmless break programs: a new reserved word, a
  `check` that refuses more, a type rule or a rule of `fmt` that changes. Until the first release such a
  change is allowed, says so in its pull request and migrates the files of the repository
  ([CONTRIBUTING.md](../CONTRIBUTING.md)); from that release, new reserved words and names follow the spec
  (section 14.1, new words).
- **The document acts on the world.** A crash is seen; a wrong result can go unseen, and a wrong PASS
  sends a faulty product out of the test bench. How the language keeps a result from being wrong in
  silence is partly open (Q-16); a number that its type cannot hold is a fault (Q-25).
- **Two writers for one program.** As in the apps, a gesture is a command (D-18). But an agent can also
  write the text, which is the source (D-08), while the user mostly sees the drawing.
- **LabCraft can use the other apps.** In the long term, their command registries can generate libraries
  of nodes (D-27): what is an internal interface for an app then becomes public for the programs that
  call it.
- **The tool stays in what users ship.** The runtime goes into everything a user builds (D-15), and with
  it the licence texts of LabCraft and of its dependencies (D-19).
