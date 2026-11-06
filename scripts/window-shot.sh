#!/usr/bin/env bash
# window-shot takes a picture of the real Irori window on a Mac, the twin of
# scripts/window-shot.ps1: it starts a debug build on a scratch copy of the
# data dir (scripts/scratch-mac.sh, offline), lets the app save its own
# window (app/src/shell/shot.rs: an agent session has no Screen Recording
# permission, but a process may capture its own windows), copies the last
# picture to --out and stops that pid alone.
#
#   scripts/window-shot.sh --out <png> [--query 'view=canvas&full']
#                          [--after 50] [--count 1] [--exe <irori>]
#                          [--from <data dir>] [--fresh] [--wait-sec 180]
#                          [--env K=V]...
#
# --query opens the window on index.html?dev&<query> (frontend
# src/lib/preview.ts); pass '' for the plain window. --after is seconds
# from window open to the first picture (boards take ~45 s to open on a
# copy). The exe must be a debug build (cargo build -p irori-app): a
# release build ignores the IRORI_DEV_WINDOW_* variables.
set -euo pipefail

die() { echo "window-shot: $*" >&2; exit 1; }

out="" query="view=canvas&full" after=50 count=1 wait_sec=180
start_args=(--offline)
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    --query) query="$2"; shift 2 ;;
    --after) after="$2"; shift 2 ;;
    --count) count="$2"; shift 2 ;;
    --wait-sec) wait_sec="$2"; shift 2 ;;
    --exe | --from | --env | --offset) start_args+=("$1" "$2"); shift 2 ;;
    --fresh) start_args+=("$1"); shift ;;
    *) die "unknown argument: $1" ;;
  esac
done
[[ -n "$out" ]] || die "--out <png> is required"
[[ "$count" =~ ^[1-9][0-9]*$ ]] || die "--count must be a positive number"
[[ "$after" =~ ^[0-9]+([.][0-9]+)?$ ]] || die "--after must be seconds"
[[ "$wait_sec" =~ ^[0-9]+$ ]] || die "--wait-sec must be seconds"
mkdir -p "$(dirname "$out")"
out="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")"

here="$(cd "$(dirname "$0")" && pwd)"
shots="$(mktemp -d "${TMPDIR:-/tmp}/irori-shots.XXXXXX")"
start_args+=(--env "IRORI_DEV_WINDOW_SHOTS=$shots" --env "IRORI_DEV_WINDOW_SHOT_AFTER=$after"
  --env "IRORI_DEV_WINDOW_SHOT_COUNT=$count")
[[ -z "$query" ]] || start_args+=(--env "IRORI_DEV_WINDOW_QUERY=$query")

started="$("$here/scratch-mac.sh" start "${start_args[@]}")" || { echo "$started" >&2; die "scratch start failed"; }
echo "$started"
scratch="$(sed -n 's/^scratch=//p' <<<"$started")"
pid="$(sed -n 's/^pid=//p' <<<"$started")"
home="$(sed -n 's/^home=//p' <<<"$started")"
[[ -n "$scratch" && -n "$pid" ]] || die "scratch start printed no scratch=/pid= line"

stop() { "$here/scratch-mac.sh" stop "$scratch" --rm || true; }
trap stop EXIT

# Pictures are written 1.5 s apart; wait for the last, or the deadline.
last="$shots/window-$count.png"
n=0
while [[ ! -s "$last" ]] && kill -0 "$pid" 2>/dev/null && [[ $n -lt $((wait_sec * 5)) ]]; do
  sleep 0.2; n=$((n + 1))
done
# Let ImageIO finish the file.
sleep 0.5
if [[ ! -s "$last" ]]; then
  grep -h "shot:" "$home/irori.log" >&2 2>/dev/null || true
  die "no $last after ${wait_sec}s (exited=$(kill -0 "$pid" 2>/dev/null && echo false || echo true))"
fi
cp "$last" "$out"
if [[ $count -gt 1 ]]; then
  for f in "$shots"/window-*.png; do cp "$f" "${out%.png}-$(basename "$f" .png | sed 's/^window-//').png"; done
fi
grep -h "shot: saved" "$home/irori.log" 2>/dev/null | tail -n "$count" || true
echo "saved=$out"
rm -rf "$shots"
