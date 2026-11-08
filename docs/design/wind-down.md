# Wind down: design

*2026-11-01, card #220, from Stephen's #219: "ending the day or switching accounts means winding every agent down by hand." Builds on Stop and Pause (#207, `docs/design/stop-pause.md`) on main @ 98226e4. A switch since card #787 (2026-11-07, from #773: "Wind down is meant to be used when someone literally wants everyone to stop working … it should work the same way where it's a toggle").*

**Wind down** is an on/off switch, like Pause agents (#665), in the app header, the tray, the dialog and the command palette. On asks every agent of the person's, on every one of their devices, to save its place, hand back its cards and stop, and stops Irori starting agents on all of those devices from that moment. There is no Wind down for one device (Stephen on #773: "'winding down' one machine, there is no action for that"): stopping one machine is Pause agents on it, and its cards then start on other devices with room. Off, at any time, cancels what hasn't reached an agent yet and lets work start again.

## 1. The message

A wind-down is a person's message with reason `wind_down` (`inbox::REASON_WIND_DOWN`), mode `wake`, one per agent. It is not a fourth delivery mode: it must not cut a running turn (Stop does that), so it travels like any wake. What sets it apart:

- **Only the person sends it.** `send_with` refuses reason `wind_down` from an agent or the system.
- **Pause all doesn't hold it** (`policy::hold`), as it doesn't hold a stop: the person who paused is the one winding down. This matters when Wind down runs while Pause all is already on.
- **It waits for a safe point.** With Pause all off it is an ordinary wake: an idle agent wakes on it; a busy one gets it at its next tool round as an append, or after its turn (the outstanding wake's gate), never cut. With Pause all on, `next_wake` and `next_append` still plan wind-down deliveries (and only those).
- **The wake preamble says what to do** (`fence`): a message marked `wind_down="true"` is the person winding the agent down. At its next safe point (finish the step in hand, don't start new work) it:
  1. commits and pushes its WIP;
  2. ticks its current step with `left_off` and posts a hand-off comment on each card it holds (what's done, branch/sha, what's next);
  3. `board_leave`s each card, so it goes back unheld;
  4. releases its leases and cancels its waits;
  5. replies to the wind-down with "wound down" and the cards it left, then stops: it starts nothing else.

  **The reply goes to the dialog, not the person's messages** (#307). A reply to a wind-down (`fence::is_wind_down`: reason `wind_down`, trust user) sends no message back: `reply` marks the delivery answered and keeps the reply text on it (its `note`, at most `MAX_NOTE`), and returns the wind-down itself. So the person gets no message, notification or unread count per agent; the dialog reads the text from the delivery. The `reply` tool tells the agent so.

  Every agent gets the same words from the Core, so every agent does the same thing whatever its mod or channel.

## 2. The switch

**On** (`Window::wind_down_all()`, no scope: every device that can run agents, and every device with a live agent):

1. **Starts stop first.** Pause agents (#665, `spawn_pause.rs`) turns on for each device, written by this device for the others (the `spawn.cap` row replicates; each device reads it at its next sweep, every 5 s). Its `by` is `winddown:<this device id>` (`spawn_pause::WIND_DOWN_BY_PREFIX`), not `user:…`, so Off can tell Wind down's pauses from the person's. A device already paused keeps its pause and its `by`. So no fresh agent takes a card the wound-down agents leave, on any device: nothing starts anywhere until Off.
2. **Then the wind-downs** (§1), one per live agent on every device, as a run.

**The switch's state** is read from the pause rows, not kept in the window: Wind down is on while any device's Pause agents is on with a `winddown:` `by` (`WindDownStatus.on`, with `since` and the `devices` it holds). So every device shows it, and it outlives a restart (the run's rows don't: they are this device's memory).

**Off** (`Window::wind_down_off()`), at any time, including mid-run:

1. Every open wind-down that hasn't reached its agent (queued or held) closes as skipped with `inbox::NOTE_WIND_DOWN_OFF` (`Inbox::cancel_wind_down`). One another device hosts is asked of its host (`delivery.decided`, choice `cancel`); an older Irori ignores the ask and winds that agent down anyway. A wind-down already delivered is left: its agent is winding down and finishes.
2. Every pause with a `winddown:` `by` lifts, on every device: one switch for the person. A pause the person (or an agent) set stays.
3. Lifting kicks the sweep: the cards the agents left start in queue order.

**Pause all after is gone** (it was on by default): Pause agents on every device covers "nothing starts", and Pause all (this device's wakes only) stays its own switch.

## 3. A run

`wind_down_all` returns the run: an id, per agent its key, name, device and delivery. It is kept in memory (one at a time; a new On replaces it).

`Window::wind_down_status()` reads the switch and each delivery and says, per agent:

| delivery state | shown |
|---|---|
| queued, held | waiting (not reached yet) |
| delivered, engaged, deferred | winding down |
| answered | wound down, with its reply (the cards it left), read from the delivery's note |
| skipped with "wind down turned off" | turned off (canceled) |
| skipped, failed, expired | no answer |

The run is finished when every row is wound down, no answer or turned off.

**A row never stays winding for an agent that is gone** (#787):

- An agent Irori started leaves its card (step 3) before it replies (step 5). `end_finished` sees it off its card, and `sweep_ending` would end it at once because it has posted its hand-off. It now waits while the agent has a wind-down open (`winding_down`), up to `HANDOFF_WAIT`; once it replied it ends. When it ends without replying, its open wind-downs close (`close_wind_downs`, note "recipient gone").
- For any session (one the person started too): the inbox sweep closes the person's wind-down to a session that has been out of presence past `GONE_GRACE` (`Act::GoneWindDown`, note "recipient gone"). Other messages from the person to an ended session still stay.

Either way the row reads no answer and the run finishes.

**Stop** on a row, for an agent that doesn't answer, is a Stop (#207: mode `stop`, cuts the running turn) that is itself a wind-down (reason `wind_down`), with the text "Wind down now: commit and push your work, post a hand-off and leave your cards, release your leases and reply \"wound down\"." The row follows the Stop's delivery from then on, and its reply stays out of the person's messages like the first one's.

The window re-reads the run on every inbox change, and the switch every 5 s (it can move on another device), and says `windDown:changed` when either moved.

## 4. The window

- **Header:** off, a Wind down button that opens the dialog. On, the button is pressed, "Wound down · 12 min", naming the devices it holds; clicking it turns Wind down off. The separate Pause agents button hides while Wind down is on (its pause is Wind down's).
- **Tray:** a check item. Off: "Wind down all agents…" opens the window on the dialog (the confirm stays in the window). On: checked; clicking it turns Wind down off.
- **Palette:** "Wind down all agents", or "Turn Wind down off" while it is on.
- **The dialog:** off, how many agents it reaches and what On does (no scope choice); on, which devices it holds since when, a Turn off button (mid-run too), the live list with Stop per row and a summary ("4 of 5 wound down").

## 5. Tests

- inbox: reason `wind_down` refused from an agent; a reply to one (wake or stop) is answered with its text kept and reaches neither the person's inbox nor the watcher's toasts, while an ordinary reply still does; Pause all doesn't hold it; a paused inbox plans a wake (and an append) for it and for nothing else; it waits behind a running wake (not cut); the preamble carries the instructions and `wind_down="true"`.
- inbox (#787): the person's wind-down to a session that ended closes (no answer), their other messages still stay; Off closes only wind-downs not reached yet; another device's is asked of its host, which closes it (and ignores the ask for anything else).
- core (#787): an agent that left its card while winding down is ended once it answered, not before; one that never answers is ended after the wait and its wind-down closes; Wind down holds another device's starts and Off lifts only the pauses it set.
- window: a run's status from delivery states, with each reply; a straggler's Stop is followed by its row; On holds starts at once and Off cancels and lifts; a device paused before stays paused; a row whose agent ended reads no answer and the run finishes.
- agent-check: a busy agent holding a card and a lease gets Wind down: it finishes its tool call, leaves the card with a hand-off comment, releases the lease and replies.
