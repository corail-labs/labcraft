# Contributing to LabCraft

Agents: start from [AGENTS.md](AGENTS.md). This page is the guide for people.

LabCraft is an AI-native graphical dataflow language and IDE, written to be read, run and fixed by
people and by AI agents alike. The goal is to do at least what LabVIEW does, in pure Rust, open and
cross-platform.

## Where the project is

LabCraft has **implemented M0a, the first part of its first milestone in Rust**: `labcraft-cli`
reads, checks and writes programs, but does not run them yet. The repository holds the language and file format specification, the decisions behind it, example
programs, the Rust workspace, and throwaway Python tools (a syntax checker and viewer, and a proof of
concept that runs the examples against simulated instruments and exposes them to an agent over MCP).

Code contributions in Rust open in stages. With the tag of milestone M0a, which waits for a long
fuzzing run, they open on what exists: bugs and improvements of `fmt` and `check`, cases for the
corpora of `tests/acceptance/`, other platforms, the WASM build. The interpreter (M0b) and the
diagram viewer (M0v) open each when its specification and its section of
[docs/architecture.md](docs/architecture.md) are written, so that a contribution has a contract to
meet; [docs/roadmap.md](docs/roadmap.md) lists what is still to write. Until the tag, the most
useful help is feedback on the language.

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

The open questions are listed in the spec, section 15; the decisions and their reasons are in
[docs/decisions.md](docs/decisions.md).

## Checks

Every complete code block of the spec and every file in `examples/` and `libs/` must parse, the
examples, the RFC programs (`docs/rfc/`) and the shipped libraries (`libs/`) must check and be in
canonical form (spec 5.2), and the proof of concept must stay green:

```sh
pip install -r tools/lcf-poc/requirements.txt
python tools/lcf-view/lcf_view.py examples libs --spec docs/language-spec-v0.md
python tools/lcf-poc/lcf.py check examples
python tools/lcf-poc/lcf.py fmt --check --spec docs/language-spec-v0.md examples
python tools/lcf-poc/lcf.py check docs/rfc
python tools/lcf-poc/lcf.py fmt --check docs/rfc
python tools/lcf-poc/lcf.py check libs/visa
python tools/lcf-poc/lcf.py check libs/scpi
python tools/lcf-poc/lcf.py fmt --check libs
python -m unittest discover tools/lcf-poc/tests
python tools/lcf-poc/tests/mcp_smoke.py
```

The `checks` workflow runs the same commands on every pull request, with `cargo xtask ci` (below).
The tests include the corpora of `tests/acceptance/`, which say what `check` and `fmt` must report.

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
   broken nightly or runner can never block `main`, but a red job `windows` or a red `fuzz` workflow also
   stops the merge unless a maintainer decides otherwise for that pull request
   ([fuzz/README.md](fuzz/README.md)).
2. Pull requests are squash-merged: the title becomes the commit on `main`, so give it a short
   English subject line.
3. If an AI agent helped, say so (a `Co-Authored-By:` trailer is fine).
4. A change that makes a valid program invalid, or changes what it means (a reserved word, a rule of
   `check` or `fmt`, a type rule), says so in a line of its description that starts with
   `Changes the language:`, and migrates the files of the repository in the same pull request. Until
   the first release this is allowed; afterwards it follows spec 14.1, new words.

## Licence

By contributing you agree that your contribution is licensed as MIT OR Apache-2.0, like the rest
of LabCraft ("inbound = outbound"). There is no CLA and no DCO sign-off: you keep the copyright of
your contribution. LabCraft is not affiliated with NI; product names are trademarks of their owners
(see [NOTICE](NOTICE)).
