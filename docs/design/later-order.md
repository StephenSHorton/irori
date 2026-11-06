# Later and order (card #570)

Some cards aren't for now, and cards have an order to be worked in. Agents
arrange both, and the person sees them on the Board and the phone.

**No new row kind.** Later rides on the blocker rows every device reads, with
one optional string field. Order is ordinary task waits. Per the no-server
decision (`no-server.md`), an older device never stalls on either.

Old-build behaviour below comes from reading `git show <tag>:…` at v0.10.2,
v0.11.0 and v0.12.3. It has not been run against those builds yet.

## 1. Model

A parked card is `BLOCKED` with one open blocker carrying `later`, a new
optional string on `BlockerRow`.

| It comes back when | `on` | `later` |
|---|---|---|
| A card finishes | `{kind:"task", ref:<task id>}` | `"card"` |
| A date comes | `{kind:"agent", ref:"irori"}` | `"date 2026-12-01T05:00:00Z"` |
| An app version runs | `{kind:"agent", ref:"irori"}` | `"version 0.13.0 <dev> [<dev>…]"` |
| The person says so | `{kind:"agent", ref:"irori"}` | `"word"` |

- **Order** is plain task blockers: "after #N" is a wait on #N, with no marker.
  A card's place in its chain and what's next are worked out at read time.
- **The Later section** holds only cards with an open `later` item. A card that
  is only "after #N" stays where it is and shows its chain.
- An unknown `later` value reads as `word`: the card stays parked and never
  closes by itself. `later` stays a JSON string forever, because a type
  mismatch fails the whole decode.

### Why `agent irori`, and not a new `on.kind`

A new kind is safe in the fold, since the stall rule is about `ev.kind`. But
every older window treats a kind outside `waits_by_itself` as a question: it
shows in Needs you with a reply box, and from v0.11.0 a reply runs
`back_on_board` and the card goes out. A condition packed in `ref` would also
hide "parked until #N" from `task_waits`, so circles and chains would miss it.

`agent irori` is calm on every tag:

- `agent` is in `waits_by_itself` at v0.10.2, v0.11.0 and v0.12.3.
- `store/src/board.rs` `key()` accepts `irori` at those tags.
- `key_gone_in` is false for it (no `/`, and it is `SWEEPER`), so it never
  strands. `no_lead` applies only to a part's question.
- The sweep has no `agent` arm, so nothing old closes it.

`external` was rejected for three traps: the lease rewrite, `merge_wait` on any
PR in its text, and the gone wait when its raiser ends. `on: version` was
rejected for version parks because v0.11.x windows don't know it, and a version
park is raised exactly when a device lags.

Fields on `task.edited` were rejected because the card would stay `SUBMITTED`,
and an old origin would hand it out.

## 2. Rows

Park on a date (version and word have the same shape):

```json
{"kind":"task.status","payload":{"taskId":"01M…","state":"blocked",
 "reason":"Later: voice calls, not before December",
 "blockedBy":{"kind":"agent","ref":"irori"},
 "blocker":{"id":"b2-k7q2xm","question":"Later: voice calls, not before December",
  "context":"Comes back on 1 Dec","on":{"kind":"agent","ref":"irori"},
  "later":"date 2026-12-01T05:00:00Z"},
 "by":"dev/heron","at":"…"}}
```

Then, if someone held it, `task.left`. The blocker goes first, because `leave`
only writes `SUBMITTED` from `WORKING`.

Return is always a per-item `blocker.resolved`, never `blocker.closed`, which
isn't tied to an item and could close a newer park when it arrives late:

```json
{"kind":"blocker.resolved","payload":{"taskId":"01M…","blockerId":"b2-k7q2xm",
 "answer":"1 Dec came","option":-1,"closed":true,"restored":"submitted",
 "by":"irori","at":"…"}}
```

`after: ["#12","#13"]` writes one blocker row per card:

```json
{"state":"blocked","reason":"After #12: Spaces",
 "blockedBy":{"kind":"task","ref":"01M…12"},
 "blocker":{"id":"b1-3fzq0d","question":"After #12: Spaces",
  "on":{"kind":"task","ref":"01M…12"}}}
```

Field rules:

- Hold and after items take an id no other device picks: the card's next
  `b<n>` plus six random characters. Every tag accepts any id string (the
  store keys `taskId/blockerId`, the window `blocker:<card>:<id>`, so no `/`
  or `:`), and `seq_of` reads it as 0 there and here, so the counter that
  names reason-only items stays the same on every device. Plain questions
  keep the bare counter.
- The replace fold sets `later` only when the row carries one: an old device's
  revise rewrites the row without it, and that must not clear it.
- After items are not `reasonOnly`. Today's `waits_on` is, and a later
  reason-only block replaces such an item.
- Each after item needs its own question, because the fold dedupes on question
  text and would overwrite `on`. Use the short ref plus title, and refuse a
  clash rather than overwrite. A new card's after items, its own reason and
  a hold it takes from its parent are all compared.
- Check the cap (8 open items), the refs and circles for the whole list before
  the first write.
- Refs take `tag#N`: with #558's per-board numbers a bare `#N` can be ambiguous.

## 3. Behaviour

### Parking

- Refused while a person's question, done check, transfer, merge wait or
  `INPUT_REQUIRED` is open: those stay Needs you, and raising would swallow
  them.
- One park per card. Changing it closes the old item with a per-item
  `blocker.resolved` and raises a new item with a fresh id, on the card and
  on the subtasks that took it. A revise of a hold's words does the same on
  the card. Close first: the fold merges a raise into an open item with the
  same question. Never a `replace`: one crossing a close of the same id
  leaves the card `BLOCKED` with nothing open, on every tag.
- Two devices can each park a card apart. It then has two holds and stays
  held until the last one closes.
- A block or question with the hold's own words is refused: the fold's
  same-question merge would rewrite the hold's `on`. A hold takes no options
  through revise either.
- A pending spawn is ended on park, where the cancel counts. Binding an agent
  to a held card is refused, and ends its request; the session started for it
  is ended. "Start an agent" on a held card is the person's word: it brings
  the card back first.
- A held card: parking drops its holders, because a kept holder would come back
  silently as `WORKING`. Only the holder or a person may park a held card.
- Order keeps the holder, as `board_block` does.

### What ends a park

All refusals are at write time. A fold-level refusal would diverge from old
devices.

| Path | Rule |
|---|---|
| `board_status` | Agents refused on a parked card, except canceled and failed |
| `board_unblock`, no id | Agents refused on a parked card |
| `board_unblock blocker_id` | Allowed, except a `word` item: person only |
| `claim_task`, `assign` | Agents refused; a person's assign is their word |
| `back_on_board` | Another item's answer can't unpark the card |
| `answered_left` | Skips `later` items |
| `hand_on` | Refused on a parked card |

### Coming back

- `gone_away` gets one arm for `date` and `version`. `card` needs nothing new,
  and `word` has no arm.
- `sweep_rest` closes every item per item with `blocker.resolved`, never
  `blocker.closed`, which names no item and would end a hold the sweeping
  device hasn't seen. The last close sets `restored`.
- A returned card is `SUBMITTED` and unheld, and goes out as if just posted.
  `agent` and `task` are in `on_work`, so offer freshness already works.
- A card parked on an old origin still returns: a new device writes the close,
  and the old origin folds it and hands the card out. With only old devices on,
  date, version and word stay parked, which is the safe direction.

### Circles

- `order_reaches(from, to)` runs in the task arm of `check_blocker`, which every
  wait passes through, `main-red` included. `wait_chain` isn't reused: its
  64-card cap would silently pass a long chain.
- Edges are `task_waits` plus parent to open subtasks, because a parent can't
  finish first. `split` adopting a card makes the same check.
- A `later:"card"` item is an edge for free.
- A wait with options is no edge, so it is checked when it becomes one:
  revise dropping its options, or a raise merging into an optionless item
  with the same question.
- Two devices can each make half a circle offline. Neither can refuse that, and
  the fold must accept both rows, so every device converges on the same loop.
  #481's read-time loop item still shows it, which is the one exception to
  "never Needs you": a Later section must not hide a loop.

### Subtasks and parents

- A parked parent doesn't hold its children, so parking a parent writes the same
  item on each open, unheld subtask, and new subtasks inherit it. Real rows mean
  old origins honour it.
- Parking a parent with a held subtask is refused.
- Bringing back a `word` parent closes the items on its subtasks too.
- A parked child doesn't set the parent's rollup to `BLOCKED`. Old windows still
  will, with "A subtask needs a call".

### Shared boards

- A sweeper close counts only from the raiser's person's device, or the
  creator's person's or a board owner's. Widening that would diverge from old
  folds. The sweep checks this before every item it closes, so no other
  device writes a dead row each pass.
- A task wait must name a card on the same log, which the store enforces. Order
  works across a space's boards, not across spaces or Personal. Order per board
  is grouping by `spaceBoard` at read time, with nothing stored per board.
- A version park keeps `check_version_wait`'s board refusal.
- `word` on a board means any member person.
- Only the card author's device may end its spawn request. Another member's
  park writes no cancel there; the author's device ends it, on bind or in its
  sweep.
- Sharing a parked card to a board is refused in the preview: blockers stay
  private, so it would go out at once.

## 4. Known limits, ranked

1. **Older agents can bypass a park.** Refusals are at write time, so an agent
   on an old core can still `board_unblock`, `board_status working` or
   `board_join`. Hand-out isn't affected. Old windows' Stop waiting also closes
   a park, which is a confirmed act by the person.
2. **Board returns need a device up.** With the raiser's, creator's and owner's
   devices off or old, a date or word park waits.
3. **Phones never sweep.** With only a phone on, nothing returns until a desktop
   runs.
4. **A park racing a status row.** `fold_status` has no RowPos guard, so a park
   and a concurrent `working` can differ between devices until restart.
5. **Old windows and rollup.** A parked child shows "A subtask needs a call" on
   the parent there, though it doesn't count as a decision.
6. **Wall clock.** The first device past the date closes it, so one wrong clock
   returns a card early.
7. **Old windows have no Later section.** They show "Waiting on Irori" with the
   reason, or "Waiting on #N".
8. **v0.10.2 keeps `heard`** on return, so an agent that already passed may not
   hear the card again.
9. **A replace crossing a close still sticks.** An older build's revise of a
   hold, or any build's revise of a plain question or reason-only block, can
   cross that item's close and leave the card `BLOCKED` with nothing open.
   No sweep heals it: the heal would be a status row, which closes every
   open item on a device whose fold disagrees, and would send out a card
   whose hold was lost.
10. **Plain question ids still collide.** Two devices raising questions on
   one card apart both pick `b<n>`, and one is dropped. Pre-existing; holds
   and after items no longer do this.
11. **A bind racing a park.** If the author's device binds the agent before
   another member's park reaches it, the park's device can't drop a holder it
   hasn't seen, so the agent stays on the held card. Old author devices bind
   with no check at all.

## 5. Bugs found on the way

From reading only, none run. Not part of #570:

- Fixed in #570: `sweep_rest` had no writer check before `close_blocker`, so
  a non-raiser's device on a board wrote a dead `blocker.closed` every 5s.
- `work_done` reads raw state, so a wait on a parent whose finish lives only in
  its rollup may never close.
- A wait on a deleted card never closes: `gone_away` acts only on `Some(o)`.

## 6. Moving today's fake holds

Close then raise, because revise refuses `on`. Re-check each card first. The
list is #419, #420, #421 and #571 (voice calls, from #569). #563 is no longer a
hold.
