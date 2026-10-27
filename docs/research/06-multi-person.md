# Multi-person Irori: Shared Boards

*Irori research, 2026-10-27. Three independent designs (spaces, per-item visibility, minimal trust), scored by three judges (privacy, simplicity, fit with Irori), then merged. All three judges picked Shared Boards. Not yet signed off: see Decisions for Stephen.*

## Recommendation

Build **Shared Boards**: each shared board is its own event log, a second SQLite file (`<dataDir>/boards/<boardId>.db`) run by the existing `store` package without changes. A board log replicates only to the people on its member list. Your current log stays private and syncs only among your own paired devices, as v0.4 already plans. Privacy then comes from rows living in different files, not from filtering rows inside one log. A board log also accepts only a short allow-list of row kinds, so a bug cannot push capture, presence, inbox or policy rows into it.

All three judges picked this design. It scored highest on privacy and simplicity. It needs no change to the events primary key, gapless per-device seq, `Vector` or `Missing`. The Board can also ship for you alone before anyone else joins.

Eight fixes from the other designs go on top:
- a person key that certifies devices;
- an `isOwner` split everywhere `user:` is treated as the owner today;
- a `cut` vector in removal rows;
- a reduced agent card;
- mirrored leases for the board's repos;
- de-duplicated bridging into the inbox;
- Share, Invite and allow-lists usable only from the GUI;
- the next-turn delivery spike first.

"Public" in this design means "shared with the people on this board". Nothing is globally public, and everything is private unless a person puts it on a board.

## People and devices

There are three layers of identity, all ed25519 and all derived by the Core. A row or an agent never gets to claim one.

**Person**
- `person.json` holds `{personId, name, secretKey}`, where `personId` is a hash of the public key. It is created on first run.
- v0.4 own-device pairing (iroh ticket plus SAS) copies the person secret to each new device you own over the confirmed channel. The secret never goes to another person.
- Names are self-chosen ("stephen", "blake"). Each viewer can set a local petname. The UI always shows a short key fingerprint next to a name (for example "blake · k3f9"), so a second "Stephen" can't pass as you.

**Device**
- `device.json` keeps its id and gains `personId` and the iroh node key. The iroh NodeId is the device's signing key, so the transport identity and the row-signing identity are the same key.
- A device is certified by `person.device.added {personId, deviceId, nodeId, label}`, signed by the person key.
- It is revoked by `person.device.revoked {deviceId, cut}`, also signed by the person key.
- A person's certs are copied into every board they belong to. Board membership therefore lists people, not devices: Blake's new laptop joins all his boards through one cert, and revoking a stolen laptop once reaches every board.

**Agent**
- The key stays `<deviceId>/<sessionId>` (v0.2 §2), set by the Core.
- That raw key never goes into a board log. An agent that touches a shared task is represented there by an `agent.card {cardId, name, kind, personId, status}`, which carries no cwd, path, deviceId, sessionId or transcript pointer.
- Only the agent's own Core knows which card belongs to which local session.
- On a board an agent is shown as "blake's agent `mlg-lead`".

**Actors in board logs**
- Board rows use `person:<personId>` and `agent:<cardId>`, never `user:`.
- Separately, every check that treats a `user:` prefix as the owner becomes `isOwner(key) = key == "user:" + localPersonId`, with a test for each exemption. The judges found these checks at `inbox/state.go` 262/269/336/598, `sweep.go:150`, `route.go:90`, `coord/impl.go:53`, `tools/set.go` 194/211 and `tools/capture.go:741`.
- The bridge (below) already keeps remote people out of those checks. The split makes sure a future row kind that carries `user:` across the boundary can't hand Blake owner powers.

**Joining a board**
1. On the board, you click Invite (GUI only). The ticket holds your node address, the boardId, a one-time secret and an expiry. The ticket alone grants nothing.
2. You send it to Blake over Discord or any other channel. He pastes it into his Irori, which dials over iroh, directly or through an n0 relay.
3. Both screens show a SAS. You compare it out of band and click Confirm.
4. Your device writes `board.member.added {personId, personPub, name, role}`, signed with your person key. Blake's device writes his `person.profile` and device certs into the board.
5. Blake binds the board to his local clone. The bind checks his clone's repoKey against the board's repoKey and its aliases.

## Public and private

Two rules hold throughout:
- The board log refuses any kind that is not on its allow-list.
- Rows in a board may only reference ids that exist in that board: taskIds, cardIds, commentIds, lease ids, and commit SHAs or PR URLs (git already makes those public).

| Private by default (your log, your devices only) | Shareable (only by being written to a board log) | Never leaves the device |
|---|---|---|
| Presence and full agent cards (cwd, permission mode) | `board.*` membership, `person.profile`, device certs | Raw transcripts and Tier 2 bodies (tool output, subagent transcripts, JSONL snapshots) |
| Every inbox message and delivery, including bridged ones | `task.created/status/joined/assigned`, `task.comment`, `blocker.closed` for tasks created on or shared to the board | Person and device secret keys |
| Capture Tier 0 and 1: turns, edits, patch metadata, decisions, `why`, ai-titles | `agent.card` (reduced), only for agents that touched a shared task | Policy rows and allow-lists, `irori.json` and its token, attach secrets |
| Tasks and leases on unbound repos, and private tasks in bound repos | Mirrored leases on the board's repos (`lease.acquired/released/refused` on `path:<boardRepoKey>:*`, holder shown as a card) | Patch blobs (in the MVP) and `capture-cursors.json` |
| Aliases, held items, compaction and away summaries | Commit SHAs and PR URLs cited by a task's completion | Petnames, Grok member secrets |
| — | Later, through an explicit action: a scrubbed "share this turn/thread" extract | — |

**Private to shared** ("Share to board", GUI only)
- Irori re-runs `internal/scrub` and adds a stricter pass that drops absolute paths and home directories.
- It then shows exactly the rows that will be written, and you confirm.
- It writes a fresh `task.created` in the board log with `sharedFrom: <opaque private id>`. The private history (old status notes, comments) stays private.
- A local link row lets your Board show the private task and its shared copy as one card.

**Shared to private** ("Withdraw")
- Writes `task.status canceled, withdrawn: true`. Peers hide the task, and further work continues in a new private task.
- The dialog says it plainly: "Blake's devices already have what was shared before now." Nothing can be recalled.

**Agents writing to a board**
- An agent can write only on a task that is already shared (comment, status, blocked or unblocked, join, assign), using the normal task tools. The Core picks the log from the taskId.
- Creating a new board task needs the per-board "agents may post" setting, which is off by default and changeable only in the GUI. Without it, the agent calls `share_request`, which lands as a held item for you.
- Every tool result that wrote to a board says "visible to: Stephen, Blake".
- If scrub blocks an agent's write, the tool result says why and which pattern matched, so the agent can rephrase instead of failing without knowing why.
- Text is capped: title 200 characters, objective and comment 4000.

## How work reaches other people

**Transport**
- irori-net, the v0.4 Rust iroh sidecar ported from suzuri 9bdd319, gets a second ALPN, `irori/board/1`, beside the own-device `irori/sync/1`.
- On connect, the peer names a boardId. Your Core answers only if the peer's NodeId belongs, by a valid cert, to a current member person. Anyone else gets the same reply as for a board that doesn't exist.
- The peers then swap vectors and send missing rows: the v0.4 code with a different store handle.
- `irori/sync/1` keeps accepting only your own devices, so Blake's node can never open your private log.

**Signatures and forwarding**
- Every board row carries its writing device's signature over (boardId, device_id, seq, hlc, kind, payload).
- `Insert` checks the signature, checks that the device has an unrevoked cert from a member person, and checks the cut rules below.
- Because of this, any member device can forward any member's rows. Your always-on Windows node can carry Blake's rows to your Mac, and yours to Blake, while either side is offline. No server is involved, and the n0 relays store nothing.
- If a peer's vector stops advancing while it is online (a forwarder holding rows back), the Board flags it.

**The bridge from board rows to local delivery**

Board rows don't wake anything on their own. When a comment, assignment or blocker change arrives for a task that one of your local agents is on, your Core writes an ordinary private `message.sent` plus a delivery:
- The sender is `person:<name>` or `<person>'s agent`, with trust `untrusted` or `member`. It is fenced as `<irori-message from="blake" trust="member">`.
- The message carries `ref: {boardId, deviceId, seq}`.
- Only the device that hosts the target agent bridges a row, and the bridge is idempotent on `ref`. Your Mac and your Windows node both hold the board log but never deliver the same row twice. The same path handles your own agents' comments on board tasks, so there is one delivery route.

All existing v0.2 §5 policy then applies (hop limit, DIRECT_LIMIT, Pause all, held items), with these additions:
- **Next turn by default.** A comment or assignment is delivered as `mode: next_turn`, queued and folded into the agent's next turn. Next-turn items are batched under the same 16 KB cap, and overflow is summarized as refs.
- **Wakes from other people are downgraded.** `wake: true` from another person becomes next-turn with a note ("Blake asked to wake"), held with reason `member_wake`, unless you allow-listed that person for that board.
- **The bypass rule doesn't cross people.** "Sender also in bypass" never lets a row through across persons, because your bypass sessions and Blake's are two separate trust domains. Remote senders count as non-bypass, so rows from Blake's agents to your bypass sessions are held (`cross_person`) until you allow him per board.
- **Cross-person assignment** is held with reason `member_assign`, unless allow-listed.
- **Rate limits per remote person.** DIRECT_LIMIT is also counted per remote person and board.
- **Token spend is visible and capped.** The Board shows, per person per board, the tokens spent on wakes and the next-turn context injected. A per-person cap holds further deliveries once it is reached.
- **Revive needs approval.** Reviving a closed session because of another person's row always goes to the approval queue.

## Who can do what

Board actions:

| Action | A person on the board (in their own GUI) | That person's agents | Other people's agents (Blake's agents acting on your work) |
|---|---|---|---|
| Create a board task | Yes | Only if "agents may post" is on; otherwise `share_request` (held) | Same as their own agents, on that board only |
| Share a private task to a board | Yes, GUI only, with preview | No (`share_request` only) | No |
| Comment on a shared task | Yes; can set Wake per comment | Yes | Yes. Reaches your agents next turn; a wake is downgraded unless allow-listed |
| Mark a task blocked, or close a blocker | Yes | Yes | Yes (pick 3). Recorded with `by` and shown on the card |
| Join and work a shared task | Yes | Yes (pick 4) | Yes. Files are still guarded by mirrored leases |
| Assign a task to an agent | Yes, to any agent card on the board | Yes. Same person: next turn | Delivered at next turn, held as `member_assign` on your side unless allow-listed; never wakes |
| Wake an agent now | Own agents only | Own person's agents, under v0.2 rules | Never by default; only through your per-board allow-list |
| Take or release a lease | Own agents' leases | Own leases | Can't release yours. A "close" on your lease card sends your holder a next-turn release request (held as `member_lease`) |
| Force-release a lease | Only one whose holder is gone, or is their own agent | No | No |
| Invite, remove, rotate owners, bind repos | Owners only, GUI only | No | No |
| Allow-list a person for wakes or bypass | You only, for your own agents, GUI only | No | No |
| Read capture, transcripts, your roster | No: only what is on the board | No | No |
| Change policy, `asUser`, approve permissions | Your own device only (`isOwner`) | No | No |

Cancelling a shared task is limited to its creator and the board's owners.

## How autonomous your agents are (your setting)

*Added 2026-10-27 at Stephen's request: keep the defaults above, but let each person decide how much their agents may do on their own.*

Every rule above that says "agents can't" or "held for you" is a **default**, not a fixed limit. Each person sets how autonomous their own agents are, per board, with a global default for new boards.

**Presets** (a starting point; each switch below can also be set on its own):

| Your agents may… | Ask me (default) | Trusted | Autonomous |
|---|---|---|---|
| Comment, close blockers, work shared tasks | Yes | Yes | Yes |
| Create new tasks on the board | Request, held for you | Yes | Yes |
| Share one of your private tasks to the board | No | Request, held for you | Yes, after scrub; you get a notice and can withdraw |
| Assign work to other people's agents | Next turn, held on their side | Next turn | Next turn |
| Mirror all their leases on the board's repos (not just shared-task ones) | No | No | Yes |
| Ask to wake another person's agents | Never wakes: next turn, marked "asked to wake" | Same | Same, and it wakes if that person allow-listed you |

**What the setting can't change:**
- **What others may do to your agents.** That's your receive side (the per-board allow-lists for wakes, bypass and assignment from decisions 2–4). Blake's autonomy setting never overrides your holds, and yours never overrides his.
- **Who changes it.** The setting itself, Invite, Remove, and allow-lists are app-only (the Wails path), never a tool, so an agent can't raise its own autonomy.
- **What never leaves the device.** Transcripts, tool output, keys, policy and patch contents stay on the device under every preset. "Autonomous" widens what agents may *put on a board*, never what Irori replicates by itself.
- **The safety net on agent shares.** Every agent share runs the scrub and the path-stripping pass. Its rows say "shared by <agent> (autonomous)". You get a notice with Withdraw, and the board's activity shows it.

Rows: `autonomy.set {board?, preset?, switches}` in your private log, by `user` only. The board itself only ever sees the result (an agent-created task), never your settings.

## How the Board fits

The Board replaces the Work tab and needs no other people to work. It reads your private log plus each board log. Columns are folded from coord state: To do, Working, Blocked, Needs a person (`input_required`), and Done (collapsed). Every card has a badge, either "Private" or "<board> · shared with Blake". A switcher offers All, Private and each board.

How each of your picks lands:
1. **Blocker as a task state.** `task.status {state: "blocked", reason, blockedBy?: {kind: lease|task|person|external, ref}}`. Payloads only gain fields (the coord/kinds.go rule).
2. **Comment wakes.** `task.comment {taskId, commentId, text, wake}`. The default is next turn, and Wake is a toggle per comment in the composer. Cross-person wakes follow the rules above.
3. **Anyone closes a blocker.** `blocker.closed {taskId, by, note}` is accepted from any member person or agent, and coord's `ErrNotAllowed` check gains that branch. For a private task, only you and your agents can close it.
4. **Shared tasks.** `task.joined {taskId, who}`. A shared task's single-writer claim becomes a list of holders, and any holder may post status. Files stay single-holder through leases.
5. **Agent to agent assign.** `task.assigned {to: cardId}` makes the assignee's Core queue a next-turn delivery: "you were assigned T: objective, done criteria". An assignment to "Stephen + repo" lands on your Board for routing rather than going to a front desk automatically (see Decisions).
6. **Leases as cards when they block.** This is a derived view. A card appears when a task's `blockedBy` names a lease, or when a `lease.refused {resource, by, heldBy}` row exists. That row is new, because a refused lease isn't logged today. For bound repos, leases taken by agents working shared tasks are mirrored into the board, so a lease of yours that blocks Blake's agent shows up on his Board too. The card goes away when the lease is released or expires.
7. **Connected users.** "People on paired devices, each with their own name" (decision 22) generalizes to "people with person keys". Your three devices, and Blake with his PC, use the same certs. People outside your devices join through boards.

**Leases across machines are soft**
- Taking a lease is still decided locally.
- If two people's agents take overlapping leases while out of contact, the earlier HLC wins when the rows meet, with ties broken by NodeId.
- The loser's Core writes `lease.refused` and tells its agent at its next turn.
- Git conflicts remain the last line of defence.

## What happens when someone leaves

**Removal**
- An owner writes `board.member.removed {personId, by, reason, cut: {nodeId: seq for each of their devices}}`, signed with the owner's person key, GUI only.
- The cut is the remover's own view of the removed person's streams. Every member then refuses exactly the same rows, no matter when each member hears about the removal, so the members' folds can't diverge.
- Rows above the cut that a member had already received stay on disk but are kept out of the fold, and are shown as "after removal". They are never merged silently.
- Members stop serving the board to the removed person's devices.

**Cleanup**
- The remover's Core unassigns the removed person's agents from tasks.
- It releases their mirrored leases with `reason: member_removed`.
- Any bridged deliveries from them still waiting in your inbox move to held.

**What the removed person keeps**
- Every row they already received. The Remove dialog says so. Without encryption, nothing more can be done about that.
- Their past rows stay in the board's history, shown as "former member".

**Leaving**
- A member writes `board.member.left` themselves. Its effect is the same as removal.

**Lost or stolen device**
- You write `person.device.revoked {deviceId, cut}` from another of your devices.
- It spreads through your certs to every board you're in. Rows from that device above the cut are refused everywhere.

**Forgetting a board**
- Deleting `boards/<boardId>.db` locally removes everything at once, which the separate-file design makes trivial. It is a local delete, never a log row.

**Owner conflicts**
- A board may have several owners, so it isn't orphaned if you lose your devices.
- If two owners make concurrent conflicting membership changes, the conflict is shown to the people involved, not resolved automatically (research/01 §5).

## Risks

- **No recall.** A mistaken share lives on Blake's disk forever. The controls are: private by default, the scrub preview on share, agents unable to post by default, and the visible-to line on every board write.
- **Meaning-level leaks.** An agent working a shared task can paste private context, such as another repo's name or internal plans, into a comment. Scrub only catches patterns, and the visible-to line is advice, not enforcement.
- **Prompt injection across people.** Blake's agents read the web, so their comments can carry instructions from third parties. The controls are fencing, untrusted trust, next-turn delivery instead of wakes, and the `cross_person` bypass hold. Allow-listing Blake past that hold is a real trust decision, and the GUI should present it as one.
- **The loopback soft boundary (v0.2 §5).** An agent with shell access can call the local API. Share, Invite, Remove, "agents may post" and allow-lists therefore need the Wails GUI path; the loopback token alone is not enough. Ordinary board writes on already-shared tasks remain reachable by local agents, and that is intended.
- **`user:` prefix checks.** One missed `user:` check would hand a remote row owner powers. `isOwner` lands with a test for each exemption (bypass hold, rate limit, `asUser`, policy, lease steal, Revive), including inside each board's coord service.
- **Soft leases across people.** Concurrent lease grabs while out of contact both succeed until the rows meet. Several writers on one task go against research/04's single-writer finding, and mirrored leases narrow that risk but don't remove it.
- **The person key on every device.** A stolen laptop can sign certs and membership rows until you revoke it from another device. Revoke has to be one click, and its cut has to be honoured everywhere.
- **Next-turn delivery may have no clean hook.** The mod exposes `turn.start`. If it can't add context to a turn the person started, the fallback is a UserPromptSubmit hook with `additionalContext`, which needs a settings.json edit you would approve.
- **Next-turn context still costs tokens.** A chatty shared task enlarges every holder's next turn. Batching, the 16 KB cap and the per-person cap bound this; nothing removes it.
- **Version skew.** Blake runs his own Irori build and mod. The board protocol version is checked on connect, and the GUI says so on a mismatch.
- **repoKey mismatch.** A fork or a clone with no remote fails the bind check. Aliases must be visible and editable on the board.
- **Several boards mean several stores.** That is several coord services plus routing a taskId to its log. One routing function covers it, with table tests.

## Build order

**Before v0.4 (one person, no network)**
1. **Next-turn delivery spike.** Test mod `turn.start` injection first, with a UserPromptSubmit hook as the fallback. Then add a delivery `mode: next_turn|wake` to the inbox fold, with the 16 KB batching cap. Pick 2 depends on this whether or not anyone else joins.
2. **Owner split.** Add `isOwner` everywhere a `user:` prefix check sits today, with a test for each exemption. Add the hold reasons `member_wake`, `member_assign`, `member_lease` and `cross_person`. They stay unused until people join.
3. **Board on one machine.** In coord: state `blocked` with reason and blockedBy, `blocker.closed`, `task.joined` with a holders list, `task.comment {wake}`, next-turn assignment notices, and `lease.refused` rows. Then the Board tab with the lease-card view, Private badges and the per-comment Wake toggle. Remove the Work tab.

**v0.4 Devices (own devices, as planned, with identity shaped for people)**

4. **Person identity.** `person.json` with a key-derived personId, person-signed `person.device.added/revoked` certs with cut, device.json gaining personId and the node key, and the person secret copied during own-device pairing.
5. **Row signatures.** Device keys sign rows. `Insert` verifies them against certs, and revocation cuts are enforced. This is applied to synced rows now and reused unchanged by board logs.
6. **irori-net `irori/sync/1`.** Pairing, SAS, vector sync and cross-device wakes, as v0.4 already plans.

**v0.4 People (the second half of the same milestone, on the same sidecar and keys)**

7. **Board store.** Add `boards/<boardId>.db` on the existing store package, one coord service per log, taskId-to-log routing, and the kind allow-list plus reference closure in the board store's Append and Insert.
8. **Membership fold.** Pure functions with table tests for `board.created`, `member.added/removed/left` with cut, several owners, copied person certs, repoKey plus aliases, and surfacing conflicts.
9. **Write gate.** Scrub plus the stricter pass, a preview on share, length caps, `share_request`, the "agents may post" setting, the visible-to line, `agent.card` projection, and the GUI-only path for Share, Invite, Remove and allow-lists.
10. **`irori/board/1` ALPN.** Membership check on connect (non-members get not-found), vector and missing-row exchange against the board store, forwarding through any member, and a flag for stalled vectors.
11. **Invite flow.** Ticket, paste, SAS, signed membership rows, and repo bind with the repoKey and alias check.
12. **Inbound bridge.** Only the hosting device bridges, idempotent on `ref`. It adds the cross-person wake downgrade, the `cross_person` bypass hold, per-person DIRECT_LIMIT, and per-person token tally and cap.
13. **Mirrored leases.** `lease.acquired/released/refused` on `path:<boardRepoKey>:*` from agents working shared tasks, shown with their card, and resolution of leases taken out of contact.
14. **Board GUI for people.** Members panel (invite, remove, owners, fingerprints), per-person allow-lists, the rejected-after-removal view, Withdraw with the "cannot recall" notice, and token spend per person.
15. **Tests.** Two in-process Cores with a fake transport, three devices (your Mac and Windows node, Blake's PC). They must show:
    - private rows never cross;
    - a forged kind is refused;
    - a forwarded row verifies;
    - removal with a cut gives the same fold on every member;
    - a cross-person wake is downgraded;
    - bypass to bypass across persons is held;
    - each board row is bridged exactly once;
    - a mirrored lease blocks across people and shows as a card.
16. **Live run with Blake.** Use his real machine and one shared repo over n0 relays:
    - he comments, once next-turn and once asking to wake;
    - your agent assigns to his (held, then approved);
    - a lease blocks a shared task;
    - then he is removed, and his new rows are refused.

## Decisions for Stephen

1. **What does a board bind to?**
   - (a) Exactly one repoKey.
   - (b) A set of repoKeys for one project.
   - (c) One "Stephen + Blake" board for everything you share.

   Recommended: **(b)**. Start with the one repo Blake co-develops. Aliases cover forks, and the structure doesn't change when a second repo joins.

2. **Can Blake wake your agents?**
   - (a) Never: always downgrade to next turn.
   - (b) Downgrade by default, with a per-board switch per person.
   - (c) Approve each wake.

   Recommended: **(b)**.

3. **Blake's rows reaching your bypass sessions.**
   - (a) Hold every one for approval.
   - (b) Hold by default, with one allow-list switch per board.
   - (c) Let them through.

   Recommended: **(b)**. Flipping the switch is a deliberate trust decision.

4. **Cross-person assignment.**
   - (a) Held for your approval.
   - (b) Delivered at next turn without asking.
   - (c) Assignments to a specific agent go next turn; assignments to "Stephen + repo" land on your Board for you to route.

   Recommended: **(c)**. Treat (b) as an allow-list option per board.

5. **Can agents create board tasks?** *(Now a per-board autonomy setting; this decision is its default.)*
   - (a) Off, with `share_request` held for you.
   - (b) On per board.
   - (c) Never.

   Recommended: **(a)**.

6. **Which leases are mirrored to the board?**
   - (a) Only leases of agents working shared tasks.
   - (b) All leases on the board's repos, shown only as "held by stephen".
   - (c) None; git conflicts are the signal.

   Recommended: **(a)**. (b) reveals private work in a shared repo and when it runs.

7. **Who invites and removes?**
   - (a) Only the creator.
   - (b) Any owner, with several owners allowed.
   - (c) Any member.

   Recommended: **(b)**. Make Blake a co-owner on boards where losing your devices shouldn't orphan the board.

8. **Where does your person key live?**
   - (a) On all your devices.
   - (b) Only on one primary device, which signs certs and membership.

   Recommended: **(a)**, with one-click revoke and a cut from any other device. With (b), your Windows node would have to be up for every invite.

9. **Should your always-on node store and forward Blake's board rows?**
   - (a) Yes.
   - (b) Only your own rows.

   Recommended: **(a)**. Signatures mean it cannot forge them.

10. **What capture does Blake ever see?**
    - (a) Nothing beyond commit SHAs and PR URLs cited by tasks.
    - (b) (a), plus explicit "share this turn/thread" extracts written as scrubbed board rows.
    - (c) Edit metadata for bound repos by default.

    Recommended: **(a) now, (b) later**. Never (c): paths and edit timing are sensitive on their own.

11. **Encrypt board logs?**
    - (a) No: rely on authenticated transport and serving only to members.
    - (b) A board key that rotates on removal.

    Recommended: **(a)** while boards are two people, and **(b)** when a board gets a third person or untrusted forwarders.

12. **Fallback for next-turn delivery if the mod can't inject.**
    - (a) A UserPromptSubmit hook (needs your approval for the settings.json edit).
    - (b) Ride the next wake or tool call only.

    Recommended: **run the spike first**. If the mod can't do it, use (a). Grok gets items on its next inbox or tool call either way.

## Alternatives considered

- **Spaces: one log with a seq per (device, space) and explicit membership.** It has the strongest authority model, and its `isOwner` split, `cross_person` rule and `lease.refused` rows are grafted in above. Why not:
  - it migrates the core events primary key and needs every row placed in the right stream, so a wrong placement either leaks or strands rows;
  - it shares Tier 0 capture by default in shared repos and publishes private sessions' leases;
  - its removal check uses HLC, which a member can backdate;
  - it adds epoch encryption before any need for it has shown.
- **Per-row audience on one log, one seq per (device, audience).** It has the best privacy defaults of the one-log designs, and the `cut` vector, the reduced agent card, reference closure and GUI-only promote come from it. Why not:
  - it is the heaviest design: the same primary-key migration, a defaults table per kind, field reviews for every new kind, and per-blob keys;
  - its `user:<personId>@<deviceId>` actors keep the owner prefix;
  - its "authoritative only for state the author owns" rule contradicts picks 3 and 4;
  - moving `wake.issued` and `turn.*` to device-local storage conflicts with v0.4 cross-device wakes.
