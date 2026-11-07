#!/bin/sh
# Builds Irori's Rune phone prototype (card #680; not shipped: main's Tauri
# phone app stays) into a debug-signed APK: cargo builds app/ as
# libirori_app.so for arm64, and Gradle (app/android) wraps it with Rune's
# Kotlin host (RuneActivity, for the on-screen keyboard).
#
#   sh scripts/android-rune.sh [--install]
#
# The APK lands in target/android/irori-rune.apk. --install also installs
# and starts it with adb (set ANDROID_SERIAL to pick a device). It builds
# release: on an emulator's software GPU a debug Rune build hadn't drawn its
# first frame after a minute (Rune's scripts/android-apk.sh).
#
# Needs (docs/android.md): rustup target aarch64-linux-android; ANDROID_HOME
# with platforms;android-35, build-tools;35.0.0 and an NDK; JAVA_HOME (JDK
# 17); cmake and libclang for whisper.cpp and opus.
set -eu

install=0
for arg in "$@"; do
  case "$arg" in
    --install) install=1 ;;
    *) echo "unknown option $arg" >&2; exit 2 ;;
  esac
done

root=$(cd "$(dirname "$0")/.." && pwd)
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
: "${JAVA_HOME:?set JAVA_HOME to a JDK 17}"
if [ -z "${NDK_HOME:-}" ]; then
  NDK_HOME=$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)
fi
[ -d "${NDK_HOME:-}" ] || { echo "no NDK: set NDK_HOME or install one under \$ANDROID_HOME/ndk" >&2; exit 1; }
export NDK_HOME
case "$(uname -s)" in
  Darwin) host=darwin-x86_64 ;;
  Linux) host=linux-x86_64 ;;
  *) host=windows-x86_64 ;;
esac
llvm=$NDK_HOME/toolchains/llvm/prebuilt/$host/bin
api=26

# The env check.sh's Android step uses (docs/android.md, Voice input).
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$llvm/aarch64-linux-android$api-clang"
export CC_aarch64_linux_android="$llvm/aarch64-linux-android$api-clang"
export CXX_aarch64_linux_android="$llvm/aarch64-linux-android$api-clang++"
export AR_aarch64_linux_android="$llvm/llvm-ar"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$llvm/../sysroot --target=aarch64-linux-android$api"
export CMAKE_TOOLCHAIN_FILE_aarch64_linux_android="$root/scripts/android/arm64.toolchain.cmake"
export ANDROID_NDK_HOME="$NDK_HOME" ANDROID_PLATFORM=android-$api
if [ -z "${LIBCLANG_PATH:-}" ] && [ -d /Library/Developer/CommandLineTools/usr/lib ]; then
  export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib
fi

cd "$root"
target=aarch64-linux-android
# A cdylib only here: app/'s library is an rlib for the desktop builds.
cargo rustc -p irori-app --lib --release --target "$target" --crate-type cdylib

meta=$(cargo metadata --format-version 1)
target_dir=$(printf '%s' "$meta" | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
# Rune's Kotlin host, from cargo's checkout of the rev app/Cargo.toml pins.
rune_manifest=$(printf '%s' "$meta" | tr '{' '\n' | sed -n 's/.*"manifest_path":"\([^"]*crates\/rune\/Cargo.toml\)".*/\1/p' | head -1)
[ -n "$rune_manifest" ] || { echo "rune isn't in cargo metadata" >&2; exit 1; }
kotlin=$(dirname "$rune_manifest")/android/kotlin
[ -d "$kotlin" ] || { echo "no Kotlin host at $kotlin (a Rune rev without Android?)" >&2; exit 1; }

so=$target_dir/$target/release/libirori_app.so
out=$target_dir/android
stage=$out/irori-rune
rm -rf "$stage" && mkdir -p "$stage/lib/arm64-v8a"
cp "$so" "$stage/lib/arm64-v8a/"
"$llvm/llvm-strip" --strip-debug "$stage/lib/arm64-v8a/libirori_app.so"
# whisper.cpp and opus link the C++ runtime dynamically.
cp "$llvm/../sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" "$stage/lib/arm64-v8a/"

(cd app/android && ./gradlew -q assembleDebug "-Prune.jniLibs=$stage/lib" "-Prune.kotlin=$kotlin")
apk=$out/irori-rune.apk
cp app/android/build/outputs/apk/debug/*-debug.apk "$apk"
echo "$apk"
ls -l "$apk" | awk '{print "size: " $5 " bytes"}'

if [ "$install" = 1 ]; then
  adb=$ANDROID_HOME/platform-tools/adb
  "$adb" install -r "$apk"
  "$adb" shell am start -n com.stephenshorton.irori.rune/dev.rune.RuneActivity
fi
