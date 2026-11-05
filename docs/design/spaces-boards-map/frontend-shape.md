FRONTEND ARCHITECTURE FOR THE SPACE PAGE (#558). Read only, nothing changed. Paths are relative to frontend/src unless they start with crates/, scripts/ or docs/. I read code only: I did not run the app, the preview or any tests.

## 1. HEADLINE: the window has no card-to-inner-board link, so this needs backend work first

**Bug today.** The per-board chips cannot work in the real app.
- The coord Task knows its inner board: crates/coord/src/board/types.rs:382 (tag), :385 (board), set at fold.rs:1143 via tag_of (fold.rs:1992).
- The window drops that. card_summary (crates/window/src/cards.rs:1461) sets no tag, and no assignment to it appears in a grep of cards.rs.
- The only tag and board writes are cards.rs:2112-2113 and :2200-2201, and both use the SPACE's board_id and tag.
- So every shared card reaches the frontend with board = space id and tag = space tag. CardBoard (gen/bindings.ts:800-811) has the same space-level shape; I did not read board_detail, which fills it, but the frontend treats onBoard.id as the space.
- cardsOnBoard (components/boards/board-page.tsx:433-436, `(c.tag || b.tag) === tag`) therefore leaves any non-first chip empty.
- A card made on `net` should also display the space's tag, not net#N, because ShortRef (components/cards/bits.tsx:445-450) reads card.tag. This is inferred from the code, not observed.
- The logic is only exercised by the unit test (components/boards/space-boards.test.ts:18-25). The preview has no Netcode cards either: every sample shared card has tag "mlg" (lib/sample-cards.ts:1818-1820, 1853-1855, 1878-1880, 1914-1916).

**Other backend gaps.**
- NewCardInput.board (gen/bindings.ts:1683-1687, crates/window/src/new_card.rs:25) is the space id.
- create_on_board (new_card.rs:123-146) never sets NewTask.board, though coord accepts it (crates/coord/src/board/tasks.rs:46, 141-146).
- No coord operation changes Task.board after creation. The only `.board =` in crates/coord/src/board/ is tasks.rs:146.
- SpaceBoard (gen/bindings.ts:2044-2049) has no counts.
- BoardSummary.cards, doneWeek and lastAt are per space.

**Bindings the frontend needs from the backend area:**
- a. A per-card inner board: a new CardSummary field for the inner board id, plus the real tag (stop overwriting it at cards.rs:2113 and :2201). Same on CardBoard.
- b. An inner board id on NewCardInput, passed to NewTask.board.
- c. A move command, e.g. board_move_card(taskId, boardId). This needs a new coord operation and row. Also settle what the ref does on a move: Task.tag folds from board, so net#3 may become mlg#3 and old refs must still resolve (has_tag, fold.rs ~2000; short_was may fit).
- d. A rename-inner-board command. space.board.renamed is read (spaces.rs:92-96) and declared (crates/store/src/kinds.rs:288) but has no writer.
- e. A delete-inner-board kind and command.
- f. Space name separate from first board name. BoardSummary.name and boards[0].name are both m.name today (spaces.rs:127-131). rename_board is at crates/core/src/service/boards.rs:1093.
- g. Decide whether Closed gets a per-board scope. See section 4.

Everything else in section 3 can be built against the sample first (for s2 mock-ups) and wired once a to f exist.

## 2. CURRENT FRONTEND MAP

**Routing, no router library**
- lib/nav.ts: Section union 11-22 (`boards` at 18, `board {id}` at 19), sectionFromHash 28-44 (`#board/` regex 41-42), sectionHash 46-55 (board at 53), go 59-63, sameSection 65-72 (board at 67).
- App.tsx: initial section 207-215 (`?boards` at 213), boardFilter 221-225, doneScope 227, popstate 337-349 (resets boardFilter at 345), goTo 567-573, onPaletteBoard 574-575, `where` 703-705.
- Card pages overlay the section, so `#card/<id>` needs no change. The `open` state is at 237.

**App render and wiring**
- Header title 770-786; BoardHeaderBar slot 787-791.
- `boards` case 884-893; `board` case 903-918; Home 924-968.
- NewCardDialog board prop 995-1002.
- Palette commands: nav.boards 613-618, New card title 639-640, board.new 647-653.
- Space state 165-177, with `shared` at 169.
- spaceCards is applied on Home, but BoardPage gets all `cards` (907) and BoardsPage too (888).

**Space model: lib/spaces.ts**
- Space type 19-25 (id, name, others; no boards).
- spacesOf 29-43, known 55-58, cardInSpace 62-66, convInSpace 72-76, boardsInSpace 80-87, threadsSpace 91-93.
- `?space=` flag 99-101, useSpace 111-135.

**Boards page: components/boards/boards-page.tsx**
- Lists spaces as "boards": intro copy 41-45, Personal text 62-66, empty state 77-85.
- NewBoardDialog 87-95. BoardTile 166-245 (go to board at 188, card match at 175). boardSegs 114-131 and StateBar 135-160 are reusable as they stand.

**components/boards/board-page.tsx. This is already the space page, misnamed.**
- BoardPage 50-146. Threads are by space id (78, AskBox 122). inSpace chip state 88-89.
- BoardView scope is the space id (108); the tag-narrowed `shown` goes in at 102 and 109.
- BackToBoards 148-155.
- BoardHeaderBar 161-295: palette commands 168-197, rename 200-210 (space rename today), pin 211-219.
- BoardSettingsButton 301-337 ("Board settings" 311-326). BoardActionsMenu 342-428 (Leave, Forget).
- SpaceBoards 442-536: chips, "+ Board" at 490-500, add form 502-533, tag required. BoardInvite 573-623.

**components/boards/parts.tsx**
- NewBoard 91, NewBoardDialog 158-186 (actually makes a space). InvitePanel 281-303. BoardSettingsPanel 522.

**components/cards/board-view.tsx**
- Scope and filter logic: ScopeOwn 47, inScope 52-56, homeBoards 60-74, activeFilter 80-102 (board: filter 98-99).
- BoardView 151, `mine` 207, members come from scope 213-216, empty case 341-362, DoneGrid 436-443.

**components/cards/card-grid.tsx**
- Filter type 62-71, asFilter 82-87, matches 111-122 (board: at 119 compares the space id).
- BoardBar labels 242 and 264. MemberBar 282.

**Sidebar: components/app-sidebar.tsx**
- Header comment 1-11, pinnedBoards 258-260, `here` 270, SpaceSwitcher 367-370.
- Boards item 420-426, "Pinned boards" group 448-459, PinRow 699. Pins are per space id.

**Space switcher: components/space-switcher.tsx**
- Trigger 59-83 (title at 65 says "Pick the space Home, Talk and Boards show"), items 85-102.
- A pick sets state only and does not navigate.

**Header and phone**
- components/app-header.tsx: New board only on `boards` (84-93); New card otherwise (95-108, title 100-104).
- components/phone-tabs.tsx: tab type 7, phoneTab 11-15, tabs 17-21.

**Command palette: components/command-palette.tsx**
- Kind 33, board items 143-151, group 194, placeholder 405-406.

**New card: components/cards/new-card-dialog.tsx**
- `board?: BoardSummary` 60, title 144, description 147-151, create call 119-122.
- Selects are in the 2-col grid 185-216.

**Card detail: components/cards/card-detail.tsx**
- PageHeader 813-878, its chip row 826-841, `extra` passed at 273-286.
- BoardActions 761-808: transfer 777-784, withdraw 787-794, Share 800-806.
- Visible to 331-338, SharedToBar 731-754.
- ScopeBadge (bits.tsx:350-356) shows scopeLabel only, so the card names its space but never its board.

**Share dialog: components/cards/share-dialog.tsx**
- ShareButton 24 picks a space (choices 102-107). WithdrawButton 193. TransferButton 227.

**Types**
- BoardSummary gen/bindings.ts:665-703 (boards at 702), zero value gen/models.ts:232-233.
- SpaceBoard bindings.ts:2044-2049, models.ts:652. CardSummary bindings.ts:982.
- lib/boards.ts is the gateway: actions 213-488, create 220-245 (this makes a space), addToSpace 261-271, rename 273-279, useBoards 554-561.

**Other surfaces**
- components/profile/profile-page.tsx: comment 4-6, friendsOf 36-51, go boards 250, Spaces column 259-264.
- components/notes/notes-page.tsx 49-60: tabs are keyed by board tag, resolved against BoardSummary.tag only (58), so notes are per space in practice. Keep that; wording only.
- components/talk/talk-page.tsx 105-129, 148, 366: threads and channels keyed by space id. components/cards/ask-box.tsx 38-51 likewise.
- These support "Talk belongs to the space" at the frontend level, and no frontend change looks needed there. s5 should still confirm the backend.

## 3. LEAST-CHURN PROPOSAL

Keep the BoardSummary-is-a-space shape and all binding names. Do not add a Space object now. Add a helper in lib/spaces.ts, `boardsOf(space: BoardSummary): SpaceBoard[]`, taking over the fallback at board-page.tsx:455. Default board = boards[0], which is the first in the list (spaces.rs:85).

**Routes (lib/nav.ts)**
- Extend the kind: `{ kind: "board"; id: string; board?: string }`.
- `#board/<spaceId>` is the space page, opening on its boards. `#board/<spaceId>/<boardId>` is one board's list.
- Existing links and pins keep working.
- Changes:
  - Parse the two-segment form before the one at nav.ts:41.
  - Update sectionHash :53 and sameSection :67.
  - Update App `where` :705 and sidebar `here` :270.
  - Add the new case to nav.test.ts:14-15.
- `#boards` stays as the hash. Under All spaces it lists every space with its boards nested. With a space picked, App 884 renders that space's page, so the Boards tab opens on the picked space. Personal is blocked on Stephen's answer.

**Components**
- New components/boards/space-page.tsx: `SpacePage`, a grid of board tiles. Reuse BoardTile 166-245 and StateBar by moving them out of boards-page.tsx (keep boardSegs exported; boards-page.test.ts imports it), with tile cards narrowed by the new inner field. Each tile calls `go({kind:"board", id, board})`. Below or beside it go AskBox and the Threads panel, unchanged and space-scoped, plus "+ Add board".
- BoardPage 50-146: if no `board` in the section, render SpacePage. If set, keep the existing BoardView path with `scope={spaceId}` and cards narrowed to that board. Do not change the scope to the inner id: inScope :52-56, members :213-216 and doneScope all need the space id.
- Delete the chip row (SpaceBoards 469-501) and the inSpace state (88-89). Keep the add form 502-533 as the "+ Add board" control. Rewrite cardsOnBoard 433-436 to match the new field, and update space-boards.test.ts.
- BoardsPage: becomes the spaces list, wording 41-45 and 77-85. BoardTile links to the space.
- BoardHeaderBar: with `board` set, show "Space / Board" breadcrumb with the board name. Rename there renames the board. Without it, the bar is the space's: Rename space, Space settings, Leave the space. Leave and Forget stay on the space only.
- Back: BackToBoards 148-155 goes to the space when inside a board.

**New card**
- Add a Board ChoiceSelect in the grid at new-card-dialog.tsx:185-216, shown when the space has more than one board.
- Change the prop to `board?: BoardSummary` plus `inner?: string`, defaulted from section.board. App 995-1002 passes section.board. On the space page the default board is selected.
- Pass it through `board` in NewCardInput per 1b.
- sampleCreate (lib/cards.ts ~787-810) must honour it.

**Move card (card detail)**
- New `MoveBoardButton` in share-dialog.tsx, shaped like TransferButton 227, rendered in BoardActions after Transfer at card-detail.tsx:777-784.
- Show it when detail.onBoard, the space has more than one board, the card is not over, and it has no parent (same rule as Withdraw 787). A parent takes its subtasks along; the dialog says so using the `family` pattern at :791.
- Control: Dialog with ChoiceSelect of the space's other boards, or a DropdownMenu for one click.
- Extend ScopeBadge (bits.tsx:350-356) to "MLG · Netcode", so the card shows where it sits. On the phone the chip row wraps (826), so check card-shared in check:phone.
- ask cards excluded, matching 798.

**Add, rename, delete a board**
- Add: the existing form, with name and tag.
- Rename: BoardHeaderBar's inline form 223-239, pointed at a new boards.renameInSpace when `board` is set.
- Delete: a destructive item in BoardActionsMenu 402-414 through ConfirmDialog, shown only inside a non-default board.
- Rules that need agreeing with the backend area: the default board can't be deleted; cards on a deleted board move to the default board, and the confirm text says how many. Owner-only, matching b.owner at :183.
- lib/boards.ts gateway additions: renameInSpace, removeFromSpace; lib/cards.ts: moveToBoard. Each with an inWails branch and a sample branch.

**Space settings, sidebar, palette**
- Wording only: Pinned spaces (app-sidebar 449), Share to board dialog says the space (with an optional Board select).
- Palette: group the spaces, add one item per inner board (143-151), New space at App 647-653.
- Home's `board:` filter stays space-level; relabel card-grid 242 and 264 to "space". homeFilterKey saved values keep working.
- Optional: list the picked space's boards under Boards in the sidebar with PinRow 699.
- phoneTab 11-15 needs no change because the kind is unchanged.

**For s2 to decide:** does a one-board space show the tiles page or go straight to its list? The brief says every space opens on its boards, so I recommend always showing tiles. That is one rule, and "+ Add board" stays visible.

## 4. OTHER TRAPS

- Closed is not narrowed by board today. `done` is scoped by space (App 227-229) and DoneGrid 436-443 gets it unfiltered, while useDoneCards pages server-side (lib/cards.ts:2030-2037, 2070). On a board's list this needs a scope such as `<space>/<board>` in done_cards_in, or a client filter that gives short pages. The same applies to clear-done (DoneGrid byAge=scope at :438).
- The space page should keep the `mine.length === 0` empty case: BoardView 341-362 already handles an empty board, and the empty copy lives at board-page.tsx:129-134.
- Card counts in BoardHeaderBar 247-251 (b.cards) are per space. Inside a board, count client-side.
- lib/owners.ts sharedBoard 38-46 uses c.board = space id. If 1a adds a new field and leaves board alone, this keeps working. Do not repurpose `board` itself: cardInSpace, inScope, matches, owners, sample scoping and the done scope all depend on it.
- #538 (no-board DM spaces): a space with boards=[] needs SpacePage's empty state; boardsOf must not fabricate one if that rule is agreed. spacesOf 29-43 would also list hidden ones unless filtered.

## 5. PREVIEW AND MOCK MODE

**How it runs**
- Outside the app, `inWails()` is false and each lib/*.ts runs its sample branch (frontend/PREVIEW.md "How it works").
- previewFlag (lib/preview.ts:28-31) reads query params and returns null inside the app.
- Sample boards: lib/boards.ts:116-179 (MLG with Netcode at 161-164, Website refresh). The bridge to cards is setSampleBoards at 196 and cards.ts:783.
- Sample cards: lib/sample-cards.ts, shared ones from ~1818.
- PREVIEW.md is partly stale: it still says Go and Wails, calls `?boards` a dialog (it is the page, App 213), and has no `?space=` row.

**Changes needed for s2**
- Give some sample cards the inner field and `net` tag. Add a third board and an unsplit single-board space (Website refresh already is one).
- Sample branches for rename, delete and move. Add a helper like sampleMoveCard beside sampleWithdraw (cards.ts:1198).
- New preview URLs: `/#board/sample-board-mlg` (space page), `/#board/sample-board-mlg/sample-board-mlg-net` (one board), `?space=sample-board-mlg#boards`.
- Add those to PREVIEW.md, scripts/preview-shots.sh, and PAGES in frontend/scripts/phone-check.ts:35-61 (which already has boards and board-mlg). Worth adding a `?move=<card>` flag like `?transfer` (card-detail:782) and a preview.ts comment line.

**Running it**
- This worktree has no frontend/node_modules and no dist. Run `cd frontend && bun install --frozen-lockfile` first.
- `bun run preview:look` serves http://127.0.0.1:9246/, loopback only.

**Screenshots**
- Desktop: `sh frontend/scripts/preview-shots.sh` writes to frontend/preview-shots/ (gitignored). On a Mac it needs BROWSER_EXE set to Chrome's binary, because its default paths are Windows ones (:11-14). Also PREVIEW_MODE=light, PREVIEW_URL. To add a shot, add a `shot name "url" [w h]` line. It shoots all rows; for just a few, use a small script file.
- Phone: `bun run check:phone [names]` runs at 390x844 via DevTools emulation, fails on overflow, and writes to preview-shots/phone/. It finds Mac Chrome itself. Do not use --window-size under 500 (scripts/phone-check.ts:14-18).
- User memory notes:
  - Claude-in-Chrome may be on the Windows PC and cannot reach Mac localhost; use local headless Chrome with --user-data-dir and --virtual-time-budget=8000.
  - In a worktree session, put the browser call in a script file, because the Bash guard refuses a command name held in a variable.
  - `screencapture` does not work for agent sessions on the Mac; whole-app shots need the person to grant Screen Recording. scripts/window-shot.ps1 is Windows only.
  - ffmpeg can stack PNGs into a contact sheet.
  - I did not verify Chrome's presence here.

## 6. STYLE

- There is no CLAUDE.md in this repo and no .claude directory at the worktree root.
- frontend/.prettierrc: no semis, double quotes, trailing comma es5, 80 cols, tailwind class sort. Older files, including board-page.tsx, run past 80, so expect noise if you reformat. `bun run format` rewrites everything, so avoid it on a focused diff.
- eslint.config.js: flat config, react-hooks recommended, src/gen ignored. src/gen is generated: never hand-edit it.
- Comment convention: each exported component or function opens with a plain-language comment naming what it is, citing card numbers. Components never import src/gen; they go through lib/*.ts gateways (lib/boards.ts:3-4).
- plainLine() on all user-supplied text, cn() from lib/utils, `glass-panel`, text-ink, text-ink-2, text-muted-foreground, text-[12.5px] / text-[13px], bg-primary/20.
- Controls: ChoiceSelect (components/common/choice-select.tsx) for one-of-a-list, ConfirmDialog/ConfirmButton (components/common/confirm-button.tsx) for confirms, Button size="sm" or variant ghost, DropdownMenu from components/ui, lucide icons with aria-hidden, errors through errorText.
- UI text is plain sentences. On the phone, use useNarrow and pointer-coarse sizing.
- Writing: no filler words; "Space" and "Board" per the card.

## 7. TESTS AND CHECKS

**Frontend**
- bun:test, 49 test files under src, run with `bun test` in frontend/. It is plain logic with no DOM renderer, so keep new logic in exported pure functions (as with boardSegs and cardsOnBoard).
- Tests to touch: lib/spaces.test.ts, lib/nav.test.ts, components/boards/boards-page.test.ts, components/boards/space-boards.test.ts, lib/boards.test.ts, lib/needs.test.ts, components/cards/board-view.test.ts, components/cards/card-grid.test.ts, components/command-palette.test.ts.
- Other commands: `bun run typecheck`, `bun run lint`, `bun run build`.

**scripts/check.sh (run before every merge or push; there is no CI)**
- Slot-gated by scripts/check-slot.sh (CHECK_SLOTS, default 2).
- Frontend: bun install --frozen-lockfile; `bun scripts/gen-models.ts --check`; build; typecheck; lint; bun test.
- Rust:
  - cargo fmt --check
  - cargo clippy --workspace --all-targets -D warnings
  - cargo test --workspace
  - `cargo xtask bindings --check`
  - cargo test -p irori-capture -- --test-threads=1
- Then scripts/no-third-party.sh, and an Android cargo check if the NDK is present.
- The app crate embeds frontend/dist, so the frontend builds first.
- After any window type or command change: run `cargo xtask bindings`, then `bun scripts/gen-models.ts`, and commit src/gen.

**Tool goldens**
- testdata/golden/tools/ (budget.json, scenario.json, specs.json plus transcripts), checked by crates/tools/tests/golden.rs. The goldens were exported by tools/goldens/export.sh and tools/goldens/tools_golden_test.go. Read those before updating; I did not establish the regeneration path.

**scripts/agent-check.sh**
- 7 scenarios, each a real Claude session that spends tokens. It is by hand before a release (docs/releasing.md step 1) and not part of check.sh.
- `sh scripts/agent-check.sh --only 1,2` runs a subset. No scenario covers spaces or boards. Because #558 changes agent-facing text and the board tool, I recommend running `--only 1,2,3` as a smoke test and the full set at release. Consider a new scenario for "agent sees a space with its boards" (crates/dev/src/agent_check.rs).

**UI gate**
- Before landing: `bun run check:phone` with the new pages, plus desktop shots in dark and light for the "Land it like this?" blocker.