#!/usr/bin/env bash
# The agent half of scenario 2 (docs/acceptance-m0.md), run by hand before the M0a tag.
#
# For each case of tests/acceptance/type-errors/, a fresh Claude Code session gets a copy of the case outside the
# repository, without expected.json, and the human-readable diagnostic of `labcraft-cli check`. It has the file tools
# only: no shell and no MCP server, so it cannot run the checker, and its first fix is the one judged. A case is fixed
# when `check --deny-warnings` then reports nothing.
#
#   bash tests/acceptance/agent/run.sh OUT_DIR          all the cases
#   ONLY=implicit-narrowing bash tests/acceptance/agent/run.sh OUT_DIR
#
# OUT_DIR must not exist yet, and must be outside the repository. Each case leaves there: the prompt, the transcript
# (stream-json), the diff of the fix and the report of `check` after it; `versions.txt` says what ran. MODEL picks the
# model (default claude-opus-5-5).
set -u
REPO=$(cd "$(dirname "$0")/../../.." && pwd -P)
OUT=${1:?usage: run.sh OUT_DIR}
MODEL=${MODEL:-claude-opus-5-5}
lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }

[ -e "$OUT" ] && { echo "$OUT exists: give a new folder" >&2; exit 2; }
parent=$(cd "$(dirname "$OUT")" 2>/dev/null && pwd -P) || { echo "the parent of $OUT does not exist" >&2; exit 2; }
OUT=$parent/$(basename "$OUT")
# Paths are compared without case, since Windows and macOS ignore it.
case "$(lower "$OUT")/" in "$(lower "$REPO")"/*) echo "OUT_DIR must be outside the repository" >&2; exit 2 ;; esac

cargo build --release --quiet -p labcraft-cli --manifest-path "$REPO/Cargo.toml" || exit 2
CLI=$REPO/target/release/labcraft-cli
[ -x "$CLI" ] || CLI=$CLI.exe
command -v claude > /dev/null || { echo "claude is not on the PATH" >&2; exit 2; }
if [ -n "${ONLY:-}" ] && [ ! -d "$REPO/tests/acceptance/type-errors/$ONLY" ]; then echo "no case $ONLY" >&2; exit 2; fi

mkdir "$OUT" || exit 2
{
  echo "labcraft-cli: $("$CLI" version) at $(git -C "$REPO" rev-parse HEAD)$(git -C "$REPO" diff --quiet HEAD || echo ' with changes')"
  echo "claude: $(claude --version)"
  echo "model: $MODEL"
} > "$OUT/versions.txt"
for src in "$REPO"/tests/acceptance/type-errors/*/; do
  name=$(basename "$src")
  [ -n "${ONLY:-}" ] && [ "$name" != "$ONLY" ] && continue
  dir=$OUT/$name
  mkdir -p "$dir/lib" "$dir/before"
  for f in "$src"*.lcf "$src"*.lct; do [ -e "$f" ] && cp "$f" "$dir/lib/" && cp "$f" "$dir/before/"; done
  diag=$(cd "$dir/lib" && "$CLI" check . 2>&1)
  prompt="This folder is a LabCraft library (.lcf files are functions, .lct files are types). \`labcraft-cli check .\` reports this diagnostic:

$diag

Fix the program so that it checks cleanly. You cannot run the checker: make your fix in one go."
  printf '%s\n' "$prompt" > "$dir/prompt.txt"
  (cd "$dir/lib" && claude -p --model "$MODEL" --restricted --strict-mcp-config --disable-slash-commands \
     --tools "Read,Edit,Write,Glob,Grep" --allowedTools "Read Edit Write Glob Grep" --permission-mode acceptEdits \
     --no-session-persistence --output-format stream-json --verbose "$prompt" > "$dir/transcript.jsonl" 2> "$dir/stderr.txt")
  echo "claude exit $?" > "$dir/agent-exit.txt"
  (cd "$dir/lib" && "$CLI" check --deny-warnings . > "$dir/after.txt" 2>&1; echo "check exit $?" >> "$dir/after.txt")
  (cd "$dir" && diff -ruN before lib > fix.diff)
  echo "$name: $(tail -1 "$dir/after.txt")"
done
