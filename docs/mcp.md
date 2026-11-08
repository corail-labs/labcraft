# The MCP server

*Last updated: 2026-11-08. Status: draft for milestone M0b, nothing implemented.*

`labcraft-cli mcp` lets an agent write, check, run and fix LabCraft programs on its own (D-18). It is a
thin layer over the commands of [commands.md](commands.md): each tool is one command, with its
parameters, its result and its errors. Its acceptance criterion is scenario 1 of
[acceptance-m0.md](acceptance-m0.md): an agent given one prompt writes the PASS/FAIL program of the
multimeter, runs it against two simulated instruments and an absent one, and fixes it, with no human
message and at most 20 tool calls. Values marked *proposed* are starting points.

The server of the proof of concept (`tools/lcf-poc/mcp_server.py`: `check`, `list_functions`, `run`) was
the starting point; section 7 says what changes.

## 1. Starting it

```text
labcraft-cli mcp [--project DIR] [--lib-path DIR]... [--max-report N] [--max-file-size BYTES]
                 [--max-nesting N]
```

- **Transport:** stdio, JSON-RPC 2.0, one message per line, as MCP defines it. Nothing but MCP messages
  on stdout; stderr is for wrong usage at start, as for the other commands of cli.md.
- **Protocol revisions:** 2025-11-25, and 2025-06-18, which clients still send *(proposed)*; the server
  answers `initialize` with the one the client asked for, or the latest. Not 2025-03-26, which requires
  accepting batches of JSON-RPC messages, which the later revisions removed.
- **The project** is `DIR`, or the current folder: the root against which every path is resolved, and
  the only place the server writes ([commands.md](commands.md), section 2). An agent started in a library
  folder with no option works on that folder. A root that is the user's home folder, a folder above it,
  or the root of a drive is refused at start (wrong usage, exit 2): an agent started from its home folder
  would otherwise reach every program under it. A folder inside it is named with `--project`.
- **The options** are those of `check` (cli.md, section 1), applied to every command of the session.
- **Headless in M0b.** The session lives in the process. It checks the root as it starts and keeps the
  result, so that a tool that checks answers in the time of the libraries it touched
  ([commands.md](commands.md), section 6.2). When the IDE exists, the same server will
  attach to it through a local control channel protected by a token, so that the human watches the agent
  work (D-18); the tools stay the same.
- **Ends** when stdin closes: a run still going is aborted (spec 8.2) and its resources are released.

## 2. Tools

- **One tool per command** of [commands.md](commands.md), section 6. The name of the tool is the id of
  the command with `_` for `.`: `run_start`, `file_write`. Dots would be clearer, but the names of tools
  given to a model through the Claude API, among others, allow only letters, digits, `_` and `-`.
- **Description and schema** come from the registry: the `summary` and `description` of the command are
  the description of the tool, its `params` the `inputSchema`. Nothing is written twice.
- **Annotations** follow `effects`, and all four are given, since MCP takes a missing `destructiveHint`
  or `openWorldHint` as true: a command with no effect is `readOnlyHint`; `file.write`, `file.rename`,
  `file.delete` and `file.format` are `destructiveHint`, since they can replace or remove a file; `run.start` is
  `openWorldHint`, since it reaches instruments; every other hint is false.
- **Two ids, one name:** `a.b_c` and `a_b.c` would give one tool name, so the registry refuses such a
  pair ([commands.md](commands.md), section 1).
- **Eighteen tools in M0b**, few enough for each to be read. When the catalog grows (the commands of the
  viewer, then editing in M1), the server keeps the tools an agent uses most and reaches the others through
  `commands_list`, `commands_describe` and a `commands_execute`, as SolidCraft does *(proposed)*.

## 3. Results and errors

- **A result** is the JSON object of the command, in `structuredContent`, and the same JSON as text in
  `content`, for the clients that read only text.
- **A failed command** is a result with `isError: true` and the error object of commands.md, section 3:
  the model reads it, as MCP intends, and fixes its call. Wrong parameters are reported the same way
  rather than as a protocol error, so that the agent sees which one was wrong, as revision 2025-11-25 asks
  (SEP-1303: a client should pass a tool error to the model, and may drop a protocol error). SolidCraft
  still answers them with `-32602`.
- **A protocol error** (malformed JSON-RPC, an unknown method, an unknown tool) is a JSON-RPC error, with
  the codes of JSON-RPC (`-32700`, `-32600`, `-32601`, `-32602`). The server answers every request but one
  the client cancelled (MCP asks for no answer then), and keeps running after any of them. MCP needs no diagnostic code, so the family `LC9xxx` stays free (cli.md,
  section 5).
- **A program that does not check is not an error**: `library_check` and `file_write` return a report whose
  `status` is `error`, and `run_start` a report whose `status` is `refused`.

## 4. What the agent is told

**Instructions** of the server, given to the client at `initialize` (the text, *proposed*):

> LabCraft is a graphical dataflow language for test benches; its programs are text files. A library is
> a folder: one function per `.lcf` file, one type per `.lct` file, each file named after what it
> declares. Loop: `guide_read` once, `functions_list` to see what you can call, `project_describe` to see
> the files and the simulation files, `file_write` (it checks the library and returns the diagnostics),
> fix until the report has no error, then `run_start` against a simulation file (`simulation_describe`
> lists its instruments) and read `status`, `outputs` and `error`.
> Every diagnostic has a code, the node and the wires at fault, and often a fix.

They name no example and no file of the repository: in scenario 1 the agent has only the server and the
skills shipped with LabCraft, and `examples/` holds the answer.

**The skills** of `skills/` (roadmap) are what the agent learns the language from: syntax, types and
units, errors and the error wire, the order of side effects, `match`, loops, the panel, how to read a
diagnostic. A client that supports skills installs them; `guide_read` serves the same files from copies
built into the binary, as SolidCraft's `skill_reference` does, so that every MCP client reads them, and
they always match the version of the tool. Their code blocks are checked like those of the spec. They
hold no program of the acceptance scenarios.

**Progress.** A `run_start` that carries a `progressToken` gets a progress notification each second of
real time, with the clock and the iterations so far *(proposed)*. A cancellation (`notifications/cancelled`)
of a `run_start` or a `run_wait` stops the wait, not the run, which `run_abort` stops.

## 5. Bounds

The server reads hostile requests as `check` reads hostile files (scenario 4, family MCP):

- **A request** larger than 16 MiB *(proposed)*, as in SolidCraft, and above what the 4 MiB of a
  `file_write` take once quoted in JSON ([commands.md](commands.md), section 6.2), is refused before it is
  parsed, so its `id` is unknown: the answer is the JSON-RPC error `-32600` with `id: null`, and the server
  drops what follows, without keeping it, up to the next line feed, then reads on.
- **Nesting** of a JSON request is bounded at `--max-nesting` plus 16 levels *(proposed)*, so that an
  argument of any type that `check` accepts can be sent, and a deeper request is malformed. The values of a
  report are cut at 64 levels ([commands.md](commands.md), section 5).
- **Requests run in order**, except that `ping`, `run_status`, `run_set_element` and `run_abort` answer
  while a `run_start` or a `run_wait` is waiting: an agent can always stop what it started. These never
  wait in the queue. At most 16 other requests wait their turn *(proposed)*; one more gets the JSON-RPC
  error `-32000` ("server busy") at once.
- **A panic** in a command is caught by the session and returned as `internal`; the server goes on.
- **A report** is bounded by `--max-report` (cli.md, section 1) and by the rules of commands.md, sections 5
  and 7.

## 6. What an agent may touch

- **Files:** the server writes only `.lcf`, `.lct` and simulation files under the root of the project, and
  its traces under `.labcraft/` ([commands.md](commands.md), section 2). It never writes a shipped library.
- **Instruments:** in M0b only the `SIM::` resources of a simulation file exist, since `visa` has no real
  transport before M4. A real instrument can switch on a supply, so from M4 an agent reaches one only if
  the human allowed it when starting the server (an option of `mcp` such as `--allow-hardware RESOURCE`,
  *to be specified in M4*); no tool can grant it.

## 7. From the proof of concept to M0b

| Proof of concept | M0b | Why |
|---|---|---|
| The agent writes files with its own tools | `file_write` writes and checks in one call | One call instead of two in the loop, and a client with no file tools can work |
| `check` gives its own diagnostics | `labcraft-check/1`, the report of `labcraft-cli check --json` | One schema for the CLI and MCP (scenario 2) |
| `list_functions` with `builtins` | `functions_list`, signatures written as the language writes them | The agent reads what it will write |
| Instructions point to the spec and `examples/` | They point to `guide_read` | Scenario 1: the agent does not see the answer |
| Instruments found by reading the TOML | `simulation_describe` | The agent learns the resource names without the repository |
| `run` returns its own report: a status (`ok`, `error`, `refused`, `aborted`, `fault`), the simulated time, the sessions left open and the commands exchanged | `labcraft-run/1`: the four statuses of spec 8.2 and `refused`; the same facts, with the resources opened and leaked, and the commands dated per instrument | Scenarios 1 and 3 read these; the fields of the proof of concept are kept, under the names of the report |
| Panel settings (`set`) and an abort by `timeout`, given when the run starts | `run_abort`, `run_set_element`, runs that outlast a call | Scenario 3 |
| The trace inline only | Also a file, `labcraft-trace/1`, that the viewer reads and `run_trace` reads in pages, filtered by node | Scenario 6, point 7; an agent debugs without a file tool |

## 8. Scenario 1, call by call

What a direct agent spends, to check the bound of 20 calls *(proposed)*:

| # | Tool | Why |
|---|---|---|
| 1 | `guide_read` | The skills, and the one that writes a program |
| 2 | `functions_list` | `visa.open`, `scpi.query_f64`, `visa.close`, `core.ge`… |
| 3 | `project_describe` | The simulation files, of the folder and shipped with `visa`, and their resources |
| 4 | `file_write` `Verdict.lct` | The enumeration of the verdict, checked |
| 5 | `file_write` `check_voltage.lcf` | The program, checked, with, say, one error |
| 6 | `file_write` `check_voltage.lcf` | Fixed, clean |
| 7–9 | `run_start` × 3 | `dmm-230V`, `dmm-250V`, `no-instrument`: PASS, FAIL, an error and not PASS |

Nine calls, with room for two or three rounds of fixes. The scripted client of the CI plays the same
loop over real stdio ([acceptance-m0.md](acceptance-m0.md), scenario 1, verification).

## 9. Left for later

Taken from SolidCraft, each when what it serves exists:

- **Attached to the IDE** (M1): the control channel of the desktop app, on `127.0.0.1` only, a random
  token for each launch in a file only the user can read, a file that gives the port so that `mcp` with no
  option finds the open app, and a `running` tool that says what the app runs for every client. The tools
  stay those of section 2.
- **`batch`**: several commands in one call, stopped at the first failure and rolled back, once commands
  edit the graph (M1).
- **`crash_reports`**: the crash reports of this computer, each with the commands that led to the bug,
  so that it replays.
- **`mcp setup`**: writes the entry of the server in the configuration of the MCP clients found on the
  computer (Claude Code, Cursor, Codex…), changing nothing else in those files.
- **Resources**: the catalog and the project as MCP resources, for the clients that read them.
