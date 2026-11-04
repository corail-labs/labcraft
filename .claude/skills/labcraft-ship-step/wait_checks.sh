#!/bin/sh
# Waits until the check of a pull request has finished. Exit 0: green. Exit 1: a check failed, or the
# pull request cannot be read. Exit 2: not finished in time. Read-only.
#
# Green means a check run exists for the head commit, all are completed with success, and the pull
# request is CLEAN: GitHub can report the workflow run as finished while the check is still in progress.
#
# usage: wait_checks.sh PR_NUMBER [SECONDS]

pr=${1:?usage: wait_checks.sh PR_NUMBER [SECONDS]}
limit=${2:-540}
waited=0

head=$(gh pr view "$pr" --json headRefOid --jq .headRefOid) || exit 1
[ -n "$head" ] || exit 1
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner) || exit 1

while [ "$waited" -le "$limit" ]; do
    runs=$(gh api "repos/$repo/commits/$head/check-runs" --jq '[.check_runs | length, ([.[] | select(.status != "completed")] | length), ([.[] | select(.status == "completed" and .conclusion != "success")] | length)] | join(" ")' 2>/dev/null)
    state=$(gh pr view "$pr" --json mergeStateStatus --jq .mergeStateStatus 2>/dev/null)
    set -- $runs
    total=${1:-0} open=${2:-0} failed=${3:-0}
    echo "${waited}s: $total checks, $open running, $failed failed, pull request $state"
    if [ "$failed" != 0 ]; then
        echo "FAILED"
        exit 1
    fi
    if [ "$total" != 0 ] && [ "$open" = 0 ] && [ "$state" = CLEAN ]; then
        echo "GREEN"
        exit 0
    fi
    sleep 20
    waited=$((waited + 20))
done
echo "NOT FINISHED after ${limit}s"
exit 2
