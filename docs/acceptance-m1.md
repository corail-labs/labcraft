# M1 acceptance scenarios

*Last updated: 2026-11-28. Status: the scenarios pass for 0.4.0 (tag `v0.4.0`), except the time of the new drawing
after a drop (scenario 6, point 2), above its proposed budget (#376), and in scenario 1, the unit of `with_unit`, which
has no gesture yet (#378).*

M1 is the diagram editor ([roadmap](roadmap.md)): the user draws a program with the mouse and the keys, an agent
edits the same files through the MCP server attached to the IDE, and both share one history. M1 is done when these
scenarios pass; 0.4.0 closes it with the two exceptions of the status line. Each gives its setup, what is expected,
and how it is verified. The contract is in [commands.md](commands.md) (section 9),
[visual-notation.md](visual-notation.md) (2.4 and 2.6), [ide.md](ide.md) and the spec (section 11); the arrangement of the code is [architecture.md](architecture.md), section 12.

Values marked *proposed* are starting points, to adjust when the scenarios are automated.

## Common conditions

- **The conditions of M0 hold** ([acceptance-m0.md](acceptance-m0.md)): never a crash, deterministic runs by
  default, the expected values of a test worked out by hand or by another tool, never by the program under test.
- **Through the tool, as a user would.** The IDE on the desktop and on the web, `labcraft-cli`, and the MCP server,
  headless or attached to the IDE. Every edit is a command of `engine`, whoever makes it.
- **The file is the truth** (commands.md 9.1): what the diagram shows, the file keeps; closing and opening the
  project gives the same drawing.
- **Automated** in the tests of the workspace and the `fuzz` workflow, except what a person or an agent plays and what
  only a reference machine measures: scenario 1, what the window shows in scenario 2, and scenario 6, done by hand
  before the tag of M1. The
  agent is Claude Code with the most capable model of the time; its transcripts go with the pull request of the run.
- **WASM** (D-22): `edit` and `engine` pass the tests of scenarios 3 and 5 in a `wasm32-unknown-unknown` build.

## Scenario 1: a program drawn by hand

**Setup.** A new project (`New project`), the function `main` empty, and, copied from `examples/`, the files that no
gesture of M1 makes: the type `Verdict` (`Verdict.lct`) and a simulation file of a multimeter
([libs/visa/simulation.md](../libs/visa/simulation.md)). The owner builds
`check_voltage` of `examples/`, from the documents of the example alone, with the mouse and the keys, in the `Diagram` view, without
opening the `Text` view.

**Expected.**

1. Every step goes through a gesture of notation 2.4 or a key of 2.6: quick insertion by name, wiring, constants,
   the signature, renaming.
2. The file passes `labcraft-cli check` with no error, and `labcraft-cli run` with that simulation file gives PASS.
3. At no moment does the file fail to parse (commands.md 9.1); while it is incomplete, the diagram shows `LC2101` where
   it is.

**Verified** by hand, by the owner, before the tag; the time it took and what got in the way are written in the pull
request of the tag.

**Checked on 2026-11-27** by the owner, in the desktop app built from `9e70754`: about 40 minutes from the empty `main`
to a file that `check` passes; `run` with the simulated multimeter of `libs/visa/simulation.md` gave `Verdict.Pass`,
230.071217 V, as the example does. Each of the 106 versions that the app saved was read by `check`: none failed to
parse, and the incomplete ones gave `LC2101` and `LC4002`. Two steps had no gesture that worked:

- The unit of `with_unit` has no gesture: it is no terminal, and no field opens for it (#378). The drawing used
  `core.mul(value, 1 V)` instead.
- A wire dropped on the `_` of a `_ = handle` that gave nothing yet was refused by nothing and did nothing, so the
  `handle` that gives `Verdict.Error` was left empty (#379, fixed before the tag by #381: the drop adds the output).

The `handle` was finished the same day, in the app built from `5743324`, by gestures alone (the journal has no write
from outside): a constant `Verdict.Error` in its arm `_`, its output wired to `verdict`, and the value of the `match`
yielded by its arm `ok`. None of the 13 versions saved failed to parse; the two incomplete ones gave `LC2101`. `run`
gave `Verdict.Pass`, 230.071217 V, and `Verdict.Error` with `visa.resource_not_found` for an instrument that the
simulation file does not have.

What got in the way besides: a double-click on an input that holds nothing does not open a field for a constant, and
the value of a constant node is not edited in place, so a constant comes from quick insertion on a wire dropped on the
canvas; a wire dropped on the right edge of a `match` does nothing; a terminal of the frame of the function, dragged,
moves along its edge, so no wire starts from it (#380).

## Scenario 2: the agent edits, the user watches and undoes

**Setup.** The IDE open on a project with `check_voltage`; an agent attached (`labcraft-cli mcp`, ide.md 8) with the
policy `Edits`.

**Expected.**

1. The agent adds a node, wires it and sets a constant, through the direct tools, and through `commands_execute` for a
   command that is not one (mcp.md 2). The history signs each step with the name of the agent, and the window shows
   the edit, the nodes it touched highlighted (ide.md 8).
2. The user presses Ctrl+Z: the last step of the agent is undone, the status bar says whose it was, and the file is
   the one before that step, byte for byte.
3. While the user draws a wire, an editing command of the agent waits, then runs when the gesture ends (commands.md
   9.1); a command whose `expected_revision` is stale by then is `file.stale` and changes nothing. A gesture that
   lasts more than 30 s *(proposed)* gives the agent `gesture.busy`.
4. The agent calls `history_undo` after a step of the user: `history.not_yours` (commands.md 9.5).
5. Under the policy `Ask`, an editing command waits for the user's answer; refused, it is `agent.not_allowed`.

**Verified.** Points 1, 3, 4 and 5, as commands and their results: a test of `mcp` over the control channel on the
loopback, with a session of `engine` whose gestures and answers the test gives. Point 2, and what the window shows
of the agent: by hand, with an agent, before the tag.

**Checked on 2026-11-28** with Claude Code (Opus 5.5) attached to the desktop app built from `9e1be6a`, under the
policy `Edits`, on a copy of `examples/`. In `check_voltage`, the agent added `deviation = core.sub()` (`node_add`),
wired `voltage` to `deviation.a` (`wire_connect`), set `230 V` on `deviation.b` (`input_set_constant`) and moved
`deviation` by 80 units through `commands_execute` (`node.move`); each step went into the journal of the window,
signed `claude-code`. The window drew each edit as it came, highlighted the node it touched, and its pill named the
agent and its policy. Ctrl+Z undid the move, the status bar said `Undo claude-code Move Nodes`, and the file was the one
before the move, byte for byte (`cmp`, sha256 `0c5361eb…`). What got in the way: a `labcraft-cli mcp` started before
the window stays headless, as ide.md 8 says, and its edits reach the window as writes from outside; the client must
start it again once the window is open. The transcript of the agent is attached to the pull request of the tag.

## Scenario 3: undo gives back every byte

**Setup.** Every function of `examples/`, `docs/rfc/` and `libs/`, copied into memory (`engine::Memory`).

**Expected.**

1. For each command of commands.md 9.4 that applies to the function (each node, each wire, each input that the command
   can take), the command then `history.undo` gives back the bytes of every file it touched, even a file that was not
   canonical before.
2. A sequence of n editing commands then n undo gives back the bytes of the start; then n redo gives the bytes after
   the sequence.
3. A `commands.batch` is one step: one undo takes all of it back; a batch whose command fails writes nothing.

**Verified** in CI, on the native and WASM builds. The expected texts after each command come from the unit tests of
`edit`, written by hand; undo is checked against the bytes read before.

## Scenario 4: editing at random never breaks

**Setup.** A target of `fuzz/`: a project in memory from the examples, then a sequence of at most 500 steps *(proposed)*,
below the bound of the history: editing commands with parameters drawn at random (names that exist and names that do
not, positions, paths, revisions), undo and redo, and external writes of random bytes, on a clock that the target
gives (`engine::Still`), so that the merging of external writes is the same at each run.

**Expected.** Never a panic, and never `internal`: every error is another code of commands.md 3. After each editing
command, every file that it wrote parses (commands.md 9.1); undo and redo write back what was there, which may be the
random bytes of an external write. After the whole sequence, as many undo give back the bytes of the start, external
writes included.

**Verified** by the `fuzz` workflow, a short run of each target on each pull request that changes code; 8 hours
before the tag.

**Long fuzzing, run (2026-11-25 to 2026-11-27)** with `fuzz/long.sh`, in parts of one hour, on the commit of #356
(`6d7c7c6`), before the tag of 0.4.0. `diagram` ran 8 hours without a crash, an out-of-memory or a leak; its 6 timeouts
each passed when run again alone (the clock of Docker Desktop). `edit` first stopped on timeouts of 3 to 11 s: run again
alone, an input took 0.1 to 0.3 s without a sanitizer, 0.7 to 2.4 s under it on a quiet machine, and up to 26 s beside a
build, the time of each step flat, nothing quadratic, most of it the check after each command; #359 gave `edit` 60 s an
input, as the other checking targets. Then `edit` ran 8 hours (2026-11-26, 22:36Z to 06:36Z) without a crash, an
out-of-memory, a leak or an `internal`. 12 of its parts were cut by a negative timeout, `ALARM: working on the last Unit
for -11 seconds`, the clock of WSL2 jumping back, and each of their units passed alone; `long.sh` counts only whole
parts, so its log says 2 hours of 8 (#375). Since that commit, #367 added `project.download` to `engine`, run by the
short fuzzing of its pull request.

## Scenario 5: moving a node changes one line

**Setup.** Every function of `examples/` with at least one node.

**Expected.**

1. Moving one node that `layout` does not hold adds one entry to `layout`, and moving a node that it holds changes
   that entry alone; no other line of the file changes (commands.md 9.1, spec 11). The diff of git is that one line,
   or, when the file had no `layout` section, the section with that one entry (spec 5.2); a node that the section
   cannot name gets its label too, on its own line (commands.md 6.7, `node.move`; decision of the owner, 2026-11-17).
2. The other nodes stay automatic, and move as little as the placement allows: the figure of stability (notation 2.1:
   how many nodes a neutral node added on a wire moves) is, on each example, no worse than at the tag `v0.3.0-m0`,
   and, since the primitives of 24 (#264), then the blocks of `core` (#305), no worse than there: their sizes changed
   the figures by design (decisions of the owner, 2026-11-20 and 2026-11-22); since #266, where a shift of the whole
   drawing, which the editor follows, stopped counting as a move, no worse than there (decision of the owner,
   2026-11-23); since #279, where the wires between neighbours are straightened, no worse than there (decision of the
   owner, 2026-11-23: a better drawing for figures somewhat worse). That is the threshold the roadmap left to M1. It is a guard against a regression, not an expected
   value: the figures are written once, by the code of the tag, then by that of #264, of #305, of #266 and of #279, into a file
   of `tests/figures/`, never again from the code under test.
3. Resizing the frame, moving a terminal and routing a wire by hand each change one entry of `layout` (spec 11), as
   in point 1.

**Verified** in CI: points 1 and 3 by a test that moves each node, terminal and frame and counts the entries that
change; point 2 by `cargo xtask figures --against` that file, which fails when a figure is worse.

## Scenario 6: 60 frames per second while editing 1,000 nodes

**Setup.** The function of 1,000 nodes and 2,000 wires of `cargo xtask frames` (architecture 11.6), open in the
desktop app, in release, on the reference machine of M0.

**Expected.**

1. Dragging 100 nodes at once keeps 60 frames/s: the preview of the drag (architecture 12.4) takes at most 4 ms a frame
   *(proposed)*.
2. At the drop, the file is written and the new drawing comes within 300 ms *(proposed)*; no frame waits for it.
3. Wiring, selecting with a rectangle and quick insertion keep 60 frames/s.

**Verified** by hand before the tag, on the reference machine, as for M0: `cargo xtask frames` gains a scripted drag and
reports the time of the preview and of the new drawing, then the gestures by hand. Times on the shared runners of CI
vary too much to be a check.

**Checked on 2026-11-27** with `cargo xtask frames`, on the laptop of M0 (Intel Iris Xe through Vulkan, a 60 Hz
screen), the desktop app built from `9e70754`, three runs:

1. The drag of 100 nodes held 60 frames/s, no frame late of 181 in each run; the preview took at most 0.41 ms at the
   95th percentile, 1.79 ms at most, of 4.
2. After the drop, no frame waited: 60 frames/s, the longest frame 18.2 ms. The new drawing came in 399, 417 and
   417 ms, above the 300 ms proposed; `labcraft-cli` checks the same file in about 55 ms and draws it in
   about 100 ms, so the rest is on the way of the desktop app (#376). Until #377, the scripted drag came back where it
   started, and its drop moved nothing: the first runs could not time the new drawing.
3. The motion of 12 s held 60 frames/s, no frame late of 721 in each run; wiring, selecting with a rectangle and quick
   insertion by hand, 60 frames/s, no frame late of 7644 in 127 s, and no stutter seen.

With the screen off, Windows presents one frame in two: every frame then takes 31 ms and the window shows 32 frames/s,
whatever it draws. The runs above were made with the screen on.

## Scenario 7: a write from outside is a step like any other

**Setup.** The IDE open on a project; a file of it rewritten by another program (an editor, git, `labcraft-cli`).

**Expected.**

1. The diagram shows the new file, and the history gains a step signed `external` (commands.md 9.1); several writes
   of the file within 1 s *(proposed)* are one step.
2. Ctrl+Z gives back the bytes of the file before the external write; redo gives back those after it.
3. An agent whose `expected_revision` names the old revision gets `file.stale`, with the current revision.
4. A write that renames a value in the text (spec 11) keeps the place of its node and the routes of its wires: the
   entries of `layout` take the new name, in the same step, and Ctrl+Z gives back both the text and the `layout`.

**Verified** in CI, with a session of `engine` and writes made by the test to the disk; the watch of the desktop app
by a test of the app, as in M0v.

## Scenario 8: the web page edits and saves

**Setup.** `labcraft-web`, built by `cargo xtask web`, in headless Chrome.

**Expected.**

1. The page opens an example as a project in memory, and the same window as the desktop edits it: a node added and
   wired through the commands of the page.
2. Ctrl+Z gives back the file byte for byte.
3. `Download` gives a zip whose files, unpacked, pass `labcraft-cli check` and hold the edit.
4. The settings of the user stay after the page is loaded again (`localStorage`), and those of the project are in
   the zip, in `.labcraft/config.toml`.

**Verified** by `cargo xtask web-smoke`, which the job `web` runs. Points 3 and 4, the zip of `Download`, since #367;
the download of the browser itself and a real folder dropped on the page were tried by hand there.

## Scenario 9: a function made from a selection

**Setup.** `examples/check_voltage.lcf` open in the IDE; the user selects `above`, `below` and `in_range`, then "Make
a function…" and types `in_limits` (visual notation 2.6, commands.md 9.8).

**Expected.**

1. `in_limits.lcf` appears beside it, with the three nodes, the inputs `voltage`, `low` and `high` and the output
   `in_range`; one call stands where the nodes were, and the `match` still reads `in_range`.
2. `labcraft-cli check` gives no new error, and `run` gives the same outputs as before the gesture.
3. One Ctrl+Z removes `in_limits.lcf` and gives back `check_voltage.lcf` byte for byte; redo makes both again.

**Verified** in CI by tests of `engine` that make the same gesture on functions written for them, their texts worked out
by hand from commands.md 9.8, and the field by a test of the window; on `check_voltage` itself, with point 2, by hand
before the tag.

**Checked on 2026-11-27** by the owner, in the desktop app built from `9e70754`, on a copy of `examples/`, with the
name `in_limit` typed in the field. Before the
gesture, `labcraft-cli check` gave no error and no warning, and `run --sim` with the multimeter of
`libs/visa/simulation.md` and `--arg resource=SIM::DMM0` gave `verdict = Verdict.Pass`, `voltage = 230.071217 V`,
`err = none`. After it, `in_limit.lcf` held the three nodes, the inputs `voltage`, `low` and `high` and the output
`in_range`, and one call `in_range = in_limit(voltage, low, high)` stood in `check_voltage`; `check` gave no error and
no warning, with one function more, and `run` gave the same three outputs. One Ctrl+Z removed `in_limit.lcf` and gave
back `check_voltage.lcf` byte for byte (the same SHA-256 before and after), and redo made both again.

## Not covered by these scenarios

The commands that each scenario does not name (structures, the block of a function, the mask of a library, the
`Text` panel) have their own tests, as ordinary tests of `edit`, `engine` and `ide`; scenario 3 covers their undo.
Detached windows, the start page and `Connect an AI agent` are tried by hand before the tag.

## Summary

| Scenario | In CI | By hand, before the tag |
|---|---|---|
| 1. Drawn by hand | | The owner builds `check_voltage` |
| 2. Agent and user | Points 1, 3, 4, 5, as commands | An agent attached, point 2, what the window shows |
| 3. Undo byte for byte | All, native and WASM | |
| 4. Editing at random | Short fuzzing, each pull request | 8 hours of fuzzing |
| 5. One line per move | All, the figure of stability against `v0.3.0-m0` | |
| 6. 60 frames/s | | Frame rate, time of the preview and of the drawing |
| 7. External writes | All | |
| 8. Web | `web-smoke` | |
| 9. A function from a selection | The gesture, as commands and in the window | On `check_voltage`, point 2 |
