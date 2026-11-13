#!/usr/bin/env bash
# Scenario 1 (docs/acceptance-m0.md), run by hand before the M0b tag: an agent writes the PASS/FAIL program.
#
# Each session is a fresh Claude Code session in an empty folder outside the repository, connected to
# `labcraft-cli mcp` on that folder, with the skills of skills/ copied into its .claude/skills (skills/README.md), and
# given the prompt once, verbatim. Its tools are those of the MCP server, Skill, and Read, which is not allowed beforehand:
# Claude Code then lets it read the folder of the session only, and refuses the rest without asking (-p), the repository
# included, where examples/ holds the answer. The checks are made afterwards, by the CLI and not by what the agent said:
# scenario-1.py, which also fails a session that read anything outside its folder or used a skill not of LabCraft.
#
#   bash tests/acceptance/agent/scenario-1.sh OUT_DIR            five sessions, then scenario-1.py
#   SESSIONS=1 bash tests/acceptance/agent/scenario-1.sh OUT_DIR one, to try the script
#
# OUT_DIR must not exist yet, and must be outside the repository. Each session leaves there: its folder session-N/ (the
# files the agent wrote), the transcript session-N.jsonl (stream-json), its MCP configuration, stderr and exit code;
# `versions.txt` says what ran, `summary.md` and `summary.json` what scenario-1.py found. MODEL picks the model (default
# claude-opus-5-5). Needs Claude Code and Python 3 on the PATH.
set -u
REPO=$(cd "$(dirname "$0")/../../.." && pwd -P)
OUT=${1:?usage: scenario-1.sh OUT_DIR}
MODEL=${MODEL:-claude-opus-5-5}
SESSIONS=${SESSIONS:-5}
PROMPT='Read the voltage of a multimeter over SCPI, check that it is between 220 and 240 V, return PASS or FAIL.'
lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }
# The form of a path that a Windows program reads (C:/...) under Git Bash; the path itself elsewhere.
native() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

[ -e "$OUT" ] && { echo "$OUT exists: give a new folder" >&2; exit 2; }
parent=$(cd "$(dirname "$OUT")" 2>/dev/null && pwd -P) || { echo "the parent of $OUT does not exist" >&2; exit 2; }
OUT=$parent/$(basename "$OUT")
# Paths are compared without case, since Windows and macOS ignore it.
case "$(lower "$OUT")/" in "$(lower "$REPO")"/*) echo "OUT_DIR must be outside the repository" >&2; exit 2 ;; esac
case "$SESSIONS" in '' | *[!0-9]*) echo "SESSIONS must be a number" >&2; exit 2 ;; esac

cargo build --release --quiet -p labcraft-cli --manifest-path "$REPO/Cargo.toml" || exit 2
CLI=$REPO/target/release/labcraft-cli
[ -x "$CLI" ] || CLI=$CLI.exe
command -v claude > /dev/null || { echo "claude is not on the PATH" >&2; exit 2; }
# The first that runs: under Windows, python3 may be a stub that only opens the Store.
PYTHON=
for candidate in python3 python; do "$candidate" -c '' 2> /dev/null && { PYTHON=$candidate; break; }; done
[ -n "$PYTHON" ] || { echo "python is not on the PATH" >&2; exit 2; }

mkdir "$OUT" || exit 2
{
  echo "labcraft-cli: $("$CLI" version) at $(git -C "$REPO" rev-parse HEAD)$(git -C "$REPO" diff --quiet HEAD || echo ' with changes')"
  echo "claude: $(claude --version)"
  echo "model: $MODEL"
} > "$OUT/versions.txt"
for i in $(seq 1 "$SESSIONS"); do
  dir=$OUT/session-$i
  mkdir -p "$dir/.claude/skills"
  cp -r "$REPO"/skills/labcraft-* "$dir/.claude/skills/"
  config=$OUT/session-$i.mcp.json
  printf '{"mcpServers":{"labcraft":{"command":"%s","args":["mcp","--project","%s"]}}}\n' \
    "$(native "$CLI")" "$(native "$dir")" > "$config"
  started=$(date -u +%FT%TZ)
  # Project settings only: the skills of the folder; no shell, and no editing tool but those of the server.
  (cd "$dir" && claude -p --model "$MODEL" --setting-sources project --strict-mcp-config --mcp-config "$(native "$config")" \
     --tools "Read,Skill" --allowedTools "mcp__labcraft" "Skill" \
     --no-session-persistence --output-format stream-json --verbose "$PROMPT" > "$OUT/session-$i.jsonl" 2> "$OUT/session-$i.stderr.txt")
  echo "claude exit $? from $started to $(date -u +%FT%TZ)" > "$OUT/session-$i.exit.txt"
  echo "session $i: $(cat "$OUT/session-$i.exit.txt")"
done
"$PYTHON" "$REPO/tests/acceptance/agent/scenario-1.py" "$OUT" "$CLI"
