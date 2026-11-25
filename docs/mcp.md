# The MCP server

*Last updated: 2026-11-19. Status: milestone M0b: implemented in `labcraft-mcp` and `labcraft-cli mcp` (step 9a),
with `guide_read` and the skills of [skills/](../skills/README.md) it serves (step 9b).*

`labcraft-cli mcp` lets an agent write, check, run and fix LabCraft programs on its own (D-18). It is a
thin layer over the commands of [commands.md](commands.md): each tool is one command, with its
parameters, its result and its errors. Its acceptance criterion is scenario 1 of
[acceptance-m0.md](acceptance-m0.md): an agent given one prompt writes the PASS/FAIL program of the
multimeter, runs it against two simulated instruments and an absent one, and fixes it, with no human
message and at most 20 tool calls. Values marked *proposed* are starting points.

The server of the proof of concept (`mcp_server.py`: `check`, `list_functions`, `run`; retired after M0, 2026-11-15) was
the starting point; section 7 says what changed.

## 1. Starting it

```text
labcraft-cli mcp [--project DIR] [--lib-path DIR]... [--max-report N] [--max-file-size BYTES]
                 [--max-nesting N] [--allow-hardware ENDPOINT]... [--allow-path DIR]...
```

- **Transport:** stdio, JSON-RPC 2.0, one message per line, as MCP defines it. Nothing but MCP messages
  on stdout; stderr is for wrong usage at start, as for the other commands of cli.md. A line of whitespace
  only is read as nothing, and a line may end with CR LF. A byte order mark at the very start of a line is
  skipped before its JSON, as when `--args` reads a file (RFC 8259, 8.1, lets a reader ignore one);
  anywhere else outside a string, the line is not JSON (`-32700`), and so is a line of a byte order mark
  alone or followed by spaces only.
- **Protocol revisions:** 2025-11-25, and 2025-06-18, which clients still send *(proposed)*; the server
  answers `initialize` with the one the client asked for, or the latest. Not 2025-03-26, which requires
  accepting batches of JSON-RPC messages, which the later revisions removed.
- **The project** is `DIR`, or the current folder: the root against which every path is resolved, and
  the only place the server writes ([commands.md](commands.md), section 2). An agent started in a library
  folder with no option works on that folder. A root that is the user's home folder, a folder above it,
  or the root of a drive is refused at start (wrong usage, exit 2): an agent started from its home folder
  would otherwise reach every program under it. A folder inside it is named with `--project`.
- **The options** are `--lib-path` and the limits of cli.md, section 1, applied to every command of the
  session, and, from 0.5.0, `--allow-hardware` and `--allow-path`, the real endpoints and the folders outside the
  project that the runs of the agent may reach (section 6); a relative `--allow-path` is resolved against the current
  folder, as `--project` is, and one that does not read is wrong usage whether the server attaches or not. Not `--deny-warnings`: `library_check` and `run_start` take `deny_warnings` on each call.
- **Headless in M0b.** The session lives in the process. It checks the root as it starts and keeps the
  result, so that a tool that checks answers in the time of the libraries it touched
  ([commands.md](commands.md), section 6.2). From M1, when a window of the IDE shows the
  project, the same server attaches to it through a local control channel protected by a token, so that the human
  watches the agent work (D-18; [ide.md](ide.md) 8); otherwise it stays headless. The tools stay the same. Headless
  while windows show other projects, it says so after its instructions (`initialize`) and on stderr: which projects
  they show, at most 4 *(proposed)*, and that the user starts the client in that folder or gives `--project`, so that
  the agent asks for it rather than drive the window of LabCraft through its interface (decision of the owner,
  2026-11-22). It never attaches to another project by itself.
- **Attached** (M1). A window that shows a project listens on `127.0.0.1`, on a port the system gives, and writes a
  file that only the user can read (mode `0600` on Unix; on Windows, the inherited rights removed and the user alone
  given them): `<hash of the canonical folder of the project>.json` in `labcraft/attach/` of the local data of the user,
  or in the folder of `LABCRAFT_ATTACH_DIR` (empty: no channel, the server is always headless; the tests set it, so
  that they never find a window of the user). It holds the port, a token new at each launch, the process and the
  project, and goes when the window closes. `mcp` reads the file of its project and connects; each side then proves
  that it holds the token, which never goes on the connection (each sends a nonce, and the other the SHA-256 of its
  side, that nonce and the token). Then `mcp` relays stdin and stdout to the window, whose server runs the calls on the
  session of the window: `--lib-path` and the limits are then those of the window, and `--allow-hardware` and
  `--allow-path` are not used, since the window asks ([io.md](io.md) 4); the server says so on stderr (decision of the
  owner, 2026-11-24). A file whose port takes no
  connection is left over from a window that did not close, and the server starts headless. What takes the connection
  and refuses it, or does not prove it holds the token, stops the server (exit 2, the reason and the file on stderr), as
  does a window whose switch of AI is off ([ide.md](ide.md) 8): it would otherwise edit the project of a window from a
  session of its own, or give the agent what another process says. When the window closes the connection, `mcp` ends (exit 1).
  The same channel takes `labcraft FILE` for a project that the window shows ([cli.md](cli.md) 8): its first line
  names the file to show, from the root, the proofs are made the same way, and the connection ends there, with no MCP
  and no agent.
- **Ends** when stdin closes: a run still going is aborted (spec 8.2) and its resources are released. Attached, only a
  run that the agent started is aborted, not one of the user.

## 2. Tools

- **One tool per command** of [commands.md](commands.md), section 6, once it is no longer `planned`, in M0b; from M1, one
  tool per direct command, the others through `commands_execute` (below). The name of the tool is the id of
  the command with `_` for `.`: `run_start`, `file_write`. Dots would be clearer, but the names of tools
  given to a model through the Claude API, among others, allow only letters, digits, `_` and `-`.
- **Description and schema** come from the registry: the `summary` and `description` of the command are
  the description of the tool, its `params` the `inputSchema`. Nothing is written twice.
- **Annotations** follow `effects`, and all four are given, since MCP takes a missing `destructiveHint`
  or `openWorldHint` as true: a command with no effect is `readOnlyHint`; `run.start` is `openWorldHint`, since it
  reaches instruments (its effect `hardware`, io.md 4), and from 0.5.0 `destructiveHint`, since what a run writes with
  `file` is no step of the history; `commands_execute` is `openWorldHint` too, since its effects are those of the
  command it runs, `run.start` among them (decision of the owner, 2026-11-24); every other hint is false. In M0b, `file.write`, `file.rename`, `file.delete` and `file.format`
  were `destructiveHint`, since they could replace or remove a file; from M1 every command that writes a file makes a
  step of the history, which undo reverses, and `destructiveHint` is kept for a command whose effect the history cannot
  undo (decision of the owner, 2026-11-15; [architecture.md](architecture.md), 12.8).
- **Two ids, one name:** `a.b_c` and `a_b.c` would give one tool name, so the registry refuses such a
  pair ([commands.md](commands.md), section 1).
- **Nineteen tools in M0b**, `guide_read` and `diagram_svg` among them, few enough for each to be read. The `view.*` commands serve a session that has a viewer: a headless server does not give them
  (decision of the owner, 2026-11-11), and an agent sees a diagram through `diagram_svg`. Attached to the open IDE,
  an agent reads the state of the window (`view.get_state`) but never changes what the user sees: the other
  `view.*` commands are refused to it (decision of the owner, 2026-11-15, which replaces that of 2026-11-11 on
  this point; [ide.md](ide.md), 8).
- **Direct tools and the others** (decision of the owner, 2026-11-15). From M1 the catalog is too large for a tool per
  command: each tool costs every agent its description in its context. A command marked `direct` in the registry is a
  tool of its own; the others are reached through `commands_list`, `commands_describe` and `commands_execute`
  (commands.md 9.4), as SolidCraft and VectorCraft do. The direct tools are the nineteen of M0b, which the skills name,
  and `commands_execute`, `commands_batch`, `node_add`, `node_delete`, `constant_add`, `wire_connect`,
  `wire_disconnect`, `input_set_constant`, `value_rename`, `history_undo`, and `view_get_state` when attached
  *(proposed)*: about thirty ([architecture.md](architecture.md), 12.8). Twenty-seven in M1 step 0, twenty-nine from step 4
  (`input_set_constant` and `value_rename`); `view_get_state` comes with the server attached to the IDE. Headless,
  `commands_execute` does not reach a command of the window either (`command.not_enabled`).
- **The author of every call** is the client, by the `clientInfo.name` it gives at `initialize`: the history signs
  its steps with that name, and an agent undoes only its own ([commands.md](commands.md) 9.5).

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

**The skills** of [skills/](../skills/README.md) are what the agent learns the language from: syntax, types and
units, errors and the error wire, the order of side effects, `match`, loops, the panel, how to read a
diagnostic. A client that supports skills installs them; `guide_read` serves the same files from copies
built into the binary, as SolidCraft's `skill_reference` does, so that every MCP client reads them, and
they always match the version of the tool. Their code blocks are checked like those of the spec. They
hold no program of the acceptance scenarios.

**Progress.** A `run_start` or a `run_wait` that carries a `progressToken` gets a progress notification each
second of real time that it waits *(proposed)*: `progress` counts them from 1, and `message` gives the clock
and where each loop is, its iteration counted from 1 as the command line shows it (cli.md 6.7). A
cancellation (`notifications/cancelled`) of a `run_start` or a `run_wait` stops the wait, not the run, which
`run_abort` stops. A cancelled request gets no answer, whether it waited its turn or ran; one that waited is
dropped unrun. Attached to the IDE, an editing command that waits for the end of a gesture of the user (architecture
12.5) gets the same notification each second, its `message` `waiting for the user`, and a cancellation drops it
unrun.

## 5. Bounds

The server reads hostile requests as `check` reads hostile files (scenario 4, family MCP):

- **A request** larger than 16 MiB *(proposed)*, as in SolidCraft, and above what the 4 MiB of a
  `file_write` take once quoted in JSON ([commands.md](commands.md), section 6.2), is refused before it is
  parsed, so its `id` is unknown: the answer is the JSON-RPC error `-32600` with `id: null`, and the server
  drops what follows, without keeping it, up to the next line feed, then reads on.
- **Nesting** of a JSON request is bounded at `--max-nesting` plus 16 levels *(proposed)*, so that an
  argument of any type that `check` accepts can be sent. A deeper request is refused as a larger one is,
  `-32600` with `id: null`, since its `id` is not read. A request is read without recursion, and a key given
  twice is refused at any depth, as in `--args` (cli.md 6.3): `-32700`, as for a line that is not JSON or not
  UTF-8. A batch (a JSON array) is `-32600`. The values of a
  report are cut at 64 levels ([commands.md](commands.md), section 5).
- **Requests run in order**, except that `ping`, `run_status`, `run_set_element` and `run_abort` answer
  while a `run_start` or a `run_wait` is waiting: an agent can always stop what it started. These never
  wait in the queue. At most 16 other requests wait their turn *(proposed)*; one more gets the JSON-RPC
  error `-32000` ("server busy") at once. Those that wait hold at most the bytes of one request in all,
  16 MiB: a request of many small values takes far more memory read than as text, so a request that waits
  is kept as its text, and read again when its turn comes; one that would pass these bytes is "server busy"
  too.
- **A panic** in a command is caught by the session, or by the runs for the tools answered at once, and
  returned as `internal`; the server goes on.
- **A report** is bounded by `--max-report` (cli.md, section 1) and by the rules of commands.md, sections 5
  and 7.

## 6. What an agent may touch

- **Files:** the server writes only `.lcf`, `.lct` and simulation files under the root of the project, and
  its traces under `.labcraft/` ([commands.md](commands.md), section 2). It never writes a shipped library. From M1
  it also writes, through the commands of commands.md 9, the manifest `labcraft.toml` of a library of the project
  (`library.set_mask`), the settings of the project (`.labcraft/config.toml`) and of the user (`settings.set`, which
  ide.md 8 bounds), and the journal of its commands (`.labcraft/journal/`, [architecture.md](architecture.md) 12.5).
- **Instruments and devices:** in M0b only the `SIM::` resources of a simulation file exist, since `visa` has no real
  transport before M4. A real instrument can switch on a supply, so from 0.5.0 a run of the agent reaches a real
  endpoint (`tcp:10.0.0.5:5025`, `serial:COM3`) only if the human allowed it when starting the server, by
  `--allow-hardware ENDPOINT`, and a file outside the project only under an `--allow-path DIR`
  ([io.md](io.md) 4 and 5); no tool can grant it. Without the right, the run goes on and the open gets the error
  `io.not_allowed`, which names the option. Attached to the IDE, the window asks instead (ide.md 8).

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

## 9. Connecting the MCP clients: `mcp setup`

```text
labcraft-cli mcp setup [--dry-run] [--client NAME]...
```

`Connect an AI agent`, in the IDE ([ide.md](ide.md) 8), does the same. Both write the server `labcraft`, whose
command is the full path of the `labcraft-cli` that writes it (for the IDE, the one installed beside it) and whose
argument is `mcp`, in the configuration of the user of each MCP client found on the computer (decisions of the owner,
2026-11-22, after SolidCraft's `mcp setup`):

| `--client` | Client | File, and the object that lists its servers |
|---|---|---|
| `claude-code` | Claude Code | `~/.claude.json`, `mcpServers` |
| `codex` | Codex (its command line, its extension and its app) | `config.toml` of `$CODEX_HOME`, else of `~/.codex`, `[mcp_servers.labcraft]` |
| `copilot` | GitHub Copilot CLI | `~/.copilot/mcp-config.json`, `mcpServers` |
| `cursor` | Cursor | `~/.cursor/mcp.json`, `mcpServers` |
| `gemini` | Gemini CLI | `~/.gemini/settings.json`, `mcpServers` |
| `opencode` | OpenCode | `opencode.json`, or `opencode.jsonc` when only it is there, of `$XDG_CONFIG_HOME/opencode`, else of `~/.config/opencode`, `mcp` |
| `vscode` | VS Code | `mcp.json` of its folder of the user: `%APPDATA%\Code\User`, `~/Library/Application Support/Code/User`, `~/.config/Code/User`; `servers` |
| `windsurf` | Windsurf | `~/.codeium/windsurf/mcp_config.json`, `mcpServers` |
| `zed` | Zed | `settings.json` of `%APPDATA%\Zed`, else of `~/.config/zed`, `context_servers` |

- **Found**: a client is found when its folder of configuration is there (for Claude Code, `~/.claude.json` or
  `~/.claude`). No program of a client is started. Claude Desktop is not among them: it starts a server in no folder of
  a project, and `mcp` would serve none (section 1).
- **The project**: the entry has no `--project`. `mcp` serves the folder the client starts it in, that of its session
  or of its workspace (section 1), and attaches to the window that shows it: the user opens the same folder in the
  client and in LabCraft. For a client that starts it elsewhere (an app may start it in a folder of its own), the user
  adds `--project` by hand, and setup then leaves that entry as it is.
- **Only the entry changes.** The rest of the file (other servers, other settings, comments, trailing commas, order,
  line endings, a byte order mark) stays as it was, byte for byte: in a TOML file, the new table goes after what the
  file holds. A file that is a link is written where it leads. A file is replaced at once, its version before kept beside it as
  `NAME.bak`, and only when it did not change since it was read: a client shares no lock with LabCraft, so this narrows
  the race with its own writes without excluding one. A file that does not read as JSON (comments allowed) or as TOML,
  or that holds over 64 MiB *(proposed)*, is left alone, and said.
- **An entry already there**: one that runs this `labcraft-cli` with `mcp` stays. One that runs a `labcraft-cli` from
  another folder with `mcp`, as after an update moved it, gets this path, its other fields kept. Any other (another
  program, other arguments, a `labcraft-cli` without a folder, found on the `PATH`) is the user's, and stays as
  written.
- **What it says**: a line for each client found (added, set up already, path updated, left as written, or nothing
  written and why), and for each client named with `--client` that is not found; then, when it wrote, that those
  clients must be quit and started again, since one left open may write its file again without the entry; with none
  found, which it looked for.
  `--dry-run` says what it would do and writes nothing. Exit codes: 0; 1 when a client could not be set up (its file
  does not read, changed meanwhile, or could not be written); 2 for wrong usage, as in [cli.md](cli.md), section 1.
- **By hand**, for a client not listed: a server on stdio whose command is the full path of `labcraft-cli` and whose
  argument is `mcp`, started in the folder of the project (or given `--project DIR`), as
  `claude mcp add labcraft -- PATH/labcraft-cli mcp`. Taking the entry out is done by hand too, or by the client
  (`claude mcp remove labcraft`).

## 10. Left for later

Taken from SolidCraft, each when what it serves exists:

- **A `running` tool**, taken from SolidCraft with the channel attached to the IDE (section 1): what the app runs for
  every client. The channel came in M1 without it.
- **`batch`** (M1): several editing commands in one call, stopped at the first failure and rolled back. It is the
  command `commands.batch` ([commands.md](commands.md), 9.4), and so the tool `commands_batch`.
- **`crash_reports`**: the crash reports of this computer, each with the commands that led to the bug,
  so that it replays.
- **Resources**: the catalog and the project as MCP resources, for the clients that read them.
