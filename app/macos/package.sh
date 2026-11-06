#!/usr/bin/env bash
# Package a built irori binary as Irori.app, sign it, and write the release's
# .app.zip (the in-app updater's payload, Go's and this build's) and .dmg
# (drag to Applications). A port of the Go build's build/darwin/package.sh
# (Go be67a86); the bundle layout, Info.plist keys and signing steps are the
# same, so the Go v0.5.x updater's bundle checks pass on it (surface.md 7.3).
# check-bundle.sh runs those checks on the zip before this script finishes.
#
#   app/macos/package.sh --binary target/aarch64-apple-darwin/release/irori \
#     --version 0.6.0 --out dist/release/v0.6.0-mac \
#     --zip irori-0.6.0-macos-arm64.app.zip --dmg irori-0.6.0-macos-arm64.dmg
#
# Signing follows the environment (scripts/release-mac.sh passes it on):
#   MACOS_CODESIGN_IDENTITY  empty or "-": ad-hoc signature, no notarization.
#                            "Developer ID Application: ...": hardened
#                            runtime + secure timestamp + entitlements.plist,
#                            then notarization when credentials are set (see
#                            notarize.sh).
#   IRORI_NOTARIZE=0         skip notarization even with Developer ID.
#
# A test bundle, for rehearsing an update relaunch beside the live Irori
# (architecture.md 3.2, card #611). `open` drops the shell's environment, so
# the bundle carries its own in Info.plist LSEnvironment:
#
#   app/macos/package.sh --binary target/debug/irori --version 0.13.0 \
#     --out <dir> --test-home <scratch data dir> [--test-id x] \
#     [--test-claude <empty dir>] [--test-env K=V]... [--test-online] [--zip z]
#
#   --test-home    IRORI_HOME for the bundle (absolute; made if missing).
#   --test-claude  CLAUDE_CONFIG_DIR (default <test-home>-claude, made empty).
#   --test-env     more LSEnvironment entries (IRORI_DEV_KEK_DIR, ...).
#   --test-online  leave out IRORI_DEV_OFFLINE=1 (on by default: a copy
#                  holds this device's key and must not join the mesh).
#   --test-id x    CFBundleIdentifier com.stephenshorton.irori.test-x, so
#                  quitting it by id can't reach the live app. The updater
#                  refuses any id but com.stephenshorton.irori, so build an
#                  update payload zip without --test-id (still with
#                  --test-home, or the relaunched app runs on ~/.irori);
#                  check-bundle.sh is skipped with --test-id.
# A test bundle is always signed ad hoc and never notarized; --zip and
# --dmg are optional (--dmg with --zip).
# IRORI_DEV_OFFLINE keeps the Core off the mesh, but the updater doesn't
# read it: a binary with a real IRORI_VERSION checks the real release feed
# and, on quit, installs a stock Irori.app (no LSEnvironment) over the test
# bundle, which then relaunches on ~/.irori. For a rehearsal build the
# binary with IRORI_UPDATE_API pointed at a fake feed; otherwise use a
# dev-version build (no IRORI_VERSION), whose updater never runs.
set -euo pipefail

BINARY="" VERSION="" OUT="" ZIP="" DMG=""
TEST_HOME="" TEST_ID="" TEST_CLAUDE="" TEST_ONLINE=0
TEST_ENV=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BINARY="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --zip) ZIP="$2"; shift 2 ;;
    --dmg) DMG="$2"; shift 2 ;;
    --test-home) TEST_HOME="$2"; shift 2 ;;
    --test-id) TEST_ID="$2"; shift 2 ;;
    --test-claude) TEST_CLAUDE="$2"; shift 2 ;;
    --test-env) TEST_ENV+=("$2"); shift 2 ;;
    --test-online) TEST_ONLINE=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
REQUIRED=(BINARY VERSION OUT ZIP DMG)
if [[ -n "$TEST_HOME" ]]; then
  REQUIRED=(BINARY VERSION OUT)
  if [[ -n "$DMG" && -z "$ZIP" ]]; then echo "error: a test bundle's --dmg needs --zip too" >&2; exit 2; fi
elif [[ -n "$TEST_ID$TEST_CLAUDE" || ${#TEST_ENV[@]} -gt 0 || $TEST_ONLINE == 1 ]]; then
  echo "error: --test-id, --test-claude, --test-env and --test-online need --test-home" >&2; exit 2
fi
if [[ -n "$TEST_ID" && ! "$TEST_ID" =~ ^[A-Za-z0-9-]+$ ]]; then
  echo "error: --test-id takes letters, digits and dashes" >&2; exit 2
fi
for v in "${REQUIRED[@]}"; do
  if [[ -z "${!v}" ]]; then echo "error: --$(echo "$v" | tr 'A-Z' 'a-z') is required" >&2; exit 2; fi
done
[[ -f "$BINARY" ]] || { echo "error: no binary at $BINARY" >&2; exit 1; }

HERE="$(cd "$(dirname "$0")" && pwd)"
BUNDLE_ID="com.stephenshorton.irori"
if [[ -n "$TEST_ID" ]]; then BUNDLE_ID="com.stephenshorton.irori.test-$TEST_ID"; fi
# CFBundleShortVersionString must be X.Y.Z: drop a prerelease or build part.
NUMERIC="${VERSION%%[-+]*}"

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
APP="$OUT/Irori.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

echo "==> assembling Irori.app $VERSION"
cp "$BINARY" "$APP/Contents/MacOS/irori"
chmod 755 "$APP/Contents/MacOS/irori"
cp "$HERE/Info.plist" "$APP/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$NUMERIC" "$APP/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$NUMERIC" "$APP/Contents/Info.plist"
if [[ -n "$TEST_HOME" ]]; then
  # LSEnvironment expands nothing: absolute paths only.
  mkdir -p "$TEST_HOME"
  TEST_HOME="$(cd "$TEST_HOME" && pwd -P)"
  if [[ "$TEST_HOME" == "$(cd "$HOME" && pwd -P)/.irori" ]]; then
    echo "error: --test-home is the live data dir" >&2; exit 2
  fi
  [[ -n "$TEST_CLAUDE" ]] || TEST_CLAUDE="$TEST_HOME-claude"
  mkdir -p "$TEST_CLAUDE"
  TEST_CLAUDE="$(cd "$TEST_CLAUDE" && pwd -P)"
  PLIST="$APP/Contents/Info.plist"
  plutil -replace CFBundleIdentifier -string "$BUNDLE_ID" "$PLIST"
  plutil -insert LSEnvironment -dictionary "$PLIST"
  plutil -insert LSEnvironment.IRORI_HOME -string "$TEST_HOME" "$PLIST"
  plutil -insert LSEnvironment.CLAUDE_CONFIG_DIR -string "$TEST_CLAUDE" "$PLIST"
  if [[ $TEST_ONLINE == 0 ]]; then plutil -insert LSEnvironment.IRORI_DEV_OFFLINE -string 1 "$PLIST"; fi
  for kv in "${TEST_ENV[@]+"${TEST_ENV[@]}"}"; do
    k="${kv%%=*}"
    if [[ "$kv" != *=* || ! "$k" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
      echo "error: --test-env wants K=V, got $kv" >&2; exit 2
    fi
    plutil -replace "LSEnvironment.$k" -string "${kv#*=}" "$PLIST"
  done
  echo "==> test bundle $BUNDLE_ID, LSEnvironment:"
  plutil -extract LSEnvironment json -o - "$PLIST"
  echo
fi
plutil -lint "$APP/Contents/Info.plist"
cp "$HERE/icons.icns" "$APP/Contents/Resources/"
if [[ -f "$HERE/Assets.car" ]]; then cp "$HERE/Assets.car" "$APP/Contents/Resources/"; fi

IDENTITY="${MACOS_CODESIGN_IDENTITY:-}"
[[ -n "$IDENTITY" ]] || IDENTITY="-"
[[ -z "$TEST_HOME" ]] || IDENTITY="-"
DEVELOPER_ID=0
if [[ "$IDENTITY" == "Developer ID Application"* ]]; then DEVELOPER_ID=1; fi

echo "==> codesign identity=$IDENTITY developer_id=$DEVELOPER_ID"
if [[ "$IDENTITY" == "-" ]]; then
  # Ad-hoc: Apple silicon will not run an unsigned arm64 binary at all, and a
  # stable identifier keeps the in-app updater's bundle check meaningful.
  codesign --force --deep --sign - --identifier "$BUNDLE_ID" "$APP"
else
  # Inside out: the executable, then the bundle (which seals Info.plist and
  # Resources). Hardened runtime + timestamp are what notarization requires.
  # entitlements.plist is required: without its audio-input key voice records
  # silence and nothing else fails, so a build that lost it must not pass.
  # WKWebView under the hardened runtime needs no entitlement for a page
  # served from the app itself.
  ENTITLEMENTS="$HERE/entitlements.plist"
  [[ -f "$ENTITLEMENTS" ]] || { echo "error: no $ENTITLEMENTS" >&2; exit 1; }
  args=(--force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --timestamp --options runtime --entitlements "$ENTITLEMENTS")
  codesign "${args[@]}" "$APP/Contents/MacOS/irori"
  codesign "${args[@]}" "$APP"
fi
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv --verbose=2 "$APP" 2>&1 | sed -n '1,12p' || true

if [[ "$DEVELOPER_ID" == 1 && "${IRORI_NOTARIZE:-1}" != 0 ]]; then
  bash "$HERE/notarize.sh" "$APP"
fi

if [[ -n "$TEST_HOME" && -z "$ZIP" ]]; then
  echo "==> test bundle: $APP (stop it by its pid; never quit Irori by name)"
  exit 0
fi

echo "==> $ZIP"
rm -f "$OUT/$ZIP"
# ditto keeps the bundle's symlinks, modes and signature, and --keepParent
# puts exactly one Irori.app at the top of the zip; the updater unpacks it
# with ditto too.
(cd "$OUT" && ditto -c -k --keepParent "Irori.app" "$ZIP")

if [[ -n "$TEST_ID" ]]; then
  echo "==> skipping the updater's bundle checks: the updater refuses test id $BUNDLE_ID"
else
  echo "==> Go updater bundle checks on $ZIP"
  bash "$HERE/check-bundle.sh" "$OUT/$ZIP" "$VERSION"
fi

if [[ -z "$DMG" ]]; then
  ls -la "$OUT"
  exit 0
fi
echo "==> $DMG"
STAGE="$OUT/dmg-stage"
rm -rf "$STAGE" "$OUT/$DMG"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Irori.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Irori" -srcfolder "$STAGE" -ov -format UDZO "$OUT/$DMG" >/dev/null
rm -rf "$STAGE"
if [[ "$DEVELOPER_ID" == 1 ]]; then
  codesign --force --sign "$IDENTITY" --timestamp "$OUT/$DMG"
  if [[ "${IRORI_NOTARIZE:-1}" != 0 ]]; then
    bash "$HERE/notarize.sh" "$OUT/$DMG"
  fi
fi

ls -la "$OUT"
