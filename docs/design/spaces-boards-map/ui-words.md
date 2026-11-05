REPORT #558: person-visible "board" strings. Read-only; nothing changed.

Rule used: every element of useBoards()/BoardSummary[] is a space today (spacesOf at lib/spaces.ts:29, boardsInSpace at lib/spaces.ts:81). Text about one of those is SPACE. Only text about b.boards[] items is BOARD.

Surface paths: 
- Rust errors reach the window through errorText(err).
- Invite failures show at parts.tsx:307 (s.error).
- Membership conflicts show to owners at parts.tsx:637.
- b.warnings show at board-page.tsx:549 and boards-page.tsx:237.

No onboarding or welcome surface exists. lib/host.ts has no person-facing board text. app/src has none either (comments only). Paths are under frontend/src unless they start with crates/.

Format: file:line | current | proposed

A. MEANS SPACE

Sidebar, phone, palette, page titles (the page lists spaces today; see NOTE 1)
- components/app-sidebar.tsx:420 | Boards | Spaces
- components/app-sidebar.tsx:425 | Cards on your boards waiting on you | Cards in your spaces waiting on you
- components/app-sidebar.tsx:449 | Pinned boards | Pinned spaces
- components/phone-tabs.tsx:20 | Boards | Spaces
- components/space-switcher.tsx:65 | Pick the space Home, Talk and Boards show | Pick the space Home, Talk and Spaces show (or keep, if Boards stays as the per-space list)
- App.tsx:614 | Boards (palette Go to) | Spaces; keywords at :617 add "boards"
- App.tsx:774 | Boards (page title) | Spaces
- App.tsx:648 | New board | New space; keywords at :650 add "board"
- App.tsx:1065 | Waiting on you on your boards: | Waiting on you in your spaces:
- App.tsx:1073 | a board | a space
- components/command-palette.tsx:149, 194 | Boards (group) | Spaces
- components/command-palette.tsx:151 | others: "board" | "space" (search keyword; keep both)
- components/command-palette.tsx:405 | Jump to a card, board or agent, or run a command… | Jump to a card, space or agent, or run a command…
- components/command-palette.tsx:406 | Search cards, boards, agents and commands | Search cards, spaces, agents and commands
- components/app-header.tsx:88 | New board | New space
- components/app-header.tsx:89 | Make a board for a project, or join one with an invite | Make a space for a group of people, or join one with an invite
- components/app-header.tsx:92 | + New board | + New space
- components/app-header.tsx:102 and App.tsx:640 | Make a card on this board / New card on this board | ...in this space, until the header knows the open board (s3), then board is right

Boards page (components/boards/boards-page.tsx)
- :41 | Boards | Spaces
- :43-45 | A board is a project you share: its cards, the people on it and their agents. Make one per project; anyone can be on as many as they like. Boards are private: only the people you invite see them. | A space is a group of people: its boards, Talk and their agents. Anyone can be in as many as they like. Spaces are private: only the people you invite see them.
- :64-65 | Personal holds no shared boards: your own cards are on Home. Pick All spaces or a shared space in the sidebar to see its boards. | Wording pending the Personal question; the second sentence is already right
- :70 | Your boards | Your spaces
- :79-80 | No boards yet. Make one for a project, then invite people to it; or join one with an invite link someone sent you. | No spaces yet. Make one, then invite people to it; or join one with an invite link someone sent you.
- :83 | + New board | + New space

Space page, header, menu, settings (components/boards/board-page.tsx)
- :96 | This board isn't here any more: you may have left or forgotten it. | This space isn't here any more: ...
- :131-132 | No cards on this board yet. Make one with New card, or share a card to it from its page. | No cards in this space yet. ... (board, once a board is its own list)
- :135 | Nothing on this board waits on you. | Nothing in this space waits on you.
- :152 | Boards (back button) | Spaces
- :173 | Unpin this board / Pin this board | Unpin this space / Pin this space
- :187 | Rename this board | Rename this space
- :199 | Board (fallback h1) | Space
- :229 | Board name (rename input) | Space name
- :311, 312, 326 | Board settings | Space settings
- :359 | Leave the board | Leave the space
- :370 | Coming back needs a new board. | Coming back needs a new space.
- :398 | Board actions (aria and title) | Space actions
- :592 | Only an owner can invite people to this board: ask one of them. | Only an owner can invite people to this space: ask one of them.
- Add, no "board" today: the Invite someone section (:612) can take the brief's "Invite to the space".

New and Join dialog, invite (components/boards/parts.tsx)
- :123 | Board name | Space name
- :124 | New board's name | New space's name
- :135 | New board's tag, the short name in card refs like tag#12 | Tag for its first board, the short name in card refs like tag#12 (see NOTE 2)
- :144 | Another board is tagged ${t} already. | KEEP, tags are per board
- :186 | New board / Join a board | New space / Join a space
- :189 | One board per project. Only the people you invite see it. | A group of people, with its boards and Talk. Only the people you invite see it.
- :194 | New board or join one | New space or join one
- :197 | Make a board | Make a space
- :284 | the board (fallback name) | the space
- :442 | Any free agent of yours reviews other people's PRs on this board... | ...in this space...
- :446, 454, 490 | the whole board | the whole space
- :620 | anything this board accepts | anything this space accepts
- :692 | Nobody else is on this board yet. | Nobody else is in this space yet.

Share and withdraw (components/cards/share-dialog.tsx). All of these pick a space today; there is no inner-board picker. See NOTE 3.
- :84 | Share to board: put a scrubbed copy on one of your boards | Share to a space: put a scrubbed copy in one of your spaces
- :88, 93 | Share to board | Share to a space
- :95 | This is exactly what goes on the board. | This is exactly what goes to the space.
- :102 | Board (select label) | Space
- :211 | Take this card and its subtasks off the board? | Take this card and its subtasks out of the space?
- components/cards/card-detail.tsx:743 | Shared to ... "a board" as | "a space"

Profile (components/profile/profile-page.tsx)
- :220 | share: boards, channels, DMs | share: spaces, channels, DMs
- :233 | Nobody yet. Invite someone to a board and they show here. | Nobody yet. Invite someone to a space and they show here.
- :300 | Only you so far. Invite someone from its board page. | Only you so far. Invite someone from Space settings.

Settings
- components/settings/settings-dialog.tsx:77 | Who is on your boards, and joining someone else's. | Who is in your spaces, and joining someone else's.
- components/settings/settings-dialog.tsx:275 | No boards yet. Make one on the Boards page, then invite people to it. | No spaces yet. Make one on the Spaces page, then invite people to it.
- components/settings/settings-dialog.tsx:278 | People on your boards | People in your spaces
- components/settings/settings-dialog.tsx:471 | Saved. Your other devices and your boards pick it up as they sync. | ...and your spaces pick it up...
- components/settings/settings-dialog.tsx:472 | What people on your boards see you as... | What people in your spaces see you as...
- components/settings/devices-panel.tsx:1112 | Your colour on shared boards | Your colour in shared spaces
- components/settings/devices-panel.tsx:1119 | ${c} on shared boards | ${c} in shared spaces
- components/settings/agent-permissions.tsx:70 | On a shared board, or when a card quotes someone else | In a shared space, or when...
- components/settings/maintenance-pane.tsx:12 | here and on your boards | here and in your spaces

Cards, filters, new card, ask box, notes
- components/cards/card-grid.tsx:242 | Filter cards by board | Filter cards by space (chips are spaces)
- components/cards/card-grid.tsx:250 | Your private cards: on no shared board | Your private cards: in no shared space
- components/cards/card-grid.tsx:264 | Cards on the board ${name} (tag#…) | Cards in the space ${name}
- components/cards/card-grid.tsx:914 | Cards on a shared board can't be deleted here. | Cards in a shared space...
- components/cards/clear-done.tsx:98 | on your boards too | in your spaces too
- components/cards/bits.tsx:433 | Made by someone who is no longer on the board | ...no longer in the space
- components/cards/card-detail.tsx:1450 | Files don't go on a shared board's cards from the app yet | ...on a shared space's cards...
- components/cards/new-card-dialog.tsx:148 | ...it waits there for someone on the board to take it up. | ...for someone in the space to take it up.
- components/cards/ask-box.tsx:66 | This board isn't ready here yet. | This space isn't ready here yet.
- components/cards/ask-box.tsx:71 | Files don't go on a board thread yet | Files don't go on a space thread yet
- components/notes/notes-page.tsx:135 | ...once anyone on the board accepts them | ...anyone in the space...
- components/notes/notes-page.tsx:136 | even when a note is about a shared board | ...a shared space

lib hints and hold reasons
- lib/boards.ts:78 | New cards on the board ask you first | New cards in the space ask you first
- lib/boards.ts:79 | They create cards on the board | They create cards in the space
- lib/boards.ts:85 | ...see your threads on this board. | ...in this space.
- lib/talk.ts:90, 92 | your receive setting for that board; on a shared board | that space; in a shared space
- lib/talk.ts:96 | no longer on that board | no longer in that space
- lib/cards.ts:489 | files go with a thread on your own board only; on a shared board, attach them on a card | see section D

Rust text in the window
- crates/window/src/cards.rs:1211, 1271 | On the board (person's where line) | In the space
- crates/window/src/new_card.rs:45 | files on a shared board's new card | a shared space's new card
- crates/window/src/comment_edit.rs:40 and crates/coord/src/board/comment_edit.rs:26 | a comment on a shared board can't be edited | in a shared space
- crates/window/src/board.rs:275, 378, 431 | shared board clauses | shared space (the own-board half of :275 is section D)
- crates/window/src/threads.rs:276 | your threads on this board... in the board's settings | ...in this space... in Space settings
- crates/window/src/boards.rs:286 and crates/core/src/service/boards.rs:80 | core: no such board on this device | no such space on this device
- crates/core/src/service/boards_status.rs:117 | You're no longer on this board: an owner removed you. | You're no longer in this space: ...
- crates/core/src/service/boards_status.rs:124 | this board | this space

Rust errors, invite
- crates/invite/src/flow.rs:170 | another of your boards | another of your spaces
- crates/invite/src/flow.rs:175 | A row of this board is too large to send. | ...this space...
- crates/invite/src/flow.rs:283, 558 | This device doesn't hold that board. / already holds that board. | ...that space.
- crates/invite/src/flow.rs:286 | Only an owner of the board can invite. | Only an owner of the space can invite.
- crates/invite/src/flow.rs:439 | That invite is for another board. | ...another space.
- crates/invite/src/bundle.rs:121, 124, 127, 131, 134, 145, 169, 177, 180, 185, 188, 196, 201, 206, 228 | all "board" | "space" (they reach the window via .map_err(fail) at flow.rs:498, 692)

Rust errors, identity and store
- crates/identity/src/membership.rs:538, 557, 564, 566, 629 | the board was created / renames the board / not a board name / not on the board | space
- crates/identity/src/board.rs:729 | is already pinned on this board | this space
- crates/store/src/error.rs:69-71 | board log / this board | internal; low priority, change only if seen

Rust errors, core
- crates/core/src/service/boards.rs:980, 988, 996 (create_board) | a board name... / a board tag... / already board {other}'s | a space name; the tag lines stay board
- crates/core/src/service/boards.rs:1099, 1109, 1116 (rename_board) | a board name / only an owner renames board {id} / the board's newest | space (:1109 prints an id, reword)
- crates/core/src/service/boards.rs:1743 | is not a person on this board | in this space
- crates/core/src/service/boards.rs:2177 | a board ask goes to every member's free agents | a space ask
- crates/core/src/service/boards.rs:2505 | nobody on this board takes offered cards | in this space
- crates/core/src/service/boards_members.rs:45, 56, 101, 107, 163, 214 | only an owner ... on board {id}; leave the board instead; isn't on board; this board's only owner | space (:45, :107, :163 print ids, reword)
- crates/core/src/service/boards_own.rs:841-842 | the card was shared to a board... a board | a space
- crates/core/src/service/boards_own.rs:853 | already on a board | already in a space
- crates/core/src/service/boards_own.rs:958 | board {} isn't ready on this device yet | space
- crates/core/src/service/boards_own.rs:1193 | a board owner withdraws; stays on the board | space owner; stays in the space
- crates/core/src/service/boards_own.rs:1217 | only a card on a shared board changes owner | in a shared space
- crates/core/src/service/boards_own.rs:1227 | withdrawn from the board | withdrawn from the space (see NOTE 4)
- crates/core/src/service/mirror.rs:113 | no shared board holds a lease | no shared space
- crates/core/src/service/notes.rs:47, 483 | every device on the board / a person on the board | in the space
- crates/core/src/service/spawn_permissions.rs:96 | bypass is never used on a shared board | in a shared space

Rust errors, coord (shared with agents, so these also move goldens)
- crates/coord/src/board/owner.rs:128, 149
- crates/coord/src/board/review.rs:255, 649, 663
- crates/coord/src/board/mirror.rs:89, 120
- crates/coord/src/board/stranded.rs:202
- crates/coord/src/board/comments.rs:373, 419
- crates/coord/src/board/tasks.rs:111, 189, 264
- crates/coord/src/board/subtasks.rs:600, 688
- crates/coord/src/board/mod.rs:468, 1682, 1701, 1705
- crates/coord/src/board/fold.rs:774

All of these change "shared board" or "the board" to the space form.

B. CORRECT: BOARD MEANS TASK LIST (keep)
- components/boards/board-page.tsx:483 Boards in this space; :485 All boards / Every board in this space; :495 Another board for the same people: one invite shares them all; :498 + Board; :508-509, 518 Board name and New board's name/tag; :522 Add board; :546 Another board is also tag#
- components/boards/parts.tsx:144 tag clash
- components/profile/profile-page.tsx:259 Spaces: a group of people that holds many boards and channels; :299 not per board; :359 Boards
- crates/core/src/service/spaces.rs:190, 198, 213 (add to space)
- crates/core/src/service/boards_own.rs:704, 730, 750, 759 (tag lookups)
- crates/core/src/service/notes.rs:83, 90 (tag lookup)
- crates/coord/src/board/tasks.rs:144 no board X in this space
- crates/coord/src/board/short.rs:207, 216
- Syntax, stays as stored: notes tag board:<tag> at components/notes/parts.tsx:222 and lib/notes.ts:193 (see NOTE 5)
- Likely task list but read by the person as one word: components/canvas/switch.tsx:61 Board view, :159 Board canvas. Both are the view over a card list, so keep them.

C. AMBIGUOUS: "the Board" = agents' work board in Irori's general vocabulary (needs a call)
- components/settings/settings-dialog.tsx:71 | Your devices share one inbox and one Board.
- components/cards/card-detail.tsx:686 | It disappears from the Board on all your devices
- lib/cards.ts:1639 (sample); crates/core/src/service.rs:2483, 2489, 2494 | {} is back on the Board: ...
- lib/thread.ts:454 | on the Board (status line for mcp__irori__board* tools)
- lib/conversations.ts:429 (sample) | both are on the Board unheld.
- design/gallery.tsx:272 | Put work on the Board for a free agent.
- crates/notify/src/lib.rs:618 | Open Board (test notification button). It opens tab "board", which is Home/own cards, not a space. Proposed: Open Irori or Open Home.
- crates/coord/src/board/stranded.rs:208, 294 | already on / can't go back on the Board
- crates/coord/src/board/fold.rs:2266 | BACK_NOTE "back on the Board". DO NOT reword casually: fold.rs:2270 matches it with starts_with, and cards.rs:1499 writes it into notes.
- crates/coord/src/board/comments.rs:1655, 1681 and crates/core/src/service/spawn.rs:653 | agent-facing, same vocabulary
- crates/inbox/src/fence.rs:121, 127, 133 and crates/inbox/src/search.rs:189 | "the Irori Board", agent-facing

Recommendation: these mean "Irori's queue of open work", which is neither a space nor a task list. Under "board only ever means a task list" they break the rule. Person-facing ones can become "open work" or "the card list"; the agent-facing proper noun is for whoever holds agent text. Needs Stephen's yes.

D. PENDING THE PERSONAL QUESTION: "own board" = the private log
Wording follows the Personal blocker. If Personal is a space, these become "your Personal space"; otherwise "your private cards".
- components/settings/agents-pane.tsx:53 | On your own board, Irori starts a fresh agent
- components/home/spawn-notice.tsx:22 | each open card on your own board
- components/boards/boards-page.tsx:64 | Personal holds no shared boards
- lib/cards.ts:489 and crates/window/src/threads.rs:207 | files go with a thread on your own board only
- crates/window/src/board.rs:191, 275, 281, 604 | ...on your own board only
- crates/coord/src/board/review.rs:329, 490
- crates/coord/src/board/comments.rs:176
- crates/coord/src/board/desk.rs:380 and desk_person.rs:25 | not a shared board's

NOTES
1. Nav label depends on s2. If the page stays a list of spaces, use "Spaces". If it becomes the current space's boards, "Boards" is right there and the new-space entry point moves. Identifiers (kind:"boards", #boards, nav.boards, board.new, palette keys) stay; none are shown to a person.
2. The create dialog still needs a tag because the first board's tag makes card refs. Rename the name field to the space, and label the tag as the first board's. Separating the names (card item 3) may also need a separate first-board name field.
3. Share, New card and BoardBar target a space with no inner-board choice. Once cards go on a board (s3) these strings may split: "Space" select plus "Board" select. crates/window/src/boards.rs and lib/boards.ts need no text change for that beyond the items above.
4. "withdrawn from the board" (boards_own.rs:1227, coord mod.rs:1705) is written into rows as a note. Old rows keep the old text; only new ones change.
5. notes/parts.tsx:222 and lib/notes.ts:193 show board:<tag> as typed syntax. On a note, this tag names the space's tag today (notes/parts.tsx:256 uses b.tag). Keep the syntax and flag it to the holder of the notes wording.

Sample and preview only (mirror the changes so the browser preview matches)
- lib/boards.ts:187, 225, 228, 231, 266, 311, 464 (Sample board)
- lib/cards.ts:1025, 1159, 2010
- lib/sample-cards.ts:490 (the board's allow list)
- design/gallery.tsx:234, 245, 385 (What is a board?)

Tests and goldens that pin these strings
- Frontend: lib/boards.test.ts, lib/thread.test.ts, components/command-palette.test.ts, components/talk/no-raw-keys.test.ts
- Rust: crates/window/tests/{cards,services,no_copy_answer}.rs, crates/core/tests/boards.rs, crates/api/tests/boards.rs, crates/notify/tests/notifier.rs, crates/coord/tests/{stranded_test,back_on_board_test,unheld_pr_test,spawn_test,placed_test}.rs
- Goldens: testdata/golden/tools/{specs,scenario.transcript}.json, testdata/golden/api/go.json, testdata/golden/mcp/wire.json

Text-keyed code: no frontend code matches on these error strings. inviteError (components/boards/board-page.tsx:560) matches only mesh/network text. The only prose-keyed match found is BACK_NOTE (fold.rs:2270); the other "board" prefixes are identifiers.