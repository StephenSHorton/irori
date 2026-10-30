# Irori in Rust: architecture and build plan

This is the plan for the Rust rewrite of Irori: Rust, Tauri 2 for the window, and iroh 1.x for networking (1.4.1 was current when the port specs were written: mesh.md §5.1, surface.md §5). The per-area detail is in `docs/port/*.md` (store, identity, mesh, coord, inbox, capture, surface). This file decides how those areas fit together, and settles the points where the specs disagree or leave something open.

Conventions:
- `I:` paths are in the Go repo `../irori-go` (a checkout of the Go Irori repo), and `R:` paths are in the go-room repo.
- `<area>.md §n` means a section of `docs/port/<area>.md`.
- Format claims carry a citation; plan statements do not.
- **Port base:** Go `main` at `be67a86` ("shared boards stage 1"), which is tag `v0.5.0`. Every spec was read against that tree. Section 9.2 covers Go changes made after it.

---

## 0. What has to stay the same, and what may change

The Go code has one clean seam: the `mesh.Transport` / `Room` / `Conn` interfaces (I:internal/mesh/mesh.go:99-138, mesh.md §0). The specs read differently about Go/Rust compatibility because they describe different sides of that seam. store.md §0 says Go and Rust devices "can sync", while mesh.md §0 says they "will not connect". Both are true:

| Layer | Go ↔ Rust | Why |
|---|---|---|
| Data at rest: `irori.db`, `boards/*.db`, `device.json`, `device.key`, `person.json`, the other data-dir JSON files | **Byte-compatible both ways.** Rust opens Go's files in place, and Go can still open them after Rust has written to them. | store.md §2, identity.md §2.1, mesh.md §2.5, surface.md §2.1 |
| Row signatures, `RowHash`, person signatures (Canonical JSON) | **Byte-identical.** | store.md §2.5, identity.md §3 |
| Sync frames (`hello`, `rows`, `vector`, …), `blob/` and `board/` stream bodies | **Byte-compatible framing and JSON.** | mesh.md §2.1-2.3 |
| Transport (go-room WebRTC/DTLS/SCTP, TLS hub, TURN, LAN `GR1` datagrams) | **Replaced by iroh. Not wire-compatible.** | mesh.md §0, §7.1 |
| Pairing SAS inputs (DTLS fingerprints) | **Changed.** A Go device and a Rust device cannot pair with each other. | identity.md §7.3 trap 14, mesh.md §5.2 |
| Agent-facing surface: the `irori` binary and its subcommands, the loopback HTTP API, MCP tools, tool text, attach NDJSON | **Same behaviour, byte-exact text** where agents read it. | surface.md §1, §2.4-2.8 |
| Window | Same React frontend. Only the Wails runtime and bindings change. | surface.md §2.9-2.11, §7.6 |

**Recommendation (decision D1): switch the PC and the Mac together.** A Rust device and a Go device simply don't sync live. History is not at risk: every row already exchanged sits in both logs, and rows written while the two builds differ are exchanged by vector as soon as both run Rust (mesh.md §0). Porting go-room instead (webrtc-rs plus a hub/TURN reimplementation) would be a second network stack that exists only to be thrown away, and it defeats the iroh goal.

---

## 1. Cargo workspace

```
irori-rust/
  Cargo.toml                 workspace; [workspace.package] version, edition 2024; [workspace.dependencies] pins
  rust-toolchain.toml        pinned stable toolchain
  crates/
    gocompat/                Go-semantics primitives every area needs (see 1.2)
    hlc/                     hybrid logical clock
    store/                   signed SQLite log, kinds registry, board-log rules
    ledger/                  PR-authorship view and PR-reference parsing
    identity/                device.json, keys, keystores, Chain, BoardVerifier, board secrets, owner table
    sys/                     OS glue: paths, proc, fswatch, awake, Mark of the Web, console, downloads, login-shell PATH
    blob/                    blob store (patches/summaries) and attachment Files
    scrub/                   secret scrubber
    search/                  shared search types (Source, Ref, Doc, Hit, Page) and the index
    presence/                Claude Code session watcher, AgentCard
    capture/                 transcript tailer, gitlink, repo resolver, gitsha, decisions in capture
    coord/                   the Board fold + approvals (ask), decision, roster, names, devcolor
    inbox/                   messaging, deliveries, wake policy, sweeper
    toast/                   port of go-toast: native Win32 / AppKit card windows
    notify/                  Notifier, Watch, settings.json
    mesh/                    sync protocol above the seam: Node, session, frames, Servers, files, boards, FakeNet
    net/                     below the seam: iroh Endpoint, epoch-open proof, stream labels, LAN lookup, relay client
    home/                    home server: iroh-relay server, port mapping, node.addr, status lines
    lan/                     the sealed LAN lookup on UDP 38419 (an iroh AddressLookup) and this machine's networks
    pair/                    add-a-device flow (and stage 2 invites)
    tools/                   agent tool specs, dispatch, all tool text
    api/                     loopback HTTP server + client, attach protocol, irori.json lockfile, attach runner
    mcp/                     `irori mcp` stdio server (rmcp)
    update/                  updater (GitHub releases, SHA256SUMS, NSIS / .app swap)
    core/                    wires every service for one data dir; diag (irori.log, crash.log)
  app/                       the `irori` binary: CLI roles, Tauri shell, commands, events, tray
    tauri.conf.json, build.rs, nsis/ (installer template), macos/ (Info.plist, package, notarize, bundle check), icons/
  frontend/                  copy of I:frontend at the port base, then edited here (section 4)
  tools/
    xtask/                   local checks, binding export and check, golden import
    goldens/                 Go overlay files that export golden fixtures from the Go tests (section 6.2)
  testdata/
    golden/                  committed Go-produced vectors and fixture dirs
    private/                 git-ignored: copies of real data dirs, never committed
  docs/
```

### 1.1 Go package → crate map

| Go (`I:internal/…` unless noted) | Crate | Spec |
|---|---|---|
| `hlc` | `hlc` | store.md §2.6 |
| `store` | `store` | store.md |
| `ledger` | `ledger` | store.md §8 |
| `identity`, `device`, `owner` | `identity` | identity.md |
| `pair` | `pair` | identity.md §4.7 |
| `mesh` (not `roomnet`) | `mesh` | mesh.md §1-3, §6 |
| `mesh/roomnet` + `R:` (go-room) | `net` (rewritten on iroh, not ported) | mesh.md §5 |
| `home` | `home` | mesh.md §3.3, §5.4 |
| `discovery` | `api::lockfile` | mesh.md §3.5, surface.md §2.2 |
| `presence` | `presence` | mesh.md §3.4 |
| `coord`, `ask`, `decision`, `roster`, `names`, `devcolor` | `coord` | coord.md |
| `inbox` | `inbox` | inbox.md |
| `notify` | `notify` | inbox.md §3.11-3.12 |
| go-toast module v0.2.0 | `toast` | inbox.md §3.13, §4 |
| `awake` | `sys::awake` | inbox.md §4 |
| `capture` (+`transcript`, `gitlink`), `repo`, `gitsha` | `capture` | capture.md |
| `blob` | `blob` | capture.md §2.6 |
| `scrub` | `scrub` | capture.md §7.3 |
| `search` | `search` | capture.md §1 |
| `proc`, `fswatch`, `paths` | `sys` | capture.md §4, surface.md §4 |
| `attach` (+`runner`, root `attach.go`) | `api::attach` + `app` role | surface.md §2.6, capture.md §2.9 |
| `api` | `api` | surface.md §2.4-2.7 |
| `tools` | `tools` | surface.md §2.8 |
| `mcpserver` (+`fallback_tools.json`) | `mcp` | surface.md §2.8 |
| `update` | `update` | surface.md §3 |
| `diag`, `core` | `core` | surface.md §2.3, §1 |
| root package (`main.go`, `*_service.go`, `gui.go`, `tray*.go`, `*_cmd.go`, `console*.go`, `guipath_*.go`, `downloads_*.go`, `servermode_*.go`, `attach.go`, `mcp.go`, `events.go`, `board_route.go`, `notify_gui.go`, `update_card.go`) | `app` | surface.md §1, §2.9-2.10, §4 |
| `mod/` (TypeScript Claude Code mod) | unchanged; it stays a client of `irori attach` and the API | surface.md §7.5 |

Layering: dependencies point down this list, and nothing below `core` depends on `core` or `app`. The order is `gocompat → hlc → store → identity → {blob, scrub, search, sys} → {presence, capture, coord, inbox, mesh} → {net, home, pair, notify, toast} → tools → core → {api, attach, mcp} → update → app`: the Core holds the tools crate's `Set` (Go `c.tools`) but does not serve its own API; the api crate hosts the Core over HTTP (`Core::write_lockfile` once it listens) and serves the Core's tools through `irori_api::CoreTools`. As in Go, coord never depends on inbox; it returns `Notice`s and the caller routes them (coord.md §1.1).

### 1.2 `gocompat`: one home for Go semantics

Several specs each found the same Go behaviours that serde, chrono and std get wrong. Each one is written once, here, and tested against Go-produced vectors:

| Module | Must reproduce | Spec |
|---|---|---|
| `json::valid` | Go `json.Valid`: accepts invalid UTF-8 inside strings, depth up to 10000, non-recursive | store.md §7.4 |
| `json::marshal` | `json.Marshal` / `MarshalIndent`: struct order, `<>&` and U+2028/2029 escaped, bad UTF-8 → U+FFFD, `omitempty` vs `omitzero`, zero time `"0001-01-01T00:00:00Z"`, `[]byte` as base64, trailing `\n` from `Encoder` | store.md §7.2, inbox.md §7.1, surface.md §7.8 |
| `json::canonical` | identity `Canonical`: sorted keys, `<>&` raw, U+2028/2029 always escaped, `\b` `\f` short forms | identity.md §3.2, §3.4 |
| `json::lenient` | `json.Unmarshal` into a struct: case-insensitive keys, last duplicate wins, a type error fails the whole value, bad UTF-8 → U+FFFD | store.md §7.5, capture.md §7.5, identity.md §7.3 trap 7 |
| `json::anymap` | round-trip through `map[string]any`: sorted keys, numbers via f64, `'f'`/`'e'` float rule | surface.md §7.8(h) |
| `time` | RFC3339Nano with trailing fraction zeros trimmed, `Z` for UTC, local offset kept, years 0..9999 | identity.md §3.3, coord.md §7.2 |
| `quote` | `strconv.Quote` / `%q` | surface.md §7.9, coord.md §7.7 |
| `text` | `unicode.IsSpace` / `IsControl` / `IsPrint` / `IsLetter`; one-rune simple lowercase; `SimpleFold` orbits; `EqualFold` | capture.md §7.2, coord.md §7.8 |
| `filepath` | `Clean`, `Rel` (case-insensitive on Windows), `Join`, `Abs` (lexical, no symlinks), `IsAbs`, `VolumeName` | capture.md §7.1, identity.md §7.3 trap 4 |
| `pathmatch` | `path.Match` validity and matching | coord.md §7.9 |
| `url` | the subset of `net/url.Parse` that ParseLink and NormalizeRemote need | coord.md §7.10, capture.md §7.9 |
| `regex` | a constructor that refuses `\w \d \s \b` unless they are ASCII-spelled (`[0-9A-Za-z_]`, `[\t\n\f\r ]`, `(?-u:\b)`) | store.md §7.12, capture.md §7.3 |
| `fnv` | FNV-1a-32 | coord.md §7.16 |
| `flag` | Go `flag` parsing: `-f`, `--f`, `-f=v`, `--f v` | surface.md §7.5 |

### 1.3 Pinned crate generations

All crate versions are pinned once in `[workspace.dependencies]`. These were checked on crates.io for the specs:

- tokio 1.54
- rusqlite 0.41.1 (`bundled`), with a hand-rolled pool (store.md §5)
- serde_json 1.0.154 (`raw_value`, no `preserve_order` by default)
- the digest-0.11 family: sha2 0.11, hmac 0.13, hkdf 0.13
- axum 0.8.9
- reqwest 0.13.5
- rmcp 3.8.0
- tauri 2.12.5
- tauri-specta `=2.0.0-rc.26` with specta `=2.0.0-rc.26` (the Tauri 2 line is pre-release only; stable `tauri-specta` 1.0.2 depends on `tauri ^1.2.4`, crates.io dependency list), or `ts-rs` 12.0.1 as the fallback
- `ed25519-dalek` 3.x (3.0.0 current; `iroh-base` 1.4.1 requires `>=3.0.0-rc.0, <4.0.0`, crates.io dependency list), so only one dalek copy is built (mesh.md §7.13). Verification uses non-strict `VerifyingKey::verify` to match Go: it rejects non-canonical S like Go, and only `verify_strict` adds the small-order R and weak-key checks Go does not make (docs.rs ed25519-dalek 3.0.0 `VerifyingKey`; identity.md §6).
- No `ulid` crate: coord and inbox each hand-roll their Go generator (store.md §1.3, inbox.md §2.3).

---

## 2. Process model and async runtime

**Process model: the same as Go.** One Core per data dir, found through `irori.json` (surface.md §1, §2.2):
- `irori` / `irori gui` hosts the Core in-process with the window and tray.
- `irori serve --headless` hosts it with no UI.
- `irori mcp`, `irori attach`, `irori events` and `irori pair join` are thin clients of the loopback API.

**Runtime: tokio, multi-threaded, one per process.** `main` builds the runtime before anything else. In the GUI role it hands the runtime to Tauri with `tauri::async_runtime::set(handle)`, so the Core, iroh, axum and the window share one runtime and one shutdown path. The client roles (`attach`, `mcp`, `events`) use a current-thread runtime, since they are small, long-lived children per agent session.

Rules:
- Go `context.Context` becomes `tokio_util::sync::CancellationToken`. Each service owns a `JoinSet`. Core shutdown waits 5 s, matching the Go close wait (surface.md §3).
- Fold state (coord, inbox, capture, roster, Chain) lives behind `parking_lot::RwLock`. A lock is never held across an `.await`.
- Lock discipline carries over exactly. Inbox callbacks into coord run outside the inbox lock, and `Options.Stale` runs inside it (inbox.md §7.6).
- Subscriptions keep Go's buffer and overflow rules, not tokio `broadcast`:
  - store: 256 slots, the last one reserved for resync (store.md §3.9);
  - inbox, capture and coord: 64, and a full queue drops the change (inbox.md §1.1, capture.md §1).
  - These use a small per-subscriber bounded queue type in `gocompat::sub`.
- SQLite runs on blocking threads. One write `Connection` sits behind a mutex, beside a small pool of read connections, and every connection is set up with busy_timeout 5000 and synchronous NORMAL. Every write transaction is `BEGIN IMMEDIATE` (store.md §2.2, §7.8).

**Work that must not run as a tokio task:**

| Work | Where it runs | Why |
|---|---|---|
| Windows keep-awake hold | its own `std::thread` per hold | `SetThreadExecutionState` belongs to the calling thread (inbox.md §4, §7.15) |
| Win32 toast windows | one dedicated UI thread with a message loop | go-toast's locked thread (inbox.md §4) |
| AppKit toast panels, the tray, SMAppService | the AppKit main thread Tauri already runs, through `run_on_main_thread` | inbox.md §7.16 |
| FSEvents stream | its own thread with a CFRunLoop | capture.md §4 |
| ReadDirectoryChangesW | a blocking thread per watch, armed before `watch()` returns | capture.md §4, §7.12 |
| Parent and process waits (attach, presence) | blocking threads (`WaitForMultipleObjects`, kqueue) | surface.md §4 |
| The capture loop | one task owns all tail and reflog state; readers use the RwLock'd fold | capture.md §7.13 |

---

## 3. Networking on iroh

### 3.1 Pieces

```
                        PC (home server, always on)
 ┌────────────────────────────────────────────────────────────────────┐
 │ iroh Endpoint  (SecretKey = device ed25519 seed → EndpointId = devicePub)
 │   ALPNs: irori/mesh/1, irori/pair/1  (stage 2: irori/board/1, gossip)
 │ iroh-relay server  TCP N = 25817 (relay over HTTPS), UDP N (QUIC address discovery)
 │ port mapper for N (UPnP / NAT-PMP / PCP), writes node.addr
 │ LAN lookup  UDP 38419  (sealed IR1 datagrams)
 └────────────────────────────────────────────────────────────────────┘
              ▲ relay URL + pinned cert from the PC's node.addr row
              │ (LAN IP at home, public IP:N away)
 ┌────────────┴───────────────────────────────────────────────────────┐
 │ Mac: behind two routers (double NAT)
 │ iroh Endpoint (device key), RelayMode::Custom([PC relay]), LAN lookup 38419
 └────────────────────────────────────────────────────────────────────┘
```

- **Endpoint.** Built from `Builder::empty()` and **never** from `presets::N0`, since that would bring n0's public relays and DNS/pkarr (mesh.md §5.1, §7.5). The endpoint is bound lazily on the first "join" (`TestNothingOpensBeforeTheFirstJoin`, mesh.md §3.2). It uses:
  - `relay_mode(RelayMode::Custom(<home servers>))`, or `Disabled` while no server is known;
  - `address_lookup(MemoryLookup + LanLookup)`;
  - `hooks(...)` running `CheckDevice` in `after_handshake`, so a peer is refused before any frame (mesh.md §5.2).
- **Identity (D2).** The iroh `SecretKey` is the device ed25519 seed from `device.key`, so `EndpointId` equals the certified `devicePub`. Every device the person already paired is then known by its iroh id from the cert rows in the log. No re-pairing and no new identity file are needed. The cost is that one key both signs rows and authenticates TLS, with TLS 1.3 contexts as the domain separation (mesh.md §7.11).
- **Rooms become epochs.** go-room put each connection in a topic per person-key epoch. iroh has no topics, so the first message on the `sync` stream is a new epoch-open proof, `HMAC-SHA256(MeshSecret(E).key, "irori/mesh-open/1" || EKM32)`. Then the unchanged `hello` and frames follow (mesh.md §5.2). `MeshSecret` derivation stays exactly as in Go (mesh.md §2.4), and the revoke and rotation tests keep their meaning (mesh.md §7.12).
- **Streams.** Each bi stream starts with `u32 BE len || label`:
  - `sync`: `u32 BE len || frame`, capped at 8 MiB;
  - `blob/<16hex>` and `board/<id>`: the Go request and reply bodies, unchanged; the server half-closes with `finish()` (mesh.md §2.2-2.3, §5.2, §7.14).
- **Who dials.** The lower device id dials. A duplicate connection is closed `REPLACED`, and close reasons keep Go's strings (mesh.md §5.2).
- **Home server (`home`).** iroh-relay `Server` with:
  - relay over HTTPS on TCP N and QUIC address discovery on UDP N, so "forward TCP and UDP port N" stays true (mesh.md §5.4). In iroh-relay 1.4.1 the QUIC server fails to spawn without TLS 1.3 (`QuicConfig.server_config`, else `RelayConfig.tls`), and with TLS set the relay is served on `TlsConfig.https_bind_addr`, not `RelayConfig.http_bind_addr` (docs.rs iroh-relay 1.4.1 `RelayConfig`, `TlsConfig`, `QuicConfig`). So TCP N is `https_bind_addr`, and the required plain `http_bind_addr` goes to a loopback port nobody forwards; relay URLs are `https://host:N`;
  - port N kept from `home-server.json` (`25817` on the PC, mesh.md §2.5; random in 20000-29999 on first start, I:internal/home/home.go:51-54);
  - per-EndpointId access through `RelayConfig.access: Arc<dyn DynAccessControl>`, implemented with the `iroh_relay::server::AccessControl` trait: `on_connect(&ClientRequest) -> Access` (id from `ClientRequest::endpoint_id()`), `on_disconnect(EndpointId, ConnectionId)`, `Access::{Allow, Deny { reason: Option<String> }}` (docs.rs iroh-relay 1.4.1). Certified, unrevoked devices; guests only during a pairing window; revocation means `Deny` plus dropping live clients (mesh.md §5.4).
  - Port mapping runs separately for N, because iroh's own portmapper maps only the endpoint's socket (mesh.md §7.16).
  - `node.addr` rows, status lines and the awake hold port unchanged (mesh.md §3.3).
- **Relay TLS (D3).** A home PC has no domain, so the relay uses a self-signed certificate and clients pin it. Preferred: the certificate's key is the PC's device key, and clients accept the server when its SPKI equals that server's certified `devicePub`, so `node.addr` needs no new field. Both sides do take custom TLS in 1.4.1: the server through `CertConfig::Manual { server_config: rustls::ServerConfig }`, the client through `Builder::ca_tls_config(CaTlsConfig::custom_server_cert_verifier(..))` or `CaTlsConfig::custom_roots(..)`, which docs.rs says applies to relay connections (docs.rs iroh 1.4.1 `tls::CaTlsConfig`, iroh-relay 1.4.1 `CertConfig`). What M1 still has to prove is an Ed25519 device-key certificate through rustls on both ends. Fallback: a separate cert whose SHA-256 goes into a new `node.addr` field `relayCert` (mesh.md §5.4). A plain `http://` relay is not an option, because QUIC address discovery needs TLS.
- **Finding peers.**
  - The `Servers` fold of `node.addr` rows gives each home server's relay URL and direct addresses, in Go's `Addrs` order (mesh.md §3.1 "Servers.Addrs order"). This is what lets the Mac find the PC with no typed address.
  - The LAN lookup on UDP 38419 (`LANPort`, I:internal/mesh/servers.go:21) keeps go-room's privacy design (D6): `"IR1" || tag8 || nonce24 || secretbox(...)`, with a rotating tag keyed by the mesh secret, the Windows Public-profile skip and a 200/s rate limit (mesh.md §5.3). mDNS is not used, because `iroh-mdns-address-lookup` broadcasts the stable device id on every network (mesh.md §7.6).
  - `hub-addrs.json` `last`/`hint` become direct-address hints (mesh.md §2.5).
- **Double NAT.** The Mac sits behind a second router (a double-NAT home network). Away from home, and at home whenever the two are on different subnets, the relay path is the baseline; a direct path after hole punching is a bonus that `Status` reports as `direct`. A LAN broadcast does not cross the inner NAT, so at home the Mac reaches the PC through the `lan` addresses in `node.addr`. Which subnet the PC sits on was not determined; M1 records it.
- **Dropped.**
  - TURN, `turn.key` (left on disk and still readable, identity.md §7.3 trap 15), the TLS hub, SplitTCP.
  - `relay.json`, the advanced TURN setting (D4). It has no iroh meaning (mesh.md §7.9) and does not exist on the PC. The setting reports `none` with a note.
  - `iroh-blobs`: it addresses by BLAKE3 while attachments use sha256 (mesh.md §5.1).

### 3.2 Side by side with Go on one machine

A Go build and a Rust build on one PC would both want:
- UDP 38419 and TCP/UDP 25817;
- `irori.json`;
- `irori.db` and the identity files (mesh.md §7.2).

Development builds therefore always run with `IRORI_HOME=<copy>` and a port offset (`IRORI_DEV_PORT_OFFSET`, which also moves the 38419 lookup). Both are compiled into dev builds only. Two builds never write one data dir.

### 3.3 Stage 2: shared boards with other people, straight on iroh

Stage 1 (Go `be67a86`) moves a board's rows on `board/<boardId>` streams inside the private mesh connection. That shortcut holds only while every member is you, and stage 2 must remove it (I:docs/design/v0.5-shared-boards.md:428-429). The Rust port carries stage 1 as is, then builds stage 2 (build steps 5-11 at I:docs/design/v0.5-shared-boards.md:438-444) this way:

- **One board connection, separate from the private mesh.**
  - ALPN `irori/board/1`. Its first message is a board-open proof: `HMAC(DeriveBoardTopic(boardId, epoch, secret).key, "irori/board-open/1" || EKM32)`, using the board key derivation as it is (identity.md §4.6).
  - It serves only `board/<id>` streams for that board, and only to counted member devices (`BoardVerifier.Cert`, identity.md §4.5).
  - Another person's device never reaches `irori/mesh/1`, which still admits only your own certified devices.
- **Per-board gossip topics.** `iroh-gossip` (0.101.0, mesh.md §5.1) with `TopicId = sha256(board topic string)`. The topic string derives from the board secret, so only members can compute it.
  - Gossip carries announcements only: `{boardId, epoch, vector}`. Rows still move on the gated `board/` pull streams. Gossip content is visible to every topic member (mesh.md §5.3), and pull streams keep the per-board admission check in one place.
  - Gossip connections are gated in `after_handshake` to devices counted on at least one board this device holds.
- **Server admission (decision 35 B1).** The `Access` rule widens to the counted member devices of boards this server holds, for those boards' connections only.
- **Invites as tickets.** The v0.5 invite link `irori-invite:<code>@<server host:port>#<person fingerprint>` (I:docs/design/v0.5-shared-boards.md:130) becomes `irori-invite:<base32 ticket>`. The ticket holds the one-use code, the inviter's `EndpointAddr` (id, home relay URL and its pin, direct addresses) and the person fingerprint.
  - The invitee dials ALPN `irori/invite/1`. The inviter's relay admits the invitee as a guest only while the invite window is open: at most 1 hour, and at most 2 invite windows per member (I:docs/design/v0.5-shared-boards.md:130).
  - The five words come from the code-derived key plus EKM, as pairing does (section 3.4). The voice-call confirmation from decision 35 B4 stays.
- **Relay guests are the hard piece** for both pairing and invites. iroh has no built-in guest admission, and the home/away bridge needs a code-keyed LAN announcement (mesh.md §5.4, §7.7).

### 3.4 Pairing on iroh (D5)

`Conn.Fingerprints()` returned DTLS certificate fingerprints, which the SAS words cover (identity.md §4.7 "Info", trap 14). In Rust both sides use `fp = colon-hex(export_keying_material(32, "EXPORTER-irori-sas", ""))`, which binds the words to the QUIC/TLS session as before (mesh.md §5.2). Everything else in pairing ports exactly: code alphabet, scrypt room secret, HKDF SAS, HMAC proofs, word lists and states (identity.md §4.7). The ROOM, INFO, FP and SEED vectors still apply; only the fingerprint inputs change. The PC and the Mac are already paired, so pairing is not needed for the cutover itself. It still ships before the cutover, because Add device and Reset device are in the window.

---

## 4. The Tauri 2 shell

### 4.1 One binary, Go's command line

- `app` builds `irori` / `irori.exe` (`mainBinaryName: "irori"`) with `#![windows_subsystem = "windows"]`.
- `main` parses the role **before** Tauri starts, using `gocompat::flag` so that `-flag` works as it does in Go. The roles are `gui`, `serve --headless`, `mcp`, `attach`, `events`, `pair`, `node`, `version`. Unknown commands and errors keep Go's texts and exit codes (surface.md §1, §7.5).
- CLI roles other than `gui` and `mcp` attach the parent console only when the std handles are unusable; `mcp` never attaches a console (surface.md §4, §7.15).
- `irori version` prints `irori <version>`. The version comes from `IRORI_VERSION` at build time, else `dev`, and `dev` never offers updates (surface.md §7.20).
- Panics go to `crash.log` through a panic hook, because Rust has no runtime crash file (surface.md §7.16).

### 4.2 Window, tray, single instance, login

| Item | Value (from Go) | Rust |
|---|---|---|
| Window | title `Irori`, 1100×720, background RGB(233,238,248), close hides (I:gui.go:136-148) | `tauri.conf.json` + a close-requested handler |
| Single instance | id `com.stephenshorton.irori`, plus `.` + hex(sha256(lower(Clean(IRORI_HOME)))[0:6]) when set (I:gui.go:28-40) | `tauri-plugin-single-instance` 2.5.5 takes no id: it reads `app.config().identifier` (Windows window class `{id}-sic`, macOS `/tmp/{id}_si.sock`; plugin src/platform_impl/windows.rs:81-88, macos.rs:63-73). So `main` sets `context.config_mut().identifier` to the Go id before the plugin is built. That identifier also names Tauri's WebView2 data folder, so a test copy gets its own webview profile (surface.md §7.17). The lockfile check stays the real guard against a Go GUI (surface.md §7.18) |
| Tray | menu rebuilt whole, exact lines (surface.md §3 "Tray") | Tauri `tray-icon` |
| Launch at login | Windows HKCU `Run` value `Irori` = `<exe> --startup` (wails v3.0.0-alpha.98 `autostart_windows.go:43-63`); macOS `SMAppService.mainAppService` (`autostart_darwin.go:26-45`; surface.md §4, §7.4) | own code: `winreg` + `objc2-service-management` (D9), so the Wails registration is seen and removable |
| Open URL / clipboard | `Browser.OpenURL`, `Clipboard.SetText` | `tauri-plugin-opener`, `tauri-plugin-clipboard-manager` |
| Attachments | `/irori/attachment/<sha>` with Range, ETag, CSP sandbox, nosniff (surface.md §2.11) | `register_asynchronous_uri_scheme_protocol("irori")`; `AttachmentInfo.url` becomes absolute per platform and is allowed in CSP `connect-src img-src media-src` (surface.md §7.7) |
| Cards (toasts) | go-toast custom windows, not OS notifications (inbox.md §4) | `toast` crate, a native port (D7) |

### 4.3 Moving the frontend

The frontend is copied from `I:frontend` at the port base into `irori-rust/frontend`. Only the files that touch Wails change:
- `src/lib/*.ts` and `src/updater` import the bindings (surface.md §2.9);
- `@wailsio/runtime` is imported in `irori.ts`, `cards.ts`, `boards.ts`, `devices.ts`, `settings.ts` and `clipboard.ts`.

Components and the sample mode stay as they are. The move has two steps, so the window works from M1:

1. **A compatibility shim (M1).** A Vite alias points `@wailsio/runtime` at `frontend/src/host/wails-compat.ts`:
   - `Call.ByID(id, ...args)` looks up a table that maps each binding id to a Tauri command name and argument names. The table is generated by `xtask` from the 49 committed binding files.
   - `Create.*` keeps the `$$createType` converters.
   - `Events.On(name, cb)` wraps `listen(name, e => cb({data: e.payload}))`.
   - `Browser` and `Clipboard` forward to the plugins.
   - `inWails()` becomes `inHost()`, which checks `window.__TAURI_INTERNALS__`. Otherwise WebView2 still reports true and every call fails (surface.md §7.6).
   - Rejections become `Error`s, since Tauri rejects with the serialized error string while `I:frontend/src/lib/irori.ts:52-55` reads `err.message`.

   The 49 generated files and all of `src/` then run unchanged.
2. **Generated bindings (M6), via tauri-specta.** Each Go service becomes a Rust module of `#[tauri::command] #[specta::specta]` functions named `<service>_<method>`. Events are typed with tauri-specta's event derive. `xtask bindings` writes `frontend/src/gen/bindings.ts`.
   - The `src/lib/*` gateway files switch from `bindings/…` to `src/gen`. They map positional to named arguments and keep the error and `{data}` conventions, so components never see the change. Then `frontend/bindings/` and the shim are deleted.
   - The `$$createType` converters were load-bearing only because Go emits `null` for nil slices and maps (I:frontend/bindings/github.com/StephenSHorton/irori/models.ts:112-142, surface.md §7.6). Window-facing Rust types use `Vec`/`BTreeMap`, never `Option<Vec>`, and always serialize their fields, so the converters are no longer needed.
   - Where a model class is constructed directly, the code switches to object literals: about 20 sites, mostly in `sample-*.ts` and tests, plus `TaskState.*` enum constants and two `createFrom` calls.
   - The tauri-specta line must target Tauri 2. Stable `tauri-specta` 1.0.2 is Tauri 1 only; the Tauri 2 line is `2.0.0-rc.26`, pinned with `=` together with `specta =2.0.0-rc.26` (section 1.3). Adopting a release candidate is the cost; `ts-rs` 12 is the fallback.

Window JSON does not have to match Go byte for byte; only the API and MCP shapes do. Window commands reuse the area types wherever their JSON already suits the frontend.

Event payload shapes and coalescing stay the same: `roster:changed`, `inbox:changed` (250 ms), `board:changed` (250 ms), `boards:changed`, `capture:changed` (300 ms), `devices:pairing`, `settings:launch-at-login`, `app:open`, `update:status` (surface.md §2.10).

---

## 5. Compatibility plan

### 5.1 Open everything in place

The data dir is `$IRORI_HOME`, else `%LOCALAPPDATA%\irori`, else `~/.irori` (I:internal/paths/paths.go:12-26). On the PC it holds `device.json`, `device.key`, `person.json`, `turn.key` and `irori.db` (identity.md §2.1).

| File | Rust rule | Ref |
|---|---|---|
| `irori.db` | Opened in place at **schema v3**. No migration is added, and none of the existing ones is re-run. WAL mode, `BEGIN IMMEDIATE`. Payload bytes are never re-encoded. An absent `sig` is SQL NULL. | store.md §2.2-2.4, §7.1, §7.3, §7.9 |
| `boards/<id>.db`, `.secrets` | Same store code with `board: true`; secrets are wrapped like keys. | store.md §2.1, identity.md §2.1 |
| `device.json` | 2-space MarshalIndent, no trailing newline; `os` uses Go names (`darwin`). | I:internal/device/device.go:35-72 |
| `device.key`, `person.json`, `turn.key` | Wrapped `{v, keystore, blob}` (I:internal/identity/keystore.go:35-39). DPAPI entropy is exactly `irori/key/1` (I:internal/identity/keystore_windows.go:13-49). The Keychain KEK is read through `/usr/bin/security`, with account `kek-` + hex(sha256(Go-cleaned abs path)[0:8]) (I:internal/identity/keychain.go:60-75). | identity.md §2.1, §5, §7.3 traps 4-5 |
| `mesh-peers.json`, `hub-addrs.json`, `home-server.json` | MarshalIndent, BTreeMap key order, tri-state `outside`/`awake` as `Option<bool>` (I:internal/home/home.go:42,79-82). | mesh.md §2.5, §7.4 |
| `settings.json` | Unknown keys survive; sorted keys, 2-space indent, trailing newline; temp file then rename. | inbox.md §7.14 |
| `capture-cursors.json` | Keys spelled exactly as Go writes them (native separators, Windows reflog keys lowercased). | capture.md §0 I3 |
| `names.json`, `pair-rows.json`, `blobs/`, `files/` | As written by Go. | surface.md §2.1, identity.md §2.1, capture.md §0 I4 |
| `irori.json` | Temp file, fsync, rename with retry; 2-space indent, no trailing newline. | I:internal/discovery/lockfile.go:28-75 |

Rust-only state stays out of `irori.db` until after the cutover. If any is needed, it goes in a separate file. Adding schema v4 makes Go refuse the database (store.md §7.9), so v4 is the point of no return and may only come after the cutover (D8).

### 5.2 Rehearse on copies, then switch

1. **Copies.** All development and testing reads copies of the real data dirs under `testdata/private/` or another `IRORI_HOME`, made while Irori is stopped. DPAPI works on a copy for the same Windows user.
   - The macOS Keychain account is a hash of the data dir path, so a copied dir on the Mac fails with `ErrKEKMissing` by design (identity.md §7.2 `keychain_test` "moved dir errors").
   - Dev builds therefore accept `IRORI_DEV_KEK_DIR=<original dir>`, which names the account from the original path. Only debug builds (`debug_assertions` on) read it; a release build ignores it (`identity::dev_kek_dir`).
2. **Rollback stays open.** Rust writes only schema v3 and Go-shaped files, so reinstalling the Go build continues from the same data. The only addition is the optional `relayCert` field, and payloads only gain fields. Back up both data dirs before installing.
3. **Mixed period.** If the Mac must stay on Go for a while, nothing extra is needed: the Rust PC already holds every Go row up to the switch, and the two catch up by vector once both run Rust. An offline row bridge (`irori rows export|import`, using the signed `pair-rows.json` row format, identity.md §2.1) is cheap because `Insert` verifies every row. It is built only if Stephen asks for it.
4. **Cutover (M8).**
   - Install the Rust build on the PC, then on the Mac.
   - Check sync, then commit the Rust tree over `~/projects/irori` and release from there.
   - Launch at login and the installed exe path carry over (section 7).

---

## 6. Testing

### 6.1 Port the Go tests, area by area

Every spec lists its Go tests and what each proves: store.md §6, identity.md §7.2, mesh.md §6, coord.md §6, inbox.md §6, capture.md §6, surface.md §6. Each crate ports them under the same names in snake_case. Table-driven Go tests are copied verbatim, including ParseLink, Overlaps, ParseResource, NormalizeRemote, scrub regressions and the ledger tables. The kinds test is replaced by the macro-generated registry check in store.md §6 "kinds".

### 6.2 Goldens exported from the Go build, without touching the Go repo

`internal/` packages cannot be imported from another module. Instead, `tools/goldens` holds `*_golden_test.go` files that are injected with `go test -overlay overlay.json` into the Go packages at `I:`. Go test files can then reuse each package's own harness, while the Go repo on disk stays unchanged.

Each exporter writes into `testdata/golden/`:
- identity vectors: Canonical, times, escapes, room, SAS, board topic, seed (identity.md §7.1);
- `MeshSecret` and person-secret sealing (mesh.md §2.4);
- tool text: `(tool, args, caller) → text` over the existing test fixture Cores (surface.md §7.10);
- notice, show and digest texts (coord.md §3.18);
- wake prompts and `wake.issued` row fields (inbox.md §6 `TestWakeIssuedRowFields`);
- row dumps (raw payload bytes) from each coord, inbox and capture test, which Rust replays to compare the resulting `Task`/`Delivery`/timeline JSON (coord.md §6, inbox.md §6 replay tests);
- `fallback_tools.json`, which is embedded in Rust and asserted equal to `GET /v1/tools?client=grok` (surface.md §2.8);
- a fixture data dir written by Go with the `file` keystore.

### 6.3 Differential tests on real data (local only)

- **Rows:** open a copy of the PC's and the Mac's `irori.db` with Rust and verify every row signature and `RowHash`. The `Chain` device list must equal Go's `/v1/devices`.
- **Folds:** start the Rust Core read-only on a copy and compare `/v1/board`, `/v1/inbox`, `/v1/timeline`, `/v1/events` and `/v1/devices` with the Go Core on an identical copy. JSON is canonicalized first, because Go escapes `<>&` (surface.md §7.8).
- **Live keystore checks:** unseal `device.key` and `person.json` on the PC (DPAPI) and on the Mac (Keychain) and compare public keys, printing nothing (identity.md §7.1).
- Private data stays in git-ignored `testdata/private/`.

### 6.4 Agent acceptance

The mod's suites (`mod/unit/lib.spec.ts`, `mod/tests/irori.test.tsx`) are hermetic: the test kit has no process or fetch, and the suites answer `process.spawn` and `http.fetch` themselves, so they cannot be pointed at the Rust `irori attach` or API. They pin the mod's half of the contract; the Rust half is the real `irori attach` exe spawned with the mod's argv against the real API server (`app/tests/attach_exe.rs`), the ported attach tests and the attach goldens, all through `tools/mod-suite.sh`. Only a live session through a copy of the mod runs the two halves together (§8 M4). MCP is checked with an rmcp client: tool list, `irori_` names, channel push and the not-running fallback (surface.md §6 "mcpserver").

### 6.5 Network tests: loopback only

- Every test socket binds `127.0.0.1`, or Windows Firewall raises an Allow dialog for every test exe (mesh.md §4 "Tests").
- iroh test endpoints use `RelayMode::Disabled`, or a local `iroh-relay` on 127.0.0.1 for the relay-only cases (IP transports cleared). They use no mDNS, and the LAN lookup sends unicast to 127.0.0.1 instead of broadcasting.
- A shared `testnet` helper builds these, and it panics if asked for a non-loopback address.
- The mesh suite runs twice: once on the in-memory `FakeNet` (mesh.md §6) and once on real iroh over loopback (the roomnet list in mesh.md §6, ported).

---

## 7. Build and release

### 7.1 Local checks (no CI checks)

Run from the repo root before every merge or push. On the Mac, run them too when a change touches macOS code. Cross-checking macOS from Windows is not possible, because the bundled SQLite C build needs the macOS toolchain.

```sh
(cd frontend && bun install --frozen-lockfile && bun run build && bun run typecheck && bun test)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p irori-capture -- --test-threads=1      # timing-sensitive, as in Go
cargo xtask bindings --check                         # generated TS matches the Rust commands
cargo xtask no-third-party                           # fails on presets::N0, RelayMode::Default, mdns or pkarr crates in the tree
```

As built (`scripts/check.sh`): the frontend steps, fmt, clippy, the workspace tests and the single-threaded capture run as above; `no-third-party` is `scripts/no-third-party.sh` (there is no xtask crate yet); `bindings --check` waits for the M6 bindings, and until then the shim's table is checked by `gen-compat-table.ts --check`.

### 7.2 Release workflow

There is no CI (no workflow builds or checks anything): `release.yml` is replaced by local scripts, `scripts/release.ps1` on the PC and `scripts/release-mac.sh` on the Mac, each with a dry run (the default) and `--publish` to a draft release that is published once both platforms' assets and `SHA256SUMS` are up (docs/releasing.md). `pages.yml` (deploys `site/` to GitHub Pages on pushes to `main` that touch `site/**`, I:.github/workflows/pages.yml:1-9) carries over unchanged.

**Windows:**
- The installer is `app/nsis/irori.nsi`, a port of `I:build/windows/nsis/project.nsi` + `wails_tools.nsh`, built with `makensis` directly as Go's was (not Tauri's bundler and its Handlebars template, which differs in all of the points below and could not be run here). It must keep everything the Go updater and the existing install rely on (surface.md §7.1-7.2):
  - per-user `%LOCALAPPDATA%\Programs\Irori\irori.exe` (I:build/windows/nsis/project.nsi:91-92);
  - uninstall key `Stephen HortonIrori` (I:build/windows/nsis/wails_tools.nsh:25-28);
  - `/S`, `/NORELAUNCH`, `/WAITPID=<pid>` with a 60 s wait (I:build/windows/nsis/project.nsi:103-246);
  - rename the old exe aside to `irori.exe.old[N]`, exit code 3 on give-up;
  - Start Menu and Desktop shortcuts;
  - `Run\Irori` removal on uninstall.
- Assets: `irori-<v>-windows-amd64-installer.exe`, `irori-<v>-windows-amd64.zip`, `SHA256SUMS` (I:.github/workflows/release.yml:121-130,239-240).

**macOS:**
- `CFBundleIdentifier com.stephenshorton.irori`, `CFBundleShortVersionString` = bare `X.Y.Z`, executable `irori`, `LSMinimumSystemVersion` 12.0.0.
- Developer ID signing and notarization (ports of `I:build/darwin/package.sh` and `notarize.sh`, not yet read).
- `codesign --verify --deep --strict` must pass, because the Go updater checks all of this on the zip (surface.md §7.3).
- Assets: `irori-<v>-macos-arm64.app.zip`, `.dmg`.

**Smoke and tests:**
- The workflow runs `irori version` against the exact text (surface.md §1).
- A Rust test asserts that the asset names in `release.yml` match the updater's matchers, as `workflow_test.go` does (surface.md §6 "update").

### 7.3 Updater

The Go updater design ports as is: GitHub `releases/latest`, `SHA256SUMS`, NSIS silent install or `.app` swap, and the phases and card (surface.md §3 "Updater rules"). `tauri-plugin-updater` is not used, because it expects its own signed manifest and the first Rust release must install through the **Go** updater. That first release needs a version above the Go `v0.5.x` line under Go's `isNewer` (surface.md §3), for example `v0.6.0` (D11).

---

## 8. Milestones

Each milestone ends in something runnable with a check that passes or fails. Until M8, all of them run on copies or with dev ports while the Go build keeps running.

**M1 Spike: window plus a relayed PC↔Mac connection with no typed address.**
- Build:
  - the workspace skeleton and the `app` crate with the Tauri shell;
  - the copied frontend through the compatibility shim, with `roster_version` and `roster_device` (from `device.json`) as real commands and everything else answered from the sample data;
  - a read-only identity loader that unseals `device.key` and creates and rewrites nothing;
  - a `spike net` role. On the PC it runs iroh-relay and an Endpoint keyed by the device seed on dev ports. On the Mac it reads the PC's newest `node.addr` (`lan`/`public`) and the PC's `devicePub` from person rows in a copy of the Mac's `irori.db`, sets the relay to that host on the dev port, and dials the PC's EndpointId, first relay-only and then with direct paths allowed.
- Check:
  - the window shows the current frontend on Windows and macOS;
  - the Mac reaches the PC through the PC's relay without any address or id typed in;
  - `Status` shows `relay`, then `direct` if hole punching works, with RTT.
- Settles:
  - TLS on the relay (device-key cert pin or `relayCert`);
  - whether `http://` relay URLs are accepted;
  - the iroh-relay access-control trait name;
  - whether the Keychain prompts a new binary;
  - the PC's subnet relative to the Mac's double NAT.

**M2 Log and identity on real data.**
- Build: `gocompat` (json, time, quote), `hlc`, `store`, `identity` (keystores, Chain, BoardVerifier, board secrets, owner), `ledger`, plus the identity goldens.
- Check:
  - `irori-dev verify --home <copy>` checks every row signature and hash in the PC and Mac logs;
  - the Chain device list equals Go's `/v1/devices`;
  - all store, identity and ledger tests pass;
  - after a Rust append on a copy, the Go build still opens that copy.
- Results so far:
  - 2026-10-29, PC (`sh tools/goldens/verify-live.sh` on a copy of `%LOCALAPPDATA%\irori`, Go at be67a86): passed. 12389 `events` rows with every signature ok, 18 `local_events`, 0 `refused_events`, 0 forks; all 12407 RowHashes equal Go's; Chain 2 certified devices, 0 bad identity rows; the Rust `device.started` append verified in the Go Core (`TestGoOpensRustCopy`).
  - 2026-10-30, PC, `IRORI_VERIFY_DEVICES=1` on a fresh copy: passed. 13548 signatures ok, 0 RowHash diffs, the Chain equals Go's `/v1/devices` (the PC and the Mac), and `TestGoOpensRustCopy` passed (check 2).
  - Not yet run: the whole Mac half (its log and Keychain, on the Mac).

**M3 Folds replay-equal.**
- Build: `coord`, `inbox`, `capture` (replay and tailing), `presence`, `roster`, `blob`, `scrub`, `search`, `sys`.
- Check:
  - row-dump goldens replay to the same JSON as Go;
  - a headless read-only Rust Core on a copy matches the Go Core's board, inbox, timeline and decisions views (6.3).
- Results so far:
  - 2026-10-29, PC, first run (Go at be67a86): all 134 views equal, with every same-`at` run of a timeline sorted whatever its kind. That normalization was wider than Go's nondeterminism (a Rust mis-ordering of turns, summaries or PRs would have passed), so that run does not show the timelines match; the rerun below replaces it.
  - 2026-10-29, PC, rerun after the M3 review (`sh tools/goldens/replay-live.sh` on fresh copies of `%LOCALAPPDATA%\Irori`, Go at be67a86; `irori-dev replay` against `TestGoReplayViews`, tools/goldens/core_replay_test.go): all 139 views equal. 12574 rows over 2 devices, Rust appended nothing. Board for the person, for nobody and for 5 agent viewers (the agents that hold or made a card; 41 cards, doneTotal 38) and leases byte-equal; inbox list (270 deliveries), held history (16) and policy byte-equal, the open list (7) equal once sorted by id; decisions byte-equal; 126 timeline files (63 agents, with and without subagents), 113 byte-equal and 13 equal once only commit and open-turn entries are reordered within same-`at` runs. The Go Core appended only its `device.started` row on its own copy; `--compare` now fails when it appends any other kind, since its views are taken while its loops run.
  - Normalized, because Go leaves the order to map iteration or an unstable sort: the inbox open list (by id; Go ranges over a map), and in timelines, within a run of equal `at`, the commit entries among their own slots (Go ranges over its commit map) and open-turn entries among theirs (Go's unstable `sort.Slice`; none in a headless replay). Turns, summaries and PRs keep their order, as Go's stable sort fixes it. The Board clock is pinned to the copy time on both sides. Views are the services' values; the API's nil-to-`[]` wrappers come with M4.
  - Empty on this log, so not checked by it: decisions (the PC log has none, so the decision fold and `decision::check` never ran on real rows) and leases (0, so no lease path ran). Those need a synthetic copy or the Mac half.
  - Not yet run: the Mac half.

**M4 Agents on Rust (one machine, no network).**
- Build:
  - `api`, `tools`, `mcp`, the attach stream and runner, `events`, the lockfile, `core` wiring, notices, sweeps;
  - the timers M3 left as plain calls: coord's expiry timer (Go's `armExpiry`, over `Coord::next_expiry`/`expire`), `handle`/`tick` and the sweeps;
  - capture's FTS5 search index and its Search, Show, Overview, Why and RecordDecision (capture.md §1, §2.7), which dispatch to the coord and inbox `search::Source`s M3 built; the real decision fold replaces irori-dev's stand-in.
- Check:
  - capture.md §6's search, show and overview tests pass: FTS5Available, MatchExpr, MergeOwner, AddSourcePanics, SearchPullPaging, SearchCaptureKinds, SearchRankingAndFilters, SearchLimits, SearchSnippet, SearchPullsSources, SearchOffsetPaging, SearchViewer, HideMessages, ShowPaging, ShowRelatedCap, ShowOpenTurn, OverviewCapsAndRepoFallback;
  - coord.md §6 "expiry timer fires without a row" passes as a behaviour test;
  - the capture tests that M3 only replays (the rig-only list in crates/capture/tests/capture_service.rs) pass end to end through the Rust tailer and repo pass;
  - the mod's two suites pass on their side of the contract and the Rust side passes against the real API server (§6.4: the mod's suites are hermetic and cannot run against Rust);
  - the tool-text goldens match;
  - a real Claude Code session on the PC with the mod's `iroriPath` set to the dev exe and a test `IRORI_HOME` gets wakes, sends messages and uses the Board.

- Results so far (api):
  - 2026-10-29, PC: `crates/api` serves every `/v1` route Go serves, in Go's check order, with Go's JSON bytes, texts and statuses, the attach and events streams, and a small client; `irori serve --headless` hosts it. Golden `testdata/golden/api` (scenario.json run by `tools/goldens/api_golden_test.go` on a Go Core with its own api.Server, Go at be67a86): 155 steps, every one the Rust replay runs byte-equal once ids, times and HLCs are masked (`crates/api/tests/api_golden.rs`), the 5 tools steps included since the merge wired the tools crate in. 1 step waits on a service M4 does not have: `/v1/devices` on the mesh (M5); health's `net` is dropped from the compare for the same reason.
- Results so far (acceptance):
  - 2026-10-29, PC, main at 5810e19, Go at be67a86:
    - `sh scripts/check.sh` passed: 883 tests passed, 0 failed, 5 ignored (the live-copy tests that need env vars).
    - `sh tools/goldens/export.sh` was re-run. Only ULIDs and wall times changed. The tools, api and mcp goldens (and coord, inbox and capture) pass against both the fresh set and the committed one. The committed set is kept.
    - `sh tools/mod-suite.sh` passed: `claude plugin test` 19 pass, `bun test` 20 pass, plus the Rust attach side (`irori-attach` tests and `attach_exe`). The review below found that this showed less than it said: the mod's suites are hermetic (they test the mod alone), and the Rust side then ran against a second, test-only attach server.
  - Agent acceptance, run three times on the `irori mcp` member path.
    - Setup:
      - The dev exe ran `serve --headless` with no network, on a scratch `IRORI_HOME`. It held a fresh copy of the PC's `irori.db` and none of the identity files, so the Core minted a test device and person and nothing could fork.
      - `CLAUDE_CONFIG_DIR` pointed at an empty scratch dir, so the Core never read the real `~/.claude`.
      - The second member was `dummy-peer`: another dev `irori mcp` driven over stdio JSON-RPC. It answered each channel push it got with `irori_reply`.
      - The agent was `claude -p` 2.1.312 in a scratch cwd, run with `--mcp-config` (server `irori-dev` = the dev exe `mcp`, env `IRORI_HOME`), `--strict-mcp-config` and `--setting-sources project`. `CLAUDE_CODE_PLUGIN_DIRS` was unset. A probe confirmed the only Irori tools were `mcp__irori-dev__irori_*` and that no installed-mod attach ran.
    - Each run:
      - The agent joined with `irori_join` as a member (`<device>/grok-…`).
      - It read the Board. As an agent viewer it saw 2 working, 1 open and 38 done of 41 cards; the person's view is in the M3 replay.
      - It posted cards #42 and #43 with `irori_board_create`, and sent to `dummy-peer` with `irori_send`.
      - The dummy got its wake as `notifications/claude/channel` and replied. The agent read the reply with `irori_inbox` and closed it with `irori_skip`.
      - The Core wrote 6 `wake.issued` and 6 `message.sent` rows.
    - The agent's wake was issued and pushed. A tee on the agent's `irori mcp` stdout captured `notifications/claude/channel` with `[irori-wake:…]` and meta `{delivery_id, delivery_ids, from:"dummy-peer", message_id, trust:"untrusted"}`.
    - Claude Code `-p` never turned a channel push into a turn. This held with `--dangerously-load-development-channels server:irori-dev`, and with a stream-json session kept open for 60 s after its result. That is Claude Code behaviour, not Irori's: the model saw the wake only through `irori_inbox`.
  - `sh tools/goldens/replay-live.sh` on fresh copies after M4: all 147 views equal. 12883 rows over 2 devices; Rust appended nothing and Go appended only `device.started`.
    - Board: 8 byte-equal, 41 cards, doneTotal 38.
    - Decisions: 1 byte-equal.
    - Inbox: 3 byte-equal and 1 equal once sorted (279 deliveries).
    - Timelines: 121 byte-equal and 13 normalized as in M3.
- Results so far (M4 review, main at 0441498):
  - Fixed, 2026-10-29, PC, Go at be67a86:
    - One attach server. The test-only copy in `crates/attach` (stream, control, protocol, version, testhost) is gone; the crate keeps the `irori attach` runner and its client and re-exports `irori_api::attach`. The ported Go attach tests (two sessions talking with the tools-send step, supersede, next-turn appends, the unconfirmed wake, the appended-wake engage, keepalive, auth), TestGrokMember and TestGrokMemberSupersede on the Core's member table, the attach goldens, the runner tests and `attach_exe` all run against `irori_api::Server`, the one `irori serve` hosts.
    - `irori events` (app/src/events.rs over a new `gocompat::flag`, Go's flag texts and usage): the printer, the recent list and `--follow` with resync, backoff and redial. events_test.go and follow_test.go are ported (the follow tests on a real API server over a bare store). `main.rs` dispatches as main.go: a first argument starting with `-` is the window, an unknown command gets `unknown command "x" (gui, serve, mcp, attach, events, pair, node, version)` and exit 1; `pair` and `node` say they are not in this build yet, also exit 1.
    - API deadlines: ReadHeader 5 s, Read 10 s, Write 30 s and Idle 60 s are per-connection socket deadlines (`crates/api/src/conn.rs`) that the attach and events streams lift, as Go's `ResponseController` calls do. Read bounds the head and the body; once the body is in, the handler runs under the write deadline only (Go's background read instead cancels the request context at the read deadline). A body is read only after the Origin, Host, method and token checks, and only for a POST. Tests: an idle keep-alive closes after the idle timeout and not at the header one, a stalled head or body is cut, a tokenless POST is answered without its body, a stream outlives every deadline.
    - Diagnostics (`irori_core::diag`, Go internal/diag): every role but `version` opens `crash.log`, and a panic hook writes the panic and a backtrace there; `gui` and `serve` write the start line there and to the rotated `irori.log`, which then takes the Core's log lines with Go log's `2006/01/02 15:04:05 ` prefix (stderr too); unrouted lines get the same prefix. Not like Go: no runtime fatal-error hook (an abort that is not a panic is not caught), the start line names the Rust toolchain and has no vcs revision.
    - `scripts/check.sh` adds the single-threaded capture run and `scripts/no-third-party.sh`.
    - Ported Go tests: core TestSearchReachesBoardAndInbox, TestSearchKeepsMessagesToTheirAgents, TestStartMigratesAV03DataDir; mcpserver TestStreamCutKeepsSecret, TestRestartFailedRejoinRecovers, TestFreshDialSameRunNoRejoin, TestAttachErrorHidesSecret, TestSessionlessNameReuseIsNewMember; api TestV03RoutesWithoutCapture503, TestBoardRoutesWithoutServices503, TestEventsStreamResyncAndClose, TestV03WhyUnknownRepoIsReadOnly (no repo and no `repo.seen` row added), and the success shapes of `/v1/capture/status`, a blob found and a commit found; tools TestLeases (the clash refusal and steal), TestSendValidation, TestCallMalformed, TestUnboundCaller, TestDeferSkip, TestWatchDigestsMergeUntilRead.
  - Checks after the fixes (main at 98d631a and the deadline follow-up in the next commit):
    - `sh scripts/check.sh` passed: 923 tests passed, 0 failed, 5 ignored in the workspace run, plus 102 in the single-threaded capture run; no-third-party ok.
    - `sh tools/goldens/export.sh` re-run from Go at be67a86: every Go package passed; the same 8 golden files changed (ids, times and HLCs only). The whole workspace passed against the fresh set (923/0/5); the committed set is kept.
    - `sh tools/mod-suite.sh` passed: the mod's own suites 19 and 20 pass (the mod alone), and the Rust side, now against the real API server: `irori-attach` 21 tests and `attach_exe`.
    - `sh tools/goldens/replay-live.sh` on fresh copies: all 147 views equal. 12925 rows over 2 devices; Rust appended nothing, Go appended only `device.started`. Board 8 byte-equal (41 cards, doneTotal 38); decisions 1 byte-equal; inbox 3 byte-equal and 1 equal once sorted (279 deliveries); timelines 123 byte-equal and 11 normalized as in M3.
  - Not changed, with the reason:
    - `server/discover` stays "method not found" (crates/mcp/src/lib.rs). Answering it as go-sdk does switches the session to the stateless 2026-07-28 protocol, whose result shapes would all have to match go-sdk's; Claude Code and go-sdk clients take the error as "use initialize". It matters only for a stateless-only client.
    - The expiry timer in a headless Core: the Rust `feed-board` pump keeps coord subscribed for the Core's life, so expired leases are pruned and announced on time, where Go's headless serve does it lazily. Equal or better; views can differ from Go's only for the moment around an expiry.
    - Events-stream replay modes (bare-integer `since`, resync, limit above 1000) stay out of the API golden; `crates/api/tests/api.rs` covers them.
  - Ported after the review, 2026-10-29, PC, Go at be67a86: all 64 tools tests (`crates/tools/tests`: ported.rs holds tools_test.go, board.rs board_test.go, blocker_rich_test.go, room_test.go and owner_test.go, capture.rs capture_test.go and explore_test.go over a capture-on harness seeded as Go's `withCapture`, notify.rs notify_test.go), mcpserver TestClipAndOneLine, and api TestBoardSearchViewerScoping, TestBoardSearchFiltersAndPaging, TestBoardShowViewerScoping, TestBoardShowPaging, TestBoardOverview and TestBoardPersonNeedsYou on the real Core (`crates/api/tests/board.rs`). Not like Go: TestToolsUseTheOwnerRule and TestNotifyFailedPostFreesSlot test unexported helpers, so they are ported as behaviour (assign to `user:dev1` vs a made-up `user:` key; a failing post); TestClashedShortRefIsRefused feeds the foreign `task.created` row to the live coord (`Coord::handle`) where Go rebuilds the coord. Every ported test passed without a code change.
  - Owed:
    - The mod-attach live path: a real Claude Code session, through a `--plugin-dir` copy of the mod with `iroriPath` set to the dev exe, getting wakes as turns. `irori attach` answers 404 until the Core sees the session's card, so the Core must watch that session's `sessions/` dir. A probe on 2026-10-29: `claude -p` with `CLAUDE_CONFIG_DIR` at an empty scratch dir creates `sessions/` there, so a dev Core on that dir would see the card, but it is not logged in there ("Not logged in · Please run /login"). Running it needs either a login in that scratch config dir (by Stephen) or the Core watching the real `~/.claude/sessions`, which this run's rules keep off.
    - The Mac half: the M4 checks above have run on the PC only.
  - Done since: `xtask bindings --check` is a check.sh step (M6).

**M5 Mesh on iroh.**
- Build: `mesh` (FakeNet suite), `net`, `home` (relay server, port mapping, `node.addr`, awake), and LAN lookup.
- Check:
  - the FakeNet suite and the loopback iroh suite pass, including revoke, rotation, restore and fork, backfill, blobs and boards stage 1;
  - two dev Cores on copies of the PC and Mac data dirs sync rows both ways through the PC relay, at home and from outside.
- Results so far (home, LAN lookup, awake):
  - 2026-10-29, PC, loopback only: `crates/home` ports I:internal/home onto iroh. The embedded iroh-relay serves HTTPS on TCP N and QAD on UDP N with a self-signed certificate on the device key (D3), plus a v6-only TCP `[::]:N` listener feeding the same relay service while outside is on (iroh-relay binds one HTTPS address); admission is `CheckDevice` by key (this device plus certified, unrevoked devices), and a person row drops live clients that no longer pass (`Clients::disconnect`). Port mapping is go-room's `portmap` ported (UPnP through igd-next 0.17.1, NAT-PMP hand-written), separate from iroh's portmapper. `node.addr` rows on change, `home-server.json` with tri-state `outside`/`awake`, the Status lines and the awake hold follow Go. Tests: all of home_test.go and awake_test.go (the revoke test as relay Deny, with a fixture device's relay-only iroh endpoint pinned through `irori_net::tls`), portmap_test.go's mapper and NAT-PMP cases, and golden `testdata/golden/home` (`tools/goldens/home_golden_test.go` in I:internal/home, Go at be67a86): node.addr, Settings and Status bytes, Same/Dial/Outside, HostPort and splitPublic, CGNAT, and a Go server's Status and row for five router states, all equal from the Rust server with the same fake router (port masked).
  - `sys::awake` holds idle sleep on a dedicated thread per hold (Windows) or an IOPM assertion (macOS, not run yet); TestRealHold and TestTwoHolds pass on the PC.
  - The LAN lookup is its own crate, `crates/lan` (`irori-lan`), so both net (its `AddressLookup`) and home (its `netif` reads) use it without a net↔home cycle: `"IR1" || tag8 || nonce24 || secretbox(json{v,id,addrs,relay?,at,bye?})`, tag `sha256("irori/lan/1"||topic||u64be(unix/300))[:8]` with ±1 window, 2-minute age, 200 opens/s, directed broadcasts skipping Windows Public networks (Network List Manager), port-taken retry and roomnet's LANError lines. An iroh endpoint dials another by EndpointId alone through it on 127.0.0.1.
  - Merged into the Core: `Core::start` builds `home::Server` (Go core.go:290) unless `Options::no_home`, with `Options::home` as Go's `Options.Home` edit; `wanted` is another certified, unrevoked device until the mesh's HasPeer and the pairing window (M7) take over; `Core::home_server`, `set_home_server`, `set_home_server_awake` and `can_serve` are Go core/home.go; the run loop is one of the Core's waited loops, so `node.addr` lands before the log closes. Left for the mesh and net merges: `on_relay` (Go `Net.SetLocal`), one copy of `NodeAddr`/`CheckDevice`, `Servers.heard` from `irori_lan`.
  - Not yet run: the UPnP client against a real router or a fake IGD (only the mapper over in-process gateways and NAT-PMP over a 127.0.0.1 fake), port mapping and the relay on the PC's real network, the Public-profile read on a Public network, the macOS awake hold, and pairing guests on the relay (M7).
- Results so far (transport):
  - 2026-10-29, PC: the seam is `crates/meshseam` (`irori-meshseam`, no iroh dependency): `Transport`/`Room`/`Conn`/`RoomConfig`/`Stream`, Go's close reasons, `u32 BE` framing, and `mesh_secret` (Go golden `testdata/golden/meshseam`, `tools/goldens/meshseam_golden_test.go`). Differences from Go's interface are listed in its header: `Secret.epoch`, no `Pub`/`Sign`, async `join`, presence sends dial the address book, EKM fingerprints.
  - `irori_net::transport::IrohNet` implements it on iroh 1.4.1: lazy bind, the Chain gate in `after_handshake` (`admit::ChainAdmit`, Go `CheckDevice` as `admit::check_device`), the epoch-open exchange on `sync`, lower id dials (the higher after `dial_grace`), `replaced`, 4 outgoing / 9 incoming streams, `recheck()` closing revoked peers, a shared relay pin set (`tls::PinSet`) so relays from later `node.addr` rows work after bind, and `gossip` (iroh-gossip 0.101, ALPN `irori/gossip/1`, `TopicId = sha256(board topic)`, announcements only). The epoch-open message gains `deviceId` and a role byte in the HMAC input over mesh.md §5.2 (an acceptor only sees a key, and a role keeps one side from echoing the other's proof).
  - `crates/net/tests/mesh_iroh.rs`, 15 tests on loopback with real Chains and signed logs (the identity test harness), an embedded relay, two and three nodes: sync relay-only through the home relay, 1300×4 KB backfill on a direct path, a third node carrying history, duplicate → `replaced`, an uncertified device refused, revoke (live connections closed `revoked`, redials refused), rotation (epoch 2 refuses the old key; the old room closes `leaving`), restore (own rows back), fork, the 8 MiB cap, blobs (resume, zero-byte refusal, 10 fetches on 4 slots), board pulls (2000-row pulls in chunks) after a gossip announcement across three nodes, a relay learned after bind. Rows move with a test-only vector exchange until the mesh crate's session runs on `IrohNet`.
  - Merged with home on `next`: one `CheckDevice` (`irori_net::admit`, home's relay admission calls it), one `NodeAddr` (`home::nodeaddr`; net's `fromdb` stays spike-only), and `NetConfig::lan` adds `irori_lan::LanLookup` beside the address book. Left for the mesh merge: `IrohNet` built in the Core, home's `on_relay` feeding `set_servers` (Go `Net.SetLocal`), the `Servers` fold and `Servers.heard` from `irori_lan`, `recheck()` on person rows, the mesh session and FakeNet scenarios on `IrohNet` (then `MiniSync` goes).
- Results so far (mesh):
  - 2026-10-29, PC: `crates/mesh` ports I:internal/mesh above the seam (Node, session, frames, secret, files, boards stage 1, devices, Servers, peers, relay) over async `Transport`/`Room`/`Conn` traits (`mesh::transport`), with `Conn::accept_stream` in place of Go's `OnStream` and the Node also dialing every allowed device on its presence tick, since iroh has no presence bus. The FakeNet suite (`crates/mesh/tests`: sync, files, servers, devices, boards) passes, every Go mesh test ported. Golden `testdata/golden/mesh` (tools/goldens/mesh_golden_test.go, Go at be67a86): MeshSecret, person.secret sealing, every frame type, the board/blob bodies, node.addr, mesh-peers.json, hub-addrs.json, relay.json and the Status/DeviceRow JSON are byte-equal.
  - Not yet (superseded by the net review fixes below for the iroh twin and the Core wiring): both Mac checks.
  - Merged on `next` beside `crates/meshseam`, not yet joined to it: the two crates hold separate seams (`mesh::transport` pulls streams with `accept_stream` and has one `Box<dyn ByteStream>`; `meshseam` pushes them through `on_stream` with split halves, a `done()` future and typed `ConnError`) and two `Secret`/`mesh_secret` copies. Picking one seam (port mesh onto meshseam, or an adapter over `IrohNet`) comes before the Core builds the Node.
  - Net review fixes on `next` (2026-10-29, PC, loopback only): `mesh::OnSeam` runs the real Node on `IrohNet`, so the two seams are joined by that adapter (two trait sets remain) and `mesh::Secret` is the seam's. The Core builds the mesh (`crates/core/src/meshnode.rs`): one `IrohNet` for the Node and pairing, the address book fed from `Servers` and the Chain (`irori_net::book`), `Core::mesh()`/`Core::net()`, and `Options::revoke` defaulting to the Node's revoke. `crates/net/tests/node_iroh.rs` (7) runs the real Node on loopback iroh: sync, relay-only sync, revoke with rotation, the tie-break, restore, fork, attachments both ways; `mesh_iroh.rs` is down to 12 (its fake rotation/restore/fork tests went, `MiniSync` only drives transport traffic). Also: the LAN lookup drops replays per device and room, a second key for a held device id is refused, revoke returns the store error, `serve_blob` reads through tokio's file, `set_servers` unpins servers no longer listed.
  - Duplicates removed on `next` (2026-10-29, PC): one trait set, `irori-meshseam`'s. FakeNet implements it (a handle per device key, `FakeNet::keyed`, as `IrohNet` is keyed), the Node takes any seam transport and keeps only `mesh::PeerConn` (its pull view of a connection), and `mesh::OnSeam` and the pair tests' adapter are gone. One `NodeAddr` (`mesh::servers`, re-exported by `home::nodeaddr`; net's `fromdb` reader stays the spike tool's), one CheckDevice rule (`Chain::check_device`), one relay TLS construction (`irori_net::tls::server_config_on_seed`), one EKM/open-proof helper for the mesh and pairing transports. Golden tests read testdata through the `CARGO_MANIFEST_DIR` cargo sets at run time.
  - Status (built and checked on the PC, `sh scripts/check.sh` passes on `next` 2026-10-29): the FakeNet suite and the loopback iroh suite including revoke, rotation, restore, fork, backfill, blobs and boards stage 1 (check 1). The four PC items once open here (lazy LAN lookup, host-name relays, the server through its own relay, no public STUN/DNS) are closed, see the next bullet. Needs the Mac and the two-machine run: check 2, two dev Cores on copies of the PC and Mac data dirs syncing both ways through the PC relay, at home and from outside; the macOS awake hold; UPnP and the relay on a real router.
  - Closed on the PC (2026-10-29, loopback only): the LAN lookup on UDP 38419 is in the default transport (`NetConfig::lan_config`, `devices::default_net_config`) and starts when the endpoint binds (the mesh's first join or a pairing room, Go roomnet `start`), with `PairLan::wire` applied and its sightings feeding `Servers::heard` through the Chain (`IrohNet::set_on_heard`); `IrohNet::lan_error` is Go's `LANError` (merged on `next` with devices-view, it also covers a lookup started at bind, so Devices shows it). Server addresses that are host names (typed hints, a manual public address) become relays as `https://name:N/`, resolved at connect time. Every endpoint uses `irori_net::dns::LocalDns`: IP literals, the OS resolver for names, `localhost` answered locally, TXT and n0's names refused, so iroh's default resolver and its public fallback tier are never built. Tests: `node_iroh` `the_server_reaches_peers_through_its_own_relay_and_a_name_finds_it` (the PC homed on its own relay at 127.0.0.1 through `book::own_relay` dials the Mac, which knows the server only as `localhost:N`; both relay-only) and `crates/core/tests/no_public_contact.rs` (a default Core binds nothing alone; after a pairing window: no relay, no QAD, address lookups = book + LAN + gossip, no DNS asked, and on Windows `netstat` shows only loopback sockets for the process). Closed after (2026-10-30, `next`): home's `on_relay` now carries the relay's LAN address (`home::RelayUp`), and `Mesh::set_local_relay` names it to the bridge through `PairLan::set_relay` at the primary LAN IPv4 (none without a LAN; cleared when the server stops; test `crates/core/tests/pair.rs` `the_home_server_names_its_relay_to_the_bridge`); dev builds take the lookup's port from `irori_lan::dev_lan_port` (38419 + `IRORI_DEV_PORT_OFFSET`, ignored in release builds; a bad value turns the lookup off rather than leave it on Go's port). Left: the macOS arm of the socket guard (`lsof`), and the LAN lookup and a host-name server on a real network.

**M6 The window on Rust.**
- Build:
  - every called service method as a Tauri command, with events, tray, single instance, attachments protocol, settings and launch at login;
  - the `toast` card engine on Win32 and AppKit, and `notify`;
  - tauri-specta bindings; the shim and `frontend/bindings` are removed.
- Check: the Rust GUI on a copy can be used for every window flow on both machines, and the cards match Go's look and timings (inbox.md §3.13).
- Results so far (shell):
  - 2026-10-29, PC: `app/src/shell` hosts the Core and its API in the window role (lockfile `headless: false`), with Go's role split (`irori --startup` is the window; unknown commands keep Go's text and exit 1), the parent console for CLI roles, single instance by Go's instance id (`com.stephenshorton.irori[.<hash of IRORI_HOME>]`; a second launch shows the first's window and exits, forwarding nothing, as Go), close-to-tray, Quit closing the Core, the tray menu in Go's lines, and launch at login on the Wails registrations (HKCU `Run\Irori`, SMAppService or the LaunchAgent). Golden `testdata/golden/shell` (`tools/goldens/shell_golden_test.go` in Go's root package): instance ids, tray texts and role tables equal. `app/tests/shell_exe.rs` runs the real exe on a scratch copy with the dev shell probe (no window on screen, no tray icon): hand-off and quit pass. Go registers no URL scheme, so there are no deep links.
  - Not yet run: the macOS half (SMAppService, the template tray icon, Dock reopen), and the tray and window by hand on both machines.
- Results so far (window backend):
  - 2026-10-29, PC: `crates/window` ports every service method the frontend calls (surface.md §2.9) as a `#[tauri::command] #[specta::specta]` function `<service>_<method>`, the window pumps (§2.10), `settings.json`, and the `irori` URI scheme for attachments (§2.11: Range, ETag, nosniff, CSP sandbox; the page fetches it cross-origin, so it answers with the app origin in `Access-Control-Allow-Origin`). The commands live in that crate, not `app`, so `cargo xtask bindings` needs no `frontend/dist`. `frontend/src/gen/bindings.ts` is generated and checked (`cargo xtask bindings --check` in check.sh); `src/gen/models.ts` (from it, `frontend/scripts/gen-models.ts`) gives each model a factory with Go's zero values for the samples and tests. The `src/lib` gateways call `src/lib/host.ts`; the shim, `frontend/bindings` and the compat table are gone.
  - Area types (Task, Delivery, AgentCard, …) go to the window as their own Go-exact JSON, typed by hand-written shapes in `crates/window/src/ts.rs`; `tests/services.rs` checks every shape against the JSON a scratch Core sends, both ways.
  - The app on a copy of `%LOCALAPPDATA%\Irori` (`IRORI_HOME`, an empty `CLAUDE_CONFIG_DIR`) shows the real Board (grid and an opened card) and the Inbox's held history.
  - Seams (`crates/window/src/seams.rs`): the mesh, pairing, the notifier, launch at login and the updater are filled on `next` (see the review fixes below). Shared boards (M9) list nothing.
  - Merged with the shell on `next`: `shell::run` hosts the window backend (its specta invoke handler, the `irori` scheme, the pumps, the reviver; `app/src/gui.rs` dropped), the login seam is the shell's registration (`ShellLogin`, which also rebuilds the tray).
  - Window review fixes on `next`: the Devices calls are the Core's (`devices::CoreMesh`, `devices::CorePairing`: the home server, Remove device, Add/Join/the words/the key, run on blocking threads) and `events::start` forwards the Pairer's changes as `devices:pairing`. Since the net merge the Core runs a mesh Node and Remove device is its revoke; the mesh view below makes `CoreMesh` read it. The tray takes its counts from the pumps' own scans (`Window::on_inbox_counts`, `on_decide_count`, as Go's `onCounts` and `board.pump(.., tray.SetDecide)`), and its pause toggle is `InboxService.SetPaused` (drops the snapshot, nudges the pump). The window crate's own settings.json writer is gone: the notifier seam is `settings::NotifierSeam` over the notify crate. The page has a CSP allowing the `irori` scheme in `connect-src img-src media-src` (§4.2). Tests: Go's update card tests (`app/src/shell/update.rs`), the pumps and `devices:pairing` on loopback iroh (`crates/window/tests/pumps.rs`), the capture fold, and Go's card thread, message, answer, resolve, pin and comment tests (`crates/window/tests/cards.rs`).
- Results so far (toast and notify):
  - 2026-10-29, PC: `crates/toast` (the card engine on Win32; AppKit written, not compiled) and `crates/notify` (Notifier, settings.json, the Watch over inbox, Board and asks; golden `testdata/golden/notify`). Merged on `next`: `Options::notifier` gives the Core the app's Notifier (`None` is headless, so tests and `serve --headless` show no card and run no Watch); the Core adapts it to `tools::Notifier`, starts `notify::watch` after its loops and stops it first on close. The shell makes one Notifier per data dir (headless under the dev shell probe), its opener shows the window and emits `app:open`, the tray's held line opens the Held tab, and the window's notifier seam is the Notifier (options, set_options, post_test).
  - `app/build.rs` embeds `app/irori.exe.manifest` (per-monitor-v2 DPI, Common-Controls 6, `asInvoker`), the Go build's equivalent.
  - Not yet: the AppKit cards (`crates/toast/MAC.md`), and the cards in the Rust GUI by hand.
  - Status (`next`, 2026-10-29): built and checked on the PC by tests (window commands and shapes, pumps, cards, settings, attachments scheme, update card) and by the app on a copy showing the Board and Inbox. `CoreMesh` reading the running mesh (live device state, `fetch_file`/`offer`, the home view's `elsewhere`/`lan`/`remote`) is done (M7 "Devices mesh view"). Needs a person: every window flow and the cards by hand against Go's look and timings (inbox.md §3.13), the tray and window. Needs the Mac: the AppKit cards (never compiled), SMAppService, the template tray icon, Dock reopen, and the same window pass there.

**M7 Devices.**
- Build: pairing on iroh (EKM SAS), relay guest windows, the home/away bridge, Reset device, Remove device, the Devices window.
- Check: on loopback and between two dev machines, the pairing flow tests pass (identity.md §7.2 "flow tests"), and a fresh dev data dir pairs with a copy of the PC's.
- Results so far:
  - 2026-10-29, PC, loopback only: `crates/pair` (`irori-pair`) ports I:internal/pair onto the `irori_meshseam` seam: code, word lists (generated from the Go file), scrypt room secret, Info/SAS/proofs, `pair-rows.json` and `learn_saved`, the Pairer with Go's states, timers and texts. Goldens ROOM/INFO/FP (`testdata/golden/pair/vectors.json`, from identity.md §7.1) match. Go's nine flow tests pass on the M5 FakeNet with real Nodes (`crates/pair/tests/flow.rs`, through a test-only adapter from `mesh::transport` to the seam, which goes once the two seams are one).
  - Pairing on iroh (`irori_net::pairnet`, ALPN `irori/pair/1`, the seam's epoch-0 room). go-room's bus is gone, so the room is turned round: A's room is a LAN lookup room under the code-derived key and A announces itself in it; B's hello is a dial of every device heard in it (first stream `pair`, `{"t":"hello",deviceId,typ,body,tag,proof}`, proof = HMAC(room key, `"irori/pair-open/1"` ‖ EKM(`EXPORTER-irori-pair`) ‖ role)); A's `on_msg` gets the hello and A takes the held dial up with `connect` (`{"t":"ok",deviceId,proof}`); `pair.busy` is the held dial's close reason. The SAS fingerprints are the session's `EXPORTER-irori-sas` bytes on both sides (D5). The endpoint gate admits unknown ids on `irori/pair/1` only while a pairing room or a guest window is open, never on `irori/mesh/1`.
  - Relay guest windows (`irori_net::guest`, critic finding 3): a window is the room secret plus its owner; the home relay (`home::relay`) admits an unknown endpoint whose relay auth token is `hex(HMAC(room key, "irori/pair-guest/1"))[..32]` while a window is open, at most 8 guests, 2 windows per member, 10 minutes, and drops guests whose window closed. A room joined with `guest_ttl` opens a window on its own relay and, over `{"t":"guest",topic,key,ttlMs}` on `irori/pair/1`, on every home server it knows, held while that connection lives. Away from home B types `host:port`: it reads the server's device key from the relay certificate (D3; trust on first use, the words and typed key decide), joins the relay with the token, and dials the server, which answers `{"t":"at",id}` for the member whose window it holds (by `tag` = sha256(`"irori/pair-tag/1"` ‖ topic)[..16]).
  - The home/away bridge: a server holding a member's window announces that member's room on its LAN as an `irori_lan::Proxy` (`id` = the member, `relay` = the server's relay, new `relayKey` = the server's key), so B at home with only the code pins that relay, joins it as a guest and dials the away member through it. `irori_lan::Config` gains `on_room` (each announcement with its room's topic) and `proxies`.
  - Tests: `crates/pair/tests/iroh.rs` (on the LAN by the code alone; as a guest of A's relay with a typed `127.0.0.1:N`; through S's bridge with A on S's relay only and B on S's LAN only), each ending with both devices meeting in the person's mesh room admitted by the Chain alone; `crates/core/tests/pair.rs`: two Cores on fresh scratch data dirs with fixture identities pair through the Devices calls with Go's texts, and the joiner keeps its cert across a restart before any sync.
  - In the Core (`crates/core/src/devices.rs`): `learn_saved` at start, the Pairer (`Core::pairing`, `Options::pair_net`, `Options::pair_window`), home's `wanted` includes an open window and its relay shares the guest windows, a window kicks the home server, Reset is the existing `request_reset`/`reset`, and `Core::remove_device` calls `Options::revoke`. Left for the mesh merge: one `IrohNet` for mesh and pairing built in the Core with its LAN lookup wired through `PairLan::wire`, `Pairer::set_mesh(Node::kick, Servers)`, home's `on_relay` → the endpoint's own relay and `PairLan::set_relay`, `revoke` → `Node::revoke`. Until then the default pairing transport has no LAN lookup and no own relay, so only a typed server reaches it.
  - Not done (the join side and the window seam are done since, see below): the loopback API's join-side routes (Go routes_pair.go) and `irori pair join`/`irori pair reset`; the `crates/window` `Pairing` seam adapter (the window crate is now on `next` but still has its stand-in; `devices::{add_device, join_with_code, confirm_words, confirm_key}` are its calls); a two-machine run (PC and Mac copies, fixture identities) at home and from outside.
  - Net review fixes on `next` (2026-10-29, PC, loopback only): the mesh merge items above are done (one `IrohNet` in the Core, `revoke` → the Node's revoke, `PairNetFn` returns `IrohNet`); relay guests are dropped when their window closes (on the kick) or expires (a 2 s sweep), so stale guests hold no slots; when a pairing closes, the guest relays and pins it added are removed unless the mesh's relays name them (the via-hub path is reasoned, not tested). `crates/core/tests/pair.rs` also checks two Cores syncing through the LAN lookup alone and that Remove device rotates the key. The window's `Pairing` seam is the Core's (`devices::CorePairing`, M6 review fixes), so that item above is done.
  - Devices mesh view on `next` (2026-10-29, PC, loopback only): `devices::CoreMesh` reads the Core's running mesh through `irori_core::meshview` (Go devices_service.go over `core.Mesh()`, home.go, attachview.go). Device rows are the Node's `devices(false)` (connected/recent/offline, last seen, Go's warnings and actions); the Node now gets the folds' ignored counts (Go `Ignored: c.ignoredBy`), so "N of its rows were ignored" shows. The home view's `elsewhere` is `Servers::list()` non-empty, `lan` the LAN lookup's error line (`IrohNet::lan_error`), and `remote` Go's `remoteLine` while the net is on, from `Core::remote_servers()`: each other server the address book feeds as relays, timed from when it was first fed (Go `HubState.Since`, 30 s grace). Up means the endpoint's home relay is that server and connected, or is another server and connected: iroh keeps one home relay and reports connection state for it only, so unlike Go's per-server hubs an unreachable server that isn't the home relay never produces the line. Files held elsewhere are `Node::fetch_file` within Go's 2-minute `FetchTimeout`, offline holders named as Go's `fileOffline`/`deviceNames`; `offer` is the Node's. Tests: `crates/window/tests/mesh.rs` (two Cores: live state both ways, a file fetched from the other, an ignored row's warning, a 127.0.0.1 home server seen `elsewhere` and up as the other's relay, then gone: `recent`, server down, offline texts; three Cores: each sees the other two connected, a file crosses between the two joiners); `meshview` unit tests port `TestRemoteServersGrace`. `/v1/devices[?detail=1|true]` is `Backend::mesh_devices` → `Core::mesh_devices(detail)` (2026-10-30; 503 for a backend without a mesh; tests `crates/api/tests/bare.rs` `devices_route_like_go`, `api.rs` `devices_lists_this_device_first`). Left: health's `net` field (Go adds `{state, peers}` for a Core with a mesh); the PC↔Mac run of the Devices window (live dots, the home server seen from the Mac at home and away, a file held on the other machine).
  - Join side on `next` (2026-10-29, PC, loopback only): the loopback API's `/v1/pair/{join,status,key,cancel}` (`irori_api::pair`, Go routes_pair.go and client_pair.go: Go's statuses, texts and the Add view showing only mode and state; `Backend::pairing` gives the Pairer and its runtime), `irori pair join` (app `pair_cmd.rs`: the prompts, the running Core through the API or a Core of its own with its API and lockfile, every way out but "Paired." cancelling its own join, Ctrl+C/SIGTERM/SIGHUP after the prompts), `irori pair reset` (`irori_core::reset::reset`) and `irori node install|uninstall` (app `node_cmd.rs`, schtasks as Go). No role answers "not in this build yet" any more. Tests: Go's api_pair_test.go (`crates/api/tests/pair.rs`, two Cores with fixture identities on loopback iroh and LAN; Go's `203.0.113.7` server is `127.0.0.1:9` so nothing dials off the machine), pair_cmd_test.go and node_cmd_test.go (`app/src/pair_cmd/tests.rs`, `node_cmd.rs`). Not run: `irori pair join` from a real terminal against the installed app, `irori node install` against the real Task Scheduler (only its arguments are tested).
  - Status: pairing, guest windows and the bridge pass on loopback (`crates/pair/tests`, `crates/core/tests/pair.rs`), and so does the join side above. The default transport's lazy LAN lookup is closed (M5 above). Needs the two-machine run and a person: PC and Mac pairing and syncing through the PC relay at home and from outside, and a fresh dev data dir pairing with a copy of the PC's.

**M8 Release and cutover.**
- Build: the NSIS template, the macOS bundle, signing and notarization, the updater, a dry-run `release.yml`, and launch-at-login migration.
- Check:
  - with the Go build running, the Rust installer run as `/S /WAITPID=<go pid>` replaces it in a scratch per-user install and relaunches;
  - the `.app.zip` passes the Go updater's bundle checks on the Mac;
  - a full rehearsal on copies passes.
- Then: back up both data dirs, install on the PC and the Mac, confirm sync and agents, commit the Rust tree over `~/projects/irori`, and tag the first Rust release.
- Results so far (release):
  - 2026-10-29, PC: `crates/update` (`irori-update`) ports I:internal/update: `releases/latest`, the asset matchers, `SHA256SUMS` lines, `download_verified`, the phases (`restart_to_update` flips to installing before quit, a launch failure goes back to ready, `install_on_exit` only from ready, never two launches), `/S [/NORELAUNCH] /WAITPID=<pid>`, `cleanup_old_exes`, and the macOS stage/install (ditto, plutil, codesign, the swap script byte-equal to Go's) behind `cfg(target_os = "macos")`. Tests: service_test.go and macapp_test.go against a fake GitHub on 127.0.0.1, `go_050_takes_rust_060` (the Go rules offer `v0.6.0` over `0.5.0`), and `release_names.rs` (workflow_test.go over the release scripts, plus the NSIS and Info.plist contract).
  - `scripts/installer-check.ps1` passes: the installer built from `app/nsis/irori.nsi` with `-DIRORI_SCRATCH` (no shortcuts, registry or WebView2 step) installs into a scratch `Programs\Irori` with dummy exes: fresh install starts the new exe; `/S /WAITPID=<running dummy>` waits for it, replaces `irori.exe` and relaunches the new one; with a second process still running the old exe it is renamed to `irori.exe.old` (deleted by the next install); `/NORELAUNCH` starts nothing; a GUI that never exits gives exit code 3 with the old exe kept and relaunched.
  - Launch at login: the installer and the `.app` swap keep Go's paths, so Go's `Run\Irori` / SMAppService / LaunchAgent registration stays valid and the M6 shell reads it; `update::login::plan` is the GUI-start check that rewrites a registration for this exe into the Rust form (name `Irori`, `--startup`), leaving other exes' values alone.
  - macOS (`app/macos`: Info.plist, `package.sh`, `notarize.sh`, `check-bundle.sh` = the Go updater's bundle checks on the zip) is scripted, not run: nothing macOS was compiled or executed on the PC.
  - Wired on `next` (`app/src/shell/update.rs`, Go `setupUpdater` and update_card.go): one `Updater` in the GUI role on `<data dir>/updates`, the window's `Updater` seam (`check_and_download` runs as a task on the shared runtime and waits on a channel), `update:status` to the window, quit to `AppHandle::exit`, every transition to `Tray::set_update` and the sticky "Irori v<v> is ready" card (Install / Later), `auto_update` at start (not under the shell probe), `install_on_exit` after the run loop. Not run live: dev builds are `dev` and never check.
  - Release review fixes (`next`): the updater logs through a `with_logger` seam set to `irori_core::log` (irori.log once diag is in; the GUI exe has no stderr); the window seam's wait is `block_in_place` on a runtime worker; `with_running_app`/`with_pid` seams (Go `macPlatform(s, app, pid)`) and `crates/update/tests/mac.rs` ports mac_darwin_test.go's four tests, which type-check and pass clippy for `aarch64-apple-darwin` on the PC (a stub C compiler for ring) but have not run; the update card's eight update_card_test.go tests pass; `shell::login::LoginItem::cutover` runs `update::login::plan` at GUI start (scratch-key tested); `$FreshInstall` is decided before the rename-aside (installer-check covers it), and installer-check compiles the release-mode installer; the release scripts refuse to publish from a dirty tree or a HEAD that is not the tag, `-Finalize`/`--finalize` require all five assets and their `SHA256SUMS` lines, both build with `tauri/custom-protocol`, the WebView2 bootstrapper must be Microsoft-signed, and the Mac GUI smoke is opt-in (`IRORI_SMOKE_GUI=1`).
  - Rehearsal tooling (`tools/rehearse/rehearse.ps1`, `rehearse.sh` for the Mac): legs on fresh copies of the data dir, then a check that the real Run value, installed exe, uninstall entry, Tauri WebView2 folder and the Go repo's status did not change. `verify` and `replay` run the existing live scripts. `probe` (`irori-dev probe`) starts the dev `irori serve --headless` on a copy with `IRORI_DEV_OFFLINE=1` (`irori_core::offline`: a closed transport that never binds, plus `no_home`; honoured in every build since it only removes the network; the start line must say `offline` or the probe stops it) and an empty `CLAUDE_CONFIG_DIR`, asks each Go route once (token, no identity header, invalid POST bodies so nothing is written; an answer is a status below 500 that is not the router's own 404) and compares `irori mcp`'s `tools/list` with `GET /v1/tools?client=grok` and Go's `fallback_tools.json`. `update` runs Go's own internal/update code (`tools/rehearse/update_rehearse_test.go` via `go test -overlay`) against a fake GitHub on 127.0.0.1 serving the Rust installer, built with `IRORI_SCRATCH` and the new `IRORI_SCRATCH_INSTDIR` (Go passes no `/D=`), over a stub Go exe held open as an `irori mcp`; the go test process is the Go GUI (`/WAITPID`). Mode `restart` relaunches the dev v0.6.0 build under the shell probe (now also hides the window) on a fresh scratch home, offline, where `IRORI_DEV_RUN_KEY` (probe only, `Software\irori-rust-rehearse\…` only) runs the launch-at-login switch-over against a scratch Run key holding the value Wails wrote for the Go install; mode `exit` checks `/NORELAUNCH`.
  - Rehearsal on the PC (2026-10-29, `next` + this branch, live Go app running): verify passed (every row and RowHash, one Rust append, the Go Core opened the Rust copy); replay passed (all views equal); update passed both modes (Go found, downloaded and SHA256-verified v0.6.0, the installer waited for the Go GUI, renamed the held exe to `irori.exe.old`, wrote `irori 0.6.0`; the relaunched Rust build kept the Go Run value as the Rust form and read launch at login as on, another app's value untouched; `/NORELAUNCH` started nothing); untouched passed. probe FAILED on 5 of 34 routes, which are stubs on `next`: `/v1/devices` (503 "this Core has no devices layer") and `/v1/pair/status|join|key|cancel` (503 "this Core has no pairing"); the other 29 answered and `irori mcp` lists the same 39 tools as the Core and Go. So the full rehearsal does not pass until those routes are wired.
  - Release-mode update leg (2026-10-30, `next` 1ca3a68 + this branch, live Go app running): `rehearse.ps1 -Release -Skip verify,replay,probe` first runs `scripts/release.ps1 -Version 0.6.0 -SkipChecks` (its first dry run: `--release --locked --features tauri/custom-protocol`, `lto = true`, `codegen-units = 1`; smoke `irori 0.6.0`; the Microsoft-signed WebView2 bootstrapper fetched and packed; a 16.8 MB installer, a 15.0 MB zip, a 40.3 MB exe, and `SHA256SUMS` with a line for each), checks that `target\release\irori.exe` is the zip's exe byte for byte, and the update leg wraps that exe in the scratch installer (release.ps1's own installer is never run: its `$INSTDIR` is the real install and Go passes no `/D=`). Both modes passed every check, and so did untouched: Go v0.5.0's updater found, downloaded and SHA256-verified v0.6.0, the installer waited for the Go GUI, renamed the held exe aside and wrote the release exe (`irori 0.6.0`); in `restart` the relaunched release build came up under the probe with its window hidden, loaded the frontend from the embedded assets (`page-load http://tauri.localhost`), served (`GET /v1/health` on the port it noted: ok, version 0.6.0), kept the Go Run value and read launch at login as on, and quit through the probe; `/NORELAUNCH` started nothing. To run the shipped exe itself, a release build now honours the shell probe (`IRORI_DEV_SHELL_LOG`, and with it `IRORI_DEV_RUN_KEY`) only together with `IRORI_DEV_OFFLINE=1`; a debug build honours it alone (`probe::enabled`, unit-tested). Like the offline switch it only takes things away: no window on screen, no tray, no auto-update, no cutover on the real Run key, no network. The probe also notes `page-load <origin>` and `api-port <port>` now. A release (`windows_subsystem = "windows"`) exe's `version` text reaches PowerShell only through a pipe, so the leg reads it with `| Out-String`. The release artifacts were deleted afterwards (`target\release`, `dist\release\v0.6.0`; this paragraph is their record).
  - Not yet: the cutover against the real Run key; tests/mac.rs, `rehearse.sh` and the Mac half run on the Mac (a Go → Rust `.app` swap through Go's updater is not scripted); the release scripts run end to end; a two-machine run.
  - Status (`next`, 2026-10-29): merged with the window and net review fixes; `sh scripts/check.sh` passes; installer-check passed in a scratch dir. Needs the Mac: `cargo test -p irori-update --test mac`, `release-mac.sh`, the `.app.zip` against the Go updater's bundle checks (check 2), signing and notarization. Needs a person or a scratch Windows install: a real Go v0.5.0 → Rust v0.6.0 update with the release-mode installer and the launch-at-login cutover against a real Go Run value (check 1). The scratch Go v0.5.0 → Rust v0.6.0 update and the scratch-key launch-at-login switch-over now pass on the PC (rehearse.ps1); the full rehearsal on copies (check 3) failed only on the unwired `/v1/devices` and `/v1/pair/*` routes; `/v1/pair/*` is wired on `next` since the pair-cli merge and the devices view has landed, and `/v1/devices` in `crates/api` is wired since (2026-10-30), and the probe leg re-run (`rehearse.ps1 -Skip verify,replay,update`, 2026-10-30, fresh copy) passed: 34 of 34 routes answered, `/v1/devices` 200, `irori mcp` the same 39 tools as Go, untouched passed; `irori-dev probe --go-lockfile` (under `IRORI_VERIFY_DEVICES=1`) found Rust's and the running Go Core's `/v1/devices` equal in deviceId, name, role, os, this and revoked for both rows, differing only in live fields (this PC's own-row warning, the Mac's state/lastSeen) as expected offline. So every leg of check 3 has now passed on the PC, though not yet all in one run. `release.ps1` has had one dry run (`-SkipChecks`, 2026-10-30, the release-mode update leg above); never run: `-Publish`/`-Finalize`, `release-mac.sh`; then the backup, install on both machines and the first Rust tag.

**Owed after the gap parts** (2026-10-30, `next` with devices-view, lan-relays, pair-cli, dedupe, test-ports and rehearsal merged, then devices-route, release-mode and flaky; `sh scripts/check.sh` passed on that merged tree with no fix needed: 1403 tests passed, 0 failed, 5 ignored across all its runs). M1-M8 are built; what each still owes, by who or what it needs:
- PC code (no other machine or person needed):
  - `GET /v1/health`'s `net` (`{state, peers}`, Go's for a Core with a mesh; `Node::connected_count` is ready).
  - `IRORI_DEV_PORT_OFFSET` moves only the LAN lookup's UDP 38419; the home server's port is not offset (it moves off a taken port by itself).
  - One full `rehearse.ps1 -Release` run with every leg on (each leg has passed on its own, M8 check 3).
  - The Devices home view's 30 s remote-server grace is covered by unit tests only, not end to end (M7). Known gap from Go: iroh reports state for its one home relay only, so "can't reach your home server" shows only when that relay is down.
- Mac (birch or on the Mac):
  - The Mac halves of M2 (its log and Keychain) and M3 (replay), and the M4 checks there.
  - The `lsof` arm of `no_public_contact`'s socket guard; the macOS awake hold (M5).
  - The AppKit cards (never compiled), SMAppService, the template tray icon, Dock reopen (M6).
  - `irori pair join`'s SIGTERM/SIGHUP handling and the non-Windows `irori node` text, which compile only there (M7).
  - The capture test fixes from the flaky pass (repo-key lookup allowed capture's 15 s git limit; the edit time taken in the first 0.8 s of a second, the Mac's "unattributed" failure) need a run on the Mac.
  - `cargo test -p irori-update --test mac`, `rehearse.sh`, `release-mac.sh`, the `.app.zip` against the Go updater's bundle checks, signing and notarization; a Go → Rust `.app` swap through Go's updater is not scripted (M8).
- Two machines (PC and Mac copies, fixture identities where the rules ask):
  - M5 check 2: two dev Cores on copies of the PC and Mac data dirs syncing both ways through the PC relay, at home and from outside; the seam merge (dedupe) has run on loopback only.
  - The LAN lookup between two machines, a typed host-name server, UPnP and the relay on a real router, the Public-profile read on a Public network (M5).
  - The pairing bridge's relay named from home (`PairLan::set_relay`) on a real LAN; tested on loopback only (M7).
  - PC↔Mac pairing through the window and through `irori pair join`, at home and from outside, and a fresh dev data dir pairing with a copy of the PC's (M7 check).
  - The Devices window across machines: live dots, the home server seen from the Mac at home and away, a file held on the other machine (M7).
- A person (Stephen):
  - The M4 mod-attach live path (a Claude Code login in a scratch config dir).
  - Every window flow, the tray and the cards by hand against Go's look and timings, on both machines (M6).
  - `irori pair join` from a real terminal and `irori node install` against the real Task Scheduler (M7).
  - The launch-at-login cutover against the real Run key and a real Go v0.5.0 → Rust v0.6.0 update with the release-mode installer (M8 check 1; the release exe through Go's updater into a scratch install passes, `rehearse.ps1 -Release`); the release scripts for real (`release.ps1` has had one dry run); then the backup, install on both machines, and the first Rust tag.

**M9 Stage 2 boards on iroh** (section 3.3), after the cutover. This covers transport, invites, bridge, autonomy, mirrored leases, asks and review, following the v0.5 build steps 5-11.

---

## 9. Process notes

### 9.1 Rules for the port

- Nothing in `~/projects/irori` or `~/projects/go-room` is edited until the cutover commit.
- Tests and probes bind `127.0.0.1` only.
- Rust code never reads or logs secrets. Tokens and the person secret are never printed; `irori.json`'s token stays out of every log (surface.md §2.2, §7.13).

### 9.2 Go keeps moving

Go development continues during the port. `docs/port/BASE` (created at M1; it does not exist yet) records the Go commit the specs match (`be67a86`). At the start of each milestone, `git -C ~/projects/irori log be67a86..main` is reviewed: deltas in the areas already ported are ported, the frontend copy takes the same edits, and BASE moves forward.

---

## 10. Decisions for Stephen

| # | Decision | Recommendation |
|---|---|---|
| D1 | Go↔Rust on the wire: interoperate, or switch both machines together? | **Switch together.** Data and history carry over byte-compatibly, and an interop layer would mean porting go-room just to discard it (section 0). |
| D2 | iroh `SecretKey` = the device ed25519 key? | **Yes.** Already-paired devices are known by their cert rows, with no re-pairing and no new key file. A separate iroh key bound by a signature is the alternative if one key for TLS and row signing is unwanted. |
| D3 | Relay TLS on a home PC with no domain | **Self-signed and pinned.** Prefer a cert on the device key, pinned to the certified `devicePub` (no row change; this extends D2's single-key use to the relay cert); fall back to a `relayCert` field in `node.addr`. M1 decides between them. |
| D4 | `relay.json` (the advanced TURN relay) | **Drop it.** It has no iroh meaning and is absent on the PC. The setting shows `none` with a note. |
| D5 | Pairing SAS channel binding | **TLS exporter bytes (EKM)** in place of DTLS fingerprints. The words change, and Go↔Rust pairing is impossible either way. |
| D6 | LAN discovery | **Keep the sealed rotating-tag lookup on UDP 38419**, not iroh mDNS, which broadcasts the stable device id. |
| D7 | Cards engine | **A native port of go-toast** (Win32 layered windows and AppKit panels). OS notifications change look and behaviour. A transparent Tauri webview per card would still need no-activate, capture-exclude and topmost behaviour proven on both OSes. |
| D8 | Rust-only tables in `irori.db` before the cutover | **None.** Schema v4 locks the Go build out, so it comes only after the cutover. |
| D9 | Launch at login | **Own `winreg` + `objc2-service-management`**, not `tauri-plugin-autostart`, so the Wails-made registrations are found and replaced. |
| D10 | Frontend bindings | **tauri-specta behind the `src/lib` gateway**, after an M1 compatibility shim that runs the current bindings unchanged. |
| D11 | First Rust release version and updater | **The next minor above Go's latest (e.g. `v0.6.0`), installed by the Go updater; the Go updater design ported**, not tauri-plugin-updater. |
| D12 | Go development during the port | **Keep shipping Go fixes**, but avoid changing wire formats, row payloads or the data dir in Go while the port runs. Deltas get ported per milestone (9.2). |
| D13 | Stage 2 gossip | **iroh-gossip per board for announcements only.** Rows move on member-gated `board/` streams over `irori/board/1`. |
| D14 | Mac testing on copies | **A dev-only `IRORI_DEV_KEK_DIR`** so a copied Mac data dir can find its Keychain KEK, compiled out of release builds. |

## Not determined here

- The PC's subnet relative to the Mac's double NAT, and whether the Mac has `relay.json` or `hub-addrs.json` (mesh.md §8). M1 records both.
- iroh questions for M1 (mesh.md §8): an Ed25519 device-key certificate through rustls on relay server and client, whether `insert_relay` works on an endpoint bound with `RelayMode::Disabled`, and the `PathStats` fields. (Answered in this review: the access-control trait is `AccessControl`; custom TLS is accepted on both sides; `http://` relays are ruled out by QUIC address discovery; tauri-specta for Tauri 2 is `2.0.0-rc.26`.)
- The contents of `I:build/darwin/package.sh` and `notarize.sh` (surface.md "Not determined").
- The schema version of the live Mac database (expected v3, store.md §7.9).

## Critic findings

This review checked the specs against Go `be67a86` (the Go `HEAD` still equals the port base, so section 9.2 has no delta yet), and the iroh, Tauri and crate claims against docs.rs and crates.io on 2026-10-29.

**Checked and holding:** the schema v1-v3 DDL and `schema_migrations` (I:internal/store/sqlite.go:80-133,299-335 = store.md §2); signing bytes and `RowHash` (I:internal/store/store.go:173-176,297-311 = store.md §2.5); `Canonical` (I:internal/identity/rows.go:111-129 = identity.md §3.2); the data-dir rule (I:internal/paths/paths.go:12-26); the NSIS uninstall key (I:build/windows/nsis/wails_tools.nsh:25-28); sync frame names and caps (I:internal/mesh/frames.go:16-33). All 82 kind strings in I:internal/store/kinds.go, all 41 tool names, every non-test `/v1/` route and all 97 bound service methods in `I:frontend/bindings` appear in a spec. Every `internal/` package has a crate in section 1.1. The iroh `Builder` calls (`empty`, `relay_mode`, `address_lookup`, `hooks`, `secret_key`, `alpns`), `EndpointHooks::after_handshake` with `AfterHandshakeOutcome::Reject`, `tauri::async_runtime::set(TokioHandle)` and `register_asynchronous_uri_scheme_protocol` exist in iroh 1.4.1 and tauri 2.12.5.

**Fixed in place** (sections 1.1, 1.3, 3.1, 4.2, 4.3, 7.2, 9.2 and "Not determined"; mesh.md §5.4 and §8; surface.md §5): tauri-specta for Tauri 2 is `2.0.0-rc.26`, not 1.0.2; the relay's TCP N is `TlsConfig.https_bind_addr`, not `http_bind_addr`; the access-control trait name; custom TLS is accepted on both relay ends; the single-instance plugin takes its id from `config.identifier`; ed25519-dalek is 3.x; `pages.yml` exists; the Mac data dir; the full root-file list.

**Open, bigger than a doc edit:**

1. **The cutover commit must keep repo-root paths the installed mod depends on.** `I:.claude-plugin/marketplace.json` publishes the plugin from `"source": "./mod"`, and `I:mod/.claude-plugin/plugin.json` sits under it. The section 1 layout has neither `mod/` nor `.claude-plugin/`, and neither does it have `site/`, `assets/brand/` or the Go `docs/` tree. Section 1 should add them as carried-over paths, or committing "the Rust tree over `~/projects/irori`" (section 5.2 step 4) breaks the plugin marketplace entry Stephen's Claude Code installs from. `mod/dev/fakecore` and `mod/dev/fakeirori` are Go programs with their own `go.mod` (I:mod/dev/go.mod); only fakecore has a replacement plan (section 6.4), so fakeirori needs one too.
2. **Relays known after bind.** The endpoint binds lazily, but `node.addr` rows can add or move home servers while it runs. `Endpoint::insert_relay` / `remove_relay` exist in iroh 1.4.1. What is not known is whether they work on an endpoint bound with `RelayMode::Disabled` (the "no server known yet" state in section 3.1). If not, `net` needs a rebind path, or it must bind with a placeholder custom map. M1 should test it.
3. **Guest admission has an existing channel.** The client-side `iroh::RelayConfig` has `auth_token` / `with_auth_token`, and the server's `ClientRequest` exposes `auth_token()` and `query_pairs()` (docs.rs iroh and iroh-relay 1.4.1). A pairing or invite window can therefore admit a guest by a code-derived token instead of by an unknown EndpointId. That narrows "relay guests are the hard piece" (section 3.3, mesh.md §5.4), and the design should be rewritten around it.
4. **Windows manifest.** `I:build/windows/wails.exe.manifest` declares Common-Controls 6, `dpiAwareness permonitorv2,permonitor` and `asInvoker`. Per-monitor-v2 DPI matters to the native `toast` windows (D7). No spec compares this with the manifest `tauri-build` embeds; `app/build.rs` must embed an equivalent one.
5. **`I:docs/releasing.md` is unread.** Together with `I:build/darwin/package.sh` and `notarize.sh`, it is the release procedure that section 7.2 ports. Read it before M8. `I:Taskfile.yml`, `I:build/Taskfile.yml` and `I:build/config.yml` (the Wails build entry points) are replaced by cargo/xtask, and section 7.1 should say so.
6. **Linux is out of scope, but the docs don't say so.** `I:build/linux/*` (AppImage, nfpm) is a Wails template leftover, the updater refuses Linux (surface.md §3), and the `_other.go` files mean "file keystore, no tray specifics" (I:internal/identity/keystore_other.go:1-6). The plan should state that Linux is neither built nor released, and that the `_other` paths exist only so the code compiles there.
7. **`specta`/`tauri-specta` are release candidates.** Pinning `=2.0.0-rc.26` works, but every rc bump can change the generated TS. `xtask bindings --check` (section 7.1) catches the drift; D10 should note the rc risk, or prefer `ts-rs` 12.0.1 (stable) for models plus hand-written command wrappers.
