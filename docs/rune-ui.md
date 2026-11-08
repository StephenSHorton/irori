# rune-ui: Irori with a Rune window (card #674)

This branch replaces Tauri and the React webview with plain Rust:
[Rune](https://github.com/StephenSHorton/rune) and its component kit draw the whole window.
It started as a spike; the window is now a full port of main's React app (main merged in up
to `c6dd78c8`, Nov 7 2026). It is not merged into main.

## What runs

`cargo build -p irori-app` builds `target/<profile>/irori`. It needs only cargo: no bun, node,
vite, Tauri CLI or webview. `scripts/check.sh` passes on it.

- **Every role.** `serve --headless`, `mcp`, `node`, `attach`, `events`, `pair`,
  `permission-hook` and `version` live in `crates/roles`. The binary hands their dispatch only
  its window role.
- **The window** (`app/src/shell.rs`) starts the Core and its API in-process, then the
  window's backend (`crates/window`, the same `Window` the Tauri commands called) and a Rune
  app on top of it.
- **Every screen of main's window**, one package per directory under `app/src/` (78k lines of
  Rust): Home and Needs you (`home`), the Board list (`board`), a card's page with the New
  card and Share dialogs and permission prompts (`card`), the canvas (`canvas`), terminals
  (`term`), Talk (`talk`), huddles, calls, screen share and dictation (`call`), the Boards
  tab and space picker (`spaces`), Notes (`notes`), Notifications, Held and Message
  (`inbox`), the command palette (`palette`), Merge desks (`desk`), Settings with every pane
  (`settings`), Profile (`profile`), the meters, updater banner, Keep this device up to date,
  launch at login and tray (`status`, `login`). Shared pieces: rich text, the mention
  composer and attachments (`text`), card and agent bits, faces and hover cards (`widgets`),
  the TS libraries' pure logic with their tests ported (`logic`), and the app shell,
  navigation, theme, prefs and event bus (`ui`). `ui/mod.rs` documents the package contract.
- **Live updates.** Every event `crates/window` emits goes to every package on the UI thread
  (`ui::dispatch`); a burst of one event is one dispatch with its newest payload.
- **Window state** the React app kept in localStorage (theme, sidebar, filters, folds,
  drafts) lives in `<data>/window-prefs.json`, so a scratch copy keeps its own.
- **Desktop pieces Tauri plugins used to provide:**

  | Piece | How it works now |
  |---|---|
  | Tray | Rune's `Tray` on Windows and Linux; `tray-icon` on macOS (`app/src/tray_mac.rs`). The full menu (status, held, pause, wind down, launch at login, update). |
  | Single instance | A second launch sends `show` to a loopback port the running window writes to `<data>/desk-show.port` (`app/src/instance.rs`). |
  | Clipboard | Rune's `Tx::copy`, `Tx::copy_checked` where a failed copy must show, and paste (text, images). |
  | Opener, file picker | `open`/`explorer`/`xdg-open`; Rune's `native-file-dialog` (rfd). |
  | Updater, launch at login | `crates/update`, with main's shell glue restored without Tauri (`app/src/status/`, `app/src/login/`). |
  | Windows icon, version resource, manifest | `app/build.rs` (`embed-resource`). |

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
- **Android:** a prototype only (Android, below). Rune runs on Android after small patches,
  but it isn't a product (#676, #677).

## Numbers (Mac, M-series; 2026-11-07)

| | main (Tauri + React) | rune-ui, spike (Board only) | rune-ui, full port |
|---|---|---|---|
| `cargo build --release -p irori-app` | 429 s clean, plus `bun install` and a 5 s `bun run build` | 375 s clean, cargo only | 440 s (a warm target: deps reused), cargo only |
| `irori` binary | 54.5 MB, links WebKit.framework | 53.1 MB, no WebKit | 65.3 MB, no WebKit |
| Non-Rust lines (TS/TSX/Go/sh/ps1/Kotlin/CSS/JS) | 97,394 | 6,444 | 6,534 (mod/, site/, release and packaging scripts, the Android prototype's Gradle) |
| Window UI | ~50k lines of product TS/TSX | 1,137 lines of Rust | 77.8k lines of Rust in `app/src` (with ~1,000 unit tests' worth of ported TS test cases) |

Caveats:
- Other agents were building on this machine the whole time, so build times show direction
  only.
- main was built without `--features tauri/custom-protocol` (which release-mac.sh adds).
- The full port's binary grows by about 12 MB over the spike: the whole UI, rune-kit with
  markdown, H.264 decode for screen share (moq-video) and image decoders. The Core (iroh,
  whisper.cpp, SQLite, media) is still most of it.
- Screenshots and scripted checks ran on an offline copy of the data dir (sqlite
  `VACUUM INTO`, `IRORI_DEV_OFFLINE=1`).

## Tauri's job across OSes

Cargo was always what compiled Irori for each OS; Tauri wasn't. Desktop releases already had
Tauri's bundler off: the Mac bundle is `app/macos/package.sh` and Windows uses NSIS, both kept.
Tauri's one remaining build job was Android packaging (`cargo tauri android build`), and the
phone app now waits for Rune on Android.

## Android (prototype, card #680)

Not shipped: main's Tauri phone app stays. `app/` is a library too (`app/src/lib.rs`), and on
Android its `android_main` runs `shell::run_android`: the same Core, window backend and
packages as the desktop, in the app's private files dir, with no spawner
(`set_runs_no_agents`), no tray, single instance or exit hook. Rune's Kotlin host
(`RuneActivity`, from the Rune fork's Android shell, #677 and #679) loads `libirori_app.so` and
takes the on-screen keyboard. The phone frame (`State::phone`) has no sidebar, pads 32 dp for
the status bar, and leaves 360 dp under a card's page so its comment box scrolls above the
keyboard. Join with a code (`app/src/phone`) shows until the phone has another device. Home,
the Board and the card page are the full rune-ui ports, which fit phone width as they are.

```sh
sh scripts/android-rune.sh --install     # needs the env in docs/android.md (ANDROID_HOME, JAVA_HOME, NDK)
```

It builds `target/android/irori-rune.apk` (66 MB, debug-signed; the library is 64 MB of it,
plus 1.8 MB of libc++_shared). The applicationId is `com.stephenshorton.irori.rune`, so it
never replaces a Tauri install. Gradle (`app/android`) only wraps the cargo-built library,
and the script runs it from a clean build dir: its incremental packager kept a replaced
library inside the zip (a 120 MB APK). The Kotlin host comes from cargo's checkout of the
pinned Rune rev, so its keyboard protocol always matches.

To pair it on an emulator without touching your own Irori, use a scratch peer on the Mac:
`irori-dev pair-peer --home <empty dir> --claude <empty dir> [--port 24817]`. It makes a
fresh person with a few cards, turns its home server on and opens an Add window (the API
keeps no Add route on purpose). It prints the code and the key. On the phone, give the
server as `10.0.2.2:24817`: the emulator's NAT hides the LAN lookup. Then type
`words <the phone's words>` into the peer.

Checked on an API 35 emulator (2026-11-07; screenshots in `docs/img/rune-android/`):
- It paired with the scratch peer, and Home showed the peer's cards.
- New cards and steps ticked on the peer appeared within seconds, without a touch.
- A comment typed with Gboard reached the peer's thread, written by the phone's device. After
  a cold start the first one took about 1.5 min, until the mesh redialed.
- After a trip to the background, it sent its rows, redrew and caught up.
- The Core is up in about 110 ms on a fresh data dir. main's phone took 21 s to open a fully
  synced one.

Gaps:
- The system Back closes the app.
- The keyboard can come back after a resume with no field focused (typing goes nowhere until
  a field is tapped).
- Enter in the composer adds a line; Comment sends.
- Insets are hand-padded: Rune feeds neither `safe_area` nor the keyboard's height on Android.
- Settings, Devices and Talk weren't checked at phone width.
- Notifications, calls and updates have no Kotlin side. Their move from Tauri's plugin bridge
  to JNI is costed on card #680.

## site/ and mod/

- `site/`: rewrite it as hand-written static HTML and CSS (about a day). That removes bun,
  vite, React, Tailwind, three.js and mokei, and the 3D hero becomes the existing CSS still or
  a captured image. Rune-web would take 3–5 days, can't draw the 3D scene, and needs Rust,
  wasm-bindgen and a heavier Pages job.
- `mod/`: keep it as TypeScript (the person chose this). Only its Go fakes went.

## How the port was done

A foundation (shell, routes, event bus, theme, a stub per screen), then 13 package agents in
parallel worktrees, 3 integration passes wiring them to each other, two verification rounds
(each area audited feature by feature against main's TSX and fixed, then re-checked by an
independent agent), main's 79 newer commits merged and their UI changes ported, and
`check.sh` run end to end.

## What is left, and why

Not checked live (headless renders and `--script` runs only, on an offline copy of the data):
a real huddle with peers and a mic, screen share pictures (H.264 decoded with moq-video),
answering a real permission prompt, Launch at login and the updater's restart, and macOS
Cmd-Q / Windows logoff shutdown in a real window.

Rune's gaps were fixed in the fork, branch `card-674-gaps` of github.com/StephenSHorton/rune
(pinned in `app/Cargo.toml`): caret-anchored popups, multi-file picks, dialogs that wrap and
scroll, kit Slider, Avatar, Skeleton, Progress, ColorPicker and swatches, dashed and flowing
strokes, the mono fallback, WebP and GIF, tray check items on Windows and Linux, and
`Tx::copy_checked`.

Small differences from the React app that remain:
- Card chips (`widgets/bits.rs` `badge()`) are hand-built: kit `Badge` has no font or
  corner-radius setter to match them.
- A tap off a pinned agent card closes it and does nothing else, and the first tap of a run
  landing on a face reaches its tile: Rune has no press that dismisses and passes through.
- Glass (frosted) styles are dropped; popovers are solid everywhere, as main is off the Mac.
- The Mac tray stays on `tray-icon` (`app/src/tray_mac.rs`): the fork has no Mac tray.
- The terminal grid keeps `Family::SystemMono`: it caches one cell advance and needs a fixed face.

The phone layout is the separate Android prototype (card #680, `app/src/phone`).

