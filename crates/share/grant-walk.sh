#!/usr/bin/env sh
# The whole Screen Recording grant test for #657, for the person at the Mac:
# `sh crates/share/grant-test.sh walk`. About five minutes. It asks before
# each step that needs a click, and writes everything to
# target/share-grant/walk.log for the agent.
#
# 1. The picker with no grant (SharePicker, ad hoc).
# 2. Ad hoc: ask, switch on, capture; "update" (a new signature), capture.
# 3. Developer ID: the same.
# 4. Optionally take the ad hoc and picker entries out of Settings again.
#    ShareDevDevID stays on: the agent measures captured windows with it,
#    then removes it (tccutil reset).
#
# None of the test bundles is Irori: each has its own bundle id
# (com.stephenshorton.irori.sharedev-*), so the live app's grant is untouched.
set -u
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/../.."
t="sh crates/share/grant-test.sh"
target=${CARGO_TARGET_DIR:-target}
mkdir -p "$target/share-grant"
log="$target/share-grant/walk.log"
: >"$log"
settings="x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"

say() { printf '\n== %s\n' "$*" | tee -a "$log"; }
wait_enter() { printf '%s [Enter] ' "$*"; read -r _; }
ask() { printf '%s [y/n] ' "$1"; read -r a; echo "Q: $1 A: $a" >>"$log"; }
run() { $t run "$@" 2>&1 | tee -a "$log"; }
build() {
  if out=$($t build "$@" 2>&1); then
    printf '%s\n' "$out" | grep -E "^Identifier=|^Authority=|^Signature|^CDHash=|^TeamIdentifier=" | tee -a "$log"
  else
    printf '%s\n' "$out" | tee -a "$log"
    say "build $* failed: stopping the walk"
    exit 1
  fi
}

say "macOS $(sw_vers -productVersion)"
say "starting clean: removing any earlier test entries"
for k in adhoc devid picker; do
  tccutil reset ScreenCapture "com.stephenshorton.irori.sharedev-$k" 2>&1 | tee -a "$log"
done
say "building the three test apps"
build picker 1
build adhoc 1
build devid 1

say "1. picker, no grant"
echo "A system picker opens. Click any window (a Finder or Notes window is fine)."
run picker
ask "Did the picker show without any Screen Recording prompt first?"
ask "After you picked, did macOS show any extra prompt or alert?"

say "2a. ad hoc: ask for the grant"
echo "A Screen Recording prompt should appear for ShareDevAdhoc."
run adhoc grant --request
open "$settings"
wait_enter "In System Settings > Privacy & Security > Screen & System Audio Recording, switch ShareDevAdhoc ON (if it offers Quit & Reopen, pick Later). Then come back here and press"
say "2b. ad hoc v1: capture 3 s"
run adhoc grant
run adhoc display --secs 3
say "2c. ad hoc v2: same app, new signature (an update)"
build adhoc 2
run adhoc grant
run adhoc display --secs 3
open "$settings"
ask "Does Settings still show ShareDevAdhoc switched ON?"
ask "Did a new Screen Recording prompt appear just now?"

say "3a. Developer ID: ask for the grant"
run devid grant --request
open "$settings"
wait_enter "Switch ShareDevDevID ON in the same Settings list (Later if it offers Quit & Reopen), then press"
say "3b. Developer ID v1: capture 3 s"
run devid grant
run devid display --secs 3
say "3c. Developer ID v2: new signature (an update)"
build devid 2
run devid grant
run devid display --secs 3
ask "Did any Screen Recording prompt appear after the Developer ID update?"

say "4. clean up"
ask "Take ShareDevAdhoc and SharePicker out of the Screen Recording list now (ShareDevDevID stays for the agent's measurements)?"
if tail -1 "$log" | grep -q 'A: y'; then  # ask writes only its Q/A line to the log
  for k in adhoc picker; do
    tccutil reset ScreenCapture "com.stephenshorton.irori.sharedev-$k" 2>&1 | tee -a "$log"
  done
fi
say "done: the agent reads $log"
