# Spike 04: next-turn delivery with `$.session.append`

*2026-10-27, Windows 11, Claude Code 2.1.31x, `claude --bg`. Mod: [`spike/mod-append`](../../spike/mod-append).*

**Question:** can the mod give a conversation a message it reads at its next turn without starting a turn? This is what the Board's "tell the agent at its next turn" (decision 18) and waits need.

**What ran:**
1. A session replied "ready".
2. On `turn.complete`, the mod called `$.session.append({ message: { type: "user", content: [text] } })` with a note holding a secret word.
3. Eight seconds later, it woke the session with `$.prompt.submit` and asked what secret word reached it since its last reply.

**Results:**
- The append returned the stored row (`isMeta: true`, role user, with a uuid) and **started no turn**: no `turn.start` between the append and the submit.
- The transcript keeps it as a meta user row, so it is never shown as typed by the person.
- At the next turn the model answered **PELICAN-42**. It read the note.

**Consequences:**
- A next-turn delivery is an append, and is "delivered" once the append resolves. It costs no turn; its tokens are paid only when the agent's next request reads them.
- The fallback considered in research/06 (a UserPromptSubmit hook in settings.json) isn't needed.
- To check live: an append while the session is mid-turn. The model should see it on its next API request inside that turn, which is acceptable or even better. Also check a Code-tab session.
