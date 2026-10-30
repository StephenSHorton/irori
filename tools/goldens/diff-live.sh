#!/usr/bin/env sh
# Differential check on real data (architecture.md §6.3 "Rows"): Go and
# Rust each hash every row of their own scratch copy of a real irori.db, and
# the Rust test requires the two lists to be equal.
#
#   sh tools/goldens/diff-live.sh <data-dir> <scratch-dir> [path-to-go-irori]
#
# The live database is never opened: irori.db, -wal and -shm are copied
# together (the Go build may be running and holds the file with WAL), then
# the copy is copied again so Go and Rust each open their own. Everything
# made under <scratch-dir>/irori-diff is deleted at the end. Output is row
# counts and the keys of mismatched rows, never row contents.
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
work="$scratch/irori-diff"
rm -rf "$work"
mkdir -p "$work/go" "$work/rust"
trap 'rm -rf "$work"' EXIT

for f in irori.db irori.db-wal irori.db-shm; do
  if [ -f "$data/$f" ]; then cp "$data/$f" "$work/go/$f"; fi
done
for f in irori.db irori.db-wal irori.db-shm; do
  if [ -f "$work/go/$f" ]; then cp "$work/go/$f" "$work/rust/$f"; fi
done

cat >"$work/overlay.json" <<EOF
{"Replace":{"$(native "$goirori")/internal/store/store_golden_test.go":"$(native "$here")/store_golden_test.go"}}
EOF
before="$(git -C "$goirori" status --porcelain)"
(cd "$goirori" &&
  IRORI_DIFF_DB="$(native "$work/go/irori.db")" IRORI_DIFF_OUT="$(native "$work/go-hashes.tsv")" \
    go test -count=1 -overlay "$(native "$work/overlay.json")" -vet=off -run '^TestDiffLiveRowHashes$' ./internal/store)
after="$(git -C "$goirori" status --porcelain)"
if [ "$before" != "$after" ]; then
  echo "diff-live: the Go repo's status changed; it must stay untouched" >&2
  exit 1
fi
(cd "$root" &&
  IRORI_DIFF_DB="$(native "$work/rust/irori.db")" IRORI_DIFF_GO="$(native "$work/go-hashes.tsv")" \
    cargo test -q -p irori-store --test live -- --ignored --nocapture --exact live_row_hashes_match_go)

# The other way: Rust appends and inserts on its copy, then Go opens that
# copy, checks every RowHash against Rust's list, and appends.
(cd "$root" &&
  IRORI_DIFF_DB="$(native "$work/rust/irori.db")" IRORI_DIFF_MANIFEST="$(native "$work/rust-hashes.tsv")" \
    cargo test -q -p irori-store --test live -- --ignored --nocapture --exact live_rust_writes_for_go)
(cd "$goirori" &&
  IRORI_RUST_DB="$(native "$work/rust/irori.db")" IRORI_RUST_DEVICE=irori-rust-diff \
  IRORI_RUST_MANIFEST="$(native "$work/rust-hashes.tsv")" \
    go test -count=1 -v -overlay "$(native "$work/overlay.json")" -vet=off -run '^TestDiffOpenRustDB$' ./internal/store |
    grep -E '^(---|ok|FAIL|\s+store_golden_test)')
after="$(git -C "$goirori" status --porcelain)"
if [ "$before" != "$after" ]; then
  echo "diff-live: the Go repo's status changed; it must stay untouched" >&2
  exit 1
fi
echo "diff-live: passed"
