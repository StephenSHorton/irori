# Huddles as places, with assistants and a canvas (card #1029)

A huddle becomes a place, like a thread or a card. It has a title, a roster, its transcript and a canvas. Each person's Office Assistant is connected while the huddle runs, is called by the name its person gave it, and writes on the canvas. Nothing real gets started until the huddle is over: during the call the canvas is everything, and afterwards anyone can divide it into cards with their own assistant.

From thread #1028. It builds on [call-transcripts.md](call-transcripts.md) (#656) and [calls.md](calls.md). Nothing here is built yet. Section 13 is the build plan.

## 1. What the person decided

| # | Decision | Where it lands |
|---|---|---|
| 1 | A huddle is a place: title, roster, transcript, canvas. | §3 |
| 2 | Everyone in the call hears and sees it live. Anyone outside reads it afterwards and acts on it. | §3, §10 |
| 3 | **No cards, tasks or other real work during a live huddle.** "During the call, the canvas is everything, every open question, every answer, every task." Something said early may change. | §7, §9 |
| 4 | After the call, anyone opens the finished canvas and divides it into cards with their own assistant. | §9 |
| 5 | Each person can name their assistant. There's a default. The name shows on the canvas and is how people call it. | §5 |
| 6 | No segregation by call: "if you're in a workspace with someone, you have chosen to work with them." Any agent in the space may read a huddle and follow it. | §10 |
| 7 | Don't shove it in ordinary agents' faces: they use tools if they need it. Office assistants are always connected. | §4, §10 |
| 8 | No fixed timer for invoking an assistant. | §6 |
| 9 | All of it is wanted, in no particular order. | §13 |

The design settles three questions the thread left open: ambient fill (§6, one note-taker), de-duplication (§7) and the cost default (§12). One needs the person's yes: whether a two-person huddle opens beyond the two people (§10).

## 2. What exists today, and what's missing

**There:**
- Each device transcribes only its own person (Whisper finals on the cleaned mic). The speaker comes from the connection or the signed row, never a guess from audio. That already answers "who is asking what".
- A finished phrase goes to the call's peers as a `ControlMsg::Transcript` frame and is written as a `call.segment` row in the conversation's log. Rows reach every member device, including ones not in the call.
- **The Office Assistant** (`crates/window/src/assistant.rs`, #783) is the corner chat. It is not an agent. Each question is one headless `claude -p` turn, off the roster, with no session kept, and with only `tools::ASSISTANT_TOOLS` (14 read-only tools, reading as the person). It proposes actions in its reply (`<<thread>>`, `<<go …>>`), and the window acts only on the person's tap. It needs Claude Code, so it runs on computers, never on a phone. Today it has no name, no huddle awareness and one model.
- **The canvas** (`frontend/src/components/canvas/`) is React Flow laid out by ELK (decision 59, [docs/canvas-layout.md](../canvas-layout.md)).

**Missing:**
- A huddle has no title and no page: only start and end posts and the `call:<id>` transcript.
- Live lines reach no agent or assistant. Call state is in memory and only the window reads it.
- No row or tool can put anything on a call's canvas. `task.placed` is card-only.
- `talk_read` refuses a person DM's huddle to agents, and `talk_list` hides it.

## 3. The huddle as a place

The huddle is identified by its `callId`, as now. It stays in its conversation (channel, person DM or Personal channel) for access: **no new access rule comes with the place.**

**What it holds:**

| Part | From | Notes |
|---|---|---|
| Title | `call.title` rows (§8) | A person's newest title always beats an assistant's. |
| Live or over | Start and end `CallMark`s, plus `Live` (below) | |
| Roster | `Live.devices` while live; the end mark's `people` after; `call.assist` rows for assistants | A device not in the call already holds `Live.devices`, from `ring`/`join` frames. |
| Transcript | `call.segment` rows and fallback posts, folded as `call_posts` | Unchanged. |
| Canvas | `call.item`, `call.item.set` rows (§8) | |

**"Over" needs its own definition.** Today a huddle is live while it has no end mark, so a call whose last device crashed stays "live" for ever. Over means an end mark exists, **or** the huddle is absent from `Live` (no `join` heard for 50 s, `LIVE_FOR`) and this device isn't in it. Every rule that says "after the call" uses this one function, on Core.

**The page.**
- Route `#huddle/<callId>` (`frontend/src/lib/nav.ts`, a new `Section` kind).
- Header: title (click to edit), place, live or length, Join when live.
- Roster: each person with their assistant beside them, showing connected, not connected or taking notes.
- Desktop: canvas in the centre, transcript on the right (`TranscriptPane`, live captions while live).
- Phone: tabs for Canvas, Transcript and People.
- The in-call `CallPanel` stays as it is and gets an "Open huddle" link. Being a route of its own means the page doesn't fight `CallPanel` for the conversation's right-hand slot.

**Finding a huddle.**
- The start and end posts show the title and open the page.
- Talk gets a Huddles list for the picked space, newest first: title, place, people, length, live.
- `talk_list` shows the title.

**Titles.** Anyone in the conversation can set one. If nobody has, the note-taker proposes one from the first couple of minutes (§6), and updates it at the end. A title set by a person is never replaced by an assistant.

## 4. Who gets what

| Who | Live feed pushed? | Can read | Can write |
|---|---|---|---|
| People in the call | Captions, canvas | Everything | Title, canvas |
| People in the space, not in the call | No | The page, the transcript, the canvas | Title, canvas |
| Office assistants of people in the call | **Yes, always** | The huddle and a packet of the space's open work, with no tools (§7) | The canvas only, items as open |
| Ordinary agents in the space | **No** | `talk_read`, `huddle_status`, `huddle_canvas` | The canvas, with `huddle_note` (§11) |

"Always connected" needs no switch ([decision 52](../decisions.md)): an assistant connects when its person joins. Settings only turn it off (§12).

## 5. The assistant's name

**Storage.**
- `assistant.name` private row: `{name, by, at}`, actor `user`, newest wins, `Replicated, Refuses`, set in the app only (the shape of `assistant.names.set`).
- Other people's devices need the name to display and match it, so it also rides `member.profile` as an unsigned field (`assistantName`), as colour, avatar and GitHub login do.
- **No row means the default, "\<profile name\>'s assistant"** ("Addam's assistant"). It is computed, so nothing is written and every device agrees. It's unique per person, it follows a rename of the person, and it's how people already talk. #1030 uses the same default and the same row: there is one name store for both designs, owned by card 3 here.
- A chosen name is 1 to 24 characters, letters and spaces. Set in Settings › Agents. The chat header and the assistant's system prompt use it.
- **Shown as "Juno · Addam's assistant"** wherever others see it (canvas, roster). "Addam" comes from the person who signed the row, through their `member.profile`, never from the row's text. So nobody can write under someone else's assistant's name, and a chosen name can't pose as a person. With no chosen name it's just "Addam's assistant".

**Matching it in speech.** A plain Rust function over each line, no model. Whisper mishears names, so matching is loose in spelling and strict in position:

1. Lower-case, strip punctuation, split into words. "Addam's" becomes `addam` plus a possessive mark.
2. A word matches a name if it's equal, or within edit distance 1 for names of 4+ letters, or has the same phonetic key. A two-word name matches as two adjacent words. Whisper writes "Adam's assistant" for "Addam's assistant": rule 2 covers it.
3. **The forms that call an assistant:**
   - its chosen name ("Juno");
   - \<person\>'s assistant, where \<person\> is the first word of their profile name or the whole of it ("Addam's assistant", "Addam Lee's assistant"), with or without a chosen name;
   - "my assistant", from its own person only;
   - "\<person\>'s Juno", a person's name before a chosen name, to pick one of two Junos. Tried first.
4. **Address position only:** within the first three words of the line, right after `hey`, `ok`, `so`, `and` or `ask`, or at the line's end ("…right, Juno?"). "I was telling Addam's assistant yesterday" doesn't count.

**Two assistants answer to one form.** Two people both called Stephen, or two Junos. The speaker's own wins. If neither is theirs, no look runs and a note goes on the canvas: "Two assistants answer to Juno. Say whose." Setting a name that matches another member's name, or another assistant's, shows a warning but is allowed.

Seeding Whisper's prompt with the roster's names would help further (`crates/voice` has no such option yet). It's a tuning step, not v1.

## 6. When the assistant is invoked

**The answer to "every 5 seconds?": never on a timer.** Lines are collected in Core at no cost, and the model runs only when a layer fires. The cheapest check is first and every layer but L3 needs nothing from the model to decide.

| Layer | Fires when | Who | Decided by |
|---|---|---|---|
| **L1 Addressed** | A line matches the assistant's name (§5) or someone uses Ask on the page | That one assistant | Rust match |
| **L2 Own person asks** | A line from the assistant's **own** person ends in `?` or starts with a request stem ("do we", "can someone", "let's make", "who's", "what's") | That person's assistant only | Rust rule |
| **L3 Digest** | At least 30 s of new talk since its last look, and then a natural gap: 4 s with no line, or the speaker changes. Forced at 90 s of new talk. Never without new talk. | The note-taker only | Rust |
| **L4 End** | The huddle is over | The note-taker: summary, title, tidy of open items. Others stop. | Rust |

L2 is what makes the person's own example work: "do we have anyone working on X?", said with no name, goes to the asker's own assistant, and one question wakes one assistant. A question for the room from someone else wakes nobody, and lands on the canvas through the digest as an open question.

**A look is the turn of the held session.** Each assistant keeps one running `claude -p` for the huddle, and each look sends only the lines since its last look. There's no turn between looks, and it costs nothing while it waits. Details in §7.

**Rules that keep it bounded:**
- One look at a time per assistant. Lines that land during a look go in the next one.
- L1 isn't throttled apart from that. L2 at most every 15 s. L3 at least 30 s apart.
- **Hold for the rest of the sentence.** Whisper cuts phrases at 800 ms of silence, so "Juno, do we have anyone" and "working on signing?" can be two lines. After a match, wait for that speaker's next line or 1.5 s, whichever is first. A phone speaker's next line can take 5 s, so a line from the same speaker within 8 s of a look starts one more.
- A cap of 300 looks per huddle per assistant. Past that, L2 and L3 stop and L1 goes on, and the roster says so.
- **Canvas items never trigger anything.** Only spoken lines (and Ask) do. Assistants can't set each other off.

**Where the check runs.** The thread proposed the speaker's device. It has to be the **assistant's host** instead: my device can't run your assistant, and every Core serves only its own person's agents. The host sees the line anyway, because the row reaches every member device. Each host evaluates the same lines against the same replicated names, so nothing has to coordinate.

**Ambient fill: one note-taker, not everyone.** The thread's lean was every assistant on, posting only about what it knows. This design picks **only the assistant of whoever started the huddle**, handed to the next person by join order if they leave or their assistant can't run. Reasons:
- "Only what it knows" can't be checked by Core, and a model told to post when relevant will narrate. Several assistants reading the same words would write the same notes.
- It costs one assistant's digest, not N.
- What other assistants uniquely know still arrives, through L1 and L2, which are always on for everyone.

## 7. The assistant in a huddle

It's the Office Assistant, not an agent: still off the roster, not counted against agent limits, and no cards handed to it.

**Which device hosts it.** The person's devices each work this out, with no messages: among their online devices that can run Claude Code and aren't stopped (Stop here, Stop everywhere), prefer one in the call, then the smallest device id. If none qualifies (a person on a phone with their computer off) the roster shows "not connected: needs a computer that's on". The host writes `call.assist` when it connects and when it stops.

**The held session.** A new kind of run beside `Job`:
- The flags of a no-tools job (`job_args` with `tools: false`, as the thread namer and weigher use): `--no-session-persistence`, `--strict-mcp-config` with no `--mcp-config`, `--tools ""`, `--permission-mode dontAsk`. No assistant secret is written. Add `--input-format stream-json`, with stdin kept open.
- Started when the person joins, so the first look is warm.
- It has its own pool, not `MAX_RUNS`.
- Each look writes one user message and reads until `result`, with a 60 s timeout.
- If it dies, the next look starts a new one seeded with the title, the canvas and the last 5 minutes.
- Model: the Office Assistant's, `assistant::MODEL` (Haiku today), and #1014's setting once that lands.

**Measured** (spike, 2026-11-08, Claude Code 2.1.319, Haiku, no tools, no MCP, about 5k tokens of system prompt and talk; four looks each way, costs at API rates):

| | Time per look | Cost per look |
|---|---|---|
| Held session, after the first look | 1.1–1.9 s | $0.00013–0.00026 |
| Held session, first look | 4.1 s | $0.0012 (writes the cache) |
| Fresh `claude -p` per look | 3.7–5.0 s | $0.00040–0.00049, growing with the talk |

Not measured: looks that call tools, an hour-long session, and Sonnet on low effort (#1014's default).

**A trust boundary.** A transcript line is untrusted: anyone in the call can say anything, and a phone-transcribed line is no proof of who means what.
- Lines reach the model as data, one line each, marked with the speaker. They're never `trust="user"`, even for the person's own voice.
- The assistant has **no tools at all**. It writes by putting blocks in its reply, which the Core parses. That's the `<<thread>>` pattern, and the Core binds every block to this huddle. The model can't name another huddle, space or card.
- So the worst a spoken instruction can do is put an open item on this huddle's canvas.

**What it knows: a packet, not tools.** The Office Assistant's chat reads **as the person** (`ASSISTANT_TOOLS`), which includes their private cards and notes, and a huddle's items go on a canvas other people see. "Juno, what's on Stephen's private board?", said by Addam, must not work. So a huddle look gets no reads at all. Instead the Core builds a **packet** from the folded view of the huddle's own space log and puts it in the session's first message:
- the space's open cards: ref, title, state, holders, at most 40, newest activity first;
- the agents working in the space, with the card each holds;
- the huddle's title and canvas so far.

Each later look carries the lines since the last look, plus cards that changed since then. That answers the person's own example, "do we have anyone working on X?", from the space's cards and holders. A question the packet can't answer gets an open item saying so, for after the call.

**The rule, shared with #1030:** no turn that runs unattended and contains another person's words has person-level reads. A person's own chat may hold both, because nothing leaves it without that person's tap (§9).

| Huddle in | The packet comes from |
|---|---|
| A shared space's channel | That space's log |
| A Personal channel | The person's Personal boards and their spaces: only the person is there, so nobody else's words are in the turn |
| A person DM | The spaces both people are members of (a hidden DM space has no boards of its own) |

This is stricter than #1030's "shared-space cards fine, private cards with the owner's OK", on purpose: there's no moment in a live call to ask. It's also faster: no tool round trips (§12).

**Output.** One or more blocks, anything else ignored, so a chatty reply writes nothing (the spike's model did not reliably say NOTHING). At most 5 items per look, each at most 2000 characters:

```
<<item kind="answer" reply="ITEM-ID">>Addam has #412 on Windows signing.<</item>>
<<title>>Release planning and the call canvas<</title>>
```

A block has no status: the Core writes every assistant item as `open` (§8).

**The firm rule, in the code.** No block makes a card, task or anything else, because no such block exists, and the look has no tools. The Office Assistant's corner chat can still draft a thread for the person to post, on their own tap, as today.

**Fast acknowledgement.** On an L1 match, before the model runs, the host writes a placeholder item: "Juno is looking: do we have anyone on signing?". The answer replaces it, or it says "Juno found nothing". Everyone sees that within about a second of the line landing, which matters more to how it feels than the few seconds behind it.

**De-duplication.** Mostly structural: one digest writer, and one addressee per ask. On top of that, each look's message lists what others added since the last look, and the Core drops an item whose normalised text equals one already on the canvas.

## 8. Rows

All four are `call.*`, `Replicated, Accepts` (a `call.*` kind must never be `Refuses`). Each carries `convId`, so `space_may_touch` applies. Add each to the `kinds!` table, `coord::talk::KINDS`, `SPACE_TALK_KINDS`, the fold, and the golden test. Older devices store them unread (`call.` is a tolerated family since `tolerant` 2), so no protocol bump is needed.

```json
call.title    {"callId","convId","title","assistant":false,"by","at"}
call.item     {"itemId","callId","convId","kind","text","status","reply":"<itemId>?","assistant":true,"by","at"}
call.item.set {"itemId","callId","convId","text?","status?","x?","y?","card?","by","at"}
call.assist   {"callId","convId","device","state":"on|off","by","at"}
```

- **`kind`:** `question`, `answer`, `decision`, `task`, `note`, `summary`.
- **`status`:** `open`, `settled`, `dropped`, `carded`, shown by kind (a question reads open or answered, a decision proposed or agreed). `summary` and `note` have none.
- **`call.item.set`** folds per field, newest wins. Only the writer's person edits `text`. Any person sets `status` or `x`/`y`.
- **New kinds, read only by this fold.** No other fold reads `call.item`: a `task` item is never a board task, and nothing on the Board, in agent starts or in the inbox looks at canvas rows.
- **An assistant's row is never the person's decision.** An assistant writes items as `open` only. A `call.item` or `call.item.set` with `assistant: true` that carries any other status is still stored and replicated like any `call.*` row, but the fold ignores that status (an item reads as `open`, a set's status field is dropped). Only a person's own row (no `assistant` flag) settles, drops or cards an item. During the call a question with an answer shows as "open · 1 answer" until a person settles it.
- **Append-mostly:** an assistant adds items and edits only its own placeholder's text. Nothing is deleted: `dropped` is a status. Agents never move anything.
- **`card`** is a plain ref string set when an item becomes a card (§9). It isn't `taskId`, because `store::board::resolves` refuses a `taskId` that isn't on that board, and a DM's cards live on the promoter's Personal.
- **Writes after the fact:** while the huddle is live the window won't write `status: carded` or `card`, and the fold ignores them if they arrive.

**"Agent-written" is a flag.** Assistant rows are written by the person's device under the person's own key, with `assistant: true`.
- That is as provable as the alternative. A row's signature proves only the person (`bind_actor`). Any of a person's devices can already sign as any of their agent cards, so an `agent:<card>` actor proves nothing more.
- It keeps the person-DM door nearly closed: `dm_conv_of` keeps "written by one of the pair under their own key" and only gains kinds, with no agent identity in hidden spaces and no new mint path around `card_for`'s roster check.
- It's shown on every item, always, as the assistant's name with an agent mark. An item with `assistant: true` is never shown as the person's own words.
- Ordinary agents write with their own `agent:<card>` actor (§11). They do so in shared spaces and Personal channels only.

**The gate.** Rows go only when `Core::space_tolerance(space).ready()` holds, read once on entering the call like `Script.rows`. Personal channels have no gate. Widen the `== CALL_SEGMENT` check in `spaces.rs` to the `call.` prefix. Not ready means **no rows at all mid-call**, so no live feed and no canvas: the notice says "Agents and the canvas are off: someone's Irori in this space needs updating", and the transcript still arrives at the end.

**The feed.** One path, the rows. A host need not be in the call (a person on a phone has their assistant on their computer), and `call.segment` already reaches every member device. Call `Core::huddle_line(row)` after a successful fold on the live paths only (own write, the space follow, the private-log follow), never in the refold at start. A device in the call also gets frames sooner, but the row hop is small next to the look, so a second source isn't worth its dedupe. If a card's measurement says otherwise (§12), add it then.

## 9. After the call

Promotion is one step, and only after the huddle is over.

1. The page's canvas becomes **the finished canvas**: items grouped by kind, open questions and proposed tasks first.
2. **Divide with my assistant** opens the Office Assistant chat with this huddle in context (any person can, whether or not they were in the call). They talk it through: merge these, split that, this one's Addam's. This chat keeps its person-level reads while holding other people's words, which §7's rule allows only because the person is there: the canvas and transcript go in fenced as untrusted data (as `inbox::fence` does), and nothing leaves the chat until the person taps Make cards, seeing the exact text first.
3. The assistant ends its reply with a draft block, in the `<<thread>>` pattern:
   ```
   <<cards>>
   {"title":"…","objective":"…","items":["<itemId>"],"board":"…"}
   <</cards>>
   ```
4. The chat shows the drafts with **Make cards**. Nothing is made until the person taps it.
5. Each card goes through the normal create path, then writes `call.item.set{status:"carded", card:"#123"}`, so the canvas shows what became what.

**Where cards go.** A shared-space channel's huddle goes to that space (board picker). A Personal channel's goes to Personal. A person DM's goes to the promoter's Personal, since a hidden space has no boards. Assignment and agent starts then follow the normal rules (decisions 40, 50).

Two people may divide the same canvas. `carded` shows up live, and the drafts leave out items already carded when the block is shown. A card made twice from one item is a duplicate the person can see and close.

## 10. Access and consent

**Rule: everyone in the space the huddle lives in, and all their agents.** Ordinary agents may read and follow it. Only assistants are pushed.

| Huddle in | Who, and their agents |
|---|---|
| A shared space's channel | Every member of the space |
| A Personal channel | The person |
| A person DM | **The two people** (to confirm) |

**The person DM: this needs a yes.** A person DM sits in a hidden space whose only members are the two people, and it isn't in any workspace. "Members of the space it lives in" therefore gives those two and all their agents, which drops #656's rule. Going wider isn't a setting: no third person's device holds a copy of that log. Someone who wants the workspace to hear a call huddles in a channel. If the person meant that a one-to-one call should reach everyone in a shared space, the call would have to be held in one (and a DM huddle would stop being private to two).

**Changes to #656's agent rules:**
- `call_parent` no longer refuses a person DM's `call:<id>`.
- `talk_list`'s huddles section shows every huddle of the agent's spaces with its title, not only channels it joined, and includes DMs.
- `call_end_bridge`, `tell_call_end` and the hidden-space gate in `boards.rs` keep their checks: the end wake stays as it is for channel members, and **no wake is added for a person DM's agents.** Reading it by tool is enough (row 7 of §1's table).

**Earlier DM huddles stay closed.** Those people were told "written down for the two of you". Each `call.segment` row gains `agents: true` when it's written under the new notice. In a person DM, agents see only lines with the mark, so older huddles and lines from an older build stay closed. A channel needs no mark: its notice has always said "and their agents".

**The notice.** Before the mic opens (`transcriptNotice`, `frontend/src/lib/call.ts`), present tense:
- Channel: "Agents hear this huddle. What you say is written down as you speak, for #x's members and all their agents."
- DM: "Agents hear this huddle. What you say is written down as you speak, for the two of you and your agents."
- Personal: "Agents hear this huddle. What you say is written down as you speak, for you and your agents."
- Also fix a bug the readers found: `Names::conv` never says `personal`, so a Personal huddle's Join shows the channel text.

During the call the "Transcript on" badge titles add "Agents hear this huddle", and the roster shows each assistant connected.

**Honest limits.**
- Turning off "Transcribe what I say in calls" is the only real off, because it stops the text being made. Nothing else can be enforced: peers receive each line, and one device can't bind another person's agent.
- A transcript is permanent on every member device, and an agent's reading is not private to anyone's call.
- The earlier "hide my lines" idea (`call-transcripts.md`, Open) isn't built, and the person's answer on segregation means it isn't planned.

## 11. Tools for ordinary agents

Nothing is pushed. They reach for these when they need them.

- `talk_read conv=call:<id>`: as today, with speakers named (it prints another person's raw `person:<id>` today; fix it). While the huddle is live, its result ends with: "This huddle is still going. What's said may change: don't start work from it until it ends."
- `huddle_status {call}`: title, live or over, people, whose assistant is connected, line and item counts. Read-only.
- `huddle_canvas {call}`: the items, paged. Read-only.
- `huddle_note {call, kind, text, reply?}`: adds one item with its own actor, in shared spaces and Personal channels. Refused in a person DM, and under Ask me autonomy. It can't make a card: with nothing pushed to them, agents write only when something asked them to.

Listing rules follow `talk_read`: the same `call_parent` check. `talk_read`'s cursor moves before the result is clipped to 8 KiB, so clipped lines count as read. `huddle_canvas` must budget before it advances.

## 12. Latency and cost

**What it will feel like: a colleague taking notes, not someone in the conversation.**

| Step | Mac speaker | PC speaker | Android speaker |
|---|---|---|---|
| End of speech to a finished line (800 ms pause + Whisper final) | about 0.9 s | at least 1.9 s | 4.5–5.1 s |
| Line to the assistant's host (row) | not measured; should be under 1 s | same | same |
| Hold for the rest of the sentence | up to 1.5 s | same | same |
| The look: no tools, the packet already in the session | 1.1–1.9 s measured with about 5k tokens; estimated 2–4 s with a 40-card packet | same | same |
| Item to everyone's canvas (row) | under 1 s | same | same |
| **Spoken to answer on canvas** | **about 4–8 s** | **about 5–9 s** | **about 8–13 s** |

The acknowledgement (§7) shows at about 2–3 s (about 7 s from a phone). A busy machine, or a cold session (+2–3 s), can push the answer towards 30 s.

Finals only is a hard floor: a word isn't known until its phrase ends, and unbroken speech is cut at 25 s (`MAX_PHRASE`). The floor belongs to the **speaker's** device: a Mac listener can't speed up a phone speaker's lines.

**What would speed it up, best first:**
1. Keep the session warm (already in the design).
2. No tool calls per answer: the packet is in the session (already in the design, §7). A tool round trip would add 1–3 s each.
3. A smaller Whisper model on Android for calls. It's the biggest single number, and it costs accuracy.
4. Partials for address detection only: `Options.partials` exists, but needs a new frame type, and costs a pass per 700 ms, which nearly fills the phone. Not v1.
5. Taking live frames on an in-call host: saves only the row hop.

**Cost.** Every look spends the owner's own Claude usage, with one assistant per person in the call. From the spike, a held look on Haiku is a fraction of a tenth of a cent: **cents per assistant-hour**. A 40-card packet adds cached input, and Sonnet on low costs more (neither measured). That fits decision 52.

**Settings** (Settings › App, on by default; the setting only turns things off):
- "Connect my assistant to huddles": off means no assistant from this person.
- "Let my assistant take notes in huddles I start": off passes the note-taker to the next person.

## 13. Build order

Dependencies, not preference: the place first, then who can read, then the names and the assistant's engine, then the feed, the canvas on top of it, and the after-call step last.

```
1 Place ──────────────┬─> 6 Canvas rows ──> 7 Canvas UI ──> 9 After-call
2 Access and notice   │        ^                 ^
3 Names ──> 5 Feed, L1, L2 ────┘                 │
4 Held run, packet ────────^      8 Note-taker ──┘
                                  10 Agent tools, after 6
                                  11 Real-device check, last
```

Cards are proposed, not started. All are normal weight unless marked.

**1. Huddle as a place.**
Files: `crates/store/src/kinds.rs`, `crates/coord/src/talk.rs` (`call.title`, `CallInfo` title and people, "over"), `crates/core/src/service/spaces.rs` (`SPACE_TALK_KINDS`, the tolerance prefix), `crates/core/src/service/dm_spaces.rs` (kinds in `dm_conv_of`, `dm_refusal`), `crates/window/src/talk.rs`, `frontend/src/lib/nav.ts`, a new `frontend/src/components/talk/huddle-page.tsx`, `huddles-list.tsx`.
Done when: a huddle in a channel, a DM and a Personal channel each has a page with a title anyone can set, its roster and transcript; the Talk list and end post show the title; a call whose last device crashed reads as over within a minute; `sh scripts/check.sh` passes.

**2. Access, names in reads, the notice.**
Files: `crates/tools/src/talk.rs` (`call_parent`, `huddles()`, speaker names), `crates/coord/src/talk.rs` and `crates/net/src/call.rs` (the `agents` mark), `crates/core/src/service/calls.rs`, `frontend/src/lib/call.ts`, `frontend/src/components/talk/huddle.tsx`, `crates/window/src/call.rs` (`Names::conv`), `docs/design/call-transcripts.md`.
Done when: an agent of either person reads a new DM huddle and is refused an older one; `talk_list` shows every huddle in its spaces, with titles; speakers print as names; no new wake happens; the notice reads as §10 in all three places and on a Personal Join; tests cover each.

**3. The assistant's name.**
Files: `crates/store/src/kinds.rs`, a new `crates/core/src/service/assistant_name.rs`, `crates/identity/src/person.rs` (the profile field), `crates/core/src/service/boards.rs` (profile refresh), `crates/window/src/assistant.rs` (prompt), Settings › Agents, `frontend/src/components/assistant/assistant-chat.tsx`. Coordinate with #1030, which needs the same name.
Done when: a person can set a name and see it on all their devices and on another member's, shown as "Juno · Addam's assistant" with the person taken from the row's signer; a person who hasn't reads as "Addam's assistant" everywhere with nothing written, and follows a rename; #1030's agent has this card's number (#1030's build cards sit on it).

**4. Held run and the packet.**
Files: `crates/window/src/assistant.rs` (held runner on `job_args` with `tools: false`, block parser), `crates/core/src/service/thread_names.rs` (the seam it copies), a new `crates/core/src/service/huddle_packet.rs` (the packet from a space's folded log; share it with #1030's `assistant_answers.rs` packet if that lands first), `app/src/shell/mod.rs` (installing the hook).
Done when: a held session takes several looks and restarts from a seed after being killed; its arguments carry no `--mcp-config` and no secret; a packet test with a private card, a Personal card and another space's card contains none of them, for a channel and a DM huddle; a reply with no blocks writes nothing; cost and time per look are recorded in this doc for Haiku and Sonnet low with a 40-card packet.

**5. Feed, L1, L2.**
Files: `crates/core/src/service/calls.rs`, `boards.rs` and the private-log follower (`huddle_line`), a new `crates/core/src/service/huddle_feed.rs` (host pick, per-huddle buffer, matcher, layers, caps, `call.assist`).
Done when: pure tests cover §5's matching (each form: chosen name, "Addam's assistant", "my assistant" only from its own person, "Stephen's Juno"; position; near-spellings such as "Adam's"; collisions) and §6's rules (one look at a time, the hold, the cap); a line naming an assistant in a two-Core test gets a look, a mid-line mention doesn't; another person's question wakes nothing; a refold at start triggers no look; a person with no computer on shows "not connected".

**6. Canvas rows and writes.**
Files: `crates/store/src/kinds.rs`, `crates/coord/src/talk.rs` (fold), `crates/core/src/service/dm_spaces.rs`, `spaces.rs`, `huddle_feed.rs` (placeholder, block to row, de-dupe), `crates/window/src/talk.rs`.
Done when: blocks become items on all members' devices in a channel, a DM and a Personal channel; `dm_conv_of` still refuses anything not by the pair; `carded` written while live is ignored by the fold; an `assistant: true` row with any status but `open` is stored but its status is ignored by the fold; no fold but this one reads the new kinds; a person's item and an assistant's show differently.

**7. Canvas UI.**
Files: `frontend/src/components/canvas/data.ts` (a `NoteNode`, in `GraphNode`), `elk.ts` (a huddle layout), `reactflow/index.tsx`, a new `canvas/huddle.tsx`, `huddle-page.tsx`.
Done when: people and their connected assistants head the canvas, items flow from who asked, and a new item arrives without resetting the view or a drag; it works at phone width (`bun run preview:look`) and in light and dark; screenshots on the card.

**8. Note-taker: L3 and L4.**
Files: `huddle_feed.rs`, `crates/core/src/service/calls.rs` (handing it on), Settings › App.
Done when: only one assistant digests per huddle and it passes on when its person leaves; no digest runs without new talk; a title appears after a couple of minutes and doesn't replace a person's; the end produces a summary; both settings turn off what they say.

**9. After-call.**
Files: `huddle-page.tsx`, `frontend/src/lib/assistant.ts`, `assistant-chat.tsx`, `crates/window/src/assistant.rs` (the `<<cards>>` block, huddle context), `crates/window/src/new_card.rs`.
Done when: from a finished canvas a person gets drafts with their assistant and Make cards makes them on the right board; items show as carded with a link; nothing can be made while live; an agent starts only by the normal rules.

**10. Ordinary agents' tools (light).**
Files: `crates/tools/src/specs.json`, `specs.rs`, `talk.rs`, `crates/api/src/backend.rs`, `crates/tools/tests/it/`.
Done when: `huddle_status`, `huddle_canvas` and `huddle_note` work with §11's refusals; the live-huddle line appears on `talk_read`.

**11. Real-device check. (needs:mac and a phone)**
Done when: a real huddle between a Mac and the Pixel, with a third person's assistant on a computer outside the call, has each assistant answer by name; §12's table is filled in with measured numbers, row latency included; a live feed source is added only if row latency is over 2 s.

## 14. Not in this design

- An assistant talking to other people's assistants (#1030). "Who knows about X?" in v1 is answered from what the space can see.
- Asking the room ("assistants, who knows…") and handing that between assistants. Later, by decision 38's rule.
- Voice addressing of ordinary agents, and cards or any other real work from a live huddle.
- Wake words other than the name, screen-share reading, and audio retention. No audio is kept, as today.
