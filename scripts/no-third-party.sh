#!/usr/bin/env sh
# architecture.md §7.1 `no-third-party`: Irori talks only to its own
# relay and devices. Fails when the tree holds a public discovery or relay
# path: iroh's N0 preset or default relays in code, or the mDNS, pkarr or
# swarm-discovery crates in Cargo.lock. (A shell check in place of the
# planned `cargo xtask no-third-party`; there is no xtask crate yet.)
set -eu
cd "$(dirname "$0")/.."

bad=0
crates=$(grep -E '^name = "(pkarr|mdns[a-z-]*|swarm-discovery|iroh-mdns[a-z-]*)"' Cargo.lock || true)
if [ -n "$crates" ]; then
  echo "no-third-party: discovery crates in Cargo.lock:"
  echo "$crates"
  bad=1
fi
# Code, not comments: lines whose first non-blank characters are not //.
code=$(grep -rnE 'presets::N0|RelayMode::Default' --include='*.rs' crates app \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
if [ -n "$code" ]; then
  echo "no-third-party: public relay or discovery presets in code:"
  echo "$code"
  bad=1
fi
[ "$bad" -eq 0 ] || exit 1
echo "no-third-party: ok"
