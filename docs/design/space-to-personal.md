# Moving a one-person space into Personal (card #640)

*2026-11-06. Stephen's Wind Waker, Lemmings and Irori spaces were meant to be private boards. They became spaces because they were made as shared boards before Personal could hold boards ([spaces-boards.md](spaces-boards.md)). This page says how a space with only its owner in it becomes a Personal board. Line numbers are from main at 3a5e9d90.*

## 1. Settled

- **No new row kinds** (decision 48). The move writes rows the private log already folds: `space.board.created`, the card kinds, `attach.offered` and `board.forgot`. Each copied row carries one extra field, `moved` (`<spaceId>/<device>/<seq>` of the row it copies), which older decoders drop as they drop the space's `hop`. Every Irori from #558 on (v0.12.4) reads the rows, so only the device that runs the move needs the new code. The phone and the PC just receive rows.
- **Cards keep their task id, number and times.** wind#N and lemmings#N find the same card, and every `task:<id>` in old messages, parents and blockers still resolves. `Services::route` (boards.rs:1717) asks the private log first, so the copy answers as soon as it is written.
- **Who can move:** a space where nobody but its owner (this person) is an active member. That is the test `leave_board` already uses to forget everywhere (boards_members.rs:207-212). Horton has another person, so it is refused. A hidden DM space is refused, and so is a space with boards of its own inside (none of the three has any).
- **The new board gets a new id** (`new_board_id()`), never the space's id, so nothing keyed by board id (switcher pins, `board:<id>`) mixes the gone space with the new board.
- **How it runs (Stephen, b1 on #640):** nothing in the window and no agent tool. The installed Irori has one local call, `POST /v1/spaces/move-to-personal?space=<id>[&keep=1][&missing=1]` (the lockfile's token, like the other POST routes), and an agent makes it once per space on the Mac after the Mac has the release. Running inside the live app means it is online (files fetch over the space), the forget reaches the other devices at once, and Irori is never quit or opened by another build.

## 2. The steps, in one Core call

`Core::move_space_to_personal(space_id, MoveOpts)` (`crates/core/src/service/space_move.rs`), run on one device:

1. **Check.** One-person space, open on this device, not hidden, one board. Its tag is held by nobody but this space, or by the board an earlier run of this move made (§2.7).
2. **Files.** Every file a comment or a blocker attaches is made local (fetched over the person's devices, then the open space's) and offered in the private log (`attach.offered`), so the other devices can fetch it after the space is gone. A file no device gives stops the move before anything is written, unless `allow_missing_files` (`missing=1` on the route). Each fetch has 120 s; Wind's seven PC-only files are about 1.1 GB, so they are best fetched to the Mac before the run.
3. **Board.** `space.board.created {boardId: new, name, tag, moved: <spaceId>}` in the private log. The space's own name and tag carry over.
4. **Copy.** Every row of the space log, in fold order (`Store::list`: hlc, device, seq), that is card content is appended to the private log in that order (§3).
5. **Fold, quietly.** The private fold never sees this device's own rows coming back from the store (its writes fold as they're made, `Coord::handle`), so after the copy it is rebuilt once as a replay (`Coord::refold`). A replay queues nothing for watchers and runs no duty, so no old row sends an update note, and no duty acts on a card that is half copied.
6. **Compare.** Each card of the space against its copy: number, title, state, comments, steps and their ticks, blockers and open ones, subtasks, parent, thread, and that it is on the new board. Any difference stops the move here with the space kept.
7. **Forget everywhere.** `forget_everywhere(space_id)` writes `board.forgot`. Every device of the person forgets the space and deletes its files when that row arrives (`reconcile_boards`, boards_own.rs:450).

**Going on after a stop.** A run that stopped part way is run again: the board it made (its `moved` names the space) is used again, and rows whose `moved` mark is already in the private log are skipped. `keep=1` copies and compares but keeps the space, for a dry run on the live data.

Because a device's private rows reach the others in order, the copies land before the forget everywhere. Until the forget lands on a device, `wind#N` is ambiguous there between the space and the board ("more than one board here is tagged wind"). That lasts one sync.

## 3. Row rewrite

| Space row | Private row | Change |
|---|---|---|
| `task.created` | `task.created` | `board` = the new board's id; `num`, `taskId`, `at`, `parent`, `ask` kept |
| `board.ask.edited` / `.requested` / `.state` / `.moved` | `task.edited` / `take.requested` / `take.state` / `ask.moved` | kind only (`take_kind`, asks.rs:147, plus `ask.moved`) |
| other `task.*`, `blocker.*`, `review.*` | same kind | keys mapped (below); `hop` dropped |
| `pr.state` | `pr.state` | only the newest per (card, PR) |
| `task.offered`, `task.owner`, `attach.offered`, `agent.card`, `member.*`, `board.*` | not copied | hand-out state, space identity and the space's file offers (step 2 offers files again) |

**Keys.** A person's key (`person:<me>`), wherever it is a whole string in the payload, becomes `user:<this device>`, and a person's row is written with actor `user`, so it folds as the person's in every check (`person_row`, fold.rs:2134). A person blocker's `ref` is dropped, as private ones carry none. An agent (`agent:<cardId>`) and `irori` keep their keys: `owner::may_write` lets a key that names no device through, while a session key from another device (the PC's agents) would drop that agent's blocker comments and edits (`writer_ok`, fold.rs:704).

**Agents' names.** The window names a gone agent by the last name it went by (`agent-names.json`). The move notes each space agent's name under its `agent:<cardId>` key, and every device also maps an `agent:<cardId>` key it has no name for through the private log's `board.cardof` rows to the session's name (`card_key_names`).

## 4. Open cards and notes

- Personal has hand-out and spawning on, and a device hands out only the open cards it made (`hands_out`, offers.rs:546). A card made by `agent:<cardId>` names no device, so nobody hands it out or starts an agent for it. The open cards today (wind#1, wind#4, lemmings#1, #29 and #43) were all made by agents, so they stay open with nobody on them, as in the space. The move lists any open card the person made, which the device that ran it would hand out.
- **On the other devices** the copied rows arrive by sync and fold live, which no Irori can change for 0.12.4. Their fold queues update notes for the cards' old watchers, which go to those agents' long-gone sessions: stored only, never shown to the person, and nothing revives or starts an agent for them (inbox/sweep.rs:107-118). On the Mac copy the live fold wrote 184 such messages; the quiet fold on the moving device writes none.
- Alerts: none of the moved cards has an open blocker, so nothing enters Needs you. The open cards Irori had already put back with "nobody is on it" (wind#1, wind#4, lemmings#29, #43) can each post that alert once more on each device, since the alert is keyed by the row and the copy's row is new.
- Each move rebuilds the private fold once; on the Mac's copy that took 6 to 10 s, during which the window and agents' tool calls wait.

## 5. What is left behind

- The space's own rows that aren't card content (members, devices, agent sessions, Irori's offers, older PR states). The three spaces have no channels, notes or waits.
- Private rows that name the old space id: `board.joined`, `board.cardof`, `task.pinned "board:<id>"`, `autonomy.set`. The forget makes them inert; `board.cardof` is still read for agent names.
- **Downgrade:** an Irori before #558 shows the moved cards on Personal's default board, numbered there. Every device of Stephen's is on v0.12.4.

## 6. Proof

`irori-dev space-to-personal --home <copy> --claude <empty dir> --space <id> ... [--allow-missing-files]` runs an offline Core (no network, `refuse_live`) on a copy of a data dir, moves each space, and prints per space, from the space's fold and then from Personal's (after the move, and again after a restart): cards, comments, steps and ticked steps, blockers and open ones, subtasks, threads, closed cards; the space log's own row counts; whether every `tag#N` finds the same card; and every other row the move wrote.

On a copy of the Mac's data (2026-11-06), all three moved with every count equal before, after and after a restart: Wind Waker 91 cards, 653 comments, 454 steps (428 ticked), 123 blockers, 77 subtasks, 4 threads; Lemmings 45, 141, 237 (174), 49, 42; Irori empty. Other rows written: 5 file offers. Seven Wind files exist only on the PC; offline they can't be fetched, so the live run fetches them over the space first.

The PC's copy can only be opened on the PC (DPAPI). In its place: the Mac's copy of each space holds every row of every device, with no gaps, at or past each device's last hello (PC: Wind 1020 vs 1019, Lemmings 1181 vs 1180; phone equal). The move runs on the Mac, so the Mac's copy is the one it reads.

Tests: `crates/core/tests/it/space_to_personal.rs` (a space with cards, comments, steps, a blocker, a subtask moves with the same refs and counts, and again after a restart; a second run writes nothing; the tag stays the space's; a space with an inner board is refused; a second device forgets the space and gets the board and cards) and `crates/api/tests/it/boards.rs` (the POST route).
