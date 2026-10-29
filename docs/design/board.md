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
- sprints, milestones, epics, sub-task trees;
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
> When you take a card, give it steps and keep them ticked.
> When you need the person, raise one blocker per question (never several asks in one note): context for a newcomer, options when there are real choices, and your recommendation marked.

*v0.4.2, from the first two-machine run:* an agent put three asks in one `input_required` note and the person saw one fat card. So `board_status input_required` is refused unless the card has an open blocker (and on a card that has them it changes nothing, since a state change would close them); the error says to raise each question with `board_block`. An option can be marked `recommended` (at most one per raise; a lone option is the recommendation; `board_recommend` always recommends, and marks an option already offered). The window shows the recommended one first with a badge, the others as options, and the field for an answer in the person's own words. Older `input_required` cards still show their note as one item.

The tools enforce what they can:
- a title is at most 200 characters, and an objective or comment at most 4000;
- a card's links are URLs or repo paths;
- there is no field for anything the list above leaves out.

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
- A task gains `Steps` (ordered, short ids `s1`…, at most one doing; progress is done/total, no bar without steps), `Blockers` (items: question, context for a newcomer, raiser, recommended `Options`, `Resolved`; ids `b1`…) and `Watchers`. Blocked stays a state (decision 17): `Raise` adds an item and enters Blocked, `Answer` resolves one (anyone, decision 19) and the task leaves Blocked when none is open; `Block` is Raise with the reason as the question and keeps replacing its own item; `Unblock` and a state change close every open item.
- Each item has its own `On` (what it waits on); a raised question defaults to a person, so the card is in the person's Needs you. `Task.BlockedBy` is the first open item's `On`. The sweeper closes only the items whose lease or task went away; a question left open keeps the card blocked.
- `Recommend` is for the holders, watchers, the raiser, the creator or a person. `Answer`'s notice goes to the raiser, with the answerer's wake choice.
- Watch notices are not returned per call: coord queues the card's state, step, blocker and PR-link changes in memory (never on replay) and `SweepNotices` hands out one `watched` digest per card and watcher, routed from Irori at the next turn.
- New kinds: `blocker.option`, `blocker.resolved`, `task.steps`, `task.step`, `task.watched`; `task.status` gains `blocker`. Older reason-only rows fold to one item.
- Tools: `board_steps`, `board_step`, `board_recommend`, `board_watch`, `board_unwatch`; `board_create` takes `steps` (v0.4.2: `NewTask.Steps`, a `task.steps` row right after `task.created`); `board_block` takes `question` (alias `reason`), `context`, `options` (each with `recommended`, v0.4.2); `board_unblock` takes `blocker_id` and `pick`. The GUI reads `BoardService.Cards`, `Card` and answers with `Answer`.
- A card without steps shows a holder line in place of the bar (v0.4.2, `CardSummary.Holder`): who holds it and whether that agent is active, idle, offline or gone; "Waiting for <agent>" when the assignment's notice is still queued or held in the inbox (`AgentRef.Waiting`), and "Assigned to <agent>" when it was told but hasn't taken it up, so neither looks like work in progress.

**Integration (after A1–A3)**
- Core: `AddSource(coord)`, `AddSource(inbox)`; call `SweepNotices` in place of `Sweep`; route every `Notice` through `SendWith(…, {Mode: next_turn unless Wake, Ref, Notice: true})`.
- Tools: replace `task_*` with `board`, `board_create`, `board_comment`, `board_block`, `board_unblock`, `board_join`, `board_assign`, `board_link`, and add `overview`, `search` and `show`. The agent instructions above go in their descriptions.
- GUI: a `BoardService` over `Board()` and the Wake toggle per comment. `InboxService.Send` moves to `SendWith`, defaulting to next_turn.
