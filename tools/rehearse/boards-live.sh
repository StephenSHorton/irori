#!/usr/bin/env sh
# The live two-person shared-board run (card #48 s3, v0.5-shared-boards
# §11 "Live"), one side per machine; the PC's is boards-live.ps1.
#
#   sh tools/rehearse/boards-live.sh guest '<irori-invite:...>'   (the Mac, person B)
#   sh tools/rehearse/boards-live.sh host                         (person A)
#
# Each side is `irori-dev boards-live` (crates/dev/src/boards_live.rs): a
# real Core in-process on a fresh scratch IRORI_HOME, as a made-up person
# with one made-up Claude session, stepping through the run and printing
# PASS or FAIL per step. Person A (the PC) turns its home server on,
# creates a board and an invite and prints the link; send it to B. B joins
# and prints five words; read them out over the call, and A types them.
# Then: cards both ways, A's comment reaching B's agent through the bridge,
# share then withdraw, a board ask, a review request from A's agent to B's
# agent, and B leaving. Each side waits on the other by what it sees on
# the board, up to IRORI_LIVE_STEP_WAIT seconds per step (default 300).
#
# No Keychain item: the scratch person's keys are sealed with the file
# keystore inside the scratch home (debug build), so nothing asks
# Stephen's OK and nothing is left in the Keychain. The running Irori
# app, ~/.irori and the Keychain are never touched.
# IRORI_DEV_PORT_OFFSET (default 200) moves the LAN lookup and the home
# server's ports off the running app's. The scratch dirs are deleted at the
# end; the exit status is the number of failed steps.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
role="${1:-}"
link="${2:-}"
case "$role" in
  host | guest) ;;
  *) echo "usage: sh $0 host | guest '<irori-invite:...>'" >&2; exit 2 ;;
esac
if [ "$role" = guest ] && [ -z "$link" ]; then
  echo "boards-live: the guest needs the invite link" >&2
  exit 2
fi
export IRORI_DEV_PORT_OFFSET="${IRORI_DEV_PORT_OFFSET:-200}"
work="$(mktemp -d "${TMPDIR:-/tmp}/irori-boards-live.XXXXXX")"
trap 'rm -rf "$work"' EXIT INT TERM
export IRORI_HOME="$work/home"
(cd "$root" && cargo build -q -p irori-dev) || exit 100
dev="$root/target/debug/irori-dev"
if [ "$role" = host ]; then
  "$dev" boards-live host --home "$IRORI_HOME" --claude "$work/claude" \
    --link-out "$work/link.txt" --step-wait "${IRORI_LIVE_STEP_WAIT:-300}"
else
  "$dev" boards-live guest --home "$IRORI_HOME" --claude "$work/claude" \
    --link "$link" --step-wait "${IRORI_LIVE_STEP_WAIT:-300}"
fi
