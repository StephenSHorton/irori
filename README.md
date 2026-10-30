# Irori (Rust)

The Rust and Tauri rewrite of Irori, using iroh for networking. It is developed here until it is done and tested, then it replaces the Go code in ~/projects/irori.

## Build and check

```sh
(cd frontend && bun install --frozen-lockfile && bun run build)   # the app embeds frontend/dist
cargo build -p irori-app                                          # target/debug/irori.exe
sh scripts/check.sh                                               # every local check
```

Run a dev build only with `IRORI_HOME` set to a scratch dir. `scripts/window-shot.ps1` starts a build on one, captures the window and closes it by process id.

## Release

`scripts/release.ps1` (PC) and `scripts/release-mac.sh` (Mac) build the release assets locally and publish one GitHub release; `scripts/installer-check.ps1` checks the installer's silent-update paths in a scratch install folder. See [docs/releasing.md](docs/releasing.md).
