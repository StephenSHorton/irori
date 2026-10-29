#!/usr/bin/env sh
# Screenshots of the browser preview (frontend/PREVIEW.md) with an installed
# Chrome or Edge, headless, into frontend/preview-shots/ (gitignored).
# Start `bun run preview:look` first. A throwaway profile keeps it off your
# own browser session.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
out="$here/preview-shots"
base="${PREVIEW_URL:-http://127.0.0.1:9246/}"
browser="${BROWSER_EXE:-}"
for c in "/c/Program Files/Google/Chrome/Application/chrome.exe" \
  "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
  "/c/Program Files/Microsoft/Edge/Application/msedge.exe"; do
  [ -z "$browser" ] && [ -x "$c" ] && browser="$c"
done
[ -n "$browser" ] || { echo "no Chrome or Edge found; set BROWSER_EXE" >&2; exit 1; }
profile=$(mktemp -d)
trap 'rm -rf "$profile"' EXIT
mkdir -p "$out"

shot() { # name query [width height [reduce]]
  "$browser" --headless=new --disable-gpu --hide-scrollbars --no-first-run \
    ${5:+--force-prefers-reduced-motion} \
    --user-data-dir="$profile" --window-size="${3:-1440},${4:-900}" \
    --virtual-time-budget=4000 --screenshot="$(cygpath -w "$out/$1.png" 2>/dev/null || echo "$out/$1.png")" \
    "$base$2" >/dev/null 2>&1
  echo "$out/$1.png"
}

shot 01-cards ""                                  # the grid; sample-card-4 spinning
shot 02-card-answers "?open=board:task:sample-card-2" # blocker with recommended answers
shot 03-card-done "?open=board:task:sample-card-5"  # done card, merged PR
shot 04-devices "?devices"                          # Devices dialog
shot 05-settings "?settings"                        # settings popover
shot 06-held-message "?open=inbox:sample-delivery-1" # a message Irori held
shot 07-update "?update=ready"                      # update card
shot 08-empty "?empty"                              # no cards yet
shot 09-narrow "" 760 1100                          # the grid, narrow
shot 10-page-busy "?open=board:task:sample-card-4"  # card page: busy agent, spinning step
shot 11-page-blockers "?open=board:task:sample-card-1" 1440 1400 # blockers, PR, thread
shot 12-page-narrow "?open=board:task:sample-card-1" 800 2400 # below 900px: one column
shot 13-page-reduced-motion "#card/sample-card-4" 1440 900 reduce # a still dot, no spin
