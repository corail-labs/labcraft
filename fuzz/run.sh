#!/usr/bin/env bash
# Runs a target for a number of seconds: what the job `fuzz` does, and what `README.md` says to do by hand.
# Fails on a crash, and also when too little was executed: libFuzzer stops early and calls it a success when
# its clock jumps backward, and a job that did not fuzz must not be green.
#
# usage: run.sh TARGET SECONDS
set -euo pipefail

target=${1:?usage: run.sh TARGET SECONDS}
seconds=${2:?usage: run.sh TARGET SECONDS}
# Far below what was measured, with the deep seeds: about 2,000 a second on 4 cores for `read`, about 1,400 a second
# per process in a container for `round_trip`, about 200 for `check`, which checks each input three times.
case "$target" in
    check) least=$((seconds * 30)) ;;
    *) least=$((seconds * 100)) ;;
esac
# Seconds for one input. A function whose budget of work runs out (`Checked::cut_short`) takes about 9 s under the sanitizer,
# and an input of 64 KiB holds up to three of them.
timeout=10
if [ "$target" = check ]; then timeout=60; fi

cd "$(dirname "$0")"
seeds=target/seeds
log=target/$target.log
cargo run --quiet --locked --bin deep_seeds -- "$seeds"
mkdir -p "corpus/$target"

# New inputs go to the first folder; the others are only read, and libFuzzer walks them recursively.
cargo fuzz run "$target" "corpus/$target" ../examples ../libs ../docs/rfc ../tests/acceptance "$seeds" -- \
    -max_total_time="$seconds" -max_len=65536 -timeout="$timeout" -print_final_stats=1 2>&1 | tee "$log"

runs=$(sed -n 's/^stat::number_of_executed_units: *//p' "$log")
if [ "${runs:-0}" -lt "$least" ]; then
    echo "error: ${runs:-0} executions in ${seconds} s, at least $least expected: this did not really fuzz" >&2
    exit 1
fi
