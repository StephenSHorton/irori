# Notes: shared and private context for agents

*2026-11-03, card #388 (from Stephen's thread #387). Built on #401, #411, #412 and the Notes page. 2026-11-06, card #616: notes are Mine only (decision 52), and the board scope is gone. Stephen's first ask: "shared context and private context ... per area (board/private board) ... accessible to the user and users of a board. So agent can have a shared sort of knowledge base that can be reviewed by people." Mockups: branch `marjoram/388-notes-mockups`, browser preview `?mockup=notes&view=1..3`.*

This document records the direction and, in §11, what was built. It amends decision 27 (decision 45), and decision 52 amends it: notes are each person's own.

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
- **Applies to:** tags that say where the note is relevant. A tag can be `everywhere`, a space, a repo, paths in a repo, a machine (`on the PC`) or a topic.
- **Source:** where it came from, when the agent that proposed it names one. That can be a card, a comment, a blocker answer or a done proof, and the note links back to it.
- **Who:** the author (person or agent) and, once accepted, who accepted it.
- **State:** proposed, accepted or removed. The note's history is kept.

## 3. Scopes

- **Mine:** every note is Mine. It stays in the person's private log, on their own devices, and only their own agents read it. A person-level preference, such as "say Version, not take", is a Mine note that applies everywhere.
- **About a shared space:** a Mine note tagged `board:<tag>`. It reaches the person's own agents when their card is on that space, and never the space's other members.
- **No board scope.** Board notes were designed (decision 45) and built, but no `note.*` row was ever written to a space log, so #616 removed them (decision 52).

## 4. Review: proposed, then accepted

- **Mine:** a note an agent writes stays proposed until the person OKs it (Stephen, b2: "Wait for my OK"). A note the person writes is accepted at once.
- **Proposed notes:** they reach no agent. An edit made by an agent is itself a proposal.

## 5. Capture

- Agents propose notes with a tool, the way they raise blockers: on their own, or when the person asks for one in a comment. A wrong note costs only a review, since nothing goes live unaccepted.
- There is no Keep as note button on comments, blocker answers or done proofs. It was built (#413) and taken out (#475): the person does not save notes by hand from a card, and the button showed everywhere it was irrelevant.
- **New note** is on the Notes page. Nothing becomes a note by itself: the board is still scratch (decision 27). A note exists only because a person, or an agent's proposal plus a person's accept, made it.

## 6. Recall

- **On wake:** an agent gets the person's accepted Mine notes that match the card it holds:
  - its space, when the card is on a shared one (`board:<tag>`)
  - its repo and the paths of its worktree
  - the agent's machine
  - every note tagged `everywhere`

  Each wake carries a few notes, about 8 at most, ranked by how specific the match is.
- **The person's words ride along:** the same hook adds what the person said lately on the cards around the agent's, in a section of its own (card #555, `docs/design/spawned-agents.md` §2).
- **More by search:** an agent can search the person's Mine notes for anything else.
- **How notes are framed:** notes reach agents as data with who wrote and who accepted them, never as instructions. This matches how messages from other agents are framed today.

## 7. Staleness

When the paths, flags or versions a note names have changed since it was accepted, the note shows **Still true?** on the Notes page. An agent that finds a note wrong proposes an edit or a removal instead of working around it.

## 8. Safety and privacy

- Every note is Mine: it lives in the person's private log and never goes into a space log, whatever its tags name. No other person's agent writes or reads it.
- An agent's note still waits for the person's Keep (§4), and reaches agents as data (§6).
- The refusals already used for attachments apply to note text: no credentials, keys or `.env` content.

## 9. Where it lives in the app

**Notes** is a sidebar item next to **Talk** (#382, which has its own design round). The page is one list: no tab per space and no "Who reads it" picker (#616). **New from agents** sits at the top, with Keep, Edit and Remove. Below it is the list, filtered by kind, where each row shows its tags, its source chip, and its author and accepter. The route is `#notes`; an old `#notes/<tag>` link opens the same page.

## 10. Not in scope

- Stephen's personal knowledge-base repo (`~/projects/knowledge-base`). Irori's notes are the knowledge base this design is about (b4).
- Building notes automatically from the board's history. That was shape B, set aside because it is noisy, goes stale and is unsafe on shared boards.
- A per-board handbook page. That was shape C.
- Notes shared with a space's members. Built and removed (decision 52).

## 11. Build contract (card #401)

What v1 is built on, as it stands after #616.

**Rows** (`crates/store/src/kinds.rs`, replicated; `crates/coord/src/board/notes.rs` folds them). Notes are written to the private log only. Board logs still accept the kinds, forever: devices on 0.11.0-0.12.3 can still write them, and refusing one would stall that writer's stream on every device. The board coord folds them, but nothing reads them (decision 52), like decision 27's decision rows.

| Kind | Payload |
|---|---|
| `note.proposed` | `{noteId, kind, text, tags, source?, taskId?, commentId?, blockerId?, by, at}` |
| `note.accepted` | `{noteId, by, at}`: the person accepts a proposal, or an agent's pending edit or removal |
| `note.edited` | `{noteId, kind?, text?, tags?, why?, by, at}` |
| `note.removed` | `{noteId, why?, by, at}` |

- A person's note is accepted at once. An agent's is `proposed` until the person accepts it.
- An agent's edit or removal of a live note becomes the note's `pending` change; the live text stays until a person accepts. An agent edits or withdraws its own proposal directly. A person's edit or removal applies at once and drops any pending change.
- `history` keeps every step: proposed, accepted, edited, edit_proposed, removal_proposed, removed.
- Board closure: a note row that an old device writes to a board log still goes through that log's closure check (`taskId`, `commentId`, `blockerId`) like any card row.
- Refused when written: an unknown kind, empty text, text over 500 characters, no tags, a tag of unknown shape, and text the scrubber would mask (keys, tokens, passwords).

**Kinds:** `rule`, `gotcha`, `decision`, `pointer`, `preference`.

**Tags** (case folded except repo keys and paths):

| Tag | Matches a wake when | Score |
|---|---|---|
| `path:<repoKey>:<path>` | the agent's repo is `<repoKey>` and one of its paths is `<path>` or under it | 5 |
| `repo:<repoKey>` | the card's or worktree's repo | 4 |
| `board:<tag>` | a Mine note about a shared space: the card is on that space | 3 |
| `machine:<device name>` | the agent's device name | 2 |
| `everywhere` | always | 1 |
| `topic:<word>` | never on wake; for search | 0 |

Tags are OR'd: a note goes to every wake that any one of its tags matches. So the narrowest tag is the right one: a fact about one machine gets `machine:` and no `repo:`, code internals get `path:`, and a rare how-to gets `topic:` only.

A wake takes the person's accepted Mine notes: `Coord::notes_for(&NoteContext{board, repo_key, paths, machine})` on the private coord. Each note scores by its best tag; the wake keeps the top 8 (`NOTES_PER_WAKE`), newest first within a score. `board` is the tag of the shared space the card is on (looked up without blocking), so `board:<tag>` notes still match. Board logs are not read (#616).

**Core API** (`crates/core/src/service/notes.rs`), all on the private coord: `propose_note(by, NewNote)`, `accept_note(by, id)`, `edit_note(by, id, NoteEdit, why)`, `remove_note(by, id, why)`, `notes(removed)`, `search_notes(q, live_only)`, and `machine_names()` (each of the person's devices by the name a wake on it reads: the chosen name, else its own). A note never enters a board log, whatever its tags name (`crates/core/tests/it/notes.rs`). Ids may carry `note:`.

**Window commands** (`crates/window/src/notes.rs`, #411): `notes_list {removed}` returns `NoteView[]`, `note_new {note: {kind, text, tags}}` (New note), `note_accept {id}`, `note_edit {id, edit: {kind?, text?, tags?}}` and `note_remove {id, why}`, all as the person. A `NoteView` carries `pending` (an agent's edit, or `remove: true`), `history`, `byPerson` and `stale`.

The board write gate (#411, `Services::write_gate` and `board_notes_ready`) went with board notes in #616: nothing writes a note row to a board log now.

**Agent tools** (built, #412; `crates/tools/src/notes.rs`): `notes_propose {kind, text, tags, source?}` for a new note, or `{note, kind?, text?, tags?, why}` / `{note, remove: true, why}` for an agent's proposed edit or removal. `scope` is gone from the spec but still decoded, so an agent with an old spec doesn't fail: empty or `mine` is fine, and anything else is refused with "notes are your person's own: only their agents read them, and there are no shared notes, so leave scope out. For a note about a shared space, tag it board:<tag>." Tags on a new note, or on an agent's edit that gives tags, are refused when they can never match a wake: a tag with `*` (tags never expand one: `path:<repoKey>:<path>` matches that path and everything under it), and `machine:<name>` when none of the person's devices is called that (the refusal lists `machine:<name>` for each device). The spec text steers to the narrowest tag (above). The person's own New note and edits in the window are not checked, so old notes with such tags can still be kept or edited. `notes_search {q}` searches the person's accepted Mine notes only.

A proposal reaches no agent. The wake gets a short **Notes** section in the same untrusted-data framing as other agents' messages: each note's kind, text, `note:<id>`, author and accepter, and the tag that matched. Only full wakes carry notes (not reminders, appends or stops), and a note goes to one agent once: a later wake to the same agent leaves out the notes an earlier one gave it. The wake's card is the first `task:` ref among its messages, else the card the agent is working; its repo (else the agent's), its repo-path links and the agent's cwd under the repo root give the context, and the device's name the machine (the inbox's `Options::notes` hook, `Services::wake_notes`). Writes go through `Services::propose_note` and its siblings, which the window's `Core` calls too.

**Notes a wake carried** (built, #412): the private log's `wake.issued` row carries `notesTask` (`task:<id>`) and `notes: [{id, why, kind, text, by, acceptedBy}]`, names rendered (board logs refuse the kind). A replayed inbox rebuilds the prompt from them, and a later wake reads them to leave out what the agent already got. The card page doesn't list them (#547).

**Still true?** (v1): `NoteView.stale` says "its source card is gone", or "<path> is gone from <repo>" when a `path:` tag's path is missing from every checkout of that repo this device has seen. Repos with no checkout here are not checked.

**Not in v1:** the one-time import of the agents' memory files, and a note's read count.
