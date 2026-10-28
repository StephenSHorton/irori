# Waits and signals: draft

*2026-10-27, draft for Stephen. Comes from his ask: "conversations can wait upon one another in an event-based manner, maybe they can even set up a trigger for a particular conversation to pull when appropriate that any conversations can subscribe to, and it can be cleared." Notifications (a later design) build on this, since "tell me when X is done" is a wait whose target is a person.*

## The two pieces

**A signal** is a named flag that a conversation raises when something is ready, and clears when it no longer holds.
- Names are scoped. For example, `toru/notifications-ready` is in the toru repo; `ww/release-0.6.0` is in ww-multiplayer.
- A signal carries a note, which can be short ("NotificationService is merged on main, see PR #57") and a link.
- Any agent or person can raise one. Raising it again updates the note.
- Clearing it is explicit, or happens when its expiry passes.
- Rows: `signal.raised {name, by, note, link?, expires?}` and `signal.cleared {name, by, reason}`. The current state is the newest row for each name.

**A wait** is "tell me when this happens". It is registered by a conversation or a person, and it fires once.
- What it can wait for:
  - `signal:<name>`, when that signal is raised (or is already up);
  - `agent:<name>` with `finished`, `turn-ended`, `idle` or `gone`;
  - `task:<id>` with `done`, `blocked` or `unblocked`;
  - `pr:<owner/repo>#<n>` with `merged` or `closed`;
  - `commit:<repo>` touching a path.
- What happens when it fires:
  - a message to the waiter, delivered at its next turn by default (no extra tokens), or as a wake if the waiter asked for one;
  - for a person, a notification (see Notifications).
- The message says what happened and includes the target's last reply excerpt or the signal's note. The waiter doesn't need to go and read anything to know what changed.
- Waits can be listed and cancelled, and they expire (default 24 h).
- Rows: `wait.created {id, by, on, deliver: next_turn|wake|notify, note, expires}`, `wait.fired {id, event ref}` and `wait.canceled {id, by}`.

Both are event-based. The Core matches each new log row against the open waits; nothing polls. The match runs inside the store subscriber that already feeds the GUI. A wait created when its condition already holds (the signal is up, the PR is already merged) fires at once.

## "Finished" for a conversation

A conversation going idle isn't the same as it finishing: it may only be waiting for you. In order of strength:
1. **It says so.** The conversation raises its signal (`toru/notifications-ready`) or marks its Board task done. This is the only reliable meaning, and the mod's tool descriptions tell agents to do it when they finish shared work.
2. **Its PR merged.** The capture ledger already knows which session opened which PR.
3. **It went idle with no open task of its own and no question to you.** This is weak, and labelled as weak in the fired message.
4. **It ended** (the session is gone).

`wait {for: "agent:toru-notifications", until: "finished"}` takes the first of 1–3 to happen, and says which one it was.

## Tools (both front ends)

| tool | args | does |
|---|---|---|
| `signal` | `name`, `note?`, `link?`, `expires_minutes?` | raise or update |
| `signal_clear` | `name`, `reason?` | clear |
| `signals` | `prefix?` | list what's up |
| `wait` | `for`, `until?`, `deliver?` (`next_turn` default, `wake`), `note?` | register; fires once |
| `waits` | | my open waits |
| `wait_cancel` | `id` | |

The GUI gets a "Waits & signals" strip on the Board: what's raised, who's waiting on what, and a **Notify me when…** button for people.

## Guards

- **Loops:** a fired wait's message counts as a hop. A wait can't be registered on yourself.
- **Abuse:** at most 20 open waits per agent.
- **Cost:** next-turn delivery is the default, as with the Board's comments (decision 18).
- **Cross-person (Shared Boards):** signals live in a board log only if raised on a board; waits are always private to whoever made them.

## Build order

1. Signals: rows, fold, tools.
2. Waits: rows, a matcher over the store subscription, and firing through the inbox with next-turn delivery. This shares the next-turn delivery work with the Board.
3. "Finished" detection from the Board's task state, the PR ledger and presence.
4. GUI strip and "Notify me when…", once notifications exist.

## Notifications (to design after toru-notifications lands)

Requirements gathered so far:
- **Actionable approvals:** share requests (decision 26), and wake or assignment requests on boards with stricter receive settings. Each shows buttons (Share / Review / No, Release / Dismiss), and answering one clears it on every device.
- **Waits for people:** "Notify me when…" fires a notification.
- **Agents notifying a person:** a `notify` tool with a person, all of their devices or one device as the target, text, a level, and up to two actions whose answers come back to the agent at its next turn.
- **Look and sound:** follow what toru-notifications builds (top-right stack, never steals focus, the s&box sounds), so Stephen's apps feel the same.

**Built (single device):** `internal/notify` on [go-toast](https://github.com/StephenSHorton/go-toast) (Light palette, 10 dip corners, hidden from screen capture unless `IRORI_TOAST_CAPTURE=1`). Settings (on, sound, volume, corner) live in `settings.json` in the data folder and in the app's header gear, with a test card. Every card has one id (`held:<sender key>`, `needs:<task>`, `msg:<delivery>`, `ask:<id>`), so nothing stacks twice, and it is cleared when its thing is resolved elsewhere. `notify.Watch` follows the inbox and coord change streams (no polling) and posts: held deliveries, one card per sender (decision 34, after ~11 separate cards at the first PC+Mac pairing): the card counts that sender's held messages ("alpha sent beta 3 messages Irori held", the newest one's reason and text), is updated in place without a sound while it is up, and its Release / Dismiss (Release all / Dismiss all) act on every one it counts, as the person (a press that leaves any held, such as a repo message no agent can take yet, puts the card back quietly with the error and its buttons; one already resolved elsewhere is no error); Pause-all holds stay quiet; a Board card an agent moved into the person's Needs you lane (Answer opens the Board at it, Later; the person's own moves, read from the task's `updatedBy`, stay quiet), and a message to the person (Open). The inbox stream drops changes for a slow reader, so the watcher re-reads the open deliveries each time it has drained the stream, and the Board on every coord change. A card body opens the window at its tab (`app:open`). The `notify` tool posts to the person at this device; with buttons, the answer (button, dismissed, or no answer after 5 minutes) reaches the agent at its next turn from Irori; 5 cards per agent per 10 minutes, the slot taken when the count is checked so parallel calls can't beat it. Headless never shows cards. Still to do: other devices (v0.4), share requests, "Notify me when…".
