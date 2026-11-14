# Contributing to LabCraft

Agents: start from [AGENTS.md](AGENTS.md). This page is the guide for people.

LabCraft is an AI-native graphical dataflow language and IDE, written to be read, run and fixed by
people and by AI agents alike. The goal is to do at least what LabVIEW does, in pure Rust, open and
cross-platform.

## Where the project is

LabCraft has **done M0a and M0b, the first two parts of its first milestone in Rust**: `labcraft-cli`
reads, checks, writes and runs programs against simulated instruments, and serves them to an agent over MCP. The repository holds the language and file format specification, the decisions behind it, example
programs, the Rust workspace, and throwaway Python tools (a syntax checker and viewer, and a proof of
concept that runs the examples against simulated instruments and exposes them to an agent over MCP).

Code contributions in Rust open in stages. Since the tag of M0 (`v0.3.0-m0`, after `v0.1.0-m0a` and
`v0.2.0-m0b`), they have been open on all that exists: bugs and improvements of `fmt`, `check`, `run` and the MCP
server, the interpreter, `visa` and its simulated instruments, the drawing of `diagram` and the viewers `labcraft`
and `labcraft-web`, cases for the corpora of `tests/acceptance/`, other platforms, the WASM build.
[docs/architecture.md](docs/architecture.md) is the contract a contribution meets; [docs/roadmap.md](docs/roadmap.md)
says what comes next. Feedback on the language and on the viewer remains the most useful help.

## How to help now

- **Discuss the language.** Design questions are open as issues titled `[RFC] …`, with the
  [`rfc` label](https://github.com/corail-labs/labcraft/issues?q=label%3Arfc). Experience from
  people who build test and measurement systems, in LabVIEW or anything else, is what we need
  most: what hurts, what must not change, what a text format must make easy. To propose one, open
  an issue with the RFC template. [Discussions](https://github.com/corail-labs/labcraft/discussions)
  are for questions and ideas.
- **Report a problem in the spec or the examples** as an issue: a contradiction, a hole, an
  example that does not do what the text says.
- **Fix one.** A pull request to the spec or the examples is welcome; see the checks below.
- **Improve `fmt` and `check`.** A bug, a missing case in the corpora of `tests/acceptance/`, another
  platform or the WASM build; see the checks below.
- **Improve `run` and the MCP server.** A bug, a case for the corpus of runs (`tests/acceptance/runs/`), a
  simulated instrument that `visa` should describe; see the checks below.

The open questions are listed in the spec, section 15; the decisions and their reasons are in
[docs/decisions.md](docs/decisions.md).

## Checks

Every complete code block of the spec and every file in `examples/` and `libs/` must parse, the
examples, the RFC programs (`docs/rfc/`) and the shipped libraries (`libs/`) must check and be in
canonical form (spec 5.2), and the proof of concept and the hooks of the agents (`.claude/hooks/`) must
stay green:

```sh
pip install -r tools/lcf-poc/requirements.txt
cargo xtask spec
```

`cargo xtask spec` runs each check, prints `ok` or `FAIL` for it, and the output of those that fail; the
list is in `xtask/src/spec.rs`. The `checks` workflow runs it on every pull request, with the parts of
`cargo xtask ci` (below), which needs no Python.
The tests include the corpora of `tests/acceptance/`, which say what `check` and `fmt` must report.
An expected value in a test or a corpus never comes from the output of the program under test: work it
out by hand or with another tool, and say how.

The proof of concept was the executable reference for running a program until the interpreter of M0b passed the
corpus of runs (`docs/architecture.md`, section 10.11); that corpus and the Rust `labcraft-cli run` are the
reference since then. The proof of concept no longer follows every rule of `check`: there the corpora of `tests/acceptance/` and the Rust
`labcraft-cli check` are the reference, and the spec decides where they differ. A proof-of-concept check
above that fails only because it lags behind a rule is dropped, in the pull request that brings the rule.

The grammar exists twice, in the spec (section 14, EBNF) and in `tools/lcf-view/lcf_view.py`
(Lark): a change to one goes in the other. `.lcf` and `.lct` files use LF line endings.

## Rules that aren't negotiable

1. **Independent of NI.** LabCraft is designed from public documentation, general domain knowledge
   and user feedback. Don't use LabVIEW to design LabCraft: no running, scripting, inspecting or
   screenshotting it for this project, no `.vi`, `.ctl`, `.lvproj` or `.lvlib` files, no LabVIEW
   benchmarks. Don't contribute NI code, icons, help text, screenshots or examples. Describing your
   own experience as a user is welcome. The reasons are in [docs/decisions.md](docs/decisions.md)
   (D-23).
2. **No GPL, LGPL or AGPL code, not even reading it.** LabCraft is MIT OR Apache-2.0. Permissive
   dependencies are fine; say which one you add and why.
3. **No private material.** No customer data, machine designs or details of real projects: examples
   stay generic.

In Rust code, the Craft rules apply as well: never crash (no `unwrap`, `expect`, `panic!` or `unsafe`
in production code), everything is a command with a test, layered crates, the WASM build never
breaks, every asset attributed. See [docs/craft-ecosystem.md](docs/craft-ecosystem.md). `cargo xtask
ci` runs the checks of the Rust code.

## Sending a change

1. Every change goes through a pull request, the maintainers' too. Keep it to one topic and say
   what you checked; the `checks` workflow must be green. GitHub requires only its job `checks`, so that a
   broken nightly or runner can never block `main`, but a red job `windows`, a red `fuzz` workflow or a red
   `release` workflow also stops the merge unless a maintainer decides otherwise for that pull request
   ([fuzz/README.md](fuzz/README.md)).
2. Pull requests are squash-merged: the title becomes the commit on `main`, so give it a short
   English subject line.
3. If an AI agent helped, say so (a `Co-Authored-By:` trailer is fine).
4. A change that makes a valid program invalid, or changes what it means (a reserved word, a rule of
   `check` or `fmt`, a type rule), says so in a line of its description that starts with
   `Changes the language:`, and migrates the files of the repository in the same pull request. Until
   the first release this is allowed. Afterwards, a new reserved word or a name refused to types follows
   spec 14.1 (new words); the policy for the other changes is still to be written.

## Licence

By contributing you agree that your contribution is licensed as MIT OR Apache-2.0, like the rest
of LabCraft ("inbound = outbound"). There is no CLA and no DCO sign-off: you keep the copyright of
your contribution. LabCraft is not affiliated with NI; product names are trademarks of their owners
(see [NOTICE](NOTICE)).
