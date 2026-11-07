#!/usr/bin/env sh
# The whole Screen Recording grant test for #657: `sh crates/share/grant-test.sh walk`.
# An agent starts it; the person needs no Terminal. Every step that needs
# them shows a dialog on the Mac's screen saying what to do, and waits for
# their click, so it can wait for hours until they sit down. Everything is
# written to target/share-grant/walk.log for the agent.
#
# 1. The picker with no grant (SharePicker, ad hoc).
# 2. Ad hoc: ask, switch on, capture; "update" (a new signature), capture.
# 3. Developer ID: the same.
# 4. Take the ad hoc and picker entries out of Settings again.
#
# WALK_STEPS="adhoc devid" runs only those parts (default: all three).
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
title="Irori screen-share test (#657)"

say() { printf '\n== %s\n' "$*" | tee -a "$log"; }
# A dialog on the Mac's screen; waits up to a day for a click. $1 is the
# text (no double quotes), the rest are the buttons, last one the default.
# Prints the button clicked.
dialog() {
  text=$1
  shift
  buttons=""
  for b in "$@"; do buttons="$buttons${buttons:+, }\"$b\""; done
  osascript -e "with timeout of 86400 seconds" \
    -e "display dialog \"$text\" buttons {$buttons} default button ${#} with title \"$title\"" \
    -e "end timeout" 2>&1 | sed -n 's/.*button returned:\([^,]*\).*/\1/p'
}
tell() { echo "SHOWN: $1" >>"$log"; dialog "$1" "${2:-OK}" >/dev/null; }
ask() {
  a=$(dialog "$1" "No" "Yes")
  echo "Q: $1 A: $a" | tee -a "$log"
}
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
granted() { $t run "$1" grant 2>&1 | grep -q "granted before: true"; }
# Ask for the grant for app $1 (shown as $2), open Settings, and wait until
# the person has switched it on: the dialog closes by itself once a fresh
# launch of the app holds the grant, or after 10 minutes.
switch_on() {
  run "$1" grant --request
  open "$settings"
  msg="In System Settings (just opened), Privacy and Security, Screen and System Audio Recording: find $2 in the list and switch it ON. Not Irori: leave Irori as it is. If $2 isn't there yet, wait a few seconds or click the + button. If macOS offers Quit and Reopen, choose Later. This box closes by itself once $2 is on."
  echo "SHOWN: $msg" >>"$log"
  osascript -e "display dialog \"$msg\" buttons {\"Skip\"} default button 1 giving up after 600 with title \"$title\"" >/dev/null 2>&1 &
  box=$!
  i=0
  while kill -0 "$box" 2>/dev/null; do
    if granted "$1"; then
      kill "$box" 2>/dev/null
      say "$2 is switched on (after about $((i * 3)) s)"
      return 0
    fi
    i=$((i + 1))
    sleep 3
  done
  say "$2 was not switched on (skipped or 10 minutes passed)"
  return 1
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

tell "Hi! This is the Screen Recording test for screen share, about 3 minutes. It uses test apps called ShareDevAdhoc, ShareDevDevID and SharePicker. Please don't touch Irori's own switch. Each step tells you what to do in a box like this one. Click Start when you're ready." "Start"

steps=${WALK_STEPS:-picker adhoc devid}
has() { case " $steps " in *" $1 "*) return 0 ;; esac; return 1; }

if has picker; then
say "1. picker, no grant"
tell "Picker: when you click OK, the system window picker opens. Click any window in it (a Finder or Notes window is fine)."
run picker
ask "Did the picker appear without a Screen Recording prompt first?"
ask "After you picked a window, did macOS show any extra prompt or alert?"
fi

if has adhoc; then
say "2a. ad hoc v1: ask for the grant"
tell "Ad hoc signed app: a Screen Recording prompt for ShareDevAdhoc will appear, then System Settings opens. Click OK to begin."
switch_on adhoc ShareDevAdhoc
say "2b. ad hoc v1: capture 3 s"
run adhoc display --secs 3
say "2c. ad hoc v2: same app, new signature (an update)"
build adhoc 2
run adhoc grant
run adhoc display --secs 3
open "$settings"
ask "The test app was just updated. Does Settings still show ShareDevAdhoc switched ON?"
ask "Did a new Screen Recording prompt appear just now?"
fi

if has devid; then
say "3a. Developer ID v1: ask for the grant"
tell "Developer ID app: the same for ShareDevDevID, the Developer ID signed test app. Click OK to begin."
switch_on devid ShareDevDevID
say "3b. Developer ID v1: capture 3 s"
run devid display --secs 3
say "3c. Developer ID v2: new signature (an update)"
build devid 2
run devid grant
run devid display --secs 3
ask "The Developer ID test app was just updated. Did any Screen Recording prompt appear?"
fi

say "4. clean up"
for k in adhoc picker; do
  tccutil reset ScreenCapture "com.stephenshorton.irori.sharedev-$k" 2>&1 | tee -a "$log"
done
tell "All done, thank you! The agent takes it from here. ShareDevDevID stays in the list for a few minutes while the agent measures, then the agent removes it."
say "done: the agent reads $log"
