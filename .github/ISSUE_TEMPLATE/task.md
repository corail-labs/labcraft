---
name: Task
about: A small, well-specified piece of work that a person or a coding agent can take and finish in one pull request
title: ""
labels: ""
---

<!-- Write it for someone who has never seen the project: an agent will follow it to the letter, so check each
claim against the documents before you post it. -->

**Documents:** the sections of `docs/` this task rests on (for example: spec 5.4, `docs/architecture.md` 8).

## Context

What is wrong or missing today, and where: the file and line, or the command and what it gives.

## What to do

The change, in a few points.

## Out of scope

What this task must not change, even when it looks related. A pull request that does more is closed without review.

## Files

The files the change should touch, as far as they are known.

## How to check

The commands to run, and the output each must give. Work the expected output out by hand or from the documents,
never from the program under test, and say how.

```sh
```

## Done when

- [ ] `cargo xtask ci` passes.
- [ ] …
