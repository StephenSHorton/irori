# #558 build reports and reviews, saved at wind-down

## build:core

The full run is still going, and the monitor will report when it ends or fails.

## build:words

Card #558 wording is mostly done and pushed to `card-558-words` (head 03438211, 8 commits on `origin/worktree-card-558-spaces-boards`), but most coord "shared board" errors are unchanged and still reach the window. Those are listed under Not done.

**Done**
- **Frontend:** palette, header, space switcher, phone tab comments, space page and its phone rename item, New space dialog, share dialog, card grid and detail, ask box, notes, profile, settings, hold reasons, samples and gallery.
- **New-card text:** it picks "on this board" or "in this space" from `section.board`. This changes text only.
- **"Own board" wording:** it now says "in Personal", because Personal is a space (spec b1).
- **Rust:** `crates/window`, `crates/inbox/src/watch.rs` `hold_reason`, `crates/invite` (`flow.rs`, `bundle.rs`), `identity/src/board.rs:729`, `boards_status.rs`, `spawn_waiting.rs` (found by me, not in any report).
- **Core errors:** `boards.rs`, `boards_members.rs`, `boards_own.rs`, `mirror.rs`, `notes.rs`, `spawn_permissions.rs`. Three errors no longer print an id.
- **Four coord lines no report assigns to agent text:** `comments.rs:419`, `fold.rs:774`, `owner.rs:149`, `tasks.rs:111`. These are in their own commit, c78b62de.
- **Outside the named surfaces:** `site/src/App.tsx`, the iOS `project.yml` and `Info.plist` (edited by hand, xcodegen not run), `docs/android.md`, `docs/ios.md`.
- **Move and delete check:** no added line mentions moving a card between boards or deleting a board. Every move or delete hit in the diff is about cards, people or files.

**Not done**
- **Most of coord's "shared board" errors are unchanged and still reach the window.** agent-text.md §4 lists them too, so I left them for that branch rather than risk conflicting wording. If that branch doesn't take them, they stay wrong:
  - `review.rs:255,329,490,649,663`
  - `owner.rs:128`
  - `desk.rs:380`, `desk_person.rs:25`
  - `comment_edit.rs:26`
  - `mirror.rs:89,120`
  - `stranded.rs:202`
  - `comments.rs:176,373`
  - `mod.rs:468,1682,1701`
  - `tasks.rs:189,264`
  - `subtasks.rs:600,688`
- **Pins on those strings:** `coord/tests/boards2_test.rs:2081` pins "your own board", and `tools/tests/boards2.rs:50` pins the `mirror.rs:120` text.
- **`crates/tools/src/notes.rs:170`** still says "a person on the board". That text is the tools crate's, so I left it and `core/tests/notes.rs` keeps that pin.

**Overlap to reconcile**
- These core lines are also listed in agent-text.md §4, and I changed them here:
  - `notes.rs:47,483`
  - `mirror.rs:113`
  - `spawn_permissions.rs:96`
  - `boards_status.rs:117,124`
  - `boards_members.rs:214`
  - `boards_own.rs:841-842,853,1193,1217`
  - `boards.rs:980,1099,1743,2177,2505`
- The same applies to `watch.rs:229,231`.

**Other things to know**
- `boards_own.rs:1227` now writes "withdrawn from the space" into new rows. Old rows keep the old text. `coord/mod.rs:1705` keeps "withdrawn from the board" because `testdata/golden/coord/replay.json` pins it, so the two defaults differ.
- Left on purpose:
  - `membership.rs`, store errors, tools, mcp, `specs.json`, `bridge.rs`, `fence.rs` and all goldens.
  - "the Board" as a proper noun, including `notify` "Open Board" (`notify/lib.rs:618`).
  - Tag and card-ref lines, which keep "board".
  - `core/notes.rs:83,90`, `api` errors, and identifiers.

**Test results**
- **`bun run typecheck`:** passes.
- **`bun test`:** 545 pass, 1 fail, 1 error, all from one cause. `space-boards.test.ts` imports `cardsOnBoard` from `./board-page`, but that export moved to `space-boards.tsx` on the base commit. It was already broken, so I didn't fix it.
- **`cargo fmt --check`:** OK.
- **`cargo clippy --workspace --all-targets -- -D warnings`:** clean, run before the last test-only commit. It needed `bun run build` first, because `app/build.rs` wants `frontend/dist`.
- **`cargo test` for coord, tools, api, notify, window, identity, inbox and invite:** 154 targets OK, 1316 passed, 6 ignored, 0 failed.
- **`cargo test -p irori-core`:** the last full run was 37 of 38 targets OK (282 passed, 2 ignored). The one failure was my own wrong pin edit in `tests/notes.rs`, now reverted. After that, `--test notes` (3 passed) and `--test boards` (27 passed) were rerun and pass. The full core suite was not repeated after that revert.

## build:talk

B1, B2, B3, B5 and B6 are fixed with tests, and `docs/design/talk-channels.md` is amended. B4 is not fixed. `bun test` has one failure that comes from the base commit, not this work. The branch `card-558-talk` is pushed, 6 commits on top of d6714dea, HEAD 5b003c3f. I added no new row kinds, made no `SPACES_PROTOCOL` change, wrote no move or delete code, and did not touch `short.rs`.

Commits:
- a918316e A thread from a space channel lands in the space's log
- b5c13d71 Agent Talk tools reach space threads
- b9a6f1a2 Every thread lists with its space
- 27fbc52b All spaces lists every thread, and a space's card stays in the space
- 2e302b63 Talk's design follows the space
- 5b003c3f The preview's threads carry their space

**Done**
- **B1:** `talk_thread_from` now starts the thread in the channel's space log, keeping `ask.channel`, with no repo.
  - The pointer in the channel is the tagged `short_ref()` (`mlg#N`), so a member's click goes through `cardRef` to the thread.
- **B2:** `talk_list`, `talk_read` and `talk_post` now route across every log.
  - A bare `#N` held by two logs is refused naming both.
  - `space=` picks the log for a bare ref; `mlg#N` and ids work from anywhere.
  - The refusal is tested on the real router in `five_cores`, including that nothing lands on the private card.
- **B3:** new `Window::threads_all` and the `board_threads_all` command.
  - `ThreadSummary.spaceId` is added; Talk filters by the pick with `convInSpace`.
  - A thread outside the pick still opens.
  - Each log keeps its own 20 newest resolved threads, so a busy space can't crowd out a quiet one.
- **B5:** Make a card in a space channel now makes the card in that space, and its chip reaches the members. A private card's chip stays yours.
- **B6:** private threads about a space you left now show under Personal and All spaces.

**Decisions to review**
1. With "My threads" set to private, a thread from a space channel stays in your own log and the channel gets no pointer.
2. `ask.channel` is now kept on space-log asks. The field already exists on the row, and by my reading older peers blank it at fold without stalling; no older device was run. `brief_channels` only trusts a channel from that same space.
3. `TaskRouter::private_key` is added with an identity default. Reads map board keys back without minting an `agent.card`, because `board_coord` with keys writes one. The test confirms that reading adds no card and a first post does.
4. `Core::space_gate` is added. `talk_thread_from` checks it first, so a thread can't start and then have its pointer refused. The gate refusal is tested at Core level; the `talk_thread_from` wiring to it is not exercised by a test.
5. `talk.card` goes in the space log when the card is in that space; if that write is refused it falls back to Personal. This uses an existing kind and the existing fold.
6. `localRef` no longer resolves a bare ref to a thread that has a `spaceId`, so the backend decides.

**Not done**
- **B4** (`cards.rs:2113`, `:2201`, the window tag overwrite): left alone because it is on the spec's Window list and I judged it another agent's. Until it lands, inner-board cards still show the space's tag.
- **Board pick in Make a card:** it uses the space's first board, since an inner-board pick is the `new_card.rs` gap.
- **`specs.json` tool descriptions** still say `a thread (#12)`. I left them unchanged, so the goldens don't move.
- **Old threads:** threads started from space channels before this fix stay in the private log. The doc says so.
- **`spaces.md:35`:** not touched; it depends on B4.
- **Live app:** not run. Bob's side of B1 is tested at Core level through `board_ask`, not through two windows. The window path is tested on a one-person space.

**Merging note:** the pointer relies on `tag` and `short` from the per-board numbering branch, and `bindings.ts` may conflict with other branches. If it does, rerun `cargo xtask bindings`.

**Results**
| Check | Result |
|---|---|
| `cargo test -p irori-coord -p irori-tools -p irori-window -p irori-core` | 1242 passed, 0 failed, 6 ignored |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean; needs `bun run build` first for `app/build.rs` |
| `cargo xtask bindings --check` | current |
| `bun scripts/gen-models.ts --check` | clean |
| `bun run typecheck` | clean |
| `bun run lint` | clean |
| `bun run build` | ok |
| `bun test` | 547 pass, 1 fail |

- **The `bun test` failure:** `SyntaxError: Export named 'cardsOnBoard' not found in module .../frontend/src/components/boards/board-page.tsx`. `space-boards.test.ts` imports `cardsOnBoard` from `./board-page`, which only imports it from `./space-boards`. Neither file was changed on this branch, so the failure comes from the base commit.
- **After the last commit:** the Cargo table above was run before 5b003c3f, a one-line frontend sample change. Typecheck, lint and `bun test` were rerun after it; `bun run build` and `gen-models --check` were not.

Key files, under /Users/stephenhorton/projects/irori/.claude/worktrees/wf_0e89c9f2-111-2/:
- crates/window/src/talk.rs
- crates/window/src/threads.rs
- crates/tools/src/talk.rs
- crates/coord/src/talk.rs
- crates/coord/src/board/asks.rs
- crates/core/src/service/spaces.rs
- crates/core/src/service/spawn.rs
- crates/core/tests/five_cores.rs
- crates/window/tests/boards.rs
- frontend/src/App.tsx
- frontend/src/components/talk/talk-page.tsx
- frontend/src/components/talk/conversation.tsx
- frontend/src/lib/cards.ts
- docs/design/talk-channels.md

## build:personal+agents

Part 2 is done and pushed to `origin/card-558-core` (head `e6813db1`), but `scripts/check.sh` will still fail: `bun test` has 1 failure from the WIP mock-up, not from this work.

**Branch**
- `card-558-core` was not on origin, so `git fetch origin card-558-core` failed. It existed only locally, checked out in worktree `wf_0e89c9f2-111-1`, so I could not check it out here.
- I worked on a new local branch, `card-558-core-part2`, started from `card-558-core`, and pushed with `card-558-core-part2:card-558-core`. That push also put part 1's 3 commits on origin for the first time.
- The local `card-558-core` ref is still at part 1 (`9a8360c4`) and is behind origin, so whoever continues should fetch from origin.
- The first push (`7460dab3`, `836e7cff`) went out after targeted tests only. The full gate ran before the second push.

**Commits on top of part 1**
- `7460dab3` Personal is a space with boards of its own
- `836e7cff` The window lists, adds and renames Personal's boards
- `be6ec32d` Refusals and the fence say space for the group of people
- `a5f9b5b3` The board tool shows spaces with their boards
- `e6813db1` The last refusals and a note's scope say space

**1. Personal**
- **Boards in the private log:** `space.board.*` rows go there with no gate, written as `user`. The sentinel `PERSONAL_BOARD = "personal"` lets the default board be renamed.
- **Refs and numbering:** the private coord now has `board_tag` and `board_tags`, and `task_ref` resolves `home#3`. Numbering is per board, and subtasks stay on their parent's board.
- **Tags:** Personal and the spaces share one tag set, checked both ways.
- **Existing cards:** they stay on the default board as `#N`.
- **Window:** new commands `boards_personal`, `boards_add_to_personal` and `boards_rename_in_personal`. New card takes `spaceBoard` off a space. Private cards now carry their tag and `spaceBoard`, and `board` stays empty so Home keeps them. Bindings are regenerated.
- **Live updates:** a board row arriving from another of the person's devices refreshes the list.

**2. Agents**
- **Board tool:** it prints each space once, with people and autonomy, and its boards underneath. `BoardInfo` gained `space_name`. Labels now name the space an inner board is in, and hidden spaces are filtered out.
- **Wording:** specs, tool results, bridge messages, the fence and coord refusals now say space. "the Board" is unchanged. I skipped the lines `card-558-words` already changed.
- **Goldens:** all three are hand-edited (`tools/specs.json`, `mcp/wire.json`, `api/go.json`), and `crates/tools/src/specs.json` is updated to match.

**Test results**

| Check | Result |
|---|---|
| `cargo test --workspace` (final tree) | 2365 passed, 0 failed, 11 ignored |
| `cargo fmt --all --check` | ok |
| `cargo clippy --workspace --all-targets -- -D warnings` | ok |
| `cargo xtask bindings --check` | current |
| frontend `gen-models --check`, build, typecheck, eslint | ok |
| `bun test` | 545 pass, 1 fail, 1 error |

- **Golden and size tests:** `test_specs`, `specs_and_instructions_match_go`, `tool_texts_match_go`, `budget_texts_match_go` and `wire_matches_go` all pass.
- **New tests:** 10 in `crates/core/tests/personal_boards.rs`, plus `personal_boards_from_the_window` and `the_board_tool_shows_spaces_with_their_boards`.
- **Not run:** the remaining `check.sh` steps (the single-threaded capture rerun, `no-third-party.sh`, the Android build).

**Needs attention**
1. **Deliberate departure from spaces-boards.md §2:** in the private log only, a bare `#N` means the default board first instead of being ambiguous. A default-board card's whole ref is `#N`, so `home#3` would otherwise leave private card `#3` unreachable by short ref. Off the default board, the ambiguous rule still applies. Stephen should confirm this.
2. **`bun test` failure is not from this work:** `space-boards.test.ts` imports `cardsOnBoard`, which WIP commit `05615f48` removed from `board-page.tsx`. I only changed generated frontend files, and I left this for the frontend owner. It will fail `check.sh` until fixed.
3. **`bridge2` failed once and I could not reproduce it:** `a_board_ask_reaches_the_other_persons_free_agents` got 2 deliveries instead of 1 in the first workspace run. It then passed 30 of 30 runs idle, 30 of 30 under CPU load, 8 of 8 whole-binary runs and the final workspace run. It looks like a timing race on row order, but that is unproven and it is not fixed.

**Left as they are**
- **Tools and Personal:** Personal boards are not listed in the tools and agents can't post to them (`board_create` assumes a space id, and personal.md item 11 says to keep them out of `boards()`). Refs like `home#3` still resolve for agents.
- **`board_share` to an inner board:** it still lands on the space's first board. This is agent-text.md bug (a), and the printed label names the inner board, so agents are told the wrong place.
- **Late board rows:** a card row that arrives before its `space.board.created` keeps an empty tag until a refold. Spaces already had this gap, and Personal now has it too.
- **Person-facing text:** "your own board" lines in `crates/window` are unchanged.

I fixed one extra small thing: the ambiguous-tag refusal now offers the inner board's id, which is agent-text.md bug (b).

## review:2

**Review of #558 on `card-558-core`, `card-558-talk` and `card-558-words`, lens: refs and names. 7 defects, all found by reading the code; nothing was run.**

Line numbers are from each branch's tree. `core` is `card-558-core`, `talk` is `card-558-talk`.

**1. High: the window never uses the new board features, so spec items stay broken for the person (core, talk, words).**
- **New card on an inner board:** `frontend/src/App.tsx:993-1004` opens `NewCardDialog` with only `board`, and `lib/cards.ts:1799` `create` sends only `...input`.
  - Outside samples and generated files, nothing in the frontend sets `spaceBoard`.
  - Scenario: open the `net` list (`#board/<space>/<board>`) and press New card. The card lands as `wind#N` on the first board and does not appear in the list you are on. This is the §5 bug at `new_card.rs:131`, and only the backend half is fixed.
- **Rename:** `boards_rename_in_space` has no caller on any of the three branches.
  - The only rename is `components/boards/board-page.tsx:211`, `actions.rename(b.boardId, name)`, which goes to `boards_rename`. That is owner-only and renames the space.
  - So §4 / b4 is unreachable: a member can't rename a board, and neither an inner board nor a first board can be renamed apart from its space.
- **Personal's boards:** `boards_personal`, `boards_add_to_personal` and `boards_rename_in_personal` also have no caller. Personal's boards can't be seen, added or renamed in the window.

**2. High once Personal boards are in use: a printed bare `#N` silently opens a different card (core).**
- `crates/notify/src/watch.rs:756,836,1016,1046` build titles with `(#{t.short})`, dropping the tag. Line 1016 is commented "what the person says to an agent about it".
- `short_hits` (`short.rs:253`) makes a private bare `#3` mean the default board first.
- Scenario: an agent needs you on `home#3`, and the note says `(#3)`. You tell the agent "#3", and it acts on default-board card `#3` with no refusal.
- This is the §2 departure builder 1 asked Stephen to confirm. It makes every untagged printer below a wrong-card risk rather than just imprecise.

**3. Medium: other printers drop the tag, so a `#N` copied from them is refused or means another card now numbers are per board (core, talk).**
- `frontend/src/components/notifications-page.tsx:203`; `NotificationCard` (`crates/window/src/notifications.rs:105`) has no tag at all.
- `cards/card-detail.tsx:855,870,981`
- `cards/card-grid.tsx:444,950,952`
- `cards/agent-hover.tsx:186`
- `cards/agent-strip.tsx:132,137,146`
- `cards/threads-panel.tsx:162`
- `talk/threads.tsx:123`
- `talk/conversation.tsx:578`: the "Made a card" chip, now reachable for space cards through B5.
- `talk/talk-page.tsx:73,993`: `ThreadSummary` has no tag, so the channel pointer says `mlg#5` while the list says `#5`. Searching Talk for `mlg#5` finds nothing.
- Scenario: a subtask of `net#3` shows "part of #3". If `wind#3` exists, `#3` is refused as ambiguous.

**4. Medium: `board_of` names the wrong board for an inner-board card (core `crates/core/src/service/bridge.rs:754`).**
- `tools_board(s)` always returns the first board's name and tag, and `board_of` ignores the card's `t.board`.
- Core newly set `name` to `boards[0].name`, so `label()` now states a specific board. Before, it only named the space.
- Scenario: first board renamed to "Tasks" in space Wind, card `net#3`. `shared.rs:515` says "that card is already on Tasks (wind) in Wind". The same wrong label reaches the other `label()` sites fed by `board_of`, for example `shared.rs:191,350`.
- This is separate from the known `board_share` bug.

**5. Medium once Personal boards are in use: a private card on a Personal board loses "Start an agent with this" (core `frontend/src/components/cards/card-detail.tsx:121`).**
- `startable` returns false on `d.tag`, using a tag to mean shared.
- `card_summary` now puts `t.tag` on private cards (`crates/window/src/cards.rs:1492`), and `CardDetail` flattens it.
- So `home#3` hides the action even though it is the person's own open card with nobody on it.
- `onBoard` alone already covers shared cards.

**6. Low: the fold's bare-ref readers don't follow the Personal rule (core).**
- `mention_cards` (`crates/coord/src/board/fold.rs:1500`) and `thread_hearers` (`threads.rs:92`) match `t.short == n` over all boards and drop the ref when two match. `task_ref` and the window chip use the default-first rule instead.
- Scenario 1: with `#3` and `home#3`, a thread saying "#3" shows a chip for default `#3`, but its holders don't hear it and the card gets no "mentioned this" line.
- Scenario 2: `t.id != from` removes the commented card from the matches first. A comment on `#3` saying "#3" therefore writes a mention on `home#3`. The same happens with `wind#3` and `net#3`.

**7. Low: deleted default-board cards break the rule (core `short.rs:253,276-282`).**
- A deleted card is gone from `tasks`, so `short_hits` no longer sees a default-board hit.
- Scenario 1: delete default `#3`, then a stale `#3` reaches `home#3` instead of "was deleted".
- Scenario 2: the `gone` check ignores the tag, so a nonexistent `home#5` reports "home#5 was deleted" when default `#5` was deleted.

**Checked, no defect**
- **Refs and numbering:**
  - `next_short`, including asks on `""`, the refold and the `short_tops` choice.
  - `has_tag` on emptied boards.
  - Tagged and cross-log `task_ref`.
  - `localRef` with Personal tags.
  - `card_named` with channel names, `task:` ids and `space=`.
- **Names and first-board rename:**
  - First-board name folding, `boardsOf` fallback and `space_name` on inner entries.
  - `find_board`.
- **Branch names:** tags go into `card-<tag><n>`, so same-numbered cards don't collide. Pre-existing and not counted: `names_any_card` misses tagged names like `card-net3`, so that worktree isn't flagged as another card's.
- **Hidden spaces:** `is_hidden_space` is still a stub returning false, so nothing hidden is live. Worth noting for #538: asks write `task.created` directly (`asks.rs:821`), which skips the `no_cards` check in `create_locked`.
- **Words branch:** no ref or name logic changed, and no code matches the changed strings.

## review:3

**Review of card-558-core, -talk and -words: Talk and agents lens.** One high finding, three low. I read the diffs and the merged code; I did not build or run anything. All three branches merge pairwise with no conflicts. Applying the words patch on top of core+talk also left no rejects.

**1. HIGH: `talk_post` into a space thread skips the secret and path refusal** (card-558-talk)
- **Where:** `crates/tools/src/talk.rs:1168` calls `crate::board::board_comment` directly.
- **Why it happens:**
  - The shared-space guard is `around_board` in `crates/tools/src/shared.rs`. It only runs at dispatch, keyed on the tool name (`crates/tools/src/lib.rs:472`).
  - `talk_post` is not in `CARD_WRITES`, so the guard never fires. `board_comment` has no check of its own, and coord doesn't scrub comments.
  - The other direct route, `comment_on_card` (used by reply), does check `reasons_in`.
- **Why it is new here:**
  - The old `resolve` used `b.coord()`, so `Target::Thread` could only be a private card.
  - `card_named` now routes across every log. This covers any card in a space, not just threads, because `conv` accepts any ref.
- **Failure:**
  - An agent runs `talk_post conv=mlg#12 text="key sk-ant-... at /Users/x/.ssh/id"`. The text goes into the space log and replicates to every member.
  - `board_comment task_id=mlg#12` with the same text returns "nothing was written".
  - The agent also never gets the "Visible to:" line, so it isn't told who saw it.
- **Tests:** none cover this. `tools/tests/shared.rs` has no `talk_post`, and the `five_cores` `talk_post` calls don't use secret text.
- **Fix:** run the same check in the `Target::Thread` branch, or call through `around_board("board_comment", ...)`.
- **Related, not a regression:** `tools::talk::post` has no such check for space channels either, so that gap is already on the base.

**2. LOW: Talk shows a space thread's ref without its tag** (card-558-talk)
- **Where:**
  - `ThreadSummary` has no tag (`crates/window/src/threads.rs:445`, and `:460` for its made cards).
  - `talk-page.tsx:996` renders `#{t.short}`.
  - `talk-page.tsx:73` searches only `#N`.
  - `conversation.tsx:582`, the "Made a card" chip, ignores `c.tag` although the card has it.
- **Why it is new:** All spaces now mixes every log's threads, where it used to list private ones only.
- **Failure:**
  - Private thread #3 and `mlg#3` both show as `#3` in the open thread's header.
  - The channel pointer says `mlg#3`, but searching Talk for `mlg#3` finds nothing.
  - If the person repeats `#3` to an agent, it is refused as ambiguous.
- **Same pattern:** `notifications-page.tsx:203` also drops the tag, which matters now that boards number separately.

**3. LOW: agent-read refusals still say board for a space** (card-558-core and -words)
- **Where:** `crates/core/src/service/notes.rs:83` and `:90`. Words left them on purpose and core didn't take them.
- **Failure:** `notes_propose scope=xyz` returns `no board "xyz": give mine, or a board's tag or id`, while the tool's spec and `tools/src/notes.rs` now say space.
- **Same kind:** `crates/coord/src/board/mod.rs:1709` (`:1701` on the words base) still says "only its creator or a board owner withdraws". A person or agent can read it, and it is the only line on words' "Not done" list that core didn't fix. I found no pin on the notes strings, and nothing suggests the `mod.rs` one is pinned, unlike the `:1713` default. `:1713` stays as reported, because the golden pins it.

**4. LOW: the `space` argument's spec doesn't match what it does** (card-558-talk)
- **Where:** `crates/tools/src/specs.json` entries 48, 49, 51 and 52 say `space` is only "the shared space a channel name is in".
- **What the code does:** `card_named` and `talk-channels.md` §7 also use it to pick the log for a bare `#12`, and the ambiguous refusal doesn't mention it. The builder left the specs alone so the goldens wouldn't move.
- **Failure:** the agent isn't told about `space=mlg`, but it can recover, because the refusal lists each `task:<id>`.

**Checked, no defect**
- **Move/delete wording:** nothing about moving cards between boards or deleting boards was added. Every hit is about cards, people, files or code identifiers.
- **Words' "Not done" coord list:** core covers everything on it except the `mod.rs` line in finding 3.
- **Threads reaching members:**
  - The pointer `mlg#N` resolves through `card_ref`, then `allThreads`.
  - `talk.card` is on the space allow-list and folds, and it falls back to private when refused.
  - `threads_all` has no duplicates, because inner boards share one log.
  - `thread_logs` skips inner boards.
  - A space id or an inner tag both resolve to the space in `find_board` and `space_arg`.
  - `brief_channels` filters by log correctly.

Scratch copies are in `/tmp/rv558` (core+talk+words) and `/tmp/rv558w` (words). Line numbers above are from the branches, except where noted.

## review:1

**Review of #558 for convergence and existing data: 4 defects, none blocking, no stall path found**

I read `core` at e6813db1, `talk` at 5b003c3f and `words` at 03438211. I ran no code or tests, so every scenario below is traced from the source. Line numbers are for each branch's tree. I did not review a merge of the three branches.

**Defects**

1. **Medium: the same card shows different refs on different devices until a restart** (`card-558-core`). The builder listed this under "Late board rows", but it is a real divergence and per-board numbering makes it worse.
   - **Cause:**
     - `crates/coord/src/board/fold.rs:1156` stamps `t.tag` once, by reading the store when the `task.created` row folds.
     - No other coord code sets a card's tag, and `space.board.*` isn't in `FOLLOW_KINDS` (`fold.rs:111`).
     - The Core only calls `boards.moved()` when those rows land (`boards_own.rs:305`, `boards.rs:432`).
     - Numbering itself converges, because `ShortClaim.board` is the raw id.
   - **Personal scenario:**
     - Device A adds board `home`, and B, which already has that row, makes a card on it as #3.
     - C receives B's rows in an earlier batch than A's.
     - C folds the card with tag `""`, so its ref is `#3`, the same as default-board `#3`, and the window puts it on the default board (`crates/window/src/cards.rs:1493`).
     - `home#3` is NotFound on C: `has_tag` passes, then `short_hits` filters on `t.tag` (`short.rs:251`). A resolves it.
   - **Space scenario:**
     - Bob's `net` card #3 folds on Carol's device before Alice's `space.board.created`, so it stamps `wind`.
     - Carol then has two `wind#3` cards. `wind#3` is refused as ambiguous there, and `net#3` is NotFound.
     - Under the old single run the number could not repeat on a second board.
   - **Lasts until:** a replay (restart or cut refold).
   - **Fix:** when a `space.board.*` row lands, restamp cards whose `t.board` matches, or refold.

2. **Low-medium: a shared post's card chip can differ between members** (`card-558-talk`).
   - **Cause:** `crates/coord/src/talk.rs:1093` does an unconditional `s.cards.insert`, so the last row to arrive wins. That was tolerable while `talk.card` stayed on one person's devices. `link_card_in` now writes it to the shared space log.
   - **Scenario:**
     - Alice and Bob each use Make a card on one channel post before hearing from each other. The button only hides once `post.card` is set (`conversation.tsx:404`).
     - Alice ends up showing Bob's card and Bob shows Alice's.
     - After a restart each space replays in HLC order, so the winner can change.
     - A private `talk.card` also loses to any space row on replay, because space rows fold after it.
   - **Fix:** keep the link by row order (first or last by `RowPos`) rather than by arrival.

3. **Low: a deleted private card's number can silently resolve to another card** (`card-558-core`).
   - **Cause:** `short_hits` (`short.rs:244-256`) drops deleted cards before the default-first rule, and `task_ref_locked` only checks `gone` after an empty result (`short.rs:273`).
   - **Scenario:** default `#3` is deleted and `home#3` is live. A bare `#3` now returns `home#3`, so an agent's comment or status lands on the wrong card. Before this change it was refused with "#3 was deleted".
   - **Next step:** Stephen is already being asked to confirm this private default-first rule, so settle this case with that decision. Checking for a deleted match on the default board before the hits would fix it.

4. **Low: old lettered refs stop resolving when the two claimants are on different boards** (`card-558-core`).
   - **Cause:** the `(board, number)` key in `claim_short` and `settle` (`short.rs:144`, `:167`), with exact matching on lettered refs.
   - **Scenario:** a pre-#558 space where a first-board card and an inner-board card both claimed 5 while apart. They showed `#5a` and `#5b`, and they now fold to `wind#5` and `net#5`. `#5a` in old comments or agent memory gives a bare NotFound.
   - **Related:** this is the one case I found of a pre-#558 ref being lost. No card is lost. The same happens when a device on an older Irori posts `#1a` during a mixed-version period. That makes §2's "Cosmetic" claim slightly too strong.

**Checked, no defect**

- **Stalls:** I found no row that can stall a `spaces: 1` device.
  - No branch adds a kind or changes an Accepts/Refuses verdict; the `kinds.rs` diff is comment only, and `talk` and `words` don't touch `store`, `mesh` or `identity`.
  - `talk.card` was already `Accepts` and in `SPACE_TALK_KINDS`, and it goes through the gate in `append_space_talk`.
  - `space.board.renamed` always serializes `name` and `tag`, has `by == actor`, and neither the verifier nor the old fold has a holder check. A spaces:1 device ignores a first-board rename, which is harmless.
  - `ask.channel` is nested in the row. `check_shape` and `bind_actor` never read it, and base `asks.rs` blanks it at fold.
  - In the private log, `space.board.*` rows are stored and folded past.
- **Numbering:** it converges. The key uses the raw board id, `settle` orders by `RowPos`, and legacy `num == 0` rows land on `("", n)`.
- **Board row order:** `space_board_rows` reads in `(hlc, device_id, seq)` order, so concurrent renames and duplicate creates converge.
- **Pre-#558 private data:** every existing private card has `board == ""`, because `create_task` could not set it and `ask.board` is a separate field. All `#N` refs are unchanged.
- **`short_tops` not cleared on refold:** this matches the old `short_top`. The only effect is a gap in the numbers.
- **`is_hidden_space` stub:** `is_dm_space` isn't on the base or on main, so this is deferred work, not a defect.
- **`withdrawn from the space`:** nothing matches on that note text, so the two defaults differ only in wording.