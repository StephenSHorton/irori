# Spaces and boards: the build (card #558)

*2026-11-05. Stephen's model: a space is a group of people (many boards, devices and people; Talk and threads are shared across it). A board is a list of tasks. This page says how the build does it. It amends [spaces.md](spaces.md).*

The raw mapping (every string, every file:line) is in `spaces-boards-map/` while the card is open. That folder goes before landing.

## 1. Settled

- **Personal is a space with its own boards** (b1): private, no invites. It stays the private log. `space.board.*` rows go in that log, which already stores them, and older devices ignore them without stalling. Its default board is implicit, so every private card stays on it. Own id "" dies at `spaces.rs:57`, so Personal needs a sentinel id in the fold.
- **Each board has its own number run from 1** (b2). This replaces spaces.md §3's "one run per space". See §2.
- **No-board rule** (agreed with #538): every space that shows has at least one board; a hidden DM space has none and never gets any; deleting a space's last board would be refused once deletes exist. Checks go in the Core paths (`space_boards`, adding a board, making a card) behind one `is_hidden_space(id)`, wired to #538's `is_dm_space`. Never key on the `~dm` tag or name.
- **Stored kinds stay** (`board.*`, spaceId = first board's id). No data migration.

## 2. Numbering

Numbers are claimed in `task.created.num`, so existing rows don't change.

- `next_short` becomes per board: the board's highest claimed number + 1. A board with cards carries on from its highest; a new board starts at 1.
- `short_groups` and `short_idx` key on (board, number), not number alone (`short.rs:89-163`). `refold_if_cut` clears maps by hand (`tasks.rs:974-998`), so any new map is cleared there.
- Moves and deletes (§3) claim numbers too. One walk in RowPos order derives every card's board and number; `short_hist` keeps each number a card left, so an old ref still finds it.
- Bare `#N` in a space: the card if it is unique across the space, else the ambiguous error naming each `tag#N`.
- `has_tag` is card-derived (`fold.rs:2000`), so an emptied board breaks `net#4`. It reads every tag the log has named instead.
- Devices still on an older Irori fold numbers as one run, so two boards' `#1` show as `#1a`/`#1b` there until they update. Cosmetic: the row is accepted and nothing stalls.

## 3. Moves and board deletes (card #563)

[no-server.md](no-server.md) §3 is the design, with its ten fold rules. There are no new kinds and no protocol bump. It replaces this page's earlier plan of `task.board`, `space.board.deleted` and `SPACES_PROTOCOL` 2.

- **A move** is `board.ask.edited {taskId, board, num, by, at}`, or `task.edited` in Personal. `Coord::move_task` writes it: one row per card, parent first, and a subtask can't move alone.
- **A delete** is `space.board.renamed {boardId, name: "", tag: "", deleted: true, by, at}`. It sits behind the same gate as adding a board, in a space. Personal has no gate. No row is written per card.
- **The fold is one walk.** `renumber` goes through the claims and the deletes in RowPos order. A card sits where its newest claim put it. A delete hands each card on the board the next number on the first board, at the delete's own place, and a claim naming an already deleted board does the same. So a number depends only on what sits before it, and every device gets the same answer whatever order rows arrive in.
- **Claims that lost drop out of letters.** Only a card's newest claim takes part in a clash. Its older ones sit in `short_hist`, where they still find the card once nobody holds that number.
- **Core folds the board list; coord reads it.** `fold_space_boards` keeps the rules a delete needs (who wrote it, and that it wins), and `coord::BoardList` is what coord takes from it, read again when a `space.board.*` or `board.member.*` row lands. Who may delete is read at the row's place, with `Membership::active_at`.
- **Personal's default board** is `""` in rows, like a space's first board. `BoardList.own` lets a row name it by id too.

## 4. Names

- Space name: `board.created` + `board.renamed`, as now (owner only).
- First board's name: `space.board.renamed {boardId: spaceId, name}`; falls back to the space's name until such a row exists. Fold name only on the own id, never the tag. `boards[0].name` is empty on a joined, not-ready space, so `boardsOf` falls back per field.
- Agreed with Stephen (b4): any active member renames a board, matching add, and a first board keeps the space's name until someone renames it. A name-only rename skips tag validation and the holder check.
- `tools_board` feeds both `boards()` and `board_of()` (`bridge.rs:546, :749`): change both.

## 5. Bugs found on the way (fix in this card)

Window: inner tag and board never reach `CardSummary` (`cards.rs:2113, :2201`); no way to make a card on an inner board (`new_card.rs:131`); `summary_row` misses `space.board.*` (`boards.rs:431`); subtasks fall on the first board (`subtasks.rs:393`); `create_board` misses inner tags (`boards.rs:991`); `SpaceBoardRow` fields need `serde(default)`.

Talk: a thread from a space channel lands in the private log (`window/src/talk.rs:231`); agent Talk tools read only the private coord (`tools/src/talk.rs:508, :583`); All spaces hides space threads (`spaces.ts:91`).

`SINCE_GO` is a sized array (`store/tests/golden.rs:77`): bump it with the new kinds.
