# Notes: shared and private context for agents

*2026-11-03, card #388 (from Stephen's thread #387). This document records the direction only; nothing has been built. Stephen: "shared context and private context ... per area (board/private board) ... accessible to the user and users of a board. So agent can have a shared sort of knowledge base that can be reviewed by people." Mockups: branch `marjoram/388-notes-mockups`, browser preview `?mockup=notes&view=1..3`.*

This document records the direction. It amends decision 27 (decision 45). Build details (storage rows, tool names, caps) get their own card when Stephen says go.

## 1. Why

Agents relearn the same lessons on each machine because what they learn lives in each session's private memory files. On 2026-11-03 there were 40 memory files on the Mac, at least 44 on the PC, and about three new ones a day. They fall into these kinds:

- rules about not breaking other agents' live work (10)
- Board working rules (9)
- Stephen's preferences (9)
- project decisions (8)
- machine and build gotchas (clearly more of these on the PC)

None of it crosses machines or reaches friends' agents. The clearest case: PC agents knew "never drive Stephen's mouse" on 11-02, and a Mac agent did it on 11-03 (#309, #327). Other cases:

- "No PRs on Irori" and the blocker etiquette exist in both machines' memories and again in the tool text.
- CMAKE and LIBCLANG_PATH are in `docs/releasing.md`, yet three PC agents hit the problem anyway.
- An informal cross-machine rules file has already appeared (`~/ww-shared/inbox/WW-MAC-RULES.md`).

The memories also go stale unseen. Some still give Go paths after the Rust rewrite, and the ww poke address differs between a memory and the rules file.

## 2. What a note is

A note is a short fact that an agent should know before it acts. It has these parts:

- **Kind:** rule, gotcha, decision, pointer or preference.
- **Text:** one or two sentences, a few hundred characters at most. Notes are not documents; long material stays in the repo's docs.
- **Applies to:** tags that say where the note is relevant. A tag can be `everywhere`, a board, a repo, paths in a repo, a machine (`on the PC`) or a topic.
- **Source:** where it came from, when the agent that proposed it names one. That can be a card, a comment, a blocker answer or a done proof, and the note links back to it.
- **Who:** the author (person or agent) and, once accepted, who accepted it.
- **State:** proposed, accepted or removed. The note's history is kept.

## 3. Scopes

- **Mine:** the note stays on the person's own devices and is read by their own agents. It never goes to a board's other members, even when it applies to a shared board ("on MLG"). A person-level preference, such as "say Version, not take", is a Mine note that applies everywhere. There is no separate person scope.
- **A board:** the note is in the board's log, and every member's agents read it, as they read the board's cards.

## 4. Review: proposed, then accepted

- **Mine:** a note an agent writes stays proposed until the person OKs it (Stephen, b2: "Wait for my OK"). A note the person writes is accepted at once.
- **A shared board:** a note stays proposed until any person on the board accepts it. Once accepted, it is live for every member's agents (Stephen, b3: "Any person on the board, for everyone"). Any member can edit or remove an accepted note, and the board's Notes page shows who accepted what and when.
- **Proposed notes:** they reach no agent. An edit made by an agent is itself a proposal.

## 5. Capture

- Agents propose notes with a tool, the way they raise blockers: on their own, or when the person asks for one in a comment. A wrong note costs only a review, since nothing goes live unaccepted.
- There is no Keep as note button on comments, blocker answers or done proofs. It was built (#413) and taken out (#475): the person does not save notes by hand from a card, and the button showed everywhere it was irrelevant.
- **New note** is on the Notes page. Nothing becomes a note by itself: the board is still scratch (decision 27). A note exists only because a person, or an agent's proposal plus a person's accept, made it.

## 6. Recall

- **On wake:** an agent gets the accepted notes that match the card it holds:
  - its board
  - its repo and the paths of its worktree
  - the agent's machine
  - every note tagged `everywhere`

  Each wake carries a few notes, about 8 at most, ranked by how specific the match is.
- **The person's words ride along:** the same hook adds what the person said lately on the cards around the agent's, in a section of its own (card #555, `docs/design/spawned-agents.md` §2).
- **More by search:** an agent can search notes for anything else, within the scopes it can read: Mine for the person's own agents, and the boards it is on.
- **How notes are framed:** notes reach agents as data with who wrote and who accepted them, never as instructions. This matches how messages from other agents are framed today. On a shared board a note is something a teammate wrote, not the person's own word.

## 7. Staleness

When the paths, flags or versions a note names have changed since it was accepted, the note shows **Still true?** on the Notes page. An agent that finds a note wrong proposes an edit or a removal instead of working around it.

## 8. Safety and privacy

- On a shared board, a note is another person's agent's words that reach yours. The review step, together with the data framing in §6, keeps such notes from steering agents. One accept by any person makes a board note live, so the board's Notes page shows newly accepted notes, and any member can remove one.
- Mine notes never go into a board log. A note that applies to a shared board, written in Mine, stays in the private log.
- The refusals already used for attachments apply to note text: no credentials, keys or `.env` content. Things that must never be shared in practice include home IPs, key and notary names, private paths and personal finances. These stay in Mine, and the New note form offers Mine first.

## 9. Where it lives in the app

**Notes** is a sidebar item next to **Talk** (#382, which has its own design round). The page has one tab for Mine and one per shared board. **New from agents** sits at the top, with Keep or Accept, Edit and Remove. Below it is the list, filtered by kind, where each row shows its tags, its source chip, its author and accepter, and how often it was read.

## 10. Not in scope

- Stephen's personal knowledge-base repo (`~/projects/knowledge-base`). Irori's notes are the knowledge base this design is about (b4).
- Building notes automatically from the board's history. That was shape B, set aside because it is noisy, goes stale and is unsafe on shared boards.
- A per-board handbook page. That was shape C.

## 11. Build contract (card #401)

What v1 is built on. The rows and the coord/Core layer are on main; the window, the agent tools and the Notes page build on them.

**Rows** (`crates/store/src/kinds.rs`, replicated, board logs accept them; `crates/coord/src/board/notes.rs` folds them):

| Kind | Payload |
|---|---|
| `note.proposed` | `{noteId, kind, text, tags, source?, taskId?, commentId?, blockerId?, by, at}` |
| `note.accepted` | `{noteId, by, at}`: a person accepts a proposal, or an agent's pending edit or removal |
| `note.edited` | `{noteId, kind?, text?, tags?, why?, by, at}` |
| `note.removed` | `{noteId, why?, by, at}` |

- A person's note is accepted at once. An agent's is `proposed` until a person accepts it: on Mine, the person; on a board, any member's `person:` key, written from that member's own device.
- An agent's edit or removal of a live note becomes the note's `pending` change; the live text stays until a person accepts. An agent edits or withdraws its own proposal directly. A person's edit or removal applies at once and drops any pending change.
- `history` keeps every step: proposed, accepted, edited, edit_proposed, removal_proposed, removed.
- Board closure: `taskId`, `commentId` and `blockerId` go through the board log's closure check like any card row, so a board note's source is always on that board.
- Refused when written: an unknown kind, empty text, text over 500 characters, no tags, a tag of unknown shape, and text the scrubber would mask (keys, tokens, passwords).

**Kinds:** `rule`, `gotcha`, `decision`, `pointer`, `preference`.

**Tags** (case folded except repo keys and paths):

| Tag | Matches a wake when | Score |
|---|---|---|
| `path:<repoKey>:<path>` | the agent's repo is `<repoKey>` and one of its paths is `<path>` or under it | 5 |
| `repo:<repoKey>` | the card's or worktree's repo | 4 |
| `board:<tag>` | a Mine note about a shared board: the card is on that board | 3 |
| `machine:<device name>` | the agent's device name | 2 |
| `everywhere` | always | 1 |
| `topic:<word>` | never on wake; for search | 0 |

A wake takes the accepted notes of every log the agent reads: Mine for the person's own agents, plus the card's board when it is on one. Each note scores by its best tag; the wake keeps the top 8 (`NOTES_PER_WAKE`), newest first within a score. `Coord::notes_for(&NoteContext{board, repo_key, paths, machine})` does one log; the wake merges Mine and the board's.

**Scopes and routing** (`crates/core/src/service/notes.rs`): `mine`, or a board's tag or id. A Mine note never enters a board log, whatever its tags name (`crates/core/tests/it/notes.rs`). Core API: `propose_note(by, scope, NewNote)`, `accept_note(by, id)`, `edit_note(by, id, NoteEdit, why)`, `remove_note(by, id, why)`, `notes(scope, removed)`, `search_notes(q, live_only)`. Each returns `ScopedNote {scope, boardId?, ...note}`. Ids may carry `note:`.

**Window commands** (`crates/window/src/notes.rs`, #411): `notes_list {scope, removed}` returns `NoteView[]` (an empty scope lists Mine and every board), `note_new {note: {scope, kind, text, tags}}` (New note), `note_accept {id}`, `note_edit {id, edit: {kind?, text?, tags?}}` and `note_remove {id, why}`, all as the person. A `NoteView` carries `pending` (an agent's edit, or `remove: true`), `history`, `byPerson` and `stale`.

**Board writes are gated** (#411): an older Irori refuses an unknown kind, and every later row of that writer's stream then fails its gap check, so one `note.*` row would stall this device's stream on any member device that predates notes. The note kinds landed on main before the board hello's `spaces` field (#392), so a device heard to read space rows reads note rows too: `Services::board_notes_ready` (applied by `Services::write_gate`) refuses a board-scope write while `space_not_ready` names any member device, with "every device on the board must run an Irori with notes first; waiting on …". Mine writes never wait. A later kind that older notes-reading devices don't know needs its own hello level.

**Agent tools** (built, #412; `crates/tools/src/notes.rs`): `notes_propose {scope?, kind, text, tags, source?}` for a new note, or `{note, kind?, text?, tags?, why}` / `{note, remove: true, why}` for an agent's proposed edit or removal. `notes_search {q}` searches accepted notes only, in the logs the agent reads. A proposal reaches no agent. The wake gets a short **Notes** section in the same untrusted-data framing as other agents' messages: each note's kind, text, `note:<id>`, author and accepter, and the tag that matched. Only full wakes carry notes (not reminders, appends or stops), and a note goes to one agent once: a later wake to the same agent leaves out the notes an earlier one gave it. The wake's card is the first `task:` ref among its messages, else the card the agent is working; its repo (else the agent's), its repo-path links and the agent's cwd under the repo root give the context, and the device's name the machine (the inbox's `Options::notes` hook, `Services::wake_notes`). Writes go through `Services::propose_note` and its siblings, which the window's `Core` calls too, so a board write gate there covers both. The gate (`Services::write_gate`, #411) sits in those bodies.

**Notes a wake carried** (built, #412): the private log's `wake.issued` row carries `notesTask` (`task:<id>`) and `notes: [{id, why, kind, text, by, acceptedBy}]`, names rendered (board logs refuse the kind). A replayed inbox rebuilds the prompt from them, and a later wake reads them to leave out what the agent already got. The card page doesn't list them (#547).

**Still true?** (v1): `NoteView.stale` says "its source card is gone", or "<path> is gone from <repo>" when a `path:` tag's path is missing from every checkout of that repo this device has seen. Repos with no checkout here are not checked.

**Not in v1:** the one-time import of the agents' memory files, and a note's read count.
