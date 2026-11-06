#!/usr/bin/env bash
# scratch-mac runs a dev Irori beside the live one on a Mac, on a copy of
# its data dir, and stops it alone (architecture.md 3.2, card #611).
#
#   scripts/scratch-mac.sh start [--exe <irori>] [--from <data dir>] [--fresh]
#                                [--offset N] [--online] [--env K=V]...
#   scripts/scratch-mac.sh stop <scratch dir> [--rm]
#
# start copies the live data dir (default ~/.irori) into a new scratch dir:
# every .db with sqlite3 .backup (they are WAL; a cp can tear them), plus
# device.key, device.json, person.json, settings.json, names.json,
# agent-names.json and spaces/. It never copies irori.json (the live app's
# lockfile) or irori.log. --fresh starts on an empty data dir instead.
# The exe (default: the main checkout's target/debug/irori; build it with
# `cargo build -p irori-app`) runs with
#   IRORI_HOME=<scratch>/home  CLAUDE_CONFIG_DIR=<scratch>/claude (empty)
#   IRORI_DEV_PORT_OFFSET=N    IRORI_DEV_KEK_DIR=<from> (a copy only)
#   IRORI_DEV_OFFLINE=1        on a copy, unless --online
# and each --env K=V. A copy holds this device's real key: online it would
# join the mesh as this device and fork its history (core offline.rs), so
# --online is for a --fresh dir. --fresh with a debug build makes a new
# device and an "Irori device key" item in the login Keychain for the
# scratch dir; remove it in Keychain Access after. It prints the scratch dir, `pid=<pid>` and the stop
# command. stop kills only that pid (after checking it is still this exe),
# then says whether the live app's irori.log got a line from the test.
#
# Never quit Irori by name or bundle id (osascript, pkill irori): every
# bundle is com.stephenshorton.irori, so that quits the live app.
set -euo pipefail

die() { echo "scratch-mac: $*" >&2; exit 1; }

repo_root() {
  # The main checkout, also from a worktree: worktrees have no target/.
  local common
  common="$(git -C "$(dirname "$0")" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" ||
    die "not in a git checkout; pass --exe"
  dirname "$common"
}

# The live log's line count, or 0.
lines() { if [[ -f "$1" ]]; then wc -l <"$1" | tr -d ' '; else echo 0; fi; }

backup_db() {
  sqlite3 "$1" ".timeout 10000" ".backup '$2'" || die "sqlite3 .backup $1 failed"
}

cmd_start() {
  local exe="" from="$HOME/.irori" fresh=0 offset="" online=0
  local -a extra=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --exe) exe="$2"; shift 2 ;;
      --from) from="$2"; shift 2 ;;
      --fresh) fresh=1; shift ;;
      --offset) offset="$2"; shift 2 ;;
      --online) online=1; shift ;;
      --offline) online=0; shift ;;
      --env) [[ "$2" == *=* ]] || die "--env wants K=V, got $2"; extra+=("$2"); shift 2 ;;
      *) die "unknown argument: $1" ;;
    esac
  done
  [[ -n "$exe" ]] || exe="$(repo_root)/target/debug/irori"
  [[ -x "$exe" ]] || die "no exe at $exe (cargo build -p irori-app)"
  exe="$(cd "$(dirname "$exe")" && pwd)/$(basename "$exe")"
  # A port offset moves every port the dev build binds; random by default
  # so two scratches (two agents) don't collide. It must be a number.
  [[ -n "$offset" ]] || offset=$((1000 + RANDOM % 20000))
  [[ "$offset" =~ ^[0-9]+$ ]] || die "--offset must be a number"
  from="$(cd "$from" 2>/dev/null && pwd -P)" || die "no data dir to copy at $from"

  local base="${TMPDIR:-/tmp}"
  local dir
  dir="$(mktemp -d "${base%/}/irori-scratch.XXXXXX")"
  dir="$(cd "$dir" && pwd -P)"
  local home="$dir/home" claude="$dir/claude"
  mkdir -p "$home" "$claude"
  [[ "$home" != "$from" ]] || die "scratch home is the live data dir"

  if [[ $fresh == 0 ]]; then
    command -v sqlite3 >/dev/null || die "sqlite3 is not on PATH"
    echo "scratch-mac: copying $from"
    [[ -f "$from/irori.db" ]] || die "no irori.db in $from"
    backup_db "$from/irori.db" "$home/irori.db"
    local f
    for f in device.key device.json person.json settings.json names.json agent-names.json; do
      if [[ -f "$from/$f" ]]; then cp -p "$from/$f" "$home/$f"; fi
    done
    if [[ -d "$from/spaces" ]]; then
      mkdir -p "$home/spaces"
      # Per-board stores: .db files by backup, anything else as is; never
      # their -wal/-shm, which .backup has folded in.
      local p rel
      while IFS= read -r -d '' p; do
        rel="${p#"$from/spaces/"}"
        mkdir -p "$home/spaces/$(dirname "$rel")"
        case "$p" in
          *.db) backup_db "$p" "$home/spaces/$rel" ;;
          *.db-wal | *.db-shm) ;;
          *) cp -p "$p" "$home/spaces/$rel" ;;
        esac
      done < <(find "$from/spaces" -type f -print0)
    fi
  fi

  local live_log="$from/irori.log"
  [[ $fresh == 0 ]] || live_log="$HOME/.irori/irori.log"
  {
    echo "exe=$exe"
    echo "home=$home"
    echo "from=$([[ $fresh == 1 ]] && echo "$HOME/.irori" || echo "$from")"
    echo "live_log=$live_log"
    echo "live_lines=$(lines "$live_log")"
  } >"$dir/scratch.env"

  local -a envs=(
    "IRORI_HOME=$home"
    "CLAUDE_CONFIG_DIR=$claude"
    "IRORI_DEV_PORT_OFFSET=$offset"
  )
  # The copy's keys are sealed under the Keychain item named after the
  # data dir they came from (identity keystore.rs, debug builds only).
  [[ $fresh == 1 ]] || envs+=("IRORI_DEV_KEK_DIR=$from")
  [[ $online == 1 ]] || envs+=("IRORI_DEV_OFFLINE=1")
  if [[ $online == 1 && $fresh == 0 ]]; then
    echo "scratch-mac: warning: --online on a copy of $from joins the mesh as this device" >&2
  fi
  envs+=("${extra[@]+"${extra[@]}"}")
  printf '%s\n' "${envs[@]}" >"$dir/env"

  # env -i would drop PATH and HOME, which the app needs (shell PATH,
  # Keychain); start from this env and override.
  nohup env "${envs[@]}" "$exe" >"$dir/stdout.log" 2>&1 </dev/null &
  local pid=$!
  disown "$pid" 2>/dev/null || true
  echo "$pid" >"$dir/pid"

  # Wait for the scratch's own log: proof it runs on the copy.
  local n=0
  while [[ ! -s "$home/irori.log" ]] && kill -0 "$pid" 2>/dev/null && [[ $n -lt 300 ]]; do
    sleep 0.2; n=$((n + 1))
  done
  kill -0 "$pid" 2>/dev/null || { cat "$dir/stdout.log" >&2; die "irori exited at start (log: $dir/stdout.log)"; }
  echo "scratch=$dir"
  echo "home=$home"
  echo "offset=$offset"
  echo "pid=$pid"
  if [[ -s "$home/irori.log" ]]; then echo "log=$home/irori.log"; else echo "log=(none yet) $home/irori.log"; fi
  echo "stop: $0 stop $dir"
}

cmd_stop() {
  local dir="${1:-}" rm=0
  [[ "${2:-}" != "--rm" ]] || rm=1
  [[ -n "$dir" && -f "$dir/pid" && -f "$dir/scratch.env" ]] || die "usage: $0 stop <scratch dir> (the scratch= line start printed)"
  local pid exe live_log live_lines
  pid="$(cat "$dir/pid")"
  exe="$(sed -n 's/^exe=//p' "$dir/scratch.env")"
  live_log="$(sed -n 's/^live_log=//p' "$dir/scratch.env")"
  live_lines="$(sed -n 's/^live_lines=//p' "$dir/scratch.env")"
  [[ "$pid" =~ ^[0-9]+$ ]] || die "bad pid file in $dir"

  if kill -0 "$pid" 2>/dev/null; then
    # A pid can be reused once its process is gone: kill it only while it
    # is still this exe.
    local running
    running="$(ps -o comm= -p "$pid" 2>/dev/null || true)"
    if [[ "$running" != "$exe" ]]; then
      die "pid $pid is now \"$running\", not $exe; not killing it"
    fi
    # The other half of the done check: the test holds nothing open in
    # the live data dir (its log, its db).
    local from_dir held
    from_dir="$(sed -n 's/^from=//p' "$dir/scratch.env")"
    if [[ -n "$from_dir" ]]; then
      held="$(lsof -p "$pid" -Fn 2>/dev/null | sed -n 's/^n//p' | grep -F "$from_dir/" || true)"
      if [[ -n "$held" ]]; then
        echo "live_files=OPEN by the test:" >&2; echo "$held" >&2; LIVE_HELD=1
      else
        echo "live_files=none open"
      fi
    fi
    kill -TERM "$pid"
    local n=0
    while kill -0 "$pid" 2>/dev/null && [[ $n -lt 50 ]]; do sleep 0.2; n=$((n + 1)); done
    if kill -0 "$pid" 2>/dev/null; then
      echo "scratch-mac: $pid still running after 10s; SIGKILL"
      kill -KILL "$pid" 2>/dev/null || true
    fi
  fi
  if kill -0 "$pid" 2>/dev/null; then echo "exited=false"; else echo "exited=true"; fi

  # The done check: nothing from the test in the live log. The live app
  # keeps writing its own lines, so look for the scratch's paths and pid in
  # what was added since start.
  if [[ -f "$live_log" ]]; then
    local now new hits
    now="$(lines "$live_log")"
    new=$((now - live_lines))
    if [[ $new -lt 0 ]]; then
      echo "live_log=rotated since start; check $live_log.1 by hand"
    else
      # Its data path, or the start line a build writes ("pid=<pid> ").
      hits="$(tail -n "$new" "$live_log" | grep -cF -e "$dir" -e "pid=$pid " || true)"
      echo "live_log=$live_log gained $new lines, $hits from this scratch"
      [[ "$hits" == 0 ]] || { tail -n "$new" "$live_log" | grep -F -e "$dir" -e "pid=$pid " >&2; exit 3; }
    fi
  fi
  [[ "${LIVE_HELD:-0}" == 0 ]] || exit 3
  if [[ "$rm" == 1 ]] && ! kill -0 "$pid" 2>/dev/null; then
    rm -rf "$dir"
    echo "removed $dir"
  else
    echo "scratch dir kept: $dir (rm -rf it, or stop with --rm)"
  fi
}

case "${1:-}" in
  start) shift; cmd_start "$@" ;;
  stop) shift; cmd_stop "$@" ;;
  *) sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac
