# Irori architecture plan (draft, 2026-10-25)

> Output of a design panel: four independent architectures, each adversarially critiqued, then synthesized. "hub" below = Irori. Decisions made since are in [../decisions.md](../decisions.md); they override this draft where they differ.


## 1. Verdict
Yes, it can be built, and most of it already exists in your repos:
- suzuri has the workspace model, the MCP channel push, the iroh engine and a release pipeline for both Mac and Windows.
- tsukumo has the routing logic, already separate from Discord, with tests.
- toru shows how Windows installs and updates should work.

What remains is mostly porting and wiring that code. The new parts are the delivery state machine, the replication of the event log between devices, and capture. The one real risk is that waking a Claude Code session depends on the undocumented mod API (`$.prompt.submit`). Prove it on day 1, including on Windows, and keep the passive and resume fallbacks. Delta-style live replication of the worktree is out of scope: code still moves through git, and the hub replicates the conversation, the edits and who made them.

## 2. The one app: a new Wails app, built mostly from suzuri's code
It should be a new app, not grown out of suzuri, for three reasons:
- **The UI can't live in suzuri.** Its default UI on main is classic ebiten/Charm, and its chrome rule is "No React. No HTML. No Chromium." mokei is React/shadcn, so it needs a webview.
- **The hub has to run when no terminal is open.** It must wake Claude desktop Code-tab sessions, grok-build, and Windows sessions while suzuri is closed. suzuri also has a history of crashes and Windows freezes.
- **Wails has the pattern you like.** One Go binary is both GUI and MCP server (mead/wc3-forge).

It is one binary, `hub` (name it later). It is the tray GUI, the daemon (`hub serve --headless`), the stdio MCP shim (`hub mcp`) and the mod's stream (`hub attach`).

This doesn't add a new stranded tool, because the total number of tools goes down:
- suzuri loses `internal/workspace`, the `workspace_*` tools and `internal/workspacesync`, on a fixed milestone.
- aizuchi's Scratch Pad is retired.
- tsukumo, tsukumo-archive and ai-office are archived.

suzuri stays as the terminal, and its panes report their state to the hub through `aicontrol`.

## 3. What gets absorbed

| Repo | What moves into the hub | What happens to the old repo |
|---|---|---|
| tsukumo | `router/core.ts`, `lib/route.ts`, `take.ts`, `sweep.ts`, `ledger.ts`, `refs.ts`, ported to Go with their `*.test.ts` as the spec. Also DIRECT_LIMIT 8 messages per pair per 10 min, and the `claude-mod` skeleton (wake prompt, fencing, held messages, block/allow) | Discord code deleted. Archived at v0.4, along with tsukumo-archive |
| suzuri | `internal/workspace` model (members, availability, tasks, leases, session identity), the `mcpsrv/notify.go` channel push, the `libs/transfer` iroh engine (hato-core identity), `internal/update`, and `release.yml` | Workspace tools deleted at v0.2. Stays as the terminal and becomes a hub client |
| toru | Windows flow from `internal/update/service.go`: silent NSIS installer, `InstallOnExit`, `/NORELAUNCH`. Also the `UpdateBanner`/`useUpdater` frontend and the tray/autostart patterns, copied and adapted | Unchanged (it's a capture app) |
| aizuchi | Scratch Pad notes, the note_* tool names, and the decision/question/handoff/fyi intents | Scratch Pad and its MCP server retire. Meetings stay in aizuchi |
| knowledge-base | Read-only FTS search, `kb_search/kb_read`, and a "Promote to KB" button that only you can press | Stays the curated git repo |
| ai-office | Lessons only: attach real agents, no orchestrator, no server, no 3D | Archived |
| mokei | The whole UI (shadcn base-nova, `@mokei/*`, `applyTheme`) | Dependency |
| grok-build / rock | Clients through `hub mcp` with `--channels`, or ACP | Unchanged |

## 4. Architecture
```
 Claude Code ──mod──(hub attach, spawned)──┐      ┌── hub UI (Wails + mokei)
 grok-build/rock ──hub mcp (stdio)─────────┤      │
 passive: ~/.claude/sessions, *.jsonl ─────┼─► Core (Go services) ◄─ suzuri aicontrol
                                           │   SQLite event log + blobs
                                           │   router · deliveries · capture
                                           └── hub-net (Rust iroh sidecar) ◄─► other devices
```

**Core and discovery**
- One Go service layer (Agents, Threads, Deliveries, Tasks, Leases, Capture, Notes, KB, Devices). Wails bindings, loopback HTTP and MCP are thin adapters over it. Every action is visible in an activity panel, as in wc3-forge.
- Agents find the hub through `~/.hub/hub.json` (`%LOCALAPPDATA%` on Windows), which holds the port and token. This follows the suzuri aicontrol / wc3-forge lockfile pattern. There are no unix sockets.

**Claude Code connects through the mod only**
- `session.append` and `tool.call` feed capture.
- `turn.start` and `turn.complete` track whether a session has taken a message.
- `$.tool.register` adds the hub tools.
- `$.prompt.submit` wakes the session.
- The wake stream comes from `$.process.spawn` of `hub attach`. That is tsukumo's proven pattern without bun, and it avoids depending on long-poll `$.http.fetch`.
- No http hooks are stacked on top of the mod.

**Delivery**
- One delivery state machine (queued → delivered → engaged → answered, deferred, skipped) owns every way a message can arrive: mod submit, Stop-continue, context added at the next prompt, and the Grok channel push. Each item is consumed exactly once.
- Delivered text is always fenced as `<hub-message from=… trust=untrusted>`.
- `asUser:true` is allowed only from the local GUI.
- Messages to targets that can't be reached wait in a persistent inbox and are never dropped.

**Capture**
- Each edit is an event with the `structuredPatch`/`bashEditDiff` from the transcript, before/after blob hashes (git-compatible SHA) and `promptId`/`toolUseId`.
- Watching `.git/logs/HEAD` links each commit to the turns that produced it.
- Anything changed through Bash is labelled "heuristic". Grok reports through `hub_status` plus file watching, and gets a fidelity badge.

## 5. Cross-device sync
**Data**
- One SQLite database per device (`modernc.org/sqlite`, FTS5).
- Events are append-only rows `(device_id, seq, hlc, actor, kind, payload)`. Devices exchange `{device: maxSeq}` vectors and send each other the missing rows. Rows never change, so they never conflict.
- Large data (patches, transcript excerpts) are blobs fetched on demand. Full transcripts stay on the device that recorded them.
- Mutable state has one owner device:
  - The target session's device holds the take-lock for a message.
  - The holder's device owns a lease.
  - The device that created a note decides last-writer-wins for it.
  - Conflicts found when devices reconnect are shown to you, not merged automatically.
- Files are keyed by `repo_key` (root commit, or normalized remote) plus a POSIX path, so the Mac and Windows clones of a repo match.

**Transport**
- `hub-net` is a Rust sidecar forked from suzuri `libs/transfer` (iroh 1.0, hato-core identity). It talks NDJSON over stdio to the Go host, the same way suzuri drives `suzuri-transfer`.
- Pairing uses a pasted iroh ticket or a QR code, with a short confirmation string (SAS) and revoke. There is no hato-mailbox, so no server to host.
- Traffic goes direct or through n0's relays, which store nothing.
- Start the sync code from `9bdd319` on `feat/workspace-mcp-channels`. It was never merged into main, so it is not something to recover from main's history.
- An optional `hub serve --headless` on an always-on Windows desktop gives store-and-forward.

## 6. Install, releases, updates
- Start from suzuri's `release.yml`. It already builds Windows NSIS (user scope), a portable zip, a signed and notarized macOS dmg/.app.zip, and SHA256SUMS. Add the Wails build, the bun build of the mokei frontend, and `hub-net`.
- **Updater:** start from suzuri's `internal/update`, which handles both OSes. Adapt `heal_darwin.go`/`resign_darwin.go` to the Wails .app layout. On Windows use toru's approach: run the real NSIS installer on quit, not an exe swap.
- **Claude plugin:** publish it through the repo's `.claude-plugin/marketplace.json`. Code-tab sessions run the cached copy, so a version bump plus `claude plugin update` is needed; the Doctor tab flags a stale mod. Any change to settings.json is shown as a diff and never written silently.

## 7. Milestones (each one ships and retires something)
- **v0.1 See:** the shell, tray and autostart, with updates proven on Mac and Windows. Presence comes from `~/.claude/sessions/*.json` plus the transcript tail. Spike: does `$.prompt.submit` and `$.process.spawn` work on Windows?
- **v0.2 Talk (local):** the router port, the mod, the Grok channel wake, the inbox, tasks and leases. Retires: suzuri's `workspace_*` tools, and tsukumo on the Mac.
- **v0.3 Capture:** timelines, edit → turn → commit links, `decision_record`. Retires: digging through terminal scrollback.
- **v0.4 Devices:** hub-net pairing, replication, waking sessions on another machine. Retires: Discord. tsukumo is archived.
- **v0.5 Notes/KB:** import from aizuchi, reading whatever store the Tauri app persists (not just `notes.json`), plus KB search. Retires: aizuchi Scratch Pad.
- **v1.0 Delta extras:** anchored comments `(repo_key, path, base sha, range, hash + context)` that are re-anchored fuzzily, review worktrees via `git worktree add`, an opt-in revive through `claude --bg --resume`, and suzuri pane badges.

## 8. What NOT to build
- Claude-to-Claude messaging on one machine. Native ListAgents/SendMessage already does it, and with Remote Control it reaches sessions on other machines too. The hub only observes it through `session.send`/`session.receive`.
- Busy/idle detection. `sessions/<pid>.json` already has it.
- Transcript storage. The JSONL files are the source of truth, and the hub stores pointers and excerpts.
- Session revival. `claude --bg --resume` already does it.
- Plugin distribution. The marketplace already does it.
- An agent runtime of its own, or Loro/CRDT copies of files.

## 9. Open decisions for you
1. **Always-on machine:** recommend running headless on a Windows desktop.
2. **Auto-revive closed sessions:** recommend off by default, with opt-in per repo.
3. **Full capture syncing to other devices:** recommend off, so only summaries, edits and messages replicate.
4. **Grok capture depth:** recommend MCP self-reports for v1, with grok-fork posting hook events later.

## 10. Name ideas
- **Musubi** (結び): a tie or knot that binds things together. This is my recommendation.
- **En** (縁): a bond or fated connection.
- **Ito** (糸): thread, both as a conversation thread and as a string between agents.
- **Yoriai** (寄合): a village gathering.
- **Tsudoi** (集い): a meeting of people.