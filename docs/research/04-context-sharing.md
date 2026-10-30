# Agent hub research: what makes the best context-sharing hub

*Irori research, 2026-10-25. "hub" below = Irori. This revises open decision 3 in [01-architecture-plan.md](01-architecture-plan.md) (its §9 open decisions). Claims marked [refuted] or [corrected] failed or changed under fact-checking. Claims marked [unverified] were not re-checked. Changes from the critic pass are listed in "Critic notes" at the end.*

## 1. Short answer: eight principles

1. **By default, agents send each other messages. A full conversation is opened by pointer, or by an explicit fork. It is never pushed.** Claude Code's messaging sends "a piece of text one Claude writes to another, never the sender's conversation history or files. To move a whole conversation… resume the session instead" ([cross-session-messaging](https://code.claude.com/docs/en/cross-session-messaging)). Agent-team teammates start without the lead's history ([agent-teams](https://code.claude.com/docs/en/agent-teams)).
   - **Exception:** full-context sharing has a real place when someone asks for it on purpose. Claude Code fork subagents "inherit the parent conversation." Cognition's 2026 "80/20 solution" is "share a fork of the full context of the primary model with the smart model" ([2026](https://cognition.com/blog/multi-agents-working)).
   - **Zed Delta** goes furthest. It replicates "the conversation and the worktree together," and "transcripts stay whole." Threads are "private until you share them," and sharing is an explicit link ([Delta](https://zed.dev/blog/introducing-delta)).
   - So the hub should offer "fork full context to X" and "share this thread" as actions Stephen or an agent chooses. They should never be the default way context moves.

2. **Small, focused context beats full history.** In Chroma's LongMemEval test, focused prompts of about 300 tokens did significantly better than full prompts of about 113k tokens in every model family tested ([context-rot](https://www.trychroma.com/research/context-rot)).
   - Caveat: Claude's large gap is mostly because it declines to answer when the question is ambiguous.
   - Caveat: length and irrelevant content aren't separated in the test.
   - Caveat: Chroma sells retrieval products.
   - Caveat: the test is a memory-QA benchmark. That it applies to handing work between agents is an inference.
   - Anthropic's subagents return summaries that are "often" 1–2k tokens ([context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)).

3. **Share actions and decisions, not only chat.** Cognition says to "share full agent traces," because "actions carry implicit decisions" ([2025](https://cognition.com/blog/dont-build-multi-agents)). The hub mostly meets this by recording edits, tool calls and decision records as typed events. That is our inference, not Cognition's claim. When it isn't enough, use the opt-in full-context fork from principle 1.
   - **[refuted]** An earlier summary said Cognition's 2026 post limits full-context sharing to narrow cases. It doesn't. The post still says to share as much context as possible.
   - What the 2026 post narrows is who writes. Multi-agent setups work when "writes stay single-threaded" and the other agents contribute thinking, not actions. It describes three working patterns:
     - a smart-friend model given a forked copy of the full context;
     - a reviewer given deliberately no shared context;
     - a manager that coordinates child agents "without fragmenting decisions."
   - Swarms of parallel writers still fail.

4. **The file list and change history are captured mechanically, never summarized by an LLM.** In Factory's evaluation, "artifact trail" (which files changed) was the weakest part of every compression method, at 2.19–2.45 out of 5 ([Factory](https://factory.com/news/evaluating-compression)) [unverified]. Summaries should be anchored and incremental: intent, files, decisions, next steps. The file list is filled in from capture.

5. **Coordinate first: claims, leases, done-criteria and verification.** **[corrected]** MAST covers 1,642 traces from 7 frameworks ([MAST](https://arxiv.org/html/2503.13657)).
   - Problems with system design and task specification are the largest group, at about 44%. Inter-agent misalignment is about 32%, and verification about 23.5%. These shares are summed from the paper's per-mode figures, which may be shares of failure occurrences rather than of traces.
   - Failures from missing context are rare: lost history 2.8%, conversation reset 2.2%, withheld information 0.85%.
   - The ChatDev fixes that helped were clearer role specs (+9.4%) and a step that checks the result against the objective (+15.6%). That supports explicit done-criteria and verification.
   - Your own sessions point the same way. The cross-session traffic was about splitting file ownership by hunk and booking GPU time (§6) [unverified].

6. **Shared memory is curated, scoped, attributed and anchored.** A2A, LangGraph, Letta, Anthropic memory stores and Graphiti all keep the conversation record separate from shared curated state [unverified synthesis].
   - Copilot memory stores citations to code and re-checks them when the memory is read ([GitHub](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)) [unverified].
   - Delta's comments stay "anchored to the code as it evolves" ([Delta](https://zed.dev/blog/introducing-delta)).
   - This fits your rule that only you promote things to the KB.

7. **Raw transcripts are sensitive and stay on their own device by default.** Amp removed public threads because "it's getting too hard to review a thread to ensure it doesn't contain any snippets of sensitive files." The decision was proactive and "isn't prompted by any incident" ([Amp](https://ampcode.com/news/end-of-public-threads)).
   - Your hub is a private, single-user mesh, so Amp's public-exposure risk applies only partly.
   - The risk that does apply: every extra copy is another place a secret lives, and a peer agent could read it into its context.
   - Claude Code transcripts are plaintext and include tool output ([claude-directory](https://code.claude.com/docs/en/claude-directory)) [unverified].

8. **(New) Peer messages are untrusted input. They never grant authority.** Claude Code's own messaging enforces this:
   - each message is labeled as coming from another session, not the user;
   - a message can't approve prompts or change config;
   - senders are rate-limited and identical repeats are dropped;
   - messages to a bypass-mode receiver are held unless the sender is also in bypass ([cross-session-messaging](https://code.claude.com/docs/en/cross-session-messaging)).

   The hub has to give at least the same guarantees across devices and across agent kinds (§5, Security and human control).

## 2. The context model (records the hub stores)

Everything below lives in the plan's append-only log, `(device_id, seq, hlc, actor, kind, payload)`. The views listed at the end are rebuilt from it.

| Record | Contents | Written by | Read by |
|---|---|---|---|
| **agent card** | device, kind (claude/grok/rock), cwd→repo_key, capabilities (wake: mod/channel/resume), peerFeatures, permission mode, liveness | hub, from `sessions/<pid>.json`, the mod and the ACP handshake | router, UI roster |
| **message** | from (device, session, name), reason, trust, sender permission mode, hop count / causal parent, parts[] (A2A-shaped), reference_task_ids | agent tool call or GUI | peer agent (fenced as untrusted) |
| **delivery** | per recipient: queued → held → delivered → engaged → answered, or expired/skipped/rejected | hub | hub, UI |
| **thread** | contextId: a session plus its edits, commits, messages and handoffs (the Delta unit); private until shared | hub | human (timeline), peer (via extract) |
| **turn index** | user prompt, assistant prose, readable thinking text, tool name with args or path, exit code, ids (promptId, toolUseId, uuid, parentUuid), timestamp, cwd | capture (mod `session.append`, hook `transcript_path`, JSONL) | FTS search, `hub_why`, extraction |
| **edit** | repo_key, path, patch blob, before/after git SHA, promptId/toolUseId, fidelity (exact or heuristic) | capture | human diff view, conflict warnings, `hub_why` |
| **commit / PR link** | SHA or PR → session or turns (from `.git/logs/HEAD` and `pr-link` rows) | capture | routing of CI/review replies, the "open session behind this commit" action |
| **task** | A2A TaskState enum (working, input_required, completed, failed, rejected…), objective, done-criteria, scope paths, depends_on, single writer | agent or Stephen | agents, UI |
| **lease/claim** | (repo_key, path glob) or named resource (`gpu/qwen`, `port:8799`), holder, TTL, heartbeat | agent tool call; auto-expires when the session dies | all agents, UI |
| **decision / fact** | statement, anchor (repo_key, path, sha, range), valid_from, superseded_by, source_event_ids | agent `decision_record` or Stephen | peers (re-validated on read), human |
| **handoff card** | goal, branch/HEAD, done, next, decisions, open questions, do-not rules, files touched, refs | hub-generated at Stop, SessionEnd or compaction; human-editable | the peer that receives it |
| **summary** | Claude Code's `away_summary` (~230–670 B) [unverified], compaction summaries (~12–17 KB, measured), ai-title; hub summary only when none of these is fresh | harvested by capture | roster status line, handoff card input |
| **deliverable report** | what changed, SHA/PR, test results, what's next | agent tool call | Windows agents, Stephen |
| **pinned block** | small named context per repo_key ("conventions", "who's on what"), size limit, ro/rw | Stephen or an agent with rw | injected at session start and after compaction |
| **artifact / blob** | content-addressed tool output, image, thumbnail, JSONL snapshot; encrypted per blob so it can be crypto-shredded | capture | fetched lazily by `hub_fetch(ref)` |
| **approval / audit** *(new)* | human decision on a cross-device wake, fork, share or permission relay; who/when/what | Stephen (GUI, phone) | hub, UI |

## 3. What a peer gets when it joins or wakes

The wake packet is injected once, as a single consolidated message at a turn boundary. Budget: **≤4k tokens total.**
- Handoff card or thread extract: about 1.5–2k tokens.
- Open claims and leases for its repo, plus unread inbox items, batched: about 1k.
- Pinned blocks for the repo: ≤1k.
- Refs (`hub://…`) for everything else.

Why this budget:
- Anthropic's subagents return summaries of often 1–2k tokens (principle 2).
- Chroma's focused-context result (principle 2).
- "Lost in Multi-Turn" reports a 39% average drop when instructions arrive in pieces over several turns, which argues for batching rather than trickling ([arXiv](https://arxiv.org/html/2505.06120v1)) [unverified].
- Your own sessions started from pasted briefs of about 3 KB (§6). That is about 1k tokens, the same order as this budget.

The 4k figure is a starting point, not a measured optimum. Tune it in the spike (§8).

Tool results and thread reads:
- Every MCP tool result is capped at about 2k tokens and comes with a cursor.
- Reading another thread uses `hub_thread_read(thread, goal)`. It returns a goal-conditioned extract built near the data, on the origin device or the always-on node. This is Amp's `read_thread` pattern ([Amp](https://ampcode.com/news/read-threads)).
- That extraction is itself an LLM call, which costs tokens and possibly GPU time (§5, Cost).

Wake packet content is fenced as untrusted peer data. Only pinned blocks you wrote, and handoff cards you edited, carry user-level trust.

The same "your hub state" block is re-injected after compaction through a SessionStart(compact) hook, because compaction drops whatever was in context ([context-window](https://code.claude.com/docs/en/context-window.md)) [unverified]. Your long sessions compact roughly every 6–12 hours [unverified; 8 compactions over ~4 days in 78e7ff77 is about one every 12 h].

## 4. Decision: should full transcripts sync across devices?

**No eager sync of raw transcripts. Do eagerly replicate a stripped turn index, and keep the full transcript reachable and preserved.**

The reasons, in order of weight:
1. **Secrets.** A replicated append-only row can't be scrubbed later (principle 7).
2. **Context quality.** Nothing raw should ever be injected into an agent (principle 2).
3. **Format instability.** The JSONL entry format "is internal to Claude Code and changes between versions" ([sessions](https://code.claude.com/docs/en/sessions)).
4. **Noise.** Most bytes are duplicated images, api-request copies and subagent chatter (§6).

Bandwidth is not one of the reasons. About 160 MB/day of raw data is modest over iroh on a LAN, and the meaningful stripped tier is about 10 MB/day, about 4 MB/day compressed. So size argues for what to drop, not for refusing to sync.

Tiers and defaults:

- **Tier 0: always replicated, a few MB/day.** Agent cards, messages and deliveries, tasks, leases, decisions, handoff cards, harvested summaries, edits (patch blobs fetched on demand), commit/PR links, deliverable reports, approvals.
- **Tier 1: always replicated and FTS5-indexed, estimated at a few MB/day.**
  - Contents: the main-session turn index (prompts, assistant prose, tool name/args/path), readable thinking text with signatures removed, and text tool results truncated to about 4 KB (head plus tail plus full length plus a ref). It also keeps parentUuid, timestamp and cwd.
  - Estimate: capped text results come to about 3.8 MB/day and conversation text to about 1.2 MB/day across all sessions, less for main sessions only.
  - Subagent and workflow transcripts are 76% of bytes. They get only a one-line roster entry plus their final answer in Tier 1, and their bodies go in Tier 2.
- **Tier 2: fetched on demand from the origin, zstd-compressed.**
  - Contents: full tool results, images (by hash, deduplicated, about 47% of image blocks are repeats, with a thumbnail), subagent and workflow transcripts, file-history entries.
  - A **per-session JSONL snapshot is taken at SessionEnd** and at each compaction. This matters because Claude Code deletes transcripts, subagents/, tool-results/ and file-history/ after `cleanupPeriodDays` (default 30; you haven't overridden it), so a pointer would go dangling. Copied excerpts survive ([claude-directory](https://code.claude.com/docs/en/claude-directory)).
  - The sweep skips `claude -p --bare` sessions and Desktop/Cowork sessions, so the age at which a source disappears varies.
  - The always-on Windows node can mirror Tier 2 opt-in per repo, about 50–80 MB per 10 days compressed for the main transcripts [unverified].
- **Tier 3: never synced.** `api-request*` records (about 98–99% duplicate; the ~1.5% unique part is harness nudges), thinking signatures, attachment, queue-operation and permission records.
  - Exception: attachment records hold about 4.5 MB of `structured_output`, which is real content. Keep it in Tier 2 rather than dropping it.

**Secrets.** Redact at ingest, before an event is sealed, because a replicated row can't be scrubbed later.
- Drop the whole tool result whenever the tool input touched `.env`, `auth.json` or credential files, whatever the output was.
- Run a pattern scan over tool results, prompts and readable thinking text. The scan skips base64 payloads, because all 22 "AIza" matches were inside images [unverified].
- Encrypt blobs with per-blob keys so that deleting the key erases a blob that slipped through (crypto-shredding). The append-only log otherwise has no way to forget.
- The scan of your files found no confirmed secrets, but it did find 9 Bash calls that read `.env` or credential files [unverified].
- One sweep saw the string `CLAUDE_CODE_MESSAGING_TOKEN` in two transcripts, but didn't check whether a value was logged. Resolve this before v0.1 (§8 Q5).

**Size.** Measured on this Mac (details in §6): about 1.64 GB in 10 days, under 1% of it prose, 76% of it subagent and workflow transcripts.

**Resume on another device.** It works. `claude --resume <absolute .jsonl path>` resumes a session from a copied file ([sessions](https://code.claude.com/docs/en/sessions)) [unverified]. Constraints:
- Always pass `--fork-session`. Resuming one session in two places interleaves both into one transcript.
- Never put a second copy into `~/.claude/projects`, or resume by ID reports not-found.
- Rewrite `cwd` through a repo_key mapping on parsed JSON, never with a string replace. Windows paths are stored with doubled backslashes.
- Re-pass `--mcp-config`, `--settings`, `--plugin-dir` and `--add-dir`, which resume doesn't restore.
- Only the tail after the last `compact_boundary` is loaded, so only that needs to be shipped: 173 MB becomes 13.4 MB, and 109 MB becomes 2.1 MB [unverified].

The default way to "pick this up on the other machine" should be a **fresh session seeded with the handoff card**:
- it matches how work actually moved between your devices (a ~3 KB brief, §6);
- it matches the focused-context evidence;
- resuming on a cold prompt cache re-bills the whole context [cost inference].

"Continue exactly" is opt-in. It is the full-context fork from principle 1. Mac→Windows is experimental and gated behind a Doctor-tab check.

**Remote Control.** It already mirrors transcripts of RC-connected Claude sessions through Anthropic's servers ([remote-control](https://code.claude.com/docs/en/remote-control.md)) [unverified]. Link to it rather than duplicate it. The hub's iroh path covers what RC doesn't: grok-build, rock, API-key sessions and serverless storage.

## 5. Interop

- **Speak MCP.** Design for the 2026-07-28 spec, which is stateless ([changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)):
  - no sessions and no `initialize`; `server/discover` does capability discovery, and the version and capabilities travel in `_meta` on each request;
  - `subscriptions/listen` replaces `resources/subscribe`;
  - tasks move into the optional `io.modelcontextprotocol/tasks` extension.

  So every hub tool takes explicit ids that the hub mints (thread_id, task_id, delivery_id). Hub tasks are hub data, not MCP tasks.

- **Claude channels** (`notifications/claude/channel`) work for push, with limits ([channels-reference](https://code.claude.com/docs/en/channels-reference)):
  - They are never acknowledged.
  - Events are silently dropped if the session didn't load the server as a channel, or if org policy blocks it.
  - Meta keys with characters other than letters, digits and underscores are silently dropped.
  - In `-p` mode and under the Agent SDK the dev flag is ignored and the channel doesn't register at all. **So a headless always-on node can't receive channel pushes.**

  Consequences for the hub:
  - "Delivered" must be confirmed by a `hub_ack`/`hub_reply` call or a matching `turn.start`.
  - Headless wake uses the mod's `$.prompt.submit` or `--resume`.
  - Permission relay (`…/permission_request`) gives remote approval. Since v2.1.234 it reaches only servers registered as channels that declare `claude/channel/permission`.

- **Zed's Agent Client Protocol (ACP)** gives the hub structured capture for grok-build and rock, if the hub acts as the ACP client.
  - **[corrected]** The variant names are `agent_thought_chunk` and `current_mode_update`, not `thought_message_chunk` or `mode_change`. The list also includes `compaction_summary_chunk` and `session_info_update`, which are useful for harvesting.
  - Full replay on load is a v1 `session/load` behaviour. v2 uses `session/resume` without replay ([prompt-turn](https://agentclientprotocol.com/protocol/prompt-turn)).
  - Ignore IBM's ACP, which merged into A2A.

- **A2A: borrow the shapes, don't run the protocol.** Use contextId, Task, Message.parts, Artifact with append/lastChunk, the TaskState enum and agent cards ([A2A 1.0](https://a2a-protocol.org/latest/specification/)).
  - An `INPUT_REQUIRED` task continues with a new message carrying the same taskId *and* contextId.
  - An `/a2a` adapter can come later.
  - Skip AGNTCY. Adoption looks thin, and nothing says its ACP spec was archived because of A2A.

- **Native ListAgents/SendMessage** stays the Claude↔Claude path, and the hub observes it. The hub fills these gaps:
  - grok-build and rock;
  - no RC sign-in required;
  - inboxes that last past the 100-message hold limit and the 12h `notify_when_idle` window;
  - idle notices across machines (native ones are same-machine only);
  - an audit trail.

  Native messages to a bypass-mode receiver are held unless the sender is also in bypass, and the hub should keep that rule (below). Run the Windows node natively, not in WSL, because WSL 2 and native Windows sessions can't reach each other.

### Security and human control *(new)*

- **Device trust.** Only enrolled iroh node IDs (your Mac and Windows machines) can write to the log. Each event is signed by its device key. Agent identity is (device, session pid/uuid, kind), set by the hub, never claimed by the agent itself.
- **Prompt injection between agents.** Peer messages, extracts and fetched blobs are injected inside a fenced "untrusted peer content" envelope that carries the sender and its trust level.
  - Text pulled from tool output, such as web pages or files, can carry instructions from a third party into another agent. Extraction on the node therefore returns quotes with refs, not imperative rewrites.
  - A peer message can never approve a permission, change settings, grant or extend a lease held by someone else, or edit pinned blocks.
- **Bypass-mode receivers.** Mirror the native rule. Hold any message to a session running with bypassed permissions (common for headless and auto-revived sessions) unless you approve it, or unless it comes from an allow-listed sender for that repo.
- **Auto-revive gate.** Auto-revive is approved, but reviving a closed session *because a peer asked* is a wake with full tool access. Default policy:
  - same-device revive for your own messages: automatic;
  - cross-device or peer-triggered revive into bypass mode: approval queue, with a phone notification through permission relay or PushNotification;
  - a per-repo allow-list can relax this.
- **Loops and runaway traffic.** Messages carry a hop count and causal parent. The hub stops ping-pong after N hops, applies per-sender rate limits (native already throttles and drops identical repeats), and offers a global "pause all wakes" kill switch in the tray.
- **Single writer.** Following Cognition, a task or path lease has one writer. Other agents contribute reviews and messages, not edits.

### Cost *(new)*

- Every wake and every resume is a billed turn. A cold-cache resume re-bills the whole kept context. Track tokens per wake in the delivery record, and show a daily total in the UI.
- `hub_thread_read` and handoff-card generation are LLM calls. Prefer harvested `away_summary` and compaction summaries, which are free, and generate only when they are stale.
- If extraction runs on a local model on the Windows node, it takes a `gpu/*` lease like any other job (your one-big-model-at-a-time rule).
- Your stated direction is "no cost limits," so these are visibility features, not caps. Loop limits still apply, because they are about correctness.

## 6. What your own sessions show

Sources: `~/.claude/projects/**/*.jsonl` and throwaway local analysis scripts (not kept).

**Volume**
- 1.64 GB in about 10 days. The count of .jsonl files ranged from 773 to 791 across sweeps taken on 2026-10-25, because sessions were still writing. The earlier "761" figure is stale.
- About 160 MB/day as a straight average. Dated per-day totals understate it, because `api-request-blob` lines (about 461 MB) have no timestamp. "Peak day" figures are inflated when files are bucketed by last-modified date.
- 76% of bytes are subagent and workflow transcripts: 1.24 GB, almost all under `subagents/workflows/`, against 394 MB in 16 main sessions.
- Two root sessions: 173 MB (78e7ff77) and 109 MB (57bc55f1), plus about 458 MB and 478 MB of their own subagent files.

**What the bytes are**
- Tool results are about 47% of the corpus, and about 93% of tool-result bytes are images. 3,913 of the 4,003 image results come from Read. 81% of image blocks are PNG, and about 47% are duplicates.
- `toolUseResult` largely duplicates the tool_result content. In 78e7ff77 it is 61.8 MB against 60.5 MB.
- **[corrected]** `api-request*` records are about 28–33% of the whole corpus but only about 21% of the 57bc55f1 file. They are about 98–99% duplicate, not 100%.
- Attachment records are about 9% of the corpus but only 3.5% of 57bc55f1. Whether they are redundant was not checked, apart from `structured_output`.
- **[refuted]** The claim that "thinking is useless" doesn't hold. 11% of thinking blocks (about 0.45 MB) carry readable reasoning. Drop the signatures (about 30 MB) and keep the text.

**Strip ladder**
- Raw 1640 MB → 100.7 MB stripped → 42.6 MB compressed → 12 MB conversation → 1.7 MB main-thread prose.
- Only the 100 MB tier keeps the meaning of the work. It still needs parentUuid, timestamps and cwd added back, about 10–15 MB more. The 12 MB and 1.7 MB tiers drop all tool calls, so they are summaries, not lossless forms.

**What actually moved work**
- Both long sessions started from pasted briefs of about 3 KB (3,041 and 3,466 characters). The 57bc55f1 brief was explicitly a handoff from a Windows session.
- You typed about 12.4 KB after the brief in 78e7ff77, over about 4 days.
- Inside that session there were 8 compaction summaries of 12.4–16.5 KB each.
- The raw transcript was never what carried the work.

**Coordination traffic**
- SendMessage was used to split commit ownership of `factory_http.py` by hunk and to book a GPU window [unverified].
- Status reports with SHAs flowed through Claude Remote, hearthbot and tsukumo-discord [unverified].
- Only 7 files were re-read across root sessions. 58% of reads were repeats inside one session family [unverified].

**Project identity**
- 757 transcripts sit under `-Users-stephenhorton`, so the directory name doesn't tell you the project. Use the cwd on each event, mapped to the git remote [unverified].

**Grok**
- grok-build already keeps per-session summaries (about 2.5 KB median) and FTS docs (about 8.5 KB) [unverified].
- Its bus roster and sockets are stale, so check liveness by PID or heartbeat [unverified].

**Format**
- About 10 Claude Code versions (2.1.299–310) are mixed in the corpus, and the docs call the format internal. They recommend `/export`, hook `transcript_path`, `-p --output-format stream-json`, a SessionEnd hook or the Agent SDK instead of parsing JSONL directly ([sessions](https://code.claude.com/docs/en/sessions)).
- **[refuted]** The claim that "new record types are appearing" is unsupported.

## 7. Changes to the draft plan

- **§5 Data:** replace "Full transcripts stay on the device that recorded them" with the Tier 0–3 model in §4. That includes the SessionEnd and compaction JSONL snapshots into the origin device's blob store, redaction before sealing, and per-blob encryption keys.
- **§8, "Transcript storage… pointers and excerpts":** rewrite it. The 30-day cleanup leaves pointers dangling, so copy at capture time. Excerpts stored as blobs are fine. Add a Doctor-tab check that shows `cleanupPeriodDays`, and never change it silently.
- **§9.3:** answer it as: "a stripped turn index plus structured events replicate eagerly. Blobs, subagent bodies and full JSONL are fetched on demand. Full-context fork or share is an explicit action. Nothing raw is injected into an agent."
- **§4 Delivery:**
  - Add `held`, `expired` and `rejected` states.
  - Batch and supersede queued messages into one per turn boundary.
  - A channel push never counts as delivered. Require `hub_ack` or `turn.start`.
  - Messages carry reason, trust, sender mode, hop count and causal parent.
- **§4 Capture:**
  - Prefer the documented interfaces (hook `transcript_path`, SessionEnd hook, stream-json, ACP) and keep a tolerant JSONL parser as the fallback.
  - Allow-list record types and tolerate unknown ones.
  - Harvest `away_summary`, compaction summaries, ai-title and `pr-link`.
  - Try `file-history-delta` before reconstructing from `structuredPatch`.
  - Assign the repo per event from cwd mapped to the git remote.
- **New:**
  - handoff card schema;
  - `hub_context(session)`, `hub_thread_read(thread, goal)`, `hub_fetch(ref)`, `hub_why(repo_key, path, line)`;
  - `hub_fork(thread, to)` as the explicit full-context path;
  - deliverable-report, pinned-block and approval kinds;
  - decisions with supersession, and anchors re-validated on read.
- **Leases:**
  - named resource leases (gpu, ports) as well as path globs;
  - advisory, with TTL, and released automatically when `sessions/<pid>.json` disappears;
  - one writer per task;
  - conflict warnings raised from edit events.

  An agent's self-reported task status is untrusted and is reconciled against commits and edits.
- **New: security and human control.** Enrolled device keys, signed events, untrusted-content fencing, the bypass-receiver hold, the approval queue for cross-device and peer-triggered revives, hop and rate limits, and a pause-all switch (§5).
- **New: cost visibility.** Tokens per wake and per extraction, and the daily total in the UI.
- **§9.4 (Grok):** prefer ACP-client capture over MCP self-reports. It may be cheap enough for v0.3.
- **§7 v1.0 revive:**
  - Cross-device revive always uses `--fork-session` and records a `forked-from` event.
  - Same-device auto-revive resumes in place.
  - A peer-triggered revive into bypass mode goes through the approval queue.
- **MCP surface:** no per-connection state; explicit ids everywhere.

## 8. Open questions for the v0.1 spike

1. Does the mod's `session.append` give full records, including `toolUseResult`, or should capture tail the JSONL or use hook `transcript_path`? Which works on Windows?
2. How large is the Tier 1 turn index per day once readable thinking and 4 KB tool heads are included? The estimate is a few MB/day across all sessions. Measure main-only and all-sessions over a week.
3. Can `claude --resume <path> --fork-session` with a JSON-rewritten cwd revive a Mac session on Windows, and the reverse?
4. Do `away_summary` and compaction summaries make good enough handoff cards, or does the hub need its own generator? Compare against your ~3 KB hand-written briefs.
5. What are the precision and recall of ingest redaction on a seeded test transcript? Check whether `CLAUDE_CODE_MESSAGING_TOKEN` values were logged.
6. Can a Claude channel push reach a session started from the Code tab, and does the permission relay work end to end?
7. Can the Windows headless node run native Claude Code with the mod and be woken with no UI attached, given that channels don't register under `-p` or the SDK?
8. Does `hub_thread_read` extraction on the always-on node stay under the 2k-token cap and still answer "what did X decide about Y"? What does it cost per call?
9. *(new)* Red-team test: a peer message containing instructions, and a tool result carrying injected text, sent to a bypass-mode receiver. Confirm they are held or fenced and can't trigger edits or permission changes.
10. *(new)* Loop test: two agents told to reply to each other. Confirm the hop limit and the pause-all switch stop it.

---

## Critic notes

- **Does the transcript-sync answer follow from the data?** Mostly yes, but the original leaned on size. The evidence says 160 MB/day is modest for iroh, and "too large" was a judgement, not a measurement. The decision is now justified by secrets, context quality, format instability and noise. The stripped turn index (~4 MB/day compressed) is eagerly replicated, which the data clearly supports.
- **Ignored evidence: Zed Delta.** The app is Delta-inspired, yet the report never mentioned that Delta replicates whole transcripts with the worktree, shares only explicitly, and anchors comments. Added to principles 1 and 6, and to the thread record.
- **Ignored counter-evidence.** Fork subagents inherit the parent conversation, and Cognition 2026 endorses forking full context to a "smart friend." Principle 1 was softened from "never pushed" to "by default," with an explicit `hub_fork` path.
- **Cognition 2026.** Added the three working patterns, including the manager and the no-context reviewer, and tied single-writer rules into the leases and tasks.
- **MAST.** Added the caveat that the percentages may count occurrences rather than traces, and the ChatDev interventions that support done-criteria and verification.
- **Channels.** Made explicit that channels don't register at all under `-p` or the SDK, which matters for the headless node. Added the silent drop of bad meta keys and the v2.1.234 permission-relay capability requirement.
- **Contradictions fixed:**
  - Compaction summaries were "12–24 KB" in §2 but "12–16" in §6. The measured range is 12.4–16.5 KB.
  - Tier 1 was "1–10 MB/day" in §4 but "under 1 MB/day" in open question 2. Both are now a derived estimate of a few MB/day.
  - The .jsonl count of 791 is now shown as a 773–791 range.
  - "3,913 of those come from Read" now names its base (4,003 image results).
- **Unverified items newly flagged:** `away_summary` size, plaintext-transcript citation, compaction cadence, and the status-report channels.
- **Missing angles added:**
  - principle 8 and §5 "Security and human control" (device trust, prompt injection between agents, bypass-mode hold, approval gate on peer-triggered auto-revive, hop and rate limits, kill switch);
  - crypto-shredding for the append-only log;
  - §5 "Cost" (wake and resume billing, extraction calls, GPU lease for local extraction);
  - an approval/audit record;
  - two new spike questions (injection red-team, loop test).
- **Capture robustness.** Added the docs' recommended interfaces (`transcript_path`, SessionEnd hook, stream-json, `/export`) ahead of raw JSONL parsing.
- **Smaller additions:**
  - subagent bodies moved explicitly to Tier 2;
  - `structured_output` attachments kept rather than dropped;
  - sessions the cleanup sweep skips (`--bare`, Desktop);
  - A2A `INPUT_REQUIRED` needs taskId plus contextId;
  - MCP `server/discover`;
  - ACP `compaction_summary_chunk`;
  - the Chroma generalization caveat.