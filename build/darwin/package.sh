#!/usr/bin/env bash
# Package a built irori binary as Irori.app, sign it, and write the release's
# .app.zip (the in-app updater's payload) and .dmg (drag to Applications).
#
#   build/darwin/package.sh --binary bin/irori --version 0.4.0 \
#     --out dist-release --zip irori-0.4.0-macos-arm64.app.zip \
#     --dmg irori-0.4.0-macos-arm64.dmg
#
# Signing follows the environment (release.yml passes the repo secrets):
#   MACOS_CODESIGN_IDENTITY  empty or "-": ad-hoc signature, no notarization.
#                            "Developer ID Application: ...": hardened
#                            runtime + secure timestamp, then notarization
#                            when credentials are set (see notarize.sh).
#   IRORI_NOTARIZE=0         skip notarization even with Developer ID.
#
# This is build/darwin/Taskfile.yml's create:app:bundle done by hand (the
# Taskfile runs bun install and go mod tidy in parallel, which CI avoids),
# plus the signing, notarization, zip and dmg steps ported from suzuri's
# packaging/macos/build-app.sh.
set -euo pipefail

BINARY="" VERSION="" OUT="" ZIP="" DMG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BINARY="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --zip) ZIP="$2"; shift 2 ;;
    --dmg) DMG="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
for v in BINARY VERSION OUT ZIP DMG; do
  if [[ -z "${!v}" ]]; then echo "error: --$(echo "$v" | tr 'A-Z' 'a-z') is required" >&2; exit 2; fi
done
[[ -f "$BINARY" ]] || { echo "error: no binary at $BINARY" >&2; exit 1; }

HERE="$(cd "$(dirname "$0")" && pwd)"
BUNDLE_ID="com.stephenshorton.irori"
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
plutil -lint "$APP/Contents/Info.plist"
cp "$HERE/icons.icns" "$APP/Contents/Resources/"
if [[ -f "$HERE/Assets.car" ]]; then cp "$HERE/Assets.car" "$APP/Contents/Resources/"; fi

IDENTITY="${MACOS_CODESIGN_IDENTITY:-}"
[[ -n "$IDENTITY" ]] || IDENTITY="-"
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
  args=(--force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --timestamp --options runtime)
  if [[ -f "$HERE/entitlements.plist" ]]; then args+=(--entitlements "$HERE/entitlements.plist"); fi
  codesign "${args[@]}" "$APP/Contents/MacOS/irori"
  codesign "${args[@]}" "$APP"
fi
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv --verbose=2 "$APP" 2>&1 | sed -n '1,12p' || true

if [[ "$DEVELOPER_ID" == 1 && "${IRORI_NOTARIZE:-1}" != 0 ]]; then
  bash "$HERE/notarize.sh" "$APP"
fi

echo "==> $ZIP"
rm -f "$OUT/$ZIP"
# ditto keeps the bundle's symlinks, modes and signature; the updater
# unpacks it with ditto too.
(cd "$OUT" && ditto -c -k --keepParent "Irori.app" "$ZIP")

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
