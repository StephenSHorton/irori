# Agents see the device's free room, and share heavy jobs: plan

Card #1018, from the person on #1017. Agents are not the problem; what they run is: "instead of running something that would dump 100GB on the computer when there's no space left or running a program that's gonna take 20GB of memory when there's only 11 left, they stop and wait prudently or ask me". And: "maybe they even coordinate running stuff with other agents".

Out of scope, by the person's word: holding agent starts, and a max-agents setting. (`spawn.cap` and the spawn queue already exist and don't change.)

Every claim below was checked against the code on 2026-11-08 by reviewers told to break it. Nothing has been built or run. Line numbers are approximate: go by the names.

## 1. What ships

1. **A tool, `device_free`**, that reads this device now: free disk for a folder, available memory, CPU busy, and who holds the heavy and script slots here.
2. **A rule in the brief** that names the tool, says what to do with the numbers, and says a timeout, out-of-memory or disk-full failure means check, not retry.
3. **Heavy slots**: two per device, taken through the `lease` tool.

No setting and nothing to turn on (decision 52).

## 2. Decisions

**A tool, not numbers in the brief.** A brief is read once and an agent lives for hours, so its numbers would be wrong when they matter. The brief carries the rule and the tool's name, and the tool does the reading. There is no snapshot in the brief.

**Which disk.** The volume holding `path`; with no `path`, the caller's working folder (`cwd_of`, which is where the agent's builds land); with no known folder, Irori's data dir. A relative `path` joins to the caller's folder, never the Core's. A folder that doesn't exist yet walks up to its nearest existing parent.

**Margins.** The tool prints what can be used after keeping some free, so agents compare one number with their estimate. Both saturate at 0.

| | Keep free | Why |
|---|---|---|
| Disk | 5% of the volume, at least 10 GB, at most 50 GB | Room for logs, the databases and other agents' small writes |
| Memory | 10% of total, at least 2 GB, at most 16 GB | The system and the agents themselves |

These are judgment calls and can move. "Available" memory is total minus used as `sys::meters` already reads it: on the Mac it leaves out file cache, and on the PC `ullAvailPhys` counts standby as available. CPU is a 1-second sample, noisy, so it is printed as a figure with no margin.

**Heavy slots are advisory and fixed at two per device.** The name is `heavy/<first 8 hex of the device id>`, the slots `heavy/<id>/slot-1` and `slot-2`.

- *Why the id, not the name:* device names can be renamed, can be "this device", aren't unique between a person's devices, and may not pass the resource pattern. Hostnames aren't unique either, and two devices sharing a family would block each other, for as long as one is offline. The id is stable and passes the pattern, and scripts can read it (`deviceId` in `irori.json`). `device_free` prints the exact call, so agents never build the name.
- *Why fixed, and not sized to the free numbers:* a slot can't know how big a job is, and adding up declared sizes double-counts a job whose use has already landed in the free figure. The person asked for awareness and coordination, not accounting. The numbers answer "does this fit", the slots answer "who else is doing this now".
- *Why two:* the same as `check.sh`'s default. `CHECK_SLOTS` shows the shape works.
- *Not a limit on agents (decisions 23, 51):* nothing stops an agent that doesn't take one, and the person's say-so overrides it. If two turns out wrong, a setting can tune it then.

`check.sh` keeps its own `check/<host>` slots and is not wired to heavy slots. A run that queues on its own slots needs no heavy slot, so up to two checks plus two heavy jobs can run at once.

**The `lease` tool takes the slot; there is no new parameter.** Any resource that is exactly `heavy/<id>` goes to `Coord::acquire_slot` with `HEAVY_SLOTS`, before the shared-board mirror. A caller-chosen count would let an agent pass `slots=16` and sit beside two holders; a plain lease on the base name would be an unrelated lease that occupies no slot. The handler refuses `steal`, refuses a direct `heavy/<id>/slot-N`, and refuses another device's id, saying what the caller's own name is. Leasing again renews the same slot.

**When every slot is taken, the agent retries.** On the refusal it messages the holder if that helps, says on its card what it waits for, and leases again later. Retrying takes the first free slot atomically.

`board_block` is not the default, for reasons in the code: an unblock reaches a Claude agent only at its next turn, so an idle one is never woken; a family wait closes only when every slot is free at once, and can starve; past the idle limit the agent is ended, and a fresh one starts in a new tree. For a wait longer than that, the agent commits and pushes, then raises `board_block on="lease heavy/<id>/slot-N"`, naming one slot held now (the tool prints the soonest to free). A shared-board card can't use the lease form and uses `on="external heavy/<id>/slot-1..2"`.

**What agents can see.** `device_free` lists this device's holders: agents by name and held card; `check.sh` and other scripts as `script <run> (no card)`, because a run has no agent or card. The card comes from the board, and the capture "doing" line is optional because it is empty with capture off. Holders are filtered by the holder's device id and the resource prefix, so the Rust side never needs `check.sh`'s hostname slug. Output goes through `one_line` and starts with `untrusted()`, since names and card titles are other agents' words.

**Across devices.** `leases` already shows the person's other devices' slots, because private leases replicate. An offline device's slot can show past its expiry until that device returns. Live numbers aren't published across devices: `publish_host` writes a replicated row each time its contents change, and numbers change every read.

**Long jobs.** The default lease is 30 minutes and expiry is silent, so the tool prints the call with `ttl_minutes` for the agent to set to its estimate (at most 240), and says to lease again to renew. A foreground job can't renew, so the agent sizes the lease up front. An agent stops or finishes its heavy job before it hands off: the gone sweep frees the slot while the job would go on running. Scripts need no new code: `POST /v1/slots` already takes `heavy/<id>` with its renew loop.

## 3. Who hears the rule

| Agent | How |
|---|---|
| Started by Irori (card, ask, thread, shared-board card, helper, restart after a crash or usage limit) | The brief |
| Terminal session, attached with the new mod | One line once per session, naming the tool (`sideReminder`'s path) |
| Revived session from before this ships; already running at upgrade | Not reached |
| Grok member | The tool's description only |

`crates/tools/src/specs.rs`'s `INSTRUCTIONS` don't reach Claude agents, so they are not the place for it. The tool is deferred in Claude Code until loaded, so its description alone can't teach an agent that doesn't know to look: only the name shows.

`readOnly: true` on the spec does not stop a terminal prompt: the mod drops it at registration. The tool behaves like every other `mcp__irori` tool. Spawned agents are allowed by `spawn_permissions.rs:140`.

## 4. The brief's words

Beside `BUILD_OUTPUT` in `crates/core/src/service/spawn.rs`, pushed at both `b.push_str(BUILD_OUTPUT)` sites:

> This computer is shared, and what you run can fill it. Before anything big (a full build or test run, a large download, generating data, a program that needs a lot of memory), call device_free and compare what it says you can use with a rough estimate of the need. If it won't fit with room to spare, don't start it and hope: wait and check again, or ask the person on your card or thread. For a heavy job, take this device's heavy slot first, as device_free shows; if they're taken, it says who holds them, so message them or check again later. Treat a timeout, out-of-memory or disk-full failure as a reason to check device_free, not to retry in a loop.

It goes to agents in every repo, so it names no repo's scripts. Tests limit the wording: a test in `spawn.rs` (`assert!(!b.text.contains("side"))`) fails if the brief holds the substring `side` anywhere (consider, inside, beside), and `ask_spawn.rs:172` fails on `board_status` or `Left off`.

## 5. Doing cards

Order: A, then B; C on its own; D after A and C; E after D. The Windows half is its own card because only the PC compiles it. D works on the PC with CPU and memory as "not available" until B lands.

### A. `sys::meters::quick(path)`: API, Mac, and disk on unix (needs:mac)

- `crates/sys/src/meters/mod.rs`: `pub fn quick(path: &Path) -> Quick` with `disk`, `mem` and `cpu` each an `Option`; extend the stub at `:35` so Linux, Android and iOS compile with CPU and memory `None`; don't gate disk on `SUPPORTED`. Update the module's table.
- `macos.rs`: move the tick read (`:54-90`) into a free function under `Sampler::cpu` and `quick`; make `mem()` `pub(super)`. Two tick reads 1 second apart.
- Disk: `statfs` on the Mac (`u64` block counts; `statvfs`'s are 32-bit), `statvfs` (`f_bavail * f_frsize`) on other unix. A path with an interior NUL returns an error, not a panic; walk up only on not-found.
- Memory reads `None` when total or used is 0: today a failed read on the Mac is "used 0", which would say all memory is free.
- `windows.rs`: a `None`-returning stub, filled in by B, and don't call `mem()` from it yet.
- `check.sh` runs clippy with `-D warnings` and the casts differ by target, so expect to need `#[allow(clippy::unnecessary_cast)]`.

Done when: a test reads this machine (as `samples_this_machine`, `mod.rs:173`) and a missing path resolves to its parent; `sh scripts/check.sh` passes.

### B. `quick` on the PC (needs:pc, after A)

- `windows.rs`: CPU from two `GetSystemTimes` reads, `busy = (kernel + user - idle) / (kernel + user)` (kernel time includes idle); cores from `GetActiveProcessorCount` as at `:62`. It can differ a little from the top bar's `% Processor Utility`; that's fine here.
- Disk with `GetDiskFreeSpaceExW`, using the caller-available and caller-total values; give it a folder, the parent when the path is a file. `Win32_Storage_FileSystem` and `Win32_System_Threading` are already features of `crates/sys`.
- Memory reads `None` on a 0 total (`:118-120`).

Done when: the same test passes on the PC and `sh scripts/check.sh` passes there.

### C. Heavy slots through `lease` and `release`

- `crates/tools/src/lib.rs`: a defaulted `Backend::device_id()`, implemented in `impl tools::Backend for ToolsView` (`crates/core/src/service.rs`). `pub const HEAVY_SLOTS: usize = 2` and a function that names this device's family.
- `lease` in `crates/tools/src/board.rs`: route as §2 says, before `mirror_acquire`. On `Held`, name the holders and slots, using `c.leases()` filtered by `<base>/slot-` as `crates/api/src/slots.rs:109-117` does.
- `release` in the same file: call `release` first; on not-found call `release_slot(base)`; on `Ok(false)` say "you hold no slot of X", never "Released".
- `specs.json`: `lease` and `release` descriptions say how heavy slots work.
- `frontend/src/lib/wait-name.ts` (`:36`, `:55`) and its test: a `heavy/` branch beside `check/`, reading "a free heavy slot".
- Goldens: `IRORI_BLESS=1` as `docs/architecture.md` says, then rerun without it.

Done when: tests in `crates/tools/tests/it/` show the base name takes a slot under an agent key, a repeat renews the same slot, the third taker is refused with names, `steal` and another device's id are refused, and release works by base and by full name; `sh scripts/check.sh` passes.

### D. The `device_free` tool (after A and C)

- `specs.json`: one entry, `path` optional and described, `additionalProperties: false`, clients `claude` and `grok`, `readOnly: true`; description at most 500 bytes (`ported.rs:542-578`), carrying the rule in short.
- `crates/tools/src/lib.rs`: handler in the `handlers()` table (its array length goes up by one); a defaulted `Backend::device_free(path) -> Option<DeviceFree>` with a tools-side struct, so `crates/tools` gets no `irori-sys` dependency and its tests don't sleep. `ToolsView` implements it from `sys::meters::quick` and `Services.data_dir`.
- `capturetools.rs:34`: `cwd_of` becomes `pub(crate)`. Leave the tool out of `CALLER_READS` (`lib.rs`), so a caller with no session still gets the data-dir default.
- Handler: read the numbers outside any lock. Answer in plain lines, for example:

  ```
  On MacBook, now:
  Disk (/Users/me/projects): 1.9 TB free of 3.6 TB. Up to 1.8 TB can be used (keeping 50 GB).
  Memory: 92 GB available of 128 GB. Up to 79 GB can be used (keeping 13 GB).
  CPU: 71% busy over 1 s, 18 cores.
  Heavy slots (2): slot-1 held by kitsune on #1018, until 11:40; slot-2 free.
  Take one: lease resource=heavy/3dc30c94 ttl_minutes=<your estimate, at most 240>. Lease again to renew; release when done.
  Script slots: script check-card-999-1-4411 (no card).
  These numbers don't count what the jobs holding slots will still use.
  ```

  What can't be read says "not available here". When both slots are held, name the one with the soonest expiry and how to wait on it.
- Tests that have to change: `crates/tools/tests/it/ported.rs` has two lists, `want` and `read_only`, and the tool goes in both; `crates/mcp/tests/wire.rs` hard-codes the tool count (`registered().len()`) and bless doesn't change it, so raise it by hand; bless the goldens (`tools/specs.json`, `api/go.json`, `mcp/wire.json`).
- `docs/mod.md`: add it to the tool list.

Done when: tool tests with a fake `Backend` cover all numbers, "not available" and holders of each kind; in a scratch app (never the live one: CLAUDE.md) a session calls `device_free` and the disk figure matches `df`; `sh scripts/check.sh` passes.

### E. The rule reaches agents (after D)

- The const from §4, `pub`, pushed at both sites with the same `\n\n`.
- Tests: assert it in a card brief (`crates/core/tests/it/spawn_trees.rs:180`) and in a thread brief (`ask_spawn.rs:164-172`), so losing either push fails.
- Terminal sessions: `deviceReminder()` in `mod/hooks/lib.ts` beside `sideReminder`, added to `texts` in `withHeld` (`register.tsx:743`), once a session, the flag set only after the line was really returned (`withHeld` can return early before attach, `:739`) and reset at `session.start` (`:767`). A case in `mod/unit/lib.spec.ts`.
- `docs/decisions.md`: a row for this card, saying heavy slots are advisory and not a limit on agents.

Done when: both brief tests pass, the mod's tests pass, and a scratch app's spawned agent's brief holds the line; `sh scripts/check.sh` passes.

## 6. Left out, and why

- **Adding up declared job sizes in coord**: see §2. Two agents can still both read the same free disk and both start; the slots make that rarer and the tool says so. If it bites, a job's size can go on the lease.
- **`wait` on a lease, or waking an agent when its lease blocker closes**: the retry is enough to start. The wake is a small change where the sweep closes lease waits in `crates/coord/src/board/comments.rs`, but `board_test.rs:1603` pins today's behaviour, so it is a decision of its own.
- **Numbers from other devices**: see §2.
- **Holding agent starts**: the person ruled it out.
