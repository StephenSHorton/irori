# Later and order: review findings to fix (card #570)

From the review of the coord build. None checked or fixed yet: the fix stage was stopped by a wind-down. Delete this file once they are dealt with.

## 1. revise with no options closes a circle that was never checked — fixed: revise runs the circle check when a task wait loses its options.

## 2. raise with options on a same-question item writes an unchecked edge — fixed: raise_locked checks the circle when the same-question item it merges into has no options.

## 3. ask_close into a held parent inherits a hold with no circle check — fixed: ask_close runs the adopt check, circle included, before its first write.

## 4. add_after treats a hold or a reason-only waits_on as order, so the order can vanish (medium)

`crates/coord/src/board/later.rs:865`

have = task_waits(&t) counts every task wait, and new_waits has the same skip at later.rs:959, so no after row is written and the call still returns Ok. Case 1: park(A, Back::Card(K)), add_after(A, [K]), bring_back(A). Observed: A is submitted with no waits while K is still open, so A goes out ahead of K. Case 2: a waits_on card, then add_after on the same card, then block(reason). Observed: 1 open item, task_waits empty. Case 3: NewTask with blocked_by X and after [X], then block(reason) gives the same result. Doc section 2 says after items are not reasonOnly precisely because a later block replaces those. Fix: count only open, non-later, non-legacy waits as already after, and write a real after item otherwise.

## 5. Loops through a parent, or longer than 64 cards, surface nowhere (medium)

`crates/coord/src/board/wait_chain.rs:13`

Doc section 3 says a loop two devices make must still show. The write check uses order_edges, but the window's read check (window/src/cards.rs:3053) uses wait_chain, which has no parent-to-subtask edge and stops at MAX_CARDS=64. board() runs order_loop only for parked cards (comments.rs:1416). Case 1: K is a subtask of P, K is after X, and a remote row makes X wait on P. Observed: order_reaches is true both ways, wait_chain gives no loop and no stuck card for P, K or X, and board() gives needs_you 0, blocked 3. Case 2: a 70-card ring closed by a remote row. Observed: order_reaches is true both ways, wait_chain loop_ids is empty from cards 0 and 30, stuck is empty. The cards stall forever and nothing says so. Fix: give read-time detection the same edges as the write check, with no cap.

## 6. A person's board() read goes quadratic on chains of parked cards, under the coord lock (low)

`crates/coord/src/board/comments.rs:1416`

Release build, a chain where each card is parked until the one before. board() as a person: 500 cards 30ms, 2000 cards 0.89s, 5000 cards 7.0s. The same 5000-card log read as an agent takes 5.9ms; a plain after chain read as a person takes 12.6ms. Each parked card runs its own BFS via order_loop, all inside the lock. notify/src/watch.rs:583 reads as the person; api/src/board.rs:135 passes the caller's key, so the cost lands there only when the caller is the person. Write time is fine: one order_reaches across 5000 cards took 2.5ms, a refused add_after 7.7ms. Date and word parks have no edges, so only long card chains pay this. Fix: find loops once per read in one pass over the graph.

## 7. A parent can't be held until its own held subtask — fixed: park and adopt skip the hold's own subtask before the held check, so it stays free.

## 8. New-card question clash overwrites a wait instead of refusing — fixed: new_waits compares after items, the card's own reason and the inherited hold, and refuses a clash.

## 9. remove_after by #N can't clear a wait on a deleted card (low)

`crates/coord/src/board/later.rs:908`

Next to doc section 5 bug 3. A is after #1, the person deletes #1, and the sweep leaves A blocked with the wait still open. remove_after(A, ['#1']) fails with 'no such task or lease: #1 was deleted', because task_ref_locked refuses deleted short refs. remove_after(A, [raw id]) works and A goes to submitted. Someone holding only the short ref is told the card is gone while A stays stuck. Fix: resolve deleted refs for removal, or match against the item's on.reference.

## 10. An agent can take or be handed a held card through a step owner — fixed: set_step_owner and set_steps refuse a new owner on a held card for an agent, as claim does; a person's ends the hold first, as assign does.

## 11. A block or raise with the hold's own words rewrites its `on` — fixed: raise_locked (block too) refuses when the same-question item is a hold.

## 12. A sweep that hasn't seen the hold writes blocker.closed and ends a word hold on the device that has it — fixed: sweep_rest closes every item per item with blocker.resolved, never blocker.closed.

## 13. On a board, another person's park can't end a pending spawn, so an agent binds onto the held card — fixed: bind_spawn refuses a held card and ends its request from the author's device; the board bind loop ends that session.

## 14. A wait on a gone agent beside a hold puts the held card in Needs you (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/comments.rs:1551`

VERIFIED by running probes c1 and c2. check_parkable only refuses person items, so these waits are allowed beside a park. c1: agent BOB holds a card, raises an external wait 'Vendor reply' (no PR), then parks it; the lane is later. BOB goes gone and the lane becomes needs_you through gone_waits_in at :1551. Read, not run: a spawned holder is ended right after a park, so this may need no extra step. c2: a wait on agent CAROL plus a park, CAROL gone, also gives needs_you. The looped branch in board() is the spec's one exception, and this isn't it. Read, not run: hand_on_refusal now returns 'bring it back first', so the row offers only bring-back. c1 can't be refused at write time because the agent goes after the park, so the fix belongs at read time: skip gone waits for a parked card when only held items and waits remain.

## 15. Revising a parent's hold breaks same_hold, so bringing it back leaves its subtasks held on the person's word — fixed: rewording a hold rewords what its subtasks took too, and a revise of an inherited hold on the subtask is refused.

## 16. A hold accepts options through revise — fixed: revise refuses options on a hold.

## 17. Changing a parent's word hold to wait on its own subtask leaves that subtask held, so neither comes back — fixed: changing a hold closes what every subtask took before raising the new one (a67c7a55); regression test added.

## 18. Reopening a finished subtask of a held parent sends it out with no hold — fixed: reopen_locked runs inherit_park after the reopen row.

## 19. A park replace crossing a close of the same id leaves one device BLOCKED with nothing open, and ends the new hold on the other — fixed: changing a hold closes the old item per item and raises a fresh id, so no replace crosses its close.

## 20. A hold on a parent finished by rollup, or on a deleted card, never returns (known section 5 bugs 2 and 3) (low)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/comments.rs:1318`

VERIFIED by running probes h1 and h2. These are doc section 5 bugs 3 and 2, left alone as the task allowed, and listed because a hold makes them lasting. h2: park T with Back::Card(P), then complete P's only subtask. P's raw state stays submitted while its lane is done, so work_done (:2019) never fires, and after a sweep T stays blocked and parked with nothing shown. h1: park T with Back::Card(X), then the person deletes X. gone_away acts only on Some(o) at :1318, so T stays parked. check_back accepts a parent target, and agents are refused on the held card, so only a person's bring-back recovers it. Consider fixing bug 2 before the agent tool ships, since 'not yet, until that parent is done' is a natural thing to ask for.

## 21. A taken ask closed as a card under a held parent stays WORKING with no hold — fixed: ask_close refuses a held parent when the taker keeps the card.

## 22. Changing a hold races its close: the card sticks BLOCKED with no open item and the devices disagree — fixed: park and a hold's revise close the old item and raise a new one with a fresh id, never a replace; no sweep heal, see later-order.md section 4.

## 23. A board park by anyone but the card author's person leaves the pending spawn, and the agent binds onto the held card — fixed: a park writes a cancel only where it counts, and the author's device ends the request on bind or in its sweep.

## 24. Two devices raising on one card share a blocker id: one row is dropped, and a date close can end a word hold — fixed: hold and after items take a b<n>-<random> id no other device picks; plain questions keep the counter, see later-order.md section 4.

## 25. v0.12.3's answered_left sweep ends a hold, a word hold included, after a person replies on its thread (medium)

`/Users/stephenhorton/projects/irori/.claude/worktrees/card-570-later-order/crates/coord/src/board/cards.rs:1567`

Ran end to end. A HEAD copy produced the rows: board card, agent:c1 claims it, parks it with Word (task.left puts c1 in t.left), then P1 answers on the hold. HEAD's own sweep leaves it parked.

Those rows were replayed on the real v0.12.3 build as a device of the raiser's person (p1). Its own sweep wrote blocker.resolved by irori, closed=true, note 'the agent it went to left the card', and the card went submitted, live and after a restart. That row folds on every device, so the hold ends everywhere and the card goes out.

answered_left is in v0.12.3 (comments.rs:1200) and absent at v0.10.2 and v0.11.0. By reading, it needs may_write(raiser, dev), so this is a board with the same person on a v0.12.x device. Private logs should not reach it, because the agent's own device is the new one.

Section 3's 'answered_left skips later items' holds only on new builds, and known limit 1 names older agents and Stop waiting, not an old sweep. A new-side mitigation at write time: don't mark a later item ANSWERED in answer() (keep the reply as a plain comment), since that status is what arms the old sweep. At minimum, add it to section 4.

## 26. A non-raiser's sweep writes a dead blocker.resolved every pass for plain after items — fixed: sweep_rest runs may_sweep_close (raiser's, creator's person or owner) before every item it closes, plain ones included.

## 27. A subtask made on an old device under a held parent is not held, and an old origin hands it out — fixed: a new device's sweep puts the parent's hold on a never-held subtask nobody is on, where the row counts; limit 1 corrected.

