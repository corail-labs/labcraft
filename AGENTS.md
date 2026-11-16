# LabCraft — instructions for coding agents

LabCraft is an AI-native graphical dataflow language and IDE, aiming to do at least what LabVIEW
does, in pure Rust, open and cross-platform. It follows the conventions of the Craft apps
(storytold, SolidCraft); see `docs/craft-ecosystem.md`.

This file is for any coding agent that works on this repository (Codex, Claude Code, Cursor, Copilot, Gemini or
another), for a contributor or for a maintainer. The guide for people is [CONTRIBUTING.md](CONTRIBUTING.md).

**Where the project is:** M0 is done (tags `v0.1.0-m0a`, `v0.2.0-m0b` and `v0.3.0-m0`): `labcraft-cli` reads,
checks, writes and runs programs and serves them over MCP, and a viewer draws them. M1, the diagram editor, is under
way (`docs/architecture.md`, section 12; `docs/roadmap.md`).

## Documents

`README.md` lists the documents. The ones you will need most:

- `docs/decisions.md`: decisions D-01 to D-28. **All are retained**: a pull request never changes one. To question
  one, open an issue.
- `docs/language-spec-v0.md`: language and file format spec, grammar in section 14, open questions in section 15.
- `docs/architecture.md`: how the code is arranged, and the contract a change of code meets.
- `docs/roadmap.md`: where the project stands, milestones, future layers and repository layout.

Read the sections your task needs, not whole documents: the spec and `docs/architecture.md` are large. Documents
are in English (D-21).

## Non-negotiables (D-23)

- **No use of LabVIEW to design LabCraft.** The NI licence (January 2025, section B) forbids
  reverse engineering and any attempt to discover non-literal aspects (structure, file formats,
  ideas, algorithms), publishing performance data, and using LabVIEW scripting to build a
  graphical editor. So: never run, script, inspect or screenshot LabVIEW for this project; never
  open, read, search for or list LabVIEW files: `.vi`, `.ctl`, `.lvproj`, `.lvlib`, `.vit`, `.ctt`, `.llb`,
  `.lvclass`; never publish LabVIEW benchmarks. Design from public documentation, general domain knowledge and user
  feedback.
- **The CI holds it too:** `cargo xtask clean-room`, in `cargo xtask lint`, fails when the repository tracks a file with
  one of these extensions, in any case, and the job `lint` runs on every pull request. `.ignore` keeps them out of a
  search with ripgrep; a tool that does not read `.ignore` will still find them, so never search for them.
- **Real projects**: never record details of real or client projects anywhere in this repository;
  examples stay generic.
- **No GPL/LGPL/AGPL code**: don't read or copy it (for example OpenPLC, Beremiz, Snap7). `cargo xtask licences`
  refuses such a dependency; code you read or copy, no check can see: the template of a pull request asks you to say so.
- No NI icons, help text, screenshots or examples; NI names only to describe compatibility.

## A change, from the issue to the pull request

1. **Start from an open issue.** A pull request without a linked issue, or that does what the issue puts out of
   scope, is closed without review; a fix of a few lines of prose (a typo, a broken link) is the only exception.
   A maintainer's pull request is exempt, and links its issue when there is one. Claim the issue first, as [CONTRIBUTING.md](CONTRIBUTING.md) says, and check that nobody else holds it.
2. **Read the issue against the documents.** An issue can be wrong. When it contradicts the spec or another
   document, say so in a comment on the issue and wait: don't follow it to the letter.
3. **Do what the issue asks, and no more.** Ask the person you work for before you change a behaviour that the issue
   does not name, or a rule of the language, the file format or a command.
4. **Prove it.** Run `cargo xtask ci` (`cargo xtask setup` installs what it needs), and give in the pull request the
   commands you ran and their output, or a picture for a change that shows: the template asks for each.
5. **Commit or push only when the person you work for asks.** Never push to `main`, which is protected: one branch
   per topic, then a pull request that fills the template (`.github/pull_request_template.md`). The `checks`
   workflow must be green.
6. **A maintainer merges**, by squash, with `gh pr merge --squash --match-head-commit HEAD_SHA` on the head whose
   checks are green; never with `--admin`, never through `gh api`. GitHub requires only the job `checks`, but a red
   `fuzz`, a red job `windows` or a red `release` (which runs on a pull request that changes it or a note of
   `docs/releases/`) stops the merge unless a maintainer decides otherwise for that pull request.

The subject line is short, in English: it becomes the commit on `main`. A `Co-Authored-By:` trailer for an agent is
accepted, not required: no rule of this repository requires it, as house rule 15 of the Craft repos asks, but unlike
that rule it does not forbid it. Contributions are licensed MIT OR Apache-2.0, inbound = outbound, no DCO or CLA
(D-19).

## When you change the spec or the examples

- `cargo xtask ci` (or `cargo xtask test`) checks them, in Rust: every complete code block of the spec and of
  the `.md` files of `skills/` must read and be in canonical form (spec 5.2: order of lines and layout of the
  text), `crates/format/tests/doc_blocks.rs`; `examples/`, `docs/rfc/` and each library of `libs/` must check
  (`apps/labcraft-cli/tests/check.rs` finds a new library by itself) and be in canonical form
  (`apps/labcraft-cli/tests/fmt.rs`, `crates/format/tests/round_trip.rs`); the
  corpora must be sound (`apps/labcraft-cli/tests/corpora.rs`). Code blocks containing `…` are illustrative
  fragments and are skipped. A grammar the spec gives before the parser reads it goes in
  `AHEAD_OF_THE_PARSER` of `doc_blocks.rs`; the test fails when such a block starts to read: remove the entry.
  The grammar exists once: spec section 14 (EBNF).
- An expected value in a test or a corpus never comes from the output of the program under test: work
  it out by hand or with another tool, and say how. For a case of `tests/acceptance/`, its `expected.json` is
  worked out from the documents alone, by a person or a separate agent session that never sees the code, the output
  of the program or your values. When the program disagrees with it, don't edit its `expected.json`: the program or
  the documents are wrong, and a question of the documents goes to a maintainer, on the issue.
- The corpus of runs of `tests/acceptance/runs/` and the Rust `labcraft-cli run` are the reference for running a
  program; the corpora of `tests/acceptance/` and the Rust `labcraft-cli check` are the reference for `check`, and
  the spec decides where they differ.
- The figures of `docs/images/` are drawn by `labcraft-cli diagram --svg --theme light --out DIR FOLDER FUNCTION`
  (and `--theme dark`), which writes `DIR/FUNCTION.svg`, saved as `FUNCTION-light.svg` and `FUNCTION-dark.svg`;
  rerender them when a file changes.
- `.lcf` and `.lct` files use LF line endings (`.gitattributes`).

## Rust code

`cargo xtask ci` must pass: formatting, layers, licences, the `libs/` boundary, the diagnostic codes
against `docs/cli.md`, the commands against `docs/commands.md`, LabVIEW files, clippy, tests, WASM.
It needs the `wasm32-unknown-unknown` target and `wasm-bindgen-test-runner`: `cargo xtask setup` installs what is
missing, and says each command before it runs it. On Windows, a folder of the path of the repository whose name
starts with `x` or `u` breaks the WASM tests; `setup` says so.
The web viewer is built by `cargo xtask web` (Trunk 0.21.14) and tried by `cargo xtask web-smoke` (Chrome or Chromium),
which the job `web` of `checks` runs; neither is part of `ci`.

`fuzz/` is outside the workspace, with its own `Cargo.lock`, and the workflow `fuzz` runs it on a pinned
nightly, under Linux: `fuzz/README.md` says how to run it, and what to do when it finds something.

The Craft rules apply (see `docs/craft-ecosystem.md`): never crash (no `unwrap`, `expect`, `panic!` or
`unsafe` in production code), everything is a command, layered crates, never break the WASM build,
every asset attributed. `docs/architecture.md` says how the code is arranged.

## Working economically

- Be concise. Prefer one good document change over many small exchanges.
