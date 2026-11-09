# Event-driven window: signals that say what changed: design

*2026-11-08, card #1138. It comes from the person's question on #1134: "I don't know why we update that many times when we're just looking at it… Can we get more event driven?" Design only: nothing here is built yet. The build cards are in §9. Checked against main at `3f8d9224` (after #1136) by ten readers: five mapped the code and five checked the design's inputs. Line numbers are approximate, so go by the names.*

Irori's devices already push changes to each other. A row written on one device reaches the others over the mesh with no polling, which is the "webhook-like system" the person asked about. The waste is inside each device. The Core tells its window only that *something* changed. `board:changed`, `talk:changed`, `notifications:changed`, `devices:changed`, `boards:changed` and `assistant:changed` all carry `{}`, so on each one the window reads its whole lists again:

- all ~830 cards;
- every thread in every space;
- every conversation with all its posts;
- the notifications;
- the desk;
- the full card behind every card on screen.

This design gives each signal the ids it touched and a sequence number. Each list then reads just those rows. Anything the Core can't name, any lost event and any gap the page notices still means one full read, so a list can't go stale.

## 1. Decisions

| | Decision | Why |
|---|---|---|
| D1 | **Every list signal carries `seq`, `all` and the ids it touched** (§4). `all: true` means "read everything", as today. | The ids are already in the coalesced batch (`irori_core::events::coalesce` hands the pump a `Vec<CoreEvent>`); the pumps throw them away (`|_|`). |
| D2 | **When in doubt, `all`.** A kind the pump can't attribute to ids, any `Resync` in the batch, a whole-table change and any lost event set `all`. | Patching is only safe when the ids are complete. A wrong `all` costs one full read (today's cost); a missing id costs a stale row. |
| D3 | **Every hop that drops events silently gets a lost flag, so a drop becomes `all`** (§3.1). | Today a dropped event is harmless because the next one re-reads everything. Once events carry ids, it is a stale row that nothing brings back. |
| D4 | **`seq` is counted per event name on `Window`, when the event is sent. A gap at the page means `all`.** The page takes the first `seq` it sees as its starting point. | Events from the pump to the page arrive in order. A gap means the page didn't hear one: a held `OnShown`, a late listener, a reloaded webview. One shared listener module, attached before the first read, makes "the first seq seen is the start" sound (§5). |
| D5 | **The window expands the touched ids to every card or thread whose row shows them** (§3.3). The page asks for exactly the ids it is sent. | A row shows other cards: parent, subtasks, followers, wait chains, made cards. Only the window has the graph. It needs the expansion for the tray count anyway (§6.1). |
| D6 | **The card grid keeps its order in Rust**: a summary cache in the window, a patch read that returns the changed rows and the whole order (§6.1). | The TypeScript port of the sort used by the sample already differs from Rust in four ways (ms vs ns times, no id tiebreak, kids by `parseInt(short)`, kids by `parent.id`). Two copies of the sort would drift apart without anyone noticing. |
| D7 | **A slow full read while shown checks the patches.** Every 5 minutes, each patched list does one full read and logs how many rows differed (`drift`). | It is the test for "lists never go stale", and it catches what a patch missed. A bug in the expansion shows up in the log, not in the person's view. |
| D8 | **Measure first** (§8, card 1). Every signal is counted, every list named, every read path counted, and IPC is counted per command, all in the per-minute `window: lists` line. | Today the line counts 3 of 7 signals and names 3 lists, and the card store, desk, boards, refs, freshness and follower reads don't show. The Mac log's slowest read on 11/08 was 209 s, logged as "a list". A before/after needs names. |
| D9 | **No setting.** It is how the window reads. | Decision 52. |

## 2. What is there today

Checked on main at `3f8d9224`.

### 2.1 The signals

The pumps are in `crates/window/src/events.rs` `start`. Each one subscribes to a Core feed (`Core::feed(&[Topic::…])`, a 64-slot queue) and coalesces at 250 ms (`BOARD_EMIT_EVERY`, `INBOX_EMIT_EVERY`).

| Event | Payload now | Carried by the Core event | Who publishes |
|---|---|---|---|
| `board:changed` | `{}` (`Empty`) | `CoreEvent::Board(coord::Change{kind, task_id, resource, holder, reason})`. There is no board id. | `feed-board` pump (`service.rs`) for the private board. Shared boards **skip the hub**: `Boards::tell` → `subscribe_boards` → the window relay pushes into the board feed's own queue. `AGENTS_CHANGED` comes from `spawn_capable.rs`. |
| `boards:changed` | `{}` | a zero `Change` from `Boards::moved()` (about 30 sites), a pin in the board batch, or `Window::boards_pin` directly | — |
| `talk:changed` | `{}` | `CoreEvent::Talk`, no data | about 20 sites (§3.2). Most local writes are published twice: once by the writer, once by the store follower. |
| `notifications:changed` | `{}` | `CoreEvent::Notifications`, no data | `record_notification`, `settle_notifications` (on every Board, Inbox and Asks change), `read_notifications`, the follower for synced rows |
| `devices:changed` | `{}` | `CoreEvent::Devices` | avatar, person colour/avatar, devname and sides rows. **Not** device colour (`devcolor`), which applies with no publish. |
| `assistant:changed` | `{}` | `CoreEvent::AssistantChat` | chat rows (none written in production yet) and every resync. No frontend listener yet. |
| `inbox:changed` | `InboxCounts{unread, held, paused}` | `CoreEvent::Inbox(inbox::Change{kind, to, delivery_id})`, which already carries ids | `feed-inbox` |
| `roster:changed` | `{change, agents}` | `Presence` | not coalesced. Already a payload. |
| `capture:changed`, `talk:replying`, `call:state`, `usage:changed`, `meters:changed` | payloads | — | already driven by their payload: the model to copy |

Side work in the pumps:
- The board pump calls `Window::board_changed` → `decide_count` on **every batch** while the tray is listening. That runs the whole shared-board summary pipeline (`board_cards`) just to count, whether the window is open or not.
- The inbox pump runs `refresh_inbox`, a full inbox scan, for the counts.

### 2.2 The lists and what wakes them

Frontend, `frontend/src/lib`. **B** board, **D** devices, **I** inbox, **T** talk, **N** notifications, **Bs** boards, **R** roster, **F** focus.

| List (hook) | Wakes on | Reads | Notes |
|---|---|---|---|
| grid, `useCards` (`cards.ts`) | B D F | `board_cards` = `cards_in(SCOPE_ALL)`: every live card with `people()`, then per card `activity`, a full `comments` clone (`talking`) and `family_of` (order chains, `wait_chain`, loops), then a global sort, pins and families | each read also JSON-compares the whole answer (`sameAs`) |
| card store, `useCard` / `createCardStore` | B D I F | `board_cards_by_id(ids, have)`. #1136's rev gate returns ~100 B for an unchanged card, but the host still builds the **full `card()` of every watched id** (one `people()`, `Followers::read` and `order_loops` per id) | watched: the open card, every Notifications row, every Needs you row, every thread view |
| all threads, `useAllThreads` | B D **I** F, **T** (`Events.On`, not held while hidden) | `board_threads_all`: `people()` N_boards+2 times, and builds **every thread ever made**, resolved too, before cutting to 20 resolved per log | `ThreadSummary` holds **nothing** from the inbox, so the I wake is wasted. Mounted app-wide. |
| board threads, `useThreads(board)` | B D I F | `board_threads(board)` | doesn't hear T, so `unread` goes stale |
| Ask Irori follower (`assistant-follow.ts`) | B D | `board_threads_by_id` (= `threads_all` filtered: no saving on the host) + `cardsById` | 3 s minimum gap (#1136) |
| idle line, `useIdleReport` | B D F, roster status | `board_idle_report` | an aggregate, with no rows to patch |
| closed tab, `useDoneCards` | B D F | `board_done_cards`: every task of every log | only while shown |
| `#ref` chips, `useCardRef` | B D | `board_card_ref` per distinct ref, one `people()` each | #1136: one refresh per 1.5 s burst |
| conversations, `useConversations` | T F | `talk_conversations`: every channel's and DM's **full post list**, `mirrored()` (O(all deliveries)), every thread in every log | the window never uses the thread rows (`listed()` keeps channels and DMs) |
| posts, `usePosts(conv)` | T F | `talk_posts(conv)`, all posts then paged | wakes on talk in **any** conversation |
| huddles, transcript | T | — | wake on any talk |
| notifications, `useNotifications` | N **B** **I** F | `notifications_list` (500, `get_task` per row, asks) | B for card titles, I so `DeliveryLive` re-fetches delivery outcomes |
| unread badge | N F | count | fine |
| desk, `useDesks` | B, 30 s poll | `desk_list`: per desk, every task of every log (`desk_cards`) plus a PR scan | not deduped. Not counted in the log. |
| notes, `useNotes` | B | `notes_list`: a `get_task` and a **filesystem `exists`** per `path:` tag, on every board change | — |
| inbox, held, repo names | I | `inbox_list(200)` + held + policy | already keyed by delivery id |
| boards, `useBoards` | Bs, and **every reader mount** | `boards_list` | `OwnerChip` on **every grid tile** mounts a reader, so N tiles fire N reads on mount |
| personal boards, `usePersonalBoards` | Bs | `boards_personal` | up to 5 separate subscriptions |
| devices, `useDevices` | R D, 5 s poll | `devices_devices` | 2+ mounts. The 5 s poll is the steady 12 reads/min seen in the log. |
| freshness | Bs, 30 s poll | `sync_freshness` | — |

Since #765 and #1134, the reloads wait while the window is hidden (`Events.OnShown`, `whileShown`) and run once when it shows. This design is about the window **while the person is looking at it**.

### 2.3 What the log shows

Mac `irori.log`, 3,339 `window: lists:` lines, 11/06–11/08 (the PC log the card quotes said 40–460 reloads/min, reads up to 1–7 s):
- `board:changed` per minute: median 4, p90 18, max 74. `inbox:changed`: median 0, p90 9, max 106. Talk, notifications and boards aren't counted.
- Reads per minute: median 40, p90 159, max 732. 1,128 lines have reads but no counted signal, most of them exactly 12/min (the devices poll).
- 396 lines have a read over 1 s. The slowest is almost always "a list" (unnamed), up to 209.6 s on 11/08. `threads-all` reached 57 s.

The 100–200 s reads are lock contention or a catch-up, not read count (§10, Q3). Fewer reads take load off the coord lock, but this design doesn't promise to fix them.

## 3. The Core side

### 3.1 No silent drops

Each of these becomes `all` instead of a lost event:

| Hop | Today | Change |
|---|---|---|
| Shared board changes: the "changes" thread in `start_board` → `Boards::tell` → window relay → the board feed's queue | `Boards::tell` ignores `push`'s result. The relay pushes straight into the hub feed's queue, bypassing `Hub::publish`'s lost flag. | **Reroute**: the changes thread calls `s.hub.publish(CoreEvent::Board(ch))`. Delete the relay's `board_q` push. `Boards::tell`/`moved()` stay for `boards:changed` only, and `moved()`'s zero `Change` stops going into the card feed. Then one producer path feeds the Board topic and the hub's existing lost flag covers private and shared alike. |
| Inbox fan-out, `Inner::unlock` (`inbox/src/state.rs`) | `let _ = q.push(c)` | a lost flag per subscriber, and a new `ChangeKind::Resync` swapped in while it is set |
| `Kick::notify` (window `events.rs`) | ignores `push` | an `AtomicBool` the inbox pump reads and clears, so a kick can't be lost |
| `Boards::tell` (now only `boards:changed`) | ignores `push` | a lost flag, so a lost one still sends `boards:changed` |
| approvals `Book::flush` | ignores `push` | not needed: no window list reads asks except through `settle_notifications`, which is idempotent. Left as is. |

Both resync forms must mean `all` in the pump: `CoreEvent::Resync` (the hub lost one) and `Change{kind: "resync"}` (coord's `Fold::flush` lost one, `refold`, `catch_up`).

### 3.2 Every change names its ids

**Board.** Every `TASK_CHANGED`, `COMMENT_ADDED` and `BOARD_PIN_CHANGED` already carries `task_id`. Card ids are unique across boards, and `Window::read` routes by id, so no board id is needed. The gaps, where a card's view changes with no `note()` for it:

| Where | Change |
|---|---|
| `mention_cards` (`fold.rs`) adds "mentioned this in #X" to card Y's activity | note `TASK_CHANGED` for each Y |
| `fold_wait` (`waits.rs`): no note at all for created, fired or canceled | note `w.task_id`, and each open card whose holders include `w.by` (read the wait before removing it) |
| `Coord::spawn_now` (`asked_now` feeds `cant_start`) | note the card |
| `fold_desk` (`desk.rs`) | note a new kind `desk`, so the desk can listen for it (§6.6) |
| `devcolor` rows (`service.rs`) | `hub.publish(CoreEvent::Devices)` |
| a usage hold set, released or expired (`spawn_limits.rs`) | `AGENTS_CHANGED` (→ `all`) |

**Talk.** `CoreEvent::Talk` becomes `Talk(TalkChange{convs, calls, all})`.
- Every `talk.*` row names its conversation (`convId` or `channelId`) except `talk.card`, which is rare and becomes `all`.
- A thread's read cursor is `convId = <thread task id>`, so thread ids appear in `convs`.
- Whole-table sites set `all`: `talk.load()` on a store resync, `drop_space`, `dm_refold`.
- A DM mirrored from an agent's message: the conv id is `dm_id(from, delivery.to)`. The store follower sees `message.sent` before the inbox has folded the delivery, so publish it from `feed-inbox` on the delivery's change instead, and drop the follower's `message.sent` path.
- The pump takes the union over the batch, which also folds away the double publishes.

**Notifications.** `CoreEvent::Notifications(NotifChange{keys, all})`.
- The keys are in scope at every site: `r.key`, `row.keys`, `keys`, the synced row's `key`/`keys`.
- A store resync sets `all`.

**Inbox.** `inbox::Change` already has `delivery_id`. Changes with an empty one (wake issued, turn engaged/ended, receive settings) touch no delivery. `POLICY_SET`, the mesh re-fold and the new `Resync` set `all`.

### 3.3 The window expands ids

The board pump folds a batch into a set of touched task ids **T**, then expands it into the ids it sends:

1. **Threads that made a card in T.** There is no reverse index from a made card to its thread today: `Task.from` is not set when a thread changes an existing card, only `th.made` is. Build `made_by: HashMap<card, Vec<thread>>`, kept in `fold_made`, or scanned from threads once per batch.
2. **Cards whose row shows a card in T.** For the grid this is the summary cache's dependency index (§6.1). Before that card lands: parent, subtasks, `after_cards` neighbours, and blocked-on cards, one hop.
3. **Cards whose queue words moved.** Diff `spawn_queue().places` and `no_agent()` before and after the batch, and add the ids that changed. One card starting moves every queued card's place.
4. **Cards due by the clock.** `stale`, `talking`, a spawn's grace and a usage limit all end at known times. The summary cache keeps the earliest `due_at`, and the pump wakes then and sends those ids. Today nothing ticks, so this is better than now.
5. **A cap.** More than 200 ids means `all`. A catch-up folds thousands of rows, and reading them one by one would be slower than one full read.

## 4. The wire contract

One envelope per signal. Each is a struct deriving `Serialize, Type`, registered with `.typ::<T>()` in `commands.rs`, followed by `cargo xtask bindings`. The `seq` is minted by `Window::emit_change(name, …)`, never inside `coalesce`, because `boards:changed` is also sent from `Window::boards_pin`.

| Event | Payload | `all` when |
|---|---|---|
| `board:changed` | `CardsChange{seq, all, cards: [task id], notes: [note id], desk: bool}`. `cards` includes thread ids and is already expanded (§3.3). | a `Resync` of either form; `AGENTS_CHANGED`; `LEASE_*` (until card 6 maps holders to cards); a usage hold; a zero `Change`; any kind not in the table in `events.rs`; more than 200 ids |
| `talk:changed` | `TalkChange{seq, all, convs, calls}` | `Resync`; `talk.load`; `drop_space`; `dm_refold`; `talk.card` |
| `notifications:changed` | `NotifChange{seq, all, keys, asks}`. `asks` comes from adding `Topic::Asks` to this pump. | `Resync`; store resync |
| `inbox:changed` | `InboxCounts` plus `seq, all, deliveries, cards`. `cards` is resolved in the pump: `message.ref_` `task:` → id, `comment:` → its task, a message recorded on a card → the comment with that `msg_id`. | `Resync`; `POLICY_SET`; mesh re-fold |
| `devices:changed` | `{seq, all: true}` | always: rare, and names, colours and faces show on every row |
| `boards:changed` | `{seq}` | its lists are small and always read whole |
| `assistant:changed` | `{seq, all}` now. Later `turns: [said turn id]` for one-assistant.md §5's rule that `<<go>>` fires only for a reply that arrives live. | `Resync` |

The Core events change shape (`Talk`, `Notifications`), but nothing else consumes them: the window is the only `Core::feed` caller. The phone runs the same pumps and emitter, so there is no second transport to change. `crates/api`'s `/v1/events` serves store rows, not window events.

Pinned today and to update: `pumps.rs` `board_pump_forwards_and_coalesces` asserts `== json!({})`. `docs/port/surface.md` §2.10 and the table at the top of `events.rs` list `{}`. The doc comment on `Empty` says only `board:changed` and `boards:changed` use it, but six events do.

## 5. The page side: one listener module

A new `frontend/src/lib/changes.ts` replaces the per-hook listeners: `cards.ts` `subscribe`/`useFresh`/`cardEvents`, `desk.ts`, `notes.ts`, `notifications.ts`, `conversations.ts` `useTalkFresh`, `huddles.ts`, `transcript-pane.tsx` and `freshness.ts`.

- **One `Events.On` per signal, for the whole window**, attached when the module loads, before any list's first read. It keeps the last `seq` per event.
- It hands each list a notice: `{all: true}` or `{ids}` for the buckets that list follows. These turn a notice into `{all: true}`:
  - a seq gap;
  - `all` in the payload;
  - the page showing again after it was hidden. Today's `OnShown` merges a hidden stretch into one run, and this keeps that: one full read per hidden stretch;
  - the first notice after a list mounts (its mount read is a full read).
- **Focus** stays a full read, but at most once a minute per list. Alt-tabbing shouldn't re-read everything.
- **The drift check (D7).** Every 5 minutes while shown, each patched list does one full read. The module compares it with the list as patched and counts rows that differ (`drift <list> <n>` in the lists line). The full read wins.
- **Reads in flight.** `latestReads` already queues one follow-up read behind a running one. A patch list merges the ids of every notice that came while a read ran into that one follow-up, so no id is lost.

## 6. Each list

### 6.1 The grid (`useCards`): a summary cache in the window

`Window` keeps `HashMap<card id, Aux>` for scope "", plus the order.
- **`Aux` holds what comes from the logs:** `last_at`, the talk lines inside `TALK_WINDOW_MS`, `family_of`'s outputs (later card, order steps, `wait_pr`, raw `gone_wait`, `wait_you`, part lead, `wait_stuck`, wait name, parent / from-thread / blocked-subtask refs), `pos`, `ours`, `given_no`, the pin, place and owner. It also keeps `deps`, every card id and agent key read while computing it, and `due_at`.
- **What it doesn't cache:** the fields built from `people()` (holder state, agent status and names, leases, queue words, `accent`, `no_agent`, inbox `waiting`). These are rebuilt on each read with one fresh `people()`. They are cheap lookups. Caching them is how a cache goes wrong on every roster change.
- **Dirty rows** for a batch:
  - **T**;
  - `deps⁻¹(T)`;
  - `deps⁻¹` of the old and new refs of each card in T (its `after_cards`, waits, parent, from-thread, later card, subtasks). This catches a new edge that extends someone else's chain;
  - cards whose loop ring changed: `order_loops` once per batch, diffed;
  - rows past `due_at`;
  - the rows whose agent keys appear in a Presence or Roster change, or a lease holder. The board pump subscribes to those topics too, and the cache's `deps` maps keys to cards. This keeps holder and agent status current without a full read. Today it is current only because every board change re-reads every card.
  - `all` clears the cache.
- **`board_cards_patch(ids) → CardsPatch{rows, order}`**:
  - `rows` are the live rows among `ids`, freshly built;
  - `order` is every live id in grid order, **always sent**, about 36 B × 830 ≈ 30 KB against several hundred KB for every summary. Order, pins and families stay in Rust, unchanged, running over the cache;
  - a card that closed is simply missing from `order`.
- The full `board_cards` read stays, now built from the cache, and serves `all`.
- **`pending.ts` stays as it is**, because the page always builds the whole merged list (upsert rows, order by `order`, drop what isn't in it) before `observeSummaries`. Being absent from that list still means gone. If `order` is ever left out, `observeSummaries` must be told the read was partial, and never count a missing card as agreeing.
- **`decide_count` counts over the cache.** The pump applies the batch, recomputes dirty rows, counts `open_blockers > 0 || gone_wait` and shares the work with the page's next read. Until this card lands, card 3 gives a cheaper count on its own (§9).
- **Test:** after every batch in a test, the cache equals a cold `cards_in("")`.

### 6.2 The card store (`useCard`)

On `{ids}`, read only `ids ∩ watched`, with #1136's rev gate as now. On `all`, read every watched id. The pump's `cards` covers a watched card's family. Thread items' agent status already follows `roster:changed` through `LiveRoster`.

Inbox: it follows `inbox:changed.cards` instead of reading everything on every inbox change.

On the host, `cards_by_id` builds `people()`, `Followers` and `order_loops` once per call, not once per id. That is the part #1136 left: it cut the bytes but not the building.

### 6.3 Threads (`useAllThreads`, `useThreads`, the follower)

- **A real `threads_by_id(ids) → {rows, gone}`**, replacing today's `threads_all` + filter:
  - one `people()`, slimmed: thread rows never read the inbox parts;
  - per id: route, task, comments, rests, cursor and made cards;
  - the bucket worked out per id: a shared board's log; Personal; a space about it; or left.
- **The page** upserts the rows, drops `gone`, re-sorts `(resolved, updated_at desc, id)` in nanoseconds, and cuts each bucket to 20 resolved. A thread leaving resolved (reopened, withdrawn, cleared) can't bring back the 21st, so the host sets `all` for those.
- **Wakes:**
  - thread ids in `board:changed.cards`;
  - thread ids in `talk:changed.convs` (the read cursor; this fixes `useThreads`' stale `unread`);
  - `devices:changed` (names → `all`);
  - **drop `inbox:changed`**, since nothing in a thread row comes from the inbox. `useAllThreads`' `talk:changed` listener moves into the module, held while hidden.
- **The follower** skips a round unless the batch touches its followed, made or known ids, or says `all`.

### 6.4 Talk

- **`talk_conversations_by_id(convs)`**: split the per-kind bodies of `tools::talk::conversations` into `channel_row`, `dm_row` and `thread_row`. A DM row still scans the inbox for `mirrored()`, filtered inside the scan.
- Add a flag that leaves out the thread rows, which the window never shows (check `talk-page.tsx` and `space-leave.ts`, which fall back to them).
- **The page** re-reads the rows in `convs` and keeps its `listed()` sort.
- **`usePosts(conv)`** reloads only when its conv is in `convs`. The huddles and transcript lists only for their `calls`.
- **Members' status** in a conversation row comes from board agents and the roster, and goes stale today too. It isn't made worse here.

### 6.5 Notifications

- **`notifications_by_key(keys)`**. The page upserts rows (newest first, cap 500) and re-reads:
  - the keys in `notifications:changed`;
  - the rows whose `taskId` is in `board:changed.cards` (card titles);
  - the rows whose ask is in `asks`.
- **Drop the full re-list on `inbox:changed`.** `DeliveryLive` re-fetches its delivery only when that `deliveryId` is in `inbox:changed.deliveries` or the change says `all`.
- **The unread badge** reads its count only on `notifications:changed`, as now.

### 6.6 The small lists (no by-id reads)

| List | Change |
|---|---|
| desk | reload only on `board:changed.desk`, a card in its verdicts, or `all`. The 30 s poll stays (poll times are per device). |
| notes | reload only on `board:changed.notes` non-empty or `all`; upsert by id is optional |
| idle line | at most one read per 5 s, trailing |
| `#ref` chips | drop the cached refs whose card id is in `cards`. `all` clears them all. A new card or a renumber notes every card it touches, so their ids arrive. |
| closed tab | re-read the shown page only when an id in `cards` is on it or now closed |
| boards, personal boards | one shared store each, read once per `boards:changed` and once on first mount, not on every reader mount. `OwnerChip` stops starting reads. |
| devices | one shared store. Ignore `roster:changed` unless its device state changed. Keep the 5 s poll only while the devices panel is open. |
| inbox, held | upsert by `deliveries` with `InboxService.Get`. `all` re-reads the list. |

## 7. Fallback rules, per signal

| Signal | Patched with | Full read when |
|---|---|---|
| `board:changed` | `cards` (grid patch, card store, threads, refs, closed tab, notification titles), `notes`, `desk` | `all`, a seq gap, shown again, the first notice after mount, focus (at most once a minute), the 5-minute drift check |
| `talk:changed` | `convs` (conversations, posts, thread unread), `calls` (huddles, transcript) | same |
| `notifications:changed` | `keys`, `asks` | same |
| `inbox:changed` | `deliveries` (inbox, held, delivery outcomes), `cards` (card store) | same |
| `devices:changed` | — | always |
| `boards:changed` | — | always (small lists) |
| `assistant:changed` | — | always (no list yet) |

## 8. Measuring it

Card 1 comes before any behaviour change.

- **Count every signal.** Add `talk:changed`, `notifications:changed`, `boards:changed`, `roster:changed` and `assistant:changed` to `window-log.ts` `Signals`.
- **Name every list.** The 10 `useLatest` calls named "a list" get names: posts, devices, notifications, unread, held, repo names, gone, notes, me, idle.
- **Count every read path.** `cardStore`, `cardsById`, the follower, `useDoneCards`, refs, `reloadBoards`, `usePersonalBoards`, desk, freshness, huddles and transcript get a `noteRead`.
- **Count IPC by command.** Wrap the bindings' `commands` object once in `host.ts` with a counting Proxy, plus the two calls that bypass it (`main.tsx` `coreReady`, `card-look.ts`). The lists line ends `; ipc: board_cards 12, board_cards_by_id 30, …`.
  - Wrapping `window.__TAURI_INTERNALS__.invoke` doesn't work: Tauri defines it non-writable.
  - The line works on both machines. On the PC it can also be read over CDP: WebView2 honours `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=N`, which is not yet tried on a scratch copy. The Mac's WKWebView has no CDP, so the Mac's number is the log line.
- **The perf harness** (`frontend/src/perf/harness.ts`):
  - `emit(event, payload)` with the new envelopes;
  - answer `board_cards_by_id` with `rev`/`unchanged`;
  - answer the new by-id commands;
  - let a talk event change a conversation.
- **Baseline:** a scratch copy (`IRORI_HOME` copy, empty `CLAUDE_CONFIG_DIR`, `IRORI_DEV_PORT_OFFSET`) with agents working for 30 minutes. Record the lists lines and the IPC counts on #1138. The last card repeats it.

## 9. Build cards

Each is a subtask of #1138, in this order (`after`). Cards 6–11 can run side by side once card 5 lands. File names are the main ones, not every file.

| # | Card | Weight | After | Done when |
|---|---|---|---|---|
| 1 | **Measure: every signal, list, read and IPC command in the lists line** (§8) | normal | — | the lists line names every list and counts every signal and IPC by command (`window-log.test.ts` updated); the harness takes payloads; a baseline from a scratch copy with busy agents is on #1138 |
| 2 | **Core: no silent drops, every change names its ids** (§3.1, §3.2): reroute shared-board changes through the hub; inbox lost flag + `Resync`; `Kick` atomic; `Boards::tell` lost flag; the coord `note()` gaps (mentions, waits, `spawn_now`, desk kind); `devcolor` publishes; `CoreEvent::Talk(TalkChange)` and `Notifications(NotifChange)`; the mirrored DM published from `feed-inbox` | normal | — | tests: a full queue gives `Resync` at each hop; each coord gap notes the right ids; talk/notification sites carry their ids (core and coord `tests/it`). The window's payloads are still `{}` |
| 3 | **Tray count without the full shared-board scan**: `decide_count` builds `card_summary` per task plus `gone_waits`, no `board_cards`, and counts what it counts today | normal | — | same counts in a test across private, shared and thread cards; no `board_cards` call left in `decide_count` |
| 4 | **Window: payloads with ids and seq** (§3.3, §4): the envelopes, `emit_change` + per-name `seq`, the batch → ids fold with the `all` rules, the made-card index, one-hop family, queue diff, the 200 cap, `inbox:changed.cards`, `Topic::Asks` in the notifications pump, bindings, the `events.rs` table and surface.md | normal | 2 | `pumps.rs`: payload ids for each kind; `all` for each `all` rule; `seq` counts up per name, including `boards_pin` |
| 5 | **Page: one listener module** (§5): `changes.ts`, seq tracking, gap/shown/mount → `all`, focus once a minute, the drift check; every list moved onto it, **still reading everything** | normal | 4 | tests: gap → `all`, out of order → `all`, a hidden stretch → one `all`, ids merge into an in-flight follow-up; no list behaves differently yet (harness counts equal before and after) |
| 6 | **Grid: summary cache and patch read** (§6.1), including `decide_count` from the cache, the `deps` index for agent keys and leases, `due_at` timers | heavy (the cache's invalidation and dependency index are the hard part) | 5, 3 | the cache equals a cold `cards_in("")` after every batch in a randomized test; `useCards` patches; `board_cards` reads per minute drop to drift checks and `all`s |
| 7 | **Card store by id** (§6.2): `ids ∩ watched`; inbox cards; `people()`/`Followers`/`order_loops` once per call | normal | 5 | `card-store.test.ts`: a notice reads only the watched ids in it; harness: CardsById ids per notice = touched ∩ watched |
| 8 | **Threads by id** (§6.3): real `threads_by_id` → `{rows, gone}`; the page's upsert, sort and cap; drop inbox; talk convs; the follower's skip | normal | 5 | a test: by-id rows equal `threads_all`'s for those ids; a reopened thread sends `all`; `useAllThreads` no longer reads on `inbox:changed` |
| 9 | **Talk by conversation** (§6.4): `talk_conversations_by_id`, the thread-rows flag, posts/huddles/transcript filtered by `convs`/`calls` | normal | 5 | a test: by-id rows equal the full list's; a post in conv A doesn't re-read conv B's posts |
| 10 | **Notifications by key** (§6.5): `notifications_by_key`, title rows by `taskId`, asks, `DeliveryLive` by `deliveries` | normal | 5 | `notifications.test.ts`; no full re-list on `inbox:changed` |
| 11 | **Small lists and shared stores** (§6.6): desk, notes, idle, refs, closed tab, boards/personal boards stores, devices store, inbox/held upsert | normal | 5 | each list's trigger is in a test; `boards_list` reads on Home's first load = 1 (not one per tile) |
| 12 | **Check it end to end**: the baseline again on a scratch copy with busy agents (lists line and IPC by command, before/after); drift 0 over 30 minutes; a forced seq gap and a forced queue overflow each give one full read | normal | 6–11 | numbers on #1138: about one small by-id read per real change, `drift 0`, the fallback cases shown |

## 10. Open questions

1. **Shared boards' `all`.** One board's resync sets `all` for every board, because the Board feed is one queue. If a busy shared board resyncs often, a per-board `all` would need a board id on `Change`. Card 12's numbers will tell.
2. **Others' agents (`AGENT_CARD`)** change names and status on cards and conversation rows with no event today. This design doesn't make that worse. Card 6's `deps` index could take those keys later.
3. **The 100–200 s single reads** on the Mac on 11/08 are a different problem: the coord lock held through a catch-up, or contention. Fewer reads lower the contention, but if card 12 still shows reads over 10 s, they need their own card.
