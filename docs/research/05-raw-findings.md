# Raw research findings (2026-10-25)

The five research sweeps behind [04-context-sharing.md](04-context-sharing.md), as returned before synthesis. Up to six of each sweep's higher-confidence claims were adversarially fact-checked, and the claims that failed are listed at the end. Read 04 for the corrected conclusions; this file is the evidence trail.

## Sweep: Protocols and memory systems relevant to the hub's data model. Covered: MCP 2026-07-28 (stateless core, subscriptions/li

### In MCP 2026-07-28 the core protocol is stateless. Protocol sessions and the Mcp-Session-Id header are gone, and so is the initialize handshake. Servers that need state across calls must mint explicit handles and pass them as ordinary tool arguments. resources/subscribe is replaced by one long-lived subscriptions/listen stream, and tasks moved out of core into an optional extension (io.modelcontextprotocol/tasks).

- **Confidence:** high
- **Evidence:** The official changelog lists these as major changes 1, 2, 4 and 6, citing SEP-2567, SEP-2575 and SEP-2663. Roots, Sampling and Logging are deprecated.
- **Design implication:** Don't build the hub's MCP surface (hub mcp) on per-connection session state. Every hub tool should take explicit ids such as thread_id, task_id, delivery_id and agent_id that the hub mints and that live in the event log. Don't model hub tasks on MCP tasks, which are an optional extension that is still changing. Hub tasks are hub data and MCP only exposes them through tools. If agents should watch threads, expose read-only resources (hub://thread/{id}) and use subscriptions/listen for change notices.
- **Sources:** https://modelcontextprotocol.io/specification/2026-07-28/changelog

### Claude Code channels (notifications/claude/channel) are not acknowledged. An event is silently dropped if the session didn't load the server as a channel. Events that arrive while Claude is busy are batched into the next turn. Custom channels need --dangerously-load-development-channels, which is ignored in -p and Agent SDK mode.

- **Confidence:** high
- **Evidence:** From the channels reference: 'Claude Code doesn't acknowledge notifications... the await resolves when the message is written to the transport'; 'drops the events silently'; 'If several notifications arrive while Claude is busy, they're delivered together on the next turn'; the development flag is ignored with -p or the Agent SDK. Content arrives as <\channel source=... meta-attrs>, and meta keys must be identifiers. Permission relay is notifications/claude/channel/permission_request, answered with a verdict carrying a 5-letter request_id.
- **Design implication:** In the delivery state machine, 'delivered' must never be inferred from a push. A successful push only means the message was written to the transport. The 'engaged' and 'answered' states need an ack from the agent (a hub_ack or hub_reply tool, which the docs themselves recommend) or a matching turn.start/turn.complete or transcript entry. Waking a headless or -p session cannot rely on a custom channel. Use the mod's $.prompt.submit, or resume with --resume and the message as the prompt. Put the hub's trust and sender fencing into channel meta, for example from_agent, thread_id, trust=untrusted. Permission relay is a ready-made way to approve tool calls from the hub UI or another device.
- **Sources:** https://code.claude.com/docs/en/channels-reference

### The core objects of A2A 1.0 are Task {id, contextId, status{state,message,timestamp}, artifacts, history, metadata}, Message {messageId, contextId, taskId, role, parts, referenceTaskIds, extensions}, Part (exactly one of text/raw/url/data, plus mediaType and filename) and Artifact {artifactId, name, description, parts, metadata, extensions}. TaskState has SUBMITTED, WORKING, INPUT_REQUIRED, AUTH_REQUIRED, COMPLETED, FAILED, CANCELED and REJECTED.

- **Confidence:** high
- **Evidence:** The A2A specification marks 1.0.0 as the latest released version, with spec/a2a.proto as the normative source. contextId groups related tasks and messages, and ListTasks can filter by it. To continue an INPUT_REQUIRED task, the client sends a message with the same taskId.
- **Design implication:** Use A2A's shapes as the hub schema without speaking the protocol. thread = contextId. Each message has a parts[] array of typed parts with mediaType. A task row stores a state from the A2A enum. You need 'input_required', which maps to an agent waiting on Stephen or a peer, and 'rejected', which covers a declined handoff. Each message carries a reference_task_ids column. Matching A2A's names keeps a later A2A bridge mechanical. The hub's delivery states (queued, delivered, engaged, answered) are a separate per-recipient axis, so don't overload TaskState with them.
- **Sources:** https://a2a-protocol.org/latest/specification/

### A2A streams artifacts in chunks: TaskArtifactUpdateEvent carries {taskId, contextId, artifact, append, lastChunk}, and TaskStatusUpdateEvent carries {taskId, contextId, status}. Agent Cards live at /.well-known/agent-card.json and list skills {id, name, description, tags, examples, inputModes, outputModes}.

- **Confidence:** high
- **Evidence:** From the Section 4 data objects and the method table (SendMessage, SendStreamingMessage, GetTask, ListTasks, CancelTask, SubscribeToTask, and push notification config CRUD).
- **Design implication:** Every event that crosses devices should be one of a few kinds shaped like A2A: message, task.status, artifact.update (with append/last_chunk, so a long patch or summary can stream without changing earlier rows), plus hub-only kinds (edit, commit.link, decision, fact, presence). Give each connected agent a small 'agent card' record (device, kind claude/grok/rock, cwd/repo_key, skills/tags, capabilities such as wake:mod|channel|resume, peerFeatures). That registry is what ListAgents-style routing and the UI roster read from.
- **Sources:** https://a2a-protocol.org/latest/specification/

### Two different 'ACP's exist. Zed's Agent Client Protocol (agentclientprotocol.com) connects an editor to an agent. Its session/update notifications stream user_message_chunk, agent_message_chunk, thought_message_chunk, tool_call, tool_call_update, plan and mode_change, and session/load must replay the entire conversation as session/update. IBM's Agent Communication Protocol (BeeAI) is a separate protocol that was merged into A2A under the Linux Foundation in August 2025, and new work should target A2A.

- **Confidence:** high
- **Evidence:** The ACP session-setup docs (v1/v2) say 'The Agent MUST replay the entire conversation to the Client in the form of session/update notifications'. v2 adds session/resume without replay, plus session/close. IBM Research's project page says 'ACP is now part of A2A under the Linux Foundation'.
- **Design implication:** Wherever the plan says 'ACP' for grok-build/rock, make clear it means Zed's Agent Client Protocol. This changes open decision #4 (Grok capture depth). If the hub is the ACP client for grok-build, it receives structured tool_call and tool_call_update events and message chunks for free. That gives it Claude-level capture fidelity without MCP self-reports, and session/load can backfill history. Normalize ACP session/update kinds and Claude JSONL entry types into the same hub event kinds. Ignore IBM ACP entirely.
- **Sources:** https://agentclientprotocol.com/protocol/session-setup, https://agentclientprotocol.com/protocol/v2/session-setup, https://research.ibm.com/projects/agent-communication-protocol, https://tyk.io/learning-center/agent-protocols-a-complete-guide-to-mcp-a2a-and-acp/

### AGNTCY (Cisco's project, donated to the Linux Foundation in July 2025) provides OASF agent schemas, a directory, identity and SLIM messaging. Its own Agent Connect Protocol was archived after A2A became the standard, and one independent review calls adoption light.

- **Confidence:** medium
- **Evidence:** Linux Foundation and press coverage of the July 29, 2025 donation, plus an independent review reporting Directory v1.4.0 in May 2026 and light adoption. That last point comes from a single source.
- **Design implication:** Don't adopt OASF, SLIM or the AGNTCY directory. For a single user with about five agents on two or three devices paired over iroh, they add identity and discovery layers the hub already has: hato-core keys plus the hub agent registry. Revisit only if the hub ever has to federate with other people's agents.
- **Sources:** https://www.hpcwire.com/off-the-wire/linux-foundation-welcomes-agntcy-to-tackle-ai-agent-fragmentation/, https://rywalker.com/research/agntcy

### Letta's shared memory is a block (label, value, limit, read-only or read-write) created once and attached by id to many agents. An edit by one agent is visible to the others immediately, so they can coordinate without messaging. Newer Letta Code uses MemFS 'context repositories': git-backed Markdown files with YAML frontmatter, where files in system/ go into the prompt every turn and the rest load on demand. Shared repos are separate git repos that several agents attach.

- **Confidence:** high
- **Evidence:** Letta's shared-memory docs show client.blocks.create then block_ids, plus read-only/read-write access. The MemFS docs: Markdown plus frontmatter, a system/ directory pinned in context, no vector index by default, agents commit changes, and a sleep-time reflection subagent writes memory in a worktree.
- **Design implication:** Add a small 'pinned context' primitive: named blocks scoped per repo_key or workspace, each with a size limit, an owner and ro/rw access. Examples are 'current goals', 'who is doing what', and 'conventions for repo X'. The hub injects them at session start and on change, through hooks or the mod's context. Keep them small and curated, not a dump. Storing them as Markdown files with frontmatter fits the user's existing knowledge-base and memory-file habits. Every edit is still an event, so each block has a history.
- **Sources:** https://docs.letta.com/guides/core-concepts/memory/shared-memory/index.md, https://docs.letta.com/guides/agents/shared-memory-blocks, https://docs.letta.com/concepts/memfs, https://docs.letta.com/concepts/shared-memory

### Anthropic's two memory primitives are both file-shaped. The memory tool (memory_20250818) is a client-side /memories directory, where Claude issues view/create/str_replace/insert/delete/rename and the application backs the storage. Managed Agents memory stores (public beta April 2026) are workspace-scoped document collections mounted as directories, shareable across agents with per-store scopes (for example an org store read-only and per-user stores read-write). They support concurrent agents without overwrites, and an audit log records which agent and session wrote each memory, with rollback and redaction.

- **Confidence:** high
- **Evidence:** From the platform memory-tool docs and the Managed Agents memory docs and launch blog.
- **Design implication:** Use 'a directory of text documents with scoped access and an attributed audit log' as the model for the hub's notes, decisions and facts layer. Agents already know how to work on files, so expose the shared memory as a tool surface shaped like a file tree (hub_memory view/create/str_replace, matching the memory-tool verbs) or as a synced directory. Record agent_id+session_id+device on every write, and support rollback and redact. Add Managed Agents' read-only org scope against read-write per-agent scope directly to the access model.
- **Sources:** https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool, https://platform.claude.com/docs/en/managed-agents/memory, https://claude.com/blog/claude-managed-agents-memory

### Graphiti (Zep) stores facts as edges that carry two timelines. valid_at and invalid_at record when the fact was true in the world, and created_at and expired_at record when the system learned or superseded it. A new contradicting fact invalidates the old edge instead of deleting it, and each edge lists the episodes (source messages or JSON) that mentioned it.

- **Confidence:** high
- **Evidence:** Graphiti temporal-model docs and the Zep blog post on temporal knowledge graphs. Episodes are ingested as message, text or JSON and stored as Episodic nodes, and edges keep provenance through an episodes list.
- **Design implication:** Store decisions and facts as rows that get superseded: {fact_id, subject (repo_key/path/topic), statement, valid_from, superseded_by, recorded_at, source_event_ids[]}. This fits the append-only log naturally, since 'decision X replaced by Y' is just another event, and it answers 'what did we believe last Tuesday'. Don't adopt a graph database or LLM entity extraction in v1. The source_event_ids link back to the messages that drove each fact, which gives you provenance for free.
- **Sources:** https://mintlify.com/getzep/graphiti/concepts/temporal-model, https://blog.getzep.com/beyond-static-knowledge-graphs/, https://pypi.org/project/graphiti-core/0.5.0/

### Mem0 scopes every memory by user_id, agent_id and run_id (at least one required). On add() it extracts facts with an LLM, compares them against existing memories in the same scope, and writes changes to a per-memory history table. Its graph memory is now built internally instead of in an external Neo4j store.

- **Confidence:** medium
- **Evidence:** From Mem0's docs on graph memory and its LangGraph integration (user_id scopes all reads and writes, while agent_id and run_id narrow them), plus third-party analyses of the source (update() keeps the scope ids and logs to a SQLite history table). I did not confirm the exact update decision labels, so they are omitted here.
- **Design implication:** Use a three-level scope key on all memory and context rows: workspace or repo_key (≈user), agent (≈agent_id) and session or thread (≈run_id). Retrieval APIs filter on it. Treat LLM auto-extraction of facts as optional and later. If you add it, run it as a background 'reflector' agent on the always-on Windows node that proposes fact or decision events for Stephen or an agent to accept. Never let it silently rewrite memory. That matches the existing KB rule that only deliberate docs get promoted.
- **Sources:** https://docs.mem0.ai/platform/features/graph-memory, https://mem0.ai/library/integrations/how-to-add-memory-to-langgraph-agents, https://instagit.com/mem0ai/mem0/how-does-mem0-handle-memory-updates-and-what-triggers-memory-modifications.md

### LangGraph splits memory in two. A checkpointer holds the full state of one thread_id (short-term, resumable). A BaseStore holds cross-thread long-term memory keyed by namespace tuples, such as (user_id, 'memories'), plus a key. It supports put, search and list_namespaces, optional semantic indexing and TTL, and result order depends on the backend.

- **Confidence:** high
- **Evidence:** From the LangGraph stores docs and the BaseStore reference.
- **Design implication:** Draw the same line in the hub. Per-session state, meaning the full Claude Code transcript, is the 'checkpointer' and stays on the device that ran it, since Claude already persists and resumes it. The hub's replicated store is the cross-thread 'store': hierarchical namespace keys such as (repo_key, 'decisions') or (agent, 'notes'), an FTS5 index first, and embeddings only if needed. Sort explicitly by hlc instead of relying on insertion order across devices.
- **Sources:** https://docs.langchain.com/oss/python/langgraph/stores.md, https://reference.langchain.com/python/langgraph.store/base/BaseStore

### Recent agent event-sourcing designs make the append-only log the source of truth and every view a deterministic projection of it. ESAA has agents emit validated JSON intentions, and an orchestrator appends events and projects state with hash-verified replay, across Claude, Codex and Gemini agents working at the same time. ActiveGraph claims replay, cheap forking at any event, and lineage from goal to model call. Critics point out that replay is really a recording, because model outputs are stored and not re-run.

- **Confidence:** medium
- **Evidence:** ESAA (arXiv 2602.23193, Feb 2026) and ActiveGraph 'The Log is the Agent' (arXiv 2605.21997, May 2026). The critique is secondhand.
- **Design implication:** This supports the plan's (device_id, seq, hlc, actor, kind, payload) log. Make the projections (threads, task state, inbox, presence, the facts and decisions table) disposable and rebuildable from the log, and put a projection version in SQLite so a schema change means a rebuild instead of a migration. 'Replay as recording' is exactly what the hub wants: it never re-runs models, it only records what happened. Validate the payload of every event kind against a schema at write time (as ESAA does), because once a bad row has replicated it can't be fixed in place.
- **Sources:** https://arxiv.org/abs/2602.23193, https://arxiv.org/abs/2605.21997, https://www.developersdigest.tech/blog/log-is-the-agent-event-sourced-ai

### Zed's DeltaDB, which motivates this project, records operations between commits, each with a stable identity, and keeps 'a message and the edit it produced... side by side'. Comments and references are anchored to deltas instead of line numbers, so they survive edits. DeltaDB embeds CRDT worktrees for concurrent editing, and threads are multiplayer: agents and teammates can join by link.

- **Confidence:** high
- **Evidence:** From the Zed blog post 'Software Is Made Between Commits', which describes these as design intentions ahead of the beta.
- **Design implication:** The real lesson from Delta is the link between a conversation turn and the edit it produced, not the CRDT. Make the edit event carry {thread_id, message or turn id (promptId/toolUseId), agent, repo_key, path, base and after blob sha, patch blob}. Anchor comments and decisions to those edit ids, with a fuzzy path+range fallback, as the plan's v1.0 proposes. Keep the plan's decision not to replicate worktrees, because git already moves the code.
- **Sources:** https://zed.dev/blog/introducing-deltadb, https://zed.dev/deltadb

### Measured on this Mac, the Claude Code transcripts in ~/.claude/projects total about 1.6 GB across 761 JSONL files, and about 1.38 GB (696 files) changed in the last 7 days. The median file is about 590 KB, the 90th percentile about 4 MB and the largest about 165 MB, with 8 files over 20 MB. In typical files, human-typed prompts plus assistant reply text are only about 1-5% of the bytes. The rest is tool results, thinking, tool_use inputs, api-request/blob records, attachments and images. Transcripts also contain native file-history-snapshot and file-history-delta entries.

- **Confidence:** high
- **Evidence:** From local measurement. In ~/.claude/projects/-Users-stephenhorton/207837f6-7e68-476a-ae9e-274c870713db.jsonl (1756 KB), user prompts were about 24 KB plus about 62 KB of text blocks, assistant text about 19 KB, and tool_result about 214 KB. In ~/.claude/projects/-Users-stephenhorton-projects-ai-asset-pipeline/076407b7-bf19-4bec-81c0-6b24c220ae13.jsonl (597 KB), message content totalled about 65 KB, of which assistant text was 4 KB. The largest active file, ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl (about 165 MB), had about 1.4 MB of assistant text against about 77 MB of tool_result and about 3 MB of images. It also had 83 file-history-snapshot and 22 file-history-delta entries.
- **Design implication:** Don't eagerly replicate full transcripts. Mirroring them would push over 1 GB a week per machine through iroh, mostly tool output the other side never reads, and it would also copy secrets that tools printed into results. Instead, replicate the high-signal projection eagerly: human prompts, assistant text replies, edit events with patch blobs, commits, task and decision events, and an optional summary per turn. That comes to a few percent of the bytes. Keep a pointer (device, session_id, uuid range) and fetch full slices on demand over iroh, with the always-on Windows node optionally pulling cold copies for backup. Try reading the native file-history-snapshot and file-history-delta entries for edit capture before reconstructing edits only from structuredPatch.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/207837f6-7e68-476a-ae9e-274c870713db.jsonl, ~/.claude/projects/-Users-stephenhorton-projects-ai-asset-pipeline/076407b7-bf19-4bec-81c0-6b24c220ae13.jsonl, ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl

### Every system surveyed separates the conversation record from curated shared context. That holds for A2A (history versus artifacts), LangGraph (checkpointer versus store), Letta (message history versus blocks and MemFS), Anthropic (session context versus memory stores) and Graphiti (episodes versus fact edges). Shared multi-agent state always lives in the curated layer, scoped, access-controlled and attributed. None of them shares raw transcripts between agents as the way agents coordinate.

- **Confidence:** medium
- **Evidence:** This is a synthesis of the sources cited in the other findings: A2A Task.history versus artifacts, LangGraph's stores docs, Letta's shared blocks and MemFS, Managed Agents memory stores and Graphiti episodes.
- **Design implication:** The hub's core schema should have three tiers. (1) An event log: immutable, replicated and attributed (message, task.status, artifact.update, edit, commit.link, presence, delivery). (2) Projections: threads, tasks with the A2A state enum, an inbox and deliveries, an agent registry (cards), and an artifact and edit index. (3) Curated memory: pinned blocks, notes, and decisions and facts with supersession and provenance. An agent arriving in a thread gets tier 3 plus the tier-1 thread messages, never a raw peer transcript, and fetches transcript excerpts by pointer only when it asks.
- **Sources:** https://a2a-protocol.org/latest/specification/, https://docs.langchain.com/oss/python/langgraph/stores.md, https://docs.letta.com/concepts/shared-memory, https://platform.claude.com/docs/en/managed-agents/memory, https://mintlify.com/getzep/graphiti/concepts/temporal-model

### MCP is the only protocol that all of Stephen's agents already speak. Claude Code supports MCP tools plus the claude/channel extension, and grok-build can use MCP or Zed's ACP. A2A targets HTTP-reachable remote agents with Agent Cards, push webhooks and OAuth-style security. None of his agents expose an A2A endpoint, and native ListAgents/SendMessage already cover Claude-to-Claude messaging.

- **Confidence:** high
- **Evidence:** From the channels reference, the A2A spec's methods and push-notification config, the ACP docs, and the facts verified on his machine in the task (peerProtocol:1, SendMessage reaching Remote Control sessions).
- **Design implication:** Interop priority: (1) MCP tools and resources via hub mcp, with the Claude channel extension for push; (2) Zed's Agent Client Protocol for grok-build/rock capture and control; (3) optionally, read-only observation of native SendMessage traffic. Don't build an A2A server in v1. Keep the schema field-compatible with A2A (contextId, taskId, parts, artifacts, TaskState) so an /a2a adapter on the always-on node could be added later if an outside agent ever needs to reach the hub. Use MCP's _meta traceparent convention (SEP-414) to carry the hub's event or thread id across tool calls for correlation.
- **Sources:** https://code.claude.com/docs/en/channels-reference, https://a2a-protocol.org/latest/specification/, https://agentclientprotocol.com/protocol/session-setup

**This sweep's answer on transcript sync:** Don't sync full Claude Code transcripts across devices by default. Sync a compact event projection eagerly and fetch full transcript slices only on demand.

The evidence is from his own Mac. ~/.claude/projects holds about 1.6 GB in 761 JSONL files, and about 1.38 GB changed in the last week. Single transcripts reach about 165 MB (median about 590 KB, 90th percentile about 4 MB). Human prompts plus assistant reply text are only about 1-5% of the bytes. The rest is tool results, thinking, api-request/blob records and images. Mirroring all of it would push over 1 GB a week per machine over iroh, mostly content no other agent reads, and it would spread secrets that tools printed.

Every memory and protocol system surveyed draws the same line. A2A separates history from artifacts, LangGraph its per-thread checkpointer from the cross-thread store, Letta message history from shared blocks, Anthropic session context from memory stores, and Graphiti episodes from facts. Shared state between agents is curated, scoped and attributed, never raw transcripts.

So the plan should replicate the following eagerly:
- messages between agents and from Stephen
- human prompts and assistant text replies as thread events, which are small
- edit events with patch blobs (fetched lazily), linked to promptId/toolUseId and to commits
- task state (A2A-style enum)
- decisions and facts with supersession and provenance
- presence
- optional per-turn summaries

Each session gets a pointer (device, session_id, uuid range) so any device or agent can request a full slice over iroh when needed. The always-on Windows node may optionally pull cold full copies for backup and search, as an opt-in per repo. That is the plan's recommendation #3, now backed by data, with one refinement: assistant reply text and human prompts are cheap enough to replicate in full, so other devices can read the conversation without the tool noise.

## Sweep: Sources and how deeply each was read:
- Read in full through WebFetch:
  - Anthropic, "How we built our multi-agent rese

### The vendor whose transcripts the hub would sync has decided that agents send each other messages, not conversations. Claude Code's cross-session messaging sends only plain text that one Claude writes to another, never the sender's history or files.

- **Confidence:** high
- **Evidence:** Claude Code docs: "A message is a piece of text one Claude writes to another, never the sender's conversation history or files. To move a whole conversation or its context, resume the session instead." They also list 'Plain text only' under limitations. Agent teams follow the same rule: "The lead's conversation history does not carry over."
- **Design implication:** Make the unit an agent receives a message or handoff that the sender wrote, the same model as native SendMessage. Give a full conversation only through 'resume or revive on the device that owns it', never by injecting it into another agent.
- **Sources:** https://code.claude.com/docs/en/cross-session-messaging, https://code.claude.com/docs/en/agent-teams

### Almost all of a Claude Code transcript's bytes are raw tool output. The prose where decisions live is a tiny fraction.

- **Confidence:** high
- **Evidence:** Measured on the 165 MB transcript 78e7ff77-....jsonl:
- tool_result content: about 118 MB, in 1,968 records
- api-request-blob, api-request and api-request-shape records: about 27 MB
- assistant thinking: about 2.9 MB
- assistant tool_use inputs: about 1.5 MB, of which Bash is 1.1 MB and Write is 0.12 MB
- assistant prose: about 0.2 MB
- human and other user records: about 2.3 MB
The ~/.claude/projects folder holds 1.6 GB in 761 files, 26 of them over 10 MB.
- **Design implication:** Sync these three tiers by default: what was said (user prompts and assistant prose), what was done (tool_use name, arguments and edit patches) and decisions. Together they are a few MB per heavy session. Keep tool_result bodies and api-request* records local, as blobs fetched on demand. Capture should only ingest an allow-list of record types (user, assistant, system, attachment metadata), never the JSONL wholesale.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl, ~/.claude/projects

### Giving another agent the full history makes its answers worse. Focused context beats full context on every model family tested.

- **Confidence:** high
- **Evidence:** Chroma tested 18 models. On LongMemEval, focused prompts of about 300 tokens scored significantly higher than full prompts of about 113k tokens for every family, and Claude had the largest gap (no exact percentages are given). A single distractor already lowers accuracy.

Anthropic's context engineering post says recall decreases as context grows, and every token "depletes" an attention budget.
- **Design implication:** The MCP surface should return small, targeted results such as 'what did session X decide about file Y'. It should not dump threads. Paginate and summarise by default, with a hard token budget per tool result (for example 2k tokens) and a cursor to read more.
- **Sources:** https://www.trychroma.com/research/context-rot, https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents

### Anthropic's own multi-agent systems pass condensed summaries and references, not traces. Subagents return 1,000 to 2,000 token summaries and store larger outputs outside the conversation as 'lightweight references'.

- **Confidence:** high
- **Evidence:** Context engineering post: each sub-agent returns "only a condensed, distilled summary of its work (often 1,000-2,000 tokens)." Agents keep "lightweight identifiers (file paths, stored queries, web links)" and load the content just in time.

Research-system post: subagents "store their work in external systems, then pass lightweight references back to the coordinator," which reduces the "game of telephone."
- **Design implication:** Make artifacts-by-reference the core primitive. Every event carries stable ids: (device, session, record uuid), plus a blob hash and a (repo_key, path, sha, range) anchor. Messages and handoffs embed those references, and an MCP tool `hub_fetch(ref)` resolves them lazily, across devices over iroh.
- **Sources:** https://www.anthropic.com/engineering/multi-agent-research-system, https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents

### Cognition's counterpoint still matters: actions carry implicit decisions, and lossy summaries cause conflicting work. Their 2026 update keeps full-context sharing only for narrow cases: forking full context to a stronger 'smart friend' model, and keeping writes single-threaded.

- **Confidence:** high
- **Evidence:** 2025: "Share context, and share full agent traces, not just individual messages" and "Actions carry implicit decisions." In the Flappy Bird example, two subagents built pieces in mismatched styles. Cognition says LLM compression of history into "key details, events, and decisions" is "hard to get right."

2026: the smart friend gets a forked copy of the full context, "a reasonable 80/20 solution." A reviewer works best with no shared context and rediscovers what it needs from the diff. Writes stay single-threaded and the other agents contribute intelligence.
- **Design implication:** The hub should share actions and decisions, not just chat. Record edits with their tool_use ids and prompt ids, and add an explicit decision_record. Offer full context as an on-demand escalation path only: fork or resume the session on its origin device, or stream the transcript to a reviewer agent when asked. Default review flows should give the reviewer only the diff.
- **Sources:** https://cognition.com/blog/dont-build-multi-agents, https://cognition.com/blog/multi-agents-working

### In the MAST taxonomy, which covers 1,642 traces across 7 frameworks, the biggest failure modes are coordination and verification problems that sharing more chat would not fix.

- **Confidence:** high
- **Evidence:** Shares of traces by failure mode:
- Step repetition: 15.7%
- Reasoning-action mismatch: 13.2%
- Unaware of termination conditions: 12.4%
- Disobey task specification: 11.8%
- Incorrect verification: 9.1%
- No or incomplete verification: 8.2%
- Task derailment: 7.4%
- Fail to ask for clarification: 6.8%
- Premature termination: 6.2%
- Loss of conversation history: 2.8%
- Conversation reset: 2.2%
- Ignored other agent's input: 1.9%
- Information withholding: 0.85%

Interventions on ChatDev: a better role spec added 9.4%, and a step that verifies the high-level objective added 15.6%. Inter-agent misalignment is one of MAST's three categories.
- **Design implication:** Spend design effort on these, not on transcript replication:
- Tasks with explicit done criteria and termination conditions.
- Dedupe and 'already done by X' surfacing, to stop step repetition.
- A verify or judge step before a task closes.
- A first-class 'ask for clarification' message kind, routed to Stephen or the owner.
Lost history and withheld information are rare failure modes.
- **Sources:** https://arxiv.org/html/2503.13657

### MetaGPT and ChatDev both found that broadcasting everything overloads agents. Structured documents passed through publish/subscribe, or only each phase's solutions, work better.

- **Confidence:** high
- **Evidence:** MetaGPT: "Sharing all information with every agent can lead to information overload." Agents subscribe by role profile to a shared message pool, and they communicate "through documents and diagrams (structured outputs) rather than dialogue."

ChatDev: sharing "only the solutions of each subtask rather than the entire communication history" reduces overload.

AutoGen group chat is the opposite design: everyone subscribes to one topic and each agent appends every message to its own history.
- **Design implication:** Run the event log as a pub/sub pool. Agents subscribe by repo_key, path globs, thread or task, and get a filtered feed, not a global firehose. Make handoffs typed documents with fixed fields (goal, state, decisions, open questions, refs) rather than free-text recaps.
- **Sources:** https://arxiv.org/html/2308.00352v7, https://arxiv.org/html/2307.07924, https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/group-chat.html

### Models keep structured records more reliably than Markdown. Anthropic's harness for long-running agents uses a JSON feature list plus a progress file and the git log for handoffs between context windows.

- **Confidence:** high
- **Evidence:** The authors chose JSON because "the model is less likely to inappropriately change or overwrite JSON files compared to Markdown files." Each new context window starts by reading claude-progress.txt, the git log and the feature list. Two failure modes are named: declaring victory early, and one-shotting the whole app.
- **Design implication:** Store tasks, decisions and handoffs as schema-validated records behind MCP tools, for example a task status enum and a passes or done field. Do not expose them as editable Markdown. Link commits from .git/logs/HEAD to the records so a fresh session's start-up read is 'open tasks + recent decisions + recent commits'.
- **Sources:** https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents

### On a shared codebase, flat peers coordinating through locks failed. Cursor's 20 agents ran at the effective throughput of 2 to 3. What worked was optimistic concurrency plus a planner, worker and judge hierarchy, with workers that do not talk to each other.

- **Confidence:** high
- **Evidence:** Cursor: "Twenty agents would slow down to the effective throughput of two or three." Locks were held too long or left behind by crashed agents. Optimistic concurrency was "simpler and more robust." Without a hierarchy, agents were risk-averse. Workers "don't coordinate with other workers." Cursor still needs "periodic fresh starts to combat drift and tunnel vision."
- **Design implication:** Make leases advisory, with a TTL and heartbeat, and expire them automatically when a session dies, using sessions/<pid>.json liveness. Prefer version checks such as base sha or blob hash on write and claim over hard locks. Do not encourage open peer chatter between workers. Route through Stephen or a designated lead session, and make 'fresh session from handoff' a one-click action.
- **Sources:** https://cursor.com/blog/scaling-agents

### Same-file parallel edits are a known failure mode, and Anthropic's own guidance is ownership by files.

- **Confidence:** high
- **Evidence:** Agent teams docs: "Two teammates editing the same file leads to overwrites. Break the work so each teammate owns a different set of files." Task claiming uses file locking, tasks can declare dependencies, and the docs recommend 3 to 5 teammates. Teams are not recommended for "sequential tasks, same-file edits, or work with many dependencies."
- **Design implication:** Leases should be keyed by (repo_key, path glob) and shown in the UI. When an agent's edit event touches a path leased to another agent, the hub should raise a conflict warning to both agents, from captured structuredPatch events. Tasks need depends_on fields.
- **Sources:** https://code.claude.com/docs/en/agent-teams

### Multi-agent setups help on parallel work but hurt on sequential work. Without a central check, errors amplify far more.

- **Confidence:** medium
- **Evidence:** Google's 'Towards a Science of Scaling Agent Systems' (v2, from the abstract and summaries):
- Independent agents amplify errors 17.2 times, against 4.4 times with a centralised orchestrator acting as a 'validation bottleneck'.
- Every multi-agent variant tested degraded sequential reasoning tasks by 39 to 70%.
- Centralised coordination improved parallelisable tasks by about 80.8%.
- **Design implication:** The hub should be the validation point, not a mesh. Messages that create or close work should go through the hub, where they are visible and judgeable. Splitting a single sequential task across agents should be discouraged in the UI. Fan-out suits research, review and disjoint files.
- **Sources:** https://arxiv.org/abs/2512.08296, https://research.google/blog/towards-a-science-of-scaling-agent-systems-when-and-why-agent-systems-work/

### Information that arrives piecemeal over many turns degrades LLM reliability. Agents fixate on early assumptions.

- **Confidence:** medium
- **Evidence:** 'LLMs Get Lost in Multi-Turn Conversation' (ICLR 2026 oral) found an average drop of 39% across 6 tasks when instructions were sharded over turns rather than given in one complete prompt. The drop came mainly from unreliability, with early assumptions and premature attempts at a final answer. A claimed 15 to 20% gain from recap or snowball prompting comes only from a secondary blog and is unverified.
- **Design implication:** Do not trickle many small hub messages into a busy session. Batch pending deliveries into one consolidated, fully specified message at the next turn boundary, and allow a message to supersede an earlier queued one. This fits the delivery state machine's 'context added at next prompt' path.
- **Sources:** https://arxiv.org/html/2505.06120v1, https://iclr.cc/virtual/2026/oral/10009147

### Native Claude Code messaging already provides a delivery model the hub should mirror and not fight, but it has limits the hub must fill.

- **Confidence:** high
- **Evidence:** From the cross-session messaging docs:
- Messages arrive between tool calls, or start a turn if the session is idle.
- Each message is delivered, held or refused according to the crossSessionInbound setting and the permission class.
- Up to 100 held messages are kept, and held messages expire after the dialogExpiry deadline, 5 minutes by default.
- About 50 accepted messages can be queued, with loop throttling.
- Messages are capped at about 1M characters.
- Cross-machine messages go through Anthropic servers via Remote Control, which requires a claude.ai sign-in. Without Remote Control on the sender there is no reply address, and offline targets receive the message on reconnect.
- WSL 2 and native Windows sessions on the same computer cannot reach each other.
- notify_when_idle works only on the same machine and expires after 12 hours.
- A message never counts as consent.
- **Design implication:** The hub's own state machine should model 'held' and 'expired' as states, not just queued, delivered, engaged and answered. It should keep messages durably past the native 5-minute and 12-hour limits, and handle grok-build and rock, which native messaging cannot reach. Agents should be told that hub messages are untrusted and never count as consent.

The Windows headless node should run Claude Code natively, not in WSL, if it must exchange messages with native Windows sessions.
- **Sources:** https://code.claude.com/docs/en/cross-session-messaging

### Blackboard-style shared boards, where agents volunteer for posted requests rather than a controller that must know every agent, perform competitively and use fewer tokens. This evidence comes from abstracts only.

- **Confidence:** medium
- **Evidence:** Han and Zhang (2025) used a blackboard where the next agent is chosen from the board's state: competitive with state-of-the-art multi-agent systems at fewer tokens. Salemi et al. (2025, revised 2026): requesters post to the board, agents choose whether to respond, up to 9% relative F1 gain over master-slave baselines.
- **Design implication:** Support 'post to the board' requests addressed to a capability or repo rather than a named session, for example 'anyone in repo X, who knows about Y?'. Idle agents can claim these, which suits the always-on Windows node. Keep a single claimant, via the take-lock, so requests are not done twice.
- **Sources:** https://arxiv.org/abs/2507.01701, https://arxiv.org/abs/2510.01285

### Delegating with vague task descriptions causes duplicate work. Each task needs an objective, an output format, tool or source guidance and boundaries.

- **Confidence:** high
- **Evidence:** Anthropic research system: vague delegation led three subagents to duplicate each other's work. Early versions spawned 50 subagents for simple queries. Cursor: "the prompts matter more" than the harness.
- **Design implication:** Make task and handoff creation a schema with required fields: objective, done-criteria, output format, scope or out-of-scope paths, refs. Check new tasks against open ones for likely duplicates before accepting them.
- **Sources:** https://www.anthropic.com/engineering/multi-agent-research-system, https://cursor.com/blog/scaling-agents

### DeltaDB's real value is durable links from code locations to the conversations and decisions behind them, not shipping whole conversations around.

- **Confidence:** medium
- **Evidence:** Zed describes DeltaDB as operation-level version control with "character-level permalinks that survive any code transformation," letting agents query the assumptions and decisions behind existing code.
- **Design implication:** Prioritise the edit → turn → decision → commit index and an anchored-comment query tool, such as `hub_why(repo_key, path, line)`. That index is small and syncs cheaply. It is where the hub beats raw transcript syncing.
- **Sources:** https://zed.dev/blog/sequoia-backs-zed

**This sweep's answer on transcript sync:** Don't replicate full Claude Code transcripts to every device, and never push them into another agent's context. Make them reachable by reference, fetched on demand.

Three independent lines of evidence support this:
1. **Cost.** On this Mac, ~/.claude/projects holds 1.6 GB. In the largest 165 MB transcript, raw tool output is about 118 MB and api-request* records about 27 MB. The parts that carry meaning are small: assistant prose about 0.2 MB, user prompts about 2.3 MB, tool_use calls with their arguments about 1.5 MB, and thinking about 2.9 MB.
2. **Quality.** Focused context beats full context on every model family (Chroma's context rot study; LongMemEval at about 300 tokens versus about 113k). Multi-turn trickle costs about 39% ('Lost in Multi-Turn'). Anthropic's own subagents hand back 1,000 to 2,000 token summaries.
3. **Precedent.** Anthropic's cross-session messaging deliberately sends "never the sender's conversation history or files", and says to resume the session to move a conversation.

What should replicate by default:
- Messages and handoffs, as typed records.
- Tasks, leases and decisions.
- Edit events with their patches and blob hashes.
- A turn index: user prompts, assistant prose, tool names and arguments, plus ids. This keeps the implicit decisions in actions visible, which is Cognition's concern.

What stays on the device that recorded it:
- Tool_result bodies, thinking, api-request* records and raw JSONL, addressable as (device, session, uuid) and fetched over iroh on request.

Keep an escalation path for the cases where full context does help, such as Cognition's 'smart friend' fork. For those, resume or fork the session on its origin device, or stream the transcript to a reviewer that asks for it. Do not pre-sync it.

This confirms open decision 3 in the plan ('full capture sync off'). It refines it in two ways: sync the prose and actions tier, not just summaries, and capture should use an allow-list of record types.

## Sweep: Full transcripts vs summaries/edits/messages across devices: bandwidth/storage, secrets, cross-device resume, search, co

### Stephen's raw transcript volume is too large to replicate eagerly: 1.63 GB in about 10 days (Oct 16-25), about 160 MB/day on average, with a 555 MB peak day. Median file is 0.6 MB, p90 4.1 MB, p99 20.6 MB, max 173 MB.

- **Confidence:** high
- **Evidence:** Measured read-only: 761 *.jsonl files totalling 1,626,555,077 bytes. 1.43 GB of it was written in the last 7 days. Daily totals ranged from 10 MB to 555 MB (Oct 25).
- **Design implication:** Eager full-transcript replication would push hundreds of MB/day through iroh to every paired device, including the always-on Windows node. Make full transcripts lazy, fetched from the origin device on demand, and never part of the always-replicated event log.
- **Sources:** ~/.claude/projects/**/*.jsonl (measured with find/stat)

### 76% of transcript bytes are subagent and workflow transcripts, not the main conversations the user sees. Main session files total 393 MB; subagents/workflows total 1.235 GB.

- **Confidence:** high
- **Evidence:** Depth-7 paths are ~/.claude/projects/<proj>/<session>/subagents/workflows/wf_*/agent-*.jsonl. One workflow alone has 81 agent files. The 16 main files hold 393,514,129 bytes and the subagent files 1,235,094,989 bytes.
- **Design implication:** Do not replicate subagent transcripts by default. Capture only the condensed result each subagent returns to its parent. Anthropic notes subagents 'return only a condensed, distilled summary of its work (often 1,000-2,000 tokens)'. Keep subagent JSONL as Tier 2 that can be fetched on demand.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/5f71a2ce-86e4-4f01-8526-bf8b64fa6e5c/subagents/workflows/, https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents

### The useful part of a session (human text, assistant text, edits, thinking) is about 0.5% of main-transcript bytes. Tool results dominate.

- **Confidence:** high
- **Evidence:** For the 393 MB of main transcripts: human text 0.87 MB, assistant text 0.57 MB, Edit/Write inputs 0.40 MB, visible thinking 0.27 MB, tool_result content 126.5 MB. Across the whole corpus by record type: user tool_result 47.2%, api-request-blob 28.3%, attachment 9.1%, api-request-shape 3.3%, api-request 1.9%, assistant tool_use 4.9%, assistant text 0.4%, user text 0.7%.
- **Design implication:** Replicating messages, text and edits costs about 2 MB per ~400 MB of transcript, which is under 1 MB/day for Stephen. The text layer is cheap enough to replicate eagerly and index with FTS5 on every device. The bulk (tool output and bookkeeping) stays local.
- **Sources:** ~/.claude/projects (python record-type breakdown, read-only)

### About a third of transcript bytes are harness bookkeeping with no value to a peer or a human: api-request, api-request-blob and api-request-shape records hold copies of system prompts and request shapes. A further 9% are attachment records, which are injected instructions and memory.

- **Confidence:** high
- **Evidence:** An api-request-blob sample contains the repeated system-reminder/MEMORY.md text, and an api-request-shape sample contains the full system prompt. Together they are 33.5% of bytes, and attachments 9.1%.
- **Design implication:** Add a never-sync class (Tier 3) for api-request*, attachment and queue-operation records. Exclude them from search indexes too, or search results fill with system-prompt boilerplate.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/57bc55f1-4326-4edc-94ae-25e4359ff405.jsonl (sampled)

### Claude Code's JSONL format is officially internal and unstable, and new record types are already appearing.

- **Confidence:** high
- **Evidence:** Docs: 'The entry format is internal to Claude Code and changes between versions, so scripts that parse these files directly can break on any release.' Records include recent types such as api-request-blob, atis-latch and turnCompanion keys, and versions 2.1.305 and 2.1.310 are mixed in the corpus.
- **Design implication:** Capture primarily through the mod and hooks (structured events, plus transcript_path from hook input, plus a SessionEnd archive). Parse JSONL only as a tolerant, version-gated fallback. Store transcripts as opaque content-addressed blobs and never make the replicated schema depend on JSONL internals.
- **Sources:** https://code.claude.com/docs/en/sessions

### Transcripts are deleted after 30 days by default, so the plan's 'hub stores pointers and excerpts' design will leave dangling pointers.

- **Confidence:** high
- **Evidence:** Docs: projects/<project>/<session>.jsonl, subagents/, tool-results/ and file-history/ are deleted once older than cleanupPeriodDays (default 30). Stephen's settings.json has no cleanupPeriodDays override.
- **Design implication:** At capture time, copy everything the hub needs to last into the event log or the blob store: messages, decisions, summaries, edit patches with before/after hashes, and commit links. Then give the hub its own retention policy. Per session, optionally snapshot the zstd-compressed JSONL into the origin device's blob store at SessionEnd so a lazy fetch still works after Claude Code's sweep. Do not raise cleanupPeriodDays silently.
- **Sources:** https://code.claude.com/docs/en/claude-directory, ~/.claude/settings.json

### Transcripts are plaintext and are where secrets land. Tool results, the bulk of the bytes, are the main exposure.

- **Confidence:** high
- **Evidence:** Docs: 'Transcripts and history are not encrypted at rest... If a tool reads a .env file or a command prints a credential, that value is written to projects/<project>/<session>.jsonl.' A regex scan of Stephen's corpus found 0 sk-ant/sk-/xai-/GitHub PAT/AWS/private-key/bearer/Discord-token matches, and 5 *TOKEN|SECRET|API_KEY|PASSWORD=value assignments in 4 files. Values were not printed.
- **Design implication:** The Tier 0/1 vs Tier 2 boundary is also the privacy boundary. Dropping tool_result payloads removes most exposure. Run a secret scanner over everything that replicates (edit patches can contain .env writes) and redact before an event is sealed into the append-only log, because log rows are immutable and cannot be scrubbed later. Encrypt Tier 2 blobs at rest on the receiving device.
- **Sources:** https://code.claude.com/docs/en/claude-directory, ~/.claude/projects (regex count only)

### A copied transcript can be resumed directly by path. 'claude --resume <transcript-path>' resumes the session stored at that absolute .jsonl path, and '--resume <session-id>' searches every project on the machine.

- **Confidence:** high
- **Evidence:** Docs table: 'claude --resume <transcript-path> | Resumes the session stored in the .jsonl transcript file at that absolute path.' Also: 'You can run claude --resume <session-id> from any directory... The cross-project search resolves the ID only when exactly one other project holds a transcript with messages for it, so a hand-copied duplicate makes Claude Code report not-found.'
- **Design implication:** Cross-device revive does not need the hub to reverse-engineer the ~/.claude/projects directory naming. Fetch the JSONL into a hub-owned import directory and pass the path. Never place a second copy of the same session ID into ~/.claude/projects on one machine, or resume by ID breaks with not-found.
- **Sources:** https://code.claude.com/docs/en/sessions

### Cross-device resume is still path-dependent: every record stores an absolute cwd, ~/.claude.json keys project state by absolute path, and Windows paths are JSON-escaped, so naive find/replace silently fails.

- **Confidence:** medium
- **Evidence:** Observed top-level keys on records include cwd, gitBranch, sessionId and version. The project directory name is the cwd with non-alphanumerics replaced by '-', so C:\Users\x\repo becomes C--Users-x-repo. The community write-up says find/replace on C:\Users\... matches nothing because JSON stores doubled backslashes, and credentials and the .claude.json projects key do not carry over.
- **Design implication:** For an opt-in 'continue on device B', map cwd through repo_key (root commit or remote) to the local clone path. Rewrite cwd on a parsed copy (JSON-aware, never string replace) and launch from the mapped directory. Make the first version Mac-to-Mac only and treat Mac-to-Windows as experimental behind the Doctor tab. Tool results inside the history will still cite old paths, and the model generally copes with that.
- **Sources:** https://code.claude.com/docs/en/sessions, https://dev.to/gowtham-sai-yadav/why-you-cannot-copy-a-claude-code-session-to-another-machine-2008, ~/.claude/projects/*/*.jsonl (cwd field)

### A resume restores only the post-compaction tail, so a cross-device resume only needs to ship from the last compact_boundary onward. That tail is 2-38% of the file in Stephen's sessions.

- **Confidence:** medium
- **Evidence:** Docs: 'Claude continues with the last compaction's summary, the messages that compaction left unsummarized, and every message since. The messages that the last compaction summarized aren't loaded again.' Measured: 173 MB file → 13.4 MB tail; 109 MB → 2.1 MB; 44.6 MB → 17.0 MB; 3.1 MB → 0.56 MB.
- **Design implication:** The 'resume on another device' fetch should transfer the compact summary plus the tail, not the whole file. That cuts the worst case from 173 MB to about 13 MB, and to about 2 MB after zstd. Keep the pre-compaction history as lazy Tier 2 for human replay only.
- **Sources:** https://code.claude.com/docs/en/sessions, ~/.claude/projects/-Users-stephenhorton/*.jsonl (byte offset of last compact_boundary)

### Resuming the same session ID in two places interleaves both conversations into one transcript, and several launch flags are not restored on resume.

- **Confidence:** high
- **Evidence:** Docs: 'If you resume the same session in two terminals without forking, messages from both interleave into one transcript.' Also: '--mcp-config, --settings, --plugin-dir, --fallback-model, or directories added with --add-dir' must be passed again, and a running background session is attached rather than reloaded.
- **Design implication:** A cross-device revive must always use --fork-session and record a 'forked-from (device, session, seq)' event, so the original keeps a single writer. This fits the plan's single-owner rule for mutable state. Before reviving, the Doctor check should confirm that the target device has the same plugins/mod version, MCP config and add-dirs. Same-device auto-revive (already approved) can resume in place with claude --bg --resume.
- **Sources:** https://code.claude.com/docs/en/sessions

### Resuming a long transcript on a cold cache is expensive, so a fresh session seeded with a handoff card often beats shipping and resuming the full transcript.

- **Confidence:** medium
- **Evidence:** Docs: the 'Resume this conversation?' dialog estimates 'the share of your 5-hour usage limit that the saved conversation alone would use' when resuming after a long break or on a different model. Another device always starts with a cold prompt cache.
- **Design implication:** Make the default cross-device 'pick up this work' a new session on device B whose first prompt is the replicated handoff card (Tier 0), with hub_search/hub_fetch tools for detail. Offer full transcript resume only as an explicit 'continue exactly' action.
- **Sources:** https://code.claude.com/docs/en/sessions

### Claude Code already writes free, high-quality summary records into the transcript: compact summaries (about 16 KB each), away_summary recaps (about 670 B) and AI-generated titles.

- **Confidence:** high
- **Evidence:** Corpus counts: 17 compact summaries totalling 275,617 bytes, 46 away_summary records totalling 30,919 bytes, and 1,791 ai-title records. Sample away_summary: 'Your mlg-farming project is trying to open in the s&box editor... waiting at the splash screen.'
- **Design implication:** Tier 0 can collect these with no LLM calls of its own: replicate the latest title, away_summary and compact summary for each session as 'state' events. Generate the hub's own handoff card only when none is fresh, for example at Stop/SessionEnd or before a delivery to a peer.
- **Sources:** ~/.claude/projects (record counts)

### A peer agent and a human need different granularity. A peer wants a compact, structured record of state. A human wants a replayable timeline with evidence.

- **Confidence:** high
- **Evidence:** Anthropic: good compaction keeps 'key decisions, open bugs, and implementation details' and drops 'redundant tool outputs'. Agents should 'maintain lightweight identifiers (file paths, stored queries, web links)' and 'dynamically load data into context at runtime using tools'. Subagent handoffs are 1-2k tokens.
- **Design implication:** Define a handoff card schema for peers (target 1-2k tokens): goal, current state, decisions (with why), open questions/blockers, files touched (repo_key+path+hash), branch/HEAD, next step, and IDs to fetch. Return it from hub_context(session). Build the human view in the GUI as a timeline (prompts → assistant text → edits with diffs → commits), with tool output expandable through lazy fetch. Never inject the timeline into an agent's context.
- **Sources:** https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents

### Compression helps the lazy tier substantially: zstd -19 shrinks transcripts 4.7-8.7x and gzip 2.4-5x.

- **Confidence:** high
- **Evidence:** Measured on three files: 3.04 MB → zstd 0.65 MB / gzip 1.25 MB; 1.80 MB → 0.21 / 0.36 MB; 1.12 MB → 0.13 / 0.29 MB.
- **Design implication:** Store and transfer Tier 2 blobs as zstd. Even a full snapshot of 10 days of main transcripts (393 MB) is about 50-80 MB, which the always-on Windows node could mirror as an opt-in backup without strain.
- **Sources:** ~/.claude/projects (sample files, compressed to stdout only)

### Search value sits in the text layer, which is small enough to index everywhere. Full-text search over raw transcripts mostly returns tool-output and system-prompt noise.

- **Confidence:** medium
- **Evidence:** Human plus assistant text is about 1.4 MB per 393 MB of main transcript, while 33.5% of bytes are api-request boilerplate and 47% tool output (see earlier findings).
- **Design implication:** Replicate and index (FTS5) prompts, assistant text, decisions, summaries, edit file paths and commit messages on every device. For tool-output search, do a remote query on the origin device ('search on device X') rather than replicating its index.
- **Sources:** ~/.claude/projects (measured)

**This sweep's answer on transcript sync:** Do not sync full Claude Code transcripts by default. Replicate a small structured layer eagerly and fetch everything else lazily from the device that recorded it. Stephen's own data makes the case. He produced 1.63 GB of JSONL in about 10 days, and 76% of it is subagent/workflow transcripts. In the main sessions, the human text, assistant text and edits that a peer or a search actually needs are about 0.5% of the bytes. The rest is tool output (47%, which is also where secrets land) and harness bookkeeping (about 33% api-request* records). The JSONL format is officially internal and unstable, and Claude Code deletes it after 30 days, so the hub must copy what it needs at capture time, not keep pointers.

Recommended tiers and defaults:

- **Tier 0 (always replicate, all devices, a few KB/day):** presence, messages and deliveries, tasks/leases, decisions, open questions, handoff cards, session title, away_summary and the latest compact summary, edits as patches with before/after git hashes, and commit→turn links. Secrets are scanned and redacted before an event is sealed.
- **Tier 1 (always replicate, FTS5-indexed, under 1 MB/day):** per-turn user prompts and assistant text, and tool-call metadata (name, file path, exit code) without the payloads.
- **Tier 2 (lazy, fetched on demand from the origin device, cached with a TTL, zstd):** tool results, thinking, subagent transcripts, file-history snapshots, and the full JSONL. The origin device snapshots each session's JSONL into its own blob store at SessionEnd, so a fetch still works after Claude Code's 30-day sweep. The always-on Windows node may opt in to mirroring Tier 2 as a backup (about 50-80 MB per 10 days compressed).
- **Tier 3 (never sync):** api-request/api-request-blob/api-request-shape, attachment and queue-operation records.

Peers and humans get different views. A peer agent gets the 1-2k-token handoff card through hub_context, plus hub_search/hub_fetch tools. A human gets a replayable timeline in the GUI, with Tier 2 expanding on click.

Cross-device continuation defaults to starting a fresh session on the other device, seeded with the handoff card. That avoids a cold-cache resume of a huge context. The opt-in "continue exactly" action works like this:

1. Fetch only the last compact_boundary onward, which is 2-38% of the file in Stephen's sessions (for example 173 MB → 13 MB).
2. Rewrite cwd through the repo_key mapping with a JSON-aware rewrite, never a string replace.
3. Launch with `claude --resume <imported-path> --fork-session`.
4. Before launching, the Doctor tab checks plugins, MCP config and add-dirs on the target.

Treat Mac→Windows as experimental. Same-device auto-revive can resume in place.

## Sweep: I surveyed shipped products for how they share context between agents and humans. For each product the questions were: w

### Zed Delta's unit of sharing is the thread: conversation plus agent plus worktree, replicated together. DeltaDB records every edit and conversation between commits inside the existing git repo, so non-Delta users still see a normal repo. Comments attach to live lines and also to conversation text, plan steps and thinking blocks. They stay anchored as the code changes. Threads are private until shared. Claude Code sessions sync live into a thread.

- **Confidence:** high
- **Evidence:** Zed blog: threads are private until shared; a shared thread carries conversation and code; 'Every edit and conversation' between commits; comments stay 'anchored to the code as it evolves'; comments can target conversation text, plan steps or thinking blocks. Its stated lesson is that tools 'shouldn't hide or truncate output'; diffs open in full and transcripts stay whole.
- **Design implication:** Make the hub's primary object a Thread: session plus its edits plus its commits plus its messages, with git as the bridge. Let anchored comments target transcript turns and plan steps as well as code ranges, which extends the plan's v1.0 anchors (repo_key, path, sha, range). Default threads to private on the originating device, with sharing as an explicit act. The plan already keeps worktree sync out of scope; that matches Delta's 'git stays the bridge'.
- **Sources:** https://zed.dev/blog/introducing-delta

### Claude Code's native cross-session messaging deliberately sends only plain text, 'never the sender's conversation history or files'. It caps message size, rate-limits, holds or refuses by permission class, and labels every message as coming from another session, not the user. The docs point to resume when a whole context needs to move.

- **Confidence:** high
- **Evidence:** cross-session-messaging doc: 'A message is a piece of text one Claude writes to another, never the sender's conversation history or files. To move a whole conversation or its context, resume the session instead.' About 1M-character cap on the same machine; the receiver queues at most 50 accepted messages; at most 100 held; notify_when_idle expires after 12h and only works same-machine from the main conversation. Default policy holds messages sent to a bypass-permissions receiver. Messages can't approve prompts or change config. Cross-machine messages go through Anthropic servers via Remote Control and carry no reply address unless the sender is RC-connected.
- **Design implication:** Follow the same split. Messages are small fenced text, which the plan's <hub-message trust=untrusted> already mirrors. Whole-context transfer is a separate, explicit handoff or revive operation, never stuffed into a message. Reuse native SendMessage for Claude-to-Claude traffic, as the plan's 'what NOT to build' says. The hub's real value is the gaps native lacks: no Remote Control or API-key requirement, grok-build and rock as peers, durable inbox past 12h and 100 held messages, cross-machine idle notices, and an audit history.
- **Sources:** https://code.claude.com/docs/en/cross-session-messaging.md

### Claude Code teammates and subagents get a spawn prompt plus project context (CLAUDE.md, MCP, skills), not the lead's history. Coordination runs through a shared task list with dependencies and file-lock claiming, plus per-agent JSON mailboxes. Idle notifications carry the final answer. Documented limits: task status lags, /resume doesn't restore teammates, there is one team per session, and the lead is fixed.

- **Confidence:** high
- **Evidence:** agent-teams doc: 'The lead's conversation history does not carry over.' Mailbox at ~/.claude/teams/{team}/inboxes/{agent}.json; tasks at ~/.claude/tasks/{team}/ with pending, in-progress and completed states and dependencies; 'Task claiming uses file locking'; 'Teammates sometimes fail to mark tasks as completed, which blocks dependent tasks.' Best practice: put task-specific detail in the spawn prompt.
- **Design implication:** The hub's Tasks and Leases should treat agent self-reported status as untrusted. Reconcile it against observed evidence (turn.complete, commits, edits) and flag drift, which the native task list cannot do. Observe ~/.claude/tasks and inboxes passively and don't write to them. When the hub wakes or spawns an agent, generate an explicit briefing: goal, relevant files, decisions, links. Don't assume history carries over.
- **Sources:** https://code.claude.com/docs/en/agent-teams

### Cognition's foundational lesson is to share full agent traces, not just messages, because actions carry implicit decisions. Its fix for long horizons is a dedicated compression step that produces key details, events and decisions. Devin's handoff packages repo and branch, `git diff HEAD` (truncated to 100KB), and 'whatever the calling agent has learned so far: files it examined, root-cause hypotheses, partial fixes.'

- **Confidence:** high
- **Evidence:** Cognition: 'share context, and share full agent traces, not just individual messages'; 'conflicting decisions carry bad results'; it suggests compressing 'a history of actions & conversation into key details, events, and decisions.' The Devin handoff doc lists the diff cap and extra-context examples; it runs local to cloud and is callable from Claude Code and Codex.
- **Design implication:** Give the hub a first-class Handoff packet: repo_key and branch, a working diff with a size cap, files touched, hypotheses, decisions, open questions, and a link back to the full source thread. Keep the full trace reachable, which argues against the plan's 'never leaves the device'. Record decisions as their own events (the plan's decision_record), because conflicting implicit decisions are the main failure mode when parallel agents share a repo.
- **Sources:** https://cognition.com/blog/dont-build-multi-agents, https://docs.devin.ai/work-with-devin/devin-handoff

### Amp replaced compaction with Handoff. You state a goal; Amp reads the old thread and drafts a new-thread prompt plus a list of relevant files, which you can edit before sending. Thread mentions (@T-… or a URL) make the agent read another thread and pull out what matters for the current task, instead of loading it whole.

- **Confidence:** high
- **Evidence:** Amp: compaction is 'lossy'; whether the summary keeps the right things 'is up to the agent'; it 'encourages long, meandering threads'; 'focused threads... is how agents yield the best results.' The threads doc covers handoff and @T- mentions, plus Private, Workspace, Group and Unlisted levels; full JSON export is limited to the creator, and others get Markdown.
- **Design implication:** Reference by pointer and extract on demand. An agent asking for thread X should get a goal-conditioned extract built from the full source, not a pre-made generic summary. Build `hub_thread_read(thread, goal)` that runs extraction near the data, on the origin device or always-on node, and returns an extract plus file list. Show handoff drafts to Stephen for editing before they go out.
- **Sources:** https://ampcode.com/news/handoff, https://ampcode.com/docs/threads

### Amp removed public, discoverable thread sharing on 2026-06-02. Its reason was that reviewing a thread for snippets of sensitive files had become too hard, since agents now read more files into context.

- **Confidence:** medium
- **Evidence:** Release note quote: 'It's getting too hard to review a thread to ensure it doesn't contain any snippets of sensitive files.' Existing public threads became Unlisted.
- **Design implication:** Raw transcripts are high-risk payloads. Even on Stephen's own paired devices, run a redaction pass at capture (env-var values, .env reads, known token formats) before anything replicates. Never replicate raw tool_result bodies by default. Pairing must be revocable, as the plan already says.
- **Sources:** https://www.havoptic.com/r/amp-end-of-public-threads

### Stephen's own transcripts are large and dominated by tool output and images. The conversational spine is a small fraction.

- **Confidence:** high
- **Evidence:** ~/.claude/projects is 1.6 GB across 761 JSONL files; 1.38 GB was written in the last 7 days. In the largest file (179 MB, 15,924 lines), 'user' entries are 126 MB, tool_result parts 79 MB, of which tool results with images are 76 MB; api-request-blob 22.5 MB; assistant 13.3 MB. Assistant text parts are 1.5 MB, tool_use 2.2 MB and thinking 3.3 MB, about 7 MB together or roughly 4% of the file.
- **Design implication:** Eagerly replicating raw JSONL would move about 1.4 GB per week per device, mostly images. Replicate a normalized spine instead: user prompts, assistant text, tool_use names and args, edit patches, and truncated tool-result heads. That is about 5% of the bytes. Store the rest as content-addressed blobs fetched on demand.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl, ~/.claude/projects/

### A replicated raw Claude Code JSONL can be revived directly on another machine: `claude --resume <transcript-path>` resumes from a .jsonl at an absolute path. But the entry format is internal and 'changes between versions', and resuming one session in two terminals without forking interleaves both into one transcript.

- **Confidence:** high
- **Evidence:** sessions doc table: 'claude --resume <transcript-path> | Resumes the session stored in the .jsonl transcript file at that absolute path.' 'The entry format is internal to Claude Code and changes between versions, so scripts that parse these files directly can break.' 'If you resume the same session in two terminals without forking, messages from both interleave into one transcript.' Compacted sessions resume from the last summary; summarized messages aren't reloaded.
- **Design implication:** This is the case for a third tier: the full JSONL fetched lazily from the origin or always-on node only when Stephen chooses revive-on-this-device. Cross-device revive must use --fork-session, or take the hub lease and stop the origin, so the transcript never forks silently. Check CC versions before revive. For search and extraction, parse into the hub's own normalized schema at capture so a format change only breaks the capturer.
- **Sources:** https://code.claude.com/docs/en/sessions.md

### Remote Control already syncs full Claude Code transcripts across devices by storing them on Anthropic servers while connected. Execution stays local. It allows one remote session per interactive process, and the session goes offline when the local process stops.

- **Confidence:** high
- **Evidence:** remote-control doc: 'While Remote Control is connected, the session transcript, including your messages, Claude's responses, and tool activity, is stored on Anthropic servers... Execution and filesystem access stay on your machine.' Outbound HTTPS only. Limits: one remote session per process outside server mode; the local process must keep running; /resume and /teleport don't push earlier history to the connected device.
- **Design implication:** For Claude sessions with RC on, cross-device viewing and steering already exist. The hub should not try to be a second live transcript mirror. Link to the RC session instead and avoid capturing twice. The hub's iroh path serves things RC doesn't: grok-build and rock, API-key sessions, no third-party storage, multi-agent indexing and search, and the always-on node.
- **Sources:** https://code.claude.com/docs/en/remote-control.md

### Factory measured compression strategies on more than 36k production messages. Artifact trail (file paths, which files changed) was the weakest dimension for every method, scoring 2.19 to 2.45 out of 5. Anchored iterative summarization scored best: summarize only the newly truncated span and merge it into a persistent structured summary with intent, file modifications, decisions and next steps.

- **Confidence:** high
- **Evidence:** Factory: 'Artifact trail is the weakest dimension for all methods'; Factory 3.70 vs Anthropic 3.44 vs OpenAI 3.35 overall (vendor's own eval); 'The right optimization target is not tokens per request. It is tokens per task.'
- **Design implication:** Never derive the artifact trail from LLM summaries. The hub captures it mechanically: edits with patches and blob hashes, files read, and commits. That is the plan's capture layer, and it is exactly what summaries lose. Session summaries should be anchored and incremental (intent, files, decisions, next steps), updated per turn-batch, with file lists filled from capture rather than generated.
- **Sources:** https://factory.com/news/evaluating-compression, https://docs.factory.ai/guides/power-user/evaluating-context-compression

### Claude Code compaction drops summarized history from context but keeps it on disk. Afterward it re-injects CLAUDE.md, auto memory, the plan-mode plan, skill bodies (capped) and up to 5 recently touched files, and runs SessionStart hooks that match source=compact.

- **Confidence:** high
- **Evidence:** context-window doc, 'What survives compaction' table, including 'SessionStart hooks that match the compact source: Claude Code runs them and adds their output to the compacted context.' The sessions doc says the original transcript stays on disk.
- **Design implication:** Use a SessionStart(compact) hook, or the mod equivalent, to re-inject the hub's live state after compaction: open inbox items, held leases, the active task and the thread's anchored summary. Otherwise agents silently forget hub obligations mid-session. Since the full pre-compaction history is still on disk, the hub's capture must read the JSONL, not the post-compaction context.
- **Sources:** https://code.claude.com/docs/en/context-window.md, https://code.claude.com/docs/en/sessions.md

### Anthropic's multi-agent research system has subagents write outputs to external storage and pass lightweight references back, which avoids a lossy 'game of telephone'. The lead saves its plan to Memory because context over 200k gets truncated. Fresh subagents keep continuity through careful handoffs. Token usage explained 80% of performance variance.

- **Confidence:** high
- **Evidence:** 'Subagents call tools to store their work in external systems, then pass lightweight references back to the coordinator.' 'Saves its plan to Memory to persist the context.' 'spawn fresh subagents with clean contexts while maintaining continuity through careful handoffs.' The synchronous lead is a bottleneck.
- **Design implication:** Make artifacts first-class: an agent posts a finding or report as a stored blob and messages a reference (hub://artifact/…), not a 1 MB paste. That avoids the telephone effect and the message caps. Delivery should be async (inbox plus wake), never a synchronous RPC that blocks the sender on another agent.
- **Sources:** https://www.anthropic.com/engineering/multi-agent-research-system

### GitHub Copilot's agentic memory stores each fact with citations to code locations and validates them at read time against the current branch. On contradiction, the agent re-stores a corrected memory. Memories are shared across Copilot agents (coding agent, code review, CLI). An A/B test showed PR merge rate rising from 83% to 90%, and seeded adversarial memories were neutralized by citation checks.

- **Confidence:** high
- **Evidence:** GitHub blog: memories 'with citations: references to specific code locations'; the agent 'verifies the citations in real-time'; 'store a corrected version'; '90% with memories vs. 83% without'; 'The citation verification mechanism robustly prevented the risk of misleading memories.' Memories are written only by users with write access.
- **Design implication:** Shared notes and decisions in the hub should carry anchors (repo_key, path, sha, range, hash) and be re-validated when served. A note whose anchor no longer matches is shown as stale, not served as fact. This reuses the plan's anchored-comment machinery for the notes and decision store, and it limits poisoning by one agent's wrong conclusion. It also fits Stephen's rule that the curated KB stays human-promoted only.
- **Sources:** https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/, https://docs.github.com/en/copilot/concepts/agents/copilot-memory

### Cursor supports local-to-cloud agent handoff (prefix with &, resume on web or mobile). Cloud-to-local only works when a local agent for the repo already exists. Otherwise users check out the branch and 'lose the entire chat history'. Cursor staff call this a known limitation that is not being built out.

- **Confidence:** medium
- **Evidence:** Cursor CLI changelog Jan 16, 2026: 'Push your local conversation to a Cloud Agent... Prepend & to any message.' Forum thread from June 4-8, 2026, Cursor 3.6.31: staff call it 'a known limitation, not a bug'; cloud-to-local for cloud-origin sessions 'isn't being actively built out right now'.
- **Design implication:** Moving a branch without its conversation is the common failure, so design handoff to carry both: branch or diff plus thread link plus handoff packet. Make it symmetric across devices, Mac to Windows node and back. Treat 'continue this on the other machine' as a v0.4 acceptance test.
- **Sources:** https://cursor.com/changelog/cli-jan-16-2026, https://forum.cursor.com/t/cloud-origin-agents-have-no-move-to-machine-option-in-agents-window-glass-cant-hand-off-to-local/162470

### Parallel-agent managers (Conductor, Sculptor) isolate each agent in its own worktree or container. Shared context is filesystem-level and manual: Conductor gives each workspace a gitignored .context folder for notes, plans, attachments and handoffs, and its docs say to 'copy the useful context into the winning workspace.' Sculptor uses Pairing Mode to sync an agent's container changes locally for testing, then merge or discard.

- **Confidence:** medium
- **Evidence:** Conductor parallel-agents doc: workspaces are isolated (branch, files, env, processes); agents in one workspace share context; cross-workspace transfer is manual copy. Conductor 0.28.1 changelog (Dec 23, 2025): new workspaces have .context, where attachments, plans and notes are stored. Sculptor details come from AlternativeTo; Imbue's mngr-pair (unison sync) is an unconfirmed link.
- **Design implication:** There is a gap no shipped manager fills: cross-workspace context propagation ('what landed in sibling worktree B that affects me'). The hub's per-repo_key event stream can tell every session on the same repo across worktrees and devices when a sibling commits or records a decision. Treat worktree isolation as the default for parallel Claude sessions, and the hub as the context bus between them.
- **Sources:** https://www.conductor.build/docs/core/parallel-agents, https://www.conductor.build/changelog/0.28.1-context-directory-set-working-directories, https://conductor.build/docs/concepts/git-worktrees, https://www.alternativeto.net/software/sculptor/about/, https://pypi.org/project/imbue-mngr-pair/

### Cloud agents (GitHub Copilot coding agent, OpenAI Codex cloud, Google Jules) converge on one handoff unit for humans: a task that yields a branch, diff or PR, plus a reviewable session log. Copilot ties each session to a draft PR whose timeline links 'View session' logs, which can be streamed via `gh agent-task --log --follow`. Codex shows diffs and test results, then supports follow-ups or a PR. Jules shows a plan for approval before acting and runs a critic pass before the human sees code (secondary sources).

- **Confidence:** medium
- **Evidence:** GitHub docs: session logs are reachable from the PR timeline's 'Copilot started work' event and contain reasoning and tool use; the agent works on a copilot/ branch and draft PR. Codex docs: 'Each task has its own workspace'; inspect changed files, request follow-ups, commit or open a PR. Jules: plan approval (2025 coverage) and a critic that reviews the full generation (Google Developers Blog, Aug 2025). The Codex claim that cloud tasks can't read uncommitted local files is blog-only.
- **Design implication:** Link each commit, branch and PR to the session or thread that produced it, and make 'open the session behind this commit' one click. That is the plan's .git/logs/HEAD linkage. Session logs are for humans to audit, so render them in the UI. Agents get extracts, not the raw log. Consider a plan-approval gate as a hub task state for risky cross-agent requests.
- **Sources:** https://docs.github.com/en/copilot/using-github-copilot/coding-agent/using-the-copilot-coding-agent-logs, https://learn.chatgpt.com/docs/cloud, https://developers.googleblog.com/ko/meet-jules-sharpest-critic-and-most-valuable-ally/, https://hipertextual.com/2025/05/google-jules-agente-ia-programacion-autonoma-io-2025

### Durable, human-curated memory (CLAUDE.md, AGENTS.md, Devin Knowledge and Playbooks) is a separate layer from session context in every product. Claude Code auto memory is per repository, shared across worktrees, loads the first 200 lines or 25KB, and lives under ~/.claude/projects/<project>/memory/ on the local machine. Devin Knowledge items carry trigger descriptions for retrieval, and playbooks can be distilled from shared session links.

- **Confidence:** medium
- **Evidence:** memory doc: auto memory 'Per repository, shared across worktrees'; 'first 200 lines or 25KB'; CLAUDE.md is shared via source control. Sessions doc: auto memory under projects/<name>/memory/. Devin docs: Knowledge is retrieved by trigger, and you can turn successful sessions into playbooks by sharing session links.
- **Design implication:** Auto memory is per machine, so Mac and Windows Claude sessions on the same repo learn different things. The hub can surface and diff per-device auto-memory for the same repo_key and offer a human-approved merge or promote, never an automatic write; this respects the KB-only-deliberate rule. Keep three layers distinct: messages (ephemeral), thread context (synced spine plus extracts), durable memory (curated, anchored).
- **Sources:** https://code.claude.com/docs/en/memory.md, https://code.claude.com/docs/en/sessions.md, https://docs.devin.ai/work-with-devin/advanced-capabilities

**This sweep's answer on transcript sync:** Don't choose between full transcripts and summaries only. Sync in three tiers, based on what shipped products learned and on Stephen's own data.

**Tier 1: replicate eagerly to every paired device**
This is a normalized "spine" parsed at capture into the hub's own schema, so a change in Claude Code's internal JSONL format only breaks the capturer. It contains:
- user prompts
- assistant text
- tool_use names and arguments
- edit patches with before/after blob hashes
- commit links
- messages and decisions
- truncated tool-result heads
- an anchored, incremental summary (Factory-style: intent, files, decisions, next steps) where the file list comes from capture, not the LLM

A redaction pass runs before replication. In the measured 179 MB transcript the spine is about 4-5% of the bytes. This tier gives search, timelines, cross-device awareness and the input for goal-conditioned extraction.

**Tier 2: content-addressed blobs, fetched on demand**
These are full tool outputs and images, which made up 76 MB of that 179 MB file. They are pulled from the origin device or the always-on Windows node only when a human or agent opens them. The node can also cache them so they stay reachable when the laptop sleeps.

**Tier 3: the full raw JSONL, fetched only for an explicit "revive here"**
`claude --resume <transcript-path>` can revive a .jsonl directly, so the full transcript should be reachable, not stranded on its origin device. Revive must use --fork-session, or take the hub lease and stop the origin, because resuming one session in two places interleaves the transcript. It also needs matching Claude Code versions.

**What agents receive**
Agents never get raw transcripts pushed into their context. They get messages (small fenced text, like native SendMessage, which deliberately excludes history) and handoff packets or goal-conditioned extracts (Amp handoff and @T- mentions, Devin handoff), each with a link back to the full source. That satisfies Cognition's "share full traces" by keeping them reachable, without Amp-style context bloat.

**Why not raw-JSONL sync**
Volume is about 1.38 GB per device per week, mostly images. Sensitivity is the other reason: Amp killed public threads because transcripts can't be reviewed for leaked file contents, and two of Stephen's recent transcripts contain the string CLAUDE_CODE_MESSAGING_TOKEN; I didn't check whether a token value was logged. Also, Remote Control already mirrors full Claude transcripts through Anthropic servers when it is on. The hub should link to the RC session rather than duplicate it, and use its own iroh path for Grok, rock, API-key sessions and serverless storage.

**Change to the plan**
This revises §9.3. Instead of "full capture off, only summaries, edits and messages replicate", it becomes: spine replicates, blobs and full JSONL are fetchable on demand from the origin or the always-on node, and nothing raw is ever injected into an agent's context.

## Sweep: Empirical, read-only sweep of Stephen's Mac. Covered all 761 Claude Code transcripts under ~/.claude/projects (16 root s

### Raw transcripts grow at roughly 160 MB/day on average and peaked at 335 MB on 2026-10-25. Two root sessions are 173 MB and 109 MB on their own, and their subagents add another 458 MB and 478 MB.

- **Confidence:** high
- **Evidence:** There are 1633 MB across 761 JSONL files in 10 days. Bytes per day by record timestamp: 10-18 163 MB, 10-20 164 MB, 10-21 165 MB, 10-23 128 MB, 10-25 335 MB. Root sessions 78e7ff77 and 57bc55f1 each ran about 4 days, with 242 and 209 subagent files. Subagent files total 1235 MB, 76% of all bytes.
- **Design implication:** Do not replicate raw JSONL over iroh. At about 5 GB/month per device it would swamp the log and the always-on Windows node. Treat a transcript as a local file the hub points to with (device, path, byte offset), not as replicated content.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl, ~/.claude/projects/-Users-stephenhorton/57bc55f1-4326-4edc-94ae-25e4359ff405.jsonl, (local scratch)/research/strip.py, (local scratch)/research/sessions.json

### Almost all transcript bytes can be dropped without losing meaning. Raw 1633 MB becomes 100 MB once images, thinking signatures and API-request blobs are removed, 42 MB when compressed, 12 MB as conversation text only, and 1.7 MB as main-thread conversation (human plus assistant prose).

- **Confidence:** high
- **Evidence:** strip.py output: raw 1633.3, stripped 100.3, stripped zlib 42.4, conversation 12.0, main conversation 1.7 (MB). Those are 6% / 2.6% / 0.7% / 0.1% of raw. Stripped per day is 4 to 22 MB.
- **Design implication:** Use tiered replication. Always replicate the conversation and event tier (MBs per day). Replicate the stripped text tier compressed, with a TTL or on demand. Never push the raw tier eagerly. The event log schema should store these tiers separately.
- **Sources:** (local scratch)/research/strip.py

### Images are 93% of tool-result bytes, nearly all from the Read tool on PNG renders. Half of them are duplicates.

- **Confidence:** high
- **Evidence:** Image tool results: 4003 results, 937 MB (93%). Text tool results: 22,930 results, 73 MB. Bytes by tool: Read 925 MB, Bash 61 MB, blender look 17 MB. There are 4575 base64 image occurrences (1069 MB) but only 2142 distinct images (499 MB).
- **Design implication:** Store images content-addressed: record a hash, the source path and a thumbnail, and fetch the full image lazily over iroh blobs. The asset-pipeline workflow produces many visual checks, so the hub UI should show thumbnails, not embedded base64.
- **Sources:** ~/.claude/projects/**/*.jsonl, (local scratch)/research/textresults.txt

### Thinking blocks contain nothing another agent could use, and api-request-blob records duplicate content already in the transcript.

- **Confidence:** high
- **Evidence:** There are 9,806 thinking blocks. Only 1,201 have readable text, and 19.7 MB is opaque signatures. api-request-blob is 16% of main-session bytes and 32% of subagent bytes. It is a hashed copy of request payloads, including system reminders and MEMORY.md. api-request-shape and attachment records are another 5 to 15%.
- **Design implication:** The capture layer should allowlist record types (user, assistant, system/away_summary, compact summaries, pr-link, file-history-delta, queue-operation notifications) rather than tailing whole files. Drop thinking signatures, api-request*, attachment and permission/mode records at ingest.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/5f71a2ce-86e4-4f01-8526-bf8b64fa6e5c.jsonl (record types sampled), (local scratch)/research/sessions.json

### Most text tool output is small. Capping each text tool result at about 4 KB roughly halves the text tier.

- **Confidence:** high
- **Evidence:** Text tool results have a median of 1,047 bytes, p90 of 8.5 KB, p99 of 27 KB and a max of 68 KB. The total is 73 MB, or 38 MB with a 4 KB cap per result.
- **Design implication:** In the replicated tier, keep the head and tail of each tool result plus its byte length and a pointer to the full text on the origin device. The full text is fetched only on demand.
- **Sources:** (local scratch)/research/textresults.txt

### A short brief, not the transcript, is what carries work between sessions and devices. Both long sessions were started by a pasted brief of about 3 KB, and human steering afterwards was tiny.

- **Confidence:** high
- **Evidence:** 78e7ff77 opened with a 3,041-character pasted brief ('I'm continuing work on my asset pipeline... Two things to pick up'). 57bc55f1 opened with a 3,466-character brief that mentions 'What's on the branch (from a Windows session, which couldn't run Hunyuan)', which is a manual handoff across devices. Over 4 days, 78e7ff77 has 81 prompts and 15 KB of typed text against 173 MB of transcript. The median prompt in history.jsonl is 58 characters, and only 2 of 278 prompts used paste.
- **Design implication:** Make the handoff brief a first-class, versioned object in the hub. It should hold the goal, the branch/SHA, constraints, what's done, what's next and do-not rules. Generate it on session end or compaction, and let a new session on any device start from it in one click. This replaces Stephen's copy and paste between machines.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl (first human record), ~/.claude/projects/-Users-stephenhorton/57bc55f1-4326-4edc-94ae-25e4359ff405.jsonl (first human record), ~/.claude/history.jsonl

### Claude Code already writes summaries the hub can harvest instead of generating its own: away_summary (about 230 characters) and compaction summaries (12 to 24 KB).

- **Confidence:** high
- **Evidence:** 78e7ff77 has 15 away_summary records averaging 228 characters and 8 compact boundaries with summaries of 12.7 to 16.8 KB. 57bc55f1 has 14 away summaries and 7 compactions of 12.5 to 24 KB. One sample away summary: 'the Holstein cow is made, rigged, and waiting for your critique. Next, tell me what to fix...'
- **Design implication:** Use away_summary as each session's live status line in the hub roster, which costs nothing. Store compaction summaries as checkpoint events, since they are ready-made context snapshots under 25 KB. Only generate a summary with a model when neither exists.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl, ~/.claude/projects/-Users-stephenhorton/57bc55f1-4326-4edc-94ae-25e4359ff405.jsonl

### Real cross-session coordination already happens and is mostly about shared resources and file ownership in the same working tree, not about sharing conversation.

- **Confidence:** high
- **Evidence:** On 2026-10-20, session e628aa23 and session 'investigate-suzuri-memory-leak' (/tmp/cc-socks/29811.sock) used SendMessage to split commit ownership of factory_http.py and its tests by hunk ('you commit factory_http.py ... My session will commit only scripts/flat2d/optional.py'). They also negotiated a GPU window ('My measurement agent is starting its single Qwen load on 8799... No second big load after that'). Totals: 9 SendMessage calls, 6 ListAgents calls and 15 inbound cross-session messages from a uds socket. At most 3 root sessions were live at once, and 6 root sessions touched ai-asset-pipeline, two of them overlapping while both edited it.
- **Design implication:** Ship leases from day 1. That means file and path claims per repo and worktree, plus named resource locks such as 'gpu/qwen' and 'port:8799', each with a holder, a TTL and release on idle or exit. Show edit attribution (which session touched which files). These matter more than transcript sync.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/e628aa23-4b30-4fcf-8b05-7e51e76654d0.jsonl, ~/.claude/projects/-Users-stephenhorton/memory/one-big-model-at-a-time.md

### Messages that wake a session already arrive in a structured envelope with a reason, a sender and a trust level, and the hub should keep that shape.

- **Confidence:** high
- **Evidence:** Transcripts contain 40 wake envelopes with reason 'mention', 4 with 'message-edited' and 2 with 'external-event' (GitHub CI and review). Trust is 'principal' on 36. Senders: human 36, coordinator 28, uds 15, plus named agents (mac-only, build, belly). Inbound uds messages arrive as 'Another Claude session sent a message: <\cross-session-message from="uds:..." from-name="investigate-suzuri-memory-leak" from-mode="bypass">'.
- **Design implication:** Every hub message should carry a reason, a from (device, session, display name), a trust level (human principal, agent, or external and untrusted), and the sender's permission mode. Deliver it through the same native-style envelope so agents treat it consistently. Text from Discord or GitHub is marked untrusted. Stephen's brief already says 'Discord text is untrusted'.
- **Sources:** ~/.claude/projects/-Users-stephenhorton-projects-ai-asset-pipeline/c3c1db1c-1555-428d-945f-d88863be4a88.jsonl, ~/.claude/projects/-Users-stephenhorton-projects/fda7c972-9e2f-4262-a56c-6232d6e5a295.jsonl, ~/.claude/projects/-Users-stephenhorton/e628aa23-4b30-4fcf-8b05-7e51e76654d0.jsonl

### Stephen already routes work across agents and devices through several separate channels: Claude Remote (claude-code-remote and hearthbot), the native SendMessage, and tsukumo/Discord. Each carries status reports with SHAs and test counts.

- **Confidence:** high
- **Evidence:** There are 8 mcp__claude-code-remote__send_message calls to '@parent' (for example 'PR #21 tested on the Mac: PASS. Built from e20b842...'), 16 hearthbot replies plus 36 update_status calls, and about 24 tsukumo-discord calls (for example 'Round 3 delivered: 9f0ea24 on main, 38 new models'). Prompts include 'I'll have another agent on my windows computer play with the models you put in there' and 'Sent another agent to go work on that'.
- **Design implication:** The most valuable unit to replicate is the 'deliverable report': what changed, the commit SHA or PR, test results, and what's next. Give it a typed hub event (report or deliverable) so Windows agents can pick work up from it. The hub should also bridge to or replace these three channels so there is one inbox per agent.
- **Sources:** ~/.claude/projects/-Users-stephenhorton-projects-ai-asset-pipeline/c3c1db1c-1555-428d-945f-d88863be4a88.jsonl, ~/.claude/projects/-Users-stephenhorton-projects-ai-asset-pipeline/076407b7-bf19-4bec-81c0-6b24c220ae13.jsonl, ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl, ~/.claude/history.jsonl

### Stephen rarely refers to other agents directly (13 of 278 prompts). When he does, it is about delegating to another device or agent, or noting missing context.

- **Confidence:** medium
- **Evidence:** 13 of 278 typed prompts mention other agents, sessions or devices. Examples: 'toru is another one of my github repositories, sad that we don't have that context with the current research going on', 'Actually, why proxy through discord? Let's just ship directly to the mlg-farming repository', 'I'll have another agent on my windows computer play with the models'.
- **Design implication:** Coordination should be agent-driven by default, with low friction for Stephen. Agents should discover peers and repo context themselves, for example by checking `gh repo list` and the hub's repo index, rather than waiting for him to relay. The UI should optimize for glanceable status, not chat.
- **Sources:** ~/.claude/history.jsonl

### Context is re-derived mostly inside one session family (subagents and workflows), not across sessions or devices.

- **Confidence:** medium
- **Evidence:** Of 1,156 file reads, 672 (58%) repeat a file already read in the same root session and its subagents. Only 7 files were read in 2 or more different root sessions, all in ai-asset-pipeline. 54 files were read by 3 or more distinct agents. hassei/src/hassei/dev/flesh.py was read by 49 agents and face.py by 47, both inside 830a5598's workflows.
- **Design implication:** A shared per-repo 'map' (a file digest keyed by path and git blob SHA, with a short summary) served over MCP would cut the biggest redundancy, which is fan-out subagents re-reading the same code. Cross-device transcript sync would not. Treat this cache as an optional later optimization; it is not the core of the hub.
- **Sources:** (local scratch)/research/sessions.json, ~/.claude/projects/-Users-stephenhorton/830a5598-0e77-4808-8b54-35a98f9309b9/subagents/

### The transcript directory does not identify the project. 757 of 761 transcripts sit under -Users-stephenhorton because sessions start in ~ and then cd across repos, and per-project memory dirs are empty.

- **Confidence:** high
- **Evidence:** -Users-stephenhorton has 757 files, -projects-ai-asset-pipeline 3 and -projects 1. The memory dirs for -projects and -projects-ai-asset-pipeline are empty. All 16 memory files (21 KB) are under the home dir. Session 2d10c58a touched knowledge-base, rts-sbox, suzuri and others, and 57bc55f1 mentions 40 distinct repos including its subagents.
- **Design implication:** Assign events to a project per event from the record's cwd, gitBranch and touched paths, mapped to a git remote (owner/repo). Do not use the transcript's folder. Repo identity should be the git remote so the Mac and Windows checkouts merge into one project.
- **Sources:** ~/.claude/projects/, ~/.claude/projects/-Users-stephenhorton/memory/MEMORY.md, (local scratch)/research/agg.py

### The durable knowledge Stephen trusts across agents is tiny and curated: 16 Claude memory files (21 KB), a 28-entry knowledge base (276 KB) he keeps deliberately, and Grok memory.

- **Confidence:** high
- **Evidence:** There are 16 memory .md files, 21.4 KB in total. ~/projects/knowledge-base/entries has 28 entries (276 KB), and memory note kb-permanent-docs-only says 'never add entries unasked'. Grok memory has 299 files (21 MB), mostly an index.sqlite with 2,365 chunks (2,275 session, 74 global, 15 dream).
- **Design implication:** Keep three layers separate. (1) Curated memory and knowledge base, which humans own and which replicate everywhere; the hub should never auto-write it. (2) Generated summaries and briefs, which the hub owns. (3) Raw transcripts, which stay local. Offer memory rules as shared, read-mostly context that applies to Claude and Grok alike.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/memory/, ~/projects/knowledge-base/INDEX.md, ~/.grok/memory/stephenhorton-81449290/index.sqlite

### Grok-build already has much of what the hub needs: a local peer bus, full-text session search, per-session markdown summaries and a vector memory index. Its liveness data is stale.

- **Confidence:** high
- **Evidence:** ~/.grok/bus has roster.json (94 sessions, last_active as old as Sept), live/ (48 socket files, many from Sept 18 and 30) and mail/. ~/.grok/sessions/session_search.sqlite holds 110 session docs in an FTS table averaging 8.5 KB each, against 1.6 GB of raw Grok sessions. The 202 per-session summaries have a median of 2.5 KB and a p90 of 11.8 KB. Claude, by contrast, has exactly 2 sockets in /tmp/cc-socks matching 2 live PIDs in ~/.claude/sessions.
- **Design implication:** Write a Grok adapter that reads Grok's summaries and FTS docs and its bus mail rather than its raw sessions. Use the same per-session summary schema of about 2 to 10 KB for both agents. Never trust socket files or a roster alone for liveness; confirm with a PID check or heartbeat, as the Claude session files do with status and updatedAt.
- **Sources:** ~/.grok/bus/roster.json, ~/.grok/bus/live/, ~/.grok/sessions/session_search.sqlite, ~/.grok/memory/stephenhorton-81449290/sessions/, ~/.claude/sessions/, /tmp/cc-socks/

### tsukumo's authorship ledger is the one routing record already used in production: it records which session made which PR or message so follow-ups reach that session.

- **Confidence:** high
- **Evidence:** ~/projects/tsukumo/.ledger.json has 14 entries (9 'author' entries on PRs, 5 'poster' entries on Discord messages) across 3 sessions, dated 10-21 to 10-25. ledger.ts keeps entries for 45 days and ranks author and poster above touched and reviewer. Claude Code transcripts natively emit 'pr-link' rows (prNumber, prRepository), which tsukumo scans.
- **Design implication:** Make ownership edges first-class in the hub: session to PR, branch, commit or message. Build them from pr-link rows and the hub's own capture, and keep them longer than the session itself. Route GitHub and CI events and replies to the owning session, reviving it if needed, before falling back to a front desk.
- **Sources:** ~/projects/tsukumo/.ledger.json, ~/projects/tsukumo/src/lib/ledger.ts, ~/.claude/projects/-Users-stephenhorton/57bc55f1-4326-4edc-94ae-25e4359ff405.jsonl (pr-link records)

### Stephen's own retrospective on pairing two agents gives hard constraints: an idle poll must never wake a session, one writer per cursor, stop means delete, and a shared GitHub identity cannot tell agents apart.

- **Confidence:** high
- **Evidence:** From slack-dual-ai-pairing-what-worked.md: 'The 5-minute poll woke the human session'. 'Main session and poll subagent both wrote ~/.grok/grok-pair-state.json... One writer, or a lock, is required'. 'Two sessions of Stephen's Grok dual-claimed the same human bug'. 'Claim-before-code, disjoint leftovers' worked. 'Extra Slack ACK is ping-pong'.
- **Design implication:** The delivery state machine should be push-based: wake only on an addressed message or event, never on a poll that finds nothing. Use single-writer cursors per agent in the log. Give each agent its own identity (device plus session plus name) used in claims. Make claims atomic and visible, suppress ACK-of-ACK messages, and make 'stop' revoke the subscription.
- **Sources:** ~/projects/knowledge-base/entries/slack-dual-ai-pairing-what-worked.md

### Stephen has already written down that continuity, project state and live chat are separate layers, and that P2P chat must not become the store for project state.

- **Confidence:** medium
- **Evidence:** p2p-conversation-pairing-after-transfer-handshake.md lists three layers: (1) the project's lined-up work in the repo (docs/NOW.md, AGENTS.md), (2) session and memory state across computers ('closer to sync a directory than to a product'), and (3) live agent-to-agent conversation. It says 'Layer 2 is not a reason to build layer 3. Layer 3 is not a substitute for layer 1', and 'Do not use P2P chat as the store for lined up for Yardscale'.
- **Design implication:** The hub should write project state back into the repo, for example a handoff brief committed to docs/NOW.md, instead of keeping it only in the hub. Its own log should hold layers 2 and 3 only.
- **Sources:** ~/projects/knowledge-base/entries/p2p-conversation-pairing-after-transfer-handshake.md

### No confirmed secrets were found in the transcripts, but agents do run commands against .env and credential files. Pattern scanning alone is not enough protection.

- **Confidence:** medium
- **Evidence:** I scanned for 15 secret patterns (Anthropic, OpenAI, xAI, GitHub, AWS, Google, Slack, Discord bot, Stripe, JWT, private-key blocks, HF, ENV=secret, Bearer). There were 22 'AIza' Google-key matches across 10 distinct strings, all inside base64 image payloads (false positives). The 4 'service_role' matches were this sweep's own script. No keyword=value secrets were found. There were 9 Bash calls touching .env or credential files, for example 'grep -c DISCORD_TOKEN= ~/projects/tsukumo/.env', and agents redacted output on their own ('sed s/=.*/= <redacted>/').
- **Design implication:** Redact at ingest, before anything is replicated. Drop or mask the full tool result whenever the tool input references .env, auth.json, mcp_credentials or similar, whatever the result contains, and run a pattern scanner too. Exclude base64 blobs from scanning to avoid false positives. Keep raw transcripts device-local so any leak stays on one machine.
- **Sources:** (local scratch)/research/secrets.json, (local scratch)/research/sec2.py

### Long sessions compact often, and compaction is when context gets lost. In 2 long sessions there were 16 compaction boundaries, roughly one every 6 to 12 hours of work.

- **Confidence:** high
- **Evidence:** There were 8 compact_boundary records in 78e7ff77 (4 days) and 7 in 57bc55f1 (4 days), plus 1 each in 2d10c58a and 830a5598. The KB retrospective notes that after compaction 'The agent re-kicks protocol, forgets last_ts, or re-claims'.
- **Design implication:** Keep claims, cursors and subscriptions in the hub, never in the agent's context. On a compaction or SessionStart hook, re-inject a small 'your hub state' block (open claims, unread messages, current brief) so a compacted or revived session continues where it left off.
- **Sources:** ~/.claude/projects/-Users-stephenhorton/78e7ff77-6171-4137-b5ba-e17a2e16b6c5.jsonl, ~/projects/knowledge-base/entries/slack-dual-ai-pairing-what-worked.md

**This sweep's answer on transcript sync:** No. The data argues against syncing full Claude Code transcripts across devices. Replicate a stripped, typed event tier, and leave the raw transcript on the device that made it, with a pointer to it.

Numbers from this Mac (2026-10-16 to 10-25, 761 files):
- Raw transcripts are 1,633 MB (about 160 MB/day, peaking at 335 MB/day).
- 93% of tool-result bytes are base64 images, mostly Read on PNG renders, and half of those are duplicates.
- api-request-blob duplicates are 16 to 32% of bytes.
- Thinking is 20 MB of opaque signatures with almost no readable text.
- Removing images, signatures and request blobs leaves 100 MB (42 MB compressed). Main-thread conversation is only 1.7 MB.

What actually moved work between sessions and devices was much smaller:
- Hand-pasted handoff briefs of about 3 KB, one of them written from a Windows session.
- Claude Code's own away_summaries (about 230 characters) and compaction summaries (12 to 24 KB).
- Deliverable reports carrying SHAs and test counts, sent through Claude Remote, hearthbot and Discord.
- Direct SendMessage negotiation over file ownership and the GPU.
- Cross-session re-reading of files was nearly zero (7 files). Re-reading inside one session's subagents was 58%.

Recommended tiers:
- **Always replicate:** messages, claims and leases, ownership edges (from pr-link rows, commits and PRs), edit events (file path, git SHA, which session), away and compaction summaries, handoff briefs, and deliverable reports. This is a few MB per day.
- **Replicate compressed, with a TTL or on demand:** user and assistant text, tool_use inputs, and text tool results truncated to about 4 KB each with their full length recorded (about 4 to 10 MB/day).
- **Keep local, fetch lazily over iroh:** raw JSONL and images, referenced by (device, path, offset) and content hash.
- **Never sync:** thinking signatures, api-request*, attachment and permission records, and any tool result whose input touched .env or credential files.

This keeps everything that carried an outcome. The hunk-ownership negotiation and the Discord deliverable posts are both user/assistant text and survive stripping. It also follows Stephen's own rule in the knowledge base that syncing sessions is "closer to sync a directory than to a product", and that project state belongs in the repo.

Note that Grok already does the same thing: it keeps about 2.5 KB per-session summaries and about 8.5 KB FTS session docs against 1.6 GB of raw sessions. The hub can use one shared summary schema for both agents.

## Claims refuted or unconfirmed by fact-checkers

- **[research]** Cognition's counterpoint still matters: actions carry implicit decisions, and lossy summaries cause conflicting work. Their 2026 update keeps full-context sharing only for narrow cases: forking full context to a stronger 'smart friend' model, and keeping writes single-threaded.
  - *Fact-check:* The quotes are accurate, but the summary of the 2026 post misstates what it concludes. Both posts are by Walden Yan.

What checks out:
- 2025 post: it has both principles, "Share context, and share full agent traces, not just individual messages" and "Actions carry implicit decisions, and conflicting decisions carry bad results." The Flappy Bird example matches: one subagent built a Mario-style background and the other a bird that doesn't fit. It also says LLM compression into "key details, events, and decisions" "is hard to get right."
- 2026 post ("Multi-Agents: What's Actually Working", 2026.04.22): it does say "a reasonable 80/20 solution is to just share a fork of the full context of the primary model with the smart model." The coding agent and the reviewer work best when they "do not share any context beforehand." It also says "multi-agent systems work best today when writes stay single-threaded and the additional agents contribute intelligence rather than actions."

What is wrong:
1. The 2026 post does not limit full-context sharing to narrow cases. Its "Refresher on Context Engineering" section restates "Share as much context as possible between the agents... stay on the same page (todo list, plan files)..." and adds "the need for thoughtful context engineering has not [changed]." What it does narrow is which multi-agent patterns work: "a narrower class of patterns... where multiple agents contribute intelligence to a task while writes stay single-threaded." The parallel-writer critique "still hold[s] today for parallel-writer swarms."
2. "Keeping writes single-threaded" is not a case of sharing full context. It is a separate rule about who is allowed to write.
3. The claim leaves out the post's third working pattern: a manager that coordinates child agents "without fragmenting decisions."
4. Minor: the post says the reviewer reasons "backward from the implementation without the spec." "Rediscovers from the diff" is a fair paraphrase, not a quote.

Accurate restatement: In 2026 Cognition keeps both 2025 principles as general guidance. Its finding is that multi-agent setups work when only one agent writes and the others add intelligence. Three patterns fit that: a smart-friend model given a forked copy of the full context (the 80/20 solution), a reviewer given deliberately no shared context, and a manager coordinating child agents. Swarms of parallel writers still fail.
- **[research]** In the MAST taxonomy, which covers 1,642 traces across 7 frameworks, the biggest failure modes are coordination and verification problems that sharing more chat would not fix.
  - *Fact-check:* The numbers check out against arXiv 2503.13657 (HTML). It covers 1,642 traces across 7 frameworks. Its three categories are FC1 System Design Issues, FC2 Inter-Agent Misalignment and FC3 Task Verification. All 13 percentages match the paper's FM-1.1 to FM-3.3 figures. The ChatDev results are +9.4% and +15.6%, though the paper is inconsistent about the +9.4%: the introduction credits it to giving the CEO agent the final say, while Section 4 credits it to better role specifications.

What fails is the framing. The biggest failure modes are mostly not coordination or verification problems. Three of the top four belong to FC1 System Design / specification: step repetition 15.7%, unaware of termination conditions 12.4%, and disobey task specification 11.8%. Only reasoning-action mismatch (13.2%) is in FC2, the coordination category. Summing the paper's figures by category gives FC1 about 44%, FC2 about 32% and FC3 about 23.5%, so design/specification is the largest category and verification is the smallest. The paper itself never says which category is largest.

A defensible version would read: "MAST (1,642 traces, 7 frameworks) finds failures dominated by system-design/specification problems (~44%), then inter-agent misalignment (~32%) and verification (~23.5%). Modes caused by missing context are rare: loss of conversation history 2.8%, conversation reset 2.2%, information withholding 0.85%. The ChatDev interventions that helped were better role specs (+9.4%) and a high-level objective-verification step (+15.6%)."

Whether "sharing more chat would not fix" these failures is the claimant's inference, not something the paper states. The low rates for history-loss and withholding make it plausible. The figures may also be shares of failure-mode occurrences rather than shares of traces, which I did not confirm.
- **[protocols-memory]** Two different 'ACP's exist. Zed's Agent Client Protocol (agentclientprotocol.com) connects an editor to an agent. Its session/update notifications stream user_message_chunk, agent_message_chunk, thought_message_chunk, tool_call, tool_call_update, plan and mode_change, and session/load must replay the entire conversation as session/update. IBM's Agent Communication Protocol (BeeAI) is a separate protocol that was merged into A2A under the Linux Foundation in August 2025, and new work should target A2A.
  - *Fact-check:* The main point holds: two different ACPs exist. Zed's Agent Client Protocol connects an editor to an agent. IBM/BeeAI's Agent Communication Protocol merged into A2A under the Linux Foundation (LF AI & Data); Tyk dates the announcement to August 2025, and IBM's page says "ACP is now part of A2A under the Linux Foundation" without a date. New work should target A2A. Several details in the claim are wrong:
(1) Two session/update variant names are invented. The prompt-turn docs list user_message_chunk, agent_message_chunk, agent_thought_chunk (not "thought_message_chunk"), tool_call, tool_call_update, plan, available_commands_update, current_mode_update (not "mode_change"), config_option_update, session_info_update, usage_update, notice, compaction_update and compaction_summary_chunk.
(2) The v1 session-setup page does say "The Agent MUST replay the entire conversation to the Client" in the form of session/update notifications (user_message_chunk / agent_message_chunk) on session/load. The v2 session-setup page does not mention session/load or that sentence. It uses session/resume, which by default restores context without replaying history. So the evidence's "(v1/v2)" attribution of the MUST-replay quote is wrong for v2.
(3) "v2 adds session/resume and session/close" is misleading. The v1 session-setup page already documents both, gated by the sessionCapabilities.resume and sessionCapabilities.close capabilities. In v2 they are required for agents that support the session method surface.
For the hub: a stored ACP stream would use agent_thought_chunk and current_mode_update. Under v1, a full transcript is only needed to support session/load replay.
- **[own-sessions]** Thinking blocks contain nothing another agent could use, and api-request-blob records duplicate content already in the transcript.
  - *Fact-check:* The byte figures hold up. The thinking half of the claim does not, and the blob half is "nearly all", not "all".

**Byte shares are confirmed.** Re-running the sweep's agg.py on sessions.json gives the same split. In main sessions, api-request-blob is 16.4% of bytes, attachment 3.5% and api-request-shape 1.4%. In subagent sessions, api-request-blob is 32.0%, attachment 10.8% and api-request-shape 3.9%.

**Thinking blocks: refuted.** A rescan of all 791 transcripts under ~/.claude/projects/ found:
- 14,045 thinking blocks (the count has grown since the sweep's 9,806).
- 1,577 of them (11%) contain readable text, about 0.45 MB in total.
- 29.7 MB is opaque signature data.

So most thinking bytes are useless signatures, but the readable ones are not empty. They hold real working reasoning and facts another agent could use, for example:
- "the download is proceeding at roughly 6MB/s, meaning the full 33GB could take around 90 min"
- "at DPR 2 those 4 device-pixel runs actually correspond to only 2 CSS pixels"
- what to check next, such as "check pixel-look.js to understand how the low-res upsampling is verified"

The sweep's own number (1,201 readable) contradicts "nothing". The accurate version: thinking is about 89% signature-only and its bytes are about 98% signature. Drop the signatures and keep the small readable text, or summarize it, instead of treating all thinking as worthless.

**api-request-blob: mostly holds.** The blobs hash-copy request payloads: the system prompt, environment, system-reminders with MEMORY.md, and user turns. For 19,368 of the blob text blocks, the first 150 escaped characters also appear in a non-blob record of the same file, often in api-request-shape, which is itself a request-copy record. The blocks that did not match total 0.56 MB of 37.5 MB of blob text, about 1.5%. They include system-only injections that appear nowhere else in the transcript, such as "# Environment update ... working directory changed", "The user hasn't heard from you in a while" nudges and `<\total_tokens>` notes. A few user prompts also did not match, possibly because of escaping or a forked session. So blobs are about 98–99% duplicate and safe to drop for syncing, but they are not a pure duplicate.

**Not verified:** whether attachment records duplicate anything. Attachments make up 3.5% of main-session and 10.8% of subagent bytes, and the claim lumps them in with api-request-shape without showing they are redundant.
- **[sync-tradeoffs]** About a third of transcript bytes are harness bookkeeping with no value to a peer or a human: api-request, api-request-blob and api-request-shape records hold copies of system prompts and request shapes. A further 9% are attachment records, which are injected instructions and memory.
  - *Fact-check:* The description of what the records contain is right, but the percentages are wrong. I re-measured the whole cited file (~/.claude/projects/-Users-stephenhorton/57bc55f1-4326-4edc-94ae-25e4359ff405.jsonl, 108,701,511 bytes, last modified Oct 21) by summing the bytes of each JSONL line by its top-level type. The api-request-blob records are 17.7% of bytes (19.29 MB), api-request records are 2.0% (2.22 MB) and api-request-shape records are 1.2% (1.30 MB). Together that is about 20.9%, roughly a fifth, not 33.5%. Attachment records are 3.5% (3.76 MB), not 9.1%. Looking only at the start of the file does not reproduce the claimed numbers either: the api-request family plus attachments come to 22.5% + 2.0% over the first 1,000 lines, 20.4% + 2.0% over the first 2,000 and 13.2% + 1.5% over the first 5,000. The content description does check out. An api-request-shape record holds the full system prompt ("You are Claude Code..."), and an api-request-blob record holds the repeated system-reminder text (the userEmail context). Attachment records hold injected environment and system-reminder snapshots. The largest share of bytes is user records at 65.6% (71.3 MB); these are probably mostly tool results, though I did not break them down. Assistant records are 8.7%. Corrected statement: about 21% of this transcript's bytes are harness bookkeeping (the api-request, api-request-blob and api-request-shape records, which copy system prompts and request shapes), and about 3.5% are attachment records holding injected context.
- **[sync-tradeoffs]** Claude Code's JSONL format is officially internal and unstable, and new record types are already appearing.
  - *Fact-check:* Only the first half holds. The docs quote is real and word-for-word. On https://code.claude.com/docs/en/sessions, under "Where transcripts are stored", it says: "The entry format is internal to Claude Code and changes between versions, so scripts that parse these files directly can break on any release. To build on session data, use /export or the script interfaces instead." The same page lists those interfaces: claude -p --output-format json/stream-json, claude -p --resume, the hook/statusline transcript_path field, a SessionEnd hook, and the Agent SDK.

The second half, "new record types are already appearing", is not supported by the evidence. The cited types do exist in ~/.claude/projects/**/*.jsonl: "type":"api-request-blob" appears 24,101 times, "type":"atis-latch" 1,990 times and a "turnCompanion" key 234 times. But nothing shows they are new. The api-request-blob and atis-latch lines have no "version" field, so they can't be dated by version. The turnCompanion key appears in every version in the corpus, including 2.1.299, the oldest one. The corpus probably only covers the 30-day default retention window, so it has no baseline to say a type appeared recently.

On mixed versions, the evidence understates it. The corpus mixes 10 versions: 2.1.299, 301, 302, 304, 305, 306, 307, 308, 309 and 310. 2.1.310 has the most lines (24,884) and 2.1.308 has one. So it isn't just 305 and 310.

Supportable restatement: Anthropic's docs officially call the JSONL entry format internal and say it changes between versions. The local corpus mixes about 10 Claude Code versions written within the retention window and contains many undocumented internal record types, such as api-request-blob, atis-latch, deferred_tools_record and prompt_snapshot. A hub should therefore not depend on parsing the raw JSONL. It should use hooks/transcript_path, claude -p --output-format stream-json, or the Agent SDK, or keep a tolerant parser that skips unknown types.
