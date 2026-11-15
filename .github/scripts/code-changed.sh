#!/usr/bin/env bash
# Says whether a pull request changes more than prose, for the jobs that build or run code (workflows `checks`
# and `fuzz`). Writes `code=true` or `code=false` to $GITHUB_OUTPUT.
#
# A changed file is prose when it is a Markdown file at the root, directly in docs/, in .claude/agents/ or
# anywhere under .claude/skills/ (instructions for agents, which no code reads; the `*` of the pattern crosses `/`,
# so any depth counts), or a file of docs/images/ or design/ (FigCraft files, which no code reads), and, for
# `checks`, no Rust string names it: the tests read docs/language-spec-v0.md, docs/cli.md and
# docs/visual-notation.md, so those count as code. An instruction file of .claude/ is named by its whole path, or
# by a string literal that starts with `.claude` (a path built piece by piece), not by its bare name: `skills/`
# gives its own SKILL.md, which the engine names. `.claude/README.md` and `.claude/skills/README.md` stay code,
# since neither sits in a subfolder. `fuzz` skips that test, since the fuzz targets read no document of docs/
# outside docs/rfc/ (a seed, so code): a Markdown file there is prose for it, whatever a test reads. Every other
# file is code: docs/rfc/ and docs/releases/, skills/, examples/, the workflows.
# Anything but a pull request runs everything.
#
# The files are those of the merge commit that the job checked out, against its first parent (checkout with
# fetch-depth 2): the commit under test, whatever its size. A file renamed counts under both names. CHANGED_FILES,
# one path per line, replaces that list, to try the script by hand.
#
# Usage: code-changed.sh checks|fuzz, with EVENT set by the workflow.
set -euo pipefail
workflow=${1:?usage: code-changed.sh checks|fuzz}
case "$workflow" in checks | fuzz) ;; *) echo "unknown workflow: $workflow" >&2; exit 2 ;; esac

say() {
  echo "code=$1"
  echo "code=$1" >> "$GITHUB_OUTPUT"
}

if [ "${EVENT:-}" != pull_request ]; then
  echo "Not a pull request: everything runs."
  say true
  exit 0
fi

if [ -n "${CHANGED_FILES:-}" ]; then
  files=$CHANGED_FILES
else
  files=$(git diff --name-only --no-renames HEAD^1 HEAD)
fi
if [ -z "$files" ]; then
  echo "No changed file listed: everything runs."
  say true
  exit 0
fi

code=false
while IFS= read -r file; do
  case "$file" in
    design/*) echo "prose: $file"; continue ;;
    # Instructions for agents: prose, unless a Rust string names the file by its path, or starts a path under
    # `.claude/` (a bare `SKILL.md` belongs to `skills/`, which is code).
    .claude/agents/*.md | .claude/skills/*/*.md)
      if [ "$workflow" = fuzz ]; then
        echo "prose: $file"
        continue
      fi
      status=0
      grep -rqF --include='*.rs' -e "\"$file\"" -e "/$file\"" -e '".claude' crates apps fuzz tests || status=$?
      if [ "$status" -eq 1 ]; then
        echo "prose: $file"
      else
        echo "code (a Rust string names it, or grep failed): $file"
        code=true
      fi
      continue ;;
    */*.md) [[ "$file" == docs/*.md && "$file" != docs/*/* ]] || { echo "code: $file"; code=true; continue; } ;;
    *.md) ;;
    docs/images/*) ;;
    *) echo "code: $file"; code=true; continue ;;
  esac
  # A Rust string names a file of docs/ when it ends with its name, whatever comes before: "../../docs/cli.md",
  # join("docs").join("cli.md") and format!("{}/docs/{}", root, "cli.md") all count, and a document cited in a
  # comment, in backquotes, does not. A root file must be reached from above ("../README.md") or joined to the root:
  # a bare "README.md" is a name, not the file. Fixed strings, never a pattern. xtask is left out: what it reads
  # (docs/cli.md, docs/commands.md), the job lint checks, and lint always runs.
  # The fuzz targets read none of these documents: prose, whatever a test of `checks` reads.
  if [ "$workflow" = fuzz ]; then
    echo "prose: $file"
    continue
  fi
  name=${file##*/}
  if [[ "$file" == */* ]]; then
    strings=(-e "\"$name\"" -e "/$name\"")
  else
    strings=(-e "../$name\"" -e "join(\"$name\")")
  fi
  status=0
  grep -rqF --include='*.rs' "${strings[@]}" crates apps fuzz tests || status=$?
  if [ "$status" -eq 1 ]; then
    echo "prose: $file"
  else
    echo "code (a Rust string names it, or grep failed): $file"
    code=true
  fi
done <<< "$files"
say "$code"
