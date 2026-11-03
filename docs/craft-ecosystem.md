# The Craft ecosystem

*Last updated: 2026-10-30. Sources: public storytold repositories on GitHub, the public
repository tangency-labs/solidcraft, ArtCraft launcher catalog (v0.10.0, 2026-10-29).*

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

### tangency-labs (SolidCraft)

- **SolidCraft**: 3D parametric CAD in pure Rust, desktop, browser, CLI and MCP server. A project
  independent of storytold but following its conventions (its `AGENTS.md` cites the storytold
  projects as a reference); entered the launcher catalog as a "community app" on 2026-10-29.
- **SolidCore**: geometry kernel. **PcbCraft**: printed circuit board design.

LabCraft would be in the same situation as SolidCraft: independent, Craft conventions, candidate
for the catalog (D-20 in [decisions.md](decisions.md)).

## The common conventions

Collected from the `AGENTS.md` files of SolidCraft and GridCraft and from craft-libs.

| Convention | What it means | For LabCraft |
|---|---|---|
| **Everything is a command** | Every visible action is a command (`id`, label, documented parameters, `enabled`, `run`, tests). The UI, CLI, scripts, MCP and plugins call the same ones | Adopted (D-18). Also the basis of undo and the journal |
| **No dialog for a programmatic call** | Empty parameters → default values or an error | Adopted |
| **Never crash** | No `unwrap`, `expect`, `panic!` or `unsafe` outside tests; errors as `Result`; hostile inputs; `Session::execute` under `catch_unwind`; replayable crash reports; regression test for every fix | Adopted, and strengthened: a user program that crashes must never bring down the IDE |
| **Checked layers** | Crates arranged in levels L0…L7, dependencies checked by `cargo xtask layers`; nothing below the UI depends on egui | Adopted (see [roadmap.md](roadmap.md)) |
| **Replaceable, thin UI** | Panels read the engine state and act through `app.run(id, params)`; colors through theme tokens | Adopted |
| **Quality gates** | `cargo xtask ci`: fmt, clippy `-D warnings`, tests, asset attribution, layers, WASM build | Adopted |
| **Never break WASM** | The web build passes on every commit | Adopted (D-22) |
| **Clean room** | Black-box observation of the target product; never its code, icons or texts; no GPL code | Adopted and adapted to NI (D-23) |
| **Attributed assets** | Every asset has a line in `ATTRIBUTION.md` | Adopted |
| **Versioned format** | `solidcraft/N`, step-by-step migrations, clean refusal of future formats, a corpus of old files that must still open | Adopted (`labcraft/N`, D-07) |
| **CLI and MCP** | `<app>-cli run / eval / mcp`; MCP headless or attached to the app through a local token-protected control channel | Adopted (D-18) |
| **Local `plan/`** | A local, git-ignored `plan/` folder for working notes | Adopted, except the ADRs: public in [decisions.md](decisions.md) (see [roadmap.md](roadmap.md)) |
| **Progress docs** | One-page `ROADMAP.md`; detail in `docs/` (parity, ranked gaps, architecture) | Adopted |
| **One task identifier per commit** | For example `M2.1: borders gallery` | Adopted |
| **Licence** | MIT OR Apache-2.0, Rust edition 2024 | Adopted (D-19) |

## What LabCraft can reuse from craft-libs

| Crate | Use in LabCraft |
|---|---|
| `craft-ui` | Docking, tabs, SVG icon buttons, themes: the shell of the IDE |
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

## What LabCraft brings that is new to the ecosystem

The Craft apps edit documents (drawings, tables, parts). LabCraft is the first whose documents
**run**: parallel programs, real hardware, time. Points to design without an existing model in the
ecosystem:

- A runtime isolated from the IDE: a program that hangs or crashes does not freeze the interface.
- Long-running commands (run, acquisition): start, follow, stop. SolidCraft's render task model
  (`render.start`, `render.status`, `render.cancel`) is the starting point.
- Hardware access and the associated permissions, including for an AI agent: an agent must not be
  able to drive a real power supply without explicit consent.
