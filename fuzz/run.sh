#!/usr/bin/env bash
# Runs a target for a number of seconds: what the job `fuzz` does, and what `README.md` says to do by hand.
# Fails on a crash, and also when libFuzzer stopped early: it calls that a success when its clock jumps, and a run that did
# not fuzz must not be green. Time is measured on /proc/uptime, which setting the clock does not move; it still counts a
# suspend, and without it (macOS) the check uses the clock of libFuzzer and cannot see a jump.
#
# FUZZ_FORK=N runs N workers at once (the fork mode of libFuzzer), for the long run before a tag on a machine with several
# cores. That mode first reads the seeds in one process, within SECONDS, and goes on past a timeout: run.sh then prints the
# worker-hours of the run, that reading counted as one worker, as long.sh counts each part, which reads the corpus again
# within its hour; and it runs again alone each timeout written meanwhile, and fails if one times out
# alone too. A crash, an out-of-memory or a leak still stops the run; one on a seed shows only as the file libFuzzer wrote,
# so any other file written meanwhile, but a slow unit, fails the run too.
#
# It also fails when libFuzzer spent its time reading the seeds: it fuzzed less than a tenth of SECONDS.
#
# usage: run.sh TARGET SECONDS        fuzz; exit 3 when libFuzzer stopped early, 4 when it hardly fuzzed
#        run.sh TARGET --replay FILE  run one input again, alone, with the same limits
set -euo pipefail

usage="usage: run.sh TARGET SECONDS | run.sh TARGET --replay FILE"
target=${1:?$usage}
second=${2:?$usage}
# Seconds for one input. A function whose budget of work runs out (`Checked::cut_short`) takes about 9 s under the sanitizer,
# and an input of 64 KiB holds up to three of them; `diagram` checks its input first. `edit` checks the whole project
# after each of its hundreds of steps: an input of 5 KB takes 2.4 s under the sanitizer on a quiet machine, up to 10
# times more while the other targets run beside it.
timeout=10
case "$target" in check | diagram | edit | run | mcp) timeout=60 ;; esac

if [ "$second" = --replay ]; then
    file=$(realpath "${3:?$usage}")
    cd "$(dirname "$0")"
    exec cargo fuzz run "$target" "$file" -- -max_len=65536 -timeout="$timeout" -runs=1
fi
seconds=$second
workers=${FUZZ_FORK:-1}
case "$workers" in
    '' | *[!0-9]* | 0* | ?????*) echo "FUZZ_FORK is a whole number, from 1 to 9999" >&2; exit 1 ;;
esac
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
mkdir -p "corpus/$target" "artifacts/$target"
# New inputs go to the first folder; the others are only read, and libFuzzer walks them recursively. `edit` reads its
# input as steps, a byte at a time, on a project built into the target, and each input of the repository takes about
# 0.6 s: all of them took more than its 60 s in the job `fuzz`. It reads only `examples/`, for long inputs from the
# start; with none, libFuzzer grows its inputs slowly, and 60 s stayed at 4 bytes.
folders=("corpus/$target" ../examples)
if [ "$target" != edit ]; then folders+=(../libs ../docs/rfc ../tests/acceptance "$seeds"); fi
# Built before the clock starts, so that the time of the build does not hide a run that stopped early.
cargo fuzz build "$target"

fork=()
if [ "$workers" -gt 1 ]; then
    fork=(-fork="$workers" -ignore_crashes=0 -ignore_ooms=0 -ignore_timeouts=1)
fi
# When fuzzing starts: libFuzzer writes this line once it has read the seeds, `INITED` with one worker and
# `starting to fuzz` with several.
fuzzing=target/$target.fuzzing
rm -f "$fuzzing"
started="INITED"
if [ "$workers" -gt 1 ]; then started="starting to fuzz"; fi
mark() {
    local line
    while IFS= read -r line || [ -n "$line" ]; do
        printf '%s\n' "$line"
        case $line in *"$started"*) [ -e "$fuzzing" ] || now > "$fuzzing" ;; esac
    done
}

# What libFuzzer writes from now on is newer than this file.
touch "target/$target.started"
start=$(now)
cargo fuzz run "$target" "${folders[@]}" -- \
    ${fork[@]+"${fork[@]}"} -max_total_time="$seconds" -max_len=65536 -timeout="$timeout" -print_final_stats=1 2>&1 |
    mark | tee "$log"
end=$(now)
elapsed=$((end - start))

if [ "$elapsed" -lt $((seconds * 9 / 10)) ]; then
    runs=$(sed -n 's/^stat::number_of_executed_units: *//p' "$log")
    echo "error: libFuzzer stopped after $elapsed s of $seconds, with ${runs:-0} executions: its clock jumped, so this did not really fuzz" >&2
    exit 3
fi

# libFuzzer does not cut the reading of the seeds short: when it takes the whole budget, the run ends without fuzzing, as a
# success. Running it again would read the same seeds: exit 4, not 3.
if [ ! -r "$fuzzing" ]; then
    echo "error: libFuzzer never started to fuzz in $elapsed s" >&2
    exit 4
fi
fuzzed=$((end - $(cat "$fuzzing")))
if [ "$fuzzed" -lt $((seconds / 10)) ]; then
    echo "error: libFuzzer read the seeds for $((elapsed - fuzzed)) s and fuzzed $fuzzed s of $seconds: it did not really fuzz" >&2
    exit 4
fi

if [ "$workers" -gt 1 ]; then
    reading=$((elapsed - fuzzed))
    echo "$workers workers fuzzed $fuzzed s each, after $reading s reading the seeds on one:" \
        "$(awk -v w="$workers" -v s="$fuzzed" -v r="$reading" 'BEGIN { printf "%.1f", (r + w * s) / 3600 }') worker-hours" |
        tee -a "$log"
    # What libFuzzer wrote during the run. Fork mode reads the seeds through a merge that drops a seed which crashes, and
    # exits 0: such a crash shows only as a file here. A slow unit is not a failure.
    failed=0
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        name=$(basename "$file")
        case $name in
            slow-unit-*) ;;
            timeout-*)
                if bash run.sh "$target" --replay "$file" < /dev/null > "target/$target.$name.out" 2>&1; then
                    echo "$name passes alone" | tee -a "$log"
                else
                    echo "error: $name times out alone too (target/$target.$name.out)" | tee -a "$log" >&2
                    failed=1
                fi
                ;;
            *)
                echo "error: libFuzzer wrote $file during the run" | tee -a "$log" >&2
                failed=1
                ;;
        esac
    done <<< "$(find "artifacts/$target" -newer "target/$target.started" -type f)"
    exit "$failed"
fi
