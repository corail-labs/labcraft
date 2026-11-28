# Contributing to LabCraft

Agents: start from [AGENTS.md](AGENTS.md). This page is the guide for people.

LabCraft is an AI-native graphical dataflow language and IDE, written to be read, run and fixed by
people and by AI agents alike. The goal is to do at least what LabVIEW does, in pure Rust, open and
cross-platform.

## Contribute with a coding agent

You don't need to know Rust: a coding agent (Codex, Claude Code, Cursor, Copilot, Gemini or another) can do the
work, and you decide. You need a GitHub account, [git](https://git-scm.com/), [Rust](https://rustup.rs) and, to fork
and open the pull request from the agent, the [GitHub CLI](https://cli.github.com/) (`gh auth login` once). Open
your agent in an empty folder and paste this prompt:

```text
I want to contribute to LabCraft (https://github.com/corail-labs/labcraft) with you as my coding agent.

1. Fork and clone it here: gh repo fork corail-labs/labcraft --clone, then work inside
   the folder labcraft it makes. Read AGENTS.md: its rules bind you, the
   non-negotiables first.
2. List the open issues labelled "good first issue" that nobody holds: no comment
   "I'm working on this" from the last 7 days (gh issue view N --comments), no open
   pull request that links them (gh pr list --search N).
   Show me up to three, with what each asks, and let me choose.
3. Read the issue I choose against the documents it names. If it contradicts them,
   tell me and draft a comment for the issue, then stop.
4. Draft the comment "I'm working on this" for the issue; post it when I agree.
5. Run cargo xtask setup. Do what the issue asks and nothing it puts out of scope;
   ask me before you change any other behaviour.
6. Run cargo xtask ci and the commands of the issue's "How to check", and show me
   their output.
7. When I agree, push a branch to my fork and open a pull request whose body is
   .github/pull_request_template.md filled in, every box answered: "Closes #N",
   what changed, the commands you ran and their output.
```

**Claiming an issue.** Comment "I'm working on this" on the issue before you start. The claim holds 7 days, and each
comment that shows progress (a commit, a draft pull request) renews it; a comment without either does not. After 7 days without news anyone may take the issue over,
and the pull request credits both (a `Co-authored-by:` trailer). When two pull requests close the same issue, the
first one that does what it asks is merged.

**What gets reviewed.** Review is what costs the project, so a pull request without a linked issue, or that does
what the issue puts out of scope, is closed without review; a fix of a few lines of prose (a typo, a broken link) is
the only exception. A maintainer's pull request is exempt, and links its issue when there is one. To propose
something else, open an issue first (the templates are `Task`, `Bug` and `RFC`). An
issue can be wrong: when it contradicts the documents, say so on the issue instead of following it.

## Where the project is

LabCraft has **done M0, its first milestone in Rust** (tag `v0.3.0-m0`): `labcraft-cli` reads, checks, writes and
runs programs against simulated instruments and serves them to an agent over MCP, and a viewer draws them. Release
0.4.0 (tag `v0.4.0`) makes the viewer an editor, with an agent attached to the window; 0.5.0, the first real bench,
is under way. The repository holds the language and file format specification, the decisions behind
it, example programs and the Rust workspace. The throwaway Python tools of the specification phase (a proof of
concept and a viewer) were retired after M0 (2026-11-15).

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

Every complete code block of the spec and of `skills/`, and every file in `examples/` and `libs/`, must read, the
examples, the RFC programs (`docs/rfc/`) and the shipped libraries (`libs/`) must check, and all of them must be in
canonical form (spec 5.2). Rust tests check this, as part of `cargo xtask ci`, which the `checks` workflow runs on
every pull request; a pull request that changes only prose skips the parts that build and run code
(`docs/architecture.md`, section 8). The hooks of the agents (`.claude/hooks/`) have Python tests, with the
standard library only: `cargo xtask hooks` runs them.
The tests include the corpora of `tests/acceptance/`, which say what `check` and `fmt` must report.
An expected value in a test or a corpus never comes from the output of the program under test: work it
out by hand or with another tool, and say how.

The corpus of runs and the Rust `labcraft-cli run` are the reference for running a program, and the corpora of
`tests/acceptance/` and the Rust `labcraft-cli check` are the reference for `check`; the spec decides where they
differ. The grammar exists once, in the spec (section 14, EBNF). `.lcf` and `.lct` files use LF line endings.

## Rules that aren't negotiable

1. **Independent of NI.** LabCraft is designed from public documentation, general domain knowledge
   and user feedback. Don't use LabVIEW to design LabCraft: no running, scripting, inspecting or
   screenshotting it for this project, no `.vi`, `.ctl`, `.lvproj` or `.lvlib` files, no LabVIEW
   benchmarks. The CI refuses a pull request that adds such a file (`cargo xtask clean-room`). Don't contribute NI code, icons, help text, screenshots or examples. Describing your
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

1. Every change goes through a pull request, the maintainers' too, linked to its issue (above; a maintainer links one when there is one). Keep it to one topic
   and fill the template: what you checked, with the commands and their output; the `checks` workflow must be green.
   `cargo xtask setup` installs what `cargo xtask ci` needs. GitHub requires only its job `checks`, so that a
   broken nightly or runner can never block `main`, but a red job `windows`, a red `fuzz` workflow or a red
   `release` workflow also stops the merge unless a maintainer decides otherwise for that pull request
   ([fuzz/README.md](fuzz/README.md)).
2. Pull requests are squash-merged: the title becomes the commit on `main`, so give it a short
   English subject line.
3. Saying that an AI agent helped is accepted, not required: a `Co-Authored-By:` trailer is fine, and so is
   nothing.
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
