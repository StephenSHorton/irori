# Spaces and boards: the build (card #558)

*2026-11-05. Stephen's model: a space is a group of people (many boards, devices and people; Talk and threads are shared across it). A board is a list of tasks. This page says how the build does it. It amends [spaces.md](spaces.md).*

The raw mapping (every string, every file:line) is in `spaces-boards-map/` while the card is open. That folder goes before landing.

## 1. Settled

- **Personal is a space with its own boards** (b1): private, no invites. It stays the private log. `space.board.*` rows go in that log, which already stores them, and older devices ignore them without stalling. Its default board is implicit, so every private card stays on it. Own id "" dies at `spaces.rs:57`, so Personal needs a sentinel id in the fold.
- **Each board has its own number run from 1** (b2). This replaces spaces.md §3's "one run per space". See §2.
- **No-board rule** (agreed with #538): every space that shows has at least one board; a hidden DM space has none and never gets any; deleting a space's last board is refused. Checks go in the Core paths (`space_boards`, adding a board, making a card) behind one `is_hidden_space(id)`, wired to #538's `is_dm_space`. Never key on the `~dm` tag or name.
- **Stored kinds stay** (`board.*`, spaceId = first board's id). No data migration.

## 2. Numbering

Numbers are claimed in `task.created.num`, so existing rows don't change.

- `next_short` becomes per board: the board's highest claimed number + 1. A board with cards carries on from its highest; a new board starts at 1.
- `short_groups` and `short_idx` key on (board, number), not number alone (`short.rs:89-163`). `refold_if_cut` clears maps by hand (`tasks.rs:974-998`), so any new map is cleared there.
- `task.board {taskId, board, num, by, at}` moves a card and claims a number on the new board. Last writer wins by RowPos. A move row that sorts before `task.created` is kept and applied after, as `links` does (`subtasks.rs:34-60`).
- A card keeps a history of (tag, number), folded from its rows, so an old ref still finds it. `short_was` won't do: replay stamps only the final tag.
- Bare `#N` in a space: the card if it is unique across the space, else the ambiguous error naming each `tag#N`.
- `has_tag` is card-derived (`fold.rs:2000`), so an emptied board breaks `net#4`. It reads every tag the log has named instead.
- Devices still on an older Irori fold numbers as one run, so two boards' `#1` show as `#1a`/`#1b` there until they update. Cosmetic: the row is accepted and nothing stalls.

## 3. The version gate

`task.board` and `space.board.deleted` are new kinds, so a `spaces: 1` peer stalls on them. `SPACES_PROTOCOL` goes to 2, and `not_ready_for_spaces(min)` takes the level: Talk, add, rename and notes stay at 1; move and delete use 2. Keep "waiting on" in the refusal (pinned by `spaces.rs:553`, `five_cores.rs:1335`). The private log needs no gate.

Open (b3): `heard` is memory only, so one sleeping device after a restart refuses a move.

## 4. Names

- Space name: `board.created` + `board.renamed`, as now (owner only).
- First board's name: `space.board.renamed {boardId: spaceId, name}`; falls back to the space's name until such a row exists. Fold name only on the own id, never the tag. `boards[0].name` is empty on a joined, not-ready space, so `boardsOf` falls back per field.
- Default until Stephen says otherwise: any active member renames or deletes a board, matching add. A name-only rename skips tag validation and the holder check.
- `tools_board` feeds both `boards()` and `board_of()` (`bridge.rs:546, :749`): change both.

## 5. Bugs found on the way (fix in this card)

Window: inner tag and board never reach `CardSummary` (`cards.rs:2113, :2201`); no way to make a card on an inner board (`new_card.rs:131`); `summary_row` misses `space.board.*` (`boards.rs:431`); subtasks fall on the first board (`subtasks.rs:393`); `create_board` misses inner tags (`boards.rs:991`); `SpaceBoardRow` fields need `serde(default)`.

Talk: a thread from a space channel lands in the private log (`window/src/talk.rs:231`); agent Talk tools read only the private coord (`tools/src/talk.rs:508, :583`); All spaces hides space threads (`spaces.ts:91`).

`SINCE_GO` is a sized array (`store/tests/golden.rs:77`): bump it with the new kinds.
