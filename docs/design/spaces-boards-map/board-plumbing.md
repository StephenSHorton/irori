REPORT: card #558, board lists plumbing. Read only, nothing changed. Every finding comes from reading the source; I ran no code or tests.

**0. BUGS ALREADY IN THE TRACE (fix these first)**

**a. Inner-board tags never reach the window.**
- `card_summary` (crates/window/src/cards.rs:1461) never sets `tag`.
- The only three writers set the SPACE's tag: cards.rs:2113 (`s.tag = b.tag`), cards.rs:2201 (`cs.tag = s.tag`), window/src/boards.rs:415.
- `Task.tag` is correct in coord (fold.rs:1143), but the window drops it.
- Results in the real app:
  - The chip for an inner board matches nothing and the first board's chip matches everything (board-page.tsx:433-436).
  - `net#2` displays as `wind#2`, and a typed `wind#2` is refused (short.rs:215-216, crates/core/tests/boards.rs:468).
- space-boards.test.ts misses this because it builds CardSummary with hand-set tags.

**b. The window has no inner-board id.**
- `CardSummary.board` is the space id (cards.rs:2112, 2200). `CardBoard` has only the space's id, name and tag (cards.rs:809-818).
- `Task.board` is never passed through.

**c. The window cannot create a card on an inner board.**
- `NewCardInput.board` is the space id (new_card.rs:23-25).
- `create_on_board` leaves `NewTask.board` at its default (new_card.rs:131-136).

**d. A peer's new board does not refresh the list live.**
- `summary_row` (crates/core/src/service/boards.rs:431-433) needs `is_board_identity_kind`, which requires the prefix `board.` or `member.` (crates/store/src/kinds.rs:376-378). `space.board.*` fails that.
- The window hears nothing until some other trigger fires.

**e. Subtasks fall onto the first board.**
- `create_task` takes the board only from `nt.board` (tasks.rs:141-147). `clean_new` sets it empty (tasks.rs:133).
- The split path (subtasks.rs:393) never sets it. Tools tell agents a subtask goes on its parent's board (crates/tools/src/shared.rs:404-405), but it lands on the space's own.

**f. `create_board` misses inner tags.**
- It checks `tag_holder` only (boards.rs:991), not `board_tag_holder`. A new space can take a tag an inner board already holds.
- The test at core/tests/boards.rs:487-489 covers only `create_space_board`.

**g. `SpaceBoardRow` has required fields.**
- `name` and `tag` have no `serde(default)` (spaces.rs:38-39).
- A row without them fails decode and is silently skipped (spaces.rs:54-56). A `{boardId, by, at}` delete row would be lost.

**h. Coord never sees `space.board.*`.**
- The kind is not in FOLLOW_KINDS (fold.rs:91-108), so it is absent from both replay and the live feed.
- Rows from this device are skipped at mod.rs:1503 anyway.
- A tag is stamped once at fold (fold.rs:1143). The only live update path is a restart or refold, as spaces.md:40 already admits.

**TRACE of `board`**
1. Tool or NewTask `board=net`: shared.rs:417 sends `space_board`. tasks.rs:141-147 validates it through `board_tag`.
2. Row: rows.rs:53-55. Written at tasks.rs:216.
3. Fold: `tag_of` at fold.rs:1143-1144, 1992-1997. `tag_of` falls back to the space tag for an unknown board.
4. `board_tag` hook: boards.rs:1425-1431 calls `board_tag_in` (spaces.rs:108-119), which re-lists and refolds up to 10,000 rows per card (spaces.rs:98).
5. Task fields: types.rs:382, 385.
6. `short_ref`: types.rs:530-535.
7. Window: lost at the 0a lines.
8. Summary: `BoardSummary.boards` from `space_boards` (boards_own.rs:521), to window/boards.rs:150-158, to bindings.ts:702.
9. Chips: board-page.tsx:442-536, using `cardsOnBoard`.
10. Add: board-page.tsx:460, lib/boards.ts:261, host.ts:358, bindings.ts:162, commands.rs:751, window/boards.rs:302, spaces.rs:179.

**1. MOVING A CARD BETWEEN BOARDS**

**Nothing can do it today.** No row changes `board` after `task.created`. `task.edited` is refused on board logs (kinds.rs:133).

**Smallest change: a new kind `task.board {taskId, board, by, at}`,** where `board` is empty for the first board, shaped like `TaskParentRow` (rows.rs:448-455).
- The prefix `task.` is followed automatically (fold.rs:92, 111).
- `resolves` already checks `taskId` and `by` (store/board.rs:368, 415-450).
- `aside_gate` holds it until the card exists (fold.rs:2128).

**Rejected alternative: a `board` field on an accepted kind.** It needs no gate, but nothing fits. On `task.placed`, older folds would overwrite x/y (placed.rs:59). On `task.parent`, the meaning is wrong and relink would act on it.

**Fold arm, beside fold.rs:964:**
- On a board only, set `t.board` and `t.tag = tag_of(board)`, then `touch` ("moved it to net").
- Do NOT validate the board in the fold, or the row is lost when it arrives before `space.board.created` (spaces.md:40). Validate in the writer. `tag_of` already falls back safely.
- Last writer wins by RowPos with a new `board_at: HashMap<id, RowPos>`. Live arrival order differs by device; replay uses `(hlc, device, seq)` (db.rs:1608). `self.links` plus the relink sort is the model (subtasks.rs:39-44, 60).
- `short_moved.insert` so search re-reads the card (short.rs:168, mod.rs:1637). Do not reuse the "renumbered" text at short.rs:180.

**Writer:** `Coord::move_card`, on a board only.
- `task_ref_locked`, then `board_tag(board).is_some()` as tasks.rs:143 does. Empty means the first board.
- Refuse threads and asks.
- Write one row per subtask, so a family stays together.
- `NewTask.board` refuses a space id (tasks.rs:143), so the window maps first board to "".

**Ref display: the number stays, the tag changes (`wind#4` becomes `net#4`).** Old refs then break in five places:
- short.rs:206 (`has_tag`) and short.rs:215
- fold.rs:1488 (mentions)
- threads.rs:93
- `names_card` via coord/board/cards.rs:2100-2109, because a `card-wind4` branch stops matching and `other_card_place` can misfire

**Recommended rule:** `tag#n` in a space means the space's #n, whichever board holds it now. That is safe only with one numbering run.
- Drop the per-card compare at short.rs:215. Change core/tests/boards.rs:468 to match.
- Let `has_tag` (fold.rs:2000) accept any tag the space's log ever named, through a new Options hook, because a replay stamps only the final tag and a `tag_was` field would be lost.

This gives the answer to the card's question. Keep one run per space, because a per-board run would make a move renumber and collide.

**2. RENAME WRITER, DELETE KIND, THE GATE**

**Would a new kind stall older peers? Yes.**
- A peer with `spaces: 1` knows only created and renamed.
- `check_shape` refuses `space.board.deleted` and `task.board` (store/board.rs:256-258, reached from db.rs:1303). That writer's stream then waits on the gap (spaces.md:44).
- So `spaces >= 1` is not enough and the number must move.

**Do not simply bump the constant.**
- `not_ready_for_spaces` compares to `SPACES_PROTOCOL` (boardmesh.rs:590).
- A bare bump would also block every row already gated at 1 on any `spaces: 1` device, until it updates:
  - Talk (spaces.rs:372)
  - board adds (spaces.rs:218)
  - notes (notes.rs:40)

**Recommended:** `SPACES_PROTOCOL = 2`, and `not_ready_for_spaces(min)` with `space_not_ready(id, min)`.
- Talk, notes, add and rename stay at `min` 1. Delete and move use 2.
- There is no wire change, since `spaces` is already an i64 and a v1 reader only tests `< 1`.
- The alternative is a separate hello field as `spawns` does (boardmesh.rs:84, 120-123, 361-363, 1272, 1390). That is more code for nothing extra.
- `move_card` lives in coord and has no gate access, so the window or Core must check before calling, as Talk does.
- Fix the refusal text (spaces.rs:271), which says "with spaces".
- Known exposure, unchanged from today: the gate only looks at devices at write time. An older device that joins later still stalls on rows already written.

**`space.board.deleted {boardId, by, at}`:**
- Add it at kinds.rs:288 as Replicated, Accepts.
- Add it to `SINCE_GO` at store/tests/golden.rs:102-103, along with `TASK_BOARD`.
- Add it to `space_board_rows` (spaces.rs:94-97).
- Fold (spaces.rs:50-88): collect deleted ids first as tombstones so order cannot resurrect a board. Ignore a delete of the own board.
- `resolves` needs no change.

**Cards on a deleted board land on the first board, with no card rows and the same number.**
- `board_tag` returns None, so `tag_of` gives the space's tag (fold.rs:1996). A concurrent move or create onto the dead board lands there too.
- Keep `t.board` raw (needed for the spaces.md:40 edge) and let the window treat an id not in `boards` as the first board.
- Live folds need a retag, which is one call rather than a FOLLOW_KINDS change:
  - Add `Coord::retag_boards()`: for each `t.board` non-empty, set `t.tag = tag_of(t.board)`.
  - Call it from `follow_board_row` (boards.rs:1572) on `space.board.*`. That subscription sees this device's rows too (boards.rs:1520-1532).
  - It also fixes the spaces.md:40 edge without a restart.
- Alternatives I rejected: refusing delete while cards remain, or writing `task.board` per card before the delete. The second is many rows, still misses concurrent cards, and is not atomic.
- Refuse deleting the first board in both writer and fold. It is the default board and the landing place.
- UI: confirm with "N cards move to <first board>".

**Rename writer: `rename_space_board(space, board, name)`.**
- Copy spaces.rs:185-220 for validation, active member and the gate at `min` 1.
- Append `SPACE_BOARD_RENAMED` with `tag: ""`.
- Name only. The fold keeps the tag on empty (spaces.rs:77-79), so no ref changes, no retag and no holder check are needed. If a tag rename is ever wanted, exclude the board itself from `board_tag_holder`.

**Who may rename or delete:** any active member, matching add (spaces.rs:203, spaces.md:34). Delete loses no cards, and the fold has no author check for these rows, so owner-only would be writer-side only. Confirm with Stephen.

**3. SEPARATING THE FIRST BOARD'S NAME**

**The tie today:**
- spaces.rs:126-130 uses `m.name` and `m.tag`. boards_own.rs:513-514 uses the same.
- `rename_board` (boards.rs:1093, 1119-1123) writes `board.renamed`, which sets `membership.name` (membership.rs:236-243, 560-569).
- `space.board.renamed` for the own id is dropped at spaces.rs:57.

**Which row carries which name:**
- SPACE name: `board.created` plus `board.renamed`, unchanged. Signed, owner-only. Every other reader already means the space:
  - joined rows and name: boards.rs:1037, invites.rs:524, boards_own.rs:575
  - share preview: boards_own.rs:825, 959, 965
  - board info: boards.rs:1135
  - sidebar and switcher: spaces.ts:35
- FIRST BOARD name: `space.board.renamed {boardId: spaceId, name}`. No new kind, no migration.
  - Until such a row exists, it falls back to `m.name`, so existing spaces look the same.
  - Peers on the shipped fold drop it at spaces.rs:57 and just show the old name. No stall.
- First board TAG: stays `membership.tag`. It is tied to refs, `tag_fn` (boards.rs:1332), `joined` rows and bridge.rs:227.

**Fold change at spaces.rs:57:** for RENAMED on the own id, set `own.name`, then continue. Make `own` mutable or fold into it. `board_tag_in` passes an empty own id, so such a row falls through and is ignored (the `get_mut` at spaces.rs:73 finds nothing). No change needed there.

**Limit to raise with Stephen:** an unnamed first board still follows a space rename. That direction is not what the card asked for, but it is real. Optional fix: in `rename_board`, first pin the old name with a `space.board.renamed` row, best effort and only if the gate is open. Do not write the row at space creation, since the creator's other devices may be unheard.

**Agents:** bridge.rs:733 gives the own entry the space name; it should take `inside[0].name`.

**For #538:** spaces.rs:116, 247, boards_own.rs:677 and bridge.rs:735 all `skip(1)`, assuming own is first. Keep own always present in the fold and hide it in display; do not make a space with no boards in the fold.

**4. TAG RULES**

**Format:** `valid_board_tag` (membership.rs:742), and `valid_tag` in coord (short.rs:66): a lowercase letter, then up to 15 of a-z, 0-9, '-'.

**Checkers, both local to this device and at write time only:**
- `tag_holder` (boards.rs:1057-1089): own tags of shown spaces, plus `board.joined` not yet forgotten.
- `board_tag_holder` (spaces.rs:240-253): the above plus inner boards of shown spaces only.
- Join does not check. The only callers are boards.rs:991 and spaces.rs:241/209.

**Clashes:**
- `tag_clash` counts space tags only (boards_own.rs:560-568). `SpaceBoard` has no clash flag.
- Two members adding `net` while apart gives duplicates in one space; the fold does not dedupe.
- That is harmless for refs, since one number run means `net#5` is still one card. It does conflate a tag-based filter, so filter by board id.
- Across spaces, a shared tag makes `boards_by_tag` return two logs, and refs then behave like a bare number (boards_own.rs:670-680, 696-709).

**After a delete:**
- The tag frees up. Old `net#5` refs then fail: `boards_by_tag` gives 0 hits, "no board here is tagged" (boards_own.rs:702-705).
- If another space takes `net`, an old ref points into the wrong space.
- Recommendation: keep deleted tags reserved. Have `board_tag_holder` and `boards_by_tag` read a retired list from the fold, the same "tags ever named" source as §1.

**5. CODE CHANGES, IN ORDER**

Fixes:
1. window/cards.rs:2113, 2201, window/boards.rs:415: use `t.tag`, falling back to the space's tag when empty.
2. window/cards.rs:205-228: add `space_board` (resolved id; the space id for the first board or a deleted one). Set it at 2112, 2200 and in `board_detail` (boards.rs:414). Update the CardSummary shape check (window/tests/services.rs:617) and regenerate bindings.ts:991-996 and models.ts.
3. frontend board-page.tsx:433-436: filter on `spaceBoard`. Update space-boards.test.ts. board-view.tsx:71 must stop reading `c.tag` as the space's tag; `homeBoards` needs a lookup.
4. boards.rs:431-433 `summary_row`: add `space.board.*`.
5. boards.rs:991: use `board_tag_holder`.
6. spaces.rs:38-39: add `serde(default)`.
7. tasks.rs:148-152 and subtasks.rs:393: inherit the parent's board.

Store and mesh:
8. kinds.rs:288: `SPACE_BOARD_DELETED`. Near kinds.rs:137: `TASK_BOARD`. Add both to golden.rs:102-103.
9. boardmesh.rs:77, 582-593: `SPACES_PROTOCOL` 2 and `min`. Update the comment at boardmesh.rs:71-76.
10. spaces.rs:137, 144, 218, 372, 486, notes.rs:40: pass `min`. spaces.rs:264-271: reword. Tests at five_cores.rs:1147, 1326, 1361, 1606 and core/tests/boards.rs:432, 520, 542. spaces.rs:550-551 pins the `gate_refusal` wording.

Coord:
11. rows.rs (by line 449): `TaskBoardRow`. fold.rs:964: new arm. New `move_card` plus a public wrapper in mod.rs. New `retag_boards`.
12. short.rs:206, 215, fold.rs:1488, 2000, threads.rs:93, coord/board/cards.rs:2100: the rule from §1. coord/board/mod.rs:271-274: add the hook. boards.rs:1425: wire it.

Core:
13. spaces.rs:50-104: fold delete, tombstones, own name, retired tags. spaces.rs:176-236: add `rename_space_board` and `delete_space_board`. spaces.rs:492: public wrappers.
14. boards.rs:1572 `follow_board_row`: call `retag_boards` on `space.board.*`.
15. boards_own.rs:670-680: use retired tags. bridge.rs:733: first board's own name.

Window and frontend:
16. window/boards.rs:302-318: add `boards_rename_in_space`, `boards_delete_from_space`, and `board_move_card`. Keep `boards_rename` as the space rename, since an inner id fails `self.board()` there (boards.rs:1104).
17. commands.rs:749-764 and the list at 1621-1623. Regenerate bindings.ts:160-162.
18. host.ts:358, lib/boards.ts:261-279 (plus sample versions).
19. new_card.rs:25, 131-136: add a `space_board` input. new-card-dialog.tsx:60, 120, 147-148.
20. board-page.tsx:186-187, 204: "Rename this space". board-page.tsx:442-536: boards as lists, with rename, delete and a move control. The card page needs the move control too.

Docs and tests:
21. spaces.md:33, 35, 38, 40, 46-47, 66, 79. work-personal-separation.md:107, 145.
22. New tests:
   - a card moves and both devices converge when moves cross
   - an old ref still resolves after a move
   - delete puts cards on the first board, live and after a refold
   - a first board rename leaves `membership.name` alone
   - a `spaces: 1` peer is not sent the new kinds and Talk still works
   - extend `hello_says_it_reads_spaces` (boardmesh.rs:1571-1589)
   - core/tests/boards.rs:434-474 and five_cores.rs:1153

**OPEN FOR STEPHEN**
1. Any member, or owners only, for board rename and delete? I recommend any member.
2. Should renaming a space leave an unnamed first board alone? This is the optional pin in §3.
3. Personal: `board_tag` is off for private logs (fold.rs:1985), so boards in Personal would need more than this list covers.
4. Rollout: delete and move stay refused until every device in the space runs the new build and has connected since the last start.