#!/usr/bin/env bash
# Card #468: scripts/perf.sh over Home, Notifications (30 card rows),
# Talk and Notes, one line each: IPC calls, commits and components
# rendered per board:changed (the median of 3) and per talk:changed, and
# the commands a board:changed sent.
set -euo pipefail
cd "$(dirname "$0")/.."
for q in '' '?notifs=30#notifications' '#talk' '#notes'; do
  ./scripts/perf.sh "$q" | python3 -c '
import json, sys
d = json.load(sys.stdin)
def med(rs, k):
    v = sorted(r[k] for r in rs)
    return v[len(v) // 2]
b, t = d["boardChanged"], d["talkChanged"]
page = d["page"] + (" (%d rows)" % d["notifs"] if d["notifs"] else "")
calls = ", ".join("%s %d" % kv for kv in b[-1]["calls"].items())
print("%-28s board:changed ipc %3d commits %d rendered %4d | talk:changed ipc %d rendered %4d | mounted %s | %s" % (
    page, med(b, "ipc"), med(b, "commits"), med(b, "rendered"), med(t, "ipc"), med(t, "rendered"), d["mounted"], calls))
if d["errors"]: print("  errors:", d["errors"])
'
done
