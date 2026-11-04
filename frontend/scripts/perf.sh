#!/usr/bin/env bash
# Card #468: runs perf.html (src/perf/harness.ts) in headless Chrome against
# a running vite dev server and prints its JSON: commands invoked and React
# commits per board:changed and talk:changed.
#
#   bunx vite --host 127.0.0.1 --port 9268 --strictPort &
#   scripts/perf.sh '?notifs=20#notifications'
#
# PERF_URL overrides the server (default http://127.0.0.1:9268).
set -euo pipefail
query="${1:-}"
base="${PERF_URL:-http://127.0.0.1:9268}"
chrome="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
profile="$(mktemp -d)"
trap 'rm -rf "$profile"' EXIT
# PERF_LOG=file keeps the page's console there.
log="${PERF_LOG:-/dev/null}"
"$chrome" --headless=new --disable-gpu --user-data-dir="$profile" \
  --enable-logging=stderr --v=0 \
  --virtual-time-budget=60000 --window-size=1400,900 \
  --dump-dom "$base/perf.html$query" 2>"$log" |
  sed -n 's/.*<pre id="perf-out">\([^<]*\)<\/pre>.*/\1/p' |
  sed 's/&quot;/"/g; s/&amp;/\&/g'
