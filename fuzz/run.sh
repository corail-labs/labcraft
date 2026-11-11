#!/usr/bin/env bash
# Runs a target for a number of seconds: what the job `fuzz` does, and what `README.md` says to do by hand.
# Fails on a crash, and also when libFuzzer stopped early: it calls that a success when its clock jumps, and a run that did
# not fuzz must not be green. Time is measured on /proc/uptime, which setting the clock does not move; it still counts a
# suspend, and without it (macOS) the check uses the clock of libFuzzer and cannot see a jump.
#
# usage: run.sh TARGET SECONDS        fuzz; exit 3 when libFuzzer stopped early
#        run.sh TARGET --replay FILE  run one input again, alone, with the same limits
set -euo pipefail

usage="usage: run.sh TARGET SECONDS | run.sh TARGET --replay FILE"
target=${1:?$usage}
second=${2:?$usage}
# Seconds for one input. A function whose budget of work runs out (`Checked::cut_short`) takes about 9 s under the sanitizer,
# and an input of 64 KiB holds up to three of them; `diagram` checks its input first.
timeout=10
if [ "$target" = check ] || [ "$target" = diagram ] || [ "$target" = run ] || [ "$target" = mcp ]; then timeout=60; fi

if [ "$second" = --replay ]; then
    file=$(realpath "${3:?$usage}")
    cd "$(dirname "$0")"
    exec cargo fuzz run "$target" "$file" -- -max_len=65536 -timeout="$timeout" -runs=1
fi
seconds=$second
cd "$(dirname "$0")"

now() {
    if [ -r /proc/uptime ]; then
        local up
        up=$(cut -d' ' -f1 /proc/uptime)
        echo "${up%.*}"
    else
        date +%s
    fi
}

# One folder of seeds per target, so that targets can run side by side.
seeds=target/seeds/$target
log=target/$target.log
cargo run --quiet --locked --bin deep_seeds -- "$seeds"
mkdir -p "corpus/$target"
# Built before the clock starts, so that the time of the build does not hide a run that stopped early.
cargo fuzz build "$target"

start=$(now)
# New inputs go to the first folder; the others are only read, and libFuzzer walks them recursively.
cargo fuzz run "$target" "corpus/$target" ../examples ../libs ../docs/rfc ../tests/acceptance "$seeds" -- \
    -max_total_time="$seconds" -max_len=65536 -timeout="$timeout" -print_final_stats=1 2>&1 | tee "$log"
elapsed=$(($(now) - start))

if [ "$elapsed" -lt $((seconds * 9 / 10)) ]; then
    runs=$(sed -n 's/^stat::number_of_executed_units: *//p' "$log")
    echo "error: libFuzzer stopped after $elapsed s of $seconds, with ${runs:-0} executions: its clock jumped, so this did not really fuzz" >&2
    exit 3
fi
