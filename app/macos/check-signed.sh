#!/usr/bin/env bash
# Checks that a release .app.zip is signed the way a Mac release must be
# (docs/releasing.md): Developer ID of this team, so its designated
# requirement names the Team ID and macOS keeps its permissions across
# updates; hardened runtime and a timestamp; and the mic entitlement on the
# executable, the process that records. check-bundle.sh is the updater's
# checks and passes ad hoc builds too; this one doesn't.
#
#   app/macos/check-signed.sh irori-0.13.0-macos-arm64.app.zip GP6632G333
set -euo pipefail

ZIP="${1:-}" TEAM="${2:-}"
[[ -f "$ZIP" && -n "$TEAM" ]] || { echo "usage: $0 <app.zip> <team id>" >&2; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/irori-signed-check.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
fail() { echo "check-signed: $*" >&2; exit 1; }

/usr/bin/ditto -x -k "$ZIP" "$WORK"
APP="$WORK/Irori.app"
EXE="$APP/Contents/MacOS/irori"
[[ -f "$EXE" ]] || fail "$(basename "$ZIP") has no Irori.app/Contents/MacOS/irori"

# Captured first: `codesign | grep -q` under pipefail can fail when grep
# exits early and codesign takes SIGPIPE.
for target in "$APP" "$EXE"; do
  name="${target#"$WORK/"}"
  info="$(/usr/bin/codesign -dvv -r- "$target" 2>&1)"
  [[ "$info" == *"Authority=Developer ID Application"* ]] || fail "$name is not signed with Developer ID"
  [[ "$info" == *"TeamIdentifier=$TEAM"* ]] || fail "$name is not signed by team $TEAM"
  [[ "$info" == *"certificate leaf[subject.OU] = $TEAM"* ]] || fail "$name's designated requirement doesn't name team $TEAM"
  [[ "$info" != *"cdhash"* ]] || fail "$name's designated requirement is a cdhash"
  [[ "$info" == *"(runtime)"* ]] || fail "$name has no hardened runtime"
  [[ "$info" == *"Timestamp="* ]] || fail "$name has no secure timestamp"
done

entitlements="$(/usr/bin/codesign -d --entitlements - --xml "$EXE" 2>/dev/null)"
[[ "$entitlements" == *"com.apple.security.device.audio-input"* ]] || fail "the executable has no audio-input entitlement, so voice would record silence"

echo "check-signed: $(basename "$ZIP") passes (Developer ID, team $TEAM, hardened runtime, audio-input)"
