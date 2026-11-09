# Event-driven window: signals that say what changed: design

*2026-11-08, card #1138. It comes from the person's question on #1134: "I don't know why we update that many times when we're just looking at it… Can we get more event driven?" Design first; card 2 (#1144) notes where the build differs ("As built"). The build cards are in §9. Checked against main at `3f8d9224` (after #1136) by ten readers (five mapped the code, five checked the design's inputs), then by four adversarial reviews (staleness, core correctness, the frontend, buildability), each finding checked by a skeptic. The 23 findings that held are folded in. Line numbers are approximate, so go by the names.*

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
| D4 | **`seq` is counted per event name on `Window`, when the event is sent. A gap at the page means `all`.** The page takes the first `seq` it sees as its starting point. | Events from the pump to the page arrive in order. A gap means the page didn't hear one: a held `OnShown`, a late listener, a reloaded webview. Tauri's `listen` goes live some time after it is called, so a list's first notice after it mounts is always `all` (§5). That rule, not the order things attach in, is what makes "the first seq seen is the start" safe. |
| D5 | **The window expands the touched ids to every card or thread whose row shows them** (§3.3). The page asks for exactly the ids it is sent. | A row shows other cards: parent, subtasks, followers, wait chains, made cards. Only the window has the graph. It needs the expansion for the tray count anyway (§6.1). |
| D6 | **The card grid keeps its order in Rust**: a summary cache in the window, a patch read that returns the changed rows and the whole order (§6.1). | The TypeScript port of the sort used by the sample already differs from Rust in four ways (ms vs ns times, no id tiebreak, kids by `parseInt(short)`, kids by `parent.id`). Two copies of the sort would drift apart without anyone noticing. |
| D7 | **A slow full read while shown checks the patches** (§5). Every 5 minutes, each patched list does one full read and logs how many rows differed (`drift`). For the grid that read is **cold**: it is built without the summary cache, so a cache bug shows up too (`drift cards-cache`). | It is the test for "lists never go stale", and it catches what a patch missed. A bug in the expansion shows up in the log and heals within 5 minutes; the person doesn't see it. |
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
| Shared board changes: the "changes" thread in `start_board` → `Boards::tell` → window relay `irori-window-boards` → the board feed's own queue | `Boards::tell` ignores `push`'s result. The relay pushes straight into the hub feed's queue, so `Hub::publish`'s lost flag never covers these changes. | **Reroute card changes through the hub.** In the changes thread, `s.hub.publish(CoreEvent::Board(ch))` replaces `s.boards.tell(ch)` inside the not-DM arm. The `is_dm_space` skip, `asks_kick` before it and `settle_notifications` after it stay as they are. The relay stops pushing real changes into `board_q`. **A board opening, being left, forgotten, renamed, changing members or a DM space being hidden** (`Boards::moved()`'s zero `Change`, about 30 sites) still has to reach the card lists. Today it does so only through this relay: an opened board's cards arrive by replay, and `Fold::note` sends nothing while replaying. **As built (card 2, #1144):** the Core's own `feed-board-set` pump hears `Boards::subscribe` and publishes each zero `Change` on the hub as `CoreEvent::Board(Change{kind: "resync"})`, so it reaches the Board feed with the hub's lost flag, and Core tests see it. (A bare `CoreEvent::Resync` has no topic: `hub.publish` would hand it only to all-topics feeds.) The window relay keeps one job: a zero `Change` pushes to `moved_q` (`boards:changed`). |
| `Boards::tell` (now only zero changes) | ignores `push` | a lost flag per subscriber. A lost one sends a zero `Change` next, so both `boards:changed` and `board:changed{all}` follow. |
| Inbox fan-out, `Inner::unlock` (`inbox/src/state.rs`) | `let _ = q.push(c)` | a lost flag per subscriber, and a new `ChangeKind::Resync` while it is set. *As built:* the Resync goes just before the next change that fits, which still goes (the agent stream in `api` acts on a held or moved change). |
| `Kick::notify` (window `events.rs`) | ignores `push` | keep the push, which wakes the pump, and also set an `AtomicBool`. The pump reads and clears it every batch and treats it as `all`, so a kick still counts when the queue was full. |
| approvals `Book::flush` → `feed-asks` | ignores `push`. A `Book` refold re-queues **every** ask key at once, so everything past 64 is dropped. | a lost flag per subscriber. `feed-asks` turns it into `CoreEvent::Asks("")` (the empty id means all). A refold sends that one marker instead of every key. The notifications pump maps an empty ask id to `all` (§4). |

Both resync forms must mean `all` in the board pump: `CoreEvent::Resync` (the hub lost one) and `Change{kind: "resync"}` (a board-set change from `feed-board-set`, or coord's `Fold::flush` lost one, `refold`, `catch_up`).

The test `five_cores.rs` `nothing_of_a_dm_space_shows_anywhere` reads card changes from `subscribe_boards()`. It moves to `core.feed(&[Topic::Board])` and matches `CoreEvent::Board(ch)`. It keeps both the wait for the shared card's id and the check that hidden card and ask ids never show.

### 3.2 Every change names its ids

**Board.** Every `TASK_CHANGED`, `COMMENT_ADDED` and `BOARD_PIN_CHANGED` already carries `task_id`. Card ids are unique across boards, and `Window::read` routes by id, so no board id is needed. The gaps, where a card's view changes with no `note()` for it:

| Where | Change |
|---|---|
| `mention_cards` (`fold.rs`) adds "mentioned this in #X" to card Y's activity | note `TASK_CHANGED` for each Y (collect the ids first, for the borrow) |
| `fold_wait` (`waits.rs`): no note at all for created, fired or canceled | note `w.task_id`, and each open card whose holders include `w.by` (read the wait before removing it) |
| `Coord::spawn_now` (`asked_now` feeds `cant_start`) | note the card |
| `fold_desk` (`desk.rs`) | note a new kind `desk`, so the desk can listen for it (§6.6) |
| `devcolor` rows (`service.rs`) | `hub.publish(CoreEvent::Devices)` |
| a usage hold set, released or expired (`spawn_limits.rs` `hold_usage`, `release_usage_hold`, and the clock expiry in `usage_held`) | publish `AGENTS_CHANGED` (→ `all`) |
| a board's spawn setting (`SPAWN_SET`, here or synced) changes every card's weight line | publish `AGENTS_CHANGED` (→ `all`) |

**Talk.** `CoreEvent::Talk` becomes `Talk(TalkChange{convs, calls, all})`. It has **no empty `Default`**. Its constructors are `TalkChange::all()`, `::convs(..)` and `::calls(..)`, and the pump treats a change with no convs, no calls and no `all` as `all`. A site nobody converted then reads everything instead of nothing. Every one of the ~25 `CoreEvent::Talk` publishes is checked, not only the row-driven ones:
- **Rows.** Every `talk.*` row names its conversation (`convId` or `channelId`). `talk.card` carries only a post id, is rare and becomes `all`.
- **Calls.** Every `call.*` row (`call.segment`, `call.title`, `call.assist`, `call.item`, `call.item.set`) adds its `callId` to `calls` and its `convId` to `convs`. A `talk.post` with a call mark (a huddle's start or end, `post_call_mark`) adds that call to `calls` as well as its conv to `convs`.
- **Thread read cursors.** The window marks a thread read on `convId = "thread:<task id>"` (`threads.rs` `read_conv`). Talk's own thread cursor uses the bare task id. The pump strips `thread:`, so `convs` always holds bare thread ids.
- **Publishes with no row:**

  | Site | Gives |
  |---|---|
  | `talk_dm` (a draft DM) | `convs: [id]` |
  | `talk_dm_leave` / `drop_draft` | `convs: [conv]` |
  | huddle_host's hosting changes, `write_assist` | `calls: [call]`, and its `convs: [conv]` (as built) |
  | `DmHooks::hello` (an old peer: only `waiting` changes) | `all` |
  | `fold_space_talk` (a space opened, a member re-added, `dm_refold`) | `all` |
  | `talk.load()` on a store resync, `drop_space` | `all` |

- **A DM mirrored from an agent's message.** Its conv id is `dm_id(m.from.key, d.to)`, from the delivery. The store follower sees `message.sent` before the inbox has folded the delivery, so `feed-inbox` publishes it instead. It looks up `ch.delivery_id`, and publishes `Talk(convs: [dm_id(..)])` **only** when the change is the delivery's creation and `tools::talk::mirrors(&message)` holds. A `MovedAway` changes `to`, so it names both the old and the new DM. No other delivery change publishes Talk. The follower's `message.sent` path is dropped. The test `test_only_messages_that_show_as_dms_change_talk` (`core/tests/it/core.rs`) still expects exactly one Talk for one agent message among 20 notices and a stop. *As built:* `inbox::Change` carries `created` (the delivery's first row folded here), and `MovedAway`'s second DM is `dm_id(from, d.moved_to)`, since `moved_to` is the new recipient's key. An inbox `Resync`, or the inbox's re-fold after a revoke's cut (`Updated` naming nobody and no delivery, as a policy write also sends), publishes Talk `all`. A delivery folded before its message (rows from another device) publishes `all` then, and the inbox notes it `created` again when the message folds, so its DM is named. An existing delivery given a new recipient without a move (an assign, a release) carries `was_to`, and both DMs are named.
- The pump takes the union over the batch, which also folds away the double publishes (a direct write, then its follower).

**Notifications.** `CoreEvent::Notifications(NotifChange{keys, all})`, built the same way: no empty `Default`, and empty means `all`.
- The keys are in scope at every site: `r.key`, `row.keys`, `keys`, the synced row's `key`/`keys`. *As built:* `coord::notifications::keys_of(ev)` keys them as the fold does, so a post with no key (an agent's notify) is `row:<device>:<seq>`, not empty.
- A store resync sets `all`.

**Inbox.** `inbox::Change` already has `delivery_id`. Wake issued, turn engaged/ended and the route settings carry a non-empty `to` and touch no delivery. `POLICY_SET`, the mesh re-fold and the receive/autonomy/threads settings all send `Updated` with **both** `to` and `delivery_id` empty. The pump treats exactly that shape as `all`, since they are rare, and treats the new `Resync` as `all`.

### 3.3 The window expands ids

The board pump subscribes to Board, Presence, Roster and Inbox. It folds a batch into a set of touched task ids **T**:
- `task_id` of each card change;
- the cards of each inbox change (§4 says how a delivery maps to a card), because a queued assignment notice shows on the card as "waiting".

It then expands T into the ids it sends:

1. **Threads that made a card in T.** There is no reverse index from a made card to its thread today: `Task.from` is not set when a thread changes an existing card, only `th.made` is. Build `made_by: HashMap<card, Vec<thread>>`, kept in `fold_made`, or scanned from threads once per batch.
2. **Cards whose row shows a card in T.** This is the summary cache's dependency index (§6.1, card 6). Before card 6 lands: parent, subtasks, `after_cards` neighbours and blocked-on cards, one hop.
3. **Cards whose agents moved.** For the agent keys in a Presence or Roster change, or a lease's holder, add the cards whose rows show them (the same index).
4. **Cards whose queue words moved.** Diff `spawn_queue().places` and `no_agent()` before and after the batch, and add the ids that changed. One card starting moves every queued card's place.
5. **Cards and threads due by the clock.** `stale`, `talking`, a spawn's grace (cards and threads) and a usage limit all end at known times. The window keeps the earliest `due_at`, wakes then, and sends those ids. Today nothing ticks, so this is better than now.
6. **A cap.** More than 200 ids means `all`. A catch-up folds thousands of rows, and reading them one by one would be slower than one full read.

A batch with only Presence or Roster changes that dirties no row sends nothing.

## 4. The wire contract

One envelope per signal. Each is a struct deriving `Serialize, Type`, registered with `.typ::<T>()` in `commands.rs`, followed by `cargo xtask bindings`.

`Window::emit_change(name, …)` mints `seq`, never `coalesce`, because `boards:changed` is also sent from `Window::boards_pin` and from three threads. It holds one mutex across minting and the emitter call, so the page never sees seq 8 before 7.

**The board pump's kind map.** Card 4 writes it as a table in `events.rs`. Anything not in it is `all`.

| Change kind | Goes to |
|---|---|
| `task`, `comment` | `cards` ← `task_id` |
| `note` | `notes` ← `resource` (the note id) |
| `board_pin` | `boards:changed` only, not `cards` (its `task_id` is `board:<id>`) |
| `desk` (new) | `desk: true` |
| `lease`, `lease_expired`, `lease_refused` | the holder's cards through the index (§3.3 step 3). `all` until card 6 has it. |
| `resync`, `agents.changed`, an empty kind | `all` |

| Event | Payload | `all` when |
|---|---|---|
| `board:changed` | `CardsChange{seq, all, cards: [task id], notes: [note id], desk: bool}`. `cards` includes thread ids and is already expanded (§3.3). It is also sent for Presence, Roster, lease and inbox batches that dirty rows. | a `Resync` of either form; a board opened, left, forgotten, renamed, its members or owner changed, or a DM space hidden (`Boards::moved`, via the relay); `AGENTS_CHANGED` (also a usage hold, a spawn setting); `LEASE_*` until card 6; an unmapped kind; more than 200 ids |
| `talk:changed` | `TalkChange{seq, all, convs, calls}`: conv ids (bare thread ids for thread cursors), call ids | `Resync`; `talk.load`; `drop_space`; `fold_space_talk`; `dm_refold`; `DmHooks::hello`; `talk.card`; an empty change; an inbox `Resync` or re-fold, and a delivery folded before its message (`dm_change`, as built) |
| `notifications:changed` | `NotifChange{seq, all, keys, asks}`. `asks` comes from adding `Topic::Asks` to this pump. | `Resync`; store resync; an empty ask id (asks lost or refolded); an empty change |
| `inbox:changed` | `InboxCounts` plus `seq, all, deliveries, cards`. `cards` is resolved in the pump: `message.ref_` `task:` → id, `comment:` → its task, a message recorded on a card → the comment with that `msg_id`. | `Resync`; `Updated` with empty `to` and `delivery_id`; a kick |
| `devices:changed` | `{seq, all: true}` | always: rare, and names, colours and faces show on every row |
| `boards:changed` | `{seq}` | its lists are small and always read whole |
| `assistant:changed` | `{seq, all}` now. Later `turns: [said turn id]` for one-assistant.md §5's rule that `<<go>>` fires only for a reply that arrives live. | `Resync` |

The Core events change shape (`Talk`, `Notifications`), but nothing else consumes them: the window is the only `Core::feed` caller. The phone runs the same pumps and emitter, so there is no second transport to change. `crates/api`'s `/v1/events` serves store rows, not window events.

Pinned today and to update:
- `pumps.rs` `board_pump_forwards_and_coalesces` asserts `== json!({})`.
- `docs/port/surface.md` §2.10 and the table at the top of `events.rs` list `{}`.
- The doc comment on `Empty` says only `board:changed` and `boards:changed` use it, but six events do.

**As built (card 4, #1145).** `crates/window/src/signals.rs` has the envelopes (`CardsChange`, `TalkSignal`, `NotifSignal`, `InboxSignal` = the counts flattened plus `seq, all, deliveries, cards`, `DevicesSignal`, `BoardsSignal`, `AssistantSignal`; `seq` is a `u32`, as specta has no 64-bit number) and the folds; the kind table is `signals::KINDS` (re-exported in the `events.rs` header). Where the build differs or chooses:
- **Presence and Roster before card 6's index.** A Presence change names the cards whose rows show that agent key, from an index the pump keeps (holders, step owners, who left); a Roster change (no key) names the cards held by any agent of another device. A batch that names no row sends nothing. Waits `by` an agent aren't in the index, so the tray count (`decide_count`) also runs when a presence change removes an agent or reports it gone.
- **Expansion state** (`Expander`) is built by one scan of every log on the first batch, again after a lost event, and kept by re-reading each card a batch names: its one-hop edges (parent, subtasks, thread, after cards, blockers on a card) both ways, `made_by` (card → threads, from `Task.made`), the agent index and the clock times. Closed cards are in the scan.
- **Due timer.** A thread (`irori-window-due`, 1 s) hands over the cards whose time passed: `last activity + STALE_AFTER`, newest comment `+ TALK_WINDOW`, a pending spawn `+ SPAWN_GRACE`, and every queued card when a usage hold ends.
- **The tray count** runs on a batch that sent `board:changed`, held a Board, Inbox or `Resync` event, or removed an agent: not on each agent line.
- **Inbox cards** come from the delivery's message: a `task:` ref, a `comment:` ref's card, and the comment with the message's id (a message recorded on a card).
- **Debug switches** are `IRORI_DEV_SKIP_SEQ=N` (every Nth emit of a name jumps a seq) and `IRORI_DEV_FEED_QUEUE=N` (the Core feeds and `moved_q` get N slots; `Core::feed_sized`). A release build ignores both.

## 5. The page side: one listener module

A new `frontend/src/lib/changes.ts` takes over **every `Events.On`/`OnShown` of `board:changed`, `talk:changed`, `notifications:changed`, `inbox:changed`, `devices:changed`, `boards:changed` and `assistant:changed` in `frontend/src`**, except `window-log.ts`, which only counts. At `3f8d9224` that is:
- `cards.ts`: `subscribe`, `useFresh`, `cardEvents` and `useAllThreads`' talk listener;
- `talk.ts`: `useInbox`, `useHeld`, `useRepoNames`;
- `desk.ts`, `notes.ts`, `storage.ts` `useStorageFresh`, `notifications.ts`;
- `conversations.ts` `useTalkFresh`, `huddles.ts`, `transcript-pane.tsx`;
- `boards.ts` `onBoardsMoved`, `freshness.ts`, `devices.ts`, `profile.ts` `useMe`, `irori.ts` (the header's device read).

`OnShown`/`whileShown` take `() => void` and drop the payload. So the module uses `Events.On` with its own `appPage()` hidden check. It keeps counting `seq` while hidden and only holds the reload.

- **One listener per signal for the whole window.** It keeps the last `seq` per event.
- **It hands each list a notice**: `{all: true}` or `{ids}` for the buckets that list follows. These turn a notice into `{all: true}`:
  - a seq gap;
  - `all` in the payload;
  - the page showing again after it was hidden. Today's `OnShown` merges a hidden stretch into one run, and this keeps that: one full read per hidden stretch;
  - the first notice after a list mounts, or after its load key (conversation, board) changes. Its mount read is a full read.
- **A hook's own `reload()`** stays a full read: the one callers `await` after an action (held-dialog `act`, card-grid `done.reload`, `ClearDone`, the transcript's reload). That keeps `latestReads`' promise that "`await reload()` after an action sees the action". Only notices from the module trigger patch reads.
- **Focus** stays a full read, but at most once a minute per list. Alt-tabbing shouldn't re-read everything.
- **Reads in flight: new code, not something `latestReads` gives.**
  - `latestReads` queues a follow-up only for the **same** load function. A different function starts beside the running read and drops its answer. A read stuck past 15 s also gets a second read beside it, and the late answer is dropped.
  - So each patch list has **one stable load**. It never closes over a notice's ids. When `latestReads` starts it, it takes everything pending for that list from the module (`{all, ids}`).
  - Notices, focus and the drift check only add to that pending state and call `reload()` with the same load.
  - A read whose answer errors or is dropped puts its ids back (or marks the next read `all`). A retry after a failure reads `all`.
  - The patch is merged into the rows on screen when the answer is shown, not inside the load. This is a small hook built on `latestReads`, next to `useLatest`.
- **The drift check (D7).** Every 5 minutes while shown, staggered across lists, each patched list does one full read **through its own queue**, so it never runs beside a patch of the same list.
  - The module notes each followed signal's `seq` when the read starts. If any notice for that list's buckets arrived before the answer, or a patch read is waiting, it drops the sample without comparing. It counts the drops.
  - Otherwise it compares the id set, the order, and each row as the page stores it (serialized), and logs `drift <list> <n>` (and `dropped <n>`). Then the full read replaces the list.
  - Card 5 builds the check, but it does nothing until a list patches.
  - **The grid's drift read is cold.** The host builds `cards_in("")` without the summary cache, diffs it with the cached rows, logs `drift cards-cache <n>` with the ids, and replaces the cache with it. Otherwise a missed invalidation would serve the same stale row to the patch and to the check. One cold read every 5 minutes is far less than today's read on every change.

**As built (card 5, #1146).** `changes.ts` is the listener module (`onChange(signal, fn, {follow, held, focus})`, one `Events.On` per signal, `createChanges` for tests) and `patched.ts` the patch hook (`patchedReads` core, `usePatched`, the drift schedule). Where the build differs or chooses:
- **Every list is on `onChange` and still reads everything**: the handler ignores the notice and calls `reload()`, so the reads per notice are as before (checked on the perf harness: the same IPC per `board:changed` and `talk:changed` on Home, Notifications, Talk and Notes).
- **Held or not.** A subscriber is held while hidden by default (the `OnShown` lists, and now `useAllThreads`' `talk:changed`, the one intended change besides focus). Those that were plain `Events.On` and stay so with `held: false`: the storage page's board listener, the transcript's `talk:changed`, `useMe`'s and the header's `devices:changed`.
- **Focus** is `focus: true` on one signal of a list: a full read, at most once a minute (`FocusEveryMs`) per subscriber. It replaces each list's own `window` `focus` listener.
- **`rekey()`** on the unsubscribe function marks a load-key change (the next notice is `all`). A hook that resubscribes on a key change gets the same from the new subscription.
- **Signals with no id buckets** (devices, boards, assistant) are always `all`, whatever the payload says. The desk flag is the bucket `desk` holding `"desk"`.
- **The patch hook.** `patchedReads` has one stable load that drains `{all, drift, ids}` (ids keyed `signal.bucket`) as `latestReads` starts it. A read that errors puts its ids back and marks the next read `all`. A dropped one (`latestReads`' new optional `dropped` callback) puts its ids back and asks for a follow-up. A notice naming none of the list's buckets is ignored. The hook's `reload()` is a full read.
- **Drift.** `driftCheck()` does nothing until a patch has been merged since the last full read, and only for a list that gives `rows()`. It sets a `drift` flag that makes the next read full through the same queue; the sample is dropped (`noteDriftDropped`) if a notice for the list came while it read or a patch is waiting, else compared (`rowsDiffer`: ids in one list only, rows that serialize differently, and one if the shared rows' order differs) and logged (`drift <list> <rows> of <samples>`, before `ipc:` in the lists line). Either way the full answer replaces the list. Lists register on a `DriftSlots`-way stagger of `DriftEveryMs`, skipping a hidden window.
- **The perf harness** needed `mockWindows("main")` and answers for `plugin:window|…`: since #1134 `appPage()` asks the Tauri window, and without one the page never mounted.

## 6. Each list

### 6.1 The grid (`useCards`): a summary cache in the window

`Window` keeps `HashMap<card id, Aux>` for scope "", plus the order.
- **`Aux` holds what comes from the logs:** `last_at`, the talk lines inside `TALK_WINDOW_MS`, `family_of`'s outputs (later card, order steps, `wait_pr`, raw `gone_wait`, `wait_you`, part lead, `wait_stuck`, wait name, parent / from-thread / blocked-subtask refs), `pos`, `ours`, `given_no`, the pin, place and owner. It also keeps `deps` and `due_at`.
  - `deps` is every card id passed to `get_task` and every agent key passed to `p.ref_`/`name_of` while computing the row.
- **What it doesn't cache:** the fields built from `people()`: holder state, agent status and names, leases, queue words, `accent`, `no_agent`, inbox `waiting`. Any row that is read is rebuilt with one fresh `people()`; these are cheap lookups, and caching them is how a cache goes wrong on every roster change. But **only rows in `ids` are read**, so every input to these fields needs its own wake: §3.3 steps 3 and 4 and the inbox cards in T.
- **Dirty rows** for a batch:
  - T, which includes the inbox cards;
  - `deps⁻¹(T)`;
  - `deps⁻¹` of the old and new refs of each card in T (its `after_cards`, waits, parent, from-thread, later card, subtasks). This catches a new edge that extends someone else's chain;
  - cards whose loop ring changed: `order_loops` once per batch, diffed;
  - rows past `due_at`;
  - rows whose agent keys appear in a Presence or Roster change, or a lease holder (`deps` maps keys to cards). Today these stay current only because every board change re-reads every card.
  - `all` clears the cache.
- **`board_cards_patch(ids) → CardsPatch{rows, order}`**:
  - `rows` are the live rows among `ids`, freshly built;
  - `order` is every live id in grid order, **always sent**: about 36 B × 830 ≈ 30 KB, against several hundred KB for every summary. Order, pins and families stay in Rust, unchanged, running over the cache;
  - a card that closed is simply missing from `order`;
  - an id in `order` that the page holds no row for and that isn't in `rows` is skipped. It comes from a batch whose notice is still on its way, and that notice's patch brings the row.
- The full `board_cards` read stays, now built from the cache, and serves `all`. The drift read is cold (§5).
- **`pending.ts` stays as it is**, because the page always builds the whole merged list (upsert rows, order by `order`, drop what isn't in it) before `observeSummaries`. Being absent from that list still means gone. If `order` is ever left out, `observeSummaries` must be told the read was partial, and must never count a missing card as agreeing.
- **`decide_count` counts over the cache.** The pump applies the batch, recomputes dirty rows, counts `open_blockers > 0 || gone_wait` and shares the work with the page's next read. Until card 6 lands, card 3 gives a cheaper count on its own (§9).
- **Test:** after every batch in a randomized test, the cache equals a cold `cards_in("")`.

**As built (card 6, #1147).** `crates/window/src/cards_cache.rs`. Where the build differs or chooses:
- **What is kept.** Each log's open cards as their log holds them (`Entry`: the task, and whether it needs the person), and per grid row an `Aux`: `family_aux`'s outputs (`family_of` split in two: `family_aux` reads other cards through a getter, `family_apply` adds what moves with no event), the newest activity time, the comments still inside the talk window, a shared card's comment authors and the canvas position, with `deps` (every card id the getter was asked for, misses and closed cards included, and the row's own). **Read on every read, never kept:** `card_summary` and everything from People; the gone wait (`gone_waits` reads the roster, and gone-ness flips with no event: `mark_ended`, a `gone_rows` entry cleared at a sweep, a revoked device); a part's lead; a shared card's private keys (`priv_task`, `ours`, `talking`'s authors: a `board.cardof` row from another device changes them silently), its owner, `given_no`, place and weight; and the person's pins on shared cards (`pinned_ranks`).
- **Dirty rows.** The pump's T, plus every row whose `deps` hold a card of T or one of its old or new refs (the expander's one-hop edges, kept for every card of every log, threads and withdrawn cards included, since `Followers` counts them), plus rows whose loop ring changed (diffed per batch; the randomized test shows the refs rule already catches every ring change, so the diff is a backstop). A dirty row drops its Aux; the next read builds it. The patch read always reads its ids again.
- **Clearing.** A resync, a lost event, an unmapped kind or a card change with no id clears it (`Folded::cold`). `AGENTS_CHANGED` still sends `all` but keeps the cache: what it moves is built on every read. A device change clears it too (a device's sides change `side_wait` with no card change). More than 200 ids sends `all` and keeps the cache, its rows already dropped.
- **Live only with a pump.** The board pump turns it on after it subscribes and off when it stops. Without one, `cards()`, `cards_patch` and `cards_cold` read cold, as before, so tests that call `cards()` with no pump see every write.
- **Agent keys.** Each read notes the keys `People::ref_` resolves for a row, and the keys its gone wait could read (`assigned`, agent waits, `sent_to`/`raised_by`). A Presence change or a lease holder names those rows; a Roster change names the rows that show another device's agent. Lease kinds are their own bucket now: the holder, and every holder whose conflict began or ended (diffed, since a conflict's note names only its winner).
- **Commands.** `board_cards` (from the cache), `board_cards_patch(ids) → CardsPatch{rows, order}` and `board_cards_cold` (the drift read: a fresh cache, `drift cards-cache <n> <ids…>` in irori.log, then kept). A separate command, so the lists line tells a drift read from an `all`.
- **`decide_count`** counts the kept entries' blocked flag and reads each gone wait then; private threads are entries too.
- **Core.** Pinning a shared card (the person's own log, `pin_ref`) now notes `TASK_CHANGED` for the card; before, no event said so.
- **Page.** `useCards` is on `usePatched` (`readGrid` routes drift → cold, `all` → whole, ids → patch; `mergeCards` upserts and lays rows out by `order`). `patched.ts` now drops a drift sample that drained a notice's ids or `all` into its own read. On the perf harness, the board:changed rounds after the first (always `all` after mount) call `board_cards_patch`, not `board_cards`.
- **Holes drift heals** (no event says so; within 5 minutes): a `TASK_PARENT` row that changes no link still adds an activity line (`last_at`), and `note_others` sets a kept card's `others` unnoted.

### 6.2 The card store (`useCard`)

- On `{ids}`, read only `ids ∩ watched`, with #1136's rev gate as now. On `all`, read every watched id.
- A card page shows chains several cards long (`wait_stuck` walks the whole wait graph; `order` walks up to 24 cards each way). So the store depends on card 6's dependency index and agent mapping; with one hop, it would miss them. Thread items' agent status already follows `roster:changed` through `LiveRoster`.
- **Inbox:** it follows `inbox:changed.cards` instead of reading everything on every inbox change.
- **On the host,** `cards_by_id` builds `people()`, `Followers` and `order_loops` once per call, not once per id. That is the part #1136 left: it cut the bytes but not the building.

**As built (card 7, #1148).**
- **Page.** `createCardStore`'s `events` callback now takes ids: `again(ids)` reads the watched ones among them (the unwatched are dropped before any read is queued, so a notice naming none starts none), `again()` reads every watched id. `cardEvents` follows `board:changed.cards` (via `subscribeIds`, with focus) and `inbox:changed.cards`; `devices:changed`, focus, a gap, a first notice and the failed-read retry stay `all`. The `have` revs gate is as before.
- **The chain comes from the signal, not the page.** The board pump already adds `cache_batch`'s rows (card 6's `readers` index, which holds every card a row's chain read, wait graph and `order` included) to the notice's `cards`, so a card page far down a chain is named when a card on it moves. The page keeps no index of its own. The index only exists while the grid's cache is built (its first read builds it); a card page opened with no grid read yet is named by the one-hop expander only until then.
- **Host.** `Window::cards_by_id` reads through `card_with(id, &CardShared)`: the people are built once for the call, and the order loops and `Followers` once per log (keyed by the log's `Arc`), not once per id. `card` (one id) builds its own. `Window::card_people_builds()` counts the people builds for the test.
- **Tests and harness.** `card-store.test.ts` ("by id"); `cards_by_id_builds_people_once_per_call` (`tests/it/cards.rs`); the perf harness's snapshot has `cardIds`, the ids of each `board_cards_by_id` read.

### 6.3 Threads (`useAllThreads`, `useThreads`, the follower)

- **A real `threads_by_id(ids) → {rows, gone}`**, replacing today's `threads_all` + filter:
  - one `people()`, slimmed: thread rows never read the inbox parts;
  - per id: route, task, comments, rests, cursor and made cards;
  - the bucket worked out per id: a shared board's log; Personal; a space about it; or left.
- **The page** upserts the rows, drops `gone`, re-sorts `(resolved, updated_at desc, id)` in nanoseconds (not `Date.parse`), and cuts each bucket to 20 resolved. Add the same id tiebreak to Rust's `threads_all` and `thread_rows` sorts, so both sides order exact ties alike. A thread leaving resolved (reopened, withdrawn, cleared) can't bring back the 21st, so the host sets `all` for those.
- **Wakes:**
  - thread ids in `board:changed.cards`, which includes queue moves and spawn-grace timers (§3.3);
  - thread ids in `talk:changed.convs`, i.e. the read cursor. This fixes `useThreads`' stale `unread`;
  - `devices:changed` (names → `all`);
  - **drop `inbox:changed`**, since nothing in a thread row comes from the inbox. `useAllThreads`' `talk:changed` listener moves into the module, held while hidden.
- **The follower** skips a round unless the batch touches its followed, made or known ids, or says `all`.

**As built (card 8, #1149).**
- `threads_by_id(ids)` returns `{rows, gone}`: one `thread_people()` (`people()` without the inbox parts), then `thread_row` per id, which `thread_rows` also uses, so a by-id row is the full list's row (a test compares them in every bucket, a left space included). `gone` is every id with no row, non-thread cards among them (the page asks for the cards of a notice too). Rows are not capped and come in `threads_all`'s order. `newest` (resolved, `updated_at` desc, id) now orders `threads_all` and `thread_rows`.
- **Leaving resolved.** The `Expander` keeps the resolved thread ids; a batch where one is no longer resolved (reopened, withdrawn, gone, or a rebuild that finds it so) sends `all`. A cleared thread has no change of its own, so the page also reads the whole list when a patch shows a thread it holds as resolved turning open or gone (`readThreads`).
- **Page.** `thread-list.ts` has the merge (upsert, drop `gone`, sort by the nanosecond time, cap 20 resolved per bucket: a space, Personal, and the private threads about a space you left), the wakes (`threadFollow`: `board:changed.cards`, `talk:changed.convs`, `devices:changed`, no `inbox:changed`) and `readThreads`. `useThreads` and `useAllThreads` are on `usePatched`; they are its first users. Only a full read moves `asOf`, so `withStarted` still shows a thread started since until a read covers it.
- **Follower.** `listen` hands `again` the ids a change names; the follower skips it unless one is a followed thread or a card it has made or read from one.

### 6.4 Talk

- **`talk_conversations_by_id(convs)`**: split the per-kind bodies of `tools::talk::conversations` into `channel_row`, `dm_row` and `thread_row`. A DM row still scans the inbox for `mirrored()`, filtered inside the scan.
- Add a flag that leaves out the thread rows, which the window never shows. Check `talk-page.tsx` and `space-leave.ts` first: they fall back to those rows.
- **The page** re-reads the rows in `convs` and keeps its `listed()` sort.
- **`usePosts(conv)`** reloads only when its conv is in `convs`.
- **The huddles list** re-reads in full whenever `calls` is non-empty. It must not filter to the huddles already shown, because new ones arrive this way. **The transcript pane** re-reads only when its own call is in `calls`.
- **As built (card 9, #1150).** `tools::talk` has `channel_row`, `dm_row` and `thread_row`, and `conversations_with(b, reader, threads)` / `conversations_by_id(b, reader, convs, threads)` over them (`conversations` is the full list with threads, as before). `mirrored` takes an optional set of DM ids and skips the rest inside its scan; `posts` of one DM uses it too. An id with no row (archived, left, resolved, not the reader's) is simply absent, and the page drops it (`gone` = asked − returned). Commands: `talk_conversations(threads)` and `talk_conversations_by_id(convs, threads)`. **The thread flag:** `useConversations` and `talkApi.conversations()` pass `threads: false`. Checked first: `talk-page.tsx` looks a thread up in `useAllThreads` before it falls to the conversations; `space-leave.ts` reads `v.threads` (all open threads) before them and treats a thread neither list has as staying once both loaded; the assistant's screen line (`assistant-screen.ts`) passes `threads: true` so a thread still reads as "thread <title>". `useConversations` and `usePosts` are on `usePatched` (`follow talk:changed.convs`, focus on the list); `usePosts` reads only on `all`/its own conv in `convs`, else `readPosts` returns null and nothing is read. The huddles list reads in full on any non-empty `calls` (`names(n, "calls")`); the transcript pane reads only when its own call (with or without `call:`) is in `calls`.
- **Members' status** in a conversation row comes from board agents and the roster, and goes stale today too. It isn't made worse here.

### 6.5 Notifications

- **`notifications_by_key(keys)`**. The page upserts rows (newest first, cap 500) and re-reads:
  - the keys in `notifications:changed`;
  - the rows whose `taskId` is in `board:changed.cards` (card titles);
  - the rows whose ask is in `asks`.
- **Drop the full re-list on `inbox:changed`.** `DeliveryLive` re-fetches its delivery only when that `deliveryId` is in `inbox:changed.deliveries`, or the change says `all`.
- **The unread badge** reads its count only on `notifications:changed`, as now.

### 6.6 The small lists (no by-id reads)

| List | Change |
|---|---|
| desk | reload only on `board:changed.desk`, a card in its verdicts, or `all`. The 30 s poll stays (poll times are per device). |
| notes | reload on `board:changed.notes` non-empty, on a card in `cards` that a note names (`taskId`, for "Still true?"), or `all`. Check files again on show and focus, since files have no event. |
| idle line | at most one read per 5 s, trailing |
| `#ref` chips | each entry remembers the card id it resolved to. A notice with ids drops the entries whose id is in `cards`, **plus every entry that resolved to nothing or is still pending**, then bumps `refGen` (keeping the 1.5 s burst wait). Null entries are few, and a new card or a renumber always comes with ids. `all` clears everything. |
| closed tab | re-read the shown page only when an id in `cards` is on it or now closed |
| boards, personal boards | one shared store each, read once per `boards:changed` and once on first mount, not on every reader mount. `OwnerChip` stops starting reads. |
| devices | one shared store. Ignore `roster:changed` unless its device state changed. Keep the 5 s poll only while the devices panel is open (it also covers a revoke, which has no event). |
| inbox, held | upsert by `deliveries` with `InboxService.Get`. Held drops a row whose new state isn't held or whose `Get` fails, and the inbox drops rows past its 200 that aren't held. `all` re-reads the list. |
| repo names, storage (cleared list), `useMe`, the header's device read | full read on every notice of their signal, through the module, so they are held while hidden |

**As built (card 11, #1152).** The decisions are plain functions in `frontend/src/lib/small-lists.ts` (`deskWakes`, `notesWake`, `doneWakes`, `refsToDrop`, `rosterWatch`, `trailing`, `upsertRows`, `heldUpsert`, `createStore`), each tested in `small-lists.test.ts`. Where the build differs or chooses:
- **Desk** follows `desk` and `cards`; its verdict cards are each PR's author card (`authors[pr].card`). The 30 s poll stays.
- **Notes** follow `notes` and `cards` (a card some note names by `taskId`), and read again when the window shows and on focus (at most once a minute) for the `path:` files.
- **Idle line** reads at most every 5 s (`IdleEveryMs`, trailing); the mount's read and a retry after an error are direct.
- **#ref chips.** Each cache entry remembers the card it resolved to and whether it answered. A notice drops the entries whose card it names plus the null and pending ones, then the 1.5 s burst wait bumps `refGen`. `devices:changed` is `all`.
- **Closed tab.** "Now closed" is told by the grid: `liveCards` keeps the cards the grid last showed, and a card that leaves it reads the tab (`onLeft`). Until the grid has answered once, any named card reads. A card that arrives already closed from another device is not named by either rule, so it shows on the next focus, show or `all`.
- **Boards and personal boards** are one store each (`subscribeBoards`, `subscribePersonal`); only the first reader reads. A reader swapped in the same commit (a page change, React's strict-mode remount) keeps the store, because the last reader's stop waits a microtask. `OwnerChip` starts no read, so Home's first load is one `boards_list`.
- **Devices** are one store (`createStore`). `roster:changed` is read from its payload and ignored unless the set of `deviceId=deviceState` pairs moved; while hidden it is owed and read once on show. The 5 s poll runs only while a `useDevices(open, true)` (the devices panel) is mounted, with one timer for all.
- **Inbox and held** are on `usePatched`: `inbox:changed.deliveries` are read by `InboxService.Get` and upserted; a failed `Get` makes the inbox read whole, and drops the row from held. Held with History open reads whole. The drift check covers both.
- **Full-read lists** (repo names, storage's cleared list, `useMe`, the header's device read) are all on `onChange` and now held while hidden; card 5 had left `useMe`, the header and storage on `held: false`.

## 7. Fallback rules, per signal

| Signal | Patched with | Full read when |
|---|---|---|
| `board:changed` | `cards` (grid patch, card store, threads, refs, closed tab, notification titles, notes), `notes`, `desk` | `all` (§4), a seq gap, shown again, the first notice after mount, focus (at most once a minute), the 5-minute drift check (cold for the grid) |
| `talk:changed` | `convs` (conversations, posts, thread unread), `calls` (huddles in full, transcript by its call) | same |
| `notifications:changed` | `keys`, `asks` | same |
| `inbox:changed` | `deliveries` (inbox, held, delivery outcomes), `cards` (card store; the grid gets them through `board:changed`) | same |
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
- **Baseline:** a scratch copy (`IRORI_HOME` copy, empty `CLAUDE_CONFIG_DIR`, `IRORI_DEV_PORT_OFFSET`) with agents working for 30 minutes. Record the lists lines and the IPC counts on #1138. It must be recorded **before card 2 merges**: card 2's new `note()`s and publishes add signals, and today each signal is a full read. The last card repeats it.
- **Forcing the fallbacks:** card 4 adds two debug-build-only switches, read from the environment at start: one skips a `seq` now and then, and one shrinks the feed queues so they overflow. A release build ignores them, like `IRORI_DEV_PORT_OFFSET`. Card 12 uses them on the scratch copy.

## 9. Build cards

Each is a subtask of #1138, in this order (`after`). Cards 7–11 can run side by side once card 5 lands, and card 7 waits for card 6 too. File names are the main ones, not every file.

| # | Card | Weight | After | Done when |
|---|---|---|---|---|
| 1 | **Measure: every signal, list, read and IPC command in the lists line** (§8) | normal | — | the lists line names every list and counts every signal and IPC by command (`window-log.test.ts` updated); the harness takes payloads, rev-gated `board_cards_by_id` and by-id commands; a baseline from a scratch copy with busy agents is on #1138 |
| 2 | **Core: no silent drops, every change names its ids** (§3.1, §3.2): card changes rerouted through the hub, with the relay keeping board-set changes as `Resync`; lost flags on `Boards::tell`, the inbox fan-out and `Book::flush` (refold sends one marker); `Kick` push + atomic; the coord `note()` gaps (mentions, waits, `spawn_now`, desk kind); `devcolor` publishes Devices; usage hold and `SPAWN_SET` publish `AGENTS_CHANGED`; `CoreEvent::Talk(TalkChange)` and `Notifications(NotifChange)` with no empty default, at every publish site; the mirrored DM from `feed-inbox` | normal | 1 | tests: a full queue gives a resync at each hop; opening, leaving, forgetting, renaming a shared board and hiding a DM space each reach the Board feed as `Resync`; each coord gap notes the right ids; every Talk/Notifications site carries its ids or `all`; `nothing_of_a_dm_space_shows_anywhere` moved to `core.feed(&[Topic::Board])`; `test_only_messages_that_show_as_dms_change_talk` still counts 1. The window's payloads are still `{}` |
| 3 | **Tray count without the full shared-board scan**: `decide_count` builds `card_summary` per task plus `gone_waits`, no `board_cards`, and counts what it counts today (private threads with open blockers included, as now) | normal | — | same counts in a test across private, shared and thread cards; no `board_cards` call left in `decide_count` |
| 4 | **Window: payloads with ids and seq** (§3.3, §4): the envelopes; `emit_change` with `seq` under one mutex; the kind map in `events.rs`; the board pump on Board + Presence + Roster + Inbox; the made-card index, one-hop family, queue diff, due timers, the 200 cap; `thread:` stripped from convs; empty changes → `all`; `inbox:changed.cards`; `Topic::Asks` in the notifications pump; the debug switches (§8); bindings, the `events.rs` table and surface.md | normal | 2 | `pumps.rs`: payload ids for each kind; `all` for each `all` rule (including a remote `POLICY_SET`, a re-fold, an empty ask id, a board-set `Resync`); `seq` counts up per name, including `boards_pin` |
| 5 | **Page: one listener module** (§5): `changes.ts` over every list-signal listener, seq tracking, gap/shown/mount/load-key → `all`, a hook's own `reload()` stays full, focus once a minute, the pending-ids patch hook on `latestReads`, the drift check (does nothing until a list patches); every list moved onto it, **still reading everything** | normal | 4, 1 | tests: gap → `all`, out of order → `all`, a hidden stretch → one `all`; notices during a read go into one follow-up with no read beside it; a dropped (stuck) and an errored patch answer each lead to a read covering their ids; a drift sample with a notice during it is dropped. The only intended changes: focus at most once a minute, and the talk listener held while hidden (each tested). Harness counts otherwise equal before and after |
| 6 | **Grid: summary cache and patch read** (§6.1): `Aux` + `deps`, dirty rows, `board_cards_patch`, the cold drift read on the host, the agent/lease index (§3.3 step 3 and the lease kinds), `decide_count` from the cache | heavy (the cache's invalidation and dependency index are the hard part) | 5, 3 | the cache equals a cold `cards_in("")` after every batch in a randomized test; a queued→delivered assignment notice re-reads that card's row; the drift read bypasses the cache and logs `drift cards-cache`; `useCards` patches; `board_cards` reads per minute drop to drift checks and `all`s |
| 7 | **Card store by id** (§6.2): `ids ∩ watched`; inbox cards; `people()`/`Followers`/`order_loops` once per call | normal | 6 | `card-store.test.ts`: a notice reads only the watched ids in it; harness: CardsById ids per notice = touched ∩ watched |
| 8 | **Threads by id** (§6.3): real `threads_by_id` → `{rows, gone}`; the page's upsert, sort and cap; the id tiebreak in Rust's sorts; drop inbox; talk convs; the follower's skip | normal | 5 | a test: by-id rows equal `threads_all`'s for those ids; a reopened thread sends `all`; marking a thread read patches its `unread`; `useAllThreads` no longer reads on `inbox:changed` |
| 9 | **Talk by conversation** (§6.4): `talk_conversations_by_id`, the thread-rows flag, posts by `convs`, huddles on any `calls`, transcript by its call | normal | 5 | a test: by-id rows equal the full list's; a post in conv A doesn't re-read conv B's posts; a huddle's start mark shows the new huddle |
| 10 | **Notifications by key** (§6.5): `notifications_by_key`, title rows by `taskId`, asks, `DeliveryLive` by `deliveries` | normal | 5 | `notifications.test.ts`; no full re-list on `inbox:changed`; an ask answered elsewhere updates its row |
| 11 | **Small lists and shared stores** (§6.6): desk, notes, idle, refs (with nulls), closed tab, boards/personal boards stores, devices store, inbox/held upsert, the full-read-only lists | normal | 5 | each list's trigger is in a test; a chip for an unknown ref shows the card after the notice that creates it; `boards_list` reads on Home's first load = 1 (not one per tile) |
| 12 | **Check it end to end**: the baseline again on a scratch copy with busy agents (lists line and IPC by command, before/after); drift over 30 minutes; the debug switches force a seq gap and a queue overflow | normal | 1, 6–11 | numbers on #1138: about one small by-id read per real change; `drift 0` and `drift cards-cache 0` over the compared samples, with the dropped samples reported; each forced fallback gives one full read |

## 10. Open questions

1. **Shared boards' `all`.** One board's resync sets `all` for every board, because the Board feed is one queue. If a busy shared board resyncs often, a per-board `all` would need a board id on `Change`. Card 12's numbers will tell.
2. **Others' agents (`AGENT_CARD`)** change names and status on cards and conversation rows with no event today. This design doesn't make that worse. Card 6's `deps` index could take those keys later.
3. **The 100–200 s single reads** on the Mac on 11/08 are a different problem: the coord lock held through a catch-up, or contention. Fewer reads lower the contention, but if card 12 still shows reads over 10 s, they need their own card.

   *Answered (card #1234):* it was neither. The coord lock is quiet under the load: ~25,000 takes a minute, about 1 s held and under 1 s waited in total, the longest single wait 12 ms. The reads over 10 s come once a minute, from the **inbox's** lock. `Inbox::sweep` (every 60 s) marks every agent with an open delivery as touched, and the lock's release (`Inner::unlock`) then asked `has_submit` and `has_append` of each one. Each of those scanned every open delivery about five times. On a copy of the person's data, with 7,078 open deliveries for 711 agents, mostly old ones to agents long gone, that held the inbox lock **16–18 s** a minute (debug build, Mac, six pretend agents). Every read that touches the inbox waited it out: `inbox_get` 17.8 s, `board_cards_patch` 16.3 s (16.2 s of it lock wait), `desk_list` 18.5 s (17.5 s), `board_idle_report` 17.5 s (17.5 s). They wait through the window's board pump (`expand` → `Inbox::get`), `people()` and the agents' streams. The release now groups the open deliveries by agent in one pass (`Inner::ready_flags`). irori.log names any lock wait or hold of 1 s or more by caller (`lock: held … at inbox/sweep.rs:…`) and any read of 1 s or more with its lock wait (`read: desk_list … lock wait …`). A dev-probe scratch run also writes per-minute lock and read totals beside the lists line.
