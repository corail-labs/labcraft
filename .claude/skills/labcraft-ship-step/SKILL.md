---
name: labcraft-ship-step
description: Take a finished LabCraft change from the local gate to a merged pull request, without calling it green too early and without merging or starting the next step on your own. Use when a step or topic is ready to be proposed, or when the owner asks to open or merge its pull request.
---

# Ship a change

Each stage needs what the stage before it proved. Commit, push and merge are the owner's call (`AGENTS.md`): ask about one thing at a time, and an answer covers only what the question plainly asked.

## 1. Local gate

On a topic branch, never on `main`:

```sh
cargo xtask ci
```

If the spec, `examples/`, `libs/` or `docs/rfc/` changed, also run the checks listed in `AGENTS.md`, "When you change the spec or the examples".

## 2. Independent review

First, when the change has real code (not a change of documents alone, nor of one line), run the `simplify` skill, so that the review spends itself on defects rather than on cleanups; then run the local gate again. Say in the pull request whether it ran, or why not.

Then, unless this change has had one, launch the `labcraft-reviewer` agent with the scope, the documents and the owner's decisions on this change, even those not written anywhere yet: they are part of the contract. Never give it your own conclusions.

- **Name sections, not documents**: "spec, sections 7.3 and 14", "architecture, section 9.4", not the whole spec. The large documents cost more to read than the review itself; the agent opens another part only to check a precise point.
- **Choose its model and effort** with the `model` and `effort` of the Agent tool:
  - Opus, `high`: the change decides or changes a rule of the language, or touches reading, checking or running hostile input (bounds, recursion, sizes);
  - Opus, `medium` (its default): other code;
  - Sonnet, `medium`: tests, configuration, CI, or documents that decide nothing.

When the change is mostly code, also run the `code-review` skill at `medium` (the owner's decision, 2026-11-04, level `medium` since 2026-11-15): it finds line-level bugs that the agent misses, while the agent covers the contract, the bounds and the later steps. On a change that is mostly tests, configuration or documents, the agent alone is enough.

Sort the findings of both into:

- simple fixes, which you make;
- decisions, which go to the owner with a recommendation;
- what you leave as it is, and why.

If the rewrite was large, one targeted check of it is enough. Then stop reviewing: the pull request takes over.

Say in the pull request which reviews ran, with the model and effort of the agent, and why one did not run.

## 3. Propose, then commit

Tell the owner what is ready, the rules of the language that it decides, and what is still open. Commit and open the pull request only once they agree.

- One branch per topic, a short English subject, and the attribution trailers of the session.
- Write the body for someone who never saw this conversation: what it adds, what to read closely, what is not settled, and how it was tested. Say what was not run, and which results come from probes that are not in the repository.
- If the change makes a valid program invalid or changes what it means, the body has a line that starts with `Changes the language:` (`CONTRIBUTING.md`, point 4), and the files of the repository are migrated in the same pull request.

Name the files in `git add`, rather than adding everything.

## 4. Wait for the check

`gh pr checks --watch` gives up at once when GitHub has not created the check yet. Use the script of this skill, in the background:

```sh
sh .claude/skills/labcraft-ship-step/wait_checks.sh PR_NUMBER
```

Exit 0 means green, 1 failed, 2 not finished in time.

**Green means all three:** `gh pr checks` passes, each check run of the head commit is `completed` / `success`, or `skipped` when its job is meant to be skipped (the publishing job of `release` on a pull request), and `mergeStateStatus` is `CLEAN`. The status of the workflow run alone can disagree with these, so never report it as green.

If no check appears, look at the "Actions" component on githubstatus.com before suspecting the repository. Don't push or reopen anything without asking.

## 5. Merge

Only on the owner's word for this pull request, and only when green:

```sh
gh pr merge PR_NUMBER --squash --subject "SUBJECT (#PR_NUMBER)" --body "TRAILER" --match-head-commit HEAD_SHA
```

If a permission check refuses the merge, say so and leave the choice to the owner. Don't look for another way to do it.

## 6. After the merge

```sh
git switch main
git pull --ff-only
```

Check in the logs of the run that the jobs `lint`, `test`, `release` and `web` really ran their steps. If `plan/` exists, do what `plan/ship-step.md` says.

Then stop, and report. Starting the next step is the owner's decision.
