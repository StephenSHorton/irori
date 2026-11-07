#!/usr/bin/env sh
# The Mac's Screen Recording grant, ad hoc vs Developer ID (#657).
#
# Wraps the share-dev example in an app bundle of its own, so TCC sees that
# bundle and its signature, not the terminal that started it, and runs it
# with `open` (launchd starts it, so it is its own responsible process).
#
#   sh crates/share/grant-test.sh build adhoc|devid|picker [version]
#   sh crates/share/grant-test.sh run   adhoc|devid|picker [share-dev args...]
#   sh crates/share/grant-test.sh info  adhoc|devid|picker
#   sh crates/share/grant-test.sh walk   (the whole test, for the person)
#
# adhoc and devid bundle share-dev; picker bundles the share-picker example
# (the SCContentSharingPicker spike), signed ad hoc, under its own bundle id
# so a grant to the others can't help it.
#
# Each kind has its own bundle id, never Irori's own
# (com.stephenshorton.irori), so nothing here touches the live app's grant.
# devid signs like release-mac.sh: Developer ID, hardened runtime, timestamp.
# Building again with another version changes the signature's cdhash, as an
# update does: an ad hoc grant is keyed to the cdhash, a Developer ID one to
# the Team ID (docs/releasing.md, Signing).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/../.."

cmd=${1:?build, run, info or walk}
if [ "$cmd" = walk ]; then
  exec sh "$here/grant-walk.sh"
fi
kind=${2:?adhoc, devid or picker}
shift 2
case "$kind" in
  adhoc) name=ShareDevAdhoc example=share-dev ;;
  devid) name=ShareDevDevID example=share-dev ;;
  picker) name=SharePicker example=share-picker ;;
  *) echo "kind is adhoc, devid or picker" >&2; exit 2 ;;
esac
target=${CARGO_TARGET_DIR:-target}
dir="$target/share-grant"
app="$dir/$name.app"
id="com.stephenshorton.irori.sharedev-$kind"

case "$cmd" in
build)
  version=${1:-1}
  cargo build --release -p irori-share --example "$example"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  cp "$target/release/examples/$example" "$app/Contents/MacOS/$example"
  cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleExecutable</key><string>$example</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSUIElement</key><true/>
</dict>
</plist>
EOF
  if [ "$kind" = devid ]; then
    ident=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)
    [ -n "$ident" ] || { echo "no Developer ID Application identity" >&2; exit 1; }
    codesign --force --options runtime --timestamp -s "$ident" "$app"
  else
    codesign --force -s - "$app"
  fi
  codesign -dvvv "$app" 2>&1 | grep -E "^Identifier=|^Authority=|^Signature|^CDHash=|^TeamIdentifier="
  ;;
run)
  log="$dir/$name.log"
  : >"$log"
  open -n -W --stdout "$log" --stderr "$log" "$app" --args "$@"
  cat "$log"
  ;;
info)
  codesign -dvvv "$app" 2>&1 | grep -E "^Identifier=|^Authority=|^Signature|^CDHash=|^TeamIdentifier="
  codesign -d -r- "$app" 2>&1 | grep designated
  ;;
*) echo "build, run, info or walk" >&2; exit 2 ;;
esac
