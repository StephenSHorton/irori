# Agents that stay present: identity, rest and a context budget

*Card #1209, from the person's thread #1207, 2026-11-09. This is a plan only; nothing here is built. Code references are against main @ d9cd094d. Line numbers drift, so go by the names. Four reviewers checked the plan against the code from different angles, and their fixes are in it. Sibling card #1208 was the small first step: a thread's live agent takes the cards made from that thread. It is on main (decision 68), and this plan builds on it. The details a builder needs are in Appendix A.*

## 1. What the person asked, and the short answer

The person, on #1207, said three things:

- "Agents disappear a little too much in irori … I see like 3 different agents cycle through a thread because they couldn't pick up any of the tasks that were part of it, or they became idle for 30 minutes and because they were only started for the thread they just go away."
- "I obviously want to save tokens by not racking up a lot of context on a single agent. So like 1 agent probably shouldn't stick around for 100 different tasks."
- "the vision for irori [was] that agents would feel alive and fluid in this workspace. but they're actually brief and ethereal, and for that reason I think we see much less collab."

**How it works now** (spawned-agents.md §2, §4). A spawned agent is briefed for one item. It does that item and is removed: exit protocol, `agent.gone`, `claude stop`, `claude rm`.
- Its name is seeded again on the next start (`spawn_name`).
- A mention of it by name fails ("names no live agent").
- A message to it lands on its card as "for <name>, who has finished".

Nothing in the workspace outlives one item, so there is nobody to collaborate with.

**The answer: separate an agent's identity from its process.** An agent keeps three things after its process stops: its name, the items it served, and its Claude Code session. Irori resumes that same session (`claude --bg --resume`, decisions 3 and 7) when something is for it again:
- a follow-up in its thread;
- the person's answer on a card it worked;
- a card made from its thread;
- a mention.

A free agent that is awake can also take a new card in its own repo, before Irori starts a fresh agent.

**When an agent retires for good:**
- when its context passes a size budget (200k tokens by default, the person's pick), or it compacts;
- when the person or an agent ends it;
- after a quiet day with nothing open. This is the one clock, and it only ends agents nobody is waiting on.

The parts, each with its own section:
1. **Measure** each agent's context size (§4).
2. **The budget.** Under it, an agent stays reusable. Past it, the agent finishes what it holds and retires. The budget never interrupts work (§5).
3. **Rest instead of end.** An idle agent's process stops; its identity stays (§6).
4. **Wake** a resting agent for what is its own (§7).
5. **The next item.** Its own items first, then cards from its thread, then same-repo cards ahead of a fresh spawn (§8).

## 2. What does not change

- **Fresh context is still the default for unrelated work.** A card in another repo, at another weight, or on another computer gets a fresh agent, as now.
- **State lives on cards, threads and notes**, not in an agent's head. Left off, Where, closing comments and notes stay the hand-off. Every stop Irori makes writes one (§6).
- **One card at a time per agent.** `board_join` still refuses a second card. **A resting agent holds no card**: it leaves its card at the same points where today's code ends it, so a card is never stuck behind an agent that is asleep (§6).
- **Hand-out stays event-driven** (decisions 38 and 40). Reuse adds candidates to the existing offer: the free agent idle longest gets it, and the card is handed on when that agent gives it back, ends its turn without taking it, or goes away. No timer decides a hand-on. The 55-minute "still warm" check (§5) is read at the moment an offer is decided, the way decision 42's 5 minutes are.
- **Terminal agents stay the person's.** "Offer it cards" is unchanged (§10).
- **Shared boards and other people's agents get nothing new** (§11).

## 3. The weak spot: does "same name" mislead?

Myrtle asked on #1209 whether "same name, new session" would feel continuous or misleading. **It would mislead, so this design never does it.**

A name always means one Claude Code session, with one memory. Suppose `@sorrel` brought back a fresh session seeded from a summary. It would answer as sorrel with none of sorrel's working memory. Then it would either say so, which breaks the illusion, or fill the gaps by guessing, which is worse. The person would trust it more than they should.

So:
- **Rest and wake keep the same session.** `--resume` restores the transcript and keeps the session id unless `--fork-session` is passed (`claude --help`, checked 2026-11-09). For a `--bg` job, the spike (§7) found two more ways to get a copy under a new id: passing flags while the job record exists, and resuming before the old process has exited. The wake avoids both. The agent that answers really did the earlier work. Its wake lists what happened while it rested (§7).
- **A successor gets a new name** and an explicit line: "quill takes over from sorrel (retired: context past 200k)". The successor's brief quotes sorrel's closing comments. The thread, the card and the roster all show the link (§9, §12). Nobody is led to think quill remembers.

## 4. Measuring context

**The measure** is the context of the agent's most recent main-chain API call. That is `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` from the newest assistant row in the session's own transcript.

Four rules keep it right:
- read only the last iteration of type `message` (on a row that called the advisor, the top-level total counts about double);
- ignore subagent files;
- skip `<synthetic>` rows;
- treat a compaction as "worn out", not "small again".

Appendix A.1 has the details found in real transcripts.

**Where it is computed: presence.** Presence already re-reads the end of each live transcript when the file changes, and it already skips subagent rows. It adds three fields to the agent's presence card:
- `ctx`: the tokens;
- `ctx_at`: when that call was made;
- `compacted`: set once a compaction is seen, and it stays set.

These ride on `agent.status`, which is written at each busy-to-idle edge, so other devices can show the figure without any extra rows. **A resting agent has no presence card**, so `agent.rest` carries its last `ctx`, `ctx_at` and `compacted`. The roster and the budget read those while it rests.

## 5. The budget

**The rule.** An agent at or under the budget is reusable. One past it finishes the item it holds and then retires: it is never handed another item and never woken for a new one. If it has an open thread, a successor takes the thread over (§9).

**It never interrupts work.** The budget is checked only between items: when an item settles, when the agent goes idle, and when an offer or wake is about to go out. An agent that passes 200k halfway through a card keeps going and finishes that card as it would today. The only hard limit mid-work is the model's own window, which Claude Code handles by compacting; a compacted agent retires at its next item boundary.

**Default: 200k tokens of context.** The person picked this on #1209 b1 ("200k: maximum continuity").
- It is set in Settings › Agents › "When Irori stops and asks". The choices are **80k, 120k, 200k (default), 300k, and "Always start a fresh agent"**. The last one turns reuse off, so the setting only tunes or turns off a feature that works by itself (decision 52).
- It is stored on the private board's `spawn.set` row for each device, the way the idle limit is: 0 means the default, -1 means always start fresh, and `check()` accepts only the listed values.

### What the budget means, from 300 real sessions

These figures come from 300 recent transcripts on this Mac, read on 2026-11-09:

| Context | Median | p90 |
|---|---|---|
| At the first call (system prompt, tool descriptions, brief) | 53.5k | 55.5k |
| At the end, ask and Talk agents | 88k | 112k |
| At the end, Sonnet card agents | 112k | 232k |
| At the end, Opus card agents | 243k | 407k |

At 200k:
- **Ask and thread agents always stay reusable.** They are the cycling the person saw.
- **Most Sonnet card agents stay.**
- **About one Opus card agent in three stays.** The rest retire after their card, which is the work most likely to have used up its context anyway.

So the budget sorts agents the way the person described, without a rule about kinds of work: the light ones stay and the heavy ones go.

### What it costs

Every one of these sessions writes its cache with the **1-hour lifetime** (99.3% of cache writes). So a cache write costs twice the input price, and a cache stays warm for 60 minutes.

Prices are API prices from `spawn_cost.rs`: Opus 5.5 is $4/M input and $0.20/M cache read; Sonnet 5.5 is $2/M and $0.20/M. On a Claude plan this is usage, not a bill.

- **A fresh start.** Its first call writes the per-session part (23-25k tokens) and reads a shared prefix of about 30k, which is almost always warm. That costs about **$0.21 on Opus and $0.10 on Sonnet**, or $0.44 and $0.21 when the shared prefix is cold. This is the card's "$0.10-0.44".
- **A reused agent.** It carries C extra tokens into every call of its next item. While the cache is warm, each call costs C × $0.20/M more.

Net extra cost of reuse over a fresh start, for a median next card (77 calls on Opus, 32 on Sonnet):

| Extra context carried (C) | Total context | Opus card | Sonnet card |
|---|---|---|---|
| 30k | ~85k | +$0.26 | +$0.09 |
| 60k | ~115k | +$0.72 | +$0.29 |
| 100k | ~155k | +$1.33 | +$0.54 |
| 145k | ~200k | +$2.03 | +$0.83 |

**Where reuse breaks even:**
- For an ask-sized item (13 calls), reuse is cheaper up to about C = 80k on Opus and 40k on Sonnet.
- For a median card, reuse costs more once it carries about 15k extra, which is about 70k in total.

**On cost alone**, then, the budget would sit near 70-80k. **200k is a continuity choice**, and the person made it knowing the trade:
- An agent near the budget costs about +$0.83 (Sonnet) to +$2.03 (Opus) more on a median card.
- A small thread item costs about nothing extra.
- In return, the agent has already read the thread and the code, so it spends fewer calls rediscovering them (the transcripts can't measure this), and other agents can still ask it.

The b1 option text said +$0.5 to +$1.3, which was too low. The corrected figure is on the card.

**Cold reuse always costs more.** After 60 minutes without a call, the next call writes the whole context again: (B + C) × 2 × input. At 200k that is about +$1.39 (Opus) or +$0.70 (Sonnet) once, over a fresh start. So:
- **Same-repo reuse** (§8, step 3) goes only to an agent whose last call was **under 55 minutes ago**. With the default 30-minute idle limit an awake agent always passes. The check matters only when the person raises the limit.
- **An agent's own continuity** (its thread, its card coming back, a mention, §7) may wake it cold. The person asked for that continuity, and it costs at most one re-write.

**Why tokens and not turns.** API calls per session range from 4 to 247, while context grows steadily at about 2.1k per call, so tokens are the better measure. The budget also bounds the number of items. A small item adds at least about 5k, so 200k leaves room for at most about 30 small items and far fewer card-sized ones. Never 100.

## 6. Rest instead of end

**What the 30-minute idle limit guards today** is the process: about 0.5 GB of memory each, and a slot under the device cap (spawned-agents.md §6). Today the only way to free those is to end the agent. Resting frees both and keeps the identity. So **the idle limit stays, relabelled "Rest an idle agent after"**, and it removes nobody. Its row and range don't change.

**When an agent rests.** It rests where today's `end_finished` (`core/src/service/spawn.rs:3298`) ends it, and on two new events. Each case:

| Today's end, or new event | Now |
|---|---|
| The card is done, canceled, or no longer held | The next item (§8) if under budget, else rest when idle; retire if over budget |
| The done check is unanswered after `DONE_GRACE` (5 min) | Leaves the card as today, then the next item or rest. A later "Not done" asks its last agent first (§8, step 1) |
| Blocked on a question for the person, past the idle limit | Leaves the card as today, then rests. The answer brings the card back to it first |
| Parked on other work past the idle limit (#474) | Leaves the card as today, then rests. When the card is offered again it goes to its last agent first |
| A thread or ask quiet past the idle limit | Gives up the take as today, then rests. The person's follow-up wakes it (§7) |
| A thread or ask taken by another agent | Rest |
| A helper's finished part past the idle limit | Rest (helpers are never reused for other cards, §8) |
| "Not done" lifts the card's weight (decision 61) | Retire: the card needs a stronger model |
| **New:** a queued start needs its slot | The idle agent idle longest that holds nothing rests. Only for a card whose hand-out already ran and found nobody, and never an agent that is a candidate for that card or holds an offer |
| **New:** Claude Code dropped the session itself | A rest if the agent was idle and held nothing WORKING, else a crash as today (below) |

The last row matters because Claude Code drops background sessions about 60 minutes after they go idle (card #498). Today that is recorded as a crash, and two in a row ask the person "start another?".

**What rest does:**
1. **The hand-off**, through the exit protocol with rest wording: Where, Left off, and a closing comment in the shape of §12 on each item, plus a note proposal if it learned something about the repo. A resting agent may never wake again, so every stop Irori makes pays this one turn.
2. **It lets go.** It leaves its card itself, gives up any take, and its leases are released. "Leaves the card as today" in the table above means the same point in time, not the same path. Today `return_gone` writes the leave after `agent.gone`, but a resting agent stays on the roster and writes no `agent.gone`. So rest writes `task.left` with a new reason, **`rested`**. That reason:
   - counts no crash;
   - sends no "nobody is on it" notice;
   - shows on the card as "agent resting";
   - records the agent as the card's last agent (A.2).
3. **It writes `agent.rest`**, then stops the process with **`claude stop` only**. There is no `rm`: the job and its files stay, because a wake reuses the options the job saved. The spawn's settings file stays at its path for the same reason (§7, spike results).

**A drop-off has no hand-off**, because the process is already gone. Irori writes "rested without a hand-off" on its items. When it wakes, its wake notice asks it to write Left off before any other work on the item, once Irori's tools answer. They can be missing for the first several seconds of a wake (§7, spike results).

**Restarts keep resting agents.** Today, at every start of Irori (and so after every update), `clean_after_start` ends each spawned agent that Claude Code lists as not running. A resting agent is exactly that kind of agent, so it would be retired. With rest, `clean_after_start` and `reap_ended` skip any agent whose current life is resting. After a device restarts, other devices keep its resting agents' cards (Appendix A.4).

**Stop here and Stop everywhere retire agents; they never rest them** (decision 60: the person rejected "pause" because it "makes it seem like they can resume the tasks").

**How it is recorded.** There are new rows, `agent.rest`, `agent.wake` and `agent.retire`, each carrying the agent's `life`. `life` counts its processes: it is 1 at spawn and goes up by 1 with each wake. On every device the roster shows a resting agent with status **resting**. That is a third state beside idle and busy, not "gone":
- Hand-out skips it, because it isn't idle.
- Mentions, messages and thread follow-ups treat it as reachable, and a delivery that would wake an awake agent wakes it instead (§7).

`agent.gone` stays for a retire only. Appendix A.4 gives each place that reads these.

**Resting agents don't count toward the device cap.** An agent started by another agent still counts under decision 51 while it rests, because it can come back.

## 7. Waking a resting agent

**One rule: a delivery that would wake an idle agent wakes a resting one**, if its sender is allowed to wake it (below). That covers:

1. a follow-up in a thread it answered;
2. a mention of it, or a `send` to it;
3. a notice about one of its own items: its card coming back to it (§8, step 1), a wait it set firing, or a review result;
4. a card made from its thread, when §8 picks it;
5. the person's Wake button on the roster or the card.

Messages that are for its next turn (decisions 18 and 63) wait in its inbox until it wakes, as they do for any idle agent.

A follow-up in a thread it answered needs one extra change. Today `thread_follow_up` sends only to live repliers and otherwise reopens the thread to a fresh agent. With this change it counts a resting replier as reachable. That is the cycling the person saw, fixed.

**Who may wake one.** A wake starts a process with shell access. This **amends spawned-agents.md §7's "redirected mentions, sends and comments never start an agent"**, for resting agents only:
- **The person's own words** wake it. So do Irori's notices about its own items.
- **The person's own agents' mentions and sends** wake it when "My agents may start agents" is on and there is room under its limit, and that wake counts under decision 51. Otherwise the message waits for its next turn.
- **Never from anyone else.** Another member's text on a shared board, their agents, or a message redirected from anyone else never wakes it. That message waits for its next turn, and if the agent retires first it lands on the item as today ("for sorrel, who has finished").

**A wake is a start.** It passes the same gates a spawn does:
- Pause agents on its device (decision 56);
- Stop here and Stop everywhere (decision 60);
- Pause all;
- a Claude usage-limit hold;
- the device cap, where it queues with spawns.

While a gate holds it, the item says why ("wakes when the Mac is back", "queued: 1st for the Mac"), the way `spawn_why` explains a start.

**How it wakes.** It builds on the person's existing Revive (`window/src/roster.rs` about 295-433), which runs `claude --bg --resume <id>` in the session's folder after checking that the folder exists and is trusted. The spike (below) settled how flags go with it:
- **A plain wake passes no flags:** `claude --bg --resume <id> "<prompt>"`. Claude Code's job record keeps the spawn's flags (`respawnFlags` in `~/.claude/jobs/<job>/state.json`) and applies them again: `--name`, `--model` and `--effort` (so the agent keeps its run level, §8), `--permission-mode`, `--settings` and `--add-dir`. The prompt lands as the agent's next turn.
- **The prompt is a fixed line**, as a spawn's is (`prompt_for`, `window/src/spawn.rs`). The wake notice holds card and person text, so it is written to a file in `briefs/` and the prompt names that file. That keeps such text off the command line.
- **Never pass a flag beyond `--bg --resume <id>` while the job record exists.** Claude Code then starts a copy under a new session id instead of continuing the session.
- **The settings file stays at the spawn's path** for the agent's whole life, and every wake rewrites it in place, an `rm` wake too. A wake reads the file again at that path, and fails before it starts if the file is gone.
- **Which wake** depends on the job record, read first:
  - **There is a record and no flag must change:** the plain wake.
  - **There is a record and a flag must change:** `claude rm <job>`, then `claude --bg --resume <id>` with every flag a spawn passes. With no record left, the passed flags apply and the session id holds.
  - **There is no record** (Claude Code removed it): the full resume with every flag, and no `rm`.

**Permission mode and the settings file are worked out again at each wake** from the board's current permissions for the item it is woken for, so a setting the person has since tightened holds:
- **Allow and deny rules** go in the rewritten file.
- **A mode change takes the `rm` wake**, which passes the new `--permission-mode`. `permissions.defaultMode` in the file does not override a saved `--permission-mode`.
- **Where `rm` can't run** (the job record names a worktree, A.4), two cases:
  - Bypass turned off goes in the rewritten file as `permissions.disableBypassPermissionsMode: "disable"`. A plain wake then runs in `auto` (Claude Code's classifier decides), not `default`, and the item says so.
  - Any other mode change can't be made, so the agent retires and a successor starts (the fallback below).

Older research said a resume doesn't restore `--settings` or `--add-dir` (`docs/research/05-raw-findings.md` 395-399). That holds for a foreground `claude --resume`, and for a `--bg` resume after `claude rm`. It does not hold for a `--bg` job that still has its record (Claude Code 2.1.319).

Irori writes `agent.wake` just before starting the process. Resume keeps the session id, so the key is the same and no bind is needed. A wake can still fail: Claude Code makes a copy, the settings file is missing, or the process exits before it starts. Then Irori writes `agent.rest {reason: wake_failed}` for the new life, so the roster never shows an idle agent with no process, and whatever woke it starts a successor (the fallback below). Its first turn gets a wake notice that says:
- why it was woken;
- when it rested;
- what changed on its items since then, through the same hooks a wake uses today.

**Races.**
- Rest, wake and retire rows are written only on the agent's own device, one at a time per agent.
- A Wake pressed on another device is a request that the agent's device serves, the way End is served today. It is dropped if that life has already retired.
- A trigger that arrives while a rest is still finishing waits until the process has stopped, then wakes it. "Stopped" means `claude agents --json` no longer lists the session as live: `claude stop` can return while the process is still exiting, and a resume then starts a copy under a new id.
- End on a resting agent retires it straight away. Its hand-off was written when it rested.

**The folder.**
- A thread or ask agent runs in the main checkout, which is always there.
- A card agent runs in its card's tree. Treeclean keeps a tree while any item it served is open, and may remove the tree and its `target/` once they have all settled. A wake then recreates a plain worktree **at the same path**, detached at `origin/main`, because Claude Code finds the transcript by the folder. The recreated folder must pass the same trust check. The agent then switches to the branch its item needs (§8).

**Names.** A resting agent keeps its word for its whole life. The namer neither gives the word to another agent nor renames the agent (`core/src/namer.rs`: `NAMES_BUSY` is 2 hours today, and `settle()` can rename). On retire, today's protection applies: 7 days before the word is reused, 30 days in the lists.

**If resume doesn't hold up.** The first doing card (#1211) was a live spike with plain `claude`. It checked the five things below, and all held (spike results, below). The fallback now applies to one wake that fails, or to every wake if a later Claude Code breaks one of them:
- that the transcript is found after the folder is recreated;
- that the session id stays the same;
- that the name, model, effort and settings stick;
- that a resume within the hour reads the cache warm;
- that Claude Code accepts a resumed `--bg` job the same way.

When that happens, the design falls back:
- **rest becomes retire**: the same hand-off is written at the same points, and the name goes;
- **whatever would have woken the agent starts a successor** (§9), seeded with the hand-off and the thread.

Everything else still works without resume: the budget, same-repo reuse while awake, and the roster's free group. Continuity then lasts only while the process is up, up to the idle limit. That still fixes the cycling for follow-ups that come inside the limit.

### Spike results (#1211, 2026-11-09)

**Go: resume holds up, with the wake command changed.** "How it wakes" above now says how. As first written, the wake passed the spawn's flags again, and that fails the same-id check: Claude Code starts a copy. A wake with no flags, or one after `claude rm`, keeps the session. Checked on the PC with Claude Code 2.1.319, using plain `claude` in a throwaway worktree, `.claude/worktrees/spike-1211`.

**The spawn.** Session `e53a4589-6684-4ee3-87c1-ef641523a064` ("spike1211-a") was started at 19:02:58Z with a spawn's flags, in a spawn's order:
- `--add-dir <scratch dir>`, `--bg`, `--name spike1211-a`, `--permission-mode default`, `--model sonnet`, `--effort low`, `--settings A` (an env marker and one Bash allow rule);
- the env had `DISABLE_AUTOUPDATER=1` and no `CARGO_TARGET_DIR`.

It read a file in the added folder with no prompt, printed the marker from settings A, and was told to remember a word ("heron"). Its last call (19:03:31Z) used input 2, cache_creation 880 and cache_read 58,461, so the context was about 59.3k. Every call asked for the 1-hour cache (`ephemeral_1h_input_tokens`).

**Each check:**
1. **Stop keeps the transcript.** After `claude stop e53a4589` the process was gone. Its transcript, `~/.claude/projects/C--Users-4step-projects-irori--claude-worktrees-spike-1211/e53a4589-….jsonl`, kept its 59 lines. The job record `~/.claude/jobs/e53a4589/state.json` stayed with its `respawnFlags`. `claude agents --json --all` listed it as `state: done` with no pid, and `claude agents --json` (live sessions) no longer listed it.
2. **The transcript is found after the folder is recreated.** Yes. The worktree was removed with `git worktree remove --force` and recreated at the same path, detached at origin/main. The resumes below all ran from the new folder and continued the old conversation.
3. **The session id stays the same, but only without flags.**
   - Resume with every spawn flag again and a new settings file B (19:10:39Z): Claude Code printed "note: background session e53a4589 keeps its own saved options, so the flags you passed started a copy as 62ea85a0. Without flags, the same command continues e53a4589 itself."
   - The copy, `62ea85a0-d5d2-4253-b564-fe5a6d84efca`, had the whole context (it answered "heron") and settings B. The original's transcript did not change (471,182 bytes).
   - A plain `claude --bg --resume e53a4589-… "<prompt>"` (19:14:28Z) printed "woke session e53a4589 with its saved options (--add-dir, --name, --permission-mode, --effort, --settings, --model)". The same file grew from 59 to 79 lines, the prompt landed as the next user row, and it answered "heron".
4. **Name, model, effort and settings stick.**
   - After the plain wake the session kept its name (`agent-name` rows and the agents list: spike1211-a), its model (`requestedModel` and `message.model`: claude-sonnet-5-5), its effort (`effort: low`) and its mode (`permissionMode: default`). It read the added folder again with no prompt.
   - Settings file A had been rewritten in place before the wake, and the wake printed the new marker (`rewritten-A2`). So the file is read again from its saved path.
5. **A resume within the hour reads the cache warm.**
   - The cleanest reading is the copy's first call, 7m24s after the last call (past the 5-minute cache): cache_read 59,341 and cache_creation 483, about the whole prior context.
   - Later wakes read warm too (cache_read 59,824 and 60,167), but the copy had refreshed the shared prefix by then, so they don't time the cache.
   - A wake within an hour therefore costs about one cache read of the context. §5's 55-minute rule stands.
6. **Claude Code accepts a resumed `--bg` job the usual way.** `claude agents --json` listed it as `id e53a4589`, `kind background`, the same `sessionId`, `name spike1211-a`, `status idle`. `claude logs`, `stop` and `rm` worked on it as on any job.
7. **`claude rm` after stop doesn't break a later wake.**
   - `rm` deleted `~/.claude/jobs/e53a4589/` (and the saved flags with it). It left the transcript and the folder in place; `worktreePath` was null.
   - A resume with new flags (`--effort medium`, settings C) kept the id. The file grew from 79 to 99 lines and the agent answered "heron". `effort: medium` and settings C applied, and it read the cache warm (cache_read 60,167).

**Other findings that shape the wake:**
- **A missing settings file stops a plain wake.** Settings B was moved away, then the copy was woken with no flags. Its job failed with "exit 1 before init — Error: Settings file not found: …settings-B.json".
- **The settings file can turn Bypass off.** Session `f853da4d-525b-4519-8a72-b2aa782b000b` ("spike1211-b") was spawned in `bypassPermissions`. After a stop, its file was rewritten with `permissions.disableBypassPermissionsMode: "disable"` and it was woken with no flags. Its user row then said `permissionMode: auto`, and a `whoami` that no rule allowed was approved by the classifier.
- **The settings file can't change other modes.** Session `054e253d-58a8-4a99-a8b0-a21c54b0c148` was spawned in `default`. Its file was rewritten with `permissions.defaultMode: "acceptEdits"`, and after a plain wake it still ran in `default`.
- **Waking too soon after a stop makes a copy.** A resume sent right after `claude stop` returned said "session 054e253d is already running in the background, so this started a copy as 355af3f4". The copy did not come up with the saved options: it ran in `bypassPermissions` without the added folder, and never answered. The race note under Races covers this, and a wake stops and removes any copy it gets back (A.4).
- **Irori's tools return several seconds into the first turn.** Four wakes had the `irori` MCP server's tools loaded beforehand. On each, the tools dropped out for about 7-18 s at the first turn and came back within it (e53a4589: gone at 19:14:48Z, back at 19:15:00Z). A fresh spawn also starts without them, so the wake notice must not depend on an Irori tool being there for its first call.

**Not checked:**
- When Claude Code drops a background session itself after an idle hour (card #498), the spike doesn't show whether the job record survives. The `rm` wake path covers that case either way, as long as Irori passes every flag.
- A cold wake, past the hour (§5 prices it).
- The Mac.

**Cleanup.** Five jobs were started: e53a4589, 62ea85a0, f853da4d, 054e253d and 355af3f4. Each was stopped and removed with `claude rm` (every `worktreePath` was null), and `claude agents --json --all` lists none of them. The throwaway worktree was removed. Its transcript folder was deleted once the numbers above were recorded.

## 8. The next item

**Who may take an item:** an agent that is under budget, not compacted, and **holds nothing open**. "Nothing open" means four things:
- every card it took is settled or left;
- it has no open step;
- it has no assigned card it hasn't joined;
- it holds no live offer or handed ask.

When a card, ask or thread needs an agent, Irori tries these in order, on the device that hands it out:

1. **Its own last agent.** This is the item coming back: a "Not done", the person's answer, or a parked card that can go on. It goes first to the agent that last worked it and left it, if that agent is free by the rule above and under budget. Awake, the agent gets the card by assignment, as #1208 does. Resting, it is woken for it. If that agent is busy, retired, or can't start (any gate in §7), the card goes on to step 2.
2. **Its thread's agent** (#1208, decision 68, now on main). A card made from a thread goes to that thread's agent. #1208 built this as `route_to_thread_agents` (`core/src/service/spawn.rs` about 2193), which hands the card over with `hand_off` when the agent is on this device's roster and holds no other card. It left `thread_agent_reusable(key)` (about 2240, `true` for now) as the place for this design's check. This plan:
   - fills that function with the budget and the same run level (below), through the shared check in A.2;
   - extends the agent lookup to a resting thread agent, which is woken for the card instead of a fresh agent starting;
   - takes the card back when a wake can't start, so it goes on to step 3.
3. **A same-repo agent that is awake**, through the ordinary offer (decision 40). The candidates must be all of these:
   - spawned by Irori, awake and idle; resting agents aren't woken just to be asked;
   - **on the device that hands the card out**: v1 is same-device only (§11);
   - **in the same repo**: both repo keys set and equal;
   - **at the same run level** as the card's `Task::run_level`;
   - free and under budget, as above, with the last call under 55 minutes ago;
   - not a helper on a led card, and not working in a tree another live agent uses.

   They are ordered as `free_for` orders them: `machine_rank` first, then the one idle longest, then key. The offer is handed on by the existing events.
4. **A fresh spawn**, as today.

**Why the same run level.** An agent's model and effort stay as spawned for its whole life: a plain wake applies them again (§7), and this design never changes them. Giving it a lower-level card costs more for nothing. Giving it a higher-level card defeats decision 61, whose lift exists to get a stronger model.

**No double start.** Today, on a board that starts agents, the spawn sweep runs before hand-out and the offer path is switched off. So a new card is spawned for at once, and no offer would ever be made. The fix: on such a board, a card is spawned for only after hand-out has run for it and found nobody, and the two switches that turn offers off there are removed. Without it, #453's "two agents on one card" comes back. Appendix A.2 names the places.

**Handing the item over.**
- **The record.** Irori writes `agent.took` for the item. This is the multi-item mark (Appendix A.3).
- **The brief.** Irori writes a brief file for the item and sends a "You now have #M; read its brief at <path>" notice, the way it sends the end notice today. A next-item brief leaves out the fixed parts the agent already read (build output, base check, device_free, Talk, splitting). It keeps the card, the person's words nearby, Left off and the exit rules.
- **The worktree.** A running session's folder and environment can't change.
  - A card agent keeps its tree. Its brief says to push the last card's branch, then run `git switch -c card-<M>-<g> origin/main` in the same tree. The new card's Where records that branch, and treeclean keeps the tree until both cards settle. The shared `target/` stays warm, which is a small win.
  - A thread agent works in the main checkout and has no tree. #1208's hand-off note tells it to "make or enter this card's own worktree (EnterWorktree)". This plan has Irori make the tree itself with `spawn_tree` and put the path in the note. The tree is then tied to the card, so treeclean keeps it while the card is open and removes it once the card settles, and an agent can't forget to make one.
  - **The reopen guard.** If card N is reopened while a reused agent works card M in N's old tree, N's new agent must get a fresh tree on its own branch, not N's old tree, which is now on M's branch.
- **The wording.** Every brief, notice and refusal that says "take only this card" or "Irori ends you after that" changes. They become: "work only #N now; when it settles, Irori may give you your next item, let you rest, or retire you" (Appendix A.5).

## 9. Retiring

**Why an agent retires:**
- **budget**: it was past the budget when its item settled, or when it was about to rest;
- **compacted**: at its next item boundary or rest;
- **quiet**: it is resting, it holds no card, and nothing has been for it for 24 hours. An open thread counts as settled once nobody has written in it for 24 hours.
- **crowded**: a device has more than 20 resting agents. When one more rests, the one quiet longest retires. 20 is a fixed number, not a setting.
- **ended**: the person's End, an agent's `agent_end`, Stop here or Stop everywhere;
- **lifted**: a "Not done" raised its card's level (decision 61).

The quiet and crowded checks run in the spawn sweep over the device's resting agents. Today's end loop walks only presence, which never lists a stopped session.

**What retiring does.** An awake agent first runs the exit protocol; a resting one already wrote its hand-off. Then Irori:
1. writes `agent.retire`;
2. writes `agent.gone` "finished", the same row today's end writes, so everything that reads it still works;
3. runs `claude stop` (if the agent is awake) and `claude rm`;
4. releases its name to the namer's normal protection.

**The successor starts on the next event, never at retire.** The retired agent's items are let go as today's end lets them go. When something is next for one of them (a follow-up, an answer, a mention), its next agent starts: a **successor**. Its brief opens with "You take over from sorrel (retired: context past 200k). sorrel's closing notes:" and quotes them. The thread or card gets one line: "sorrel retired (212k); quill takes over."

**A message or mention for a retired agent** lands on its last item as today ("for sorrel, who has finished"), and adds "quill took over" when there is a successor.

### As built (#1215, 2026-11-09)

Rest and retire are on main, with §7's fallback, so nothing that would wake a resting agent strands it. The code is `core/src/service/resting.rs`, `coord/src/board/rest.rs`, and the `agent.rest` / `agent.retire` arms in `coord/src/roster.rs`.

- **One resting map.** `SpawnState::resting` holds each resting agent's last card, keyed by agent. Core folds it from its own rows at open, with the roster's own `apply` (`roster::resting_of`), so a restart keeps resting agents. The local roster closure lists them with status `resting`, and coord reads their rest times through `Options::resting`. Every change to the map happens under `SpawnState::rest_lock`, whichever thread makes it: the sweep, the presence recorder or the person's End. The map is written before `agent.rest`, and the rows before `claude stop`.
- **Rest.** `begin_rest` always asks for the hand-off (notice kind `spawn.rest`, with the §12 shape). The hand-off counts only if written since the rest began. The rest itself (`rest_now`) runs in this order:
  1. the "rested without a hand-off" line, if the hand-off never came;
  2. `Coord::rest_leave`: `task.left` reason `rested` (no crash, nobody told; the card shows "Nobody on it: X is resting"), the ask or thread take given back, offers passed, leases released;
  3. `agent.rest`;
  4. `Spawner.stop` (`claude stop` alone).
  
  `claude stop` is confirmed on later sweeps: an agent Claude Code still lists live is stopped again after `END_RETRY`. With a spawner that has no `stop`, an agent ends as before.
- **Which settles rest and which retire.** Every private-board end in `end_finished` goes through `settle_agent`:
  - Stop here, Stop everywhere or an open wind-down: retire `ended`;
  - past the budget, compacted, or "Always start a fresh agent": retire `budget` or `compacted`;
  - otherwise: rest `idle`.
  
  A "Not done" lift retires `lifted`. End and `agent_end` retire `ended`, at once for a resting agent and with no notice. A shared board's agent keeps today's one-item end (§11).
- **Drop-off.** In `record()`, a Removed key that rests, or is mid-rest, writes nothing. A spawned agent of this device's own log that was idle and that nothing waits on rests `dropped`. "Nothing waits on" means it holds no card that is submitted or working (assigned counts) and has no ask or thread take. The sweep writes its let-go and the line. Anything else is still `agent.gone`, a crash. An agent whose prompt never got in (card #677) still holds its card assigned and submitted, so it stays a crash.
- **The fallback** is the one switch `coord::RESTING_WAKES` (off):
  - Coord's `here_now` counts a resting agent as not there wherever a delivery is chosen at send time: a thread's follow-up, a comment's hearers, watch digests, the named-ask filter, the "tell the poster" checks. `turn_state` reads resting as gone.
  - `ask_waits_for_person` (was `ask_resting`) holds a thread or ask whose agent rests until the person writes after it let the ask go.
  - Core's `sweep_resting` retires a resting agent with the new reason **`woken`** when an open wake-mode delivery is waiting for it. Only these senders count: the person, Irori, or the person's own agent while "My agents may start agents" is on. Its leftover notices are closed, and plain messages take the "who has finished" path.
  - A fresh agent started for an item a resting agent served retires that agent `woken` too (`retire_for_start`).
  - #1216 turns the switch on and builds the wake in place of these retires.
- **Retire** writes `agent.retire`, then today's `end_now` (`agent.gone` "finished", stop and `rm`). The quiet (24 h), crowded (more than 20) and Stop retires run in `sweep_resting`, because presence never lists a stopped session.
- **Elsewhere:**
  - `clean_after_start` and `reap_ended` skip resting agents, and `spawned_here` and `spawned_on` don't count them.
  - Storage › Briefs keeps the files of agents that run or rest (`Core::kept_brief_stems`).
  - Stop's wind-down skips resting agents; the sweep retires them.
  - Shared boards' `agent.card` shows resting as gone.
  - Review picks and Talk's "replying" pass resting agents by.
- **From the review and the scratch run:**
  - **The let-go time.** An agent let go when its rest was *asked* (`agent.rest`'s `at`). Coord keeps that time per agent after it retires (`rest_times`, from `Coord::mark_rested` and replay), and `ask_waits_for_person` measures from it. A word the person wrote during the hand-off wait, or while it rested, still starts the next agent after a woken or quiet retire.
  - **Races.** The person's End or a lift during a rest's hand-off wait turns it into a retire. `sweep_ending` carries the kind it read and skips a key another sweep (End runs one off the sweep thread) has already finished. A retire writes its row and its end under `rest_lock`.
  - **Restarts.** Once per run, Core reconciles every resting agent's let-go, because a drop-off's let-go is queued in memory. A presence Added or Updated for a resting key ends the rest only if that session started after the rest, as a Revive or a resume by hand does. The rested life's own process, still exiting across a restart, is stopped again and stays resting. #1216: check what `startedAt` a resumed `--bg` session's file carries.
  - **`came_for`.** It passes by Irori's own notices from before the rest: they are about what the agent handed off. Anything from the person still counts. A usage-limit end clears a pending rest or retire.
- **Not built here:**
  - s6's slot rest covers threads and asks only. On a board that starts agents, a card's offer row is written without a hand-out (`OFFER_WAITING`, empty `to`), so "the hand-out found nobody" can't be told apart until #1217 removes those skips.
  - `life` is always 1 (`FIRST_LIFE`) until #1216 writes `agent.wake`. #1214's origin item still says `life: 0`.
  - The successor brief's "You take over from…" line, and the roster's Free and Resting groups, are #1217 and #1218.
  - On older devices a resting agent shows its last status, usually idle, until it retires.

## 10. Terminal agents and "Offer it cards"

- **Unchanged for terminal agents.** "Offer it cards" still decides whether an idle terminal agent is handed unheld cards. On a board that starts agents, today's code never offers one a card. This plan's no-double-start fix (§8) brings back that offer, which spawned-agents.md §6.3 always intended. Both kinds of agent are then candidates in one offer, ordered by decision 40.
- **`set_agent_offer` still refuses spawned agents**, because the budget decides their reuse, not a switch. Its refusal text ("it is never offered other cards", `core/src/service/spawn.rs:2000`) changes.
- **Wording that changes.** Text that says "an idle agent with Offer it cards on" now also covers reusable spawned agents, and becomes "an idle agent that takes cards" (Appendix A.5).
- **The New card dialog.** On a board that starts agents, it says a fresh agent "starts for this card … and ends when it's done". That becomes "a free agent in this repo, else a fresh one". The person's own cards go through the same order. Their "Give to" and assignment still override it.

## 11. Shared boards and other devices

- **v1 covers the private board and the person's Talk.** A spawned agent on a shared board keeps today's one-item life. A shared board's bypass rules and other-people's-text taint are fixed at spawn (card #1075), and reusing an agent across boards would cross logs.
- **v1 is same-device.** Hand-out runs on the device that hands the card out, and context is measured only where the transcript is. Other devices show the gauge (§4) but don't offer cards to another device's agents.

## 12. What the person sees

**The hand-off.** Each rest and each retire ends with one closing comment per item, in a fixed shape. No new row kind is needed.

```text
Done: <what is finished, with the commit or PR>
Learned: <what the next agent needs to know, or "nothing new">
Open: <what is left, or "nothing">
```

Left off and `notes_propose` stay as they are. "Learned" becomes a note when it is about the repo.

**The roster** is grouped by the roster's own state, not by which cards have working rows. That way thread agents, and agents between items, show where they belong:

```text
AGENTS                          2 on cards · 2 free · 5 resting
Yours
  ● pebble      irori · working
On cards
  ● sorrel      #1212 · working · 64k
  ● quill       #1215 · blocked on you · 101k
Free
  ○ tansy       irori · idle 12m · 41k of 200k · served #1207, #1208
  ○ marram      ww-multiplayer · idle 3m · 88k of 200k · served #1190
Resting (5)                                                   ▸
```

Opened, Resting lists each agent with its repo, its context, its open threads and when it rested: "yarrow · irori · 52k · thread #1201 open · rested 2h ago".

**The hover.** It replaces "Irori started it for #N" with:
- "Served #1207 (thread), #1208";
- the context: "41k of 200k · retires past 200k";
- the state: "awake, idle 12 min" or "resting since 14:02".

Its actions are Attach and Logs (awake), Wake (resting), Rest now (awake), and Retire. It still shows the model and effort.

Elsewhere:
- "Free" counts idle terminal agents with "Offer it cards" on plus reusable spawned agents.
- The sidebar's "each ends when its card is Done" changes.
- Settings › Running lists resting agents.
- The mention picker lists resting agents, marked resting.

**The noisy-roster risk.** Today every spawned agent leaves, so the roster stays short. With rest it could fill with agents from last week. Four things keep it readable:
- Resting folds into one count by default.
- A resting agent retires after a quiet day.
- No more than 20 agents rest on a device.
- A resting agent shows only what it still has open.

If the person still finds it noisy, the next step is a shorter quiet day, not removing agents sooner.

## 13. Risks

- **Cost creep.** Each reuse carries context. Bounded by the budget, the 55-minute rule for same-repo reuse, and the setting. Settings › Agents already shows the day's cost.
- **Resume isn't what it seems.** The spike (#1211) says go, with a plain wake that passes no flags (§7, spike results). A later Claude Code could change how saved options work, so the wake checks the session id it got back. §7 names the fallback.
- **The wrong agent for a card.** Same repo is not the same subject. Mitigated by the same run level, the budget, an offer the agent can decline, and the brief's "work only #N".
- **Two agents in one tree.** Covered by the reopen guard (§8).
- **Restarts and updates.** Every Irori start would retire resting agents without the `clean_after_start` change (§6). A test pins it.
- **Waking from someone else's text.** Prevented by §7's rule, with a test for each kind of text that must not wake an agent.
- **Older devices.** An older Irori ignores the new rows. On its roster a resting agent shows as gone. Older devices offer spawned agents nothing, so nothing goes wrong, it just looks different there.

## 14. Build order: the doing cards

The cards sit on the Board after #1209. Each has steps, files and done-when, and each can be built without more design.

1. **#1211 Spike: resume a stopped spawned agent.** Live, with plain `claude` in a throwaway worktree. Gates 5 and 6. Done: go, with a plain wake that passes no flags (§7, spike results).
2. **#1212 Context measure** in presence, carried on `agent.status` and `agent.rest`.
3. **#1213 Budget setting and the shared `reusable` check.** After 2.
4. **#1214 The multi-item mark** (`agent.took`; Appendix A.3).
5. **#1215 Rest instead of end**: rows, roster state, letting go, drop-off, restart, cap, names, retire rules. After 1, 3 and 4. It ships together with 6, or with §7's fallback until 6 lands, so a resting agent is never stranded.
6. **#1216 Wake a resting agent**: the wake rule, who may, start gates, flags and permissions, folder, races. After 1 and 5.
7. **#1217 The next item**: last agent first, thread cards, same-repo offer, no double start, next-item brief, worktree switch, reopen guard, wording. After 3, 4 and 6 (#1208 is on main).
8. **#1218 Roster, hover and Settings.** After 2 to 6.

## Appendix A: notes for builders

### A.1 Context in transcripts (card 2)

- **Where to read.** The newest assistant row in `<claude>/projects/<slug>/<session>.jsonl` with `isSidechain` false and a real model. Context = `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`. Output tokens are not added; they come back as the next call's input.
- **Advisor rows.** On a row that called the advisor, the top-level usage adds up every `usage.iterations[]` entry (message, advisor_message, message), about twice the real context. Read the last iteration with type `message`. Use the top-level fields only when there are no iterations. 88 of 300 recent sessions used the advisor.
- **Subagents.** Their rows are in `<session>/subagents/**/agent-*.jsonl`, and they don't grow the main context. `spawn_cost.rs` merges those files into its totals, so its figure is the wrong source.
- **Compaction.** It appears as a `{"type":"system","subtype":"compact_boundary","compactMetadata":{"preTokens":…}}` row, then a user row with `isCompactSummary: true`. The next call reports a small size again. A boundary newer than the last usage means "compacted, size unknown". None of 300 recent sessions compacted; an older session with a 200k window did, at about 222k.
- **The window size isn't recorded**, so the budget is an absolute count of tokens.
- **The hover's "4.8M tokens"** (`spawn_cost.rs` `Tokens::total`) sums cache reads over every call. It stays a cost figure.
- **Presence.** `refresh_tail` (`presence/src/watcher.rs`) and `read_tail` (`presence/src/transcript.rs`) already re-read the last 256 KiB on change, and skip sidechain rows.
  - Add `usage` to the `MESSAGE` schema.
  - Add `ctx`, `ctx_at` and `compacted` to `Tail`, copied in `card()` to `presence::AgentCard` (serde, `skip_serializing_if`) and to `coord::roster::AgentCard` (`go_struct`, omit when zero).
  - Keep the old value when the tail holds no assistant row.
  - They ride on `agent.status` (`record()`, `core/src/service.rs` about 5605-5680; `AgentStatus`; `StatusRow` and the `AGENT_STATUS` arm in `coord/src/roster.rs` about 416-435), and on `agent.seen` through `row_card`.
  - **Do not** add them to `record()`'s equality check or presence's `changed()`. Either would write a replicated row on every token.

### A.2 Hand-out wiring (cards 3 and 7)

- **The shared check.** Coord can't read files under its lock. Core measures off-lock, and the fold reads a live map, the same pattern as "Offer it cards" (`SpawnState.offers`, the `offers` closure on `coord::Options`, `core/src/service.rs` about 1414-1437 and `boards.rs` about 1514-1562).
  - `reusable(key)` returns `{ctx, ctx_at, compacted, budget}`.
  - The fold compares `ctx_at` with its own clock (`self.stamp()`), so coord tests can use their fake clock. There is no stored "warm" flag.
  - #1208's `thread_agent_reusable` (`core/src/service/spawn.rs` about 2240) calls the same check.
- **Who is offered.**
  - `may_hand_to(key, t)` (`coord/src/board/spawn.rs:256`) gains the step 3 arm (`reusable_for(t)` names `key`). This is what `free_for` (`offers.rs:282`) calls to pick.
  - `spawned_may_take` (`:265`) and `check_free` (`offers.rs:207-226`) gain "a live offer on `t` names `key`", so `board_join` accepts it.
  - `takes_handout(key)` (`:249`) takes no card and also drives ask and thread hand-out (`asks.rs` 1361 and 1693), so it stays false for spawned agents.
  - `idle_report` and `unserved_offers` (`offers.rs` about 350-376) use `may_hand_to`, so Home doesn't say "no agent is free" while one is.
  - "Holds nothing open" is new: `busy_with` (`offers.rs` about 171-187) counts only WORKING cards, so add BLOCKED, INPUT_REQUIRED, parked, assigned-unjoined and a pending done check.
- **No double start.**
  - Core runs `sweep_spawns()` before `sweep_asks_partial()` (`core/src/service.rs` about 4735 and 5564).
  - `offer_card` (`offers.rs:427`) and `sweep_offers` (`offers.rs:809-815`) skip boards with `spawning_on()`. Remove both skips.
  - `spawn_candidates` (`coord/src/board/spawn.rs:1686`) and `request_spawn` (`:481`, its `needs_agent` check) skip a `needs_agent` card whose `offer` is None (hand-out hasn't run yet) or whose live offer names someone.
  - Test: a new card with a reusable agent idle gets `task.offered {to}` and no `task.spawn` in the same sweep.
- **The pinned test.** `a_spawning_board_never_hands_a_new_card_to_an_idle_agent` (`coord/tests/it/offer_opt_in_test.rs:92`) becomes "never to an idle terminal agent without Offer it cards; first to a reusable spawned agent". Build it with `with_offers`: `e.opts()` leaves `offers` None, which counts every terminal agent as opted in.
- **`turn_state`** (`asks.rs:1317`): add an arm so a resting holder of an offer counts as Gone. This is for safety, since resting agents aren't offered.
- **Steps 1 and 2 assign** (`Coord::assign`, `tasks.rs:421`), which checks neither busy nor handed. Run the "holds nothing open" check before assigning. A wake that can't start unassigns.
- **Last agent.** The fold already keeps it: `Task.let_go` is set to the leaver's key when the last holder leaves (`fold.rs` about 978). A card's last agent is `let_go` when that leave's reason was `finished` or `rested`.

### A.3 The multi-item mark (card 4)

`task.spawned` makes one mark per key. `fold_spawned` (`coord/src/board/spawn.rs` 403-450) refuses a later mark for another card. That refusal also stops another member's Irori from re-pointing the person's agent on a shared board, so **it stays**.

- The mark becomes the agent's **origin**.
- A new row, `agent.took {key, taskId, life, at, by}`, appends to `Spawned.items`. Its writer rule is `spawn_writer_ok` plus the board's `key_person` check, in the same log only.
- `Spawned::current()` is the newest item not yet settled, else the newest item.

**Finding the call sites.** On d9cd094d this grep gives 66 hits in 23 non-test files:

```sh
grep -rn 'spawn_card_of\|spawned_for(\|\.spawned(&\|coord\.spawned(\|spawned_card(\|is_spawned(' crates --include='*.rs' | grep -v /tests/
```

It misses direct reads of the map, so also run:

```sh
grep -rn '\.spawned\.get(\|\.spawned\.iter\|\.spawned\.contains_key' crates --include='*.rs'
```

| Where | Today | Becomes |
|---|---|---|
| `coord/src/board/spawn.rs` `fold_spawned`, `Spawned` | one task | origin unchanged; `items` from `agent.took`; `current()` |
| `spawn.rs` `may_hand_to` (own card or subtask), `spawned_may_take` (plus own merge card) | as listed | plus A.2's arms, for any of its items |
| `offers.rs` `check_free` (207-226) | refuses any card but its own | the same arms |
| `mod.rs` `spawned_card` (1760) | the mark's card | `current()`'s card |
| `mod.rs` `spawned` (1913), `is_spawned` (1771) | mark | unchanged (now also returns items) |
| `spawn.rs` `take_up_started` (766), `mod.rs` 1969 | claims the mark's card | claims `current()` |
| `spawn.rs` `in_chain` (838), `counted_live` (857) | by mark | origin's `counted` OR the current life's `agent.wake.counted`; a resting key still counts |
| `spawn.rs` `mark_spawned_elsewhere` (~811), `spawned_live` (~871) | by mark | by origin; `spawned_live` keeps resting keys |
| `spawn.rs` `spawned_since` (~1745) | today's count | unchanged (by origin) |
| `fold.rs` crash count (~986) and `asks.rs` TAKE_GONE (~447) | `m.task_id == t.id` | counts on the item the agent held when it went; a rest-leave counts nothing |
| `fold.rs` shared-board `TASK_ASSIGNED` writer check (~2480) | `s.task_id == t.id` | any of `items` |
| `runs.rs` (~51) run tally | by mark's run | unchanged (model comes from the origin) |
| `side_gate.rs` (~122), `asks.rs` (~1538), `comments.rs` (~1944 finished flag) | the mark's item | the item in question, by `items` |
| `core/src/service/spawn_boards.rs` `spawn_card_of` (607) and its callers in `spawn.rs`, `spawn_limits.rs`, `spawn_waiting.rs`, `spawn_remote.rs`, `spawn_agents.rs`, `trees.rs`, `boards.rs`, `service.rs` | the mark's card | `current()`'s card |
| `core/src/service/spawn.rs` `end_finished` (3298) | ends on the mark's card | decides per `current()`, by §6's table |
| `spawn.rs` `handed_off` (3540) | any comment since `m.at` | since the current rest or end began, on the items held |
| `tools/src/started.rs`, `notice.rs`, `messages.rs`, `talk.rs`, `explore.rs` | the mark's card | `current()`; `agents_started` lists every item |
| `window/src/agents.rs` `agents_running`, `agent_usage`/`spawned_card` (506), `spawned_here` (426) | one card | `current()` plus `served` |
| `window/src/cards.rs` (~1310) `spawned` flag | copied onto every working row of the key | only on rows of its items |

### A.4 Rest, wake and retire in the code (cards 5 and 6)

- **Rows.**
  - `agent.rest {key, life, at, reason: idle | slot | dropped | wake_failed, ctx, ctx_at, compacted, name, by}` (`name` added by #1215, so a device that lost the card can still show it)
  - `agent.wake {key, life, at, why, counted, by}`
  - `agent.retire {key, life, at, reason: budget | compacted | quiet | crowded | ended | lifted | woken, by}` (`woken`: §7's fallback until #1216)
  - Each is written only on the agent's device.
  - A Wake from another device is `wake.requested {key, life}`, served like `EndRequest` (`spawn_remote.rs` about 992-1017).
- **This device's roster.** It is presence plus members, not the mesh fold (`coord/src/roster.rs` 149-152, 267-274; the `local` closure at `core/src/service.rs` about 1203-1211). Presence drops a stopped session, so Core keeps the set of resting keys (current life has `agent.rest` and no `agent.retire`). The `local` closure adds a card with status `resting` for each one. That puts the agent in front of the inbox (`roster_cards`, `inbox/src/state.rs:1601`; `still_gone`, `inbox/src/sweep.rs:348`), mentions (`tools/src/mention.rs`), the namer and `key_gone_in` (`coord/src/board/stranded.rs` about 57-65).
- **Other devices.** Add `agent.rest`, `agent.wake` and `agent.retire` to the roster's `KINDS` (`coord/src/roster.rs:74`) and `apply`: rest sets `resting`, wake sets `idle`, retire removes the card. The `DEVICE_STARTED` arm (about 389-393) keeps `resting` cards.
- **`answers()`** (`coord/src/roster.rs:70`) stays as it is. Resting is neither gone nor not-answering, so it counts as reachable for mentions, `talk_replying` and the thread liveness check. Hand-out already requires `status == "idle"` (`offers.rs` about 293). Its other readers on d9cd094d, each fine:
  - `observe` (`fold.rs` ~2015) and `return_gone` (`comments.rs` ~1893): a resting agent holds no card, so there is nothing to take back.
  - `taker_gone` (`asks.rs` ~2161): it holds no take.
  - The not-answering notices (`comments.rs` ~1814).
  - The ask owner's device count (`asks.rs` ~1373).
  - `turn_state` (`asks.rs` ~1329): gets the A.2 arm.
  - An ask that names a resting agent (`asks.rs` ~1676) is handed to it, and that hand-out is a delivery that wakes it (§7).
- **The rest-leave.** Rest writes `task.left {who: key, reason: "rested"}` itself (§6). Add `LEFT_RESTED` beside `LEFT_FINISHED` in `fold.rs` (about 72-82):
  - no crash count (the crash branch about 986-992 already resets the count on any reason but `gone`);
  - no `NOTICE_RETURNED`;
  - `HolderLine` shows "agent resting".
- **The inbox.** A wake-mode delivery to a resting key whose sender may wake it (§7) starts a wake instead of `hand_off` → `on_recipient_gone` → `record_for_finished` (`core/src/service.rs` about 1291-1330). A next-turn delivery stays queued.
- **Thread follow-ups.** The `around` closure that `thread_follow_up` (`coord/src/board/threads.rs:331`) uses counts a resting replier on a device that can wake it as reachable.
  - Today's `ask_resting` (`coord/src/board/spawn.rs:901`) reads `ended_by_irori`. Make it also true while the request's last agent rests, so no second agent starts for that thread.
  - Rename it (for example `ask_waits_for_person`) so it doesn't clash with the new "resting". (#1215 did both.)
- **Presence `Removed`.** `record()` (`core/src/service.rs` about 5664-5676) writes `agent.gone` unconditionally. Change it:
  - For a spawned key of this device that has `agent.rest` for its current life, write nothing.
  - For one that isn't in `spawn.ending`, was idle and held nothing WORKING, write `agent.rest {reason: dropped}` and the "rested without a hand-off" line.
  - Otherwise keep `agent.gone`, which is a crash as today.
- **Once-per-key guards.** `mark_ended` and `ended_by_irori` (`coord/src/board/mod.rs` 1779-1813), `spawn.removed`, and `end_now`'s `NO_HANDOFF` gain a `(key, life)` guard beside the key. `ended_by_irori(key)` stays true after any retire, for today's readers (`ask_resting`, `asks.rs` ~450, `stranded.rs` ~57, `comments.rs` ~1890 and ~1944).
- **The process.**
  - `Spawner` gains `stop` (`claude stop <job>`, with the job looked up as `end_in` does in `window/src/spawn.rs`). Rest calls `stop`; retire calls `end` (stop and `rm`).
  - `Spawner` gains `wake`, two ways (§7, spike results):
    - The plain way is `claude --bg --resume <session id> "<fixed prompt naming the wake notice file>"` with **no other flags**. It runs from the session's folder, with `DISABLE_AUTOUPDATER=1` and the tree's `CARGO_TARGET_DIR`.
    - The full way passes every flag `window/src/spawn.rs` `check()` builds for a spawn. It is used when the job record is gone. It is also used when a flag must change, after `claude rm <job>`.
    - The wake reads `~/.claude/jobs/<job>/state.json` first to choose between them. `claude rm` runs only when the record's `worktreePath` is null; otherwise the wake can't change a flag (§7). Today's `holds_tree` refuses `rm` whenever the folder is a tree, so the wake needs a narrower guard. The spike saw `rm` leave a worktree folder alone when `worktreePath` was null.
  - Before either, the wake waits until `claude agents --json` no longer lists the session as live. It then checks the session id the resume printed: a different short id means Claude Code made a copy. That copy is stopped and removed, and the wake fails (`agent.rest {reason: wake_failed}`, §7).
  - `rm` already never removes an Irori tree (`holds_tree`, card #565); treeclean does.
  - `clean_after_start` (`core/src/service/spawn.rs:3127`) and `reap_ended` (`:3748`) skip keys whose current life rests.
  - Storage › Briefs (`window/src/storage.rs` about 940-947) keeps the briefs and the settings file of any agent that hasn't retired. A plain wake fails before it starts if the settings file is gone.
- **Starts.** A wake goes through the start gates in `core/src/service/spawn.rs` about 2100-2158: `agents_paused`, the inbox's `paused`, the usage hold, the cap and `can_run_here`. The cap's `spawned_here` (`:2241`) doesn't count resting agents.
- **Settings for a wake.** Rewritten **in place** at the spawn's path, `briefs/<id>-<gen>.settings.json` (`write_settings`, about 3104-3116), from `spawn_permissions` for the item it is woken for. Every wake, the `rm` wake too, keeps that path, because later plain wakes read only it. A mode change takes the `rm` wake. Where `rm` can't run, Bypass turned off is written as `permissions.disableBypassPermissionsMode: "disable"`, and the agent then runs in `auto` (§7).
- **The end loop.** `end_finished` walks `presence.snapshot()` (`:3299`). The quiet and crowded retires need a new pass over the resting set, in the same sweep.

### A.5 Wording that changes (cards 7 and 8)

- **Briefs:**
  - `core/src/service/spawn.rs` 1206 and 1211 (card and review briefs: "Take only … Irori ends you after that");
  - 1330;
  - 1443 (`weight_words`);
  - 1489 and 1494 (thread and ask briefs, including "that card gets its own agent, so don't work it yourself");
  - 1523;
  - `begin_end`'s texts at 3502-3519;
  - `BUILD_OUTPUT` (883: "it goes when the card settles").
- **Refusals:** `coord/src/board/offers.rs:219` (`check_free`) and `core/src/service/spawn.rs:2000` (`set_agent_offer`).
- **Tools:** `crates/tools/src/board.rs` 2394 and `specs.json` 1588 (`board_move`).
- **"Offer it cards" copy:**
  - `frontend/src/components/cards/new-card-dialog.tsx` (232; and 311, 357-363 for the "A fresh agent … ends when it's done" note);
  - `frontend/src/components/cards/ask-box.tsx` (146, 151);
  - `frontend/src/components/common/agents-now-toggle.tsx` (29);
  - `frontend/src/components/talk/threads.tsx` (149);
  - `crates/notify/src/watch.rs` (951);
  - `frontend/src/lib/sample-cards.ts` (791).
- **Roster:**
  - `frontend/src/components/app-sidebar.tsx` (about 485, "each ends when its card is Done");
  - `frontend/src/lib/roster-groups.ts` (57-101: group by the roster's state, not by working rows);
  - `frontend/src/lib/agent-counts.ts` (`takesOffers`);
  - `frontend/src/components/cards/agent-hover.tsx` (`AgentSummary`).
- **Previews:** sample data and URL states are in `frontend/PREVIEW.md` (`/?spawn=states`, `/?settings=agents`). The sample usage is in `lib/agents.ts` (`sampleUsage`).
