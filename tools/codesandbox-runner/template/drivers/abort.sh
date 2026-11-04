#!/usr/bin/env bash
# Stop a cloud agent run: the driver (symphony-run-agent) and everything it started, which is the
# agent (mini / pi) and the commands that agent ran. Signalling only the driver would leave those
# running, and still pushing. Run by the bridge for an `abort` request; it never fails.
#
# The tree is frozen (SIGSTOP) and rescanned until no new process turns up, then stopped: a process
# that is still running could start another one between the scan and the signal, and that one would
# be out of reach as soon as its parent is gone.
set -u

# The bracket keeps this script's own command line from matching the pattern.
PATTERN='[s]ymphony-run-agent'

# $1 and everything below it, children before parents so nothing is orphaned out of reach.
tree() {
  local child
  for child in $(pgrep -P "$1" 2>/dev/null); do tree "$child"; done
  echo "$1"
}

pids=()   # every process of the run found so far
fresh=()  # the ones the latest scan found for the first time

scan() {
  fresh=()
  local root pid
  for root in $(pgrep -f "$PATTERN" 2>/dev/null); do
    [ "$root" = "$$" ] && continue
    for pid in $(tree "$root"); do
      case " ${pids[*]-} " in *" $pid "*) continue ;; esac
      pids+=("$pid")
      fresh+=("$pid")
    done
  done
}

for _ in 1 2 3 4 5 6 7 8 9 10; do
  scan
  [ "${#fresh[@]}" -eq 0 ] && break
  kill -STOP "${fresh[@]}" 2>/dev/null
done
[ "${#pids[@]}" -eq 0 ] && exit 0

# A stopped process only acts on SIGTERM once it is continued.
kill -TERM "${pids[@]}" 2>/dev/null
kill -CONT "${pids[@]}" 2>/dev/null

# Up to five seconds to exit cleanly, then whatever is left is killed.
for _ in 1 2 3 4 5 6 7 8 9 10; do
  alive=0
  for pid in "${pids[@]}"; do
    kill -0 "$pid" 2>/dev/null && alive=1
  done
  [ "$alive" -eq 0 ] && exit 0
  sleep 0.5
done
kill -KILL "${pids[@]}" 2>/dev/null
exit 0
