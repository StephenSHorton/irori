REVIEW #558: spot-check of the mapping reports at 23093055. I ran no tests or app. The repo is unchanged; the only file written was a scan script at /tmp/r558/scan.py. Most report claims hold. The errors, gaps and conflicts come first, then a per-report verdict.

**A. TRAPS IN NO REPORT, OR UNDERSTATED**

1. **The gate is a routine blocker, not only a rollout one.**
   - `heard` is in memory only, and a device not heard since this start counts as not ready (boardmesh.rs:579-592, :415).
   - A dead but unrevoked device blocks forever; five_cores.rs:1315-1335 pins exactly that with "old laptop".
   - If move and delete use this gate, one sleeping phone after a desktop restart refuses a card move. Talk and add-board already behave this way, but move is the core action in "Done when".
   - board-plumbing's Open item 4 mentions it only as rollout. It should be a blocker for Stephen.

2. **`spaces: 1` peers are in the field.** 9d5a72dc is in every tag v0.11.0 to v0.12.3. Any v0.11.0+ device that joins or reconnects after a `task.board` row exists stalls on that writer's stream, because the gate is write-time only.

3. **Gate-free option nobody weighed.**
   - `space.board.renamed` is accepted by every `spaces: 1` peer. `SpaceBoardRow` has no deny_unknown_fields, and empty name and tag are a no-op (spaces.rs:72-81).
   - A row such as `{boardId, name:"", tag:"", deleted:true}`, or one carrying `taskId` for a move, would not stall those peers. `resolves` still checks `taskId`.
   - It needs no SPACES_PROTOCOL bump and no `min` argument.
   - Costs: the kind name lies, and v1 devices show the old placement until they update.
   - A coord move fold would need "space.board." added to FOLLOW_KINDS, and it still needs the min-1 gate, so item 1 stays.
   - Put it beside the new-kind plan for Stephen. I am not recommending it over that plan.

4. **`refold_if_cut` clears each map by hand (tasks.rs:974-998).** The proposed `board_at` must be cleared there. Otherwise, after a cut, stale positions make the replay drop real move rows and the card lands on the wrong board. The same applies to any retired-tags cache kept in Fold.

5. **A move row can be lost on replay.**
   - `aside_gate` asks the store, which holds the card already, so during replay the row passes.
   - If it sorts before `task.created`, `tasks.get_mut` finds nothing and the row is gone, though it folded live.
   - HLC normally prevents this, but `observe_bounded` caps the clock on verifier stores (db.rs:1284-1288).
   - Copy the `links` model: record the move even with no task, and apply it at fold_created or in a post-replay pass, as relink does (subtasks.rs:34-60).
   - Severity is low; the fix is cheap. I did not check whether rows past the bound are refused earlier.

6. **`retag_boards` as written is expensive.** `tag_of` then `board_tag_in` lists and refolds up to 10,000 rows per card (spaces.rs:91-119, boards.rs:1425-1431), under the coord lock, on each `space.board.*` row. Read the space's boards once per pass. No race with the coord thread: the tag source is the store.

7. **`has_tag` is card-derived** (fold.rs:2000): `tag == self.tag() || any task has the tag`. A board with no cards, or one emptied by a move, makes `net#4` return "a card on another board" even after the per-card compare at short.rs:215 is dropped. The "tags ever named" hook has to cover all of `space.board.*`, not just retired tags.

8. **`boards[0].name` is empty on a joined, not-ready space.** `boards` is filled at boards_own.rs:521 from `m.name`, before the `joined_name` fallback at :524-525. The fallback at board-page.tsx:455 does not fire, because the list is non-empty. The `boardsOf` helper must fall back per field.

9. **`tools_board` feeds both `boards()` and `board_of()`** (bridge.rs:546, :749). If only the own entry in `boards()` takes `inside[0].name`, `board_of` still prints the space name, so "already on X" and `label()` disagree after a first-board rename.

10. **Personal's default board cannot be renamed through this fold.** With own id "", the row dies at `r.board_id.is_empty()` (spaces.rs:57). The Personal plan needs a sentinel id or an empty-id rule.

11. **`move_card` "on a board only" contradicts Personal step 6.** The plumbing fold arm is gated on board; Personal needs the same arm on the private log. Decide after Stephen answers on Personal.

12. **Subtask rules are incomplete.** A parent's move writes one row per child (not atomic). Neither report refuses moving a subtask alone at the writer; only the frontend hides it. Adoption across boards is unspecified (`task.parent` via subtasks.rs:393 onward, `check_parent`).

13. **Two rename paths, two authorities.** `rename_board` needs owner plus current key epoch (boards.rs:1093-1125). `rename_space_board` as proposed needs only an active member. When copying spaces.rs:185-220, drop the tag validation and the holder check, or a name-only rename fails.

14. **The gate refusal text will still say the wrong thing, and tests pin it.** Rewording spaces.rs:271 breaks spaces.rs:552 ("run an Irori with spaces"). Keep "waiting on": spaces.rs:553 and five_cores.rs:1335 pin it. The report cites :550-551; the assert is at :552.

15. **`SINCE_GO` is a sized array** (`[&str; 24]`, store/tests/golden.rs:77). Adding two kinds means bumping the length to 26.

**B. PER REPORT**

**ui-words**

Confirmed:
- Prose-keyed code: BACK_NOTE is the only production match (fold.rs:2266-2270). The only other prose matches are in test asserts; frontend prefix matches are identifiers. thread.ts:454 keys a tool name.
- My scan of frontend/src, crates/window, site, app/src and crates/notify found no visible "board" string missing from the report. That scan is a heuristic and excludes samples, tests and comments.

Errors and gaps:
- MISSED: crates/inbox/src/watch.rs:228, 229, 231 (`hold_reason`). These reach the window through crates/window/src/mentions.rs:91, :129, and watch.rs:661 uses them too. The report lists only the lib/talk.ts twins. The phone report left this untraced, and it is now traced.
- MISSED pin: boards_status.rs:406 pins "You're no longer on this board".
- MISSED pin: testdata/golden/identity/board.json:291 is a serialized membership holding at least the "not on the board" and "not a board name" conflicts, compared at identity/tests/golden.rs:509. Rewording membership.rs:566 or :629 breaks it, and crates/identity/tests/membership.rs pins conflicts too; I did not check those conflict strings against the rewords. Neither file is in the report's pin list.
- Advice: leave membership.rs conflict text and the store errors alone. testdata/golden/store/board.json has 37 lines with "this board" (at least 7 are "is not a card on this board"). If you do change them, these are Go-exported vectors to hand-edit.
- OVERCLAIM: "coord ... also move goldens". In testdata, the phrases shared board / on the board / this board appear only in tools/specs.json, mcp/wire.json, api/go.json, identity/board.json and store/board.json. The transcripts have none.
- GAP in section C: tools/scenario.transcript.json has 4 "the Board" lines ("folds into Done on the Board", "shows on the Board", "stays on the Board", "use the Board"), and their source strings are not listed. Rewording "the Board" moves that transcript.

**phone**
- B1 says an inner-board page needs a new section kind. frontend-shape extends `kind:"board"` with `board?`, so phone-tabs.tsx:13 needs no change. Frontend-shape's form is simpler; take it.
- The hold-reason open item is closed by the ui-words note above.
- I did not check native files, the plist or xcodegen.

**agent-text**

Confirmed:
- Golden counts 11/33/22/11.
- `< 4096` test, so +9 from 4086.
- Bugs a and b (shared.rs ~502, ~222-227).
- `.claude/worktrees/irori-go` is absent, so hand-editing is the only path.

Errors:
- The proposed print `mlg "Tasks" (default)` contradicts plumbing §3: no such name exists, and the first board shows the space's name until renamed. Either print that, or decide a default name for new spaces. Writing a row at creation is not safe (see plumbing).
- `own.name` change: see A9.

**board-plumbing**

Confirmed:
- `task.board` passes FOLLOW_KINDS and `resolves`.
- `aside_gate`, `check_shape` refusing unknown kinds, and RowPos order against db.rs:1608.
- `summary_row`, boards.rs:991 and tasks.rs:133-147.
- Own-id drop at spaces.rs:57 and the `board_tag_in` fall-through.
- `task.placed` would overwrite x/y (placed.rs:59), and `task.edited` is refused.
- `follow_board_row` is on the same store subscription; `retag` from there works, subject to A6.
- `fold_space_boards` re-reads the log in hlc order, so rename and delete fold order is safe across writers. Tombstones are still right.

Gaps:
- Last-writer-wins by hlc means a forward-skewed peer wins a crossing move. This is inherited from `links`; state it in the test.
- "Five places" for old refs is at least seven: add mentions.ts:543 (`localRef`) and sample cards.ts:559. Both compare `x.tag === r.tag`, so an old `wind#4` fails in the frontend even after the Rust fix.
- spaces.rs:528 and the comment at :524-525 state "own id ignored". Only CREATED stays ignored; update the comment. On the own id, fold name only and ignore `r.tag`, or `own.tag` forks from `membership.tag`.
- 0g `serde(default)` matters only to the new fold. Shipped peers refuse `space.board.deleted` at `check_shape` before decode, so row shape does nothing for them. Writing name:"" and tag:"" with the existing struct also works.
- `space_board_rows` limit 10,000 is oldest-first, so overflow drops the newest rows, including deletes. Unlikely, but note it.
- Also update kinds.rs:280-284 and spaces.md §4 ("SPACES_PROTOCOL = 1").

**personal**

Confirmed:
- `check_shape` is board-only (db.rs:1063, :1300).
- Private log stores unknown kinds; fold ignores them (fold.rs:1102).
- `AskField.board` is a separate field.

Errors:
- D's objection applies only to a bare bump (see conflict 1).
- A10 and A11 are the two plan gaps.

**talk-threads**

Confirmed:
- B1 (window/talk.rs:231).
- B2 (tools/talk.rs:508, :583-587).
- B3's `threadsSpace` (spaces.ts:91-93).
- B4 tag writes and the threads.rs:93 hearer miss.
- The line-49 DM correction against `convInSpace`.

The B1 and B2 fixes are real work, not "confirm" work, so s5 is under-scoped as written.

**frontend-shape**

Confirmed:
- Window tag loss.
- board_detail sets space fields (boards.rs:414-415, which the report says it did not read).
- The nav.ts:41 regex is greedy, so the two-segment form must parse first; `decodeURIComponent` applies per segment.

Corrections:
- CardBoard: as above.
- board-view.tsx:71 builds Home chips with `c.tag`, which turns inner after the fix; plumbing lists this and it must land together.
- Alternative for the window: resolve tag from `t.board` against `s.boards` at cards.rs:2113, :2201. That is always current after a delete, but agents still need the retag.
- Pins: profile-page.tsx:293 also reads `b.boards`; include it in the `boardsOf` change.

**C. CROSS-REPORT CONFLICTS**

1. **Gate form.** Plumbing §2 (`SPACES_PROTOCOL = 2` plus `min`) against personal D (new hello field). `h.spaces < min` keeps Talk, add and notes at 1. v1 readers only test `< 1`, nothing else reads `.spaces` outside boardmesh, and `spawns` is precedent but unnecessary. Use `min`. Keep `Core::space_not_ready(id)` as a min-1 wrapper, or edit the 7 test calls.
2. **Who renames or deletes a board.** Plumbing says any member; frontend-shape says owner-only. One blocker line for Stephen. No fold author check exists either way.
3. **Tab word.** ui-words says "Spaces". Frontend-shape has `#boards` render the picked space's page. Under that, the tab is Spaces and the page shows boards. Decide in s2.
4. **Ref on move.** Frontend-shape suggests `short_was` may fit. Plumbing rightly rejects it: replay stamps the final tag only, so take the log-wide tags hook.

**D. NEW BLOCKER LINES FOR STEPHEN** (beyond Personal and numbering)
- Move and delete behind the gate: refused while any device is unheard, or use the accepted-kind route (A1-A3).
- Any member or owners for board rename and delete.
- Should a first board keep following a space rename until it has its own name.
- Does "the Board" (queue of open work) stay as a proper noun.

**E. NOT CHECKED**
- Talk B5, B6 and the B3 consequences.
- Phone native files, the plist, and site text beyond the scan.
- `hello_says_it_reads_spaces` body.
- `label()` and `refusal()` wording.
- Spec length figures, beyond the test bound.
- Whether Rust text outside crates/window, crates/notify and the crates named in the reports reaches a person.