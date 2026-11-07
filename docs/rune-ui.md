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

## Numbers (Mac, M-series; 2026-11-07)

| | main (Tauri + React) | rune-ui (Rune) |
|---|---|---|
| Clean `cargo build --release -p irori-app` | 429 s, plus `bun install` and a 5 s `bun run build` | 375 s, cargo only |
| Crates compiled | 644 | 622 |
| `irori` binary | 54.5 MB, links WebKit.framework | 53.1 MB, no WebKit |
| Non-Rust lines (TS/TSX/Go/sh/ps1/Kotlin/CSS) | 97,394 | 6,444 (mod/ 5 files, site/ 7, scripts, packaging) |
| Rust lines | 363,186 | 356,518 (the Tauri shell and command layer went; the Rune shell and Board are 1,137 lines) |

Caveats:
- Other agents were building on this machine. The load average was 6–9 during main's build and
  11–16 during rune-ui's, so the build times show the direction, not exact figures.
- main was built without `--features tauri/custom-protocol` (which release-mac.sh adds), so it
  is not quite the binary that ships.
- The size barely moves because the Core (iroh, whisper.cpp, SQLite, media) is most of the
  binary either way. What changes is no webview at run time and no node toolchain at build time.
- The screenshots, the commenting check and the live-refresh check ran on an offline copy of
  the data dir (sqlite `VACUUM INTO`, `IRORI_DEV_OFFLINE=1`). Their test comments exist only in
  that copy.
- `check.sh` hasn't been run end to end on this branch. Its Android step now checks
  `-p irori-window` and is untested.

## Tauri's job across OSes

Cargo was always what compiled Irori for each OS; Tauri wasn't. Desktop releases already had
Tauri's bundler off: the Mac bundle is `app/macos/package.sh` and Windows uses NSIS, both kept.
Tauri's one remaining build job was Android packaging (`cargo tauri android build`), and the
phone app now waits for Rune on Android.

## Android

A Rune test app ran on an API 35 emulator with no Tauri and no Gradle, from an APK built by
hand. It needed two small Rune patches and a winit feature flag. It is not a product yet:
- The screen goes black after the app comes back from the background.
- There is no soft-keyboard text input, no safe-area insets and no accessibility.
- Irori's notification, call and update Kotlin (about 640 lines) would move from Tauri's
  plugin bridge to JNI.

Estimate for a usable phone client: 3–4 weeks, and it still needs a small Kotlin host and
Gradle. The person wants it as a prototype only: #676, #677.

## site/ and mod/

- `site/`: rewrite it as hand-written static HTML and CSS (about a day). That removes bun,
  vite, React, Tailwind, three.js and mokei, and the 3D hero becomes the existing CSS still or
  a captured image. Rune-web would take 3–5 days, can't draw the 3D scene, and needs Rust,
  wasm-bindgen and a heavier Pages job.
- `mod/`: keep it as TypeScript (the person chose this). Only its Go fakes went.

## Estimate for porting the rest of the UI

Method: the React Board surface (`board-view`, `card-grid`, `card-detail`, `bits`) is about
5,000 lines of TSX. The Rune Board and card page are about 650 lines and cover roughly a
third of it (groups, filters, steps, blockers, the thread and comments, but no canvas,
markdown, attachments, mentions, answer buttons or held and paused states). So the
Rune-to-TSX ratio is about 1 to 2.5, with crates/window already doing the data work in Rust.

The 82k lines of TS and TSX map to about 30–35k lines of Rust. At an agent's pace that is
about 8–12 agent-weeks, or 3–5 weeks of calendar time with screens ported in parallel.

The parts that take longest, each its own card:
- Terminal rendering for the inline terminals (suzuri's view is TS today).
- Talk's composer, with mentions, attachments and replying states.
- The canvas board.
- Markdown and rich text in threads (Rune has rich text and spans; it has no markdown parser).
- The settings, devices, pairing and invite flows.
- The voice and call UI.
- The glue for launch at login, the updater and Keep this device up to date (`crates/update`
  is already Rust).
- The Windows version resource and comctl6 manifest (`embed-resource`).

Rune gaps found on this spike:
- No macOS tray (the branch uses `tray-icon`).
- No Android shell.
- Text doesn't wrap unless it has `.wrap()` or `max_lines`.
- The pinned rev declares Rust 1.99.1, yet builds fine on Irori's 1.97.1.
- On macOS, Cmd-Q and logout end inside AppKit's `terminate:`. The branch runs its shutdown
  from `atexit`; that fix hasn't been checked live, since it can't run headless.

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
