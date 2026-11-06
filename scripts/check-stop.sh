#!/usr/bin/env sh
# Stops this worktree's scripts/check.sh run, and only it (card #609):
# check.sh with everything it started (cargo, rustc, test binaries, bun),
# then gives its check slot back. `pkill -f check.sh` would stop every
# agent's run on the device.
#
#   sh scripts/check-stop.sh
#
# check.sh writes its pid to check.pid in this worktree's git dir as it
# starts and removes it as it ends.
set -eu
cd "$(dirname "$0")/.."
. scripts/check-slot.sh

state=$(git rev-parse --git-path check.pid)
if [ ! -f "$state" ]; then
  echo "check-stop: no check.sh is running in this worktree"
  exit 0
fi
pid=$(sed -n 's/^pid=//p' "$state")
winpid=$(sed -n 's/^winpid=//p' "$state")

# Is $1 still a check.sh? A pid can be reused once its process is gone.
is_check() {
  if [ -r "/proc/$1/cmdline" ]; then
    tr '\0' ' ' <"/proc/$1/cmdline" | grep -q 'check\.sh'
  else
    ps -o command= -p "$1" 2>/dev/null | grep -q 'check\.sh'
  fi
}

# $1's descendants, deepest last.
descendants() {
  for c in $(pgrep -P "$1" 2>/dev/null || true); do
    echo "$c"
    descendants "$c"
  done
}

if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null || ! is_check "$pid"; then
  echo "check-stop: check.sh (pid ${pid:-?}) has already ended"
  rm -f "$state"
  exit 0
fi

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    # cargo and what it starts are Windows processes the shell's kill
    # can't reach; taskkill /T ends the whole tree from the shell's
    # Windows pid down. It runs no trap, so the slot is released below.
    [ -n "$winpid" ] || winpid=$(cat "/proc/$pid/winpid")
    # It fails on children that are already ending as their parent goes
    # ("not supported"); whether check.sh is gone is checked below.
    MSYS_NO_PATHCONV=1 taskkill /F /T /PID "$winpid" >/dev/null 2>&1 || true
    ;;
  *)
    # TERM to check.sh first: its trap releases the slot once the
    # command it waits on ends, and ending them is the next line.
    kids=$(descendants "$pid")
    kill -TERM "$pid" 2>/dev/null || true
    for k in $kids; do kill -TERM "$k" 2>/dev/null || true; done
    n=0
    while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 20 ]; do
      sleep 0.5
      n=$((n + 1))
    done
    for k in "$pid" $kids; do kill -KILL "$k" 2>/dev/null || true; done
    ;;
esac
n=0
while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 20 ]; do
  sleep 0.5
  n=$((n + 1))
done
if kill -0 "$pid" 2>/dev/null; then
  echo "check-stop: check.sh (pid $pid) is still running" >&2
  exit 1
fi

# Give the slot back now rather than after its TTL.
if slot_connect 2>/dev/null; then
  slot_ids "$pid"
  if slot_post "/v1/slots/release?resource=$slot_res&run=$slot_run" | grep -q '"released": *true'; then
    echo "check-stop: released $slot_run's check slot"
  fi
fi
rm -f "$state"
echo "check-stop: stopped check.sh (pid $pid)"
