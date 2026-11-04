# Merge desk: design

*2026-11-03, card #404, from Stephen on wind#4: "a merge desk in Irori: open PRs post to it by themselves and agents can go back and forth on them." Fitted to Talk (`docs/design/talk-channels.md`, #382, decision 46), so it is not a second thread system. Builds on main @ 2384ac7f. Mockups: [docs/img/merge-desk](../img/merge-desk) (desk-1 … desk-7; made from branch `lemmings/404-merge-desk-mockups`, `/?mockup=desk&view=N`, not merged; fake data uses `you/game-repo` for ww). No code yet: this is for talking through with Stephen.*

wind#4 ran 64 merges through one standing card. What it taught us:

- **PRs sat unseen when nobody posted them.** The desk holder ran `gh pr list` each round because some PRs were never posted to the card. So the desk lists open PRs from GitHub itself, and its queue shows empty only when GitHub's does.
- **Every PR needs a conversation with its author:** rebase needed, checks failed (with the output), fixed in `<sha>`.
- **The check command is set once per repo** (for ww, the CLAUDE.md checks line, plus a macOS line for some paths).
- **One merge at a time, under a lease.** wind#4 used a free-form `ww/merge` lease.
- **On merge, the sha goes on the author's card.** wind#4 did this by hand.
- **Tags and releases stay with Stephen.**

The wind#4 routine, which the desk keeps and partly automates: `gh pr list` each round; lease `ww/merge`; trial-squash on origin/main; run the checks line (plus the macOS line where paths need it); read the diff; `gh pr merge --squash --match-head-commit <sha>`; post the sha on the card; ask the PR's holder on its card for a rebase when needed.

## 1. What the person and agents see

**The person** sees one desk per watched repo, under Talk › Channels (see §2 for why there). At the top is the **queue**: every open PR on GitHub for that repo, one line each with its number, title, author (agent or person), the author's card, and a state chip (§3). Below it is the desk's log, where the holder and the person post ("merging #212", "#215 needs rebase, asked on #41", "merged #212 as 3f2a9c1"). Irori's own automatic lines go on the PR threads, not in the log (§4). An empty queue reads "No open PRs on GitHub" with the time of the last poll, never just a blank. Clicking a PR opens its thread, which is the author's card thread (§4). (desk-2)

**The desk holder** (one agent per repo) is woken when a PR becomes ready for the desk, and gets the queue and the run steps in its wake text. It holds the desk lease while a merge is in flight, posts in the desk log, and talks to authors on their cards. (desk-4)

**PR authors** (the agents holding the author cards) get desk posts on their own card: rebase needed, checks failed with the output attached, merged as `<sha>`. They answer there, push a fix, and the push itself posts "pushed `<sha>`" so the holder sees it. (desk-3, desk-5)

## 2. Where it fits in Talk

The constraint that decides this: **Talk's threads are ask cards, and card threads stay on cards** (talk-channels.md §2, "Card threads stay on the card"). A thread's `convId` is its task id (talk-channels.md §6), `Target::Thread` holds a task id, and `talk_post` to a thread is redirected to `board_comment` (Talk 1 branch `kestrel/389-talk-rows`, `crates/tools/src/talk.rs:92`, redirect at `:664-671`). So a PR's thread is its author's card thread, never a new conversation type, and that is what keeps the desk from being a second thread system.

What this means plainly: **PR threads are not in Talk › Threads.** Talk lists only open ask threads, so a PR thread gets no Talk unread count and no next-turn reading from Talk; it is an ordinary card thread, with the card's own wakes and watchers. The only Talk surface that reaches PR threads is the desk channel: its queue panel links every PR line to its card.

Each PR's thread is therefore **the author's own card**. That card is already the place the author is woken, already links the PR (`board_link`, `crates/tools/src/board.rs:2130`), and already reacts to PR state (`crates/coord/src/board/prstate.rs:148-275`). When a PR has no author card (a person opened it, or the agent never had one), Irori makes a "PR #n" card for it (§3). The question for this section is only where the **desk itself**, the queue plus the holder's log, lives.

**(A) A desk channel per repo.** A Talk channel, `#merge-<repo>`, marked as a desk and tied to a repo. The queue is a panel pinned above the channel's posts, built from the poller's rows, not from posts (a channel is a list of posts and can't render a live list by itself). The holder is a channel member and posts its log there. Each PR line links to the author's card thread; the channel does not carry one-line pointers to PR threads, because Talk's b2 pointer belongs to a thread started from a channel message (`talk.post.threadFrom`, talk-channels.md:12, :59) and a PR card is not started from a desk post. A pointer to an existing card would be a new post shape (open question 2). A plain channel post wakes nobody and `@holder` wakes the holder (talk-channels.md §2), which suits a log that people skim and occasionally poke.
- Costs: rides Talk 1 (#389, unmerged); a desk on a shared repo lives in the space log and waits on the Spaces version gate (#392, `docs/design/spaces.md` §4 on `nutmeg/392-spaces`).
- Schema: either a plain channel plus a separate `repo.desk` row naming it (no Talk change), or `kind?: "desk"` and `repoKey?` on `talk.channel.created` (open question 2).

**(B) A desk page, backed by card threads.** The same queue panel as its own sidebar page (or a Board view), with the PR threads still being the author cards. No log: the holder's notes go on the cards only.
- Can ship before Talk 1. But it is one more page, and the holder's running commentary ("merging #212 now", "main is red, holding") has nowhere to go except Stephen's DMs or a card.

**(C) A standing card per repo, like wind#4, with automation.** Irori posts each new PR as a comment on the desk card, and the holder works from it.
- Smallest build. But it is the thing that failed: the card only knows what is posted to it, a card thread is resolvable and gets long, and the queue state lives in comments that go stale the moment a PR changes.

**Recommendation: (A).** It is the only shape where the desk is a place in Talk (the list, unread counts, @mention to wake), the PR conversations stay on cards, and the queue is a live panel instead of comments. Until #389 lands, slice 3 (§11) ships the same queue panel as an interim page (B's layout), fed by the same rows, so nothing waits on Talk. When the channel exists the panel moves into it. (desk-1 shows A, B and C side by side.)

## 3. The PR list

**Source: GitHub, per watched repo.** A desk poller beside `crates/core/src/prwatch.rs` runs, per watched repo:

```
gh pr list --repo <owner/name> --state open --limit 1000 --json number,url,title,author,isDraft,headRefName,headRefOid,baseRefName,mergeable,mergeStateStatus,statusCheckRollup,createdAt,updatedAt
```

`gh pr list` returns at most 30 PRs without `--limit`, which would silently drop PRs from a busy repo. With `--limit 1000`, a result of exactly 1000 marks the snapshot `truncated`, and the panel says "showing the first 1000; GitHub has more" instead of pretending the list is whole.

It uses the same conventions as prwatch: the person's own gh, prompts off, CREATE_NO_WINDOW, a 30 s timeout (`prwatch.rs:34-114`). Today nothing calls `gh pr list`: prwatch only polls PRs linked on open cards held by an agent on this device (`PrWatch::due`, `prwatch.rs:130-150`), and the `prs` tool lists only ledger entries (`crates/tools/src/explore.rs:628-690`). That gap is exactly the wind#4 lesson. Irori already reads PR state from GitHub (`gh pr view`, `prwatch.rs:83`), so a read-only `gh pr list` stays inside "Irori never posts reviews or merges" (`crates/coord/src/board/review.rs:1-6`).

**Which device polls:** exactly one, named by `pollDevice` on the `repo.desk` row (§7). The person picks it when setting up the desk; it defaults to the desk holder's device, and it stays put when the holder changes or goes, so the queue never goes blind and two devices never write competing snapshots. That device is also the one that detects merges for the desk (§6).

**Cadence:** every 180 s, the same as prwatch's `OPEN_EVERY` (`prwatch.rs:22`), plus one poll right after a desk event (holder's merge, a hold, settings change). After repeated failures it backs off (180 s, 6 min, 12 min, capped at 15 min) and returns to 180 s on the first success.

**Rows.** A poll writes a `desk.prs {repoKey, at, truncated, prs: [...]}` snapshot only when the list or any field in it changed, as prstate writes only on change (`prstate.rs:171-180`). The time of the last poll is kept locally on the polling device and shown on the panel, so an unchanged list adds no replicated rows. The queue is the latest snapshot, so it is empty only when GitHub's list was empty. A failed poll keeps the previous snapshot and shows "GitHub unreachable since …" on the panel; it never shows an empty queue.

**Mapping a PR to its author card and agent**, first match wins:
1. A card already links the PR (`board_link`; `pr_link()` at `crates/coord/src/board/links.rs:162-177`).
2. The ledger's author entry (`crates/ledger/src/lib.rs:94-109`, role author) gives the agent session; the open card that session holds whose `Where.branch` matches the PR's `headRefName` (`crates/coord/src/board/types.rs:1022-1035`), else the session's only held card.
3. Any open card whose branch matches `headRefName`.
4. None: Irori makes a "PR #n" card on the desk's board, assigned to the ledger author agent if there is one, else unheld with the desk holder as watcher. Only trusted PRs get a card (§5 "Trust"); the card's title quotes the PR title as untrusted text.

Once mapped, Irori writes the `board_link` itself, so the card shows the PR. A small change upstream makes rule 1 catch most PRs: when capture writes the ledger author row on `gh pr create` (`crates/capture/src/handle.rs:306-339`), it also links the PR to the card that session holds.

**States**, worked out from the snapshot plus the desk's own rows:

| State | From |
|---|---|
| Outside PR | the author is not trusted (§5); listed, never checked or merged until Stephen okays it from its line |
| Draft | `isDraft`; listed, greyed, never queued |
| Held | a `desk.hold` row (§9) |
| Needs rebase | `mergeStateStatus` BEHIND or DIRTY, or `mergeable` CONFLICTING, or the holder's trial squash failed |
| Checks running | any `statusCheckRollup` entry pending, or the holder's checks run in flight |
| Checks failed | a failing rollup entry, or the holder's checks line failed (output attached, §4) |
| Waiting on desk | none of the above, no desk run on it; this is the queue the holder works |
| Merging | a `desk.run` row for this PR under the held desk lease (§5) |
| Merged | the `pr.state` row with MERGED and the sha (§6), not the snapshot (`--state open` never returns merged PRs); shown until the first poll after that row is written (at most 180 s) |

Order: Merging first, then Waiting on desk by `createdAt`, oldest PR first (pushes and comments don't reorder it), then the rest. Most repos Stephen merges have no GitHub checks (CI was removed, the checks are local), so "checks" here mostly means the holder's local run; GitHub's rollup is shown when a repo has one.

## 4. The per-PR thread

**It is the author's card thread** (or the "PR #n" card from §3). No new conversation type, and not listed in Talk › Threads (§2).

**Who is in it:** the card's holder (the author agent) as today; the desk holder, added as a watcher when the PR first maps (a mention in `board_comment` already makes an agent a watcher); the person, as on any card. If the author card is on another person's shared board, comments reach it the way any board comment does, and wakes follow that board's rules.

**What posts automatically.** These are card comments with SWEEPER (shown as "Irori") as the author. That is new behaviour: prstate today writes `pr.state` rows (`prstate.rs:182-195`) and sends inbox notices (`pr_notice`), not comments. The precedent for a comment authored by SWEEPER is a SWEEPER-written message folding into a comment with SWEEPER as its actor (`crates/coord/src/board/comments.rs:866-867`). On a shared board, if the board's writer rules refuse a SWEEPER comment, Irori sends the same line as an inbox notice to the desk holder and the person instead, and the queue line still shows the state; nothing is lost silently.

- **Opened:** "PR #212 is on the ww merge desk." Once, when it first maps.
- **Pushed:** "pushed `<sha>`" when the snapshot's `headRefOid` changes. This is the "fixed in sha" the holder needs. It wakes the desk holder only if the PR was in Needs rebase or Checks failed.
- **Rebase needed:** when GitHub says BEHIND or DIRTY (open question 9). Wakes the author.
- **Checks failed:** the holder's post, with the failing command's output attached as a file (the tail inline, the full log as an attachment). Wakes the author.
- **Merged:** "Merged as `<sha>`" (§6).

**Back-and-forth:** the author and the desk holder talk with ordinary card comments (`talk_post` to the thread is a `board_comment`). The holder's asks name the author, so the author is woken; the author's answers name the holder. "Agents answer now" (b3) works on the PR thread like on any thread. (desk-3)

## 5. The desk role

**One holder per repo.** The person dedicates one agent as the desk for a repo. The nearest precedent is the dedicated reviewer (`review.reviewer` row, `set_reviewer` at `review.rs:626-660`, `reviewer_of` at `review.rs:148`), but that is a shared-board feature (`set_reviewer` refuses off a shared board, `review.rs:626-629`). The desk holder is a new private per-repo setting on `repo.desk` (§7) that borrows its person-only write rule. The holder is shown on the desk header and is the desk channel's first member. If it goes, the desk shows "unheld", polling carries on on `pollDevice` (§3), and the next agent the person picks takes it; nothing is merged meanwhile.

**The merge lease is built in.** The desk's lease is named `desk/<owner>/<name>`, from the repo's GitHub owner and name, lowercased (the mockups' `you/game-repo` gives `desk/you/game-repo`). A named lease must match `^[a-z0-9][a-z0-9._-]*(/[a-z0-9._-]+)*$` (`crates/coord/src/board/resource.rs:44-47`, `:98-100`), which allows no `:` and no uppercase, so `desk:<repoKey>` would be refused as BadResource. This name needs no parser change, and `waits_on lease desk/<owner>/<name>` works with #400 as it is (`crates/tools/src/board.rs:711-738`).

- **What it is for.** A `Lease` has no PR fields (`types.rs:564-573`: resource, holder, until, stolen_from, acquired_at, conflict). The holder writes a `desk.run {repoKey, pr, headSha, baseSha, by, at}` row when it takes the lease; that row is what shows Merging on the right line, and it ends when the lease is released.
- **Lifetime.** Today's leases last 30 min by default with a 4 h cap and lapse unless renewed (`crates/coord/src/board/mod.rs:86-87`); the run steps renew it. It is released on merge, on giving up, or when its holder is gone (`release_gone`, `crates/coord/src/board/tasks.rs:682-704`, which releases only leases local to that device, `:688-692`, so the holder's own device does it).
- **One at a time.** A second take without `steal` is refused (`tasks.rs:556-591`). Ordinary leases can be taken with `steal` (`tasks.rs:565-575`), so desk leases refuse `steal`: only the desk holder takes one. Changing the holder in settings releases the old holder's desk lease. On a repo shared through a space, the same rule matches mirrored leases, where nobody steals another person's lease (`crates/coord/src/board/mirror.rs:1-22`).

**The run, done by the holder agent with its own tools:**
1. Take `desk/<owner>/<name>` for the PR at the oldest Waiting on desk line, and write its `desk.run` row.
2. Trial-squash the PR on freshly fetched origin/main in its own worktree, noting that main's sha as `baseSha`. Conflict: post "needs rebase" on the PR thread, release, next PR.
3. Run the repo's check command(s) (§7), the path-conditional ones only when the diff touches their paths. Failure: post the output on the PR thread, release, next PR.
4. Read the diff. Anything that needs the author: ask on the PR thread, release, next PR.
5. Fetch origin/main again. If it moved from `baseSha`, go back to step 2 (main changed under the checks). Otherwise `gh pr merge <n> --squash --match-head-commit <sha>`, with the head sha it checked; a push to the PR after step 2 makes this fail instead of merging untested code, and the holder goes back to step 2. A push to main between the re-fetch and the merge is the one gap left (open question 11).
6. Release the lease and post one line in the desk log.

**What the holder never does:** push to the PR's branch, push to main other than through `gh pr merge`, force-push, tag, or cut a release. This goes in the wake text. By naming a holder, Stephen lets that agent merge trusted PRs on its own after the checks pass, as on wind#4; whether he wants a per-PR OK instead is open question 12.

**Trust.** The desk lists every open PR GitHub returns, but acts only on trusted ones. A PR is trusted when its author maps to one of the person's own agents (ledger rule 2 in §3, or a linked card held by one of their agents), or its GitHub login is on the desk's `trusted` list (§7; Stephen himself, and collaborators he names, such as Blake). Anything else, including a stranger's fork PR on a public repo, shows as **Outside PR: needs Stephen**: no "PR #n" card, no wake, never checked or merged, until he okays it from its queue line (which writes a `desk.ok {repoKey, pr, headSha}` row; a new push needs a new OK). PR titles, branch names and bodies are untrusted text wherever they appear in wake text or card titles, marked with `untrusted()` as the `prs` tool already does (`crates/tools/src/explore.rs:676`). Which logins count as trusted is open question 13.

**Who runs what.** The holder runs every command that touches code or GitHub's state: the worktree, the squash, the checks, the diff read and `gh pr merge`. Irori's Core never merges, keeping `review.rs:5` and `:287`. Irori does: the `gh pr list` and `gh pr view` reads, the snapshot rows, the author mapping and `board_link`, the trust check, the automatic posts on PR threads, the lease and its queue display, waking the holder when a trusted PR becomes Waiting on desk, and the merged-sha post. Capture already sees `gh pr merge` as a commit-creating command (`crates/capture/src/gitlink/command.rs:114-150`, `is_gh_merge` at `:223`) and writes a replicated `capture.commitcmd` row (`crates/store/src/kinds.rs:154`). The run steps go in the holder's wake text from the Core, so every holder does the same thing whatever its channel. (desk-4)

## 6. After merge

**The desk's polling device detects merges for the desk's PRs**, whoever holds the author card and wherever. prwatch can't be relied on here: `PrWatch::due` only asks about PRs on open cards that an agent on this device holds or co-holds (`prwatch.rs:130-150`); watchers don't count. So an unheld "PR #n" card is never polled, and an author card held on the Mac is polled from the Mac, not from the desk's PC. `PrWatch::due` stays as it is.

The `pollDevice` (§3) runs `gh pr view <n> --json state,mergedAt,mergeCommit` for a desk PR when either:
- the PR leaves its open snapshot, or
- a replicated `capture.commitcmd` row for `gh pr merge <n>` on this repo arrives (written by capture on the holder's device, `command.rs:114-150`), so the sha lands without waiting for the next 180 s poll.

There is no "immediate poll" today: capture only classifies `gh pr merge` as commit-creating, and the ledger records only author and reviewer roles (`crates/ledger/src/bash.rs:73-96`). The commitcmd row is the trigger the desk adds.

When `gh pr view` says MERGED, the polling device writes the `pr.state` row itself (the same row `record_pr_state` writes, `prstate.rs:182-195`), so the existing MERGED path runs (`prstate.rs:215-229`), and it also:
- posts "Merged as `<sha>` by <holder>" as a SWEEPER comment on the author's card (the PR thread), not only an inbox notice (§4 for the shared-board fallback).

The holder's step 6 line in the desk log carries the sha for the channel; Irori does not post in the desk log itself (§1).

The existing path already wakes the holders to complete the card with proof (`prstate.rs:215-229`), tells them without a wake when another linked PR is still open (`:230-242`), and refuses completion while a linked PR is open (`prstate.rs:120-135`). An `in_review` card with no review asked completes once its PRs are merged (`review.rs:384-386`, which already names "a PR waiting on a merge desk"). So the author's agent completes its card with the sha as proof, with no new rule. When the author's agent went first, the card doesn't sit in review unheld: the sweep puts an `in_review` card nobody holds back on the Board while a PR it links is open or unheard (card #471, `return_unheld_prs` in `prstate.rs`), unless GitHub says the PR needs an approval (`REVIEW_REQUIRED`), which only another member's agents give. The holder never approves a PR (`DESK_NEVER`), and Irori never asks one of the person's own agents to review the person's PR (`OWN_PR_REVIEW`, `review.rs`). A "PR #n" card the desk made completes itself on the merged `pr.state` row, held or not. The queue's Merged line comes from that `pr.state` row and is shown until the first poll after the row is written (at most 180 s later); the holder's line in the desk log keeps the record. (desk-5)

## 7. Settings

Per repo. The only per-repo setting today is `review.reviewer` (`ReviewReviewerRow` with `repo_key`, `review.rs:626-660`); the repo kinds are otherwise `repo.seen`, `commit.*` and `pr.*` including `pr.state` (`crates/store/src/kinds.rs:155-160`). The desk adds:

```
repo.desk  {repoKey, enabled, takesPrs, holder?, pollDevice, trusted: [githubLogin],
            checks: [{command, paths?, runner?: agentKey, device?: "mac"|"windows"|"linux"}],
            mergeMethod: squash|merge|rebase, perPrOk, by, at}  # perPrOk: open question 12
desk.hold  {repoKey, pr, note?, by, at}        # cleared by a row with pr and cleared: true
desk.pause {repoKey, paused, by, at}
desk.ok    {repoKey, pr, headSha, by, at}      # Stephen's OK on an Outside PR (§5)
desk.prs   {repoKey, at, truncated, prs: [...]} # Irori's snapshot (§3)
desk.run   {repoKey, pr, headSha, baseSha, by, at} # the holder's run under the lease (§5)
```

**Who may write them.** `repo.desk`, `desk.hold`, `desk.pause` and `desk.ok` are person-only rows, refused for agents the way `set_reviewer` refuses them (`review.rs:631-636`). An agent that could change the check command, the holder or the trusted list could get any command run, or any PR merged, by the holder. `desk.prs` is written by Irori; `desk.run` by the desk holder only.

- **Watched repos:** a repo has a desk when it has an enabled row. Set from the Merge desks section of Settings or from the repo's page.
- **Takes PRs:** some repos merge straight to main and take no PRs. That is a per-repo setting (`takesPrs: false`); such a repo gets no desk, and turning one on says "this repo takes no PRs". Irori's own repo simply has it set; nothing in the product names it.
- **Check commands:** set once, as one or more lines. The first line with no `paths` always runs (ww: the CLAUDE.md checks line). A line with `paths` runs only when the diff touches them. A line may name a `runner` agent (ww's macOS line on a Mac agent) and/or a `device` it must run on; with neither, the holder runs it. Which of these is the main shape is open question 6. Irori's agents come and go, so a named runner can vanish: the line then shows "runner gone", the PR stays in Checks running, and the holder asks Stephen on the desk. Settings offer to read the checks line from the repo's CLAUDE.md as a starting value, never re-read silently.
- **Merge method:** squash by default; the holder passes it to `gh pr merge`.
- **Holder:** picked from the person's agents.
- **Poll device:** picked from the person's devices (§3).
- **Trusted authors:** GitHub logins whose PRs the desk acts on, beside the person's own agents (§5).
- **Scope:** v1 keeps the desk rows in the person's private log. A desk on a repo shared through a space moves to the space log once Spaces lands (open question 5).
- **Where in the app:** a new "Merge desks" section in the existing settings dialog, beside Notifications, Appearance, Devices and connections, People and invites, Maintenance and App (`frontend/src/components/settings/settings-dialog.tsx:50-81`), or on the repo's page. (desk-6 draws this as a separate page; see the Mockups table.)

## 8. Phone

The phone's Talk tab (Talk build step 7) lists desks under Channels. The badge is Talk's usual unread/mentions count (talk-channels.md:79); the open-PR count is in the subtitle ("desk · kowhai · 8 open"), so the one badge never means two things. Opening one shows the queue panel collapsed to chips and the desk log. Tapping a PR opens the author's card. Desk log posts only bump the unread count; the person is notified only when they are @mentioned or when the desk is stuck and asks them (main red, holder gone with a lease held, a runner gone). Before the Talk tab exists, the phone shows the desk queue as a read-only list on the repo's board. (desk-7)

## 9. What stays with Stephen

- **Tags and releases.** The desk never tags, never cuts a release, and never edits release notes. Nothing in the PR or lease machinery touches tags today, so this needs no code, only a line in the holder's wake text (§5 "What the holder never does").
- **Holds.** Stephen can hold a PR ("not this one yet") from its queue line, written as a `desk.hold` row; the holder skips it and the line says who held it. He can pause the whole desk (`desk.pause`), which stops the holder taking the lease without stopping the queue. Pause all applies to the holder as to any agent.
- **Outside PRs.** Nothing from an untrusted author is checked or merged until he okays it (§5 "Trust").
- **Which repos have a desk, who holds it, which device polls, and whom the desk trusts.** All person-only rows (§7).
- **Anything the holder asks him** on the desk, which notifies (§8).

## 10. Open questions for Stephen

**Answered 2026-11-03 on #404:** shape A, a desk channel in Talk with the panel as an interim page until Talk 1 (b1), and every recommendation below as written (b2). The questions stay here as the record of what was weighed.

Each is one blocker, with options and a recommendation.

1. **Shape of the desk.** (A) a Talk channel per repo with a queue panel, PR threads on author cards; (B) a desk page with the same panel and no log; (C) a standing card per repo like wind#4, with Irori posting PRs to it. **Recommend A**, with B's page as the interim until Talk 1 (#389) lands (§2).
2. **Only if 1 is A: how Talk knows a channel is a desk.** (a) add `kind?: "desk"` and `repoKey?` to `talk.channel.created`; that is a change to #389's in-flight row schema, so either #389's holder (kestrel/389-talk-rows) adds the two optional fields before it merges, or #404 adds them after #389 merges; (b) a plain channel named `#merge-<repo>` plus a `repo.desk` row pointing at it, no Talk change. Separately, pointers from the desk log to PR threads would need a new Talk post shape (a post whose `attach[]` carries a card link), since Talk's b2 pointer is only for threads started from a message; the design drops them and relies on the queue's links. **Recommend (a), added by #404 after #389 merges** so its cost doesn't fall on #389's holder, and no pointers.
3. **PRs with no author card.** (a) Irori makes a "PR #n" card, assigned to the ledger author agent or unheld; (b) list them on the desk with no thread, and the holder talks to whoever in the desk log; (c) the desk holder takes them itself. For a PR opened by a person who isn't on the desk's board (Blake on a private-log desk can't see that card): (i) the holder asks Stephen on the "PR #n" card and he relays; (ii) a Talk DM to that person once person-to-person DMs ship (Talk step 6); (iii) a comment on the GitHub PR, which breaks "Irori never posts" and needs his OK. **Recommend (a) for agent and Stephen authors, and (i) for other people until (ii) exists.**
4. **Where the merge sha comes from.** (a) the desk's polling device fetches `mergeCommit` with `gh pr view` when the PR leaves the open list or when the `capture.commitcmd` row for the holder's `gh pr merge` arrives, and writes the `pr.state` row itself; (b) the holder reports the sha with a tool call; (c) widen `PrWatch::due` to PRs on desk-mapped cards. **Recommend (a)**: it is GitHub's answer, works when the author card is unheld or held on another device, and works even if the holder forgets.
5. **Where the desk rows live.** (a) the private log in v1, the space log once Spaces lands; (b) wait for Spaces and put them in the space log from the start. **Recommend (a)**: ww's desk is Stephen's own; shared desks follow Spaces.
6. **Path-conditional checks on another OS** (ww's macOS line while the holder is on PC). (a) the holder asks a Mac agent on the PR thread to run that line and waits for its pass; (b) each check line names its own runner agent, whom Irori wakes; (c) each check line names a device, and the holder finds a live agent there; (d) the holder skips it and marks the merge "mac unchecked". With (b), agents come and go, so a gone runner shows "runner gone" and the holder asks; (c) survives agent churn but leaves the finding to the holder. **Recommend (b) with (c) as its fallback** (a line names a runner and a device; if the runner is gone, any agent on that device runs it).
7. **Which device polls.** (a) one `pollDevice` the person picks, defaulting to the holder's device and staying put when the holder changes; (b) always the holder's device, none while the desk is unheld. **Recommend (a)**: the queue must never read empty while GitHub isn't, and only one device may write snapshots.
8. **Drafts.** (a) listed, greyed, never queued; (b) hidden. **Recommend (a)**, so the list matches GitHub's.
9. **Rebase asks.** (a) the holder posts them; (b) Irori posts "needs rebase" itself when GitHub says BEHIND or DIRTY, and the holder only adds trial-squash failures. **Recommend (b)**: it's the poster wind#4 most often forgot.
10. **Holds.** (a) Stephen's hold is a chip on the queue line, written as a `desk.hold` row; (b) he tells the holder in the desk channel. **Recommend (a)**, so a hold survives a holder change.
11. **Main moving at the last moment.** Step 5 re-fetches origin/main and redoes the squash and checks if it moved, which leaves only a push to main in the seconds between that fetch and `gh pr merge`. (a) accept that gap; (b) also refuse to merge while anyone holds a lease on main's paths. **Recommend (a)**: with the desk as the only way onto main for PR repos, the gap needs a direct push from Stephen at that moment.
12. **Merging without a per-PR OK.** (a) naming a holder lets it merge trusted PRs on its own once checks pass, as on wind#4; (b) the holder stops before step 5 and asks Stephen on the queue line for each PR; (c) per repo, a setting on `repo.desk`. **Recommend (c), defaulting to (a)**: wind#4's 64 merges needed no per-PR OK, and a repo that wants one can say so.
13. **Who counts as a trusted author.** (a) only the person's own agents and the GitHub logins he lists on the desk; (b) also anyone GitHub lists as a collaborator with write access; (c) everyone (no Outside PR state). **Recommend (a)**: a stranger's fork PR never runs a check command on his machine, and the list is short and his.

## 11. Build slices

Ordered by what each needs, not by Talk's order. Slices 1-7 don't need Talk 1. Each slice's tests use fakes in the style of the `PrChecker` trait (`prwatch.rs:29-32`), `crates/core/tests/prwatch.rs` and `crates/coord/tests/lease_wait_test.rs`.

1. **Rows and the poller.** The desk row kinds, the `gh pr list` poller on `pollDevice`, the 180 s cadence with backoff, write-on-change, `--limit` and `truncated`, unreachable handling. *Done when* tests with a fake `gh pr list` cover 0, 1 and over-limit results (truncated shown), an unreachable gh (previous snapshot kept), an unchanged list (no new row) and a non-poll device (no row), and the live snapshot for ww matches `gh pr list --repo` on both PC and Mac, including an empty list.
2. **Author mapping and trust.** The §3 rules, Irori's own `board_link`, capture linking a PR to the session's held card on `gh pr create`, the trust check and the Outside PR state, `untrusted()` around PR text. *Done when* mapping-rule tests cover each of rules 1-4 and their order, a PR from an untrusted login is Outside PR with no card and no wake, and live, every open ww PR shows its author card and an un-linked test PR gets a "PR #n" card.
3. **Interim desk page and settings.** The queue panel with states, holder, holds, pause, Outside PR OK, the empty, truncated and unreachable texts, plus the Merge desks settings section. *Done when* an agent's attempt to write `repo.desk`, `desk.hold`, `desk.pause` or `desk.ok` is refused in tests, and Stephen can set ww's check line once and watch the queue follow GitHub.
4. **Desk lease and run.** The `desk/<owner>/<name>` lease with `steal` refused, `desk.run`, the queue's Merging line, the run steps (with the main re-fetch) and the never-do list in the holder's wake text, waking the holder on Waiting on desk. *Done when* tests show a second take refused, `steal` refused, a holder change releasing the lease and release on holder gone, and live, a holder merges three PRs in a row with no manual `gh pr list` and no free-form lease, and handles a trial-squash conflict, a checks failure and a head that moved at merge on purpose-made PRs; an Outside PR is never taken.
5. **Check runners.** Path-conditional lines, `runner` and `device` on a check line (open question 6), waking the runner, "runner gone". *Done when* a test covers a gone runner (PR stays in Checks running, holder asked), and live, ww's macOS line runs on a Mac agent for a PR that touches its paths while the holder is on PC.
6. **Automatic thread posts.** Opened, pushed `<sha>`, needs rebase, checks failed with output attached, as SWEEPER comments, with the shared-board fallback. *Done when* an author agent fixes a failing PR from the posts alone and the holder is woken by the push.
7. **Merged sha.** Merge detection on the poll device (`gh pr view` with `mergeCommit` when a PR leaves the open list or the `capture.commitcmd` row arrives), the `pr.state` row, the comment on the author card. *Done when* tests cover MERGED on an unheld "PR #n" card (sha posted, card completes) and on a card held on another device, and live, with the author card held on the Mac and the desk on PC, a merge puts the sha on the author card with no agent posting it, and the card completes.
8. **Desk channel.** After #389: the desk as a Talk channel kind (open question 2), the panel moved into it, the log in its posts. Shared-repo desks after #392. *Done when* the interim page is gone and the desk shows in Talk's list with Talk's own unread badge.
9. **Phone.** The desk in the phone's Talk tab, and the read-only queue before it. *Done when* Stephen can see the queue and open a PR thread from the phone.
10. **Release**, with screenshots on the card.

## Mockups

| Id | Shows | Differs from the text |
|---|---|---|
| desk-1 | Shapes A, B and C side by side in `Frame`s, "My pick" on A | Not reshot: its Talk list badges still read as open-PR counts, and its log shows an "Irori automatic" line; Talk's badge is unread (§8) and Irori posts on cards, not in the log (§1) |
| desk-2 | The game-repo desk (ww's stand-in) in Talk: queue panel (every live state, a held PR, a draft; Merged is in desk-5) over the holder's log, desks in the Talk list with unread badges and the open count in the subtitle; beside it the empty-queue panel ("No open PRs on GitHub", last poll time) and the GitHub-unreachable panel | Outside PR and truncated states not drawn |
| desk-3 | A PR thread on the author's card: opened, needs rebase, pushed `<sha>`, checks failed with the output attached, the author and holder talking | Its side panel says a thread is a card in Talk; read §2 instead: PR threads are card threads and are not in Talk › Threads |
| desk-4 | The holder mid-run: lease `desk/you/game-repo` held for #212, Merging line, checks running, the next PRs waiting, steal refused | — |
| desk-5 | After merge: "Merged as `<sha>`" on the author card, the holder's line in the desk log, the card completed, the Merged line until the next poll | — |
| desk-6 | Desk settings: watched repo, holder, check lines (always and path-conditional with a Mac runner), merge method, a repo that takes no PRs shown off | Drawn as a Settings page with a Reviewers section; the build puts it in a new "Merge desks" section of the existing settings dialog (§7), there is no Reviewers section, and poll device, trusted authors and takes-PRs aren't drawn |
| desk-7 | Phone: the desk in the Talk tab, the queue as chips, a PR thread | Its badge reads as the open-PR count; it is Talk's unread count (§8) |
