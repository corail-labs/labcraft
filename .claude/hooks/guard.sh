# Runs guard.py with the first Python 3 on the path; `.claude/settings.json` calls it before each tool call.
# Claude Code runs hooks with a POSIX shell: Git Bash on Windows, where `python3` is often the stub of the Microsoft
# Store, which the probe skips. Without Python 3, or without guard.py, it exits 1: Claude Code shows the message and
# lets the call through (fail open, the owner's choice). Exit 2 is guard.py's alone: python itself would exit 2 on a
# missing script. On Windows without Git Bash, Claude Code runs the command in PowerShell, where it fails with exit 1:
# a non-blocking error, so the calls go on unjudged. The settings set no `shell` field on purpose.

case "$0" in
*[\\/]*) hook="${0%[\\/]*}/guard.py" ;;
*) hook="guard.py" ;;
esac
if [ ! -f "$hook" ]; then
    echo "guard.sh: $hook not found; the call goes through unjudged" >&2
    exit 1
fi
for python in python3 python; do
    if command -v "$python" >/dev/null 2>&1 &&
        "$python" -c 'import sys; sys.exit(sys.version_info < (3, 8))' </dev/null >/dev/null 2>&1; then
        exec "$python" "$hook"
    fi
done
echo "guard.sh: no Python 3 on the path; the call goes through unjudged" >&2
exit 1
