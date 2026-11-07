# rune-ui: Irori with a Rune window (spike, card #674)

This branch is a spike and is not for main. It asks how far Irori can go as plain Rust, with
[Rune](https://github.com/StephenSHorton/rune) as the UI in place of Tauri and a React webview.

## What runs

`cargo build -p irori-app` builds `target/<profile>/irori`. It needs only cargo: no bun, node,
vite, Tauri CLI or webview.

- **Every role.** `serve --headless`, `mcp`, `node`, `attach`, `events`, `pair` and `version`
  live in `crates/roles`, the code app/ held before, moved without changes. The binary hands
  their dispatch only its window role.
- **The window** (`app/src/shell.rs`) starts the Core and its API in-process, then builds the
  window's backend (`crates/window`, which has no Tauri commands now). That backend includes
  the spawner, the reviver, the notifier seam and the pumps. The window itself is a Rune app on
  top of it.
- **The Board** (`app/src/board.rs`) reads `Window::cards`, the same list the React Board
  read. It shows the cards in groups (Needs you, Working, Waiting, Open, Not yet) with
  filters. A card's page shows the objective, done-when, steps, open blockers, the thread and
  a comment box.
- **Live updates.** The backend's own `board:changed` and `roster:changed` pumps call
  `AppHandle::update`, and the state re-reads.
- **Desktop pieces Tauri plugins used to provide:**

  | Piece | How it works now |
  |---|---|
  | Tray | Rune's `Tray` on Windows and Linux; `tray-icon` on macOS, since Rune has no Mac tray (`app/src/tray_mac.rs`). |
  | Single instance | A second launch sends `show` to a loopback port the running window writes to `<data>/desk-show.port` (`app/src/instance.rs`, std only). |
  | Clipboard | Rune's `Tx::copy` (Copy ref on a card's page). |
  | Opener | `open`, `explorer` or `xdg-open`. |

## Look at it without a window

Rune renders headless. On a copy of the data dir (CLAUDE.md, "Scratch builds"):

```sh
export IRORI_HOME=<copy> CLAUDE_CONFIG_DIR=<empty dir> IRORI_DEV_OFFLINE=1 \
       IRORI_DEV_KEK_DIR=$HOME/.irori IRORI_DEV_PORT_OFFSET=37
target/debug/irori --snap board.png --size 1400x900 --scale 2
target/debug/irori --snap card.png --scene detail        # the first card that needs you
target/debug/irori --scene detail --script comment.json --tree tree.json --at mark:end
```

`IRORI_DESK_TRACE=1` logs each live refresh.

## What was removed

- **The Tauri app:** `app/src` shell, `build.rs`, `tauri.conf.json`, capabilities, and the
  generated Android and iOS projects with their Kotlin plugins.
- **`frontend/`:** React, bun and vite, about 82k lines of TS and TSX.
- **Tauri in `crates/window`:** its Tauri commands and the tauri-specta TS export.
- **Go:** the Go goldens exporters (`tools/goldens`), the Go update rehearsal
  (`tools/rehearse/rehearse.*`) and `mod/dev`'s fakes. The goldens they wrote stay in
  `testdata/golden`.
- **Scripts:** `window-shot.sh` and `.ps1`, `mod-suite.sh`, `no-third-party.sh` and
  `one-test-binary.sh`. The last two are now `cargo xtask`, which is std only.

## What remains, and why

- **`mod/`** (TypeScript). It is a Claude Code plugin, and those are written in TypeScript (the
  person's call, card #674 b1).
- **`site/`** (React and vite, deployed by `.github/workflows/pages.yml`). The card asked for a
  report only. The recommendation is a hand-written static page (about a day of work). Rune-web
  can't draw the 3D hero and needs wasm-bindgen.
- **Shell scripts:**
  - `check.sh` and its slot, stop and temp helpers. They could move into xtask next, at about
    300 lines.
  - The release scripts, which handle signing, notarizing, NSIS and gh.
  - `scratch-mac.sh` and `boards-live.*`.
- **`app/macos/*.sh` and `app/nsis`:** bundle packaging and the Windows installer.
- **Android:** there is no phone app on this branch. Rune runs on Android after small patches,
  but it isn't a product (#676, #677).

## Gaps compared with the Tauri app

- Launch at login, the updater and Keep this device up to date are not wired: `crates/update`
  is Rust, but the Tauri shell's glue was not ported.
- Desktop cards (`toast`/`notify`) still post. A card's Open shows the window and opens the
  card.
- Calls, voice and terminals (suzuri) have no UI.
- The Windows exe has no version resource or comctl6 manifest; `embed-resource` in a
  `build.rs` would add them.
- A release build still has the WebView2 bootstrapper step in `release.ps1` and `irori.nsi`,
  which is now dead.
- `docs/architecture.md` still describes the Tauri app.
- The rest of the UI is not ported. See the estimate on card #674.
