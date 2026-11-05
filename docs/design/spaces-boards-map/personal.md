Report for #558, Personal and the private log. It comes from reading the code at 23093055. Nothing was run or changed. The advisor call was rate-limited, so this has no second review.

**ANSWERS**

1. **Does the private log accept `space.board.*`? Yes, today, with no change.**
   - The Board column and `check_shape` apply only to a board log. `check_shape` is called only inside `if let Some(b) = &self.board` (crates/store/src/db.rs:1063-1065 on append, :1300-1305 on insert).
   - `board_accepted` is called only from store/src/board.rs:256 and kinds.rs:377.
   - The private log refuses only Local kinds (db.rs:565, :1193), bad shape (:1169-1207), and row order or signature failures (:1306-1375).
   - `Chain::verify_row` checks the signature and doesn't filter on kind. Its only kind check is on root-cert rows (crates/identity/src/chain.rs:415).
   - Tests confirm this: store/tests/board.rs:141 appends every Refuses kind plus `future.kind` to a private log, and store/tests/live.rs:178-182 inserts a peer's `future.kind`. The doc comment at kinds.rs:20-21 says the same.
   - So `space.board.created` (kinds.rs:286) can be written to the private log as it stands. The comment at kinds.rs:280-284 says these rows are for a space log, so it needs a wording fix.

2. **Would older devices stall? Not on the private log.**
   - The private log replicates by the mesh `Node` to every certified device of the person, phone included (kinds.rs:154-157, work-personal-separation.md:9).
   - The sender doesn't filter by kind (crates/mesh/src/session.rs:364, `st.missing`).
   - An older receiver stores an unknown kind and its fold drops it (`_ => {}`, crates/coord/src/board/fold.rs:1102). Only the writer's rows that fail other checks stop the stream.
   - The private hello has no feature gate and needs none (crates/mesh/src/node.rs:796-816, session.rs:491-503).
   - Other people never get this log (talk-channels.md:45).
   - Space logs are the opposite. An unknown kind fails `check_shape` on an older device, and that writer's later rows then fail the gap check (spaces.md §4; boardmesh.rs:72-77).

3. **Degraded view on an older device.** A private card with `board` set folds as a plain `#12` card, because decode ignores nothing it needs and the tag falls back to "" (fold.rs:1143, :1992-1997). The Personal board's chip is simply absent there. A later move row would be ignored, so the card stays where it was. That is cosmetic and doesn't stall anything.

4. **How private cards are created.**
   - Window: `create_card` with empty `input.board` (crates/window/src/new_card.rs:33, :41-50, :60-70). It uses `..NewTask::default()`, so `board` is always empty. `NewCardInput.board` means a space id (:23-25), so there is no field for an inner board.
   - Agents: `board_create` with `board` set always goes to `shared::create_on_board` (crates/tools/src/board.rs:840-843). Otherwise it uses the private path at :860-875, with no board.
   - `find_board` walks only `SharedBoards::boards()` (crates/tools/src/shared.rs:190-229). Personal isn't in that list, so it answers "no shared board".

5. **How `task.created`'s `board` behaves in the private log.**
   - Write: refused. `create_task` calls `board_tag`, which is gated on `on_board` (crates/coord/src/board/fold.rs:1984-1989), so a non-empty board returns "no board … in this space" (tasks.rs:141-147).
   - Read: `fold_created` stores `p.board` in `Task.board` with no check, and the tag is "" (fold.rs:1143-1144). So a replicated row with `board` set folds without error.
   - The private `coord::Options` passes neither `board` nor `board_tag` nor `tag` (crates/core/src/service.rs:1071-1104).
   - Name clash: `AskField.board` in the private log already means a shared board's id, for private threads about a shared board (coord types.rs:333-334, rows.rs:22, asks.rs:258-261, spawn.rs:1441). That is a different field from `TaskCreatedRow.board`. Keep them apart.

6. **Short refs in the private log.**
   - One run per log, the same code as a space. `next_short` is `short_top+1` (crates/coord/src/board/short.rs:191-193, tasks.rs:198), and `claim_short`/`settle` key on the number alone (short.rs:89-163).
   - Private cards span several devices, so two apart can claim one number and get `12a`/`12b`. That already works.
   - The tag is "" off a board (fold.rs:1975-1980), so refs show `#12` (types.rs:530-535).
   - Personal boards under one run need no fold change. Per-board numbering would mean rekeying `short_groups` and `short_idx` and renumbering every existing card, so don't do it.

7. **Frontend treatment today.**
   - lib/spaces.ts: Personal is a sentinel "personal" (:17), not a BoardSummary. It is hardcoded in `spacesOf` (:40) and `known` (:56).
   - `cardInSpace` uses `!c.board` (:64), `convInSpace` uses `!c.spaceId` (:74), `boardsInSpace` returns [] (:85), and `threadsSpace` returns "" (:92).
   - boards-page.tsx:62-66 shows "Personal holds no shared boards" text and :35 lists nothing. The New board dialog is still mounted (:87-95).
   - Sidebar: pinned boards are empty under Personal (app-sidebar.tsx:258). The Boards item is at :420.
   - Personal's real page is Home: `homeScope = ScopeOwn` (App.tsx:169, :177, :924-926), with `inScope` at board-view.tsx:52-56.
   - `BoardPage` can't host Personal. It needs a BoardSummary, and it carries members, invites and notes (board-page.tsx:95-112).
   - Talk already treats Personal as a space. Channels use `spaceId` "" (conversations.ts:149-155, tools/src/talk.rs:667). So Talk needs no work for Personal.

**FINDINGS BEYOND THE QUESTION**

A. **Probable live bug, from reading only: inner-board filtering looks broken in the real window.**
   - `Task.tag`/`Task.board` never reach `CardSummary`. `card_summary` sets no tag (crates/window/src/cards.rs:1461ff), then every board card gets the space's tag (cards.rs:2113, :2201).
   - So `cardsOnBoard` at board-page.tsx:433-436 finds nothing for the `net` chip and everything for the space's own chip, and `net#5` displays as `wind#5`.
   - space-boards.test.ts passes only because it feeds hand-set tags.
   - This must be fixed for step s3 whatever Personal does. Add a `spaceBoard` id field to `CardSummary`, set the tag from `t.tag`, and filter by id.

B. **A Personal default board can't be picked by tag.** Its tag is "", and `cardsOnBoard` treats "" as all boards (:434). Same fix: filter by board id.

C. **`task_ref` breaks on Personal tags.** `home#12` goes only to `boards_by_tag`, which doesn't look in the private log, so it returns "no board here is tagged" (crates/core/src/service/boards_own.rs:693-709, :670-680). Agents would be handed refs they can't use.

D. **New space-log kinds can't reuse the gate by bumping `SPACES_PROTOCOL`.** A delete or move row needs a gate, but `not_ready_for_spaces` compares against that one constant (boardmesh.rs:590). Bumping it would also block `talk.*` and `space.board.created` for devices that read them fine. Add a separate hello field, as `spawns` did (boardmesh.rs:84, :120, :599-610, :1271-1272). No move or delete kind exists yet; `Task.board` is only set in `fold_created`.

E. **"Personal" is also a side label.** Work/Personal sides on private cards are unrelated to the Personal space (crates/coord/src/sides.rs:3-4, frontend/src/lib/sample-cards.ts:1137). Personal boards named "Work" would sit beside a Work side, so Settings wording needs care.

F. **A real one-person space (`create_board` with no guest) is not a route for Personal.**
   - Existing cards would have to move, which is a data migration.
   - Board logs refuse `task.edited`, `task.where`, `task.paused`, `task.comment.edited`, `task.archived` and `task.deleted` (kinds.rs:110, :122, :133, :142-149).

**WHAT IT WOULD TAKE: Personal stays the private log, with `space.board.*` rows in it**

Personal stays a one-person space: no members, no invites, no leave, and no `board.created` row. Its default board is implicit with id "" and tag "", so every existing card stays on it with `#N` and nothing migrates. No gate is needed. The cost is about the same as one medium step.

Core:
1. crates/coord/src/board/fold.rs:1984-1989: drop the `on_board` condition on `board_tag`. Keep `tag()` gated, so the own tag stays "".
2. crates/core/src/service.rs:1071-1104: pass `board_tag` on the private coord, using `spaces::board_tag_in` on the private store. It already works with an empty own board (spaces.rs:108-119). It is called only for cards with a non-empty board. Hold a weak ref as boards.rs:1425-1431 does.
3. crates/core/src/service/spaces.rs:
   - Add `personal_boards()`, reusing `fold_space_boards` (:50) with own = {"", "Personal", ""}.
   - Add `create_personal_board`, which appends to `self.store` with the actor `user:<device>` and no membership or gate check. `create_space_board` can't be used: it requires `self.board(space_id)` (:201) and `space_not_ready` (:218).
   - Change `board_tag_holder` (:240-253) to also check Personal tags.
4. crates/core/src/service/boards_own.rs:670-709: `task_ref` must route tagged refs to the private coord when a Personal board holds the tag.
5. crates/core/src/service/boards.rs:1057 (`tag_holder`) needs no change. The check at spaces.rs:240 covers it.
6. Rename, delete and move are shared with s3. In the private log they need no gate. A move row needs a fold arm that sets `task.board` and `task.tag`. The Board column for any new kind must be set by what the space log needs. Add it to the golden list (crates/store/tests/golden.rs:101-103) and to the gate per D.

Window:

7. crates/window/src/new_card.rs:15-29, :60-70: add a `list`/`spaceBoard` field to `NewCardInput` and pass it as `NewTask.board` on both paths (also :131-137).
8. crates/window/src/boards.rs:302-312 and commands.rs:751: accept Personal, or add `boards_add_to_personal`. Expose Personal's boards through a new `personal_boards` command.
9. crates/window/src/cards.rs:1461ff: set the inner board id and tag (bug A). Keep `CardSummary.board` as the space id, because `cardInSpace` (spaces.ts:64) and Home depend on it being empty for private cards. Regenerate frontend/src/gen/bindings.ts and models.ts.

Tools:

10. crates/tools/src/board.rs:840-843, :860-875: if `board` names a Personal board, take the private path with `NewTask.board` set.
11. Don't add Personal boards to `SharedBoards::boards()` (bridge.rs:724-747). `create_on_board`, `board_coord` and `autonomy` all assume a real space id (shared.rs:400, :428; bridge.rs:753-763).
12. Show them through a new trait method, in the s5/step-5 `board` tool work, then update the goldens.

Frontend:

13. lib/spaces.ts:78-87: return Personal's boards. Add a Personal entry to `Space` rather than a fake BoardSummary. Update spaces.test.ts:83.
14. boards-page.tsx:62-66: replace the text with Personal's lists. Hide invite and members there.
15. board-view.tsx and App.tsx:924-926: Home under Personal gets the lists row. Filter by id.
16. Add a board picker to new-card-dialog.tsx:59-60, :120 and App.tsx:995-1001. Add to the palette at command-palette.tsx:143-152.
17. Add sample data to frontend/src/lib/boards.ts. Add a test that an older card stays on the default board.

Docs:

18. docs/design/spaces.md §3 and §6, talk-channels.md:45, :50, and a new row in decisions.md. Fix the comments at kinds.rs:243 and :280-284.

Tests:

19. Private log: a card on a Personal board keeps its ref across a refold; `home#12` resolves through `task_ref`; an older-style fold ignores the row without stalling; #N numbering stays one run.

**RECOMMENDATIONS FOR THE TWO BLOCKERS**

- **Personal: yes, as boards in the private log.** Rows are already accepted, nothing stalls, nothing migrates, and the default board keeps every card. The real work is items 1-4 and 7-17, and most of the frontend work is needed for shared spaces anyway. If Stephen wants less scope, Personal can ship later with no format risk: only the wording "Personal holds no shared boards" changes now.
- **Card numbers: keep one run per space and one for Personal.** Per-board runs need a fold rewrite and renumber existing cards.

Relevant paths: /Users/stephenhorton/projects/irori/.claude/worktrees/card-558-spaces-boards/crates/store/src/kinds.rs, crates/store/src/db.rs, crates/coord/src/board/fold.rs, crates/core/src/service/spaces.rs, crates/core/src/service/boards_own.rs, crates/window/src/cards.rs, frontend/src/lib/spaces.ts.