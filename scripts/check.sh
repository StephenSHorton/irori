#!/usr/bin/env sh
# Local checks (architecture.md 7.1; there are no CI checks). Run from
# anywhere before every merge or push.
#
# The frontend builds first: tauri::generate_context! embeds frontend/dist,
# so no cargo step compiles the app crate without it.
set -eu
cd "$(dirname "$0")/.."

# At most CHECK_SLOTS (default 2) runs build at once on this device; the
# rest wait their turn (card #437). Without Irori it does nothing.
. scripts/check-slot.sh

(
  cd frontend
  bun install --frozen-lockfile
  # src/gen/models.ts follows src/gen/bindings.ts (cargo xtask bindings).
  bun scripts/gen-models.ts --check
  bun run build
  bun run typecheck
  bun run lint
  bun test
)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# frontend/src/gen/bindings.ts is what the window's commands generate (D10).
cargo xtask bindings --check
# Timing-sensitive, as in Go: capture's tests once more on one thread.
cargo test -p irori-capture -- --test-threads=1
sh scripts/no-third-party.sh

# The phone build (docs/android.md): desktop-only calls must stay behind
# #[cfg(desktop)], which only an Android build catches (#220 broke it, #234).
# Runs when an NDK and the aarch64-linux-android target are installed;
# otherwise it is skipped so machines without them still pass.
ndk="${NDK_HOME:-}"
if [ -z "$ndk" ] || [ ! -d "$ndk" ]; then
  ndk=""
  for d in "${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}"/ndk/*; do   # ANDROID_HOME default: docs/android.md (Homebrew)
    [ -d "$d" ] && ndk="$d"   # glob order: the last one is the newest
  done
fi
android_bin=""
if [ -n "$ndk" ]; then
  for d in "$ndk"/toolchains/llvm/prebuilt/*/bin; do
    [ -d "$d" ] && android_bin="$d"
  done
fi
if [ -n "$android_bin" ] && [ -e "$android_bin/aarch64-linux-android26-clang" ] &&
  rustup target list --installed | grep -qx aarch64-linux-android; then
  echo "android: cargo check (NDK $ndk)"
  # API 26 is the app's minimum (AAudio). Voice builds whisper.cpp with
  # CMake and bindgen, which need the toolchain wrapper and the sysroot
  # (docs/android.md, #240).
  NDK_HOME="$ndk" \
    CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$android_bin/aarch64-linux-android26-clang" \
    CC_aarch64_linux_android="$android_bin/aarch64-linux-android26-clang" \
    CXX_aarch64_linux_android="$android_bin/aarch64-linux-android26-clang++" \
    AR_aarch64_linux_android="$android_bin/llvm-ar" \
    BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$android_bin/../sysroot --target=aarch64-linux-android26" \
    CMAKE_TOOLCHAIN_FILE_aarch64_linux_android="$PWD/scripts/android/arm64.toolchain.cmake" \
    cargo check --target aarch64-linux-android -p irori-app --lib
else
  echo "android: skipped (no NDK)"
fi

echo "check: all passed"
