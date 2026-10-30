#!/usr/bin/env sh
# The M3 check on real data (architecture.md §6.3 "Folds", §8 M3), on fresh
# copies:
#
#   sh tools/goldens/replay-live.sh <data-dir> <scratch-dir> [path-to-go-irori]
#
# 1. Copy the data dir's identity files, boards/ and irori.db (+ -wal and
#    -shm, copied twice and compared, as verify-live.sh does; the live
#    database is never opened) into <scratch>/irori-replay/rust, and the
#    same stable set again into .../go. Two homes: the Go Core appends on
#    start (its start row, a self-cert, inbox holds), so it never runs on
#    the copy Rust reads.
# 2. `irori-dev replay` folds the Rust copy read-only (no network, no
#    tailing, nothing appended) and writes its views to .../views/rust.
# 3. The Go Core starts on the Go copy with a FakeNet and an empty Claude
#    dir (TestGoReplayViews, tools/goldens/core_replay_test.go) and writes
#    the same views to .../views/go. No copy ever runs with a real
#    transport: it holds this device's real identity.
# 4. `irori-dev replay --compare` diffs the two, view by view.
#
# On macOS set IRORI_DEV_KEK_DIR=$HOME/.irori (docs/m1-mac.md §2.2): the
# keys of a copy are sealed with the Keychain item named after the original
# path, and kek-overlay.sh points the Go Core's keystore at it too.
#
# Everything under <scratch-dir>/irori-replay is deleted at the end. With
# IRORI_REPLAY_KEEP_VIEWS=<dir> (inside the scratch dir) the two view
# folders are moved there first, for reading a difference; delete them
# after.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
data="$1"
scratch="$2"
goirori="${3:-$root/../irori-go}"
goirori="$(cd "$goirori" && pwd)"
native() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}
work="$scratch/irori-replay"
rm -rf "$work"
mkdir -p "$work/rust" "$work/go" "$work/views"
cleanup() {
  if [ -n "${IRORI_REPLAY_KEEP_VIEWS:-}" ] && [ -d "$work/views" ]; then
    rm -rf "$IRORI_REPLAY_KEEP_VIEWS" && mv "$work/views" "$IRORI_REPLAY_KEEP_VIEWS"
  fi
  rm -rf "$work"
}
trap cleanup EXIT

dbset() { # copy the live database files into $1
  rm -rf "$1" && mkdir -p "$1"
  for f in irori.db irori.db-wal irori.db-shm; do
    if [ -f "$data/$f" ]; then cp "$data/$f" "$1/$f"; fi
  done
}
same=0
for try in 1 2 3 4 5; do
  dbset "$work/db-a"
  dbset "$work/db-b"
  if diff -r "$work/db-a" "$work/db-b" >/dev/null 2>&1; then
    same=1
    break
  fi
  echo "replay-live: the database changed during copy $try; copying again" >&2
  sleep 1
done
if [ "$same" != 1 ]; then
  echo "replay-live: the database kept changing while it was copied; try again when Irori is idle" >&2
  exit 1
fi
# No irori.json: the Go Core refuses a data dir whose lockfile names
# another live process, and irori-dev refuses a dir holding one.
for h in rust go; do
  for f in device.json device.key person.json turn.key \
    mesh-peers.json hub-addrs.json home-server.json relay.json names.json settings.json; do
    if [ -f "$data/$f" ]; then cp "$data/$f" "$work/$h/$f"; fi
  done
  for f in "$work"/db-a/*; do cp "$f" "$work/$h/"; done
  if [ -d "$data/boards" ]; then cp -R "$data/boards" "$work/$h/boards"; fi
done
rm -rf "$work/db-a" "$work/db-b"

now="$(date +%Y-%m-%dT%H:%M:%S%z | sed 's/\([0-9][0-9]\)$/:\1/')"
(cd "$root" && cargo run -q -p irori-dev -- replay --home "$(native "$work/rust")" --out "$(native "$work/views")" --now "$now")

. "$here/kek-overlay.sh"
kek="$(kek_overlay)"
cat >"$work/core.json" <<EOF
{"Replace":{"$(native "$goirori")/internal/core/core_replay_test.go":"$(native "$here")/core_replay_test.go"$kek}}
EOF
before="$(git -C "$goirori" status --porcelain)"
status=0
(cd "$goirori" &&
  IRORI_REPLAY_HOME="$(native "$work/go")" IRORI_REPLAY_OUT="$(native "$work/views")" \
    go test -count=1 -v -overlay "$(native "$work/core.json")" -vet=off -run '^TestGoReplayViews$' ./internal/core \
    >"$work/go-core.log" 2>&1) || status=$?
# Only the test's own lines: the Core logs paths and names as it starts.
grep -E '^(---|ok|FAIL|panic|\s+core_replay_test)' "$work/go-core.log" || true
if [ "$status" != 0 ]; then
  echo "replay-live: the Go Core did not write its views" >&2
  exit 1
fi
after="$(git -C "$goirori" status --porcelain)"
if [ "$before" != "$after" ]; then
  echo "replay-live: the Go repo's status changed; it must stay untouched" >&2
  exit 1
fi

(cd "$root" && cargo run -q -p irori-dev -- replay --compare "$(native "$work/views")")
