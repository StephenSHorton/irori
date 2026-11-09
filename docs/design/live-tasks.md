# Live tasks on cards: design

*2026-11-09, card #1249. When an agent starts a background run (a Bash or PowerShell `run_in_background`, a Ctrl+B, an auto-backgrounded timeout, a background Agent or Workflow), Irori shows it on the card the agent holds: a load chip on the tile, a "Busy now" line on Home, and, for shell runs, a live read-only terminal tab inside the card with Stop. Builds on Irori main @ 4eb7c903: the mod's `tool.call` hook (`mod/hooks/register.tsx:871-876`), the attach-only route pattern (`crates/api/src/usage.rs`), `irori-term` on suzuri's `rsz-term` tag `term-v0.10.0` (inline-terminals.md), and Storage's End it (`crates/core/src/treeclean.rs:1344`).*

Paths starting `suzuri:` are in the suzuri checkout `~/.cargo/git/checkouts/suzuri-3adba20d4faec3a7/23538b2`; `types:` is `mod/.claude-plugin/types/claude-code-tools/index.d.ts` in the main checkout (gitignored, so absent in worktrees); every other path is in Irori.

## 1. What the person sees

- **A chip on the tile.** A card whose agent has a heavy run going shows one line per run: `cargo test · 12 cores · 5.8 GB · 4 min`. CPU and memory are summed over the run's whole process tree. A light run (a `sleep`, an idle dev server) shows no chip. The chip goes on `CardTile` (`frontend/src/components/cards/card-grid.tsx:510`) and, shorter (`12 cores · 5.8 GB`, full text in `title`), on Home's `ProgressRow` (`progress-rows.tsx:130`). Plain text: the tile is a `<button>` (`card-grid.tsx:532`), so no Stop on the tile.
- **"Busy now" on Home only.** One line above Home's cards, `Busy now: #54 cargo test 12 cores · 5.8 GB, #12 …`, ordered by load, each ref opening its card. Hidden when nothing is heavy. Board pages don't get it.
- **A run tab inside the card** (shell runs only). In the card's Terminal section, a tab named after the command opens by itself when the run starts and streams its output live. It never takes focus from the person's tab or the comment box. Read-only: scrolling and selecting work, typing does nothing. While it runs it has **Stop**. When it ends it says `Exited with code N`, or `Stopped` when the person pressed Stop, and stays until the card closes.
- **A Running section** in About (`AboutCard`, `card-detail.tsx:627`, after Who's on it at `:691-702`). It lists background runs (shell ones with "show tab"), agent and workflow runs (no tab: their output is a transcript or JSON, not a terminal stream), and the agent's **foreground** shell calls with time and CPU and no stream. A foreground row has **Attach** only per §5.3.
- **Runs on your other devices** show as a summary chip on the same card (`cargo test · 4 min · on Mac`, then exit and peaks), with no tab and no Stop, like `SpawnedActions` says "Attach and Logs are on {where}" (`spawned-actions.tsx:75-80`).
- **Words.** The card page already has a **Tasks** section for a thread's cards (`card-detail.tsx:745-748`). The UI says **Running** and **run**; types use `runs` / `CardRun`. A card "running a job" (roster status `shell`, `lib/cards.ts:3473-3500`) with no heavy run shows no chip; chips add detail, they don't invent a second busy rule.

## 2. Detection

### 2.1 Primary: the mod's tool.call hook

The pause gate (`register.tsx:871-876`) already wraps every tool call with no matcher. It grows a tail:

```ts
const why = pausedRefusal(...); if (why) return { result: why }
const started = Date.now()
if (isShell(e)) void postRun($, fgStart(e, started)).catch(() => {})   // never awaited
const r = await next(e)
void observeRun($, e, r, started).catch(() => {})
return r
```

A refused call is never reported, and the hook budget clock stops while `next(e)` is in flight (engine types, HookBudget). `observeRun` decides from the **result, not the input**. Shell means tool `Bash` or `PowerShell`: Windows agents background work through PowerShell too, and both results carry `backgroundTaskId`, `backgroundedByUser`, `timedOutAfterMs` (`types:5177-5205`).

| Result | Report |
|---|---|
| Shell, `backgroundTaskId` set | `start`, kind `bash` (a label for any shell run, PowerShell included). Covers `run_in_background`, Ctrl+B, auto-backgrounding, turn aborts. With background tasks disabled the call runs in the foreground and has no id. |
| Shell, no `backgroundTaskId` | `fg` (the foreground call finished): `startedAt`, `endedAt`, `interrupted`, `isError`. Pairs with the `fg-start` posted before `next(e)`. |
| `Agent`, `status:'async_launched'` (or `remote_launched`) | `start`, kind `agent`, `taskId = agentId`, `outputFile`. |
| `Workflow` (`{status:'async_launched'\|'remote_launched', taskId, taskType, workflowName, runId}`, `types:5844-5866`) | `start`, kind `workflow`, `taskId`, `workflowName` as the label. |
| `TaskStop` (`{task_id, task_type, command}`, `types:5745`) | an early `end{status:'killed'}`. There is no `KillShell`. |
| `Monitor` (`taskId`, `types:~5109`) | ignored on purpose: a monitor watches, it is not work with a tree to meter or stop. |

**The output path is only in the text.** The typed shell result has no path and no pid. Parse `r.text` with `/with ID: (\S+)\. Output is being written to: (.+?\.output)/` and take the path **verbatim**: the Windows folder is sometimes a 16-hex hash (`%TEMP%\claude\284f1fee44e7a201\…`) and sometimes a project slug (`C--Users-4step`). `backgroundOutputPath` is `@internal` and set only when the output is on another machine (`types:4645`), so it is not a fallback. Never rebuild the path.

**End.** A new `on('session.append', …)` (try/catch like `session.measure`, `register.tsx:920`; always `return next(e)`). On `e.origin.kind === 'task-notification'` read the structured `message.task` (`UserMessageTask {id, status, toolUseId, …}`); regex on `<task-id>`/`<status>`/`<summary>` only when it is absent. `exit code N` still comes from `<summary>`. Dedupe by task id (one notification can arrive as a `queue` row and again as a prompt row). Ignore door `note`: that is the mod's own `appendRow` (`register.tsx:418`).

**Pure helpers** in `mod/hooks/lib.ts`, unit-tested in `mod/unit/lib.spec.ts`: `RUNS_PATH`, `parseBgResultText`, `runStartFrom(e, r)`, `runEndFrom(row)`.

**Delivery.** Fire-and-forget via `callCore` (`register.tsx:112`), not `report()`'s 5×3 s retry. Unsent reports go to a new `runs` atom in `PluginState.irori` (`mod/types/index.d.ts:105-118`), capped like `MAX_WAKES`, flushed from `onHello` next to `postUsageNow` (`register.tsx:455`).

**Route.** `POST /v1/runs`, attach-only, **not** a registered tool (the model would see it in `/v1/tools`). The key is `caller.sender.key` (`crates/api/src/server.rs:514-552`), never the body. Body:

```json
{"event":"start|end|fg-start|fg","kind":"bash|agent|workflow|fg","taskId":"be9ihtlp2",
 "toolUseId":"toolu_…","command":"cargo test","description":"…",
 "outputFile":"C:\\…\\tasks\\be9ihtlp2.output","cwd":"(await $.session.cwd())",
 "startedAt":1762700000000,"endedAt":0,"status":"completed|failed|killed","exitCode":1,"interrupted":false}
```

**Coverage gap.** Only sessions on the new mod report. Marketplace installs stay old until `claude plugin update irori`, and Grok / `irori mcp` sessions have no mod. Hence §2.2.

### 2.2 Fallback: the tasks-folder scan

Claude Code writes every run to `<tmp>/claude/<project dir>/<session id>/tasks/<id>.output`:
- Windows `%TEMP%\claude\<hash or slug>\<session>\tasks`; macOS `/private/tmp/claude-<uid>/<cwd slug>/<session>/tasks`.
- Prefix `b` = shell (Bash and PowerShell, terminal text), `a` = agent (symlink to a transcript jsonl), `w` = workflow (JSON).
- A finished shell file ends with `\n\n[exited with code N]\n`, or `[killed]` only after `TaskStop`.
- **Foreground calls write `b*` files too** (seen: a foreground call created `tasks/bylwyjx0z.output` under the resumed folder), and folders keep old files for days.

**Which folders.** A folder is scanned only once it is mapped to a live agent key, and then only files newer than that key's session start. Mapping, in order, once per folder:
1. **Learned map.** Every §2.1 `start` teaches `tasks folder → key`.
2. **Process ancestry.** A file's holders (§4) lead to the claude process: on Windows the first `bash.exe` holder's native parent is `claude.exe` (MSYS breaks ppids only further down); on macOS the `/bin/zsh -c` root is a child of claude. Match that pid against `presence::AgentCard.pid` (`crates/presence/src/lib.rs:72`) with the start-time guard (`presence/src/os.rs:41-53`).
3. **Folder name equals a live key's session id.**

Unmapped folders are never opened. The folder name never names the agent by itself: a resumed session writes under its **original** session id (seen on this PC: key `698690f6-…`, runs under `e43a9506-…`). Scan every 5 s, only folders whose mtime moved.

**Foreground until proven background.** A scanned `b*` file is a **foreground** row (time and CPU, no stream, no tab, no chip) until one of these holds, and only then is it a background run:
- the session transcript (`<claude config dir>/projects/<cwd slug>/<session id>.jsonl`, read forward from a saved offset) has a tool_result whose `toolUseResult.backgroundTaskId` is the file id, or a task-notification row names it;
- it outlives its foreground call: its tool_result has landed in the transcript, or (no transcript found) the agent's roster status has left `shell`, and the file is still growing or held.

A §2.1 `start` skips the wait. "Never opened" above means never read or tailed; asking RM for a file's holders doesn't read it.

**End from the file.** At EOF with no growth for one scan and no holders, a trailer `[exited with code N]` / `[killed]` ends the run; a vanished file ends it with no code. Trust the trailer only once both hold, since a program can print the same text. This is also the only end for a subagent's `backgroundEndsWithFinalResponse` run, killed with no notification. Reconciling with the Stop hooks' `background_tasks` snapshot is not in v1.

**Kinds.** `b*` gets the full treatment. `a*` and `w*` get a Running row and a chip, no tab and no separate tree (their process is claude itself).

**On Core start** (runs outlive Irori: they are claude's children), every saved run not yet ended is re-checked: holders alive, or trailer present.

## 3. Core records

| Layer | Where | What |
|---|---|---|
| Live registry | new `crates/core/src/runs/` (§9 split), modelled on `crates/core/src/usage.rs`; `runs: crate::runs::Runs` on `Services` next to `usage` (`service.rs:303`) | `Mutex<BTreeMap<RunId, LiveRun>>`, saved as `runs.json` in the data dir with `crate::reset::write_private` |
| Mesh summary | new replicated kind `agent.run` (§7) | start and end only, background runs only |
| Window | `CoreEvent::Runs` + `runs:changed` (§6) | never in `local_events` |

`RunId = (agent key, claude task id)`. `LiveRun { key, task_id, kind: Bash|Agent|Workflow|Fg, cmd (scrubbed, ≤160 chars), started_ms, output_path, ended_ms?, exit: Option<i32>, stopped_by_person: bool, members: Vec<(pid, start)>, cpu_cores, mem_bytes, peak_cores, peak_mem, sampled_ms }`. Exit codes are `i32` everywhere (registry, row, `TermMsg`, `CardRun`). Foreground rows are kept only while running plus the last 20 per key.

**Not `local_events`:** append-only with no delete (`crates/store/src/db.rs:629`), never delivered to subscribers, and one row per foreground call would grow forever. **Not presence's `AgentCard`:** its JSON must match Go's, and it exists in five places. **No `Run`/`runs` names inside coord:** `board/runs.rs:17,28` already has `Run` and `RunTally`.

**Card join on read, not stored.** Runs are keyed by agent key only. `people_with` (`crates/window/src/cards.rs:1316`) loads `People.runs` by key, and `card_summary` (`:1984`) attaches a card's runs from its keys (`where_.who`, holders and co-holders, which catches a spawned agent before it claims), as `Window::term_folders` does (`crates/window/src/terms.rs:40-55`): on a board log the task first goes through `priv_task` (`crates/window/src/boards.rs:584` → `Core::private_task`, `crates/core/src/service/boards.rs:2810-2815`), so `agent:<cardId>` holders become the device's keys. **Never `board_key`**: it calls `card_for` (`service/boards.rs:1931-1962`, `:1969`), which **writes** an `agent.card` on first touch. `perm_card_of` (`spawn_waiting.rs:597-621`) only looks at the person's own log, so it is not used. When a key holds several cards, the run shows on its WORKING card, else on all.

**API plumbing** (copy `usage.rs`): `crates/api/src/runs.rs` with `PATH`, `post_route()`, 405 when not POST, `Some(c) if c.attach` else `v02::forbidden`, `body_too_large`; add `|| crate::runs::post_route(&r.path)` to the POST precheck (`server.rs:328-334`) and `.or_else(|| crate::runs::route(&h))` to `dispatch` (`server.rs:362-369`); `fn record_run(&self, key, ev) -> Option<Result<(), String>>` defaulting to `None` on `Backend` (`crates/api/src/backend.rs:96-118`, impl near `:217`); `Services::record_run` beside `record_usage_limits` (`service.rs:2850`). Times from `(self.s.now)()` so tests drive the clock. Stub the route in `mod/dev/fakecore/main.go` (mux at `:93-190`).

## 4. Process metering

New `crates/sys/src/runmeter/{mod,windows,macos}.rs` (core has no `windows-sys` or `libc`; sys is already a dependency of core, window, presence and api). Same shape as `meters`: `pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos"))` (`crates/sys/src/meters/mod.rs:58`); macOS code under `cfg(target_os = "macos")`, **not** `cfg(unix)` (that covers Android); Linux/Android/iOS get no-ops returning empty, so core's calls compile on mobile. Add `Win32_System_ProcessStatus` and `Win32_System_RestartManager` to `crates/sys/Cargo.toml:14-32` (both exist in the workspace's windows-sys 0.61.2).

```rust
pub struct RunKey { pub output: PathBuf, pub session_pid: u32 }
pub struct TreeSample { pub cores: Option<f32>, pub mem_bytes: u64, pub procs: u32, pub members: Vec<(u32, u64)> }
pub struct TreeSampler { /* per run: HashMap<(pid, start), cpu_ticks>, last Instant */ }
impl TreeSampler { pub fn sample(&mut self, runs: &[RunKey]) -> Vec<(PathBuf, TreeSample)> }
pub fn holders(output: &Path) -> Vec<(u32, u64)>
pub fn end_tree(members: &[(u32, u64)]) -> Result<(), String>
```

CPU is cores = Δ(user+kernel) / Δwall, so the first sample is `None` (the `meters::Sampler` pattern, `meters/mod.rs:236-273`). A Core thread `irori-runs` samples every 2 s while any run is live and sleeps otherwise. Not the window's meters thread (`crates/window/src/meters.rs:162`): the mesh summary needs Core data with no window.

### 4.1 Windows

**The pitfall:** MSYS fork/exec leaves `ParentProcessId` pointing at a dead intermediate. Probe: `claude.exe 38956 → bash.exe 6964 → bash.exe 64420 → sleep.exe 47404` whose ppid 52096 doesn't exist. A ToolHelp walk from the root never reaches the real work, and `taskkill /T` (today's `kill_pid`, `treeclean.rs:1366`) would leave `sleep.exe` running.

**Find members by the file.** Restart Manager on the `.output` path returns exactly the run's tree, because every member inherited stdout (probe: 6964, 64420, 47404; claude.exe does not hold it; 57 ms per call, no admin).
1. Per run: `RmStartSession`, `RmRegisterResources(path)`, `RmGetList` (loop on `ERROR_MORE_DATA`), `RmEndSession`. One session per file: several files in one session return the union.
2. Drop Irori's own pid (the tail reader holds the file) and the claude pid; drop entries whose `ProcessStartTime` is before the run started (a person's `tail -f` started earlier).
3. One ToolHelp snapshot per tick (template `crates/sys/src/proc.rs:118-141`) adds **native** descendants of each holder by ppid: `cargo → rustc → link.exe` use pipes and don't hold the file, but native spawns keep valid ppids. Reject a child created before its parent (reused ppid).
4. Per member: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`, `GetProcessTimes`, `GetProcessMemoryInfo` with `PROCESS_MEMORY_COUNTERS_EX`, **summing `PrivateUsage`** (summed `WorkingSetSize` double-counts shared pages).
5. With many runs, refresh RM every 3rd tick and use cached members + ToolHelp in between.

Missed: a process that redirects stdout elsewhere and isn't a native descendant (`cmd > log` inside an MSYS fork chain). The MSYS `/proc` pass (`treeclean::msys_holders`, `treeclean.rs:1476-1484`; `/proc/<p>/winpid`) is the fallback, not every tick (its note at `treeclean.rs:1480`: a per-process loop took 30 s on a busy machine).

### 4.2 macOS (untested here)

1. Root once: `proc_listchildpids(session_pid)`, then match each child's fd 1 to the output file with `proc_pidfdinfo(pid, 1, PROC_PIDFDVNODEPATHINFO, …)`. libc 0.2.186 lacks `PROC_PIDFDVNODEPATHINFO` and `vnode_fdinfowithpath`: define them by hand from `<sys/proc_info.h>` with a size assertion test, as `proc_macos.rs:11-60` does for `kinfo_proc`. Fallback `lsof -t <file>`.
2. Members: `proc_listpgrppids(root)` (the `zsh -c` root leads its own group), plus a ppid walk via `proc_listallpids` + `PROC_PIDTBSDINFO` for children that `setsid` away. Key by `pbi_start_tvsec/usec`.
3. Per pid: `proc_pid_rusage(RUSAGE_INFO_V2)`; CPU is `ri_user_time + ri_system_time` in **mach absolute units** on Apple Silicon, scaled by `mach_timebase_info`; memory sums `ri_phys_footprint`.

**Approximate by nature.** A rustc that starts and exits between ticks is never seen. A newly seen member counts its CPU since creation. The UI says "about".

## 5. The read-only tail tab and Stop

### 5.1 `irori-term` tail pane (no PTY)

New `crates/term/src/tail.rs` (desktop cfg like `pty.rs`); `stub.rs` mirrors `open_tail` returning `Err`, so core's call compiles on mobile. `replay.rs:26-40` already feeds `rsz_term::Alacritty` with no PTY.
- **Reader thread.** `File::open` (Rust's Windows share mode lets Claude keep appending and deleting). Seek to `len − 2 MB`, skip to the next `\n`. Poll at 100 ms; a shorter length means truncation, restart from 0. Turn bare `\n` into `\r\n` (no PTY means no ONLCR), with a last-byte-was-`\r` flag across chunks. At a stable EOF with the trailer, send `TermMsg::Exit{code: Some(n)}`; `[killed]` or `NotFound` sends `Exit{code: None}`.
- **Frame thread.** A copy of `frame_loop` (`suzuri:crates/term/src/pane.rs:959`), not a call: the `Engine` trait is public (`suzuri:engine.rs:54`) but `Shared` (`pane.rs:187`) and its wake/condvar are private. Without the input window: dirty + ack or 250 ms, 33 ms pacing, `sync_deadline`/`stop_sync`, `snapshot`. `Title/Bell/Clipboard` map as `dispatch` (`pane.rs:937`) does; `PtyWrite` is dropped. `set_grow_at_bottom` stays off (a ConPTY workaround).
- **`TermMsg::Exit` becomes `{ code: Option<i32> }`** (today `Option<u32>`, `crates/term/src/lib.rs:103`). The PTY path casts suzuri's `Out::Exit(Option<u32>)` (`suzuri:pane.rs:46`) at `pty.rs:304,347`; the frontend's `exit.code` type follows through `cargo xtask bindings` + `bun scripts/gen-models.ts`. "Stopped" does **not** ride `TermMsg`: an ended tree only yields `[exited with code N]` or a failed notification, and `Exit{None}` can't tell killed from deleted. The Core knows it ran `runs_stop`, so it sets `stopped` on the tail.
- **The engine outlives the run, which reworks `Terms`.** Today `remember()` (`pty.rs:398-417`) drops the pane on `Exit`, and everything after goes through `pane()` (`:282`), which errors "the shell has exited". `Entry.pane` becomes `enum Src { Pty(Arc<Pane>), Tail(Arc<Tail>) }` and every user branches on it: `in_scope`/`list` set `running` from it (`:217`), resize/scroll/redraw/`screen_text`/ack keep working on an exited tail, `write`/`paste` on a tail return `Err("read only")`, `closed()` (`:273`) and `close_all()` (`:239`) take `Vec<Src>` and stop tail threads, and `close()` never touches the run.
- `Terms::open_tail(TailSpec { scope: Card(id), path, label, run })`, `Terms::mark_stopped(id)`. `TermInfo` (`lib.rs:64`) gains `tail: Option<TailInfo { label, run, exit: Option<Option<i32>>, stopped: bool }>` so a remounted tab shows its end at once.

Spawning `irori tail <path>` in a PTY is rejected: release `irori.exe` is GUI-subsystem (`app/src/main.rs:3`) and may get no console under ConPTY, it costs a conhost per run, and the tab dies on exit.

### 5.2 Who opens the tab

The Core opens it: on a `b*` run's background `start` with a card on this device, `terms().open_tail(...)`. `term_list` returns it and `term_attach` etc. carry it, so no new frontend command. Frontend work, all new:
- `CardTerminals` (`terminal-section.tsx:52`) lists terms only on mount and hears `TermOpened` (`:63-92`). It gains a re-list on `board:changed` for its card (`onChange` with `Follows.cards`, `lib/small-lists.ts:13`) that adds tails **without** `setShown`. Tails never use `TermOpened` (its handler calls `setShown`).
- On mount, `setShown` picks the newest **non-tail** term, else nothing; the person opens a tail by clicking it. `terminal-panel.tsx:69` mounts the same component, so one fix covers both.
- `Shells` (`:259`) names a tail by its label with a run icon; its X ("Close tab") shows only after exit.
- `Terminal` (`terminal.tsx`) gets a `tail` prop: `termHost` `write`/`paste` are no-ops (**not** suzuri-view `readOnly`, which also blocks scrolling), no `view.focus()` on mount (`:89`), and the footer (today "The shell ended (exit N)", `:228-242`) reads `Exited with code N` or `Stopped`, with no Restart.

**Closing.** Nothing closes card-scope terminals today. A new Core hook on `CoreEvent::Board` (`crates/core/src/events.rs:69`) closes a card's exited tails when the card goes Done or closed. A running tail stays until its run ends.

### 5.3 Stop and Attach

- **Stop** is a window command `runs_stop(key, task_id)` → `Core::stop_run`: sets `stopped_by_person`, marks the tail stopped, then `sys::runmeter::end_tree`. `end_tree_holder` (`crates/core/src/service/storage.rs:217`) can't be reused: it refuses live trees and open cards. Members are re-resolved at Stop time and ended only if `(pid, start)` still matches. Windows: `TerminateProcess` per member. macOS: `kill(-pgid, SIGTERM)`, SIGKILL after 3 s. Keep `end_holder`'s guards: never Irori's own pid (`treeclean.rs:1345`), never the claude pid. Whatever end arrives afterwards (code, failed notification, vanished file) keeps "Stopped". The button is a `ConfirmButton` (`components/common/confirm-button.tsx:21`) gated on `AgentRef.here` (`crates/window/src/cards.rs:58`).
- **Attach** on a foreground row shows only when the session was spawned by Irori on this device **and** the row's card is that spawn's own card. `Window::agents_attach` opens on `sp.current()` (`crates/window/src/agents.rs:458-497`) while `attachAgent` dispatches `TermOpened` with the caller's `cardId` (`lib/agents.ts:577-583`), so on any other card the tab would land on the wrong card. Otherwise, no Attach button.

## 6. Tile chip, threshold and Busy now

**Two paths into the window**, never one:
- **Run facts** (cmd, start, exit, stopped, device) ride the card. `CardSummary.runs: Vec<CardRun>` (heavy and running only, `skip_serializing_if = "Vec::is_empty"`) and `CardDetail.runs` (all, including finished and foreground). `CardRun { who: AgentRef, run, kind, cmd, started, exit: Option<i32>, stopped, cpu_cores, mem_bytes, device }`, joined on read (§3). New `CoreEvent::Runs { keys }` and `Topic::Runs` (`crates/core/src/events.rs:34-125`), published on start, end, threshold crossing and remote rows; add `Topic::Runs` to the board pump's `feed_of` list (`crates/window/src/events.rs:260-262`) and an arm in `signals::fold_board` (`signals.rs:279`) that maps keys to held card ids. In the preview, mirror `runs` in `summary()` (`frontend/src/lib/cards.ts:1214-1288`).
- **Live load** every 2 s goes on its own `runs:changed` event, the `meters:changed` pattern (`crates/window/src/meters.rs:21`; frontend `lib/meters.ts:300-351`): `[{key, run, cores, memBytes, at, device}]`. Through `board:changed` it would rebuild rows, bump revs (#1136) and churn `liveCards`.

**Threshold** (on by default, tunable later, decisions.md #52): a run is heavy when its 10 s average is ≥ 1 core or its memory is ≥ 1 GB, held for 10 s. Leaving needs 30 s below half the threshold, so chips don't flicker.

**Frontend.** New `lib/runs.ts` (types, `subscribeRuns`, `useRunLoads`, `useCardLoad`; pure `heavyRuns`, `runChipText`, `busyLine`, `cardLoad`; a moving preview sample like `meters.ts` `startSample`), tested in `lib/runs.test.ts`. Reuse `fmtGB` (`meters.ts:72`), `runningSince` (`lib/agents.ts:873`) and `Tick` (`components/common/ago.tsx:17`). New `components/cards/run-chip.tsx` (`RunChips`), on its own 11px line in `CardTile` near `Left off:` (`card-grid.tsx:626`) and in `ProgressRow`'s status column (`progress-rows.tsx:185-197`). New `components/cards/busy-now.tsx`, rendered by Home only: `App.tsx` passes a `busyNow` node into its `BoardView` (`App.tsx:1256`; `board-view.tsx:161`, placed next to `{notice}` at `:455`). Styled like `IdleLine` (`App.tsx:1448`).

## 7. Other devices

- **Replicated row `agent.run`** (`AGENT_RUN = "agent.run" => Replicated, Refuses;` beside `AGENT_STATUS`, `crates/store/src/kinds.rs:69`). Not `task.*`: that prefix is in coord's `FOLLOW_KINDS` (`board/fold.rs:98`), `space_move`'s `COPIED_PREFIXES` and `TOLERATED_FAMILIES`. Add it to `SINCE_GO_REFUSED` (`crates/store/tests/it/golden.rs:136`, 41 → 42).
- **Payload** `{key, run, kind, op: start|end, cmd (scrubber.string, ≤120 chars), startedAt, endedAt?, exit?, stopped?, peakCores?, peakMemMb?, at}`. **Never the output path.** Two rows per background run; never foreground calls; never CPU on a timer (device-free.md §2, card #467's row flood).
- **Writer:** only the device running the run. The key is always the Irori key, never the folder's session id.
- **Reader:** a `RunsFold` in `runs/fold.rs` copying Roster's `start`/`follow_with`/`handle` (`crates/coord/src/roster.rs:200-265`), skipping own rows and dropping any row where `device_of(key) != ev.device_id`. (`written` in `Services::record`, `service.rs:5678-5682`, is a local, not a field, so it can't gate the writer.) It does **not** clear on `device.started`: runs outlive an Irori restart. Rebuild reads the whole log, so the fold keeps only open runs and runs ended in the last 24 h, dropping a start once its end is older. An offline device's open runs show as `on Mac · last seen …` via roster liveness. Publishes `CoreEvent::Runs`.
- **Later card:** live CPU/memory to other devices as an ephemeral `runs` mesh frame (latest-wins, ≥5 s, only while heavy), the card #663 `call` pattern (`crates/mesh/src/frames.rs:42`). Live output to other devices is also later.

## 8. Security and trust

- **Output is untrusted text.** The tail feeds bytes into the terminal engine only. OSC 52 writes need a click and links need Ctrl/Cmd+click, as in card terminals (inline-terminals.md §8). Run output never goes to an agent, a toast or a row.
- **Only the person stops a run.** `runs_stop` is a window command; no loopback route and no MCP tool ends a run. `/v1/runs` only records, and the caller's identity is its attach secret, never the body.
- **The Core never trusts the reported path as a kill target.** Stop ends only processes the Core resolved from the file and the session pid, with start-time matching, never Irori or claude.
- **Nothing new leaves the device except the summary.** The output path, contents, members and live CPU/memory stay local. `agent.run` carries a scrubbed, capped command, times, exit and peaks, to the person's own devices on `irori/mesh/1`, never a shared board log (`irori/board/1`).
- **Commands carry paths and tokens:** scrub with `self.scrubber.string()` before `runs.json`, any event and any row.

## 9. Build slices

`crates/core/src/runs/` is split so parallel slices don't collide: L2 owns `mod.rs` (`Runs`, `RunId`, `LiveRun`, `runs::start(&Services)`) and `registry.rs`, and lands `scan.rs`, `sampler.rs` and `fold.rs` as stubs, each with a no-op `pub(crate) fn start(...)` that `mod.rs::start` already calls, plus a stub `impl Core { pub fn stop_run }` in `sampler.rs` (any file in the crate may hold `impl Core`). L6 then owns `scan.rs`, L7 `sampler.rs`, L9 `fold.rs`, and none reopens `mod.rs` or `service.rs`. L5 and L7 both regenerate `frontend/src/gen/bindings.ts` and `models.ts`: the later merge reruns `cargo xtask bindings` + `bun scripts/gen-models.ts`. PC slices run targeted checks only (CLAUDE.md); "front" below means `cd frontend && bun test && bun run typecheck && bun run lint`. "mod checks" means `claude plugin validate mod`, `tsc -p mod` (copy `mod/.claude-plugin/types/` from the main checkout first, docs/mod.md:116-122), `claude plugin test mod` and `cd mod && bun test`; check.sh doesn't run them (`scripts/check.sh:85-131`).

| # | Slice | Files | Depends on | Mac | Done when |
|---|---|---|---|---|---|
| L1 | **Mod reports runs** | `mod/hooks/register.tsx`, `lib.ts`, `unit/lib.spec.ts`, `tests/irori.test.tsx` (stubs at `:110` return object results for Bash, PowerShell, Agent, Workflow; a `session.append` task-notification row; version asserts at `:195,203,848`), `mod/types/index.d.ts` (`runs` atom), `.claude-plugin/plugin.json`, `docs/mod.md:50`, `mod/dev/fakecore/main.go` | — | no | mod checks green; tests see `fg-start`, `fg`, `start` (Bash and PowerShell) and `end` from `message.task` posted to `RUNS_PATH`; detached reports flush on hello; version bumped. |
| L2 | **Core registry and `/v1/runs`** | `crates/api/src/runs.rs`, `server.rs`, `backend.rs`, `lib.rs`; `crates/core/src/runs/{mod,registry}.rs`, `service.rs`, `events.rs` (`CoreEvent::Runs`, `Topic::Runs`) | — | no | `cargo test -p irori-api -p irori-core` + clippy: posted start/end with an attach secret is read back by key; non-attach caller 403; `runs.json` survives restart. |
| L3 | **Windows metering and Stop** | `crates/sys/Cargo.toml`, `crates/sys/src/runmeter/{mod,windows}.rs`, `lib.rs` | — | no | `cargo test -p irori-sys` + clippy: a Git-bash `sh -c 'sleep 30'` writing to a file: `holders` has `sleep.exe`, `sample` reports members and memory, `end_tree` leaves none alive. |
| L4 | **macOS metering and Stop** | `crates/sys/src/runmeter/macos.rs` (hand-written proc_info + size test) | L3 | **yes** | `cargo test -p irori-sys` on the Mac: same test with a `zsh -c` child; a busy loop reads ≈1 core. |
| L5 | **Tail pane in irori-term** | `crates/term/src/tail.rs`, `pty.rs` (`Src`, `Exit` i32), `lib.rs` (`TermInfo.tail`, `TermMsg`), `stub.rs`, `tests/it/`; bindings, models | — | check.sh (Android) | `cargo test -p irori-term` + clippy: lines appear in `screen_text` with `\r\n`; trailer gives `Exit{Some(n)}`; deleted gives `Exit{None}`; scroll/redraw/screen_text work after exit; write is refused; `close_all` joins tail threads. |
| L6 | **Scan fallback and restart reconcile** | `crates/core/src/runs/scan.rs` | L2, L3 | no | `cargo test -p irori-core`: fixture folders show a mapped folder's `b*` stays foreground until a backgroundTaskId/notification or it outlives its call; unmapped and pre-start files are ignored; a resumed folder joins by ancestry; a run ended while down is ended on start. |
| L7 | **Window wiring** | `crates/core/src/runs/sampler.rs` (sampler thread, open tails, `stop_run`, close on Done), `crates/window/src/cards.rs` (`CardRun`, `People.runs`, join on read), `signals.rs`, `events.rs`, new `crates/window/src/runs.rs` (`runs:changed`, `runs_stop`), `commands.rs`; bindings, models | L2, L3, L5 | no | `cargo test -p irori-core -p irori-window` + clippy: a held card on a private and a shared board gets the run with no `agent.card` written; Stop sets `stopped` and the tail reads Stopped. |
| L8 | **Frontend** | `lib/runs.ts`(+test), `components/cards/run-chip.tsx`, `busy-now.tsx`, `runs-section.tsx`, `card-grid.tsx`, `progress-rows.tsx`, `board-view.tsx`, `App.tsx`, `card-detail.tsx`, `components/term/terminal-section.tsx`, `terminal.tsx`, `lib/term.ts`, `lib/host.ts` (`RunsService`), `lib/cards.ts` `summary()`, `lib/sample-cards.ts` (`sample-card-job`, a heavy run on `sample-card-4`, one on wren's device), `lib/preview.ts` (`?runs=`), `PREVIEW.md`, `scripts/preview-shots.sh`, `scripts/phone-check.ts` | none to start (hand-written types in `lib/runs.ts`); switch to generated types after L7 | no | front green; preview shows chips, Home-only Busy now order, Running section, a tail tab with Stop/Exited/Stopped that never steals focus; `check:phone home card-job` passes at 390 px. |
| L9 | **Mesh summary** | `crates/store/src/kinds.rs`, `tests/it/golden.rs`, `crates/core/src/runs/fold.rs` (writer + `RunsFold`) | L2 | **yes** | `cargo test -p irori-store -p irori-coord -p irori-core`: fold drops wrong-device and old ended rows; on the rig a Mac run shows on the PC card as `cmd · N min · on Mac` and ends with its code. |
| L10 | **End-to-end on both machines** | none (fixes go back to their slice) | L1-L9 merged on one branch | **yes** | Per machine: debug scratch build, `IRORI_HOME` copy, `CLAUDE_CONFIG_DIR` empty (so `--plugin-dir <worktree>/mod` is required, docs/mod.md:37), `IRORI_DEV_PORT_OFFSET=N`, `iroriPath` → the scratch irori (docs/mod.md:136); a session holding a card in that home runs a background `cargo test`, a PowerShell background run (PC), a foreground call and an old-mod session; screenshots of chip, Busy now, tab, Stop→Stopped, Running rows; then `sh scripts/check.sh` on the Mac on the merged branch; live `irori.log` gained no line. |

**Parallel waves.** Wave 1: L1, L2, L3, L5, L8 (hand-written types). Wave 2: L4, L6, L7, L9. Then L8 switches to generated types, then L10.

## 10. Open questions

1. **MSYS `/proc` fallback (§4.1):** keep it at all, or accept missing a redirected non-native child? L3 decides from what a real `cargo test` tree shows.
2. **Transcript path for a resumed session (§2.2):** unverified whether it is filed under the original or the resumed session id. L6 checks on a real resumed session; if neither is found, the roster-status rule decides.
3. **macOS `proc_info` layout (§4.2):** untested until L4 runs on the Mac; `lsof -t` is the fallback if the hand-written structs fail.
