# The four candidate architectures (2026-10-25)

Each was written independently through a different lens, then critiqued by a separate skeptic agent. The synthesis is in [01-architecture-plan.md](01-architecture-plan.md).

## 1. Yoriai (寄り合い, "a gathering"): one local-first agent hub, with Loro and an iroh op-log at its core

Yoriai is one installable app for Mac and Windows. It replaces tsukumo, suzuri's Workspace, aizuchi's Scratch Pad MCP and ai-office. It does five jobs. (1) It records every Claude Code and Grok session on every machine: prompts, assistant turns, tool calls and decisions. Live hooks provide the signal and the session transcripts are the source of truth. (2) It records every edit between git commits as character-level CRDT operations, and each edit points back to the conversation turn and tool call that made it. (3) It anchors comments to code so they stay attached through later edits. (4) It lets one agent message another and wake it, on the same machine or a different one, using tsukumo's routing logic over the hub's own transport. Discord is not involved anywhere. (5) It syncs everything peer-to-peer between your Mac and Windows PCs over iroh 1.0, the same stack suzuri already ships. Data is split into two kinds. Facts that never change (turns, tool calls, messages, wakes) go into a signed append-only log per device. Shared state that changes (file shadows, comments, threads, tasks, leases) lives in Loro documents. Packaging copies the Toru and suzuri release pipelines: a Wails v3 shell with an installer, a DMG, GitHub Releases, background download and install on quit. Being faithful to Delta, the design keeps git as the ground truth. Yoriai is a layer between commits. It never replaces git and never writes into your working tree unless you ask.

Facts I checked in the code, not guessed: (a) The tsukumo repo on disk (~/projects/tsukumo) is the Bun/TS version. It already has a Discord-free path between chats on one machine: the `direct` event in src/router/core.ts and the `message_chat` tool, delivered through the mod's `$.prompt.submit`. That is the seed of the messaging design. (b) suzuri's main branch has a local Workspace: channels, members, tasks, exclusive path leases, presence, and `workspace_*` MCP tools that wake Grok through `notifications/claude/channel`. Its iroh P2P sync (commits c8e2691 and 9bdd319: an append-only jsonl merged by message id, plus Share/Join tickets) is only on the archive/glass-chrome branch. It is NOT on main. (c) Claude Code 2.1.310 keeps a live registry at ~/.claude/sessions/<pid>.json. Each entry has the sessionId, cwd, name, status busy/idle, peerProtocol and a messagingSocketPath under /tmp/cc-socks. `claude agents --json` returns the same data. `claude --bg --resume <id>` restarts an exited session in the background. In the transcripts, each Edit result carries filePath, originalFile, structuredPatch, promptId, uuid and parentUuid. That is exactly the link a "which turn made this edit" feature needs.

### Architecture

PROCESSES (one install, three binaries)
1. `yoriai` is the shell (Go plus Wails v3, taken from Toru). It owns the window, tray, global hotkey ("summon hub"), autostart, the updater, the installer hooks, and supervising the core. Almost no domain logic. It hosts the React UI, which talks to the core directly over a WebSocket on loopback, authenticated with a per-boot token. Headless mode, `yoriai --headless`, runs only the core plus the tray. That suits an always-on Windows box that acts as a runner or anchor.
2. `yoriai-core` is a Rust sidecar, following suzuri's libs/ rule "host talks over a stable NDJSON boundary, sidecar named <app>-<name>". Everything stateful lives here:
   - Store: SQLite (rusqlite, WAL, FTS5). It holds the event log, indexes, Loro snapshots as blobs, and peer version vectors.
   - CRDT engine: Loro 1.x.
   - Sync: iroh 1.0 plus iroh-blobs, with pairing code lifted from suzuri libs/transfer (hato-core identity, hato-code SPAKE2 short codes).
   - Capture ingest: a local socket (unix socket on macOS, named pipe on Windows) for hook events, a transcript tailer, and a filesystem watcher (notify crate).
   - Router: tsukumo's router/core.ts, route.ts, take.ts, sweep.ts, ledger.ts and refs.ts ported to Rust, together with their tests.
   - MCP server: Streamable HTTP at 127.0.0.1:<port>/mcp, plus a stdio shim `yoriai mcp` for clients that need stdio.
   - Attach streams: NDJSON, one per live agent session, the same shape as tsukumo's src/attach.ts.
3. `yoriai hook` is a tiny client mode of the core binary. Claude Code command hooks run it. It forwards stdin JSON to the core socket and exits within 50 ms. It fails open: if the core is down it appends to a spool file that the core drains later.

AGENT-SIDE PIECES
- The Claude Code plugin `yoriai` ships inside the app. On install and on every update, the app writes it to <appdata>/yoriai/claude-plugin. Like tsukumo's option A, it is registered through CLAUDE_CODE_PLUGIN_DIRS plus CLAUDE_CODE_PLUGIN_DIR_WATCH=1, never through a marketplace copy. tsukumo learned the hard way that marketplace caches go stale. Changing ~/.claude/settings.json requires your one-time consent in the UI. The plugin contains:
  - hooks.json: SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop, SubagentStop, Notification, PreCompact and SessionEnd, all calling `yoriai hook`.
  - A register.tsx mod module, ported from tsukumo's claude-mod/hooks/register.tsx with the Discord parts removed. It keeps the attach stream, `$.prompt.submit` for waking, the `$.ui` pane and toasts, `$.store` for hold and block state, and `$.tool.register` for the in-session tools.
  - The tools stay the same whether called from the mod or over MCP: message_agent, list_agents, defer, skip, claim_thread, release_thread, comment, set_status, task_*, lease_*, search_history and handoff.
- Grok and grok-build connect over MCP. Waking uses the same `notifications/claude/channel` mechanism that suzuri's internal/mcpsrv/notify.go and tsukumo's `bind` already use with `grok-fork --channels`.
- Any other MCP client gets read/post/search, but cannot be woken.

DATA MODEL
A. Immutable event log (facts, no merging needed). Each device D appends events with id (D, seq), signs them with its iroh Ed25519 key, and stores a hash link to the previous event. Event kinds:
   - session.start/end/status
   - turn.prompt (promptId)
   - turn.assistant (uuid, parentUuid)
   - tool.call/tool.result (tool_use_id)
   - edit.applied, which references a Loro change ID inside a file doc
   - msg (agent↔agent or human↔agent)
   - wake/route/take/defer/skip, which together make up tsukumo's state machine as events
   - decision (an explicit "we decided X", recorded by a tool call)
   - git.checkpoint (sha ↔ Loro frontiers)
   Syncing this log is trivial: one {device: maxSeq} vector per device, and a peer sends whatever the other side is missing. This is suzuri's "append-only jsonl, dedup by id" idea made rigorous with per-device seq numbers. Bulky payloads such as full tool outputs, screenshots and attached files go into iroh-blobs by hash. Events only reference them.
B. Loro documents (mutable shared state):
   - Mesh doc: devices, display names, trusted endpoint ids, settings.
   - Thread docs: a Delta-style conversation thread. Fields are title, participants (sessions plus humans), linked repo/branch/worktree, status and pinned decisions. A LoroMovableList holds ordered references into the event log.
   - Board doc: tasks and path leases (absorbing suzuri's tasks.json and leases.json semantics). Claims are compare-and-set through a single arbiter (see the risks).
   - Repo index doc, one per (repo identity, worktree): a LoroTree of repo-relative POSIX paths. Repo identity is the root commit sha plus the normalized remote URL, so your Mac clone and Windows clone of the same repo map to one repo.
   - File shadow docs, one LoroText per tracked file per worktree. Seeded from the git blob at HEAD, with line endings stored as LF and the file's EOL style recorded.
   - Comment docs: {anchor: {fileDoc, startCursor, endCursor, quotedText, commitAtCreation}, replies: LoroList, resolved, author}.
   - Presence uses Loro's EphemeralStore and is never persisted: who is viewing which file, agent busy/idle, cursor positions.
C. Edit provenance. Before applying an edit, the core calls Loro's setNextCommitMessage / commit origin with a JSON payload {device, harness, sessionId, promptId, toolUseId, turnUuid}, then applies the splices. To answer "which turn wrote this line", take each character's Loro op ID (getCursor gives the ID), look up its change (getChangeAt), and read the message. That gives the turn. It works across later concurrent edits, because Loro IDs never change.
D. Git as ground truth. A post-commit / post-checkout / post-rewrite hook, plus a .git/HEAD watcher, records git.checkpoint with the Loro frontiers. After a checkout, rebase or stash, the core diffs the worktree against the shadow and applies an `external:git` change. That change is attributed to git, not to any agent. After N checkpoints, the core takes a Loro shallow snapshot to cap history. Anything older is still described by git.

SYNC / TRANSPORT
- Each device runs one iroh Endpoint. Its node id is the device identity, with a 32-byte secret like suzuri's identity.secret. ALPNs are `yoriai/log/1` (event-log reconciliation), `yoriai/loro/1` (exchange version vectors, then `doc.export({mode: update, from: vv})`, then stream live updates) and `yoriai/wake/1` (low-latency delivery with an ack).
- At 2 to 5 devices, peers hold direct QUIC connections. Every device dials every other one, using iroh relays for NAT traversal: n0's public relays by default, or a self-hosted iroh-relay. iroh-gossip is unnecessary at this scale; iroh-docs is the fallback if the hand-written log sync turns out shaky.
- Pairing: on the existing machine, "Add device" shows a hato-code short code (the same style as Toru's `7-guitarist-revenge`). The new machine enters it. SPAKE2 then exchanges endpoint ids plus a mesh id, and both write the other into the Mesh doc's trusted list. After that, connections are refused from any endpoint not on the list.
- UI to core: the React UI holds the loro-crdt WASM copies of the docs it shows and swaps update bytes with the core over the loopback WebSocket. The same mechanism serves peers and the UI, so editing a comment in the UI is just one more Loro peer.

UI (mokei glass-on-clay, React 19, shadcn registry @mokei/*)
- Agents: every session on every device, with live status, cwd, branch, current turn, and Wake / Message / Hand off / Open in suzuri.
- Threads: the conversation timeline, merged across sessions.
- Changes: uncommitted work per worktree. A diff since the last checkpoint, with per-hunk "made by turn X" chips that jump to the turn.
- Review: comments anchored in a read-only code view.
- Board: tasks and leases.
- Inbox: held messages, block/allow lists, and anything flagged by the sweeper.
- Following the mead/wc3-forge rule, every UI action is also an MCP tool, so the GUI and the agents share one control surface.

### Tech stack

Shell: Go 1.25 with Wails v3 (currently alpha.98, pinned the way Toru pins it). React 19, TypeScript, Vite, Tailwind 4, shadcn with the mokei registry (theme, card, badge, tabs, stepper and the rest), bun as package manager. The updater, tray, autostart_repair and hotkey code are copied from Toru's internal/update and tray.go, plus suzuri's internal/update (heal_darwin, resign_darwin, infoplist_darwin) for the macOS .app in-place update.
Core: a Rust 2024 workspace (rustc 1.91 or newer, the same requirement iroh 1.0.1 imposes on suzuri):
- CRDT: loro 1.x
- Networking: iroh 1.0, iroh-blobs 0.103, and hato-core / hato-code copied from suzuri/libs/transfer
- Storage: rusqlite (bundled, FTS5)
- Change capture: notify, plus similar for text diffs into splices
- Git reads: gix, never git writes
- Runtime and APIs: tokio, axum (WebSocket and Streamable HTTP MCP), rmcp (official Rust MCP SDK), ed25519-dalek (from iroh keys)
Frontend CRDT: loro-crdt (WASM).
Agent plugin: TypeScript mod (register.tsx) plus hooks.json, carried forward from tsukumo's claude-mod.

Why these libraries:
- Loro over Automerge: Loro has stable Cursor anchors (exactly what anchored comments need), per-change commit messages and origins (the turn-provenance link), shallow snapshots (to bound history between commits), EphemeralStore for presence, a movable tree for the file index, and identical Rust and WASM cores. Automerge 2/3 has cursors too, but Loro's per-op ID → change → message lookup is what makes blame-to-turn cheap.
- Loro over Yjs/yrs: Yjs is strongest in JS, the Rust port lags, and its update encoding has no commit metadata.
- Not cr-sqlite: it is column-level last-write-wins, has no text CRDT, and has had little maintenance since 2024.
- Not iroh-docs as the main store: it is last-write-wins key-value. Fine for blobs and indexes, wrong for text, and it would be a second sync abstraction.
- Not SpacetimeDB: it needs a central server, which is the main lesson from ai-office.
- iroh over anything else: it is already in your release pipeline (suzuri ships suzuri-transfer and, on the archive branch, suzuri-workspace-sync), it handles NAT traversal between Mac and Windows, and its pairing code exists.
- Go shell plus Rust core over a Tauri rewrite: Toru and suzuri hold your proven installer, updater and signing code, all in Go. The heart of the system (Loro, iroh) is Rust either way. Tauri v2 would mean one language but re-solving the updater and installer. That remains a reasonable alternative if the two-process split hurts.

### Consolidation plan

ABSORB, THEN RETIRE
- tsukumo (~/projects/tsukumo). Absorbed: the routing brains, ported to Rust with their tests. That is router/core.ts (direct messages, the per-pair rate cap DIRECT_LIMIT, queueing through a reload), lib/route.ts (reply-to poster → thread claimant → the session that authored or reviewed the PR → front desk), lib/take.ts (one taker per message), lib/sweep.ts (engaged/deferred/owned rules, the 10-minute and 3-minute timers, the 90 s hand-off return), lib/ledger.ts (authorship, including Claude Code `pr-link` rows from the transcripts), lib/refs.ts and lib/hook.ts (heartbeated claims). These timers and rules are hard-won; keep them word for word. The claude-mod becomes the yoriai plugin, keeping the wake prompt, the held/blocked pane, `defer_mention` → `defer`, `skip_mention` → `skip`, and the rule that the mode and the lists change only when you type something. Removed: src/discord/*, the gateway, Discord REST and 👀/typing, gif.ts, IDENTIFY budgeting, and `.env` DISCORD_TOKEN. "Mentions" become plain hub messages. Discord's threads become hub Threads. Retirement: once the hub's local messaging is at parity (milestone M2), remove tsukumo from CLAUDE_CODE_PLUGIN_DIRS and archive the repo. A Discord bridge is not built and not planned.
- suzuri (~/projects/suzuri). It stays your terminal host; it is a product, not part of the hub. Its Workspace feature (internal/workspace: channels, members, tasks, leases, inbox, wait; `workspace_*` MCP tools; the Workspace pane) is a second mini-Slack that the hub replaces. The hub absorbs the semantics of tasks, exclusive path leases (with `steal` posting a system line), the status codes idle/working/waiting/blocked/away, and session-not-name identity. After that, delete those tools from suzuri. The un-merged archive/glass-chrome workspace-sync crate is superseded and stays archived. libs/transfer's hato-core and hato-code are copied into yoriai-core, the same way suzuri itself imported hato. suzuri then gains a small integration: each PTY pane sets YORIAI_PANE=<id> so the hub can show "this session runs in suzuri tab 2, pane 1", and the hub can focus that pane. Its `suzuri mcp` diagnostics stay.
- aizuchi. Scratch Pad notes become hub Notes (Loro text docs, synced across devices), and the Scratch Pad part of its MCP server retires. The mind-map meetings product is NOT absorbed in v1. It is a separate product with real-time audio, which is different scope. It can later read hub Threads.
- knowledge-base. The private git repo of Markdown stays as the storage, because it already syncs and Grok's /kb reads it. The hub adds a read/search view and a "Promote to KB" action that writes an entry only when you trigger it, which matches your rule that the KB holds only docs you deliberately chose to keep. Decisions recorded in the hub are NOT auto-copied there.
- ai-office. Retired. Lessons carried over: (1) Host real agents (Claude Code, Grok) instead of running an in-app Ollama runtime. ai-office's agent-runtime and its orchestration loops were where the bugs were (commits "zombies on startup", "Fix 6 issues found during 15-agent full-stack test"). (2) No central DB server (SpacetimeDB). (3) The 3D office was ripped out (commits c792b43 and 4e96aff); lead with a dense, useful 2D UI.
- toru. Stays its own Windows capture product. It donates its release.yml (NSIS per-user, no UAC, portable zip, softprops/action-gh-release), internal/update and the tray/autostart code. Later: a "send capture to hub thread" action replaces Discord as the place screenshots go.
- mead / wc3-forge. The pattern is adopted rather than the code: the GUI and MCP share one control surface.
- grok-build. No changes needed beyond `grok mcp add yoriai -- yoriai mcp` and running with `--channels` for waking.

### Capture mechanism

Three sources feed the core, deduplicated by Claude's own ids (uuid, promptId, tool_use_id).
1. Hooks (live, structured). This is a command hook, not the mod's turn hooks: `yoriai hook` runs on every event and forwards the hook JSON plus env (CLAUDE_PROJECT_DIR, the session id, YORIAI_PANE) to the core.
   - SessionStart: register or refresh the session, cwd, branch and harness.
   - UserPromptSubmit: open a turn and mark any delivered wake as engaged, the same role as tsukumo's `seen/`.
   - PreToolUse(Edit|Write|MultiEdit|NotebookEdit): note the file and take an auto-lease if configured.
   - PostToolUse(Edit…): `tool_response` carries filePath, originalFile and structuredPatch, as confirmed in transcript records. The core checks originalFile against its shadow. If they match, it applies the patch hunks as LoroText splices under a commit message {sessionId, promptId, toolUseId}. If they don't match, it first records an `external` catch-up change, so the agent's edit stays attributed only to what it actually changed.
   - PostToolUse(Bash): trigger an immediate worktree rescan of that cwd. Anything that changed is attributed "probably session S (Bash tool_use X)" with confidence=heuristic.
   - Stop / SubagentStop: close the turn, and give tsukumo's `turn.complete` its meaning.
   - Notification: the session is waiting on you, so surface it.
   - PreCompact: snapshot a pointer to the transcript, so compaction never loses history.
   - SessionEnd: mark the session gone, which starts the sweeper's 3-minute timer.
2. Transcript tailer (complete, the source of truth). It reads ~/.claude/projects/<slug>/<sessionId>.jsonl incrementally by byte offset, as tsukumo's ledger already does for `pr-link` rows. That supplies assistant text, tool calls and results, and the parentUuid tree, which hooks only partly expose. Thinking blocks are left out by default. Large tool outputs go to iroh-blobs.
3. Filesystem watcher plus git watcher. notify-crate events on watched worktrees, debounced to 300 ms and filtered by .gitignore via gix. Each changed file is diffed (similar crate) against its shadow and turned into splices. Attribution order: (a) a matching PostToolUse is in flight, so attribute to that session; (b) a Bash call in that cwd within the last N seconds, so heuristic; (c) otherwise the human on this device. Git hooks and HEAD changes produce git.checkpoint and external:git rebases of the shadow.
Session discovery and status also read ~/.claude/sessions/<pid>.json (or `claude agents --json`), which reports busy/idle authoritatively without polling the model.
Grok and other harnesses: either the harness's own hooks, if it has them, or a `yoriai_report` MCP tool plus filesystem-watcher attribution. That is coarser, and the UI says so.
What capture deliberately does NOT do: write into your working tree, or record edits in folders you haven't opted in as watched worktrees.

### Messaging mechanism

Agent A calls message_agent(to, text, thread?, reply_to?), either as an MCP tool or as a mod tool. `to` can be a session id, a session name, an alias A has posted under (tsukumo's `from` alias), a role (front-desk, or a suzuri-style role like engine/pm), or a Thread. With no `to`, routing decides, using tsukumo's order:
1. the session that authored the replied-to message;
2. the thread claimant;
3. the session that opened, worked on or reviewed a named PR, from the ledger, including `pr-link` data from the transcripts;
4. otherwise the front desk.
The router takes a take-lock (one taker per message), appends a `msg` event and a `route` event, and delivers.

Delivery, in order:
(1) The target is attached locally, meaning its yoriai mod has a live NDJSON attach stream. The core pushes a `direct` event. The mod checks the block/allow lists and hold mode. If the session is idle, it calls `$.prompt.submit` with a fenced prompt such as `<hub-message from="A@mac" id=… thread=…>…</hub-message>`. If the session is busy, Claude Code queues the prompt. When UserPromptSubmit later fires with that message id, the core marks the message engaged.
(2) The target is on another device. The core sends over the `yoriai/wake/1` iroh stream to that device's core, which runs step (1) there and returns delivered or engaged acks. All of this also lands in the replicated log, so every device's UI shows it.
(3) The target session exists but no Claude Code process is running it. Exited sessions appear in the transcripts but not in ~/.claude/sessions. The core on the owning device runs `claude --bg --resume <sessionId>` in its cwd (a flag verified on 2.1.310), waits for the session to attach, then runs step (1). The UI shows it as revived. This is opt-in per session or project, because it uses your subscription in the background.
(4) Grok targets: an MCP `notifications/claude/channel` push to its bound yoriai MCP connection. It wakes only under `grok-fork --channels`.
(5) Unknown or unreachable targets: the message is held in your Inbox. It is never dropped silently, which fixes tsukumo's "messages sent while nobody watches are lost".

Then the sweeper (tsukumo's sweep.ts, unchanged): unengaged work moves to the front desk after 10 minutes. A taker that has been gone 3 minutes loses the message. A hand-off not picked up in 90 s comes back. Defers last at most 12 h. Reminders come only from the holder's own process. Rate limit: DIRECT_LIMIT of 8 per pair per 10 minutes, so two agents can't ping-pong forever.

Safety (carried over from tsukumo's AGENTS.md rules): delivered text is always fenced and labeled untrusted. A message can never change block/allow lists, wake mode, mesh trust or hook config; those change only from your UI or slash commands. Cross-device wakes need the sender's device to be on the mesh trust list. A per-project setting can require your approval before a remote device wakes a local session.

Native Claude Code peer messaging (the messagingSocketPath and peerProtocol in ~/.claude/sessions, used by ListAgents/SendMessage) is NOT used in v1. Its wire protocol is private and works only on one machine. The hub reads the registry for discovery and status only, and could later add the native path as a same-machine fast path if Anthropic documents it.

### Cross device

Devices: your Mac (Apple Silicon) and one or more Windows PCs, all equal peers on one iroh mesh. Pairing uses a hato-code SPAKE2 short code once per device. After that, each device knows the others' endpoint ids and reconnects automatically through iroh relays or a direct UDP path. What syncs:
- the whole event log (every session's turns, tool calls, messages and wakes) from every device;
- all Loro docs (threads, board, comments, notes, file shadows for watched worktrees);
- blobs, fetched lazily on open.
Same repo on two machines: the repo identity is the root commit plus the remote, so the Mac's ~/projects/foo and Windows' C:\src\foo share one repo record. Each worktree keeps its own shadow docs, because uncommitted work differs per machine. Paths are stored repo-relative with forward slashes. Text is normalized to LF in the shadow and the EOL style is recorded per file. The UI warns on case-only path collisions.
Delta's "follow someone's uncommitted work" is a read-only remote view by default: on the Mac you can see the Windows worktree's diff since its last checkpoint, with provenance chips. To actually use that work, "Materialize as review worktree" runs `git worktree add` from that checkpoint's sha on the local machine (a new folder, never your active tree) and writes the remote shadow's current text into it. That is Delta's isolated review copy. The reverse direction is the same.
Availability: there is no cloud. When the Mac sleeps, Windows can't reach it, so messages to Mac sessions queue in the sender's log and deliver on reconnect, with the sweeper's 3-minute rule moving front-desk work elsewhere. For always-on, run `yoriai --headless` on a PC that stays on: it acts as an anchor peer that stores and forwards, and as a runner that can host `claude --bg` sessions. That is the honest replacement for Delta's cloud runners.
Claude Code's own Remote Control and --cloud sessions are complementary: the hub lists them when the registry or transcripts show them, but does not drive them.

### Packaging and updates

Repo: StephenSHorton/yoriai (public, MIT, noreply email, matching your others). Layout: shell/ (Go, Wails v3), core/ (Rust workspace), frontend/ (React plus mokei), plugin/ (Claude Code plugin), packaging/.
Release workflow: tag v*.*.* triggers a matrix.
- windows-latest: wails3 task windows:build, cargo build --release of yoriai-core, then makensis with WAILS_INSTALL_SCOPE=user (per-user install, no UAC, copied from Toru's release.yml lines 34–74), plus a portable zip containing both binaries.
- macos-14 (arm64): the wails3 darwin build, with the core binary embedded in Contents/MacOS, both Developer ID signed and notarized using suzuri's packaging/macos/SIGNING.md secrets, shipped as a DMG and an .app.zip.
- Optional later: an x64 Windows ARM build.
- Publish with softprops/action-gh-release@v2, together with a latest.json manifest (version, per-asset sha256, and an ed25519 signature made by a release key held in CI secrets).
Updater: Toru's design. It checks GitHub Releases at startup and every 6 h, downloads in the background, verifies sha256 and the signature, and shows "Restart to update" in the tray, or installs on quit. The shell swaps both binaries together because the core is versioned with the shell, so there is never a version skew between them. On macOS it reuses suzuri's heal and resign path for in-place .app updates.
The Claude plugin and hooks ship inside the app bundle, and are re-copied to <appdata>/yoriai/claude-plugin on each update. PLUGIN_DIR_WATCH reloads running sessions, so no marketplace cache is involved. The plugin reports its version to the core on attach, and the core's protocol version check refuses to wake a stale plugin, the same idea as tsukumo's `routing: 2` gating.
Mesh protocol versions (ALPN /1, /2…) let a Mac on vN talk to a Windows box on vN-1 during staggered updates. The core keeps one previous protocol version.
Autostart at login (Toru's autostart_repair) and a single-instance lock.

### Milestones

- M0 Ship an empty app on both OSes (1–2 weeks). The Wails v3 shell, a yoriai-core sidecar that answers health checks, the mokei-themed window, the tray, and the full release.yml matrix (Windows NSIS plus zip, signed and notarized macOS DMG). Exit criterion: a background update from v0.0.1 to v0.0.2 works on the Mac and on a Windows PC. Every later milestone depends on this loop, so prove it first.
- M1 Local capture (2 weeks). The plugin's hooks.json, `yoriai hook` with a spool file, the transcript tailer, reading ~/.claude/sessions, and the SQLite event log with FTS. UI: the Agents list (live busy/idle, cwd, branch) and a per-session timeline. Done when three concurrent Claude Code terminals show up with correct turns and tool calls, and history search works.
- M2 Local messaging and waking, with Discord gone (2–3 weeks). Port tsukumo's route, take, sweep, ledger, refs and claims to Rust along with their tests. The mod attach stream with $.prompt.submit wakes. message_agent, list_agents, defer, skip and claim_thread over both the mod and MCP. The Inbox with hold, block and allow. Exit criterion: tsukumo is removed from CLAUDE_CODE_PLUGIN_DIRS and its repo archived.
- M3 Cross-device (2–3 weeks). iroh endpoint and identity, hato-code pairing, signed per-device log sync, the yoriai/wake/1 stream with acks, and revive via `claude --bg --resume` (opt-in). Exit criterion: a session on the Mac messages a session on Windows; the Windows session wakes and its reply wakes the Mac session. Then turn the PCs off and on and confirm queued messages arrive.
- M4 Edit capture between commits (3–4 weeks; the hardest part). Loro file shadows per watched worktree, PostToolUse patch application with commit-message provenance, the filesystem watcher with Bash and human attribution, git checkpoints and handling of checkout and rebase, shallow snapshots. UI: a Changes view with "made by turn X" chips and a jump-to-conversation link.
- M5 Anchored comments and review (2 weeks). Comment docs on Loro Cursors with an outdated state when the anchored text is deleted, a code view with comment gutters, a comment MCP tool so agents can read and answer comments, a remote worktree view, and materializing a review worktree with git worktree add.
- M6 Finish consolidating (2 weeks). The task board and path leases (removing suzuri's Workspace), Notes (replacing aizuchi's Scratch Pad), a read-only KB view plus Promote to KB, a suzuri pane id integration, grok-build over MCP with channels, and a headless anchor and runner mode for an always-on PC.

### Risks

- The Claude Code mod API is private and still changing. Waking depends on register.tsx's `$.prompt.submit`, `$.tool.register` and `$.ui`, loaded through CLAUDE_CODE_PLUGIN_DIRS. That is how tsukumo works today, but it is undocumented, Claude-only, and the `claude plugin validate` rules already constrain the code shape. Mitigations: keep the mod thin; gate it by plugin version and protocol; for any client that supports MCP channel notifications, use that path; keep `claude --bg --resume` as a fallback that needs no mod.
- Edit attribution is only partly reliable. PostToolUse(Edit) is exact. But Bash-driven changes (sed, codegen, formatters, `git apply`), two sessions working in the same worktree, and editor auto-saves can only be attributed by heuristic. Delta has an edge here because it owns the editor. The UI must show the confidence level and never claim certainty it does not have. Recommend one worktree per agent, which the board's leases encourage.
- Shadow drift against git. Checkout, rebase, stash, `git reset --hard`, LFS, generated files, huge and binary files, and .gitignore subtleties can all desync the Loro shadow from disk. Each case needs reseeding as an `external:git` change. Loro history also grows: the shallow snapshot at each checkpoint plus per-file size caps (binary files are tracked by hash only) must hold steady on large repos, and this is unproven.
- There is no always-on server. With P2P only, a sleeping Mac or an off Windows PC means delayed wakes and stale views. Store-and-forward needs an anchor peer you keep running. iroh's public relays only relay traffic; they do not store anything.
- A cross-machine wake is effectively remote prompt injection into an agent that may be running with bypassPermissions. Measures: mesh trust is held by endpoint-id allowlist; every message is fenced and labeled untrusted; hub messages can never change hub config; per-project approval for remote wakes is available. Revive-by-`--bg` spends your subscription without you watching, so it must stay opt-in.
- Private formats. Transcript JSONL fields (toolUseResult.structuredPatch, promptId) and the ~/.claude/sessions registry are not public contracts and could change in any Claude Code release. Mitigation: parse defensively, test against fixtures, and treat hooks as the stable contract wherever they carry the same data.
- Two languages in two processes (a Go shell plus a Rust core) means two toolchains, two binaries to sign and notarize, and loopback IPC. suzuri shows it works, but it costs ongoing effort. Wails v3 is still alpha. Tauri v2 is the fallback if either hurts.
- Claims and leases need exclusivity, which a CRDT cannot provide while peers are partitioned. Two offline devices can both claim the same task. Resolution: an exclusive claim is decided by the device that owns the target session's worktree, and conflicts discovered when peers reconnect are flagged to you, not auto-merged.
- Scope. Consolidating six repos plus a Delta-grade capture system is several months of work. The milestone order delivers the parts you feel daily (no Discord, agents wake each other, history captured) by M3, before the harder Delta features.

## 2. Suzuri Hub: the hub daemon ships inside suzuri, not inside the suzuri window

Verdict: make suzuri the one product that ships the hub, but run the hub as a headless daemon (`suzuri hubd`, same binary and same release) that keeps running when no terminal window is open, rather than putting it inside the terminal window. Two findings decide this. (1) The hub has to wake sessions that are not in a suzuri pane: Claude Code desktop Code-tab sessions (which tsukumo's mod supports today), sessions on a Windows PC with suzuri closed, and grok-build. So the hub cannot depend on the GUI process, and suzuri's crash and Windows-freeze history (docs/crash-investigation-2026-08-07.md, docs/windows-stability-swarm-report.md) is a second reason to keep the two apart. (2) suzuri's chrome rule is "No React. No HTML. No Chromium." (chrome/README.md), and mokei is shadcn/React, so there is no single surface that satisfies both. The hub gets two surfaces from one install: small presence and wake cues in the wgpu terminal chrome, and the full hub UI built with mokei React, served by hubd on localhost. Why suzuri and not a new app: suzuri already contains most of a hub. It has a shared workspace with channels, members, presence, @mentions, exclusive tasks and path leases (internal/workspace, docs/workspace.md). It has an MCP server that already pushes `notifications/claude/channel` wakes (internal/mcpsrv/notify.go). It injects per-pane identity into every PTY (SUZURI_PANE_ID, SUZURI_CONTROL_URL). It has an iroh P2P engine with identity, contacts and SPAKE2 pairing (libs/transfer hato-*). And it has a signed and notarized Mac plus NSIS Windows release with an in-app updater. A separate Wails app built from the toru template would get mokei for free, but it would have to copy every one of those pieces, which recreates the stranded-tools problem he wants to end. Tsukumo's routing logic is ported into hubd and Discord is removed completely.

### Architecture

PROCESSES (one install, one binary `suzuri`, plus the existing Rust sidecar):
1. `suzuri hubd`: a headless Go daemon. It starts at login through a macOS LaunchAgent or a Windows HKCU Run key, keeps one instance per user, and keeps running after the GUI closes. It owns the store, routing, capture ingestion, sync and the API. It never draws pixels.
2. `suzuri` GUI: the terminal host as it is today. It becomes a hubd client, the same way it already uses its loopback bridge. Its wgpu chrome shows a presence strip, a per-pane agent badge ("engine-2 · working · thread #pr-142"), wake toasts and an "Open Hub" palette entry. The existing aicontrol plane (127.0.0.1, bearer token) stays as the pane-control API, and hubd calls it to map a session to a pane.
3. `suzuri mcp`: the existing spawn-on-demand stdio MCP server. It becomes a thin hubd client instead of reading the workspace directory itself. It keeps the workspace_* tool names for compatibility and adds hub_* tools (hub_send, hub_threads, hub_history, hub_who, hub_decide, hub_comment, hub_edits_for). It keeps the channel-notification push for grok-build and for Claude when channels are enabled.
4. `suzuri-sync`: a Rust iroh sidecar that extends hato-core (identity.rs, contacts.rs) and restores the deleted workspace-sync crate (commit c8e2691, libs/transfer/crates/workspace-sync). It speaks NDJSON to hubd, following the existing libs/ rule ("host talks over a stable boundary").
5. `suzuri-hub` Claude Code mod (TS/TSX, Claude Code mod API). This is tsukumo's claude-mod with the Discord transport replaced by a hubd WebSocket stream. It handles wakes, the thread pane, held/blocked mentions and defer/skip.
6. `suzuri hook <event>`: plain Claude Code hooks (command type, so they also work without the mod and on Windows without bash). They forward the hook JSON to hubd over loopback.
7. Hub web UI: a React 19 + Vite + Tailwind + mokei SPA embedded in the binary with go:embed and served by hubd at http://127.0.0.1:<port>/?token=…. The palette, the tray or the CLI (`suzuri hub open`) opens it in the default browser or an app window.

DATA MODEL (SQLite in WAL mode through modernc.org/sqlite, pure Go so Windows builds need no cgo). Everything is an append-only event log, and tables are materialized from it:
- machine(node_id = iroh NodeId, name, os, last_seen)
- agent_session(id, harness: claude|grok|human|other, harness_session_id, machine, cwd, repo_root, branch, head_sha, pane_id?, display_name, role, status idle|working|waiting|blocked|away, status_note, started, ended)
- thread(id, title, kind: channel|dm|review|task, repo?, topic). This replaces both suzuri workspace channels and Discord channels.
- message(id, thread, from_session, mentions[], reply_to, body, attachments, ts)
- turn(session, prompt_excerpt, transcript_path, byte_range, started, ended, summary?)
- edit(id, session, turn, tool_use_id, repo, path, base_sha, hunk/patch, content_hash_before/after, ts, commit_sha nullable until a commit links it)
- decision/note(id, thread, session, text, pinned). This absorbs the aizuchi Scratch Pad and suzuri notes.
- comment(id, repo, path, anchor{content_hash, context_before/after, line_range at sha}, body, thread)
- task, lease (moved over unchanged from internal/workspace)
- routing state ported from tsukumo .watch/: taken, seen(engaged), deferred, held, flagged, pending handoff, direct-rate counters
- ledger (from tsukumo .ledger.json): who posted, opened, reviewed or touched which PR and message, kept 45 days

Each event carries {ulid, origin_node, lamport, type, payload}. Each machine appends only to its own log, so replicating logs never conflicts. Exclusive state (task claims, leases, mention take-locks) is decided by an owner machine: the machine that hosts the target session decides that session's take-locks, and a lease is decided by the machine that owns the repo checkout. A partition therefore delays a decision instead of producing a split-brain.

ONE CONTROL SURFACE (the mead/wc3-forge pattern he likes): hubd exposes one loopback HTTP+WS API, and the web UI, wgpu chrome, MCP server, Claude mod, hooks and CLI all use it. Agents and Stephen see the same objects.

HOW AGENTS CONNECT:
- Claude Code: the suzuri-hub mod (wake and pane), suzuri hooks (capture), suzuri MCP (tools). Discovery is free: hubd reads ~/.claude/sessions/<pid>.json (pid, sessionId, cwd, name, status busy|idle, kind, entrypoint). It reads this registry only and does not use Claude's private peer socket protocol.
- grok-build: `suzuri mcp` plus `grok-fork --channels`. This works today (branch feat/workspace-mcp-channels, cdcd1e1e "grok-fork channel wakes instead of wait-loop", merged).
- Any other MCP client: the tools only, with wake through PTY injection if it runs in a suzuri pane.

HOW DEVICES CONNECT: paired iroh peers, Mac and Windows, explained under cross_device.

### Tech stack

- Go: hubd, GUI host, MCP (github.com/modelcontextprotocol/go-sdk, already used), hooks CLI, updater. These are all in the existing suzuri module.
- SQLite through modernc.org/sqlite (no cgo, so the Windows cross-build is safe), in WAL mode with an append-only events table.
- Rust and iroh for P2P sync. This extends the existing libs/transfer workspace (hato-core identity and contacts, hato-code SPAKE2 pairing, hato-mailbox rendezvous) and ships as a `suzuri-sync` sidecar beside the existing `suzuri-transfer`.
- Rust winit and wgpu for the in-terminal chrome: presence strip, pane badges, toasts. Existing.
- React 19, Vite, Tailwind, shadcn and mokei for the hub web UI, embedded with go:embed and served on 127.0.0.1 with a token. Bun builds it in CI.
- Bun and TS/TSX for the Claude Code mod (required by the mod API; tsukumo's register.tsx is the starting point).
- Claude Code integration: hooks (SessionStart, UserPromptSubmit, PostToolUse, Stop, Notification, SubagentStop) calling `suzuri hook`, an MCP server, the mod API `$.prompt.submit` for wakes, `--channels`/`notifications/claude/channel` as a second wake path, the transcript JSONL under ~/.claude/projects, ~/.claude/sessions/*.json for presence, and ~/.claude/file-history for edit snapshots.
- CI and release: suzuri's .github/workflows/release.yml. It already has a Win/Mac matrix, the Rust sidecar build, NSIS user-scope install, Developer ID signing and notarization, and SHA256SUMS. Add the web UI build step and the mod bundle to it.

### Consolidation plan

ABSORBED INTO SUZURI (and the source retired):
- tsukumo: port the routing brains to Go in hubd, using its tests as the spec: src/lib/route.ts (105 lines, answer order: reply-to poster, then thread claimant, then PR author/worker/reviewer, then front desk), take.ts (one taker per mention), sweep.ts (151 lines: engaged/deferred/owned work never reassigned; unengaged after 10 min goes to the front desk; a taker gone 3 min loses it; a hand-off not picked up in 90 s comes back), ledger.ts (authorship, including incremental reads of pr-link rows from ~/.claude/projects/*/*.jsonl), and router/core.ts (894 lines: direct chat-to-chat messages, queueing across reloads, DIRECT_LIMIT 8 per pair per 10 min, aliases). The mod (claude-mod/hooks/register.tsx, about 1.3k lines, wake.ts) becomes the suzuri-hub mod: keep the wake prompt, fencing, hold/block/allow, defer/skip, turn.start/turn.complete engagement and the pane, and swap the router.sock stream for a hubd WS. Delete src/discord, src/gateway, call.ts, the bot token, the 1,000/day IDENTIFY concern and the 👀/typing reactions (they become hub UI state). Archive the tsukumo repo once the suzuri-hub mod reaches parity, then remove CLAUDE_CODE_PLUGIN_DIRS=…/tsukumo/claude-mod from his settings. That settings change is his to make; the installer only shows it.
- suzuri internal/workspace: becomes the hub's thread/agent/task/lease model. Channels become threads and members become agent_sessions. A one-time import moves workspace/*.json and channels/*/messages.jsonl into the event store. The Workspace split pane in the GUI remains a compact view.
- aizuchi Scratch Pad and its MCP server: notes move into hub decision/note entities (suzuri chrome already has notes; see chrome/NOTES_HOOKS.md). Retire aizuchi's MCP server. The live mind-map meetings product stays separate because it is a meeting tool, not agent tooling. It could later read hub threads, but folding audio meetings into the hub now would be scope creep.
- The deleted workspace-sync crate (c8e2691) is restored and generalized from "tail jsonl" to "replicate event logs".

KEPT SEPARATE, AND LINKED:
- knowledge-base stays a private git repo of deliberate docs. The hub indexes it read-only and can link a decision to a KB entry, but never writes to it unasked (his memory rule).
- toru stays its own Windows product. Its pattern (download in the background, install on quit, tray, autostart repair) is already copied into suzuri's internal/update ("modeled on toru"). For this product the template is suzuri's release pipeline, because toru is Windows-only and suzuri already signs and notarizes Mac builds.
- mokei stays a design-system repo, consumed as a dependency of the hub web UI.
- grok-build keeps connecting through MCP and channels, with no changes needed.

RETIRED:
- ai-office: retire it and keep its lessons. Do not depend on a hosted multiplayer database (SpacetimeDB) for something local-first. Do not build a separate agent runtime sidecar; the agents are real Claude Code and grok sessions. A 3D metaphor adds UI cost without capture value. Keep the Slack/Discord bridge idea dead.
- Discord as a transport, everywhere.

### Capture mechanism

Four layers, cheapest and most reliable first.
1. Hooks (`suzuri hook <event>`, registered in ~/.claude/settings.json or in the mod's hooks.json):
   - SessionStart registers an agent_session with harness session_id, cwd, repo root, branch, HEAD sha and SUZURI_PANE_ID if present. It returns additionalContext: who else is active in this repo and on which branch, open threads that mention this repo, unread messages, active leases, and recent decisions.
   - UserPromptSubmit records the turn start and prompt excerpt and injects unread direct messages.
   - PostToolUse on Edit, Write, MultiEdit and NotebookEdit records an edit row. The documented hook payload carries tool_name, tool_input (file_path, old_string/new_string or content) and the tool result, plus session_id and transcript_path. hubd computes the hunk against the file's content hash before the edit.
   - PostToolUse on Bash flags commands that touch git (commit, checkout, gh pr create/review, which is tsukumo's existing observer).
   - Stop marks the turn ended and the session idle. If HEAD and the worktree changed outside known edit rows (sed, codegen, formatters), it snapshots `git diff` so those changes are attributed to the turn as "unattributed tool edits".
   - Notification marks the session blocked or waiting.
2. Transcript tailing: hubd reads ~/.claude/projects/*/<session>.jsonl incrementally by byte offset, the pattern tsukumo's ledger.ts already uses. This gives full conversation history, pr-link rows, and subagent activity, and lets the UI deep-link "this edit came from this message". hubd stores pointers (path plus byte range), not copies, and syncs only excerpts and summaries across machines. A summary per turn from a small model is optional.
3. Git linkage: hubd watches .git/HEAD and refs for repos with active sessions (fsnotify). On a new commit it links the pending edit rows whose hunks appear in the commit diff. That gives Delta's core property, edits between commits tied to the conversation that produced them, without replacing git. Edits that never land in a commit stay visible as "abandoned".
4. Pane context: for sessions in suzuri panes, aicontrol's report_agent and layout give pane title, idle/busy state and tail text. ~/.claude/sessions/*.json gives busy/idle for every local Claude session at no cost.

Anchored comments store {path, content hash of the anchored lines, 3 lines of context either side, sha}. After each edit or commit hubd re-resolves the anchor with a fuzzy patch match, and marks it outdated when the match fails.

What this does not do, stated plainly: it does not replicate the live working tree in real time the way DeltaDB does. Code still moves through git. The hub replicates the metadata about edits and conversation, which is what you need to know who did what and why.

### Messaging mechanism

ADDRESSING: send to a session (by id, display name, role or alias, using tsukumo's alias rules), to a thread with @mentions, or to "whoever owns X" (a PR, file lease or task), resolved by the ported route.ts order. Every message is an event in the sender machine's log.

DELIVERY (hubd on the target session's machine decides the take-lock and delivers):
- Claude Code, primary path: the suzuri-hub mod holds a WS to the local hubd. On a mention or direct event it calls `$.prompt.submit({text})` with the message fenced as untrusted (`<hub-message from=… thread=…>`). If the session is idle this starts a new turn. If it is busy, the prompt queues behind the current turn, which is tsukumo's proven behavior (three call sites in register.tsx). The mod reports turn.start, which marks the mention engaged, and turn.complete, which clears it, so the sweeper does not reassign it.
- Claude Code, second path: `suzuri mcp` already emits `notifications/claude/channel`. Claude Code 2.1.310 contains `--channels`, `notifications/claude/channel` and the `--dangerously-load-development-channels` gate, so a session launched with that flag (suzuri can launch `claude` that way in its panes) gets wakes with no mod. This is hidden and gated, so it is a fallback, not the foundation.
- grok-build: `grok-fork --channels` with suzuri MCP. This already works.
- Any TUI agent sitting at a prompt in a suzuri pane: last resort. hubd asks the GUI through aicontrol to type the message into that pane's input. It is off by default and limited to panes that report idle and have no unsent draft.
- Claude Code's native SendMessage/ListAgents: the hub observes them through the ~/.claude/sessions registry for discovery and idle/busy status. It does not reimplement or call the private socket (`messagingSocketPath: /tmp/cc-socks/<pid>.sock`, `peerProtocol: 1`), because that path is Claude-only, single-machine and versioned. Agents on one Mac can keep using it, and the hub is the path that works across harnesses and machines.

SAFETY (from tsukumo): DIRECT_LIMIT of 8 messages per pair per 10 min to stop wake loops between agents. Block and allow lists, wake mode and claims change only by a human action in the UI or a typed slash command, never because a message asked. Remote text is always fenced. A message that moves is reported to its old holder, and the old holder's late reply is refused.

CROSS-MACHINE: the event is replicated to the paired peer through suzuri-sync. That peer's hubd sees a message for a local session and delivers it as above. If the target machine is offline, the event stays in the sender's log and is delivered when the peers reconnect (store-and-forward comes from log replication). Waking a machine that is asleep is not possible; see risks.

### Cross device

IDENTITY: each install has a persistent iroh NodeId (hato-core identity.rs). Pairing is done once per machine pair with a short code, using the existing SPAKE2 path (hato-code) over the existing hato-mailbox rendezvous, and the result is saved as a contact (hato-core contacts.rs). After pairing, peers reconnect by NodeId through iroh's hole punching, falling back to iroh relays. This works across Mac and Windows without port forwarding. It replaces suzuri workspace's current "copy a ticket each session" UX.

SYNC: each machine replicates the other's append-only event logs. Peers exchange per-origin high-water marks (origin_node, lamport/ulid) and stream the missing events. Writes are idempotent by ulid, which matches c8e2691's "append-only jsonl with id dedup" design and its A↔B converge tests. Transcripts and large attachments stay on the machine that produced them and are fetched on demand through the existing transfer engine. Repo identity across machines is the normalized git remote URL, so "the same repo" on Mac and Windows lines up even with different paths.

PRESENCE: each hubd publishes heartbeat events for its sessions. A peer that has been silent for more than 90 s shows its sessions as "machine offline (last seen …)" rather than gone, following tsukumo's rule that ownership outlives liveness.

ALWAYS-ON PEER (optional, honest about it): hato-mailbox is a zero-knowledge relay that holds a slot for at most 300 s, routes bytes for at most two parties and stores nothing ("No database, no disk"). It cannot hold messages. For delivery while the Mac sleeps, run `suzuri hubd --headless` on a machine that stays on, such as a Windows desktop, a mini PC or a small VPS. It is just another paired peer that holds logs. No hosted service is required.

CURRENT STATE: there is effectively no working cross-device sync today. c8e2691 added libs/transfer/crates/workspace-sync and docs/workspace-iroh.md, but neither exists at HEAD. internal/workspacesync shells out to a `suzuri-workspace-sync` binary that is no longer built, and docs/workspace.md still links to the missing doc. Recover that crate from git history as the starting point.

### Packaging and updates

Reuse suzuri's pipeline instead of building a new one. .github/workflows/release.yml already builds Windows amd64 and macOS arm64, builds the Rust sidecars from libs/, packages a user-scoped NSIS installer (no admin; %LOCALAPPDATA%\Programs\suzuri), a portable exe and zip, a Developer ID signed and notarized .app and .dmg when secrets are present, and SHA256SUMS, then creates the GitHub Release. internal/update ("modeled on toru's check → download → verify → replace") already does background checks against GitHub Releases, SHA256 verification and the Windows rename-running-exe-to-.old swap, plus macOS re-signing and Info.plist healing (resign_darwin.go, heal_darwin.go).

Additions:
1. Release builds now also run `bun run build` for the mokei web UI (embedded through go:embed, so it updates with the binary), bundle the suzuri-hub mod folder into the app (Contents/Resources/hub-mod on Mac, next to the exe on Windows), and build the `suzuri-sync` sidecar.
2. hubd autostart: a macOS LaunchAgent (~/Library/LaunchAgents/dev.suzuri.hubd.plist, KeepAlive) and a Windows HKCU\…\Run entry, both written by first run or the installer, with toru-style autostart repair on launch.
3. Update ownership: hubd is always running, so it owns the update check and background download. It applies the update when the GUI is closed and no hook calls are in flight (toru's "install on quit" semantics), or the GUI's "Restart to update" applies it immediately. After the binary swap hubd re-executes itself and the GUI relaunches. One binary means version skew is impossible, apart from the mod.
4. Mod versioning: the mod path is versioned with the binary. hubd checks the mod's version over the WS handshake and tells stale sessions to reload, which CLAUDE_CODE_PLUGIN_DIR_WATCH=1 does automatically. Writing CLAUDE_CODE_PLUGIN_DIRS and the hooks into ~/.claude/settings.json is offered as a one-click step that shows the diff and is never done silently. The suzuri repo can also act as a plugin marketplace for machines without a local checkout.
5. Windows remains unsigned (Authenticode), so SmartScreen will warn, as it does today.

### Milestones

- M0 Spike and recover (about 1 week): restore libs/transfer/crates/workspace-sync and docs/workspace-iroh.md from c8e2691 on a branch. Prove `suzuri hubd` can run headless on Mac and Windows from the same binary with a single-instance lock. Prove the GUI and `suzuri mcp` can attach to it over loopback with the aicontrol token pattern. Decision gate: does the mod API `$.prompt.submit` still behave as tsukumo uses it in the current Claude Code version?
- M1 Local hub core: SQLite event store, migration of internal/workspace data into threads, agents, tasks and leases, and `suzuri mcp` as a thin hubd client keeping workspace_* names and adding hub_* tools. Local Claude presence from ~/.claude/sessions/*.json plus SessionStart and Stop hooks. The GUI Workspace pane reads from hubd.
- M2 Discord removed (the first consolidation win): port tsukumo's route, take, sweep, ledger and direct logic to Go, with its test cases translated as the spec. Ship the suzuri-hub Claude mod (tsukumo's register.tsx with a hubd WS stream instead of router.sock). Agent-to-agent messages and wakes work between local Claude and grok sessions. Turn off tsukumo-discord on his Mac and archive the repo once he is satisfied.
- M3 Capture v1: PostToolUse edit rows, incremental transcript tailing, Stop-time git diff for edits made outside known tools, and commit linking through ref watching. First mokei web UI with a live session board, per-thread timeline, an edit-to-conversation view per file and commit, and a decisions list. aizuchi Scratch Pad notes are imported and its MCP server retired.
- M4 Cross-device: suzuri-sync sidecar with persistent paired contacts (SPAKE2 code once, then NodeId reconnect), per-origin log replication, and cross-machine presence and delivery between the Mac and one Windows PC. The optional `--headless` always-on peer.
- M5 Ship it like toru: release.yml builds the web UI, mod bundle and sync sidecar. hubd autostart and repair on both operating systems. The hubd-owned background update with apply-on-quit. Mod version handshake. A one-click Claude settings integration that shows the diff first. A clean-machine install test on Windows and Mac.
- M6 Delta-like extras: anchored code comments that survive edits, review threads that spin up an isolated `git worktree` for the reviewing agent, presence strip and pane badges in the wgpu chrome, and the PTY-injection wake for non-Claude TUIs behind a setting.

### Risks

- The wake path depends on Claude Code surfaces that are not stable public API. The mod API (`$.prompt.submit`, `turn.start`/`turn.complete`) is what tsukumo depends on today, and channels (`--channels`, `notifications/claude/channel`) is present in 2.1.310 but hidden and behind `--dangerously-load-development-channels`. If either changes, wakes break. Mitigation: three independent wake adapters (mod, channels, PTY injection into suzuri panes), all behind one hubd delivery interface, plus a startup self-test that reports which adapters work.
- Cross-device sync must be largely rebuilt. The workspace-sync crate and docs/workspace-iroh.md from c8e2691 were deleted from HEAD, and internal/workspacesync shells out to a binary that no longer exists. Exclusive state across machines (task claims, leases, mention take-locks) is real distributed-systems work. The owner-machine rule avoids split-brain but means decisions stall while a peer is offline.
- The UI is split in two. suzuri's chrome rule ("No React. No HTML. No Chromium.") rules out mokei inside the terminal window, so the rich hub UI lives in a browser or app window served by hubd. This is one install and one updater, but two surfaces, and he may feel that is less unified than he wants. The alternative, reimplementing mokei's look in wgpu, costs a lot more.
- Coupling to suzuri's stability. suzuri has a record of crashes and Windows UI freezes (docs/crash-investigation-2026-08-07.md, windows-stability-swarm-report.md, recent fix commits). Putting hubd in its own process isolates the hub from GUI crashes. It also means two processes and an autostart daemon to support on Windows, a known source of trouble that toru already needed autostart_repair_windows.go for.
- Windows specifics: Claude Code's native peer sockets assume /tmp/cc-socks, hook commands must not assume bash, plugin directory paths and settings layout differ, SmartScreen warns on the unsigned Windows build, and the rename-running-exe update trick has to cover two running processes (hubd and GUI) from one exe.
- Physics: an agent on a sleeping or powered-off machine cannot be woken. hato-mailbox is a zero-knowledge rendezvous with no storage ("No database, no disk", slots of at most 300 s). True asynchronous delivery needs an always-on peer that he runs himself.
- Prompt-injection and runaway loops. Agents waking other agents across machines spreads untrusted text. Keep tsukumo's protections: fencing, human-only changes to block/allow lists, DIRECT_LIMIT 8 per pair per 10 min, and refusing replies from a session that no longer holds a mention. Only paired contacts can deliver anything.
- Scope creep toward DeltaDB. Delta replicates live worktrees with a team and cloud runners behind it. This design replicates edit and conversation metadata and leaves code in git. Trying to match real-time worktree CRDTs would sink the project.
- Edit attribution has gaps. PostToolUse covers the Edit, Write and MultiEdit tools, but edits made through Bash (sed, codegen, formatters) or by Stephen in an editor only show up through the Stop-time git diff, attributed to the turn, not to a specific tool call.

## 3. Tsudoi (集い, "a gathering"): one agent hub that replaces tsukumo, suzuri's workspace, aizuchi Scratch Pad and ai-office

Tsudoi is a single tray app built with Wails v3, Go and React 19 plus mokei. It installs and updates the way toru does on Windows and the way suzuri does on macOS. Every device runs one copy, and that copy is three things in one Go binary (the mead/wc3-forge pattern): (1) a loopback daemon that records what each agent session does, (2) an MCP server exposing about 8 tools to Claude Code and grok-build, and (3) a GUI where Stephen is one more participant in the hub. Copies on his Mac and Windows PCs pair once with a ticket and copy an append-only event log to each other over iroh (the Rust engine already in suzuri/libs/transfer). Discord goes away entirely: messages between agents travel hub to hub over iroh, and each hub wakes the target session locally.

The design does not rebuild Claude Code features that already work. It reads what Claude Code already writes: ~/.claude/sessions/<pid>.json (name, cwd, busy/idle status, peer socket) and the transcript JSONL (structuredPatch, file-history-snapshot/delta, pr-link). Claude Code's own `http` hooks fill the gaps. On one machine, Claude-to-Claude messages use native SendMessage. For everything else the hub does the waking: a slim Claude mod based on tsukumo's (`$.prompt.submit`), `notifications/claude/channel` for Grok, and `claude --bg --resume` for sessions whose process has exited.

The hub is a lift-and-merge, not a fresh invention:
- suzuri's workspace data model (channels, members, presence, tasks, path leases) becomes the hub's model.
- tsukumo's router/core.ts routing logic (ownership ledger, per-message take-lock, front desk, sweeper, deferral, pair rate limits, held mentions, block/allow) becomes the hub's routing.
- toru and suzuri supply the release and update pipelines.

Every milestone replaces something Stephen uses today, so this does not become a third stranded attempt. ai-office stalled in March 2026, and suzuri's workspace is a half-built hub inside a terminal.

v1 covers Stephen's own agents and devices only. Federation with other people's agents, which tsukumo's shared-Discord peer protocol handled, is explicitly out of scope until late.

### Architecture

COMPONENTS (one Go binary, `tsudoi`, plus one bundled Rust sidecar)
1. Daemon (runs inside the GUI process; the tray keeps it alive; autostarts at login like toru's tray/autostart).
   - Listens on 127.0.0.1:<port>. Clients find it through a config-dir file (`hub.json` with port and token) plus env vars. This is the same pattern as suzuri internal/aicontrol (loopback TCP, token, discovery file, PTY env), and it works on Windows.
   - No unix sockets. tsukumo's router.sock is macOS-only in practice.
2. `tsudoi mcp`: a stdio MCP server spawned on demand by Claude Code or grok-build (the same idea as `suzuri mcp`, built on modelcontextprotocol/go-sdk, which suzuri already uses).
   - It proxies to the daemon. If the daemon is down it falls back to read-only access to the on-disk store.
   - It also carries the channel-push capability, so Grok's channel.rs can be woken through it.
3. Claude mod (`clients/claude-mod`, TS/TSX, loaded through CLAUDE_CODE_PLUGIN_DIRS like tsukumo today). A thin client with no bun and no Discord:
   - Registers the session with the daemon.
   - Opens a long-poll/SSE connection to the daemon for deliveries and wakes the session with `$.prompt.submit`.
   - Marks "engaged" on `turn.start` (tsukumo's markStarted).
   - Mirrors native peer messages into the hub log through the `session.receive` / `session.send` hooks.
   - Draws the inbox pane and the band card.
4. Claude Code `http` hooks. The app installs them into ~/.claude/settings.json with consent, and the installer and updater keep them current. They post to the daemon:
   - SessionStart, UserPromptSubmit, PostToolUse (Edit|Write|MultiEdit|NotebookEdit|Bash), Stop (`last_assistant_message`), Notification (`idle_prompt`, `permission_prompt`, `agent_needs_input`), PreCompact, SessionEnd, CwdChanged.
   - http hooks fail open: when the daemon is down, Claude Code keeps working.
5. Collectors:
   - A watcher on ~/.claude/sessions/*.json for presence (pid, sessionId, name, cwd, status busy/idle, messagingSocketPath).
   - A transcript tailer on ~/.claude/projects/**/*.jsonl that pulls structuredPatch, file-history deltas, pr-link and prompts by offset.
   - A watcher on .git/logs/HEAD and refs for the repos sessions run in, to see commits. Nothing is installed into any repo.
   - Grok: its session registry (~/.grok/active_sessions.json) plus explicit MCP calls.
6. Rust sidecar `tsudoi-net`, forked from suzuri libs/transfer (hato-core identity/contacts, hato-code SPAKE2 short codes, iroh endpoint).
   - Speaks NDJSON over stdio to Go, the same machine-mode protocol suzuri already shells out to.
   - Handles device pairing, log replication streams and file transfer.
7. GUI: React 19, Tailwind, shadcn with the mokei registry theme (glass on clay, white as primary material).
   - Views: People and agents (presence by device and repo), Session timeline, Inbox/Threads (Stephen as a participant), Repo view (edits by session between commits), Tasks/Leases, Notes, Devices, Settings/Updates.

DATA MODEL (SQLite per device using cgo-free modernc.org/sqlite so the Windows build from CI stays simple; FTS5 for search)
- `devices` (device_id = iroh endpoint id, name, os, paired_at).
- `participants`: one row per human, Claude session or Grok session. Fields: device_id, client, session_id, display name, role, cwd, repo_key, status (idle/working/waiting/blocked/away/gone), last_seen.
- `events`: append-only and replicated. Each event has id = (device_id, seq), an HLC timestamp, actor and kind. Kinds:
  - prompt, turn_end_summary, edit{repo_key, path, patch hash}, commit{sha}, message, delivery_state, task_*, lease_*, note_*, presence.
  - Payloads that can be large, such as full patches and transcript excerpts, stay on the originating device as blobs referenced by hash. They are fetched over iroh on demand.
- Derived tables, rebuilt from events:
  - `cards`: per-session status card covering current intent, files touched, branch and last Stop summary.
  - `ledger`: who posted, owns or worked on which thread/PR/file/task. This is tsukumo's .ledger.json generalized.
  - `edit_links`: edit → turn → session → commit.
  - `threads`, `inbox`.
- Files are keyed by repo identity: `repo_key` = normalized git remote URL, else the root-commit hash, plus a POSIX relative path. Absolute paths are never the key, so C:\Users\me\... and /Users/me/... resolve to the same file.

SYNC / TRANSPORT
- Same device: hooks and the mod talk to the daemon over loopback HTTP with a token.
- Between devices: iroh QUIC through the sidecar (direct when possible, n0 relays otherwise, self-hostable relay later).
- Replication is per-device logs exchanged by sequence-number vectors. Events are immutable, so merging is a union with no CRDT. The few pieces of mutable shared state (task claim, lease, message take-lock) have a single authority:
  - A task or lease is owned by the device that holds it. Claims go to the owner device; if it is offline, the claim is refused unless Stephen sets steal=true.
  - The take-lock for a message is owned by the device of the target session.
  - The result is exactly one wake, without consensus across devices.

HOW AGENTS CONNECT
- Claude Code: hooks (capture plus context injection), the mod (wake, inbox pane, native peer bridge), and the MCP tools.
- grok-build: the MCP tools, plus `notifications/claude/channel` wakes, which grok-fork already turns into turns (crates/codegen/xai-grok-mcp/src/channel.rs).
- Other MCP clients: tools only, with no wake.
- Stephen: the GUI.

HOW DEVICES CONNECT
- Pairing: the GUI shows a short code or QR (hato-code SPAKE2). The other device enters it and the two exchange long-term iroh identities, which are stored as contacts.
- After that, every paired hub dials the others at startup and keeps replicating. There is no server and no account.

### Tech stack

- App shell: Wails v3 with Go 1.26. This copies toru's build/Taskfile/tray/autostart and suzuri's darwin bits.
- Frontend: React 19 + TypeScript + Tailwind + shadcn, with components and theme from the mokei shadcn registry (stephenshorton.github.io/mokei/r/registry.json). bun is the package manager. TanStack Query talks to the Wails bindings and the daemon SSE stream.
- Store: SQLite (modernc.org/sqlite, cgo-free) with FTS5, plus a content-addressed blob dir for patches and excerpts.
- MCP: modelcontextprotocol/go-sdk (already in suzuri) for the stdio server, including channel notifications.
- P2P: a Rust sidecar built from suzuri libs/transfer (iroh + hato-core/hato-code), driven over NDJSON stdio exactly as suzuri's Go host drives `suzuri-transfer`.
- Claude integration: a TS/TSX mod using Claude Code's mod engine API (`$.prompt.submit`, `$.session.send`, `session.receive`, `turn.start`, `$.tool.register`, pane/band UI) and `type: "http"` hooks.
- Optional summarization at Stop: a cheap model call (Haiku via API key) to compress a turn into a one-line card. It is off by default; cards are built deterministically from hook data first.
- CI: GitHub Actions with a windows-latest job (NSIS per-user installer, portable zip) and a macos-latest arm64 job (dmg plus .app.zip, Developer ID signing and notarization from suzuri's packaging/macos), both publishing SHA256SUMS.

### Consolidation plan

Rule: each absorbed feature ships in the hub before the old one is retired, and every milestone replaces something Stephen uses daily.

- tsukumo → ABSORBED, then the repo is archived.
  - Port router/core.ts to Go as the hub's routing engine: the ownership ledger (posted / claimed thread / opened-worked-reviewed PR / file / task), per-message take-lock, front desk plus handoff of the front-desk role, the 10-minute unengaged sweeper, 3-minute "gone" grace, defer_mention, held mentions, everyone/allowlist modes with block/allow lists, and DIRECT_LIMIT pair rate limiting. core.ts was built to be tested without Discord, and its tests become the Go port's spec.
  - Keep wake.ts's prompt framing (fenced untrusted text, "why this chat", reply/skip/defer instructions) with Discord replaced by hub threads.
  - claude-mod is rewritten as the hub mod. Its Discord transport, watch.ts, attach.ts, the 30-minute-lifetime router and the Discord REST/gateway are deleted. Cursor-cloud-agent PR detection (prmeta.ts) can come later as a GitHub watcher.
  - Lost on purpose: routing for other people's agents in a shared Discord server (peer-protocol.md). v1 is single-owner, and federation is a late milestone.
  - Retire when M3 ships and Stephen has run a week without `/discord watch`. Remove CLAUDE_CODE_PLUGIN_DIRS→tsukumo from settings.json at that point.
- suzuri workspace → MODEL ABSORBED; suzuri stays as a terminal.
  - Lift docs/workspace.md's model into the hub: channels, members with session-not-name identity, presence codes idle/working/waiting/blocked/away, tasks.json and leases.json semantics (exclusive claim, assign, steal posts a system line), file attachments, pinned topics, join-without-noise.
  - Cut suzuri's ~30 workspace_* MCP tools down to the hub's ~8.
  - suzuri's Workspace pane becomes a thin client of the hub API (or is removed in favour of the hub GUI). internal/workspacesync and the workspace-sync sidecar are retired.
  - suzuri's iroh engine (libs/transfer) moves to the hub as tsudoi-net, and suzuri's Send/Receive can call the hub instead of carrying its own copy.
  - suzuri_notes_* merge into hub notes.
  - What suzuri keeps: being the terminal host, plus aicontrol for split/focus panes. The hub calls aicontrol to "open this session in a suzuri pane" when it spawns or resumes agents.
- aizuchi → Scratch Pad and its MCP server ABSORBED into hub notes, alongside suzuri notes, as one notes store with one tool. The live mind-map meetings are a different product and stay in aizuchi for now; they are not ported, to avoid scope creep. Its .dmg-only, unsigned release is not reused.
- knowledge-base → STAYS a private git repo with the /kb skill unchanged. The hub indexes it read-only into FTS so `hub_context` can cite KB entries. The hub never writes KB entries (Stephen's rule: deliberate docs only, never unasked).
- ai-office → RETIRED as a code source. Lessons:
  - Don't host or simulate your own agent runtime (agent-runtime sidecar); attach to real Claude Code and Grok sessions.
  - Don't depend on a hosted multiplayer DB (SpacetimeDB). Local-first with P2P replication instead.
  - Spectacle got cut: the 3D office was removed in its last commits. Build a plain, information-dense GUI with mokei polish.
- toru → DONOR ONLY. Windows packaging and update: internal/update (GitHub Releases API, SHA256SUMS verification, NSIS silent install, "ready → restart now or install on quit"), release.yml, tray, autostart repair. Toru itself stays a screenshot app; its wormhole Send Files is not used (iroh is).
- mead / wc3-forge → PATTERN ONLY. One Wails binary where the GUI and the agent share one control surface (embedded MCP).
- grok-build → NO CHANGE NEEDED for v1. It already consumes MCP tools and `notifications/claude/channel` wakes (suzuri already pushes these; see suzuri internal/mcpsrv/notify.go). The hub reuses that path.

### Capture mechanism

Principle: read what Claude Code already records. Add hooks only for real-time signals and for context injection. Store distilled cards, not raw transcripts, as the shared layer.

SOURCES (Claude Code, verified on this machine, v2.1.310)
1. ~/.claude/sessions/<pid>.json: sessionId, name, cwd, kind, entrypoint, status busy/idle with statusUpdatedAt, messagingSocketPath, peerFeatures (incl. notify_idle). This gives presence and busy/idle for free; the hub does not infer it.
2. Transcript JSONL at ~/.claude/projects/<cwd-slug>/<sessionId>.jsonl, tailed by byte offset. Record types seen:
   - user / assistant / attachment / system.
   - `toolUseResult` with `filePath`, `oldString`/`newString`, `structuredPatch`, `originalFile`, `gitOperation`, `bashEditDiff`.
   - `file-history-snapshot` and `file-history-delta`, linked to ~/.claude/file-history/<id>@vN.
   - `pr-link` (prNumber, prRepository, prUrl).
   - gitBranch, cwd, cost/turn timing.
   This already gives "every edit between commits, linked to the conversation that produced it", which is the core of DeltaDB.
3. `http` hooks to the daemon, used only for the real-time signals the transcript lacks:
   - SessionStart (register; returns additionalContext).
   - UserPromptSubmit (records intent; returns additionalContext).
   - PostToolUse on Edit|Write|MultiEdit|NotebookEdit (immediate file-touch event used for overlap warnings).
   - PostToolUse on Bash (catches `git commit` / `gh pr create` immediately, like tsukumo's ledger).
   - Stop (`last_assistant_message`, used for the turn summary).
   - Notification `idle_prompt` / `permission_prompt` / `agent_needs_input` (presence becomes "waiting on Stephen", shown on every device).
   - PreCompact (snapshots the card before context is lost), SessionEnd.
4. Git: watch .git/logs/HEAD for each repo_key that has live sessions. On a new commit, attach every edit event since the previous commit whose path and patch are in the commit diff → edit_links(commit, session, turn). Nothing is installed into repos.
5. Grok:
   - ~/.grok/active_sessions.json for presence.
   - An explicit `hub_status`-style update through MCP (Grok has no Claude-style hooks), plus whatever events grok-fork can POST to the daemon later.
   - Honest gap: Grok capture is coarser than Claude capture in v1.

DISTILLATION: what peers actually get
- Per-session CARD, about 120 tokens, built deterministically:
  - name, device, repo_key + branch.
  - intent (first prompt plus the latest prompt, trimmed).
  - status (busy/idle/waiting).
  - files touched (top 8 by recency).
  - last turn summary (first sentence of Stop's last_assistant_message, or an optional Haiku one-liner).
  - open tasks and leases, unanswered asks.
- Raw transcripts never replicate by default. Patches and excerpts replicate lazily, by hash, when a peer or Stephen opens them.

TOKEN BUDGETS FOR INJECTION (hard caps, enforced by the daemon)
- SessionStart: at most about 600 tokens. Cards for sessions in the same repo_key (max 4), active leases on this repo, unread messages addressed to this session or its role, and one line of how-to ("use hub_send / hub_inbox"). Nothing when the session is alone.
- UserPromptSubmit: 0 tokens by default. At most about 250 tokens only when relevant:
  - a new direct message, or
  - another live session touched a file this session touched in the last 30 minutes, or
  - a lease conflict.
  Deltas only, deduplicated against what this session was already shown (the daemon tracks per-session "seen" watermarks).
- PreToolUse (Edit/Write): at most about 80 tokens, warn only and never block. "src/x.ts is leased by engine-2 on WIN-PC (12 min left) / was edited by session Y 4 min ago." Blocking is opt-in per repo.
- MCP tool definitions: about 8 tools with short descriptions, roughly 1.5k tokens of schema. Claude Code defers MCP tools behind ToolSearch, but Grok may load all of them, which is why suzuri's 30 tools get cut.

### Messaging mechanism

MCP TOOLS (the whole agent surface)
- `hub_who`: cards, filterable by repo, device or role.
- `hub_context(session|path|repo|commit)`: who touched what and why, edit→turn→commit links, KB hits.
- `hub_send(to, body, kind=ask|fyi|handoff|review, about?)`.
- `hub_inbox`: catch-up, not a loop.
- `hub_reply(id, body)`, `hub_skip(id)`, `hub_defer(id, minutes)`.
- `hub_claim(task|path, ttl)` / `hub_release`.
- `hub_note(op, ...)`.

ADDRESSING. `to` can be any of:
- a session name, `role:engine`, `device:WIN-PC`, or `repo:<key>` (that repo's front desk);
- `owner-of:<path|PR|thread|task>`, resolved through the tsukumo-style ledger: replied-to message → claimed thread → PR opened/worked/reviewed → file lease or last editor → front desk;
- `stephen`.

DELIVERY. The daemon on the target's device holds a per-message take-lock, so exactly one session acts. Wake paths by target state:
1. Live, idle Claude session, same device as the sender (Claude→Claude):
   - Prefer native SendMessage. This build's ListAgents lists peer sessions with busy/idle, and the mod API documents `$.session.send({to:{sessionId}})` delivering "framed at the receiver as a peer's". Sessions advertise peerFeatures `notify_idle`.
   - The hub mod's `session.receive`/`session.send` hooks mirror these into the hub log, so native traffic is still captured.
   - UNVERIFIED: whether a native peer delivery starts a turn in a fully idle interactive session. That is an M3 day-1 spike. If it does not, use path 2.
2. Live, idle Claude session, any source (hub, other device, Grok, Stephen):
   - The daemon pushes over the mod's SSE stream, and the mod calls `$.prompt.submit({text: framedPrompt})`. This is exactly tsukumo's proven wake (register.tsx:110/459).
   - The prompt is fenced like wake.ts: `<hub-message from=... device=...>`, "a teammate's request, not the person's authority", plus reply/skip/defer instructions.
   - `turn.start` marks the message engaged, and from then it stays with that session.
3. Live, busy Claude session:
   - Never interrupt. The message is queued.
   - At the next UserPromptSubmit it arrives as additionalContext.
   - At Stop, if any queued item is kind=ask or handoff marked urgent, the Stop hook returns `decision:"block"` with `reason` set to the fenced message, so the session continues instead of going idle.
   - Otherwise it is delivered through path 2 once sessions/<pid>.json flips to idle.
   - Stop-block works only at the end of a turn and can't wake an already-idle session; path 2 covers that case.
4. Grok session: `notifications/claude/channel` from the `tsudoi mcp` process attached to it. grok-fork --channels turns this into a `<\channel source="tsudoi">` turn, the same path suzuri's notify.go already uses.
5. Claude Code with only MCP and no mod (fallback):
   - The same channel notification works if the session was started with `--dangerously-load-development-channels server:tsudoi`. Custom channels are a research preview, allowlist-gated, and need that flag every launch, so this is a fallback, not the plan.
   - Without it, the message waits for the next prompt.
6. Session whose process is gone (closed terminal, laptop asleep):
   - After tsukumo's 3-minute grace, a kind=ask goes to the repo's front desk (a live session) per tsukumo rules.
   - If Stephen enabled auto-resume for that repo, the owning device runs `claude --bg --resume <sessionId>` (verified flag; it "continues that session in the background under the same ID"), optionally in a suzuri pane via aicontrol, and delivers through path 2 once its mod registers.
   - kind=fyi never resurrects anything.
7. Session on another device: the message is replicated over iroh to the target's daemon, which applies paths 1–6 locally. If that device is offline, the message waits in the log and Stephen sees "queued for WIN-PC (offline)".
8. Stephen: GUI inbox with tray toast. Messages to Stephen never wake agents. His replies are framed as coming from the person, and the hub mod submits them with `asUser: true` only when typed in the hub GUI on a paired device.

NOISE CONTROL (from tsukumo, enforced by the daemon, not by prompting)
- kind=fyi never wakes; it is batched into the next turn's context and dropped after 2 hours unread.
- Pair rate limit (DIRECT_LIMIT-style): N messages per pair per 10 minutes, and a hop counter of 3 on reply chains. Sends past the limit are refused with "stop the back-and-forth; ask Stephen".
- "Don't reply to acknowledge" in every framed prompt; `hub_skip` closes a message without a post.
- Per-session block/allow lists and an everyone/allowlist mode, changed only by Stephen in the GUI or a slash command, never by a model or a message.
- Held messages shown in the GUI with Deliver / Dismiss.
- The 10-minute unengaged sweeper moves new work to the front desk. Deferred or engaged work never moves; reminders only.

### Cross device

- Identity: each install creates an iroh secret key on first run (hato-core identity). The device id is its endpoint id.
- Pairing: Devices → "Pair": a short spoken code or QR (hato-code SPAKE2 over the mailbox/relay), then both sides store each other as contacts. No accounts and no server Stephen has to run. Unpairing revokes.
- Replication: on connect, peers exchange {device_id: max_seq} vectors and stream missing events. Events are immutable, so merging is a union and ordering uses HLC timestamps. Large blobs (patches, transcript excerpts, attachments up to 64 MiB, matching suzuri) are pulled lazily by hash. Mutable state is owned by the authoritative device (lease/task holder, target session's device for take-locks). The design avoids distributed consensus and avoids text CRDTs; replicating the working tree is not a v1 goal.
- Paths: everything is keyed by repo_key (normalized git remote, else root-commit hash) plus a POSIX relative path, so the same file on the Mac and on C:\Users\me matches.
- What crosses devices by default: cards, messages, presence, edit metadata (path, hunk headers, patch hash), commit links, tasks/leases, notes. Full transcripts stay local unless opened.
- Remote wake: a message addressed to a Windows session goes Mac daemon → iroh → Windows daemon → that session's hub mod `$.prompt.submit` (or Grok channel push, or `claude --bg --resume` on Windows). This is the milestone where Discord is truly replaced, because Discord was also tsukumo's Mac↔Windows transport.
- Network: iroh tries hole-punching, then n0's public relays. Self-hosting a relay is a later option. When a device is offline, everything queues in the log with an explicit "queued for X (offline)" state; there is no silent loss.
- Cloud runners and Remote Control: out of v1. A cloud Claude session can't run the mod or reach loopback hooks. Revisit when a hub can run headless on a VPS as a paired device.

### Packaging and updates

Two donors, merged in M0:
- Windows (from toru):
  - The release.yml job on windows-latest stamps the version via ldflags and info.json, then builds a per-user NSIS installer (no UAC) and a portable zip, both with SHA256SUMS.
  - The updater is toru internal/update: query the GitHub Releases API, check semver isNewer (dev builds never nag), download the installer in the background, verify SHA256SUMS, enter a "ready" phase, then either Restart-to-update (silent installer, relaunch) or InstallOnExit (/NORELAUNCH).
  - Keep toru's choice of running the real installer instead of Wails pkg/updater's bare exe swap.
- macOS (from suzuri):
  - A macos-latest arm64 job builds a .dmg and .app.zip with Developer ID signing and notarization (packaging/macos/SIGNING.md).
  - The updater replaces the binary inside the .app and re-signs/heals: internal/update heal_darwin.go, resign_darwin.go, infoplist_darwin.go.
- Sidecar: the release builds tsudoi-net (Rust/iroh) per platform and ships it next to the main binary. It is signed on macOS and covered by the same SHA256SUMS.
- Agent integration ships with the app:
  - The Claude mod folder and the `tsudoi mcp` command live inside the install.
  - On first run, with explicit consent, the app writes the http hooks, `CLAUDE_CODE_PLUGIN_DIRS` entry and MCP registration (`claude mcp add tsudoi -- <path> mcp`, `grok mcp add ...`) into the user's configs.
  - Each update rewrites only its own entries; uninstall removes them.
  - A settings "Doctor" tab shows hook, mod and MCP health per agent CLI.
- Updating while agents are live:
  - The event log and queues are persisted, and hooks fail open, so a restart loses nothing.
  - The mod reconnects with backoff, and sessions keep claims through a 2-minute grace (as tsukumo's reload does).
  - Wire protocol and event schema carry versions. A newer daemon must accept events from an older paired device, and an older one ignores unknown kinds. tsukumo already handled mixed old/new watchers, and the same discipline applies across devices.
- Same tray, autostart-repair and toast patterns as toru.

### Milestones

- M0 – Empty shell that ships (1 wk): new repo StephenSHorton/tsudoi built from toru's Wails v3 template with mokei theme, tray, autostart. release.yml with a Windows NSIS/zip job (toru) and a macOS signed dmg job (suzuri), SHA256SUMS, and both updaters working end to end. Done when v0.1.0 auto-updates to v0.1.1 on the Mac and a Windows PC.
- M1 – See all my agents (local capture): daemon plus SQLite. Watchers on ~/.claude/sessions and ~/.grok/active_sessions, transcript tailer, http hooks installed via Doctor. GUI shows live sessions by repo with status, intent, files touched and a per-session timeline of prompts, edits and turn summaries. Replaces: the habit of flipping between terminals to check what each one is doing.
- M2 – Context for agents: `tsudoi mcp` with hub_who/hub_context/hub_inbox, budgeted SessionStart/UserPromptSubmit/PreToolUse injection with seen-watermarks, KB read-only index, Grok registered via MCP. Measure the token cost per turn and keep it under the budgets.
- M3 – Talk and wake on one machine (tsukumo replaced locally): start with a day-1 spike to check whether native SendMessage wakes a fully idle session. Port tsukumo router/core.ts to Go with its tests (ledger, take-lock, front desk, sweeper, defer, held, block/allow, rate limits). Hub mod with $.prompt.submit wake, turn.start engagement and inbox pane. Grok wake via channel notification. Stephen's GUI inbox and compose. Stop-hook continuation for busy sessions.
- M4 – Across devices (Discord fully gone): fork suzuri libs/transfer into tsudoi-net, pairing by code/QR, event-log replication, repo_key path normalization, remote wake Mac↔Windows, offline queues. Done when a Mac Claude session can hub_send to a Windows Claude session that is idle and get a reply, and Stephen removes tsukumo from settings.json and archives the repo.
- M5 – Absorb the workspace pieces: tasks, path leases, channels and topics from suzuri's workspace model. Unified notes (aizuchi Scratch Pad + suzuri notes, one tool). suzuri's Workspace pane becomes a hub client or is removed, and suzuri workspace-sync is retired.
- M6 – Delta-like history: edit→turn→session→commit links from .git/logs/HEAD watching. The query 'why did this line change' answers with the conversation turn even before the commit. Anchored comments stored as (repo_key, path, base commit, line range, content hash) and re-anchored through diffs, with 'orphaned' shown honestly when re-anchoring fails.
- M7 – Spawn and resume: auto-resume dead sessions (`claude --bg --resume`) per repo policy, 'open in suzuri pane' via aicontrol, front-desk recommendation to start a new session, optional isolated worktree for review requests.
- Later / out of scope for v1: federation with other people's hubs (tsukumo's shared-server use case), a headless hub on a VPS as a cloud runner, live replication of uncommitted working trees, porting aizuchi's mind-map meetings.

### Risks

- Waking is the least stable piece.
- The Claude mod API (`$.prompt.submit`, `session.send`/`receive`, CLAUDE_CODE_PLUGIN_DIRS) is what tsukumo depends on today, but it can change between Claude Code releases.
- Custom channels are a research preview, allowlist-gated, and need --dangerously-load-development-channels on every launch.
- Native SendMessage waking an idle session is unverified.
- Mitigation: put the wake backends behind one interface with ordered fallbacks (native peer → mod submit → Stop-continue → next-prompt context), add a Doctor health check per session, and pin tested Claude Code versions in CI.
- A third stranded attempt. ai-office stalled, and suzuri's workspace is a half-hub. Consolidating six tools is a rewrite trap.
- Mitigation: each milestone must retire something Stephen uses (M3 local tsukumo, M4 Discord, M5 suzuri workspace) and is not done until the old thing is switched off.
- Do not port aizuchi meetings or ai-office's office scene.
- Noise and token cost. Agents messaging each other can loop, flood context, or burn turns being polite.
- Mitigation: limits enforced by the daemon, not the prompt: FYI never wakes, pair rate limits, a 3-hop cap, hub_skip, injection token caps with seen-watermarks, a small tool surface (Grok may not defer-load MCP schemas).
- Real-world tuning will still take iterations.
- Cross-agent and cross-device prompt injection plus authority confusion. A message from a peer, or from a compromised paired device, could ask a session to do something harmful.
- Mitigation: every delivery is fenced and labelled 'not the person's authority' (as wake.ts does).
- asUser is only for messages Stephen typed in the GUI.
- Block/allow changes come only from Stephen.
- Permission prompts are never relayed or auto-approved by the hub in v1, so a remotely woken session may simply pause on a permission prompt; it shows as 'waiting on Stephen'.
- Exactly-once delivery and ownership across devices: duplicate wakes or lost messages during partitions, sleeps and reloads.
- Mitigation: single-owner authority (target device owns the take-lock), engaged/deferred state that never moves, 3-minute gone-grace, explicit offline queues.
- Edge cases will exist; tsukumo's state machine tests are the starting suite.
- Cross-platform details. Windows vs macOS paths, shells and file watching; unix sockets don't fit (use loopback TCP + token); iroh relay reachability on restrictive networks and reliance on n0's public relays; clock skew (HLC); cgo-free SQLite performance with large transcript tailing.
- Depending on undocumented Claude Code internals for capture: the ~/.claude/sessions/*.json schema, transcript JSONL record shapes (structuredPatch, file-history-delta) and the peer socket can change without notice.
- Mitigation: tolerant parsers, keep hooks (documented) as the authoritative real-time signal, and treat the internals as enrichment.
- Live updates and version skew. Paired devices run different hub versions, and the mod, hooks and daemon must stay compatible across updates.
- Mitigation: versioned event schema and wire protocol, ignore unknown kinds, the installer owns and rewrites only its own config entries, no forced restarts (toru's restart-now-or-on-quit).
- Grok parity is weaker: no hooks, so capture there is coarser (presence plus explicit MCP status) until grok-fork can POST events itself. That is acceptable since it's Stephen's fork, but it is extra work in another repo.

## 4. Yoriai (寄合, "a gathering"): a working name for the one hub app

Build one new desktop app with Toru's stack (Wails v3, Go, React 19, shadcn styled with the mokei registry). On each machine it runs from the tray and does four jobs: it is the agent hub daemon, the UI, the MCP server, and the home for notes and the knowledge base. The fastest credible path to "useful within 2 weeks" is to take parts that already exist and work, not to design from nothing.
(1) The data model comes from suzuri's `internal/workspace`. It is Go, about 3.5k lines on main, and already has channels, members with idle/working/waiting/blocked status, an inbox, claimable tasks, exclusive path leases and identity-by-session. It gets ported onto a SQLite append-only event log.
(2) The wake path comes from tsukumo's Claude Code mod. An event arrives, then `$.prompt.submit`, then a new turn in the right session. All of Discord is removed: gateway, bot token, IDENTIFY budget, eyes reactions and typing indicator, front-desk sweeper, PR-reference routing.
(3) Packaging comes from Toru: release.yml, the per-user NSIS installer plus a portable zip, the GitHub Releases updater with SHA256SUMS that installs on quit. macOS signing, notarization and .app healing come from suzuri, because Toru has never shipped on a Mac.
Week 1 replaces Discord: you type to a session in the hub and it wakes and replies, and sessions message each other through the hub. Week 2 adds aizuchi scratch-pad notes, the knowledge base, capture v1 (the record of what each agent did and edited) and signed Mac and Windows releases. Weeks 3-4 add devices, using iroh pairing with a ticket, the same idea as Send Files.
The project is honest about scope. Delta-style real-time worktree replication is out; git stays the way code syncs. A Claude session that has been closed cannot be woken. Grok's history is captured less completely than Claude's. Exclusive claims across devices are only best-effort while machines are disconnected.

### Architecture

COMPONENTS (each machine runs exactly one hub)
1. Hub app: one Wails v3 binary per machine. It is single-instance, sits in the tray, starts at login, and keeps running when the window is closed.
   - It follows the mead/wc3-forge pattern. Wails-bound Go services (Agents, Threads, Tasks, Leases, Notes, KB, Capture, Devices) are the one control surface, used by the React UI, by the MCP tools and by the mod's HTTP API.
   - It listens on 127.0.0.1:<random port> with a bearer token. It writes a discovery file at ~/.yoriai/hub.json (on Windows, %LOCALAPPDATA%\yoriai\hub.json) holding the URL, token and device id. This is the suzuri `aicontrol` pattern (loopback HTTP + token + discovery file + env injection).
   - It does not use unix sockets as tsukumo's `.watch/router.sock` does, because the hub must also run on Windows.
2. Hub mod for Claude Code: a fork of tsukumo/claude-mod with Discord stripped out.
   - It talks to the hub with `$.http.fetch` only, by long-polling `GET /v1/agents/:sid/inbox?wait=25s`. It never assumes the fetch can stream.
   - It spawns no Bun sidecar and needs no checkout. This removes tsukumo's `tsukumoDir`/`bunPath` fragility, so "install hub + enable mod" is the whole setup on both OSes.
   - It does five things. (a) On `session.start` it registers the session id, cwd, repo and a label read from `~/.claude/sessions/<pid>.json`. (b) It delivers inbox items with `$.prompt.submit`. (c) On `turn.start` and `turn.complete` it reports engagement and busy/idle status. (d) It streams capture events from `session.append` and `tool.call`. (e) It registers the hub tools with `$.tool.register` (hub_send, hub_inbox, list_agents, task_*, lease_*, note_*, kb_*, decision_record), so there is no MCP round-trip.
   - A `/hub` command and a small pane show the inbox and the senders that are held back. Block/allow lists can only be changed by a person, which is the same rule tsukumo enforces today.
3. `yoriai mcp` stdio shim: the same binary run as a subcommand, spawned on demand like suzuri `mcpsrv`. It attaches to the running hub over loopback HTTP.
   - It serves Grok (grok-build), Cursor and anything else that speaks MCP.
   - It holds the hub inbox open and sends `notifications/claude/channel`, which `grok-fork --channels` turns into a turn. This path is already proven by tsukumo's `bind` and by suzuri `workspace_wait`.
   - The hub also exposes `/mcp` over Streamable HTTP for clients that do not need to be woken.
4. `yoriai-sync`: a Rust iroh sidecar built from suzuri `libs/transfer` (hato-core identity) plus the `workspace-sync` crate. That crate is on the unmerged branch `feat/workspace-mcp-channels`, not on main.
   - It speaks NDJSON in "machine mode", like suzuri-transfer.
   - It is not needed for the single-machine MVP.
5. Passive watchers that work even when the mod is not loaded:
   - `~/.claude/sessions/*.json`: live Claude sessions with pid, sessionId, cwd, name, status busy/idle, messagingSocketPath and peerProtocol.
   - An incremental tail of `~/.claude/projects/*/*.jsonl`. Tsukumo's ledger already reads these for pr-link rows.
   - Suzuri `report_agent` pane states, through its aicontrol API, when suzuri is running.

DATA MODEL
- Storage: SQLite at ~/.yoriai/hub.db.
- Source of truth: an append-only `events` table with (device_id, seq, hlc_ts, actor_id, kind, payload_json, refs). Each row is immutable and owned by the device that created it.
- Materialized tables, rebuilt from the events:
  - devices
  - agents: id = `<label>@<device>`, kind claude|grok|human, session_id, cwd, repo, status, last_seen, alias list
  - threads: channels plus direct-message pairs, ported from suzuri workspace channels with their topics
  - messages: mentions resolved to agent ids, fenced as untrusted on delivery
  - deliveries: message to agent, with state queued | delivered | engaged (turn.start) | deferred | answered | skipped. This is the "take" from tsukumo's take.ts: one taker per message.
  - tasks and leases: from suzuri tasks.go and leases.go, with exclusive claim and TTL
  - turns: session, prompt row uuid, start, end, status
  - captures: `session.append` row uuid, door, origin, agentId, and a text excerpt rather than a full duplicate of the transcript
  - edits: turn_id, tool, path, before/after blob hash, unified patch
  - decisions
  - notes: imported from aizuchi
  - kb_index: FTS over the knowledge-base repo
  - ledger: who opened, reviewed or posted what, from tsukumo ledger.ts
- Blobs: file snapshots taken at edit time go into a content-addressed store at ~/.yoriai/blobs, keyed by `git hash-object`-compatible SHA so they can be matched to commits later.

HOW AGENTS CONNECT
- Claude Code sessions (terminal, suzuri pane or the desktop app's Code tab) connect through the mod, with the passive watchers as a fallback.
- Grok, Cursor and other clients connect through the `yoriai mcp` shim.
- Humans connect through the hub UI on any device.

HOW DEVICES CONNECT
- Each device runs its own hub. Hubs pair once with an iroh ticket (EndpointAddr) and then replicate the event log.
- Each hub delivers to the agents attached to it. Remote wake is "replicate a message event to the target's device; that device's hub delivers it locally".
- There is no central server and no SpacetimeDB.

### Tech stack

- Shell and backend: Wails v3 pinned at v3.0.0-alpha.98 (the same pin as Toru) with Go. Packages:
  - `modelcontextprotocol/go-sdk`: already used by suzuri mcpsrv; stdio plus Streamable HTTP
  - `modernc.org/sqlite`: pure Go with no cgo, so Windows cross-compiles cleanly; FTS5 must be confirmed compiled in on day 1, with LIKE plus a trigram table as the fallback
  - fsnotify
  - plain `git` called as a subprocess, not go-git: it uses the user's own config and credentials
- Toru packages to reuse: tray, autostart and the single-instance logic.
- Frontend: React 19, TypeScript, Vite, Tailwind v4, shadcn, all built with Bun.
  - Mokei supplies the look: the theme from `npx shadcn add @mokei/theme @mokei/card @mokei/badge @mokei/tabs …` out of the Pages registry, plus `mokei` as a git dependency for `applyTheme('yardline')`. That gives white glass on clay with at most three accents.
  - Views stay plain: lists, threads and a timeline. There is no 3D, which is a lesson from ai-office.
- Claude Code integration: a TSX mod written against the `$` mod API (hooks module `register.tsx`, as in tsukumo), tested with `claude plugin validate/test`.
- Cross-device: a Rust iroh 1.0 sidecar (`yoriai-sync`) forked from suzuri `libs/transfer`. It reuses hato-core identity, the ticket format and the workspace-sync merge logic.
- CI: GitHub Actions. Windows uses NSIS, macOS uses codesign, notarytool and a dmg. Rust is only added once the sync sidecar lands.

### Consolidation plan

TSUKUMO: absorbed, then archived.
- What is kept:
  - The mod skeleton: session.start registration, `$.prompt.submit` wake, `turn.start`/`turn.complete` engagement, the pane, held mentions, block/allow lists with person-only changes, and the `<discord-message>`-style fencing renamed to `<hub-message from=… trust=…>`.
  - `message_chat`/`list_chats`, which were already Discord-free, become `hub_send`/`list_agents`.
  - Concepts ported to Go and simplified: the take lock (lib/take.ts), engagement and deferral (sweep.ts minus the front-desk reassignment), liveness heartbeats (30s heartbeat, 90s = gone), the ledger (ledger.ts plus refs.ts for "who owns webapp#12"), and the anti-loop limit (`DIRECT_LIMIT` 8 messages per pair per 10 minutes).
- What is deleted:
  - The Discord gateway, the REST client, the IDENTIFY budget, eyes reactions and typing indicators, GIF upload, nonce dedupe, Cursor-PR detection, the front-desk sweeper and the hand-off timers.
  - Most of router/core.ts (894 lines) exists only because every agent shares one Discord bot. In the hub a person addresses an agent directly, so routing collapses to: explicit target, else thread owner, else ledger owner, else the "Unassigned" inbox in the UI.
- Cut-over: tsukumo's MCP server and mod keep running side by side until hub W1 passes its acceptance test. Then you switch `CLAUDE_CODE_PLUGIN_DIRS` from tsukumo/claude-mod to the hub mod (the hub shows that settings.json diff and never writes it unasked), archive the repo, and remove the Discord bot.

SUZURI: stays as the terminal host and loses its chat room.
- `internal/workspace` (store, ops, inbox, identity, tasks, leases, wait; 14 files on main) is copied into the hub as its core, moving from JSONL files to SQLite.
- Suzuri's workspace pane, its `workspace_*` MCP tools and `workspacesync` are removed, or turned into a thin client of the hub, in a later suzuri PR.
- The unmerged `workspace-sync` crate is not merged into suzuri. It becomes the basis for `yoriai-sync`.
- Kept and used by the hub: `aicontrol` (later, so the hub can open or focus a pane for an agent and read `report_agent` states), `suzuri-transfer` (unchanged), and the macOS release/sign/update code (copied).

AIZUCHI: split.
- Scratch Pad is absorbed. Hub Notes runs a one-time import of `~/.aizuchi/notes.json` (and remote-notes.json), keeps the MCP tool names `note_create/list/read/update` so habits carry over, and replaces aizuchi's LAN mDNS/TCP note sharing with hub sync.
- Live mind-map meetings (audio, diarization, transcript import) stay in aizuchi for now; they are not in the MVP. A Meetings view in the hub is a later decision.
- After the import, aizuchi's scratch-pad UI and note tools are retired.

KNOWLEDGE-BASE: the repo stays the source of truth.
- The hub mounts ~/projects/knowledge-base as a working tree, indexes entries/*.md with FTS, offers browse and edit, and becomes the syncer, running `git pull --rebase` on an interval and commit/push when you save.
- It exposes `kb_search/kb_read` to all agents.
- `kb_write` from an agent only creates a draft that Stephen must approve in the UI. This honours his rule that the KB holds only documents he chose to keep.
- The `/kb` skill symlinks keep working unchanged.

AI-OFFICE: retired; only its lessons carry over.
- It hosted its own Ollama/AI-SDK agents instead of attaching to the agents Stephen actually uses (TASKS.md is about a multi-backend LLM registry). The hub attaches to real Claude Code and Grok sessions and never runs an agent runtime.
- Its 3D scene was ripped out (commit "Remove Three.js scene"), so novelty visuals cost more than they gave.
- SpacetimeDB needed a server; the hub is server-free and local-first.
- Its autonomous reflection and meeting loops were a trap; in the hub only humans and agents send messages that wake anyone.

TORU: unchanged as a product. Its `internal/update`, release.yml, NSIS script and tray/autostart code are the template the hub copies.

GROK-BUILD: no changes; it connects through the `yoriai mcp` shim with `--channels`.

MEAD / WC3-FORGE: only the pattern is borrowed (a single binary that is both the GUI and the MCP server, sharing one service layer).

### Capture mechanism

There are three tiers, from richest to most limited.

TIER 1: Claude Code through the hub mod (live, high fidelity)
- `session.append` fires once per stored row (prompt, response block, tool result, notice) with a stable `uuid` (the same id as in the transcript file), `door`, `origin` (person / model / tool / plugin / peer) and `agentId` (subagent).
- `tool.call` gives the tool name and input. For Edit, Write, MultiEdit and NotebookEdit the mod sends the path and the old/new strings. For Bash it sends the command, and the ledger notes `gh pr create/review` from it, as tsukumo already does.
- `turn.start` and `turn.complete` give the turn boundaries.
- The mod batches all of this and POSTs it to `/v1/capture` every second or so, and also when a turn ends.
- The hub turns this into a timeline: turn → prompt → tool calls → edits.
- For each edit the hub snapshots the file after the tool result arrives into the blob store, and records a patch against the previous snapshot or against HEAD. That is the "edits between commits, linked to the conversation that produced them" part of Delta, without replicating the worktree.
- When a commit lands (a post-commit watcher on known repos, or polling `git log`), blob hashes are matched to commits, so each commit links back to the turns that produced its lines.
- Peer messages are also logged into the hub thread, so the history is complete: `session.send`/`session.receive` with origin `peer`/`peer-send-message`, i.e. native SendMessage.
- Decisions: a `decision_record` tool that agents are told about in the mod's prompt context, plus a "mark as decision" button on any message.
- Optionally later: `$.model.complete` for a one-line summary per turn.
- Claude Code's own per-session `~/.claude/file-history/<session>/<hash>@vN` backups exist, but they use an undocumented format. They are read only as a fallback for recovery, never relied on.

TIER 2: Claude Code without the mod (passive)
- The hub reads status from `~/.claude/sessions/*.json` (busy/idle, cwd, name).
- It incrementally tails `~/.claude/projects/*/*.jsonl` to rebuild turns, tool_use edits and pr-link rows.
- This gives a few seconds of lag and no way to wake the session.

TIER 3: Grok and other MCP clients
- Grok has no `session.append` equivalent. Capture is limited to what the agent reports through hub tools (`hub_status`, `decision_record`, task and lease calls) plus an fsnotify watcher on the agent's cwd, with edits attributed by time window and lease ownership.
- This asymmetry is shown in the UI as a fidelity badge per agent.
- If grok-build gets a hook later, the shim can accept the same capture schema.

VOLUME AND PRIVACY
- Excerpts and pointers are stored, not full duplicate transcripts.
- Captures are indexed by row uuid, so the full row can always be re-read from the JSONL.
- Secret scrubbing (env/token patterns) runs before storage.
- Capture is per-device. Only summaries, edits and messages replicate by default; full captures replicate only when you opt in per device.

### Messaging mechanism

ADDRESSING
- Every agent has an id `<label>@<device>`, plus aliases: its session id, its suzuri pane title, and any `from` name it has used (taken from tsukumo's aliasing).
- A message can go to an agent, to a thread (members who are watching it), to a role or task owner (from suzuri claim_role and tasks), or to the "Unassigned" inbox.

HOW A MESSAGE IS DELIVERED
1. A person, or agent A (via `hub_send`), creates a `message` event. The hub resolves the target and creates a `delivery` row in state queued.
2. The target is a Claude Code session on this device with the hub mod: its long-poll on `GET /v1/agents/:sid/inbox` returns the delivery. The mod applies the block/allow and held rules, then calls `$.prompt.submit({text: <hub-message from=A thread=… id=…>…</hub-message> + reply instructions})`.
   - That submit is "a turn of its own once the session is idle". An idle session wakes immediately, and a busy one queues it behind the current turn.
   - On `turn.start` the mod marks the delivery engaged. The reply is `hub_send reply_to=<id>`, which marks it answered. `hub_skip` and `hub_defer` (up to 12h) are ported from tsukumo.
3. The target is a Grok session: the `yoriai mcp` shim the session spawned holds the hub inbox open and sends `notifications/claude/channel`. `grok-fork --channels` makes that a turn. This is exactly how suzuri workspace and tsukumo `bind` wake Grok today.
4. The target is on another device: the message event replicates over `yoriai-sync`, and that device's hub runs step 2 or 3.
5. The target is offline (no live mod poll for more than 90s, or its process is gone): the delivery stays queued. The UI shows it as waiting, and it is delivered when the session next attaches (`--resume` plus mod registration with the same session id).
   - This improves on tsukumo, where messages sent while nothing was watching were lost.
   - Closed sessions cannot be woken. Sessions started through Dispatch refuse cross-session messages.
   - Automatically reviving a session (launching `claude --resume <id>` in a suzuri pane through aicontrol `split`, or in a new terminal) is explicitly post-MVP.

SAFEGUARDS
- Anti-loop limit of 8 messages per agent pair per 10 minutes (tsukumo `DIRECT_LIMIT`), with overflow held for a person.
- One taker per message.
- Reminders come from the holder's own mod when a deferral passes.
- Message text is always fenced as untrusted. Block/allow lists and wake mode change only by a person's action.

THE NATIVE CLAUDE CODE PATH IS COMPLEMENTARY, NOT REPLACED
- Claude-to-Claude on one machine already works with ListAgents/SendMessage over `messagingSocketPath` with `notify_idle`. When Remote Control is connected, it also reaches the same account's sessions on other machines.
- The hub does not reimplement or depend on that protocol, which is undocumented. It only observes those messages (through the mod's `session.send`/`session.receive`) so they appear in hub threads.
- The hub's own `hub_send` is the path that also works for Grok, for people and across devices without Anthropic's relay.

DISCORD: none at all.

### Cross device

MODEL
- Every machine (the Mac and each Windows PC) runs its own hub with its own SQLite.
- Pairing is one sentence, the same product language as Toru Send Files and suzuri Share: "Copy ticket" on device A, "Join" with the ticket on device B.
- `yoriai-sync` (iroh 1.0, from suzuri libs/transfer and the workspace-sync crate) keeps a persistent identity key per device (`identity.secret`) and a contact list of paired endpoint ids. After pairing, devices reconnect automatically with iroh's hole punching plus relay, with no ports to open.
- Ticket = capability, with no further authentication in v1, exactly as suzuri's own workspace-iroh doc states. Pairing in the UI must show a SAS (short authentication string) or confirmation, and you can revoke a device.

REPLICATION
- Each device's events are an append-only log keyed by (device_id, seq).
- Sync swaps version vectors and ships the missing events in both directions. Messages, captures, edits, decisions and deliveries are owned by one device and immutable, so they never conflict.
- Mutable state:
  - Note bodies, thread topics and agent labels: last-writer-wins by HLC timestamp, with the losing version kept as history. A CRDT (Loro or Yjs) for notes is later.
  - Task claims and path leases: a task created on device X is arbitrated by X when X is reachable. While devices are disconnected each side may claim, and on reconnect the hub surfaces a "double-claimed" conflict to a person. It does not pretend to be strongly consistent.
- Blobs (edit snapshots, attachments) are fetched lazily as iroh blobs by hash.

WHAT DOES NOT SYNC IN THE MVP
- The code worktree itself. Git and GitHub remain the way code moves between machines. Delta-style real-time worktree replication is out of scope.
- Full captures sync only if you opt in per device.
- The KB syncs through its own git remote, as today.

WINDOWS SPECIFICS
- Loopback HTTP and the `%LOCALAPPDATA%\yoriai` paths behave the same on Windows.
- Whether Claude Code mods (CLAUDE_CODE_PLUGIN_DIRS or a marketplace install) load on Windows must be checked on day 1. If they do not, those machines fall back to Tier 2 capture plus the MCP tools, and wake depends on mod support.

AFTER THE MVP
- An optional always-on "home" hub (the Mac, or a cheap VPS running the same binary headless) that keeps events while laptops sleep.
- Cloud runners and isolated review worktrees (`git worktree add` per review), copying Delta's idea.

### Packaging and updates

This is a blend of Toru (Windows) and suzuri (macOS), because Toru has never shipped on a Mac and deliberately avoids Wails' built-in updater.

RELEASE PIPELINE
- `release.yml` on a `v*` tag runs a matrix.
- Windows (windows-latest), copied almost verbatim from Toru:
  - wails3 task windows:build with VERSION injected through -ldflags, and the Win32 resource stamped
  - per-user NSIS installer with no UAC prompt, a portable zip and SHA256SUMS
  - no Authenticode signature at first (SmartScreen will warn), the same as Toru and suzuri
- macOS (macos-14, arm64; Intel optional), from suzuri release.yml and packaging/macos/SIGNING.md:
  - .app bundle, Developer ID signing plus notarization when the secrets are present, .dmg plus .app.zip
- Both create one GitHub Release with all assets and SHA256SUMS.
- The bundle carries the hub binary (which is also the `yoriai mcp` shim), the mod folder and, from week 3, the `yoriai-sync` sidecar for each OS. Everything therefore updates together.

UPDATER
- Generalize Toru's `internal/update` (488 lines): GitHub Releases API, semver compare, background download, SHA256SUMS check, phases idle/checking/downloading/ready/installing/error, the `update:status` event, and "Restart now" or install on quit.
- Each OS gets an installer adapter:
  - Windows: the silent NSIS installer with /NORELAUNCH on quit, which is Toru's approach and keeps the registry and shortcuts coherent.
  - macOS: replace the payload inside the .app, then use suzuri `heal_darwin.go`/`resign_darwin.go`/`infoplist_darwin.go` for re-signing and quarantine.
- Updates are checked at start and every 6 hours, and on demand from the tray.

UPDATING THE MOD
- The app extracts its mod to the stable path ~/.yoriai/mod/current, replaced atomically with a rename on each update.
- On the machine you develop on, `CLAUDE_CODE_PLUGIN_DIRS` points there and `CLAUDE_CODE_PLUGIN_DIR_WATCH=1` reloads running sessions when it changes.
- Other machines install the mod from the hub repo's `.claude-plugin/marketplace.json`, as tsukumo does; Settings shows "mod out of date, run `claude plugin update`".
- The separator for listing several plugin directories in `CLAUDE_CODE_PLUGIN_DIRS` is not verified, so the hub does not assume it can sit alongside tsukumo's mod through that variable. You swap one for the other at cut-over.
- The hub shows the settings.json change and never writes it unasked.

OTHER
- Tray and start-at-login: the hub keeps running when its window is closed, so agents can always reach it.
- Repo: github.com/StephenSHorton/yoriai, MIT, commits under the noreply email, matching his other public repos.

### Milestones

- Days 1-2 (W1): Scaffold from Toru's template. Wails v3 alpha.98 with tray, autostart, single-instance, the mokei theme and an empty Agents/Threads shell. SQLite event log with modernc (check FTS5 support). Loopback HTTP, bearer token and the ~/.yoriai/hub.json discovery file. Copy release.yml for Windows only and cut v0.0.1 with the Toru updater wired in, so the pipeline exists from day 2. On day 1, check whether Claude Code mods load on his Windows PC.
- Days 3-4 (W1): Port suzuri internal/workspace (channels, members and status, inbox, tasks, leases, identity) onto the event log as Go services. These same services back the UI (Wails bindings), the `yoriai mcp` stdio shim and `/mcp` Streamable HTTP. Passive discovery of live Claude sessions from ~/.claude/sessions/*.json, plus a JSONL tail for status.
- Days 5-6 (W1): The hub mod, forked from tsukumo/claude-mod with Discord removed. It talks to the hub only through $.http.fetch (inbox long-poll), wakes with $.prompt.submit, reports turn.start and turn.complete, and registers the hub tools. It brings over block/allow, held messages, fencing, the 8-per-10-minute limit, defer and skip, and queued delivery for offline sessions. ACCEPTANCE: the tsukumo mod is unloaded and the Discord bot is idle. Stephen types to the 'api work' session in the hub; it wakes and its reply appears in the thread. Two Claude sessions and one grok-fork --channels session message each other through the hub.
- Days 7-8 (W2): Notes and KB. One-time import of aizuchi's ~/.aizuchi/notes.json with compatible note_* tools. The knowledge-base repo mounted with FTS search, browse and edit, a git pull/commit/push loop, kb_search/kb_read for agents, and kb_write as a draft that needs approval. After this, the aizuchi scratch pad can be switched off.
- Days 9-10 (W2): Capture v1. Feed from session.append and tool.call into turns, tool calls and edits, with blob snapshots and patches. Per-agent timeline, per-file history ('which conversation touched this line'), and commit matching by blob hash. decision_record. Fidelity badges (mod, passive, MCP-only), and fsnotify-based attribution for Grok.
- Days 11-12 (W2): Release v0.1.0 for macOS and Windows: suzuri's signing and notarization, the dmg, darwin heal/resign in the updater, the mod extracted to ~/.yoriai/mod/current, and a marketplace.json for other machines. Install it on the Mac and one Windows PC. Archive tsukumo once a week of daily use passes.
- Weeks 3-4: Cross-device. yoriai-sync sidecar from libs/transfer plus the unmerged workspace-sync crate. Pair by ticket with SAS confirmation and revocation. Event-log replication by version vectors. Remote wake by replicating the message and delivering it locally. Claim conflicts surfaced to a person. Rust is added to the CI matrix.
- Later, not scheduled: comments anchored to code (file, blob hash and line range, re-anchored through diffs); isolated review worktrees with git worktree; reviving closed sessions with `claude --resume` in a suzuri pane through aicontrol; an optional always-on home hub; CRDT notes; an aizuchi Meetings view; suzuri's workspace pane becoming a hub client.

### Risks

- Wails v3 is still alpha (Toru pins v3.0.0-alpha.98). Bindings, the tray and multi-window APIs can break between alphas, and the hub would be the second production app on an alpha framework. Mitigation: pin the version, keep the Go services free of Wails so a headless `yoriai serve` mode works with no GUI at all.
- Mod API churn. The Claude Code `$` API (a 17k-line d.ts for version 2.1.310) is the only way to get a true wake and live capture. Event names and shapes (prompt.submit, session.append, tool.call) can change between releases, as tsukumo's version-bump discipline shows. Mitigation: keep the mod thin, keep Tier 2 passive capture as a fallback, and run `claude plugin test` in CI.
- Whether Claude Code mods load on Windows is unverified. If they don't, the Windows PCs get MCP tools and passive capture but no true wake, which undermines 'agents wake each other' there.
- A wake is never guaranteed. $.prompt.submit only runs once the session is idle, so a busy session queues. A closed session cannot be woken, and Dispatch-started sessions refuse cross-session messages. Cross-machine wake needs the remote hub running, the mod loaded and the target session open. Persistent inboxes help, but 'wake a closed agent' requires the post-MVP resume-in-terminal work, which is fiddly (terminal choice, cwd, permissions).
- Capture is asymmetric. Claude is captured in full through session.append and tool.call; Grok only through what it self-reports and file watching. Attributing edits when two agents share a worktree, or for edits made through Bash or sed, is heuristic. Lease discipline and per-agent worktrees reduce the problem but don't solve it.
- The iroh sync code lives on suzuri's unmerged feat/workspace-mcp-channels branch, has 'no auth beyond ticket', and adds a Rust toolchain to a two-OS release matrix. Exclusive claims and leases across devices cannot be strongly consistent while machines are disconnected (last-writer-wins plus surfaced conflicts in the MVP).
- Scope creep toward Delta. Real-time worktree replication, anchored comments that survive edits, and cloud runners are each a project in their own right. The two-week plan only holds if git stays the way code syncs and Delta-like features wait until after v0.1.
- Capture volume and secrets. JSONL transcripts are large and contain tool output that may include tokens. The plan stores excerpts and pointers with scrubbing, but replicating captures across devices widens that exposure, so full captures stay per-device unless he opts in.
- Coexistence during cut-over. The tsukumo mod is loaded via CLAUDE_CODE_PLUGIN_DIRS today, and whether several directories can be listed in that variable is unverified. The two mods may not load side by side, so the cut-over must be a clean swap.

## Critiques

### Scores: feasibility 5, usefulness 7, consolidation 7

**Claims found wrong:**

- 'suzuri's iroh P2P sync is only on archive/glass-chrome, NOT on main' is partly wrong. Commit c8e2691 is indeed not an ancestor of origin/main, and main has no workspace-sync Rust crate. But main does contain internal/workspacesync (cli.go, resolve.go, from commit a0861f0 'port workspace v2'), a Go shim that shells out to a suzuri-workspace-sync sidecar binary. So the integration point is on main; the engine is not.
- The pairing code is not 'iroh pairing code' that can simply be lifted. hato-code describes itself as 'No iroh dependency — pure crypto + WebSocket', and it needs a hato-mailbox WebSocket rendezvous server. Its SPAKE2 runs over that mailbox, not over iroh, so the design's 'no cloud' claim does not hold for pairing.
- 'PostToolUse tool_response carries filePath, originalFile and structuredPatch, as confirmed in transcript records' is overstated. The transcript toolUseResult does carry these fields (verified: filePath, originalFile, structuredPatch, oldString, newString, userModified, contentNotInModelContext, plus record-level promptId, uuid and parentUuid). But that verifies the transcript, not the hook payload. The hook's tool_response shape was not checked. 'contentNotInModelContext' also hints that originalFile may be omitted or abridged in some cases.
- 'Loro setNextCommitMessage / commit origin' as provenance: the commit message is persisted, but the commit origin is local-only event metadata and is not stored in the change. Only the message can carry provenance.
- Minor: the session registry entry inspected reports version 2.1.309 while the CLI is 2.1.310. The fields (sessionId, cwd, name, status busy/idle, peerProtocol, messagingSocketPath /tmp/cc-socks/<pid>.sock) are confirmed, and --bg with --resume and `claude agents` are confirmed in --help.
- Verified correct, for the record: tsukumo's DIRECT_LIMIT=8 per DIRECT_WINDOW_MS of 10 minutes; sweep.ts UNENGAGED 10 min, GONE_GRACE 3 min, PICKUP_GRACE 90 s, MAX_DEFER 12 h; the `direct` event in src/router/core.ts and message_chat in register.tsx; ledger pr-link rows; suzuri notify.go's notifications/claude/channel; iroh 1.0 and iroh-blobs 0.103 in libs/transfer; Toru's WAILS_INSTALL_SCOPE=user NSIS and Wails v3 alpha.98; ai-office commits c792b43, 4e96aff, 75c6ed4 and the zombie fixes; CLAUDE_CODE_PLUGIN_DIRS currently pointing at tsukumo/claude-mod with DIR_WATCH=1.

**Ideas worth keeping:**

- Splitting the data into two kinds: a signed, per-device append-only event log for immutable facts (turns, tool calls, messages, wakes), with {device: maxSeq} sync, plus a small set of mutable docs. Keep the log and shrink the CRDT surface to threads, board and notes.
- Edit provenance keyed on Claude's own ids. Transcript Edit records carry promptId, uuid, parentUuid and structuredPatch with originalFile, which is enough to link every edit to the turn that made it without owning the editor.
- Capture from three sources deduplicated by Claude ids: hooks for the live signal, the transcript tailer as the source of truth, and an fs/git watcher with explicit confidence levels (exact, heuristic, human).
- Porting tsukumo's router/take/sweep/ledger with its tests and timers unchanged, and turning Discord mentions into plain hub messages. Messages to unreachable targets are held in an Inbox instead of being lost.
- Using the ~/.claude/sessions registry and `claude agents --json` for discovery and busy/idle, and `claude --bg --resume` as an opt-in revive path that needs no mod.
- Repo identity as root commit sha plus normalized remote, so Mac and Windows clones map to one repo record, with repo-relative POSIX paths and LF normalization.
- A 'Materialize as review worktree' action via git worktree add, never touching the active tree. This is the honest local equivalent of Delta's isolated review copies.
- Headless anchor/runner mode on an always-on PC as the replacement for Delta's cloud runners, and an M0 that proves the release and background-update loop on both OSes before anything else.
- Safety rules: fenced, untrusted-labeled delivered text; messages can never change trust, hook or block config; mesh endpoint allowlist; per-project approval for remote wakes.
- The mead/wc3-forge rule that every UI action is also an MCP tool, and a clear absorb-then-retire plan (tsukumo and ai-office retired, suzuri Workspace removed, KB promote-only).

### Scores: feasibility 6, usefulness 7, consolidation 6

**Claims found wrong:**

- 'workspace-sync crate and docs/workspace-iroh.md ... deleted from HEAD': both commits that touched them (c8e2691 and 9bdd319) are not ancestors of origin/main. The work was never merged, not deleted, and a newer productized version exists on branches.
- 'grok-build: ... branch feat/workspace-mcp-channels, cdcd1e1e "grok-fork channel wakes instead of wait-loop", merged': the commit is cdc1e1e, and `git merge-base --is-ancestor origin/feat/workspace-mcp-channels origin/main` says it is NOT merged. notify.go with notifications/claude/channel is on main, but the grok-fork wake commit itself is branch-only.
- It treats the Rust wgpu chrome as 'the terminal host as it is today'. The default UI on main is classic ebiten + Charm (Go). release.yml still builds suzuri-chrome, but classic is the default and the actively developed UI.
- 'Claude Code's native SendMessage/ListAgents ... Claude-only, single-machine': the native ListAgents also lists Remote Control sessions on other machines and cloud sessions when Remote Control is connected. It is Claude-only but not strictly single-machine, which weakens part of the case for building cross-machine delivery for Claude-to-Claude.
- 'Mod versioning ... CLAUDE_CODE_PLUGIN_DIR_WATCH=1 does [reload] automatically': per tsukumo's README this is false for desktop Code-tab sessions, which run the cached install copy.
- Minor: take.ts (192 lines) and ledger.ts (187 lines) are uncounted in the port estimate, and wake.ts is 396 lines. route.ts=105, sweep.ts=151, core.ts=894, register.tsx=1347 and DIRECT_LIMIT=8 are all correct.
- Verified correct, for balance: 'No React. No HTML. No Chromium.' is in chrome/README.md; notify.go uses notifications/claude/channel; SUZURI_PANE_ID is in internal/aicontrol; hato-core has identity.rs and contacts.rs; hato-mailbox is 'No database, no disk' with a 300 s slot TTL; internal/workspacesync shells out to suzuri-workspace-sync, which main's release.yml does not build; update.go is 'toru's check → download → verify → replace'; release.yml has NSIS plus Developer ID sign/notarize; go-sdk v1.7.0 is used; ~/.claude/sessions/<pid>.json has pid, sessionId, cwd, name, status, kind, entrypoint, messagingSocketPath /tmp/cc-socks and peerProtocol 1; Claude Code 2.1.310 contains notifications/claude/channel, --dangerously-load-development-channels, SubagentStop, file-history and CLAUDE_CODE_PLUGIN_DIR(S|_WATCH); mokei is React 19.

**Ideas worth keeping:**

- Run the hub as a headless daemon (`suzuri hubd`) from the same binary and release, so it survives GUI crashes and can serve desktop, grok and non-pane sessions.
- Make suzuri the consolidation base. It really does already have workspace channels, tasks, leases, presence, an MCP server with channel push, pane identity env vars, the iroh hato engine with identity, contacts and SPAKE2, and a signed/notarized updater modeled on toru.
- One loopback HTTP+WS control surface shared by the UI, MCP, mod, hooks and CLI (the mead/wc3-forge pattern).
- Port tsukumo's routing using its tests as the spec (route, take, sweep, ledger, DIRECT_LIMIT 8, fencing, human-only block/allow), and swap router.sock for a hubd WS. Delete the Discord code completely.
- Per-origin append-only event logs with ulid dedup, plus an owner-machine rule for exclusive state (leases, take-locks), which avoids split-brain.
- Capture in layers: hooks, then transcript pointers by byte offset (not copies), then git ref watching to link edits to commits, then a Stop-time git diff for edits made outside the tools. Be explicit that this does not replicate the live worktree the way DeltaDB does.
- Read ~/.claude/sessions/*.json for presence and do not touch the private /tmp/cc-socks peer protocol.
- Be honest that sleeping machines cannot be reached. An optional self-hosted always-on peer instead of a hosted service.
- Treat the ai-office lessons as retirement criteria: no hosted SpacetimeDB, no separate agent runtime, no 3D metaphor.
- Add a startup self-test that reports which wake adapters (mod, channels, PTY) actually work.

### Scores: feasibility 5, usefulness 7, consolidation 6

**Claims found wrong:**

- 'Toru packages to reuse: tray, autostart and the single-instance logic' is wrong. Toru has no such packages. tray.go, autostart_repair_windows.go and main.go sit in package main at the repo root and are Windows-specific, so this is copy-and-adapt work, not reuse. Its internal/ has capture, hotkey, transfer, update and so on, but no tray or autostart.
- The cut-over is not forced to be 'a clean swap'. Mods already load side by side in this environment: the plugin-authoring mod and tsukumo-discord both register tools through $.tool.register in the same session. The hub mod can be installed from a marketplace while tsukumo stays on CLAUDE_CODE_PLUGIN_DIRS, so the env-var separator question doesn't matter.
- The aizuchi import is understated. Its MCP header says CRUD now goes over IPC to the running Tauri app ('there is no more direct notes.json access'). ~/.aizuchi/notes.json still exists, but it may no longer be the authoritative store, so the import has to read whatever the Tauri app persists, not just notes.json.
- Updater lineage is backwards. Suzuri's internal/update (1,178 lines) already updates on macOS and Windows (heal, resign, infoplist, darwin). Generalizing Toru's Windows-only 488-line service and bolting suzuri's darwin files onto it is more work than starting from suzuri's updater. Minor: Toru's Send Files is magic-wormhole (psanford/wormhole-william), not iroh. The 'same idea' wording is fine, but none of that code carries over.

**Ideas worth keeping:**

- Verified and sound: wake via $.prompt.submit, engagement via turn.start and turn.complete, hub tools via $.tool.register, and capture via session.append and tool.call. All exist in the mod API d.ts (16,981 lines) and in tsukumo register.tsx.
- Passive watcher of ~/.claude/sessions/*.json. The fields are confirmed: pid, sessionId, cwd, name, status, messagingSocketPath, peerProtocol, peerFeatures.
- Port these tsukumo brains: take.ts, sweep.ts, ledger.ts, refs.ts (all exist), DIRECT_LIMIT=8 (core.ts:106), and the collapse of the 894-line router/core.ts to explicit target, then thread owner, then ledger owner, then Unassigned.
- Persistent queued inbox for offline sessions, an improvement on tsukumo where messages sent while nothing was watching were lost.
- Grok wake through the stdio MCP shim emitting notifications/claude/channel. Verified in suzuri internal/mcpsrv/notify.go and tsukumo's bind/--channels.
- The workspace-sync crate exists on the unmerged branch feat/workspace-mcp-channels, so using it as the basis for yoriai-sync is a real head start.
- Mokei via the shadcn registry (@mokei/theme, card, badge, tabs) plus applyTheme('yardline'), exactly as the mokei README documents. Also: no 3D, a lesson from ai-office commit c792b43 'Remove Three.js scene'.
- Go services free of Wails, with a headless `serve` mode that hedges the Wails v3 alpha.98 pin (confirmed in toru go.mod).
- kb_write only as a draft Stephen approves, which respects his KB rule; the hub shows the settings.json diff and never writes it unasked.
- Honest scope: git stays the way code syncs, there is no Delta worktree replication, and per-agent fidelity badges are shown.

### Scores: feasibility 6, usefulness 8, consolidation 7

**Claims found wrong:**

- 'ai-office stalled in March 2026': its last commit is Apr 14 2026 (minor).
- 'hato-code SPAKE2 pairing ... no server Stephen has to run': pairing needs a hato-mailbox rendezvous server (loopback dev default; remote must be wss://), and suzuri has not built contacts/pair/listen yet (docs/transfer.md checklist).
- 'Two donors: Windows from toru, macOS from suzuri' understates suzuri. suzuri's release.yml already builds a Windows NSIS setup.exe (choco nsis, packaging/windows/suzuri.nsi) and a signed/notarized macOS .dmg, with SHA256SUMS, and suzuri internal/update handles windows and darwin. One donor covers both platforms. Note: suzuri's updater picks the portable package and never the installer, unlike toru's run-the-real-installer approach. The proposal's choice of toru for Windows is a real design choice, but it is not forced.
- 'suzuri's darwin bits' for a Wails v3 app: suzuri is not a Wails app (custom Go host/window). heal/resign/infoplist_darwin.go assume its own .app layout and need adapting, not copying.
- Notification types 'idle_prompt / permission_prompt / agent_needs_input' are not enumerated in this build's types: NotificationHookInput.notification_type is just `string`. 'agent_needs_input' in particular is unverified.
- Implied evidence that ~/.grok/active_sessions.json gives Grok presence: the file exists but holds [] on this machine. That gives no evidence it is populated or what its shape is.

**Ideas worth keeping:**

- Read what Claude Code already writes. ~/.claude/sessions/<pid>.json really does carry sessionId, name, cwd, kind, status busy/idle with statusUpdatedAt, messagingSocketPath and peerFeatures incl. notify_idle. Transcripts really do carry structuredPatch, bashEditDiff, gitOperation, file-history-snapshot/delta and pr-link (prNumber), all counted in recent JSONL. Presence and edit history come for free.
- The verified wake primitives are sound. $.prompt.submit (tsukumo register.tsx:110/343/459), turn.start for engagement, $.session.send({to:{sessionId}}) documented as 'framed at the receiver as a peer's', the session.receive/session.send events, `claude --bg --resume <id>` (continues under the same ID), and Stop/SubagentStop last_assistant_message.
- Port tsukumo src/router/core.ts (894 lines; its header says 'apart from Discord and the socket') with core.test.ts (474 lines) as the spec. This includes DIRECT_LIMIT pair rate limiting, which is already in core.ts. Only the Discord-specific error strings and reply-to-Discord-message ledger keys need rewriting.
- Grok wake via notifications/claude/channel over the stdio MCP connection. suzuri internal/mcpsrv/notify.go already wraps the go-sdk (v1.7.0) transport to push this, and grok-build crates/codegen/xai-grok-mcp/src/channel.rs exists.
- Immutable per-device event logs with sequence vectors plus single-owner authority for leases and take-locks. This avoids CRDTs and consensus.
- repo_key (normalized remote or root-commit hash) + POSIX relative path as the file identity across Mac and Windows.
- Daemon-enforced noise control and token budgets with seen-watermarks: fyi never wakes, hop cap, hub_skip, PreToolUse warn-only.
- Every milestone retires a tool Stephen uses (M3 local tsukumo, M4 Discord, M5 suzuri workspace + workspacesync). This directly addresses his 'stranded systems' complaint.
- Loopback TCP + token + discovery file, as in suzuri aicontrol, instead of unix sockets, for Windows parity.

