# Spaces: the build (card #392)

*2026-11-03. Design: [talk-channels.md §5](talk-channels.md) and decision 46 (Stephen's b4 on #382). This page says how the build does it.*

A space is a group of people that holds many boards and channels. v0.5 already has one: a shared board, with its own log, secret, topic, connections and members ([v0.5-shared-boards.md](v0.5-shared-boards.md) §2.1, §6). So a space is that board, and its other boards are rows in its log.

## 1. Identity: the space is the board

- **spaceId = the board's boardId.** Every key that names a board carries over unchanged:
  - the board secret, epoch, topic and ALPN `irori/board/1`, and the board-open proof;
  - the `board/<id>` pull stream and the invite bundle;
  - the bridge's `board:<id>/…` sender keys;
  - `board.joined`, `board.forgot`, `board.pin` and `board.cardof`;
  - `receive.set`, `autonomy.set` and `threads.set`.
- **No row is rewritten.** Board and member signatures bind no board id (only `board.created` names its own). So an existing board is already a one-board space, and the migration moves files, not data.
- **The kind names stay** (`board.*`, `member.*`). Renaming them would orphan every existing row, and every Irori already in use.

## 2. Files

`<data>/boards/<id>.db` (with `-wal`, `-shm` and `.secrets`) moves to `<data>/spaces/<id>.db`. The move:

- runs at the start, before anything can open, create or join a space (`migrate_board_files`);
- moves the `.db` last, so a start that still finds `boards/<id>.db` finishes the move;
- never overwrites a file that is already in `spaces/`;
- removes the emptied `boards/`.

`irori-dev verify` reads both folders. Reset still leaves the space logs in place, as it did board logs.

**Downgrade:** an Irori before spaces looks only in `boards/`, so it won't find a moved space. Going back means moving the files back by hand.

## 3. Boards inside a space

- `space.board.created {boardId, name, tag, by, at}` adds a board, and `space.board.renamed` renames one. The space's own board is implicit: its id is the spaceId, and its name and tag come from `board.created` and `board.renamed`.
- Any active member may add a board, as any member may add a card.
- `task.created` gains an optional `board` (a `space.board.created` id; empty means the space's own board). A card shows its board's tag (`net#5`).
- **One numbering run per space.** The space's cards share one count, so `wind#4` and `net#5` never clash and the short-ref fold is unchanged. (The design said "per board"; this is simpler and loses nothing.)
- Agents see each board in a space by its tag. `board_create board=net` puts the card on `net`, in the space's log.
- The window lists a space's boards in `BoardSummary.boards`, own board first. The board page shows them as chips that filter its cards, and + Board adds one (`boards_add_to_space`). The space switcher (#393) builds on that list.

**Edge, fixed by a restart:** if another member's card on a new board ever arrives before that board's `space.board.created` (two writers, different streams), it folds with the space's tag until the next refold. In practice the creating row arrives first: pulls go oldest first, and a writer on the new board already holds its row.

## 4. The version gate

An Irori before spaces refuses a kind it doesn't know (`check_shape`) and doesn't store it. Every later row of that writer's stream then fails the gap check, so **one new row stalls that whole device's stream on an older peer.**

- The board hello gains `spaces` (`SPACES_PROTOCOL = 1`). `v` stays 1, so older and newer Irori keep syncing everything else.
- A space writes no space-only row (`space.board.*`, and channels) until every member device (every other device of every active member) has said `spaces >= 1` in a hello since this start (`BoardMesh::not_ready_for_spaces`). The refusal names the devices it waits on.
- **What this costs:** every certified device counts. With three devices on a space, adding a board needs all three updated *and* connected since the last start. A device that is gone for good blocks it until it's revoked.

## 5. Settings and membership

These need no data migration, because their keys are the same id:

- **Membership, owners, invites and removal** are the existing `board.member.*` rows, read as the space's.
- **`receive.set`, `autonomy.set` and `threads.set`** are per space.

The per-board override is dropped (decision 46, which amends #24).

## 6. Channels

Talk 1 (#389) adds `talk.channel.created {…, spaceId}`, where an empty `spaceId` means Personal (the private log). A channel in a space goes in the space's log through the same gate (s5):

- **Where rows go.** `talk.channel.created`, and the channel's `talk.member` and `talk.post` rows, go in the log its `spaceId` names. DMs and read cursors (`talk.read`) stay in the private log; board logs refuse `talk.read`.
- **One fold.** Talk folds the private log and every space's log into one view (`coord::talk::SpaceLogs`, which the Core implements). A start, or a resync of either log, folds both again.
- **Keys.** A space's rows name people and agents as the space does (`person:<id>`, `agent:<cardId>`), so the other member can tell them apart. Talk on this device reads its own person back as `user:<deviceId>` and its own agents by their session keys. Another person shows as a person, named by their member profile.
- **The gate.** The same `spaces >= 1` as `space.board.*`, checked on every space talk row (create, join, leave and post), with the same refusal naming the devices. No new number: the hello's `spaces` came after Talk 1 marked `talk.*` as accepted on board logs, so any Irori that says it reads spaces stores them. Personal channels and DMs are never gated.
- **The window.** `talk_channel_create(name, topic?, spaceId?)`; no `spaceId` is Personal.

## 7. Checks

- **Tests:**
  - `board_logs_move_to_spaces`: a v0.10 layout with a move cut short reopens with the same cards and vector.
  - `a_second_board_in_a_space_reaches_the_other_device`: the add is refused before the other device's hello, then the board and a card on it reach it.
  - `a_space_waits_for_devices_that_dont_read_spaces`: tools.
  - `hello_says_it_reads_spaces`: wire compatibility.
  - `a_second_board_and_a_channel_in_a_space_reach_bob` (five_cores): two people; a second board and a card on it, then a channel and posts both ways through Talk's read model; cursors stay private; a device of Bob's that never said `spaces` stops new channel rows, naming it.
  - `a_space_channel_goes_in_the_space_log_behind_the_gate` (coord): where each row goes, the refusal, and a fresh fold reading both logs.
  - `the_gate_names_every_device_it_waits_on`, `talk_keys_map_into_a_space_and_back`: the gate's decision and the key mapping.
  - `cardsOnBoard`: the page's filter.
- **Real data:** the move was run on read-only copies of the Mac's and the PC's data dirs (`irori-dev migrate-spaces`), comparing event counts and per-device max seq before and after (proof on #392).
