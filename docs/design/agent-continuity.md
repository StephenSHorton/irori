# Agents that stay present: identity, rest and a context budget

*Card #1209, from the person's thread #1207 (2026-11-09). A plan only: nothing here is built. Code references are against main @ d9cd094d. Line numbers drift, so go by the names. Sibling #1208 is the small first step (a thread's live agent takes the cards made from that thread) and does not wait on this.*

## 1. What the person asked, and the short answer

The person, on #1207: "Agents disappear a little too much in irori … I see like 3 different agents cycle through a thread because they couldn't pick up any of the tasks that were part of it, or they became idle for 30 minutes and because they were only started for the thread they just go away." And: "I obviously want to save tokens by not racking up a lot of context on a single agent. So like 1 agent probably shouldn't stick around for 100 different tasks." Then: "the vision for irori [was] that agents would feel alive and fluid in this workspace. but they're actually brief and ethereal, and for that reason I think we see much less collab."

As built (spawned-agents.md §2, §4), a spawned agent is briefed for one item, does it and is removed: exit protocol, `agent.gone`, `claude stop`, `claude rm`. Its name is reseeded on the next start (`spawn_name`, `core/src/service/spawn.rs:2811`). A mention of it by name fails ("names no live agent", `tools/src/mention.rs`). A message to it lands on its card as "for <name>, who has finished" (`tools/src/notice.rs:156`). Nothing in the workspace lasts longer than one item, so there is nobody to collaborate with.

**The answer: separate an agent's identity from its process.** An agent keeps its name, the items it served and its Claude Code session after its process stops. When something is for it again (a follow-up in its thread, a card made from its thread, the person's answer to its question, a mention), Irori resumes that same session (`claude --bg --resume`, decisions 3 and 7). It retires for good only when its context passes a size budget, or when it has nothing open and nobody has wanted it for a day. A free agent that is awake can also take a new card in its own repo before Irori starts a fresh one, under the same budget.

Five parts, each a section below:
1. **Measure** each agent's context size (§4).
2. **Budget**: under it an agent stays reusable; past it, the agent finishes what it holds and retires (§5).
3. **Rest instead of end**: an idle agent's process stops, and its identity stays (§6).
4. **Wake** a resting agent for its own follow-ups and mentions (§7).
5. **Take the next item**: cards from its own thread, and same-repo cards before a fresh spawn (§8).

## 2. What does not change

- **Fresh context is still the default for unrelated work.** A card in another repo, at another weight, or on another computer gets a fresh agent, as now. Reuse is the exception, and it is guarded by the budget.
- **State lives on cards, threads and notes**, not in an agent's head. Left off, Where, closing comments and notes stay the hand-off, and every process stop still writes one (§6). Even an agent that remembers works from the card.
- **One card at a time per agent** (`board_join` refuses a second). A reused agent finishes and settles its card before it takes another.
- **Hand-out stays event-driven** (decisions 38 and 40). Reuse adds candidates to the existing offer: the free agent idle longest, handed on when it gives the card back, ends its turn without taking it, or goes away. No timer decides a hand-on. The clocks that stay (§6) decide only when a process stops, never who gets work.
- **Terminal agents stay the person's.** "Offer it cards" is unchanged (§10).
- **Shared boards and other people's agents get nothing new** (§11).

## 3. The weak spot: does "same name" mislead?

Myrtle, on #1209, asked whether "same name, new session" would feel continuous or misleading. **It would mislead, so this design never does it.** A name always means one Claude Code session, with one memory. If `@sorrel` brought back a fresh session seeded from a summary, it would answer as sorrel with none of sorrel's working memory. It would either say so, which breaks the illusion, or fill the gaps by guessing, which is worse. The person would trust it more than they should.

So:
- **Rest and wake keep the same session.** `--resume` restores the transcript and keeps the session id unless `--fork-session` is passed (checked against `claude --help` on 2026-11-09). The agent that answers really did the earlier work. It only lacks what it never read, and its wake lists what happened while it rested (§7).
- **A successor gets a new name** and an explicit line: "quill takes over from sorrel (retired at 131k tokens)". Its brief quotes sorrel's last closing comments. The card thread and the roster both show the link (§9, §12). Nobody is led to think quill remembers.

## 4. Measuring context

**The measure:** the context of the agent's most recent main-chain API call. That is `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` from the newest assistant row in the session's own transcript (`<claude>/projects/<slug>/<session>.jsonl`), with `isSidechain` false and a real model (skip `<synthetic>`). Output tokens come back as the next call's input, so they are not added.

Gotchas, all found in real transcripts:
- **Advisor rows double-count.** On a row that called the advisor, the top-level usage adds up every entry of `usage.iterations[]` (message, advisor_message, message). That total is about twice the real context. Read the **last iteration whose type is `message`**, and use the top-level fields only when there are no iterations. 88 of 300 recent sessions used the advisor.
- **Subagents don't count.** Their rows are in separate files (`<session>/subagents/**/agent-*.jsonl`), and they do not grow the main context. Read the session's own file only. `spawn_cost.rs` merges subagent files into its totals, so its figure can't be reused for this.
- **Compaction.** A `{"type":"system","subtype":"compact_boundary","compactMetadata":{"preTokens":…}}` row, then a user row with `isCompactSummary: true`. The next call reports the small size again. A compacted agent has lost detail and its size no longer shows how worn it is, so **the first compaction retires it** (§9). None of 300 recent sessions compacted (these sessions run with a large window), but an older 200k-window session did, at about 222k.
- **Cumulative totals are not context.** The hover's "4.8M tokens" (`window/src/spawn_cost.rs` `Tokens::total`) sums cache reads over every call. It stays a cost figure.
- **The window size isn't recorded**, so the budget is an absolute number of tokens, never a fraction of an assumed window.

**Where it's computed: presence.** The watcher already re-reads the last 256 KiB of each main transcript when its size or mtime changes (`presence/src/watcher.rs` `refresh_tail`, `presence/src/transcript.rs` `read_tail`, 2-second poll), and it already skips sidechain rows. `Tail` gains `ctx` (tokens), `ctx_at` (time of that call) and `compacted` (sticky once a boundary is seen). They're copied to `presence::AgentCard`. When the tail window holds no assistant row (one huge tool result), the old value is kept, the same way the watcher keeps the other fields (`watcher.rs` around `refresh_tail`). A boundary newer than the last usage means "reset, size unknown": the agent is treated as compacted.

**How other devices see it:** the three fields ride on `agent.status`. `record()` (`core/src/service.rs` around 5605) writes that row on status, name or mode changes, which in practice means each busy-to-idle edge. That is exactly when reuse is decided, so no extra rows are written. They go in `AgentStatus`, `coord::roster::StatusRow` and the `apply` arm for `AGENT_STATUS` (`coord/src/roster.rs` 416-435), and both `AgentCard` go_structs get them as omit-when-zero fields. **They must not be added** to the equality check in `record()` or to presence's `changed()`: either would write a replicated row on every token. The figure can lag one turn behind (the status can flip to idle before the last row is flushed), which is harmless for a budget.

## 5. The budget

**The rule.** An agent whose context is at or under the budget is reusable. Once it is past the budget, it finishes the item it holds and then retires: it is never handed another item, never woken for a new one, and its open thread goes to a successor (§9).

**Measured on events.** The budget is checked when an item settles, when the agent goes idle, and when a wake or offer is about to go out. It is never checked on a timer.

**Default: 120k tokens of context.** It is a per-device setting, like the idle limit since #492, in Settings › Agents › "When Irori stops and asks". The choices are **80k, 120k (default), 200k, 300k, and "Always start a fresh agent"**. The last one turns reuse off, so the setting only tunes or turns off a feature that works by itself (decision 52). It is stored as a `SpawnSetting` field (0 means the default) and checked in `check()`.

### What 120k means, from 300 real sessions

300 recent transcripts on this Mac, 2026-11-09:

| | Median | p90 |
|---|---|---|
| Context at the first call (system prompt, tool descriptions, brief) | 53.5k | 55.5k |
| Context at the end, ask and Talk agents (main checkout) | 88k | 112k |
| Context at the end, Sonnet card agents | 112k | 232k |
| Context at the end, Opus card agents | 243k | 407k |

So at 120k, **ask and thread agents nearly always stay reusable**, which is the cycling the person saw. About half of Sonnet card agents stay. Opus card agents mostly retire after their card, and that is the work most likely to have used up its context anyway. The budget sorts agents the way the person described without a rule about kinds: the small ones stay and the heavy ones go.

### What it costs

The data shows that every session writes cache with the **1-hour lifetime** (99.3% of cache writes), so a write costs 2× the input price and a cache stays warm for 60 minutes. The figures below use API prices from `spawn_cost.rs` (Opus 5.5: $4/M input, $0.20/M cache read; Sonnet 5.5: $2/M, $0.20/M). On a Claude plan this is usage, not a bill.

**A fresh start.** Its first call writes the per-session part (about 23-25k tokens) and reads the shared prefix (about 30k), which is almost always warm. That costs about **$0.21 on Opus and $0.10 on Sonnet**, or $0.44 and $0.21 when the shared prefix is cold. This is the card's "$0.10-0.44".

**A reused agent.** It carries C extra tokens of context into every call of the next item, at C × $0.20/M per call while the cache is warm. Here is the net extra over a fresh start for a median next card (77 calls on Opus, 32 on Sonnet):

| Extra context carried (C) | Total context | Opus card | Sonnet card |
|---|---|---|---|
| 30k | ~85k | +$0.26 | +$0.09 |
| 60k | ~115k | +$0.72 | +$0.29 |
| 100k | ~155k | +$1.33 | +$0.54 |

For an ask-sized item (13 calls), reuse is cheaper than a fresh start up to about C = 60k on Opus and about 30k on Sonnet.

**Honestly:** on cost alone, reuse breaks about even at **80k**, and **120k is a continuity choice**. It costs roughly +$0.30 (Sonnet) to +$0.75 (Opus) for a median card taken by an agent near the budget, and nothing extra for the small thread items the person was annoyed by. What it buys is an agent that already read the thread and the code: fewer calls rediscovering them, which the transcripts can't measure, and an agent other agents can still ask. The person picks the trade with the setting. #1209's blocker asks them which default they want.

**Cold reuse always costs more.** After 60 minutes without a call, the next call writes the whole context again: (B + C) × 2 × input. At 120k that is about +$0.75 (Opus) or +$0.38 (Sonnet) once, over a fresh start. So:
- **Automatic reuse** (same-repo cards, §8) is only offered to an agent whose last call was **under 55 minutes ago**, read from `ctx_at` when the offer is decided.
- **Waking a resting agent** for its own continuity (its thread, its question, a mention, §7) is allowed cold. The person asked for that continuity, and it costs at most one re-write.

**Not a turn or item count.** API calls per session range from 4 to 247, while context grows steadily by about 2.1k per call, so tokens are the better measure. The budget also bounds items: a small item adds at least about 5k, so 120k leaves room for roughly a dozen small items and far fewer card-sized ones, never 100.

## 6. Rest instead of end

**What the 30-minute idle limit guards today.** The process: about 0.5 GB of memory each and a slot under the device cap (spawned-agents.md §6). It ends the agent because ending was the only way to free them. Resting frees both and keeps the identity, so **the idle limit stays, as "rest after"**, and no longer removes anyone. The setting row and its range don't change. Only its label does ("Rest an idle agent after").

**When an agent rests.** It is idle and one of the following happened:
- the idle limit passed while it holds nothing that is WORKING (today's ends in `end_finished`, `core/src/service/spawn.rs:3298`: the done-check grace, a blocker for the person past the limit, parked on other work past the limit, a quiet thread past the limit);
- **a queued start needs its slot.** When the device is at its cap and a spawn is queued, the idle agent idle longest that holds nothing WORKING rests. This is an event (a queued request), not a timer.
- **Claude Code dropped it on its own.** Background sessions drop off about 60 minutes after going idle (card #498). Today that is `LEFT_GONE`, a counted crash, and two in a row ask the person "start another?" (`coord/src/board/fold.rs` around 986, `coord/src/board/spawn.rs` `CRASHED_QUESTION`). The new rule: an idle agent's drop-off while it holds nothing WORKING is recorded as a rest, never a crash. That also covers an idle limit set past 60 minutes.

An agent past the budget, or one that has compacted, retires at those same points instead (§9).

**Resting first writes the hand-off.** It is the exit protocol (`begin_end`, `HANDOFF_WAIT`) with rest wording: Where, Left off, and a closing comment in the shape of §12 on each item it holds, plus a note if it learned something about the repo. A resting agent may never wake again, and nobody can ask a stopped process, so every stop pays this one turn (about C × $0.20/M plus the output).

**How rest is recorded.** It can't use `agent.gone`, for four reasons:
- the frontend hides `status === "gone"` (`frontend/src/lib/roster-groups.ts`);
- the mesh roster deletes a gone card outright (`coord/src/roster.rs` `AGENT_GONE` arm, about 435-443);
- `return_gone` takes a gone holder off its card;
- `ended_by_irori(key)` and `spawn.removed` are once-per-key, so a woken agent that ends again would get no end record.

So, two new replicated rows:
- `agent.rest {key, life, at, reason: idle | slot | dropped, by}` and `agent.retire {key, life, at, reason: budget | compacted | quiet | crowded | ended | lifted, by}`.
- `life` counts the agent's processes. It starts at 1 at spawn and goes up by 1 on each wake. The once-per-key guards (`mark_ended`/`ended_by_irori` in `coord/src/board/mod.rs` 1779-1813, `spawn.removed`, the `NO_HANDOFF` line in `end_now`) key on `(key, life)`.
- The mesh roster keeps a resting agent's card with status `resting`. `answers()` (`coord/src/roster.rs:68-72`) treats resting as not answering now, so it gets no hand-out offers. Mentions treat it as live (§7).
- `return_gone` does not take a resting holder off its card: its item waits for it.
- When presence reports the session removed and the key has an `agent.rest` for its current life, Core writes no `agent.gone` (`record()`, `core/src/service.rs` around 5664-5676). Only `agent.retire` leads to `agent.gone` "finished", `claude stop` and `claude rm`.
- Resting agents don't count toward the device cap (`spawned_here`, `core/src/service/spawn.rs:2241`) and are not "free" in the window's counts. They still count under decision 51's "agents at once" when an agent started them, because they can come back.

**The process stop.** `claude stop` only. **No `rm`** while the agent rests: `rm` removes the job and, when nothing is unpushed, its worktree (`window/src/spawn.rs` about 154-251). The transcript survives `rm`: 213 of 264 past card agents on this Mac have no worktree left and still have their transcript. But the job and folder are what a resume needs. `rm` runs at retire.

## 7. Waking a resting agent

**What wakes one.** Each of these is an event, and each is for that agent specifically:
1. A follow-up in a thread it holds: the person writes in the thread, or a mention in it. This is the thread cycling the person saw: the same agent answers.
2. A card made from its thread, when #1208's rule picks it.
3. The person's answer on a card it holds: a blocker answer, or a "Not done" on its done check (unless that lifts the card's weight, which still means a fresh agent at the new level, decision 61).
4. A mention of it, or a `send` to it, by the person or by one of the person's own agents, on the private board or in the person's Talk.
5. The person's Wake button on the roster or the card (§12).

**Who may wake one.** A wake starts a process with shell access, so it follows spawned-agents.md §7. This **amends §7's "redirected mentions, sends and comments never start an agent"**, for resting agents only:
- the person's own words wake it;
- the person's own agents' mentions and sends wake it, and that wake counts under decision 51's limit like `agent_start` (an agent caused a start);
- another member's text on a shared board, an agent of theirs, or a redirected message from anyone else never wakes it. Their message lands on the agent's current item as a comment, as today (`record_for_finished`).

`request_revive` stays person-only for the person's own Revive of terminal agents. Waking a resting spawned agent is a new path that uses the spawn rules instead.

**How.** `claude --bg --resume <session id>`, with what the spawn passed, because resume doesn't restore those flags (`docs/research/05-raw-findings.md` 395-399):
- `--name <its word>`;
- `--permission-mode`;
- `--settings <file>` (it carries `CARGO_TARGET_DIR` and the allow lists);
- `--add-dir`.

All of these come from the spawn row kept for its `(task, generation)`, in the folder it last ran in. Irori writes `agent.wake {key, life+1, by, why, at}` before it starts the process, and binds presence's listing back to the same key the way a spawn binds. On the first turn, a wake notice says:
- why it was woken;
- when it rested;
- what changed on its items since then (new comments, the person's answers), read through the same `said_for` and Notes hooks a wake uses today (`inbox::WakeNotes`).

**The folder.**
- A thread or ask agent runs in the main checkout, so the folder is always there.
- A card agent runs in its card's tree. While any item it served is still open, treeclean keeps that tree (its items tie it, `note_tree` in `core/src/service/trees.rs`). After everything it served has settled, treeclean may remove the tree and its `target/` (gigabytes on this repo). A wake then recreates a plain worktree **at the same path**, detached at `origin/main` (`git worktree add --detach <path> origin/main`), because Claude Code finds the transcript by the folder's slug. The agent then switches to the branch its new item needs (§8).

**Names.** A resting agent keeps its word. The namer holds the word for the agent's whole life, not for `NAMES_BUSY` (2 h), so nobody else is given it and `settle()` won't rename it (`core/src/namer.rs`). On retire, today's protection applies: 7 days before reuse, 30 days in the lists.

**If resume doesn't hold up.** The first doing card is a spike that tests this on a scratch build. If the transcript can't be found after the folder is recreated, if the session id changes, or if `--name` or `--settings` don't stick, the design falls back:
- **Rest becomes retire**: the hand-off is written at the same points, and the name goes.
- **Whatever would have woken the agent starts a successor** (§9), seeded with the hand-off and the thread, and named as the successor.

Everything else (the budget, same-repo reuse while awake, the roster's free group) works the same without resume. In that case continuity lasts only while the process is up (the idle limit), which still fixes the cycling for follow-ups inside the limit.

## 8. Taking the next item

An agent that holds nothing WORKING, is under budget, and has no done check pending inside its grace can take one more item. "No done check pending" matters because a "Not done" can still come back; the agent rests through it instead (§6). There are two ways the next item reaches it.

**A. The cards from its own thread (#1208).** #1208 assigns a card made from a thread to that thread's live agent. This design adds three things:
- the budget (§5) in place of #1208's "one card at a time" stand-in;
- the same run level (below);
- for a resting thread agent, a wake (§7, trigger 2) instead of a fresh spawn.

The predicate is shared, so #1208 and this design call the same function (below).

**B. Same-repo cards before a fresh spawn.** A card that needs an agent on a spawning board is first offered to a reusable agent, through the existing offer path (decision 40), and only spawned for if nobody takes it.

- **Candidates (`reusable_for(t)`)**, all checked when the offer is decided:
  - spawned by Irori, **awake and idle** (resting agents are not offered unrelated cards: a wake just to be asked costs a turn and maybe a cold write);
  - **on the device that hands the card out** (`hands_out`, `offers.rs`); v1 is same-device only (§11);
  - **the same repo**, with both keys set and equal by `same_repo_key`. `free_for` lets an agent with an empty key through, which reuse must not.
  - **the same run level** as the card's `Task::run_level` (`types.rs` 207). The agent's model and effort were fixed at its spawn (`pick_for`), so it can take a card of its own level only. It never takes a lower level (the card would cost more for nothing), and never a higher one (decision 61's lift exists to get a stronger model).
  - context at or under the budget, not compacted, last call under 55 minutes ago (§5);
  - holds no WORKING card, has no open step, has no done check pending, is not `handed_elsewhere`, and is not in the card's skip or heard list (`free_for`'s existing checks);
  - not a helper on a led card, and not in a tree another live agent works in.
- **Order:** the one idle longest, as decision 40 says. Ties break by `machine_rank` and then key, as `free_for` does.
- **The offer:** `task.offered {to}`, handed on by `turn_state`/`pass_offer` exactly as for terminal agents. When nobody is left, the card is spawned for as today.
- **No double start.** `spawn_candidates` (`coord/src/board/spawn.rs:1686`) must skip a card whose live offer names someone. `needs_agent` never reads `t.offer`, so without this clause #453's "two agents on one card" comes back.
- **The pinned test changes.** `a_spawning_board_never_hands_a_new_card_to_an_idle_agent` (`coord/tests/it/offer_opt_in_test.rs:92`) becomes: such a board never hands a card to an idle **terminal** agent without "Offer it cards", and does offer it to a reusable spawned agent first.

**The shared predicate.** Coord can't read transcripts under its lock, so Core measures off-lock and the fold reads a live map. This is the same pattern as the "Offer it cards" switch (`SpawnState.offers`, read through the `offers` closure on `coord::Options`, `core/src/service.rs` around 1414-1437, `boards.rs` around 1514-1562).

```text
reusable(key) -> Reuse { ok: bool, why: &'static str, ctx: u64, warm: bool }
    // Core fills it from presence's AgentCard (§4) and the device's budget (§5)
may_hand_to / spawned_may_take (coord/src/board/spawn.rs 256-270):
    own item | own item's subtask | own created merge card        (today)
  | made_from_agents_thread(s, t) && reusable(key).ok             (#1208 + budget)
  | offered to it by reusable_for(t)                              (B)
check_free (offers.rs 207-226): the same arms, so board_join accepts the offer
```

**Delivering the item.**
- **The record.** When the agent takes the item (`board_join` on the offer, or #1208's assign), Irori writes `agent.took {key, taskId, life, at, by}` (multi-item mark, below).
- **The brief.** Irori writes a brief file for the item (`write_brief`, `core/src/service/spawn.rs:3097`) and sends a `spawn.next` notice, the way `begin_end` sends `spawn.end`: "You now have #M. Read its brief at <path>." Inline text is capped (`MAX_WAKE_BYTES` 16 KiB), so the brief stays a file. A next-item brief leaves out the fixed parts the agent already read (`BUILD_OUTPUT`, `BASE_CHECK`, `DEVICE_FREE`, `TALK_NOTE`, `SPLIT_WORK`) and keeps the card, the person's words nearby (`said_near`), Left off and the exit rules.
- **The worktree.** A running session's folder and environment can't change, so:
  - **A card agent** keeps its tree. Its brief says to push the last card's branch (Where must name a pushed commit), then `git switch -c card-<M>-<g> origin/main` in the same tree. The new card's Where records that branch. `note_tree` ties the tree to the new card as well, so treeclean keeps it until both settle. The shared `target/` stays warm, which is a small win.
  - **A thread agent** (main checkout, no tree) taking a code card: Irori makes the card's tree with `spawn_tree`, and the brief tells the agent to switch into it with Claude Code's `EnterWorktree <path>`. `CARGO_TARGET_DIR` isn't set for a thread agent, so cargo builds into that tree's own `target/`. This also fixes #1208's gap (told to its agent on #1208).
  - **The reopen guard.** `card_folder` and `spawn_tree` (`core/src/service/spawn.rs` 2855, 745) reuse any existing tree for a card. If card N reopens while a reused agent works card M in N's old tree, N's new agent would land on M's branch. When a live or resting agent sits in the tree on another item, a reopened card gets a fresh tree on its own branch.
- **Brief wording that changes.** These say "take only this" or "Irori ends you", which reuse contradicts:
  - `core/src/service/spawn.rs` 1206, 1211, 1330, 1443 (`weight_words`), 1489 and 1494 (thread and ask briefs, including "that card gets its own agent, so don't work it yourself"), 1523, and `begin_end`'s texts at 3502-3519;
  - `BUILD_OUTPUT` (883: "it goes when the card settles");
  - `coord/src/board/offers.rs:219` (`check_free`'s refusal);
  - `tools/src/board.rs` 2394 and `specs.json` 1588 (`board_move`).

  The new wording keeps "work only #N now", and says that when #N settles Irori may give the agent its next item, let it rest, or retire it.

**The multi-item mark.** `task.spawned` makes one mark per key, and `fold_spawned` refuses a later mark for another card (`coord/src/board/spawn.rs` 403-450). That refusal is also what stops another member's Irori from re-pointing the person's agent on a shared board, so **it stays**.
- The mark becomes the agent's **origin**. A new row, `agent.took {key, taskId, life, at, by}`, appends to `Spawned.items`. Its writer rule is `spawn_writer_ok` plus the board's `key_person` check, the same log only (no private ↔ shared crossing).
- `Spawned::current()` is the newest item that hasn't settled, else the newest.

What each place that reads the single mark becomes (find them with `grep -rn 'spawn_card_of\|spawned_for(\|\.spawned(&\|coord\.spawned(\|spawned_card(\|is_spawned(' crates --include='*.rs'`; 71 hits in 23 files on d9cd094d):

| Where | Today | Becomes |
|---|---|---|
| `coord/src/board/spawn.rs` `fold_spawned`, `Spawned` | one task | origin unchanged; `items` from `agent.took`; `current()` |
| `spawn.rs` `takes_handout`, `may_hand_to`, `spawned_may_take` | own card, its subtasks, own merge card | plus the two reuse arms above, for any of its items |
| `offers.rs` `check_free` (207-226) | refuses any card but its own | the same arms |
| `mod.rs` `spawned_card` (1760) | the mark's card | `current()`'s card |
| `mod.rs` `spawned` (1913), `is_spawned` (1771) | mark | unchanged (returns the items too) |
| `spawn.rs` `take_up_started` (766), `mod.rs` 1969 | claims the mark's card | claims `current()` |
| `spawn.rs` `in_chain` (838), `counted_live` (857) | by mark | unchanged: counting belongs to the agent, from its origin |
| `fold.rs` crash count (~986) | `m.task_id == t.id` | count on the item the agent held when it went |
| `runs.rs` (~51) run tally | by mark's run | unchanged (the model is the origin's); items are listed for display |
| `side_gate.rs` (~122), `asks.rs` (~1538), `comments.rs` (~1944 finished flag) | the mark's item | the item in question, by `items` |
| `core/src/service/spawn_boards.rs` `spawn_card_of` (607) and its callers in `spawn.rs`, `spawn_limits.rs`, `spawn_waiting.rs`, `spawn_remote.rs`, `spawn_agents.rs`, `trees.rs`, `boards.rs`, `service.rs` | the mark's card | `current()`'s card |
| `core/src/service/spawn.rs` `end_finished` (3298) | ends when the mark's card is terminal or not held | decides per `current()`: take the next item, rest or retire (§6, §9) |
| `spawn.rs` `handed_off` (3540) | any comment since `m.at` | since the current rest or end began, on the items held |
| `tools/src/started.rs`, `notice.rs`, `messages.rs`, `talk.rs`, `explore.rs` | the mark's card | `current()`; `agents_started` lists every item |
| `window/src/agents.rs` `agents_running`, `agent_usage`/`spawned_card` (506), `spawned_here` (426) | one card | `current()` plus `served` |
| `window/src/cards.rs` (~1310) `spawned` flag | copied onto every working row of the key | only on rows of its items |

## 9. Retiring

**When an agent retires** (an event each time, except the quiet day):
- **budget**: past the budget when its item settles, or when it would rest;
- **compacted**: at its next item boundary or rest;
- **quiet**: resting, every item it served settled, and nothing for it (an item, a mention, a follow-up) for 24 hours. This is a clock on ending only, like the idle limit. It decides no hand-on.
- **crowded**: more than 20 agents resting on a device. When one more rests, the one quiet longest that holds nothing open retires. 20 is a fixed constant, not a setting.
- **ended**: the person's End, or an agent's `agent_end`;
- **lifted**: a "Not done" that raises the card's level (decision 61).

**What retiring does.**
1. If the agent is awake, it runs the exit protocol. A resting agent already wrote its hand-off when it rested.
2. Irori writes `agent.retire`, then `agent.gone` "finished", `claude stop` (if awake) and `claude rm`.
3. The name is released to the namer's normal protection.
4. Any open thread or item it held goes back to hand-out. Its next agent is a **successor**: the brief opens with "You take over from sorrel (retired: context past 120k). sorrel's closing notes:", then the quoted closing comments (§12). The thread and card get one line: "sorrel retired (131k); quill takes over."

**Mentions and messages to a retired agent** land on its current item, as today ("for sorrel, who has finished"), plus "quill took over" when there is a successor, and they wake that successor if it holds the item.

## 10. Terminal agents and "Offer it cards"

- **Unchanged for terminal agents.** "Offer it cards" still decides whether an idle terminal agent is handed unheld cards. `set_agent_offer` still refuses spawned agents (`core/src/service/spawn.rs` around 1991-2027): their reuse is decided by the budget, not by a switch.
- **On spawning boards, the order for an unheld card** is: an idle terminal agent with "Offer it cards" on (spawned-agents.md §6.3; today the code skips this on spawning boards), then a reusable spawned agent, then a fresh spawn. The doing card for §8 B restores the first step in the same change, so that both offer arms go through one `free_for`.
- **Wording.** Copy that says "an idle agent with Offer it cards on" now also means reusable spawned agents. It becomes "an idle agent that takes cards":
  - `frontend/src/components/new-card-dialog.tsx`, `ask-box.tsx`, `agents-now-toggle.tsx`, `talk/threads.tsx`;
  - `crates/notify/src/watch.rs:951`.

  `takesOffers` in `frontend/src/lib/agent-counts.ts` adds reusable spawned agents.

## 11. Shared boards and other devices

- **v1 is the private board and the person's Talk.** A spawned agent on a shared board keeps today's one-item life. A shared board's settings, bypass rules and `others` taint are fixed at spawn (card #1075), and a second item from another member would carry their text into a session started under different rules. Reusing an agent across boards would cross logs.
- **v1 is same-device.** Hand-out runs on the poster's device (`hands_out`), and an agent's context is only measured where its transcript is. Other devices see the gauge (§4) but don't offer cards to another device's agents. A later card can lift this once the figure has proven reliable on `agent.status`.

## 12. What the person sees

**Hand-off format.** Each rest and each retire ends with one closing comment per item held, in a fixed shape. No new row kind is needed: `handed_off` sees the comment.

```text
Done: <what is finished, with the commit or PR>
Learned: <what the next agent needs to know, or "nothing new">
Open: <what is left, or "nothing">
```

Left off (`board_step left_off`) and `notes_propose` stay as they are. "Learned" is the line that becomes a note when it is about the repo.

**The roster.** Agents are grouped by the roster's own flag (`RosterAgent.spawned` and the new status), not by card working rows, which miss thread agents and agents between items (`roster-groups.ts:57-101`):

```text
AGENTS                          2 on cards · 2 free · 5 resting
Yours
  ● pebble      irori · working
On cards
  ● sorrel      #1212 · working · 64k
  ● quill       #1215 · blocked on you · 101k
Free
  ○ tansy       irori · idle 12m · 41k of 120k · served #1207, #1208
  ○ marram      ww-multiplayer · idle 3m · 88k of 120k · served #1190
Resting (5)                                                   ▸
```

Opened, Resting shows each agent with its repo, its context, what it holds, and when it rested: "yarrow · irori · 52k · thread #1201 open · rested 2h ago".

The hover (`agent-hover.tsx` `AgentSummary`) replaces "Irori started it for #N" with:
- "Served #1207 (thread), #1208";
- a context line, "41k of 120k · retires past 120k";
- the state: "awake, idle 12 min" or "resting since 14:02".

Its actions are Attach and Logs (awake), Wake (resting), Rest now (awake), and Retire. It still shows the model and effort, plus a note when they differ from the board's current pick.

Notes on the roster:
- Badges count "free" as idle terminal agents with "Offer it cards" on plus reusable spawned agents.
- The sidebar tooltip "each ends when its card is Done" (`app-sidebar.tsx` ~485) changes.
- Settings › Running lists resting agents too.
- The mention picker lists resting agents, marked resting.

**The noisy-roster risk.** Today every spawned agent leaves, so the roster is short. With rest it could fill up with agents from last week. Four things keep it readable:
- Resting is folded into one count by default.
- An agent retires after a quiet day once its items have settled.
- No more than 20 rest per device.
- A resting agent shows only what it still holds.

If the person still finds it noisy, the next step is to shorten the quiet day, not to remove agents sooner.

## 13. Risks

- **Cost creep.** Each reuse carries context. Bounded by the budget, the 55-minute warm rule for automatic reuse, and the setting (§5). Settings › Agents' cost line already shows the day's total.
- **Resume isn't what it seems.** The spike comes first, and §7 names the fallback.
- **The wrong agent for a card.** Same repo is not same subject. Mitigated by the same run level, the budget, an offer the agent can decline (`board_leave`), and the brief's "work only #N".
- **Two agents in one tree.** Mitigated by the reopen guard (§8).
- **A resting holder blocks a card.** Mitigated because its item wakes it, the quiet day retires it, and End is one click.
- **Cross-person wakes.** Prevented by §7's rule, with a test for each kind of text that must not wake an agent.
- **Older devices.** An older Irori ignores `agent.rest`, `agent.wake`, `agent.took` and `agent.retire`. On its roster a resting agent drops as gone once the session drops. It doesn't offer cards to spawned agents at all, so nothing is handed out wrongly.

## 14. Build order (the doing cards)

Each is its own card on the Board, after #1209, with steps, files and done-when. The spike gates the wake card only.

1. **Spike: resume a stopped spawned agent.** Live, on a scratch build. Gates 6.
2. **Context measure** in presence, carried on `agent.status`.
3. **Budget setting and the `reusable` predicate.** After 2.
4. **The multi-item mark** (`agent.took`, `Spawned.items`, the table in §8).
5. **Rest instead of end** (`agent.rest`/`agent.retire`, `life`, drop-off as rest, cap, name hold, hand-off at rest, retire rules). After 3 and 4.
6. **Wake a resting agent** (resume with the spawn's flags, the folder, triggers, who may, decision-51 counting). After 1 and 5.
7. **Next item: own-thread cards and same-repo reuse before spawning** (offer path, double-start clause, next-item brief, worktree switch, reopen guard, brief wording, pinned test). After 3 and 4.
8. **Roster, hover and Settings** (groups, gauge, served list, resting, wording, preview samples and shots). After 2, 3 and 5.
