# Tsukumo router: port spec for Irori v0.2 "Talk"

Source: `C:\Users\4step\projects\tsukumo` (`src/router/core.ts`, `src/router.ts`, `src/lib/{route,take,sweep,ledger,refs,prmeta,hook,types,filelock,channel}.ts`, `src/attach.ts`, `src/watch.ts`, `src/call.ts`, `claude-mod/hooks/{register.tsx,wake.ts}`, `claude-mod/types/index.d.ts`) plus their `*.test.ts`. Test names are cited in backticks. `**Unsure:**` marks anything not settled by code or tests. A copy is at `C:\Users\4step\AppData\Local\Temp\claude\C--Users-4step\22a67bd5-0acc-4fd9-bcc5-ac620c56e4e2\scratchpad\tsukumo-router-spec.md`.

Tags: **[agnostic]** carries over as logic. **[discord]** exists only because of Discord, the gateway or the file-based multi-process design. **[adapt]** the idea carries but its key or scope changes.

---

## 0. Big picture

One router process (`src/router.ts`) holds the single Discord gateway connection. Each Claude Code session's mod spawns `src/attach.ts`, which streams that session's events from the router over a unix socket and forwards the mod's control calls. `RouterCore` (`core.ts`) contains every decision and has no Discord or socket code. Its dependencies are injected (`CoreDeps`):

```
store: HookStore          // claims (.hooks.json): who watches what, role, threads, heartbeat
ledger: OwnerLookup       // owners(key) -> LedgerEntry[] best first
dirs: Dirs                // eight flag/lock directories (section 3)
now(), botId(), channelName(id), parentOf(id), repoOf(id), fetchMessage(ch, id)
aliasStore?               // names chats post under, persisted across router restarts
logDirect?                // direct.jsonl writer
enrich?                   // refine a route's reason (Cursor-agent PRs)
```

`src/watch.ts` is the older one-process-per-session watcher. It runs the same `routeMention`/`planSweep`/`planReminders` over the same files. Irori does not need it. **[discord]**

---

## 1. Constants

| Constant | Value | Where | Meaning | Tag |
|---|---|---|---|---|
| `UNENGAGED_LIMIT_MS` | 10 min | sweep.ts | New work the taker hasn't engaged with moves on after this | agnostic |
| `QUIET_LIMIT_MS` | 60 min | sweep.ts | Engaged or owned work with no reply and no deferral: reminder only | agnostic |
| `GONE_GRACE_MS` | 3 min | sweep.ts | Taker gone this long counts as really gone, so its work is rerouted | agnostic |
| `PICKUP_GRACE_MS` | 90 s | sweep.ts | A mention routed to a session not attached here that nobody took in this time comes back to the desk | adapt |
| `SWEEP_LOOKBACK_MS` | 6 h | sweep.ts | Older than this (from max(atMs, deferral)) is history, not work. Also the catch-up cutoff | agnostic |
| `MAX_DEFER_MS` | 12 h | sweep.ts | Deferral clamp: `min(untilMs, now+12h)` | agnostic |
| `REATTACH_GRACE_MS` | 2 min | core.ts | Detached session keeps claim and queue (reload/restart) | agnostic |
| `QUEUE_LIMIT` | 50 | core.ts | Events queued per detached session; oldest dropped | agnostic |
| `ROUTED_MEMORY` | 2000 | core.ts | FIFO set of routed message ids (dedupe live vs catch-up) | agnostic |
| `DIRECT_LIMIT` | 8 | core.ts | Direct messages per unordered pair (both directions) per window | agnostic |
| `DIRECT_WINDOW_MS` | 10 min | core.ts | Sliding window for DIRECT_LIMIT | agnostic |
| `MAX_DIRECT_CHARS` | 8000 | core.ts | Direct message content cap (after trim) | agnostic |
| deferral note cap | 500 chars | take.ts `defer` | `note.slice(0,500)` | agnostic |
| alias rules | ≤60 chars, ≤10 per session (LRU), 200 sessions persisted | core.ts `addAlias`, router.ts | | agnostic |
| `HEARTBEAT_STALE_MS` | 90 s | hook.ts | Claim row with heartbeat older than this is not live (strict `<` is fresh) | discord (file design) |
| heartbeat / tick | 30 s | router.ts `every(30_000, tick)`, watch.ts `HEARTBEAT_MS` | Heartbeat claims, expire graces | discord/adapt |
| claims refresh | 5 s | router.ts, watch.ts `CLAIMS_POLL_MS` | Re-emit `claims` if changed | discord (polling a file) |
| sweep | 60 s | router.ts (tick then sweep), watch.ts `SWEEP_MS` | | agnostic |
| ledger pr-link sync | 2 min (and once at start) | router.ts, watch.ts `LEDGER_SYNC_MS` | | adapt |
| router-state persist | 10 s (only if dirty) | router.ts | watermarks + aliases | discord |
| router linger | 30 min | router.ts `LINGER_MS` | Exit after no sessions this long | discord |
| `MIN_LOGINS_LEFT` | 25 | router.ts | Don't connect below this many gateway logins left | discord |
| `HISTORY_LIMIT` | 25 | router.ts | Messages sent as `history` per channel on a fresh attach | discord |
| `CATCH_UP_WINDOW_MS` | 6 h | router.ts | REST catch-up floor; ≤10 pages × 100 per channel and per thread | discord |
| prune flag dirs | 2 days (mtime), at startup | router.ts, watch.ts | "well past 12 h deferral + 6 h lookback" | adapt |
| direct.jsonl | truncate to 0 if > 5,000,000 bytes, before append | router.ts | | agnostic |
| router.log | truncate if > 2,000,000 bytes on router start | attach.ts | | discord |
| socket keepalive | `"\n"` every 5 s on the attach stream | router.ts | | adapt |
| attach backoff | sleep `min(16 s, 1 s·2^failures)`; fatal when failures > 5 | attach.ts | A stream that reached `ready` resets failures to 0 | adapt |
| attach router-start wait | 60 × 250 ms | attach.ts | | discord |
| attach orphan check | 30 s, `process.ppid === 1` → exit | attach.ts | Unix-only signal | adapt |
| filelock | wait 2 s, stale lock 5 s, poll 10 ms | filelock.ts | `<path>.lock` via O_EXCL | discord (file design) |
| ledger `KEEP_MS` | 45 days | ledger.ts | Entries dropped on every merge | agnostic |
| prmeta `HIT_TTL_MS` / `MISS_TTL_MS` | 24 h / 10 min | prmeta.ts | | agnostic (GitHub) |
| prmeta `gh` timeout | 10 s kill | prmeta.ts | | agnostic (GitHub) |
| enrich race | 1.5 s, first 2 refs only | router.ts | Never hold routing on GitHub | agnostic (GitHub) |
| mod seen retry | 5 attempts, 3 s apart | register.tsx `markStarted` | | agnostic |
| ROUTING_VERSION | 2 | route.ts | Ledger-routing protocol version | discord (compat) |

---

## 2. Data shapes

### 2.1 Message (`GatewayMessage`, types.ts) [discord, adapt]
```
id, content, authorId, authorTag, authorUsername, authorBot,
channelId, guildId|null, threadParentId|null,
mentionsBot,            // bot @mentioned or replied to
replyToId|null, createdAt (ISO), attachments[{id,filename,url,contentType|null,size}]
```
Routing only uses `RoutableMention = {channelId, guildId|null, threadParentId?, content, replyToId?}`.

### 2.2 Claim row (`HookScope`, hook.ts; `.hooks.json` = `{version:1, hooks:{[conversationId]: rowWithoutId}}`) [adapt]
```
conversationId, guildId, guildName, guilds?[{id,name}],
channelIds: string[] | null    // null = whole server
channelNames[], boundAt (ISO), heartbeatAt? (ISO),
role?: "owner"|"listener"      // absent = owner
threadIds?: string[], label?, routing?: number
```
- `isOwner(h) = h.role !== "listener"`.
- `describeHook(h) = label.trim() || conversationId[:8] + "…"`.
- `hookAllowsChannel(h, {channelId, guildId, threadParentId})`: false if no guildId or the guild isn't among the row's guilds. True if channelIds is null, contains channelId, or contains threadParentId. (`hookAllowsChannel` tests; `one chat can be the front desk for channels in several servers`: a matching channel id with the wrong guild is rejected.)
- Liveness (`hookIsLive`): the current conversation is always live. Otherwise live if the heartbeat is fresh (<90 s) or a Grok session dir exists. `save` prunes non-live rows. (`prunes hooks whose conversations are gone`, `a stale heartbeat is pruned`, `a fresh heartbeat keeps a non-Grok claim live`.)
- Conflicts on `save` → 409:
  - A thread id in both this row's and a live other row's `threadIds` (`a thread has one owner at a time`).
  - Both rows are owners and their channels conflict. Two channel lists conflict if they share any id. A whole-server row (`channelIds: null`) conflicts with any row in an overlapping guild (`a row can own channels in several servers; a whole-server row in one of them conflicts`, `many conversations can listen to an owned channel; a second owner cannot`, `round-trip save/load and exclusive claim`).
  - Listeners never conflict on channels.
- `update(id, fn)` re-runs the conflict checks: a listener can't become owner while an owner is live (`a listener can take ownership once the owner is gone`). `heartbeat(id)` returns false if the row is gone. `clear(id)` deletes it.

### 2.3 Take row (`taken/<messageId>`, `pending/<messageId>`) [agnostic]
```json
{"by":"<sessionId>","channelId":"<watched channel = thread parent>|null",
 "messageChannelId":"<where the message lives>|null","atMs":<epoch ms>,"kind":"owned"|"new"}
```
Parse fallbacks: missing `messageChannelId` → `channelId`; missing `atMs` → file mtime; missing `kind` → `"new"`. A non-JSON file means the whole text is `by` (legacy), with channel null, atMs = mtime and kind new.
- `kind: "owned"`: routed to the chat whose work it is (reply to its post, its thread, its PR), or a person handed it over (`take` control). It may take its time.
- `kind: "new"`: front-desk work nobody owns, which moves on if the taker doesn't engage.

### 2.4 Deferral row (`deferred/<messageId>`) [agnostic]
`{"by":"<sessionId>","untilMs":<ms>,"note":"≤500 chars"}`. `deferredUntil(id, by)` returns null if `row.by` is set and differs from `by`, so a deferral by an earlier holder doesn't count.

### 2.5 Flag files (`done`, `held`, `rerouted`, `flagged`, `seen`) [agnostic]
File content is `by` or empty. `isMarked(id)` means the file exists. `isMarkedFor(id, by)` means the content is empty or equals `by`, so an empty flag counts for anyone.

### 2.6 Ledger (`.ledger.json`) [agnostic, keys adapt]
```
{version:1, entries: LedgerEntry[], scanned: {[transcriptPath]: byteOffset}}
LedgerEntry = {key, sessionId, role: "author"|"poster"|"reviewer"|"touched", at (ISO), label?, source: "mod"|"pr-link"}
```
Keys: `msg:<discord id>`, `pr:<owner/repo lowercased>#<n>`, `branch:<owner/repo>:<name>`. **Unsure:** nothing produces `branch:` keys and routing never reads them. The `record` op accepts them. Treat them as unimplemented.

### 2.7 Router events (NDJSON lines on the attach stream) [mostly agnostic]
```
ready    {bot, botId, guildId, guildName, guilds[{id,name}], channels[{id,name}]}          [discord]
claims   {role:"owner"|"listener", threadIds[], owner: string|null, listeners: number}     [adapt]
mention  {channelName, message: GatewayMessage, route:"mine"|"other"|"none", routedTo?, reason?}
overdue  {messageId, channelId, channelName, minutes}
moved    {messageId, channelName, to}           // a mention this chat held went to `to`
handoff  {messageId, channelId, channelName, minutes, to}   // this desk sat on it; it and the role went to `to`
message  {channelName, message}                 // channel feed                                [discord]
history  {channelId, channelName, messages[]}   // backfill on fresh attach                    [discord]
direct   {id, from, fromSession, content, replyToId?, about?{channelId,messageId}, sentAt}
error    {message, fatal?}                      // fatal: router dropped this chat; attach exits 1
```
`claims` details:
- `owner` is null when this chat is an owner. Otherwise it is `describeHook` of the first live owner among rows sharing any channel.
- `listeners` = live non-owner rows sharing a channel, +1 if this chat is a listener.
- Only sent when its JSON text differs from the last one sent (`lastClaims`). Setting `lastClaims=""` forces a re-send.

### 2.8 direct.jsonl line [agnostic]
The `direct` event plus `to: <target sessionId>`: `{type:"direct", id, from, fromSession, content, replyToId?, about?, sentAt, to}`. Append-only. It is the only record of direct messages.

### 2.9 router-state.json [discord]
`{watermarks: {[channelId]: lastSeenMessageId}, aliases: {[sessionId]: string[]}}`. Only the newest 200 sessions' aliases are kept (insertion order).

### 2.10 Mod-side mention state (`claude-mod/types/index.d.ts` `Mention`) [agnostic]
`status: 'delivered'|'held'|'dismissed'|'elsewhere'|'skipped'`, plus:
- `unowned?`: held because nobody owns the place, not because of block/allow.
- `awaitingStart?`: delivered, but the wake turn hasn't started yet.
- `turnId?`: the turn the wake started.
- `routedTo?`, `reason?`.

---

## 3. The take-lock and the eight directories [agnostic logic, discord storage]

Every directory is `<dir>/<messageId>`, guarded by `SNOWFLAKE = /^\d{15,22}$/`. A non-snowflake id is rejected or ignored (`rejects a non-snowflake id instead of writing a path from it`: `takeMessage` returns `{ok:false, takenBy:"nobody (not a message id)"}`).

| Dir | Content | Written by | Cleared by | Per holder? |
|---|---|---|---|---|
| `taken` | Take row | `takeMessage` (O_EXCL create) at route time and by `take` control. `retake` (overwrite) in `moveTo` | `forget` requeue (only if held by the forgotten session); prune | n/a |
| `pending` | Take row, `by` = intended session | `routeAndDeliver` when the route target isn't attached here, the desk is attached here, and nobody has taken it | `unmark` when the target takes it, on sweep `clear`, in `moveTo`, or when the desk's take fails | n/a |
| `done` | empty | A bot message with `replyToId` (live or in catch-up); `skip` control | prune | any |
| `held` | empty | `hold` control (mod: blocked or not-allowlisted sender, or a person dismissed it) | `take` control | any |
| `rerouted` | empty | `moveTo` | prune | any |
| `flagged` | `by` | `remind` | `defer` (so a new deferral gets its own reminder); `moveTo` | yes |
| `seen` | `by` | `seen` control, only if the caller is the current holder | `moveTo` | yes |
| `deferred` | Deferral row | `defer` control, only by the holder | `moveTo` | yes (by `row.by`) |

`takeMessage(dir, id, by, channelId, now, messageChannelId, kind)`:
- Create exclusively and write the row → `{ok:true}`.
- On EEXIST, read the existing row: if `taken.by === by` → `{ok:true}` (idempotent), else `{ok:false, takenBy}` (`the first conversation wins; the second learns who has it`).

`pruneTaken(dir, maxAge)` deletes any file whose mtime is older than maxAge (`old locks are pruned`).

---

## 4. Routing: `routeMention(hooks, msg, ledger, {defaultRepo})` → `{to: HookScope|null, reason, owned}` [agnostic order, discord gates]

Inputs: the live hooks, a message (thread parent filled from the cache if missing), the ledger, and `defaultRepo` = repo of the watched channel (from the channel topic).

Definitions:
- `frontDesk(hooks, msg)` = among hooks with `isOwner && hookAllowsChannel(msg)`, the one with the earliest `boundAt` (string compare of ISO), else null (`if two rows both claim the front desk, the earliest wins everywhere`).
- `here(sessionId)` = a hook with that conversationId that allows this channel and has `routing ≥ 2`.
- `ledgerRouting = !desk || desk.routing ≥ 2`. An old or Grok desk disables steps 1 and 3 (`with an older (or Grok) front desk, everyone falls back to the old rules so nobody double-answers`).
- `toOwner(entry, what)`:
  - If `here(entry.sessionId)` exists → `{to: it, reason: what, owned: true}`.
  - Else → `toDesk("<what>, but <entry.label ?? id[:8]+'…'> isn't watching here; front desk takes it")`.
- `toDesk(reason)`:
  - With a desk → `{to: desk, reason, owned: false}`.
  - With no desk → `{to: null, reason: reason + "; no front desk, so it's held", owned: false}`.

Order, first match wins:
1. **Reply to own post.** If `ledgerRouting && msg.replyToId`, take `poster = ledger.owners("msg:"+replyToId)[0]`. If found → `toOwner(poster, "a reply to a message this chat posted")` (`a reply to a session's own post goes to that session`).
2. **Claimed thread.** If some hook has `threadIds ∋ msg.channelId` and allows the channel → `{to: it, reason: "in a thread this chat claimed", owned: true}`. This step does not check the routing version or `here()` (`a claimed thread goes to its claimant ahead of the front desk`).
3. **PR named.** If `ledgerRouting`, go through `extractPrRefs(content, {defaultRepo})` in order. For the first ref with any owner, take `owner = owners(prKey(ref))[0]`, set verb = `reviewer→"reviewed"`, `touched→"worked on"`, else `"opened"`, and return `toOwner(owner, "about <repoName>#<n>, which this chat <verb>")`, where repoName is the part after `/` (`a verdict on a PR goes to the session that opened it, wherever it's mentioned`, `the first PR named that has an owner decides`).
4. **Front desk.** `toDesk(desk ? "no chat owns this; front desk" : "no chat owns this")`. With no desk this becomes `"no chat owns this; no front desk, so it's held"` (`with nothing to go on, the front desk answers; listeners never do`, `no front desk and no owner: held, and every watcher agrees`).

Edge cases:
- Only `owners(key)[0]` is tried. If the best-ranked owner isn't live and watching here, the mention goes to the desk. The second owner is not tried. This is a quirk, not a stated design.
- A ledger owner with no live hook (closed chat) falls to the desk, and the reason names its label (`a closed or archived owner (no live hook) also falls to the front desk`, `an owner that isn't watching this channel loses it to the front desk, which is told whose it was`).
- A session whose hook has `routing` < 2 is never chosen by steps 1 and 3 (`a session on older watcher code is never sent a mention by the ledger`).

Reason strings are protocol: the mod checks `reason.includes('front desk')` and `reason.includes('Cursor cloud agent')` to add instructions to the wake prompt. Tests `toContain`:
- `"webapp#1437"`, `"no chat owns this; front desk"`
- `"went away before answering"`, `"missed while no chat was watching"`, `"its chat left before answering"`
- `"hadn't started on it after 11 min"`, `"isn't watching here"`, `"held"`, `"reply"`, `"front desk"`

`describeRoute(r)` = `"<describeHook(to)> (<reason>)"` or `"held (<reason>)"`.

### 4.1 Enrich (router.ts) [agnostic, GitHub-specific]
Runs only when `route.to` is set and `reason.startsWith("no chat owns this")`. For the first 2 PR refs, race `prmeta.get(ref)` against 1.5 s. If the PR body has a Cursor agent link, the reason becomes `"about <repo>#<n>, which a Cursor cloud agent opened (<url>); front desk"`. Errors keep the original route.

---

## 5. Delivery: `onMessage`, `routeAndDeliver`, `catchUp` (core.ts)

`onMessage(msg, {reasonPrefix?, feed?})`:
1. `sessions = here(msg)`: attached sessions whose channels or guilds allow it, with the thread parent resolved. If none, return. Nothing is marked, so done-marking only happens when someone watches that channel.
2. If the author is the bot and `replyToId` is set → `mark(done, replyToId)`. A Tsukumo reply marks its target answered (`a Tsukumo reply marks a mention answered, so the sweeper leaves it and catch-up skips it`). [discord: "answered" = the bot replied on Discord]
3. Unless `feed === false`, send a `message` event to every session here. [discord]
4. If `!shouldNotifyChannel(msg, botId)`, return. That check requires `mentionsBot` and non-empty trimmed content. The author must not be the bot; if botId is unknown, the author must not be any bot. [discord]
5. `routeAndDeliver(msg, reasonPrefix)`.

`routeAndDeliver`:
1. If the id is in `routed`, return. Otherwise add it (FIFO, evict the oldest past 2000) (`a mention seen live and again in catch-up wakes its chat once`).
2. Compute `route` (+enrich). `reason = reasonPrefix ? prefix + "; " + route.reason : route.reason`. `watched` = the channel itself if watched, else its parent if watched, else itself.
3. **Target attached here.**
   - `takeMessage(taken, id, target, watched, now, msg.channelId, route.owned ? "owned" : "new")`.
   - On ok: unmark pending; send the target `mention{route:"mine", reason}`; send every other session here `mention{route:"other", routedTo: describeHook(route.to), reason}`.
   - On failure: send all sessions here `mention{route:"other", routedTo: nameOf(holder), reason:"already taken"}`.
4. **Target not attached here** (old watcher, Grok, or a different router): if the desk for the message is attached here and nobody has taken it, write a `pending` row (`by` = route target, kind new). Fall through.
5. Send all sessions here `mention{route: route.to ? "other" : "none", routedTo?, reason}`. Route `none` is **unowned-held**: nobody takes it and no take row is written, so no sweep sees it. It waits until a person uses `take` (the mod toasts "no chat owns it. Take it from the pane").

`catchUp(messages oldest first)` [discord trigger, agnostic rule]:
- Mark done for every bot reply in the batch.
- Then for each notify-worthy message within 6 h that is neither done nor taken, call `routeAndDeliver(m, "missed while no chat was watching")` (`mentions missed while the router was down are routed on catch-up`).

---

## 6. States of a mention

Two different meanings of "held" exist and must stay separate in Irori:
- **unowned-held**: route `none`, no take row, only `take` resolves it.
- **person-held**: the `held` dir. Set by the mod for blocked or not-allowlisted senders and by dismiss. `isHeld` excludes it from sweeps and reminders. `take` clears it.

Derived states for a taken mention `t` (holder `t.by`):
- **answered**: `isMarked(done)`. Set by a bot reply to it or by `skip`.
- **skipped**: `skip` control by the holder only (else 409). It writes `done`, so it is answered and never moved or reminded (`only the chat holding a mention can skip it, and a skipped mention is never moved or reminded`).
- **engaged**: `isSeen(id, t.by) || deferredUntil(id, t.by) !== null`. `seen` is written only for the current holder (`only the holder's own turn counts as started on it`). The mod sends `seen` on `turn.start` when the turn text contains `message id <id>` for a delivered mention still `awaitingStart`.
- **deferred**: `defer` control. Returns 409 if the caller isn't the holder and 400 if `untilMs` is not finite or not in the future. It stores `min(untilMs, now+12h)` with the note, then **clears `flagged`** so the new time earns its own reminder (`only the chat holding a mention can defer it, and the sweeper honors it`).
- **owned**: `t.kind === "owned"`. Exempt from unengaged moves, but not from gone moves.
- **open** (`open(t)`):
  - `t.channelId` is non-null and in scope;
  - not answered, not person-held;
  - `now - max(t.atMs, deferredUntil(t.by) ?? 0) ≤ 6 h`, so a long deferral keeps it in view (`a long deferral keeps the mention in view: its holder dying at hour 7 still moves it`).

  A take row with `channelId: null` (a `take` control sent without `channelId`) is never open, so it is invisible to both planners. This is an edge case, not a stated design.

---

## 7. Sweeper

### 7.1 `planSweep(taken, pending, opts)` runs for each attached front desk over the channels it fronts [agnostic]

For each open `t` in `taken` (`mine = t.by === me`):
1. **Gone.** `!mine && goneFor(t.by) > 3 min` → `reroute{retake:true, from:t.by, reason:"<name> took it but went away before answering"}`. This applies to every kind, ignores engaged, owned and `wasRerouted`, and can repeat (`a taker gone 3 minutes loses it, whatever it is…; a restart blip doesn't`).
2. If engaged, owned, or `age ≤ 10 min` → skip (`engaged, deferred, or the chat's own work is never moved while the chat is around`).
3. Otherwise this is unengaged new work older than 10 min, with `minutes = floor(age/60000)`.
   - **Not mine:** if not `wasRerouted` → `reroute{retake:true, reason:"<name> hadn't started on it after <m> min"}`; else nothing (`new work a chat hasn't started on moves to the front desk after 10 minutes`).
   - **Mine:** if not `wasRerouted` and `canHandOff(channel)` → `handoff{minutes}`. Else, if not `wasFlagged(id, me)` → `overdue{to:me}` (`the front desk not starting on new work hands it and the role on, once; otherwise it's flagged`).

For each `p` in `pending`:
- If `hasTaker(p)` or `!open(p)` → `clear`.
- Else if `now - p.atMs > 90 s` → `reroute{retake:false, from:p.by, reason:"routed to <name>, which never picked it up"}` (`a mention sent to a session that never took it comes back after 90s; one it took clears`).

Thread messages keep `channelId` = parent (scope) and `messageChannelId` = thread (fetch) (`a thread message keeps its own channel for fetching and its parent for scope`). Answered, person-held, out-of-scope, null-channel and >6 h items are left alone (`answered, held by a person, other channels, unknown channels and history are left alone`).

### 7.2 `planReminders(taken, {holds})` runs over every take this process can reach [agnostic]
For open `t` (scope: any channel) with `holds(t.by)` true and not `wasFlagged(id, t.by)`:
- `due = deferredUntil(id, t.by) ?? t.atMs + 60 min`.
- If `now > due` → `overdue{minutes: floor((now - atMs)/60000), to: t.by}`.

Tests:
- `an hour with no reply and no deferral: remind the holder, once`
- `a deferral moves the reminder to its time, even past the usual lookback`
- `an earlier holder's deferral or flag doesn't count for the new holder`
- `only holders this process can reach are reminded`
- core: `a verdict on a busy chat's own PR waits for it; only after an hour does it get a reminder`

### 7.3 `RouterCore.sweep()`: executing the plans
- `goneSince`: for every taker in `taken`, delete it if its hook is live, else set it to `now` if not already set. `forget` also sets it at once. `goneFor(c) = live ? 0 : now - (goneSince[c] ?? now)`.
- Liveness here = live hook row, i.e. heartbeat < 90 s. Effective timeline when a chat vanishes:
  - stream drops;
  - 2 min reattach grace;
  - `tick` → `forget` (row cleared, `goneSince = now`);
  - more than 3 min later, a sweep reroutes (`a chat that doesn't come back loses its claim, and the front desk takes back its mention`).
- For each attached session that is the desk of ≥1 of its channels, plan, then for each action:
  - `clear` → unmark pending.
  - `overdue` → `remind`.
  - For `handoff`, and for `reroute` with `retake`, **re-check against now**:
    - the current holder must equal the planned holder (`me` for handoff, `from` for reroute);
    - `current.atMs` must equal the planned `atMs`;
    - it must not be answered, seen by that holder, or deferred by that holder.

    Otherwise skip the action.
  - `handoff`:
    - `next = handOffDesk(session, channel)`. If null → `remind` the desk instead.
    - Otherwise `moveTo(next, action, me, "<desk name> (front desk) hadn't started on it after <m> min, so it and the front desk role came to this chat")`, send the old desk `handoff{to: nameOf(next)}`, then **break**, ending this desk's actions for this sweep.
  - `reroute`:
    - If `!retake` (pending) and `takeMessage(taken, id, me, ...,"new")` fails → unmark pending and continue.
    - Otherwise `moveTo(me, action, from, "<reason>; front desk takes it")`.
- Then `planReminders` over all of `taken` with `holds = attached here`; each `overdue` → `remind`.

`moveTo(to, action, from, reason)`:
1. `retake(taken)` → `to`, with atMs = now and **kind = "new"**.
2. Unmark pending, seen, deferred and flagged; mark rerouted.
3. If the previous holder is attached here and is not `to`, send it `moved{to: nameOf(to)}`.
4. `fetchMessage(messageChannelId, id)`. If found, send `to` a `mention{route:"mine", reason}`. If the fetch fails, the take has still moved but nobody is woken. **Unsure:** whether that is intended.

Once-only semantics: after any move the item is `new`, has a fresh `atMs` and is marked `rerouted`. A second unengaged timeout on the new holder therefore can't reroute or hand off again; it yields `overdue` once (flagged per holder). Gone-reroute is not limited this way.

`remind(action)`: only if the holder is attached here and not `isMarkedFor(flagged, id, holder)`. Mark flagged with the holder, then send `overdue`. The mod then toasts and submits a reminder prompt, unless the pane shows that mention as held, dismissed or skipped (`a lone front desk that hasn't started on new work after 10 minutes is reminded once (nobody to hand it to)`).

### 7.4 Front-desk handoff [agnostic algorithm, adapt scope]
"A role belongs to a chat, not a channel": a desk that steps down anywhere steps down everywhere.

- `othersWatching(me, ch)` = attached sessions other than me, connected (sinks > 0, not in grace), watching `ch`, sorted by `boundAt` ascending.
- `canHandOff(desk, ch)` = for every channel `id` the desk watches: if `id ≠ ch` and the desk is not `frontDesk(id)` → fine; else `othersWatching(desk, id).length > 0`.
- `handOffDesk(desk, ch)`:
  1. If `!canHandOff`, return null.
  2. `fronted` = desk channels equal to `ch` or currently fronted by desk.
  3. Set desk role listener (on a store error return null).
  4. For `ch` first, then the other fronted channels:
     - if some desk already exists there now, use it (for `ch` it becomes `next`);
     - else promote the first `othersWatching` that accepts `role:"owner"` (a 409 means try the next).
  5. If there is no `next` for `ch`, or any fronted channel is left without a desk → undo: demote everyone promoted, restore desk to owner, return null ("better a slow front desk than a channel with none").
  6. Force-emit `claims` to the desk and everyone promoted. Return `next`.

Test: `a front desk that doesn't start on new work for 10 minutes hands it, and the role, to the next chat`. Desk a becomes listener and b becomes owner. a gets `handoff{to:"api work"}` and `moved`; b gets `mention{mine}` with reason containing "hadn't started on it after 11 min". `holder` from a returns `{holder:"b", label:"api work", mine:false}`.

Related tests:
- `a mention a person handed to a chat stays with it`: `take` → owned, not moved after 30 min.
- `a front desk that started on it keeps it, however long it takes`: seen → no handoff or overdue at 30 min.

---

## 8. Session lifecycle (attach / detach / forget / tick)

`attach({sessionId, label, channelIds, channelNames, channelGuilds, sink})` → `{fresh}`:
- **Same session, same channel set** (same length, every id included): add the sink, clear `detachedAt`, update the label in the store if it changed, **flush the queue to the new sink**, force-emit claims → `{fresh:false}` (`a chat reloading keeps its claim and gets what arrived meanwhile`, direct `wait for a chat that's reloading, and arrive when it reattaches`).
- **Same session, different channels:** `forget(sessionId)` without requeue, then continue as fresh. The queue is dropped and its takes stay held. This is a quirk.
- **Fresh:**
  - `previous = store.load(id)`. Keep its `threadIds` if its guilds overlap the new ones.
  - Build the row: `boundAt = now`, `routing: 2`, label.
  - Try `role:"owner"` then `"listener"` on 409. If the previous row was a listener, try only listener (`a chat's role and thread claims survive a router restart`). The first chat is desk and the second listens (`the first chat is the front desk, the second listens…`).
  - Heartbeat; aliases from `aliasStore`; emit claims → `{fresh:true}`. The caller then sends history and catch-up.

`detach(id, sink)`: remove the sink. If none are left, set `detachedAt = now`.

`send(session, ev)`: write to every sink. With no sink, only `mention|overdue|moved|direct` are queued (cap 50, drop oldest). Others (claims, message, history, handoff, error) are dropped. **Unsure:** whether dropping `handoff` while detached is intended; `moved` is sent alongside it.

`forget(id, {close?, requeue?})`:
1. Delete the session and set `goneSince` if unset. If `close`, close the sinks.
2. `wasDesk` = the row's boundAt matches and it is an owner. Clear the row only if boundAt matches ("only ever clear the row we wrote").
3. If it was the desk → `promoteSuccessor`: the longest-attached other session sharing any channel that accepts owner.
4. If `requeue`: for each queued `mention{route:"mine"}`:
   - unmark its take if held by this session;
   - delete it from `routed`;
   - `routeAndDeliver(msg, "its chat left before answering")`.

   Tests: `mentions queued for a chat that never came back are routed again when its grace ends`, `an explicit stop hands the front desk to the next chat at once`, `a second chat in one of a multi-server desk's channels listens there, and takes over when it stops`.

`tick()` (30 s, and before every sweep), for each session:
- If detached for more than 2 min → `forget(requeue)`.
- Else heartbeat. If the row is gone (someone removed it) → send `error{fatal:true, message:"This chat's watch claim was removed from .hooks.json; stopping."}` and `forget(close, requeue)` (`a chat whose claim someone removed is told and its stream closed`).
- A store-busy exception is ignored until the next tick.
- Then emit claims.

`releaseAll()` on shutdown: `forget` each session without requeue.

---

## 9. Control ops (`control(sessionId, op, input, threadParent?)`) [mixed]

Every op returns 404 `"This chat isn't attached to the router."` for an unknown session. Store errors map to their status (409 conflicts), anything else to 500.

| op | Input | Effect | Response |
|---|---|---|---|
| `release` | | `forget(requeue)`, no grace | `{ok}` |
| `own` / `listen` | | `store.update(role)`. 409 if another live owner overlaps | `{ok}` + claims re-emit |
| `claim-thread` / `release-thread` | `threadId` (snowflake, else 400 "Give a thread id.") | claim requires `threadParent ∈ session.channelIds`, else 400 "That isn't a thread in a watched channel." (router.ts fetches the parent from Discord). Adds or removes the id. 409 if another chat claims it | `{ok}` + claims |
| `take` | `messageId`, `channelId?` | `takeMessage(..., watchedChannelOf(channelId), now, channelId, "owned")`; unmark held | `{ok}` or 409 `"Already taken by <name>."` |
| `hold` | `messageId` | mark held (person-held) | `{ok}` |
| `seen` | `messageId` | mark seen with the caller only if the caller is the holder; otherwise a silent no-op | `{ok}` |
| `holder` | `messageId` | | `{ok, holder: id\|null, label, mine}` |
| `defer` | `messageId, untilMs, note?` | §6 | `{ok}` / 409 / 400 "Give a time in the future." |
| `skip` | `messageId` | holder only, else 409 `"This chat isn't the one handling that mention."`; mark done | `{ok}` |
| `alias` | `name` | `addAlias` | `{ok}` |
| `chats` | | | `{ok, chats:[{session, name, label\|null, aliases[], connected}]}` (others only) |
| `poster` | `messageId` | `ledger.owners("msg:"+id)[0]`. Reported only if it's another session attached here | `{ok, session\|null, label\|null}` |
| `direct` | `to, content, from?, replyToId?, about?` | §10 | |
| other | | | 404 `Unknown control "<op>".` |

Only own, listen, claim-thread and release-thread fall through to the forced claims re-emit. Router-only ops handled in router.ts before core: `ack` (👀 + typing on Discord) and `quiet` (remove both); after a successful `skip`, also `quiet`. [discord]

`addAlias(raw)`: trim and lowercase; ignore empty or >60 chars. Delete then re-add, so it becomes the newest. Evict the oldest past 10. Persist via aliasStore. `displayName` = newest alias, else label, else `id[:8]+"…"`.

---

## 10. Direct messages (`message_chat` / `list_chats`, direct.jsonl) [agnostic]

`direct(session, input)`:
1. `content = trim(input.content)`. Empty → 400 "Give the message text.". Over 8000 → 400 `Too long (<n> characters; at most 8000).`.
2. `resolveChat(me, to)`:
   - `wanted = trim(lowercase(to))`. Empty → 400 "Say which chat (`to`)."
   - Equal to my own id or one of my aliases → 400 "That's this chat."
   - `byId` = others whose id equals `wanted`, or whose id starts with `wanted` when `wanted.length ≥ 8`.
   - `byName` = others with `wanted` among their aliases, or a label that equals it case-insensitively.
   - `matches = byId.length ? byId : byName`, so id beats name.
   - 1 → ok. More than 1 → 409 `"<to>" matches N chats: <name (id8)>, …. Use a session id.`. 0 → 404 `No chat here called "<to>". Chats attached now: <names|none>.`
   - Test: `find a chat by session id or label, and refuse unknown, ambiguous and self`. Two chats labelled "worker" give 409; the full id works.
3. Rate limit:
   - The pair key is the two session ids sorted and joined with `|`.
   - Keep only timestamps with `now - t < 10 min`.
   - If 8 or more are kept → 429 `Not sent: <n> direct messages between this chat and <name> in the last 10 minutes. Stop the back-and-forth: post on Discord, or ask the person at your prompt.`. A refused attempt is not counted.
   - Otherwise push `now`.
   - Test: `cap the back-and-forth between two chats, then allow it again after the window`. Eight alternating sends succeed, the ninth is 429, and after window+1 s it is allowed.
4. If `from` is non-empty, `addAlias(sender, from)`. The sender becomes addressable by the name it signed with (`go to the chat named by the name it posts under, and nowhere else`: b replies `to:"Front Desk"`, which reaches a).
5. Build the event:
   - `id = "d" + base36(now) + 6 random base36 chars`;
   - `from = from || displayName(sender)`;
   - `replyToId` if it is a non-empty string;
   - `about` only if `about.messageId` is a snowflake;
   - `sentAt` as ISO.
6. `send(target, ev)` (queued if detached). `logDirect({...ev, to})` errors are ignored.
7. Return `{ok, id, to: displayName(target), session, queued: no sinks}`.

The alias store survives router restarts (`a restarted router still finds a chat by the name it posted under`). `chats` lists others with `name = displayName` (`list the other chats with the names they go by`). `poster` lets the mod divert a Discord reply to a local chat's post into a direct message (`a reply to a Discord message another chat posted can be told apart`).

Mod-side behaviour (register.tsx):
- `message_chat` → `/direct`. A `direct` event → toast plus `$.prompt.submit(directPrompt)`, fenced in `<chat-message>`. There is no block/allow check on directs.
- `send_message` with `reply_to_id` (no `post`, no gif) asks `/poster`. If another local chat posted it, the mod sends a direct with `about` instead of posting. [discord-motivated: the bot's own posts never notify]
- Also before posting a reply: `/holder`. If another chat holds it, the mod refuses unless `allow_duplicate`.
- Every `send_message` with `from` calls `/alias`.

---

## 11. Authorship ledger [agnostic]

- `record(entries, now)`:
  - Under the file lock, merge on the identity `key\0sessionId\0role`.
  - A new identity is appended.
  - An existing one keeps the **earliest** `at`, takes the new label if given, and keeps `source:"mod"` once set (`recording the same thing again keeps the earliest time`).
  - Then drop entries with `at < now - 45 days` (`entries older than 45 days are dropped`).
- `owners(key)` = entries for the key sorted by `ROLE_RANK` then `at` ascending. Code: `author 0, poster 0, touched 1, reviewer 2`. The doc comment ("then who reviewed it, then who merely touched it") contradicts this; **port the code**, which the test `owners rank who made a thing over who touched or reviewed it, then earliest` encodes: early-toucher, late-toucher, reviewer, then author on top.
- Producers:
  - `poster`: call.ts `send` (unless the REST layer reports a duplicate) and the first message of `thread`, key `msg:<id>`. [discord]
  - `author`: the mod parses `gh pr create` stdout for `github.com/<o/r>/pull/<n>`.
  - `reviewer`: `gh pr review <target>` where the target is a URL or a number. A number takes the repo from `-R|--repo <o/r>`, else from the cwd's `git remote get-url origin` via `repoFromRemote` = `/github\.com[:/]([\w.-]+\/[\w.-]+?)(?:\.git)?$/`. Regexes in `ledgerEntriesFromBash`:
    - `/\bgh\s+pr\s+create\b/` gates the stdout scan;
    - `/\bgh\s+pr\s+review\s+(\S+)([^\n;&|]*)/g`.
  - `record` op validation: key matches `/^(msg|pr|branch):/`, role is one of the four; the key is lowercased; source `mod`.
  - `touched`: `syncPrLinks(~/.claude/projects)`, an incremental scan of `<project>/<session>.jsonl`.
- `syncPrLinks` details:
  - Read only bytes `[scanned[file], size)` up to the last `\n`. A partial line waits.
  - If the file shrank, reset its offset to 0 for the next pass.
  - Skip chunks without `"pr-link"`; parse lines containing `"type":"pr-link"`.
  - From `{sessionId, prNumber, prRepository, timestamp}` produce key `pr:<repo lower>#<n>`, role touched, at = timestamp, source pr-link, label `labelFor`.
  - Test: `pr-link rows are read from transcripts, only what's new, and a half-written line waits`.
- `labelFor`: the project dir name with `/^-Users-[^-]+-?/` stripped, then `" · " + sessionId[:8]`, e.g. "development-webapp · ac2b9a60". This is a macOS path assumption. Windows project dirs (`C--Users-4step-…`) won't be stripped. [adapt]

---

## 12. PR ref parsing (refs.ts) [agnostic]

`extractPrRefs(text, {defaultRepo?, owner?, aliases?})` → `PrRef{repo, number}[]`, in order, deduplicated by `prKey`.
- `owner = opts.owner || env TSUKUMO_GITHUB_OWNER || defaultRepo's owner`.
- `aliases = opts.aliases ?? repoAliases(env TSUKUMO_REPO_ALIASES)`. The format is `alias=repo` pairs split on `,` and trimmed; the alias is lowercased and the repo kept as given (`web=webapp,api=acme/api-server`).
- First strip code: ```` ```…``` ```` (lazy, multiline), then `` `…` `` (no newline), each replaced with a space.
- Pattern (JS, global):
  ```
  github\.com\/([\w.-]+)\/([\w.-]+)\/pull\/(\d+)
  |(?<![\w/<#-])([A-Za-z][\w.-]*(?:\/[\w.-]+)?)#(\d{1,6})\b
  |(?<![\w/<#&@])#(\d{2,5})\b
  ```
  1. **URL** → `owner/repo#n`.
  2. **`name#N`** (1–6 digits). The name starts with a letter and may be `owner/repo`. It must not be preceded by a word char, `/`, `<`, `#` or `-`.
     - Name `pr` or `pull` (case-insensitive) → defaultRepo (skip if none).
     - Else apply the alias (lowercased lookup).
     - If the result has `/`, use it; else `owner/name` if an owner is known; else skip.
  3. **Bare `#N`** (2–5 digits). Not preceded by a word char, `/`, `<`, `#`, `&` or `@`, which excludes Discord `<#chan>`, `<@&role>`, `<@user>`. Requires defaultRepo.
- `prKey = "pr:" + lower(repo) + "#" + n`.
- `repoFromTopic`: `/github\.com\/([\w.-]+)\/([\w.-]+)/i`. Strip trailing dots, then `.git`. [discord: channel topic → Irori: git remote]

Tests:
- `URLs, repo#N, aliases and bare #N in order, without repeats`: the URL and `webapp#1407` dedupe; `SHOP#498` → `acme/storefront`; `api-server#143` gets the default owner.
- `bare #N needs the channel's repo; channel and role mentions are not PRs`.
- `owner/repo#N, PR#N, code spans and long numbers`: `#333333` (6 digits) is not a bare ref.
- `a repo without an owner needs one from somewhere`.
- `a channel topic names its repo`.

**Go port:** RE2 has no lookbehind. Match the alternations without it and check the preceding rune by hand:
- alt 2 rejects `[\w/<#-]` before the name start;
- alt 3 rejects `[\w/<#&@]` before `#`;
- `\b` after the digits means the next rune is not a word char.

Note that JS `\w` is ASCII.

prmeta [agnostic, GitHub]:
- `cursorAgentFrom(body)` = `/https?:\/\/(?:www\.)?cursor\.com\/agents\/(bc-[\w-]+)/` → `https://cursor.com/agents/<id>`.
- `gh pr view <n> --repo <r> --json body,author`.
- On-disk cache keyed by prKey, with TTL hit 24 h and miss 10 min (`lookups are cached, and failures are retried later`).

---

## 13. Wire protocol: mod ↔ attach ↔ router

1. The mod spawns `bun run src/attach.ts <id,id,…>` (or `src/watch.ts` when `transport === 'direct'`). Env: `TSUKUMO_SESSION_ID`, `TSUKUMO_SESSION_LABEL` (the `/discord name` label, else `<cwd basename> · <id8>`), and `TSUKUMO_WATCH_SOCKET = <tsukumoDir>/.watch/<sessionId[:12]>.sock`.
2. attach serves the per-session control socket. The mod `POST`s `http://watcher/<op>` with JSON. attach forwards to the router's `POST /control` with body `{...body, sessionId, op}` and mirrors the status and body. If the router is unreachable it returns 503 `{ok:false, error}`.
3. `ensureRouter`: `GET /status` on `.watch/router.sock`. If that fails, spawn the router detached and poll.
4. `POST /attach {sessionId, label, channelIds}`. The router validates snowflakes and server channels (400 otherwise) and replies `application/x-ndjson`.
5. After the response starts, in this order:
   - `core.attach` (emits `claims`; on a reattach it flushes queued events first);
   - `ready`;
   - if fresh, a `history` per channel;
   - REST catch-up for channels newly watched by anyone. Each yields `mention` events.

   Live traffic follows: `message`, `mention`, `overdue`, `moved`, `handoff`, `direct`, `claims`. A `"\n"` keepalive goes every 5 s; a failed enqueue means detach.
6. attach copies every non-empty line to stdout. It exits 1 on a line with `"type":"error"` and `"fatal":true`. It reconnects with backoff on stream loss and exits on SIGTERM/INT/HUP or when orphaned.
7. Router `POST /stop` → releaseAll and exit.

Mod reactions per event:
- `mention` with route `other` → pane only ("elsewhere").
- `mention` with route `none` → toast "Take it from the pane".
- `mention` with route `mine`:
  - If the block/allow policy wakes it: `deliver` = mark delivered with `awaitingStart`, `$.prompt.submit(wakePrompt)`, then `/ack`.
  - Otherwise toast and `/hold`.
- `turn.start` → `/seen`, with retries.
- `turn.complete` → `/quiet`.
- `overdue` → toast plus a reminder prompt, unless the pane shows it held, dismissed or skipped.
- `moved` → status elsewhere plus a toast.
- `handoff` → toast.
- `direct` → wake with `directPrompt`.

Wake prompt contract (wake.ts):
- Content is fenced in `<discord-message>…</discord-message>`, escaping a closing tag inside it as `<\/discord-message>`.
- It carries the "Why this chat: <reason>." line, the untrusted-content and privacy rules, and how to reply, defer or skip.

---

## 14. What carries over to Irori, and what dies

Irori messages are agent-to-agent and explicitly addressed to a **session, name, repo, thread or PR**. There are no @mentions to detect and no bot whose posts never notify. A message always knows its sender. Plan 01 §3 names, as the port: core, route, take, sweep, ledger, refs, DIRECT_LIMIT, and the claude-mod skeleton (wake prompt, fencing, held messages, block/allow).

### 14.1 Per addressing kind

| Irori address | Tsukumo step it replaces | Changes |
|---|---|---|
| **session** (`<deviceId>/<sessionId>`) | `resolveChat` byId (exact or ≥8-char prefix) | Port it, but decide whether a prefix may match across devices or only the local device; an ambiguous prefix stays 409. An explicitly addressed session is `kind:"owned"`. **Open:** whether the sweeper's `PICKUP_GRACE` (90 s) bounce to a desk should apply at all when the sender named a session. The likelier answer is no: hold in the inbox, remind, offer revive (decision 3), and let gone-reroute apply only if the sender opted in. |
| **name** | `resolveChat` byName (aliases + label), `addAlias` on `from` | Port it: lowercased, 60 chars, 10 per session, id beats name, 409/404 messages. The source of names grows: presence `Name`/`Title` from the AgentCard, plus `from` aliases. Persist in SQLite, not router-state.json. |
| **repo** (`repo_key`) | "watched channel" scope + `frontDesk` + step 4 | The channel becomes the repo_key. Membership is derived from the agent card's cwd→repo_key, not from a claim. Front desk = earliest-bound owner among sessions in that repo. The one-owner conflict rule, `promoteSuccessor`, `canHandOff`/`handOffDesk`/undo, `planSweep` (10 min unengaged / 3 min gone) and `planReminders` (60 min / deferral) all carry over with repo scope. `defaultRepo` for PR parsing comes from the sender's or target's git remote (`repoFromRemote`), not a topic. |
| **thread** (Irori thread / contextId) | step 2 claimed thread + `claim-thread`/`release-thread` | One claimant per thread (`a thread has one owner at a time`); routing to the claimant comes before the desk. The "thread must be under a watched channel" check becomes "thread belongs to a repo the session is in", or is dropped. |
| **PR** | step 3 + ledger `pr:` keys + `extractPrRefs` + pr-link sync | Carries over almost whole. Keep ROLE_RANK as coded (touched before reviewer), `owners[0]` (consider fixing the no-second-owner quirk), 45-day retention, earliest-`at` merge, and the `gh pr create/review` capture (from mod `tool.call` Bash, as now). `syncPrLinks` folds into v0.3 capture (plan 04 §7 "harvest pr-link"). An explicit `to: PR` address skips parsing; free text in a message to a repo can still be parsed for refs. Fix `labelFor` for Windows project dir names. |
| **reply** (to an Irori message) | step 1 `msg:` poster lookup | Collapses: every Irori message has `from`, so a reply routes to the original sender directly. `poster` ledger rows, the `poster` control and `replyDirectly` all die. |

### 14.2 Carries over (transport-agnostic)
- **Take-lock semantics:** exclusive first-writer-wins, idempotent for the same holder, `owned`/`new` kind, retake on move. In Irori it becomes a SQLite row with insert-if-absent (plan 01 §5: the target session's device owns it).
- **State flags as delivery fields:** answered, person-held, rerouted, flagged(by), seen(by), deferred(by, until, note). They map onto the delivery machine (plan 04 §7):

  | Irori delivery state | Tsukumo equivalent |
  |---|---|
  | queued | in the queue or pending |
  | held | person-held; also bypass-receiver hold and unowned-held, kept as distinct reasons |
  | delivered | take ok + mention{mine} + `$.prompt.submit` |
  | engaged | seen via `turn.start` matching the delivery id |
  | answered | reply. Irori needs an explicit reply/ack tool, since there's no "bot posted with replyToId" |
  | deferred | deferred |
  | skipped | skipped |
  | expired | lookback past 6 h. **Unsure** whether Irori wants expiry for explicitly addressed messages at all (the README says messages are never dropped) |
  | rejected | new |

- **Per-holder invalidation:** a move clears seen, deferred and flagged; a deferral clears flagged; deferrals and flags by an earlier holder don't count.
- **`planSweep` and `planReminders` exactly**, including re-check-before-act, the one handoff per sweep, the "once" semantics and the gone timeline. They are pure functions, and their tests port as table tests.
- **Front-desk handoff algorithm** and successor promotion.
- **Attach lifecycle:**
  - REATTACH_GRACE 2 min and QUEUE_LIMIT 50 with queue flush on reattach. Spike 01 shows mods reload mid-session, so `irori attach` must be idempotent per session id: a second attach with the same id is the reattach path.
  - Requeue on forget, and the fatal error when a claim is removed.
- **Routed-id dedupe** (2000 FIFO). Simpler in Irori: delivery ids are unique and consumed once.
- **Direct-message rules:** DIRECT_LIMIT 8 per pair per 10 min (plan 01 §3 names it), MAX 8000 chars, id/name resolution, the `queued` flag, the `from` alias, `replyToId` threading, and the append-only log (an event kind in the store instead of direct.jsonl). Add plan 04's hop count and causal parent on top. The 429 text must drop "post on Discord".
- **Mod contract:**
  - fenced wake prompt (rename to `<irori-message from=… trust=untrusted>` per plan 01 §4);
  - `turn.start` → engaged and `turn.complete` → end of working;
  - block/allow → person-held;
  - the "holder" guard before replying to something another session now holds;
  - `defer`/`skip` tools with holder-only checks;
  - the reminder prompt.
- **Event vocabulary:** claims (role, owner, listener count), mention → `message`/delivery, overdue, moved, handoff, direct, error{fatal}.
- **Reason strings as protocol.** Keep them stable or replace the mod's `includes('front desk')` with a structured field (better: `route.kind: "owner"|"thread"|"pr"|"desk"|"held"`).

### 14.3 Becomes dead code
- The Discord gateway and REST, `session.ts`, `call.ts` as a transport, login budget, and `ack`/`quiet` (👀 + typing). The `record` op's validation survives as a tool.
- `shouldNotifyChannel`, `mentionsBot` and `toChannelNotification`. Addressing is explicit; the Grok channel push is a separate v0.2 item.
- `message`/`history` feed events, watermarks, REST catch-up and `CATCH_UP_WINDOW`. A persistent inbox replaces catch-up: undelivered items are simply still queued.
- Bot-reply done-marking. Replaced by explicit reply/ack.
- `SNOWFLAKE` validation and path-safety. Hub-minted ids in SQLite replace it.
- `.hooks.json`, `filelock.ts`, `writeJsonAtomic`, the flag directories, `pruneTaken`, the heartbeat file, the 5 s claims polling and the 90 s stale heartbeat. Irori is one process with SQLite. Liveness comes from presence (`sessions/<pid>.json` plus pid) and the attach stream, so `goneFor` measures from presence `gone`/stream loss. The 3 min GONE_GRACE stays as the debounce.
- `ROUTING_VERSION`, the `routing` field, the old-desk fallback, `grokSessionExists` and `watch.ts`. These existed for mixed old and new watchers on one Discord bot.
- Multi-guild logic (`guilds`, `hookGuildIds`, guild matching).
- The unix sockets (router.sock, per-session .sock), router spawn/linger/pid, attach's router-start and orphan `ppid===1` check (Unix-only). Irori uses loopback HTTP plus the `irori.json` port/token (decision 8, plan 01 §4). **Unsure:** the cleanest control path for the mod. Either `$.http.fetch` to loopback with the token handed over in `irori attach`'s `ready` line, or `irori attach` keeps proxying. The mod sandbox has no `process.env` (spike 01), so the token must reach it through the helper or `userConfig`.
- The `poster` control, `replyDirectly`, `send_message`'s `post` flag and the Discord-side duplicate-reply detection.
- `repoFromTopic` (channel topic) and the `TSUKUMO_*` env vars for owner and aliases. Repo aliases may survive as an Irori setting.
- prmeta/Cursor enrich is not Discord-specific but only matters for PR-verdict mentions arriving from GitHub bots on Discord. It is optional in v0.2. **Unsure** whether any Irori message source produces those verdicts before v0.4.

### 14.4 Tension to note
Plan 01 §8 says "Claude-to-Claude messaging on one machine: native ListAgents/SendMessage already does it… the hub only observes it". This v0.2 port nonetheless makes Tsukumo's local direct messaging (`message_chat`) the core channel. Reconcile before building: either Irori messaging targets non-Claude agents, cross-device delivery, durable inboxes and audit (the gaps plan 04 §5 lists), or the plan's §8 line is superseded.