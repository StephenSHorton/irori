# Later and order: review findings to fix (card #570)

From the review of the coord build. None checked or fixed yet: the fix stage was stopped by a wind-down. Delete this file once they are dealt with.

## 1. revise with no options closes a circle that was never checked — fixed: revise runs the circle check when a task wait loses its options.

## 2. raise with options on a same-question item writes an unchecked edge — fixed: raise_locked checks the circle when the same-question item it merges into has no options.

## 3. ask_close into a held parent inherits a hold with no circle check (medium)

`crates/coord/src/board/asks.rs:1814`

Ask A is open. add_after(X, [A]). park(P, Back::Card(X)). The person runs ask_close(A, outcome card, parent=P). This path calls only check_parent (asks.rs:1723) and then inherit_park, never check_adopt, so A takes P's item on X. Observed: close returns Ok with state blocked, A waits on X, X waits on A, and wait_chain shows the loop. P waits on X, so it never comes back either. split refuses the same case through check_adopt's Back::Card branch. Fix: call check_adopt(&pid, id) before the first write. Its held check needs care for an ask whose taker keeps it.

## 4. add_after treats a hold or a reason-only waits_on as order, so the order can vanish (medium)

`crates/coord/src/board/later.rs:865`

have = task_waits(&t) counts every task wait, and new_waits has the same skip at later.rs:959, so no after row is written and the call still returns Ok. Case 1: park(A, Back::Card(K)), add_after(A, [K]), bring_back(A). Observed: A is submitted with no waits while K is still open, so A goes out ahead of K. Case 2: a waits_on card, then add_after on the same card, then block(reason). Observed: 1 open item, task_waits empty. Case 3: NewTask with blocked_by X and after [X], then block(reason) gives the same result. Doc section 2 says after items are not reasonOnly precisely because a later block replaces those. Fix: count only open, non-later, non-legacy waits as already after, and write a real after item otherwise.

## 5. Loops through a parent, or longer than 64 cards, surface nowhere (medium)

`crates/coord/src/board/wait_chain.rs:13`

Doc section 3 says a loop two devices make must still show. The write check uses order_edges, but the window's read check (window/src/cards.rs:3053) uses wait_chain, which has no parent-to-subtask edge and stops at MAX_CARDS=64. board() runs order_loop only for parked cards (comments.rs:1416). Case 1: K is a subtask of P, K is after X, and a remote row makes X wait on P. Observed: order_reaches is true both ways, wait_chain gives no loop and no stuck card for P, K or X, and board() gives needs_you 0, blocked 3. Case 2: a 70-card ring closed by a remote row. Observed: order_reaches is true both ways, wait_chain loop_ids is empty from cards 0 and 30, stuck is empty. The cards stall forever and nothing says so. Fix: give read-time detection the same edges as the write check, with no cap.

## 6. A person's board() read goes quadratic on chains of parked cards, under the coord lock (low)

`crates/coord/src/board/comments.rs:1416`

Release build, a chain where each card is parked until the one before. board() as a person: 500 cards 30ms, 2000 cards 0.89s, 5000 cards 7.0s. The same 5000-card log read as an agent takes 5.9ms; a plain after chain read as a person takes 12.6ms. Each parked card runs its own BFS via order_loop, all inside the lock. notify/src/watch.rs:583 reads as the person; api/src/board.rs:135 passes the caller's key, so the cost lands there only when the caller is the person. Write time is fine: one order_reaches across 5000 cards took 2.5ms, a refused add_after 7.7ms. Date and word parks have no edges, so only long card chains pay this. Fix: find loops once per read in one pass over the graph.

## 7. A parent can't be held until its own held subtask (low)

`crates/coord/src/board/later.rs:519`

kids_to_park checks held_by_someone(k) before the 'hold names this very subtask' skip, and check_adopt does the same at later.rs:293. K is a subtask of P with BOB on it. park(USER, P, why, Back::Card(K)) is refused: 'subtask #2 of #1 is held: it finishes first'. With K unheld the same call succeeds and K stays free, so the hold never touches K. Adopt case: P is parked until K, BOB claims K, then split(P, adopt K) is refused with '#2 is held, and #1 is Not yet', although inherit_park would leave K alone. Doc section 3 says 'Parking a parent with a held subtask is refused', so the park case follows the doc's letter. It still contradicts the builder's rule that this subtask stays free, so the person should decide. The fix is to move the skip above the held check in both places.

## 8. New-card question clash overwrites a wait instead of refusing — fixed: new_waits compares after items, the card's own reason and the inherited hold, and refuses a clash.

## 9. remove_after by #N can't clear a wait on a deleted card (low)

`crates/coord/src/board/later.rs:908`

Next to doc section 5 bug 3. A is after #1, the person deletes #1, and the sweep leaves A blocked with the wait still open. remove_after(A, ['#1']) fails with 'no such task or lease: #1 was deleted', because task_ref_locked refuses deleted short refs. remove_after(A, [raw id]) works and A goes to submitted. Someone holding only the short ref is told the card is gone while A stays stuck. Fix: resolve deleted refs for removal, or match against the item's on.reference.

## 10. An agent can take or be handed a held card through a step owner (high)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/cards.rs:2270`

VERIFIED by running probes a1, a2 and a3 (all probes are in /tmp/rv570/crates/coord/tests/review_probe.rs, a scratch copy of 5898f117; the worktree is unchanged). set_step_owner (:2270) and set_steps (:2027) have no parked check, and both are agent tools (board_step at tools/src/board.rs:1989, board_steps at :1896). Scenario: a person parks a one-step card on Back::Word. claim_task(BOB) is refused NotAllowed, but set_step_owner(BOB, id, s1, BOB) returns Ok and co_holders becomes [bob]: the agent is on a card claim just refused it. Handing it to someone else: set_step_owner(ALICE, id, s1, CAROL) sends CAROL a step_owner notice with wake=true saying 'gave you a step... You hear the card like a holder', so an idle agent is woken onto a held card. set_steps_notice with an owner does the same. Read, not run: co_holders then makes held_by_someone true, so kids_to_park refuses parking its parent, and park refuses agents on that card. Fix at write time: refuse a new owner on a parked card for agents, and treat a person's as their word or refuse it too.

## 11. A block or raise with the hold's own words rewrites its `on` — fixed: raise_locked (block too) refuses when the same-question item is a hold.

## 12. A sweep that hasn't seen the hold writes blocker.closed and ends a word hold on the device that has it (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/comments.rs:1614`

VERIFIED by running probe j2 through the real sweep on two_people, with no hand-built row. Card T has an after-wait on F, raised by p2. Offline from each other, p1 parks T on Back::Word on dev-a, while on dev-b p2 completes F and sweeps. dev-b sees only the wait, so ids == count_open with no later item, and it writes blocker.closed at :1614. After sync, dev-a shows state=submitted parked=false: the person's word hold is gone. dev-b shows state=blocked parked=true, so the devices disagree. Doc section 2 forbids blocker.closed for this reason, but only the park's own return was moved to per-item rows. Any plain wait beside a hold still goes through close_blocker, on new builds too. Probe j1 shows the same effect on a private log using a hand-built row. Fix at write time: sweep_rest always closes per item with blocker.resolved. Old cores stay under known limit 1.

## 13. On a board, another person's park can't end a pending spawn, so an agent binds onto the held card (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/spawn.rs:449`

VERIFIED by running probe i1. The builder's report mentions the row being ignored; this probe shows what follows, which breaks 'never spawned' now rather than in a later stage. On two_people, p1 makes a card and request_spawn is pending on dev-a. p2 parks it on dev-b. end_pending_spawns writes the cancel, but spawn_writer_ok ignores it on both folds (ignored_rows=1 on each side) and park still returns Ok. After sync the spawn is still pending on dev-a. bind_spawn(id, g, agent:c1) returns Ok(true) and leaves holders=[agent:c1] on a parked BLOCKED card. spawns_here stops a session that hasn't started, so this needs one already starting. The bind loops (core spawn.rs:1945, spawn_boards.rs:263) have no state check. Read, not run: when the hold ends, restore_state gives WORKING if that agent has taken the card up, which is the silent return the doc's holder rule exists to prevent. Fix: bind_spawn refuses on a parked card and ends the spawn from the author's device, or park refuses when it can't cancel.

## 14. A wait on a gone agent beside a hold puts the held card in Needs you (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/comments.rs:1551`

VERIFIED by running probes c1 and c2. check_parkable only refuses person items, so these waits are allowed beside a park. c1: agent BOB holds a card, raises an external wait 'Vendor reply' (no PR), then parks it; the lane is later. BOB goes gone and the lane becomes needs_you through gone_waits_in at :1551. Read, not run: a spawned holder is ended right after a park, so this may need no extra step. c2: a wait on agent CAROL plus a park, CAROL gone, also gives needs_you. The looped branch in board() is the spec's one exception, and this isn't it. Read, not run: hand_on_refusal now returns 'bring it back first', so the row offers only bring-back. c1 can't be refused at write time because the agent goes after the park, so the fix belongs at read time: skip gone waits for a parked card when only held items and waits remain.

## 15. Revising a parent's hold breaks same_hold, so bringing it back leaves its subtasks held on the person's word (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/later.rs:139`

VERIFIED by running probe e1. A person parks a parent on Back::Word, and its subtask takes the same item. Any agent revises the parent's item with board_block blocker_id, setting the question to 'Not yet: until the spring'. revise has no parked check, and only the parent's row changes. The person calls bring_back(parent). The parent returns, but same_hold compares later and question, so close_kid_parks finds no match. The result is subtask state=blocked parked=true on 'word', with no way back except a separate bring-back by a person. Read, not run: revising the subtask instead has the same effect, and a later park replace misses it too because `same` comes back None. Fix: match on something revise doesn't change, or keep the question in step.

## 16. A hold accepts options through revise — fixed: revise refuses options on a hold.

## 17. Changing a parent's word hold to wait on its own subtask leaves that subtask held, so neither comes back (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/later.rs:526`

VERIFIED by running probe f1. A parent with two subtasks is parked on Back::Word, and both subtasks take the item. The person changes the hold with park(p, 'after part 0', Back::Card(k0)). kids_to_park hits `continue` at :526 for k0 before it looks at k0's old item, so k0 keeps 'Not yet: not now' on word. The parent now waits for k0 to finish while k0 waits for a person, and the two are no longer the same hold, so bring_back(parent) wouldn't reach k0 (read, not run). This goes against the builder's rule that a parent parked until its own subtask leaves that subtask free. Fix: close k0's matching item in that branch.

## 18. Reopening a finished subtask of a held parent sends it out with no hold (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/tasks.rs:572`

VERIFIED by running probes b1 and b2. A parent has two subtasks. One is canceled, so parking skips it as terminal, and then the parent is parked on Back::Word. The person reopens the canceled subtask from the window. reopen_locked only looks at a terminal parent, so the subtask is state=submitted with no hold. sweep_asks makes 1 offer (b1) and cards_to_spawn lists it (b2), although doc section 3 says subtasks of a held parent carry the hold. waits_for_lead doesn't help, because the parent's holders were dropped. Only a person can trigger this: there is no reopen in tools/src/board.rs, tools/src/lib.rs or api/src. Fix: run inherit_park after the reopen row.

## 19. A park replace crossing a close of the same id leaves one device BLOCKED with nothing open, and ends the new hold on the other — fixed: changing a hold closes the old item per item and raises a fresh id, so no replace crosses its close.

## 20. A hold on a parent finished by rollup, or on a deleted card, never returns (known section 5 bugs 2 and 3) (low)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/comments.rs:1318`

VERIFIED by running probes h1 and h2. These are doc section 5 bugs 3 and 2, left alone as the task allowed, and listed because a hold makes them lasting. h2: park T with Back::Card(P), then complete P's only subtask. P's raw state stays submitted while its lane is done, so work_done (:2019) never fires, and after a sweep T stays blocked and parked with nothing shown. h1: park T with Back::Card(X), then the person deletes X. gone_away acts only on Some(o) at :1318, so T stays parked. check_back accepts a parent target, and agents are refused on the held card, so only a person's bring-back recovers it. Consider fixing bug 2 before the agent tool ships, since 'not yet, until that parent is done' is a natural thing to ask for.

## 21. A taken ask closed as a card under a held parent stays WORKING with no hold (low)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/asks.rs:1814`

VERIFIED by running probe l1. A parent is parked on Back::Word. BOB claims an ask and closes it with outcome card, parent=P. asks.rs writes state working with BOB as holder, and inherit_park returns silently because held_by_someone is true. The result is state=working, holders=[bob], parked=false, parent=P: a working subtask under a held parent, which the park path refuses for a held subtask. No hand-out happens, since BOB already had the ask. There is no write-time refusal or notice. Fix: refuse a held parent as the outcome's parent, or take the taker off and hold the card.

## 22. Changing a hold races its close: the card sticks BLOCKED with no open item and the devices disagree — fixed: park and a hold's revise close the old item and raise a new one with a fresh id, never a replace; no sweep heal, see later-order.md section 4.

## 23. A board park by anyone but the card author's person leaves the pending spawn, and the agent binds onto the held card (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/spawn.rs:185`

Ran on a temp copy of HEAD. Board card by P1 with a pending spawn on dev-a. P2 parks it from dev-b. end_pending_spawns (later.rs:606) writes task.spawn canceled, but spawn_writer_ok requires the writer to be the author's person, so the row is dead.

Result on dev-a: spawn_pending=true, ignored_rows=1, parked=true. Then bind_spawn(id, g, CARD1) returned Ok(true), because bind_spawn (spawn.rs:465) has no parked check and writes task.spawned and task.assigned as Irori. Both devices then show state=blocked, holders=[agent:c1], assigned=[agent:c1], parked=true. That breaks 'never spawned', and section 3 says a kept holder comes back silently as WORKING.

The builder noted only that the spawn isn't really ended, not that the agent lands on the card. The same park by P1, by CARD1 or on the private log ended the spawn cleanly (pending=false, ignored=0).

Fix at write time, never in the fold: bind_spawn skips or ends a parked card's request on the author's device, or park refuses while another person's spawn is pending.

## 24. Two devices raising on one card share a blocker id: one row is dropped, and a date close can end a word hold — fixed: hold and after items take a b<n>-<random> id no other device picks; plain questions keep the counter, see later-order.md section 4.

## 25. v0.12.3's answered_left sweep ends a hold, a word hold included, after a person replies on its thread (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/cards.rs:1567`

Ran end to end. A HEAD copy produced the rows: board card, agent:c1 claims it, parks it with Word (task.left puts c1 in t.left), then P1 answers on the hold. HEAD's own sweep leaves it parked.

Those rows were replayed on the real v0.12.3 build as a device of the raiser's person (p1). Its own sweep wrote blocker.resolved by irori, closed=true, note 'the agent it went to left the card', and the card went submitted, live and after a restart. That row folds on every device, so the hold ends everywhere and the card goes out.

answered_left is in v0.12.3 (comments.rs:1200) and absent at v0.10.2 and v0.11.0. By reading, it needs may_write(raiser, dev), so this is a board with the same person on a v0.12.x device. Private logs should not reach it, because the agent's own device is the new one.

Section 3's 'answered_left skips later items' holds only on new builds, and known limit 1 names older agents and Stop waiting, not an old sweep. A new-side mitigation at write time: don't mark a later item ANSWERED in answer() (keep the reply as a plain comment), since that status is what arms the old sweep. At minimum, add it to section 4.

## 26. A non-raiser's sweep writes a dead blocker.resolved every pass for plain after items (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/comments.rs:1641`

Pre-existing: this is the per-item side of section 5 bug 1, which the brief left alone. may_close_later (later.rs:809) guards only items with later, so the new after waits take the unguarded path, and add_after makes several waits per card common on boards.

Ran on a temp copy of HEAD. Board with owners cleared. P1's card, with add_after(CARD1, [x, y]) raised by p1's agent, x completed, synced. Four sweeps on p2's device (not raiser, creator or owner) wrote 4 rows; ignored_rows went 0 to 4; both items stayed open. sweeper_ok refuses each row on every tag, because raiser_closes and may_cancel are both false.

The sweep runs every 5s, so that is about 17,000 dead rows a day per such device in the shared log, until y closes and the raiser's device acts. Applying the same writer check before the per-item write and before close_blocker would stop it without touching the fold.

## 27. A subtask made on an old device under a held parent is not held, and an old origin hands it out (low)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/later.rs:313`

From reading only, not run. inherit_park runs only in this build's write paths (create_locked, split, asks.rs). A v0.12.3 agent running board_split on a parked parent writes a plain task.created and task.parent with no item. The subtask is SUBMITTED on every device. waits_for_lead at v0.12.3 (parts.rs:146) needs the parent to have holders, and park just dropped them. So the card's old origin offers or spawns it while the parent is held.

No fold rule can fix this, by design. But known limit 1 says 'Hand-out isn't affected', which is wrong for this case, and section 3 says new subtasks inherit the hold without the limit. Either state the limit in section 4, or have a new device's sweep put the parent's item on an unheld, unassigned open subtask of a parked parent.
