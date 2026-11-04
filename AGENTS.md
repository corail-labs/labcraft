# LabCraft — instructions for agents

LabCraft is an AI-native graphical dataflow language and IDE, aiming to do at least what LabVIEW
does, in pure Rust, open and cross-platform. It follows the conventions of the Craft apps
(storytold, SolidCraft); see `docs/craft-ecosystem.md`.

**Current phase: planning and specification. There is no product code yet.** The only code is
throwaway tooling (`tools/`). Do not scaffold the Rust workspace until the owner says so.

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

## When you change the spec or the examples

- The grammar exists twice: spec section 14 (EBNF) and `tools/lcf-view/lcf_view.py` (Lark). Keep
  them in sync.
- Every complete code block of the spec and every file in `examples/` must parse:

  ```sh
  python tools/lcf-view/lcf_view.py examples --spec docs/language-spec-v0.md
  ```

  Blocks containing `…` are illustrative fragments and are skipped. Requires `pip install lark`.
- `examples/` is also a library for the Python proof of concept: it must pass
  `python tools/lcf-poc/lcf.py check examples`, and `python -m unittest discover tools/lcf-poc/tests`
  must stay green (needs `pip install -r tools/lcf-poc/requirements.txt`).
- Examples and spec code blocks must be in canonical form (spec 5.2: order of lines and layout of
  the text): `python tools/lcf-poc/lcf.py fmt --check --spec docs/language-spec-v0.md examples`.
- `libs/visa` and `libs/scpi` (libraries shipped with LabCraft) follow the same rules: `check` each
  one, `fmt --check libs`, and `lcf_view.py` with `libs` next to `examples`.
- `docs/rfc/` (programs discussed in the RFCs) follows the same rules: `check docs/rfc` and
  `fmt --check docs/rfc`. Its diagrams in `docs/images/rfc/` are rendered by `tools/lcf-figcraft`;
  rerender them when a file changes.
- `lcf_view.py` also writes `lcf-view.html` (diagrams of `examples/`), which is ignored by git.
- `.lcf` and `.lct` files use LF line endings (`.gitattributes`).

## Commits

- Commit or push only when the owner asks. Changes in `plan/` are committed separately, directly.
- Every change to this repository goes through a pull request, the owner's too: one branch per
  topic, the `checks` workflow green, squash merge. `main` is protected: never push to it.
- Open the pull request with `gh pr create`; merge only when the owner asks.
- Short English subject line; end with the `Co-Authored-By:` trailer, as in the Craft repos.
- Contributions are licensed MIT OR Apache-2.0, inbound = outbound, no DCO or CLA (D-19).

## Working economically

- Be concise. Prefer one good document change over many small exchanges.

## Later, when code starts

The Craft rules will apply (see `docs/craft-ecosystem.md`): never crash (no `unwrap`, `expect`,
`panic!` or `unsafe` in production code), everything is a command, layered crates checked by
`cargo xtask layers`, never break the WASM build, every asset attributed.
