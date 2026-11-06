# Working in the Irori repo

Many agents work in this repo at once, often on the same machine. These rules
keep them out of each other's way and off the live app. Each links the doc
that has the details; read that rather than guessing.

## Checks

- `sh scripts/check.sh` is the check before every merge or push (there is no
  CI). What it runs: [docs/architecture.md](docs/architecture.md) §7.1.
- Other agents run check.sh on the same machine. Never `pkill -f check.sh`
  (or any pattern that matches their runs): stop your own by its PID tree. If
  your shell died (exit 144), your run may go on orphaned, so check `ps` in
  your worktree before starting another.
- Timeouts under load are usually load, not your change: "presence never
  listed both sessions" in the core/api/attach tests, or shellpath's
  `login shell PATH = ""`. Rerun the failed tests alone
  (`cargo test -p <crate> --test it <module>::<test> -- --exact --test-threads=1`) and
  compare with main before chasing them.
- Integration tests live in `tests/it/` (one binary per crate), with
  `use crate::common;`, never `mod common;`. Layout and the Windows TempDir
  and fswatch rules: architecture.md §7.1.
- PC build tools (cmake, libclang): [docs/releasing.md](docs/releasing.md),
  Build tools.

## Browser preview

- `bun run preview:look`'s port 9246 is often held by another worktree's
  vite, so you'd be checking someone else's code. Run your own on a free port
  and don't kill 9246. How, and how to check layout at phone width and in
  WebKit: [frontend/PREVIEW.md](frontend/PREVIEW.md).

## Scratch builds never touch the live app

- The person's own Irori is running beside you on `~/.irori` (on Windows
  `%LOCALAPPDATA%\irori`). Run a scratch build with `IRORI_HOME` set to a copy,
  `CLAUDE_CONFIG_DIR` set to an empty dir and `IRORI_DEV_PORT_OFFSET=N`, and
  stop it by its PID.
- On the Mac every bundle shares the bundle id `com.stephenshorton.irori`:
  never quit Irori by app name or bundle id (`osascript`, `killall Irori`),
  which quits the live one. `open Irori.app` drops your environment and runs
  on the live `~/.irori`: see [docs/releasing.md](docs/releasing.md),
  "Rehearsing an update on the Mac".
- After a scratch run, check the live `irori.log` gained no line for your
  version.

## Product rule

- Features work by themselves with sensible defaults. A setting only turns a
  feature off or tunes it; never make the person switch something on before
  it works ([docs/decisions.md](docs/decisions.md) #52).
