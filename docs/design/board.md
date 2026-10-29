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
- Who may set a parent: anyone who may edit the subtask (its holders and creator, or the person), and the parent's holders.
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
- `board` lists a subtask under its parent, indented, with its own state, and the parent's line reads `#12 [working, 1/3 subtasks] ...`. `show #12` lists the subtasks with their state, holder and progress. `show #14` says "part of #12 <title>".
- **Agent instructions** (BoardInstructions) gain one line:
  > When a card grows past one agent's piece of work, or you see related cards that are one piece of work, make it a parent: board_split it into subtasks (adopt the related cards), so other agents can take subtasks in parallel. Keep one level: split a subtask into more siblings, not deeper.

**The window.**
- **The grid.** A parent and its subtasks sit together as one **family**: a thin frame across the grid's width, with the parent's tile first and then its subtasks' tiles, which are smaller and ordered by short ref. The family takes the parent's place in the grid. A completed parent folds into Done with its subtasks. A subtask that's done while its parent is open stays in the family, shown as done.
- **Filters.** Needs a decision and My devices show a family when any card in it matches. Its other cards stay visible but dimmed, so a blocked subtask is seen in context.
- **The parent's page** lists its subtasks in the side column, in place of steps. Each shows its short ref, title, state chip, holder and progress, and opens on a click. The progress bar is the subtask count.
- **A subtask's page** shows "part of #12 <title>" under its header, linking to the parent.
- No tree view and no new screen.

**Short refs stay flat.** Subtasks take the next numbers like any card (#13, #14, #15). Rejected: "#12.1" style refs. They'd need a second counter per parent, which clashes across devices the same way short refs do, and an adopted card would have to be renumbered, so a ref someone already saw would change.

**Rejected:**
- *Parent progress from the parent's own steps:* two places to tick, and they drift apart.
- *A stored parent state, written by the parent's holder whenever a subtask changes:* the parent's state would wait for that holder's device to be online.
- *Any depth:* a tree needs a tree view and pushes the Board toward a tracker. One level answers "this grew, split it" and "these belong together".

**Build** (after #3, in the order agreed with #16 on the cards): coord (`parent` on create, `task.parent`, the one-level rule, derived progress and state, refusals on parents, digest lines) and `CardSummary` (`Parent`, `Subtasks`), then the tools and instructions, then the grid family, the parent page's subtask list and the "part of" link. Tests: arrival-order convergence for links and the derived state, the two-level link from two devices, split moving every open step, cancel cascading. Live check: an agent splits a big card into a parent and three subtasks, two agents take subtasks, the grid shows them in the family, and the parent's progress follows them.

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
