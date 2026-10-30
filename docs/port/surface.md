# Port spec: the outer surface (API, MCP, tools, CLI, GUI services, updater, release)

Source: `../irori-go` (Go 1.26, Wails v3 alpha.98). All `file:line` refs are into that repo.
Scope: `internal/api`, `internal/mcpserver` (+ `fallback_tools.json`), `internal/tools`, `internal/core`, `internal/update`,
`internal/diag`, `internal/paths`, the root package (`*_service.go`, `gui.go`, CLI subcommands), `irori.json` (internal/discovery),
`crash.log`, the mod in `mod/`, `frontend/` binding usage, `.github/workflows/release.yml`, `build/windows/nsis`, `build/darwin`.
Neighbouring specs: `store.md` (event log, `store.Event`, kinds), `inbox.md` (inbox, wakes, policy). Capture, coord, mesh, pair,
identity, presence types are only named here; their JSON shape comes from their own areas.

---

## 1. What this area does, and its interfaces

One binary `irori` (`irori.exe`) with roles chosen by the first non-flag argument (main.go:39-72):

| cmd | role | notes |
|---|---|---|
| `""`, `gui` | desktop app: Core in-process + tray + webview | `--startup` = login launch, window hidden (settings_service.go:15,33; gui.go:140) |
| `serve --headless` | Core, no UI | `serve` without `--headless` is an error (main.go:133-135); Ctrl+C stops (main.go:140) |
| `mcp` | stdio MCP server proxying the loopback API (Grok, other non-Claude agents) | mcp.go:22-48 |
| `attach --session <id> [--cwd X] [--mod-version V]` | the Claude Code mod's NDJSON wake stream on stdout | attach.go:25-57 |
| `events [--follow] [--kinds a,b] [--json] [--limit N=20] [--since SEQ=-1]` | print/follow the log | events.go:37-73 |
| `pair join` / `pair reset` | join another device's person / move identity aside | pair_cmd.go |
| `node install` / `node uninstall` | Windows Task Scheduler entry for `serve --headless` | node_cmd.go |
| `version` | prints `irori <version>` | main.go:63-64; release smoke test asserts exact text (release.yml:256-258) |
| other | `unknown command "x" (gui, serve, mcp, attach, events, pair, node, version)` exit 1 | main.go:66; all errors print `irori: <err>` to stderr, exit 1 (main.go:68-71) |

`version` is injected at link time (`-X main.version=X.Y.Z`); default `"dev"` (main.go:37). `dev` never offers updates.

The **Core** (`internal/core`) wires every service (core.go:118-392): device identity, the signed log (`irori.db`), presence,
roster, inbox, coord (Board), capture, notifier, asks, tools, attachments (`files/`), mesh + pairing + home server, and the
**loopback HTTP API**. Exactly one Core per data dir; others find it through `irori.json`.

Interfaces to other areas (what the surface consumes):

| Consumer | Uses |
|---|---|
| `irori mcp` | `GET /v1/health`, `GET /v1/tools?client=grok`, `POST /v1/tools/{name}`, `POST /v1/members`, `GET /v1/attach?member=` |
| `irori attach` | `GET /v1/health`, `GET /v1/attach?session=&mod=` |
| the mod (mod/hooks/register.tsx) | spawns `irori attach`; then, using `hello.api.{url,token}` + `X-Irori-Attach`: `GET /v1/tools?client=claude`, `POST /v1/tools/{name}`, `POST /v1/deliveries/{delivered,engaged,turn-ended}`, `GET/POST /v1/policy` (register.tsx:147,169,219,256,315,410-411,467,478,485,498,502) |
| `irori events` | `/v1/device`, `/v1/events`, `/v1/events/stream` |
| `irori pair join` | `/v1/pair/{join,status,key,cancel}` (or starts its own headless Core) |
| suzuri, CLI, tests | v0.3 capture routes, Board browsing routes, `/v1/devices` |
| The window | Wails bound methods (section 2.9) + events (2.10) + asset route `/irori/attachment/<sha>` |

---

## 2. Data formats

### 2.1 Data dir and files (paths.go, diag.go, discovery, core)

| Item | Value | Ref |
|---|---|---|
| Data dir | `$IRORI_HOME` if set; else Windows `%LOCALAPPDATA%\irori`; else `~/.irori` | paths.go:12-26 |
| Claude dir | `$CLAUDE_CONFIG_DIR` else `~/.claude` | paths.go:29-38 |
| `.claude.json` (trust map) | `$CLAUDE_CONFIG_DIR/.claude.json` else `~/.claude.json`; only `projects[path].hasTrustDialogAccepted` read | roster_service.go:467-492 |
| Dir mode | `0700` (MkdirAll) | core.go:119, lockfile.go:29 |
| `irori.json` | lockfile (2.2) | discovery/discovery.go:17-28 |
| `crash.log` (+`.1`) | runtime crash output | diag.go:23-59 |
| `irori.log` (+`.1..3`) | app log, gui/serve only | diag.go:26-31,116 |
| `irori.db` (+`-wal`,`-shm`,`.db.v0.3`) | event log (store.md) | core.go:143 |
| `names.json` | session friendly names (device-local) | core/names.go:20-33 |
| `boards/<id>.db`, `boards/<id>.secrets` | shared boards | core/boards.go:52,377-382 |
| `files/` | attachments blob store | core.go:249 |
| `updates/` | downloaded installers, `update.log` (mac), `unpacked/` | update/service.go:148-151 |
| `reset-pending` | marks a Reset for next start (RFC3339 UTC + `\n`, 0600) | core/reset.go:28,41 |
| `reset-<YYYYMMDD-HHMMSS>-<rand>/` | Reset destination (MkdirTemp) | core/reset.go:56 |
| Reset moves | `irori.db irori.db-wal irori.db-shm irori.db.v0.3 device.json <identity.DeviceKeyFile> <identity.PersonFile> <mesh.PeersFile> <pair.RowsFile> capture-cursors.json files blobs` | core/reset.go:32-36 |
| Stays across reset | `settings.json`, `relay.json`, `irori.json` (and everything not listed) | core/reset.go:30-31 |

`names.json`: JSON object keyed by **session id** → `{"name": str, "at": RFC3339Nano time, "gave"?: [str]}` (names.go:26-33).
Unreadable file → logged, starts empty (names.go:57-60). Keep 30 days (`namesKeep`, names.go:24), busy window 2 h (names.go:37).

### 2.2 `irori.json` (lockfile)

Struct, field order as written (discovery.go:17-25):
```json
{
  "pid": 1234,
  "port": 51234,
  "token": "<64 lowercase hex = 32 random bytes>",
  "version": "0.4.1",
  "deviceId": "<device id>",
  "headless": false,
  "startedAt": "2026-10-29T12:34:56.789012345Z"
}
```
- `json.MarshalIndent(lf, "", "  ")`, **2-space indent, no trailing newline** (lockfile.go:32). `startedAt` = `time.Now().UTC()` in Go RFC3339Nano (trailing zeros trimmed, `Z`) (core.go:386).
- Write: MkdirAll 0700; CreateTemp in same dir named `irori.json.*.tmp`; chmod 0600; write; fsync; close; rename over target
  with retry `wait=5ms; while wait<1s { rename; sleep(wait); wait*=2 }` then one last rename (lockfile.go:28-75). Windows rename
  fails while a reader holds the file without FILE_SHARE_DELETE; that is what the retry is for. Rust `std::fs::File::open` on
  Windows also omits FILE_SHARE_DELETE, so readers in Rust must keep the open short too.
- Read: missing → `ErrNotRunning`; bad JSON → a parse error (not ErrNotRunning) `discovery: <path> is unreadable: ...`; pid not alive → ErrNotRunning (lockfile.go:78-103).
- Remove: only if file's pid == ours (lockfile.go:105-120).
- Written **after** the API listener is open and services start (core.go:379-390); removed in Close (core.go:411).
- Token is also added to capture's exact-value scrubber (core.go:305, 643-651).

### 2.3 crash.log and irori.log (diag.go)

- `InstallCrashOutput(dir)` for every role except `version` (main.go:79-98): MkdirAll 0700; if `crash.log` > 4 MiB rename to `crash.log.1` (failure ignored); open append|create 0600; `debug.SetCrashOutput`; `SetTraceback("all")` (diag.go:39-59).
- Start line (gui/serve only), written to crash.log **and** irori.log (main.go:99-111):
  `"<RFC3339 local> irori <version> start: role=<gui|serve> pid=<pid> <go version> <goos>/<goarch>[ vcs.revision=<x>][ -tags=<x>][ WARNING: ...]\n"` (diag.go:64-78).
  Release builds use `-buildvcs=false` (release.yml:110,196) so no `vcs.revision`; `-tags=production` appears.
  The Rust line should keep `<RFC3339> irori <version> start: role=... pid=...` and replace the Go fields with `rustc`/target triple; nothing parses it.
- `irori.log`: append 0600; rotate when `size>0 && size+len(p) > 4 MiB` (and at open if size ≥ 4 MiB): rename `.2→.3`, `.1→.2`, remove `.4`, `irori.log→.1`, reopen; a failed rename keeps writing to the current file (diag.go:120-180). All `log.Printf` output goes there plus stderr (main.go:110).
- `BadToolchain` (diag.go:87-102) is Go-specific; drop it.

### 2.4 Loopback HTTP API: transport rules (api/server.go, api.go)

| Rule | Value | Ref |
|---|---|---|
| Listen | `127.0.0.1:0` (ephemeral) | api.go:397 |
| Origin header present (any value) | 403 `{"error":"browser requests are not allowed"}` | server.go:149-152 |
| Host not `127.0.0.1:<port>` or `localhost:<port>` (lowercased compare) | 403 `bad host` | server.go:132-156 |
| Method | GET everywhere; POST also on POST routes; else 405 + `Allow: GET` or `GET, POST` (pair routes: `Allow: POST`/`GET`) | server.go:157-166, pairing.go:55-62 |
| `/v1/health` | no auth; `{"ok":true,"version","deviceId","net"?:{"state","peers"}}` | server.go:28-41,167-174 |
| Auth | `Authorization: Bearer <token>` exact prefix `Bearer `, constant-time; empty token never authorizes; 401 + `WWW-Authenticate: Bearer realm="irori"`, `missing or wrong token` | server.go:176-213 |
| Order | Origin → Host → method → health → auth → v02 → v03 → board → pair → v0.1 switch → 404 `no such route` | server.go:146-202 |
| Timeouts | ReadHeader 5 s, Read 10 s, Write 30 s (streams clear both deadlines), Idle 60 s, MaxHeaderBytes 64 KiB; shutdown waits 5 s then force-closes | server.go:101-121 |
| JSON responses | `Content-Type: application/json`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`; body = Go `json.Encoder.Encode` (**trailing `\n`**) | server.go:250-256 |
| Errors | `{"error":"<text>"}` | server.go:258-264 |
| POST body | ≤ 256 KiB (`MaxBytesReader`); whitespace-only body = empty (fields keep zero values); else must parse as JSON | routes_v02.go:100, 200-212 |

Identity headers (attach/protocol.go:121-125, api.go:382):
`X-Irori-Attach` (attach secret from hello), `X-Irori-Member` (Grok member secret), `X-Irori-Origin: composer`, `X-Irori-Person`
(per-run secret, 64 hex, held only by the GUI, never written; server.go:66-85). Person check: header present ⇒ must be exactly one
value equal to the secret (constant-time), else 403 (server.go:79-85, routes_events.go:461-468).

Caller resolution (routes_v02.go:169-193): exactly one of Attach/Member. Attach → stream registry lookup; must be a mod stream;
sender name replaced by the live card's name if non-empty. Member → `MemberBySecret`; sender `{key, name, kind:"grok"}`, path channel.

Viewer for browsing routes (board.go:53-71): Person (alone) → `""`; no identity header → `"api:agent"` (AgentViewer); agent header
that fails to resolve, or two at once, → 403. `boardFor("") = "user:<deviceId>"` (board.go:76-81).

### 2.5 Route table

All need the bearer token except health. `→` = 200 body. Lists are `[]` never `null` unless noted.

**v0.1** (server.go:184-201, api.go:286-299)
| Route | Body / rules |
|---|---|
| GET `/v1/device` | `device.Identity` `{id,name,os,personId?...}` (internal/device/device.go:21+) |
| GET `/v1/agents` | `[]presence.AgentCard` (this device's LocalAgents: presence + Grok member cards) |
| GET `/v1/events?kind=K&kind=K2&limit=N&device=D` | `[]store.Event` newest first; default 100, max 1000 (larger capped); `limit` non-positive-int → 400 `limit must be a positive integer`; rows redacted unless person (server.go:215-248) |
| GET `/v1/events/stream?kinds=a,b.&since=...` | NDJSON (2.7) |
| GET `/v1/devices[?detail=1|true]` | `[]mesh.DeviceRow`; 503 `this Core has no devices layer` without mesh (server.go:269-277) |

**v0.2** (routes_v02.go). 503 `messaging is not running` when Inbox nil (routes_v02.go:124-127).
| Route | Rules |
|---|---|
| GET `/v1/attach?session=S&mod=V` | S ≤200 bytes, no `/`, no control chars else 400 `bad session id`; card must be live claude card on this device else 404 `no live session "S" on this device (yet)`; NDJSON (2.6) |
| GET `/v1/attach?member=SECRET` | unknown → 403 `unknown member secret`; both/neither → 400 `pass exactly one of session or member` (stream.go:79-118) |
| POST `/v1/members` `{"name","session"}` | name trimmed, 1..`inbox.MaxAliasLen` runes, no control chars, else 400 `name is required: 1-N characters, no control characters`; session ≤200 bytes no control chars; → `{"memberKey","memberSecret","name"}` (routes_v02.go:391-415) |
| POST `/v1/deliveries/{delivered,engaged,turn-ended}` `{"token","turnId","appended"}` | attach caller only else 403; blank token → 400 `token is required`; → `{"ok":true}`; inbox errors mapped (below) (routes_v02.go:236-285) |
| GET `/v1/tools?client=claude|grok` | default claude; other → 400 `client must be claude or grok`; → `[]tools.Spec` |
| POST `/v1/tools/{name}` `{"args":{...}}` | attach or member else 403; missing/null args → `{}`; unknown tool → 404 `no tool called "x"`; args not an object → 400 `tools: args must be a JSON object`; → `{"text": "..."}`; tool failures are 200 with text `Error: ...` (routes_v02.go:312-344) |
| GET `/v1/policy` | `inbox.Policy` |
| POST `/v1/policy` | attach caller AND `X-Irori-Origin: composer` else 403; actor `api:composer@<agentKey>`; → new Policy (routes_v02.go:346-366) |
| GET `/v1/inbox?all=1|true&limit=N` | attach or member; default `inbox.DefaultInboxList`, max 500 (routes_v02.go:368-386) |

403 text for missing identity: `this route needs a current X-Irori-Attach or X-Irori-Member secret` (routes_v02.go:195-197).
Inbox error → status (routes_v02.go:215-229): NotFound/UnknownAgent 404; NotYours 403; BadState/Ambiguous 409; RateLimited 429; TooLong 413; else 500.

**v0.3 capture** (routes_v03.go, v03.go). All GET. 503 `capture is not running`. Limits capped, not refused.
| Route | Rules |
|---|---|
| `/v1/timeline?agent=&before=RFC3339Nano&limit=&subagents=1|true` | agent required (400 `agent is required`); limit default 20 max 100 (v03.go:85-109) |
| `/v1/edits?repo=&path=&agent=&limit=` | default 50 max 500 |
| `/v1/commits?repo=&limit=` | default 50 max 500 |
| `/v1/commits/{sha}` | 7–64 hex either case else 400 `sha must be 7 to 64 hex digits`; 404 `no commit <sha> (or the prefix is ambiguous)` |
| `/v1/blobs/{id}` | raw bytes, `text/plain; charset=utf-8`, nosniff, no-store, Content-Length; bad id 400, missing 404 `no such blob` (v03.go:168-187) |
| `/v1/decisions?repo=&path=&query=&all=&limit=` | `\` in path → `/`; default 50 max 500 |
| `/v1/prs?repo=owner/name` | `[]ledger.Entry` |
| `/v1/capture/status` | `capture.Status` |
| `/v1/why?path=&line=&repo=&cwd=` | path required; line positive int; ErrNoRepo → 404 (v03.go:208-240) |

**Board/browse** (routes_board.go, board.go). GET only; viewer rules 2.4.
| Route | Rules |
|---|---|
| `/v1/board?repo=&done=N` | 503 `the board is not running`; done default `coord.DefaultDoneCard`, max `coord.MaxDoneCard`; all groups non-null (board.go:114-138) |
| `/v1/tasks/{id}` | id empty or containing `/` → 404 `no such route`; unknown → 404 `no task "id"`; → `{"task","comments"}` |
| `/v1/search?q=&kinds=a,b&kinds=c&repo=&agent=&before=&offset=&limit=` | kinds lowercased, `agent` or unknown → 400 `kinds: "x" is not a searchable kind`; offset ≥0; asks limit+1 to compute `more`; → `{"hits","more","next"?}` (board.go:163-234) |
| `/v1/show?ref=&offset=&limit=` | ref required; ErrBadRef 400; not found or not readable → 404 `nothing has the ref "x"` (board.go:236-268) |
| `/v1/overview?agent=` | AgentViewer → 403; agent viewer must match agent=; person needs agent= (400) (board.go:270-304) |

**Pairing (joining side only)** (routes_pair.go, pairing.go). Fields trimmed, ≤200 bytes, no control chars (`MaxPairField`).
| Route | Rules |
|---|---|
| POST `/v1/pair/join` `{"code","server"}` | server normalized with `roomnet.HostPort(server,0)` else 400 `the home server address is a host and port, like 203.0.113.7:24817`; ErrBadCode 400; ErrBusy/ErrInMesh 409 |
| GET `/v1/pair/status` | `JoinView(status)`: an Add shows only `{mode,state}` (pairing.go:39-44) |
| POST `/v1/pair/key` `{"key"}` | runs with `WithoutCancel` (pairing.go:148); ErrWrongKey 422; ErrNotOpen 409 `no pairing is waiting for a key` |
| POST `/v1/pair/cancel` | `CancelJoin` only; → JoinView |
| 503 | `this Core has no pairing` |

There is no Add route and none may be added (routes_pair.go:184-210).

### 2.6 Attach stream (GET /v1/attach) — NDJSON

Headers `Content-Type: application/x-ndjson`, no-store, nosniff; read/write deadlines cleared (stream.go:120-127).
Line = Go `json.Marshal(attach.Event)` + `\n`; keepalive = bare `\n` every 15 s (stream.go:21).

`attach.Event` (attach/protocol.go:67-112), **all fields omitempty except `type`**, JSON key order as listed:
`type, v, session, agentKey, name, nameSource, aiTitle, api{url,token}, attachSecret, wakeId, token, reminder, deliveryIds,
messageIds, prompt, from, mode, overflow, deliveryId, why, preview, to, unread, held, message`.
Types: `hello deliver append name held moved inbox superseded error`. `Version = 2`, `AppendSince = "0.4.0"` (protocol.go:14,22).

Sequence (stream.go:71-240):
1. Validate, build hello `{type:"hello", v:2, session, agentKey:"<device>/<session>", name, nameSource, aiTitle, api:{url:"http://127.0.0.1:<port>", token:<bearer>}, attachSecret:<48 hex>}`. Member hello: `session` = last segment of memberKey, no attachSecret.
2. Subscribe to inbox changes **before** registering. Under registry mutex: supersede old stream for the key (close its kick, drop its secret), register, `MemberStream(key,true)` for members, `SetReachable(key,path,true)`.
3. Send hello; resend outstanding wake if still Queued (same token) else `NextWake`; for mods ≥0.4.0 resend `OutstandingAppend` else `NextAppend` (emitted as `type:"append", mode:"next_turn"`); then `inbox` counts `{unread, held}`.
4. Loop: keepalive tick → `\n`, then wake/append catch-up, then a `name` event if name/nameSource/aiTitle changed (mod path); inbox change for this key (or To=="") → `held` event on HeldNew `{deliveryId, from, why, preview}`, `moved` on MovedAway `{deliveryId, to}`, then wake/append, then `inbox` if counts changed.
5. On exit: if still current, unregister + `SetReachable(false)`; members always `MemberStream(false)`; if kicked, send `{"type":"superseded"}`.
- deliver: `{type, wakeId, token, reminder?, deliveryIds, messageIds, prompt, from (first delivery's sender name), mode?:"next_turn" (Grok next-turn-only push), overflow?}` (stream.go:315-336).
- `Preview`: whitespace/control runs → single space, first 120 runes (stream.go:363-371).
- `counts`: unread = `len(For(key,false,1<<20))`, held = deliveries To==key in state Held (stream.go:351-360).
- Attach secret: 24 random bytes hex (stream.go:62-68). Registry: one live stream per agent key.

Mod-version compare `ModAtLeast(v,min)`: trim spaces and leading `v`; split ≤3 on `.`; each part = leading digits (max 6); a part
with no digits ends parsing (first part no digits → not at least anything); a suffix after digits ends the version; empty/unreadable `min` → true (attach/version.go:7-52).

### 2.7 Events stream (GET /v1/events/stream)

routes_events.go:248-416. `kinds` comma list, exact or prefix ending in `.`. `since`:
- absent → no replay; bare int ≥0 → `{selfDevice: n}`; else comma list `<deviceId>:<seq>` split at **last** `:`, seq ≥0, duplicate device → 400, empty → 400 (routes_events.go:420-451).
- Subscribe first, then replay per device in **sorted device id** order, `AfterSeq` pages of 1000; flush headers before replay.
- Live: `Seq==0` signal → `{"resync":true}`; drop rows of named devices with seq ≤ last replayed; filter kinds; reset keepalive timer after each row (keepalives only fill silence).
- Rows redacted unless person (`tools.Redact`, observe.go:212-246).

`store.Event` JSON (store/store.go:35-45), **key order matters** (client `lineHead` reads `deviceId` and `seq` before any nested value to recover from an oversized line, client_events.go:192-227):
`{"deviceId":str,"seq":int64,"hlc":uint64,"actor":str,"kind":str,"payload":<raw JSON>}`; `sig` never serialized.
`hlc` is a uint64 (`wall_ms<<16 | logical`, hlc.go:12,58) emitted as a JSON number > 2^53.

Client side (client_events.go): max line 8 MiB; an oversized line is dropped whole and reported as a resync item carrying the
skipped row's deviceId/seq (used by `irori events --follow` to move past it, events.go:181-184).

### 2.8 Tools (internal/tools) and MCP (internal/mcpserver)

**Spec** JSON (tools.go:29-41): `{"name","description","inputSchema":<raw JSON schema>,"readOnly":bool,"clients":[...]}`.
39 tools in front-end order (specs.go:78-469, mirrored byte-for-byte in `internal/mcpserver/fallback_tools.json`, which
`TestFallbackMatchesTools` pins against `GET /v1/tools?client=grok`):
`overview search show agents events send reply attachment_get inbox ack(grok) defer skip notify board board_create board_split
board_comment board_block board_recommend board_unblock board_steps board_step board_watch board_unwatch board_join board_leave
board_assign board_status ask_claim ask_close board_link lease leases release timeline why commits prs join(grok)`.
readOnly: overview search show agents events inbox board leases timeline why commits prs. Every board tool's description
embeds `BoardInstructions` (+`AskInstructions`) (specs.go:28-48, room.go:19). Schemas are built by `schema()` as
`{"type":"object","properties":{...},"required":[...],"additionalProperties":false}` (specs.go:58-64). **Port rule: generate
the Rust spec list from `fallback_tools.json` (or embed it) and assert equality, rather than re-typing 39 descriptions.**

Dispatch (set.go:118-136): unknown → `ErrUnknownTool`; trim args; empty/`null` → `{}`; first byte not `{` or invalid JSON → `ErrNotObject`;
tools that need a caller (every non-readOnly tool plus `inbox timeline overview search show board`, minus `join`, set.go:21,87-100)
with empty sender key → `Error: this connection isn't bound to an agent yet. Grok: call join first. Claude Code: the Irori mod attaches your session; reload it if this persists.`;
result clipped to 8 KiB (`MaxResult`, tools.go:24).

Arg decoding is strict (set.go:140-168): unknown field → `Error: unknown argument "x"`; type mismatch → `Error: argument F should be a string|true or false|a whole number|an array of strings`.
`errText(msg) = "Error: " + oneLine(msg, 600)` (set.go:170). Error mapping table set.go:176-212 (exact advice strings; port verbatim).

Text helpers (format.go):
- `oneLine(s,max)`: split on runs of `unicode.IsSpace || unicode.IsControl`, join with one space, clip to max runes as `max-1` runes + `…` after right-trimming spaces (format.go:405-413).
- `quoted(s,max)`: trim, clip to `max-1`+`…`, then **Go `strconv.Quote`** (format.go:418-424).
- `clip(s,max)`: note `"\n[truncated: showing %d of N bytes; narrow the query or lower limit]"`; keep = max − len(note) − 8; cut at last `\n` before keep, else back to a rune start (format.go:427-445). mcpserver's `Clip` is the same with note `...; narrow the query]` (mcpserver/text.go:529-547).
- `ago`: zero → `never`; < −1 min → `in the future`; <10 s `just now`; <1 min `Ns ago`; <1 h `Nm ago`; **<48 h** `Nh ago`; else `Nd ago` (format.go:449-467). `in`: `in Ns`, `in Nm`, `in %dh%02dm` (format.go:470-481).
- `payloadText`: compact JSON, oneLine 200 (format.go:484-493).
- Caps: name/title/cwd 240, payload 200, list text 1200, objective 400 runes (format.go:391-396).

Representative outputs, port byte-exact (full set: golden-capture, see section 7):
- agents (observe.go:28-148): head `"%d of %d agents on this device%s."` + `" Message one with send (to = its name or key). "+untrusted+"\n"`; per agent `"\n- %s%s [%s] %s, key %s\n"`, `  title:`, `  cwd:`, `  repoKey: <x|none (not in a git repo)>`, `  doing: %q-style quoted (ref)`, `  permissionMode: <mode|n/a for grok|unknown>`, `  last activity: <ago>`, `  last text (untrusted): %q`. Sort by lowercase name then key.
- events (observe.go:155-205): default 20 max 100; line `"- 2006-01-02 15:04:05 (ago) kind dev=<first 8 runes> seq=N actor=A <payload>"`, local time.
- inbox (messages.go:190-323): default 20 max 50; whole rows fit in 8 KiB before anything is marked read; channel (Grok) path marks shown Queued rows read; row format messages.go:293-323.
- send/reply/ack/defer/skip strings messages.go:29-388 (defer minutes 1..720).
- notify: 5 cards/agent per sliding 10 min, answers wait 5 min, info cards 12 s, text 500, title 80, label 24 runes (notify.go:20-27).
- lease TTL: 1..`coord.MaxTTL` minutes (board.go:1205-1208).
- `Redact(kind,payload)`: `message.sent` → `text`,`reason` = `"[left out]"`; `turn` → `prompt` left out only if it contains `<irori-message` (case-insensitive); re-marshal via `map[string]any` (sorted keys, numbers as float64); on failure `{"text":"[left out]"}` (observe.go:215-246).

**irori mcp** (mcpserver.go):
- Implementation `{name:"irori", title:"Irori", version}`; capabilities `tools.listChanged=true`, `experimental: {"claude/channel": {}}`, no logging (mcpserver.go:148-157).
- Instructions = `tools.Instructions + "\n\n" + <grok text>` (mcpserver.go:68-74).
- Each spec registered as `irori_<name>`, annotations `{title:"Irori: <name>", readOnlyHint, openWorldHint:false}` (mcpserver.go:258-267). `forGrok` keeps clients-containing-grok (or empty clients), valid names `[a-z0-9_]{1,40}`, schema type object, dedupes, ensures `join` (fallback.go:445-500). Re-sync removes vanished tools.
- At start: dial (2 s timeout); failure → fallback specs; tools always exist and answer `Irori isn't running on this device; start the Irori app.` (mcpserver.go:53, 172-174).
- `irori_join {name}`: name oneLine 60; session id precedence `_meta.{session_id,grok_session_id,conversation_id,mcp_session_id}` → env `IRORI_SESSION_ID, GROK_SESSION_ID, XAI_SESSION_ID` → `sess_<24 hex>`; cleaned oneLine 200, spaces→`_`; fixed for process life (session.go:247-290). Result text mcpserver.go:437-440.
- Forward/retry state machine mcpserver.go:323-381 (403 + issuedBy → superseded; 403 other run → rejoin+retry; 401 or unsent dial error → drop+redial+retry; mutating calls only retried when unsent). Run identity = `base + "|" + token` (core.go:83-91).
- Fixed texts: `Error: call irori_join first`, `Error: another irori mcp process took over this member; call irori_join to bind here again`, `Error: Irori couldn't answer: <oneLine 400>` (mcpserver.go:52-56,390).
- Member stream loop: backoff 1 s doubling to 16 s; reconnect rules mcpserver.go:489-577.
- Channel push: JSON-RPC notification `notifications/claude/channel` params `{"content": prompt, "meta": {...}}`; meta `trust:"untrusted"`, `delivery_id` (first), `delivery_ids` (comma join), `message_id` (first), `from`, `reminder:"true"`, `mode`; keys must match `^[A-Za-z_][A-Za-z0-9_]*$`, values oneLine 400 (channel.go:309-363). Dedupe by delivery ids (1024 remembered), reminders always pushed; failed push unmarks (mcpserver.go:641-681). Push writes share the transport write lock with responses (channel.go:369-402).
- Result: text clipped to 8 KiB, `isError = text starts with "Error: "` (text.go:552-557).
- HTTP client: no proxy, dial 2 s, 30 s per call, no redirect following; response cap 4 MiB; errors parsed into `api.StatusError{Path,Code,Message}`; attach dial errors strip the URL (secret in query) (core.go:52-200).

### 2.9 Window bindings (Wails services → Tauri commands)

The frontend imports bindings only through `frontend/src/lib/*.ts` and `src/updater`. Bound methods actually called
(grep of `frontend/src`; signatures from `frontend/bindings/github.com/StephenSHorton/irori/*service.ts`):

| Service | Method(args) → result (used ones) | Go |
|---|---|---|
| RosterService | `Agents()→AgentCard[]`, `Device()→Identity`, `Version()→string`, `Recent(limit)→Event[]` (≤0→100), `Gone(limit)→GoneSession[]` (≤0→20), `Revive(sessionID)→string` | roster_service.go:49-343 |
| InboxService | `List(limit)`, `Held()`, `HeldHistory(limit)`, `Get(id)`, `Send(to,repo,text)→Message`, `Reply(id,text)`, `MarkRead(id)`, `Release(id)`, `Dismiss(id)`, `AllowAndRelease(id)`, `Policy()`, `SetPolicy(p)`, `SetPaused(b)` | inbox_service.go:80-228 |
| BoardService | `Cards()`, `Card(id)`, `DoneCards(limit,before)`, `Message(taskID,msgID)`, `Comment(id,text,tell)`, `Answer(taskID,blockerID,option,text,wake)`, `ResolveBlocker(taskID,blockerID,note)`, `Ask(text,repoKey,wake)`, `AskWithFiles(text,repoKey,wake,files:UploadFile[])`, `AskAgain(id)`, `AskRepos()`, `CloseAsk(id,note)`, `Pin(id,pinned)`, `MovePin(id,before)`, `Reopen(id)` | board_service.go:171-498, cards_service.go:675-1276 |
| BoardsService | `List`, `Create(name,tag)`, `Rename(boardID,name)`, `Leave`, `Forget`, `SharePreview(taskID,boardID)`, `Share(taskID,boardID,hash)→string`, `Withdraw(taskID)→string` | boards_service.go:188-243 |
| DevicesService | `Devices`, `AddDevice(role)`, `JoinWithCode(code,server)`, `ConfirmWords`, `ConfirmKey`, `PairingStatus`, `CancelPairing`, `RemoveDevice`, `ResetDevice`, `SetDeviceColor`, `HomeServer`, `SetHomeServer(on,outside,public)`, `SetHomeServerAwake(on)`, `Relay`, `SetRelay(credentialURL)` | devices_service.go:69-269 |
| SettingsService | `GetNotifications`, `SetNotifications(o)→Options`, `TestNotification`, `GetLaunchAtLogin`, `SetLaunchAtLogin(b)` | settings_service.go:52-141 |
| AttachmentsService | `Info(sha)`, `Save(sha)→path`, `Open(sha)` (`Fetch` bound, unused) | attachments_service.go:70-149 |
| UpdateService | `GetStatus`, `GetVersion`, `Supported`, `CheckAndDownload→Status`, `RestartToUpdate` | update/service.go:209-339 |

Bound but **not called** by the frontend: all of CaptureService (Timeline, Commits, Commit, Repos, Edits, Patch, Why,
Decisions, RecordDecision, PRs, Status, Fidelity), BoardService `Board Task Create Block Unblock Assign SetDone AddLink RemoveLink
ReleaseLease`, InboxService `Counts Paused`, RosterService `RequestRevive`, DevicesService `Colors`, AttachmentsService `Fetch`,
UpdateService `AutoUpdate CheckForUpdate`. Port them only if cheap (tests use some).

Return shapes are the Go structs' JSON (types in root `*_service.go`, listed board_service.go:40-113, cards_service.go:45-1197,
devices_service.go:47-253, inbox_service.go:40-44, roster_service.go:34-108, attachments_service.go:44-60, update/service.go:77-95,
and the internal packages). TS models: `frontend/bindings/**/models.ts`. `[]byte` fields (UploadFile.data) are base64 strings (board_service.go:278-281).
Errors: a Go `error` rejects the promise; frontend reads `err.message` or `String(err)` (src/lib/irori.ts:52-55).

Other runtime calls: `Browser.OpenURL(href)` (src/lib/cards.ts:810), `Clipboard.SetText` (src/lib/clipboard.ts:12), plain
`fetch(attachmentURL(sha))` against `/irori/attachment/<sha>` (src/lib/attachments.ts:110). `inWails()` detects the host by
`window.chrome.webview.postMessage` or `window.webkit.messageHandlers.external.postMessage` (src/lib/irori.ts:38-50); false ⇒ the
app runs on a built-in sample roster. One localStorage key: the updater's dismissed version (src/updater/useUpdater.ts:12,44,141).

### 2.10 Window events (Go → JS), coalescing

| Event | Payload | Emitted | Ref |
|---|---|---|---|
| `roster:changed` | `{change: presence.Change, agents: AgentCard[] (MeshAgents)}` | every presence change | roster_service.go:30-37,65-69 |
| `inbox:changed` | `InboxCounts {unread, held, paused}` | coalesced ≤ every 250 ms | inbox_service.go:22-25,282-300 |
| `board:changed` | `{}` | coalesced 250 ms; also from boards pump | board_service.go:27-30,531; boards_service.go:174 |
| `boards:changed` | `{}` | shared-board list change | boards_service.go:24,162 |
| `capture:changed` | `CaptureChange` | coalesced 300 ms (no frontend listener) | capture_service.go:26-30,355 |
| `devices:pairing` | `pair.Status` | pairing change | devices_service.go:20-43 |
| `settings:launch-at-login` | bool (live OS state) | after every toggle attempt | settings_service.go:25,126-141 |
| `app:open` | `notify.Target {tab, ref?}` | notification Open/body click, after showing window | notify_gui.go:16-31 |
| `update:status` | `update.Status {phase, version, notes, error}` | every updater transition | update/service.go:63-85,224-237 |

Frontend reads Wails payloads as `ev.data` (src/lib/irori.ts:106-110); Tauri delivers `event.payload`.

### 2.11 Attachment asset route

`/irori/attachment/<sha256>` inside the webview only (gui.go:116, attachments_service.go:177-188, core/attachview.go:28-296):
GET only (405 `Allow: GET`); every answer `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox`;
errors are `no-store` plain text: 404 (bad id / no readable comment carries it), 413 (> 32 MiB `MaxPreview`), 503 holder offline
(reason text), 504 fetch timeout (2 min `FetchTimeout`), 502 other. Success: sniff first 512 bytes (Go `http.DetectContentType`);
`image/png|jpeg|gif|webp` inline with that type, else `application/octet-stream` attachment; `Content-Disposition` via
`mime.FormatMediaType(disp, {filename: SafeName})`; `Cache-Control: private, max-age=31536000, immutable`; `ETag: "<sha>"`;
`http.ServeContent` (Range, If-None-Match).

### 2.12 Member keys (persisted in the log)

`memberID = hex(sha256(deviceID + "\x00" + session)[:6])` (12 hex); empty session → 6 random bytes; key `<deviceID>/grok-<id>`;
secret 24 random bytes hex; a rejoin retires the old secret. Row `member.joined {key,name}` via `AppendLocal` (never replicated).
Synthetic card `{key, deviceId, agent:"grok", sessionId:"grok-<id>", name, status:"idle", sessionKind:"member", peerFeatures:[], startedAt, updatedAt, lastActivity}` while a stream is open or < `inbox.ReattachGrace` after close; sorted by key (core/members.go:58-167).

### 2.13 Presence → log rows (core.go:422-470)

Added → `agent.seen` with the card minus `lastText, transcript, pid, deviceState, aiTitle`, title scrubbed. Updated → `agent.status
{key,status,name,nameSource?,permissionMode?}` only when status/name/nameSource/permissionMode changed. Removed → `agent.gone {key, reason: card.Status}`. Start writes `device.started {name, os, version, headless}` (core.go:182-184). Actor `"irori"`.

---

## 3. Rules, constants, timings

| Constant | Value | Ref |
|---|---|---|
| API keepalive (attach + events stream) | 15 s | api/stream.go:21 |
| Event limits | default 100, max 1000 | server.go:22-25 |
| Inbox route max | 500 | routes_v02.go:103 |
| POST body max | 256 KiB | routes_v02.go:100 |
| Client probe/dial | 2 s; client timeout 30 s; resp cap 32 MiB | api/client.go:23-66 |
| attach runner backoff | 1 s → ×2 → 16 s; reset on hello; max line 1 MiB | attach/runner/runner.go:139-146 |
| attach parent poll fallback | 5 s | attach.go:83 |
| `events --follow` reconnect | 1 s → ×2 → 10 s | events.go:24-27 |
| mcp dial / call | 2 s / 30 s; push dedupe 1024 | mcpserver.go:80,124; core.go:73 |
| Core close wait | 5 s | core.go:402-405 |
| coord sweep / inbox sweep | 5 s / `inbox.SweepInterval` | core.go:474-498 |
| Revive window, timeout, output cap | 1 min, 30 s (never kill), 64 KiB, WaitDelay 3 s | roster_service.go:215,319,331,447 |
| Session id valid | 8–64 chars, hex/dash, no leading dash | roster_service.go:362-372 |
| Updater HTTP | 30 s API, download 10 min, SHA256SUMS ≤1 MiB | update/release.go:67,145,167 |
| Log sizes | crash 4 MiB→`.1`; irori.log 4 MiB keep 3 | diag.go:29-31 |
| Upload from window | 32 MiB per file | board_service.go:285 |
| Talk window / max talking / stale | 5 min / 4 / 2 h | cards_service.go:597-633 |
| Mod control retries | 5 tries, 3 s apart; give up on 4xx except 403 | mod/hooks/register.tsx:120-133 |

Updater rules (update/service.go, release.go):
- `isNewer(tag,current)`: current `""`/`dev` → never; compare `X.Y.Z` numerically (Atoi, bad→0), drop `+build`, final > prerelease, prereleases compared as strings (service.go:367-411).
- GET `{api}/repos/StephenSHorton/irori/releases/latest` with `Accept: application/vnd.github+json`; 404 → no update (release.go:33-57).
- Assets: Windows `isInstallerAsset`: lowercase ends `-installer.exe` and contains `windows`; macOS: starts `irori-`, ends `-macos-<GOARCH>.app.zip` or `-macos-universal.app.zip`; `SHA256SUMS` case-insensitive name; release without either → error (release.go:109-128). GOARCH names: `arm64`, `amd64`.
- SHA256SUMS line: exactly 2 whitespace fields, optional leading `*` on name, compare basename; hash lowercased (release.go:132-156).
- Download into `<data>/updates/<basename>`; delete other non-dir files there except `*.log`; hash mismatch deletes (release.go:160-197).
- Phases `idle checking downloading ready installing error`; one run in flight; ready/installing short-circuit (service.go:67-74,261-300).
- `RestartToUpdate`: requires ready; set installing before quit; launch failure reverts to ready (service.go:316-339). `InstallOnExit` in OnShutdown: only from ready, no relaunch (service.go:348-363).
- Windows install args: `/S [/NORELAUNCH] /WAITPID=<own pid>` (nsis.go:256-274). Startup cleanup removes `irori.exe.old`, `.oldN` (digits only) next to the exe (nsis.go:282-314).
- macOS: precheck running app = `.../X.app` from `os.Executable` (symlinks resolved), not under `/AppTranslocation/`, parent writable (mac_darwin.go:148-167). Stage: `ditto -x -k zip unpacked/`, exactly one `.app`, `plutil` checks `CFBundleIdentifier == com.stephenshorton.irori`, `CFBundleShortVersionString == numericVersion(v)`, executable exists and +x; `codesign --verify --deep --strict`; refuse non-Developer-ID over Developer-ID (`Authority=Developer ID Application`); `xattr -dr com.apple.quarantine`; `ditto` to `.<Name>.app.new` beside live (mac_darwin.go:171-280, macapp.go:42-55). Install: `/bin/sh -c <swapScript> irori-update <pid> <live> <next> <0|1> <updates/update.log>` detached with `Setsid` (macapp.go:57-94, mac_darwin.go:283-300). Cleanup removes `.app.new`/`.app.old`.
- Linux: unsupported (`updates not supported on linux yet`).
- Update card: id `update:<v>`, title `Irori v<v> is ready`, sticky, actions Install / Later; once per version per run (update_card.go:11-80).

Tray (tray.go:30-268): tooltip `Irori`; left-click opens window except macOS (tray_darwin.go:21, tray_other.go:11); menu rebuilt whole on every change:
`[Restart to update to v<X>, sep]`, `Open Irori`, disabled status line, `[N held message(s)…]`, sep, checkbox `Pause wakes on <device>`,
checkbox `Launch at login`, sep, `Quit Irori`. Status = `No agents running` / `1 agent (N busy)` / `N agents (M busy)` (roster_service.go:72-94),
default `Irori v<ver>` or `Irori (dev build)`, plus ` · N need a decision` ` · N unread` (tray.go:132-180).

Window: title `Irori`, 1100×720, bg RGB(233,238,248), URL `/`, close hides (gui.go:136-148); single instance id
`com.stephenshorton.irori`, or `+ "." + hex(sha256(lower(clean(IRORI_HOME)))[:6])` when IRORI_HOME set; second launch shows the
window (gui.go:28-40,127-130). If a non-headless Core already owns the lockfile, a bare app with the same id hands off and exits
(gui.go:65-69,169-177). Refuses to run as a Wails server build (servermode_on.go).

Launch at login (settings_service.go:12-141): registration name `Irori`, args `--startup`; the OS registration is the only state.

CLI rules:
- `attach`: stdout only NDJSON lines copied unchanged (non-JSON lines dropped, keepalives dropped); on first outage emits one
  `{"type":"error","message":"Irori isn't running, so messages can't reach this session. It attaches by itself once Irori starts."}`;
  404 = not on roster yet, retry; `superseded` ends with exit 0 (runner.go:168-308). Exits when the parent dies: Windows waits on
  the parent handle, macOS kqueue NOTE_EXIT, else 5 s poll; pid reuse ruled out by start time; non-Windows checks reparenting (attach.go:41-112).
- `events`: plain line `MM-DD HH:MM:SS <kind padded %-16s>[ [device]] k=v ... [ by=actor]`, summary keys first `key name status reason kind repo path sha title`, then other scalars sorted; strings Preview'd and Go-quoted if they contain space, `=` or `"`; numbers `FormatFloat('f',-1)`; ≤160 runes (events.go:280-350). `--json` prints Go `json.Marshal(ev)`; resync prints `-- some rows were dropped; resyncing --` or `{"resync":true}`. Follow without `--since` starts after this device's LastSeq and prints `--limit` recent rows once the stream opens, deduped (events.go:82-149).
- `pair join`: prompts (exact texts pair_cmd.go:63-80), reads lines from stdin (console CONIN$ on Windows), drives running Core via API or starts its own headless Core; polls every 300 ms; prints words / key prompts; any exit except `Paired.` cancels its own join (pair_cmd.go:110-195). `pair reset` asks to type `reset` (pair_cmd.go:240-258).
- `node install` (Windows only): prints `Running: <quoted argv>`, runs `schtasks /Create /TN "Irori node" /SC ONLOGON /RL LIMITED /F /TR "\"<exe>\" serve --headless"`; uninstall `schtasks /Delete /TN "Irori node" /F` (node_cmd.go).

Revive (roster_service.go:276-343): refuse live session, revived within 1 min, not seen on this device as claude, transcript missing
(card path or `<claude>/projects/<presence.Slug(cwd)>/<id>.jsonl`), cwd missing. Find `claude` without a shell: PATH (`claude.exe`
then `claude.cmd` on Windows) then `~/.local/bin`. Run `claude --bg --resume <id>` in cwd, hidden console, strip ANSI escapes with the regex at roster_service.go:460.

Attachments service: Save to Downloads (Windows known folder FOLDERID_Downloads) never overwriting (` (2)`…); Open only for
extensions at attachments_service.go:154-160 and never for bytes starting `MZ`, `#!`, `\x7fELF` or Mach-O magics; copies into
`%TEMP%/irori-attachments/<sha[:16]>` first (attachments_service.go:94-172).

---

## 4. Platform-specific code

| Concern | Windows | macOS | Other | Ref |
|---|---|---|---|---|
| Data dir | `%LOCALAPPDATA%\irori` | `~/.irori` | `~/.irori` | paths.go |
| GUI subsystem | exe linked `-H windowsgui`; CLI roles except gui/mcp `AttachConsole(ATTACH_PARENT_PROCESS)` + `CONOUT$` only for unusable std handles (GetFileType UNKNOWN); `pair` also `CONIN$` | n/a | n/a | console.go, console_windows.go |
| PATH for GUI | – | ask `$SHELL -ilc "printf '__IRORI_PATH__%s__IRORI_PATH__' \"$PATH\""` (5 s, default `/bin/zsh`), else prepend `~/.local/bin /opt/homebrew/bin /usr/local/bin`; merge dedupe | – | guipath_darwin.go |
| Child console | `HideWindow` + `CREATE_NO_WINDOW` (0x08000000) | – | – | roster_service.go:422-435 |
| Process alive | OpenProcess(QUERY_LIMITED) + exit code STILL_ACTIVE; ACCESS_DENIED = alive | kill(0) (EPERM=alive) and zombie (p_stat 5) = gone | kill(0) | proc/proc_windows.go:11-23, proc_darwin.go:20-31, proc_unix.go |
| Start time | process creation time (FILETIME) | sysctl kern.proc.pid p_starttime | none | proc |
| Wait parent | handle wait (SYNCHRONIZE) | kqueue EVFILT_PROC NOTE_EXIT | poll | proc/wait_* |
| Launch at login | HKCU `Software\Microsoft\Windows\CurrentVersion\Run` value `Irori` = `<exe> --startup`, each part quoted only if it has whitespace or `"` (so the default per-user path is unquoted; readers must accept both) | SMAppService `mainAppService` when bundled on 13+, else `~/Library/LaunchAgents/Irori.plist` (label = Identifier `Irori`; RunAtLoad, ProgramArguments) + `launchctl bootstrap gui/<uid>` | – | wails v3.0.0-alpha.98 autostart_windows.go:43-63,159-170, autostart_darwin.go:26-45,140-150 (Go module cache) |
| Tray | PNG `build/trayicon.png`, click opens | template image, click shows menu | as Windows | tray.go:20-25, tray_darwin.go |
| Downloads | KnownFolderPath(Downloads) | `~/Downloads` | same | downloads_*.go |
| Updater | NSIS silent installer | `.app.zip` swap helper | unsupported | update/check_*.go, mac_darwin.go |
| Node | schtasks | refused with advice | refused | node_cmd.go:26-28 |
| Detach | nothing needed | `Setsid` | – | update/check_windows.go:329, mac_darwin.go:123 |

---

## 5. Rust crates (crates.io, checked 2026-10-29)

| Need | Crate @ version | Note |
|---|---|---|
| Window, IPC, tray, menu | `tauri` 2.12.5, `tauri-build` 2.7.4 | `tray-icon` feature; `mainBinaryName: "irori"` |
| Single instance | `tauri-plugin-single-instance` 2.5.5 | id must include the IRORI_HOME hash (gui.go:33-40); the plugin reads `app.config().identifier` and takes no id, so set `context.config_mut().identifier` first (plugin src/platform_impl/windows.rs:81-88) |
| Launch at login | `tauri-plugin-autostart` 2.7.0 (wraps `auto-launch` 0.6.0) or own `winreg` 0.56.0 + `objc2-service-management` 0.3.2 | see trap 7.4 |
| Open URL/file, clipboard | `tauri-plugin-opener` 2.7.2, `tauri-plugin-clipboard-manager` 2.5.0 | |
| Async runtime | `tokio` 1.54.1, `tokio-util` 0.7.20, `tokio-stream` 0.1.19, `futures` 0.3.34 | |
| HTTP server | `axum` 0.8.9 on `hyper` 1.12.0, `tower` 0.5.3, `tower-http` 0.7.1, `http-body-util` 0.1.5, `bytes` 1.12.1 | write handlers to do Host/Origin/auth before routing; streaming bodies |
| HTTP client | `reqwest` 0.13.5 (`no_proxy()`, redirect none, rustls) | loopback + GitHub |
| MCP stdio | `rmcp` 3.8.0 | has `ProtocolVersion::V_2026_07_28` … `V_2024_11_05` and `CustomNotification` (for `notifications/claude/channel`); verify experimental capability and dynamic tool add/remove + listChanged |
| JSON | `serde` 1.0.229, `serde_json` 1.0.154 (`raw_value`, `preserve_order` only where needed) | |
| CLI | `clap` 4.6.7, or a hand parser to match Go `flag` (single-dash `-follow` also works in Go) | |
| Crypto/rand | `sha2` 0.11.1, `hex` 0.4.3, `rand` 0.10.3 / `getrandom` 0.4.3, `subtle` 2.6.1 | constant-time compares |
| SQLite | `rusqlite` 0.41.1 | (store area) |
| Logging | `tracing` 0.1.44, `tracing-subscriber` 0.3.23, size rotation via `file-rotate` 0.8.0 or own writer (`tracing-appender` 0.2.5 is time-based only) | |
| Win32 | `windows` 0.62.2 / `windows-sys` 0.61.2 | AttachConsole, OpenProcess, WaitForSingleObject, KnownFolderPath |
| macOS | `objc2` 0.6.5, `objc2-app-kit` 0.3.2, `objc2-service-management` 0.3.2, `nix` 0.31.3 / `libc` 0.2.191 (kqueue, sysctl, setsid) | |
| Plist (if not shelling to plutil) | `plist` 1.10.3 | keep shelling to `ditto`/`codesign` |
| Misc | `regex` 1.13.1, `url` 2.5.8, `percent-encoding` 2.3.2, `base64` 0.23.1, `tempfile` 3.27.0, `semver` 1.0.28 (not for isNewer: Go rules differ), `infer` 0.22.0 (not a Go DetectContentType clone), `dirs` 7.0.0 | |
| Binding codegen | `tauri-specta` `=2.0.0-rc.26` + `specta` `=2.0.0-rc.26` (stable `tauri-specta` 1.0.2 depends on `tauri ^1.2.4`, so it is Tauri 1 only), or `ts-rs` 12.0.1 | only if you regenerate models; see 7.6 |
| Networking | `iroh` 1.4.1 is current stable (task says 1.0) | other area |

---

## 6. Tests to port (what each proves)

**internal/api** — api_test.go: health needs no token; auth on every other route; Origin → 403; bad Host → 403; GET-only rules; `/v1/events` kind/limit params; client round trip; Dial when not running → ErrNotRunning; Serve stops on cancel.
events_test.go: matchKind/parseKinds; replay-then-live with no gap/dup; replay modes (bare seq, vector, none); resync line and close; client resync item; keepalive cadence; v0.1 rules on stream; LastSeq; oversized row → resync carrying its device/seq; lineHead.
v03_test.go: 503 without capture; v0.1 rules; 400 params; each route's shape; why on unknown repo is read-only 404.
board_test.go: v0.1 rules, 503s, viewer headers (person/agent/none/bad/both), 400s, board/task/search/show/overview shapes, search+show viewer scoping and paging, person's NeedsYou.
v04_test.go: vector since; rows redacted without person secret; token alone reads as agent.
client_v02_test.go: OpenStream error text hides the query (secret).

**internal/mcpserver** — spec registration + forwarding with `irori_` prefix; join → member stream → channel push shape; not-running fallback tools; old Core with no tool list uses fallback; reconnect rejoins with same session; restarted Core: call rejoins and retries; superseded handling (call and stream after takeover stop); stream cut keeps secret; failed rejoin recovers; fresh dial on same run doesn't rejoin; attach error hides secret; serverInfo + capabilities; meta key filter; session precedence; sessionless name reuse = new member; fallback valid; **fallback == Core grok specs**; Clip/oneLine.

**internal/tools** — tools_test.go: Specs per client; malformed args; join handled by mcp; unbound caller text; agents; events redacts text; send/reply; reply to ended sender lands on card; send validation, to repo, to PR; rate limit; held text never leaks; inbox channel marks read; defer/skip; inbox folds+truncates; leases; channel budget marks only shown rows; huge row shortened. board_test.go, blocker_rich_test.go, room_test.go, owner_test.go: board tool behaviour and texts, short refs, clash refusal, watch digests, split, blockers with links/attach, ask claim/close, owner rule. capture_test.go/explore_test.go: timeline/why/commits/prs/search/show/overview texts, paging and caps, message visibility. notify_test.go: answers come back next turn, dismiss/expiry, rate limit incl. parallel, refusals, failed post frees slot.

**internal/update** — isNewer/cmpSemver; RestartToUpdate launches `/S /WAITPID`, emits and quits, requires ready, failure reverts; InstallOnExit `/NORELAUNCH` only from ready; never launches twice; WAITPID is own pid; CleanupOldExes; findUpdate picks installer+checksum, current/dev, no releases, requires checksum; downloadVerified; platform flow. workflow_test.go: **release.yml asset names match the matchers**. mac tests (darwin): swap helper swaps bundle, refuses bad bundle, install args, cleanup; macapp: bundle root, side paths, numericVersion, asset matcher.

**internal/diag** — start line content; log rotation incl. big file at open; crash output lands in file. **internal/attach** — ModAtLeast table. **internal/discovery** — token 32 hex bytes; round trip + 0600; replace leaves no temp; missing/dead pid → ErrNotRunning; corrupt → parse error; remove only if ours.

**internal/core** (integration; need fakes from other areas): v02_test.go two sessions talk, supersede, keepalive, auth, Grok member + supersede, attach command; api_devices/api_pair: devices route + health net, join via API (wrong key twice, expiry/cancel, no Add side); reset makes a new device; migrate v0.3 dir; names (assign once, skip live names, settle clashes across devices, tie → lower key, chosen names kept, forget long gone); nextturn (append to new mod, rides wake for old mod); untaken (unconfirmed wake reaches next turn; appended wake engaged over HTTP); record replicates mode learned later; attachments and route; boards*; devices_test multi-device scenarios; mesh (lone Core opens no net).

**root package** — attach (helper sleep, parent watch); console wantsConsole/fixStdio; events formatEvent/summarize/printer/splitKinds; follow (resync before own row, startup gap, oversized own row); guipath (markers, merge, login shell); node install args; pair join (drives running Core, starts own, cancels on stdin end and every way out, follows only own join, past unreadable lockfile, askJoin); roster (card index past newest 1000, revive guard); tray lines; update card (posts on ready, install, later, cleared elsewhere, superseded, failure repost, notifications off, headless never); service tests (board/cards/inbox/capture/devices/attachments/boards) — port as Tauri command tests against a test Core.

**mod** (bun) — mod/unit/lib.spec.ts: takeEvents NDJSON splitting, helpers, editPolicy, appends, titleToSet. mod/tests/irori.test.tsx: attach lifecycle, next-turn appends, tools registration, `/irori` command, session title. Driven by `mod/dev/fakeirori` and `mod/dev/fakecore` (Go): keep them, or rebuild in Rust, and run the mod suite against the Rust `irori attach` + API as the acceptance test.

---

## 7. Hard parts and compatibility traps

7.1 **The first Rust release is installed by the Go updater.** Release assets must be `irori-<v>-windows-amd64-installer.exe`, `irori-<v>-macos-arm64.app.zip` (+`.dmg`, `-windows-amd64.zip`) and `SHA256SUMS` (`sha256sum *` format) on a non-prerelease GitHub release of `StephenSHorton/irori` (release.yml:121-130,239-240,290-302). The installer must accept `/S`, `/NORELAUNCH`, `/WAITPID=<pid>`, wait on the pid (60 s), probe-open the exe for append, rename-aside to `irori.exe.old[N]` (N up to 9), exit code 3 and relaunch the old exe when it gives up, relaunch on silent success unless `/NORELAUNCH` (project.nsi:103-246). Use Tauri's `bundle.windows.nsis.template` with a port of project.nsi; Tauri's default template differs in all of this.

7.2 **Install identity must match** or the person gets two apps: per-user `$LOCALAPPDATA\Programs\Irori\irori.exe` (project.nsi:91-92), uninstall key `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Stephen HortonIrori` (wails_tools.nsh:25-28,127-136), shortcuts `$SMPROGRAMS\Irori.lnk` (always) and `$DESKTOP\Irori.lnk` (fresh install only). Tauri's `installMode: currentUser` installs to `%LOCALAPPDATA%\<productName>` and uses its own key; override. Product name `Irori`, binary `irori(.exe)` (`mainBinaryName`).

7.3 **macOS bundle checks the Go updater runs on the Rust zip**: one `.app` at zip top level; `CFBundleIdentifier com.stephenshorton.irori`; `CFBundleShortVersionString` = bare `X.Y.Z` of the tag; `CFBundleExecutable` exists and is executable (Go Info.plist has `irori`); `codesign --verify --deep --strict` passes; Developer ID if the installed one is (mac_darwin.go:227-280). Tauri's identifier and version config must match. `LSMinimumSystemVersion` 12.0.0.

7.4 **Launch-at-login migration.** Windows: Wails wrote HKCU Run value `Irori` = `C:\...\irori.exe --startup` (quoted only when the path has spaces); it also deletes stale values pointing at the same exe (autostart_windows.go:43-63,159-170). `auto-launch` also touches `Explorer\StartupApproved\Run` and picks its own value name; the Go NSIS uninstaller deletes only `Run\Irori` (project.nsi:259). Either write the registry yourself with `winreg`, or set the name to `Irori` and check StartupApproved semantics. macOS: Wails used `SMAppService.mainAppService` for a bundled app on 13+ (no arguments are passed that way, so `--startup` is absent: the window opens at login unless the port detects the login launch another way). auto-launch uses LaunchAgent/AppleScript, so an existing SMAppService registration would be invisible and not removable. Use `objc2-service-management` for parity.

7.5 **Binary name, subcommands and flags are a contract.** The mod's `iroriPath` setting is an absolute path to `irori.exe` and it spawns `[path, "attach", "--session", id, "--cwd", cwd, "--mod-version", "0.4.1"]` (register.tsx:410-411, plugin.json userConfig). Agents' MCP configs run `irori mcp`. Task Scheduler `Irori node` runs `"<exe>" serve --headless`. Go `flag` accepts `-flag`, `--flag`, `-flag=v`, `--flag v`; clap does not accept single-dash long flags by default.

7.6 **Frontend swap.** Replace `@wailsio/runtime` and `frontend/bindings` with a Tauri gateway that keeps the module paths, function names, argument order and the generated `$$createType` converters (Wails `Create.Array` turns `null` → `[]`, `Create.Map` `null` → `{}`, struct creators fill missing fields; create.js:28-75). Without them, every Go-nil slice the Rust side emits as `null` breaks the UI. `invoke` takes named args: map positional args to names in the gateway. Events: `Events.On(name, ev => ev.data)` → `listen(name, e => e.payload)`. Errors: Tauri rejects with the serialized error (a string), not an `Error`. `inWails()` must become `inTauri()` (`window.__TAURI_INTERNALS__`) or WebView2 still says true but calls fail. `Browser.OpenURL`/`Clipboard.SetText` → opener/clipboard plugins. Only frontend files under `src/lib`, `src/updater` import bindings.

7.7 **Attachment route under Tauri**: serve `/irori/attachment/<sha>` through a custom URI scheme or `register_asynchronous_uri_scheme_protocol` with Range/ETag/If-None-Match (mp4/mp3 seek), the CSP sandbox and nosniff headers. `AttachmentInfo.url` is today the relative path `/irori/attachment/<sha>` (attachments_service.go:66) and the frontend `fetch()`es it (src/lib/attachments.ts:110); under Tauri a relative path resolves against the app origin (`tauri://localhost` on macOS, `http://tauri.localhost` on Windows), not a custom scheme. Have that one place emit the absolute, platform-specific URL (`irori://localhost/irori/attachment/<sha>` on macOS, `http://irori.localhost/irori/attachment/<sha>` on Windows), and allow it in `security.csp` `connect-src`, `img-src` and `media-src`. Content sniffing must reproduce Go `http.DetectContentType` for the 4 inline image types only (else octet-stream).

7.8 **Go JSON quirks.** (a) Go escapes `<`, `>`, `&` as `\u003c` `\u003e` `\u0026` and U+2028/2029 in every `json.Marshal`; prompts contain `<irori-message`. Stored payloads in `irori.db` carry these escapes; Redact parses before matching for that reason (observe.go:221-225). Byte diffs of API output will show this; parsers don't care. Signed rows are store.md's problem. (b) `omitempty`: zero ints/bools/empty strings/nil slices omitted (attach.Event, `Identity.personId`, `SearchResult.next`); use `skip_serializing_if`. (c) `json:"-"`: `Event.sig`, `Status.installerPath`. (d) `[]byte` → base64. (e) `time.Time` → RFC3339Nano with trailing zeros trimmed, local offset kept as written (lockfile writes UTC `Z`; Go zero time is `"0001-01-01T00:00:00Z"`, not null). (f) `json.RawMessage` payload passes through compacted (Go's Marshal compacts and HTML-escapes RawMessage). (g) Encoder adds `\n` after JSON responses. (h) Redact and `events` summary decode into `map[string]any`: keys come back sorted and every number becomes a float64, so an integer above 2^53 (an `hlc`-sized value in a payload) is re-emitted as the nearest float64's integer, a different number. Go's float encoder uses plain `'f'` notation for 1e-6 ≤ |x| < 1e21 (so `1000000` stays `1000000`) and `'e'` outside it (`1e+21`, `1e-7`). Reproduce with `serde_json::Map` (sorted without `preserve_order`), numbers routed through f64, and that formatting rule.

7.9 **Go text semantics in tool output.** `strconv.Quote` (escapes `\a\b\f\n\r\t\v\\\"`, non-printable runes as `\xNN`/`\uNNNN`/`\UNNNNNNNN`, invalid UTF-8 bytes as `\xNN`, keeps printable Unicode per `unicode.IsPrint`); `%q` is the same. Rust `{:?}` differs (e.g. `\u{..}`, `'` handling). `unicode.IsSpace` includes U+0085 and U+00A0; `char::is_whitespace` matches except edge cases; `unicode.IsControl` = Cc only. Rune counts = Unicode scalar values (`chars().count()`). Byte-length clips must back up to a char boundary. Time formats `2006-01-02 15:04:05` and `01-02 15:04:05` in local time.

7.10 **Tool text is a large, test-pinned surface** (board.go 1345 lines, explore/capture/messages). Capture goldens: run the Go tool set over fixture Cores (the existing `*_test.go` fixtures) and record `(tool, args, caller) → text`; replay against Rust. Same for fallback_tools.json (embed it; assert `GET /v1/tools?client=grok` equals it byte-for-byte after canonicalization).

7.11 **MCP SDK behaviour.** go-sdk answers both the stateless 2026-07-28 `server/discover` probe and legacy `initialize` 2024-11-05..2025-11-25 (mcpserver.go:23-25). rmcp 3.8.0 lists those protocol versions; verify discover handling, `experimental` capability passthrough, runtime `add/remove tool` with `notifications/tools/list_changed`, and that a custom notification write cannot interleave with a response on stdout. Tool names `irori_<name>`; inputSchema passed through raw (must remain `additionalProperties:false`).

7.12 **Stream semantics to keep exact**: subscribe-before-replay with per-device dedupe (events), subscribe-before-register and single registry mutex covering wake issuing (attach, stream.go:40-47,131-150), superseded event sent on the old stream before close, member stream open/close counted per stream, keepalive only after silence for events but on a fixed tick for attach. `deviceId` and `seq` must be the first two keys of every event line.

7.13 **Lockfile and Windows sharing.** Readers (`irori mcp`, `irori attach`, the mod via attach) open `irori.json` briefly; writer retries rename for <1 s. The Rust writer must use `MoveFileExW(REPLACE_EXISTING)` semantics (`std::fs::rename` does) plus the retry. Token and person secret: 32 bytes from the OS RNG, hex; never logged; the person secret never leaves process memory.

7.14 **Host/Origin checks with axum**: check the raw `Host` header (lowercased) against `127.0.0.1:<port>`/`localhost:<port>`; reject any request with an `Origin` header even if empty; do both before auth, and auth before routing (so 401 hides the route map; 404 only after auth).

7.15 **`irori attach` is spawned with stdin closed** and must never read stdin; on Windows the release exe is GUI-subsystem so no console flashes; under Rust use `#![windows_subsystem = "windows"]` and the same AttachConsole logic for CLI roles (console_windows.go). `mcp` must not attach a console (stdio pipes only).

7.16 **Crash output**: Rust has no runtime crash file. Install a panic hook (all threads' panics) that appends to `crash.log` with the start line, keep the 4 MiB `.1` rotation at startup, and open for append so every role shares the file. A Windows SEH/abort crash will leave nothing unless you add a minidump/`SetUnhandledExceptionFilter` writer.

7.17 **WebView data path changes**: Wails NSIS removes `%APPDATA%\irori.exe` (WebView2 user data) on uninstall (project.nsi:251); Tauri stores it under `%LOCALAPPDATA%\<identifier>\EBWebView`. Only the updater "Later" localStorage key is lost; acceptable but say so.

7.18 **Single-instance across Go→Rust**: a Go GUI and a Rust GUI will not see each other's single-instance lock (different mechanisms); the lockfile check (`ErrAlreadyRunning`, non-headless → hand off) is the real guard (core.go:122-124, gui.go:65-69). In Rust, when the lockfile names a live non-headless Core, forward to it via the single-instance plugin, or show nothing and exit.

7.19 **Revive on macOS** depends on the login-shell PATH fix running before anything else (gui.go:57); `claude --bg` may leave a child holding the pipes: stop reading 3 s after exit (WaitDelay) and never kill at 30 s.

7.20 **Version stamping**: Go injects `main.version`; the Rust build must produce `irori <version>` from `irori version` and report the same string in `/v1/health`, `irori.json`, `device.started`, MCP serverInfo; `dev` disables updates and never touches the network.

---

## Not determined

- Exact text of every tool result beyond those cited (board/explore/capture/notify/room/cardthread/notice/attachments): read the files or capture goldens (7.10).
- JSON shapes of `presence.AgentCard`, `presence.Change`, `inbox.*`, `coord.*`, `capture.*`, `mesh.DeviceRow`, `pair.Status`, `notify.Options/Target`, `ledger.Entry`, `core.BoardSummary/SharePreview`, and the root service result structs (cards_service.go types): only located here; take them from their areas' specs and `frontend/bindings/**/models.ts`.
- Behaviour of each root service method body beyond the signatures (board/cards/devices/boards/capture/inbox services) was not read line by line.
- Whether rmcp 3.8.0 answers `server/discover` statelessly and supports runtime tool removal: checked only that the protocol version and CustomNotification types exist.
- Whether `tauri-plugin-autostart` can register under value name `Irori` without the StartupApproved entry, and whether it supports SMAppService: not verified; plan on `winreg` + `objc2-service-management`.
- `build/darwin/package.sh` and `notarize.sh` contents (bundle layout, dmg, notarization) were not read.
- Wails v3 error-to-JS rejection shape (Error vs object) was not checked in the runtime source.
