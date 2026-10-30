#!/usr/bin/env sh
# Local checks (architecture.md 7.1; there are no CI checks). Run from
# anywhere before every merge or push.
#
# The frontend builds first: tauri::generate_context! embeds frontend/dist,
# so no cargo step compiles the app crate without it.
set -eu
cd "$(dirname "$0")/.."

(
  cd frontend
  bun install --frozen-lockfile
  # src/gen/models.ts follows src/gen/bindings.ts (cargo xtask bindings).
  bun scripts/gen-models.ts --check
  bun run build
  bun run typecheck
  bun run lint
  bun test
)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# frontend/src/gen/bindings.ts is what the window's commands generate (D10).
cargo xtask bindings --check
# Timing-sensitive, as in Go: capture's tests once more on one thread.
cargo test -p irori-capture -- --test-threads=1
sh scripts/no-third-party.sh
echo "check: all passed"
