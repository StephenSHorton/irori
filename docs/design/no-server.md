# No server: Irori stays peer to peer, and how moves and board deletes go ahead (card #562)

*2026-11-05. Agreed with Stephen on #562. Stephen held moves and board deletes (#558 b3, held card #563) until a server was designed. The design work showed they don't need one, and after weighing a hosted service, Stephen chose to stay decentralized and make that way of working solid. Inputs: decisions 30, 32, 33, 35 (B5) and 43; [spaces.md](spaces.md) §4; [public-relays.md](public-relays.md); [spaces-boards.md](spaces-boards.md) §3 on #558's branch. Line numbers are from main at 11fd8a16.*

## 1. The short answer

- **Irori has no server.** Devices are peers. Every device holds the whole log and hands rows to any admitted device. Any machine that's on often helps, and more often means fresher. An always-on PC is the best case, not a requirement.
- **No server would have removed this wait anyway.** Nothing can change what an older Irori does with a row. But the wait exists only because the plan was to write two *new* row kinds. Written as rows every current Irori already stores, a move adds no wait of its own, and a delete waits no more than adding a board does today. (A move needs a board to move to, and adding that board waits at today's gate.)
- **What we do instead:** harden the peer model, so a stall can't hide, old data doesn't look current, and machines stay up to date (§4, §5).

## 2. What the wait really is

Checked in the code, because some of the docs are off:

- **Only board logs are strict.** `check_shape` refuses an unknown kind (`crates/store/src/board.rs:255`, called at `db.rs:1303`). The private log stores unknown kinds and folds them after an update, so Personal never needed a gate.
- **A stall stops one writer, on one device.** The older device stores that writer's rows up to the unknown one and retries that row on every pull, with no backoff. Other writers' rows still land, and its own rows still go out. spaces.md §4 says later rows fail the gap check; on a pull, the same row is simply refused again. The result is the same.
- **A stall heals by itself.** Nothing is lost or forked: the row was never stored, so once the device updates it pulls from where it stopped.
- **A stall is not harmless while it lasts.**
  - Rows behind the stall aren't seen, including a member's removal or a device's revoke. An older always-on PC could keep admitting someone removed in those rows (`identity/board.rs:893-900`). This is the worst case.
  - Pulls are capped at 2000 rows and walk devices in byte order (`db.rs:1508-1534`), so on a busy space a stall can starve writers that sort after the stalled one.
  - The stalled device shows nothing about itself. Others see "X is stalled" after about two minutes (`boards_status.rs:30-32`).
- **Today's gate is weaker than it looks.**
  - `heard` is in memory (`boardmesh.rs:415, 458`) and fills only from hellos this device heard directly. It's lost at restart, or when that space's mesh closes.
  - It's checked only when writing, so an older device certified later is never counted.
  - **"Until it's revoked" (spaces.md §4) means only its owner can do it.** A member revokes only their own devices (`identity/board.rs:306-307`). If Blake's phone sleeps forever, Stephen can only ask Blake, or remove Blake from the space, and can't do that if Blake is an owner. New kinds behind that gate could be blocked for good by someone else's forgotten phone. That's the strongest reason not to build moves that way.

## 3. How moves and board deletes go ahead

### 3.1 A move

A move is an edit of the card, so it uses the edit row:

`board.ask.edited {taskId, board, num, by, at}`, or `task.edited` with the same fields in Personal.

- Board logs have accepted this kind since v0.7.1 at the latest. It's what `edit_task` (`cards.rs:2375`) writes on a board, under the name `ask_kind` gives it (`asks.rs:157-167`).
- **No gate.** `space_not_ready` is called only from adding a board, space Talk and notes, not from edits.
- `edit_gate` (`asks.rs:716`) decides whether the row folds. It doesn't lock: a person's row always folds, and a non-taker agent's row waits and lands when the ask closes.
- Decoders drop fields they don't know (`gocompat/src/json/lenient.rs`), so an older device stores the row and leaves the card where it was. It adds one "edited the card" line, which watchers and the parent's holders get as a next-turn digest. It wakes nobody, and it bumps the card's updated time. The new fold says "moved it to net" instead, and rebuilds history on the next start.
- `board` and `num` are safe names. A new field must not reuse, even by case, a field of `TaskEditedRow` (`rows.rs:497`) or of the store's closure decoder (`store/src/board.rs:97`).
- **It needs its own write path.** `edit_task` refuses an edit with no title, objective or done-when, and writes nothing if nothing changed (`cards.rs:2380-2384`, `2413-2418`). The store and fold have no such rule. #563 adds a move path, adds `board` and `num` to `TaskEditedRow`, refuses an open ask as `edit_task` does (`cards.rs:2407`), and checks the move landed.
- **A move carries its subtasks.** The writer writes one row per subtask, parent first, and a subtask can't move alone while it has a parent. Otherwise `relink` leaves a parent and its subtasks on different boards.

Tested on main, v0.11.0 and v0.10.2: this row, signed and inserted into a board store, folds as "edited the card" and the card stays put.

### 3.2 A board delete

`space.board.renamed {boardId, name: "", tag: "", deleted: true, by, at}`

- On an older device it's a stored no-op: `fold_space_boards` ignores `deleted`, and empty names change nothing (`spaces.rs:74-79`). `name` and `tag` must be present, even empty: they have no serde default, so a row without them is skipped.
- Accepted since v0.11.0, the first release with `space.board.*`; v0.10.2 refuses it. So it sits behind the same gate as adding a board. **A delete waits exactly as an add does today, no worse, and §5 shortens that wait.**
- Tested on main and v0.11.0: the row is stored and the board stays as it was. A row without `name` and `tag` is stored and skipped.
- Its cards move to the default board in the fold, taking numbers there, as agreed on #558 (b4). No row is written per card.

### 3.3 What the new fold must do

Replay gives the right answer in every order tried, but the live fold doesn't by itself: it applies edits in arrival order. In a test on main, two concurrent edits showed the older one live and the newer after a restart. So #563 needs all of these. Most come from spaces-boards.md §2, and they still apply though `task.board` is gone.

**Moves**

1. **The last move wins by RowPos, not by arrival.** The fold keeps each card's newest move position and ignores a move at an earlier one.
2. **A move that arrives before its card is kept and applied after,** as `links` does (`subtasks.rs:35-44`). Boards already set such a row aside; the private log drops it.
3. **A move claims a number.** `claim_short` ignores a card that has already claimed (`short.rs:90`), so this is a new claim on (board, number) at the row's position. A clash takes letters as creates do, the losing claim is dropped, and both boards settle. The "renumbered it" line gets its own wording.

**Deletes**

4. **A delete wins whatever the arrival order.** A later rename, a repeat create or a move onto that board doesn't bring it back.
5. **A card that names a deleted board lands on the default board.** An older device, or a newer one a moment before the delete arrived, can still make one. Signed rows can't be rewritten, so the fold maps it.
6. **Its numbers come from RowPos over all rows,** recomputed in `renumber`, never at arrival, so every device gets the same ones.
7. **A delete needs a check on who wrote it.** `fold_space_boards` sees only the kind and payload, so it has to take the writer too: an active member, per b4's "any member".
8. **The first board can't be deleted, nor a space's last board.** Older folds skip the space's own id anyway.

**Both**

9. **Derive a card's board and tag; don't store them at create.** `Task.tag` is set only at create (`fold.rs:1143`), from the create row, so today a moved card would keep its old tag for good, even after a restart. And the fold never hears `space.board.*`: `space.` must join `FOLLOW_KINDS`, and a card's board and tag come from its newest move plus the board list. A board the log doesn't hold yet isn't final, so re-derive when it arrives. Personal needs its own lookup, since `board_tag` returns None there (`fold.rs:1984`).
10. **Old refs keep working.** Each card keeps a history of (tag, number), `task_ref_locked` reads it, and `has_tag` reads every tag the log has named. A deleted board's tag is never reused, and a number a card moved away from stays claimed.

### 3.4 What this costs

While devices are mixed, an older device shows the old state until it updates:

- The moved card sits on its old board, with its old number. One card has two names, and a bare `#4` typed there can mean a different card from `#4` on an updated device.
- A deleted board still shows there. Someone can add a card to it, and that card lands on the default board after the update.
- An agent on an older device can keep working a moved card. Comments, claims and status key on the task id, so they land on the right card wherever it now sits.

All of it comes right when the device updates, because the rows are stored and replayed at each start. Nobody's access changes, since a space is one log with one secret.

Phones run no agents, and they lag longest, so a phone is where someone would see a card in its old place. Desktops download an update at launch but install it only when someone presses Install or quits, and a PC left running doesn't look again. So until §4.3's update setting ships, the window for agents can be weeks, on the machine most likely to run them.

### 3.5 Making this the last time: a tolerant reader

Carrying meaning in old rows suits these two changes, but it isn't a rule to keep using. The lasting fix is for the next Irori to store row kinds it doesn't know yet instead of stalling:

- **Pulled rows only.** A device's own writes stay strict, which catches our own bugs. `check_shape` serves both today, so it's split.
- **Only unknown `task.*` and `space.*` kinds.** Unknown `board.*`, `member.*`, `person.*` and `agent.*` stay refused: stalling on a membership or revoke row fails closed, and skipping one fails open.
- A kind stored this way must never later become `Refuses`, or that later build stalls on it.
- Signatures, certs, cuts and gap checks are unchanged. Stored rows fold after the update, as in the private log.
- The hello gains a level that says so. Once a space's devices are past it, new `task.*` and `space.*` kinds need no gate, as long as the feature is harmless on a device that doesn't read it yet.

It only helps from the release that ships it, so it can't serve moves and deletes on its own. It also stops new kinds starving other writers (§2).

## 4. Peers, not a server

### 4.1 What exists

| | Stores rows? | Helps a sleeping device? | Who runs it |
|---|---|---|---|
| **n0 public relays** (decision 43) | No. Forwards sealed QUIC; a packet for an offline device is dropped | No | n0 |
| **Home server** (`crates/home`, decision 33) | No. The same relay code, plus port mapping and `node.addr`/`board.server` | No | Each person, a per-device switch, off by default |
| **Any member device** | Yes, the whole log | Yes, while it's on | Each person |

Store-and-forward isn't a feature in the code. Every member device holds the whole log and serves it to any admitted device (`mesh/src/boards.rs:99-126`). The always-on PC forwards because it's awake and holds what everyone holds. Either person's devices serve the other's. The only difference from a laptop is the home server, the keep-awake hold, and being on. Decision 43 credits home servers with store-and-forward; that's loose wording, since the member device stores and the home crate only relays. Files are the exception: a device serves one only if it holds it, and they're fetched on demand.

### 4.2 What needs a machine on

| | Needs |
|---|---|
| Moving cards, adding and deleting boards | Nothing. They write locally and sync later |
| Your own devices staying in step | Any two on at the same time |
| Shared spaces | The same; other people's devices count too |
| Agents | The machine they run on. Their work waits while it's off |
| Files | A device that holds the file |
| Phones | They catch up when opened. Whether they sync in the background isn't confirmed |

With a machine off you lose freshness, not data. Rows wait on whichever devices have them.

**The one gap we accept:** two devices that are never on at the same time, such as a laptop and a phone with no third machine, don't catch up with each other. The same goes for two people with no machine on at once. Only something that stores rows while everyone is off closes this, and we chose not to build that (§4.4).

### 4.3 Making it solid

**Keep machines up to date.** Today Irori looks for updates only at start or from Settings, so a machine up for weeks falls behind a phone, and an older machine is the worst forwarder (§2).

- A new per-device setting, **Keep this device up to date**, makes it check every few hours and download. It's on by default where the home server is on, and any desktop can turn it on. The home switch alone isn't enough, since decision 43 lets an always-on PC run with it off.
- Agreed (b4): a device with this setting restarts by itself when no agent turn is running, and shows the Install card if that hasn't happened within a day. `architecture.md:433` and `releasing.md:7` change with it. The start-only check on #533 described the code; it wasn't a decision.
- `serve --headless` never updates itself, so it stays outside this until that changes.

**Say how fresh things are.** A space shows when it last synced, and with whom ("Last synced with the PC 3 hours ago"), so old data doesn't look current.

**Don't let a stall hide (§2).**

- A stalled device says so about itself. Today it shows nothing, and only others see "X is stalled".
- Retries back off, instead of re-downloading up to 2000 rows on every pull.
- A pull takes turns between writers, so one stall can't starve writers that sort after it (`db.rs:1508-1534`).
- The tolerant reader (§3.5) stops new kinds stalling anyone at all.

**Leave nothing stranded.**

- A device syncs as far as it can before it sleeps or closes, so the last thing you did has left the machine.
- Desktops pick up small files as they're offered, so a sleeping laptop isn't the only holder.
- Find out what phones really do in the background, and write it down.

**Say it plainly.** The window and docs stop saying a device needs an always-on PC.

### 4.4 Considered and not chosen

**A hosted mailbox that Irori runs.** Devices would leave rows sealed for a sleeping device, and the service couldn't read them. It would close §4.2's one gap and #538's mailbox follow-up. Stephen chose it at first (b2), then decided against it after weighing what it adds. It wouldn't touch this wait, since it can't read a row's kind. It would mean a new key scheme, rotation sealed per device, and a live service to pay for and look after, for a gap that rarely matters to anyone with a machine on often. It would also be the first thing that isn't a peer. The unreviewed sketch is in this file's history (commit e7cf1a2) if that ever changes.

**A rented machine as one of your own devices.** It needs a Linux build of the whole app, a headless updater and a key store, and its host could act as you, since every device holds the person secret and keys sit in a plain file off Windows and macOS. Anyone who wants one more machine that's on often can add a device they trust.

## 5. Which devices block, and how we know

The gate stays for adding or deleting a board, space Talk and notes. Three changes.

**Keep the last hello on disk.** Each device saves the last hello it heard from each device, per space. Without this a restart forgets a device known to be too old.

**Each device says what it reads, in a signed row.**

- Every device writes `board.server` with new `reads {spaces, spawns, …}` and `version` fields, into each space, whenever they change. It's the one row already bound to the device that signs it (`identity/board.rs:1109-1116`), and older decoders keep working, so nothing stalls.
- Only a device whose home server has run writes this row today, so the writer needs a path that doesn't depend on the home server, with one builder for the row. `reads` and `version` are fields on `BoardServer`, so the on-change compare sees them.
- `reads` absent means no word, not level 0, or every older home server would read as too old.
- A phone's row has `public: null`. The only production reader skips those (`meshnode.rs:207`), so a phone isn't taken for a home server.
- A live hello beats the row, which covers a downgrade.
- A saved hello and a row go by which the device wrote later, on its own row counter: a saved hello keeps the device's own entry of the hello's vector, and a row its seq. No clock decides. The same hello heard again is saved again only when a later row said otherwise (#577).
- What it's for: a device can know about a device it never met, and the Devices page and warnings can show who's behind, on any device. That fixes two gaps: the out-of-date warning skips your own devices, and it keeps only a person's newest version, so Blake's laptop hides Blake's old phone (`boards_status.rs:136-145`). It also means a future level can name a sleeping device correctly.
- What it isn't for: an older build writes no `reads`, and every build that does is past today's levels. So at today's levels these rows only ever say ready. They don't themselves remove the wait; the next change does.

**Block only on a device known to be too old.** Today a device nobody has heard from blocks. Instead:

- A device blocks if its latest word (live hello, saved hello, else row) is below the level the row needs.
- A device with no word doesn't block. It gets a line in the warnings, naming it.
- If the space has no mesh, the writer has no word on anyone, so the same rule applies.

What it buys: a device nobody has any word from can't refuse a write. A device known to be too old still can, until it updates or its owner revokes it, so a new level would still carry the §2 risk. The refusal keeps naming it. Keep "waiting on" in the text; tests pin it (`spaces.rs:553`, `five_cores.rs:1335`).

The cost: any device on a build before this one that nobody has heard since its last start can wake and stall until it updates. It heals, but until then §2's worst case applies: if that writer's stream holds a removal or a revoke behind the stall, that device keeps admitting the person. The gap is small and shrinks: it needs a device from before v0.11.0 that also goes unheard, and §4.3 keeps machines that are on most, where it matters most, from being that one. Stephen accepted this (b3).

#538 uses the same test for a hidden log's first write, but it's safe there because an older device is never handed the log. A space has no such check, which is the cost above.

**Spawns keep today's rule, every device heard.** Their gate is silent, automatic and on the busiest stream, so failing open would cost most there. Their wait refuses nobody; agents just don't start. Only the saved hello changes for them.

## 6. Order of work

1. **#563 as amended:** the move and delete of §3, in the fold and window. Its objective and spaces-boards.md §3 name `task.board`, `space.board.deleted` and `SPACES_PROTOCOL` 2; this replaces those.
2. **Saved hellos, version rows and the new gate rule (§5).** A delete benefits at once, and so do adding a board, Talk and notes.
3. **Keep this device up to date (§4.3).**
4. **Stalls that can't hide, and the tolerant reader (§4.3, §3.5).**
5. **Freshness, sync before sleep, small files, and what phones do (§4.3).**

Steps 2 to 5 are separate cards, and #563 doesn't wait on them: a delete works with today's gate, only with today's wait.

## 7. What Stephen decided on #562

1. **Moves and deletes (b1):** carry them in rows every current device reads (§3), and build it now.
2. **Which devices block (b3):** only those known to be too old; unheard ones get a warning (§5).
3. **Restart (b4):** a device with the setting restarts by itself when no agent is working (§4.3).
4. **The server:** none. A hosted service was chosen first (b2), then dropped: "solidify this decentralized way of working" (§4.4).
