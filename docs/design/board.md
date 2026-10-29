# The Board: design

*2026-10-27. Decisions 16–25. The prototype is the "Irori Board" artifact. Multi-person boards: research/06. This doc is the single-machine Board that ships first.*

## What it is

The Board is a **scratch pad where agents and people coordinate live work**: what's being done right now, what's in the way, and who picks up what next. It is not a tracker. Irori stays minimal and expansive by doing a few things well and linking out for everything else, never by growing into Linear, Notion or GitHub Issues.

**In:**
- tasks: a title, an optional objective and done-when, a state;
- blocked as a state with a reason (decision 17);
- comments (decision 18);
- shared tasks (decision 20);
- assignment (decision 21);
- leases shown as cards when they block something;
- links out.

**Out, on purpose:**
- priorities, estimates, story points, due dates;
- sprints, milestones, epics, sub-task trees (one level of subtasks is in: [Subtasks](#subtasks));
- labels, custom fields, workflow states beyond Irori's few, templates;
- reporting, burndown, velocity;
- a long-term archive you browse.

When a card needs any of those, it links to where they live.

## No retention rules

The Board doesn't time cards out. A card stays until someone marks it done or closes it, and done cards fold into a collapsed Done group, the way the prototype shows. There are no timers, stale badges or auto-archive. What keeps it from turning into a tracker is what it leaves out and what agents are told (below), not a clock.

## Instructions for agents

These are carried in the board tools' descriptions and in a short block the mod adds to the tool listing, so every agent sees them where it uses the Board:

> The Irori Board is a shared scratch pad for live coordination between running agents and people: what you are working on now, what blocks you, and hand-offs. Keep cards short.
> Do **not** use it for long-term information: plans, specs, decisions to remember, backlogs, meeting notes or documentation. Put those wherever this project or your instructions say (Linear, Notion, GitHub issues, the repo's docs, a knowledge base), and add a link on the card if it helps.
> When a card's work is done, mark it done; don't keep it as a record.
> A card the person posts or asks for is never backlog: it is work to start now (new work on an existing task, or new work to pick up immediately), so take it up or get it taken, don't leave it waiting.
> When you take a card, give it steps and keep them ticked.
> When you need the person, raise one blocker per question (never several asks in one note): context for a newcomer, options when there are real choices, and your recommendation marked.
> Coordinate about a card on the card: board_comment it, or name it as task_id when you send. Use direct messages only for things not about a card.
> When the person comments on a card you hold or watch, answer on the card with board_comment. One answer is expected: the holder answers, and a watcher answers only if asked.

*v0.4.2, from the first two-machine run:* an agent put three asks in one `input_required` note and the person saw one fat card. So `board_status input_required` is refused unless the card has an open blocker (and on a card that has them it changes nothing, since a state change would close them); the error says to raise each question with `board_block`. An option can be marked `recommended` (at most one per raise; a lone option is the recommendation; `board_recommend` always recommends, and marks an option already offered). The window shows the recommended one first with a badge, the others as options, and the field for an answer in the person's own words. Older `input_required` cards still show their note as one item.

The tools enforce what they can:
- a title is at most 200 characters, and an objective or comment at most 4000;
- a card's links are URLs or repo paths;
- there is no field for anything the list above leaves out.

## Card threads

Each card has one thread, oldest first: its comments, the messages about it, and its history lines, each with its author and device.

- **Messages about a card.** `send` with `task_id` sets the message's Ref to `task:<id>` and, once the inbox has it, records it on the card as `from -> to`. A reply keeps the Ref of what it answers (a `comment:` ref included), so it goes on the same card. The entry is a `task.comment` row whose comment id is the message id, so a retry, the sweep or another device never writes it twice. It sends no notice: the recipient already has the message, and the holders read the card. Text past 4000 characters is cut and points at `msg:<id>`, which only the message's parties and the person can read; in the app, "Show the whole message" opens it, with its files. Text goes on the card only once the wake policy has let the message through: a held message, or one to another device's agent (whose host decides with its own policy), is recorded without its text, and the device that lets it through (the person releasing it, an unpause, or the host deciding no hold) adds the text with a `fill` row, written by Irori. Attachments are recorded only as a count. Naming a card opts the message in: anyone reading the card sees the entry.
- **Recipient gone.** A message to an agent whose session has ended (out of presence for longer than three minutes; an unreachable session still in presence keeps its mail) isn't left queued for nobody. If it names a card, the inbox sweep records it there marked "recipient gone" (written by Irori, still attributed to the sender), then closes the delivery as `Skipped` with the note `recipient gone; on task:<id>`. A message from an agent about no card goes to the person, marked "recipient gone: name (key)". A card that isn't on this device may still be on its way from another one, so such a message waits for it on later sweeps, and goes to the person only once the card has been missing for ten minutes; the person's own message then stays queued, as with no card. The person's own message about no card, notices, and repo or PR work (which moves to another desk) keep today's behaviour. The send or reply result says which of these will happen when the recipient isn't running.

## Comments from the app

Every card in the app (the main window's card, which is also where a notification or the tray opens a card) has a comment box under its thread, newest last. The person writes, and Wake is on by default: they are asking now.

- **Who hears it** (`Coord.PersonComment`): the holders that work the card, then any holder still assigned, asked to answer it on the card; then its watchers, told who answers and to answer only if asked. With no holder around, the agent that created the card answers instead, if it is still around. Around means the app's roster has the agent and it isn't gone; an agent on an offline device still counts, as its device gets the delivery when it is back. The person, Irori and the commenter never hear their own comment.
- **Wake** goes to whoever answers. Watchers hear at their next turn, unless nobody else heard, in which case they carry the wake.
- **Nobody around:** the `task.comment` row carries `unheard`, and the thread shows "Nobody has heard this yet" under it until an agent writes on the card. Older peers ignore the field.
- **The answer** is a `board_comment` by whoever was asked; it replicates like any comment and shows in every device's thread. Across devices the notice is a delivery to the other device's agent, which its host wakes (v0.4 §6.3).

## Holders that go away

*Card #16. Cards sat under "working" for hours, held by a session that had ended: the holder line only showed on cards without steps.*

- **Every card says who is on it.** A card with steps shows the holder line under its bar whenever nobody is busy on it (idle, offline, gone, assigned, or nobody), not only cards without steps. A card whose holder went away carries a "Nobody on it" chip.
- **A gone holder is taken off.** When a holder of this device has been out of the roster for `HolderGrace` (10 minutes, longer than leases' `GoneGrace` so a restarting session keeps its cards), the sweep writes `task.left {who, by: irori, reason: "gone", person}`. Only the holder's own device does this (v0.4 §6.0); a holder on an offline device keeps its cards (§6.2), and the person is never taken off. Asks have their own take lock and are left to it.
- **The card comes back to someone.** When nobody is left on it, a working card goes back to open (`task.status submitted`, by Irori), and a blocked one stays blocked. Its creator hears at its next turn (`returned` notice) when that is an agent still around. Otherwise (the person made it, or the creator is gone too) `person` is set on the row, and every desktop device shows a "Nobody is on ..." notification (`returned:<taskId>`), cleared once anyone is on the card. The fold keeps `Task.Returned` until then, and the tile says who went away.
- **Stale.** A working card with no activity for two hours and nobody busy on it shows a "Stale" chip. It is derived in the cards service, never written.

## Blocker threads

A blocker is a small thread of its own. The person's answer doesn't resolve it: it goes to the agent that raised it, and the item stays open until someone resolves it. Anyone may resolve (decision 19).

| State | When | Who acts next |
|---|---|---|
| open | nobody has answered; an agent's reply before the person's answer only adds context | the person, or any agent with `board_unblock` |
| answered | the person answered last; the answer is sent to the raiser | the raiser or a holder: resolve or reply. Out of the person's Needs you, in the raiser's |
| replied | an agent replied after the person's answer | the person: answer again, or mark it resolved |
| resolved | `blocker.resolved` arrived; shows the resolver's note beside the check (else the answer) | nobody |
| closed | closed without an answer (`board_unblock` without `blocker_id`, a state change, the sweeper) | nobody |

Open, answered and replied are derived in the fold from the item's `task.comment` rows (`blockerId`, `pick`, `missing`), in log order; only resolved and closed are stored. A comment counts as the person's only when it is signed with actor `user` by the person's key; an agent's reply must come from the device its key names.

- **The person answers** (pick an option or their own words): a blocker comment sent to the raiser, who hears first, then the holders. When nobody can act (the raiser isn't a live agent and no holder is live) the app resolves it at once, as before; "Mark resolved" is the person's escape when the raiser has gone.
- **An agent replies** with `board_comment task_id= blocker_id= text= [missing=]`: to ask back, or to say in `missing` (at most six short lines) what the answer still misses. It tells nobody: the item goes back to the person's Needs you.
- **An agent resolves** with `board_unblock task_id= blocker_id= note=`. Once the person has answered, `note` is required (one line on what it will do now) and `pick` is refused: the person's answer stands, and `blocker.resolved` carries it with the note. Before any answer, `pick` or `note` is the answer, as before.
- **Older devices** fold the person's answer as a plain comment and keep the item open until a `blocker.resolved` arrives, which folds the same everywhere; answering there still resolves at once.

## Asks

*2026-10-28, card #3. Stephen: "not a 'post a card' form". He types into the window, every free agent hears it, one takes it, they talk it through on a thread, and it ends as a new card, a change to a card, or nothing.*

An **ask** is the person talking to the room. It isn't backlog: it is work to start now, and the first job is to work out what that work is. It is not a form. There is no title, objective or done-when field, just what Stephen typed.

### What an ask is: a card with an `ask` field

An ask is stored as a card. `task.created` gains `ask: {wake, repoKey?}`, and the person writes it (actor `user`, `by` the person's `user:<device>` key). The card's title is the prompt's first line, cut to 80 characters at a word. Its objective is the whole prompt, up to 4000 characters, the same cap as a comment, with files as on a comment. It claims a short ref like any card, so Stephen can say "#14".

**Why a card and not a message:**
- **The thread already exists.** The card thread has the composer with Enter to send, rich text, attachments, `send task_id=` entries, the card page (`#card/<id>`), the person's comment routing (`hearers`), watchers, search and replication. A message to everyone would give one delivery per agent and no shared place to talk, so the claimer and the agents chiming in would need a new thread store and a new page.
- **No new view.** The window is cards (minimal UI), and an ask is a card that hasn't found its shape yet.
- **"Becomes a card" is literal.** The outcome turns the ask into the card in place: same `#14`, same thread, created by the person. That makes it a card the person posted, which the instructions already say is work to start now.
- **Older devices degrade well.** A device without this change ignores `ask` and folds a plain submitted card with the prompt as its title and objective. Its tools work on it as on any card. The one thing they could bypass is the take lock, and the new fold refuses that (below).

The `ask.*` kinds and `internal/ask` already belong to approvals (v0.4 §6.7), so the new rows use the `take.*` prefix, and the logic lives in coord, because an ask is a task.

### The take lock: the posting device decides

An ask has exactly one taker. The take lock follows v0.4 §6.0 (owners and requests). The **owner is the device that wrote the ask's `task.created`**, the device Stephen typed on. `owner.Table` gains:

| Entity | Owner | Rows only the owner writes | Requests anyone may write |
|---|---|---|---|
| an ask's take | the device that wrote the ask's `task.created` (`DeviceOf` its `user:<device>` `by`) | `take.state` | `take.requested` |

**Rows** (both replicate; `store/kinds.go` classifies them, and v0.4 §5.1 gains them):
- `take.requested {taskId, by, op: claim|release|reopen, at}`. `by` is an agent key whose device is the writing device (the fold checks this, as with any `by`), or the person's `user:` key for `reopen`. `release` is the taker giving it back.
- `take.state {taskId, taker, reason, req?, told?, at}` is the current taker as a snapshot (`""` means open), written by the owner only:
  - `posted` follows `task.created`. Its `told` field is how many agents heard the ask.
  - `taken` names the taker. `req` is the `(device, seq)` of the request it applied.
  - `released`, `gone` and `reopened` set the taker to `""`. On `reopened`, `told` is set again.
  - `unclaimed` keeps the taker `""` and marks the ask.

  The fold keeps the owner's newest `take.state` by `(hlc, device, seq)`. One device's HLCs only increase (§5.3), so that is the owner's own order. A `take.state` from any other device is ignored and counted ("wrong device"), with a test.

**Claiming** (`ask_claim`):
1. If the fold already shows a taker, the tool refuses: "X (on PC) took #14. To add something, board_comment it." Nothing is written.
2. Otherwise the agent's Core writes `take.requested op=claim`.
3. The owner applies the **first claim it folds** while the ask has no taker and the requester isn't gone, and writes `take.state taken`. It writes nothing for later requests.
4. The tool waits up to **5 seconds** for a `take.state` that answers its request. When the owner is this device, that is immediate (coord's mutex orders two local claims). Between connected devices, the wait is one sync round trip.
   - `taken` naming the caller: "You have #14. Talk it through with the person on the card first if it isn't clear, then ask_close it with what it became."
   - `taken` naming someone else: "Y took #14 first."
   - No answer in time: "Requested. The Mac, which hands out #14, hasn't answered (offline?). Don't start: you'll hear at your next turn either way."
5. **The loser is told by its own Core.** When a device folds a `taken` naming another agent while one of its own agents has a claim request for that ask still open, it sends that agent a next-turn notice: "Y took #14; nothing to do unless you want to chime in." The fold remembers open requests, so this also covers a request that waited out a partition.

**Two devices claiming while partitioned.** Take the PC and the Mac apart, with the ask posted on the Mac. The Mac grants its own agent at once. The PC's request waits, and the PC window shows "Waiting for the Mac to hand it out". When they heal, the Mac folds the PC's request, sees a taker, and writes nothing. The PC folds `taken` and tells its agent. Every device converges on the Mac's row, and no clock decides anything (§5.6).

**Why not the lease rule** (both write, the lower device id stands, the loser is refused afterwards, §6.4): two leases on one file cost little until the rows meet. Two takers of an ask would both be talking with Stephen, and one might already have made the card. With the owner deciding, nobody starts until they are granted.

**The cost, stated plainly:** a claim needs the owner reachable. The owner is the device Stephen just typed on, so it almost always is. If it goes offline before granting, the ask **freezes**, like a delivery on an offline device (§6.2):
- Every other window shows "Waiting for <device> to hand it out".
- Stephen's way out is Close (a person's `task.status` from any device) and posting it again from where he is.
- If the owner is revoked, the fold closes the ask: "posting device removed".

**What the lock refuses.** On an ask that has no outcome yet, the fold ignores `task.claimed`, `task.joined` and `task.assigned`, so an older device's `board_join` can't take it around the lock. Holders are derived from the owner's `take.state`: `[taker]` or none. `board_join`, `board_assign` and `board_status` on such an ask are refused, and the error points to `ask_claim` and `ask_close`. The person can still Close it.

**Order doesn't decide it.** Sync batches come by device, not by cause, so a third device can get the taker's `ask_close` rows before the owner's `take.state taken`. The lock therefore reads a row's position, not what has folded so far: a taker's row counts when the owner's last `take.state` before it (by `(hlc, device, seq)`) names that taker, which is what replay would hold there. A row the lock can't place yet waits, per ask, and is folded again when the ask's next `take.state` or close arrives. Once the ask closed, `task.claimed`, `task.joined`, `task.assigned` and `task.status` count again from the closing row on; one written before it stays refused, as replay refuses it.

**A taker that goes away.** The owner's sweep (`SweepNotices`) releases the take, writing `take.state gone` (with the old taker in its note), and re-announces the ask to free agents, which restarts the unclaimed clock. This happens when:
- a local taker has been out of the roster for `GoneGrace`;
- or the taker's device wrote `agent.gone` or a later `device.started`, which retires its cards.

A taker on a device that is merely offline keeps the ask, as with leases and deliveries (§6.2): it is shown greyed and nothing moves on its own. The taker gives an ask back with `board_leave` (a `take.requested op=release`).

### Who hears it, and how

The owner builds the ask's notices when it posts. They go out through `SendWith(person, agent, text, {Mode, Ref: task:<id>, Notice: true})`, one per **free agent**:
- in `MeshAgents()` (every device of the person's; later the shared board's members);
- not gone, with status `idle`;
- not an agent of a revoked device;
- in the ask's repo when Stephen named one (`RepoKey` matched against the card's repo, as `FrontDesk` matches).

"Attached" is decided by the recipient's host, as for any cross-device wake (§6.3), because `Reachable` is local.

- **Mode:** `next_turn` by default. When Stephen ticks Wake it is `wake` for all of them. The claim race settles the herd, and a loser's woken turn is one tool call.
- **Holds:** the host runs its own policy (§6.3). On the Mac, Pause all holds the ask there, and a held notice doesn't count as heard: while it is on, the owner leaves its own agents out of who is told (`Core.AskAgents`), and they hear it as late arrivals once it lifts. Another device's holds are that device's own.
- **The text:** "The person asks the room (#14): <prompt, cut to 600 runes>. If you can take it on now, ask_claim #14: one agent takes it, and you hear if someone else got there first. If you're busy, leave it." `told` on `take.state posted` is the count sent.
- **Late arrivals:** while the ask is open, each owner sweep tells agents that have become free and haven't been told yet. The inbox has a delivery with `Ref task:<id>` for every agent already told. A late tell wakes only inside the person's own window: `AskUnclaimed` from a post with Wake, or from Wake them. After that, and on the re-announcement after a taker gave it back or went away, it is `next_turn`, so an old ask never wakes a session that only now went idle.
- **Once taken, the others stop hearing.** The inbox gains a hook, `Stale func(Delivery) (note string)`, which the core wires to coord. Before it appends or wakes a `Notice` delivery whose `Ref` is a taken ask, the **recipient's** Core closes it as `Skipped` with the note "taken by X", unless the recipient is the taker. The recipient's device owns that delivery (§6.0), so no other device writes it. Held notices are closed the same way.
- **The thread after the take:**
  - Stephen's comments reach the taker, who is the holder in `hearers`.
  - Agents that chimed in hear only if asked: an agent's `board_comment` on an ask adds it as a watcher (`task.watched`).
  - The rest hear nothing.
  - While nobody has taken the ask, a comment from Stephen goes the way the post went: to the agents told and still free, with the same claim line. So "also, check X" reaches the room rather than nobody. It is its own notice kind, `ask_comment`, with the ask's `Ref task:<id>`, so it goes stale with the announcement once someone takes the ask.

**The tension, for Stephen:** an idle session on `next_turn` hears only when he next types into it. Without Wake, an ask can sit unheard. That is what the unclaimed notice is for, and why it offers "Wake them".

### Nobody takes it

**`AskUnclaimed = GoneGrace` (3 minutes).** One constant, the same "a few minutes" the sweep already uses.

- If an ask has had no taker for `AskUnclaimed` since `posted` or `reopened`, the owner's sweep writes `take.state unclaimed` once. With `told = 0` it writes it at once: "no free agent".
- `notify.Watch` posts a card from that row on every desktop device, with the stable id `unclaimed:<taskId>`: "Nobody took "<title>" (#14)". It has two actions:
  - **Wake them:** a `take.requested op=reopen` from the person. The owner re-announces with `wake` and writes `reopened`.
  - **Close:** a person's `task.status canceled`.

  The card clears when a `taken` or a terminal state folds. Wake them with nobody free brings it back: "You woke them, but nobody is free to hear it."
- The tile shows "Nobody took this yet". A claim after `unclaimed` is still granted.

### Outcomes

The taker (or the person) ends an ask with `ask_close`. `task.status` gains `outcome: {kind, task?, note}`. The note is one line, required, and shown on the tile. A new kind, `task.edited {taskId, title?, objective?, doneCriteria?, by}`, changes a card's text. Nothing could do that before, and the "edit" outcome needs it. It is written by holders, the creator, a person, or an ask's taker acting for that ask, and the fold checks that (an ask's taker as the lock places it, above). The edit outcome can't target an open ask (its prompt is the person's) or a finished card. Every holder hears about it in the watch digest.

| Outcome | Rows | What the window shows |
|---|---|---|
| **card**: it becomes the card | on the ask: `task.edited` (title, objective, done-when from the tool), then `task.steps` when steps are given, then `task.claimed` by the taker, then `task.status working {outcome: card}` | the same tile, now an ordinary card: same `#14`, same thread, the taker holding it. The thread has a history line: "became a card". |
| **edit**: it changes #M | `task.edited` on #M (any fields given), a history line on #M's thread ("from the ask #14: <note>", written by Irori, attributed to the taker), then `task.status completed {outcome: edit, task: M}` on the ask | the ask folds into Done: "Changed #M: <note>", with #M as a link |
| **none**: nothing comes of it | `task.status canceled {outcome: none}` on the ask | Done: "Closed: <note>" |

- **The fold.** Once a `task.status` carrying an outcome has folded, the ask is a card. The take lock stops applying, ordinary holder rules return (the taker's `task.claimed`), and the owner's sweep stops managing it. A `take.state` written after it is ignored.
- **The person closing** from the window writes `outcome: none` with the note they typed, or "closed by the person".

### In the window

**The prompt box.** It is one line at the top of the grid, left of the filter chips (and above `EmptyCards` on a fresh install). Placeholder: "Ask the room…". It grows to a few lines as you type. The rest:
- It is the card comment composer (rich text, paste or drop files, Enter sends, Shift+Enter for a new line), lifted out of `CommentBox`.
- **Wake** is a small toggle beside it, off by default (the spec, and the herd).
- **A repo chip**, "Any repo" by default, picks from the repos the roster's agents are in. Stephen names a repo only when he wants to.

On send, the box clears. The new tile appears first, marked selected, and the window stays on the grid.

**The tile.** An open ask's tile sorts before every other card and carries an **Ask** pill beside its short ref. Its holder-line slot shows one of:
- "3 agents heard it" (`told`);
- "Waiting for <device> to hand it out";
- "Nobody took this yet";
- "<face> took it" (its device chip when remote).

The tile has no progress bar until it becomes a card. A closed ask goes to Done with its outcome line.

**The page** (`#card/<id>`, the existing card page):
- The goal section shows the prompt.
- Under the header, one status line, the same as the tile's.
- **Close** for the person, with an optional note.
- The thread and composer as on every card.

There are no blocker or step sections until it becomes a card. There is no new view, tab, list or dialog.

`CardSummary` and `CardDetail` gain `ask?: {state: open|waiting|unclaimed|taken, taker?, told, owner, outcome?}`. `BoardService` gains `Ask(text, files, repoKey, wake)`, `AskAgain(id)` and `CloseAsk(id, note)`.

### Agent tools

- **`ask_claim {task_id}`** is the take lock (above).
- **`ask_close {task_id, outcome: card|edit|none, note, title?, objective?, done_when?, steps?, task?}`**:
  - `card` takes `title` (required) and optionally the rest;
  - `edit` takes `task` (#M) and any of `title`, `objective` and `done_when` for it;
  - only the taker or a person may call it.
- **Reused, not new:**
  - talking is `board_comment` (there is no `ask_reply`);
  - giving it back is `board_leave`;
  - an open ask's thread is read with `show #14`;
  - `board` lists open asks the caller could take first, under "Asks", followed by asks it has taken.
- **Instructions** (added to `BoardInstructions` and to the two tools' descriptions): "An ask is the person talking to the room: work to start now, not backlog. If you are free and can take it, ask_claim it. One agent takes it, so if someone else has, leave it unless you have something to add (board_comment). Once you hold an ask: if it isn't clear what's wanted, talk it through with the person on the card first. Then ask_close it with what it became: a card (it becomes the card, and you hold it), a change to an existing card, or nothing, with one line on why."

### Build list

- **coord:**
  - the `ask` field on `task.created`, plus `PostAsk`;
  - the `take.requested` and `take.state` folds with the owner check;
  - `task.edited`, and `outcome` on `task.status`;
  - the lock's refusals;
  - `AskClaim` (it returns the request so the tool can wait on its answer), `AskClose`, and the owner sweep (taker gone, unclaimed, late arrivals);
  - the loser's notice;
  - `Task.Ask`.
- **owner, store:** the Table row; kinds registered as Replicated (`TestEveryKindIsClassified`); v0.4 §5.1 and §6.0 gain the kinds.
- **inbox:** the `Stale` hook at append and wake.
- **core:** routing the ask notices; wiring `Stale`; `notify.Watch` for `unclaimed:<id>`.
- **tools:** `ask_claim` and `ask_close`, the `board` section, refusals on `board_join`, `board_assign` and `board_status`, and the instructions; regenerate the fallback tool list.
- **GUI:** the prompt box (lifted from `CommentBox`), the tile and page states, `BoardService` methods; regenerate the bindings.
- **Tests:**
  - two local agents claim at once: one taker, and the other gets "Y took it first";
  - PC and Mac partitioned with the owner on the Mac, both claim, heal: the Mac's agent holds it, every device agrees, the PC agent gets a notice;
  - the taker goes away: `gone`, re-announced, a new claim granted;
  - a taker on an offline device keeps it;
  - unclaimed after `AskUnclaimed`, and at once with `told = 0`; reopen;
  - a queued notice is skipped as stale once the ask is taken;
  - a `take.state` from the wrong device is ignored and counted;
  - the take rows fed in two orders fold the same;
  - an older device's `task.claimed` on an open ask is ignored;
  - each outcome's rows and fold.

## Links out

- A card can carry links: a URL (Linear, Notion, a GitHub issue or PR), a repo path, or a commit.
- When a card links to a tracker item, the Board shows it by its own name: "Linear ABC-123", "GitHub #57".
- Irori doesn't sync or mirror trackers. Agents use whatever tools they've been given for those (Linear MCP, `gh`, and so on). The Board only points.

## Short refs

*v0.4.2.* A card's id is `task:01M62B05GRJT56Y3BVAM2D7NAG`. Nobody can say that out loud. So each card also has a **short ref**, `#12`, that Stephen can say and type, and that agents print and accept.

**The scheme: a number per person, claimed when the card is made, with clashes settled by the fold.**
- `task.created` gains `num`: one more than the highest number its device has folded. One device counts 1, 2, 3, and every device that has synced continues from the same top number.
- The short ref is never written. The fold derives it from the rows. A number with one claimant is `#12`. A number with more than one takes its claimants in `(hlc, device, seq)` order and gives each a letter: `#12a`, `#12b`, `#12c`.
- A clash happens only when two devices make a card while they can't see each other's rows (a partition, or two creates inside one sync tick). Once the rows meet, every device settles it the same way: both cards gain a letter, and each card and its `show` say "was #12".
- After a clash, `#12` alone names neither card. Every tool refuses it and names both: "cards made on devices that were apart each claimed #12, so it is now #12a "Fix login" (task:…), #12b "Ship docs" (task:…)". The PC saw its card as #12 and the Mac saw a different card as #12. If the bare number went to the earlier card, the Mac's agent typing `board_status #12 done` would close the PC's card with no error. Refusing it means a ref someone saw before the clash settled can fail, but it can never act on the other card.
- The holders and creator of a card whose ref moves hear it in their watch digest: "Irori renumbered it #12b (it was #12): …". The window shows "(was #12)" beside the ref.
- Numbering carries on past every claim. After the PC and the Mac both used #2 apart, the next card on either device is one past the highest number either of them reached.
- Rows written before `num` existed claim their rank among such rows, in the same order. They line up as 1, 2, 3, and new cards count on from there. Two cases move a number here, and both happen only in the upgrade window or with a device still on v0.4.1. A device without `num` keeps writing rank-numbered rows, which can clash with numbered cards. They settle like any clash, with letters. A pre-upgrade row that arrives late with an older HLC moves the rank of every older-style card after it. Nothing marks those as "was", but every device still converges once it holds every row.
- A revoke's cut drops the cut device's rows and re-folds. A card that had clashed only with a cut card loses its letter. It is marked moved, so search re-reads it and its holders hear about it. The highest number this device has seen doesn't drop, so the cut card's number isn't handed out again while the app runs. After a restart, the fold only knows the rows it still holds, so the number can come back.

**Why this one:**
- *It is short and sayable.* Most cards are `#7` or `#31`. A letter shows up only after a real clash.
- *A seen ref never points at another card.* A card's ref changes only when its number clashes. It gains a letter, and a later claim on the same number can move that letter along. The bare number it had is then refused, never handed to the other card. Nothing renumbers as the Board grows, and a late numbered row moves only the cards that share its number.
- *It converges with no writes.* The ref depends only on the set of `task.created` rows, never on arrival order (a test feeds the same rows in several orders, and one replays them). No device has to be online to settle a clash, so a card whose device has gone offline still settles. There is no owner and no extra row.
- *A renumber is visible and rare.* It needs a create on two devices that are apart, and the window and `show` both say what it was.

**Rejected:**
- *Rank in `(hlc, device, seq)` order, with no stored number.* That needs no field, but a card made on an offline device lands in the middle of the order when it syncs and renumbers every card after it.
- *A base32 prefix of the id that grows on a clash.* The first ten characters of an id are its millisecond time, so cards made close together share long prefixes. The random tail is unique, but `#K7QD` is hard to say. And a prefix that grows on a clash changes a ref that has already been seen.
- *A per-device prefix (`#M3`, `#P12`).* It never clashes, but every ref carries a device letter, even though most people use one device at a time.
- *A renumber row written by the losing card's device.* That gives plain integers, but the clash then stays unsettled until that device comes back online.
- *The earliest claimant keeps the bare number.* That way one fewer card moves. But the bare number then means a different card on the device whose card lost, and nothing there can tell the difference.

**Where it shows:**
- `Task.Short` and `Task.ShortWas` are fold-derived, and `CardSummary` carries them to the window. The grid tile and the detail header show `#12`.
- Every tool that takes a card accepts `#12`, `12`, `task:#12`, or the full id with or without `task:` (`Coord.TaskRef`, `ParseShort`). So does `board_block`'s `on: task #12`, which stores the full id because it replicates. A clashed bare number is `ErrAmbiguous`, and its text names each card.
- `show #12` works, and so does `show task:#12`, through coord's Source.
- Card lines print `#12 task:<id>`. Confirmations print `#12`. Notices print `(#12, task:<id>)`, because a notice's text is kept, and the id never moves.
- `Notice.Ref`, notify targets and every other machine ref stay `task:<id>`.
- Search: a task doc's body starts with `#12`, so the word `12` finds it. A query of only `#12` puts that card first. Task hits print their short ref. A card whose ref moves is re-read by the index even though its `UpdatedAt` doesn't change.

## Subtasks

*v0.4.10 design (#20), not built yet. Stephen, 2026-10-28: "When tasks are related can they have a parent task and sub tasks that are worked on? Or if a task becomes larger it should be reworked to a parent task and refactored into multiple sub tasks. And I'd like the tasks to arrange themselves in a way that indicates they are sub tasks to a task."*

A card can have a **parent**. A parent's **subtasks** are ordinary cards: each has its own short ref, holder, steps, blockers and thread, so several agents can work one piece of work in parallel. The parent shows how far the whole has got. This is one level, not a tree: "sub-task trees" stay out (What it is, above).

**The rules:**
- **One level.** A subtask can't have subtasks, and a parent can't have a parent. A card that grows inside a subtask is split into more siblings under the same parent.
- **Same board.** A parent and its subtasks share a scope: private cards with private cards, and the same shared board once shared boards exist (v0.5).
- **The parent's progress comes from its subtasks.** It's `done/total` subtasks, where done is completed and canceled subtasks don't count. Steps the parent kept from before the split show as history and count for nothing.
- **The parent's state is derived from its subtasks,** in the fold, never written:
  - blocked when it has an open blocker of its own (a question about the whole) or any subtask is blocked; its blocker line shows the first one, as "#14: <question>" when it is a subtask's;
  - working when any subtask is working;
  - completed once every subtask has ended and at least one completed, and canceled when every one was canceled;
  - otherwise open.
  A rule over the set of rows, like short refs: every device folds the same state, no device has to write a roll-up row, and an offline device's subtask changes roll up wherever they arrive.
- **What can be set on a parent.** `board_status` on a parent takes `canceled` only: it cancels the parent and writes `canceled` for each subtask still open. `completed` is refused while a subtask is open (the error lists them), and `working` and `input_required` are refused, since they're derived. `board_steps` on a parent is refused: its work lives in its subtasks. Blockers, comments, links, watchers and assignment work as on any card.
- **Who hears what.** A parent's holders and watchers get its subtasks' state, blocker and holder changes in their watch digest, one line per subtask, as "#14 (part of #12): blocked: <question>". The subtask's own holders hear their card as today. A person's comment on the parent goes to the parent's holder, who coordinates.

**Rows.** Payloads only gain fields, so older peers still fold everything.
- `task.created` gains `parent` (the parent's full task id).
- A new `task.parent {taskId, parent, by}` sets or clears (`parent: ""`) an existing card's parent.
- A link counts only when it keeps one level. Rows are applied in `(hlc, device, seq)` order. A link that would put a parent under another parent, or a card that has subtasks under a parent, is ignored and stays inert, and `show` on the child says so. Two devices can each make half of a two-level link while they're apart; the ordering settles it the same way everywhere once both rows are held.
- Who may set a parent: anyone who may edit the subtask (its holders and creator, or the person), and anyone who may edit the parent (its holders and creator, or the person), which is who may split it.
- An older peer ignores `parent`. It sees the subtasks as plain cards and the parent with the state its own rows give it. Nothing there breaks, and it catches up when it updates. No sync schema bump.

**Tools.**
- `board_create` takes `parent` (a short ref or task id): the new card is a subtask of it.
- A new **`board_split`** `task_id= subtasks=[{title, objective?, done_when?, steps?, assign_to?}] adopt?=[refs]` turns a card into a parent. Each entry makes a subtask with that parent, in this order:
  - `task.created` with `parent`;
  - its `task.steps`;
  - a `task.assigned` when `assign_to` is set;
  - one `task.steps` on the parent, keeping only its done steps.
  
  An entry's `steps` are either step ids from the card being split, which move with their done flags, or new step texts. Every open step of the card must go to exactly one subtask, or the call is refused with the steps that weren't placed. That's how a split "moves the steps that belong to them" without losing one. `adopt` makes existing cards subtasks too (one `task.parent` row each): use it when related cards already exist. The splitter keeps holding the parent.
- `board_split` on a card that already has subtasks adds more. On a subtask, it's refused with the hint "add siblings: board_create parent=#12".
- `board` keeps its state groups. A subtask's line says "part of #12", and the parent's line reads `#12 [working, 1/3 subtasks] ...`, followed by its subtasks' refs and states and, when one is blocked, "blocked by its subtask #14: <question>". `show #12` lists the subtasks with their state, holder and steps. `show #14` says "part of: #12 <title>".
- **Agent instructions** (BoardInstructions) gain one line:
  > When a card grows past one agent's piece of work, or you see related cards that are one piece of work, make it a parent: board_split it into subtasks (adopt the related cards), so other agents can take subtasks in parallel. Keep one level: split a subtask into more siblings, not deeper.

**The window.**
- **The grid** (Stephen, 2026-10-28: separate tiles, no frame). A subtask is an ordinary tile that follows its parent's tile, ordered by short ref, with a "part of #12" chip in its header. There's no frame and no nesting, so a parent and its subtasks read as a run of tiles that starts with the parent. The parent's tile shows "3 subtasks" beside its bar. A completed parent folds into Done with its subtasks. A subtask that's done while its parent is open stays after the parent, shown as done.
- **Filters.** Needs a decision and My devices show a subtask that matches with its parent's tile before it, even when the parent itself doesn't match, so a blocked subtask is seen in context.
- **The parent's page** lists its subtasks in the side column, in place of steps. Each shows its short ref, title, state chip, holder and progress, and opens on a click. The progress bar is the subtask count.
- **A subtask's page** shows "part of #12 <title>" under its header, linking to the parent.
- No tree view and no new screen.

**Asks.** An ask is outside the family until it becomes a card. An open ask is one with no outcome yet ([Asks](#asks)).
- **It can't be a subtask or a parent.** `board_create parent=`, `board_split` and `adopt` refuse an open ask in either role, the same way `board_join` and `board_assign` do. The error points to `ask_claim` and `ask_close`. The fold ignores a `task.parent` row that names an open ask as either end. It uses the lock's position rule: such a row counts only when it comes after the ask's closing row, so an older device can't attach an ask around the lock.
- **`ask_close outcome=card` takes an optional `parent`.** The ask becomes a card and a subtask of that parent in one call: a `task.parent` row follows its `task.status working`. That's how "this belongs under #12" lands without a second step.
- **An ask closed as edit or none** is a finished record, not a card. `adopt` refuses it, as it refuses any finished card: only cards still in play are adopted.

**Short refs stay flat.** Subtasks take the next numbers like any card (#13, #14, #15). Rejected: "#12.1" style refs. They'd need a second counter per parent, which clashes across devices the same way short refs do, and an adopted card would have to be renumbered, so a ref someone already saw would change.

**Rejected:**
- *Parent progress from the parent's own steps:* two places to tick, and they drift apart.
- *A stored parent state, written by the parent's holder whenever a subtask changes:* the parent's state would wait for that holder's device to be online.
- *Any depth:* a tree needs a tree view and pushes the Board toward a tracker. One level answers "this grew, split it" and "these belong together".

**Build** (after #3, in the order agreed with #16 on the cards): coord (`parent` on create, `task.parent`, the one-level rule, derived progress and state, refusals on parents, digest lines) and `CardSummary` (`Parent`, `Subtasks`), then the tools and instructions, then the grid order and the "part of" chip, the parent page's subtask list and the "part of" link. Tests: arrival-order convergence for links and the derived state, the two-level link from two devices, split moving every open step, cancel cascading. Live check: an agent splits a big card into a parent and three subtasks, two agents take subtasks, the grid shows them after the parent, and the parent's progress follows them.

## Where Irori's own long-lived records fit

- **Capture** (timeline, edits, commits) is history Irori records by itself, mechanically. Nobody writes it, so it isn't clutter.
- **Decisions** (`decision_record`) are notes anchored to code, which Irori re-checks against the file. That is a different thing from a board card. Its tool description says to keep it to decisions about this code, and to record project-level decisions wherever the project keeps them.

## Build order

1. **Next-turn delivery:** check that the mod can add a comment to a turn the person starts, and if not, fall back to a hook the person approves. Then add a delivery `mode` to the inbox.
2. **coord:** add the `blocked` state with a reason, close-blocker by anyone, a holders list for shared tasks, comments, assignment notices, links, and `lease.refused` rows.
3. **Tools:** the board tools carry the agent instructions above. Replace the task tools rather than adding more beside them.
4. **GUI:** the Board tab, following the prototype. It replaces Work, adds the Wake choice per comment, and shows link chips.
5. **Live check:** two connected chats hand a task back and forth, one blocks on you, you answer from the Board, and it carries on.

## Contracts

The binding signatures and doc comments live in the code; this is the map. Three packages build in parallel against them, then one integration pass wires them together.

**A1 · coord** (`internal/coord`: `coord.go`, `kinds.go`, `board.go`, `links.go`)
- States gain `blocked`, entered only through `Block(by, id, reason, *BlockedBy)` (or `NewTask.BlockReason` for a blocker with no task behind it) and left through `Unblock`, which anyone may call (decision 19). `BlockedBy{kind: lease|task|person|agent|external, ref}`. `SetTaskState(blocked)` is refused.
- `Holders []string` is the list of who works a task. `Holder` stays and is always `Holders[0]`. `Shared` tasks take any number of holders (`Join`, `Leave`, `SetShared`; claiming a shared task joins it). Unshared tasks keep v0.2's single writer. `Assigned` lists the holders that were assigned and haven't taken the task up yet (claimed, joined or posted its state): they see it in their NeedsYou, and Unblock restores `working` only when some holder isn't merely assigned.
- `Assign(by, id, to)` lets an agent assign to another agent (decision 21) and returns a `Notice`. `AddComment(by, id, text, wake)` writes a `task.comment` row, separate from the task, and returns a `Notice`. `Comments(id)` lists them. `Unblock` also returns a `Notice`, and so does `SweepNotices` when a blocker resolves on its own (lease released, blocking task done).
- `Links`: `ParseLink`/`LinkLabel` are implemented. Labels look like "Linear ABC-123", "GitHub #57" and "Notion Board design", and any other URL shows its host. Use `AddLink`/`RemoveLink`, at most 16 per task.
- A refused `Acquire` writes a `lease.refused` row (`Refusal`, deduped while active). `Refusals()` lists them.
- `Board(BoardFilter)` returns `BoardView{NeedsYou, Blocked, Working, Open, Done (capped, DoneTotal), Leases}`. Done is collapsed and never timed out.
- coord implements `search.Source` for `task:` and `comment:`.
- New log kinds: `task.joined`, `task.left`, `task.shared`, `task.linked`, `task.comment`, `blocker.closed` and `lease.refused`. `task.created` and `task.status` only gain fields.

**A2 · inbox + wire** (`internal/inbox`, `internal/attach/protocol.go`, `internal/api` stream, `internal/mcpserver`, `mod/`)
- `Delivery.Mode` is `wake` or `next_turn`. `SendWith(from, to, text, SendOptions{Reason, Mode, Ref, Notice})` picks a default mode when none is given: `next_turn` for a person or `SystemSender()`, `wake` for an agent. `Send` stays wake.
- `NextAppend(agent)` returns a `Wake` with `Mode next_turn` and token `[irori-append:<id>]`. A wake carries any queued next_turn items along with it. Both are batched up to 16 KB, and anything past that goes in `Overflow` ids that the agent reads with the inbox tool. next_turn never wakes, never gets a reminder, and is never moved for being unengaged.
- The wire is a new `append` event (`attach.Version` 2), sent only to mods at `AppendSince` (0.4.0) or later. The mod calls `$.session.append`, then POSTs `delivered {token}`, then at the next main-loop `turn.start` POSTs `engaged {token, turnId}`. There are no new routes. On Grok, next_turn items are pushed as before, with meta `mode=next_turn`.
- `Notice` messages need no reply: they close as skipped once engaged.
- inbox implements `search.Source` for `msg:`. Held messages stay unsearchable. A message's `Doc.Readers` are its agent sender and the agents it was delivered to: no other agent finds or shows it (the person sees all).

**A3 · capture search** (`internal/search`, `internal/capture/search.go`, `overview.go`)
- The index is SQLite FTS5 (verified compiled into modernc.org/sqlite, `TestFTS5Available`). It lives in memory and is rebuilt at New. It covers turn prompts and replies, edits, commit subjects, PRs and decisions, plus each `Source`, pulled by `SearchDocs(since)` before every search.
- `Search(Query{Text, Kinds, RepoKey, Agent, Before, Offset, Limit, Viewer})` returns `[]Hit{Ref, Kind, At, Title, Snippet, Agent, RepoKey}`. Page with `Offset` (hits already shown); `Before` is a time filter, not a pager, since text hits are ranked. `Show(ref, {Offset, Limit, Viewer})` returns `Shown{Text (capped, search.Page), More, Next, Related}`. `Viewer` is the calling agent's key (tools always pass it; "" is the person) and hides docs whose `Readers` leave it out.
- Refs: `turn:` `edit:` `commit:` `msg:` `task:` `comment:` `pr:owner/repo#n` `decision:`, plus show-only `agent:`. `search.ParseRef` is implemented.
- `Overview(for, OverviewInput)` returns live agents with a one-line "doing", the caller's inbox counts, blockers and tasks in its repo, and recent commits and PRs. Every line has a ref.

**Cards (v0.3.3, design/v0.4-devices.md §9)** (`internal/coord/cards.go`)
- A task gains `Steps` (ordered, short ids `s1`…, at most one doing; progress is done/total, no bar without steps), `Blockers` (items: question, context for a newcomer, raiser, recommended `Options`, `Resolved`; ids `b1`…) and `Watchers`. Blocked stays a state (decision 17): `Raise` adds an item and enters Blocked, `Answer` with `Resolve` resolves one (anyone, decision 19) and the task leaves Blocked when none is open (without `Resolve` it is the person's answer, which leaves the item open: Blocker threads, above); `Block` is Raise with the reason as the question and keeps replacing its own item; `Unblock` and a state change close every open item.
- Each item has its own `On` (what it waits on); a raised question defaults to a person, so the card is in the person's Needs you. `Task.BlockedBy` is the first open item's `On`. The sweeper closes only the items whose lease or task went away; a question left open keeps the card blocked.
- `Recommend` is for the holders, watchers, the raiser, the creator or a person. `Answer`'s notice goes to the raiser first, then the holders, with the answerer's wake choice; a reply on a blocker (`CommentOn` with `BlockerID`) sends none.
- Watch notices are not returned per call: coord queues the card's state, step, blocker and PR-link changes in memory (never on replay) and `SweepNotices` hands out one `watched` digest per card and watcher, routed from Irori at the next turn.
- New kinds: `blocker.option`, `blocker.resolved`, `task.steps`, `task.step`, `task.watched`; `task.status` gains `blocker`. Older reason-only rows fold to one item.
- Tools: `board_steps`, `board_step`, `board_recommend`, `board_watch`, `board_unwatch`; `board_create` takes `steps` (v0.4.2: `NewTask.Steps`, a `task.steps` row right after `task.created`); `board_block` takes `question` (alias `reason`), `context`, `options` (each with `recommended`, v0.4.2); `board_unblock` takes `blocker_id` and `pick` (and `note`, required once the person answered); `board_comment` takes `blocker_id` and `missing`. The GUI reads `BoardService.Cards`, `Card` and answers with `Answer`.
- A card without steps shows a holder line in place of the bar (v0.4.2, `CardSummary.Holder`): who holds it and whether that agent is active, idle, offline or gone; "Waiting for <agent>" when the assignment's notice is still queued or held in the inbox (`AgentRef.Waiting`), and "Assigned to <agent>" when it was told but hasn't taken it up, so neither looks like work in progress.

**Integration (after A1–A3)**
- Core: `AddSource(coord)`, `AddSource(inbox)`; call `SweepNotices` in place of `Sweep`; route every `Notice` through `SendWith(…, {Mode: next_turn unless Wake, Ref, Notice: true})`.
- Tools: replace `task_*` with `board`, `board_create`, `board_comment`, `board_block`, `board_unblock`, `board_join`, `board_assign`, `board_link`, and add `overview`, `search` and `show`. The agent instructions above go in their descriptions.
- GUI: a `BoardService` over `Board()` and the Wake toggle per comment. `InboxService.Send` moves to `SendWith`, defaulting to next_turn.
