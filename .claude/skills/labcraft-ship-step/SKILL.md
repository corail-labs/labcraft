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

Unless this change has had one, launch the `labcraft-reviewer` agent with the scope and the documents only, never your conclusions. Sort its findings into:

- simple fixes, which you make;
- decisions, which go to the owner with a recommendation;
- what you leave as it is, and why.

If the rewrite was large, one targeted check of it is enough. Then stop reviewing: the pull request takes over.

## 3. Propose, then commit

Tell the owner what is ready, the rules of the language that it decides, and what is still open. Commit and open the pull request only once they agree.

- One branch per topic, a short English subject, and the attribution trailers of the session.
- Write the body for someone who never saw this conversation: what it adds, what to read closely, what is not settled, and how it was tested. Say what was not run, and which results come from probes that are not in the repository.

Name the files in `git add`, rather than adding everything.

## 4. Wait for the check

`gh pr checks --watch` gives up at once when GitHub has not created the check yet. Use the script of this skill, in the background:

```sh
sh .claude/skills/labcraft-ship-step/wait_checks.sh PR_NUMBER
```

Exit 0 means green, 1 failed, 2 not finished in time.

**Green means all three:** `gh pr checks` passes, the check run of the head commit is `completed` / `success`, and `mergeStateStatus` is `CLEAN`. The status of the workflow run alone can disagree with these, so never report it as green.

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

Check in the log of the run that the Rust gate really ran. If `plan/` exists, do what `plan/ship-step.md` says.

Then stop, and report. Starting the next step is the owner's decision.
