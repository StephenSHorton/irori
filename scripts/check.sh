#!/usr/bin/env sh
# Local checks (architecture.md 7.1; there are no CI checks). Run from
# anywhere before every merge or push.
#
# The frontend builds first: tauri::generate_context! embeds frontend/dist,
# so no cargo step compiles the app crate without it.
set -eu
cd "$(dirname "$0")/.."

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) check_pc=1 ;;
  *) check_pc="" ;;
esac

# The PC: voice builds whisper.cpp, which needs cmake and libclang
# (docs/releasing.md, Build tools). Without them the build fails deep in
# whisper-rs-sys after minutes; stop now and say what to set (card #609).
# The Mac has both from Homebrew and Xcode.
if [ -n "$check_pc" ]; then
  check_missing=""
  if [ -n "${CMAKE:-}" ]; then
    [ -f "$CMAKE" ] || check_missing="$check_missing CMAKE (no file at $CMAKE)"
  elif ! command -v cmake >/dev/null 2>&1; then
    check_missing="$check_missing CMAKE"
  fi
  if [ -n "${LIBCLANG_PATH:-}" ]; then
    [ -f "$LIBCLANG_PATH" ] || [ -f "$LIBCLANG_PATH/libclang.dll" ] ||
      check_missing="$check_missing LIBCLANG_PATH (no libclang.dll in $LIBCLANG_PATH)"
  else
    check_dll=""
    check_ifs=$IFS
    IFS=:
    for d in $PATH; do
      if [ -f "$d/libclang.dll" ]; then check_dll=1; fi
    done
    IFS=$check_ifs
    [ -n "$check_dll" ] || check_missing="$check_missing LIBCLANG_PATH"
  fi
  if [ -n "$check_missing" ]; then
    cat >&2 <<EOF
check: voice builds whisper.cpp, which needs cmake and libclang. Not set:$check_missing
Set them first (docs/releasing.md, Build tools):
  python -m pip install --user cmake libclang
  export CMAKE="\$HOME/AppData/Roaming/Python/Python312/Scripts/cmake.exe"
  export LIBCLANG_PATH="\$(python -c "import clang,os;print(os.path.join(os.path.dirname(clang.__file__),'native'))")"
EOF
    exit 1
  fi
fi

# At most CHECK_SLOTS (default 2) runs build at once on this device; the
# rest wait their turn (card #437). Without Irori it does nothing.
. scripts/check-slot.sh
# temp_clean: a test run may leave nothing in its temp folder (card #556).
. scripts/check-temp.sh

# This run can be stopped alone, by scripts/check-stop.sh from this
# worktree (card #609): `pkill -f check.sh` would stop every agent's run.
# The state file is in this worktree's git dir, so each worktree has one.
check_state=$(git rev-parse --git-path check.pid 2>/dev/null || true)
if [ -n "$check_state" ]; then
  printf 'pid=%s\nwinpid=%s\n' "$$" "$(cat "/proc/$$/winpid" 2>/dev/null || true)" >"$check_state"
fi
echo "check: pid $$, stop with \`sh scripts/check-stop.sh\` (this worktree's run only)"
check_exit() {
  slot_release
  if [ -n "$check_state" ] && grep -qx "pid=$$" "$check_state" 2>/dev/null; then
    rm -f "$check_state"
  fi
}
trap 'check_exit' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
slot_take

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
sh scripts/one-test-binary.sh
if [ -n "$check_pc" ]; then
  # cargo fmt hands rustfmt every file of a crate with its full path in one
  # command line, and from a deep worktree (~/.claude/jobs/...) that passes
  # Windows' 32K limit: os error 206. Batches of 40 relative paths fit.
  git ls-files '*.rs' ':!vendor' | xargs -n 40 rustfmt --edition 2024 --check
else
  cargo fmt --all --check
fi
cargo clippy --workspace --all-targets -- -D warnings
# Every crate's tests run even when one fails, so one failure can't hide
# another (card #609); the run still fails at the end.
temp_clean cargo test --workspace --no-fail-fast
# frontend/src/gen/bindings.ts is what the window's commands generate (D10).
cargo xtask bindings --check
# Timing-sensitive, as in Go: capture's tests once more on one thread.
temp_clean cargo test -p irori-capture -- --test-threads=1
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
