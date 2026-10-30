# Port spec: coord, ask, decision, roster, names, devcolor

Source (read-only): `../irori-go/internal/{coord,ask,decision,roster,names,devcolor}`.
All `file:line` refs are relative to `internal/`. Go 1.26 semantics.

Two different things are called "ask"; this spec keeps them apart everywhere:

- **Approval** (`internal/ask`, kinds `ask.posted|ask.answered|ask.resolved|revive.requested`): a question with buttons to the person (the notify tool) and the revive prompt. Type `ask.Ask`.
- **Room ask** (`internal/coord/ask.go`, kinds `take.requested|take.state|task.edited` + an `ask` field on `task.created`): a Board card the person posts to every free agent; one agent takes it. Type `coord.Ask` (on `Task.Ask`).

---

## 1. What the area does, and its interface

| Package | Role | Main consumers |
|---|---|---|
| coord | The Board: a fold over the replicated log of `task.*`, `lease.*`, `blocker.*`, `take.*` rows (mesh.go:75). Tasks, holders, assignees, sharing, blocked state with blocker items, steps, watchers, comments (incl. mirrored inbox messages), links, pins, subtasks + rollup, short refs `#12`/`#12a`/`mlg#12`, room asks with a take lock, advisory leases with cross-device conflict rules, refusals, sweeper (gone holders, resolved blockers, watch digests), search docs/show text. Also runs over a shared board's log (`Options.Board`). | `internal/core` (board.go, boards.go, boards_own.go, core.go, notice.go), `internal/tools` (board.go, cardthread.go, explore.go, messages.go, notice.go, observe.go, room.go, route.go, set.go, tools.go), `internal/api` (api.go, board.go, board_overview.go, client_board.go, routes_board.go), `internal/notify/watch.go`, root `board_service.go`, `boards_service.go`, `cards_service.go`, `inbox_service.go`, `board_route.go` |
| ask | Approvals fold + the asking device's resolve duty + revive-across-devices. | core |
| decision | Decision records with code anchors re-checked on read; pure fold `View` (capture service appends `decision.recorded`, capture/capture.go:40). | internal/capture |
| roster | Mesh roster: other devices' agent cards folded from `device.started`/`agent.*` rows, with liveness. | core (`MeshAgents`), inbox routing, GUI |
| names | Friendly session name from a 127-word list. | core/names.go:132 |
| devcolor | Per-device colour defaults + replicated `device.color` picks. | core/colors.go:45-57, core/core.go:179 |

### 1.1 coord public API (Go names; keep semantics)

Construction/feeds: `New(ctx, store, roster{Agents()}, Options)` (coord.go:249) replays the log; `Follow(ctx)` (mesh.go:73) folds other devices' rows + 1 s tick of holder duties; `Subscribe(ctx) -> chan Change` (coord.go:325); `SweepNotices(ctx) -> []Notice` (board.go:379; Core calls every few seconds); `SweepAsks(ctx, agents) -> []Notice` (ask.go:863).

Tasks: `CreateTask`, `ListTasks(TaskFilter)`, `GetTask`, `ClaimTask`, `AssignTask` (v0.2 rule), `Assign` (wide rule + Notice), `SetTaskState`, `Reopen`, `Join`, `Leave`, `SetShared`, `Block`, `Unblock`, `Withdraw` (board only), `Split`, `TaskRef(ref)`, `IsPerson(key)`, `IgnoredRows()`, `IgnoredBy()`, `AsideDropped()`, `Unresolved()`, `IsBoard()`.
Cards: `Raise`, `Recommend`, `Answer(BlockerAnswer)`, `CommentOn(CommentInput)`, `SetSteps`, `SetStep`, `Watch`, `Unwatch`, `Activity(id)`, `Pin`, `MovePin`.
Comments: `AddComment`, `AddCommentWith(files)`, `PersonComment(around)`, `RecordMessage(MessageEntry)`, `Comments(taskID)`, `GetComment`, `CommentAttachment(sha)`.
Links: `AddLink`, `RemoveLink`. Leases: `Acquire`, `Release`, `StealLease`, `Leases()`, `Refusals()`, `Sweep`.
Room asks: `PostAsk`, `AskClaim(wait)`, `AskClose(AskOutcome)`, `AskAgain`, `StaleNotice(ref,to)` (inbox Stale hook; must not take coord's lock).
Views: `Board(BoardFilter) BoardView`, search.Source (`SearchKinds`, `SearchDocs`, `SearchShow`).
Free functions: `IsTerminal`, `ValidState`, `EffectiveState`, `Overlaps`, `ParseResource`, `NormalizeResource`, `NormalizeGlob`, `ParseLink`, `LinkLabel`, `PRLink`, `ParseShort`, `ParseRef`, `AskTitle`, `MergeWatch`, `Task.ShortRef/Refs/Progress/OpenBlockers/AskLine`.

coord never imports inbox: every call someone should hear about returns `Notice`s and the caller routes them (board.go:127-136): `inbox.SendWith(sender, ToAgent(to), n.Text, {Mode: next_turn unless n.Wake, Ref: n.Ref, Notice: true})`; `Person` notices go from the person, sweep notices from `inbox.SystemSender`. People (`user:*`) are never in `To`.

### 1.2 Options (coord.go:209-241)
`Now`, `GoneGrace`/`HolderGrace` (test overrides; <=0 → defaults), `IsOwner(key)` (nil → `key == "user:"+store.DeviceID()`; on a board nil → `key == Options.Person`, boardlog.go:156-161 + impl.go:323-326), `Revoked(device)`, `Board`, `Person`, `KeyPerson(key)`, `DevicePerson(device)`, `Tag()`, `BoardOwner(personID)`.

### 1.3 Key formats (owner/owner.go:73-81)
- Agent key `"<deviceId>/<sessionId>"`; person key on the private log `"user:<deviceId>"`; Irori itself `"irori"` (`sweeper`, board_impl.go:21); loopback API `"api:..."`; board keys `"person:<personId>"` and `"agent:<cardId>"` (store.PersonActorPrefix = `"person:"`, store/board.go:117).
- `DeviceOf(key)`: strip `"user:"` → device; else the part before the first `/` if non-empty and containing no `:`; else `""` (so `remote:host/x` names no device).
- `MayWrite(ownerKey, writer) = DeviceOf(ownerKey)=="" || DeviceOf(ownerKey)==writer`.
- Store actor for a caller (impl.go:960-972): `"irori"`→`"irori"`; on a board → the key itself; `IsOwner(by)` → `"user"`; prefix `api:` → as is; else `"agent:"+by`.

---

## 2. Data formats

### 2.1 Envelope and byte compatibility
`store.Event{DeviceID, Seq (1-based gapless per device), HLC uint64, Actor, Kind, Payload json.RawMessage, Sig}` (store/store.go:35-45). HLC = unix ms << 16 | 16-bit logical (hlc/hlc.go:1-2,33,58; `Wall()` = `UnixMilli(hlc>>16)`). Log order everywhere = `(hlc, deviceId, seq)` ascending, string compare on device (short.go:41-49, cards_impl.go:1094-1102).

The signature covers `sha256(Payload)` verbatim (store/store.go:297-311). So:
- **Never re-marshal a row read from the log.** Keep the raw bytes; decode into structs only to fold.
- New rows written by Rust need not match Go's bytes, but Go peers must read them: field names exact, same omit rules.
- Go `json.Marshal` (store/sqlite.go:413,499) escapes `<` `>` `&` U+2028 U+2029 as `\u003c` `\u003e` `\u0026` `\u2028` `\u2029`; emits struct fields in declaration order; map keys sorted; nil slice → `null`; float64 `1` → `1`, `1.5` → `1.5`; `time.Time` → RFC3339Nano UTC with trailing fraction zeros trimmed. Vectors (Go 1.26, verified): `{"at":"2026-10-26T12:00:00.12Z","rank":1,"s":"a\u003cb\u003e\u0026\u2028","n":null}` and `{"at":"2026-01-02T03:04:05.0012345Z","rank":1.5}`. Zero time = `"0001-01-01T00:00:00Z"`.
- Go `json.Unmarshal` matches keys case-insensitively, ignores unknown fields, accepts `null` for slices/strings/pointers, and a non-object payload fails the whole decode (the fold then drops the row). Rust: `#[serde(default)]` on every field, `Option`/null-tolerant `Vec` (`deserialize_with` null→empty), no `deny_unknown_fields`; add `alias`es only if a real row needs them (none known).
- Times: `stamp()` = `now().UTC().Round(0)` (impl.go:974): full ns precision, UTC. Windows clocks give 100 ns (7 digits). In folds, `at(t)`: a zero `at` falls back to `ev.HLC.Wall().UTC()` (impl.go:456-461).

### 2.2 coord row payloads (`omitempty` = omitted when zero/empty/nil/false; none = always written)

All carry `"at"` (always written). JSON keys in Go declaration order:

| Kind | Payload fields (order) | Source |
|---|---|---|
| `task.created` | `taskId`, `num`(omitempty int), `title`, `objective`, `doneCriteria`(oe), `repoKey`(oe), `scope`(oe []string), `shared`(oe), `links`(oe []Link), `ask`(oe {`wake`(oe),`repoKey`(oe)}), `parent`(oe), `by`, `at` | impl.go:65-83, ask.go:135-138 |
| `task.claimed` | `taskId`,`by`,`at` | impl.go:85-89 |
| `task.assigned` | `taskId`,`to`,`by`,`at` | impl.go:91-96 |
| `task.status` | `taskId`,`state`,`note`(oe),`reason`(oe),`blockedBy`(oe {kind,ref(oe)}),`blocker`(oe blockerRow),`outcome`(oe {kind,task(oe),note}),`withdrawn`(oe),`by`,`at` | impl.go:98-115, ask.go:98-102 |
| blockerRow (in task.status) | `id`,`question`,`context`(oe),`options`(oe []Option),`reasonOnly`(oe),`replace`(oe),`on`(oe BlockedBy),`links`(oe),`attachments`(oe []FileRef) | cards_impl.go:32-48 |
| `task.joined` / `task.left` | `taskId`,`who`,`by`,`at`,`reason`(oe; `"gone"`),`person`(oe) | impl.go:118-127 |
| `task.shared` | `taskId`,`shared`(always),`by`,`at` | impl.go:129-134 |
| `task.linked` | `taskId`,`link`{kind,ref,label},`removed`(oe),`by`,`at` | impl.go:136-142, links.go:244-248 |
| `task.comment` | `taskId`,`commentId`,`by`,`text`,`wake`(always),`at`,`attachments`(oe),`to`,`msgId`,`gone`,`held`,`files`,`fill`,`blockerId`,`pick`,`missing`,`unheard` (all oe) | impl.go:144-173 |
| `blocker.closed` | `taskId`,`by`,`note`(oe),`restored`(always),`at` | impl.go:175-181 |
| `blocker.option` | `taskId`,`blockerId`,`answer`,`why`(oe),`by`,`at` | cards_impl.go:50-57 |
| `blocker.resolved` | `taskId`,`blockerId`,`answer`,`option`(always int, -1 own words/close),`by`,`restored`(oe),`closed`(oe),`at`,`note`(oe) | cards_impl.go:59-73 |
| `task.steps` | `taskId`,`steps`(**no omitempty: nil → `null`**, split.go writes `null` when no done steps, subtasks.go:424-430),`by`,`at` | cards_impl.go:75-80 |
| Step | `id`,`text`,`done`(always),`doing`(oe) | cards.go:56-61 |
| `task.step` | `taskId`,`stepId`,`done`(oe *bool),`doing`(oe *bool),`by`,`at` — pointer: `false` IS written when set | cards_impl.go:82-89 |
| `task.watched` | `taskId`,`who`,`watching`(always),`by`,`at` | cards_impl.go:91-97 |
| `task.pinned` | `taskId`,`pinned`(always),`rank`(oe float64: rank 0 omitted),`by`,`at` | pins.go:381-387 |
| `task.parent` | `taskId`,`parent`(always; `""` clears),`by`,`at` | subtasks.go:25-30 |
| `task.edited` | `taskId`,`title`,`objective`,`doneCriteria` (oe *string: `""` IS written when set),`by`,`from`(oe),`note`(oe),`at` | ask.go:171-180 |
| `take.requested` | `taskId`,`by`,`op` (`claim|release|reopen`),`at` | ask.go:140-145 |
| `take.state` | `taskId`,`taker`(always),`reason`(always),`req`(oe {device,seq}),`told`(always),`heard`(oe),`since`(always),`unclaimed`(oe),`seen`(oe map device→seq, keys sorted),`was`(oe),`at` | ask.go:147-169 |
| `lease.acquired` | Lease embedded: `resource`,`holder`,`until`,`stolenFrom`(oe),`acquiredAt`,`conflict`(oe); then `stolen`(oe [{resource,holder}]) — no `at` field (Lease has `acquiredAt`) | coord.go:175-184, impl.go:193-201 |
| `lease.released` | `resource`,`holder`,`reason` (`released|holder_gone|expired`),`by`,`at` | impl.go:183-189, impl.go:31-35, mesh.go:35 |
| `lease.refused` | `resource`,`by`,`heldBy`,`held`,`until`,`at`,`conflict`(oe) | board.go:169-179 |
| `lease.stolen` | `resource`,`holder`,`acquiredAt`,`by`,`at` (actor `user`) | mesh.go:41-47 |

Nested types: `Option{answer, why(oe), by, at, recommended(oe), link(oe *Link)}` (cards.go:68-77); `FileRef{sha256, name, size int64, mime(oe), device}` (blob/files.go:36-42); `BlockedBy{kind, ref(oe)}` kinds `lease|task|person|agent|external` (board.go:42-54).

Task states: `submitted working input_required completed failed canceled blocked`; terminal = completed/failed/canceled (coord.go:39-51, impl.go:44).

Ids: `newID` (id.go:389-410) = ULID layout: 6 bytes big-endian unix ms, 10 random bytes (crypto rand), 26 chars Crockford `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, first char carries 3 bits. Task ids and comment ids. Blocker ids `b<N>`, step ids `s<N>`, minted per task and never reused (`blockerSeq`/`stepSeq` = max seen, cards_impl.go:236-247, 369, 1392). Comment id of a mirrored message = its `msg_…` id (board_impl.go:552).

Row refs (exact punctuation differs!): `Returned.Row = "<device>:<seq>"` (impl.go:609); approval `answerRef = "<device>/<seq>"` (ask/ask.go:331); revive approval id `"revive-<device>-<seq>"` (ask/ask.go:467); approval id `base36(UnixNano)+"-"+base36(counter)` (ask/ask.go:505; e.g. 2026-10-26T12:00Z → `dmer4j36rk00`); `stolenKey = resource+"\0"+holder+"\0"+acquiredAt.UTC().RFC3339Nano` (mesh.go:49-51).

### 2.3 coord derived views (wire JSON to the GUI/API)
`Task` JSON (coord.go:54-131): `id,short(oe),shortWas(oe),tag(oe),withdrawn(oe),title,objective,doneCriteria(oe),repoKey(oe),scope(oe),state,holder(oe),holders(oe),assigned(oe),shared(oe),createdBy,note(oe),createdAt,updatedAt,updatedBy(oe),reason(oe),blockedBy(oe),links(oe),comments(oe int),steps(oe),blockers(oe),watchers(oe),ask(oe),returned(oe),parent(oe),subtasks(oe),rollup(oe),pinned(oe),pinRank(oe)`. `Blocker` (cards.go:109-143): `id,question,context(oe),raisedBy,at,options(oe),resolved(oe Resolution{answer,option,by,at,closed(oe),note(oe)}),links(oe),attachments(oe),on(oe),status(oe),sentTo(oe),missing(oe)` + private `reason, legacy, asked`. `Comment` (board.go:58-102): `id,taskId,by,text,wake,at,attachments(oe),to,msgId,gone,held,files,blockerId,pick,missing,unheard (oe),device`; HLC/Seq never on the wire. `Activity{at,by,what,kind,device,blockerId(oe)}` (cards.go:207-216). `coord.Ask{wake(oe),repoKey(oe),owner,taker(oe),reason(oe),told,heard(oe),since,unclaimed(oe),waiting(oe),outcome(oe)}` (ask.go:105-129). `Returned{was,at,row(oe),person(oe)}`. `Rollup{state,done,total,blocked(oe),blockedQuestion(oe)}`. `Card = Task + lastComment(oe),resolved(oe),gone(oe),overlaps(oe)`; `LeaseCard = Lease + refused(oe),blocks(oe)`; `BoardView{needsYou,blocked,working,open,done,doneTotal,leases}` (all arrays non-null). `Notice{kind,taskId,ref,title,to(oe),by,text,wake,attachments(oe),watching(oe),person(oe)}`. `Change{kind,taskId(oe),resource(oe),holder(oe),reason(oe)}`, kinds `task lease lease_expired lease_refused comment resync`.

### 2.4 Approvals (`internal/ask`)
| Kind | Payload | Actor | Source |
|---|---|---|---|
| `ask.posted` | the whole `Ask` struct: `askId,by,name,title(oe),text,level(oe),actions,until,target(oe),deliveryId(oe),device,resolved,choice(oe),answered(oe),answerRef(oe)` — written with `device:""`, `resolved:false` (fold fields are on the wire) | `irori` | ask/ask.go:57-77, 501-509 |
| `ask.answered` | `askId,choice(always),target(oe),deliveryId(oe),by,atDevice(oe),at(omitzero)` | `user` | ask/ask.go:79-87 |
| `ask.resolved` | `askId,choice,answered(always),answerRef(oe),at(omitzero)` | `irori` | ask/ask.go:89-95 |
| `revive.requested` | `target,deliveryId(oe),by,atDevice(oe),at(omitzero)` | `user` | ask/ask.go:97-103 |
`omitzero` (Go ≥1.24) omits a zero `time.Time`; `omitempty` does not omit structs, so elsewhere zero times are written as `0001-01-01T00:00:00Z`.
Buttons `Revive`, `Not now`; `ReviveWait = 30m` (ask/ask.go:48-54). Revive ask text: Title `"Revive "+name+"?"`, Text `"Your other device asked to revive "+name+" here, to deliver a message to it."`, Name `"Irori"`, By `"irori"` (ask/ask.go:480-483).

### 2.5 decision (`decision.recorded` payload, decision/decision.go:31-67)
`Decision{id, by{key,name(oe),kind(oe)}, statement, why(oe), anchor(oe *Anchor), supersedes(oe), source(oe {messageId(oe),turnId(oe)}), trust ("user"|"untrusted"), at}`; `Anchor{repoKey, root, path, sha(oe), start(oe), end(oe), hash, context(oe)}`. `Status{state, start(oe), end(oe)}` states `ok moved changed gone unknown`.
- Line hash: lines normalised (every `\r` removed, trailing Unicode whitespace trimmed), joined with `\n`, SHA-256 lowercase hex (decision/lines.go:65-94). Vector: file `"a  \r\nb\t\r\n"`, lines 1-2 → `7e18f737311b2dc3b2f269dd78396b0351f14fb66efa879f768cb23181883c78` (decision_test.go:247-259).
- Whole file (start 0): git blob SHA-1 of the CRLF→LF form: `sha1("blob "+len+"\0"+data)` (gitsha/gitsha.go:15-29). Vectors: empty `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391`, `"hello\n"` `ce013625030ba8dba906f756967f9e9ca394464a`.
- `splitLines`: empty data → no lines; one trailing `\n` removed then split on `\n` (lines.go:81-92). Operate on bytes (Go strings may hold invalid UTF-8; `TrimRightFunc` decodes invalid bytes as U+FFFD, not space).

### 2.6 roster rows (store/kinds.go:7-17)
`device.started {name,os,version}`; `agent.seen` = a `presence.AgentCard` (presence/presence.go:29-69); `agent.status {key,status,name,permissionMode?,nameSource?}`; `agent.gone {key,reason}`.

### 2.7 devcolor row
`device.color {deviceId, color}` (+ `at` per comment; the fold reads only the two, devcolor/devcolor.go:74-76, 149-152); actor must be `user`; `""` color = back to default.

---

## 3. Rules, constants, timings

### 3.1 Constants
| Name | Value | Source |
|---|---|---|
| DefaultTTL / MaxTTL | 30m / 4h | coord.go:27-28 |
| GoneGrace (lease holder gone → release) | 3m | coord.go:29 |
| HolderGrace (holder gone → off card) | 10m | coord.go:33 |
| AskUnclaimed | = GoneGrace (3m) | ask.go:79 |
| AskWait (ask_claim wait) | 5s | ask.go:81 |
| maxAskTitle / maxAskNotice / MaxOutcome | 80 / 600 / 300 runes | ask.go:82-85 |
| maxTitle / maxObjective / maxScope | 200 / 8000 runes / 64 globs | impl.go:37-41 |
| MaxComment / MaxAttachments / MaxReason / MaxRef / MaxKey | 4000 / 10 / 500 / 300 / 200 | board.go:30-34 |
| DefaultDoneCard / MaxDoneCard | 20 / 200 | board.go:35-36 |
| MaxQuestion / MaxContext / MaxAnswer / MaxWhy / MaxOwnAnswer | 200/800/200/400/800 runes | cards.go:37-41 |
| MaxOptions / MaxOpenBlockers / KeepResolved | 6 / 8 / 10 | cards.go:42-44 |
| MaxSteps / MaxStepText / MaxActivity | 20 / 200 / 500 | cards.go:45-47 |
| MaxMissing / MaxMissingText / MaxBlockerLinks | 6 / 200 / 8 | cards.go:48-50 |
| maxWatchLines / maxWatchPending | 8 / 32 | cards_impl.go:25-26 |
| maxNoticeField | 600 runes | board_impl.go:20 |
| MaxLinks / maxLinkRef / maxLabel | 16 / 1000 / 60 | links.go:252-254 |
| lapsedWithin | 24h | mesh.go:39 |
| clockSkew | 2m | mesh.go:293 |
| subBuffer | 64 | subscribe.go:380 |
| MaxAside | 2048 | boardlog.go:181 |
| maxSubtasks per split | 20 | subtasks.go:438 |
| search defaultDocPage / showComments / showCommentCut | 500 / 10 / 600 | board_search.go:15-17 |
| Follow tick, ask Book tick | 1s | mesh.go:89, ask/ask.go:196 |
| ReviveWait | 30m | ask/ask.go:54 |
| decision MaxStatement / MaxWhy / ContextLines | 500 / 2000 / 2 | decision/decision.go:15-17 |
| decision maxCheckSize / scanBudget | 8 MiB / 64 MiB | decision/anchor.go:164,168 |
| decision List limit default/max | 50 / 500 | decision/view.go:532-536 |
| ParseShort max length | 12 chars | short.go:168 |
| board tag | `[a-z][a-z0-9-]{0,15}` | short.go:239-249 |

"Runes" = Unicode scalar count (`utf8.RuneCountInString`). `cut(s,n)` (board_impl.go:352-358): if runes>n → first n-1 runes + `…`. `capLabel` same with 60.

### 3.2 Fold (impl.go:431-829) — order of gates per row
`apply(ev)` = `fold(ev)`; `foldDeferred()`; `retryAside()`; if not replaying `relink()` (impl.go:431-438).
`fold(ev)`:
1. `writerOK(ev)` false → `ignore(dev)` (count, per device) and drop (impl.go:443-446).
2. `asideGate(ev)` (board only; `store.Unresolved(ev)!=nil` → set aside up to MaxAside, else count `asideDrop` and drop) (boardlog.go:165-178).
3. `askGate(ev)` → `(ok, wait)`; not ok with wait≠"" → defer the row under that ask id (dedupe by cursor) (impl.go:450-455; ask.go:648-655).
4. Kind switch below.

Replay (impl.go:342-365): list kinds with prefixes `task.`,`lease.`,`blocker.`,`take.` in log order, pages of `store.MaxListLimit`=1000 using the `(hlc,device,seq)` cursor; `replaying=true` (suppresses `note`, watch queueing, short "moved" notes, relink per row); after each page `prune(stamp())`; at the end `renumber()` then `relink()`. `last[dev]` = max seq folded. Live (`applyRemote`, mesh.go:198-207): skip `seq <= last[dev]`, set last, apply, `holderDuties`. Own rows fold through `write`→`apply` directly; Follow skips own-device rows (mesh.go:103). Resync signal (`ev.Seq==0` from store.Subscribe) → `catchUp`: per device page `AfterSeq: last[dev]` until vector top (mesh.go:122-154).

**refoldIfCut** (mesh.go:161-196): if any `vector[dev] < last[dev]` (a revoke moved rows away): remember each task's `Short`; reset exactly: tasks, order, nextOrder, held, comments, byTask, refusals, blockerSeq, stepSeq, activity, claims, claimed, shortGroups, shortIdx, last, conflicts, stolen, lapsed, ignored, ignoredBy, asks (askBook.reset), links, pins. **Kept**: `shortTop`, `firstAbsent`, `watchq`, `shortMoved`, `aside`, `asideDrop`, `refused`, subscribers. Replay, then for each task whose Short changed call `shortMovedTo`, then note `Resync`.

Kind rules (each also adds an activity line via `touch`, see 3.9, and notes a Change):
- **task.created**: needs `taskId`. New id gets fold order `nextOrder++` (dup keeps first order but the task struct is rebuilt). Links: keep first MaxLinks with non-empty unique Ref. Task = {State submitted, CreatedBy/UpdatedBy=by, CreatedAt=UpdatedAt=at, Tag=tag()}. `foldAskCreated` (only if `personRow(ev,by)`: Ask{Wake,RepoKey,Owner=ev.DeviceID,Since=CreatedAt}). `syncPin`. If parent: `foldParent`. `claimShort(id,cursor,num)`. Activity `"put the card up as a subtask of "+parent.ShortRef()` (watch, also parent holders) when the parent exists, else `"put the card up"`.
- **task.claimed**: task exists and by≠"": `addHolder` (not shared → Holders=[by]; shared → append once), remove from Assigned, submitted→working, Updated*. Activity `"took it up"` (watch).
- **task.assigned**: to≠"": `engaged = isHolder(to) && !in Assigned`; addHolder; if !engaged and not in Assigned → append to Assigned. Activity `"assigned it to "+to` (watch).
- **task.status**: valid state. If state canceled and `!mayCancel(t,dev)` → ignore+drop. If `t.Withdrawn && !unwithdraws` → drop (no count). Withdrawn: set when canceled+withdrawn+onBoard; cleared on any non-terminal state. If an open room ask: outcome = row's or `{none, cmpStr(note,"closed by the person")}` → `closeAskFold`. Then `State=p.State; Note=p.Note` (note replaced even when empty), remove by from Assigned. Blocked → `foldBlocked` (3.5); else `closeOpen(by,note,at)` and phrase `"reopened it"` (terminal→non-terminal) or `"set it to "+state` (changed). `syncReason`, Updated*, activity line with `BlockerID=raised`, `refreshBlocker` for a raise.
- **task.joined**: who≠"": append if absent; submitted→working; phrase `"joined"`. **task.left**: remove who; reason `"gone"` → phrase `"went away"` and if no holders left → `Returned{Was:who, At, Person:p.person, Row:"<dev>:<seq>"}`. If who≠by append `" ("+who+")"`. Remove from Assigned; `syncHolder` (clears Returned whenever someone is on it, impl.go:879-893). Not a watch line.
- **task.shared**: set Shared, Updated*. No activity line.
- **task.linked**: removed+present → delete (empty → nil); !removed+absent+<MaxLinks → append; otherwise drop silently. Phrase `"removed the link "+label` / `"linked "+label`; watch only if the link is a GitHub PR (prLink).
- **task.comment**: existing comment id → `mergeComment` (3.7). msgId≠"" clears blockerId/pick/missing. Task must exist. `primary = !gone && !fill`. Comment.Pick=max(pick,0); `person = personRow(ev,by)`; `Unheard = unheard && person`; non-primary non-held row sets `passed=text`. Append to byTask (fold order), `Comments++`; primary → `t.UpdatedAt = cm.At`. Blocker comment → line `"replied on blocker bN"` or (person) `"answered blocker bN"` with BlockerID, `refreshBlocker`; plain (no msgId) → `"commented"`; mirrored message → no line.
- **blocker.closed**: task Blocked only. `s = restored` unless invalid/blocked/terminal → `restoreState`. Clear Reason/BlockedBy, note replaces only if non-empty, `closeOpen`, phrase `"unblocked it"`+(`": "+note`) (watch).
- **blocker.option / blocker.resolved / task.steps / task.step / task.watched** → applyCard (3.5, 3.6).
- **task.pinned** → foldPin (3.10). **task.parent** → foldParent; phrase `"took it out of its parent"` or `"made it part of "+refOf(parent)` (watch, also new parent's holders).
- **take.requested / take.state / task.edited** → applyTake (3.8).
- **lease.refused**: needs resource, by, heldBy. Unparseable resource → named. `at` default. Conflict → `endConflict(name, by)`. Append refusal.
- **lease.acquired**: needs resource. First drop each `stolen` lease still held by that holder (Change reason `stolen`). Unparseable → named resource. AcquiredAt default from HLC. If `stolen[stolenKey(name,holder,acquiredAt)]` → drop (a renewal of a person-stolen lease doesn't revive). If `conflicts[name].Holder == holder` → replace contested record keeping its Conflict; return. `conflictOf` (3.4) may consume it. Else if held by same holder keep old Conflict; `held[name]=rec`.
- **lease.stolen** → applyStolen (3.4). **lease.released**: normalize resource; drop matching `lapsed`; `endConflict(name,holder)` → Change with row's reason.

### 3.3 Writer (owner) rules
| Check | Private log | Board | Source |
|---|---|---|---|
| lease.acquired / lease.released | `MayWrite(holder, dev)` | `KeyPerson(holder)!="" && == DevicePerson(dev)` (falls back to private rule if either fn nil) | impl.go:376-387, boardlog.go:44-50 |
| lease.refused | `mayWrite(r.By, dev)` | same | impl.go:388-393 |
| task.comment with msgId | `MayWrite(by,dev) || (gone||fill) && MayWrite(to,dev)` (owner.MayWrite, private rule even on board) | same | impl.go:405-419 |
| task.comment with blockerId, not personActor | `owner.MayWrite(by, dev)` | same | impl.go:407-408 |
| any row with actor `irori` | — | `sweeperOK`: lease.released ok; task.left ok iff `mayWrite(who,dev)`; task.status/blocker.resolved ok iff task unknown or `mayCancel`; every other kind refused | impl.go:372, boardlog.go:82-100 |
| personRow(ev,by) | `actor=="user" && IsOwner(by)` | `actor==by && by starts "person:" && IsOwner(by)` | boardlog.go:32-37 |
| personActor(ev) | `actor=="user"` | actor starts `"person:"` | boardlog.go:147-152 |
| mayCancel(t,dev) (cancel rows) | always true | `p=DevicePerson(dev)`, p≠"" and (p==KeyPerson(t.CreatedBy) or BoardOwner(p)); true if KeyPerson/DevicePerson nil | boardlog.go:64-73 |
| unwithdraws | — | non-terminal state && personActor && mayCancel | boardlog.go:108-110 |
| task.pinned | actor `user` && IsOwner(by) (even on a board, so pins never count there) | same | pins.go:402 |
| lease.stolen | personRow | personRow | mesh.go:332 |
| take.requested | `MayWrite(by,dev) && DeviceOf(by)!=""`; op reopen needs personRow; unknown op dropped (not counted) | same | ask.go:421-434 |
| take.state | dev == ask.Owner | same | ask.go:461 |
| task.edited | editGate (3.8) | same | ask.go:728-758 |
Malformed JSON passes writerOK (`true`, apply drops it).

### 3.4 Leases (impl.go:1492-1632, mesh.go, resource.go, glob.go)
Resource grammar (resource.go:12-29, 50-84), input trimmed:
- `path:<kind>:<value>:<glob>` kind `remote|root`; repoKey ends at the **last** `:`; `root:` value `^[0-9a-f]+$`; `remote:` value non-empty, no leading/trailing space, no `\`, no control char (resource.go:96-113). Canonical `path:<repoKey>:<normglob>`.
- `port:<n>` 1..65535 and `itoa(n)==text` (rejects `080`, `+80`).
- named `^[a-z0-9][a-z0-9._-]*(/[a-z0-9._-]+)*$`.
`NormalizeGlob` (resource.go:119-159): trim; reject empty, `\`, `:`, any control char (Cc); strip repeated leading `./`; reject empty or leading `/`; trailing `/` → append `**`; each segment non-empty, not `.`/`..`, and valid for Go `path.Match` (malformed class: `[` with no members, member starting `-` or `]` (other than closing after ≥1 member), unclosed `[`, `[a-]`, invalid UTF-8 inside a class; hi<lo is NOT malformed). Port Go's `path.Match` `matchChunk`/`getEsc` validation; no escapes since `\` is rejected.
Overlap (glob.go:190-453): split on `/`; whole-segment `**` = zero or more segments; memoised DP over segments and within a segment over tokens (`*`, `?`, class, literal rune). Literals compare case-insensitively via full `unicode.SimpleFold` orbit (`k`↔`K`↔U+212A, `s`↔`ſ`↔`S`, `µ`↔`Μ`↔`μ`, `ß`↔`ẞ`, `Σ`↔`ς`↔`σ`). Class vs class: both negated → true; both positive → true if both "have a letter" (any range with hi>0x7F or touching a-z or A-Z) else range intersection; mixed → `hasLetter(pos) || !covered(pos, neg)`. Class vs literal: any fold of the literal in ranges XOR neg. Tokenizer class quirk: `]` closes only after ≥1 member; `x-y` range only when the char after `-` isn't `]`; hi<lo → unparseable. Any unparseable glob → overlap **true**. `conflicts(a,b)` = both path → same repoKey && overlap; else canonical names equal (resource.go:163-168).

Acquire (impl.go:1492-1549): trim holder; parse; ttl≤0→30m, min(ttl,4h); prune; victims = held leases with other holder, live, conflicting, sorted by Resource. Victims and !steal → `refuse` (one lease.refused per distinct victim holder unless an active refusal with same (resource, by, heldBy) exists), then ErrHeld `"%s is leased by %s until %s (pass steal to take it)"` (RFC3339). Steal: `stolen` list of all victims, `StolenFrom` = sorted unique victim holders joined `,`. Renewal (same holder, live) keeps AcquiredAt and old StolenFrom if none new. One lease.acquired row.
Release (impl.go:1560-1587): holder or person; not live → ErrNotFound; holder on another device → `steal` (lease.stolen) else lease.released reason `released`.
`live(l)` (mesh.go:62-67): own-device (or deviceless) holder → `Until > now`; another device's → live forever unless `Revoked(device)`.
`prune(now)` (impl.go:850-864): delete non-live held; own-device holder → queue in `lapsed` unless replaying and lapse older than 24h; Change `lease_expired`; then gcRefusals.
`holderDuties` (mesh.go:212-250), every 1s and after each remote row: for each contested lease of a local holder not yet in `refused[stolenKey]` → write lease.refused{conflict:true, heldBy: winner's holder}, mark only on success; prune; for each lapsed not renewed → lease.released reason `expired` by `irori`; clear lapsed; `askDuties`.
`releaseGone` (impl.go:1614-1632): prune, observe; leases with Until>now, local holder, gone ≥ GoneGrace → lease.released `holder_gone` (sorted by resource).
`conflictOf` (mesh.go:256-289): existing live held lease on the name by another holder on a **different device**, not stolen by this row: if `rec.Until+2m <= e.AcquiredAt` → history, drop (true); if `e.Until+2m <= rec.AcquiredAt` → not a conflict (false, replaces). Else lower device id wins (`DeviceOf` string compare), each records the other in `Conflict`, winner in held, loser in conflicts, Change reason `conflict`.
`applyStolen` (mesh.go:327-348), `endConflict` (mesh.go:353-371): ending the standing lease promotes the contested one with Conflict cleared.
Refusal active (board_impl.go:765-772) iff heldBy holds a lease with `Until>now`, `AcquiredAt <= r.At`, conflicting with r's resource. `Refusals()` newest first: reverse fold order then stable sort by At desc. `Leases()`: live held + all contested, sorted by Resource.

### 3.5 Blockers (cards_impl.go, board_impl.go)
Validation: question 1..200, context ≤800, options ≤6 each answer 1..200 / why ≤400 / optional link via ParseLink; duplicate answers (case-insensitive `EqualFold`) merge (Recommended OR, keep first link); >1 recommended after merge → ErrInvalid; blocker links ≤8 (dedupe by kind+ref), files ≤10 (cards_impl.go:112-189). checkBlocker (board_impl.go:43-90): ref trimmed ≤300 runes, no CR/LF; lease → must normalize and be held now (`leaseHeld`: any unexpired lease conflicting); task → non-empty, not self, exists, not terminal; person → `""` or IsOwner; agent → non-empty and not IsOwner; external → any; other kind ErrInvalid.
Raise (`raiseLocked`, cards_impl.go:591-650): refuse on open room ask; same open question already, task Blocked, and (nb nil and not reasonOnly, or same On) and brings no new links/files → no-op returning it; caps for merged links/files; nil nb (not reasonOnly) → existing item's On, else `{person,""}`; Block (`reasonOnly`) on a blocked task targets the first open legacy item with `replace:true` and its id; new item when no open item with q and `countOpen >= 8` → ErrInvalid. Row: task.status{state blocked, note: t.Note, reason, blockedBy: nb, blocker}. Block's question = reason if ≤200 runes else `cut(reason,200)` with whole reason as context (cards_impl.go:1302-1307).
`foldBlocked` (cards_impl.go:338-419): with blocker id: replace of an open item → unless row position is before `b.asked`, overwrite question/context/reason(=row reason or question)/raisedBy/at/On/links/files, `asked=pos`, refresh; phrase `"changed the blocker: "+q`. Id exists → nothing. Same open question → set On, add new links/files (`"added to the blocker: "`). Else new item (blockerSeq=max), options deduped ≤6 with At default; replace-before-raise sets asked. Phrase `"raised a blocker: "+q`. Reason-only legacy row: q=reason (cut 200 + context when long); if was blocked replace first open legacy item; else mint next `bN`, legacy.
`refreshBlocker` (cards_impl.go:427-452): blocker comments on bid at/after `asked`, log order: none → `open`; last by person → `answered`, SentTo = its `to`; else if any earlier person comment → `replied`, Missing = last's missing.
`closeOpen` resolves all open as `{answer:note, option:-1, by, at, closed:true}`; `trimResolved` keeps the newest 10 resolved by position. `syncReason`: Reason = first open item's reason; BlockedBy = first open item with an On.
Answer (cards_impl.go:699-819): without Resolve only the person (else ErrInvalid); with Resolve and the person's last answer present and pick 0 → text is resolver note (agent: required, one line, ≤200; person ≤800), answer = person's text, option = their pick-1 (or -1 if out of range); with a person answer and an agent picking → ErrInvalid; pick out of 0..len → ErrInvalid; pick>0 → option's answer. Not Resolve → a blocker comment `{to: raisedBy, pick: opt+1}`. Resolve → blocker.resolved with `restored = restoreState` only if this was the last open item. Fold of blocker.resolved: invalid option → -1; last open & Blocked → state = restored or restoreState; trim, syncReason; phrases `"closed “q”: "+answer` / `"resolved “q”: "+note` / `"answered “q”: "+answer`.
blocker.option fold: existing answer (EqualFold) → mark recommended (no-op if already); else append (≤6) recommended; phrase `"recommended “a” for “q”"` (watch, also raiser).
`restoreState` (impl.go:906-913): working if any holder not in Assigned, else submitted.
Sweep of resolved blockers (board_impl.go:1077-1126): tasks with any open item whose lease is no longer held (`"lease released"`) or task is terminal (`"task "+state`), sorted by id; all open items gone → `closeBlocker` by irori (note = first why) + NoticeUnblocked; else blocker.resolved `{option:-1, closed:true, answer: why}` per item and NoticeAnswered to raisers + recipients.

### 3.6 Steps, watchers, activity
checkSteps (cards_impl.go:894-919): ≤20, each trimmed 1..200 runes, one line, not done+doing, ≤1 doing. SetSteps id reuse: given id if it exists and unused; else same text of an existing unused step not named by any input; else `s<++seq>`; equal list → no-op; refuses on a parent. task.steps fold: skip empty/duplicate ids, cap 20, stepSeq=max, `normSteps` (first doing only, done never doing). task.step fold via `applyStep` (cards_impl.go:1019-1039): done=true clears doing; doing=true clears every other doing and sets done=false. Phrases `ticked/unticked/started/paused “text” (d/n)`; task.steps `"set the steps (d/n done)"`. Watch: agents only (not owner, not irori, checkKey); task.watched fold adds/removes, phrase `started watching`/`stopped watching` (not a watch line).
Activity (cards_impl.go:500-532): line inserted by log position; keep top 500 by position. `watch` lines queue (outside replay) for watchers + `also`; a subtask's line is also queued on its non-terminal parent as `"<sub ShortRef> (part of <parent ShortRef>): "+what` with also = parent holders. Queue cap 32 per task, overflow counted in `dropped`.
watchNotices (cards_impl.go:1122-1186), drained each SweepNotices: per task id sorted; recipients = watchers then `also` keys, only keys whose device is this device or none, not owner/irori, unique; each recipient's lines = lines not by itself where it is a watcher or in that line's also; text via `watchText`; identical texts share one Notice. Format (cards_impl.go:1188-1275), byte-exact so `MergeWatch` interoperates:
```
Update on "<title %q>" (<Refs>), a card you watch or raised a blocker on:\n
[- (<N> earlier changes)\n]            N = hidden + dropped
- <actor> <what, cut 600>\n            ≤8 newest; actor "irori" shown as "Irori"
Now: <EffectiveState>[, subtasks d/t done | , steps d/n][, K open blockers].\nshow task:<id> for the card, its comments and links.
```
MergeWatch: concatenate earlier+newer lines, sum counts, keep last 8, newer head and foot; a line not starting `"- "` continues the previous one.

### 3.7 Comments, messages, notices
AddComment: trim, 1..4000 runes, ≤10 files, any task any state; Notice comment to `recipients(t,by)` with text `"%s commented on %q (%s):\n%s"+deeper`. `recipients` (board_impl.go:334-345): holders ≠ by, not owner, not irori; no holders → creator if agent ≠ by. `deeper(id) = "\nshow task:"+id+" for the card, its comments and links."`.
PersonComment (board_impl.go:413-528): hearers answer = engaged holders, then assigned holders, else creator; watch = watchers; filtered by `around`, never by/owner/irori, no duplicates. Unclaimed open room ask → goes to `Heard` around, NoticeAskComment. A finished card with a holder around (non-owner) and reopenable → `reopenLocked` with note `"reopened by the person's comment"` before the comment. Unheard when nobody hears. Texts at board_impl.go:487-525.
RecordMessage (board_impl.go:546-590): msgId must be `msg_`+≥1 char; Fill clears Held; Held → text ""; else over 4000 runes → cut to fit `"… (full: msg:<id>)"`; writer `irori` when Writer=="irori"; existing comment: skip if (gone&&already gone) / (fill && not held) / (primary && already primary); writer rule checked against this device. mergeComment (board_impl.go:599-629): gone ORs in; first non-primary non-held text sets passed (and fills a held entry); a primary arriving later overwrites by/to/text/wake/at/held/files/attachments/device/hlc/seq and task UpdatedAt.
Notice text templates (verbatim, Go `%q` = strconv.Quote): assigned board_impl.go:139-150 (`"\nClaim it to start."` while still Assigned); unblocked 322-327; answered cards_impl.go:781-817; reopened impl.go:1445-1451; returned board_impl.go:1187; ask/taken ask.go:339-347, 593-596; sweep partial close board_impl.go:1121.

### 3.8 Room asks (ask.go)
Post (ask.go:293-332): person only; checkComment rules; repoKey checked; task.created with `num`, title `AskTitle` (first line, whitespace collapsed, ≤80 runes else cut at last space if index > 40 then `…`, ask.go:272-283), objective = text, `ask`; files → a comment `"Files with the ask."`; free agents → take.state `{reason posted, told, heard, since: at, unclaimed: told==0}`.
freeAgents (ask.go:354-370): key non-empty, not irori/owner, status `idle`, DeviceState≠`offline`, device not revoked, repo matches ask's repo if set, not in skip; unique; sorted.
take.state fold (ask.go:452-502): ignore if not owner device; always `mark(cursor,taker)`; skip if closed or `seq <= stateSeq`; merge `seen` (max); set Taker/Reason/Told/Since/Unclaimed (taker≠"" → unclaimed false, Holders=[taker]; else none), Assigned=nil; `answerRequests`; stale map; phrases took it / gave it back / opened it again: X went away / asked the room again / nobody took it yet.
askGate (ask.go:689-725) on task.claimed/joined/assigned/status for a task with an Ask: closed ask → fold iff `!(cursor < closedAt)`; open: task.status folds iff outcome≠nil and (person or `takerAt(cursor)==by`), or no outcome, person and terminal; everything else waits on the ask id. `takerAt` = taker of the owner's latest take.state strictly before the cursor. editGate (ask.go:728-758): person ok; else `MayWrite(by,dev)` (else ignore); from==taskId → drop; no from: plain card → creator or holder; ask card → ask=self; ok iff `takerAt(ask)==by` else wait. task.edited fold: title only if non-blank; `"edited the card"` or `"changed it from the ask "+ShortRef+(": "+note)`.
Owner duties (ask.go:779-854), only for open asks owned by this device: claim → if taken or claimer gone → mark seen, done, nothing written; else take.state{taker, reason taken, req}. release → if taker matches → take.state{reason released, was, since now} + retell. reopen → if untaken, retell woken.
SweepAsks (ask.go:867-960): duties; observe; taker gone (`takerGone`: local → gone ≥3m; remote → missing from roster or `gone`) → take.state gone + retell; retell → take.state{reason, told, heard, since now, unclaimed: none free} + NoticeAsk to free minus leaver; otherwise newly free agents and the unclaimed clock (`unclaimed |= told+free==0 || now-since >= 3m`), late tell wakes only if `now-since < 3m` and (reason reopened or (posted and Wake)); a revoked owner device → this device writes the person's cancel `"posting device removed"`. Failed writes re-queue retells.
AskClaim (ask.go:999-1068): agent only; taken by another → ErrHeld; write take.requested claim, run duties, wait up to `wait` on local changes. AskClose (ask.go:1099-1238): note whitespace-collapsed, person default `"closed by the person"`, 1..300; outcomes card (task.edited + optional steps + task.status working w/ outcome + taker's own task.claimed + optional task.parent), edit (target in play, not an ask; task.edited from + completed `"Changed <ref>: note"`), answered (completed), none (canceled).
StaleNotice (ask.go:1282-1297): `"the ask closed"` or `"taken by "+taker` when taker≠to.

### 3.9 Short refs (short.go)
Each task.created claims `num` = `shortTop+1` (shortTop = highest number seen, never lowered, kept across refold). Rows without num (num≤0) get their rank among such rows in log order. Settle: claimants of one number sorted by cursor; alone → `"12"`; ≥2 → `"12"+suffix(i)` (bijective base-26 `a..z, aa, ab..`; 0→a, 25→z, 26→aa, 27→ab) with ShortWas `"12"`, and bare `"12"` resolves to none (ErrAmbiguous text short.go:278). Live: num>0 inserts into its group and settles it; num 0 or repeated id → full renumber. Moved ref (live) → shortMoved time, Change, watch line `"renumbered it #to (it was #from)"` (+ clash explanation) to watchers+holders+creator (short.go:147-155).
ParseShort: lower, trim, strip one `#`, 1..12 chars, digits (first not `0`) then a-z only. ParseRef: last `#` at index>0 splits a tag (validTag). taskRefLocked: strip `task:` (case-insensitive key), tagged ref for another tag → ErrNotFound; not a short → returned as given; short lookup → index, else a task with that literal id, else ambiguous, else ErrNotFound. `ShortRef()` = `tag+"#"+short` or `task:<id>`; `Refs()` = `ShortRef()+", task:"+id`.

### 3.10 Pins, subtasks, holders, board grouping
Pins (pins.go): newest row by cursor per task wins (person rows only); a pin row may precede its card. New pin rank = last+1 (first = 1); MovePin before X = between neighbours' mean, before first = first-1; unknown before → ErrInvalid; ranks include pins of not-yet-folded cards.
Subtasks (subtasks.go:88-216): relink sorts link rows stably by cursor; a set row is inert if child==parent, either task missing, parent already has a parent, child has kids, or either is an ask open at that position (`askAt`). Subtasks sorted by short number, then short string, then id (unnumbered last). Rollup: Done=completed; Total=non-canceled; state precedence: parent terminal → own; all ended → completed (if any) / failed / canceled; parent blocked or any blocked → blocked (Blocked/BlockedQuestion from first blocked subtask unless the parent itself is blocked); any input_required → input_required; any working or started (non-submitted) → working; else submitted. SetTaskState on a parent: canceled cascades `"its parent <ref> was canceled"`; completed only when all subtasks ended; others ErrSubtask. Reopen: parent with all subtasks ended refused; reopening a subtask first reopens a terminal parent (`"its subtask <ref> was reopened"`).
Gone holders: `observe(now)` (impl.go:1038-1086) tracks holders of non-terminal tasks and live lease holders; local, non-owner, not live (roster card missing or status `gone`) start a clock at first observe (or the `gone` card's UpdatedAt if earlier); in memory only. returnGone (board_impl.go:1143-1193): holders absent ≥ HolderGrace, local, non-owner, task not terminal and no open room ask → task.left{reason gone, person: last && (creator is owner || creator==h || creator not live)}; last holder on a working task → task.status submitted `"nobody is on it: the agent working it went away"`; NoticeReturned to creator if !person. ClaimTask on a non-shared task by another non-gone holder → ErrHeld.
Board groups (board_impl.go:909-1062): Done (EffectiveState terminal) → NeedsYou → Blocked → Working (has holders) → Open; each sorted UpdatedAt desc then id desc; Done capped (default 20, max 200), DoneTotal = all. needsYou rules at board_impl.go:1017-1062. Leases shown only if a blocked task waits on them or an active refusal; sorted Until asc then resource.
ListTasks: CreatedAt asc, fold order, id.

### 3.11 Approvals fold (ask/ask.go)
Skip `seq <= last[dev]`. posted: needs askId; `by != "irori"` must have `DeviceOf(by)==dev` (else ignore); first post wins; fold fields reset, Device=dev. answered: needs askId, actor `user` and `IsOwner(by)` (default owner `user:<self>`), appended in fold order regardless of whether the post is known. resolved: only from the asking device, first resolution wins. revive.requested → flag duties. Duties (asking device, every 1s/after rows, sorted ids): first answer → resolved{choice, answered:true, answerRef}; none and `now >= until` → resolved{answered:false}; revive asks call `Revive(target)` only if first answer is `Revive` and names the same target+deliveryId; else OnResolved callbacks. postRevives: newest-first list of revive.requested targeting this device's sessions by the person → post once with id `revive-<dev>-<seq>`, until = now+30m. RequestRevive refuses a target on this device. refoldIfCut like coord. Subscribe chan buffer 64 of ask ids.

### 3.12 roster (roster/roster.go:132-241)
Fold only other devices' rows, skip `seq <= last[dev]`. device.started → drop every card of that device. agent.seen → needs key, `DeviceOf(key)==dev` (else ignore+count); scrub LastText/Transcript/PID; status ""/gone → idle. agent.status → card must exist and key's device == dev; gone → delete; set Status, Name, NameSource; PermissionMode only if non-empty. agent.gone → delete. `Remote()` sorted by key, skip revoked devices, DeviceState = Liveness(dev) or `offline`. `Agents()` = Local() then Remote(). Resync (seq 0) → full rebuild.

### 3.13 names / devcolor
`Pick(seed, taken)` (names/names.go:39-54): start = FNV-1a-32(seed) % 127; probe the list forward with wrap; all taken → same probe with suffix `2`, then `3`… (`taken` gets the lowercase name). `Fold` = lowercase(trim). Words list (names.go:16-33) is append-only, 127 entries, order as written (note `tundra` before `tulip`). Vectors: FNV32a("s1")=139573449 → `mesa`; "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0" → 3936500947 → `lemur`.
devcolor (devcolor/devcolor.go): Palette `teal orange pink lime cyan indigo sand` (append-only). Assign: sort devices by join time then id; valid picks first (they reserve the colour; two devices may pick the same); others in order take `Palette[fnv32a(id)%7]` or the next unused, falling back to the hash colour when all used. Picks fold: actor `user`, deviceId set, colour "" or valid; newer by (hlc, writing device, seq). Vectors: fnv32a("pc")=1313756516, ("mac")=3970106408 → both `pink`; pc joined first → pc pink, mac lime.

### 3.14 decision
Validate trims; statement non-empty ≤500 runes, why ≤2000. ParseLines vectors decision_test.go:51-90 (`"40 - 52"` ok, `"007"`→7, `"+40"`, `"1-2-3"`, `"0"`, `"4 0"`, overflow bad). MakeAnchor/Check algorithm at decision/anchor.go:170-337: moved search outward from Start, ties to the earlier, hashing at most 64 MiB; files >8 MiB → changed; missing root → unknown; path must re-clean to itself. View: Add idempotent on id; supersededBy in add order; List newest `at` first then later add first; Query case-insensitive substring of statement or why; superseded hidden unless All.

### 3.15 needsYou (board_impl.go:1017-1062)
`ons` = On of every open item (task Blocked only); `asks` = those whose item Status ≠ `answered`.
- Agent viewer (`For` non-empty, not IsOwner): true if (state≠Blocked && holder && in Assigned) ‖ some `ons` is `{agent, For}` ‖ some open item has Status `answered` and (SentTo==For ‖ SentTo=="" && RaisedBy==For); else false.
- Person / empty viewer: true if state InputRequired ‖ some `asks` is kind person with (For=="" ‖ Ref=="" ‖ Ref==For ‖ IsOwner(Ref)&&IsOwner(For)); else if For=="" false; else if IsOwner(For) → any holder is an owner key; else isHolder(For).
Group test order: EffectiveState terminal → Done; needsYou → NeedsYou; EffectiveState Blocked → Blocked; holders → Working; else Open. Card extras: `Overlaps` (shared task with ≥2 holders: pairs `"<res> ~ <res>"` of different holders' unexpired conflicting leases, sorted by resource); `Resolved` when every open item went away else `Gone` ids; LastComment = last of byTask.

### 3.16 Subscribe (subscribe.go:379-482)
Channel buffer 64 per subscriber with a `lost` flag. `note()` queues a Change only when not replaying. Every lock release goes through `unlock()`: deliver each pending change to each subscriber (`deliver`: if lost, send `{kind:"resync"}` instead; clear lost; non-blocking send; full → lost=true), clear pending, `armExpiry`, unlock. `armExpiry`: while ≥1 subscriber, a one-shot timer at the soonest `Until` among held leases of local holders (other devices' leases ignored); none → stop. Timer → lock, prune(stamp) (emits lease_expired), unlock (re-arms). Cancelling the ctx removes the subscriber and closes its channel. AskClaim waits on an internal subscription.

### 3.17 decision anchors (decision/anchor.go)
`relPath(root,p)`: `\`→`/`, trim; root "" or p "" or any `..` segment → ErrBadPath; absolute native path → must lie strictly under clean(root) (prefix with separator; Windows case-insensitive) → relative; rooted without volume (`/x`) or volume-relative (`C:foo`) → ErrBadPath; `path.Clean`; `.`/leading `/`/`..` → ErrBadPath.
`MakeAnchor`: ErrBadLines if start<0, end<0, start==0&&end≠0, start>0&&end<start; file must exist and not be a dir (ErrNoFile); path rewritten by onDiskCase; Root = clean(root). start 0 → Hash = blob(LF(data)). Else lines = splitLines; end>len → ErrRange; Hash = hashLines(lines[start-1..end]); Context = up to 2 lines before (from max(1,start-2)) then up to 2 after.
`Check(a, root)`: Unknown if root "" ‖ hash "" ‖ Start<0 ‖ (Start>0 && End<Start) ‖ root not a dir ‖ relPath(root,a.Path) errs or ≠ a.Path ‖ stat error other than not-exist. Gone if not exist or a dir. size>8 MiB → Changed. Start 0: blob(LF) equal → OK else Changed. Lines: n lines; `End-Start >= n` → Changed; `size=End-Start+1`, `last=n-size+1`; Start≤last && hash at Start → OK{Start,End}. `off[i+1]=off[i]+len(line i)+1`. `lo,hi = Start-1, Start+1`, or `last, last+1` when Start>last. Loop while lo≥1 ‖ hi≤last: pick lo if lo≥1 && (hi>last ‖ Start-lo ≤ hi-Start) (then lo--), else hi (then hi++); budget -= off[s-1+size]-off[s-1], stop (Changed) when <0; hash match → Moved{s, s+size-1}. End → Changed.

### 3.18 Notice and status texts (verbatim; `%q` = Go strconv.Quote, `deeper` = `"\nshow task:<id> for the card, its comments and links."`)
- assigned (board_impl.go:139-150): `"%s assigned you a task: %q (%s)."` by,title,Refs; `"\nObjective: %s"` (cut 600) if any; `"\nDone when: %s"` (cut 600) if any; `"\nClaim it to start."` if `to` still in Assigned; deeper. To = [to] unless to is owner or to==by.
- comment (board_impl.go:403): `"%s commented on %q (%s):\n%s"`+deeper.
- unblocked (board_impl.go:318-328): who = by or `Irori` for irori; `"%s unblocked %q (%s)"` + (`": %s"` note) + `". It was blocked on: %s. It is %s again."` (cut(old Reason,600), restored) + deeper.
- answered, not resolving (cards_impl.go:781-786): `"%s answered the blocker on %q (%s, blocker %s).\nQuestion: %s\nAnswer: %s"` (answer cut 600) + pickedLine + `"\nIt is not resolved yet. When you have what you need: board_unblock task_id=%s blocker_id=%s note=\"<one line on what you will do now>\". If the answer doesn't settle it: board_comment task_id=%s blocker_id=%s with what is still missing."` + deeper.
- answered, resolving (cards_impl.go:800-817): after a person's answer `"%s resolved blocker %s on %q (%s)"` + (`": %s"` note) + `"\nQuestion: %s\nThe person's answer: %s"`; otherwise the first line of the previous form + pickedLine; then `"\nNo blockers remain: it is %s again."` or `"\n%d more blockers are still open."`; deeper. To = raiser (agent ≠ by) then recipients.
- pickedLine (cards_impl.go:823-831): recommended → `"\n(That is the answer %s recommended.)"`; offered → `"\n(That is option %d, offered by %s.)"`; own words → "".
- reopened (impl.go:1440-1452): who = by or `The person`; `"%s reopened %q (%s), which was %s. It is %s again"` + (`", with you on it: pick it up, or board_status it done again if there is nothing left to do."` if holders else `", with nobody on it."`) + deeper.
- returned (board_impl.go:1187): `"Nobody is on %q (%s) any more: %s went away, and it's open again. Take it with board_join, hand it on with board_assign, or close it with board_status."`+deeper.
- sweep partial close (board_impl.go:1121): `"Irori closed %s on %q (%s): what it waited on went away. %d more blockers are still open; board_unblock with blocker_id answers one."`+deeper; items `"b2 (lease released)"` joined `", "`.
- person comment (board_impl.go:487-525): head `"The person commented on %q (%s)"` + (`", which was %s and is open again"`). Room: `"%s, the ask nobody has taken yet:\n%s\nIf you can take it on now, ask_claim %s. If you're busy, leave it."`+deeper (kind ask_comment, Ref task:<id>, Person). Answerers: head+`":\n"`+text+`"\nAnswer it on the card with board_comment."` + (`" One answer is enough: read the card first."` if >1) + deeper. Watchers: head+`", a card you watch:\n"`+text+`"\n"` + (`"<answerers joined \", \"> answers it; answer on the card only if it asks you."` or `"Nobody else is on it: if it asks something you can answer, answer on the card with board_comment."`) + deeper; Wake only if no answerers.
- ask (ask.go:336-348): `"The person asks the room (%s)"` + (`", again: "+why`) + `":\n%s\n"` (cut(objective,600)) + `"If you can take it on now, ask_claim %s: one agent takes it, and you hear if someone else got there first. If you're busy, leave it."` + deeper; By = creator, Person.
- taken (ask.go:590-601): mine `"You have the ask %q (%s). "+youHaveIt` where youHaveIt = `"Talk it through with the person on the card first if it isn't clear what they want, then ask_close it with what it became: a card, a change to another card, or nothing."`; other `"%s took the ask %q (%s) first; nothing to do unless you want to chime in (board_comment)."`; +deeper.
- askLine (ask.go:1301-1320): closed `"closed (<kind>): <note>"` + `" (task:<id>)"`; taken `"the person asked the room; <taker> took it (ask_close ends it with what it became)"`; waiting `"the person asked the room; waiting for device <owner> to hand it out"`; unclaimed `"the person asked the room; nobody took it yet (%d heard it): ask_claim takes it"`; else `"the person asked the room; %d agents heard it, nobody has taken it: ask_claim takes it"`.
- askRefusal (ask.go:1326-1328): ErrNotAllowed `"%s is an ask: %s; use ask_claim to take it and ask_close to end it"`.
- AskTitle cut detail (ask.go:278-282): take the first 79 runes; `LastIndexFunc(IsSpace)` returns a **byte** index into that string; if it is > 40 (bytes) cut there, trim, add `…`; else the 79 runes + `…`.

### 3.19 Withdraw and repeated rows
Withdraw (boardlog.go:119-144): board only (else ErrInvalid); note ≤8000; already withdrawn → no-op; `mayCancel(t, this device)` else ErrNotAllowed; task.status canceled withdrawn, note default `"withdrawn from the board"`. Repeated task.created for a known id rebuilds the Task struct from that row (state, holders, steps, blockers reset until later rows refold; fold order kept); live, `claimShort` then renumbers everything to restore its ref (short.go:72-77).

---

## 4. Platform-specific code
Only `decision/anchor.go`: `under()` compares the root prefix case-insensitively on Windows (`runtime.GOOS=="windows"`, anchor.go:243-249); `relPath` rejects `\`-converted rooted paths, `C:foo` (`filepath.VolumeName`) and absolutes outside root (anchor.go:209-230); `onDiskCase` rewrites each segment to the on-disk spelling when exactly one directory entry matches case-insensitively (anchor.go:341-366), so Windows/macOS anchors store one spelling. The glob overlap is case-insensitive on every OS by design (glob.go:186-188). No other OS code in the area.

---

## 5. Rust crates (versions checked on crates.io 2026-10-29)
| Need | Crate | Notes |
|---|---|---|
| JSON | `serde` 1.0.229, `serde_json` 1.0.154 (`raw_value`) | keep `Box<RawValue>` payloads; BTreeMap for `seen`; custom null→empty Vec deserializer |
| Times | `time` 0.3.55 (`formatting`,`parsing`,`macros`) | custom Go RFC3339Nano writer (trim trailing zeros, `Z`); parser accepting 0-9 fraction digits and offsets; match sibling inbox.md choice |
| Regex | `regex` 1.13.1 | write `[0-9A-Za-z_]` instead of `\w` (prPathRe cards_impl.go:1279) — Rust `\w` is Unicode |
| Random | `getrandom` 0.4.3 | 10 bytes for newID; `ulid` 3.0.0 optional (same string layout; test against id.go vectors) |
| Hash | `sha2` 0.11.1, `sha1` 0.11.0, `hex` 0.4.3 | align digest generation with store.md (sha2 0.11) |
| FNV | none — hand-roll FNV-1a 32 (offset 0x811c9dc5, prime 0x01000193); `fnv` 1.0.7 is 64-bit only |
| URL | do not use `url` 2.5.8 for ParseLink validation (WHATWG: `https:example.com` parses with a host); port the needed subset of Go `net/url.Parse` (scheme, `//authority`, userinfo, host, path) |
| Case fold | hand-port Go `unicode.SimpleFold` orbits (generate a table from Go once, or from CaseFolding.txt C+S + Go's orbit rules); `unicode-case-mapping` 1.0.0 is not orbit-equivalent |
| Sync | `parking_lot` 0.13.0 or std Mutex; `tokio` 1.54.1 (`sync`,`time`) for subscribers/timers |
| Errors | `thiserror` 2.0.21 | error kinds must stay matchable like Go's `errors.Is` (ErrNotFound, ErrHeld, ErrAmbiguous, ErrBadResource, ErrBadState, ErrInvalid, ErrNotAllowed, ErrSubtask ⊂ ErrInvalid) |
| Tests | `proptest` 1.12.0 (glob overlap symmetry, replay==live), `insta` 1.49.0 (notice/show text snapshots) |

---

## 6. Tests to port (what each proves)
Harness (coord_test.go:18-139): fixed clock 2026-10-26T12:00Z, store on device `dev-a` with HLC from the clock, roster of `dev-a/alice-session`, `bob`, `carol` all idle, person `user:dev-a`, repos `remote:github.com/a/b` and `root:4b825dc…4904`; `reopen()` = New over the same store (replay check). Bodies of tests marked † were read only by name.

**coord_test.go**: CreateTask (fields, trimming, limits); ClaimSingleWriter (second claimer ErrHeld); ClaimFromGoneHolder (claim after GoneGrace); PersonHolderNeverGone; ConcurrentClaimsOneWins; Assign; SetTaskState (permissions, Blocked refused); ListTasks (filters, order); LeaseAcquireRenewExpire; LeaseConflicts (globs); LeaseSteal; Release; SweepReleasesGoneHolders; NilRosterNobodyGone; GoneGraceOverride; ReplayEquality (reopen folds identical); ReplayPagesPastListLimit (>1000 rows); ReplayTolerates (bad rows dropped).
**glob_test.go**: Overlaps table (glob_test.go:8-95, both argument orders) — port verbatim; ParseResource ok/bad tables (97-174) — verbatim; NewIDSortable.
**links_test.go**: ParseLink/LinkLabel table (links_test.go:8-59) — verbatim, incl. `https:example.com` bad, `https://github.com@evil.com/x` bad, `board.go:580` path.
**short_test.go**: counts up and resolves; clashes settle identically in any arrival order; older rows (no num) rank 1,2 then new is 3; suffix vectors; clash heard in watch digest and bare number refused; refs after refold; incremental == replay.
**board_test.go**: holders/claim/assign; shared claim not held; holder filter/permissions; observe tracks every holder; CreateTask board fields; assign permissions + notice text; long fields cut; v0.2 assign rule; join/leave/share; block validation; block replace/no-op/leave; unblock; comments; RecordMessage; no holders tells creator; links; refusals and one row per holder; board groups, done cap, cards+leases; sweep closes resolved blockers; sweep expired lease; search docs/show; replay equality/tolerance; person-comment hearers.
**board_review_test.go**: unblock keeps unclaimed assignee submitted; shared assignee not engaged until claim; Assigned follows Holders; key checks; block on terminal task refused.
**cards_test.go**: steps set/create/keep ids/tick; raise validation, dedupe, caps; recommend (+cap, duplicate merge, flag); answer (unclaimed → submitted, holders told); leaving blocked closes items; sweeper closes only what went away; raise defaults to person; Block keeps one reason item; resolved trimmed to 10; watch rules/notices/skips own changes; replay; legacy blocked rows; activity cap 500; PRLink; MergeWatch vector (cards_test.go:1003-1024); digest only from the watcher's device.
**blocker_test.go**: blocker status open/answered/replied; spoofed blocker comments ignored; comment before raise; resolve needs note after answer; answer notice recipients; NeedsYou by status; old-peer resolve; reblock asks afresh (asked position); reblock from old peer; links and files.
**ask_test.go** (coord room asks): post tells every free agent; two local claims one taker; taker gone hands it back; offline taker keeps it; unclaimed + AskAgain; outcomes; take rows from wrong device and old claims; two arrival orders + loser hears; person comment on open ask goes to room; taker's close before the take on a third device; edits need the taker; late tells wake only in window; failed sweep keeps the retell.
**boardlog_test.go**: board actors; set-aside rows resolve later; owner rule is the person; cancel only creator's person or owner; ParseRef; set-aside cap.
**holders_test.go**: gone holder returns card to creator / to person; blocked and shared cards; only on its own device.
**owner_test.go**: owner keys vs made-up `user:` keys; lease rows from wrong device ignored; thread rows carry the event's device.
**pins_test.go**: pin/unpin/move ranks; pins across devices (newest by cursor). **reopen_test.go**: answered ask completes; reopen keeps holders; subtasks and parents; person comment reopens finished card; watch a finished card. **review_test.go**: repoKeys with colons/spaces; control chars in globs; steal is one acquired row; gone card starts grace clock; expired leases pruned; same-millisecond creation order; assignee claim → working; HLC tie at page boundary. **subscribe_test.go**: mutations notify; sweep/expiry/gone; expiry timer fires without a row; slow reader gets Resync; channel closes on cancel. **subtasks_test.go**: split + rollup; one level; cancel cascades; two-level link settles in any order; subtask changes reach parent holder; asks and subtasks; show subtasks; rejected reparent keeps old parent; adoption reaches parent holder. **thread_test.go**: gone merge either order; fill merge either order; refold drops thread entries of cut rows; renderTask shows messages.
**ask/ask_test.go** (approvals): post needs fields; answer here resolves; non-owners refused; remote answers; writer rules; unanswered resolves at until; revive from another device (non-person ignored); RequestRevive; default owner = this device.
**decision**: Validate; ParseLines table; MakeAnchor paths/whole file/lines; line hash vector; CRLF==LF; Check (ok/moved/changed/gone/unknown), hostile ranges, start past end finds nearest, scan budget, large changed bounded, missing root unknown, on-disk case; View add/get, supersede, CanSupersede, list filters, limit, concurrency (`slow` = 10 under -race, race_on_test.go).
**roster**: card from another device ignored; card row scrubs local fields; cards retire (gone, device.started); liveness and revoked; own rows not folded; agent.status carries later permissionMode and nameSource.
**names**: words distinct/plain; deterministic; probes past taken; numbers when all taken; Fold. **devcolor**: unique defaults while palette lasts; adding a device never moves an older one; a pick wins and defaults avoid it; picks keep newest user row (devcolor_test.go:64-82).
**gitsha**: blob vectors, Pair.

Port with a Go-written fixture: dump each test's resulting rows (raw payload bytes) from Go, replay them through Rust and compare `Task` JSON — the strongest check that replay matches.

---

## 7. Hard parts and compatibility traps
1. **Raw payloads**: signatures hash stored bytes (store/store.go:297-311); never re-serialize. Rust-written rows will differ from Go's bytes (HTML escaping, float form, time digits) — fine for Go peers, but tests must not compare raw bytes across implementations.
2. **Time format**: Go trims fraction zeros (`.12Z`, `.0012345Z`); Rust crates print fixed 3/6/9 digits. Write a custom formatter. `stamp()` keeps nanoseconds; equality checks (`AcquiredAt.Equal`, stolenKey RFC3339Nano strings, mesh.go:50,342) need exact round-trip.
3. **Null vs empty**: `task.steps.steps` may be `null` (subtasks.go:424-430); `*bool`/`*string` fields distinguish absent from `false`/`""` (cards_impl.go:85-86, ask.go:173-175) — use `Option<bool>`/`Option<String>`. `rank` 0 is omitted; `shared`, `told`, `taker`, `since`, `option`, `restored` (blocker.closed) are always written.
4. **Order-dependence**: replay folds in log order, live folds in arrival order. Designed-convergent parts (activity by position, take lock by position via `takerAt`/`closedAt`, short refs, relink sorted, pins by cursor, mirrored-message merge) must keep their position logic; comment `byTask` order and `Comments` count are arrival order live (not convergent by design). Replay must suppress notes, watch lines and "moved" notes exactly where `replaying` is checked.
5. **Deferred rows**: askGate defers rows until the ask's next take.state/close; `foldDeferred` loops; aside rows retried on every fold and on non-followed rows (mesh.go:104-110). Avoid recursion depth issues (Go recurses fold→foldDeferred via apply only once per row).
6. **Lease liveness asymmetry**: another device's lease never expires by local clock (mesh.go:62-67); `armExpiry` ignores them (subscribe.go:432); lapsed own leases older than 24h at replay get no expiry row.
7. **Go string formatting**: `%q`/`strconv.Quote` (escapes `"` `\` and non-printables as `\t`, `\x01`, `\u…`; keeps printable Unicode like `é`) in notices, errors, show text, split's unplaced list. Vector: `"tab\there \"q\" é \x01"`. `MergeWatch` parses the digest format — any drift breaks merging with Go peers' still-unread digests.
8. **Case folding**: glob literals use full SimpleFold orbits (`K`/U+212A, `s`/`ſ`); `optionIndex` uses `strings.EqualFold` (same orbits); Rust `eq_ignore_ascii_case`/`to_lowercase` are not equivalent.
9. **Go `path.Match` validity** must be reproduced for NormalizeGlob (`[`, `a/[/]`, `[]`, `[-a]`, `[a-]` bad; `[b-a]` OK there but unparseable in overlap → true).
10. **URL parsing**: ParseLink relies on Go `net/url` (scheme without `//` → empty host → bad; `user@` rejected; `Hostname()` lowercase strip `www.`). Labels: Linear `/[^/]+/issue/([A-Za-z][A-Za-z0-9]*-[0-9]+)`, GitHub `/[^/]+/[^/]+/(issues|pull)/([0-9]+)`, Notion last path segment minus `-?[0-9a-f]{32}$`, dashes→spaces (links.go:258-347). Ref is stored as typed (trimmed), commits lowercased, paths `\`→`/`.
11. **Store actor vs payload `by`**: person-ness is the signed actor + IsOwner, never payload `by` alone (personRow). On a board `actorFor` returns the key itself. `writerOK` with bad JSON returns true.
12. **Ignored counters**: `ignore()` is called on writer failures, wrong-device take rows, cancel without right, unauthorised pins/steals/edits — but NOT on withdrawn-stick drops or unknown take ops. Devices window shows these counts.
13. **Gone clocks are in-memory** and start at the first observe after absence (impl.go:1027-1037); Core must call SweepNotices every few seconds or graces stretch. `firstAbsent` survives refold.
14. **Two ref separators**: `<dev>:<seq>` (Returned.Row) vs `<dev>/<seq>` (approval answerRef). Approval ids use base36 UnixNano (Go `strconv.FormatInt(…,36)`, lowercase).
15. **Ask posted payload includes fold fields** (`device:""`, `resolved:false`) because the whole struct is marshalled (ask/ask.go:508-509); readers must overwrite them from the envelope.
16. **FNV-1a-32** for names/devcolor/colours; list order is part of the format (append-only). Integer `%` on uint32 then cast.
17. **HLC tie at page boundary**: always page by the full `(hlc, device, seq)` cursor (review_test.go:212).
18. **unicode.IsSpace / IsControl**: Go IsControl = Cc (same as Rust `char::is_control`); IsSpace = White_Space incl. U+0085, U+00A0 (Rust `char::is_whitespace` matches). Operate on bytes where Go does (splitLines, invalid UTF-8).

### Not determined
- Exact JSON field set of rows written by other packages into `task.comment` via core (e.g. who sets `unheard` besides PersonComment) — only coord's writers were read.
- `store.Unresolved(ev)` semantics for board aside rows (owned by the store/board spec).
- Bodies of most coord tests (names only); `presence` producer details of `UpdatedAt` on gone cards.
