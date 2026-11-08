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

The design settles the questions the thread left open: ambient fill (§6, one note-taker), de-duplication (§7) and the cost default (§12). One still needs the person's yes: one note-taker rather than every assistant (§6). Who hears a two-person huddle is settled: the two people and their agents (§10).

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
| Roster | `Live.devices` while live; the end mark's `people` after; after a crash with no end mark, the people who spoke (`call.segment` signers) and the start mark's author; `call.assist` rows for assistants | A device not in the call already holds `Live.devices`, from `ring`/`join` frames. It lists only devices heard directly, refreshed every 15 s: a crashed device stays listed until the whole entry expires. |
| Transcript | `call.segment` rows and fallback posts, folded as `call_posts` | Unchanged. |
| Canvas | `call.item`, `call.item.set` rows (§8) | |

**"Over" needs its own definition.** Today a huddle is live while it has no end mark, so a call whose last device crashed stays "live" for ever. But `Live` is per-device memory, filled only by `ring`/`join` frames (a heartbeat every 15 s, `HEARTBEAT`) and empty after a restart, so "absent from `Live`" alone can't mean over: a device that just started or woke reads every running huddle as absent. One function on Core, `over(call)`, answers for every rule that says "after the call":
- **Over** if an end mark is in the rows.
- **Over** if this device heard the huddle live, then heard nothing for `LIVE_FOR` (50 s) while this device stayed online, and no `call.segment` for it is newer than 2 minutes. (Online means its own network was up, not that another member device was connected: in a two-device space the survivor of a crash has nobody else to see.)
- **Over** if this device never heard it live and the huddle's newest row (start mark, line or `call.assist`) is older than 10 minutes.
- **Otherwise live.** Not knowing counts as live, so the after-call steps (L4, Divide, Make cards) stay off.

When `over` holds without an end mark, the note-taker's host (§6) writes the end mark, with the people from the roster row above, so every device then agrees from the rows. If none appears within 3 minutes of `over`, the next connected assistant's host writes it; with no assistant at all, any member device that judges `over` does. The fold keeps the first end mark.

`Live` forgets a huddle as soon as it goes quiet: the expiry timer fires `CoreEvent::Call`, the window reads `call_snapshot`, which drops entries older than `LIVE_FOR`, and a `leave` from the last device or an `end` frame removes the entry outright. So `over` keeps its own per-device record, `heard: callId → last ring or join time`, written beside `Live` in the frame handler and never pruned by those. A display-only "probably over" (absent from `Live`) may grey the page's Join button, but nothing else uses it.

**The page.**
- Route `#huddle/<callId>` (`frontend/src/lib/nav.ts`, a new `Section` kind).
- Header: title (click to edit), place, live or length, Join when live.
- Roster: each person with their assistant beside them, showing connected, not connected or taking notes.
- Desktop: canvas in the centre, transcript on the right. `TranscriptPane` today loads once and is a 340 px aside with a close button: it gets a variant that fills its column with no close, and reloads on `talk:changed` for its call while the huddle is live. A device in the call may show `huddle.tsx`'s `Captions` instead.
- **Ask** box under the canvas: a typed question to the viewer's own assistant, only while that assistant is connected (it fires L1, §6). Hidden for a viewer whose assistant isn't connected.
- Phone: tabs for Canvas, Transcript and People.
- The in-call `CallPanel` stays as it is and gets an "Open huddle" link. Being a route of its own means the page doesn't fight `CallPanel` for the conversation's right-hand slot.
- A new `Section` kind falls through every chain today, so it touches: `App.tsx` (render branch, header title, full screen on a phone), `phone-tabs.tsx` (a huddle sits under Talk), `app-sidebar.tsx` (`here()` lights Talk for a huddle; `CallBar`'s open goes to the huddle page) and `PhoneCallBar`'s `here` in `App.tsx`. On desktop, Back returns to Talk with its list.

**Finding a huddle.**
- The start and end posts (`HuddleStarted`, `HuddleEnded` in `huddle.tsx`, from `conversation.tsx`'s `PostRow`) show the title and open the page.
- Talk gets a Huddles list for the picked space (`talk-page.tsx`), newest first: title, place, people, length, live.
- `talk_list` shows the title.

**Titles.** Anyone in the conversation can set one. If nobody has, the note-taker proposes one from the first couple of minutes (§6), and updates it at the end. A title set by a person is never replaced by an assistant.

**The #1038 prototype** (a clickable mock, PR #213) was built beside this design and differs on purpose in three places: its assistants have invented names (Pip, Kit), every assistant writes at the end, and its canvas is a plain list grouped by kind. This doc is the build plan. Worth taking from it: an item records who asked and why the assistant looked (§8), tasks are marked "proposed, not a card", and the phone tabs.

## 4. Who gets what

| Who | Live feed pushed? | Can read | Can write |
|---|---|---|---|
| People in the call | Captions, canvas | Everything | Title, canvas |
| People in the space, not in the call | No | The page, the transcript, the canvas | Title, canvas |
| Office assistants of people in the call | **Yes, always** | The huddle and a packet of the space's open work, with no tools (§7) | Canvas items, as open; the note-taker also proposes a title |
| Ordinary agents in the space | **No** | `talk_read`, `huddle_status`, `huddle_canvas` | Canvas items, as open, with `huddle_note` (§11) |

"Always connected" needs no switch ([decision 52](../decisions.md)): an assistant connects when its person joins. Settings only turn it off (§12).

## 5. The assistant's name

**Storage.**
- `assistant.name` private row: `{name, by, at}`, actor `user`, newest wins, `Replicated, Refuses`, set in the app only (the shape of `assistant.names.set`).
- Other people's devices need the name to display and match it, so it also rides `member.profile` as an unsigned field (`assistantName`, outside `personSig`), as colour, avatar and GitHub login do. The profile refresh (`keep_current` in `boards.rs`) compares it with the private row and rewrites the profile on a difference. `assistant.name` isn't a `person.*` kind, so the board-identity thread won't run `keep_current` for it: the private-log follow (`service.rs`) runs `keep_current` on every board when an `assistant.name` row arrives, as `check_desks` does for the GitHub login. A person's older device that refreshes its profile writes it without the field, so members may lose a chosen name until a newer device rewrites it.
- **No row means the default, "\<profile name\>'s assistant"** ("Addam's assistant"). It is computed, so nothing is written. It uses the profile name, else the membership name, as `space_names` does, so devices agree once the newest `member.profile` has reached them. It's one per person, it follows a rename of the person, and it's how people already talk. #1030 uses the same default and the same row: there is one name store for both designs, owned by card 4 here.
- A chosen name is 1 to 24 characters, letters and spaces. Set in Settings › Agents. The chat header and the assistant's system prompt use it.
- **Shown as "Juno · Addam's assistant"** wherever others see it (canvas, roster). "Addam" comes from the person who signed the row, through their `member.profile`, never from the row's text. So nobody can write under someone else's assistant's name, and a chosen name can't pose as a person. With no chosen name it's just "Addam's assistant".

**Matching it in speech.** A plain Rust function over each line, no model. Whisper mishears names, so matching is loose in spelling and strict in position:

1. Lower-case, strip punctuation, split into words. "Addam's" becomes `addam` plus a possessive mark.
2. A word matches a name if it's equal, or within edit distance 1 for names of 4+ letters. Names of 5+ letters also match on the same phonetic key, but only within edit distance 2: Soundex puts Juno, John, June and Jean together, so short names never match phonetically. A word that is the name of a person in the huddle never matches an assistant's chosen name. A two-word name matches as two adjacent words. Whisper writes "Adam's assistant" for "Addam's assistant": rule 2 covers it.
3. **The forms that call an assistant:**
   - its chosen name ("Juno");
   - \<person\>'s assistant, where \<person\> is the first word of their profile name or the whole of it ("Addam's assistant", "Addam Lee's assistant"), with or without a chosen name;
   - "my assistant", from its own person only;
   - "\<person\>'s Juno", a person's name before a chosen name, to pick one of two Junos. Tried first.
4. **Address position only:** within the first three words of the line, right after `hey`, `ok`, `so` or `and`, or at the line's end ("…right, Juno?"). "I was telling Addam's assistant yesterday" doesn't count.
5. **Spoken to, not spoken about.** The form must be followed by a comma, a `?`, the end of the line, or words that address it ("you", "can", "could", "do", "what", "who", "find", "check", "put", "add", "is there"). It doesn't count when a third-person verb follows ("said", "says", "thinks", "is", "was", "told", "posted"). So "Juno, who's on signing?" calls it, and "My assistant said we should…" and "I'll ask my assistant later" don't.

Card 6's tests carry those lines as negatives, plus "June third is the deadline" and "John, can you…" with a Juno in the room.

**Two assistants answer to one form.** Two people both called Stephen, or two Junos. The speaker's own wins. If neither is theirs, no look runs and a note goes on the canvas: "Two assistants answer to Juno. Say whose." Every matching host sees the clash; the one whose person id sorts first writes the note. Setting a name that matches another member's name, another assistant's, or a common English word shows a warning but is allowed.

Seeding Whisper's prompt with the roster's names would help further (`crates/voice` has no such option yet). It's a tuning step, not v1.

## 6. When the assistant is invoked

**The answer to "every 5 seconds?": never on a timer.** Lines are collected in Core at no cost, and the model runs only when a layer fires. The cheapest check is first and every layer but L3 needs nothing from the model to decide.

| Layer | Fires when | Who | Decided by |
|---|---|---|---|
| **L1 Addressed** | A line calls the assistant (§5), or its own person uses Ask on the page (§3) | That one assistant | Rust match |
| **L2 Own person asks** | A line from the assistant's **own** person ends in `?` or starts with a request stem ("do we", "can someone", "let's make", "who's", "what's"), and isn't one of the skips below | That person's assistant only | Rust rule |
| **L3 Digest** | At least 30 s of new talk since its last look, and then a natural gap: 4 s with no line, or the speaker changes. Forced at 90 s of new talk. Never without new talk. | The note-taker only | Rust |
| **L4 End** | `over(call)` holds (§3) and no `summary` item exists yet | The note-taker: a summary, the action items (open `task` items), the title, and a list of open items that look answered or superseded, by item id, for a person to settle (§9). It changes no status. Others stop. | Rust |

L2 is what makes the person's own example work: "do we have anyone working on X?", said with no name, goes to the asker's own assistant, and one question wakes one assistant. A question for the room from someone else wakes nobody, and lands on the canvas through the digest as an open question.

**L2 skips** lines of under 4 words, tag questions from a fixed list ("right?", "yeah?", "make sense?", "okay?"), and lines with a person in the huddle named in address position ("Addam, did you push it?"), since those are questions to a person. An L2 look writes the line as a `question` item and replies to it only when the packet supports an answer; otherwise it writes nothing. "Couldn't find it" items are for L1 and Ask, where someone asked the assistant.

L4 runs off `over`, apart from #656's end wake (`call_end_bridge`, `tell_call_end`), which stays as it is. It runs once per huddle: a huddle that already has a `summary` item never runs it again, so an old huddle that crashed without an end mark doesn't fire L4 at every start.

**A look is the turn of the held session.** Each assistant keeps one running `claude -p` for the huddle, and each look sends only the lines since its last look. There's no turn between looks, and it costs nothing while it waits. Details in §7.

**Rules that keep it bounded:**
- One look at a time per assistant. Lines that land during a look go in the next one.
- L1 from the assistant's own person isn't throttled apart from that. L1 from anyone else: at most one look per caller per 15 s, since each look spends the owner's usage and writes a placeholder (#1030 limits other people's asks the same way). L2 at most every 15 s. L3 at least 30 s apart.
- **Only fresh lines trigger.** A line triggers L1 or L2 only if it ended within 60 s of now. A host back from sleep or a dropped network gets minutes of rows at once through the same live path; those only join the buffer as context.
- **Hold for the rest of the sentence.** Whisper cuts phrases at 800 ms of silence, so "Juno, do we have anyone" and "working on signing?" can be two lines. After a match, wait for that speaker's next line or 1.5 s, whichever is first. A phone speaker's next line can take 5 s, so a line from the same speaker within 8 s of a look starts one more.
- A cap of 300 looks per huddle per assistant, other people's L1 included. Past that, L2 and L3 stop, the owner's own L1 goes on, and others' calls get one canvas note: "Stephen's assistant has done its looks for this huddle." The roster says so too.
- **Canvas items never trigger anything.** Only spoken lines (and Ask) do. Assistants can't set each other off.

**Where the check runs.** The thread proposed the speaker's device. It has to be the **assistant's host** instead: my device can't run your assistant, and every Core serves only its own person's agents. The host sees the line anyway, because the row reaches every member device. Each host evaluates the same lines against the same replicated names, so the layers need no coordination. (The host pick and the note-taker do: §7 and below.)

**Ambient fill: one note-taker, not everyone.** The thread's lean was every assistant on, posting only about what it knows. This design picks **only one assistant, the note-taker**. Join order isn't recorded anywhere replicated (`Live.devices` is an in-memory map keyed by device id), so the role comes from rows:
- The starter's assistant takes it (the start mark's author), if it's connected. Its host writes `call.assist` with `notes: true`.
- Otherwise, or when its person leaves while others are still in the call, or its `call.assist` is off, or its host has been away in presence for 60 s: the next connected assistant in the order of their first `call.assist on` row for this huddle (ties by person id) takes it, and writes `notes: true`.
- If two connected assistants both hold `notes: true`, the one earlier in that order keeps it; the other's host writes `notes: false` and stops digesting.
- If no `summary` item appears within 3 minutes of `over`, the next connected assistant in that order writes it.
- If no assistant is connected at all, there are no notes, and the page says so.

Why one rather than every assistant (to confirm with the person):
- "Only what it knows" can't be checked by Core, and a model told to post when relevant will narrate. Several assistants reading the same words would write the same notes.
- It costs one assistant's digest, not N.
- What other assistants uniquely know still arrives, through L1 and L2, which are always on for everyone.

## 7. The assistant in a huddle

It's the Office Assistant, not an agent: still off the roster, not counted against agent limits, and no cards handed to it.

**Which device hosts it.** The same computer #1030 uses for the person's assistant, so one person's assistant never runs in two places: the first device in `AgentDevices.homes` (what `coord.agent_home("")` reads, ranked by `refresh_agent_devices` from each device's replicated `spawn.host` row and mesh state) that `agents_paused_on` doesn't hold (Stop here, Stop everywhere).
- Each device's view of who is online is its own, so two devices can both pick themselves. **`call.assist` arbitrates:** the host writes `on` when it connects and `off` when it stops. A device that sees an `on` for this huddle from another of its person's devices that ranks above it stands down and writes `off`. An `on` whose device has been away in presence for over 60 s counts as off, the roster shows "reconnecting", and the next device takes over. A host that wakes from sleep re-runs the pick before any look.
- `spawn.host`'s `no_agents` says Claude Code is installed, not signed in. A signed-out Claude Code fails the first look: the host writes `off` and the roster shows "not connected".
- If no device qualifies (a person on a phone with their computer off) the roster shows "not connected: needs a computer that's on".

**The held session.** A new kind of run beside `Job`:
- The flags of a no-tools job (`job_args` with `tools: false`, as the thread namer and weigher use): `--no-session-persistence`, `--strict-mcp-config` with no `--mcp-config`, `--tools ""`, `--permission-mode dontAsk`. No assistant secret is written. Add `--input-format stream-json`, with stdin kept open.
- Started when the person joins, with a **priming turn**: the system prompt and the packet (below), its reply ignored. That turn writes the cache (one look's cost per assistant per huddle), which is what makes the first real look warm.
- It has its own pool, not `MAX_RUNS`.
- Each look writes one user message and reads until `result`, with a 60 s watchdog per look. The process stays up after `result`, unlike today's `Runner`, which closes stdin, ends at the first `result` and kills the run at `job.timeout`.
- If it dies, or its context passes about 60k tokens, the next look starts a new one seeded with the title, the canvas, the last 5 minutes and the current packet.
- **When it closes** (`close`, `call.assist off`): when its person leaves the call. The note-taker's stays open until L4 has written its summary, or 5 minutes after `over`; a person leaving at the very end doesn't hand the role on.
- An assistant with no look for a long time (not the note-taker) gets at most the last 10 minutes of lines and the canvas in its next look, not everything since it joined.
- **Where it lives.** The process side goes in `crates/window/src/assistant.rs` (reusing `prepare`, `command`, `kill_tree` and `Parser`), behind a new Core trait, `HuddleRunner { open, look, close }`, installed from `app/src/shell/mod.rs` on desktops, as `set_namer` is. The packet, the block parser and the rows stay in Core.
- Model and effort: the chat's, once #1014 lands (`spawn_setting("").assistant_model()` and `assistant_effort()`, default Sonnet low, passed as `--model` and `--effort`). Until then `assistant::MODEL` (Haiku).

**Measured** (spike, 2026-11-08, Claude Code 2.1.319, Haiku, no tools, no MCP, about 5k tokens of system prompt and talk; four looks each way, costs at API rates):

| | Time per look | Cost per look |
|---|---|---|
| Held session, after the first look | 1.1–1.9 s | $0.00013–0.00026 |
| Held session, first look | 4.1 s | $0.0012 (writes the cache) |
| Fresh `claude -p` per look | 3.7–5.0 s | $0.00040–0.00049, growing with the talk |

Not measured: a 40-card packet, an hour-long session (the context grows with every line, and every look re-reads it from the cache), and Sonnet on low effort (#1014's default).

**A trust boundary.** A transcript line is untrusted: anyone in the call can say anything, and a phone-transcribed line is no proof of who means what.
- Lines reach the model as data, one line each, marked with the speaker. They're never `trust="user"`, even for the person's own voice.
- The packet and the canvas carry other people's words too (card titles, other assistants' and agents' items). They go in fenced as untrusted, as `inbox::fence` does.
- The assistant has **no tools at all**. It writes by putting blocks in its reply, which the Core parses. That's the `<<thread>>` pattern, and the Core binds every block to this huddle. The model can't name another huddle, space or card.
- So the worst a spoken instruction can do is put an open item on this huddle's canvas.

**What it knows: a packet, not tools.** The Office Assistant's chat reads **as the person** (`ASSISTANT_TOOLS`), which includes their private cards and notes, and a huddle's items go on a canvas other people see. "Juno, what's on Stephen's private board?", said by Addam, must not work. So a huddle look gets no reads at all. Instead the Core builds a **packet** from the folded view of the huddle's own space log and puts it in the session's first message:
- the space's open cards: ref, title, state, holders, at most 40, newest activity first;
- the agents working in the space, with the card each holds;
- the huddle's title and canvas so far.

Each later look carries the lines since the last look, plus cards that changed since then. That answers the person's own example, "do we have anyone working on X?", from the space's cards and holders. A question put to the assistant (L1, Ask) that the packet can't answer gets an open item saying so, for after the call.

**The rule, shared with #1030:** no turn that runs unattended and contains another person's words has person-level reads. A person's own chat may hold both, because nothing leaves it without that person's tap (§9).

| Huddle in | The packet comes from |
|---|---|
| A shared space's channel | That space's log |
| A Personal channel | The person's Personal boards and their spaces: only the person sees the canvas, so nothing read reaches anyone else |
| A person DM | The spaces both people are members of (a hidden DM space has no boards of its own) |

Compared with #1030 §6 ("on its own, only this space's cards; other shared spaces and private work need the owner's OK"):
- **No owner's-OK path at all**, on purpose: there's no moment in a live call to ask.
- **Wider in a person DM:** the packet spans every space both people are in, where #1030 keeps to the asked space. A hidden DM space has no boards, and both people's devices already hold every card in those spaces, which is #1030's own D6 test (only what the asker can already see). Agreed with #1030's designer.

It's also faster: no tool round trips (§12).

**Output.** One or more blocks, anything else ignored, so a chatty reply writes nothing (the spike's model did not reliably say NOTHING). At most 5 items per look, each at most 2000 characters:

```
<<item kind="answer" reply="ITEM-ID">>Addam has #412 on Windows signing.<</item>>
<<title>>Release planning and the call canvas<</title>>
```

A block has no status: the Core writes every assistant item as `open` (§8), except `summary` and `note`, which have none. A `summary` block may carry `refs="id,id"`: the open items it judges answered or superseded.

**The firm rule, in the code.** No block makes a card, task or anything else, because no such block exists, and the look has no tools. The Office Assistant's corner chat can still draft a thread for the person to post, on their own tap, as today.

**Fast acknowledgement.** On an L1 match, before the model runs, the host writes a placeholder item: "Juno is looking: do we have anyone on signing?". The answer replaces it, or it says "Juno found nothing". Everyone should see that within about a second of the line landing (not measured), which matters more to how it feels than the few seconds behind it. On a look's timeout the host sets it to "Juno couldn't answer". A new host of the same person closes the old host's placeholders that are still "looking" when it connects.

**De-duplication.** Mostly structural: one digest writer, and one addressee per ask. On top of that, each look's message lists what others added since the last look, and the Core drops an item whose normalised text equals one already on the canvas.

## 8. Rows

All four are `call.*`, `Replicated, Accepts` (a `call.*` kind must never be `Refuses`). Older devices store them unread (`call.` is a tolerated family since `tolerant` 2), so no protocol bump is needed. Each new kind is registered in: the `kinds!` table (`crates/store/src/kinds.rs`), `coord::talk::KINDS`, `SPACE_TALK_KINDS`, `dm_conv_of` (`dm_spaces.rs`) and `dm_refusal`'s kind list (`spaces.rs`), the fold, and the golden test. `map_talk_keys` (`spaces.rs`) maps `call.item`'s `asker` between this device's key form and the space log's, as it maps `by`; it carries only people, as `call.people` does.

```json
call.title    {"callId","convId","title","assistant":false,"by","at"}
call.item     {"itemId","callId","convId","kind","text","status","reply":"<itemId>?","refs":["<itemId>"]?,"asker":"<person key>?","why":"name|own|digest|end|ask?","assistant":false,"by","at"}
call.item.set {"itemId","callId","convId","text?","status?","x?","y?","card?","assistant":false,"by","at"}
call.assist   {"callId","convId","device","state":"on|off","notes":false,"by","at"}
```

- **`kind`:** `question`, `answer`, `decision`, `task`, `note`, `summary`.
- **`status`:** `open`, `settled`, `dropped`, `carded`, shown by kind (a question reads open or answered, a decision proposed or agreed). `summary` and `note` have none.
- **`asker` and `why`:** who started the look and what woke it, filled by the Core from the line or Ask that started it (empty for a digest). When Addam calls Stephen's assistant, the row is signed by Stephen, so without `asker` nothing records Addam. Display only, never authority. The canvas groups items under `asker`, else the signer.
- **`refs`:** on a `summary`, the open items it judges answered or superseded (§6 L4). §9 hands them to the person to settle.
- **`call.item.set`** folds per field, newest wins. Only the writer's person edits `text`, and an assistant's set edits only its own `assistant: true` item's text.
- **The fold binds each `callId` to its conversation**, as the `call.segment` fold does (`call_idx`), and drops a `call.*` row whose `convId` isn't the huddle's. `space_may_touch` alone checks only that the conversation belongs to the carrying space, so without this a row in one space could add items to another conversation's huddle.
- **New kinds, read only by this fold.** No other fold reads them: a `task` item is never a board task, and nothing on the Board, in agent starts or in the inbox looks at canvas rows.

**Who may write what.** The signature proves only the person (`bind_actor`), so the rules rest on the actor and the flag:

| Kind | Actor | What counts |
|---|---|---|
| `call.title` | A person (`by` = actor) | A person's newest title beats an assistant's (`assistant: true`) |
| `call.item` | A person; or, outside hidden spaces, an `agent:<card>` (`by` = actor) | Status reads as `open` unless the row is a person's with no `assistant` flag |
| `call.item.set` | A person (`by` = actor) | `status`, `card`, `x`, `y` count only from a person's row with no `assistant` flag; on any other the fold drops them |
| `call.assist` | A person (`by` = actor) | |

- **An assistant's row is never the person's decision.** An assistant writes items as `open` only. Its rows are stored and replicated like any other; the fold just ignores a status on them. The real guarantee is that the block parser never writes a status (§7); the fold check is the backstop. During the call a question with an answer shows as "open · 1 answer" until a person settles it.
- **Agents never settle or move anything.** An ordinary agent's `call.item` is open, and its sets don't exist.
- **Append-mostly:** nothing is deleted. `dropped` is a status.
- **`card`** is a plain ref string set when an item becomes a card (§9). It isn't `taskId`, because `store::board::resolves` refuses a `taskId` that isn't on that board, and a DM's cards live on the promoter's Personal.
- **Writes after the fact are checked where they're written.** The Core's write path and §9's Make cards refuse `carded` and `card` while `over(call)` is false. The fold never checks liveness: `Live` is per-device memory, so a fold that used it would come out differently on two devices, or on a refold. The fold honours a person's `carded` whenever it comes.
- `space_talk_row` in `spaces.rs` gets those actor rules per kind. Today it drops any `call.segment` not from a person signing as itself; that check stays on `call.segment`, and the table above adds the rest. Widening it to the whole `call.` prefix would silently drop every agent's `huddle_note`.

**"Agent-written" is a flag.** Assistant rows are written by the person's device under the person's own key, with `assistant: true`.
- That is as provable as the alternative. Any of a person's devices can already sign as any of their agent cards, so an `agent:<card>` actor proves nothing more.
- It keeps the person-DM door nearly closed: `dm_conv_of` keeps "written by one of the pair under their own key" and only gains kinds, with no agent identity in hidden spaces and no new mint path around `card_for`'s roster check.
- It's shown on every item, always, as "Juno · Addam's assistant" with an agent mark. An item with `assistant: true` is never shown as the person's own words.
- **#1030 differs on purpose.** There, the same assistant posts in `#office-assistants` as an `agent:<card>` card of kind `assistant`, pointed at by `member.profile.assistantCard`. Huddle rows don't use that card: a hidden DM space and the private log hold no cards, and one rule then covers all three places. Both label the same way, from the verified signer's profile and `assistantName`; card 4 adds one label helper, and the canvas and #1030's posts both use it. #1030's "never `person:`" covers its channel posts only.
- Ordinary agents write with their own `agent:<card>` actor (§11), in shared spaces and Personal channels only.

**The gate.** Today `append_space_talk` refuses a `call.segment` while `Core::space_tolerance(space).ready()` fails, on every append, and a call reads it once on entering (`Script.rows`) to choose rows or posts. Widen that tolerance check (not `space_talk_row`'s) to the `call.` prefix. Personal channels have no gate. The consequences in a space that isn't ready:
- **No title, canvas, assistant or L4, during or after the call.** A refused canvas or title write is dropped and logged. The page says "Agents and the canvas are off: someone's Irori in this space needs updating", and the notice says the same at the start.
- The transcript still arrives at the end, as fallback posts, and Divide works from those.
- If a speaker's writes start failing mid-call, their lines fall back to posts, which aren't `call.segment` and never reach the feed. The roster says "Addam's lines reach assistants only at the end".

**The feed.** One path, the rows. A host need not be in the call (a person on a phone has their assistant on their computer), and `call.segment` already reaches every member device.
- Call `Core::huddle_line(row)` after the fold on the live paths: the own write (`calls.rs`, via `talk.write_segment`), the space follow (`follow_board_row` in `boards.rs`) and the private-log follow (`service.rs`). Refolds (`Talk::load`, `fold_space_talk`) never call it, so the refold at start feeds nothing. Rows folded by a resync mid-call aren't fed either.
- The follows also get this device's own writes, so `huddle_line` dedupes by line id (`seg_<call>_<device>_<seg>`) and item id. It drops rows of huddles this host isn't serving.
- **In a person DM, it passes only lines with `agents: true`** (§10). A line from a peer on an older build, or written under the old notice, never reaches a look, the packet, the seed or the summary. The roster says "Addam's lines aren't heard by assistants (older Irori)".
- Freshness (§6): only a line that ended within 60 s triggers a look.
- A device in the call also gets frames sooner, but the row hop is small next to the look, so a second source isn't worth its dedupe. If card 13's measurement says otherwise (§12), add it then.

## 9. After the call

Promotion is one step, and only after the huddle is over.

1. The page's canvas becomes **the finished canvas**: items grouped by kind, open questions and proposed tasks first, with the note-taker's summary on top (the action items, and the items its `refs` judge answered or superseded, with **Settle these**, which writes the person's own `settled` sets after they've seen the list).
2. **Divide with my assistant** opens the Office Assistant chat with this huddle in context (any person can, whether or not they were in the call). They talk it through: merge these, split that, this one's Addam's. This chat keeps its person-level reads while holding other people's words, which §7's rule allows only because the person is there: the canvas and transcript go in fenced as untrusted data (as `inbox::fence` does), and nothing leaves the chat until the person taps Make cards, seeing the exact text first.
   - The chat keeps no session: each turn re-renders the history (`render_prompt`), and `ASSISTANT_TOOLS` has no `talk_read`. So `assistant_ask` takes an optional `callId`, and the host puts the fenced huddle context into every turn while the chat is on that huddle: the title, the canvas with item ids, and the transcript cut to the last 20 minutes when it's longer.
   - The chat needs Claude Code, so Divide is hidden on a phone (§2).
3. The assistant ends its reply with draft blocks, in the `<<thread>>` pattern:
   ```
   <<cards>>
   {"title":"…","objective":"…","items":["<itemId>"],"board":"…"}
   <</cards>>
   <<settle>>{"settle":["<itemId>"],"drop":["<itemId>"]}<</settle>>
   ```
   `splitReply` takes both blocks out alongside `splitDraft`, before `splitGo`, so a go line inside them never fires. The block's `board` is a suggestion: the board picker sets `NewCardInput.board` and the space board.
4. The chat shows the drafts with **Make cards** and the settle list with **Apply**. Nothing is made or changed until the person taps.
5. Each card goes through the normal create path, then writes `call.item.set{status:"carded", card:"#123"}` as the person, so the canvas shows what became what. Apply writes the person's own `settled` and `dropped` sets. That's how the many items an assistant left open get closed in one tap, rather than one by one.

The #1038 prototype puts the proposed cards on the huddle page as a checklist instead of in the chat. This design keeps the chat because the person asked to "work with their own assistant" on dividing it; a checklist view of the same drafts can come later.

**Where cards go.** A shared-space channel's huddle goes to that space (board picker). A Personal channel's goes to Personal. A person DM's goes to the promoter's Personal, since a hidden space has no boards, or a space both people are in, by the picker. The other person can't open a card on the promoter's Personal, so on the DM canvas they see "carded by Addam (Personal)" with no link. Assignment and agent starts then follow the normal rules (decisions 40, 50).

Two people may divide the same canvas. `carded` shows up live, and the drafts leave out items already carded when the block is shown. A card made twice from one item is a duplicate the person can see and close.

## 10. Access and consent

**Rule: everyone in the space the huddle lives in, and all their agents.** Ordinary agents may read and follow it. Only assistants are pushed.

| Huddle in | Who, and their agents |
|---|---|
| A shared space's channel | Every member of the space |
| A Personal channel | The person |
| A person DM | **The two people** (the person's choice, below) |

**The person DM: decided.** On #1028 the person asked: "why shouldn't anyone else's agents get access? Even in realtime." What stands in the way is where the data lives, not a rule. A person DM sits in a hidden space whose only members are the two people, and it isn't in any workspace. No third person's device holds a copy of that log, so no third person's agent can read it, live or later. Asked on #1029, the person chose to keep it that way for now: a DM huddle reaches the two people and all their agents, live (which drops #656's rule), and someone who wants the workspace's agents to hear a call huddles in a channel. Holding DM huddles in a shared space instead was the other option, set aside because a one-to-one call would then never be private to two.

**Changes to #656's agent rules:**
- `call_parent` no longer refuses a person DM's `call:<id>`. Deleting the refusal isn't enough: the reader then falls through to `can_read` and `posts`, which refuse any agent ("that DM isn't yours"). So for a pair-DM parent, `call_parent` lets an agent of either person read `call:<id>` lines with `agents: true` directly, bypassing the DM's `posts` rule. The DM's own posts stay closed to agents.
- `talk_list`'s huddles section shows every huddle of the agent's spaces with its title, not only channels it joined, and includes DMs.
- `call_end_bridge`, `tell_call_end` and the hidden-space gate in `boards.rs` keep their checks: the end wake stays as it is for channel members, and **no wake is added for a person DM's agents.** Reading it by tool is enough (row 7 of §1's table).

**Earlier DM huddles stay closed.** Those people were told "written down for the two of you". Each `call.segment` row gains `agents: true` when it's written under the new notice. In a person DM, agents see only lines with the mark, so older huddles and lines from an older build stay closed. A channel needs no mark: its notice has always said "and their agents".

**The notice.** Before the mic opens (`transcriptNotice`, `frontend/src/lib/call.ts`), present tense:
- Channel: "Agents hear this huddle. What you say is written down as you speak, for #x's members and all their agents."
- DM: "Agents hear this huddle. What you say is written down as you speak, for the two of you and your agents."
- Personal: "Agents hear this huddle. What you say is written down as you speak, for you and your agents."
- Also fix a bug the readers found: a Personal huddle's Join shows the channel text, on two paths. `Names::conv` (`crates/window/src/call.rs`) knows only channel and dm, for the ring banner, and `askJoin` in `conversation.tsx` passes `conv.kind`, which is `channel` for a Personal channel. Start already maps to `personal`. The transcript's `conv_kind` (`crates/window/src/talk.rs`) has the same gap.
- `whoCanRead` (`call.ts`) says "the two of you and your agents" for a DM huddle whose lines carry `agents: true`, "the two of you" for older ones, and "you and your agents" for Personal.

During the call the "Transcript on" badge titles add "Agents hear this huddle", and the roster shows each assistant connected.

**Honest limits.**
- Turning off "Transcribe what I say in calls" is the only real off, because it stops the text being made. Nothing else can be enforced: peers receive each line, and one device can't bind another person's agent.
- A transcript is permanent on every member device, and an agent's reading is not private to anyone's call.
- The earlier "hide my lines" idea (`call-transcripts.md`, Open) isn't built, and the person's answer on segregation means it isn't planned.
- **For ordinary agents the firm rule is advisory.** Assistant looks have no tools, so they can't make work. An ordinary agent woken for something else can read a live huddle and call `board_create`, and decision 50 starts the card. Its read tools all carry the "still going" line (§11); nothing enforces it.

## 11. Tools for ordinary agents

Nothing is pushed. They reach for these when they need them.

- `talk_read conv=call:<id>`: as today, with speakers named. Another person prints today as "Addam (person:…, gone)", because `Names::who` (`crates/tools/src/lib.rs`) treats any key off the agent roster as a gone agent: name people without the "gone" tag. While the huddle is live, its result ends with: "This huddle is still going. What's said may change: don't start work from it until it ends."
- `huddle_status {call}`: title, live or over, people, whose assistant is connected, line and item counts. Read-only.
- `huddle_canvas {call}`: the items, paged. Read-only.
- Both end with the same "still going" line while the huddle is live.
- `huddle_note {call, kind, text, reply?}`: adds one `open` item with its own actor, in shared spaces and Personal channels. Refused in a person DM, and under Ask me autonomy. It can't make a card, settle or move anything (§8): with nothing pushed to them, agents write only when something asked them to.

Listing rules follow `talk_read`: the same `call_parent` check. `talk_read`'s cursor moves before the result is clipped to 8 KiB, so clipped lines count as read. `huddle_canvas` must budget before it advances.

## 12. Latency and cost

**What it will feel like: a colleague taking notes, not someone in the conversation.**

| Step | Mac speaker | PC speaker | Android speaker |
|---|---|---|---|
| End of speech to a finished line (800 ms pause + Whisper final) | about 0.9 s (estimated) | about 1.9 s or more (estimated) | 4.5–5.1 s (0.8 s plus the measured 3.7–4.3 s final) |
| Line to the assistant's host (row) | estimated under 1 s | same | same |
| Hold for the rest of the sentence | up to 1.5 s | same | same |
| The look: no tools, the packet already in the session | 1.1–1.9 s measured with about 5k tokens; estimated 2–4 s with a 40-card packet | same | same |
| Item to everyone's canvas (row) | estimated under 1 s | same | same |
| **Spoken to answer on canvas** | **about 4–8 s** | **about 5–9 s** | **about 8–13 s** |

Only the Android final and the look itself are measured; card 13 measures the rest, Mac and PC finals included. The acknowledgement (§7) should show at about 2–3 s (about 7 s from a phone). A busy machine, or a cold session (+2–3 s), can push the answer towards 30 s.

Finals only is a hard floor: a word isn't known until its phrase ends, and unbroken speech is cut at 25 s (`MAX_PHRASE`). The floor belongs to the **speaker's** device: a Mac listener can't speed up a phone speaker's lines.

**What would speed it up, best first:**
1. Keep the session warm (already in the design).
2. No tool calls per answer: the packet is in the session (already in the design, §7). A tool round trip would add an estimated 1–3 s each.
3. A smaller Whisper model on Android for calls. It's the biggest single number, and it costs accuracy.
4. Partials for address detection only: `Options.partials` exists, but needs a new frame type, and costs a pass per 700 ms, which nearly fills the phone. Not v1.
5. Taking live frames on an in-call host: saves only the row hop.

**Cost.** Every look spends the owner's own Claude usage, with one assistant per person in the call. From the spike, a held look on Haiku with about 5k tokens is a fraction of a tenth of a cent. An hour isn't measured: every look re-reads the whole session from the cache, so the cost per look grows with the call, bounded by the restart at about 60k tokens (§7) and the 300-look cap. On Haiku that's an estimated cents to tens of cents per assistant-hour, and Sonnet on low costs more. Card 5 records a one-hour run. Either way it fits decision 52: on by default, a setting to turn it off.

**Settings** (Settings › App, on by default; the setting only turns things off; stored as private rows like `assistant.names.set`):
- "Connect my assistant to huddles": off means no assistant from this person.
- "Let my assistant take notes in huddles I start": off passes the note-taker to the next person.

## 13. Build order

Dependencies, not preference: the place first, then who can read, then the names and the assistant's engine, then the feed, the canvas on top of it, and the after-call step last.

| Card | Board | Comes after |
|---|---|---|
| 1 Huddle rows and "over" | #1043 | — |
| 2 Huddle page and list | #1044 | 1 |
| 3 Access, names in reads, the notice | #1047 | 1 |
| 4 The assistant's name | #1045 | — |
| 5 Held run and the packet | #1046 | — |
| 6 Matcher and layer rules | #1048 | 4 |
| 7a Host and `call.assist` | #1049 | 1, 5 |
| 7b Feed, L1 and L2 | #1050 | 6, 7a |
| 8 Canvas rows and writes | #1051 | 7b |
| 9 Canvas UI and the roster | #1052 | 2, 8 |
| 10 Note-taker: L3 and L4 | #1053 | 8, and the person's note-taker answer (§6) |
| 11 After-call: divide into cards | #1054 | 9, 10 |
| 12 Ordinary agents' tools | #1055 | 3, 8 |
| 13 Real-device check | #1056 | 11, 12 |

The board cards are proposed and wait on #1029; each also waits on the cards it comes after. 1, 4 and 5 can start together. Looks in a person DM stay off until card 3 writes the `agents` mark. Cards are proposed, not started. All are normal weight unless marked. Every card's done-when includes `sh scripts/check.sh` passing.

**1. Huddle rows and "over".**
Files: `crates/store/src/kinds.rs` and `crates/store/tests/it/golden.rs` (`call.title`), `crates/coord/src/talk.rs` (`call.title` fold, `CallInfo` title and people, the crash roster from segment signers, "has an end mark"), `crates/core/src/service/calls.rs` (`over(call)` and its own `heard` record, §3; the crash end mark is card 10's), `crates/net/src/call.rs` and the `call.segment` fold (the `agents` field, stored and folded, written `false` until card 3), `crates/core/src/service/spaces.rs` (`SPACE_TALK_KINDS`, `dm_refusal`, the tolerance check widened to `call.`, `space_talk_row`'s per-kind actor rules), `crates/core/src/service/dm_spaces.rs` (`dm_conv_of`), `crates/window/src/talk.rs` (title and over in the view).
Done when: a title set by a person in a channel, a DM and a Personal channel reaches every member device, and beats an assistant-flagged one; tests show `over` true after an end mark, true for a call whose last device crashed within about 3 minutes (in a two-device space too), and never true on a device that starts or reconnects mid-call; a `call.title` whose `convId` isn't the huddle's is dropped; in a space that isn't ready, the title write is refused and logged.

**2. Huddle page and list.**
Files: `frontend/src/lib/nav.ts` (`#huddle/<callId>`), a new `frontend/src/components/talk/huddle-page.tsx`, `huddles-list.tsx`, `talk-page.tsx` (mounts the list), `huddle.tsx` (`HuddleStarted`, `HuddleEnded` show the title and open the page), `conversation.tsx` (`PostRow` passes the title), `transcript-pane.tsx` (a variant that fills its column, live reload), `App.tsx` (render branch, header title, phone full screen, `PhoneCallBar`'s `here`), `phone-tabs.tsx`, `app-sidebar.tsx` (`here()`, `CallBar` opens the page).
Done when: the page shows title (editable), place, live or length, Join, roster and transcript for a live and an ended huddle; the transcript follows new lines while live; the Talk list shows huddles newest first with titles; the sidebar lights Talk and the phone keeps the Talk tab; screenshots at desktop and phone width (`bun run preview:look`), light and dark, on the card.

**3. Access, names in reads, the notice.** The DM rule is settled (§10): the two people and their agents.
Files: `crates/tools/src/talk.rs` (`call_parent` lets either person's agents read `agents: true` lines of a DM huddle, bypassing `posts`; `huddles()`), `crates/tools/src/lib.rs` (`Names::who`: a `person:` key isn't a gone agent), `crates/core/src/service/calls.rs` (writing the `agents` mark `true` under the new notice), `frontend/src/lib/call.ts` (`transcriptNotice`, `whoCanRead`), `frontend/src/components/talk/huddle.tsx`, `conversation.tsx` (`askJoin`'s kind), `crates/window/src/call.rs` (`Names::conv`), `crates/window/src/talk.rs` (transcript `conv_kind`), `docs/design/call-transcripts.md`.
Done when: an agent of either person reads a new DM huddle, is refused an older one, and is still refused `talk_read` on the DM itself; `talk_list` shows every huddle in its spaces, with titles; speakers print as names; no new wake happens; the notice and "who can read" read as §10 for a channel, a DM and a Personal channel, on both Start and Join; in a space that isn't ready, the start notice says agents and the canvas are off; tests cover each.

**4. The assistant's name.**
Files: `crates/store/src/kinds.rs` and `crates/store/tests/it/golden.rs` (`assistant.name` in `SINCE_GO_REFUSED`), a new `crates/core/src/service/assistant_name.rs`, `crates/core/src/service.rs` (the private-log follow, as `assistant.names.set` has, calling `keep_current` on every board), `crates/identity/src/objects.rs` (`MemberProfile.assistant_name`, outside `personSig`), `crates/identity/src/board.rs` (`MemberFold` field, `profile_assistant_name`), `crates/identity/src/person.rs` (the row builder), `crates/core/src/service/boards.rs` (`keep_current` compares the name), `crates/window/src/assistant.rs` (prompt and header), Settings › Agents, `frontend/src/components/assistant/assistant-chat.tsx`, a new `frontend/src/lib/assistant-label.ts` (one label helper: the signer's profile plus `assistantName`, used by the canvas and #1030's posts). #1030 reads the same name, and adds its own optional `assistantCard` field on `member.profile` beside `assistantName`; whichever card builds first adds only its own field.
Done when: a person can set a name and see it on all their devices and on another member's, shown as "Juno · Addam's assistant" with the person taken from the row's signer; a person who hasn't reads as "Addam's assistant" everywhere with nothing written, and follows a rename; #1030's agent has this card's number (its label reads `assistantName` as a one-line follow-up).

**5. Held run and the packet.**
Files: `crates/window/src/assistant.rs` (the held process: `job_args` with `tools: false` plus `--input-format stream-json`, stdin held open, a watchdog per look, no end at `result`), `crates/core/src/service.rs` (the `HuddleRunner` trait and its setter), `crates/core/src/service/thread_names.rs` (the seam it copies), a new `crates/core/src/service/huddle_packet.rs` (the packet from a space's folded log, taking the title and canvas as inputs, empty until cards 1 and 8 land; share the space-cards reader with #1030's `assistant_answers.rs` if that lands first, keeping each design's selection), `app/src/shell/mod.rs` (installing it on desktops).
Done when: a held session takes a priming turn and several looks, and restarts from the seed (packet included) after being killed or past the context limit; its arguments carry no `--mcp-config` and no secret; a packet test with a private card, a Personal card and a card on a space the other person isn't in contains none of them, for a channel and a DM huddle; packet text goes in fenced; time and cost per look are recorded in §7 for Haiku and Sonnet low with a 40-card packet, and for a one-hour run.

**6. Matcher and layer rules.** Pure Rust, no I/O.
Files: a new `crates/core/src/service/huddle_rules.rs` (the §5 matcher, the §6 layers, skips, throttles, hold, freshness, cap, the note-taker order from rows).
Done when: tests cover each §5 form (chosen name, "Addam's assistant", "my assistant" only from its own person, "Stephen's Juno"), near-spellings ("Adam's"), position, the vocative rule and its negatives ("My assistant said…", "I'll ask my assistant later", "June third…", "John, can you…" beside a Juno), collisions; L2's skips (short lines, tag questions, a question to a named person); one look at a time, the hold, other callers' throttle, the cap, and stale lines triggering nothing.

**7a. Host and `call.assist`.**
Files: a new `crates/core/src/service/huddle_host.rs` (the pick from `AgentDevices.homes` and `agents_paused_on`, `call.assist` arbitration, away after 60 s, re-pick after sleep, when a session opens and closes, §7), `call.assist` registration (`crates/store/src/kinds.rs`, golden test, `crates/coord/src/talk.rs` fold, `spaces.rs`, `dm_spaces.rs`), the roster state in `crates/window/src/talk.rs`.
Done when: two devices of one person end with one host, in a two-Core test that starts with both picking themselves; a host away for 60 s hands over; the host opens a held session (card 5) when its person joins and closes it when they leave; an assistant connecting shows in the roster state of a channel, DM and Personal huddle; a person with no computer on reads as "not connected"; a signed-out Claude Code reads as "not connected" after its first look fails.

**7b. Feed, L1 and L2.**
Files: `crates/core/src/service/calls.rs`, `boards.rs` and `service.rs` (`huddle_line` on the three live paths), a new `crates/core/src/service/huddle_feed.rs` (per-huddle buffer, dedupe by line id, freshness, the DM `agents: true` filter, firing looks through `huddle_rules.rs`).
Done when: in a two-Core test a line calling an assistant gets a look and a mid-line mention doesn't; another person's question wakes nothing; a refold at start and a burst of old rows after a reconnect run no look; an own write seen again by the follow feeds once; an unmarked DM line reaches no look.

**8. Canvas rows and writes.**
Files: `crates/store/src/kinds.rs` and golden test (`call.item`, `call.item.set`), `crates/coord/src/talk.rs` (fold, `callId` binding, actor rules), `crates/core/src/service/dm_spaces.rs`, `spaces.rs` (actor rules; `asker` in `map_talk_keys`), `huddle_feed.rs` (block to row, `asker`, `why` and `refs`, placeholders, de-dupe), `huddle_packet.rs` (title and canvas in the packet and seed; each look carries cards changed and items added since the last), `crates/window/src/commands.rs` and the generated bindings (`huddle_title_set`, `huddle_item_add`, `huddle_item_set`, `huddle_ask`, with the write path's `over` check), `crates/window/src/talk.rs`.
Done when: blocks become items on all members' devices in a channel, a DM and a Personal channel; an item asked by another person groups under them on every member's device; an L1 match shows a placeholder before the look, the answer replaces it, a timeout turns it into "couldn't answer", and a new host closes the old host's; Ask gets a look and an item; `dm_conv_of` still refuses anything not by the pair; an assistant-flagged or agent row's status, card or position is ignored, tested for both; a row whose `convId` isn't the huddle's is dropped; the write path refuses `carded` while `over` is false, and the fold honours a person's `carded` row whenever it arrives, with no liveness check; no fold but this one reads the new kinds.

**9. Canvas UI and the roster.**
The board canvas assumes cards at every layer, so either open these up or (simpler, recommended) make `canvas/huddle.tsx` its own React Flow view with its own node types and layout, reusing only `canvas/reactflow/flow.css` and `KeepCenter` (private in `canvas/reactflow/index.tsx` today: export it), and leaving the board's `data.ts`, `elk.ts` and `weights.ts` alone. If shared: the `GraphNode`, `EdgeKind` and `GroupNode.by` unions, `NodeSize[kind]`, `toElk`'s card-only filter, `layoutKey`'s fallthrough, `toNodes`'s position filter, `withPlaced`/`followers`/`groupRects`, `onNodesChange`'s card-only `onPlace`, `node.tsx`'s bodies and `openTarget`.
Files: a new `frontend/src/components/canvas/huddle.tsx` (and `huddle-elk.ts` if separate), `canvas/reactflow/index.tsx` (export `KeepCenter`), `huddle-page.tsx` (canvas, roster with each assistant's state, the Ask box, person writes), `docs/canvas-layout.md` (a "Huddle canvas" section).
Three traps on main that the card must avoid: `FitWhileUntouched` refits whenever its area count changes, and a drag doesn't count as touching; `LaidOut` resets all node state at render time when the graph object changes; a new node id changes the layout key and lays out again at once. So: build the graph from the items alone, memoized on their content, never on transcript lines; no render-time reset while a node is dragged; lay out append-stable (time order, existing items keep their spot). Item text runs to 2000 characters and a placeholder grows: clamp shown text to a few lines and expand on click.
Done when: people and their connected assistants head the canvas; items group under who asked and show the assistant's label; a person adds an item, settles or drops one, and a drag saves its position, all seen on another member's device; the Ask box shows only while the viewer's assistant is connected; the roster shows connected, taking notes, reconnecting, not connected, the cap, "lines reach assistants only at the end" and "older Irori"; the page says when there are no notes, and when agents and the canvas are off in a space that isn't ready; a test appends items during a simulated drag and the dragged node and the viewport are unchanged; it works at phone width and in light and dark; screenshots on the card.

**10. Note-taker: L3 and L4.** Waits on the person's yes to one note-taker (§6).
Files: `huddle_feed.rs`, `huddle_rules.rs`, `crates/core/src/service/calls.rs` (hand-on, the end mark after a crash), `crates/store/src/kinds.rs` and `crates/store/tests/it/golden.rs` (the two settings rows, in `SINCE_GO_REFUSED`), `frontend/src/components/settings/` (Settings › App).
Done when: only one assistant digests per huddle, chosen from rows, and two that both claim it settle on one; it passes on when its person leaves mid-call or its host goes away; no digest runs without new talk; a title appears after a couple of minutes and doesn't replace a person's; the end produces one summary with action items and `refs`, and changes no status; if the note-taker's host is gone at the end, the next writes it within 3 minutes; a crashed huddle gets one end mark, from an assistant's host or else any member device; both settings turn off what they say.

**11. After-call: divide into cards.**
Files: `huddle-page.tsx` (finished canvas, Settle these from the summary's `refs`, Divide), `frontend/src/lib/assistant.ts` (`splitReply` takes `<<cards>>` and `<<settle>>` out before `splitGo`), `assistant-chat.tsx` (drafts, Make cards, Apply), `crates/window/src/assistant.rs` (huddle context, fenced, cut to 20 minutes), `crates/window/src/commands.rs` and the generated bindings (`assistant_ask` takes `callId`), `crates/window/src/new_card.rs`.
Done when: from a finished canvas a person gets card drafts and a settle list with their assistant; Make cards makes them on the board the picker shows; Apply settles and drops in one tap; items show as carded (a link where the viewer can open the card, else "carded by Addam (Personal)"); nothing can be made while `over` is false; Divide is hidden on a phone; an agent starts only by the normal rules.

**12. Ordinary agents' tools (light).**
Files: `crates/tools/src/specs.json`, `specs.rs`, `talk.rs`, `crates/api/src/backend.rs`, `crates/tools/tests/it/`.
Done when: `huddle_status`, `huddle_canvas` and `huddle_note` work with §11's refusals; `huddle_note` items are open and can't settle or move anything; the live-huddle line appears on all three read tools; `huddle_canvas` budgets before its cursor advances.

**13. Real-device check. (needs:mac and a phone)**
Done when: a real huddle between a Mac and the Pixel, with the Pixel person's assistant hosted on a computer that isn't in the call, has each assistant answer by name; §12's table is filled in with measured numbers, Mac and PC finals and the row hop included; a live frame source is added only if the row hop is over 2 s.

## 14. Not in this design

- An assistant talking to other people's assistants (#1030). "Who knows about X?" in v1 is answered from what the space can see.
- Asking the room ("assistants, who knows…") and handing that between assistants. Later, by decision 38's rule.
- Voice addressing of ordinary agents, and cards or any other real work from a live huddle.
- Wake words other than the name, screen-share reading, and audio retention. No audio is kept, as today.
