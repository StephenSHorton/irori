package api

// v0.2 routes (docs/design/v0.2-talk.md §7, §8, §10). CONTRACT for the api,
// mod, mcpserver and GUI work packages: paths, headers and JSON shapes below
// are binding. All routes keep v0.1's rules (bearer token, Host check, no
// Origin). Errors are {"error": "<text>"} with a 4xx/5xx status.
//
// Caller identity on POST routes comes from exactly one header:
//
//	X-Irori-Attach: <attachSecret from hello>   -> the attached Claude session
//	X-Irori-Member: <memberSecret from join>    -> the bound Grok member
//
// A request with neither (or an unknown/superseded secret) is 403 on every
// route that acts as an agent. The body never names the caller.
//
//	GET  /v1/attach?session=<sessionId>&mod=<version>
//	     NDJSON stream of attach.Event, keepalive "\n" every 15 s. First line is
//	     hello (with attachSecret). A second attach for the same session sends
//	     superseded to the older stream and closes it. On connect the server
//	     calls Inbox.SetReachable(key, mod, true) and then NextWake once; on
//	     every WakeReady for this agent it calls NextWake and emits deliver.
//	     On disconnect: SetReachable(key, mod, false).
//	     The session must be a live presence card on this device (404 otherwise;
//	     `irori attach` retries, since presence may lag a new session by ~2 s).
//	     There is no cwd parameter (`irori attach --cwd` is only logged).
//	     A wake issued to an earlier stream but never confirmed delivered is
//	     re-sent on the next stream with the SAME token: the mod and mcp must
//	     dedupe deliver events on token. A confirmed wake is never re-sent.
//	     inbox events omit unread/held when they are 0 (omitempty): readers
//	     treat a missing count as 0.
//
//	GET  /v1/attach?member=<memberSecret>
//	     The same stream for a bound Grok member (path channel). deliver events
//	     carry the fenced prompt; the member's `irori mcp` turns each into one
//	     notifications/claude/channel push. Nothing is marked delivered.
//	     Its hello has session "grok-<id>" (the memberKey's last segment) and
//	     no attachSecret. There is no ack route of its own: the member acks a
//	     push with POST /v1/tools/ack (or tools inbox/reply). An unacked push
//	     stays Queued until the inbox sweeper reclaims it after ReattachGrace;
//	     each reclaim pushes it again under a new token, so expect repeats.
//
//	POST /v1/members              {"name": "...", "session": "<optional client session id>"}
//	     -> {"memberKey": "<device>/grok-<id>", "memberSecret": "...", "name": "..."}
//	     Binds a Grok member (the Core adds a synthetic presence card with
//	     Agent "grok" while its stream is open plus inbox.ReattachGrace after
//	     it closes; a join with no stream yet lists no card). The same
//	     session id rejoining gets the same memberKey. Grok cards come from
//	     Core.Agents(), not presence.Subscribe.
//
//	POST /v1/deliveries/delivered {"token": "[irori-wake:...]", "appended": false} (attach only) -> {"ok": true}
//	POST /v1/deliveries/engaged   {"token": "...", "turnId": "...", "appended": false} (attach only) -> {"ok": true}
//	     appended: true for a wake the mod put in as an append after its
//	     host refused the submit (card #23). Delivered: no prompt waits in
//	     the host's queue, so it does not gate the next wake (sent again
//	     when the wake was reported delivered while its submit was pending).
//	     Engaged: no running wake, so a message from that turn has hop 0.
//	POST /v1/deliveries/turn-ended {"token": "...", "turnId": "..."} (attach only) -> {"ok": true}
//
//	Next-turn delivery (board.md "Contracts") adds NO routes. A mod stream
//	whose mod= version is at least attach.AppendSince also gets append
//	events: on connect after the deliver re-send it sends
//	Inbox.OutstandingAppend (same token) or else NextAppend, and on every
//	change for its agent (AppendReady among them) and each keepalive tick,
//	as catch-up, it calls NextAppend and emits append. A deliver carries
//	mode "next_turn" only on a Grok push of next_turn items alone, and
//	overflow when some ids didn't fit in the prompt. The mod confirms an append with the routes
//	above: delivered {token} once $.session.append resolved, engaged {token,
//	turnId} at the next main-loop turn.start. turn-ended with an append
//	token is accepted and ignored. Older mods get no append events.
//
//	GET  /v1/tools?client=claude|grok   -> []tools.Spec
//	POST /v1/tools/{name}  {"args": {...}}  -> {"text": "..."}  (attach or member)
//	     Tool-level failures are 200 with text starting "Error: ".
//
//	GET  /v1/policy                -> inbox.Policy
//	POST /v1/policy  inbox.Policy  (attach only, and X-Irori-Origin: composer) -> inbox.Policy
//	     Recorded with by and actor "api:composer@<agentKey>" (never "user": only the GUI makes user rows, v0.4 §2).
//
//	GET  /v1/inbox?all=1&limit=N   (attach or member) -> []inbox.Delivery for the caller
//	     (held deliveries never appear; same rule as Inbox.For)
