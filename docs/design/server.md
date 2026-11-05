# An Irori server, and how moves and board deletes go ahead (card #562)

*2026-11-05. Draft for Stephen to agree. Stephen held moves and board deletes (#558 b3, held card #563) until a server was designed. Inputs: decisions 30, 32, 33, 35 (B5) and 43; [spaces.md](spaces.md) §4; [public-relays.md](public-relays.md); [spaces-boards.md](spaces-boards.md) §3 on #558's branch. Line numbers are from main at 11fd8a16.*

## 1. The short answer

- **No server removes this wait, and moves and deletes don't need one.** A server can't change what an older Irori does with a row. But the wait exists only because the plan was to write two *new* row kinds. Written as rows every current Irori already stores, a move needs no wait at all, and a delete waits no more than adding a board does today.
- **The server is the always-on PC, with two new jobs.** It stays on the newest Irori, and it tells you which devices are behind. Nothing else we could build does more for this wait, and the alternatives cost a lot more (§4).
- **What a server can do for this wait, plainly:** it can help track which version each device runs, and it can help get devices updated sooner. It can't make an older device read a new row.

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

- Board logs have accepted this kind since v0.10.2 or earlier. It's what `edit_card` writes on a board (`asks.rs:157-167`).
- **No gate.** `space_not_ready` is called only from adding a board, space Talk and notes, not from edits.
- `edit_gate` (`asks.rs:716`) already decides who may write it, and it keeps an open ask's take lock.
- Decoders drop fields they don't know (`gocompat/src/json/lenient.rs`), so an older device stores the row, adds one "edited the card" line, and leaves the card where it was. The new fold says "moved it to net" instead, and rebuilds history on the next start.
- `board` and `num` are safe names. A new field must not reuse one of that row's existing fields, even by case (`from`, `to`, `by`, `note`, `parent`).

### 3.2 A board delete

`space.board.renamed {boardId, name: "", tag: "", deleted: true, by, at}`

- On an older device it's a stored no-op: `fold_space_boards` ignores `deleted`, and empty names change nothing (`spaces.rs:74-79`). `name` and `tag` must be present, even empty: they have no serde default, so a row without them is skipped.
- Accepted since v0.11.0, the first release with `space.board.*`. So it sits behind the same gate as adding a board. **A delete waits exactly as an add does today, no worse, and §5 fixes that wait.**
- Its cards move to the default board in the fold, taking numbers there, as agreed on #558 (b4). No row is written per card.

### 3.3 What the new fold must do

1. **A delete wins whatever the arrival order.** A later rename, a repeat create or a move onto that board doesn't bring it back.
2. **A card that names a deleted board lands on the default board.** An older device, or a newer one a moment before the delete arrived, can still make one. Signed rows can't be rewritten, so the fold maps it.
3. **Re-tag live.** `Task.tag` is set only at create (`fold.rs:1143`), so a move would otherwise show the old tag until a restart.
4. **A deleted board's tag is never reused,** so old refs keep finding their cards (spaces-boards.md §2).
5. **A delete needs a check on who wrote it.** `fold_space_boards` sees only the kind and payload, so it has to take the writer too: an active member, per b4's "any member".
6. **The first board can't be deleted, nor a space's last board.** Older folds skip the space's own id anyway.

### 3.4 What this costs

While devices are mixed, an older device shows the old state until it updates:

- The moved card sits on its old board, with its old number. One card has two names, and a bare `#4` typed there can mean a different card from `#4` on an updated device.
- A deleted board still shows there. Someone can add a card to it, and that card lands on the default board after the update.
- An agent on an older device can keep working a moved card. Comments, claims and status key on the task id, so they land on the right card wherever it now sits.

All of it comes right when the device updates, because the rows are stored and replayed at each start. Nobody's access changes, since a space is one log with one secret. Phones run no agents, and desktops fetch updates by themselves, so the mixed window for agents should be short. Phones lag longest, and the phone is where someone would see a card in its old place.

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
| **Home server** (`crates/home`, decision 33) | No. The same relay code, plus port mapping and `node.addr`/`board.server` | No | Each person, a switch on one of their devices |
| **Always-on PC** | Yes, as every member device does | Yes, by staying awake | Each person |

Store-and-forward isn't a feature in the code. Every member device holds the whole log and serves it to any admitted device (`mesh/src/boards.rs:99-126`). The always-on PC forwards because it's awake and holds what everyone holds. Either person's PC serves the other's devices. The only difference from a laptop is the home server, the keep-awake hold, and being on.

So anything that stores rows for a sleeping device has to be a member device, which is what decision 35 (B5) already says. The always-on PC is that device.

### 4.2 Recommended: the server is a role the always-on PC takes on

Each person runs it on their own machine, as decisions 30 and 33 say. Nothing new to install, and no new trust. It gains two jobs:

**1. It stays on the newest Irori.** Today the tray app looks for updates only at start or from Settings, so a PC up for weeks can fall behind a phone, and an older PC is the worst forwarder (§2). The device with the home server on checks every few hours, downloads, and installs when no agent turn is running; it raises a notification if that doesn't happen within a day. Other desktops keep today's behaviour. (`serve --headless` never updates itself, so it can't take this role until that changes.)

**2. It tells you who's behind.** Every device writes what it reads into the space, and the window shows it (§5). The always-on PC isn't special in this: a signed row each device writes about itself beats one machine's memory, since that machine only knows what it heard while it was up, and it can't vouch for another device. Its part is carrying those rows to devices that were asleep, which it already does.

### 4.3 Not recommended now

**A rented machine you own (a VPS as one of your devices).** Always on and off your home network, so it would serve a space when your PC is down. As things stand it would need:

- A Linux build. There's no Linux target; the app links Tauri and its web view.
- A headless updater and a way to switch on the home server. Both are window-only or missing.
- A key store. Off Windows and macOS, keys sit in a plain file, and every device holds the person secret, so the host can act as you on every space.

It does nothing more for this wait than the PC does.

**A service Irori runs for everyone.** A relay of our own is cheap and allowed (public-relays.md, "Not done here"), but it stores nothing. A service that stores rows is a non-member carrying boards, which brings back board-log encryption under B5, a new key scheme, and a new way to check members. It couldn't hear hellos either, so it couldn't even track versions. Decision 32 already rejected this for good reason. It would make sense only if Irori wants offline delivery with no always-on PC at all. That's a separate question, also behind #538's mailbox follow-up, and this design leaves room for it.

## 5. Which devices block, and how we know

The gate stays for rows that need it (adding, renaming or deleting a board, space Talk, notes, and spawns). Two changes.

**Remember what each device reads: a signed row each device writes.**

- Every device writes `board.server` with a new `reads {spaces, spawns, …}` and `version`, into each space, whenever they change. It's the one row already bound to the device that signs it (`identity/board.rs:1109-1116`), and older decoders drop the extra fields, so nothing stalls.
- Phones don't write `board.server` today. They'd write it with `public: null`, which relays ignore (`meshnode.rs:207`). Every reader of `board_servers` has to be checked first, so a phone isn't taken for a home server.
- A live hello beats the row, which covers a downgrade.
- Now a restart loses nothing, and a device can know about a phone it never met directly. The Devices page and the warnings read the same rows, fixing two gaps: the out-of-date warning skips your own devices, and it keeps only a person's newest version, so Blake's laptop hides Blake's old phone (`boards_status.rs:136-145`).

**Block only on a device known to be too old.** Today a device nobody has heard from blocks. Instead:

- A device blocks if its latest word (live hello, else row) is below the level the row needs.
- A device nobody has any word from doesn't block. It gets a line in the warnings, naming it.
- #538 already needs this rule for a hidden log's first write, so it's one rule rather than two.

The cost: a really old device nobody has heard from can wake up and stall until it updates. That heals (§2), and the warnings name it. What it buys: no sleeping or forgotten device can refuse a write. A device known to be too old still blocks, and the refusal keeps naming it. Keep "waiting on" in the text; tests pin it (`spaces.rs:553`, `five_cores.rs:1335`).

## 6. Order of work

1. **#563 as amended:** the move and delete of §3, in the fold and window. Its objective and spaces-boards.md §3 name `task.board`, `space.board.deleted` and `SPACES_PROTOCOL` 2; this replaces those.
2. **Version rows and the new gate rule (§5).** A delete benefits at once, and so do adding a board, Talk and notes.
3. **The always-on PC's updates (§4.2).**
4. **The tolerant reader (§3.5).**

Steps 2 to 4 are separate cards, and #563 doesn't wait on them: a delete works with today's gate, only with today's wait.

## 7. Choices for Stephen

Raised on #562. Recommended picks are the ones written above.

1. **Moves and deletes:** carry them in rows every current device reads (§3), or new row kinds behind a level 2 gate, which any device that never updates can block.
2. **The server:** a role on the always-on PC (§4.2), or a rented machine as well (§4.3).
3. **Which devices block:** only those known to be too old (§5), or everyone not yet heard from, as today, but remembered across restarts.
