#!/usr/bin/env bash
# Builds the macOS release assets locally (Apple silicon) and, with
# --publish, adds them to the draft release scripts/release.ps1 made on the
# PC (architecture.md 7.2; there is no CI). The local twin of the Go build's
# release.yml macos job.
#
#   scripts/release-mac.sh 0.6.0                # build, package, check, smoke (dry run)
#   scripts/release-mac.sh 0.6.0 --publish      # + upload to the draft, extend SHA256SUMS
#   scripts/release-mac.sh 0.6.0 --finalize     # publish the draft
#
# Signing: without MACOS_CODESIGN_IDENTITY the app is signed ad hoc, as the
# Go releases are. With a Developer ID identity in the login keychain set
# MACOS_CODESIGN_IDENTITY="Developer ID Application: ... (TEAMID)", and for
# notarization either APPLE_API_KEY_ID + APPLE_API_ISSUER_ID +
# APPLE_API_KEY_P8 (path) or APPLE_ID + APPLE_TEAM_ID +
# APPLE_APP_SPECIFIC_PASSWORD (app/macos/notarize.sh). Once a release is
# Developer ID signed, keep signing: the updater refuses an update that is
# not.
#
# The asset names are a contract with crates/update and the Go updater
# (crates/update/tests/release_names.rs reads them below).
set -euo pipefail

VERSION="${1:-}"
MODE="${2:-}"
REPO="${IRORI_RELEASE_REPO:-StephenSHorton/irori}"
v="${VERSION#v}"
if ! [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then echo "usage: $0 X.Y.Z [--publish|--finalize]" >&2; exit 2; fi
tag="v$v"
cd "$(dirname "$0")/.."
ROOT="$(pwd)"

appzip="irori-$v-macos-arm64.app.zip"
dmg="irori-$v-macos-arm64.dmg"
# The PC's two, as scripts/release.ps1 names them (checked at --finalize).
win_assets=("irori-$v-windows-amd64-installer.exe" "irori-$v-windows-amd64.zip")

if [[ "$MODE" == "--finalize" ]]; then
  # All five assets, and a SHA256SUMS line for each of the four, before
  # anything installed can see the release: a Go v0.5.x Mac finding no
  # .app.zip line fails with "release vX has no macOS arm64 app asset".
  names="$(gh release view "$tag" --repo "$REPO" --json assets --jq '.assets[].name')"
  sums="$(gh release download "$tag" --repo "$REPO" --pattern SHA256SUMS --output -)"
  for n in "$appzip" "$dmg" "${win_assets[@]}" SHA256SUMS; do
    if ! grep -qxF -- "$n" <<<"$names"; then
      echo "draft $tag has no $n; not publishing" >&2
      exit 1
    fi
  done
  for n in "$appzip" "$dmg" "${win_assets[@]}"; do
    if ! awk -v n="$n" '$2 == n && length($1) == 64 { found = 1 } END { exit !found }' <<<"$sums"; then
      echo "SHA256SUMS on $tag has no line for $n; not publishing" >&2
      exit 1
    fi
  done
  gh release edit "$tag" --repo "$REPO" --draft=false
  echo "release $tag is published"
  exit 0
fi

# The assets go up against the tag, so they must be built from it: a clean
# tree at the tag's commit (Go's CI built the tag itself). A dry run only
# warns.
problem=""
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  problem="the tree has uncommitted changes"
elif ! tag_commit="$(git rev-parse --verify --quiet "$tag^{commit}")"; then
  problem="there is no tag $tag here"
elif [[ "$tag_commit" != "$(git rev-parse HEAD)" ]]; then
  problem="HEAD is not $tag ($tag_commit)"
fi
if [[ -n "$problem" ]]; then
  if [[ "$MODE" == "--publish" ]]; then
    echo "$problem; check out $tag in a clean tree to publish" >&2
    exit 1
  fi
  echo "warning: $problem (fine for a dry run)" >&2
fi

DIST="$ROOT/dist/release/$tag-mac"
mkdir -p "$DIST"

if [[ "${IRORI_SKIP_CHECKS:-0}" != 1 ]]; then sh scripts/check.sh; fi

(cd frontend && bun install --frozen-lockfile && bun run build)

# The deployment target matches Info.plist's LSMinimumSystemVersion.
# tauri/custom-protocol is what `tauri build` adds: without it the binary is
# compiled with cfg(dev) (tauri::is_dev() true, the build machine's config
# path embedded).
export MACOSX_DEPLOYMENT_TARGET=12.0
IRORI_VERSION="$v" TAURI_CONFIG="{\"version\":\"${v%%[-+]*}\"}" \
  cargo build --release --locked -p irori-app --target aarch64-apple-darwin --features tauri/custom-protocol
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/aarch64-apple-darwin/release/irori"
file "$BIN"

if [[ -z "${APPLE_ID:-}" && -z "${APPLE_API_KEY_ID:-}" ]]; then export IRORI_NOTARIZE=0; fi
bash app/macos/package.sh --binary "$BIN" --version "$v" --out "$DIST" --zip "$appzip" --dmg "$dmg"

# Smoke (Go release.yml): the bundled binary reports the version.
exe="$DIST/Irori.app/Contents/MacOS/irori"
out="$("$exe" version)"
echo "$out"
[ "$out" = "irori $v" ]

# The GUI smoke (the desktop app from the bundle on a throwaway IRORI_HOME,
# still running 20 s later) is opt-in. Go ran it on a CI runner; here it
# runs a full Core on this Mac, so each run leaves a Keychain item in the
# login keychain (service "Irori", account named after the temp dir) and
# goes to the real network with a fresh identity. IRORI_SMOKE_GUI=1 runs it.
if [[ "${IRORI_SMOKE_GUI:-0}" == 1 ]]; then
  SMOKE_HOME="$(mktemp -d "${TMPDIR:-/tmp}/irori-smoke.XXXXXX")"
  IRORI_HOME="$SMOKE_HOME" "$exe" >"$SMOKE_HOME.log" 2>&1 &
  pid=$!
  sleep 20
  if ! kill -0 "$pid"; then
    echo "the desktop app exited early:"
    cat "$SMOKE_HOME.log"
    exit 1
  fi
  echo "the desktop app is still running after 20 s"
  kill "$pid"
  wait "$pid" 2>/dev/null || true
  rm -rf "$SMOKE_HOME" "$SMOKE_HOME.log"
  echo "remove its Keychain item: Keychain Access, service Irori, the newest item"
else
  echo "GUI smoke skipped (IRORI_SMOKE_GUI=1 runs it; it leaves a Keychain item)"
fi

if [[ "$MODE" != "--publish" ]]; then
  echo "dry run: assets in $DIST"
  exit 0
fi

# Add the Mac lines to the draft's SHA256SUMS (sha256sum format) and upload.
rm -f "$DIST/SHA256SUMS"
gh release download "$tag" --repo "$REPO" --pattern SHA256SUMS --dir "$DIST"
grep -v -e "  $appzip\$" -e "  $dmg\$" "$DIST/SHA256SUMS" >"$DIST/SHA256SUMS.new" || true
(cd "$DIST" && shasum -a 256 "$appzip" "$dmg") >>"$DIST/SHA256SUMS.new"
mv "$DIST/SHA256SUMS.new" "$DIST/SHA256SUMS"
cat "$DIST/SHA256SUMS"
gh release upload "$tag" --repo "$REPO" --clobber "$DIST/$appzip" "$DIST/$dmg" "$DIST/SHA256SUMS"
echo "Mac assets added to the draft $tag; publish it with --finalize"
