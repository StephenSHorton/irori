#!/usr/bin/env sh
# Screenshots of the browser preview (frontend/PREVIEW.md) with an installed
# Chrome or Edge, headless, into frontend/preview-shots/ (gitignored).
# Start `bun run preview:look` first: it writes its URL to
# frontend/.preview-url, which this reads (PREVIEW_URL overrides). A
# throwaway profile keeps it off your own browser session.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
out="$here/preview-shots"
base="${PREVIEW_URL:-}"
[ -n "$base" ] || [ ! -f "$here/.preview-url" ] || base=$(tr -d '\r\n' <"$here/.preview-url")
[ -n "$base" ] || { echo "no preview running in this checkout: start \`bun run preview:look\` or set PREVIEW_URL" >&2; exit 1; }
case "$base" in */) ;; *) base="$base/" ;; esac
curl -sf -o /dev/null "$base" || { echo "nothing at $base: start \`bun run preview:look\` or set PREVIEW_URL" >&2; exit 1; }
# Unless PREVIEW_URL was set by hand, it must be this checkout's preview: a
# stale .preview-url can point at a port another worktree's preview took.
if [ -z "${PREVIEW_URL:-}" ]; then
  norm() { tr '\\' '/' | tr 'A-Z' 'a-z' | sed 's#/$##'; }
  mine=$( (cygpath -m "$(cd "$here" && pwd -P)" 2>/dev/null || (cd "$here" && pwd -P)) | norm)
  served=$(curl -sf "${base}__preview-root" | norm)
  [ "$served" = "$mine" ] || { echo "$base is another checkout's preview (${served:-unknown}): start \`bun run preview:look\` in this one" >&2; exit 1; }
fi
browser="${BROWSER_EXE:-}"
for c in "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" \
  "/c/Program Files/Google/Chrome/Application/chrome.exe" \
  "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
  "/c/Program Files/Microsoft/Edge/Application/msedge.exe"; do
  [ -z "$browser" ] && [ -x "$c" ] && browser="$c"
done
[ -n "$browser" ] || { echo "no Chrome or Edge found; set BROWSER_EXE" >&2; exit 1; }
profile=$(mktemp -d)
trap 'rm -rf "$profile"' EXIT
mkdir -p "$out"

# PREVIEW_MODE=light shoots every row in the light palette (?mode=light).
mode="${PREVIEW_MODE:-}"
withmode() { # query -> query with ?mode=<mode> added
  [ -n "$mode" ] || { printf '%s' "$1"; return; }
  case "$1" in
    \?*) printf '?mode=%s&%s' "$mode" "${1#\?}" ;;
    *) printf '?mode=%s%s' "$mode" "$1" ;;
  esac
}

shot() { # name query [width height [reduce]]
  # Headless Chrome won't lay out narrower than 500 px: a phone shot here
  # would be a 500 px page cropped. Phone widths go through check:phone.
  [ "${3:-1440}" -ge 500 ] || { echo "$1: ${3}px is under Chrome's 500 px minimum; use \`bun run check:phone\`" >&2; exit 1; }
  "$browser" --headless=new --disable-gpu --hide-scrollbars --no-first-run \
    ${5:+--force-prefers-reduced-motion} \
    --user-data-dir="$profile" --window-size="${3:-1440},${4:-900}" \
    --virtual-time-budget=4000 --screenshot="$(cygpath -w "$out/$1.png" 2>/dev/null || echo "$out/$1.png")" \
    "$base$(withmode "$2")" >/dev/null 2>&1
  echo "$out/$1.png"
}

shot 01-cards ""                                  # the grid; sample-card-2 and -4 spinning
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
shot 14-page-tall-side "?open=board:task:sample-card-1" 1440 640 # side column taller than the window: its own scroll
shot 15-page-wide "?open=board:task:sample-card-1" 1920 1300 # a wide window: options side by side
shot 16-grid-after-back "?from=sample-card-4"                 # the grid as Back leaves it: card marked, focused
shot 17-page-loading "?slow#card/sample-card-1"               # a reload on a card while it loads
shot 18-page-unknown "#card/sample-card-gone"                 # a card that isn't there
shot 19-page-ask "?open=board:task:sample-ask-2"             # an ask an agent took: its status line, Close, the thread
shot 20-page-ask-open "?open=board:task:sample-ask-1"        # an ask nobody has taken yet
shot 21-grid-done "?filter=done"                            # the Done tab: completed, canceled, failed, a closed ask
shot 22-boards "?boards"                                     # New or join a space (card #689), the space picker's dialog
shot 23-grid-board "?filter=board:sample-board-mlg"          # the grid on a board's chip: mlg#1, mlg#2
shot 24-page-shared "?open=board:task:sample-board-1"        # a shared card: mlg#1, Visible to, Withdraw
shot 25-share-preview "?share=sample-card-10#card/sample-card-10" # Share to board: the scrubbed preview
shot 26-meters "?meters=open"                                # hardware meters panel and Claude usage left (card #599)
shot 27-meters-900 "?meters=open" 900 900                    # a narrower window: CPU, Mem and Claude in the strip
shot 28-meters-board "#board/sample-board-mlg" 1440 300      # a board's page: the strip gives the board's name room
shot 29-usage-none "?meters=open&usage=none"                 # no Claude usage reading yet
shot 30-usage-stale "?meters=open&usage=stale"               # an old usage reading: "as of"
shot 31-meters-nogpu "?meters=open&gpu=none"                 # a machine with no GPU
shot 32-board-closed-fold "?closed#board/sample-board-mlg"   # tiles with descriptions, Closed boards open (card #690)
shot 33-board-description "#board/sample-board-mlg/sample-board-mlg-net" # a board's description at its top, More
shot 34-board-no-description "#board/sample-board-mlg/sample-board-mlg-art" # no description: nothing shown
shot 35-board-close "?close=sample-board-mlg-net#board/sample-board-mlg/sample-board-mlg-net" # Close board with open cards: move or cancel
shot 36-board-closed "#board/sample-board-mlg/sample-board-mlg-proto" # a closed board: read-only, Reopen
shot 37-personal-closed "?closed#board/personal"             # Personal's boards and its Closed boards fold
shot 38-board-new "?addboard#board/sample-board-mlg"         # New board with its optional description
shot 39-board-edit-description "?description=sample-board-mlg-net#board/sample-board-mlg/sample-board-mlg-net" # Edit description
