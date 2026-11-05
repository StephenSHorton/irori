# An Irori server, and how moves and board deletes go ahead (card #562)

*2026-11-05. Draft for Stephen to agree. Stephen held moves and board deletes (#558 b3, held card #563) until a server was designed. Inputs: decisions 30, 32, 33, 35 (B5) and 43; [spaces.md](spaces.md) §4; [public-relays.md](public-relays.md); [spaces-boards.md](spaces-boards.md) §3 on #558's branch. Line numbers are from main at 11fd8a16.*

## 1. The short answer

- **No server removes this wait, and moves and deletes don't need one.** A server can't change what an older Irori does with a row. But the wait exists only because the plan was to write two *new* row kinds. Written as rows every current Irori already stores, a move adds no wait of its own, and a delete waits no more than adding a board does today. (A move needs a board to move to, and adding that board waits at today's gate.)
- **The server is a hosted mailbox that Irori runs for everyone (§4.3).** Devices leave sealed rows there, and a sleeping device collects them when it wakes, with no always-on PC needed. It can't read what it holds. That needs new key work, so it's the biggest job here, and moves and deletes don't wait on it.
- **The always-on PC stays, with two new jobs (§4.2).** It stays on the newest Irori, and it carries the rows that show which devices are behind. It's still where agents run.
- **What a server can do for this wait, plainly:** it can help track which version each device runs, and it can help get devices updated sooner. It can't make an older device read a new row, and a mailbox that can't read rows can't even hold one back from an older device.

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

Phones run no agents, and they lag longest, so a phone is where someone would see a card in its old place. Desktops download an update at launch but install it only when someone presses Install or quits, and a PC left running doesn't look again. So until §4.2 ships, the window for agents can be weeks, on the machine most likely to run them.

### 3.5 Making this the last time: a tolerant reader

Carrying meaning in old rows suits these two changes, but it isn't a rule to keep using. The lasting fix is for the next Irori to store row kinds it doesn't know yet instead of stalling:

- **Pulled rows only.** A device's own writes stay strict, which catches our own bugs. `check_shape` serves both today, so it's split.
- **Only unknown `task.*` and `space.*` kinds.** Unknown `board.*`, `member.*`, `person.*` and `agent.*` stay refused: stalling on a membership or revoke row fails closed, and skipping one fails open.
- A kind stored this way must never later become `Refuses`, or that later build stalls on it.
- Signatures, certs, cuts and gap checks are unchanged. Stored rows fold after the update, as in the private log.
- The hello gains a level that says so. Once a space's devices are past it, new `task.*` and `space.*` kinds need no gate, as long as the feature is harmless on a device that doesn't read it yet.

It only helps from the release that ships it, so it can't serve moves and deletes on its own. It also stops new kinds starving other writers (§2).

## 4. The server

### 4.1 What exists

| | Stores rows? | Helps a sleeping device? | Who runs it |
|---|---|---|---|
| **n0 public relays** (decision 43) | No. Forwards sealed QUIC; a packet for an offline device is dropped | No | n0 |
| **Home server** (`crates/home`, decision 33) | No. The same relay code, plus port mapping and `node.addr`/`board.server` | No | Each person, a per-device switch, off by default |
| **Always-on PC** | Yes, as every member device does | Yes, by staying awake | Each person |

Store-and-forward isn't a feature in the code. Every member device holds the whole log and serves it to any admitted device (`mesh/src/boards.rs:99-126`). The always-on PC forwards because it's awake and holds what everyone holds. Either person's PC serves the other's devices. The only difference from a laptop is the home server, the keep-awake hold, and being on. Decision 43 credits home servers with store-and-forward; that's loose wording, since the member device stores and the home crate only relays. Files are the exception: a device serves one only if it holds it, and they're fetched on demand.

So today, anything that stores rows for a sleeping device is a member device, as decision 35 (B5) assumes. A non-member can hold rows only if it can't read them, which is what §4.3 builds.

### 4.2 The always-on PC: two new jobs

Each person runs it on their own machine, as decisions 30 and 33 say. Agents run here, and the mailbox doesn't change that. It gains two jobs, which are small and don't wait on §4.3:

**1. It stays on the newest Irori.** Today the tray app looks for updates only at start or from Settings, so a PC up for weeks can fall behind a phone, and an older PC is the worst forwarder (§2). A new per-device setting, **Keep this device up to date**, makes it check every few hours and download. It's on by default where the home server is on, and any desktop can turn it on; the home switch alone isn't enough, since decision 43 lets an always-on PC run with it off. Other desktops keep today's behaviour. (`serve --headless` never updates itself, so it can't take this role until that changes.)

The open question is the restart. Today Irori asks first (releasing.md, the Install / Later card). The recommended pick is that a device with this setting restarts by itself when no agent turn is running, and shows the card if that hasn't happened within a day. That's a change to how updates are approved, so it's choice 4 in §7. `architecture.md:433` and `releasing.md:7` change with it. The start-only check on #533 described the code; it wasn't a decision.

**2. It tells you who's behind.** Every device writes what it reads into the space, and the window shows it (§5). The always-on PC isn't special in this: a signed row each device writes about itself beats one machine's memory, since that machine only knows what it heard while it was up, and it can't vouch for another device. Its part is carrying those rows to devices that were asleep, which it already does.

### 4.3 The hosted mailbox (decided, b2)

**What it is.** A small Linux server program that holds rows sealed by devices. It has no keys, can't read or check what it holds, and runs no board logic. To a device it's one more peer that's always awake.

**What it does.**

- A writes a row and pushes it. B's phone wakes hours later and pulls it. A and B never have to be online together, and neither needs an always-on PC.
- It closes #538's mailbox follow-up: a DM reaches someone whose devices are all asleep.
- The mesh stays as it is, and the mailbox is added beside it. If the service is down or switched off, everything works as today.

**What it doesn't do.**

- **It doesn't remove the wait.** It can't see a row's kind, so it can't filter one or hold one back. An older device that used it would refuse the same row. It adds nothing to §3 or §5.
- **It doesn't track versions by itself.** The signed rows of §5 ride through it sealed. It only makes them reach sleeping devices sooner.
- **It doesn't replace agents' machines.** It can't run agents, hand out asks or hold leases, since it can't read cards. Those stay on devices.
- **Files aren't in the first version.** They stay fetched on demand from a device that holds them.

**Sealing.**

- A new log key per space and epoch: `logKey = HKDF(boardSecret(epoch), salt "irori/log/1", info boardId || epoch)`. The board secret isn't a log key today (v0.5 §6.1).
- A device seals each whole row, signature included, before pushing. Only the mailbox copy is sealed: the local log, the mesh and all row formats stay as they are, so there's no migration.
- A pull opens the row and hands it to the same `insert_board_rows` as a mesh pull, so `check_shape`, signature, cert and gap checks all apply. A row the mailbox altered or invented fails to open.
- The mailbox numbers what it receives, and a device pulls "everything after N". It sees no writer, seq or kind.
- A device pushes only its own rows, in order. That gives per-writer order, and a device that fell behind catches up from any source. A row pulled twice is already held.

**Rotation is the hard part.**

- The board secret rotates on every removal, leave and device revoke (v0.5 §6.1), which gives a new log key and a new mailbox address, so a removed person can't read new rows.
- Today the new secret goes only over live connections, sealed under the *old* board key. That can't go in a mailbox: the removed person holds the old key.
- So a rotation has to be sealed to each remaining device separately, with a per-device key, and left in the mailbox. A device can then wake and find the new secret with nobody else online. Devices have signing keys today; this needs an encryption key per device, published in the cert rows or derived from the signing key.
- A person's private log needs the same, under the person secret. **Spaces come first, Personal second.**
- A new member gets the whole log in the invite bundle, so never needs old mailbox epochs.

**Who may use a mailbox.** Access is a proof from a key derived from `logKey`, so only someone holding the epoch's secret can push or pull. The service stores a public key per mailbox and knows no people. The next epoch's mailbox is registered with a proof from the last, so nobody can take its address first.

**What the operator sees and can do.**

- Sees: IP addresses, mailbox ids, row sizes, times, and which addresses use the same mailbox. That's about what n0 sees (decision 43), plus how much each space holds.
- Can: drop rows, or refuse service. A member notices only through the mesh. Can't: read, forge or reorder a row without it being caught.
- A later leak of a space's secret opens anything still held from that epoch. Retention limits that.

**Keeping it small.**

- It's a mailbox, not an archive. Rows go after a fixed time, 30 days to start with. A device asleep longer catches up from any peer.
- Each mailbox has a size limit. One release once wrote about 3.4M `task.spawn` rows (`crates/dev/src/spawn_rows.rs`), so a limit isn't theoretical. When a mailbox is full, new pushes are refused and the mesh carries on.
- With no accounts, creating a mailbox costs a small proof of work, and addresses are rate limited.

**How it's built.**

- A separate binary with no Tauri, so it needs no desktop libraries and can run on any Linux host. It's the first Linux target, and it holds no keys, so a rented host is fine.
- Devices reach it on its own iroh protocol (`irori/mailbox/1`) with the server's key pinned, as home relays are.
- The address is a setting, so anyone can run their own, like the relay list.

**This reverses part of decisions 32 and 35.** B5 says row encryption "returns only if non-members ever carry boards". This is that case, and it applies to the mailbox copy only. It needs its own decision row.

### 4.4 Not planned

**A rented machine as one of your own devices.** It would need a Linux build of the whole app, a headless updater and a key store, since off Windows and macOS keys sit in a plain file and every device holds the person secret. The host could act as you. The mailbox gives most of the benefit with none of that exposure.

## 5. Which devices block, and how we know

The gate stays for adding or deleting a board, space Talk and notes. Three changes.

**Keep the last hello on disk.** Each device saves the last hello it heard from each device, per space. Without this a restart forgets a device known to be too old.

**Each device says what it reads, in a signed row.**

- Every device writes `board.server` with new `reads {spaces, spawns, …}` and `version` fields, into each space, whenever they change. It's the one row already bound to the device that signs it (`identity/board.rs:1109-1116`), and older decoders keep working, so nothing stalls.
- Only a device whose home server has run writes this row today, so the writer needs a path that doesn't depend on the home server, with one builder for the row. `reads` and `version` are fields on `BoardServer`, so the on-change compare sees them.
- `reads` absent means no word, not level 0, or every older home server would read as too old.
- A phone's row has `public: null`. The only production reader skips those (`meshnode.rs:207`), so a phone isn't taken for a home server.
- A live hello beats the row, which covers a downgrade.
- What it's for: a device can know about a device it never met, and the Devices page and warnings can show who's behind, on any device. That fixes two gaps: the out-of-date warning skips your own devices, and it keeps only a person's newest version, so Blake's laptop hides Blake's old phone (`boards_status.rs:136-145`). It also means a future level can name a sleeping device correctly.
- What it isn't for: an older build writes no `reads`, and every build that does is past today's levels. So at today's levels these rows only ever say ready. They don't themselves remove the wait; the next change does.

**Block only on a device known to be too old.** Today a device nobody has heard from blocks. Instead:

- A device blocks if its latest word (live hello, saved hello, else row) is below the level the row needs.
- A device with no word doesn't block. It gets a line in the warnings, naming it.
- If the space has no mesh, the writer has no word on anyone, so the same rule applies.

What it buys: a device nobody has any word from can't refuse a write. A device known to be too old still can, until it updates or its owner revokes it, so a new level would still carry the §2 risk. The refusal keeps naming it. Keep "waiting on" in the text; tests pin it (`spaces.rs:553`, `five_cores.rs:1335`).

The cost: any device on a build before this one that nobody has heard since its last start can wake and stall until it updates. It heals, but until then §2's worst case applies: if that writer's stream holds a removal or a revoke behind the stall, that device keeps admitting the person. The gap is small and shrinks: it needs a device from before v0.11.0 that also goes unheard, and §4.2 keeps the always-on PC, the device where it matters most, from being that one. Stephen should accept this knowingly, so it's choice 3 in §7.

#538 uses the same test for a hidden log's first write, but it's safe there because an older device is never handed the log. A space has no such check, which is the cost above.

**Spawns keep today's rule, every device heard.** Their gate is silent, automatic and on the busiest stream, so failing open would cost most there. Their wait refuses nobody; agents just don't start. Only the saved hello changes for them.

## 6. Order of work

1. **#563 as amended:** the move and delete of §3, in the fold and window. Its objective and spaces-boards.md §3 name `task.board`, `space.board.deleted` and `SPACES_PROTOCOL` 2; this replaces those.
2. **Saved hellos, version rows and the new gate rule (§5).** A delete benefits at once, and so do adding a board, Talk and notes.
3. **The always-on PC's updates (§4.2).**
4. **The tolerant reader (§3.5).**
5. **The hosted mailbox (§4.3), in this order:** the key work (log key, per-device sealing, rotation through the mailbox) as a spike first, since the rest depends on it; then the server program; then the device side for spaces; then Personal; then running it.

Steps 2 to 5 are separate cards, and #563 doesn't wait on them: a delete works with today's gate, only with today's wait.

## 7. Choices for Stephen

Decided on #562:

1. **Moves and deletes (b1):** carry them in rows every current device reads (§3), and build it now.
2. **The server (b2):** a hosted service Irori runs for everyone (§4.3).
3. **Which devices block (b3):** only those known to be too old; unheard ones get a warning (§5).

Still open:

4. **The always-on PC's restart (b4):** by itself when no agent is working (§4.2), or keep the Install / Later card and only check and download on a timer.
5. **Can the service read what it holds?** Sealed (§4.3), or readable.
6. **Who runs it and pays?**
7. **Is it on by default?**
