#!/usr/bin/env bash
# rune-ui (card #674): the Tauri phone app is gone, so `cargo tauri android
# build` below has nothing to build until Rune runs on Android (#677). Kept
# for its signing steps and because crates/update tests read its asset name.
# Builds the signed Android release APK on the Mac and, with --publish, adds
# it to the draft release scripts/release.ps1 made on the PC (card #269;
# there is no CI). Run it after scripts/release-mac.sh, before --finalize.
# Run at the same time, the two wait for each other where they would clash:
# the frontend build and the draft's SHA256SUMS (scripts/release-lock.sh).
#
#   scripts/release-android.sh 0.11.0              # build, sign, verify (dry run)
#   scripts/release-android.sh 0.11.0 --publish    # + upload to the draft, extend SHA256SUMS
#   scripts/release-android.sh --new-key           # make the release key (once, ever)
#
# Signing: the release key is a keystore outside the repo
# (IRORI_ANDROID_KEYSTORE, default ~/.config/irori/android/irori-release.jks,
# alias "irori") whose password is in the login Keychain (service
# irori-android-release, account irori). Android only installs an update
# signed with the same key as the installed app: keep a backup of the
# keystore and its password, and never make a second key once a release is
# out (--new-key refuses when the keystore exists).
#
# Toolchain: docs/android.md. JAVA_HOME, ANDROID_HOME, NDK_HOME and the
# whisper.cpp cross-compile variables default to the Homebrew paths it
# installs.
#
# The asset name is a contract with crates/update (neither desktop updater
# may take it; crates/update/tests/it/release_names.rs reads it below).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
REPO="${IRORI_RELEASE_REPO:-StephenSHorton/irori}"
KEYSTORE="${IRORI_ANDROID_KEYSTORE:-$HOME/.config/irori/android/irori-release.jks}"
KEY_ALIAS=irori
KC_SERVICE=irori-android-release

export JAVA_HOME="${JAVA_HOME:-/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home}"
export ANDROID_HOME="${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}"
export NDK_HOME="${NDK_HOME:-$ANDROID_HOME/ndk/27.2.12479018}"
BUILD_TOOLS="$ANDROID_HOME/build-tools/35.0.0"
KEYTOOL="$JAVA_HOME/bin/keytool"

if [[ "${1:-}" == "--new-key" ]]; then
  if [[ -e "$KEYSTORE" ]]; then
    echo "$KEYSTORE exists; a second key would stop every installed phone from updating" >&2
    exit 1
  fi
  if security find-generic-password -s "$KC_SERVICE" -a irori >/dev/null 2>&1; then
    echo "the Keychain already has a $KC_SERVICE password but there is no keystore at $KEYSTORE;" >&2
    echo "restore the keystore from its backup instead of making a new key" >&2
    exit 1
  fi
  mkdir -p "$(dirname "$KEYSTORE")"
  chmod 700 "$(dirname "$KEYSTORE")"
  IRORI_KS_PASS="$(openssl rand -base64 33 | tr -d '/+=\n' | cut -c1-40)"
  export IRORI_KS_PASS
  "$KEYTOOL" -genkeypair -keystore "$KEYSTORE" -storetype PKCS12 -alias "$KEY_ALIAS" \
    -keyalg RSA -keysize 4096 -validity 36500 \
    -dname "CN=Stephen Horton, O=Irori" \
    -storepass:env IRORI_KS_PASS -keypass:env IRORI_KS_PASS
  chmod 600 "$KEYSTORE"
  # -T: security(1) itself may read it without a prompt (the build below).
  security add-generic-password -s "$KC_SERVICE" -a irori -w "$IRORI_KS_PASS" -T /usr/bin/security
  "$KEYTOOL" -list -v -keystore "$KEYSTORE" -alias "$KEY_ALIAS" -storepass:env IRORI_KS_PASS |
    grep -E 'SHA256:|Valid'
  unset IRORI_KS_PASS
  echo "release key made: $KEYSTORE (password: Keychain service $KC_SERVICE)."
  echo "Back up both now; losing either means no phone can update again."
  exit 0
fi

VERSION="${1:-}"
MODE="${2:-}"
v="${VERSION#v}"
if ! [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
  echo "usage: $0 X.Y.Z [--publish] | $0 --new-key" >&2
  exit 2
fi
tag="v$v"
apk="irori-$v-android-arm64.apk"

# Built from the tag, as release-mac.sh (a dry run only warns).
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

[[ -f "$KEYSTORE" ]] || { echo "no release keystore at $KEYSTORE (restore it from its backup; --new-key only for the very first release)" >&2; exit 1; }
IRORI_KS_PASS="$(security find-generic-password -s "$KC_SERVICE" -a irori -w)" ||
  { echo "no Keychain password for $KC_SERVICE" >&2; exit 1; }
export IRORI_KS_PASS

# Android needs a growing integer per release: Tauri's own formula.
IFS=. read -r major minor patch <<<"${v%%[-+]*}"
version_code=$((major * 1000000 + minor * 1000 + patch))

DIST="$ROOT/dist/release/$tag-android"
mkdir -p "$DIST"

# whisper.cpp for arm64 (docs/android.md, "Voice input").
P="$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64"
export CXX_aarch64_linux_android="$P/bin/aarch64-linux-android26-clang++"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$P/sysroot --target=aarch64-linux-android26"
export CMAKE_TOOLCHAIN_FILE_aarch64_linux_android="$ROOT/scripts/android/arm64.toolchain.cmake"

# release-mac.sh may be running too: frontend/dist is shared, so this
# tree's frontend lock is held until the app that embeds it is built.
. scripts/release-lock.sh
release_lock "$ROOT/dist/release/frontend.lock" "building with frontend/dist"

(cd frontend && bun install --frozen-lockfile && bun run build)

# app/gen/android is committed. The release build type is unsigned here; it
# is signed below with apksigner so no key or password reaches Gradle.
# The version goes in twice: TAURI_CONFIG for the compiled app (tauri-build)
# and --config for the CLI, which writes it into Gradle's tauri.properties
# (the CLI ignores TAURI_CONFIG; the check below caught versionCode 1).
version_config="{\"version\":\"${v%%[-+]*}\",\"bundle\":{\"android\":{\"versionCode\":$version_code}}}"
(cd app && IRORI_VERSION="$v" TAURI_CONFIG="$version_config" \
  cargo tauri android build --target aarch64 --apk --ci --config "$version_config")

out="$ROOT/app/gen/android/app/build/outputs/apk/universal/release"
unsigned="$(ls "$out"/*-release-unsigned.apk 2>/dev/null | head -1)"
[[ -n "$unsigned" ]] || { echo "no unsigned release APK in $out" >&2; exit 1; }
release_unlock "$ROOT/dist/release/frontend.lock"

rm -f "$DIST/$apk" "$DIST/aligned.apk"
"$BUILD_TOOLS/zipalign" -P 16 -f 4 "$unsigned" "$DIST/aligned.apk"
"$BUILD_TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "$KEY_ALIAS" \
  --ks-pass env:IRORI_KS_PASS --key-pass env:IRORI_KS_PASS \
  --out "$DIST/$apk" "$DIST/aligned.apk"
rm -f "$DIST/aligned.apk" "$DIST/$apk.idsig"
unset IRORI_KS_PASS

# Smoke: signed with the release key, the right version, 16 KB aligned.
"$BUILD_TOOLS/apksigner" verify --print-certs "$DIST/$apk" | grep -E 'Signer #1 certificate (DN|SHA-256)'
badging="$("$BUILD_TOOLS/aapt2" dump badging "$DIST/$apk" | sed -n 1p)"
echo "$badging"
grep -qF "versionCode='$version_code'" <<<"$badging" || { echo "versionCode is not $version_code" >&2; exit 1; }
grep -qF "versionName='${v%%[-+]*}'" <<<"$badging" || { echo "versionName is not ${v%%[-+]*}" >&2; exit 1; }
"$BUILD_TOOLS/zipalign" -c -P 16 4 "$DIST/$apk"

if [[ "$MODE" != "--publish" ]]; then
  echo "dry run: $DIST/$apk"
  exit 0
fi

# Download to upload holds the tag's SHA256SUMS lock (as release-mac.sh),
# so neither script's lines are lost to the other's upload.
sums_lock="$(git rev-parse --path-format=absolute --git-common-dir)/irori-release-$tag-SHA256SUMS.lock"
release_lock "$sums_lock" "updating SHA256SUMS on the draft $tag"
rm -f "$DIST/SHA256SUMS"
gh release download "$tag" --repo "$REPO" --pattern SHA256SUMS --dir "$DIST"
grep -v -e "  $apk\$" "$DIST/SHA256SUMS" >"$DIST/SHA256SUMS.new" || true
(cd "$DIST" && shasum -a 256 "$apk") >>"$DIST/SHA256SUMS.new"
mv "$DIST/SHA256SUMS.new" "$DIST/SHA256SUMS"
cat "$DIST/SHA256SUMS"
gh release upload "$tag" --repo "$REPO" --clobber "$DIST/$apk" "$DIST/SHA256SUMS"
release_unlock "$sums_lock"
echo "Android APK added to the draft $tag; publish it with --finalize"
