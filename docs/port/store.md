# Port spec: store, hlc, kinds registry, board logs, ledger

Source tree: `../irori-go` (Go). All `file:line` references are into that tree, abbreviated:
`S` = `internal/store/store.go`, `Q` = `internal/store/sqlite.go`, `K` = `internal/store/kinds.go`,
`B` = `internal/store/board.go`, `H` = `internal/hlc/hlc.go`, `L` = `internal/ledger/*.go`.

Goal: a Rust crate (`irori-store`, plus `irori-hlc`, `irori-ledger`) that opens the **same** `irori.db` and
`boards/<id>.db` files a Go build wrote, keeps them openable by the Go build afterwards (until cutover), produces
**byte-identical signing bytes and row hashes**, and makes the same accept/refuse decisions as Go on every
replicated row, so a Go device and a Rust device can sync with each other.

---

## 1. What the area does, and its interface

### 1.1 Responsibilities

| Piece | What it is |
|---|---|
| `hlc` | Hybrid logical clock. `u64` = wall ms `<< 16` + 16-bit logical counter (H:1-61). |
| `store` (private log) | One SQLite DB per device, `<DataDir>/irori.db`. Append-only rows keyed `(device_id, seq)`; seq is 1-based and gapless per device (S:34-45). Replication by vector exchange: `Vector()` / `Missing(v)` / `Insert` (S:1-8). Device-local rows in a second table `local_events` (S:9-14). Rows past a revocation cut moved to `refused_events`; conflicting rows recorded in `forks`. |
| `store` (board log) | Same code and schema, opened with `Board: true`, at `<DataDir>/boards/<boardId>.db` (core/boards.go:51-52, 382). Adds a kind allow-list, actor shape rule and reference closure (B:12-30). |
| Kinds registry | Map kind -> Side (Replicated/Local/Unclassified) (K:137-175), and kind -> Board verdict (B:58-98). |
| `ledger` | **Not storage.** Pure, in-memory PR-authorship view + PR-reference parsing (L ledger.go:1-5). The capture area feeds it entries folded from the log. See section 8. |

The store **holds no keys and does no ed25519** (S:14). It takes two trait objects at open: a `Signer` and a `Verifier`
(identity area implements them: `identity.Chain` for the private log, `identity.NewBoardVerifierFor(id)` for boards;
core/core.go:139-144, core/boards.go:381-383).

### 1.2 Public API (keep these shapes; other areas call them)

```text
Event { device_id: String, seq: i64, hlc: Timestamp(u64), actor: String, kind: String,
        payload: RawJson(bytes), sig: Vec<u8> /* never in JSON */ }                      S:35-45
Event JSON (API /v1/events): {"deviceId","seq","hlc","actor","kind","payload"}           S:36-44
  hlc is a JSON integer (u64 digits; > 2^53, so JS readers lose precision — same as Go).
Cursor { hlc, device_id, seq }; zero == unset; Event::cursor()                          S:47-59
Vector = map device_id -> max seq                                                       S:61-62
Filter { kinds: [String] (exact, or prefix ending "."), device_id, after_hlc (strict),
         after: Cursor (strict, total order), after_seq (strict), limit (def 200, max 1000),
         desc, local (read local_events) }                                              S:64-77
trait Signer   { sign_row(msg) -> Result<Vec<u8>> }                                     S:79-82
trait Verifier { verify_row(&Event) -> Result<()>;   // ErrNoCert / ErrBadSig / ErrBoardActor wrapped
                 cut(device) -> Option<i64>;  cut_claims() -> Vec<CutClaim>;
                 observe(&Event);  reset() }                                            S:84-105
OpenOptions { signer?, verifier?, max_ahead (default 2 min), now (wall clock fn), board } S:107-132
CutClaim { device_id, seq, hash }                                                        S:163-169
Fork { stored: Event, other: Event, other_hash, reason }  JSON keys stored/other/otherHash/reason  S:187-193
BoardCounts { kind, actor, outside }  JSON keys kind/actor/outside                       B:143-148

open(path, device_id, clock)                      = open_with(.., OpenOptions::default())   S:202-211
close()                                            idempotent; closes subscriber channels   Q:381-396
device_id()                                                                                 S:217
append(actor, kind, payload: impl Serialize) -> Event                                       S:222
append_local(actor, kind, payload) -> Event                                                 S:229
insert(ev)                         = insert_batch([ev])                                     S:240, Q:549-552
insert_batch(evs) -> (applied: usize, Result)                                               S:245
hold_appends(seq)                                                                           S:253
reload_verifier()                  no-op without a Verifier                                 S:258-262
vector() / missing(v, limit) / list(filter) / refused(limit) / forks()                     S:265-282
subscribe() -> stream of Event; Event{seq:0} == "resync"                                    S:284-288
signing_bytes(&Event) -> Vec<u8>;  row_hash(&Event) -> hex String                           S:171-176, 290-311
is_board() / board_refused() / unresolved(&Event) -> Option<Err>                           B:150-174
kind_side(k) / is_local(k) / is_person_kind(k) / kind_board(k) / board_accepted(k) /
is_board_identity_kind(k) / board_actor_ok(actor)                                           K:179-186, B:100-139
schema_version() == 3 (len(migrations))                                                     Q:251
```

Errors callers match with `errors.Is` (make them enum variants; keep a `source`/context string):
`ErrLocalKind, ErrNotLocal, ErrNoCert, ErrBadSig, ErrUnsigned, ErrPastCut, ErrFork, ErrHLCOrder, ErrAhead,
ErrRestoring` (S:135-161); `ErrOwnDevice, ErrGap, ErrClosed` (Q:35-44); `ErrNotBoardKind, ErrBoardActor,
ErrOutsideBoard` (B:33-40). The Verifier's errors must be recognisable as `ErrNoCert`/`ErrBadSig`/`ErrBoardActor`
because the store and callers branch on them (Q:770-773; sync code holds rows on ErrNoCert, S:140-142).
Error message texts appear in logs and some reach agents (closure reasons, B:348-431); keep them verbatim where cheap.

### 1.3 Ids in this area

| Id | Format | Where minted |
|---|---|---|
| Row id | none; the key is `(device_id, seq)` | — |
| device_id | 16 random bytes, lowercase hex (32 chars) | internal/device/device.go:47-53 |
| boardId | 16 random bytes, lowercase hex; validated `len==32 && hex && lowercase` | core/boards.go:129-148 |
| task/msg ids | ULID-like, Crockford base32, 26 chars. **Two different generators**: coord/id.go:10-33 (48-bit ms + 80 random bits, no monotonic counter) and inbox/ids.go:46-100 (monotonic: same ms counts the random part up; first random byte masked `&0x7f`). Owned by coord/inbox areas, not this one. | |

The `ulid` crate (3.0.0) reproduces neither generator exactly; port each by hand (~30 lines) in its own area.

---

## 2. Exact data formats

### 2.1 Files

| Path | What |
|---|---|
| `<DataDir>/irori.db` (+ `-wal`, `-shm`) | private log (core/core.go:143) |
| `<DataDir>/irori.db.v0.3` | backup taken before the v0.4 migration (Q:63-66, 274-297) |
| `<DataDir>/irori.db.v0.3.tmp` | transient; removed before and after VACUUM INTO (Q:284-295) |
| `<DataDir>/boards/<boardId>.db` | board log (core/boards.go:382) |
| `<DataDir>/boards/<boardId>.secrets` | board secrets — identity area, not this one (core/boards.go:377) |

Reset deletes `irori.db, irori.db-wal, irori.db-shm, irori.db.v0.3` (core/reset.go:33). Parent dir created `0700` (Q:169).

### 2.2 Connection settings

DSN (Q:175): `path?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)&_txlock=immediate`

| Setting | Value | Rust (rusqlite) |
|---|---|---|
| `busy_timeout` | 5000 ms, per connection | `conn.busy_timeout(Duration::from_millis(5000))` on **every** connection |
| `journal_mode` | WAL (persistent in the file; test asserts `wal`, store_test.go:558-563) | `PRAGMA journal_mode=WAL` once per connection (cheap) |
| `synchronous` | NORMAL, per connection | `PRAGMA synchronous=NORMAL` on every connection |
| `_txlock=immediate` | every `BeginTx` is `BEGIN IMMEDIATE` (migrate, append, append_local, insert_batch, open-time apply_cuts) | `conn.transaction_with_behavior(TransactionBehavior::Immediate)` everywhere — **not** the default DEFERRED |
| pool | `SetMaxOpenConns(8)` (Q:180); writes serialized by an in-process mutex (`writeMu`, Q:145-148) | one write connection behind a mutex + a small read pool (or one `Mutex<Connection>`). Observable behaviour is the same. |
| foreign_keys | not set | leave off |

SQLite version: Go build uses modernc.org/sqlite v1.61.0 (go.mod), whose changelog's latest entry vendors SQLite 3.54.1.
rusqlite 0.41.1 `bundled` ships SQLite 3.54.0 (libsqlite3-sys 0.39.0 `sqlite3.h`). Schema needs ≥3.38 (`unixepoch()`)
and ≥3.27 (`VACUUM INTO`). File format is compatible both ways.

### 2.3 Schema (exact DDL; migrations are append-only, Q:68-133)

`schema_migrations` is created first in the migrate transaction (Q:305-310):
```sql
CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL DEFAULT (unixepoch())
)
```

**v1** (v0.1–v0.3), Q:82-92:
```sql
CREATE TABLE events (
    device_id TEXT    NOT NULL,
    seq       INTEGER NOT NULL,
    hlc       INTEGER NOT NULL,
    actor     TEXT    NOT NULL,
    kind      TEXT    NOT NULL,
    payload   BLOB,
    PRIMARY KEY (device_id, seq)
) WITHOUT ROWID;
CREATE INDEX events_hlc ON events (hlc);
CREATE INDEX events_kind ON events (kind, hlc);
```

**v2** (v0.4), Q:94-130:
```sql
ALTER TABLE events ADD COLUMN sig BLOB;
CREATE TABLE local_events (
    device_id TEXT NOT NULL, seq INTEGER NOT NULL, hlc INTEGER NOT NULL,
    actor TEXT NOT NULL, kind TEXT NOT NULL, payload BLOB, sig BLOB,
    PRIMARY KEY (device_id, seq)
) WITHOUT ROWID;
CREATE INDEX local_events_kind ON local_events (kind, hlc);
CREATE TABLE refused_events (
    device_id TEXT NOT NULL, seq INTEGER NOT NULL, hlc INTEGER NOT NULL,
    actor TEXT NOT NULL, kind TEXT NOT NULL, payload BLOB, sig BLOB,
    reason     TEXT    NOT NULL,
    refused_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (device_id, seq)
) WITHOUT ROWID;
CREATE TABLE forks (
    device_id TEXT NOT NULL, seq INTEGER NOT NULL,
    other_hash TEXT NOT NULL,
    hlc INTEGER NOT NULL, actor TEXT NOT NULL, kind TEXT NOT NULL, payload BLOB, sig BLOB,
    reason  TEXT    NOT NULL DEFAULT 'two rows',
    seen_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (device_id, seq, other_hash)
) WITHOUT ROWID;
```
(Formatting in the Go source differs in whitespace; use the exact Go string literals if you want `sqlite_master.sql` identical.
Not required for compatibility, but copying them verbatim costs nothing.)

**v3** (v0.4): Go code, `backSignAndMoveLocal` (Q:131-132, 337-379). See 3.2.

No other tables, indexes, triggers or views. `refused_at`/`seen_at`/`applied_at` are unix **seconds**.

### 2.4 Column encodings

| Column | Write | Read |
|---|---|---|
| `hlc` | `int64(ts)` — plain bit cast `ts as i64` (Q:24-32, 965-966) | `hlc::Timestamp(h as u64)` (Q:1164) |
| `payload` | bound as **BLOB** (`[]byte(ev.Payload)`, Q:966). Append: `json.Marshal` output, never empty. Insert: empty payload replaced by the 4 bytes `null` before verify and store (Q:540-542). | read as bytes; NULL -> empty/None (Q:1165); Go renders an empty payload as JSON `null` (only reachable for cut-hash fork rows). Text-typed values written by hand (tests insert `'null'` as TEXT, v04_test.go:357) must also read back as bytes — use `ValueRef::as_bytes_or_null` semantics that accept TEXT and BLOB. |
| `sig` | nil -> **SQL NULL**, non-empty -> BLOB (modernc binds nil `[]byte` as NULL and `[]byte{}` as zero-length BLOB; modernc sqlite conn.go:1454-1475). Unsigned store (no Signer) writes NULL. | NULL **or** zero-length -> "no sig" (Q:1166-1168) |
| text columns | TEXT | TEXT |

Write `sig` as `Option<&[u8]>` with `None` when empty. The open-time integrity check is `sig IS NULL` (Q:193): a zero-length
BLOB would slip past it, so never write one.

### 2.5 Signing bytes (byte-exact; S:290-311)

```
"irori/row/1"                         11 ASCII bytes, no length prefix, no NUL
|| u32be(len(device_id)) || device_id
|| u64be(seq as u64)
|| u64be(hlc)
|| u32be(len(actor))     || actor
|| u32be(len(kind))      || kind
|| sha256(payload)                    32 bytes, over the exact stored payload bytes
```
Lengths are UTF-8 byte lengths. `seq` is cast int64 -> uint64 (two's complement). Payload hash is over the bytes as stored
(after the empty->`null` substitution on Insert). The signature is ed25519 over these bytes (v04_test.go:30-34) — done
by the Signer, not the store.

**RowHash** (S:171-176): `hex_lower(sha256(signing_bytes(ev) || ev.sig))`. Used as `forks.other_hash` and compared to a
revoke's `cutHash` (CutClaim.hash).

### 2.6 HLC (H:1-61)

- `Timestamp = u64`; wall part `t >> 16` (unix ms), logical `t as u16` (H:57-61). Zero = unset.
- `now()`: `wall = (now_unix_ms as u64) << 16`; if `wall > last` then `last = wall` else `last += 1`; return `last` (H:30-40).
  Logical overflow simply carries into the wall bits. Mutex-protected.
- `observe(r)`: `last = max(last, r)` (H:43-49). `observe_bounded(r, max) = observe(min(r, max))` (H:53-55).
- `New(nil)` uses system time; tests inject a fixed clock (H:22-27). Keep an injectable `now: Fn() -> SystemTime`.
- `UnixMilli` of a pre-1970 time is negative and wraps; not reachable in practice.

### 2.7 Constants

| Name | Value | Ref |
|---|---|---|
| `MaxReplicatedHLC` | `1 << 62` | Q:46-48 |
| `DefaultMaxAhead` | 2 minutes | S:131-132 |
| `DefaultListLimit` / `MaxListLimit` | 200 / 1000 | Q:51-53 |
| `DefaultMissingLimit` / `MaxMissingLimit` | 1000 / 10000 | Q:54-56 |
| `subBuffer` | 256 (last slot reserved for resync) | Q:58-60 |
| `BackupSuffix` | `".v0.3"` | Q:66 |
| busy timeout | 5000 ms | Q:175 |
| `ForkTwoRows` / `ForkCutHash` | `"two rows"` / `"cut hash"` | S:179-185 |
| refused reasons | `"past cut"` / `"revoke past cut"` | Q:835-839 |
| Board actor prefixes | `"person:"`, `"agent:"`; plus literal `"irori"` | B:115-134 |
| board id in actor | non-empty, `len <= 128` bytes, none of `: / \ space \t \r \n` | B:136-139 |
| ledger `Keep` | 45 × 24 h | L ledger.go:21-22 |

### 2.8 Kinds registry (K:137-175, B:58-98) — copy these tables literally

Side `Local` (go in `local_events`, never replicate): `policy.set`, `member.joined`, `board.cardkey`, `board.pin`,
`board.forgotten` (K:145, 147, 175).

Side `Replicated`, with Board column:

| Board **Accepts** | Board **Refuses** |
|---|---|
| `task.created task.claimed task.assigned task.status task.joined task.left task.linked task.comment task.steps task.step task.watched task.parent blocker.closed blocker.option blocker.resolved lease.acquired lease.released lease.refused lease.stolen attach.offered board.created board.renamed board.member.added board.member.role board.member.removed board.member.left board.server member.profile member.device.added member.device.revoked member.key.rotated agent.card review.requested review.reviewer` (B:79-94) | `device.started agent.seen agent.status agent.gone person.device.added person.device.revoked person.key.rotated person.profile local.moved message.sent delivery.state wake.issued agent.alias agent.name turn.engaged turn.ended capture.turn capture.edit capture.summary capture.commitcmd pr.ledger repo.seen commit.seen commit.linked decision.recorded delivery.decided ask.posted ask.answered ask.resolved revive.requested node.addr device.color task.shared task.edited take.requested take.state task.pinned board.joined board.forgot receive.set autonomy.set board.sharedcopy board.cardof` (B:60-77, 96-97) |

Any other kind is `Unclassified`: the **private** log stores and replicates it (peers' newer kinds; K:126-128), a board log
refuses it (`BoardUnclassified`, B:45-48; `BoardAccepted` requires `BoardAccepts`, B:105).

Derived predicates:
- `is_local(k) = side(k) == Local` (K:183)
- `is_person_kind(k) = k.starts_with("person.")` (K:186)
- `board_accepted(k) = !is_local(k) && board(k) == Accepts` (B:105)
- `is_board_identity_kind(k) = board_accepted(k) && (k starts "board." || "member." || k == "agent.card")` (B:111-113)
- identity kinds of a store: private `is_person_kind`; board `is_board_identity_kind` (B:480-485)
- identity List prefixes: private `["person."]`; board `["board.", "member.", "agent.card"]` (exact, no dot) (B:488-493)
- revoke kinds (honoured past their writer's cut): private `[person.device.revoked]`; board
  `[member.device.revoked, board.member.removed, board.member.left]` (B:499-504)
- never held by `hold_appends`: `person.device.revoked, person.key.rotated, member.device.revoked, member.key.rotated` (Q:449-450)

Payload shapes noted in the registry (owned by the writing areas): K:5-120.
`local.moved` payload is exactly `{}` (K:29-32, Q:367).

---

## 3. Rules and algorithms

### 3.1 Open sequence (Q:162-248) — order matters

1. `device_id` empty -> error; clock missing -> error.
2. `create_dir_all(parent)` mode 0700.
3. Open pool with the settings in 2.2.
4. **backup_v03** (Q:274-297): read schema version (0 if `schema_migrations` table absent, Q:253-263). Only if version
   == 1 **and** `SELECT COUNT(*) FROM events` > 0: remove `<path>.v0.3.tmp` (ignore not-exist), run
   `VACUUM INTO ?` with the tmp path (captures WAL contents), then rename tmp -> `<path>.v0.3` (replacing an old one).
   On failure remove tmp and fail Open. Retaken on **every** start that finds schema v1 with rows (rollback case,
   v04_test.go:378-437). Fresh DB: no backup (v04_test.go:366-376).
5. **migrate** (Q:299-335): **one** IMMEDIATE transaction: create `schema_migrations`, read `MAX(version)`; if
   `have > 3` fail with "database schema vN is newer than this build (v3); update Irori"; run each missing migration's SQL
   then its Go step, then `INSERT INTO schema_migrations (version) VALUES (v)`; commit. Any failure rolls back
   everything including the `ALTER TABLE` (so a v1 DB opened without a Signer stays v1; v04_test.go:343-364).
6. With a Signer: `SELECT COUNT(*) FROM events WHERE device_id = ? AND sig IS NULL` for own id; >0 -> fail
   "store: %d of this device's rows are unsigned; the log was changed outside Irori" (Q:189-201).
7. Seed clock: `clock.observe(SELECT COALESCE(MAX(hlc),0) FROM events WHERE hlc <= 1<<62)` (Q:202-209). Only
   `events`, not `local_events`/`refused_events`.
8. Default `max_ahead` (if <= 0) to 2 min, `now` to system time (Q:210-215).
9. Board: build the closure index from every `events` row in `(hlc, device_id, seq)` order, paging 1000 by Cursor
   (B:460-476).
10. Verifier: `load_verifier` (3.7), then an IMMEDIATE tx running `apply_cuts` and commit (Q:227-246).

### 3.2 Migration v3: back-sign and move local rows (Q:337-379)

- Select own rows `WHERE device_id = self ORDER BY seq`. None -> done (no Signer needed; fresh DBs pass).
- Rows exist and no Signer -> error "this log needs the v0.4 migration, which signs rows; open it with a Signer".
- For each own row in seq order: if `is_local(kind)`: `local_seq += 1` (starting at 1), insert into `local_events`
  `(device_id, local_seq, hlc, actor, kind, payload)` with **sig NULL**; then rewrite the row in memory to
  kind=`local.moved`, payload=`{}`. Sign `signing_bytes(row)` (with the possibly rewritten kind/payload) and
  `UPDATE events SET kind=?, payload=?, sig=? WHERE device_id=? AND seq=?`.
- HLC, actor, seq unchanged. Other devices' rows untouched (v0.1-v0.3 had no transport).
- Idempotent through `schema_migrations`; a second open moves nothing (v04_test.go:329-340).

### 3.3 Append (Q:405-492)

1. `kind == ""` -> error. `is_local(kind)` -> `ErrLocalKind`.
2. `raw = json_marshal(payload)` (Go `nil` -> `null`).
3. Board: `check_shape(actor, kind, raw)` (3.8) **before** the write lock.
4. Take write lock; closed -> `ErrClosed`.
5. Board: `resolves(kind, raw)` under the lock; failure -> `counts.outside += 1`, return the error.
6. BEGIN IMMEDIATE. `last = COALESCE(MAX(seq),0) FROM events WHERE device_id = self`.
7. Hold: if `hold_seq > 0` and kind not in the never-held list: if `last < hold_seq` -> `ErrRestoring`
   ("(hold %d, have %d)"); else `hold_seq = 0`.
8. `ev = {self, last+1, clock.now(), actor, kind, raw}`. If `hlc > i64::MAX` -> error. (The clock has advanced even if
   the append later fails — same as Go.)
9. Signer present: `ev.sig = sign(signing_bytes(ev))`.
10. Insert into `events`.
11. Verifier present and identity kind: `verifier.observe(ev)`, then `apply_cuts(tx)` -> resync flag; on error
    `reset_verifier()` and fail.
12. Commit; on failure `reset_verifier()` if observed.
13. Board: `define(kind, raw)`. Notify subscribers with `ev`; if resync, `notify_resync()`.

### 3.4 AppendLocal (Q:494-522)

`!is_local(kind)` -> `ErrNotLocal`. Marshal, write lock, closed check, IMMEDIATE tx,
`seq = MAX(seq) FROM local_events WHERE device_id = self` + 1, `hlc = clock.now()`, insert into `local_events` with
**sig NULL** (never signed), commit. No hold check, no subscriber notification, no board rules.

### 3.5 Insert / InsertBatch (Q:549-759)

Batch: empty -> `(0, Ok)`. Write lock; closed -> `ErrClosed`. One IMMEDIATE tx. For each row in order call
`insert_one`:
- `Ok(stored=true)`: applied++, remember as fresh; if Verifier and identity kind: observe + `apply_cuts`.
- `Ok(stored=false)` (identical duplicate, or a seq already moved to refused_events): applied++.
- honoured revoke (internal sentinel): applied++, `apply_cuts`; not "fresh", not notified.
- any other error: stop; **commit the rows before it**; return `(applied, err)` (S:242-247; v04_test.go:148-165).
- `apply_cuts` error: `reset_verifier`, return `(0, err)` and the tx is dropped (rolled back).
Commit (failure -> `reset_verifier` if anything observed, `(0, err)`). Then: board `define` for **all** fresh rows first,
then for each fresh row: clock `observe_bounded(ev.hlc, ahead_bound())` with a Verifier, `observe(ev.hlc)` without, and
notify. Then `notify_resync` if any cut moved rows. Only committed rows move the clock (Q:628-633; store_test.go:199-222).

`ahead_bound() = (((now() + max_ahead) as unix ms) << 16) | 0xffff` (Q:642-645).

**insert_one, in this exact order** (order decides which error a bad row gets):

1. `check_row` (Q:524-547), first match wins:
   1. `device_id == self && no Verifier` -> `ErrOwnDevice`
   2. `device_id == ""` -> error
   3. `seq < 1` -> error
   4. `kind == ""` -> error
   5. `hlc > 1<<62` -> error (covers everything ≥ 2^63 too)
   6. `is_local(kind)` -> `ErrLocalKind`
   7. empty payload -> set to `null`; then payload must be valid JSON (Go `json.Valid`) else error.
2. Board: `check_shape(actor, kind, payload)`; error wrapped `"<dev>/<seq>: <err>"`.
3. `last = MAX(seq)` for `ev.device_id` in `events`.
4. `seq <= last`: load held row from `events`. Missing -> `Ok(false)`. `same_row(held, ev)` (hlc, actor, kind,
   payload bytes, sig bytes all equal; Q:826-829) -> `Ok(false)`. Otherwise: with a Verifier, `verify(ev)` first
   (failure returns that error, nothing recorded); then `record_fork` (INSERT OR IGNORE into forks with
   `other_hash = row_hash(ev)`, all columns, reason `two rows`, Q:848-853) and return `ErrFork`. The fork row is
   committed by the batch commit even though the batch stops.
5. Verifier and `cut(dev) = Some(cut)` and `seq > cut` (checked **before** the gap check):
   - if kind is a revoke kind **and** sig non-empty: `verifier.verify_row(ev)` (error -> return it); if `last > 0`
     read `hlc` of `(dev, last)` and require `ev.hlc > held` else `ErrHLCOrder`; require `ev.hlc <= ahead_bound()`
     else `ErrAhead`; `record_refused(ev, "revoke past cut")` (INSERT OR IGNORE); `verifier.observe(ev)`; return
     honoured-revoke sentinel.
   - else `ErrPastCut` "(cut %d)".
6. `seq > last + 1` -> `ErrGap`.
7. Verifier: `verify(ev)`: empty sig -> `ErrUnsigned`; else `verify_row`; in a board store an `ErrBoardActor` result
   increments `counts.actor` (Q:761-775).
8. `last > 0`: `ev.hlc` (as i64) must be `> hlc of (dev, last)` else `ErrHLCOrder` (applies with or without Verifier;
   v04_test.go:132-146).
9. Verifier and `ev.hlc > ahead_bound()` -> `ErrAhead` (not stored; retry later).
10. Insert into `events`.
11. Verifier: `check_cut_claims(only = ev)`.
12. `Ok(true)`.

With a Verifier, this device's own rows **are** accepted back (restore from a peer), verified like any other (S:115-117).

### 3.6 Cuts, refused rows, forks (Q:777-903)

`apply_cuts(tx) -> moved` (Q:855-903): for each `(device_id, MAX(seq))` in `events` except self: if
`cut(dev) = Some(c)` and `max > c`:
`INSERT OR IGNORE INTO refused_events (...cols..., reason) SELECT ..., 'past cut' FROM events WHERE device_id=? AND seq>?`
then `DELETE FROM events WHERE device_id=? AND seq>?`; moved = true. Finally `check_cut_claims(None)`.

`check_cut_claims(only)` (Q:781-813): for each claim from the Verifier: with `only`, consider only the claim at exactly
that `(dev, seq)` and compare against `only`; otherwise load the held row from `events`, else `refused_events`
(`held_row`, Q:815-824), skipping claims whose row is not held. If `row_hash(held) != claim.hash`:
`INSERT OR IGNORE INTO forks (device_id, seq, other_hash, hlc, actor, kind, reason) VALUES (?, ?, claim.hash, 0, '', '', 'cut hash')`
(payload and sig NULL).

`forks()` (Q:1108-1144): `SELECT ... FROM forks ORDER BY device_id, seq, seen_at`; `other` built from the fork row
(payload NULL/empty -> None, sig likewise); `stored` = the row in `events` at that `(dev, seq)` or a zero Event.

`refused(limit)` (Q:1100-1106): `ORDER BY hlc, device_id, seq LIMIT min(limit or 200, 1000)`.

`vector()` (Q:970-1000): `SELECT device_id, MAX(seq) FROM events GROUP BY device_id`; with a Verifier, cap each
non-self device at its cut. `local_events` never counted.

`missing(v, limit)` (Q:1002-1033): limit <=0 -> 1000, cap 10000. Devices with `have[dev] > v[dev]` (absent = 0),
**sorted by device_id (byte order)**; per device `WHERE device_id=? AND seq>v[dev] AND seq<=have[dev] ORDER BY seq LIMIT remaining`
until the limit fills. Uses the capped vector, so rows past a cut are never offered.

### 3.7 Verifier loading (Q:905-955)

`load_verifier`: page `list({kinds: identity_prefixes, after: cursor, limit: 1000})` and `observe` each; stop when a page
has < 1000 rows. Then observe every `refused_events` row whose kind is a revoke kind, `ORDER BY hlc, device_id, seq`.
`reset_verifier`: `verifier.reset()` then `load_verifier` **ignoring cancellation** (Q:951-954) and ignoring errors.
`reload_verifier()` public = `reset_verifier` (S:255-262).

### 3.8 Board rules (B:253-458)

`check_shape(actor, kind, payload)` on Append and Insert:
1. `!board_accepted(kind)` -> `counts.kind++`, `ErrNotBoardKind`.
2. `!board_actor_ok(actor)` -> `counts.actor++`, `ErrBoardActor` (B:121-134: `irori`, or `person:`/`agent:` + valid id).
3. `kind == agent.card`: unmarshal payload into the board row struct; failure **or** `actor != "agent:" + cardId` ->
   `counts.actor++`, `ErrBoardActor`.

`define(kind, payload)` after commit (B:282-326); unmarshal failure -> no-op. Index sets: tasks (`task.created.taskId`),
comments (`task.comment.commentId`), blockers (`task.status`: `taskId + "/" + blocker.id`), steps (`task.steps`: each
`taskId + "/" + steps[].id`), cards (`agent.card.cardId`, even empty), hashes (`attach.offered.sha256`), leases
(`lease.acquired.resource`), persons (`board.created.creator.personId` if valid board id; `board.member.added.personId`
if valid). Sets only grow (nothing is removed on release/removal).

`resolves(kind, payload)` — Append only, and `unresolved(ev)` for the fold (B:328-433). Kinds starting `member.` or
`board.` always pass. Else unmarshal; failure -> `ErrOutsideBoard` "payload is not an object". Checks, first failure wins
(message format `"<ErrOutsideBoard>: <kind>: <reason>"`):
taskId held unless kind is task.created; parent held; blockerId as `taskId/blockerId`; stepId as `taskId/stepId`;
commentId held unless kind is task.comment; any non-empty msgId refused; then build key list
by,to,who,holder,heldBy, each `stolen[].holder` (and each `stolen[].resource` must be a held lease — checked while
building), each comma-split+trimmed `stolenFrom` entry, `conflict`; `outcome.task` held; then each key via `key()`;
then every `attachments[].sha256` and `blocker.attachments[].sha256` must be offered; then `blockedBy` and `blocker.on`:
kind `task` -> tasks, `lease` -> leases, `agent`/`person` -> `key()`; finally `lease.released`/`lease.stolen`
`resource` held; `lease.refused` `held` held if non-empty.

`key(k, self_card)` (B:435-458): `""` or `irori` ok; `person:<id>` needs valid id and `id ∈ persons`; `agent:<id>` needs
valid id and (`id == self_card` (the card being written by agent.card) or `id ∈ cards`); anything else ->
"is a private key; a board names only person: and agent: keys".

Board row JSON fields read (B:197-251): `taskId parent commentId blockerId stepId msgId cardId sha256 resource held by
to who holder heldBy blockedBy{kind,ref} blocker{id,on{kind,ref},attachments[{sha256}]} attachments[{sha256}]
steps[{id}] stolen[{resource,holder}] stolenFrom conflict outcome{task} personId creator{personId}`.

### 3.9 Subscribe (Q:1189-1246)

Contract to reproduce with any primitive (e.g. per-subscriber `Mutex<VecDeque<Event>>` + `Notify`, or
`tokio::sync::mpsc::channel(256)` using `sender.capacity()` to derive length):
- Capacity 256. After commit, under the write lock and a subscriber lock, for each subscriber:
  `len < 255` -> push row, `lagging = false`; else if `!lagging` -> push zero Event (seq 0), `lagging = true`;
  else drop the row. Writers never block.
- `notify_resync`: if `!lagging && len < 256`, push zero Event, `lagging = true`.
- Delivery order = commit order (writes serialized). Rows from append and fresh inserted rows are delivered;
  duplicates, honoured revokes, local rows are not.
- Stream ends when the subscriber's context ends or the store closes; subscribe after close returns an already-closed
  stream. Close closes all streams; later append/append_local/insert return `ErrClosed`.
- Expected under a stalled reader (store_test.go:471-503): 255 rows then one resync; after draining, the next row arrives.

### 3.10 List (Q:1035-1098)

WHERE clauses ANDed: kinds OR-group — prefix `k` ending in `.` -> `(kind >= k AND kind < k[..len-1] + "/")` (relies on
`/` = `.`+1 under BINARY collation), else `kind = k`; `device_id = ?`; `seq > after_seq` (if >0); `hlc > after_hlc`
(if != 0; if > i64::MAX return empty); cursor `(hlc > h OR (hlc = h AND (device_id > d OR (device_id = d AND seq > s))))`
(if non-zero; > i64::MAX -> empty). Order `hlc, device_id, seq` (or all DESC). Limit `<=0 -> 200`, cap 1000. Table
`local_events` when `local`. Note the cursor is "strictly after" even with `desc` (Go does not flip it).

### 3.11 Hold appends (Q:398-403, 446-455)

`hold_appends(seq)` sets `hold_seq` under the write lock. Cleared by an append that finds `last >= hold_seq`, or by
`hold_appends(0)`. Never-held kinds bypass without clearing.

---

## 4. Platform-specific code

None in this area's Go code. Points to carry over:
- `os.MkdirAll(dir, 0o700)`: mode is meaningful on macOS, ignored on Windows. Rust: `DirBuilder::new().recursive(true)`
  + `std::os::unix::fs::DirBuilderExt::mode(0o700)` under `#[cfg(unix)]`.
- `os.Rename` over an existing file (backup) replaces it on both OSes; `std::fs::rename` does too (MoveFileExW with
  REPLACE_EXISTING on Windows). A Go/Rust process holding `irori.db.v0.3` open on Windows would make it fail — same as Go.
- WAL `-shm` files: two processes (Go and Rust) must never open the same DB at once during testing on Windows; the
  busy timeout only covers lock waits, not mismatched builds.
- Data dir location is the paths area (`internal/paths`), not this one.

---

## 5. Rust crates (crates.io, checked 2026-10-29)

| Crate | Version | Use |
|---|---|---|
| `rusqlite` | 0.41.1, features `bundled` | SQLite (bundles 3.54.0). Use `TransactionBehavior::Immediate`. |
| (pool) | — | `r2d2_sqlite` 0.35.0 pins `rusqlite ^0.40`, incompatible with 0.41: hand-roll one write `Mutex<Connection>` + a few read connections, each initialised with busy_timeout/synchronous. |
| `sha2` | 0.11.1 | sha256 for signing bytes and RowHash |
| `hex` | 0.4.3 | RowHash hex |
| `serde` / `serde_json` | 1.0.229 / 1.0.154 | payload (de)serialization; enable `raw_value` for `Box<RawValue>` payload passthrough, `preserve_order` or BTreeMap for deterministic output, `unbounded_depth` (see 7.4) |
| `thiserror` | 2.0.21 | error enum |
| `tokio` | 1.54.1 (`sync`) | subscriber streams / Notify |
| `parking_lot` | 0.13.0 | mutexes (optional; std is fine) |
| `regex` | 1.13.1 | ledger patterns — **with Unicode off**, see 7.6 |
| `ed25519-dalek` | 3.0.0 | tests only here (identity area owns real signing). 3.x is a new major; check API vs 2.x examples. |
| `tempfile` | 3.27.0 | tests |

Not recommended: `ulid` 3.0.0 (matches neither Go id generator, 1.3).

---

## 6. Tests to port (what each proves)

### hlc (internal/hlc/hlc_test.go)
| Test | Proves |
|---|---|
| ObserveBoundedCapsTheJump :8 | far-future observe capped at bound; next `now` = bound+1; lower observe ignored; then bound+2 |
| NowIncreasesWhenWallStalls :22 | fixed wall: second `now` = first+1, logical 1, wall decodes exactly |
| ObserveMovesPastRemote :34 | `now` after observe exceeds the remote |

### store (store_test.go)
| Test | Proves |
|---|---|
| ConcurrentAppendsAreGapless :46 | 8×200 concurrent appends: seqs 1..1600 gapless, HLC strictly increasing with seq |
| ReopenContinuesSeq :85 | seq continues after reopen; payload bytes preserved (`{"name":"one"}` — Go compact output) |
| InsertIsIdempotentAndRefusesOwnID :109 | 3× same insert = 1 row, round-trips exactly; own id -> ErrOwnDevice (no Verifier); seq gap -> ErrGap |
| InsertObservesClock :135 | inserted far-future row pushes own next HLC past it |
| HLCRoundTripsAsInt64 :151 | HLC 1, now, year 3000, 1<<62 store/read exactly; append after the ceiling still works and is > ceiling |
| InsertRefusesHLCPastCeiling :182 | 1<<62+1, MaxInt64, MaxInt64+1, MaxUint64 refused; clock unmoved |
| RefusedInsertDoesNotMoveClock :199 | gap and fork rows do not move the clock; different row at held seq -> ErrFork |
| OpenSeedsClockFromHeldRows :224 | reopen seeds clock from MAX(hlc) |
| InsertRefusesInvalidJSONPayload :243 | invalid JSON refused; list output marshals |
| SubscribeGoroutineEndsOnClose :264 | 100 subscriptions leak nothing after close (Rust: tasks/streams dropped) |
| ReplicationBothWays :304 | 3 stores, Missing pages of 7, transitive sync; equal row sets and vectors; final Missing empty |
| MissingRespectsVectorAndLimit :338 | starts after vector, honours limit |
| ListFilters :352 | prefix vs exact (`agent` exact matches nothing; `agentx.` not under `agent.`), device, after, limit, desc, combined |
| ListLimitBounds :400 | 0/-1 -> 200, 5000 -> 1000 |
| SubscribeOrder :419 | concurrent appends delivered in seq order; inserted row delivered; no spurious resync |
| SubscribeResyncOnSlowReader :471 | 255 delivered + 1 resync with 296 appends unread; writers never block; flow resumes |
| SubscribeClosesOnCancelAndClose :505 | stream ends on cancel and on close; subscribe after close closed; append/insert after close -> ErrClosed |
| OpenCreatesParentDir :531 | nested parent dirs created |
| MigrationsRecorded :540 | MAX(version)==3; DB with version 4 refused; journal_mode is `wal` |

### store v0.4 (v04_test.go)
| Test | Proves |
|---|---|
| AppendSignsEveryRow :36 | ed25519 over signing bytes verifies; changing actor breaks it; `sig` absent from JSON |
| LocalKindsStayInLocalEvents :64 | Append(local) -> ErrLocalKind; AppendLocal(replicated) -> ErrNotLocal; local seqs 1..n; vector/missing ignore local; List{local} works; Insert(local kind) -> ErrLocalKind |
| InsertRecordsForks :103 | identical copy ok; different row -> ErrFork twice but one fork row (INSERT OR IGNORE on hash); stored row unchanged |
| InsertRefusesHLCThatDoesNotAdvance :132 | equal or lower HLC than previous -> ErrHLCOrder; +1 ok |
| InsertBatchCommitsUpToTheFirstRefusal :148 | applied=3 (duplicate counts), ErrGap, rows before kept |
| CursorAtPageBoundaryWithEqualHLCs :169 | cursor paging returns all ties once; AfterHLC skips (documents the bug) |
| HoldAppendsUntilOwnRowsReturn :202 | hold -> ErrRestoring; hold(0) clears |
| OneTimeMigrationOfAV03Log :259 | **key compat test**: build a v1 DB (v03DB helper :215-248, exact rows), migrate: backup is v1 with all rows; own rows signed; local kinds -> `local.moved` `{}` and moved to local_events in order with same HLC/payload; appends continue at seq 8; second open idempotent and backup file byte-identical |
| MigrationNeedsASignerAndRefusesUnsignedOwnRows :343 | v1 DB without Signer fails and stays v1; later unsigned own row fails open with "unsigned" |
| FreshDatabaseTakesNoBackup :366 | no `.v0.3` for a new DB |
| BackupRetakenAfterARollback :381 | backup retaken from the rolled-back-and-used v1 DB; no `.tmp` left |

### store boards (board_test.go) — uses a `devVerifier` fake (board_test.go:17-50)
| Test | Proves |
|---|---|
| BoardRefusesKinds :121 | every Refuses kind + an unknown kind refused by Append and Insert and counted; private log accepts them |
| BoardActors :171 | actor table (irori/person:p1/agent:c1 ok; user, user:*, api:*, agent:dev/s, agent:, person:, person:p1/x, "" refused) for Append and Insert; agent.card must be written by its own card |
| BoardClosure :216 | 15 refused closure cases with reasons; outside count; resolvable rows accepted; Insert stores unresolved rows and `unresolved` clears as references arrive; index rebuilt at open; private log never checks |
| BoardVerifierHook :320 | board Verifier observes member.*/board.* rows on write and at open; uncertified device -> ErrNoCert |

### kinds (kinds_test.go)
`EveryKindIsClassified` :23 parses Go source (go/ast) for `Kind*` string constants in store, inbox, coord, capture,
core, identity and requires each in the registry; every Replicated kind has a Board verdict; every Board entry has a
Side; spot checks of local/replicated kinds. Cannot be ported literally. Rust equivalent: define every kind as a
`pub const` in one `kinds` module (or an enum with `as_str`), build both registries from a single `static` table,
and test (a) every const in the module is in the table (macro-generated list), (b) the three consistency rules above,
(c) the table equals the Go table in 2.8 (golden list), (d) `len >= 40`.

### ledger (internal/ledger/*_test.go)
Table-driven; copy tables verbatim: `TestFromBash` bash_test.go:8 (gitOperation created, gh pr create URL scraping,
review targets/-R parsing, quoting), `TestKey/RepoKey/RepoFromKey` keys_test.go:5-52, `TestExtractRefs` refs_test.go:18
(URLs, name#N, aliases, bare #N, Discord `<#…>`/`<@&…>` mentions ignored, code spans stripped),
`TestExtractRefsKeepsRepoCase` :105, `TestParseTarget` :113 (37 cases incl. dot-only segments, look-alike hosts),
store_test.go: OwnersRank :34, OwnersTieBreakOnSession :50, Add :62 (identity merge rules), AddKeepsEarliestAndFirstURL
:153, Keep :171 (exactly 45 days kept, 1 ns older dropped, future kept, entry not deleted), ForRepo :219 (case-insensitive,
newest number first), ForSession :257, ViewsAreCopies :280, Concurrent :292.

---

## 7. Hard parts and compatibility traps

### 7.1 Payload bytes are opaque
Signatures, `same_row`, RowHash and fork detection all use the **exact stored payload bytes** (S:297-298, Q:826-829).
Never parse-and-reserialize a payload anywhere in the store or the sync path. Store `Vec<u8>`/`Box<RawValue>`; return it
unchanged. A Rust re-encode of a Go row (key order, `\u003c` escapes, number formatting) changes the hash -> false
`ErrFork` / `ErrBadSig` against Go peers.

### 7.2 New payloads differ from Go's encoder (allowed, but know it)
Go `json.Marshal` (Q:413): struct fields in declaration order, map keys sorted, `<` `>` `&` escaped as `\u003c \u003e
\u0026`, U+2028/U+2029 escaped, invalid UTF-8 replaced by U+FFFD, `[]byte` as base64, nil slice/map as `null`,
`time.Time` as RFC3339Nano. serde_json does none of the escaping. Signature compatibility is unaffected (each row signs its
own bytes), but: tests that compare payload strings, and any code that dedupes by payload text, must not assume Go bytes.
Use BTreeMap/structs for deterministic order. Field shapes (`omitempty`, null vs missing) belong to each writing area's
spec and do matter to Go readers.

### 7.3 NULL vs empty
`sig`: write NULL when absent, never a zero-length BLOB (2.4). Read NULL or empty as unsigned. `payload`: Insert turns
empty into `null` **before** verifying, so a peer row signed over an empty payload fails verification in Go too — keep it
that way. Fork rows from cut-hash claims have NULL payload/sig, hlc 0, actor/kind `''` (Q:806-808).

### 7.4 JSON validity must match Go's `json.Valid` (Q:543)
A row Go accepts and Rust refuses (or the reverse) stalls replication for that device at that seq. Differences:
- Go accepts **invalid UTF-8** inside strings; `serde_json` rejects it.
- `serde_json` has a 128-level recursion limit by default; Go's limit is 10000. Enable `unbounded_depth` and use
  `serde_stacker` or a non-recursive validator.
Recommendation: port Go's JSON scanner as a small non-allocating validator (RFC 8259 grammar, bytes ≥0x80 accepted in
strings, control chars <0x20 rejected, depth ≤ 10000).

### 7.5 Board payload decoding must mimic Go `json.Unmarshal` into a struct (B:197-251, 267-268, 284-287, 343-345)
- Keys match struct tags **case-insensitively** (exact match preferred, else Unicode simple fold) — `"TASKID"` sets
  `taskId`. Duplicate keys: last wins. Unknown keys ignored.
- `null` for a field leaves it empty, no error. Top-level `null` -> success with all fields empty (so `resolves` passes,
  `define` no-ops).
- A **type mismatch anywhere** (e.g. `"steps":"x"`, `"by":5`, top-level array/string/number) makes Unmarshal return an
  error: `define` then records nothing, `resolves` returns "payload is not an object", `agent.card` shape check refuses.
  Go still partially fills fields on type errors, but this code discards on any error, so "any error = whole row fails"
  is the rule.
- Do not use `#[derive(Deserialize)]` naively (case-sensitive; errors on duplicate fields). Parse to `serde_json::Value`
  (map keeps last duplicate), then extract with a case-insensitive key lookup and strict type checks.

### 7.6 HLC as i64
Plain bit cast both ways. Insert refuses `> 1<<62` (Q:535); Append refuses `> i64::MAX` (Q:457). Clock seeding ignores rows
above `1<<62` (Q:205) — own rows can exceed it after observing a ceiling row (store_test.go:172-179); keep the Go quirk.
Comparisons in SQL are on the signed value; all valid values are positive so order is preserved.

### 7.7 Check order is behaviour
Which error a bad row gets (fork vs past-cut vs gap vs unsigned vs HLC order vs ahead) drives the sync area's reaction
(hold, stop stream, record fork). Follow 3.5's order exactly, including "cut before gap" and "verify before recording a
fork".

### 7.8 Transactions
Every write transaction is `BEGIN IMMEDIATE` (Q:173-175). Batch stops at the first refusal but still **commits** the
earlier rows and any fork row written for the refused one. `apply_cuts` failure aborts the whole batch (return 0).
Verifier state is in-memory and observed before commit; any failed commit must `reset()` and reload it from disk
without honouring cancellation.

### 7.9 Migration and schema versioning
- Rust must write `schema_migrations` versions 1..3 exactly and stop at 3 while Go builds may still open the file. Adding
  a Rust-only table means version 4, after which Go refuses the DB ("newer than this build"). Keep Rust-only state out of
  `irori.db` until cutover, or accept that v4 is the point of no return.
- The whole migrate (DDL + back-sign) is one transaction; SQLite allows `ALTER TABLE`/`CREATE` inside it.
- `VACUUM INTO` cannot run inside a transaction; run it on a plain connection before migrate. Parameter binding for
  the filename works (`VACUUM INTO ?`).
- Real data: Stephen's PC and Mac logs are expected to be at v3 (v0.4+ shipped); not verified against either machine's file. The v1 path matters only for very old installs
  and for the `OneTimeMigrationOfAV03Log` test.

### 7.10 Subscriber semantics
Go relies on `len(ch)` of a buffered channel. tokio `mpsc` lacks a direct `len()`; use `max_capacity() - capacity()`,
or a custom queue. Reproduce the 255+resync rule (3.9) exactly; folds in other areas treat seq 0 as "re-list".

### 7.11 List prefix range
`kind < prefix-without-dot + "/"` assumes BINARY collation (default). Do not add `COLLATE NOCASE` anywhere.

### 7.12 Ledger regexes (L refs.go:9-22, bash.go:10-18)
Go RE2 `\w \d \s \b` are ASCII; Rust `regex` makes them Unicode by default. Write every pattern with `(?-u:...)` or
explicit classes: `\w` -> `[0-9A-Za-z_]`, `\d` -> `[0-9]`, `\b` -> `(?-u:\b)`, `\s` -> `[\t\n\f\r ]` (Go's `\s` excludes
`\v`; Rust's ASCII `\s` includes it). `prevBlocks` (refs.go:33-39) checks one preceding byte and assumes ASCII word
bytes; keep it byte-based. Go `strings.ToLower`/`EqualFold` are Unicode-aware: use `to_lowercase()` (note Rust's
final-sigma rule differs from Go for `Σ`; irrelevant for GitHub names) and a Unicode simple-fold compare for `ForRepo`
(`unicase` crate or `to_lowercase` both sides). `strconv.Atoi` -> parse as `i64`; overflow = not a PR.

### 7.13 Things that look like bugs but must be kept
- AfterHLC paging skips ties (v04_test.go:196-199) — kept; the Cursor exists to fix it.
- Desc listing with a Cursor still means "strictly after" in ascending order.
- `define` never removes ids (released leases, removed members stay resolvable).
- `resolves` checks `stolen[].resource` before the key checks (error message order).
- `apply_cuts` never moves this device's own rows (Q:882-884).

---

## 8. Ledger (PR authorship) in brief

Pure, `RwLock`-guarded `Vec<Entry>` + index `key\0session\0role -> position` (L store.go:10-24).
- `Entry {key, repo, number, url (omitempty), session, role, source, at}` JSON keys as L ledger.go:25-34.
- `Role`: `author`(0) < `touched`(1) < `reviewer`(2) (L ledger.go:11-19, store.go:11).
- `key(repo,n) = "pr:" + lower(trim(repo)) + "#" + n`; `repo_key = "remote:github.com/" + lower(trim(repo))`;
  `repo_from_key` returns `owner/name` only when exactly two non-empty segments (L keys.go:10-30).
- `add`: ignore empty key/session, zero `at`, unknown role; new identity appended; existing keeps earliest `at` and
  first non-empty URL; returns changed (L store.go:26-53).
- live = `!(at < now - 45d)` (inclusive) (L store.go:57-59). Owners: role rank, earliest at, session. ForRepo: repo
  equal-fold (or all), number desc, key asc, then owner order. ForSession: at desc, key asc, role rank. All stable sorts,
  return copies (L store.go:74-119).
- `extract_refs` (L refs.go:59-123): strip ```` ```…``` ```` then `` `…` `` (replace with a space), scan byte by byte trying URL
  (only at `g`), name#N (letter start, previous byte not word or `/<#-`; `pr`/`pull` -> default repo; aliases by lowercased
  name; bare name needs an owner), bare `#N` 2-5 digits (previous byte not word or `/<#&@`; needs default repo); dedupe
  by key; repo segments may not be all dots.
- `parse_target` (L refs.go:125-142), `from_bash` + `shell_words` + `review_target` + `repo_arg` (L bash.go:30-246):
  port line for line; test tables cover the edge cases.
