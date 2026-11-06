#!/usr/bin/env sh
# Each crate's integration tests are one binary, tests/it/main.rs (#566): a
# file in tests/ is its own binary, with its own link and its own copy of the
# crate's debug info. Run by check.sh.
#
# A test that changes process state (env, cwd) or counts this process's
# sockets can't share a process: add it to `alone` with why at its top.
set -eu
cd "$(dirname "$0")/.."

alone="crates/capture/tests/repo_git_env.rs
crates/core/tests/no_public_contact.rs"

bad=0
for dir in crates/*/tests app/tests; do
  [ -d "$dir" ] || continue
  # A crate with one test file is one binary already.
  [ -d "$dir/it" ] || [ "$(ls "$dir" | grep -c '\.rs$')" -gt 1 ] || continue
  for f in "$dir"/*.rs; do
    [ -e "$f" ] || continue
    if ! printf '%s\n' "$alone" | grep -qx "$f"; then
      echo "one-test-binary: move $f to $dir/it/ and add it to main.rs" >&2
      bad=1
    fi
  done
  # A file main.rs doesn't name never compiles, so its tests never run.
  for f in "$dir"/it/*.rs; do
    [ -e "$f" ] || continue
    m="$(basename "$f" .rs)"
    [ "$m" = main ] || grep -qx "mod $m;" "$dir/it/main.rs" || {
      echo "one-test-binary: add \`mod $m;\` to $dir/it/main.rs" >&2
      bad=1
    }
  done
done
exit "$bad"
