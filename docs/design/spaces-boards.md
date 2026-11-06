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
- Moves and deletes (§3) claim numbers too. One walk in RowPos order derives every card's board and number; `short_hist` keeps each number a card left, so an old ref still finds it.
- Bare `#N` in a space: the card if it is unique across the space, else the ambiguous error naming each `tag#N`.
- `has_tag` is card-derived (`fold.rs:2000`), so an emptied board breaks `net#4`. It reads every tag the log has named instead.
- Devices still on an older Irori fold numbers as one run, so two boards' `#1` show as `#1a`/`#1b` there until they update. The row is accepted and nothing stalls, and those letters still resolve on a newer device: `one_run_claim` (`short.rs`) reads them from the claims, as it does a `#5a` from before this card. An exact ref on a board comes first.
- A card's tag comes from its board's row, which can land after the card. `space.board.*` is folded for that alone: it stamps the board's cards again, so devices agree with no restart.
- Personal's rule: a bare `#N` means the default board first, deleted cards included, so a stale `#3` is refused rather than passed to `home#3`. Comment mentions and thread hearers read refs by the same rule (`short_hits`).

## 3. Moves and board deletes (card #563)

Stephen first held these for a server design (b3). Decision 48 dropped the server. [no-server.md](no-server.md) §3 is the design, with its ten fold rules. There are no new kinds and no protocol bump. It replaces this page's earlier plan of `task.board`, `space.board.deleted` and `SPACES_PROTOCOL` 2.

- **A move** is `board.ask.edited {taskId, board, num, by, at}`, or `task.edited` in Personal. `Coord::move_task` writes it: one row per card, parent first, and a subtask can't move alone.
- **A delete** is `space.board.renamed {boardId, name: "", tag: "", deleted: true, by, at}`. It sits behind the same gate as adding a board, in a space. Personal has no gate. No row is written per card.
- **The fold is one walk.** `renumber` goes through the claims and the deletes in RowPos order. A card sits where its newest claim put it. A delete hands each card on the board the next number on the first board, at the delete's own place, and a claim naming an already deleted board does the same. So a number depends only on what sits before it, and every device gets the same answer whatever order rows arrive in.
- **Claims that lost drop out of letters.** Only a card's newest claim takes part in a clash. Its older ones sit in `short_hist`, where they still find the card once nobody holds that number. A lettered old ref means one claimant, by RowPos among everyone who held the number, so `net#5b` never finds a card that wasn't `net#5b`.
- **A family stays together.** A move writes rows only for family members not on the target board, so moving a parent to where it already is brings a split family back. Adopting a card into a parent on another board moves it there.
- **A catch-up walks per row, as live does.** #567's paged catch-up must leave what live folding leaves, notices included, and a notice written mid-page names a card by the number the walk gave it. A walk is only needed when a claim can change an earlier answer (a move, a delete, or a claim behind the last walk); plain creates settle their own group.
- **Known limit:** the board list reads the verifier without `verify_mu`, so during a verifier reset (a store error path) a delete can briefly not count. A failed store read keeps the old list, but this case heals only at the next board or member row, resync or restart.
- **Core folds the board list; coord reads it.** `fold_space_boards` keeps the rules a delete needs (who wrote it, and that it wins), and `coord::BoardList` is what coord takes from it, read again when a `space.board.*` or `board.member.*` row lands. Who may delete is read at the row's place, with `Membership::active_at`.
- **Personal's default board** is `""` in rows, like a space's first board. `BoardList.own` lets a row name it by id too.

## 4. Names

- Space name: `board.created` + `board.renamed`, as now (owner only).
- First board's name: `space.board.renamed {boardId: spaceId, name}`; falls back to the space's name until such a row exists. Fold name only on the own id, never the tag. `boards[0].name` is empty on a joined, not-ready space, so `boardsOf` falls back per field.
- Agreed with Stephen (b4): any active member renames a board, matching add, and a first board keeps the space's name until someone renames it. A name-only rename skips tag validation and the holder check.
- `tools_board` feeds both `boards()` and `board_of()` (`bridge.rs:546, :749`): change both.

## 5. Bugs found on the way, all fixed on this card

- Window: an inner board's tag and board reach `CardSummary` (`card_place`), New card can go on any board, subtasks stay on their parent's board, and `summary_row` follows `space.board.*`.
- Talk: a thread from a space channel lands in the space's log, agent Talk tools reach space threads, All spaces lists them, and `talk_post` on a space's card goes through the shared-space fence like `board_comment`.
- Agents: `board_share` and `board_create` onto an inner board land on that board, and labels name a card's own board.
