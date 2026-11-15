# LabCraft — instructions for agents

LabCraft is an AI-native graphical dataflow language and IDE, aiming to do at least what LabVIEW
does, in pure Rust, open and cross-platform. It follows the conventions of the Craft apps
(storytold, SolidCraft); see `docs/craft-ecosystem.md`.

**Current phase: before M1.** M0 is done: M0a, M0b and M0v (tags `v0.1.0-m0a`, `v0.2.0-m0b` and
`v0.3.0-m0`, `docs/architecture.md`, sections 9 to 11). The throwaway Python tools were retired after M0 (2026-11-15); `tools/` keeps only independent oracles.

## Start every session here

1. If `plan/` exists, read `plan/STATUS.md` (current phase, next task).
2. `README.md` lists the documents. The ones you will need most:
   - `docs/decisions.md`: decisions D-01 to D-28. **All are retained**; don't change a retained
     decision without the owner's agreement.
   - `docs/language-spec-v0.md`: language and file format spec, grammar in section 14, open
     questions in section 15.
   - `docs/roadmap.md`: where the project stands, milestones, future layers and repository layout.
3. Documents are in English (D-21).

## `plan/` is private

- `plan/` is gitignored: it is never part of this repository. If it is a git repository of its
  own, commit it separately, from inside `plan/`; `plan/README.md` explains how it is kept.
- **Private matters go only in `plan/`**: company or client names, client projects, business
  strategy, personal constraints. Nothing of the kind goes in `docs/` or anywhere in this
  repository.

## Non-negotiables (D-23)

- **No use of LabVIEW to design LabCraft.** The NI licence (January 2025, section B) forbids
  reverse engineering and any attempt to discover non-literal aspects (structure, file formats,
  ideas, algorithms), publishing performance data, and using LabVIEW scripting to build a
  graphical editor. So: never run, script, inspect or screenshot LabVIEW for this project; never
  read `.vi`, `.ctl`, `.lvproj`, `.lvlib` files; never publish LabVIEW benchmarks. Design from
  public documentation, general domain knowledge and user feedback.
- **Real projects**: never record details of real or client projects anywhere in this repository;
  examples stay generic.
- **No GPL/LGPL/AGPL code**: don't read or copy it (for example OpenPLC, Beremiz, Snap7).
- No NI icons, help text, screenshots or examples; NI names only to describe compatibility.
- **Hooks**: `.claude/settings.json` runs `.claude/hooks/guard.py` before each tool call of Claude Code. It
  refuses a path, glob or command that names a LabVIEW file (`.vi`, `.ctl`, `.lvproj`, `.lvlib`, `.vit`,
  `.ctt`, `.llb`, `.lvclass`; `.ignore` keeps them out of broad searches), and, for this repository only
  (a fork is free), a `git push` that writes `main` and a merge without `--match-head-commit`, with
  `--admin` or through `gh api`. A guard rail, not a sandbox: never look for a way around it. Other agents
  have only these rules, `.ignore` and the protection of `main`. Its tests run in `cargo xtask hooks`.

## When you change the spec or the examples

- `cargo xtask ci` (or `cargo xtask test`) checks them, in Rust: every complete code block of the spec and of
  the `.md` files of `skills/` must read and be in canonical form (spec 5.2: order of lines and layout of the
  text), `crates/format/tests/doc_blocks.rs`; `examples/`, `docs/rfc/` and each library of `libs/` must check
  (`apps/labcraft-cli/tests/check.rs` finds a new library by itself) and be in canonical form
  (`apps/labcraft-cli/tests/fmt.rs`, `crates/format/tests/round_trip.rs`); the
  corpora must be sound (`apps/labcraft-cli/tests/corpora.rs`). Code blocks containing `…` are illustrative
  fragments and are skipped. A grammar the spec gives before the parser reads it goes in
  `AHEAD_OF_THE_PARSER` of `doc_blocks.rs`; the test fails when such a block starts to read: remove the entry.
- `cargo xtask hooks` runs the Python tests of the hooks (`.claude/hooks/`, standard library only). The grammar
  exists once: spec section 14 (EBNF).
- An expected value in a test or a corpus never comes from the output of the program under test: work
  it out by hand or with another tool, and say how. For a case of `tests/acceptance/`, the agent
  `labcraft-oracle` works it out from the documents alone and writes its `expected.json`: give it the
  case folders, never the code, the output or your values. When the program disagrees with it, don't
  edit its `expected.json`: the program or the documents are wrong, and a question of the documents
  goes to the owner.
- The corpus of runs of `tests/acceptance/runs/` and the Rust `labcraft-cli run` are the reference for running a
  program; the corpora of `tests/acceptance/` and the Rust `labcraft-cli check` are the reference for `check`, and
  the spec decides where they differ. The Python proof of concept that was the reference until M0b was retired
  after M0 (2026-11-15).
- The figures of `docs/images/` are drawn by `labcraft-cli diagram --svg --theme light --out DIR FOLDER FUNCTION`
  (and `--theme dark`), which writes `DIR/FUNCTION.svg`, saved as `FUNCTION-light.svg` and `FUNCTION-dark.svg`;
  rerender them when a file changes.
- `.lcf` and `.lct` files use LF line endings (`.gitattributes`).

## Commits

- Commit or push only when the owner asks. Changes in `plan/` are committed separately, directly.
- Every change to this repository goes through a pull request, the owner's too: one branch per
  topic, the `checks` workflow green, squash merge. `main` is protected: never push to it.
- Open the pull request with `gh pr create`; merge only when the owner asks. The skill
  `labcraft-ship-step` goes through it, and the agent `labcraft-reviewer` does the independent review
  before a pull request.
- Short English subject line. A `Co-Authored-By:` trailer for an agent is accepted, not required: no rule
  of this repository requires it, as house rule 15 of the Craft repos asks, but unlike that rule it does not
  forbid it.
- Contributions are licensed MIT OR Apache-2.0, inbound = outbound, no DCO or CLA (D-19).

## Working economically

- Be concise. Prefer one good document change over many small exchanges.

## Rust code

`cargo xtask ci` must pass: formatting, layers, licences, the `libs/` boundary, the diagnostic codes
against `docs/cli.md`, the commands against `docs/commands.md`, clippy, tests, WASM.
It needs the `wasm32-unknown-unknown` target and `wasm-bindgen-test-runner`; `cargo xtask wasm` gives
the command that installs the runner.
The web viewer is built by `cargo xtask web` (Trunk 0.21.14) and tried by `cargo xtask web-smoke` (Chrome or Chromium),
which the job `web` of `checks` runs; neither is part of `ci`.

`fuzz/` is outside the workspace, with its own `Cargo.lock`, and the workflow `fuzz` runs it on a pinned
nightly, under Linux: `fuzz/README.md` says how to run it, and what to do when it finds something.
GitHub requires only the job `checks`, but a red `fuzz`, a red job `windows` or a red `release` (which runs on a pull
request that changes it or a note of `docs/releases/`) stops the merge unless the owner decides otherwise for that
pull request.

The Craft rules apply (see `docs/craft-ecosystem.md`): never crash (no `unwrap`, `expect`, `panic!` or
`unsafe` in production code), everything is a command, layered crates, never break the WASM build,
every asset attributed. `docs/architecture.md` says how the code is arranged.
