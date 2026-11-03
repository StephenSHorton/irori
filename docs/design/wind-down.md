# Wind down all: design

*2026-11-01, card #220, from Stephen's #219: "ending the day or switching accounts means winding every agent down by hand." Builds on Stop and Pause (#207, `docs/design/stop-pause.md`) on main @ 98226e4.*

One action, **Wind down all**, in the app header, the tray and the command palette (#213, once it lands). It asks every agent of the person's to save its place, hand back its cards and stop, shows who has, and then turns Pause all on.

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

## 2. A run

`Window::wind_down_all(scope)` sends one wind-down to each live agent: `this` = this device's sessions, `all` = every device's. It returns a run: an id, the message ids and, per agent, its key, name and device. The run is kept in memory (one at a time; a new press replaces it).

`Window::wind_down_status()` reads each delivery and says, per agent:

| delivery state | shown |
|---|---|
| queued | waiting (not reached yet) |
| delivered, engaged, deferred | winding down |
| answered | wound down, with its reply (the cards it left), read from the delivery's note |
| failed, expired, skipped, canceled | no answer |

**Stop** on a row, for an agent that doesn't answer, is a Stop (#207: mode `stop`, cuts the running turn) that is itself a wind-down (reason `wind_down`), with the text "Wind down now: commit and push your work, post a hand-off and leave your cards, release your leases and reply \"wound down\"." The row follows the Stop's delivery from then on, and its reply stays out of the person's messages like the first one's.

**Pause all after** (on by default, a checkbox in the dialog): when no delivery of the run is still queued, delivered, engaged or deferred, the window sets Pause all (`inbox_set_paused(true)`) once. The check runs on every status read and on the window's event tick, so it fires even with the dialog closed. Pause all is this device's policy; with scope `all` the other devices' agents still wind down but their devices stay unpaused (said in the dialog).

## 3. The window

- **Header:** a Wind down button beside Pause all (the inbox header's switch), with a confirm that names how many agents and cards it reaches.
- **Tray:** "Wind down all agents…" opens the window on the dialog (the confirm stays in the window).
- **Palette (#213):** a "Wind down all agents" command, once the palette lands.
- **The dialog:** scope (this device / all my devices), Pause all after, then the live list with Stop per row and a summary ("4 of 5 wound down").

Resume the next day is the existing Pause all switch; the cards the agents left are unheld, so the Board hands them out again.

## 4. Tests

- inbox: reason `wind_down` refused from an agent; a reply to one (wake or stop) is answered with its text kept and reaches neither the person's inbox nor the watcher's toasts, while an ordinary reply still does; Pause all doesn't hold it; a paused inbox plans a wake (and an append) for it and for nothing else; it waits behind a running wake (not cut); the preamble carries the instructions and `wind_down="true"`.
- window: a run's status from delivery states, with each reply; a straggler's Stop is followed by its row; Pause all once every agent answered, not before.
- agent-check: a busy agent holding a card and a lease gets Wind down: it finishes its tool call, leaves the card with a hand-off comment, releases the lease and replies.
