@AGENTS.md

## Claude Code

What follows holds only for Claude Code; the rest of the rules are in `AGENTS.md`, for every agent.

- **Hook.** `.claude/settings.json` runs `.claude/hooks/guard.py` before each tool call. It refuses a path, glob or
  command that names a LabVIEW file (D-23), and, for this repository only (a fork is free), a `git push` that writes
  `main` and a merge without `--match-head-commit`, with `--admin` or through `gh api`. A guard rail, not a sandbox:
  never look for a way around it. Its tests run in `cargo xtask hooks`.
- **Agents.** For a case of `tests/acceptance/`, the agent `labcraft-oracle` works its `expected.json` out from the
  documents alone: give it the case folders, never the code, the output or your values. Before a pull request, the
  agent `labcraft-reviewer` reviews the change independently: give it the scope, the sections of the documents that
  are its contract and the decisions of the person you work for, never your conclusions.
