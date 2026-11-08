#!/usr/bin/env bash
# The long run before a tag (`README.md`): one target for HOURS hours, in parts of one hour, each with its own output.
# A part that stopped early (`run.sh` exit 3) is run again. A timeout is run again alone: if it passes, the clock or the
# load made it, and the part is run again; if it times out again, or on any other failure, the target stops.
#
# usage: long.sh TARGET HOURS
# Results in target/long/: TARGET.log, one line for each part, TARGET.N.out, the output of part N, and
# TARGET.N.<artifact>.out, the output of a timeout run again alone. A later sitting goes on with the next N.
# Exit 0 when HOURS hours are done, 1 on a failure, 2 after too many parts run again.
# LONG_PART=60 makes each part last 60 s instead of an hour, to try the script.
set -uo pipefail

usage="usage: long.sh TARGET HOURS"
target=${1:?$usage}
hours=${2:?$usage}
case "$hours" in
    '' | *[!0-9]* | 0) echo "$usage (HOURS is a whole number, at least 1)" >&2; exit 1 ;;
esac
part_seconds=${LONG_PART:-3600}
cd "$(dirname "$0")"
out=target/long
log=$out/$target.log
mkdir -p "$out"

say() { echo "$(date -u +%FT%TZ) $*" | tee -a "$log"; }

# The parts of an earlier sitting are kept: go on after the last one.
part=$(find "$out" -maxdepth 1 -name "$target.*.out" 2>/dev/null | sed -n "s|^$out/$target\.\([0-9]*\)\.out$|\1|p" | sort -n | tail -n 1)
part=${part:-0}
done_hours=0
again=0
say "start $target, $hours h, after part $part"
while [ "$done_hours" -lt "$hours" ]; do
    part=$((part + 1))
    output=$out/$target.$part.out
    bash run.sh "$target" "$part_seconds" > "$output" 2>&1
    status=$?
    # What libFuzzer wrote in this part, whether or not an earlier part wrote the same file; a slow unit is not a failure.
    written=$(sed -n 's/^.*Test unit written to //p' "$output" | grep -v '/slow-unit-' || true)
    runs=$(sed -n 's/^stat::number_of_executed_units: *//p' "$output")
    if [ "$status" -eq 0 ]; then
        done_hours=$((done_hours + 1))
        say "part $part: hour $done_hours of $hours done, ${runs:-?} executions"
        continue
    fi
    if [ "$status" -eq 3 ] && [ -z "$written" ]; then
        say "part $part: stopped early, run again"
    elif [ -n "$written" ] && ! echo "$written" | grep -qv '/timeout-[0-9a-f]*$'; then
        while IFS= read -r file; do
            name=$(basename "$file")
            if bash run.sh "$target" --replay "$file" < /dev/null > "$out/$target.$part.$name.out" 2>&1; then
                say "part $part: $name passes alone, run again"
            else
                say "part $part: $name times out alone too: stop"
                exit 1
            fi
        done <<< "$written"
    else
        say "part $part: failed (exit $status, written: ${written:-nothing}): stop"
        exit 1
    fi
    again=$((again + 1))
    if [ "$again" -gt 40 ]; then
        say "more than 40 parts run again: stop"
        exit 2
    fi
done
say "$target: $hours h done"
