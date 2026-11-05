TALK AND THREADS ON #558: REPORT
Read only; nothing changed. All findings come from reading the code at 23093055. No tests or app were run.

VERDICT
- Channels are space-wide. No bug.
- Threads are never tied to an inner board. A space thread lists across the whole space log.
- Card threads are keyed by task id, so an inner board makes no difference to them.
- There are still real gaps. Threads started from a space channel land in the wrong log (B1). Agent Talk tools can't see space threads (B2). All spaces hides space threads (B3). Card tags are wrong for inner-board cards, which breaks refs in Talk (B4).
- No Talk view filters by inner-board id anywhere, because the frontend never receives one.

WHAT IS CORRECT

Channels
- `spaceId` is forced to the log's id on write (crates/core/src/service/spaces.rs:397-401) and re-stamped on every folded row (spaces.rs:327-329).
- The row is written in crates/coord/src/talk.rs:510-548.
- A channel never moves space (coord/talk.rs:930). Rows from one space can't touch another's channels (coord/talk.rs:895-900).
- Name uniqueness is per space (coord/talk.rs:766-771).
- `talk_channel_create_in` accepts only a space log id: inner ids fail with "no space" (crates/core/src/service/talk.rs:99).
- The frontend only passes `threadsSpace(space)` (frontend/src/App.tsx:849 into talk-page.tsx:366 and :825).
- `talk_post_bridge` filters `c.space_id == b.id` (crates/core/src/service/bridge.rs:256 onward).
- No talk.* row carries a board id.

convInSpace (frontend/src/lib/spaces.ts:72-76)
- It compares against `conv.spaceId`, which is always the space id. Correct.

cardInSpace (spaces.ts:62-66)
- `c.board === space` is correct for inner-board cards, because `CardSummary.board` is always the space id, never the inner id (crates/window/src/cards.rs:2112, :2200; crates/window/src/boards.rs:414).
- `Task.board` (crates/coord/src/board/types.rs:385) is not exposed to the window at all.

threadsSpace with a shared pick
- `Window::threads(space)` lists every task in the space log, whatever its board (crates/window/src/threads.rs:338-345), plus your private threads about that space (:333-337).
- Thread tasks carry no inner board. `AskField.board` is only ever the space id for private threads (crates/coord/src/board/asks.rs:841-845).

threads.set
- Keyed by space id only (crates/inbox/src/bridge.rs:186-191, :339, :668-692; crates/window/src/boards.rs:738). Correct: spaces.md §5.

Card threads
- Comments key on task id. `ThreadView` uses `useCard(t.id)` (frontend/src/components/talk/threads.tsx:148).
- The card page finds its log with `core.board_of`, which routes by task id (crates/core/src/service/boards_own.rs:657-664).
- `detail.onBoard.id` is the space id, so the mention picker works for a card on an inner board (card-detail.tsx:265, mentions.ts:90).
- By design, card comment threads do not appear in Talk at all, on any board (talk-channels.md:21, :26). A card on an inner board surfaces only on its card page. Only `ask.thread` tasks reach the list (crates/window/src/threads.rs:31-33).

Agents hearing a space thread
- `board_ask_agents` uses the whole space roster (crates/core/src/service/boards.rs:2133, :2169). Inner boards don't scope it.

Tools `space=` arg
- crates/tools/src/talk.rs:726-737 calls `find_board` (crates/tools/src/shared.rs:190-215).
- It accepts a space tag or id, or an inner-board tag or id, and resolves all to the space id via `..own.clone()` (crates/core/src/service/bridge.rs:732-740). `space=net` quietly means the parent space. Harmless, but document it.

REAL BUGS

B1. A thread from a space channel's message goes to the private log.
- crates/window/src/talk.rs:231 calls `start_thread_in(text, &[], &[], "", None, conv)`. `board_id` is hard-coded to "" whatever `channel.space_id` is.
- The thread is written to your own log and heard only by your agents. Other space members never get it.
- The pointer `#N` is then posted into the space channel (talk.rs:232-237). `N` is a private-log number, so for other members it dangles, or names a different card #N.
- Under the shared pick it isn't listed either: `about(t)` is "" and doesn't equal the space id (crates/window/src/threads.rs:334). It shows only under Personal or All spaces.
- This breaks talk-channels.md b2 (line 12) and §2 "Belongs to a space" (line 21).
- Passing the space id alone won't fix it:
  - The shared branch drops `channel` and `repo_key` (crates/window/src/threads.rs:211-224).
  - coord blanks `channel` on a board log (asks.rs:846-850; types.rs:1047-1049 says "private log only").
  - `board_ask` refuses a repo (boards.rs:2174-2179).
  - The channel's lines in a spawned agent's brief depend on this field.
  - The private `threads.set` mode must be honoured too; `private_board_thread` (crates/window/src/threads.rs:256) doesn't take a channel.
- Test coverage is private channels only (crates/window/tests/services.rs:1073, :1103, :1130; crates/core/tests/ask_spawn.rs:252).

B2. Agent Talk tools read only the private coord, so space threads don't exist for them.
- crates/tools/src/talk.rs:508 (`conversations`), :583-592 (`resolve`) and :765-767 (`posts`) all use `b.coord()`. That is the private log (crates/core/src/service.rs:386).
- `talk_list` omits every space thread, even ones the agent answered.
- `talk_read conv=<space thread id>` and `talk_post` to one fail with "is no channel, thread or agent here" (talk.rs resolve, then :1173 and :1081).
- Worse, a bare `#12` resolves to the private log's #12 if that exists, so the agent reads or posts on the wrong card (talk.rs:583-587). `talk_post` then comments on that wrong card (:1085-1092).
- The `reading` notice tells agents to run exactly this call: "talk_read conv={t.id} gives the whole thread" (crates/coord/src/board/threads.rs:377). `thread_reading` (threads.rs:196, :299) is coord code that isn't restricted to the private log. I read that from the code and did not exercise it on a space log.
- The fix should use the router that `Inner::on` already uses (crates/tools/src/board.rs:70-79), and the same in `posts`.
- Tests are channels only (crates/tools/tests/talk.rs:349-490).

B3. Space threads vanish under All spaces, while channels don't.
- `threadsSpace(AllSpaces)` is "" (frontend/src/lib/spaces.ts:91-93), and `useThreads` then returns private-log threads only (App.tsx:272; crates/window/src/threads.rs:333-337).
- Under All spaces, channels from every space show (spaces.ts:73), but every shared thread is missing. So are your private threads about a space (`about != ""`, threads.rs:334).
- Also affected:
  - Talk's badge (App.tsx:716-720) undercounts.
  - Talk search can't find them (talk-page.tsx:136).
  - The Resolved list omits them.
- This contradicts the file's own header, "All spaces ... shows everything" (spaces.ts:5-7). It does match the #393 note "what the window showed before" (talk-channels.md:50), so this is an inherited choice, but it fails #558 item 4.
- Opening one by #ref under a pick that excludes it:
  - `openThread` falls back to the card page (App.tsx:530-533).
  - `go({kind:"talk", id})` from a notification (App.tsx:429) or `?thread=` (App.tsx:209) finds neither `current` nor `conv`, so it shows the empty "Start a thread" pane (talk-page.tsx:141-146, :450). The comment at talk-page.tsx:142 only promises this for channels.
- `ThreadSummary` has no space id (crates/window/src/threads.rs:38-79). `TalkConversation.space_id` is documented absent for threads (crates/window/src/talk.rs:68). A fix needs an all-logs `threads` call plus a `spaceId` on each thread, then `convInSpace` over it, the same as channels.

B4. A card on an inner board shows the wrong tag, which breaks refs in Talk.
- coord folds the right tag: `tag: self.tag_of(&p.board)` (crates/coord/src/board/fold.rs:1143, :1992-1996).
- The window overwrites it with the space's tag: crates/window/src/cards.rs:2113, :2201; crates/window/src/boards.rs:415. `s` is one entry per space (crates/core/src/service/boards_own.rs:511-514).
- So a card coord knows as `net#5` reaches the frontend as `wind#5`. Consequences:
  - The # picker in the Talk and thread boxes inserts `wind#5` (frontend/src/components/common/mention-textarea.tsx:132; also frontend/src/lib/mentions.ts:68).
  - `thread_hearers` then skips that card's holders because `t.tag != wind` (crates/coord/src/board/threads.rs:93), so the people who should hear the thread don't.
  - `mention_cards` skips it too (fold.rs:1488), so no cross-reference line lands on the card.
  - `task_ref_locked` refuses it as "a card on another board" (crates/coord/src/board/short.rs:215-217), so the ref is dead in `card_ref` (crates/window/src/threads.rs:121).
  - Correctly typed `net#5` fails `localRef` in the frontend (mentions.ts:543) and `cards.ts:559`, because `x.tag` is `wind`.
  - `cardsOnBoard(b, cards, "net")` always returns nothing in the real app, and the "wind" chip shows everything (frontend/src/components/boards/board-page.tsx:433-436). The test passes only because it invents tags (frontend/src/components/boards/space-boards.test.ts:18-25).
  - spaces.md:35 "A card shows its board's tag (net#5)" is false in the window.
- The fix is to stop overwriting when `t.board` is non-empty, and to expose the inner board id on `CardSummary` (cards.rs:217). s3 needs that anyway.

B5. "Make a card" from a space channel always makes a private card.
- `NewCardDialog` gets no `board` (frontend/src/components/talk/conversation.tsx:308-315), so `input.board` is undefined (new-card-dialog.tsx:120) and the card goes to your own log.
- The `talk.card` link is deliberately private (crates/coord/src/talk.rs:596-622), so other members see no chip.
- For a space channel this should default to the space and offer a board pick.
- `create_on_board` also can't take an inner board, since `NewCardInput.board` is the space id and `NewTask.board` isn't set (crates/window/src/new_card.rs:25, :115 onward). That is the s3 gap.

B6. Private threads about a space are orphaned after leaving it.
- `threads(space)` returns empty when `board_coord` fails (crates/window/src/threads.rs:339-341), and that discards the private ones already found at :333-337.
- Personal never lists them because `about != ""` (:334), and the pick falls back through `known` (spaces.ts:55-58). They stay in your log but no list shows them. Relevant to "Leave the space".

Limits, not bugs:
- Files on a space thread are refused, and the error says "own board" and "shared board" (crates/window/src/threads.rs:205-209). The wording needs fixing under s4.
- Merge desks list only under All or Personal (frontend/src/lib/desk.ts:338-346), by design.

TRAPS FOR THE BUILD (s3)
- `inScope` (frontend/src/components/cards/board-view.tsx:55), `matches` "board:" (card-grid.tsx:119), `boards-page.tsx:175` and `useThreads(boardId)` (board-page.tsx:78) all assume a board id is a space id. An inner-board route would silently show nothing.
  - `threads(innerId)` swallows the error and returns an empty list (crates/window/src/threads.rs:339).
  - `talk_channel_create_in(innerId)` errors (crates/core/src/service/talk.rs:99).
  - Always pass the space id to Talk.
- The board page's Threads panel (board-page.tsx:78, :136) is space-wide. On a per-board page it would misleadingly repeat on every list, so move it to the space page or Talk.
- Delete board: `tag_of` falls back to the space tag (fold.rs:1996), so old `net#5` text in threads stops resolving (short.rs:215). Tag rename: `t.tag` is set at fold time and old refs die the same way.
  - Recommend that rename changes the name only and the tag stays, or that old tags keep resolving.
- The first board can't be fully "deleted": spaceId is its id, and every channel, `threads.set` key and Talk row hangs on that id.
- `spacesOf` (frontend/src/lib/spaces.ts:29-37) must hide #538's two-person spaces. `fold_space_boards` always returns `own` (spaces.rs:85), which conflicts with talk-channels.md:49 "It has no boards". That is the rule to agree with #538.

docs/design/talk-channels.md: WHAT MUST BE AMENDED
- Line 5 and line 14 (b4): "a layer above boards". Restate as: a space is the people, their devices, their boards and all Talk; a board is only a task list; spaceId is the first board's id, kept for storage.
- Line 21 (§2): add that a thread belongs to the space, never an inner board, and that one started from a space channel's message lives in that space's log (needs B1 first). Add that card threads stay on the card and aren't listed in Talk on any board.
- Line 26: same point; say an inner board changes nothing here.
- Line 40 (§4): say where the card from a space channel goes and which board (B5).
- Line 44: "Boards become a field inside a space". Name it: `task.created.board`, a `space.board.created` id, empty for the space's own board. Say no talk.* row names a board.
- Line 49: "DMs show in Talk whichever space is picked" is false. `convInSpace` shows DMs only in All spaces and Personal (spaces.ts:68-76, talk-page.tsx:148). Pick one and correct it. "It has no boards": record the rule agreed with #538.
- Line 50: "All spaces ... is what the window showed before" must change. Either state that it now lists all threads (recommended, B3) or state the limit plainly.
- Line 54: "all in the space log" is wrong. `talk.read` and `talk.card` stay private (spaces.rs:279-286 lists `TALK_CARD`, but `link_card` writes Personal, coord/talk.rs:609-620). DMs stay private too.
- Line 57: note `spaceId` is always the space id, never an inner board's, and empty means Personal.
- Lines 71-73 (§7): `talk_post`, `talk_read` and `talk_join` also take `space`, which accepts an inner tag and means its space. `talk_list` must include space threads (B2).
- Line 75: `board_create from=<postId>` isn't what the code does. `from` is a thread's ref or id (types.rs:~642; crates/tools/src/board.rs:871). "Make a card" links the post from the window only.
- Lines 82-91 (§9): steps 4 to 6 need current status. Add #558.

Also, in docs/design/spaces.md: line 35 needs correcting (B4). Line 38 (chips) needs replacing once boards are real lists.

TESTS #558 SHOULD ADD
- A thread from a space channel is in the space log and reaches the other member, and its pointer resolves.
- `talk_list` and `talk_read` on a space thread, with a real router and not the fake backend.
- `CardSummary.tag` is `net` for a card on an inner board, and `cardsOnBoard` passes through the real window.
- A # picker ref to an inner-board card tells its holder.
- A card on an inner board keeps its comments and thread after a move.
- Threads listed under All spaces.