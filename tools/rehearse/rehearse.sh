#!/usr/bin/env sh
# The rehearsal on copies for the Mac (architecture.md 5.2, 8 M8; the PC's
# is rehearse.ps1).
#
#   sh tools/rehearse/rehearse.sh <scratch-dir> [data-dir] [go-repo] [app-zip]
#
# Legs, each on its own fresh copy of the data dir (default ~/.irori):
#   verify   tools/goldens/verify-live.sh
#   replay   tools/goldens/replay-live.sh
#   probe    irori-dev probe: `irori serve --headless` on a copy with
#            IRORI_DEV_OFFLINE=1 (no socket binds; the copy holds this
#            device's real key), every Go API route answers, `irori mcp`
#            lists Go's tools
#   update   the ported mac_darwin tests (cargo test -p irori-update --test
#            mac); given an irori-<v>-macos-arm64.app.zip,
#            app/macos/check-bundle.sh (the Go updater's bundle checks); and
#            Go v0.5.0 -> Rust v0.6.0 in a scratch copy of the installed
#            Irori.app (IRORI_REHEARSE_GOAPP, default /Applications/Irori.app,
#            only read). The client is Go's own internal/update code
#            (tools/rehearse/update_rehearse_mac_test.go via go test
#            -overlay) against a fake GitHub on 127.0.0.1 serving the Rust
#            .app.zip: a debug build with the embedded frontend, packaged by
#            app/macos/package.sh. Go's swap helper replaces the scratch
#            bundle (InstallOnExit: a relaunch goes through `open`, which
#            would drop the scratch environment), then the swapped app runs
#            under the shell probe: no window, tray, login item change or
#            auto-update, offline, on a copy of the data dir.
#
# IRORI_DEV_KEK_DIR is set to the original data dir, so the copies' keys
# unseal with the Keychain item named after it (docs/m1-mac.md 2.2). That
# is why the update leg packages a debug build: a release build ignores it.
# Everything under <scratch-dir>/irori-rehearse is deleted at the end.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
scratch="$1"
data="${2:-$HOME/.irori}"
goirori="${3:-$root/../irori-go}"
appzip="${4:-}"
goirori="$(cd "$goirori" && pwd)"
data="$(cd "$data" && pwd)"
work="$scratch/irori-rehearse"
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
export IRORI_DEV_KEK_DIR="$data"

failed=0
result() { # leg status
  if [ "$2" = 0 ]; then echo "== $1 passed"; else echo "== $1 FAILED"; failed=$((failed + 1)); fi
}

# copy_home <dir>: a copy of the data dir the Rust build can open. The
# database is copied until two copies in a row match (Go may be writing),
# then the identity and state files. Fails when it never settles.
copy_home() {
  same=0
  for try in 1 2 3 4 5; do
    for s in a b; do
      rm -rf "$1.db-$s" && mkdir -p "$1.db-$s"
      for f in irori.db irori.db-wal irori.db-shm; do
        if [ -f "$data/$f" ]; then cp "$data/$f" "$1.db-$s/$f"; fi
      done
    done
    if diff -r "$1.db-a" "$1.db-b" >/dev/null 2>&1; then same=1; break; fi
    sleep 1
  done
  if [ "$same" != 1 ]; then
    rm -rf "$1.db-a" "$1.db-b"
    echo "rehearse: the database kept changing while it was copied" >&2
    return 1
  fi
  mkdir -p "$1"
  for f in "$1".db-a/*; do cp "$f" "$1/"; done
  rm -rf "$1.db-a" "$1.db-b"
  for f in device.json device.key person.json turn.key mesh-peers.json hub-addrs.json \
    home-server.json relay.json names.json settings.json capture-cursors.json pair-rows.json; do
    if [ -f "$data/$f" ]; then cp "$data/$f" "$1/$f"; fi
  done
  if [ -d "$data/boards" ]; then cp -R "$data/boards" "$1/boards"; fi
}

# wait_for <seconds> <command...>: poll every half second until it succeeds.
wait_for() {
  n=$(($1 * 2))
  shift
  while [ "$n" -gt 0 ]; do
    if "$@"; then return 0; fi
    sleep 0.5
    n=$((n - 1))
  done
  return 1
}

(cd "$root/frontend" && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null) || exit 1
(cd "$root" && IRORI_VERSION=0.6.0 cargo build -q -p irori-app -p irori-dev) || exit 1
exe="$root/target/debug/irori"
[ "$("$exe" version)" = "irori 0.6.0" ] || { echo "rehearse: the built exe is not 0.6.0" >&2; exit 1; }

for leg in verify replay; do
  echo "== $leg"
  mkdir -p "$work/$leg"
  (cd "$root" && sh "tools/goldens/$leg-live.sh" "$data" "$work/$leg" "$goirori")
  result "$leg" $?
done

echo "== probe"
p="$work/probe"
mkdir -p "$p/claude"
if copy_home "$p/home"; then
  "$root/target/debug/irori-dev" probe --irori "$exe" --home "$p/home" --claude "$p/claude" \
    --go-tools "$goirori/internal/mcpserver/fallback_tools.json"
  result probe $?
else
  result probe 1
fi
rm -rf "$p"

echo "== update: mac_darwin tests"
(cd "$root" && cargo test -q -p irori-update --test mac)
result "update: mac_darwin tests" $?
if [ -n "$appzip" ]; then
  bash "$root/app/macos/check-bundle.sh" "$appzip" 0.6.0
  result "update: bundle checks" $?
fi

# --- update: Go v0.5.0 -> Rust v0.6.0 -----------------------------------------
echo "== update: Go v0.5.0 -> Rust v0.6.0"
u="$work/update"
live="$u/apps/Irori.app"
shell_log="$u/shell.log"
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
up_fails=0
check() { # ok description
  if [ "$1" = 0 ]; then echo "  ok   $2"; else echo "  FAIL $2"; up_fails=$((up_fails + 1)); fi
}
# Stop anything still running from the scratch bundle and drop the bundle
# from Launch Services (running it registers it under Irori's bundle id).
update_cleanup() {
  if [ -n "${u:-}" ] && [ -d "$u" ]; then
    pkill -f "$u/apps/" 2>/dev/null
    if [ -d "$live" ]; then "$lsregister" -u "$live" >/dev/null 2>&1; fi
  fi
}
trap 'update_cleanup; rm -rf "$work"' EXIT
mkdir -p "$u/apps" "$u/home" "$u/claude"
goapp="${IRORI_REHEARSE_GOAPP:-/Applications/Irori.app}"
gover="$(plutil -extract CFBundleShortVersionString raw -o - "$goapp/Contents/Info.plist" 2>/dev/null)"
ditto "$goapp" "$live"
check $? "copied $goapp (v$gover) to the scratch bundle"

# The Rust .app.zip: a debug build (IRORI_DEV_KEK_DIR works) with the
# frontend embedded, as `tauri build` and release-mac.sh build it.
zip="$u/dist/irori-0.6.0-macos-arm64.app.zip"
(cd "$root" && IRORI_VERSION=0.6.0 cargo build -q -p irori-app --features tauri/custom-protocol) &&
  IRORI_NOTARIZE=0 MACOS_CODESIGN_IDENTITY= bash "$root/app/macos/package.sh" \
    --binary "$root/target/debug/irori" --version 0.6.0 --out "$u/dist" \
    --zip "$(basename "$zip")" --dmg irori-0.6.0-macos-arm64.dmg >"$u/package.log" 2>&1
check $? "packaged the Rust v0.6.0 .app.zip (debug, embedded frontend; app/macos/package.sh)"

overlay="$u/overlay.json"
printf '{"Replace":{"%s":"%s"}}\n' "$goirori/internal/update/update_rehearse_mac_test.go" \
  "$here/update_rehearse_mac_test.go" >"$overlay"
(cd "$goirori" && IRORI_HOME="$u/home" IRORI_REHEARSE_APPZIP="$zip" IRORI_REHEARSE_LIVEAPP="$live" \
  IRORI_REHEARSE_PIDFILE="$u/go.pid" go test -count=1 -v -overlay "$overlay" -vet=off \
  -run '^TestRehearseGoUpdatesToRustMac$' ./internal/update) >"$u/go.log" 2>&1
go_ok=$?
grep -E '^(---|ok|FAIL|panic)|update_rehearse_mac_test' "$u/go.log" | sed 's/^/  /'
check $go_ok "the Go updater found, downloaded, verified and staged v0.6.0 and started the swap helper"

if [ "$go_ok" = 0 ]; then
  swapped() {
    [ "$(plutil -extract CFBundleShortVersionString raw -o - "$live/Contents/Info.plist" 2>/dev/null)" = 0.6.0 ] &&
      [ ! -e "$u/apps/.Irori.app.old" ] && [ ! -e "$u/apps/.Irori.app.new" ]
  }
  wait_for 60 swapped
  check $? "the swap helper waited for the Go process and replaced the scratch Irori.app (no .old or .new left)"
  sed 's/^/  update.log: /' "$u/home/updates/update.log" 2>/dev/null
  codesign --verify --deep --strict "$live" 2>/dev/null
  check $? "the swapped bundle's signature verifies"
  v="$("$live/Contents/MacOS/irori" version 2>&1)"
  [ "$v" = "irori 0.6.0" ]
  check $? "the swapped exe says 'irori 0.6.0' (got '$v')"

  # The swapped app under the shell probe, on a copy of the data dir.
  if copy_home "$u/data"; then
    IRORI_HOME="$u/data" CLAUDE_CONFIG_DIR="$u/claude" IRORI_DEV_OFFLINE=1 \
      IRORI_DEV_SHELL_LOG="$shell_log" "$live/Contents/MacOS/irori" >"$u/app.log" 2>&1 &
    app_pid=$!
    has() { [ -f "$shell_log" ] && grep -qx "$1" "$shell_log"; }
    wait_for 180 has ready
    check $? "the swapped Rust app came up (shell probe: no window, offline, a copy of the data dir)"
    has "version 0.6.0"
    check $? "it runs v0.6.0"
    page_load() { grep -q '^page-load ' "$shell_log" 2>/dev/null; }
    wait_for 60 page_load
    has "page-load tauri://localhost"
    check $? "the hidden window loaded the embedded frontend ($(grep -m1 '^page-load ' "$shell_log" 2>/dev/null))"
    port="$(sed -n 's/^api-port \([0-9][0-9]*\)$/\1/p' "$shell_log" | head -n 1)"
    health="$(curl -s --max-time 10 "http://127.0.0.1:${port:-0}/v1/health")"
    printf '%s' "$health" | grep -q '"ok":true' && printf '%s' "$health" | grep -q '"version":"0.6.0"'
    check $? "it serves: GET /v1/health on its api-port says ok, version 0.6.0"
    : >"$shell_log.quit"
    gone() { ! kill -0 "$app_pid" 2>/dev/null; }
    wait_for 60 gone
    check $? "it quit through the probe"
    has exiting
    check $? "the probe saw it exit cleanly"
    if ! gone; then kill "$app_pid" 2>/dev/null; fi
  else
    check 1 "copied the data dir for the swapped app"
  fi
fi
update_cleanup
[ "$up_fails" = 0 ]
result "update: Go v0.5.0 -> Rust v0.6.0" $?

if [ "$failed" != 0 ]; then echo "rehearse: $failed failed"; exit 1; fi
echo "rehearse: all passed"
