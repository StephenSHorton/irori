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
- **Source:** where it came from. That can be a card, a comment, a blocker answer or a done proof, and the note links back to it.
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

- **Keep as note** appears on any blocker answer, comment or done proof. It opens prefilled from the text, with tags guessed from the card (board, repo), and links back to its source. A blocker answer is often already a decision.
- Agents propose notes with a tool, the way they raise blockers. A wrong note costs only a review, since nothing goes live unaccepted.
- **New note** is on the Notes page. Nothing becomes a note by itself: the board is still scratch (decision 27). A note exists only because a person, or an agent's proposal plus a person's accept, made it.

## 6. Recall

- **On wake:** an agent gets the accepted notes that match the card it holds:
  - its board
  - its repo and the paths of its worktree
  - the agent's machine
  - every note tagged `everywhere`

  Each wake carries a few notes, about 8 at most, ranked by how specific the match is.
- **More by search:** an agent can search notes for anything else, within the scopes it can read: Mine for the person's own agents, and the boards it is on.
- **How notes are framed:** notes reach agents as data with who wrote and who accepted them, never as instructions. This matches how messages from other agents are framed today. On a shared board a note is something a teammate wrote, not the person's own word.
- **Visible on the card:** the card page can show which notes its agent got on wake and why, so the person can see how an agent knew something.

## 7. Staleness

When the paths, flags or versions a note names have changed since it was accepted, the note shows **Still true?** on the Notes page. An agent that finds a note wrong proposes an edit or a removal instead of working around it.

## 8. Safety and privacy

- On a shared board, a note is another person's agent's words that reach yours. The review step, together with the data framing in §6, keeps such notes from steering agents. One accept by any person makes a board note live, so the board's Notes page shows newly accepted notes, and any member can remove one.
- Mine notes never go into a board log. A note that applies to a shared board, written in Mine, stays in the private log.
- The refusals already used for attachments apply to note text: no credentials, keys or `.env` content. Things that must never be shared in practice include home IPs, key and notary names, private paths and personal finances. These stay in Mine, and the Keep-as-note form defaults to Mine.

## 9. Where it lives in the app

**Notes** is a sidebar item next to **Talk** (#382, which has its own design round). The page has one tab for Mine and one per shared board. **New from agents** sits at the top, with Keep or Accept, Edit and Remove. Below it is the list, filtered by kind, where each row shows its tags, its source chip, its author and accepter, and how often it was read. Keep as note sits on blocker answers, comments and done proofs.

## 10. Not in scope

- Stephen's personal knowledge-base repo (`~/projects/knowledge-base`). Irori's notes are the knowledge base this design is about (b4).
- Building notes automatically from the board's history. That was shape B, set aside because it is noisy, goes stale and is unsafe on shared boards.
- A per-board handbook page. That was shape C.

## 11. For the build card

- **Storage:** note rows (`note.proposed`, `note.accepted`, `note.edited`, `note.removed`) go in the private log for Mine and in the board log for a board. They must be added to the board kind allow-list (v0.5-shared-boards.md §2.2).
- **Tools:** agent tools to propose and search notes. The wake text gets a short notes section.
- **Tag matching:** how worktree paths and machine tags are matched, and the ranking and cap per wake.
- **Import:** a one-time pass that proposes notes from the agents' existing memory files, for Stephen to review.
