#!/usr/bin/env bash
# Runs the Go updater's bundle checks (Go internal/update/mac_darwin.go
# stage, onlyApp, checkBundle, checkSignature; surface.md 7.3) on a release
# .app.zip, so a zip that passes here is one the installed Go v0.5.x Irori
# (and crates/update's mac.rs, the same checks) will take:
#   - ditto -x -k unpacks it to exactly one .app at the top level;
#   - CFBundleIdentifier is com.stephenshorton.irori;
#   - CFBundleShortVersionString is the release's X.Y.Z;
#   - Contents/MacOS/<CFBundleExecutable> is a regular executable file;
#   - codesign --verify --deep --strict passes;
#   - with --installed <Irori.app>: refused, as Go refuses, when that app is
#     Developer ID signed and the update is not.
#
#   app/macos/check-bundle.sh irori-0.6.0-macos-arm64.app.zip 0.6.0 [--installed /Applications/Irori.app]
set -euo pipefail

ZIP="${1:-}" VERSION="${2:-}"
[[ -f "$ZIP" && -n "$VERSION" ]] || { echo "usage: $0 <app.zip> <version> [--installed <app>]" >&2; exit 2; }
INSTALLED=""
if [[ "${3:-}" == "--installed" ]]; then INSTALLED="${4:-}"; fi
NUMERIC="${VERSION#v}"
NUMERIC="${NUMERIC%%[-+]*}"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/irori-bundle-check.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
fail() { echo "check-bundle: $*" >&2; exit 1; }

/usr/bin/ditto -x -k "$ZIP" "$WORK/unpacked" || fail "unpack $(basename "$ZIP") failed"
apps=()
for e in "$WORK/unpacked"/*.app "$WORK/unpacked"/.*.app; do
  if [[ -d "$e" ]]; then apps+=("$e"); fi
done
[[ ${#apps[@]} -eq 1 ]] || fail "update zip holds ${#apps[@]} .app bundles, want 1"
APP="${apps[0]}"
PLIST="$APP/Contents/Info.plist"
get() { /usr/bin/plutil -extract "$1" raw -o - "$PLIST" || fail "read $1 from $PLIST"; }

id="$(get CFBundleIdentifier)"
[[ "$id" == "com.stephenshorton.irori" ]] || fail "update bundle id is \"$id\", want \"com.stephenshorton.irori\""
v="$(get CFBundleShortVersionString)"
[[ "$v" == "$NUMERIC" ]] || fail "update bundle is version \"$v\", want \"$NUMERIC\""
exe="$(get CFBundleExecutable)"
[[ -f "$APP/Contents/MacOS/$exe" && -x "$APP/Contents/MacOS/$exe" ]] || fail "update bundle has no executable Contents/MacOS/$exe"
min="$(get LSMinimumSystemVersion)"
[[ "$min" == "12.0.0" ]] || fail "LSMinimumSystemVersion is \"$min\", want 12.0.0"
/usr/bin/codesign --verify --deep --strict "$APP" || fail "update signature does not verify"

# Captured first: `codesign | grep -q` under pipefail can fail when grep
# exits early and codesign takes SIGPIPE.
devid() {
  local out
  out="$(/usr/bin/codesign -dv --verbose=2 "$1" 2>&1 || true)"
  [[ "$out" == *"Authority=Developer ID Application"* ]]
}
if [[ -n "$INSTALLED" ]] && devid "$INSTALLED" && ! devid "$APP"; then
  fail "this Irori is signed with Developer ID and the update is not"
fi
echo "check-bundle: $(basename "$ZIP") passes (id $id, version $v, exe $exe)"
