#!/usr/bin/env bash
# Runs `run.sh` for several targets at once, each for the same number of seconds: what the job `fuzz` does, one target per
# vCPU. Each output goes to target/<target>.out and is printed when every target has ended, or when the job stops this
# script, one after the other, so that they do not mix. Fails when one of them failed, and names it.
#
# usage: side_by_side.sh SECONDS TARGET...
set -uo pipefail

usage="usage: side_by_side.sh SECONDS TARGET..."
seconds=${1:?$usage}
shift
[ $# -gt 0 ] || { echo "$usage" >&2; exit 2; }
cd "$(dirname "$0")"
mkdir -p target

targets=("$@")
pids=()

outputs() {
    for target in "${targets[@]}"; do
        # A group folds in the log of GitHub Actions, and is two plain lines elsewhere.
        echo "::group::$target"
        cat "target/$target.out" 2>/dev/null
        echo "::endgroup::"
    done
}
# A timeout or a cancelled job stops this script with a signal: the outputs so far still say where each target was.
trap outputs EXIT
trap 'kill "${pids[@]}" 2>/dev/null; exit 130' INT TERM

for target in "${targets[@]}"; do
    bash run.sh "$target" "$seconds" > "target/$target.out" 2>&1 &
    pids+=($!)
done

failed=()
for index in "${!pids[@]}"; do
    wait "${pids[$index]}" || failed+=("${targets[$index]}")
done

trap - EXIT INT TERM
outputs
if [ ${#failed[@]} -gt 0 ]; then
    echo "error: failed: ${failed[*]}" >&2
    exit 1
fi
