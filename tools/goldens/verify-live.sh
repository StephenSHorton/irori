#!/usr/bin/env sh
# The M2 check on real data (architecture.md §6.3, §8 M2), on a fresh copy:
#
#   sh tools/goldens/verify-live.sh <data-dir> <scratch-dir> [path-to-go-irori]
#
# 1. Copy the data dir's identity files, boards/ and irori.db (+ -wal and
#    -shm together; the live database is never opened) into
#    <scratch-dir>/irori-verify/home, and the database alone again into
#    .../gohash for Go. The three database files are copied twice and the
#    two sets compared, retrying while they differ, so a checkpoint between
#    the plain copies can't leave a torn set. (No sqlite3 `.backup`: it is
#    not installed here, and it would open the live file.)
# 2. Go hashes every row of its copy (TestDiffLiveRowHashes).
# 3. `irori-dev verify` on the Rust copy: every row signature and RowHash,
#    board logs, then one signed Rust append. Only with
#    IRORI_VERIFY_DEVICES=1 is the Chain also compared with the running Go
#    Core's /v1/devices: irori.json is copied to <scratch>/irori-verify/lock
#    (never into the copied home) and irori-dev reads its port and bearer
#    token in code, never printing them. A plain run reads no token.
# 4. The whole Go Core starts on the Rust copy with a FakeNet and checks
#    Rust's row (TestGoOpensRustCopy, tools/goldens/core_rustcopy_test.go).
#    The copy is never run as `irori serve`: it holds this device's real
#    identity, and a real transport would fork its history on other devices.
#
# On macOS set IRORI_DEV_KEK_DIR=$HOME/.irori (docs/m1-mac.md §2.2): the
# keys of a copy are sealed with the Keychain item named after the original
# path, and kek-overlay.sh points the Go Core's keystore at it too.
#
# Everything under <scratch-dir>/irori-verify is deleted at the end.
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
work="$scratch/irori-verify"
rm -rf "$work"
mkdir -p "$work/home" "$work/gohash"
trap 'rm -rf "$work"' EXIT

# No irori.json: the Go Core refuses a data dir whose lockfile names
# another live process.
for f in device.json device.key person.json turn.key \
  mesh-peers.json hub-addrs.json home-server.json relay.json names.json settings.json; do
  if [ -f "$data/$f" ]; then cp "$data/$f" "$work/home/$f"; fi
done
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
  echo "verify-live: the database changed during copy $try; copying again" >&2
  sleep 1
done
if [ "$same" != 1 ]; then
  echo "verify-live: the database kept changing while it was copied; try again when Irori is idle" >&2
  exit 1
fi
for f in "$work"/db-a/*; do cp "$f" "$work/home/"; done
rm -rf "$work/db-a" "$work/db-b"
if [ -d "$data/boards" ]; then cp -R "$data/boards" "$work/home/boards"; fi
for f in irori.db irori.db-wal irori.db-shm; do
  if [ -f "$work/home/$f" ]; then cp "$work/home/$f" "$work/gohash/$f"; fi
done

cat >"$work/store.json" <<EOF
{"Replace":{"$(native "$goirori")/internal/store/store_golden_test.go":"$(native "$here")/store_golden_test.go"}}
EOF
. "$here/kek-overlay.sh"
kek="$(kek_overlay)"
cat >"$work/core.json" <<EOF
{"Replace":{"$(native "$goirori")/internal/core/core_rustcopy_test.go":"$(native "$here")/core_rustcopy_test.go"$kek}}
EOF
before="$(git -C "$goirori" status --porcelain)"
(cd "$goirori" &&
  IRORI_DIFF_DB="$(native "$work/gohash/irori.db")" IRORI_DIFF_OUT="$(native "$work/go-hashes.tsv")" \
    go test -count=1 -overlay "$(native "$work/store.json")" -vet=off -run '^TestDiffLiveRowHashes$' ./internal/store)

set -- --home "$(native "$work/home")" --go-hashes "$(native "$work/go-hashes.tsv")" \
  --append "$(native "$work/manifest.tsv")"
if [ "${IRORI_VERIFY_DEVICES:-}" = 1 ]; then
  mkdir -p "$work/lock"
  cp "$data/irori.json" "$work/lock/irori.json"
  set -- "$@" --lockfile "$(native "$work/lock/irori.json")"
fi
(cd "$root" && cargo run -q -p irori-dev -- verify "$@")

status=0
(cd "$goirori" &&
  IRORI_RUST_HOME="$(native "$work/home")" IRORI_RUST_MANIFEST="$(native "$work/manifest.tsv")" \
    go test -count=1 -v -overlay "$(native "$work/core.json")" -vet=off -run '^TestGoOpensRustCopy$' ./internal/core \
    >"$work/go-core.log" 2>&1) || status=$?
# Only the test's own lines: the Core logs paths and names as it starts.
grep -E '^(---|ok|FAIL|panic|\s+core_rustcopy_test)' "$work/go-core.log" || true
if [ "$status" != 0 ]; then
  echo "verify-live: the Go Core did not pass on the Rust copy" >&2
  exit 1
fi
after="$(git -C "$goirori" status --porcelain)"
if [ "$before" != "$after" ]; then
  echo "verify-live: the Go repo's status changed; it must stay untouched" >&2
  exit 1
fi
echo "verify-live: passed"
