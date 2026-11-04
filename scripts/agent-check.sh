#!/usr/bin/env sh
# What a real Claude agent does with Irori's tools (card #92). Run by hand
# before each release (docs/releasing.md step 1); not part of check.sh
# because every scenario is a real Claude session and spends API tokens.
#
#   sh scripts/agent-check.sh            every scenario
#   sh scripts/agent-check.sh --only 2   just one (1 to 7; comma list)
#
# `irori-dev agent-check` (crates/dev/src/agent_check.rs) hosts a real
# Core on a throwaway IRORI_HOME, acts as the person through the window's
# backend, and starts one `claude -p` session per scenario with a copy of
# mod/ pointed at this tree's debug `irori`, so the session attaches and is
# woken exactly as a real one. After each turn it reads the board:
#   1 an ask arrives: the agent takes it and closes it with an outcome;
#   2 the person must choose: the agent raises a blocker, not a question
#     in its reply;
#   3 the person's comment changes a card's purpose: the agent renames it
#     and redoes its steps;
#   4 the person answers the agent's blocker: the agent resolves it in the
#     turn it acts on the answer (card #204);
#   5 the person stops a busy agent: its turn is cut and it answers first
#     (card #207);
#   6 the person presses Wind down all while a busy agent holds a card and
#     a lease: it hands the card back with a hand-off, releases the lease
#     and answers (card #220);
#   7 the person says Not done with a reason on the agent's done check:
#     the agent answers on that check's thread, and any new check shows
#     that round with changed proof (card #476).
# It prints PASS or FAIL per scenario and exits non-zero on any failure.
#
# Needs `claude` signed in (or CLAUDE_BIN). AGENT_CHECK_MODEL picks the
# model (default: claude's own); AGENT_CHECK_TURN_WAIT caps one turn in
# seconds (default 600). The claude session runs --restricted with
# --strict-mcp-config: no settings files, other plugins, MCP servers or
# claude.ai connectors; only the file tools inside its scratch folder, and
# Irori's. It never sees the running Irori, ~/.irori or the Keychain.
# Scratch dirs are deleted after a pass and kept (path printed) after a
# failure.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

# The app embeds frontend/dist and refuses to build a stale one.
fe="$root/frontend"
if [ ! -f "$fe/dist/index.html" ] ||
  [ -n "$(find "$fe/src" "$fe/public" "$fe/index.html" -newer "$fe/dist/index.html" 2>/dev/null | head -1)" ]; then
  (cd "$fe" && bun install && bun run build) || exit 100
fi
(cd "$root" && cargo build -q -p irori-app -p irori-dev) || exit 100

work="$(mktemp -d "${TMPDIR:-/tmp}/irori-agent-check.XXXXXX")"
mkdir -p "$work/mod"
for f in .claude-plugin hooks types bunfig.toml tsconfig.json; do
  [ -e "$root/mod/$f" ] && cp -R "$root/mod/$f" "$work/mod/"
done
rm -rf "$work/mod/.claude-plugin/types"

set -- --home "$work/home" --claude "$work/claude" --work "$work/agents" \
  --irori "$CARGO_TARGET_DIR/debug/irori" --mod "$work/mod" \
  --claude-bin "${CLAUDE_BIN:-claude}" \
  --turn-wait "${AGENT_CHECK_TURN_WAIT:-600}" "$@"
if [ -n "${AGENT_CHECK_MODEL:-}" ]; then
  set -- "$@" --model "$AGENT_CHECK_MODEL"
fi
"$CARGO_TARGET_DIR/debug/irori-dev" agent-check "$@"
code=$?
if [ "$code" -eq 0 ]; then
  rm -rf "$work"
else
  echo "agent-check: transcripts and the scratch home are kept in $work"
fi
exit "$code"
