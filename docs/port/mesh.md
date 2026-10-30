# Port spec: mesh, home server, discovery, presence (and go-room)

Scope: `internal/mesh` (+ `mesh/roomnet`), `internal/home`, `internal/discovery`, `internal/presence`, and `../go-room`.
Citations: `I:` = the Go irori checkout (`../irori-go/`, Go be67a86), `R:` = `../go-room/`. Line numbers are the file's own.

## 0. The one decision to make first

There is a clean seam in the Go code: `mesh.Transport` / `Room` / `Conn` (I:internal/mesh/mesh.go:99-138).

| Layer | What it is | Rust port |
|---|---|---|
| **Above the seam**: `mesh.Node`, `session`, frames, boards, files, `Servers`, `Devices`, peers/relay/secret files (I:internal/mesh/*.go except fake.go) | the sync protocol, epoch/key handling, the node.addr fold, data-dir JSON | **Port 1:1. Byte-compatible frames and files.** |
| **Below the seam**: `roomnet` + go-room (secretbox signaling, LAN `GR1` datagrams, the TLS hub, pion WebRTC data channels, the auth handshake, TURN, SplitTCP) | how bytes move | **Replace with iroh 1.x.** It is not wire-compatible with go-room. |

**Non-goal, and it is Stephen's call:** a Go build and a Rust build **will not connect to each other**. go-room is WebRTC/DTLS/SCTP plus a custom TLS hub, and iroh is QUIC with raw-public-key TLS. Neither can be shimmed into the other without reimplementing go-room in Rust. History still carries over, because the SQLite log, the row signatures (`SigningBytes`, I:internal/store/store.go:297-311), the sync frame JSON and every data-dir file stay identical. So: **cut both the PC and the Mac over together.** A Rust PC and a Go Mac in between simply don't sync, and they catch up once both run Rust, because rows are exchanged by vector (§2.3). The other option is to port go-room itself: the same pieces in Rust (`webrtc-rs` plus a hub/TURN reimplementation). That is a much larger body of work, and it defeats the iroh goal.

**Terminology trap:** `internal/discovery` is the **`irori.json` loopback lockfile** that agents and `irori mcp` use to find the local Core (I:internal/discovery/discovery.go:1-4). It is *not* the UDP 38419 LAN discovery. That is go-room's LAN carrier (R:lan.go), used through roomnet and the server directory (I:internal/mesh/servers.go:21).

## 1. What the area does and its interface

### 1.1 Responsibilities
- **mesh.Node**: starts networking only once the person has a second certified device (I:internal/mesh/node.go:207-214, 231-244). It keeps one "room" per open person-key epoch (node.go:399-447) and runs the sync protocol on every authenticated connection (session.go). It handles revoke and key rotation (node.go:986-1026), person-secret hand-off to devices still behind (node.go:826-937), restore and fork detection (session.go:417-453), attachment fetch (files.go) and board-log pull (boards.go). It reports device liveness and status (node.go:1028-1154, devices.go).
- **mesh.Servers**: folds `node.addr` rows into a directory of home servers and where to dial them (servers.go:309-384).
- **home.Server**: on the always-on PC it runs the hub (signaling), TURN, port mapping and an awake hold, and writes `node.addr` (I:internal/home/home.go:1-11).
- **discovery**: the `irori.json` lockfile (port, token, pid) for local agents.
- **presence**: watches Claude Code session files and transcripts on *this* device and builds `AgentCard`s (I:internal/presence/presence.go:1-18). It has nothing to do with the network: `agent.seen` and similar rows cross devices by ordinary sync. Device presence on the mesh is a separate thing: the `presence` envelope (node.go:25-34).

### 1.2 Public interface used by other areas (keep these names and semantics)
| API | Ref | Used by |
|---|---|---|
| `mesh.New(Options)`, `Run(ctx)`, `Kick()`, `KickBoards()`, `HasPeer()` | node.go:74-103,168,198,207,219; boards.go:59 | core |
| `Status() Status`, `Devices(detail) []DeviceRow`, `Liveness(dev) "connected"/"recent"/"offline"`, `Connected() int` | node.go:1065,1131; devices.go:39,186 | api, devices UI, roster (core.go:196) |
| `Revoke(ctx, dev, reason)` | node.go:986 | devices UI |
| `Offer(ctx, FileRef)`, `FetchFile(ctx, FileRef) (fromDev, error)`, `*OfflineError{Holders}`, `ErrNoFiles` | files.go:62,109,51-58 | core attachments |
| `CheckDevice(chain, self, id, pub)` (the single admission rule) | node.go:310-322 | hub, TURN, welcome, connections |
| `MeshSecret`, `MeshInfo`, `MeshApp` | mesh.go:32-56 | pair tests |
| `Servers`: `NewServers`, `Load`, `Follow`, `Row`, `List`, `Addrs`, `Heard`, `Worked`, `Hint`, `Hints`, `OnChange` | servers.go:128-384 | core, pair, home |
| `NodeAddr{…}.Same/Dial/Outside`, `LANPort=38419`, `Reach*` | servers.go:21-88 | home |
| `Relay`, `LoadRelay`, `SaveRelay`, `RelaySetter`, `ScrubURLs` | relay.go:16-114 | settings |
| `Transport` / `RoomConfig` (`GuestTTL`, `HubAddr`, `Offerer`, `OnMsg`, `OnPeer`) and `Conn.Fingerprints()` | mesh.go:68-138 | **pair** (I:internal/pair/pair.go:522-535, 706) |
| `roomnet.HostPort(in, port)` | roomnet.go:940-959 | home, pair UI |
| `home.New/Run/Kick/Set/SetAwake/Status/Settings`, `LoadSettings` | home.go:184-281, awake.go:9 | core, settings |
| `discovery.Write/Read/Remove/NewToken`, `Lockfile`, `ErrNotRunning` | discovery.go:14-42 | api, cli, mcp |
| `presence.New/Run/Snapshot/Subscribe/Rescan`, `AgentCard`, `Change`, `Slug` | presence.go:116-139; transcript.go:28 | core, roster |

## 2. Exact data formats

### 2.1 Sync frames (above the seam: keep byte-compatible)
One frame is one JSON object per transport message (frames.go:37-87). Go `encoding/json` rules apply. `omitempty` drops `0`, `false`, `""`, nil or empty maps, nil slices and nil pointers. Unknown fields are ignored. An unknown `type` is ignored (session.go:364-366). A missing `type` is a protocol error (frames.go:125-127).

| `type` | Fields sent | Ref |
|---|---|---|
| `hello` | `v`(=1), `deviceId`, `personId`, `epoch`, `coreVersion`, `schema`, `wallMs`, `vector`, `personPub`, opt `top` (hex RowHash of *receiver's* row at `vector[receiver]`), opt `topRow` (that Row) | node.go:731-744; frames.go:41-61,78-81 |
| `rows` | `events` [Row], opt `loose:true`, `batch` (int64, per connection, from 1; absent on loose/row.at replies) | frames.go:69-73; session.go:280-282, 361 |
| `vector` | `vector`, opt `ack:true`, opt `batch` (echo) | session.go:236, 545 |
| `ping` / `pong` | `t` unix ms (pong echoes) | session.go:211, 350 |
| `person.secret` | `epoch`, `from` (0 = recipient's current), `sealed` (base64 std) | node.go:858; frames.go:83-86 |
| `row.at` | `deviceId` (asker), `seq` | session.go:444; frames.go:25 |

**Row** (frames.go:91-99): `{"d":dev,"s":seq,"h":hlc,"a":actor,"k":kind,"p":base64(payload bytes),"sig":base64(sig)}`.
- `p` and `sig` have **no omitempty**: a nil value encodes as JSON `null`. Accept `null` and treat it as empty.
- `h` is a uint64 HLC = `unixMs<<16 | counter` (I:internal/hlc/hlc.go:33). That exceeds 2^53, so it must never pass through f64.
- `p` carries the exact stored payload bytes. **Never re-serialize a payload.** The signature and `RowHash` cover those bytes (store.go:173-176, 297-311; test `TestBatchesKeepTheLimits` round-trips `{"b": 2 }` with its spaces).
- `vector` is `{"deviceId": maxSeq(int64)}`.

### 2.2 Sync channel framing on the wire
go-room's sync channel prefixes each message with a 4-byte big-endian length, cut into SCTP pieces of 16 KiB or less (R:channel.go:83-85, 110-153, 177-216). The cap is **8 MiB** (`mesh.MaxMessage`, frames.go:33, passed in at roomnet.go:642; go-room's own default is 4 MiB, R:mesh.go:28). In Rust over QUIC, keep the `u32 BE len || bytes` message framing and drop the SCTP chunking. A length over 8 MiB, or a stream that ends mid-message, closes the connection.

### 2.3 Blob and board streams (keep the protocol, move it to QUIC streams)
- **Blob** (files.go:18-31, 207-238, 242-291): the label is `blob/<16 hex>` (8 random bytes). The opener writes `u32 BE len || JSON {"id","sha256","offset","len"}`. The request is 4 KiB at most and a length of 0 is refused. The server then writes raw bytes, `min(size-offset, len)` of them (len 0 means to the end), and closes. A refusal is a close with **zero bytes**, which the client reads as `errNotServed` (files.go:174, 199-201). The server serves only a hash that some `attach.offered` row names **and** that it holds, and only to a `certified` peer (files.go:248-271). The copy buffer is 16 KiB (files.go:27). Client: one fetch per sha at a time (`lockFetch`, files.go:311-328). It reads at most `want+1` bytes; more than `want` is a mismatch and the partial file is dropped. It tries 3 times per holder and resumes by offset (files.go:30, 151-166). Holder order is the origin, then other holders in log order (files.go:132-141). After a good fetch the device writes its own `attach.offered` (files.go:154).
- **Board** (boards.go:25-38, 104-247): the label is `board/<boardId>`. The puller writes a chunk `{"vector":{...}}` (1 MiB at most). The server writes chunks `{"events":[Row]}` from `Missing(vector, 500)`, cut by `batches`, until 2000 rows in all (`boardPullRows = 4*MaxBatchRows`) or nothing is missing, then closes. Chunk = `u32 BE len || JSON`; len 0 or len > limit is an error (boards.go:233-247). The puller reads until EOF, stops early past 2000 rows, and inserts per device in (device, seq) order. It retries runs refused with `ErrNoCert` while any other run made progress (boards.go:146-185). Pulls run on hello (polled every 20 ms until hello is done, boards.go:76-84), on every `Vector` tick (30 s) and on `KickBoards`. Only certified peers are served, and only boards this device holds (boards.go:189-197).

### 2.4 Mesh topic and person-secret crypto (port exactly)
- `MeshSecret(seed, personId, epoch)` = HKDF-SHA256(ikm=seed, salt=`"irori/mesh/1"`, info=`personId || u64be(epoch)`, 48 bytes). key = [0..32], topic = `"irori/mesh/1/" + hex([32..48])` (mesh.go:32-56). With iroh the topic is only an *identifier* (§5.3), but it must be derived exactly like this.
- `person.secret` (secret.go:20-65): k = HKDF-SHA256(ikm=old seed, salt=`"irori/person-secret/1"`, info=`personId || u64be(epoch) || recipientDeviceId`, 32 bytes). sealed = `nonce(24) || NaCl secretbox(newSeed, nonce, k)`. Opening fails if `len < 24+16`. Test: `TestPersonSecretSealing`.
- `RowHash` = hex(sha256(SigningBytes || sig)) (store.go:173-176). It is used for `top` and revoke `cutHash`.

### 2.5 Data-dir files (device-local; data dir is `%LOCALAPPDATA%\irori` on the PC, as seen; Mac path: see the paths spec)
All of these are written as `MarshalIndent(v, "", "  ")` (two spaces; Go sorts map keys, so Rust uses `BTreeMap`, not `HashMap`, for `mesh-peers.json` and `hub-addrs.json`) to `path.tmp`, mode 0600, then renamed over the target.

| File | Shape | Ref | Notes |
|---|---|---|---|
| `mesh-peers.json` | `{"<deviceId>": {"epoch": int, "personPub": hex?}}` | peers.go:14-19, 39-63 | Only entries with epoch>0. Unreadable means empty. Present on the PC (keys `epoch`,`personPub`). |
| `hub-addrs.json` | `{"<serverId>": {"last": "host:port"?, "hint": ["host:port"]?}}` | servers.go:93-98, 372-384 | Absent on the PC (it is the server). Under iroh, `last` and `hint` stay host:port strings; reuse them as direct-address hints (§5.4). |
| `relay.json` | `{"credentialUrl"?, "turnUrls"?: []}`; file **removed** when there is no credential URL | relay.go:16-25, 90-107 | Absent on the PC. **No iroh equivalent** (§7). |
| `home-server.json` | `{"on":bool,"port"?:int,"outside"?:bool,"public"?:str,"tcpExternal"?:u16,"udpExternal"?:u16,"awake"?:bool}` | home.go:42, 60-108 | PC: `{"on":true,"port":25817}`. **`outside` and `awake` are tri-state**: absent means on (home.go:79-82). Rust must use `Option<bool>` and must never write `false` for an absent value. |
| `turn.key` | sealed 32-byte HMAC key, via `Ident.LoadOrCreateSecret` | home.go:46, 413 | Secret, never read here. Unused under iroh. Leave it in place. |
| `irori.json` | `{"pid","port","token"(64 hex),"version","deviceId","headless","startedAt"(RFC3339Nano)}` | discovery.go:17-28 | Written with CreateTemp `irori.json.*.tmp` in the dir, chmod 0600, fsync, rename with retry (lockfile.go:28-75). |

### 2.6 Log row payloads this area writes
- `node.addr` (store kinds.go:45; servers.go:34-51): `{"v":1,"port"?:N,"lan"?:[ipv4],"v6"?:[ipv6],"public":{"ip","tcp","udp"}|null,"via":""|"upnp"|"natpmp"|"manual","reach":"ok"|"cgnat"|"unmapped"|"off","at":unixMs}`. `public` has **no omitempty**, so `null` is written. `via` and `reach` are always present. The off row is `{"v":1,"public":null,"via":"","reach":"off","at":…}` (home.go:300). `via` is the portmap `Method()` verbatim (R:portmap/upnp.go:44, natpmp.go:38). `Same` compares the JSON with `at` zeroed (servers.go:54-59). Rows are written only on change, compared with the newest row in the log, not with memory (home.go:327-331, 686-701).
- `attach.offered`: `{"sha256","size","name"?,"mime"?}` (files.go:33-39). The actor is `"irori"`.
- `person.device.revoked` + `person.key.rotated`: built by identity, appended here (node.go:1010-1023).

### 2.7 Signaling envelopes (below the seam; for reference and for the go-room port option)
- Envelope JSON `{"v":1,"id":16hex,"from","to"?,"type","at":unixMs,"body"?}` sealed as `nonce(24)||secretbox(json, key)` (R:signal.go:31-39, 157-176). Receive drops: no open, `v!=1`, from self, `to` set and not self, |age| > 2 min (`DefaultMaxAge`). Dedupe by id, cap 2048, prune entries older than 2*MaxAge (signal.go:181-259).
- Mesh-level types: `presence` `{"deviceId","epoch","coreVersion"}` and `leave` `{}` (node.go:25-34). go-room owns `rtc.want/offer/answer/bye` (R:mesh.go:19-24).
- LAN datagram: `"GR1" || tag8 || sealed`, tag = sha256(`"go-room/lan/1"||topic||u64be(unix/300)`)[:8]. Accept windows w-1, w, w+1. Max 8 KiB. 200 opens per second. udp4 wildcard bind on :38419 (R:lan.go:14-37, 111, 185-214, 216-270).
- Hub wire: TLS 1.3, ALPN `go-room-hub/1`, self-signed P-256 certificate that is never verified. Frames are `u32be len(type+body) || u8 type || body`, 64 KiB max, types 1..13. Member hello signs `ctx||0||exporter32||u8 len||id` with the exporter label `EXPORTER-go-room-hub` (R:hubwire.go:19-213). Ping every 15 s, idle 45 s, guest grace 45 s, guest TTL 10 min at most (R:hubwire.go:58-67, hub.go:25).
- TURN REST credentials: `"<expiry>:~"+b64url(deviceId XOR HMAC-SHA256 keystream)`, credential `b64std(HMAC-SHA1(key, username))`, realm `relay`, TTL 12 h (R:turn.go:25-51, 255-319; I:home.go:57).

## 3. Rules, constants, timing

### 3.1 Node
| Constant | Value | Ref |
|---|---|---|
| `Protocol` (hello v) | 1; mismatch → close "version" | frames.go:12; session.go:378 |
| `MinPeerSchema` | 3; older → `schemaOld`, close "version" | node.go:21; session.go:384 |
| `MaxBatchRows` / `MaxBatchBytes` / `MaxMessage` | 500 / 1 MiB / 8 MiB | frames.go:31-33 |
| Row size estimate | `96+len(d)+len(a)+len(k)+(len(p)+len(sig))*4/3`; a single oversized row goes alone | frames.go:112-114, 134-151 |
| Held rows per session | 2000 rows or 8 MiB (payload+sig); a dup seq doesn't count; over the cap it is dropped | node.go:62-65; session.go:662-685 |
| Intervals | presence 30 s, ping 15 s, vector 30 s, backoff 1 s doubling to 60 s | node.go:38-58 |
| Liveness | connected = ping within 45 s on a live helloed conn; recent = sync within 10 min; else offline | node.go:68-71, 1131-1154 |
| Ahead flag | shown while `now - lastAhead < 2*Vector` | node.go:1083 |
| In-flight batch timeout | `2*Vector` without an ack → resend | session.go:228-230 |
| Control queue | 64 frames; when full it drops (logged) | session.go:60, 73-79 |
| `maxPending` secrets | 8 (the newest kept) | node.go:862 |
| Shutdown wait | 5 s for sessions | node.go:721-727 |

**Session algorithm** (session.go):
1. The writer sends `hello` first (session.go:176). The reader requires `hello` before any other frame, else it closes `"protocol: frame before hello"` (335-338). A second hello is ignored.
2. onHello checks in order: `v`, then `deviceId==conn.ID && personId==mine` (else "person"), then schema, then `hex(conn.Pub()) == cert.DevicePub` (else "key") (371-397). It records epoch, personPub, skew (`wallMs - now`), lastSync and lastPing. It saves `mesh-peers.json` if epoch or pub moved.
3. Fork check: if `top != ""` and `0 < at=vec[self] <= have` and my row at `at` hashes differently, then `stop(self, forkedSelf)` and send my row as a loose rows frame (417-423).
4. Restore: if `at > have` and not self-stopped, then require `provesOwn(topRow, at, top)`, meaning signed by **my** device key (node.go:801-810). Without it: refused++, log, ignore. With it: if `have>0`, set `ownCheck=have` and send `row.at{deviceId:self, seq:have}`. Then `Store.HoldAppends(at)` and `restoring=peer` (428-453).
5. Then: `connected(peer)` resets the backoff, `sendSecret`, `Kick`, `poke`.
6. Writer loop: control frames take priority. Next: once, after hello, the **loose** set: every `person.*` row with seq > peer's vector, plus honoured revokes from `refused_events` (signed), plus `ForkTwoRows` `Other` rows (signed). Then, while no batch is in flight, one batch from `Missing(peerVec with stuck devices set to MaxInt64, and self set to MaxInt64 if self is stopped)`, 500 rows at most, numbered `batch++` (241-322).
7. onRows (non-loose): group by device in arrival order. Skip stopped devices. If the device has held rows, or it is self during `ownCheck`, hold. Else `InsertBatch` and route the refusal of the rest. Then `synced` and **ack** = `vector{ack:true,batch:f.batch}` (500-531).
8. onRows loose: insert each row alone. A `person.*` row refused with Gap/NoCert is `Chain.Learn`ed, repeating until no progress; learned → Kick. A self row equal to `ownCheck` clears the check and retries held rows (588-636). Afterwards send a non-ack vector.
9. onVector: merge by max. Un-stick devices that advanced. An ack matching the in-flight batch (or `batch==0`) clears in-flight. A device whose acked seq is in `[first-1, last)` of the batch extent is marked stuck until the next tick (465-497). The tick clears stuck and resends the periodic vector (225-237).
10. Refusal mapping (549-583): NoCert → hold; Ahead → `ahead[dev]=now`; self with Fork/HLCOrder → stop self and send my row loose (seq, else seq-1); Fork → stop dev "signed two different histories"; HLCOrder → stop dev "its clock went backwards"; Gap/Closed/Canceled → nothing; anything else → refused++ and log.

**Epoch rooms** (node.go:326-514): keep the current epoch open. Also keep each older epoch that a certified, unrevoked device last helloed with, if its key is still held; otherwise keep `PrevEpoch` (454-478). `hasCurrent` = peer epoch > cur, or equal with a matching key or an empty key (351-356). On an older epoch, `allow` only devices that are not hasCurrent. When only the current epoch is wanted, `Ident.Forget(cur)` drops the older keys (410-416). A room whose topic changed (ReplaceSecret) is closed and reopened. Closing sends `leave` first (439-442). A join failure sets `detail = "Can't open the network: …"` and is retried on the presence tick. `Status` reports `error` when on, 0 topics and detail set (1070-1072).

**Rotation tie-break** (node.go:945-971): the winning `person.key.rotated` for an epoch is the lowest `deviceId`, then the lowest seq, among writers ever certified (or self). A device that lost never hands its key on (841-843). A pending secret is adopted only when the winner names its pub: `AdoptSecret` for a newer epoch, `ReplaceSecret` for the same one (897-937). `sendSecret` goes once per (epoch/pub) per connection (session.go:96-105). It is sealed from the peer's epoch, or from `PrevEpoch` when the two rotated apart (node.go:833-840).

**Reconnect** (node.go:591-636): after a session ends, retry 1 s, doubling to 60 s, until connected, connecting in every room that allows the peer. Presence on a topic is also a `Connect` (546-556). `leave` zeroes lastPresence.

**Devices warnings** (devices.go:46-84): precedence and exact strings live in that function. Port the text verbatim. "Removal pending: seen by %d of %d devices." uses `removalSeen` (105-144).

**Servers.Addrs order** (servers.go:269-308): the heard LAN IP with the row's port (or the first hint's port), then the row's `Dial()` (LAN, then v6, then public tcp if reach ok), then `last`, then the hints only while there is no row. Deduplicate. A row with reach `off` gives none. `Heard` is kept only for ids that have a row or a hint (313-325). A revoked writer's rows are ignored (220-236). `Follow` reloads after subscribing (189-205, test `TestServersFollowKeepsARowWrittenBeforeItSubscribed`).

**relay.json rules** (relay.go:32-55): the credential URL must be https, or http on 127.0.0.1/::1/localhost. TURN URLs need `turn:`/`turns:` and need a credential URL. `ScrubURLs` regex `(https?://[^\s?#"']*)[?#][^\s"']*` → `$1?…` (relay.go:75-79). It must be applied to every log and status line (§8).

### 3.2 go-room (behaviour the iroh build must reproduce or consciously drop)
| Item | Value | Ref | iroh equivalent |
|---|---|---|---|
| Offerer | the lower id offers; the other sends `rtc.want` at most once per 1 s | R:mesh.go:33, 253-258, 269-290 | lower id dials (§5.2) |
| Connect timeout / auth timeout | 30 s / 10 s; gather cap 4 s; ICE timeouts 5/15/2 s | R:mesh.go:27-40, 184 | `connect` timeout 30 s |
| Auto reconnect after an up peer *fails* | 6 tries, 1 s doubling; never after a local close, a bye, a refusal or a replace | R:mesh.go:36-39, 553-565 | same rule, keyed on the close code |
| Make-before-break | a newer offer is answered beside an up peer and replaces it only after the handshake; the old one closes with `ErrReplaced` | R:mesh.go:374-438, 597-625 | duplicate-connection rule (§5.2) |
| Bye | `rtc.bye{session,token,reason}`; it closes an up peer only with the token from inside DTLS; 1 s grace for a bye after a transport drop | R:peer.go:453-512, mesh.go:451-470 | QUIC `close(code, reason)` is in-band and authenticated, so no grace is needed |
| Streams | MaxStreams 4 outgoing (semaphore), 8 incoming; label `sync` reserved; Close flushes up to 30 s | R:peer.go:150-200, 423; channel.go:351-363; roomnet.go:643 | §5.2 |
| Flow control | high water 1 MiB / low water 256 KiB | R:channel.go:20-25 | QUIC flow control |
| Hub limits | members 64, guests 8, handshakes 4 per IP (an IPv6 /64 counts as one) and 64 in all, 8 topics per conn, 2 guest topics per member, pub 50/s burst 100, fail backoff 1 s doubling to 30 s | R:hub.go:49-71 | relay `Limits` + access control |
| SplitTCP | first byte 0x16 → hub, 0x00/0x01 → TURN; 5 s peek; 64 pending, 8 per IP | R:split.go:16-121 | not needed (the relay is HTTP) |
| TURN | 12 h TTL, 8 allocations per user, 32 total, 4 guest, 64 TCP / 8 per IP, 10 min allocation lifetime, 5 min TCP idle; peer filter: members to private subnets plus relayed ports, guests only to relayed ports | R:turn.go:37-57, 419-444 | the iroh-relay forwards by EndpointId |
| Relay credentials | refresh at 80% of TTL, at least 15 s; retry 5 s doubling to 2 min; 10 s fetch timeout; `?peer=<deviceId>` | R:ice.go:107-116, 261-373 | n/a |
| ICE list | only this device's own server, connected hubs, the guest hub and the advanced relay; **never public STUN** (`DefaultSTUN` only applies when `ICE==nil`) | roomnet.go:438-491; R:ice.go:27-30; R:peer.go:257-260 | **never use `presets::N0`** (§5.1) |
| LAN retry | port taken → retry every 30 s | roomnet.go:115, 197-208 | same, for the 38419 lookup socket |
| Nothing before first Join | a lone device binds no socket and dials no hub | roomnet.go:64-65, 137-164 | bind the iroh Endpoint lazily (test `TestNothingOpensBeforeTheFirstJoin`) |

### 3.3 Home server
| Rule | Ref |
|---|---|
| Runs only if `on` **and** (another certified device **or** an open pairing window) (`Wanted`) | home.go:293-304; core.go:293 |
| Port N: random in 20000-29999 on first start, saved; on a bind failure up to 8 tries with new random ports; a moved port sets `moved` and the status line "Port %d was taken…" | home.go:51-54, 374-409, 792-794, 819 |
| TCP+UDP both on N; `tcp4/udp4` while outside is off, dual-stack while it is on; restart when the outside setting flips or the primary LAN IPv4 changes | home.go:338-360, 305-315 |
| LAN address: private IPv4 on up interfaces with prefix /8../30, filtered to the default gateway's subnet (cached by address set; a failed lookup is not cached) | home.go:826-875; R:lan.go:442-474 |
| v6: global unicast, not private, not 4in6; listed only while outside is on | home.go:879-902, 634-640 |
| TURN only with a LAN IPv4; key `turn.key`; realm `relay`; Authorize = cert exists and not revoked (checked on every request) | home.go:412-430, 505-513 |
| Hub Authorize = `CheckDevice`; topics = this device's own open rooms + open guest topics; the LAN bridge carries guest topics | home.go:431-445 |
| On every person row: `dropRevoked` (hub `Drop` + guest topics closed) | home.go:240-242, 517-533 |
| Port mapping when outside is on and there is no manual public address: tcp+udp N→N (or the last external port), description `"irori"`; stale mappings from an earlier run removed first; granted external ports saved | home.go:537-584, 672-684 |
| `addr()` waits while the mapper is pending (no flapping "unmapped" row); manual public → via `manual`, reach `ok`; mapped → method, ok/cgnat | home.go:617-669 |
| Re-check every 30 s (DHCP and network changes) | home.go:191-193, 246-248 |
| Status `Reach`: `home`/`outside`/`forward`/`cgnat` with fixed lines; Public-profile line overrides; port-moved line | home.go:717-799 (strings at 778, 783, 788, 793, 796) |
| Awake hold while running and setting on; failure keeps serving, line "Can't keep this PC awake: …" | awake.go:27-69 |

### 3.4 Presence (agent sessions)
| Rule | Value | Ref |
|---|---|---|
| Session files | `<ClaudeDir>/sessions/<digits>.json` only; **never open `*.key`**; read ≤64 KiB; invalid JSON → skip (no repair) | session.go:12, 37-94 |
| Fields | pid, sessionId, cwd, name, nameSource, status, kind, entrypoint, version, peerFeatures, startedAt(ms), max(updatedAt,statusUpdatedAt)(ms), procStart (string digits / lstart text / number) | session.go:52-94 |
| Liveness | Windows: FILETIME ticks, slack 10,000,000 (=1 s); macOS: lstart→unix s, slack 2 s, ignore procStart > 2^36; other: pid only | live_windows.go:7-28; live_darwin.go:12-36; live_other.go:9 |
| lstart | `"Mon Jan _2 15:04:05 2006"` UTC after collapsing spaces; year<2000 rejected | session.go:132-141 |
| Transcript | `projects/<Slug(cwd)>/<id>.jsonl` else glob `projects/*/<id>.jsonl`; id must be `[0-9A-Za-z_-]+` | transcript.go:28-71 |
| Tail | last 256 KiB, start 1 byte early, skip first partial line; LastText ≤240 runes with `…`; non-sidechain assistant text only | transcript.go:16-24, 109-164, 217-243 |
| Permission mode | `permission-mode` rows or non-sidechain `user` rows with top-level permissionMode; cold-start backward scan ≤8 MiB once per file | transcript.go:21, 169-213; watcher.go:357-363 |
| Sticky across tails | title/custom/ai/lastText/mode kept from cache when the file grew; reset on shrink | watcher.go:332-356 |
| Naming | precedence in `name()`; sources `user`/`irori`/"" | watcher.go:245-279 |
| Timers | fallback interval 2 s, safety rescan 30 s, settle 100 ms, sub buffer 64 (drop when slow) | watcher.go:16-23 |
| Change detection | Updated only if a non-LastActivity field changed or LastActivity moved ≥1 s; removed cards get status `gone`, UpdatedAt=now | watcher.go:392-442 |
| Sort | LastActivity (else UpdatedAt) desc, then Key | watcher.go:446-459 |
| Card key | `"<deviceId>/<sessionId>"`, `agent:"claude"`, peerFeatures `[]` not null | watcher.go:203-219 |

### 3.5 discovery
Token = 32 random bytes, hex. `Read` returns `ErrNotRunning` if the file is missing or the pid is dead. `Remove` only removes the file if it names our pid (lockfile.go:17-120). The Windows rename is retried with backoff from 5 ms, doubling, for about 1 s total (lockfile.go:66-75).

## 4. Platform-specific code
| Where | What | Rust |
|---|---|---|
| R:lan_windows.go | Network List Manager COM (`{DCB00C01-…}`) → interfaces on the **Public** profile (category 0) get no broadcast; also drives the "set network to Private" lines | `windows` crate COM (`INetworkListManager`) |
| R:lan.go:105-117 | udp4 wildcard bind (Windows only delivers broadcasts to a wildcard bind); ignore read errors (ICMP unreachable on Windows) | `socket2` with SO_BROADCAST and SO_REUSEADDR off |
| I:home.go:856-875 | default gateway (`jackpal/gateway`, runs `route print` on Windows) | `netdev` 0.47 (`get_default_gateway`) |
| I:internal/awake | `Holder.Acquire()→Hold.Release()` (another area; Windows `SetThreadExecutionState`, macOS IOPMAssertion) | `windows-sys`, IOKit FFI |
| presence live_* | `proc.Alive`, `proc.StartTime`, `proc.FiletimeTicks` (another area) | `windows-sys` `GetProcessTimes`; macOS `proc_pidinfo` / `sysinfo` 0.40 |
| presence exit_* | Windows `OpenProcess(SYNCHRONIZE)`+`WaitForMultipleObjects` with a stop event; macOS kqueue NOTE_EXIT via `proc.WaitExit`; other: none (rescan) | `windows-sys`; `kqueue` 1.2 |
| presence run.go | `fswatch` (sessions non-recursive, projects recursive, Overflow → full scan, `ErrUnsupported` → interval) | `notify` 8.2 |
| discovery | rename retry for Windows sharing violations | same loop |
| Tests | `LoopbackOnly`: every socket on 127.0.0.1 so no Windows Firewall prompt (R:mesh.go:825-831) | **required**: bind iroh to 127.0.0.1 in tests, no relays, no mDNS |

## 5. Rust and iroh design (how each need is met)

### 5.1 Crates (crates.io, checked 2026-10-29)
| Crate | Version | Use |
|---|---|---|
| `iroh` | 1.4.1 (Sources: docs.rs/iroh/latest, docs.rs/iroh-relay/latest, crates.io API) | Endpoint, Connection, streams, hooks, `MemoryLookup`, custom `AddressLookup` |
| `iroh-relay` | 1.4.1, feature `server` | home-server relay: `server::{Server, ServerConfig{relay, quic}, RelayConfig{http_bind_addr, tls, limits, key_cache_capacity, access}, Access::{Allow, Deny{reason}}}` |
| `iroh-mdns-address-lookup` | 0.6.1 | **not recommended** (it publishes the stable device id on every LAN, §7) |
| `iroh-gossip` | 0.101.0 (iroh ^1) | optional (§5.3) |
| `iroh-blobs` | 0.104.0 | **not used**: BLAKE3-addressed, while `attach.offered` and `FileRef` are sha256; the blob/ protocol above is simpler and keeps holders and offers in the log |
| `portmapper` | 0.20.0 | UPnP/NAT-PMP/PCP for port N (or `igd-next` 0.18 + own NAT-PMP). It must report external IP, external ports and method for `node.addr` |
| `tokio` 1.54, `serde`/`serde_json` 1.0.154, `base64` 0.23, `sha2` 0.11, `hmac` 0.13, `hkdf` 0.13, `crypto_secretbox` 0.1.1, `ed25519-dalek` (match iroh's version, see §7), `socket2` 0.6.5, `if-addrs` 0.15 / `netdev` 0.47, `notify` 8.2.1, `windows-sys` 0.61.2, `kqueue` 1.2.1, `sysinfo` 0.40, `scrypt` 0.12 (pair) | | |

### 5.2 Connections (replaces go-room Mesh/Peer/Channel/Stream)
- **One `Endpoint` per process.** Start from `Builder::empty()` (confirmed), **never `presets::N0`** (that would use n0's public relays and DNS/pkarr and break "no third party", test `TestICENeverListsPublicSTUN`). Use `.secret_key(…)`, `.alpns(vec![b"irori/mesh/1", b"irori/pair/1"])`, `.relay_mode(RelayMode::custom([home-server RelayUrls]))` (variants `Disabled/Default/Staging/Custom(RelayMap)`; `Disabled` when no server is known), `.address_lookup(MemoryLookup + Irori LAN lookup)` and `.hooks(…)`. Bind lazily on the first "Join" (§3.2).
- **Identity:** recommended: the iroh `SecretKey` **is** the device ed25519 seed, so `EndpointId` == device pub. Then `conn.remote_id()` replaces go-room's auth handshake (R:mesh.go:707-813), and `CheckDevice(chain, self, remote_id.to_string_hex, pub)` runs in `EndpointHooks::after_handshake` (reject = close before any frame). The other option: a separate iroh key, bound by an extra first message carrying the device key's signature over `export_keying_material`. This needs the identity area to expose the seed (the device key is sealed, `device.key`).
- **Epoch binding** (replaces "a connection belongs to a room/topic", R:mesh.go:728-743 signed the topic hash). New Rust-only protocol: the dialer opens the first bi stream, label `sync`. Both sides' first message on it is `{"v":1,"epoch":E,"proof":hex(HMAC-SHA256(MeshSecret(E).Key, "irori/mesh-open/1" || EKM32))}`, with EKM = `export_keying_material(32, b"EXPORTER-irori-mesh", b"")`. Each side checks the proof with its own key for E and `allow(E, peer)`. The session's `epoch` = E. Then the hello and the frames follow unchanged. `Status.Topics` (node.go:1069-1072) becomes the number of epochs in `wantEpochs` while the endpoint is bound, and 0 when binding failed, so the `on` + 0 topics + detail → `error` rule still works. The proof shows the peer holds that epoch's person secret, just as being in the room did.
  - As built in M5 (`irori_net::transport`): the message is `{"v":1,"epoch":E,"deviceId":id,"proof":…}` and the HMAC input is `"irori/mesh-open/1" || EKM32 || role` (0 from the dialer, 1 from the acceptor). `deviceId` lets the acceptor run the mesh's admission by device id; the role byte keeps one side from passing by echoing the other's proof.
- **Who dials:** the lower device id dials. The higher one dials too when it has gone longer than its backoff without a connection (iroh has no "want" message). On two live connections between one pair, keep the one **dialed by the lower id** and close the other with code `REPLACED` (mirrors `ErrReplaced`). Do not auto-retry after REPLACED, a local close or a refusal.
- **Streams:** every bi stream starts with `u32 BE len || label` (UTF-8, ≤255). `sync` is opened once by the dialer, and messages on it are `u32 BE len || bytes` (≤8 MiB). For `blob/…` and `board/…`, the opener writes the request and the server writes the reply then `finish()`es; an EOF on the read side ends it. A refusal is `finish()` with no bytes. Outgoing semaphore 4; `set_max_concurrent_bi_streams(1 + 8)`.
- **Close reasons** (keep the strings; carry them as `close(code, reason)`): `version, person, key, protocol, protocol: frame before hello, hello, revoked, epoch, leaving, shutting down, not running, replaced, refused, timeout`. The mapping from `Conn.Err()` to auto-reconnect follows R:mesh.go:553-565.
- **Path / RTT:** `conn.paths().iter().find(|p| p.is_selected())`, then `is_relay()` → `"relay"` else `"direct"`, and `rtt()`.
- **Fingerprints()** (pairing SAS, I:pair.go:706): no DTLS certificates exist. Return `(fp, fp)` where fp = colon-hex of `export_keying_material(32, b"EXPORTER-irori-sas", b"")`. Both sides get the same bytes, which binds the SAS to the QUIC/TLS session as before. This changes `pair.Info` inputs, so coordinate with the pair spec.

### 5.3 Finding peers (replaces signaling: presence, LAN carrier, hub)
| Go need | iroh build |
|---|---|
| `presence` every 30 s → `Connect` (node.go:546-556), `lastPresence` | No envelope bus is needed for 2 to N devices. For each certified, unrevoked device whose `allow` passes and that has no session, try `endpoint.connect(EndpointAddr{id, addrs})` with the 1 s→60 s backoff, plus an immediate try on any address event. Keep `lastPresence` = last address-lookup sighting. |
| `leave` | connection close `leaving`, plus the LAN lookup's "bye" datagram |
| LAN carrier on UDP 38419 (finds devices with no server) | **Custom `AddressLookup` on UDP 38419**, same privacy design as `GR1`: datagram `"IR1" || tag8 || nonce24 || secretbox(json{v,id,addrs[],relay?,at})`. Tag as R:lan.go:185-195 but label `"irori/lan/1"`, key = current MeshSecret key, ±1 window, 2 min age, 200/s rate, broadcast to private subnets skipping Windows Public networks, every 30 s and on change. It feeds iroh via `AddressLookup`. `OnHeard` → `Servers.Heard` stays. |
| Hub (TLS signaling at the server) | **iroh-relay** on the server: devices away from home reach each other through it, and the relay URL goes in their `EndpointAddr`. |
| Home server directory → hub dials | `Servers.List()` → each server's `RelayUrl` (built from `Addrs()`, `http(s)://host:N`) into the `RelayMap`, and the server's own id + direct addrs into `MemoryLookup`. `Worked(id, addr)` = the address the relay connection came up on. |
| TURN (relay path for blocked networks) | the relay path through the same iroh-relay. `RelayOnly` tests use an endpoint with IP transports cleared (`clear_ip_transports`). |
| iroh-gossip | Optional. Use it only if a broadcast bus is wanted (boards step 5). If so: TopicId = sha256(mesh topic), members gated by `after_handshake` on the gossip ALPN. Gossip content is not secret from other topic members. |

### 5.4 Home server on iroh
- Bind **TCP N** for the relay HTTPS server (`TlsConfig.https_bind_addr = 0.0.0.0:N`, or `[::]:N` while outside is on). With `RelayConfig.tls` set, iroh-relay 1.4.1 serves the relay on `https_bind_addr`; the mandatory plain `RelayConfig.http_bind_addr` goes to an unforwarded loopback port (docs.rs iroh-relay 1.4.1 `RelayConfig`, `TlsConfig`). Bind **UDP N** for QUIC address discovery (`ServerConfig.quic`), so "forward TCP and UDP port N" (home.go:788) stays true. Port 25817 is kept from `home-server.json`.
- **TLS** (superseded by §8.1: the M1 spike settled D3 as a device-key certificate pinned to `devicePub`, so no `relayCert` field)**:** the iroh-relay QUIC (QAD) side requires TLS, and a home PC has no domain name. Use a self-signed certificate and **publish its SHA-256 (or DER) in `node.addr`** as a new field (`"relayCert"`; payloads only gain fields). Clients trust exactly that certificate through `Builder::ca_tls_config`. Server side, `RelayConfig.tls` is an `Option`, but `QuicConfig` fails to spawn without TLS 1.3, so a TLS-less relay loses QAD and is ruled out (docs.rs iroh-relay 1.4.1 `QuicConfig`). The server takes `CertConfig::Manual { server_config }`; the client takes `CaTlsConfig::custom_roots` or `custom_server_cert_verifier`, which apply to relay connections (docs.rs iroh 1.4.1 `tls::CaTlsConfig`).
- **Access** (`RelayConfig.access: Arc<dyn DynAccessControl>`, implemented via trait `iroh_relay::server::AccessControl { on_connect(&ClientRequest) -> impl Future<Output = Access>; on_disconnect(EndpointId, ConnectionId) }`, id from `ClientRequest::endpoint_id()`; `Access::{Allow, Deny { reason: Option<String> }}`, docs.rs iroh-relay 1.4.1): members via `CheckDevice` (cert pub == id); revocation → `Deny` plus dropping live clients (`dropRevoked`, test `TestRevokeDropsTheHubConnectionAndTURN`). **Guests (pairing B away from home)** have no cert. Admit unknown ids only while a member has a pairing window open on this server: ≤8 guests, ≤2 windows per member, TTL ≤10 min, closed 45 s after the owner disappears (R:hub.go:25, 49-71, 459-466). The window is opened by a member over its mesh connection to the server (a new frame or stream `guest.open{ttlMs}`, the replacement for frame 7). Members still refuse guest ids on `irori/mesh/1`; only `irori/pair/1` accepts them, and only during the window.
- **Bridge** (a home device with only the code ↔ a member away, R:hub.go:568-629, test `TestPairThroughTheServersBridge`): under iroh, B at home finds A through the relay once A's pairing announcement reaches B. That needs a pairing-room LAN lookup keyed by the **code-derived** secret (`SecretFromCode("irori-pair-v1", code)`, I:pair/code.go:32-104) that carries A's EndpointAddr (relay URL included).
- `node.addr`, the port-mapping, awake and Status logic port unchanged (§3.3). `turn.key` is left alone.

## 6. Tests to port (what each proves)
Use a Rust `FakeNet` equal to I:internal/mesh/fake.go: in-memory rooms, ordered envelopes, `Partition/Isolate/Heal/HealAll/Drop/Connected/Joins/Frames/Members/Topics`, `Path()=="direct"`, RTT 1 ms, real Accept/Allow with real keys. Fast intervals as in harness_test.go:25.

**mesh (FakeNet), sync_test.go:** VerifierResetOnDeadContextKeepsCerts · SingleDeviceOpensNothing · PairingStartsTheNetAndEarlierRowsReplicate · LocalKindsNeverCross (`local_events` never sync) · ForgedRowsRefusedEvenWithCertCheckOff (both layers hold) · CatchUpThroughTheNodeWithTheMacGone (a third device relays history) · BackfillInBatches (1300×4 KB rows) · ReconnectAfterADrop (partition, heal, drop) · RevokeConvergesAndRotatesTheKey · OfflineDeviceGetsNewKeyOnPreviousTopic · TwoDevicesThatRevokeApartSettleOnOneKey (lowest-id rotation wins) · ADeviceOfflineThroughTwoRevokesCatchesUp (jump 1→3) · KnownEpochsSurviveARestart (`mesh-peers.json`) · ARestoredDeviceGetsItsOwnRowsBack · ARestoredDeviceThatWroteOverItsRowsIsAFork · …WroteFewer…IsAFork (`row.at` check) · AForgedVectorDoesNotHoldAppends (`provesOwn`) · RowsFromAClockTenMinutesAheadWaitThenLand · HeldRowsAreCapped · BatchesKeepTheLimits (500 rows, 1 MiB+64 KiB, payload bytes exact) · MeshSecretMovesWithTheEpoch · PersonSecretSealing · RelaySetting.
**mesh other:** files_test (origin then second holder with live rows flowing; resume plus only-offered-served; a waiter's deadline honoured) · servers_test (fold order, off, revoked, heard-first, hints until row; `Same` ignores `at`; Follow race) · devices_test (net fails → device works, detail shown, retried).
**roomnet (real transport, loopback only):** MeshSecretIsGoRoomsSecretFromKey · TwoNodesSyncOverTheHub (backfill, live rows, revoke closes) · TwoNodesSyncOverTheLANAlone · ServerOffTheOthersStillSyncOnTheLAN · ConnStreamsOverGoRoomOnLoopback · NothingOpensBeforeTheFirstJoin · ICENeverListsPublicSTUN · TwoNodesConnectThroughTheServersTURN (relay-only) · LANErrorWithNowhereToSend · files_test AttachmentsCrossBothWaysDuringABackfill · pair_test: PairCodesAreGoRooms, PairOnTheLAN, PairAsAGuestOfTheHub, PairThroughTheServersBridge · relay_test RelayErrorsDropTheURLsQuery. Port every one of these to iroh on 127.0.0.1 with a local `iroh-relay` (relay-only = IP transports cleared).
**home:** WritesItsAddressOnlyOnChange (and the off row plus mapping removal) · RestartWritesNoNewRow · ServerStatusLines · IPv6OnlyWithOutside · ServerWaitsForAnotherDevice · RevokeDropsTheHubConnectionAndTURN (→ relay Deny) · awake ×3 (held while serving, released when off, a failure keeps serving).
**discovery:** NewToken, round trip, replace leaves no temp, missing, dead pid, corrupt, RemoveOnlyIfOurs, Windows WriteSurvivesBriefReader.
**presence:** events_test ×7 (event path without rescan, overflow → full, `.key` events ignored, dead watcher → interval, unsupported, liveness on safety timer, Run stops all) · exit ×2 · lstart ×2 · reused pid (Windows) · names ×4 · presence_test ×12 (Slug, isSessionFile, fixtures in `testdata/`, partial line, title fallback, oneLine, **never opens key files**, dead pid, tail only on move, sub-second drift, subscribe added/removed, invalid JSON) · v02 ×7 (permission mode, repo key once per cwd, cold-start scan beyond the tail).
**go-room tests** (R:*_test.go, 60 in all) only matter if go-room is ported. Otherwise keep the behavioural ones as iroh tests: forged envelopes, make-before-break replace, stream cap overlap, stream close unblocks a writer, LAN tag windows, replay doesn't move a member, rate limit and junk.

## 7. Hard parts and compatibility traps
1. **No Go↔Rust interop on the wire** (§0). Cut over both devices together. Data carries over; live sync between versions does not.
2. **Side-by-side testing on one PC collides:** UDP 38419, TCP/UDP 25817, `irori.json`, `irori.db` and the identity files. The Rust test build must use its own data dir and ports, or the Go build must be stopped. Never let both write the same data dir.
3. **Never re-serialize row payloads**; carry `p` as bytes. `h` is u64 > 2^53. `p`/`sig` may be `null`.
4. **Tri-state `outside`/`awake`** in `home-server.json`, and `public: null` / `via: ""` in `node.addr`. Go `omitempty` semantics must be mirrored when writing (no `skip_serializing_if` on `public`, `via`, `reach`, `at`, `v`).
5. **`presets::N0` is forbidden**: it adds n0 relays and DNS/pkarr, a third party the design rules out (roomnet.go:5-6, 433-437).
6. **mDNS leaks the stable device id** on every network. go-room rotated LAN tags every 5 min for this reason (R:lan.go:21-26). Use the custom sealed 38419 lookup, which also keeps the Windows Public-profile skip and the `LANError` lines (roomnet.go:223-234).
7. **Pairing guests on the relay** (§5.4) is new design with no direct iroh feature. It is the hardest piece, together with the home/away bridge.
8. **Relay TLS without a domain**: a self-signed certificate pinned through `node.addr` (new field), or plain HTTP if iroh allows it (unverified).
9. **relay.json (advanced TURN) has no iroh mapping**: iroh relays are not TURN. Ignore it with RelayStatus `none` plus a note, or reinterpret it as an extra iroh relay URL. No relay.json exists on the PC.
10. **SAS inputs change** (DTLS fingerprints → EKM), so this is cross-area with pair.
11. **Device key as iroh key**: only if identity can hand over the seed. It also means the TLS handshake key equals the row-signing key (domain-separated by TLS 1.3 contexts, but a deliberate choice).
12. **Epoch semantics** must survive without topics (the epoch-open proof, §5.2), or revoke/rotation tests 9-12 lose their meaning (old-epoch "rooms" exist only to hand over keys).
13. **Crypto crate generations**: `sha2` 0.11 / `hkdf` 0.13 / `hmac` 0.13 are the digest-0.11 family, and `ed25519-dalek` 3.0 exists. Use whatever ed25519 version iroh 1.4 re-exports for `SecretKey` (via `iroh-base`) to avoid two dalek copies.
14. **Board/blob half-close**: Go Streams close both directions. Under QUIC the server must `finish()` its send side; the client must not wait for the server to read EOF.
15. **Status JSON** (api area): `PeerState.rtt` and `skew` are Go `time.Duration`, which serialize as integer **nanoseconds**. `lastPing`/`lastSync` are RFC3339Nano with `omitzero` (node.go:1030-1052).
16. **Port mapping**: iroh's built-in portmapper maps the endpoint's own UDP socket, not port N. `node.addr` needs the external IP, ports and method, so run a separate mapper for N with description `irori` and stale-mapping cleanup.

## 8. Not determined
- Whether the Mac has `relay.json` or `hub-addrs.json` (not inspected). The Mac data dir is `~/.irori` unless `IRORI_HOME` is set (I:internal/paths/paths.go:12-26).
- Whether the sealed `device.key` seed can be given to iroh (identity spec).
- Answered by the critic pass: the trait is `AccessControl` (§5.4); a non-TLS relay is ruled out by QAD (§5.4).

### 8.1 Settled by the M1 spike (`crates/net`, iroh 1.4.1, iroh-relay 1.4.1, rustls 0.23.46 ring, 2026-10-29)

Each answer is backed by a passing loopback test in `crates/net/tests/loopback.rs` (all sockets on 127.0.0.1).

- **Relay TLS (D3): the device-key certificate works, so no `relayCert` field.** The relay serves a self-signed X.509 certificate whose key *is* the device ed25519 key (PKCS#8 v1 of the seed → `rcgen::KeyPair::from_pkcs8_der_and_sign_algo(.., &PKCS_ED25519)`), TLS 1.3 only, through `CertConfig::Manual { server_config }`; `QuicConfig.server_config = None` inherits it, so HTTPS and QAD share it. rustls' ring provider signs and verifies `ED25519` on both ends. Clients use `Builder::ca_tls_config(CaTlsConfig::custom_server_cert_verifier(..))` with a verifier (`irori_net::tls::PinnedDeviceKey`) that accepts the end-entity cert only when its SPKI (`rustls-webpki` `EndEntityCert::subject_public_key_info()`) equals `302a300506032b6570032100 || devicePub` of a pinned server, and delegates the handshake signature check to `rustls::crypto::verify_tls13_signature`. Names and dates are ignored (it is a pin; the URL host is an IP literal). That one `ca_tls_config` covers both the relay HTTPS client and QAD. Tests: `relay_only_echo_through_the_embedded_relay` (right pin connects); `qad_probe_succeeds_through_the_pinned_ca_config` (the net report, read through iroh's test-only `unstable-net-report` feature, shows a QAD IPv4 round trip against the device-key cert and returns the endpoint's own loopback socket as `global_v4`); `relay_tls_refuses_a_server_with_another_key` (a rustls handshake through `PinnedDeviceKey` with the wrong pin fails with `InvalidCertificate(ApplicationVerificationFailure)` and with the right pin completes; in iroh the wrong pin leaves the net report with no HTTPS or QAD latency, so no home relay is chosen and `home_relay_status()` stays `[]` rather than carrying an error; the same relay with the right pin then connects). Consequence: a client pins each home server to the `devicePub` in that server's cert row, the same id it dials, and `node.addr` gains no field.
- **`insert_relay` on an endpoint bound with `RelayMode::Disabled`: does not work.** `Builder::relay_mode(Disabled)` removes the relay *transport* (`iroh-1.4.1/src/endpoint.rs:566-585`); `insert_relay` only edits the map, so `home_relay_status()` stays `[]` and the endpoint is not reachable through the relay (`insert_relay_after_bind`). **Fix: bind with `RelayMode::Custom(RelayMap::empty())` instead of `Disabled`.** That binds (despite the `relay_mode` doc's warning about an empty map), keeps the relay transport, and a later `insert_relay(url, RelayConfig{quic: Some(RelayQuicConfig::new(N))})` connects and is reachable (`custom_empty_map_then_insert_relay`). A Custom map holding a dead placeholder plus an inserted relay also works. `irori_net::endpoint::bind` uses `Custom(empty)` whenever no server is known, so architecture §3.1's lazy bind needs no rebind path. Relays inserted later must already be in the pin set (`ca_tls_config` is fixed at bind; the spike passes `extra_pins`); M5 should make the verifier read a shared, updatable pin set.
- **Path type and RTT fields.** `Connection::paths()` → `PathList` snapshot (`paths_stream()` for changes, `path_events()` for open/close). Per `Path`: `is_selected()`, `is_relay()`, `is_ip()`, `remote_addr() -> &TransportAddr` (`Ip(SocketAddr)` / `Relay(RelayUrl)`), `local_addr()`, `rtt()` (= `stats().rtt`), `stats() -> PathStats`. `PathStats` (noq-proto 1.4.0) has `rtt: Duration`, `udp_tx`/`udp_rx: UdpStats`, `frame_tx`/`frame_rx`, `cwnd`, `bandwidth_estimate: Option<u64>`, `congestion_events`, `sent_packets`, lost counters; it has **no** relay/direct flag, so the kind comes from `Path::is_relay()` on the selected path. `Connection::rtt(PathId)` also exists. `irori_net::path::report` is the `Status` mapping: selected path, `relay`/`direct`, RTT. Observed on loopback: relay-only `path=relay rtt≈2.6ms`; with direct allowed and only the relay URL dialed, `path=relay` first, then `path=direct rtt≈0.6ms via=127.0.0.1:<port>` after about 110 ms, with the relay path kept open beside it (`direct_paths_allowed_reports_the_path_type`).
- **Access control.** `AccessControl::on_connect` returning `Access::Deny { reason }` shows on the client as `RelayStatus::auth_denied_reason() == Some(reason)` and the client never gets a home relay; a dial to a member through that relay then fails. On a direct path, an `EndpointHooks::after_handshake` `Reject` closes the connection before any stream (`closed by peer: refused (code 1)`) (`an_endpoint_off_the_allow_list_is_refused`).
- **Firewall rule for every endpoint.** `Builder::empty()` still pre-binds `0.0.0.0` and `[::]`. Tests and the default CLI call `clear_ip_transports()` then `bind_addr("127.0.0.1:0")` (relay-only: no IP transport at all, which binds fine), `portmapper_config(PortmapperConfig::Disabled)` and `NetReportConfig.captive_portal_check = false`. The relay binds HTTPS, QAD and its plain-HTTP captive-portal port on 127.0.0.1. Builder::empty() needs an explicit `crypto_provider` (ring).
- Still open for the Mac run (docs/m1-mac.md): the PC's subnet relative to the Mac's double NAT, the Keychain prompt for a new binary, and whether hole punching gets `direct` across the double NAT.
