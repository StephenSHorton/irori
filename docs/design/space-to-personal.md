# Moving a one-person space into Personal (card #640)

*2026-11-06. Stephen's Wind Waker, Lemmings and Irori spaces were meant to be private boards. They became spaces because they were made as shared boards before Personal could hold boards ([spaces-boards.md](spaces-boards.md)). This page says how a space with only its owner in it becomes a Personal board. Line numbers are from main at 3a5e9d90.*

## 1. Settled

- **No new row kinds and no new fields** (decision 48). The move writes rows the private log already folds: `space.board.created`, the card kinds, and `board.forgot`. Every Irori from #558 on (v0.12.4) reads them, so only the device that runs the move needs the new code. The phone and the PC just receive rows.
- **Cards keep their task id, number and times.** wind#N and lemmings#N find the same card, and every `task:<id>` in old messages, parents and blockers still resolves. `Services::route` (boards.rs:1717) asks the private log first, so the copy answers as soon as it is written.
- **Who can move:** a space where nobody but its owner (this person) is an active member. That is the test `leave_board` already uses to forget everywhere (boards_members.rs:207-212). Horton has another person, so it is refused. A hidden DM space is refused too.
- **The new board gets a new id** (`new_board_id()`), never the space's id, so nothing keyed by board id (switcher pins, `board:<id>`) mixes the gone space with the new board.

## 2. The steps, in one Core call

`Services::move_space_to_personal(space_id)`, run on one device:

1. **Check.** One-person space, open on this device, not hidden. Its tag (and any inner board's tag) is held by nobody but this space. Every inner board becomes its own Personal board; these spaces have none.
2. **Board.** Append `space.board.created {boardId: new, name, tag}` to the private log. The tag check skips the space being moved, and only it: the space holds its own tag until it is forgotten.
3. **Copy.** Read the space log in fold order (`Store::list`, which orders by hlc, device, seq) and append each card row to the private log in that order (§3). Rows already copied (task id or comment id already in the private log) are skipped, so a rerun after a failure is safe.
4. **Fetch files.** Any file a copied comment or blocker names that this device lacks is fetched over the space while it is still open, and every referenced file this device holds is offered in the private log (`attach.offered`), so the other devices can fetch it after the space is gone.
5. **Compare.** Fold the private log again and compare, per card: number, title, state, comment count, steps (text and ticks), blockers, subtasks and parent. Any difference stops the move here, with the space untouched.
6. **Forget everywhere.** `forget_everywhere(space_id)` writes `board.forgot`. Every device of the person forgets the space and deletes its files when that row arrives (`reconcile_boards`, boards_own.rs:450).

Because a device's private rows reach the others in order, the copies land before the forget everywhere. A card that already waits on Stephen keeps its id, so the alert watcher (notify/watch.rs, keyed by card id) doesn't post it again.

Until the forget lands on a device, `wind#N` is ambiguous there between the space and the board ("more than one board here is tagged wind"). That lasts one sync.

## 3. Row rewrite

| Space row | Private row | Change |
|---|---|---|
| `task.created` | `task.created` | `board` = the new board's id; `num`, `taskId`, `at`, `parent`, `ask` kept |
| `board.ask.edited` / `.requested` / `.state` / `.moved` | `task.edited` / `take.requested` / `take.state` / `ask.moved` | kind only (`take_kind`, asks.rs:147, plus `ask.moved`) |
| `task.comment`, `task.steps`, `task.step`, `task.status`, `blocker.*`, `task.claimed`/`assigned`/`joined`/`left`, `task.linked`, `task.parent`, `task.watched`, `review.*` | same kind | keys mapped (below); `hop` dropped |
| `pr.state` | `pr.state` | only the newest per (card, PR) |
| `task.offered`, `task.owner`, `attach.offered`, `agent.card`, `member.*`, `board.*` | not copied | hand-out state, space identity and the space's file offers (step 4 offers files again) |

**Keys.** A person's row (`person:<me>`) is written as `user:<this device>` with actor `user`, so it folds as the person's in every check (`person_row`, fold.rs:2134). An agent (`agent:<cardId>`) and `irori` keep their keys: `owner::may_write` lets a key that names no device through, while a session key from another device (the PC's agents) would drop that agent's blocker comments and edits (`writer_ok`, fold.rs:704). Agents' names come from the private log's `board.cardof` rows, which outlive the space. The same mapping applies to `who`, `to`, mentions and watchers.

**Order.** Rows are written in the space's fold order, so the copy replays the same way after a restart. A row's `at` is kept, so times on the card stay what they were.

## 4. Open cards

Personal has hand-out and spawning on (`spawn.set` on, every device), and a device hands out only the open cards it made (`hands_out`, offers.rs:546). A card made by `agent:<cardId>` names no device, so no device hands it out or starts an agent for it. The open cards today (wind#1, wind#4, lemmings#29 and #43 submitted, lemmings#1 blocked) were all made by agents, so they move as they are and stay open with nobody on them, as in the space. A person-made open card would be handed out by the device that ran the move; the move notes any such card in its result.

## 5. What is left behind

- The space's own rows that aren't card content (members, devices, agent sessions, Irori's offers). The three spaces have no channels, notes or waits.
- Private rows that name the old space id: `board.joined`, `board.cardof`, `task.pinned "board:<id>"`, `autonomy.set`. The forget makes them inert; `board.cardof` is still read for agent names.
- **Downgrade:** an Irori before #558 shows the moved cards on Personal's default board, numbered there. Every device of Stephen's is on v0.12.4.

## 6. Proof

`irori-dev space-to-personal --home <copy> --space <id>` runs an offline Core (no network, `refuse_live`) on a copy of a data dir, moves the space, and prints per board, before and after: cards, comments, steps, ticked steps, blockers, subtasks and threads, and whether every `tag#N` finds the same card. It is run on copies of the Mac's and the PC's data (the PC run on the PC, whose keys only DPAPI there unseals), and the counts go on #640.

Tests: `crates/core/tests/it/space_to_personal.rs`. A space with cards, comments, steps, blockers, a subtask, a thread and a file moves to Personal with the same refs and counts; a second run writes nothing; a space with another person is refused; the tag clash is refused for any other space; and with three devices of one person (`common::mesh::person`) the space leaves every device's switcher while the board and its cards reach each one.
