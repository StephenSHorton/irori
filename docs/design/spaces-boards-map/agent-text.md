REPORT #558: agent-facing text, board tool, goldens. Read-only; the repo is unchanged. Two scan scripts went in /tmp. The advisor call was rate-limited, so this is unreviewed.

**1. HOW THE BOARD TOOL PRINTS TODAY**

Path:
- `Set::call` (crates/tools/src/lib.rs:468) calls `around_board` (shared.rs:243).
- For name=="board" it appends `boards_section()` after the card list (shared.rs:252-257, 281-299).

Format (shared.rs:286-296), one flat line per entry:
```
Shared boards you may post on (board_create board=<tag>; anything there is seen by everyone on it):
- mlg "MLG farming": Stephen, Blake; your autonomy there: trusted
- net "Netcode": Stephen, Blake; your autonomy there: trusted
```

Data (bridge.rs:723-747):
- `board_list()` filtered by ready. Per space, `own` comes from `tools_board` (bridge.rs:546-554).
- Inner boards are `space_boards().skip(1)`, built with `..own.clone()`, overriding tag, name and space_board. Own is inserted first (:743).
- So an inner entry inherits board_id (the space's log id) and people, but loses the space's name and tag.
- `BoardSummary.boards` already holds this list (boards_own.rs:158, :521), so the second `space_boards` call at bridge.rs:732 is redundant.

Card lines:
- Cards from all spaces and boards merge into one view by state (board.rs:106-120).
- The only board marker is the ref prefix: board.rs:41 and types.rs:530 give net#12, and fold.rs:1984-1996 resolves the tag, falling back to the space tag.
- There is no grouping by board, and "spaces with their boards" doesn't need it.

**2. WHAT "SPACES WITH THEIR BOARDS" NEEDS**

tools::BoardInfo (shared.rs:27-38) has board_id, tag, name, people, space_board.

Recommended: keep `boards()` flat as post targets and add one field, `space_name: String`.
- All four fakes use `..Default::default()` (tests/shared.rs:96, tests/talk.rs:354, 546, 652), so they still compile.
- `find_board` (shared.rs:190), `space_known`, `space_tag`, `space_name_arg` (talk.rs:654, 693, 708) and their OddSpaces pins keep working.
- The rest of the flat model needs no restructuring. Inner entries already share board_id with own, and own comes first, so `boards_section` can group adjacent entries by board_id.

A nested `spaces()` trait method would touch every fake and the talk helpers for no gain.

Bridge change (bridge.rs:730-745, 546-554):
- Fill space_name on every entry.
- Once card item 3 lands, own.name must be the default board's name, not the membership name. Today spaces.rs:126-130 takes both name and tag from the membership.

Keep the default board's TAG equal to the space tag:
- mlg#12 refs (fold.rs:1996) depend on it.
- `board=mlg` then means "the default board".
- The tool layer needs no new tag field; only the name separates.

Proposed print:
```
Spaces you may post on (board_create board=<board tag>; anything there is seen by everyone in the space):
- MLG farming: Stephen, Blake; your autonomy there: trusted
  boards: mlg "Tasks" (default), net "Netcode"
```
Autonomy and people belong on the space line: `inbox.autonomy` is keyed by space id (shared.rs:295), so today it repeats per inner board.

Other printing changes:
- `label()` (shared.rs:46) should say which space an inner board is in.
- `refusal()` (shared.rs:173-182) says "X is shared"; it should name the space.

Rules to settle:
- If a default board can be deleted, or #538 spaces have no board, `board=<space tag>` has no target. Today `fold_space_boards` always returns own, and bridge.rs:733-743 always lists it.
- Nothing in tools or the bridge hides a future DM space from this list. The only filter is `s.ready` (bridge.rs:728). #538's hidden spaces would show as postable unless filtered.

**3. BUGS FOUND**

a. board_share to an inner board silently lands on the space's first board.
- shared.rs:502 and :625 pass `b.board_id`.
- `SharedBoards::share` has no inner-board parameter (shared.rs:68, bridge.rs:779, boards_own.rs:1034).
- Yet the result prints the inner `label()` (shared.rs:510, 533, 629).
- Found by reading the code, not run. Tests only cover mlg, and the card's item 2 needs this fixed.

b. The ambiguous-tag refusal (shared.rs:220-227) offers `board_id`. For an inner hit that is the space id, which then resolves to the space's own board (shared.rs:201), not the inner one. It should print space_board when set.

c. notes scope matches only space id or tag (core notes.rs:72-78), so an inner tag gets "no board". talk `space=` accepts an inner tag and maps it to the parent (talk.rs:736).

d. `board_of` (bridge.rs:749) always returns the space level with space_board empty. "already on X" (shared.rs:486) therefore names the space, never the inner board.

**4. TEXT THAT SAYS BOARD BUT MEANS SPACE**

crates/tools/src/specs.json (11 "shared board" hits):
- :102 agents
- :486 board_create board arg
- :777 board_block `for`
- :1019 wait
- :1107 board_share
- :1117 board_share board arg
- :1140 board_review task_id
- :1520 release
- :1663 notes_propose scope
- :1678 tags `board:<tag>` explanation
- :1708 notes_search

Also in specs.json:
- :1657 "board note"
- :1107 "Share to board" (a UI label; follow the window's wording)
- "shared card" ×8, including :1107, :1134, :1148, :1164 ×2, :1208, :1520. This phrase is still true under the new model; I'd keep it.
- :660 and :1148 mention "your own board" and "board:… key". The word in :660 hangs on the Personal question.
- :761 "several by space" is whitespace, a false hit.
- :1772, 1803, 1844, 1870 already say space.
- The `board` tool's description (:440) never mentions spaces, and has room (see limits in section 6).

crates/tools/src:
- shared.rs: 193, 211, 214, 287, 309, 476, 483, 638, 659, 749, 765, 851, 857, 865
- shared.rs:954 "your own board" (Personal question)
- shared.rs:405 "parent's board" is correct, keep it
- mention.rs:248 "(on the board)" (pinned by tests/shared.rs)
- board.rs:1204, 2600
- observe.rs:71, 81, 112 ("agent on board {}"; the value comes from BoardAgent.board, lib.rs:185)
- waits.rs:96
- notes.rs:170
- lib.rs:170

crates/core/src/service:
- bridge.rs:447, 452, 498 ("started a thread on this board"). The card says threads belong to the space, so these should change with item 4.
- bridge.rs:469, 515 ("Your autonomy on this board")
- notes.rs:47, 83, 483
- mirror.rs:113
- spawn_permissions.rs:96
- boards_status.rs:117, 124
- boards_members.rs:214
- boards_own.rs:841-842, 853, 1193, 1217
- boards.rs:980, 988, 1099 ("a board name" on create_board, which is a space name), 1743, 2177, 2505

crates/inbox/src:
- fence.rs:112
- watch.rs:229, 231

crates/coord/src/board (errors reach agents through map_err):
- review.rs:255, 490, 649, 663
- owner.rs:128
- desk_person.rs:25, desk.rs:380
- comment_edit.rs:26
- mirror.rs:89, 120, 173
- stranded.rs:202
- comments.rs:176, 373
- mod.rs:468, 1682, 1701
- tasks.rs:189, 264
- subtasks.rs:600, 688

No change needed:
- Brief templates spawn.rs:518, 653, 787, 792 ("the Board's rules"). spawn.rs:602 already says shared space.
- crates/mcp: only "Board notices" at lib.rs:92, plus the embedded fallback list from tools.
- mod/hooks: lib.ts and register.tsx have no shared-board or board_create hits.

Keep as they are (wire or stored names):
- tool names board_*
- args `board` (it now correctly means a task list) and `space`
- the `board=` message attribute and `board:` sender keys
- the `board:<tag>` note tag syntax (reword the explanation only)

Open, depends on the Personal blocker:
- "The Irori Board" / "your Board" / "own board" in BOARD_INSTRUCTIONS (specs.rs:49 on) and in waits.rs:96.
- The aggregated view across Personal and all spaces isn't one task list under the new model.

**5. GOLDENS**

Where:
- testdata/golden/tools/: specs.json, scenario.json, scenario.transcript.json, budget.json, budget.transcript.json
- testdata/golden/mcp/wire.json: `fallback`, plus one recv line in run "not running"
- testdata/golden/api/go.json: step tools-grok

Regeneration:
- There is no working tool. tools/goldens/export.sh needs a Go checkout at ../irori-go (Go be67a86). None exists at projects/irori-go, and I didn't check .claude/worktrees/irori-go, which is where the worktree default points. It would also export Go's old text.
- I found no Rust writer in crates/tools/tests, scripts or the crates/dev sources I grepped.
- History shows these files edited by hand or script alongside the source: 5c20c1dd and 07a48083 touch crates/tools/src/specs.json plus exactly api/go.json, mcp/wire.json and tools/specs.json. Commit 82468eb6 calls this "regenerated from the code".

Procedure: replace the literal phrase in all four files.

| File | Copies per sentence | "shared board" count |
|---|---|---|
| crates/tools/src/specs.json | 1 | 11 |
| testdata/golden/tools/specs.json | 3 (specs "", claude, grok; other is []) | 33 |
| testdata/golden/mcp/wire.json | 2 | 22 |
| testdata/golden/api/go.json | 1 | 11 |

- Plain words have the same bytes at every nesting level. Text with quotes or < needs per-level escaping (\u003c, nested strings).
- BOARD_INSTRUCTIONS and ASK_INSTRUCTIONS exist in specs.rs AND inlined in each description (ported.rs:513, :528 check `contains`). They are also separate keys in tools/specs.json.
- scenario and budget transcripts have zero hits for shared, space or "Visible to" (219 results; the harness has no shared boards). The shared.rs, mention and bridge rewording won't touch them.
- Changing any text the private harness prints, such as in board.rs or observe.rs, could touch them. Their test diff shows which.

**6. TESTS THAT PIN THIS**

Goldens:
- crates/tools/tests/golden.rs:212 `tool_texts_match_go`, :217 `budget_texts_match_go`, :222 `specs_and_instructions_match_go`
- crates/mcp/tests/wire.rs:54 `wire_matches_go` (instructions :96, fallback :100-104)
- crates/api/tests/api_golden.rs (compare around :417; step defined at api/scenario.json:86)

Spec limits (crates/tools/tests/ported.rs:399 `test_specs`):
- A description must be under 4,096 chars (:540-546).
- Own text must be at most 500 bytes, excluding the instructions blocks (:534-539).
- Phrase pins at :589 and :632; tests/board.rs:1807 (:1848) has more.
- Current maximum: board_comment 4086, board_split 4085, board_block and board_unblock 4084. BOARD_INSTRUCTIONS can grow by 9 characters at most.
- Own text before that block: board_create 490 bytes, board_review 486, board_share 440, board 375.
- wait is 466 and notes_propose 485, with no block.
- "space" is shorter than "shared board", so the swaps are safe.

Phrases:
- tests/shared.rs:488-505 `the_board_tool_lists_shared_boards` (header and line format, "no shared board", "Yours: mlg.")
- tests/shared.rs also pins "isn't on that person" and "(on the board)"
- tests/boards2.rs:50 ("is leased on the shared board by")
- tests/talk.rs:481 ("no shared board" through `space=`)
- tests/talk.rs:540-644 (OddSpaces, flat tag counting), and "(on the space)"
- core/tests/boards.rs:543-578 (the only tool-level inner-board test: levels#1 then lemmings#2, one number run per space), :934, :1442
- core/tests/five_cores.rs ("no longer on this board"; "Visible to" at :419, :935)
- core/tests/notes.rs ("a person on the board")
- spawn.rs:2639

I didn't check every coord and inbox string for pins. Grep each phrase before changing it.

Also, unpinned: docs/design/board.md:35 holds a copy of the instructions block. The gate is scripts/check.sh:26 (`cargo test --workspace`).