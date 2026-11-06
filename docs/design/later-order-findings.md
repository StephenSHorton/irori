# Later and order: review findings to fix (card #570)

From the review of the coord build. None checked or fixed yet: the fix stage was stopped by a wind-down. Delete this file once they are dealt with.

## 1. revise with no options closes a circle that was never checked — fixed: revise runs the circle check when a task wait loses its options.

## 2. raise with options on a same-question item writes an unchecked edge — fixed: raise_locked checks the circle when the same-question item it merges into has no options.

## 3. ask_close into a held parent inherits a hold with no circle check — fixed: ask_close runs the adopt check, circle included, before its first write.

## 4. add_after treats a hold or a reason-only waits_on as order, so the order can vanish — fixed: add_after and new_waits count only an open plain after item as already after, and write a real one beside a hold, a waits_on or a blocked_by.

## 5. Loops through a parent, or longer than 64 cards, surface nowhere — fixed: `order_loops` finds every loop on the write check's edges (subtasks included, no cap), and the window names loops from it.

## 6. A person's board() read goes quadratic on chains of parked cards, under the coord lock — fixed: board() runs `order_loops` once per read (one Tarjan pass), only when a held card asks.

## 7. A parent can't be held until its own held subtask — fixed: park and adopt skip the hold's own subtask before the held check, so it stays free.

## 8. New-card question clash overwrites a wait instead of refusing — fixed: new_waits compares after items, the card's own reason and the inherited hold, and refuses a clash.

## 9. remove_after by #N can't clear a wait on a deleted card — fixed: remove_after resolves a deleted card's short ref (removal_ref).

## 10. An agent can take or be handed a held card through a step owner — fixed: set_step_owner and set_steps refuse a new owner on a held card for an agent, as claim does; a person's ends the hold first, as assign does.

## 11. A block or raise with the hold's own words rewrites its `on` — fixed: raise_locked (block too) refuses when the same-question item is a hold.

## 12. A sweep that hasn't seen the hold writes blocker.closed and ends a word hold on the device that has it — fixed: sweep_rest closes every item per item with blocker.resolved, never blocker.closed.

## 13. On a board, another person's park can't end a pending spawn, so an agent binds onto the held card — fixed: bind_spawn refuses a held card and ends its request from the author's device; the board bind loop ends that session.

## 14. A wait on a gone agent beside a hold puts the held card in Needs you — fixed: `gone_waits_in` returns none for a held card, so it stays in Later and off the tray until it comes back.

## 15. Revising a parent's hold breaks same_hold, so bringing it back leaves its subtasks held on the person's word — fixed: rewording a hold rewords what its subtasks took too, and a revise of an inherited hold on the subtask is refused.

## 16. A hold accepts options through revise — fixed: revise refuses options on a hold.

## 17. Changing a parent's word hold to wait on its own subtask leaves that subtask held, so neither comes back — fixed: changing a hold closes what every subtask took before raising the new one (a67c7a55); regression test added.

## 18. Reopening a finished subtask of a held parent sends it out with no hold — fixed: reopen_locked runs inherit_park after the reopen row.

## 19. A park replace crossing a close of the same id leaves one device BLOCKED with nothing open, and ends the new hold on the other — fixed: changing a hold closes the old item per item and raises a fresh id, so no replace crosses its close.

## 20. A hold on a parent finished by rollup, or on a deleted card, never returns — fixed: work_done reads the rollup lane, and the sweep closes a wait on a deleted card, per item where its close counts (section 5 bugs 2 and 3).

## 21. A taken ask closed as a card under a held parent stays WORKING with no hold — fixed: ask_close refuses a held parent when the taker keeps the card.

## 22. Changing a hold races its close: the card sticks BLOCKED with no open item and the devices disagree — fixed: park and a hold's revise close the old item and raise a new one with a fresh id, never a replace; no sweep heal, see later-order.md section 4.

## 23. A board park by anyone but the card author's person leaves the pending spawn, and the agent binds onto the held card — fixed: a park writes a cancel only where it counts, and the author's device ends the request on bind or in its sweep.

## 24. Two devices raising on one card share a blocker id: one row is dropped, and a date close can end a word hold — fixed: hold and after items take a b<n>-<random> id no other device picks; plain questions keep the counter, see later-order.md section 4.

## 25. v0.12.3's answered_left sweep ends a hold after a person replies on its thread — fixed: a reply on a hold is a plain comment on the card, so the hold is never ANSWERED; an older window's reply is known limit 12, see later-order.md section 4.

## 26. A non-raiser's sweep writes a dead blocker.resolved every pass for plain after items — fixed: sweep_rest runs may_sweep_close (raiser's, creator's person or owner) before every item it closes, plain ones included.

## 27. A subtask made on an old device under a held parent is not held, and an old origin hands it out — fixed: a new device's sweep puts the parent's hold on a never-held subtask nobody is on, where the row counts; limit 1 corrected.

