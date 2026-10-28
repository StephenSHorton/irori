#!/usr/bin/env bash
# Submit a Developer ID signed Irori.app or .dmg to Apple's notary service,
# wait for the verdict, and staple the ticket. Ported from suzuri's
# packaging/macos/notarize.sh.
#
# Credentials (first match wins):
#   1. App Store Connect API key: APPLE_API_KEY_ID, APPLE_API_ISSUER_ID and
#      APPLE_API_KEY_P8_BASE64 (or APPLE_API_KEY_P8, a path to the .p8)
#   2. Apple ID: APPLE_ID, APPLE_TEAM_ID, APPLE_APP_SPECIFIC_PASSWORD
#
#   build/darwin/notarize.sh dist-release/Irori.app
set -euo pipefail

TARGET="${1:-}"
if [[ -z "$TARGET" || ! -e "$TARGET" ]]; then
  echo "usage: $0 <signed .app or .dmg>" >&2
  exit 2
fi
TARGET="$(cd "$(dirname "$TARGET")" && pwd)/$(basename "$TARGET")"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/irori-notary.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# notarytool takes a file: zip an .app first.
SUBMIT="$TARGET"
if [[ -d "$TARGET" && "$TARGET" == *.app ]]; then
  SUBMIT="$WORK/submit.zip"
  ditto -c -k --keepParent "$TARGET" "$SUBMIT"
fi

AUTH=()
if [[ -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]; then
  KEY="${APPLE_API_KEY_P8:-}"
  if [[ -z "$KEY" && -n "${APPLE_API_KEY_P8_BASE64:-}" ]]; then
    KEY="$WORK/AuthKey.p8"
    echo "$APPLE_API_KEY_P8_BASE64" | base64 --decode >"$KEY"
  fi
  if [[ -z "$KEY" || ! -f "$KEY" ]]; then
    echo "error: set APPLE_API_KEY_P8_BASE64 (or APPLE_API_KEY_P8)" >&2
    exit 1
  fi
  AUTH=(--key "$KEY" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID")
  echo "==> notary auth: App Store Connect API key"
elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]]; then
  AUTH=(--apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD")
  echo "==> notary auth: Apple ID + app-specific password"
else
  echo "error: no notarization credentials (docs/releasing.md, macOS signing)" >&2
  exit 1
fi

echo "==> notarytool submit $(basename "$SUBMIT")"
# A first submission for a new certificate can sit in Apple's queue a while.
xcrun notarytool submit "$SUBMIT" "${AUTH[@]}" --wait --timeout "${IRORI_NOTARY_TIMEOUT:-90m}"

echo "==> staple $(basename "$TARGET")"
xcrun stapler staple "$TARGET"
xcrun stapler validate "$TARGET"
if [[ -d "$TARGET" ]]; then spctl -a -vv "$TARGET" 2>&1 || true; fi
