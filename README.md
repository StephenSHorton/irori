# Irori (Rust)

The Rust rewrite of Irori, using iroh for networking. On the rune-ui branch the window is Rune instead of Tauri and a React webview (docs/rune-ui.md). It is developed here until it is done and tested, then it replaces the Go code in ~/projects/irori.

## Build and check

```sh
cargo build -p irori-app     # target/debug/irori: every role and the Rune window
sh scripts/check.sh          # every local check
```

Run a dev build only with `IRORI_HOME` set to a scratch dir. `irori --snap board.png` renders the window headless on it (docs/rune-ui.md).

## Release

`scripts/release.ps1` (PC) and `scripts/release-mac.sh` (Mac) build the release assets locally and publish one GitHub release; `scripts/installer-check.ps1` checks the installer's silent-update paths in a scratch install folder. See [docs/releasing.md](docs/releasing.md).
