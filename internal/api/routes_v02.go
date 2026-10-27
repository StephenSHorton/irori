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
//
//	GET  /v1/attach?member=<memberSecret>
//	     The same stream for a bound Grok member (path channel). deliver events
//	     carry the fenced prompt; the member's `irori mcp` turns each into one
//	     notifications/claude/channel push. Nothing is marked delivered.
//
//	POST /v1/members              {"name": "...", "session": "<optional client session id>"}
//	     -> {"memberKey": "<device>/grok-<id>", "memberSecret": "...", "name": "..."}
//	     Binds a Grok member (the Core adds a synthetic presence card with
//	     Agent "grok" while its stream is open plus inbox.ReattachGrace). The same
//	     session id rejoining gets the same memberKey.
//
//	POST /v1/deliveries/delivered {"token": "[irori-wake:...]"}      (attach only) -> {"ok": true}
//	POST /v1/deliveries/engaged   {"token": "...", "turnId": "..."} (attach only) -> {"ok": true}
//	POST /v1/deliveries/turn-ended {"token": "...", "turnId": "..."} (attach only) -> {"ok": true}
//
//	GET  /v1/tools?client=claude|grok   -> []tools.Spec
//	POST /v1/tools/{name}  {"args": {...}}  -> {"text": "..."}  (attach or member)
//	     Tool-level failures are 200 with text starting "Error: ".
//
//	GET  /v1/policy                -> inbox.Policy
//	POST /v1/policy  inbox.Policy  (attach only, and X-Irori-Origin: composer) -> inbox.Policy
//
//	GET  /v1/inbox?all=1&limit=N   (attach or member) -> []inbox.Delivery for the caller
//	     (held deliveries never appear; same rule as Inbox.For)
