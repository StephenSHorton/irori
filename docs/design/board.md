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

The tools enforce what they can:
- a title is at most 200 characters, and an objective or comment at most 4000;
- a card's links are URLs or repo paths;
- there is no field for anything the list above leaves out.

## Links out

- A card can carry links: a URL (Linear, Notion, a GitHub issue or PR), a repo path, or a commit.
- When a card links to a tracker item, the Board shows it by its own name: "Linear ABC-123", "GitHub #57".
- Irori doesn't sync or mirror trackers. Agents use whatever tools they've been given for those (Linear MCP, `gh`, and so on). The Board only points.

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
- `Holders []string` is the list of who works a task. `Holder` stays and is always `Holders[0]`. `Shared` tasks take any number of holders (`Join`, `Leave`, `SetShared`; claiming a shared task joins it). Unshared tasks keep v0.2's single writer.
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
- inbox implements `search.Source` for `msg:`. Held messages stay unsearchable.

**A3 · capture search** (`internal/search`, `internal/capture/search.go`, `overview.go`)
- The index is SQLite FTS5 (verified compiled into modernc.org/sqlite, `TestFTS5Available`). It lives in memory and is rebuilt at New. It covers turn prompts and replies, edits, commit subjects, PRs and decisions, plus each `Source`, pulled by `SearchDocs(since)` before every search.
- `Search(Query{Text, Kinds, RepoKey, Agent, Before, Limit})` returns `[]Hit{Ref, Kind, At, Title, Snippet, Agent, RepoKey}`. `Show(ref, {Offset, Limit})` returns `Shown{Text (capped, search.Page), More, Next, Related}`.
- Refs: `turn:` `edit:` `commit:` `msg:` `task:` `comment:` `pr:owner/repo#n` `decision:`, plus show-only `agent:`. `search.ParseRef` is implemented.
- `Overview(for, OverviewInput)` returns live agents with a one-line "doing", the caller's inbox counts, blockers and tasks in its repo, and recent commits and PRs. Every line has a ref.

**Integration (after A1–A3)**
- Core: `AddSource(coord)`, `AddSource(inbox)`; call `SweepNotices` in place of `Sweep`; route every `Notice` through `SendWith(…, {Mode: next_turn unless Wake, Ref, Notice: true})`.
- Tools: replace `task_*` with `board`, `board_create`, `board_comment`, `board_block`, `board_unblock`, `board_join`, `board_assign`, `board_link`, and add `overview`, `search` and `show`. The agent instructions above go in their descriptions.
- GUI: a `BoardService` over `Board()` and the Wake toggle per comment. `InboxService.Send` moves to `SendWith`, defaulting to next_turn.
