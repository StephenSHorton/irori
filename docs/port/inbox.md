# Port spec: inbox, notify, awake

Source (read-only): `../irori-go/internal/{inbox,notify,awake}`, plus
`internal/tools/notice.go` (digest dedupe) and the go-toast module
`github.com/StephenSHorton/go-toast@v0.2.0` (Go module cache). Paths below are
relative to `internal/` unless they start with `go-toast/` or `tools/`.

The goal is byte-compatible log rows and identical behaviour toward agents (wake prompts,
tokens, MCP tool semantics) and the person (cards, Held list).

---

## 1. What the area does

| Package | Role |
|---|---|
| `inbox` | The messaging core. It routes a message to exactly one recipient, keeps a per-recipient **delivery** state machine, applies the wake **policy** (holds), the rate and hop limits, plans **wakes** (a prompt that starts a turn) and **appends** (text the model reads on its next turn), runs the **sweeper**, and handles cross-device ownership (`delivery.decided`). All state is folded from the event log. It knows nothing about HTTP, Wails or the mod (`inbox/inbox.go:1-13`). |
| `notify` | Desktop cards. The `Notifier` wraps go-toast (`notify/notify.go:182-222`). `Watch` turns inbox, coord and ask change streams into cards (`notify/watch.go:111-195`). Settings persist in `settings.json` (`notify/settings.go`). |
| `awake` | Holds off idle system sleep while the home server runs (`awake/awake.go`). It is used only by `home/home.go:147,178,204`. |
| `tools/notice.go` | Routes `coord.Notice` through the inbox and merges watch digests (`tools/notice.go:32-75`). |

### 1.1 Public interface of `inbox.Inbox` (`inbox/inbox.go:385-581` unless noted)

| Method | Semantics |
|---|---|
| `New(ctx, store, roster, Options)` | Replays the log, then returns. Errors on a nil store or roster (`state.go:245-293`). |
| `Send(from, to, text, reason)` | Equals `SendWith(..., {Reason, Mode: wake})` (`messages.go:26-28`). |
| `SendWith(from, to, text, SendOptions)` | Returns `(Message, Delivery)`. A held delivery still counts as a successful send. |
| `Reply(from, deliveryID, text)` | Allowed from Queued, Delivered, Engaged or Deferred. Sends a `ToReply` with `Mode: wake` and `Ref: orig.Ref`, then marks the original Answered (`messages.go:195-226`). |
| `NextWake(agent, path)` / `NextAppend(agent)` | Plan and record one wake or append, or return nil (`wake.go:23-120`). |
| `OutstandingWake(agent)` → `(*Wake, State)`; `OutstandingAppend(agent)` | Rebuild the gating wake or pending append with its original token so a reattached stream can resend it (`wake.go:334-372`). |
| `MarkDelivered`, `MarkAppendedDelivered(agent, token)` | `wake.go:245-274` |
| `MarkEngaged`, `MarkAppendedEngaged(agent, token, turnID)` | `wake.go:276-315` |
| `TurnEnded(agent, token, turnID)` | `wake.go:317-328` |
| `Ack(agent, did)` | Channel path: Queued→Delivered→Engaged (`messages.go:231-257`). |
| `MarkRead(agent, did)` | Queued→Delivered with via `channel`. Idempotent (`messages.go:262-276`). |
| `Defer(agent, did, minutes, note)`, `Skip(agent, did, note)` | `messages.go:278-319` |
| `Release(did, by)`, `Dismiss(did, by)`, `Move(did, to, by)` | Person actions. On another device's delivery they write a `delivery.decided` request instead (`messages.go:326-380`, `mesh.go:319-338`). Release and SetPolicy then run `fillCards`. |
| `For(agent, all, limit)` | The recipient's own view. It never includes Held or Dismissed (`messages.go:398-407`). |
| `List(limit)`, `HeldHistory(limit)`, `OfMessage(id)`, `Open()`, `Get(id)`, `Message(id)`, `Aliases(agent)` | Read views (`messages.go:383-492`). |
| `Policy()`, `SetPolicy(p, by)` | `policy.go:138-190` |
| `Sweep(ctx)` | Runs sweep, then `handOff`, then `fillCards` (`inbox.go:553-561`). Core calls it every `SweepInterval` (60 s). |
| `EndedFor(agent)`, `RunningWake(agent)`, `Reachable(agent)` → `(mod, channel)`, `FrontDesk(repo)`, `RecordRoster(ctx)` | `route.go:255-289`, `wake.go:386-391` |
| `SetReachable(agent, path, on)` | `route.go:190-222` |
| `Subscribe(ctx)` → `chan Change` | Buffer 64. A full channel drops the change (`state.go:29,694-701,829-842`). |
| `Follow(ctx)`, `CatchUp(ctx)` | Mesh fold and owner pass (`mesh.go:75-136`). |
| `Owned(did)`, `IgnoredRows()`, `IgnoredBy()` | `mesh.go:342-349`, `owner.go:64-90` |
| `Attachment(reader, sha)` | `attachments.go:10-44` |
| `SearchKinds`, `SearchDocs`, `SearchShow` | `search.Source` for `msg:` refs (`search.go`). |
| `FencePrompt(token, reminder, msgs)`, `NormalizePR(s)` | `inbox.go:579`, `pr.go:22` |

`Options` (`inbox.go:340-378`):

- `DeviceID`, `Now`.
- `PROwners(prKey) []string`.
- `IsOwner(key)`. The default accepts exactly `"user:"+DeviceID`.
- `OnRecipientGone(m, to) (taskID, err)`.
- `OnPassed(m, to) err`.
- `Revoked(dev)`.
- `Stale(Delivery) note`. It runs under the inbox lock and must not call the inbox.
- Duration overrides `UnengagedLimit, GoneGrace, QuietLimit, ReattachGrace, NoCardGrace`. A value of 0 or less means the default (`state.go:256-276`).

`Roster` is `Agents() []presence.AgentCard`. The fields used are `Key`, `Name`, `Status` ("gone" is excluded), `Agent` ("grok"), `PermissionMode`, `RepoKey`.

Errors that callers branch on (`inbox.go:26-40`, `state.go:18-23`). The Display strings are user-visible tool output, so keep them exactly:

| Error | Text |
|---|---|
| ErrUnknownAgent | `inbox: no agent by that name or key` |
| ErrAmbiguous | `inbox: that name matches more than one agent` |
| ErrRateLimited | `inbox: too many messages between these two agents` |
| ErrNotYours | `inbox: that delivery isn't addressed to you` |
| ErrNotFound | `inbox: no such delivery or message` |
| ErrBadState | `inbox: not allowed in the delivery's current state` |
| ErrTooLong | `inbox: message text is too long` |
| ErrNoCard | `inbox: the message names no card here` |
| ErrSelf | `inbox: that's you; address another agent` |
| ErrEmpty | `inbox: give the message text` |
| ErrBadPR | `inbox: not a pull request address` |

Errors are wrapped as `"%w: detail"`, which gives `"<base>: <detail>"`. Rust needs an enum with a `kind()` plus the full formatted message.

---

## 2. Data formats

### 2.1 Log kinds (`inbox/kinds.go:8-30`, `mesh.go:26`)

| Kind | Table | Payload struct |
|---|---|---|
| `message.sent` | replicated | `Message` (`inbox.go:111-135`) |
| `delivery.state` | replicated | `deliveryRow` (`state.go:89-109`) |
| `wake.issued` | replicated | `wakeRow` (`state.go:121-131`) |
| `agent.alias` | replicated | `aliasRow {key, alias, at}` (`state.go:147-151`) |
| `agent.name` | replicated | `nameRow {key, name, at}` (`state.go:153-157`). The name is lowercased and trimmed. |
| `turn.engaged` / `turn.ended` | replicated | `turnRow {agent, wakeId, turnId, at}` (`state.go:165-170`) |
| `delivery.decided` | replicated | `decidedRow` (`mesh.go:35-46`) |
| `policy.set` | **local_events** (device-local, `store/kinds.go:117,145`) | `policyRow` = Policy fields flattened plus `by`, `at` (`state.go:159-163`) |
| `agent.seen` | replicated (read only) | `presence.AgentCard`; only `key` and `name` are used (`state.go:613-620`) |

Replay reads replicated `replayKinds` (`state.go:34-37`), then local `policy.set` (`state.go:40,309-333`). It pages by `store.Cursor` with `Limit: store.MaxListLimit` until a short page arrives.

### 2.2 JSON field order and omission (Go `encoding/json` emits struct order)

**Message** (`inbox.go:111-135`):

- Always present: `id`, `from{key,name,kind}`, `to{kind,value}`, `text`.
- `reason` (omitempty), `replyTo` (omitempty), `parentId` (omitempty).
- Always present: `hop` (an int, even 0), `trust` ("untrusted" or "user"), `sentAt` (a time, never omitted).
- `ref` (omitempty), `notice` (omitempty bool), `attachments` (omitempty array of `blob.FileRef{sha256,name,size,mime?,device}`, `blob/files.go:36-42`).
- The unexported `writer` is never serialized. It is set from `ev.DeviceID` at fold.

**deliveryRow** (`state.go:89-109`), in this order:

`deliveryId, messageId, to` (always present; `to` may be `""`), then `repoKey?, mode?, state` (always), then `hold?, via?, wakeId?, note?, for?, until?` (**omitzero**), then `movedTo?, movedFrom?, reminded?, engagedOnce?, appended?, by?, at?` (**omitzero**).

- `mode` is written only when non-empty. Every new row sets it, because `placed()` keeps `r.Mode` from the send (`messages.go:153`). A missing `mode` folds to `"wake"` (`state.go:471-473`).

**wakeRow** (`state.go:121-131`): `wakeId, to, token, reminder` (always, a bool), `deliveryIds` (always an array; it includes the overflow ids), `path?, mode?` (only `"next_turn"` is written, `wake.go:170-172`), `overflow?, at?`.

**decidedRow** (`mesh.go:35-46`): `deliveryId, choice` (`release`/`dismiss`/`move`), `to?, by, atDevice?, at?, undo?`. It is written with actor `user`.

**policyRow**: `mode, block, allow, paused, by, at?`. `block` and `allow` are always arrays, never null: `setPolicy` cleans them to `[]string{}` (`policy.go:152-161`).

**Time values.**

- `b.now()` defaults to `time.Now()`, the **local** zone. Go marshals it as RFC 3339 with nanoseconds, trailing zeros trimmed, and the local offset, for example `2026-10-29T10:11:12.1234567-05:00`, or `Z` only when the zone is UTC.
- The zero time is `0001-01-01T00:00:00Z`. It appears in API `Delivery` JSON because `until` is `omitempty` there (`inbox.go:212`), and omitempty never omits a struct. In `deliveryRow` the zero time is omitted (`omitzero`).
- When `at` is missing, readers fall back to `ev.HLC.Wall()` = `UnixMilli(hlc>>16)` (`state.go:407-412`, `hlc/hlc.go:58`).

**JSON escaping.** The store marshals payloads with `json.Marshal` (`store/sqlite.go:413,499`). Go's default escapes `<` `>` `&` as `\u003c` `\u003e` `\u0026`, and U+2028/U+2029 as `\u2028`/`\u2029`. It replaces invalid UTF-8 with U+FFFD. serde_json does **none** of this. Read old rows as raw bytes (never re-serialize them). For new rows, either match Go's escaping or confirm with the store spec that signatures cover the exact stored bytes, whatever they are. Message text often contains `<irori-message>`-like text and `&`.

### 2.3 IDs (`inbox/ids.go`)

- Prefixes: `msg_`, `dlv_` and `wake_` + a 26-character Crockford base32 ULID-like value (alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, `ids.go:11`). Sites: `messages.go:119,153`, `wake.go:80,118`, `sweep.go:528`.
- Layout: 48-bit ms (big-endian) + 80 random bits. The 128 bits are encoded as a 130-bit number with 2 leading zero bits (`ids.go:51-67`).
- Monotonic: within the same or an earlier millisecond, it keeps `lastMs` and increments the 10 random bytes as a big-endian counter. On a new millisecond it draws fresh random bytes, then sets `last[0] &= 0x7f` (`ids.go:22-48`).
- One generator is shared by msg, dlv and wake. The `ulid` crate's monotonic generator differs: it has no 0x7f mask and a different overflow rule. Hand-roll it.
- Sorting by id equals sorting by mint time. Code relies on this: `sortReady` (`wake.go:181-188`), `newestIn` (`messages.go:161-177`), `heldNote` newest (`watch.go:522-524`).

### 2.4 Tokens and the fence (`wake.go:16-21,393-484`)

**Tokens:**

- `[irori-wake:<wakeId>]`, `[irori-reminder:<wakeId>]`, `[irori-append:<wakeId>]`.
- Reminder and append ids also use the `wake_` prefix.
- Lookup is by `strings.TrimSpace(token)` (`wake.go:235`).

**Escaping:**

- `escapeText`: the regex `(?i)</(irori-message)` becomes `<\/$1`, then `(?i)\[(irori-(?:wake|reminder|append):)` becomes `[\$1`. The case of the matched text is kept.
- `escapeAttr` = `escapeText`, then replace `&`→`&amp;`, `<`→`&lt;`, `>`→`&gt;`, `"`→`&quot;` (`wake.go:393-405`).
- Go's `strings.NewReplacer` does a single pass. Order matters only for `&`, so do a one-pass replace in Rust.

**Prompt, byte for byte** (`wake.go:413-484`). Let N = len(msgs) + len(overflow), and plural = "" when N == 1, else "s".

1. Header, chosen by type:
   - reminder: `"{token} Irori reminder: {N} message{s} from other agents, delivered to you earlier through Irori, still have no reply from you. They are repeated below. "`
   - append (token starts with `[irori-append:`): `"{token} Irori added {N} message{s} for your next turn. No turn was started for them: take them into account in what you are doing now or next. "`
   - otherwise: `"{token} Irori delivered {N} message{s} from other agents. "`
2. The fixed trust paragraph (`wake.go:429`), ending in `\n`.
3. Conditional lines, in this order:
   - PR line, if any message has `to.kind=="pr"` (`:430-432`).
   - next_turn line, if not an append and any delivery has mode next_turn (`:433-435`).
   - ref line (`:436-438`).
   - notice line (`:439-441`).
   - attachment line (`:442-444`).
4. The "To answer..." line (`:445`).
5. Per message: `\n<irori-message from="" key="" delivery="" trust=""`. Then, in this order and only when present:
   - ` mode="next_turn"`
   - ` reply-to=""`
   - ` reason=""`
   - ` ref=""`
   - ` notice="true"`
   - ` pr=""` (pr kind only), followed by ` for=""` (pr kind only, if `d.For` is set).
6. Close the tag with `>\n`, then `escapeText(text)`, then `\n`.
7. Per attachment: `<irori-attachment sha256="" name="{blob.SafeName}" size="{n}"/>\n`.
8. `</irori-message>\n` closes each message.
9. Finally the overflow line: `"\n{k} more waiting: read them with the irori inbox tool: {escapeText(ids joined ", ")}\n"`, or `""` when there is no overflow (`wake.go:226-231`).

Copy every literal sentence verbatim from the cited lines. Mod and channel tests assert on substrings.

**Batching** (`wake.go:194-223`). Measure sizes in **bytes** (UTF-8 length).

- Grow n from 1 while `len(fence(msgs[:n+1])) <= 16384`. The first message always goes in, however big.
- `base = len(fence(full, nil)) - countLen(n)`, where countLen = the number of digits, +1 when n≠1.
- Add overflow ids one by one while `base + countLen(n+len(next)) + len(overflowLine(next)) <= 16384`.
- Anything beyond that stays queued.

---

## 3. Rules, constants, timings

### 3.1 Constants (`inbox.go:48-70`, `state.go:26,29`, `messages.go:21,24`, `search.go:29`)

| Name | Value | Name | Value |
|---|---|---|---|
| MaxText | 8000 runes | MaxWakeBytes | 16384 bytes |
| DirectLimit / DirectWindow | 8 per 10 min sliding window, per unordered pair | MaxHop | 6 (hold when `hop > 6`) |
| UnengagedLimit | 10 min | GoneGrace | 3 min |
| NoCardGrace | 10 min | QuietLimit | 60 min |
| MaxDefer | 12 h | ReattachGrace | 2 min |
| MaxAliases | 10 | MaxAliasLen | 60 runes |
| MinKeyPrefixLen | 8 | SweepInterval | 60 s |
| DefaultInboxList | 50 | MaxWakeTries | 3 |
| MaxNote | 500 runes | MaxRef | 200 runes |
| MaxAttachments | 10 | subBuffer | 64 |
| maxSearchDocs | 500 | SystemKey | `irori` (Sender `{irori, Irori, irori}`) |
| BypassMode | `bypassPermissions` | APIKeyPrefix | `api:` |
| NoteRecipientGone | `recipient gone` | | |

`clip(s, n)` trims, then cuts to n **runes**, with no ellipsis (`messages.go:12-18`). Text is trimmed before the length check and the rune count (`messages.go:56-62`).

### 3.2 Delivery states (`inbox.go:138-160`)

- States: `queued`, `held`, `delivered`, `engaged`, `answered`, `deferred`, `skipped`, `rejected`, `dismissed`, `moved`.
- Open states: queued, held, delivered, engaged, deferred.
- Hold reasons: `blocked`, `not_allowed`, `bypass_receiver`, `paused`, `hop_limit`, `unowned`.
- Via: `mod`, `channel`, `gui`.
- Modes: `wake`, `next_turn`.

**Fold of `delivery.state`** (`state.go:437-512`). This logic is load-bearing; port it line by line.

- **New id.** `CreatedAt = at`, `firstWriter = ev.DeviceID`, append the id to `order`, `assignedAt = at`. On the message's first delivery (`counted`), when `movedFrom == ""` and the rate counts, add `m.SentAt` to the pair's rate window. This rebuilds the rate limiter on replay.
- **wakeTries.** Reset to 0 when `state != queued` or `to` changed.
- **Decision.** When the previous state was Held or Dismissed and the state changes: `DecidedBy = r.by`, `DecidedAt = at`.
- **HeldFor.** When `state == held` and `hold != ""`: `HeldFor = hold`. HeldFor is never cleared.
- **appended** = `r.appended && state == delivered`.
- **Search visibility.** Keep a per-message first-visibility record. The first row Held or Dismissed sets `heldFirst`; any other first row sets `visibleAt = at`.
- **Change emitted:**
  - `held` if it became Held, whether new or from another state.
  - `moved` (To = prevTo) if Moved.
  - otherwise `updated`.

**Fold of `wake.issued`** (`state.go:514-549`). For each listed delivery:

- If reminder: set `Reminded = true`.
- Else, if the delivery is Queued: set `WakeID = r.wakeId`. If the wake gates and path ≠ channel, increment `wakeTries`.

Then a reminder clears `reminders[to]`.

**Other folds:**

- `turn.engaged` sets `running[agent]`.
- `turn.ended` clears it only when the wakeId matches.
- `agent.alias` → addAlias: lowercase and trim, drop any existing duplicate, append, keep the last 10.
- `agent.name` and `agent.seen` set `known[key] = lower(trim(name))`.

### 3.3 Writer rules (owner table, `owner.go:16-55`, `owner/owner.go:73-89`)

**DeviceOf(key):**

- `user:<dev>` → dev.
- Else the prefix before the first `/`, if it has no `:`.
- Else `""`.

**MayWrite(ownerKey, dev)** = `DeviceOf == ""` or `DeviceOf == dev`.

**`delivery.state` rows:**

- A first row for a remote recipient (`To` set, and the writer may not write it) is accepted only as a suggestion: `state == queued && hold == "" && wakeId == ""`.
- Any other first row is accepted.
- Later rows require `MayWrite(d.To, writer)`. When `To` is empty, the writer must be `firstWriter`.

**`wake.issued`** needs `MayWrite(to)`. **`turn.*`** needs `MayWrite(agent)`.

**`delivery.decided`** is accepted only with actor `"user"` and `IsOwner(r.by)`. Otherwise it is ignored and counted (`state.go:604-607`).

Rejected rows increment `ignored` and `ignoredBy[dev]`.

**actorFor** (`state.go:385-396`):

- `""` or `irori` → `irori`.
- An owner key → `user`.
- `api:` prefix → the key as is.
- Else `agent:<key>`.

### 3.4 Wake policy: hold order (`policy.go:99-125`)

1. If `recipient == ""`: return `unowned`.
2. If the sender is the system (key `irori`): return `blocked` only if the block list matches, strict.
3. Else, if the sender is not the person (kind `user` and IsOwner(key)), check in order:
   - **blocked**: block-listed, strict.
   - **not_allowed**: mode `allowlist` and not allow-listed.
   - **bypass_receiver**: all of the following hold:
     - mode ≠ `open`;
     - `bypass(recipient)`: the card exists, the card's agent ≠ grok, and permissionMode == `bypassPermissions`;
     - not allow-listed;
     - not `ownAgent`;
     - and at least one of: sender kind == grok, the sender is not the writer, or the sender is not bypass.
   - **hop_limit**: `m.Hop > 6`.
4. If paused: return `paused`. This applies to everyone, including the person.
5. Otherwise return `""` (queued).

**listed** (`policy.go:21-48`):

- An entry matches when it equals the key case-insensitively, or equals a candidate name in lowercase.
- Candidate names: the live card name, else `known[key]`.
- Strict matching (block) adds the self-claimed `from.Name` and all aliases.
- Allow never uses self-chosen names. This is a security rule.

**ownAgent** (decision 34, `policy.go:64-80`):

- The sender has kind `claude`, its key does not start with `user:`, and its card is not grok.
- `DeviceOf(key) == message writer`.
- And either the device is this device, or (`IsOwner("user:"+dev)` and not `Revoked(dev)`).

**Mode open** ("Full auto", decision 36) only skips the `bypass_receiver` hold.

**Who decides.** A recipient on another device gets `h = ""`: its host decides (`messages.go:149-152`). The host runs `hostHold` once per delivery when it folds a remote Queued row that has no hold or wake (`mesh.go:232-253`). The `hostChecked` flag lives in memory only.

**setPolicy** (`policy.go:144-190`):

- Mode `""` becomes everyone. An unknown mode is an error with the text `inbox: unknown policy mode %q (everyone, allowlist or open)`.
- Lists are trimmed and deduplicated with exact comparison.
- The row goes to `emitLocal`.
- Unpause re-evaluates every `held/paused` delivery in `order`, using `placed(row, hold(...))`.

**normPolicy**: any mode other than allowlist or open becomes everyone. Nil lists become `[]`.

### 3.5 Send (`messages.go:30-158`)

**Mode default:** `next_turn` when the sender is the person or the system, else `wake`. Any other mode is an error.

**Routing:**

| Address | Rule |
|---|---|
| agent | `resolveAgent` (`route.go:76-178`). |
| repo | `desk(repo, exclude=sender)`. An empty value gives the error `inbox: say which repo`. |
| reply | The original's sender. Replying to self is ErrSelf. Replying to the system is ErrBadState "that is Irori's own notice; it needs no reply". |
| pr | `routePR` (`pr.go:45-75`). |

**resolveAgent** (`route.go:76-178`). Candidates are live cards, plus `known`, plus alias keys that are not owners. Precedence:

1. Exact key, case-insensitive.
2. A unique prefix of at least 8 characters, matched against the key or its session part.
3. Card name, live before gone.
4. Alias, live before gone.

When only self matches, return ErrSelf. Several matches give ErrAmbiguous with the text `%q matches %d agents: name (key), ...; use a key`. No match gives `no agent called %q; agents here now: <sorted names or none>`.

**routePR** (`pr.go:45-75`):

1. Use the first live owner that is not the sender.
2. Else use the desk of repo `ledger.RepoKey(ref.Repo)`. `For` is set to `clip(describe(firstNonLiveOwner), 500)`.
3. Else the delivery is held unowned.

**Desk** (`route.go:226-253`). Candidates are live cards with `RepoKey == repo` that are reachable. Sort by `reach.firstAt`, then by key.

**Reachable** (`route.go:182-188`): mod, or channel, or the mod path was lost less than 2 min ago. `firstAt` resets when the agent goes from unreachable to reachable.

**Rate limit** (`messages.go:107-116`, `state.go:816-827`):

- Applies only when the sender is not the person and both ends count: non-empty, not an owner, not the system.
- Prune entries where `now - t >= 10m`.
- Refuse when 8 or more remain. The error text is `"%w: %d messages between you and %s in the last %d minutes; stop the back-and-forth or ask the person"`.
- The pair key is `min|max`.
- A send does **not** add itself to the window directly. The fold of its first delivery row does (`state.go:454-460`).

**Hops** (`messages.go:127-134`):

- If the sender has a running wake: parent = the newest message in that wake (`max id`), and `hop = parent.hop + 1`.
- Else, for a reply: the parent is the replied message.

**Alias** (`messages.go:137-143`). Not for the person or the system. Write an `agent.alias` row only if the normalized name differs from the last alias.

**Row order on send:**

1. `agent.name` rows from `liveCards` for any card whose name changed, sorted by key (`route.go:16-33`).
2. `agent.alias` (optional).
3. `message.sent`.
4. `delivery.state`.

`trust` is `user` only for the person.

### 3.6 Wakes, appends, ride-along, untaken

**gates(w)** (`state.go:145`) is true only for a wake that is not a reminder, not an append, and not a next_turn-only push.

**outstanding(agent)** (`state.go:781-805`), looking over open deliveries of the agent whose `wakeId` points to a gating wake:

- If any of them is Queued, return that wake with state Queued.
- Else return the earliest-issued Delivered one that is not `appended` and was updated less than `UnengagedLimit` ago.

**nextWake(agent, path)** (`wake.go:23-86`):

1. Return nil if paused, if the agent is not local, or if outstanding is non-nil. Before the outstanding check it runs `dropStale`.
2. Split Queued deliveries that have no wake into:
   - `wakeItems`;
   - `ride`: next_turn items, plus (on a non-channel path) **untaken** items, meaning `!nextTurn && wakeTries >= 3`.
3. Sort each by (messageId, id).
4. If there are wake items: send wakeItems + ride.
5. Else, on the channel path with ride items: send the ride items with mode next_turn.
6. Else send due reminders, then clear `reminders[agent]`.

**nextAppend** (`wake.go:92-120`):

1. Return nil if paused, not local, or `pendingAppend` is non-nil.
2. Run `dropStale`.
3. Take Queued deliveries with no wake where `queuedBehind || forAppend` holds.
4. `queuedBehind` is outstanding == Delivered (card #28): wake items ride an append behind a waiting prompt.
5. Log `"inbox: %s: %d wakes to %s went unconfirmed; sending it for the next turn instead"` for untaken items.
6. The path is always mod.

**markDelivered** (`wake.go:245-274`):

- Reminders are a no-op. `appended` counts only when the wake is not itself an append.
- Skip a delivery already in another wake.
- Otherwise it goes Delivered when it is Queued. A **late confirmation after a reclaim still counts**. The `appended` flag can be upgraded once on an already-Delivered item.
- Via is the wake's path, else `mod`.

**markEngaged** (`wake.go:276-315`):

- For Queued or Delivered deliveries in this wake: set Engaged.
- Overflow ids only get `engagedOnce` and are not closed as notices. The others go through `engaged()`: a notice closes immediately as Skipped with the note `notice` (`wake.go:376-382`).
- Then a `turn.engaged` row is written, unless any of these holds: the wake is an append, the call is `appended`, or the same `(wakeId, turnId)` is already running.

**turnEnded** ignores reminders and appends, and writes only for the running wake.

**hasSubmit and hasAppend** (`state.go:727-757`) drive the edge-triggered `wake_ready` and `append_ready` changes. They are computed in `unlock()` for every touched agent (`state.go:679-703`). Replay sets the baseline silently (`state.go:335-338`).

**dropStale** (`wake.go:126-154`). It takes local Queued or Held deliveries with no wakeId whose message has `Notice` and `Ref`. Each one where `Options.Stale(d) != ""` becomes Skipped with that note, by `irori`.

### 3.7 Sweeper (`sweep.go`)

**Tracking pass** (`sweep.go:193-238`), for local, non-owner recipients of open deliveries:

- `goneSince`: not live, or not reachable.
- `endedSince`: not live in presence, whatever the reachability.
- Entries for agents no longer seen are pruned.

**Plan per owned open delivery** (`planOne`, `sweep.go:83-164`). At most one action, first match wins:

1. **Held**: only `unowned` repo work → **assign** to `Desk(repo, exclude sender)`.
2. A state that is not open is skipped.
3. **gone move**. Let gr = repo, else the PR fallback repo. If gr is set and `GoneFor(to) > 3m`: move to `Desk(gr, exclude to, sender)`; an empty desk means unowned.
4. **ended agent work**. Requires: no gr, a To, not a notice, not from the system, and `EndedFor(to) > 3m`.
   - If the ref starts with `task:` or `comment:`: **goneCard**.
   - Else, if not from the person: **gonePerson** to `user:<self>`.
5. **reclaim**. In flight (Queued with a non-reminder wakeId) and `now - issuedAt > ReattachGrace`: clear wakeId.
6. **Deferred**: requeue when `now >= until`, else nothing.
7. **next_turn**: nothing more.
8. **Repo work** that was never engaged, Queued or Delivered, with `now - assignedAt > 10m`:
   - Move once (only if it was not itself moved) to another desk.
   - Else remind once, if Delivered, not reminded, not paused and reachable.
9. **Reminder**. State Engaged or Delivered, not reminded, not paused, reachable, and `now - updatedAt > quiet`. Quiet is 10m for Delivered-never-engaged, else 60m.

**Actions** (`sweep.go:286-328`). All are written by `irori`.

- `assign` re-runs `hold()` for the new recipient.
- `remind` adds to `reminders[to]` in memory only. The next NextWake issues a reminder token.

**move and moveWith** (`sweep.go:516-545`):

1. Write the new delivery row first, with `movedFrom` set, the mode copied, and `repoKey = prFallbackRepo`.
2. The hold is: `""` for a remote recipient, `unowned` when `to == ""`, else `hold()`.
3. Then the old row becomes Moved with `movedTo`.
4. For owner-routed PR work, `For = describe(old to)`.

**moveGone.** `For = "recipient gone: <name (key)>"` and the old row's note is `recipient gone`.

**handOff** (`sweep.go:351-371`) runs outside the lock. It calls `OnRecipientGone`:

- ok → `closeGone`: Skipped, note `recipient gone; on task:<id>`.
- `ErrNoCard` → `noCard`. Start a `noCardSince` clock; after more than 10 min, `moveGone` to the person. The person's own messages stay. With no hook, the move happens at once.
- Any other error: log and retry on a later sweep.
- `stillGone` re-checks under the lock (`sweep.go:377-386`).

**passed and fillCards** (`sweep.go:414-469`). When an owned delivery with a card ref (not a notice) goes from Held or Dismissed to Queued, or the host passes a remote row, queue `OnPassed`. Retry for up to NoCardGrace, then log and drop. This state is memory only.

At the end of the sweep every agent with open work is touched, so time-based gate openings emit `wake_ready` (`sweep.go:331-333`).

### 3.8 Mesh (`mesh.go`)

- **Ownership.** `owns(d)` is `local(d.To)`, or, when To is empty, the first writer is this device. Only the owner writes holds, wakes and sweeps.
- **Follow.** It subscribes to the store. `Seq == 0` is the resync signal and triggers `catchUp`. It ignores own-device rows and non-replay kinds. `applyRemote` skips any `seq <= last[dev]`.
- **catchUp.** If any `vector[dev] < last[dev]`, a revoke cut has happened: **refold**. Refold resets every row-derived map but keeps `handled`, reachability, gone clocks, reminders and subscribers (`mesh.go:155-170`). Then it pages each device after `last[dev]`, then runs `ownerPass`.
- **applyDecided** (`mesh.go:259-298`):
  - The first request per delivery wins (`handled`, memory only).
  - `undo:true` (a release asked by a device that saw the delivery dismissed) bypasses `handled`, but applies only while the delivery is Dismissed.
  - Release and dismiss apply only while Held. Move applies to an open delivery with a new `to`.
  - Requests folded during replay are queued in `requests` for the owner pass.
  - Device clocks are never compared.

### 3.9 Search visibility (`search.go`)

- A message is searchable once any delivery has had a first row that is not Held or Dismissed. `Updated` is SentAt, or the first non-held time when it was held first.
- `readers` (`search.go:270-289`): the agent sender plus the recipients of non-held deliveries, owners excluded. An empty list becomes `["irori"]`, so only the person can read it.
- `Title`: `"<from.Name> -> <recipient>[: reason]"`, folded onto one line. `render` format: `search.go:185-225`. `related`: the reply-to, then the Ref, then sorted replies, capped at `search.MaxRelated`.

### 3.10 Digest dedupe (`tools/notice.go:32-75`, `coord/cards_impl.go:1189-1221`)

**RouteNotice.** The mode is next_turn unless `n.Wake`. It skips an empty recipient and the sender. It sends with `Ref`, `Notice: true`, `Reason: kind` and the attachments.

**Merge.** For kind `watched` in next_turn mode, look at `For(to, open)` for a Queued, next_turn notice with the same ref and the same sender. When `MergeWatch(old, new)` succeeds:

1. Send the merged text.
2. Then `Skip(old, "merged into a newer update on the card")`.

**MergeWatch** markers:

- head `Update on `
- head end `, a card you watch or raised a blocker on:\n`
- foot `\nNow: `
- the earlier-changes line `- (%d earlier changes)\n`
- at most 8 lines (the rest are counted)

### 3.11 Notifier (`notify/notify.go`)

**Levels.** `info|success|warn|error`. ParseLevel lowercases and trims; `""` means info.

**Card ids** (`notify.go:416-424`): `held:<senderKey>`, `needs:<taskId>`, `msg:<deliveryId>`, `ask:<id>`, `unclaimed:<taskId>`, `returned:<taskId>`, `test`.

**Post** (`notify.go:307-362`):

- Refuses an empty id, empty title+body, headless, or disabled.
- A generation counter makes only the live card's callbacks run, at most once. An outcome with no callback still `take`s the card.
- When `OnExpire` and `Expire > 0`, the Notifier owns the timer: the toast is sticky, and when the timer fires it closes the card and runs OnExpire.
- At most 2 actions.

**Clear** closes the card without callbacks. **Live** comes from the Notifier map. A card that go-toast expired on its own stays "live" until the next Post or Clear of that id.

**Config** (`notify.go:203-222`): AppName `Irori`, **Palette Dark, CornerRadius 0** (Toru look). CaptureExclude is true unless `CaptureVisible` is set or `IRORI_TOAST_CAPTURE=1`.

**settings.json** (`notify/settings.go`):

- The top-level object keeps unknown keys. Key `notifications` = `{"enabled","sound","volume","corner"}`.
- Defaults are `{true, true, 0.6, "top-right"}` (`go-toast/note.go` DefaultOptions).
- Load decodes over the defaults, so missing fields keep their defaults. A decode error means defaults. Then it normalizes: volume NaN → 0.6, clamp 0..1; an unknown corner → top-right.
- Save:
  - `MarshalIndent(all, "", "  ")` + `\n`. Go sorts map keys, so top-level keys come out **alphabetical**.
  - The notifications value is compact `json.Marshal`, re-indented by MarshalIndent.
  - Write to a temp file named `settings.json.*` in the folder, then rename. The folder is created with 0700.
  - A broken file is replaced.
  - Writes are serialized by a process mutex shared with the other settings owners. Check that other areas write the same file the same way.
- SetOptions normalizes, applies, saves, and calls the `OnOptions` listeners. Disabling makes go-toast clear the cards.

### 3.12 Watch (`notify/watch.go`)

**Held cards** (`watch.go:79-110,197-592`):

- One card per **sender key** (`held:<key>`). `paused` holds never get a card (`cardHeld`, `:377`).
- Title, by the number of held messages:
  - one: `"{from} sent {to} a message Irori held"`;
  - several with mixed recipients: `"{from} sent {n} messages Irori held"`;
  - several with the same recipient: `"{from} sent {to} {n} messages Irori held"`.
- `to` is `"repo <key>"` when unowned, `"you"` when it equals User, else the display name.
- Body: `Upper(reason words) + ": " + preview(newest text)`, level warn. After a failed press: `"Couldn't {verb} {that message|those messages}: {err}. " + body`, level error.
- Buttons: Release / Dismiss, or Release all / Dismiss all. Sticky. A body click opens `{tab: held, ref: senderKey}`.
- Only a card going **up** for a new hold makes a sound. An in-place update is silent (`flush`, `:279-297`; `heldChange` updateIfUp < postQuiet < postLoud).
- Closing with the x leaves the messages held and marks them `closed`. The card comes back **quietly** on the next reconcile.
- A press runs on the card's own goroutine and reports to the loop through a size-16 channel (`failed`). An error counts only if the delivery is still Held.

**Reason words** (`watch.go:487-494`):

| Hold | Words |
|---|---|
| blocked | "the sender is on your block list" |
| not_allowed | "the sender isn't on your allow list" |
| bypass_receiver | "the recipient runs without permission prompts" |
| hop_limit | "it is too many hops down a chain of agent messages" |
| unowned | "no agent in that repo can take it yet" |
| paused | "wakes are paused" |

**Message to the person** (Queued or Delivered, `IsOwner(to)`): `msg:<did>`, title `"{from} sent you a message"`, an Open button, body click opens `{inbox, did}`. Cleared once it is not waiting.

**Needs you:**

- Posted only when an agent (not the person) moved the card: `UpdatedBy` is not the owner and the card has a non-owner holder or creator (`:760-779`).
- Title, by case: `"{agent} is blocked on you"`, `"needs your input"`, `"handed you a card"`, `"needs you"`. Then ` (#short)`.
- Buttons Answer / Later. Sticky.
- Seeded quietly at start.

**Unclaimed ask** (`:690-755`):

- Re-posted when `(since, reason)` changes.
- Buttons "Wake them" (`Room.AskAgain`) and "Close" (`SetTaskState Canceled "closed by the person"`).
- Body variants: `:724-732`.

**Returned card** (`:640-686`):

- Keyed by `Returned.Row`, else `At.String()`.
- Title `Nobody is on “{title}” (#short)`.

**Asks** (`:326-365`):

- Card `ask:<id>`. Expire = time until `Until`, with OnDismiss answering `""`. Asks are cleared when resolved.
- Waiting asks get their card at start.

**Reconcile.** It re-reads `Inbox.Open()` whenever the inbox channel drains (`len(ich)==0`) and at start with post=true. Waiting holds and messages get their cards at start. The Board seeds quietly.

**preview** (`:824-832`): fold whitespace, then keep at most 160 runes (159 + `…`).

**upper** uppercases the first rune.

### 3.13 go-toast behaviour Irori depends on (`go-toast/note.go`, `layout.go`, `sound.go`)

**Lifetimes** (`defaultLifetime`, `note.go` ~L147-161):

| Card | Lifetime |
|---|---|
| sticky | 0 (stays) |
| `Expire` set | Expire |
| has actions | 10 s |
| warn or error | 6 s |
| other | 3 s |

**Stack:**

- At most 5 cards. A sixth drops the oldest non-sticky card, else the oldest.
- A same-id Post replaces the card in place and restarts its timer.
- Hovering pauses expiry. After the cursor leaves, each card keeps at least 1.5 s.
- Fade in 140 ms, fade out 320 ms, alpha 245.
- `clean`: when the title is empty, the body becomes the title. Invalid UTF-8 becomes U+FFFD.

**Geometry (dip)** (`layout.go:6-36`):

| Item | Value |
|---|---|
| card width | 340 |
| gap / margin | 8 / 20 |
| stripe | 3 |
| icon | left 15, size 15 |
| text | left 40, right 14 |
| padding | top 11, bottom 12 |
| title | 21 px row, Segoe UI Semibold 14 |
| body | 2 lines of 17 px, Segoe UI 12.5 |
| close box | 24, inset 5 |
| buttons | 26 high, padding 12, minimum 64, spacing 8, 10 above the row |

- Height = 11 + 21 + 12, plus (1 + 17×lines) when there is a body, plus 36 when there are buttons.
- The newest card is nearest the corner (`stackOrigins`).

**Dark palette** (`go-toast/config.go:123-135`):

| Slot | Colour |
|---|---|
| background | #111213 |
| foreground | #fafafa |
| muted | #9ea1a7 |
| border | #2c2e31 |
| accent info / success | #4cabfd |
| accent warn | #f5ae39 |
| accent error | #ea3c3f |
| faint | #5c5f66 |
| hover | #26292e |
| button | #202225 |
| primary | #4cabfd |
| primary hover | #6fbcff |
| on primary | #060a13 |

**Sounds** (`sound.go` soundFor):

| Card | Sound |
|---|---|
| has buttons, or warn | `published.wav` |
| error | `fail.wav` |
| else | `success.wav` |

The 16-bit PCM samples are scaled by volume, which is rounded to 0.01. The WAVs are embedded and MIT (Facepunch); keep `sounds/NOTICE.md`.

**Input.** The x, a right click or (on macOS) a Control-click dismisses. A body click runs OnClick, else counts as a dismiss. Callbacks run on their own thread after the card closes.

---

## 4. Platform-specific code

| Area | Windows | macOS | Other |
|---|---|---|---|
| Keep awake | `SetThreadExecutionState(ES_CONTINUOUS\|ES_SYSTEM_REQUIRED)` = `0x80000001` (no display flag). The state belongs to the **thread**, so each hold owns a dedicated OS thread that sets it, blocks until release, then calls `SetThreadExecutionState(ES_CONTINUOUS)` on the same thread. A return value of 0 is an error; ERROR_SUCCESS is mapped to ERROR_INVALID_PARAMETER (`awake/awake_windows.go:14-58`). Release is idempotent and waits for the thread to finish. | `IOPMAssertionCreateWithName("PreventUserIdleSystemSleep", kIOPMAssertionLevelOn, "Irori home server")`; release with `IOPMAssertionRelease`, once (`awake/awake_darwin.go:11-42`). A nonzero IOReturn gives the error `IOPMAssertionCreateWithName: 0x%x`. Without cgo it is a no-op. | No-op hold (`awake_other.go`). |
| Toasts | go-toast Win32/GDI: layered, topmost, `WS_EX_NOACTIVATE` tool-window popups that answer `WM_MOUSEACTIVATE` with no-activate. One locked thread owns the windows. `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`. Sound via `PlaySoundW(SND_ASYNC\|SND_NODEFAULT\|SND_MEMORY)`. Class names `IroriToast`/`IroriToastHost`. Tick via `SetTimer`; hover via `TrackMouseEvent` (`go-toast/toast_windows.go`, `win32_windows.go`). | Non-activating AppKit `NSPanel`s at status level on all Spaces, placed on the screen with the mouse (`visibleFrame`), `NSWindowSharingNone`, SF Symbol icons, AVAudioPlayer. All work goes through the main dispatch queue (`go-toast/toast_darwin.m`). | No-op. |

**These are not OS notification-center toasts.** They are custom-drawn windows. `tauri-plugin-notification` would change both the look and the behaviour: no 2-button stack, no in-place replace, no sticky cards, focus behaviour differs. See §7.

---

## 5. Rust crates (crates.io, checked 2026-10-29)

| Need | Crate (version) | Notes |
|---|---|---|
| JSON | `serde 1.0.229`, `serde_json 1.0.154` (feature `preserve_order` for the settings round trip, or sort the keys yourself) | You need a Go-compatible writer: HTML escaping, and `omitempty` versus `omitzero` (§2.2). |
| Times | `time 0.3.55` (feature `formatting`, `parsing`, `local-offset`) or `chrono 0.4.45` | Go RFC 3339 nanoseconds with trimmed zeros, in the local offset. |
| Regex | `regex 1.13.1` | `(?i)` patterns from `wake.go:394-395`. |
| Random | `getrandom 0.4.3` | For the id generator. Don't use the `ulid 3.0.0` monotonic mode. |
| Async / channels | `tokio 1.54.1` (`sync::broadcast` is wrong here: use per-subscriber `mpsc::channel(64)` with `try_send`) | Keep the drop-on-full semantics. |
| Locks | `parking_lot 0.13.0` or std `Mutex` | |
| Win32 toasts and awake | `windows 0.62.2` (features `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`, `Win32_System_Power`, `Win32_Media_Audio`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_Graphics_Dwm`) or `windows-sys 0.61.2` | |
| macOS toasts | `objc2 0.6.5`, `objc2-app-kit 0.3.2`, `objc2-foundation 0.3.2` (+ `objc2-av-foundation` for sound) | Dispatch to main via Tauri's `run_on_main_thread` or `dispatch2`. |
| macOS awake | `objc2-io-kit 0.3.2` (if it exposes `IOPMAssertionCreateWithName`), else a 3-line `extern "C"` with `#[link(name="IOKit", kind="framework")]` + `core-foundation 0.10.1` for `CFString` | |
| Off-the-shelf awake | `keepawake 0.6.1` | Optional. Check that it uses only `idle`/system-required (no display) and that the Windows hold isn't set from a pooled thread; a hand-written version is safer. |
| Atomic settings write | `tempfile 3.27.0` (`NamedTempFile::new_in(dir)` + `persist`) | Pattern `settings.json.*`. |
| Do NOT use for cards | `tauri-plugin-notification 2.8.0` | It uses OS notification centers; see §4. |

---

## 6. Existing tests (port each one; names from `grep ^func Test`)

**inbox** (the harness is in `inbox/helpers_test.go`: fake roster and clock, keys `dev1/aaaa…`, owner `user:dev1`)

| File | Tests and what they prove |
|---|---|
| `fence_test.go` | `TestFenceInjection`: close tags and token-likes are neutralized case-insensitively, attributes are escaped, there is one token at the start, and trust can't be forged. `TestFenceReminderAndExtras`: the reminder header, `reply-to` and `reason` escaping, determinism. `TestFenceAppendAndAttributes`: append header, mode, ref, notice, the overflow line, and the ride-along sentence only on wakes. `TestBatchCap`: the 16 KiB cap holds for counts 2 to 30 and sizes 200 to 9000 even when the header count gains a digit. |
| `inbox_test.go` | `SendWakeEngageReply` (the full lifecycle), `TurnEndWithoutReplyStaysEngaged`, `Routing` (key, prefix, name, alias precedence; live before gone), `Aliases`, `TextLimits`, `RateLimit` (8 per 10 min, both directions), `Hops`, `ReplyHopWithoutRunningTurn`, `ReplyErrors`, `AckDeferSkip`, `ForAndList`, `IDsSortAndDiffer`, `ConcurrentUse`, `ErrorsWrap`. |
| `policy_test.go` | `PolicyHolds`: table cases covering the full hold order, including decision 34 (own agents) and 36 (open). `PolicyAliasBlockAndValidation`, `ReleaseDismissAndUnpause`, `FullAutoKeepsTheHopLimit`. |
| `review_test.go` | Security regressions: the allow list ignores self-chosen names, an alias can't hijack a card name, held messages are not shown to the recipient, repo work is never assigned to its sender, a Delivered-never-engaged delivery is reminded, Defer clamps huge minutes, a rename survives replay. |
| `wake_test.go` | Batching order and size; overflow capped and rebuilt identically; nothing to wake; reclaim of an unconfirmed wake; a late confirm after a reclaim counts; a stale Delivered wake stops gating; reminder wake; unreachable is not reminded; a deferral comes due; `Subscribe` edges (WakeReady and AppendReady fire once per edge); TurnEnded frees the gate; a wake item rides an append behind a delivered wake; a wake item waits once the delivered wake goes stale. |
| `nextturn_test.go` | Mode defaults by sender; system sender; the append flow (token, deliver, engage, no running wake); append after delivered and its edges; appends while paused or held; next_turn on channel; a notice closes on engage; the next_turn sweep exemptions; append reclaim; append overflow; **`TestWakeIssuedRowFields`** (exact row fields, so port this one for byte-compatibility); a channel next_turn push does not gate; an overflow notice stays open. |
| `untaken_test.go` | After 3 unconfirmed mod wakes the delivery falls back to an append; an appended wake engages without a hop; an appended wake does not gate the next; an untaken item rides the next wake; wakeTries restart when requeued; channel pushes keep repeating; a wake landing mid-turn. |
| `sweep_test.go` | `TestPlanSweep` (a pure table over `planOne`), the repo front desk, unengaged repo work moves once, gone holder, gone to card, gone to person (with the NoCardGrace clock), `OnPassed` fills the card, Ended ignores reachability, a lost attach counts as gone, a held delivery is assigned with policy on arrival. |
| `pr_test.go` | `NormalizePR` forms; owner, desk or unowned routing; invalid targets; unowned then desk; a gone desk keeps the note; owner-routed work stays put; policy and rate apply to PR work; the wake names the pr and for attributes; a gone owner moves to the desk; defer keeps the owner. |
| `mesh_test.go` | The remote host wakes and sweeps; repo work is assigned only by its writer; the host holds and applies the first request; a reply inherits the ref; the host fills the card when it lets a message through; the host lets the owner's other-device agents through bypass. |
| `held_test.go` | Release undoes a Dismiss; HeldHistory ordering; a remote release undo; a remote release from before the dismiss loses; undo applies only when the delivery was seen dismissed; releasing dismissed repo work with no desk errors. |
| `owner_test.go`, `owner_rule_test.go` | Owner keys versus made-up `user:` keys (actor, trust, rate); the policy is device-local (`local_events`); owner-only rows from the wrong device are ignored and counted; a made-up user key is an agent in search. |
| `replay_test.go` | **`TestReplayReproducesState`**: a live run and a fresh `New` over the same store give equal state. `TestReplayPagesPastListCap`: paging. These are the key fixtures for reading Go-written logs: run them against a copy of a real DB. |
| `search_test.go`, `stale_test.go` | Search visibility, doc fields, show render, readers; stale notices skipped before a wake. |

**notify** (a fake Toaster records the config, options, posts and closes)

- `notify_test.go`:
  - `ConfigureAndHeadless` asserts AppName Irori, radius 0, CaptureExclude, `Palette == Dark`, the env override, and that headless and nil never touch the toaster.
  - `IDsNeverStack`, `ClearRunsNoCallbacks`, `ExpireOwnedByNotifier`.
  - `OptionsPersist`: unknown settings keys are kept, the file is normalized.
  - `Levels`.
- `watch_test.go`: Release acts as the person; held cleared when resolved elsewhere; coalescing per sender; paused holds stay quiet; opens the held list; message to the person; board needs-you; seeds quietly; headless returns; catches up after dropped changes; ignores the person's own board moves; held card comes back after a dismiss; posts waiting messages at start; a release failure keeps the card; unclaimed ask; returned card; unclaimed ask woken within one tick; returned again within one tick.
- `ask_test.go` (`AskCardsAnswerAndClear`), `owner_test.go` (`WatcherOwnerSet`: IsOwner across devices).
- `race_on/off_test.go`: only a build-tag constant.

**awake** (Windows only)

- `TestRealHold`: `CallNtPowerInformation(SystemExecutionState=16)` shows ES_SYSTEM_REQUIRED while held. `powercfg /requests` lists the exe when elevated.
- `TestTwoHolds`: overlapping holds; releasing one keeps the other.

**go-toast** has its own `note_test.go`, `config_test.go` and opt-in smoke tests (`GO_TOAST_SMOKE=1`). Port them along with the card engine.

---

## 7. Hard parts and compatibility traps

1. **Go JSON fidelity.** HTML escaping and U+2028; `omitempty` on `time.Time` never omits (API `Delivery.until` = `"0001-01-01T00:00:00Z"`), while `omitzero` in rows does; `hop` and `reminder` are always present; `block` and `allow` are never null; struct field order; Go's local-offset nanosecond times. Never re-encode a stored row.
2. **The API `Delivery` JSON** (`inbox.go:200-226`) uses `reminded` without omitempty, `decidedAt` with omitzero, and `until` with omitempty, which differs from the row. The frontend and the MCP tools read it.
3. **Fold, not snapshot.** `HeldFor`, `DecidedBy/At`, `wakeTries`, `appended`, `assignedAt`, the rate windows, visibility and `counted` are all derived during fold. Replay order is the store cursor order (HLC, device, seq), with local `policy.set` afterwards.
4. **Memory-only state that must not be persisted:** reachability, goneSince/endedSince, noCardSince, reminders, passing, `hostChecked`, `handled`, the submit/appendable edge baselines. A restart resets them by design. Transports re-announce themselves.
5. **Edge-triggered changes.** WakeReady and AppendReady fire in `unlock()` only on a false→true edge for touched agents. Replay never notifies. The mod and channel transports depend on this exact behaviour.
6. **Lock discipline.** `OnRecipientGone`, `OnPassed` and coord writes run **outside** the inbox lock (handOff, fillCards). `Options.Stale` runs **inside** it. Reproducing this avoids a deadlock with coord.
7. **Move writes the new row before closing the old one**, so a crash duplicates work rather than dropping it. Card writes come before `closeGone`.
8. **ID generator.** It is shared across prefixes, with the 0x7f mask and big-endian counter increment, and must keep sorting with old Go-minted ids (same alphabet and width).
9. **Late markDelivered after a reclaim** still moves the delivery to Delivered. `MarkAppendedDelivered` can upgrade an already-Delivered item once.
10. **Size units.** The prompt is limited in bytes (16384); text, notes, refs and aliases in **runes** (Unicode scalar values, which is `chars()` in Rust, not graphemes).
11. **Escaping is case-preserving,** and the regex is applied before attribute entity escaping.
12. **Two notification stacks.** go-toast is a custom window engine on two platforms. Rebuilding it in Rust (Win32 layered windows; AppKit panels on the main thread shared with Tauri's event loop) is the largest piece of this area. A Tauri transparent webview window per card is an alternative, but it needs no-activate, capture-exclude and topmost-over-games behaviour verified on both OSes. Do not substitute OS notification centers.
13. **Doc comment contradicts code.** `notify.go:200-202` says "Light palette, 10 dip corners". The code and test say **Dark, radius 0** (`notify.go:215-217`, `notify_test.go:128`). Follow the code.
14. **settings.json is shared** with other settings owners. Unknown keys must round-trip, the file is written with sorted keys and 2-space indent plus a newline, and writes are atomic.
15. **Windows execution state is per thread.** A tokio task migrating between workers would silently drop the hold. Use `std::thread::spawn` per hold.
16. **macOS main thread.** Card creation must happen on the AppKit main thread that Tauri runs. Never start a second NSApp loop (the go-toast README warns against `Main` inside Wails/Cocoa apps).
17. **Mesh refold.** When the store vector drops below the folded `last[dev]` (a revoke cut), rebuild everything except `handled` and the memory-only maps.

### Could not determine from this area alone

- The exact signed-bytes contract (whether the signature covers the stored payload bytes as is). See the store spec. It decides whether new Rust rows must reproduce Go's JSON escaping.
- Whether `objc2-io-kit 0.3.2` exposes `IOPMAssertionCreateWithName` (not checked on docs.rs), and whether `keepawake 0.6.1` keeps the Windows hold on a dedicated thread.
- go-toast Win32 drawing details beyond geometry and palette (font fallback, DPI-change handling) were not read line by line; `go-toast/toast_windows.go` (803 lines) and `toast_darwin.m` are the reference.
