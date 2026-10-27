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
