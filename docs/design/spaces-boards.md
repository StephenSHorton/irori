# Spaces and boards: the build (card #558)

*2026-11-05. Stephen's model: a space is a group of people (many boards, devices and people; Talk and threads are shared across it). A board is a list of tasks. This page says how the build does it. It amends [spaces.md](spaces.md).*

## 1. Settled

- **Personal is a space with its own boards** (b1): private, no invites. It stays the private log. `space.board.*` rows go in that log, which already stores them, and older devices ignore them without stalling. Its default board is implicit, so every private card stays on it. Own id "" dies at `spaces.rs:57`, so Personal needs a sentinel id in the fold.
- **Each board has its own number run from 1** (b2). This replaces spaces.md §3's "one run per space". See §2.
- **No-board rule** (agreed with #538): every space that shows has at least one board; a hidden DM space has none and never gets any; deleting a space's last board would be refused once deletes exist. Checks go in the Core paths (`space_boards`, adding a board, making a card) behind one `is_hidden_space(id)`, wired to #538's `is_dm_space`. Never key on the `~dm` tag or name.
- **Stored kinds stay** (`board.*`, spaceId = first board's id). No data migration.

## 2. Numbering

Numbers are claimed in `task.created.num`, so existing rows don't change.

- `next_short` becomes per board: the board's highest claimed number + 1. A board with cards carries on from its highest; a new board starts at 1.
- `short_groups` and `short_idx` key on (board, number), not number alone (`short.rs:89-163`). `refold_if_cut` clears maps by hand (`tasks.rs:974-998`), so any new map is cleared there.
- When moves come (§3): `task.board {taskId, board, num, by, at}` claims a number on the new board, last writer wins by RowPos, and a row that sorts before `task.created` is kept and applied after, as `links` does (`subtasks.rs:34-60`). A card keeps a history of (tag, number) so an old ref still finds it; `short_was` won't do, since replay stamps only the final tag.
- Bare `#N` in a space: the card if it is unique across the space, else the ambiguous error naming each `tag#N`.
- `has_tag` is card-derived (`fold.rs:2000`), so an emptied board breaks `net#4`. It reads every tag the log has named instead.
- Devices still on an older Irori fold numbers as one run, so two boards' `#1` show as `#1a`/`#1b` there until they update. The row is accepted and nothing stalls, and those letters still resolve on a newer device: `one_run_claim` (`short.rs`) reads them from the claims, as it does a `#5a` from before this card. An exact ref on a board comes first.
- A card's tag comes from its board's row, which can land after the card. `space.board.*` is folded for that alone: it stamps the board's cards again, so devices agree with no restart.
- Personal's rule: a bare `#N` means the default board first, deleted cards included, so a stale `#3` is refused rather than passed to `home#3`. Comment mentions and thread hearers read refs by the same rule (`short_hits`).

## 3. Moves and board deletes: card #563

Stephen first held these for a server design (b3). Decision 48 dropped the server: a move is `board.ask.edited` and a delete is `space.board.renamed {deleted: true}`, rows every current device reads. [no-server.md](no-server.md) §3 is the design and #563 builds it on this card's numbering. This card adds **no new row kinds and no protocol bump**.

## 4. Names

- Space name: `board.created` + `board.renamed`, as now (owner only).
- First board's name: `space.board.renamed {boardId: spaceId, name}`; falls back to the space's name until such a row exists. Fold name only on the own id, never the tag. `boards[0].name` is empty on a joined, not-ready space, so `boardsOf` falls back per field.
- Agreed with Stephen (b4): any active member renames a board, matching add, and a first board keeps the space's name until someone renames it. A name-only rename skips tag validation and the holder check.
- `tools_board` feeds both `boards()` and `board_of()` (`bridge.rs:546, :749`): change both.

## 5. Bugs found on the way, all fixed on this card

- Window: an inner board's tag and board reach `CardSummary` (`card_place`), New card can go on any board, subtasks stay on their parent's board, and `summary_row` follows `space.board.*`.
- Talk: a thread from a space channel lands in the space's log, agent Talk tools reach space threads, All spaces lists them, and `talk_post` on a space's card goes through the shared-space fence like `board_comment`.
- Agents: `board_share` and `board_create` onto an inner board land on that board, and labels name a card's own board.
