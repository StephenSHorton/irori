# Boards stage 2: transport interface

*2026-10-30, card #43. The contract the other stage 2 parts code against: invites (#44, `boards2-invites`), window (#45), bridge (#46), leases (#47). Inputs: v0.5-shared-boards.md step 5 and §6 (decisions B1-B5 in §13 are made), architecture.md §3.3. Everything named here compiles on `boards2` now; the parts that move bytes return `TransportError(UNSUPPORTED)` (`"boards2: transport not built yet"`) until the transport branch fills them in. No behaviour changes until then: stage 1's `board/<id>` streams on the private mesh connection keep running, and the transport branch removes them.*

## 1. Rules

- **A board's rows move only on that board's connections.** One `irori/board/1` connection per (device, device, board). It serves `board/<boardId>` pull streams for that board and nothing else.
- **`irori/mesh/1` stays private.** It admits only your own certified devices (`ChainAdmit`, unchanged). Another person's device never reaches it.
- **Admission on a board is the board's verifier:** a counted cert of a current member, with no cut (`BoardVerifier::admitted_by_pub`).
- **Gossip carries announcements only:** `{boardId, epoch, vector}`. Rows move on the gated pull streams.

## 2. The board connection (`irori/board/1`)

| Piece | Where | What |
|---|---|---|
| `BOARD_ALPN = b"irori/board/1"` | `irori-net` `board` | the ALPN |
| `BOARD_APP = "irori/board/1"` | `irori-meshseam` | topic salt and prefix, same string |
| `BoardOpenMsg {v, boardId, epoch, deviceId, proof}` | `irori-net` `board` | first message on `sync`, each way |
| `board_open_proof(key, ekm, role)` / `board_open_proof_ok` | `irori-net` `board` | real (not stubs) |
| `EKM_BOARD = b"EXPORTER-irori-board"` | `irori-net` `board` | exporter label of EKM32 |

`proof = hex(HMAC-SHA256(DeriveBoardTopic(boardId, epoch, secret).key, "irori/board-open/1" || EKM32 || role))`. Role is 0 for the dialer and 1 for the acceptor, the same shape as the mesh-open proof. The key comes from `identity::derive_board_topic` / `BoardSecrets::topics()`, unchanged. A connection whose proof fails closes `refused` before any frame.

After the open message, on `sync`:
- **`BoardHello`** (`mesh::boardmesh`): `{v, deviceId, personId, boardId, boardEpoch, coreVersion, schema, vector, wallMs}`. A mismatch on `v` (`BOARD_PROTOCOL = 1`) closes with reason `version`.
- **`vector`** and **`ping`/`pong`**: the existing frames.
- **`board.secret`** after a rotation (§4 below).

Rows move on **`board/<boardId>` pull streams**: the stage 1 request and chunk format (`mesh::boards`, `{"vector":…}` then `{"events":[…]}` chunks, at most 2000 rows). `board.*` and `member.*` rows go first on catch-up.

**Seam.** `irori_meshseam::BoardTransport::join_board(BoardRoom, RoomConfig) -> Arc<dyn Room>`, where `BoardRoom {board_id, secret: Secret}` and `secret` is the board topic of one epoch. `RoomConfig::accept` is the board's member check. Who dials is unchanged: the lower device id. `IrohNet` implements `BoardTransport` (stub).

**Above the seam** (`irori-mesh` `boardmesh`):
- `BoardMesh::new(BoardMeshOpts {board_id, self_id, person_id, core_version, log, verifier, secrets, transport})`
- `start()`: joins a room per held epoch, keeping the previous epoch's room while members move.
- `kick()`: pull now.
- `recheck()`: close connections the verifier no longer admits.
- `close()`

The Core will keep one `BoardMesh` per open board, beside the private `mesh::Node`.

## 3. Admission

| Gate | Type / fn | Crate | Rule |
|---|---|---|---|
| Endpoint `after_handshake` | `MemberAdmit {chain: ChainAdmit, boards: Arc<dyn BoardAdmit>}` impl `Admit` | `irori-net` `board` | `irori/mesh/1`, `irori/pair/1`: chain only. `irori/board/1`, `irori/gossip/1`: chain, or a member device on at least one board held here. Anything else: refused. Real. |
| Membership source | `trait BoardAdmit { board_device(board_id, &EndpointId) -> Option<String>; boards_of(&EndpointId) -> Vec<String> }` | `irori-net` `board` | the Core implements it over every open board's `BoardVerifier`. `NoBoards` is the empty one. |
| Per board, by key | `BoardVerifier::admitted_by_pub(device_pub_hex) -> Option<MemberCert>` | `irori-identity` | a counted cert with that devicePub and no cut (not revoked, person not removed or gone). Real. |
| Per-board room | `RoomConfig::accept` of the board room | `irori-mesh` `boardmesh` | `admitted_by_pub` on that board's verifier only |

The endpoint gate is coarse ("a member somewhere"). The board room's `accept`, after the open proof, is the per-board check. A member of board A who proves board B's key is still refused by B's verifier.

## 4. Board secrets on the wire

- `FRAME_BOARD_SECRET = "board.secret"`
- `BoardSecretFrame {boardId, boardEpoch, maker, sealed}`
- `seal_board_secret(seal_key, board_id, epoch, secret, recipient)` / `open_board_secret(...)` (stubs)

All of these live in `irori-mesh` `boardmesh`. The frame carries a secret in two places:
- **To your own devices:** on the private mesh `sync` channel, sealed under the mesh epoch key, as `person.secret` is. This is how your other devices get epoch 1 and every later epoch, and it replaces stage 1's "step 5" note.
- **To other members' devices after a rotation:** on each board connection, sealed under that connection's (old) board epoch key, and only to current member devices.
- **To an invitee:** inside the invite bundle (#44). That path does not use this frame.

`BoardSecrets` (`irori-identity`) stays the store. Its lower-maker rule decides between two rotations to the same epoch.

## 5. Gossip

The gossip pieces were already built in M5 (`irori-net` `gossip`), and stage 2 keeps them:
- **Topic:** `topic_id(&BoardTopic) = sha256(topic string)`.
- **Payload:** `Announcement {boardId, epoch, vector}`.
- **Channel:** `IrohNet::board_channel(board_id, topic, bootstrap) -> BoardChannel`, with `announce`, `next` and `join`.
- **ALPN:** `irori/gossip/1`.

What changes in stage 2 is only who may connect: the gate becomes `MemberAdmit`, so gossip admits member devices. An announcement from a member whose vector is ahead calls `BoardMesh::kick()`.

## 6. Servers: B1 relay admission and `board.server`

- **Relay admission.** Every member's home server admits the counted member devices of the boards its Core holds.
  - `home::relay::BoardAdmitFn = Arc<dyn Fn(&EndpointId) -> bool>`, plus `home::relay::member_admit(own, boards) -> AdmitFn`.
  - `home::Config.board_admit: Option<BoardAdmitFn>`, which defaults to `None` and is not read yet.
  - The relay only forwards sealed QUIC, so "for that board only" is enforced at the endpoints: the `irori/board/1` proof plus that board's verifier. A member device on your relay can't reach `irori/mesh/1` on your devices, because their gate refuses it.
- **`board.server`** (`KIND_BOARD_SERVER`). The payload is `mesh::boardmesh::BoardServer {deviceId, port, public: Option<PublicAddr>}`.
  - It carries the public address and mapped ports only, never LAN or v6 addresses. `public: null` means the server is off or outside reach is off.
  - It is written by the server device into each board its person is in, only when it changes.
  - Producer: `home::Server::board_server() -> Result<BoardServer, String>` (stub).
  - Fold: `mesh::boardmesh::board_servers(log, verifier) -> Vec<BoardServer>` (stub). It keeps the newest row per server whose writer is that server device and a counted member.
- **Dialing them.** `IrohNet::set_board_servers(Vec<BoardRelays {board_id, servers: Vec<RelaySpec>}>)` (stub).
  - Each `RelaySpec` is pinned to the server's devicePub from its member cert.
  - Members dial every `board.server` relay in parallel and keep the first handshake per server.
- **`IrohNet::drop_board(board_id, why)`** (stub) closes every connection of a board on removal, leave or forget.

## 7. Invites: hooks for #44

The ticket format, the invite room secret (`SecretFromCode("irori-invite-v1", code)`), the person proofs, the five words and the bundle all belong to #44. Transport gives #44 these pieces:

| Hook | Where | Contract |
|---|---|---|
| `INVITE_ALPN = b"irori/invite/1"` | `irori-net` `board` | the invite connection's ALPN |
| `INVITE_OPEN_LABEL = b"irori/invite-open/1"` | `irori-net` `invite` | first message proves the invite room key, as the board-open proof does |
| `GuestWindows::open_invite_window(owner, secret, ttl) -> Result<u64, TransportError>` (stub) | `irori-net` `guest` | at most `INVITE_MAX_TTL` (1 h), at most `MAX_INVITE_WINDOWS_PER_MEMBER` (2) per owner, beside the 2 pairing windows. Closed with the existing `close_window(id)`. While it is open, the inviter's relay admits a client presenting `invite_token(room.key)`, and the endpoint accepts `irori/invite/1`. |
| `invite_token(key)`, `invite_tag(topic)` | `irori-net` `guest` | real. The relay auth token is `hex(HMAC(key, "irori/invite-guest/1"))[..32]`; the LAN bridge tag is `hex(SHA-256("irori/invite-tag/1" \|\| topic))[..32]`. |
| `WindowKind {Pair, Invite}`, `GuestWindows::kind(id)` | `irori-net` `guest` | which kind a window is |
| `InviteAddr {id, relay, relay_quic_port, relay_pub, direct}` | `irori-net` `invite` | what the ticket must carry: the inviter's endpoint, its home relay and pin, and direct addresses |
| `IrohNet::set_invite_handler(Option<Secret>, Option<InviteHandler>)` (stub) | `irori-net` `invite` | inviter side. `InviteHandler = Arc<dyn Fn(Arc<dyn Conn>)>` is called with each connection that proved the room key. The first one wins, and #44 closes the window. |
| `IrohNet::dial_invite(&InviteAddr, &Secret) -> Arc<dyn Conn>` (stub) | `irori-net` `invite` | invitee side. It dials as a relay guest and proves the room key. |

On the resulting `Conn`:
- `send`/`recv` carry #44's protocol.
- `fingerprints()` returns the session EKM (`EXPORTER-irori-sas`) that the five words bind to, as in pairing.

After the bundle, the invitee's Core opens the board and its `BoardMesh`. Its other devices get the secret through `board.secret` on the private mesh (§4).

## 8. What the transport branch does next (card #43)

1. Fill in the stubs above.
2. Add `irori/board/1` and `irori/invite/1` to the endpoint's ALPNs and `accept_loop`.
3. Switch the Core to `MemberAdmit` with a `BoardAdmit` over its open boards. Wire `home::Config.board_admit`.
4. One `BoardMesh` per open board in the Core. Remove stage 1's `board/` streams from the private mesh session.
5. Write and fold `board.server`.
6. Loopback tests (127.0.0.1 only):
   - two people, two boards;
   - a member of A refused on B;
   - a removed member dropped;
   - `irori/mesh/1` refusing a member device;
   - gossip kick;
   - rotation through `board.secret`.

## 9. Built: the board connection (`boards2-transport-conn`)

Steps 1 (its board half), 2 (`irori/board/1`), 3 (the gate), 4 and 6 (part) are in:

- **`irori-net`**: the endpoint offers `irori/board/1`; `handle_board` / `dial_board` run the board-open exchange (a wrong proof, an unknown board or a refused device closes `refused` before a frame). `join_board` keeps one room per (board, epoch); `drop_board` closes them.
- **`irori-mesh` `boardmesh`**: `BoardMesh` joins a room per epoch `BoardSecrets::topics()` holds, admits by `admitted_by_pub` (whose device id must match), says `hello`, pulls on `board/<id>` (any other label is refused), sends `vector`/`ping`, and hands a newer epoch's secret to members still on the older one (`board.secret`, sealed under that connection's epoch key). Rows landing re-run the admission check, so a removal cuts on both sides. `seal_board_secret` / `open_board_secret` are real.
- **Stage 1 removed**: the private Node no longer serves or pulls `board/` streams (`Options.boards`, `BoardLogs`, `Session::board_loop` and `mesh/tests/boards.rs` are gone).
- **Core**: the endpoint gate is `MemberAdmit` over `NetConfig::boards`. `Core::open_board_net(BoardParts)`, `close_board_net`, `kick_board`, `recheck_board` and `board_addr_hint` are the hooks the board-open path (#44, M9) calls. Every refeed re-feeds each board's member devices and re-checks its connections.

Interface additions (none of the published signatures changed):
- `IrohNet::set_board_peers(board_id, Vec<PeerAddr>)`: a board's own address book, which `set_peers` never clears and the private mesh never dials. Also `board_conns`, `board_rooms`, `board_members`.
- `NetConfig::boards: Arc<BoardMembers>` and `board::BoardMembers`, the Core's `BoardAdmit` over open boards' verifiers.
- `BoardMesh::{set_intervals, set_log, on_rows, member_devices, epochs, peers}` and `boardmesh::MemberDevice`.

Still open:
- `board.secret` to your own devices on the private mesh `sync` channel.
- Gossip kick, B1 relay admission, `board.server`, and `settle` once every member has said hello on the new epoch.

Tests: `net/tests/board_iroh.rs` and `core/tests/boards2.rs`.
