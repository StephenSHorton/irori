#!/usr/bin/env sh
# The Irori Claude Code mod's two halves of the attach contract
# (architecture.md §6.4, §8 M4), without touching the mod:
#
#   sh tools/mod-suite.sh [path-to-go-irori]
#
# What this does and does not show:
# 1. Copies I:mod into target/mod-suite (the plugin loader writes
#    .claude-plugin/types/ into whatever it tests, so never the Go tree or
#    the installed mod) and runs both of its suites there, unchanged:
#    `claude plugin test` (tests/irori.test.tsx) and `bun test`
#    (unit/lib.spec.ts). Both are hermetic by design: the test kit has no
#    process, fetch or Node globals, and the suites answer `process.spawn`
#    and `http.fetch` themselves. So they test the mod alone and prove
#    nothing about Rust; they pin the mod's side of the contract (the argv
#    it spawns, the lines it reads, the report bodies it posts).
# 2. Runs the Rust side of that contract against the real API server
#    (`irori_api::Server`, the one `irori serve` hosts): the real `irori
#    attach` binary spawned with the mod's exact argv on a scratch
#    IRORI_HOME (app/tests/attach_exe.rs), the ported Go attach and member
#    tests and the Go-exported event goldens (crates/attach/tests).
#
# Nothing here runs the mod against Rust. That is the live check in
# architecture.md §8 M4 (a real session through a `--plugin-dir` copy of
# the mod with `iroriPath` set to the dev exe), still owed.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
goirori="${1:-$root/../irori}"
goirori="$(cd "$goirori" && pwd)"
dst="$root/target/mod-suite"

rm -rf "$dst"
mkdir -p "$dst"
for f in .claude-plugin hooks types tests unit bunfig.toml tsconfig.json; do
  cp -r "$goirori/mod/$f" "$dst/"
done
(cd "$dst" && claude plugin test .)
(cd "$dst" && bun test)
(cd "$root" && cargo test -p irori-attach && cargo test -p irori-app --test attach_exe)
echo "mod-suite: the mod's suites and the Rust attach side passed"
