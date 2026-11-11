# Skills for agents

What an agent reads to write, debug and test LabCraft programs. Each folder is one skill in the
[Agent Skills](https://agentskills.io) format: a `SKILL.md` whose `name` and `description` tell the agent when to
load it, and the files it points to.

The skills are built into LabCraft: the tool `guide_read` of its MCP server ([docs/mcp.md](../docs/mcp.md))
serves the same files, so every MCP client reads them, and they always match the version of the tool. Without a
skill, `guide_read` lists the skills and gives the `SKILL.md` of `labcraft-writing`, so one call starts an agent.

| Skill | Loads when the agent |
|---|---|
| [`labcraft-writing`](labcraft-writing/SKILL.md) | writes or changes a program; `reference.md` holds the whole language |
| [`labcraft-debugging`](labcraft-debugging/SKILL.md) | reads a diagnostic, or a run that ended otherwise than expected |
| [`labcraft-testing`](labcraft-testing/SKILL.md) | runs a program on simulated benches, writes a simulation file, sets the panel during a run |

## Install

A client that reads skills can install them; one that does not reads them through `guide_read`. Take the skills of
the version of LabCraft you run, its tag (`TAG` below), so that they say what your binary does. For Claude Code, in
every project:

```sh
git clone --depth 1 --branch TAG https://github.com/corail-labs/labcraft
mkdir -p ~/.claude/skills
cp -r labcraft/skills/labcraft-* ~/.claude/skills/
```

Windows (PowerShell):

```powershell
git clone --depth 1 --branch TAG https://github.com/corail-labs/labcraft
New-Item -ItemType Directory -Force $HOME\.claude\skills | Out-Null
Copy-Item -Recurse labcraft\skills\labcraft-* $HOME\.claude\skills\
```

For one project only, copy the folders into its `.claude/skills/` instead. Other agents that read the Agent
Skills format take the folders in the directory their documentation names.

## Writing a skill

- **English**, as the rest of the repository, and written for an agent that has the MCP tools and nothing else:
  no path of this repository, no file of `examples/` or `docs/` to read.
- **No program of the acceptance scenarios**, nor one that gives their answer: no example measures a value,
  compares it to limits and returns a verdict ([docs/acceptance-m0.md](../docs/acceptance-m0.md), scenario 1).
  Instruments are shown on a supply or by `*OPC?`. A test of `tests/harness` refuses a whole file that declares a
  function or a type of `examples/`, where those programs are, and a skill that names them.
- **Every program and simulation file is checked.** In `cargo xtask spec`, each `text` block parses and is in
  canonical form, as those of the spec are (a block that contains `…` is a fragment and is skipped). In
  `cargo xtask ci`, the blocks of a skill that are whole files (they start with `labcraft 1`) form one library, which
  `check` passes with no error and no warning, and each `toml` block is a simulation file that `visa` accepts. A
  block that shows a mistake on purpose is a fragment, not a whole file. A `json` block shows a report: copy it from
  a real report of a program of the skill, not from memory.
- **A new file** of a skill folder is built in with nothing to declare; `guide_read` serves it by its path in the
  folder (`reference.md`).
