#!/usr/bin/env sh
# Export Go-produced golden vectors into testdata/golden (architecture.md
# §6.2) without touching the Go repo: each *_golden_test.go here is injected
# into a Go package with `go test -overlay`, and the overlay file lives in a
# temporary directory.
#
#   sh tools/goldens/export.sh [path-to-go-irori]
#
# Default Go repo: ../irori-go next to this repo, a checkout of Go be67a86
# (since the swap ../irori is this repo). Run on Windows: the filepath
# exporter records Go's Windows answers (it skips elsewhere).
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
goirori="${1:-$root/../irori-go}"
goirori="$(cd "$goirori" && pwd)"
goroot="$(go env GOROOT | tr '\\' '/')"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Absolute paths in the form Go on this OS prints them.
native() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}
h="$(native "$here")"
gi="$(native "$goirori")"
gr="$(native "$goroot")"

cat >"$tmp/irori.json" <<EOF
{"Replace":{
  "$gi/internal/hlc/gocompat_golden_test.go":"$h/gocompat_golden_test.go",
  "$gi/internal/hlc/hlc_golden_test.go":"$h/hlc_golden_test.go",
  "$gi/internal/identity/identity_golden_test.go":"$h/identity_golden_test.go",
  "$gi/internal/inbox/inbox_golden_test.go":"$h/inbox_golden_test.go",
  "$gi/internal/ledger/ledger_golden_test.go":"$h/ledger_golden_test.go"
}}
EOF
cat >"$tmp/store.json" <<EOF
{"Replace":{"$gi/internal/store/store_golden_test.go":"$h/store_golden_test.go"}}
EOF
cat >"$tmp/coord.json" <<EOF
{"Replace":{"$gi/internal/coord/coord_golden_test.go":"$h/coord_golden_test.go"}}
EOF
cat >"$tmp/m3leaf.json" <<EOF
{"Replace":{
  "$gi/internal/scrub/scrub_golden_test.go":"$h/scrub_golden_test.go",
  "$gi/internal/search/search_golden_test.go":"$h/search_golden_test.go",
  "$gi/internal/blob/blob_golden_test.go":"$h/blob_golden_test.go",
  "$gi/internal/presence/presence_golden_test.go":"$h/presence_golden_test.go"
}}
EOF
cat >"$tmp/capture.json" <<EOF
{"Replace":{
  "$gi/internal/capture/transcript/fixture_test.go":"$h/transcript_fixture_test.go",
  "$gi/internal/capture/transcript/transcript_golden_test.go":"$h/transcript_golden_test.go",
  "$gi/internal/capture/harness_test.go":"$h/capture_harness_test.go",
  "$gi/internal/capture/capture_golden_test.go":"$h/capture_golden_test.go",
  "$gi/internal/capture/gitlink/gitlink_golden_test.go":"$h/gitlink_golden_test.go",
  "$gi/internal/repo/repo_golden_test.go":"$h/repo_golden_test.go"
}}
EOF
cat >"$tmp/attach.json" <<EOF
{"Replace":{"$gi/internal/api/attach_golden_test.go":"$h/attach_golden_test.go"}}
EOF
cat >"$tmp/tools.json" <<EOF
{"Replace":{"$gi/internal/tools/tools_golden_test.go":"$h/tools_golden_test.go"}}
EOF
cat >"$tmp/mcpserver.json" <<EOF
{"Replace":{"$gi/internal/mcpserver/mcpserver_golden_test.go":"$h/mcpserver_golden_test.go"}}
EOF
cat >"$tmp/api.json" <<EOF
{"Replace":{"$gi/internal/api/zz_api_golden_test.go":"$h/api_golden_test.go"}}
EOF
cat >"$tmp/home.json" <<EOF
{"Replace":{"$gi/internal/home/home_golden_test.go":"$h/home_golden_test.go"}}
EOF
cat >"$tmp/meshseam.json" <<EOF
{"Replace":{"$gi/internal/mesh/meshseam_golden_test.go":"$h/meshseam_golden_test.go"}}
EOF
cat >"$tmp/shell.json" <<EOF
{"Replace":{"$gi/zz_shell_golden_test.go":"$h/shell_golden_test.go"}}
EOF
cat >"$tmp/mesh.json" <<EOF
{"Replace":{"$gi/internal/mesh/mesh_golden_test.go":"$h/mesh_golden_test.go"}}
EOF
cat >"$tmp/notify.json" <<EOF
{"Replace":{"$gi/internal/notify/notify_golden_test.go":"$h/notify_golden_test.go"}}
EOF
cat >"$tmp/std.json" <<EOF
{"Replace":{"$gr/src/path/filepath/zz_irori_golden_test.go":"$h/filepath_golden_test.go"}}
EOF

out="$(native "$root/testdata/golden")"
tables="$(native "$root/crates/gocompat/src/text/tables.rs")"
mkdir -p "$root/testdata/golden"

before="$(git -C "$goirori" status --porcelain)"
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" IRORI_TABLES_OUT="$tables" \
    go test -count=1 -overlay "$(native "$tmp/irori.json")" -vet=off -run '^TestGolden' ./internal/hlc ./internal/ledger)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/store.json")" -vet=off -run '^TestGoldenStore' ./internal/store)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/irori.json")" -vet=off -run '^TestGoldenIdentity' ./internal/identity)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/irori.json")" -vet=off -run '^TestGoldenInbox' ./internal/inbox)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/std.json")" -vet=off -run '^TestGoldenFilepath$' path/filepath)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/coord.json")" -vet=off -run '^TestGoldenCoord' ./internal/coord)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/m3leaf.json")" -vet=off -run '^TestGolden' \
      ./internal/scrub ./internal/search ./internal/blob ./internal/presence)
# capture: every transcript and capture test runs (the overlaid fixture and
# harness files record what they feed and log), then the gitlink and repo
# vectors.
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/capture.json")" -vet=off ./internal/capture/transcript ./internal/capture)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/capture.json")" -vet=off -run '^TestGolden' ./internal/capture/gitlink ./internal/repo)
# attach: Event lines, Preview and ModAtLeast (crates/attach).
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/attach.json")" -vet=off -run '^TestGoldenAttach$' ./internal/api)
# tools and mcp: the tool-text scenario (testdata/golden/tools/scenario.json,
# read from the output dir) over the tools harness, the spec lists, and
# `irori mcp`'s initialize, tools/list and fixed texts.
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/tools.json")" -vet=off -run '^TestGoldenTools' ./internal/tools)
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/mcpserver.json")" -vet=off -run '^TestGoldenMCP' ./internal/mcpserver)
# api: a Go Core and its own api.Server run testdata/golden/api/scenario.json
# (Windows only; the file has a build tag).
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/api.json")" -vet=off -run '^TestGoldenAPI$' ./internal/api)
# home: node.addr, home-server.json and Status bytes, HostPort, CGNAT, and
# a Go server's Status and row with a fake router (crates/home).
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/home.json")" -vet=off -run '^TestGoldenHome$' ./internal/home)
# meshseam: MeshSecret / MeshInfo (crates/meshseam, the epoch-open key).
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/meshseam.json")" -vet=off -run '^TestGoldenMeshSeam$' ./internal/mesh)
# shell: the root package's instance id, tray texts and role tables
# (app/src/shell, app/src/cli.rs). The package embeds frontend/dist, which
# the Go checkout's own build leaves in place.
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/shell.json")" -vet=off -run '^TestGoldenShell$' .)
# mesh: MeshSecret, person.secret sealing, sync frames, node.addr, the
# data-dir files and the Status JSON (crates/mesh).
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/mesh.json")" -vet=off -run '^TestGoldenMesh$' ./internal/mesh)
# notify: settings.json load and save bytes (crates/notify).
(cd "$goirori" &&
  IRORI_GOLDEN_OUT="$out" \
    go test -count=1 -overlay "$(native "$tmp/notify.json")" -vet=off -run '^TestGoldenNotify' ./internal/notify)
after="$(git -C "$goirori" status --porcelain)"
if [ "$before" != "$after" ]; then
  echo "export: the Go repo's status changed; it must stay untouched" >&2
  exit 1
fi
echo "export: goldens written to testdata/golden"
