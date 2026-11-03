# Inline terminals: design

*2026-11-03, card #353, from Stephen's #350: make suzuri embeddable, so Irori can show a live terminal inside a card and beside an agent. Agents' commands run in it where you can see them, you can take over by typing, and the phone watches but can't type. Builds on Irori main @ 7011b40e and suzuri main @ 587f7a5: the terminal core `rsz-term` (`suzuri:crates/term`), suzuri's plain-JS renderer (`suzuri:ui/frame.js`, `keys.js`, `pointer.js`), Irori's window commands (`crates/window`), the mod's `tool.call` hook (`mod/hooks/register.tsx:718`) and the mesh's labelled streams (architecture.md §3.1).*

Paths starting `suzuri:` are in the suzuri repo; every other path is in Irori.

## 1. What the person sees

- **A card terminal.** A card page gets a **Terminal** section beside Thread (`CardPage`, `components/cards/card-detail.tsx:113`). It holds one live shell, opened in the card's folder on this device (§6.1 says where that comes from). It stays on the card while the card is open, and its scrollback is kept while the app runs.
- **An agent-row terminal.** In Who's on it (`People`, card-detail.tsx:1894), each agent row gets a terminal toggle. It opens the terminal that agent's commands run in (§7), under the row.
- **Watching an agent work.** When an agent runs a command through Irori (§7), the command and its output appear in that terminal as if someone typed them. A label on the terminal names whoever is typing: `moss is running…`, or `You`.
- **Taking over.** Click into the terminal and type. The terminal is then **yours**: the agent's next command is refused with a reason (§7.3) until you press **Hand back**. Ctrl+C reaches the program as usual. Whether plain typing should take it, or only an explicit Take over, is open (§11).
- **The phone watches only.** On a phone, the same card shows the same terminal, streamed from the PC that runs it. It shows **View only** and has no keyboard. It shows the **live screen only**. It can't scroll the PC's grid or select in it, because scroll and selection belong to the pane, not to a viewer (§6.2). A **History** button fetches the scrollback as plain text, which the phone can scroll and copy from on its own. If the PC is offline, it says so (§6.4).
- **Suzuri stays the terminal you live in.** These are small, task-scoped terminals. An **Open in suzuri** action is a later slice (§10, S8).

## 2. Architecture: where the shell runs

| Option | How | What it costs |
|---|---|---|
| **A. rsz-term inside Irori** (recommended) | Irori links `rsz-term` as a git dependency and owns each PTY. Frames go to Irori's own window over a Tauri `Channel`, and to the phone over the mesh. | One more native dependency, with a PTY half that is a stub on mobile (§2 below, §9). Suzuri has to make the changes in §3. |
| B. Spawn the suzuri app | Irori starts `suzuri` with a cwd and a command. | It opens a separate window, so nothing is inline. Irori never sees frames, can't stream to the phone, and agents can't drive it. Suzuri has to be installed. |
| C. IPC to a running suzuri | Suzuri serves panes over loopback HTTP (its bridge, tiny_http, `suzuri:app/src/bridge.rs:1-15, 31`, already does `submit`). Irori's Core would be the client and relay the frames itself. | Suzuri has to be running. The bridge has no frame-streaming endpoint today (it lists status, snapshot, diag, logs, submit, layout and call, `suzuri:app/src/bridge.rs:6-15`), so one would have to be added. Suzuri's pane tree lives in its webview (`suzuri:app/src/bridge.rs:13-15`), so a headless pane would be new work anyway. Two apps share pane lifetime, so a suzuri restart kills a card's terminal. |

**Recommendation: A.** rsz-term was built for this. Its own doc says "no UI, no Tauri. A `Pane` owns one shell … and hands binary frames to a sink" (`suzuri:crates/term/src/lib.rs:1-2`), and its manifest has no Tauri dependency (`suzuri:crates/term/Cargo.toml`). A is the only option where a terminal can live inside a card, keep running with no window open, and stream to a phone.

Decision 1 says Irori "is not grown inside suzuri; suzuri stays the terminal" (`docs/decisions.md:5`). That row is already stale on its stack: it still reads "Wails v3 + Go", and no row records the Rust/Tauri rewrite (a grep of decisions.md for Rust or Tauri finds nothing). A also goes the other way: suzuri's core grows inside Irori, and suzuri the app is untouched. §11 Q1 proposes one new row that records both.

**Where it sits in Irori:** a new crate, `crates/term` (`irori-term`), owned by the **Core**, not by the window.
- The Core already runs with no webview (`serve --headless`, architecture.md:151-153). A phone streams from the Core (§6), and an agent's MCP call reaches the Core (§7). So the PTYs must outlive any window.
- `crates/window` gets thin `term_*` commands over it (§5).
- `irori-term` has two halves, and **the same public API on every target**:
  - The **viewer half** (the `term/` stream client, the fan-out, `TermMsg`) builds everywhere, because the phone is a full Core that has to receive streams and hand frames to its own webview (`app/src/lib.rs:104-143`).
  - The **PTY half** (rsz-term, `Pane`, `term_run`) is real on desktop and a stub module on Android and iOS that returns "not on this device". This follows the voice crate, which keeps one API and swaps in an `other` engine module per target (`crates/voice/src/lib.rs:142-150`), with per-target dependencies (`crates/voice/Cargo.toml:28-49`). Only rsz-term itself goes under `[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]`.
  - Gating only the dependencies would not be enough. Window commands are one cfg-free `collect_commands!` list that `cargo xtask bindings --check` exports (`crates/window/src/commands.rs:1074-1076`). With the stub, every `term_*` command exists on every target and the bindings stay the same.
  - This keeps the Android check green (`scripts/check.sh:28-61`; #220 broke it once). §9 says where that check actually runs.

## 3. The rsz-term API Irori uses, and what suzuri changes

**Used as it is** (`suzuri:crates/term/src/pane.rs`):

| Need | Call |
|---|---|
| Open | `Pane::spawn(PaneConfig { cols, rows, command, cwd, env, scrollback, plain_shell, cursor }, sink)` (pane.rs:119-137, 262). Irori always passes `command: None` and `plain_shell: false` (§7.2). |
| Output | `Sink = Arc<dyn Fn(Out) + Send + Sync>` (pane.rs:116). `Out::Frame`, `Title`, `Bell`, `Clipboard`, `Shell`, `CommandDone`, `Exit`, `Progress`, `Notice`, `NoticeClose`, `NoticeAlive`, and the image messages `ImageData`, `ImageDrop`, `Placements` (pane.rs:36-61) |
| Input | `write`, `paste`, `mouse`, `scroll`, `resize`, `set_focused`, `redraw`, `ack(seq)` (pane.rs:461-504, 579-595), and `reply(bytes)` (pane.rs:485) to answer `NoticeAlive` |
| Agent commands | `submit(line) -> Submit` (pane.rs:373) today; Irori uses S-10's non-queuing `try_submit_as`. Irori does **not** use rsz-term's queue (`flush_queue`, `clear_queue`, `queue_len`, pane.rs:394-412); see §7.2. `Out::CommandDone { exit, command }` (pane.rs:44, 690-700) |
| Reading | `screen_text`, `history_tail(n)`, `cwd()` (pane.rs:524, 544-551) |
| End | `close()`, then `join()` (pane.rs:513, 601) |

**Notices.** `Notice` and `NoticeClose` (OSC 777/99) become Irori toasts on the device that runs the pane, tagged with the card. `NoticeAlive` is always answered with `Pane::reply(protocols::alive_reply(..))` (pane.rs:51, 483-487), as suzuri does (`suzuri:app/src/main.rs:143-148`), even when no window is open, so OSC 99 queries never hang.

**Rules the embedder must follow:**
- **Ack frames.** Normally one frame is in flight at a time, and the next waits for `ack(seq)` or for `ACK_TIMEOUT`, 250 ms (pane.rs:889-891, used at 910-917). **Exception:** for 100 ms after any `write()` (`INPUT_WINDOW`, pane.rs:893-896, 911-912; `input_at` is set at pane.rs:463), frames skip both the pacing and the one-in-flight limit. Every `submit` goes through `write()` (pane.rs:457), so each agent command and each burst of typing produces unacked frames. The Core acks on the viewer's behalf (§6.2), and its queues must expect these bursts.
- **Call `close()`.** There is no `impl Drop for Pane`; a grep of `suzuri:crates/term/src` finds none. A dropped pane keeps its shell and threads alive. `join()` must not be called from inside the sink: `Exit` is sent on the actor thread (pane.rs:1100), so joining there would make the actor wait on itself.
- **`join()` is unbounded.** The actor exits only after reader EOF, and then waits only 3 s more for the child (pane.rs:1081-1093). The `rsz-wait` thread blocks in `child.wait()` (pane.rs:341-347), and a reader that never sees EOF keeps the actor looping. So Irori joins with a deadline and abandons threads past it (§6.3).
- **The sink must not block.** Four threads call it, sometimes at the same time: reader, actor and frame (named in pane.rs:3-13), plus `rsz-fg` (pane.rs:349-354, 723-725). `irori-term`'s sink only pushes into a bounded queue.
- **Mind the sink cycle.** The sink usually holds `irori-term`'s pane map, and the map holds the `Arc<Pane>`. That is a cycle until the `Exit` handler removes the pane, as suzuri's does (`suzuri:app/src/main.rs:129-133`). Since `Exit` runs on the actor thread, the last `Arc<Pane>` can be dropped there.
- **Scroll and selection are the pane's, not a view's.** `Pane::scroll` moves the engine's one display offset (pane.rs:473-476), and frames carry only the visible rows plus that offset (`suzuri:crates/term/src/frame.rs:5-9`; `suzuri:ui/frame.js:104-105`). Selection is engine state (pane.rs:553-571) that reaches frames as `A_SELECTED`, and any selection makes every frame full (`suzuri:crates/term/src/engine.rs:443-446`). `write()` and `insert_block` scroll to the bottom and clear the selection (pane.rs:462; engine.rs:601-602). So two local views of one pane share scroll and selection, and every agent command resets both. §6.2 says what remote viewers do instead.

**What suzuri changes so Irori can depend on it.** Suzuri itself benefits from each.

| # | Change | Why |
|---|---|---|
| S-1 | **Publish the ConPTY patch so a dependent can use it.** Irori adds `portable-pty = { git = "https://github.com/StephenSHorton/suzuri", rev = "<sha>" }` to its own `[patch.crates-io]`. Cargo should find the package under `vendor/portable-pty`: its manifest is a standalone, normalized one with no workspace inheritance (`suzuri:vendor/portable-pty/Cargo.toml:12-16`), even though it isn't a workspace member (`suzuri:Cargo.toml:3`). Verify this in S0. If Cargo doesn't pick it up, add the package to suzuri's `members`. Add a line to suzuri's README saying embedders must do it. | A `[patch]` applies only to the workspace that declares it (`suzuri:Cargo.toml`, last lines). The vendored comment ties stock portable-pty's flag failures (Ctrl+C read as a character, rows one off) to **the bundled OpenConsole** (`suzuri:vendor/portable-pty/src/win/psuedocon.rs:88-93`), which suzuri releases ship (`suzuri:scripts/release/build-windows.ps1:35-36`, `conpty.ps1`). Passthrough is off by default and "changed nothing on the inbox conhost" (psuedocon.rs:104-108), which contradicts the workspace comment that mentions passthrough mode (`suzuri:Cargo.toml`, last lines). Irori would start on the inbox conhost, a path suzuri releases don't use, so S1 checks stock vs patched flags there (§10). |
| S-2 | **`impl Drop for Pane` that calls `close()` and never joins.** | A dropped pane leaks a shell and five threads (§3 rules). The last `Arc<Pane>` can be dropped on the actor thread (§3 rules), so a joining `Drop` would deadlock. This is a safety net; Irori still closes panes explicitly. |
| S-3 | **`PaneConfig.term_program: Option<String>`**, defaulting to `suzuri`. | `TERM_PROGRAM=suzuri` is hard-coded (pane.rs:287). Irori wants `irori` so scripts and agents can tell where they run. |
| S-4 | **A per-pane theme**: `PaneConfig.theme: Option<ThemeSpec>` as an **owned** value, threaded through the engine (`Engine::new`, the snapshot path and `insert_block`'s block colour) in place of `palette::active()`. | The palette is one process-wide `static ACTIVE` (`suzuri:crates/term/src/palette.rs:583-601`), and `Active.theme` is a `&'static ThemeSpec` (palette.rs:576-581), so an `Option<Active>` could only pick one of suzuri's built-in themes, not Irori's colours. The engine re-reads the global on every snapshot and pointer-compares it to force a full frame (engine.rs:435-442, also 195, 199), and `send_block` reads it for the block colour (pane.rs:445). Frames carry resolved RGB (engine.rs:226-240), so colour is **per pane, not per viewer**: a phone in another theme sees the PC pane's colours. |
| S-5 | **One command's output, tied to the run that caused it.** `submit_as(line, label) -> RunId`, so the command block's header names the agent and the caller gets a token. `CommandDone` gains `run: Option<RunId>` (None for a command the person typed) and `output: String`. The output range starts at the block's insertion row (`send_block` / `insert_block`, pane.rs:427-458) and ends at the next done or prompt mark. It is recorded with the engine's existing cursor marks (`Engine::mark_cursor` / `marks`, engine.rs:112-115, 708-714), **not** a start mark. | §7 returns a command's output to the agent. Today only `screen_text` and `history_tail` exist (pane.rs:544-551), and they can't tell where one command's output ends. `CommandDone` carries only `{exit, command}` (pane.rs:44, 690-700) and fires for any command, including ones the person types; on pwsh it fires whenever the Get-History id changes (`suzuri:crates/term/src/shell.rs:294`). PowerShell, the default shell on Windows (pane.rs:243-247), has **no start mark**: its bootstrap only emits the done and cwd marks from the prompt function ("PowerShell has no preexec", shell.rs:291-294). The marks are also stripped before the engine sees them (shell.rs:26-40; pane.rs:641), so nothing records their grid rows today. |
| S-9 | **A keep-the-prompt mode, which is a redesign of the block and echo path**, not only a bootstrap flag. `PaneConfig.keep_prompt: bool` makes the bootstrap keep the person's prompt while still emitting the OSC 7878 and 7879 marks; for pwsh it **wraps** the existing `prompt` function (oh-my-posh and the like) instead of replacing it (shell.rs:294). The block path then needs one of two routes: **(a)** no command block; the agent label goes on its own line above the prompt, and the shell's own echo is left alone; or **(b)** a block without echo suppression. | The bootstrap blanks the prompt because suzuri's input bar owns the command line (`suzuri:crates/term/src/shell.rs:4-8`; zsh `_rsz_quiet_prompt` :225, pwsh prompt :294). Irori has no input bar, so without a prompt, taking over means typing blind. But command blocks assume the cursor sits on a blank prompt row: the block goes "just above the cursor's line … while the cursor stays on the row the shell … expects" (`suzuri:crates/term/src/block.rs:5-10`). `send_block` writes the command's last line onto the cursor row (pane.rs:446-449; engine.rs:613) and arms the echo filter to swallow the shell's echo (pane.rs:454; `suzuri:crates/term/src/echo.rs:1-10`), which is tuned for "PSReadLine under OpenConsole" (echo.rs:31-33). With a visible prompt, `keep_cursor_line` becomes true (pane.rs:442), the prompt row scrolls up, and the cursor stays at the prompt's column while ConPTY repaints by absolute row. Nothing has tested that layout. S1 and S4 check it on screen (§10). |
| S-10 | **Non-integrated panes stop locking the queue, idleness becomes public, and a submit can refuse instead of queuing.** `busy()` ignores `awaiting` while `ShellStatus.integrated` is false. `Pane` gains `pub fn is_busy()`, `pub fn last_output_at() -> Instant`, and `try_submit_as(line, label) -> Submit` (with S-5's run id), which returns a new `Submit::Busy` and writes nothing when `busy()` is true, instead of pushing onto the internal queue. | `busy()` is also true for `QUEUE_IDLE` (900 ms, pane.rs:233) after any output (pane.rs:424, 645), including the prompt mark that ends a command, so a plain `submit` made on that prompt would always land in rsz-term's queue, which nothing in Irori drains (§7.2). Also, `busy()` checks `awaiting.is_some_and(\|n\| sh.prompts <= n)` before the quiet check (pane.rs:420). `send_block` sets `awaiting = prompts` on every submit (pane.rs:429-431), and `prompts` only rises on an OSC 7878 cwd mark (pane.rs:681-688). A shell without the bootstrap never sends that mark, so after its first submit `busy()` stays true for good. Suzuri never hits this, because its bar only submits when `integrated` is true (`suzuri:ui/app.js:276-277, 1152-1153`). `busy()` and `last_io` are private (pane.rs:195-196, 418), so Irori can't tell "quiet" on its own. |
| S-6 | **A frame version byte.** Header byte 11 is always 0 (`suzuri:crates/term/src/frame.rs:5-9`). Make it the format version, starting at 1. | The decoder lives in a second repo (§4). A mismatch should fail loudly instead of drawing garbage. |
| S-7 | **Fix the selection fill**: `fillRect(x0, …)` should be `fillRect(rx, …)` (`suzuri:ui/frame.js:176`). `x0` is a column index; `rx` is pixels (frame.js:169). | **Confirmed from the code:** any frame with a selection is full (engine.rs:443-446), so every row's `x0` is 0 and frame.js:176 fills from pixel 0. Selections that don't start at column 0 are always painted from the left edge. An on-screen check stays only as the regression test. |
| S-8 | **Tag releases.** Both crates (`rsz-term`, version 0.10.0, `suzuri:Cargo.toml:9`) and the view package (§4) get a git tag, such as `term-v0.10.0`. | Irori pins `rev` or `tag`, never a branch (§9). |

The dependency itself: `rsz-term = { git = "https://github.com/StephenSHorton/suzuri", tag = "term-v…" }` in Irori's `[workspace.dependencies]` (architecture.md:133). Both repos are public and MIT (`gh repo view`; `suzuri:Cargo.toml:11`, `Cargo.toml:43`). The crate's own dependencies are `alacritty_terminal` 0.26, `portable-pty` 0.9, `crossbeam-channel`, `parking_lot`, `base64`, `regex`, `unicode-width`, `imagesize` and `libc`, plus `windows-sys` on Windows (`suzuri:crates/term/Cargo.toml`). `crossbeam-channel` 0.5, `parking_lot` 0.12, `base64` 0.22 and `regex` 1 are already in Irori's `Cargo.lock`. `rsz-term` inherits `version`, `edition` and `license` from suzuri's workspace, and Cargo resolves that from the git checkout.

## 4. The view package (`suzuri-view`)

**What goes in:** the parts of suzuri's UI that draw one terminal and turn input into bytes.
- `frame.js` (the decoder and `Screen`), `boxdraw.js`, `gdi_cells.js`
- `keys.js` and `kitty.js`
- `pointer.js`, with its listeners removed on unmount (today it adds `window` listeners and never removes them, `suzuri:ui/pointer.js:93, 116`)
- `images.js`, made per view: today it uses `document`/`window` globals, including a page-level modal (`suzuri:ui/images.js:17, 156-173`)
- the pane DOM from `PaneView` (`suzuri:ui/app.js:165-218`): canvas, cursor, scrolled badge, link hover, image layer, IME textarea and preedit. **The package uses one textarea per view**, unlike app.js today, where "one hidden textarea holds keyboard focus for every pane" (app.js:33-35).
- its CSS, with padding as a variable instead of the fixed 6px/4px (`suzuri:ui/style.css:66-67`, app.js:486-487, 510-511)
- the Gohu font

**What has to be rewritten into a per-view controller.** Most per-terminal behaviour is not in the modules above. It is in `app.js` as module globals tied to one global `focused` pane:
- the rAF paint scheduler and ack (app.js:78-96, 550-563)
- `PaneView.onMessage` / `enqueue` / `flush` / `afterPaint` (app.js:240-271, 422-495)
- the keydown, IME and paste handlers on the single textarea (app.js:1116-1205)
- copy, paste and `clipShortcut` (app.js:1059-1106)
- wheel handling (app.js:526-541)

The package's `TermView` is that controller, one per mounted view, and suzuri's `app.js` is rewritten to use it.

**What stays in suzuri:** tabs and splits, the input bar and completion, settings and appearance, AI control, notes, the HUD, and the paint worker. Version 1 of the package paints on the main thread only, since the worker measured no faster (app.js:69-74). The worker still changes, though (next paragraph).

**Boundary:** the package touches no Tauri at all. Suzuri reads `window.__TAURI__` (app.js:21), which Irori turns off (`app/tauri.conf.json:11`). Instead the package takes a `host` object of callbacks, and each app wires that to its own IPC.

```ts
mount(el: HTMLElement, host: TermHost, opts?: { readOnly?: boolean; padding?: [number, number] }): TermView

interface TermHost {
  write(data: Uint8Array | string): void; paste(text: string | null): void
  resize(cols: number, rows: number): void; cellPx(w: number, h: number): void
  scroll(delta: number): void; ack(seq: number): void; redraw(): void
  focus(on: boolean): void; select(op: string, col: number, row: number, right: boolean): void
  copy(): Promise<boolean>; mouse(ev: MouseEv): void
  link(col: number, row: number): Promise<Link | null>; openUrl(url: string): void
  keyFilter?(e: KeyboardEvent): boolean          // the app's shortcuts go first
}

interface TermView {
  feed(frame: ArrayBuffer): void   // paints on the next animation frame, then calls host.ack(seq)
  message(msg: TermMsg): void      // title, exit, clip, img, imgdrop, imgpos, progress
  setStyle(s: { bg; selection; cursor; font: { family; px; url? } }): void
  setReadOnly(on: boolean): void   // no write, paste, mouse, scroll, select or copy reach the host;
                                   // the view shows the live screen only (history is a separate text view, §6.4)
  focus(): void; unmount(): void
}
```

- **Style and metrics per view.** The theme and font globals in `frame.js` (`VOID`, `SELECTION`, `FONT_PX`, `FAMILY`, `metrics`; frame.js:16-53) move onto each `Screen`, so two terminals on one page can differ and React remounts don't share state. That change reaches past frame.js: `cellMetrics()` / `metrics` (frame.js:53) is used by `pointer.js:29, 144` and `app.js:281, 482, 509`, and `paint-worker.js` imports `Screen`, `resetMetrics` and `applyStyle` and receives style messages (`suzuri:ui/paint-worker.js:14, 21, 111`). So pointer.js, images.js and the worker change with it, even though the worker stays in suzuri.
- **Packaging:** the package lives **inside `ui/`, at `suzuri:ui/view/`**, with its own `package.json`. Suzuri serves its frontend straight from `ui/` (`suzuri:app/tauri.conf.json:8`, `frontendDist: "../ui"`), with no bundler and no root `package.json` (its tests run as `node --test ui/test`, `suzuri:ui/test/boxdraw.test.mjs:1`). Tauri serves only `frontendDist`, so a sibling `packages/` folder couldn't be imported by `ui/` at runtime; `ui/view/` can be, with plain relative imports. It is plain ESM JS with a hand-written `index.d.ts`. Irori depends on it as **a packed tarball on the suzuri tag's GitHub release**, not a git dependency (S0c, card #367):
  - **bun can't take a git dependency on a subfolder.** bun 1.4.2 reads everything after `#` as the commit-ish. `github:…#<tag>&path:ui/view` and `git+https://…#<tag>&path:ui/view` ask codeload for a ref named `<tag>&path:ui/view` (404). `git+ssh://…#<tag>&path:ui/view` clones and then says "no commit matching". `#<tag>` alone works and pins the sha in `bun.lock`, but installs the whole repo root, which has no `package.json` for the view.
  - **The tarball route works.** In suzuri, `cd ui/view && bun pm pack` makes `suzuri-view-<version>.tgz` (just `package.json`, `index.js`, `index.d.ts` and the `files` list), and the release workflow attaches it to the tag. Irori adds `"suzuri-view": "https://github.com/StephenSHorton/suzuri/releases/download/<tag>/suzuri-view-<version>.tgz"`. bun writes the URL and a sha512 into `bun.lock`. `bun install --frozen-lockfile`, `bun run typecheck` (types from `index.d.ts` via `exports`), `bun run build` and `no-third-party.sh` all passed with a scratch package. A changed hash fails the frozen install ("Integrity check failed for tarball").
  - **Never re-upload an asset under the same tag.** GitHub's CDN served the old file at the same URL for minutes after `gh release upload --clobber`, and the lock's hash would then fail. A new view build always gets a new tag, which matches the versioning rule below (one tag moves the crate and the package).
  - **Release type:** a suzuri prerelease is safe: suzuri's updater reads `/releases/latest`, which skips prereleases, and the guest-helper download reads a different repo (`suzuri:internal/update/update.go:335`, `internal/guest/catalog.go:66`). A full release is what suzuri's updater offers, so attach the tarball to the real tag's release.
  - During development, use a `file:` path to a sibling checkout's `ui/view`.
  - A submodule would add a step to Irori's checkout and release flow.
  - A vendored copy, the way Irori vendors `netdev` (`vendor/netdev`, `Cargo.toml:179`), would drift from `frame.rs` without anyone noticing.
- **Versioning:**
  - The package version is the same as `rsz-term`'s, and both move in one suzuri tag. Irori bumps the crate `tag` and the package ref in one commit.
  - The frame version byte (S-6) is checked in `feed`. On a mismatch the view shows "terminal format v2, this app knows v1".
  - A fixture test in suzuri keeps the two sides in step: `frame.rs` writes frames into `testdata/`, and a `node --test` decodes them through `Screen` against a recording 2D-context stub, like the stub in `suzuri:ui/test/boxdraw.test.mjs:6-9`. No JS test of the frame decoder exists today.
- **In Irori:** a `<Terminal termId>` component in `components/term/` that mounts in `useEffect` and unmounts on cleanup. Its `host` is `lib/term.ts`, which calls the generated bindings through `lib/host.ts`, following the gateway rule (`frontend/src/lib/host.ts:1-5`). The font ships from `public/`; `font-src 'self' data:` already allows it (`app/tauri.conf.json:20`). The CSP needs no change.

## 5. The wire contract: Core, window and view

**Commands** in `crates/window`, each a `#[tauri::command] #[specta::specta]` named `<service>_<method>` (`crates/window/src/lib.rs:1-8`), collected in `builder()` (`crates/window/src/commands.rs:1074-1076`). They exist on every target; on a phone the PTY ones return "not on this device" (§2).

| Command | Arguments → result | Notes |
|---|---|---|
| `term_open` | `{ scope: Card(ref) \| Agent(key), cols, rows, cwd? }` → `TermInfo { id, owner, cwd, held_by }` | Opens the scope's terminal, or returns the one already running (§6.1). `cwd` is the person's choice when §6.1 finds no folder. |
| `term_attach` | `{ id, frames: Channel<…>, msgs: Channel<TermMsg> }` → `()` | Starts this view's streams with the cached state, then a full frame (§6.2). |
| `term_detach` | `{ id }` | The view went away. The pane keeps running. |
| `term_write` / `term_paste` | `{ id, data }` | The person typing. Calling it takes the terminal (§7.3). |
| `term_resize`, `term_scroll`, `term_mouse`, `term_select`, `term_copy`, `term_link`, `term_cell_px`, `term_focus` | mirror suzuri's `pane_*` commands (`suzuri:app/src/main.rs:234-518`, `pane_link` at :530; `pane_open` is :82-198) | Local window only. `term_copy` writes Irori's clipboard on the device the window runs on. |
| `term_ack` | `{ id, seq }` | Acks frames painted by this view (§6.2). |
| `term_hand_back` | `{ id }` | Ends a takeover. |
| `term_close` | `{ id }` | Kills the shell. Only the person can do this. |

- **Frames travel as raw bytes** on a `tauri::ipc::Channel`, as suzuri's channel mode sends them (`InvokeResponseBody::Raw`, `suzuri:app/src/main.rs:93, 112`). Irori has no `Channel` anywhere yet, and its JSON events are coalesced at 250-300 ms (architecture.md:309), which would make a terminal unusable.
  - tauri-specta rc.26 allows a `Channel` only as a whole command argument (`tauri-specta-2.0.0-rc.26/src/lang/js_ts.rs:153`). S0 has to check that `Channel<InvokeResponseBody>` exports to TS.
  - If it doesn't, `term_attach` is the one command registered beside the specta builder, and `lib/term.ts` wraps it by hand.
  - The phone needs the same raw Channel into its own webview (§6.4). Raw `InvokeResponseBody` on the Android and iOS webviews is unverified; S6 checks it first, with base64-in-JSON as the fallback.
- **Everything else is JSON** on the second channel, as `TermMsg`, a tagged enum exported through `.typ::<TermMsg>()`: `title`, `bell`, `clip`, `exit`, `done`, `progress`, `shell`, `img`, `imgdrop`, `imgpos`, plus Irori's `held { by }` (who is typing) and `offline`. The shapes follow suzuri's `Msg` (`suzuri:app/src/main.rs:50-76`), so the view package's `message()` needs no special case. Notices are not `TermMsg`s; they become toasts (§3).
- **Clipboard (OSC 52):** a program's clipboard write is shown as a toast with a Copy button. It is never written straight to the clipboard, because an agent's output is untrusted (§8).
- **Bindings:** `cargo xtask bindings` regenerates `frontend/src/gen/bindings.ts`, and check.sh runs `--check` (`scripts/check.sh:24`).
- **Events:** one new coalesced event, `term:changed` (`area:changed` names, `crates/window/src/events.rs:39-45`). It tells the card list which cards have a running terminal and who holds each, so tiles can show a badge. The bytes never go through events.

## 6. PTY ownership, lifetime and other devices

### 6.1 Who owns a pane

The Core's `irori-term` holds every pane in a map: `TermId → { pane, scope, owner_device, held_by, viewers, cache, runs }`.
- **Only the person opens a terminal.** `term_open` is a window command (§8). An agent's `term_run` never starts a shell; with no terminal open for its scope it is refused (§7.2). Whether agents may start one in their own agent scope is open (§11).
- **Card scope:** one terminal per card per device. A card has no local folder of its own: `Task.repo_key` is a normalized remote key such as `github.com/x/y` (`crates/coord/src/board/types.rs:337-338`), and capture only maps a folder to a repo (`repo_for(dir)`, `crates/capture/src/handle.rs:68`; `cached_repo(f, dir)`, `crates/capture/src/views.rs:186`), never the other way. So the cwd is chosen in this order:
  1. the cwd of an agent on this device that holds the card (`AgentCard.cwd`, `crates/presence/src/lib.rs:55-80`);
  2. else the most recent session cwd on this device whose repo (`repo_for`) matches `repo_key`;
  3. else a folder the person picks, remembered per device and card in a local, non-replicated row.

  When steps 1 or 2 find **several** checkouts or worktrees, the Terminal section shows them as a picker, newest first, and remembers the choice. When they find **none**, it asks for a folder (step 3). A card with no `repo_key` opens in the home folder.
- **Agent scope:** one terminal per agent session, with the session's cwd (`AgentCard.cwd`, `crates/presence/src/lib.rs:55-80`).
- **Grok agents have no agent scope.** They reach the Core through `irori mcp` as members (`irori_join`, `POST /v1/members`; `crates/mcp/src/lib.rs:5-11`), with no Claude session and no `AgentCard.cwd`. In v1 a Grok member can `term_run` only in a card terminal on a card it holds (§11 asks whether that is right).
- **A pane lives on the device that opened it.** Another device only watches it. Each desktop can have its own pane for the same card, so the PC and the Mac can each hold one. The card page lists **one tab per device-owned terminal**: this device's first, then the others, labelled by device name and marked View only. `term-list` is asked of every connected device of yours (§6.4). The phone shows the same tabs.

### 6.2 Viewers and flow control

The rsz-term pane has one sink, and its damage tracking is used up by that one sink (the damage reset is `suzuri:crates/term/src/engine.rs:466`; :465 is the full-frame check). So the Core fans each frame out itself:
- **Every viewer gets every frame in order,** so the damage is the same for all of them.
- **A new viewer gets the cached state first.** `redraw()` only forces a full grid frame (pane.rs:498-504). Image bytes are sent once, before their first placement (`Out::ImageData`, pane.rs:55-56), and title, progress and shell status only when they change. Suzuri caches only titles (`suzuri:app/src/main.rs:35-36, 114-116`). So `irori-term` caches, per pane, the live `ImageData` set (pruned on `ImageDrop`), the last `Placements`, title, progress and `ShellStatus`, and replays them to a new viewer before the `redraw()` frame. This covers the phone, a reopened card page and a resynced stale viewer.
- **The Core acks the pane, not the viewers.** It acks once the local window has acked, or straight away after fan-out when only remote viewers are attached. With no viewers at all it doesn't ack, so `ACK_TIMEOUT` (pane.rs:889-891) paces the pane at about 4 frames a second, except in the 100 ms after each write, when frames go out unpaced (§3 rules). Every `term_write` and every `term_run` submit opens such a window, so the queues below must absorb a burst per command. A slow phone never holds back the PC.
- **Scroll and selection stay with the PC's window.** Because they are pane state (§3 rules), remote viewers never call `scroll` or `select` on the pane. A remote viewer sees whatever the PC's view shows, which is the live screen unless the person on the PC has scrolled back. Scrollback for the phone is the text-only **History** request (§6.4), built from `history_tail` and `screen_text` (pane.rs:544-551). Per-viewer offset and selection inside rsz-term would be a further suzuri change, not in this design.
- **Each remote viewer has a bounded queue** of 64 slots, following the architecture's rule for subscriber queues (architecture.md:162-165). When a queue fills, the Core drops what is queued and marks the viewer stale. The viewer counts as caught up once the PC's write of the last record queued for it completes on the stream; it then gets the cached state and a full frame. All other viewers get that full frame too, which costs one larger frame.

### 6.3 Lifetime

| Event | What happens |
|---|---|
| A card's page closes | `term_detach`. The shell keeps running and so does its scrollback. |
| A card is done or archived | The person is asked "close its terminal?". Irori never kills a shell on its own while a program is running in it (`ShellStatus.program`, pane.rs:65-78). |
| The agent session ends | The agent's terminal stays open with its history and a "session ended" line, until the person closes it. |
| The shell exits | `Out::Exit` (pane.rs:45). The view shows the exit code and a **Restart** button, which opens a new pane with the same scope and cwd. |
| Irori quits, or **Restart to update** | Before either, Irori lists the terminals where `ShellStatus.program` is true (including an agent's running build) and asks "these are still running; quit anyway?". `restart_to_update` (`crates/update/src/lib.rs:700`) and quit both go through that check, in line with the wind-down flow (docs/design/wind-down.md). Then every pane is `close()`d and joined **with a deadline**, on a blocking thread (architecture.md:168-178); threads still running past it are abandoned so a stuck shell can't hang quit (§3 rules). Shells **don't survive a restart**: ConPTY and PTY masters belong to the process. |
| After the restart | A card that had a terminal shows "terminal ended when Irori restarted" and a Restart button. The marker that it had one is a per-device local row (scope, cwd), never replicated and holding no output, written at `term_open` and cleared at `term_close`. Keeping a shell alive across restarts would need a separate long-lived host process (§11). |

### 6.4 The phone and other devices

- **The phone is a full Core that runs no agents** (`app/src/lib.rs:104-143`, docs/android.md:3). It never gets the PC's webview, so it watches over the mesh.
- **Transport:** a new mesh stream label, `term/<id>`, inside `irori/mesh/1`. The receiver dispatches on the label prefix and serves only your own certified devices (`crates/mesh/src/files.rs:188-208`); today `serve_stream` serves only `blob/` streams and finishes them, so `term/` is a new long-lived kind. Away from home it goes through the home server's relay, or through n0's relays when that switch is on (decision 43, `docs/decisions.md:47`). Content is end-to-end encrypted either way.
- **Records.** Each record is `u8 type ‖ u32 len ‖ body`, under the same 8 MiB cap as sync (architecture.md:210):
  - PC → viewer: `0` frame (raw), `1` `TermMsg` JSON, `2` history text JSON (an answer to `history`).
  - viewer → PC: `16` control JSON, one of `attach`, `detach`, `resync` (after a gap on the viewer side) and `history { lines }`. These are the only records the PC accepts. There is **no input record type**; anything else closes the stream.
- **Discovery:** the phone must learn that a terminal exists without the terminal's output being stored. The PC adds `term: { id, device, held_by }` to the card's live, non-replicated state, the way `LastText` and `PID` are kept local (docs/design/v0.4-devices.md:581, 725). The phone asks each of your connected devices with `term-list` on its **own short stream** (`term-list/`, one JSON request, one JSON answer, then finished, like `blob/`), not on a `term/<id>` stream.
- **View only, enforced on the PC.** The `term/` server parses only the control records above, never input. The phone's read-only view is the UI half of that, not the guard.
- **What the phone shows:** the live screen as the PC's pane draws it, in the PC pane's colours (S-4: colour is per pane). **History** sends `history { lines }` and shows the answer as plain text the phone can scroll and copy locally. The phone's copy lands on the phone's clipboard, never the PC's (suzuri's `pane_copy` writes the host clipboard, `suzuri:app/src/main.rs:444-448`, which is why the phone never calls it).
- **No size control.** The phone never resizes the PTY. The view fits the PC's grid to the phone's width by scaling the font, and scrolls sideways if it can't.
- **Never stored and never replicated.** Terminal output doesn't become log rows. A miss in the scrubber would then be permanent on every device (v0.4-devices.md:599), and transcripts are never fetched (v0.4-devices.md:789). This is in line with decision 4's recommendation, still not signed off, that such content is "fetched on demand from the origin" (`docs/decisions.md:8`).
- **Your Mac:** sees the PC's terminals the same way the phone does, as view-only tabs next to its own (§6.1). Taking over from the Mac is open (§11).
- **Cap:** at most 4 `term/` streams per peer. The 4 comes from the Go design's blob limit (`MaxStreams = 4`, v0.4-devices.md:286, 919); the Rust mesh has no per-peer stream limit today (only the per-sha fetch lock, `crates/mesh/src/files.rs:220-226`, `node.rs:190`). So `irori-term` enforces the cap itself, refusing a fifth `term/` stream from one peer, and S6 tests it.

## 7. Agents running commands where you can see them

### 7.1 How a command gets to the terminal

| Path | How it works | Pros | Cons |
|---|---|---|---|
| **T1. A new Irori tool, `term_run`** (recommended first) | `term_run { command, card?, wait_s? }` → `{ run, exit, output, truncated, still_running }`. The Core finds the caller's terminal (the agent scope, or the card's when `card` is given). If none is open, it refuses with "ask the person to open the terminal" (§6.1). Otherwise it queues the run in `irori-term` and returns within its wait cap (§7.2). The mod lists the Core's tools at hello, so a new tool needs no change to the mod (docs/mod.md:6, 62). | Explicit and opt-in. Every agent that has Irori's tools gets it, including Grok through `irori mcp`, in card scope only (§6.1). | The agent has to choose it instead of Bash, so the tool description and the wake preamble have to say when to use it. |
| T2. Bash runs in the terminal | The mod's `tool.call` hook (register.tsx:718, already used by Pause) answers a Bash call with `{ result }` after running the command through `term_run`. A per-card **Run in terminal** switch turns it on. | Everything shows up without the agent changing anything. | Claude's Bash has its own rules: background runs, per-call timeouts, cwd carried between calls, parallel calls. One shared shell runs parallel calls one after another. It also bypasses Claude Code's own tool handling, and is bounded by the hook budget (`next.budget.remainingMs`, register.tsx:727). |
| T3. Read the transcript | Show each Bash call and its output from the transcript (`crates/capture`) as text in the terminal. | No PTY involved. | It isn't live and isn't a real shell, so nobody can take it over. |

**Recommendation:** T1 first (S4 in §10), then T2 as an opt-in switch per card once T1 has shown what goes wrong (S7). T3 isn't needed.

### 7.2 What `term_run` does

**Which panes.** Panes always open with `command: None` and `plain_shell: false`, which gives `default_shell()`: pwsh (or Windows PowerShell) on Windows, `$SHELL -l` elsewhere (pane.rs:242-253). The bootstrap and its marks apply only when `command` is `None` (pane.rs:271-277), and only to zsh, bash and pwsh/powershell started without `-c`/`-Command`/`-NoProfile`/rcfile flags (`suzuri:crates/term/src/shell.rs:153-200`). Those are the supported shells. A pane in any other shell (fish, cmd, a custom `$SHELL`) is **non-integrated**: `ShellStatus.integrated` stays false (pane.rs:66-69). The pane also uses `keep_prompt: true` (S-9), so the person still sees a prompt.

**The queue is Irori's, not rsz-term's.** Nothing in rsz-term drains its own queue. `submit` only pushes onto it while busy (pane.rs:380-386), and a queued line runs only when the embedder calls `flush_queue` (pane.rs:394-404), which suzuri does from a 300 ms `setInterval` in its webview (`suzuri:ui/app.js:389-399` → `pane_queue_flush`, `suzuri:app/src/main.rs:246-255`). Its entries record no caller, and past `QUEUE_MAX = 32` it silently drops the oldest (pane.rs:218, 234, 381-384); a dropped line never produces a `CommandDone`. So:
- `irori-term` keeps one queue per pane of `Run { id, caller, line }` entries, and submits through `Pane::try_submit_as` (S-10), which never queues inside rsz-term: when `busy()` is true it returns `Submit::Busy` and writes nothing. So rsz-term's own queue stays empty, and `flush_queue`/`clear_queue` are never needed.
- `busy()` stays true for `QUEUE_IDLE`, 900 ms, after the last output (pane.rs:233, 424), and the prompt mark that ends a command is itself output (`last_io` is bumped on every read, pane.rs:645). So the next run is started in two steps: the previous run completing (`Out::Shell` with `prompts > p`, and `CommandDone`) makes the next entry ready, and it is submitted once the pane has been quiet for `QUEUE_IDLE`. That wait is the shell's own idle rule, not a timer policy; a `Submit::Busy` in between (more output came) just waits for the next quiet point. Decisions 38 and 40's no-timers rule (`docs/decisions.md:42, 44`) is kept: nothing polls an idle pane with no queued run. Non-integrated panes are covered below.
- Every caller gets an answer. A run refused at a takeover, at `term_close`, at shell exit, or because the per-pane queue is full (a cap of 32, refused rather than dropped) is answered with the reason.

**Matching the result to the run.** `Out::CommandDone` fires for every command the shell finishes, including ones the person types (pane.rs:43-44), and on pwsh whenever the history id changes (shell.rs:294). So `term_run` never takes "the next `CommandDone`":
- With S-5, it matches `CommandDone.run == Some(id)`.
- Before S-5 lands, or as a cross-check: at submit, `irori-term` records `ShellStatus.prompts` as `p` (pane.rs:75-77). The run is complete when a `Shell` status arrives with `prompts > p`, and its result is the `CommandDone` that came just before that status (the reader sends `CommandDone` and then `Shell`, pane.rs:681-702).
- A command the person typed (`run: None`, or one finished while no run was in flight) is never returned to an agent.

**Before submitting.** Irori has no input bar, so after a takeover a half-typed line can still sit on the shell's command line, and `submit` writes the payload and a carriage return straight after it (pane.rs:452-455). `term_run` therefore clears the line first (Ctrl+U for zsh/bash; Escape for PSReadLine) when the screen's prompt row shows pending input, and refuses with "the command line isn't empty" if it can't.

**Non-integrated panes.** With S-10, `term_run` writes the line with `write()` and calls the run complete when `is_busy()` is false (no foreground child and output quiet for `QUEUE_IDLE`, pane.rs:418-425). It returns `exit: null` and says the shell isn't integrated, so there is no exit code. Its output is the text from the run's start row to the end of the screen. Here completion has to be checked, so `irori-term` polls `is_busy()` for that pane only. Without S-10, `term_run` is refused on non-integrated panes.

**The command block.** The command shows as a command block (#296, pane.rs:427-458) whose header names the agent (`submit_as`, S-5), or as an agent label line, depending on which S-9 route is chosen.

**Output:** the text of the run's range (S-5), with ANSI stripped, capped at 64 KiB as Revive's output is (`crates/window/src/roster.rs:51`), plus `truncated`.

**Returning within the API's deadline.** Tool calls go through the loopback API (`POST /v1/tools/<name>`, docs/mod.md:62). Once the body is read, a handler has a 30 s write deadline (`WRITE_TIMEOUT`, `crates/api/src/conn.rs:37`), and only streams lift it (`Deadlines::lift`, conn.rs:18). The mod's proxy is also limited by Claude Code's hook budget (`next.budget.remainingMs`, register.tsx:727), and `irori mcp` goes through the same API (`crates/mcp/src/lib.rs:5-7`). So `term_run` never blocks for the whole command:
- It waits at most `wait_s`, capped at 20 s, under the deadline. If the run finished, it returns `{ run, exit, output, truncated, still_running: false }`.
- Otherwise it returns `{ run, still_running: true }`. The command keeps running where the person can see it, and the agent is woken when it ends: the existing `wait` tool (`crates/tools/src/specs.json:986-987`) gains a `term_run: <run id>` condition, and the wake carries the result. `term_read { run }` returns a finished run's result, or the current screen text while it runs.
- Irori never Ctrl+Cs on its own.

### 7.3 The person taking over

- **Typing takes the terminal.** `term_write` sets `held_by = person` and sends `held` to every view. In card scope the shell is the person's own, so §11 asks whether plain typing should take it there, or only an explicit Take over, and whether finishing a command at the prompt should release it.
- **While the person holds it,** `term_run` returns right away with "<person> has the terminal; wait for them or ask on the card", and nothing is written. Runs waiting in `irori-term`'s per-pane queue are removed, and each caller gets that same answer. A run already in flight keeps running and returns normally.
- **Hand back** (a button, or `term_hand_back`) clears the hold. Irori does not hand the terminal back on a timer. The next agent run first clears any leftover typing on the command line (§7.2).
- **Taking over doesn't pause the agent.** It only keeps the agent from typing. To stop the agent itself, the person uses Stop or Pause (docs/design/stop-pause.md). Stephen can choose to have takeover also pause the card (§11).
- **Ctrl+C from the person** reaches the program like any key. A `term_run` waiting on that command gets its `CommandDone` with the exit code and returns normally.

## 8. Security and trust

- **Only the person opens a shell.** `term_open`, `term_write` and `term_close` are window commands, so the window is the only caller (`crates/window/src/lib.rs:1-13`). No loopback route takes keystrokes, so an agent can't type as the person. This follows the rule that only the GUI writes person actions (v0.4-devices.md:906-908; `crates/api/src/lib.rs:13-15`). `term_run` never opens a terminal: with none open, it is refused (§7.1). Letting agents start one in their own scope is a trust decision for Stephen (§11).
- **Agents run commands only through `term_run`,** which identifies the caller by its attach secret, never by its arguments (docs/mod.md:62). An agent can run in its own agent terminal. Whether it may also run in the terminal of a card it holds is open (§11): that is the person's own shell, which may be inside a REPL, an ssh session or a logged-in CLI, so it is a **new agent permission**. If allowed, it sits under decision 23's per-board autonomy switches (`docs/decisions.md:27`), off under **Ask me**. An agent can never run in another agent's terminal or in a card it doesn't hold.
- **An agent's own terminal adds no new power.** It runs as the same user, in the same folder, with what the agent's own Bash can already do. (This holds for agent scope only; card scope is the permission above.) To keep that true, `term_run` uses the same per-folder workspace-trust gate as Revive (`TrustSet`, `crates/window/src/roster.rs:500-552`): in a folder Claude doesn't trust, `term_run` is refused. `TrustSet` lives in the window crate today, and `term_run` is a Core tool, so it moves to a crate the Core can reach (`presence`, or `irori-term` itself).
- **Output is untrusted.** What `term_run` returns to an agent is wrapped in the `trust="untrusted"` fence, as peer text is (v0.2-talk.md:95). OSC 52 clipboard writes need a click (§5). Links open only on Ctrl/Cmd+click, as in suzuri (`suzuri:ui/pointer.js:44, 52-57`).
- **Shared boards never show a shell to another person.** A terminal is never streamed on `irori/board/1`, which is the only protocol other people's devices reach (architecture.md:243-246). It goes only to your own certified devices on `irori/mesh/1`. On a shared card, the Terminal section shows only to the person whose device runs it, and another person's agent can't call `term_run` on it. Showing a terminal to other people would need its own decision (§11).
- **Pause covers it.** While a card is paused, `term_run` is refused like any other non-Irori tool. It is an Irori tool, so the mod's gate lets it through (stop-pause.md:36-39); the Core refuses it instead.
- **Environment:** a pane inherits the Core's environment, which has no Irori secrets in it. `irori-term` sets `IRORI_TERM_ID` (and `TERM_PROGRAM=irori` with S-3) and nothing else. It never passes the API token or the attach secret.

## 9. Third-party and dependency rules

- **`scripts/no-third-party.sh` stays green.** That check is about discovery and relays: `pkarr`, mDNS and `swarm-discovery` crates, and `presets::N0` and `RelayMode::Default` in code (no-third-party.sh:13-26). rsz-term pulls in none of them. The terminal stream rides the existing mesh, so it adds no relay, broker or STUN beyond decision 43.
- **Crates are pinned once** in `[workspace.dependencies]` (architecture.md:133). rsz-term is pinned to a suzuri tag, and portable-pty is patched to the same suzuri rev (S-1). The rule is tag or rev, never a branch. The existing whisper-rs-sys patch doesn't follow it yet: it pins `branch = "fix/cross-compile-target-os"` (`Cargo.toml:175`), an inconsistency worth fixing separately.
- **Mobile builds** compile `irori-term` with the PTY stub (§2). check.sh's Android `cargo check` is what proves it, but it runs only when an NDK and the `aarch64-linux-android` target are installed and is skipped otherwise, and it checks only `-p irori-app --lib` (`scripts/check.sh:29-61`). So every slice that touches `irori-term` runs check.sh on a machine with the NDK (the Mac). There is no iOS check, so iOS stays unverified unless an `aarch64-apple-ios` `cargo check` is added.
- **Licences:** suzuri is MIT, as Irori is. alacritty_terminal is Apache-2.0, and portable-pty is MIT. Both are compatible with an MIT app. Run the licence list once in S1.
- **Front end:** no xterm.js or other renderer. The view is Stephen's own package (§4). The Gohu font is bundled locally, with no font CDN.
- **ConPTY host.** The patched portable-pty loads a `conpty.dll` that sits next to the exe in preference to kernel32 (`suzuri:vendor/portable-pty/src/win/psuedocon.rs:48-64`). Suzuri releases bundle `conpty.dll` and `OpenConsole.exe` (`suzuri:scripts/release/build-windows.ps1:35-36`), and its flag choice and echo filter were tuned against OpenConsole (S-1). Irori starts on the inbox conhost, which suzuri releases don't use. S1's on-screen checks decide: if Ctrl+C, row alignment or block echo misbehave there, Irori bundles OpenConsole the way suzuri does.

## 10. Build slices

Each slice ends with `scripts/check.sh`, which already covers the frontend (install, `gen-models --check`, build, typecheck, lint and test, `scripts/check.sh:10-19`). Slices that touch `irori-term` run it on the machine with the NDK (§9). There is no CI.

| # | Slice | Done when |
|---|---|---|
| S0 | **Spikes, with throwaway code**, before the suzuri extraction and the Irori work: (1) the portable-pty git patch from a suzuri subdirectory (S-1); (2) `Channel<InvokeResponseBody>` through tauri-specta (§5); (3) the bun git dependency with `&path:ui/view` (§4). | Each one works, or its fallback (members entry, hand-registered `term_attach`, release tarball) is chosen. |
| S1 | **One live terminal on a card, desktop.** **Suzuri half:** S-1 README note, S-2 `Drop` (close only), S-3 `term_program`, S-6 version byte, S-7 selection fix, S-9 `keep_prompt` (with the chosen block/echo route), S-10 public idleness, and an S-8 tag. Extract `ui/view/` (suzuri's `ui/` imports it, and the `app.js` per-view controller pieces in §4 move into it) and add the frame fixture test. **Irori half:** `irori-term` (PTY half stubbed on mobile), `term_open`/`attach`/`write`/`resize`/`ack`/`close`, a raw frame `Channel`, and `<Terminal>` in a new Terminal section on `CardPage`, with the card folder rule (§6.1). | Suzuri runs as before, and its fixture test decodes `frame.rs` frames. On the PC, opening a card picks the right folder (holder's cwd, or the picker when there are several worktrees, or the folder prompt when there are none), shows a prompt, and typing `git status` shows live output. Resizing the panel reflows it, and closing the card leaves the shell running. **On screen, on the inbox conhost, stock vs patched ConPTY flags:** Ctrl+C stops a running program, rows line up with the engine, and a visible prompt with an agent-style block doesn't misdraw, on pwsh with PSReadLine and on zsh or bash. Bindings `--check`, `no-third-party` and the Android `cargo check` all pass. |
| S2 | **Lifetime:** detach and reattach with the cached state replayed (§6.2), exit and Restart, the quit/update running-program prompt, close-with-deadline on quit, the "had a terminal" local marker, `term:changed` and a badge on the card tile. | Closing and reopening a card shows the same scrollback, title and images. `exit` shows Restart. Quitting with a running program asks first; quitting Irori leaves no `pwsh` behind (checked in Task Manager), even with a shell that ignores EOF. |
| S3 | **Agent-row terminal:** a toggle in `People`, and the agent scope with the session's cwd. | An agent's row opens a terminal in that agent's folder. |
| S4 | **`term_run`** plus `term_read` and the `wait` condition; S-5 in suzuri; `irori-term`'s per-pane run queue; result matching by run id and prompt count; clearing a leftover line; command blocks with the agent's name; the trust gate; the untrusted fence. | In an agent-check scenario, an agent runs `cargo --version` through `term_run`; the person sees it in the terminal, and the agent gets the exit code and output. A command that runs **longer than 30 s** returns `still_running`, and the agent is woken with the result when it ends. Two agents' runs queue and each gets its own result; a command the person types between them is never returned to either. A refused run (no terminal open, untrusted folder) says why. On screen, the S-9 layout holds up on pwsh+ConPTY and on zsh or bash. |
| S5 | **Takeover:** `held_by`, the label, Hand back, and agent calls refused while the person holds the terminal. | The person types during an agent's run. The agent's next `term_run` gets "has the terminal", and queued runs get the same answer. After Hand back, with a half-typed line left on the prompt, the agent's next call runs on a clean line. |
| S6 | **Phone view-only:** first check that raw Channel frames reach the Android and iOS webviews (base64 fallback); then the `term/` mesh stream with typed records, `term-list/`, fan-out with per-viewer queues and catch-up, the cached-state replay, the 4-stream cap in `irori-term`, a read-only view, History text, fit to width, and the multi-device tabs. | The phone shows the PC card's terminal live over the home relay, including after a burst of agent output. History shows scrollback as text and copies to the phone's clipboard without moving the PC's view. Typing on the phone does nothing, and the PC closes a stream that sends any record other than the control records. A fifth `term/` stream from one peer is refused. The PC going offline shows "offline". The Mac shows the PC's terminal as a view-only tab beside its own. |
| S7 | Opt-in **Bash runs in the terminal** (T2), a per-card switch in the mod's `tool.call` hook. | With the switch on, an agent's plain Bash calls show up in the card's terminal and return the same output as before. With it off, nothing changes. |
| S8 | **Per-pane theme** (S-4) and the light/dark theme; **Open in suzuri**. | A light and a dark Irori theme each colour the terminal correctly. |

S0 comes first, and everything else follows S1. S2 and S3 don't depend on each other. S4 needs S-5 from suzuri. S6 needs S2's fan-out and cache.

## 11. Open questions for Stephen

1. **A new decision row.** Decision 1 still reads "Wails v3 + Go" and no row records the Rust/Tauri rewrite (`docs/decisions.md:5`). Should one new row record both: Irori is Rust + Tauri, and suzuri's core (`rsz-term`) runs inside Irori while suzuri the app stays the terminal?
2. Should shells survive an Irori restart or update? That would need a separate long-lived terminal host process.
3. One terminal per card, or several (tabs) on one card?
4. Should a card terminal be the person's shell, or should it start empty until an agent runs something in it?
5. Should taking over also pause the card's holders (Stop and Pause), or only keep them from typing?
6. Should agents' plain Bash calls run in the terminal by default (T2), or only `term_run` (T1)?
7. Should your Mac, as well as the phone, be able to take over a PC's terminal, or only watch it?
8. Should other people on a shared board ever be able to watch a terminal? The design says never, unless you decide otherwise.
9. Should the phone be able to type later, as a separate opt-in, or always stay view-only?
10. **Who starts a shell?** The design says only you: `term_run` is refused when no terminal is open. Or may an agent start one in its own agent scope (and, if Q11 allows, on a card it holds)?
11. **May agents run in card terminals at all,** or only in their own agent terminal? A card terminal is your shell, perhaps inside ssh or a logged-in CLI. If they may, should it sit under decision 23's per-board autonomy switches, off under **Ask me**?
12. **Does typing take the card terminal?** Options: any typing takes it (current design); only an explicit **Take over** button does; finishing a command at the prompt releases it; or agent terminals are takeover-only and card terminals are yours only.
13. **Which device's terminal does a card show?** The design shows one tab per device that has one, this device's first. Is that right, or should a card show only one?
14. **Grok agents:** in v1 they can `term_run` only in a card terminal on a card they hold, since they have no session cwd. Should they instead give a cwd when they join, or get no `term_run` in v1?
