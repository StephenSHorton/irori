# Stop here / Stop everywhere (was Wind down): design

*2026-11-01, card #220, from Stephen's #219: "ending the day or switching accounts means winding every agent down by hand." Builds on Stop and Pause (#207, `docs/design/stop-pause.md`) on main @ 98226e4. A switch since card #787 (2026-11-07, from #773: "Wind down is meant to be used when someone literally wants everyone to stop working … it should work the same way where it's a toggle").*

**Update, card #787 (Stephen on #773):** what was Wind down is now two switches that stop work the same way and differ only in reach. "Wind down" and "Pause agents" are gone as names in the app:

- **Stop here** (this machine; off: **Start here**) is Pause agents (#665) made graceful. From the app it holds agent starts on this machine *and* winds its agents down: each writes its hand-off and leaves its card, and the cards go back in the pool, where the person's other machines with free slots start agents on them. The `agents_pause` tool still only holds starts.
- **Stop everywhere** (every machine; off: **Start everywhere**) does the same on every machine at once, and nothing starts anywhere until it is turned off. Stephen: "When I hit wind down I expect all agents across my devices to pause and wind down"; there is no scope choice.
- The old **Pause all** switch ("Pause wakes on <machine>", which held agents' messages) is removed from the app (#787 b2): Stop everywhere is the only "stop all". The inbox policy stays in the code.

The code keeps its names (`wind_down*`, `spawn_pause`, `WindDownStatus`). Below, "Wind down" means Stop everywhere and "Pause agents" means Stop here, unless noted.

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

**On** (`Window::wind_down_all(who)`, no scope of machines: every device that can run agents, and every device with a live agent). `who` (`StopWho`, decision 71) picks which kind of agent: On cards (Irori started it: `Core::agent_spawned`, the roster's `spawned`), Yours (the person's terminal sessions) or both, the default. Steps 1 and 2 are for On cards (alone or with Yours); Yours alone is §2.1.

1. **Starts stop first.** Stop everywhere's own pause goes on: a pause row for the reserved device `everywhere` (`spawn_pause::EVERYWHERE`, `paused:everywhere`; device ids are hex, so it names none). While it is on, no device starts an agent, whatever its own pause says: a device that joins later, or one whose own pause was lifted (Start here, or an agent's `agents_pause` off, which is refused while it is on). Then Pause agents (#665, `spawn_pause.rs`) turns on for each device, so an older Irori holds its starts too, written by this device for the others (the `spawn.cap` row replicates; each device reads it at its next sweep, every 5 s). Its `by` is `winddown:<this device id>` (`spawn_pause::WIND_DOWN_BY_PREFIX`), not `user:…`, so Off can tell Wind down's pauses from the person's. A device already paused keeps its pause and its `by`. So no fresh agent takes a card the wound-down agents leave, on any device: nothing starts anywhere until Off.
2. **Then the wind-downs** (§1), one per live agent on every device, as a run.

**The switch's state** is read from the pause rows, not kept in the window: Stop everywhere is on while its own `everywhere` pause is on, or (from an older Irori) any device's Pause agents is on with a `winddown:` `by` (`WindDownStatus.on`, with `since` and the `devices` it paused). It reads on even when every device was paused before it went on. So every device shows it, and it outlives a restart (the run's rows don't: they are this device's memory).

**Off** (`Window::wind_down_off()`), at any time, including mid-run:

1. Every open wind-down that hasn't reached its agent (queued or held) closes as skipped with `inbox::NOTE_WIND_DOWN_OFF` (`Inbox::cancel_wind_down`). One another device hosts is asked of its host (`delivery.decided`, choice `cancel`); an older Irori ignores the ask and winds that agent down anyway. A wind-down already delivered is left: its agent is winding down and finishes.
2. Every pause with a `winddown:` `by` lifts, on every device, then the `everywhere` pause: one switch for the person. A pause the person (or an agent) set stays. A Stop here run on this machine is left going: only Start here ends it.
3. Lifting kicks the sweep: the cards the agents left start in queue order.

Step 1 leaves a Yours-only run's wind-downs open (§2.1): they are told apart by their words (`WIND_DOWN_YOURS_TEXT`, `WIND_DOWN_YOURS_STOP_TEXT`), which travel with the message, so Start on another device running this build or later, or after a restart, leaves them too. Start here (`pause_here(false)`) follows the same rule. An older Irori's Start here or Start everywhere still closes them, as it closed every unreached wind-down before (its `cancel_unpaused` has no text check, and a `cancel` request is carried out by the host whatever the text); the sessions are then simply not asked, and nothing is held or stopped wrongly.

### 2.1 Yours alone (decision 71)

Irori never starts a terminal session, so there is nothing to hold: a Yours-only Stop (either Stop) writes no pause row, lights no switch and sends the wind-downs, in the Yours words, only to the terminal sessions (this machine's for Stop here), as a run with `who.cards == false`. It is a one-off: Stop here with Yours runs again each time it is clicked, and with nobody to ask it starts no run. Its rows show in the dialog and on Home ("Asking 2 of yours to hand off"). **Call off** (`Window::wind_down_call_off()`) ends it: every Yours-words wind-down not reached yet closes as with Off (every device's, through the host's `cancel` request), the ones already delivered finish, and nothing is lifted. It does not need the run in memory: `WindDownStatus.yoursWaiting` counts those asks from the inbox, so a later Stop that replaced the Yours run (a Cards-only Stop, or Stop here after Stop everywhere) or a restart leaves them offered to Call off. It leaves asks in the held words, and the rows of a held run in this window's memory (a Yours ask that run took over), for Start to end; after a restart a Yours ask a held run had taken over is Call off's too.

Which agents are On cards is read after the hold, from the spawned mark, which is add-only (`spawned_keys`): an agent Irori started stays On cards for good (the set is folded from the log, so a restart keeps it), and a mark that arrives late leaves the agent under Yours (a held Stop then misses it, and a Yours Stop asks it to hand off, which is the side that never stops a terminal session wrongly). The Office Assistant is not on the roster (presence skips its run dir), so no Stop reaches it.

**Pause all after is gone** (it was on by default): Pause agents on every device covers "nothing starts", and the old Pause all switch has left the app (above).

## 3. A run

Stop here's graceful stop is the same run (`Window::pause_here`, scope `device`): turning it on sends a wind-down to each of this machine's agents; off closes the ones not reached yet. Off of Stop everywhere closes the wind-downs not reached yet only for machines no pause still holds, so a machine stopped on its own keeps its graceful stop. An agent already winding down is not sent a second one; its row follows the first.

`wind_down_all` returns the run: an id, its `who`, per agent its key, name, device, delivery and `spawned` (the dialog groups the rows by it). It is kept in memory (one at a time; a new On replaces it).

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

All the switches' words live in one place (`frontend/src/lib/stop-labels.ts`, and the tray's in `tray_text.rs`).

- **Header:** "Stop here" (Pause agents for this machine; on: "Stopped here · 12 min", click to Start here) and "Stop everywhere" (off: opens the dialog; on: "Stopped everywhere · 12 min", naming the machines it holds; click to Start everywhere). Stop here hides while Stop everywhere is on. Stop here is one click with this computer's kept choice of who (decision 71, `frontend/src/lib/stop-who.ts`, localStorage `irori.stop.who`, both by default); a chevron beside it shows the two ticks with this machine's counts, and its hover names the group. Stop here with Yours alone lights nothing and opens the dialog on its run; it is disabled, saying why, when nothing is ticked or none of yours run on this machine.
- **Tray:** a check item. Off: "Stop everywhere…" opens the window on the dialog. On: checked; clicking it starts everywhere. The old "Pause wakes on <machine>" item is gone.
- **Palette:** "Stop everywhere", or "Start everywhere" while it is on.
- **The dialog:** off, what On does (worded for the ticks: every agent, the agents Irori started with nothing starting until Start, or your own sessions asked once with nothing held) and two ticks, "On cards (N)" and "Yours (M)" (counts from the roster's `spawned`; no choice of machines), with the button "Stop N agents", or "Ask M of yours to hand off" for Yours alone, off when nothing is ticked, and with "everywhere" added under a Stop here title ("Stop N agents everywhere"), since it still reaches every machine; a Yours-only run has Call off in place of Start, and Call off also shows ("N of yours not reached yet") while any Yours ask is waiting (`yoursWaiting`), after a later Stop or a restart, and a run of both kinds lists its rows under On cards and Yours; on, which devices it holds since when, a Turn off button (mid-run too), the live list with Stop per row and a summary ("4 of 5 wound down").

## 5. Tests

- inbox: reason `wind_down` refused from an agent; a reply to one (wake or stop) is answered with its text kept and reaches neither the person's inbox nor the watcher's toasts, while an ordinary reply still does; Pause all doesn't hold it; a paused inbox plans a wake (and an append) for it and for nothing else; it waits behind a running wake (not cut); the preamble carries the instructions and `wind_down="true"`.
- inbox (#787): the person's wind-down to a session that ended closes (no answer), their other messages still stay; Off closes only wind-downs not reached yet; another device's is asked of its host, which closes it (and ignores the ask for anything else).
- core (#787): an agent that left its card while winding down is ended once it answered, not before; one that never answers is ended after the wait and its wind-down closes; Wind down holds another device's starts and Off lifts only the pauses it set.
- window (decision 71): On cards alone reaches only spawned agents and holds starts; Yours alone reaches only terminal sessions, writes no pause row and lights no switch; Stop here takes each choice; a Yours run survives Start here and Start everywhere (also its straggler's Stop), a held run that took over a Yours ask closes it on Start, and Call off closes only Yours asks not reached yet and lifts nothing, also after a later Stop replaced their run and after a restart (a new Window), and leaves a held run's rows.
- frontend (decision 71): the kept choice (none, unreadable, nothing ticked or no storage read as both), counts by the spawned mark, the button's words (with "everywhere" under a Stop here title), the explanation for each choice, Stop here's hover and disabled reason with nothing to do, the Yours line and "called off", and no label saying Pause.
- window: a run's status from delivery states, with each reply; a straggler's Stop is followed by its row; On holds starts at once and Off cancels and lifts; a device paused before stays paused; a row whose agent ended reads no answer and the run finishes.
- agent-check: a busy agent holding a card and a lease gets Wind down: it finishes its tool call, leaves the card with a hand-off comment, releases the lease and replies.
