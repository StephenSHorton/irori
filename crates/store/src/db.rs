//! The SQLite store (store.md §2.2-2.4, §3; I:internal/store/sqlite.go).
//!
//! One write connection behind a mutex serializes every write and its
//! notification (Go `writeMu`), so subscribers see rows in commit order and
//! own seqs follow own HLCs. Reads use a small pool of other connections.
//! Every connection gets `busy_timeout(5000)`, `journal_mode=WAL` and
//! `synchronous=NORMAL`; every write transaction is `BEGIN IMMEDIATE`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime};

use gocompat::sub::{Queue, TryRecv};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::board::{BoardCounts, BoardRules};
use crate::error::{Error, ErrorKind, Result};
use crate::kinds::{self, is_local};
use crate::{
    BACKUP_SUFFIX, CutClaim, DEFAULT_LIST_LIMIT, DEFAULT_MAX_AHEAD, DEFAULT_MISSING_LIMIT, Event,
    FORK_CUT_HASH, FORK_TWO_ROWS, Filter, Fork, MAX_LIST_LIMIT, MAX_MISSING_LIMIT,
    MAX_REPLICATED_HLC, OpenOptions, SUB_BUFFER, Signer, Timestamp, Vector, Verifier, WallFn,
    row_hash, signing_bytes,
};

/// `schema_migrations`, created first in the migrate transaction.
const SCHEMA_MIGRATIONS: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (\n\t\tversion    INTEGER PRIMARY KEY,\n\t\tapplied_at INTEGER NOT NULL DEFAULT (unixepoch())\n\t)";

/// v1 (v0.1-v0.3): the log. Go's literal, byte for byte, so `sqlite_master`
/// reads the same whichever build created the file.
pub(crate) const MIGRATION_V1: &str = concat!(
    "CREATE TABLE events (\n",
    "\t\tdevice_id TEXT    NOT NULL,\n",
    "\t\tseq       INTEGER NOT NULL,\n",
    "\t\thlc       INTEGER NOT NULL,\n",
    "\t\tactor     TEXT    NOT NULL,\n",
    "\t\tkind      TEXT    NOT NULL,\n",
    "\t\tpayload   BLOB,\n",
    "\t\tPRIMARY KEY (device_id, seq)\n",
    "\t) WITHOUT ROWID;\n",
    "\tCREATE INDEX events_hlc ON events (hlc);\n",
    "\tCREATE INDEX events_kind ON events (kind, hlc);",
);

/// v2 (v0.4): signatures, device-local rows, refused rows and forks.
pub(crate) const MIGRATION_V2: &str = concat!(
    "ALTER TABLE events ADD COLUMN sig BLOB;\n",
    "\tCREATE TABLE local_events (\n",
    "\t\tdevice_id TEXT    NOT NULL,\n",
    "\t\tseq       INTEGER NOT NULL,\n",
    "\t\thlc       INTEGER NOT NULL,\n",
    "\t\tactor     TEXT    NOT NULL,\n",
    "\t\tkind      TEXT    NOT NULL,\n",
    "\t\tpayload   BLOB,\n",
    "\t\tsig       BLOB,\n",
    "\t\tPRIMARY KEY (device_id, seq)\n",
    "\t) WITHOUT ROWID;\n",
    "\tCREATE INDEX local_events_kind ON local_events (kind, hlc);\n",
    "\tCREATE TABLE refused_events (\n",
    "\t\tdevice_id  TEXT    NOT NULL,\n",
    "\t\tseq        INTEGER NOT NULL,\n",
    "\t\thlc        INTEGER NOT NULL,\n",
    "\t\tactor      TEXT    NOT NULL,\n",
    "\t\tkind       TEXT    NOT NULL,\n",
    "\t\tpayload    BLOB,\n",
    "\t\tsig        BLOB,\n",
    "\t\treason     TEXT    NOT NULL,\n",
    "\t\trefused_at INTEGER NOT NULL DEFAULT (unixepoch()),\n",
    "\t\tPRIMARY KEY (device_id, seq)\n",
    "\t) WITHOUT ROWID;\n",
    "\tCREATE TABLE forks (\n",
    "\t\tdevice_id  TEXT    NOT NULL,\n",
    "\t\tseq        INTEGER NOT NULL,\n",
    "\t\tother_hash TEXT    NOT NULL,\n",
    "\t\thlc        INTEGER NOT NULL,\n",
    "\t\tactor      TEXT    NOT NULL,\n",
    "\t\tkind       TEXT    NOT NULL,\n",
    "\t\tpayload    BLOB,\n",
    "\t\tsig        BLOB,\n",
    "\t\treason     TEXT    NOT NULL DEFAULT 'two rows',\n",
    "\t\tseen_at    INTEGER NOT NULL DEFAULT (unixepoch()),\n",
    "\t\tPRIMARY KEY (device_id, seq, other_hash)\n",
    "\t) WITHOUT ROWID;",
);

/// The schema version this build writes (v3: v1, v2 and the one-time
/// migration). Go owns the schema until cutover (D8).
pub fn schema_version() -> i64 {
    3
}

const COLS: &str = "device_id, seq, hlc, actor, kind, payload, sig";
const REFUSED_PAST_CUT: &str = "past cut";
const REFUSED_REVOKE: &str = "revoke past cut";
const IDLE_READERS: usize = 4;

/// The event log. Safe for concurrent use; `close` (or drop) ends every
/// subscription.
pub struct Store {
    inner: Arc<Inner>,
}

struct Writer {
    conn: Option<Connection>,
    /// hold_appends: append waits until own rows reach this seq.
    hold_seq: i64,
}

struct Sub {
    q: Arc<Queue<Event>>,
    /// A resync signal is queued and not yet followed by a delivery.
    lagging: bool,
}

struct Subs {
    closed: bool,
    next: u64,
    list: BTreeMap<u64, Sub>,
}

struct Inner {
    path: PathBuf,
    device_id: String,
    clock: Arc<hlc::Clock>,
    signer: Option<Arc<dyn Signer>>,
    verifier: Option<Arc<dyn Verifier>>,
    board: Option<BoardRules>,
    max_ahead: Duration,
    now: WallFn,
    writer: Mutex<Writer>,
    readers: Mutex<Vec<Connection>>,
    verify_mu: Mutex<()>,
    subs: Mutex<Subs>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn closed() -> Error {
    Error::sentinel(ErrorKind::Closed)
}

/// A connection with Go's DSN settings (store.md §2.2).
fn connect(path: &Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(Duration::from_millis(5000))?;
    let _mode: String = c.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    c.execute_batch("PRAGMA synchronous=NORMAL")?;
    Ok(c)
}

/// Bytes of a payload or sig column: NULL is empty; TEXT and BLOB are their
/// bytes (Go scans both into `[]byte`); numbers are their Go text.
fn bytes_of(v: ValueRef<'_>) -> Vec<u8> {
    match v {
        ValueRef::Null => Vec::new(),
        ValueRef::Integer(i) => i.to_string().into_bytes(),
        ValueRef::Real(f) => f.to_string().into_bytes(),
        ValueRef::Text(b) | ValueRef::Blob(b) => b.to_vec(),
    }
}

/// A text column as Go scans it into a `string`. Known gap: Go keeps the
/// raw bytes, where this replaces invalid UTF-8, and a REAL is formatted
/// by Rust, not `strconv.FormatFloat(v, 'g', -1, 64)`. Irori never writes
/// either (every TEXT column holds Go strings, no column is REAL), so a
/// row from Go hashes the same; a hand-edited one could differ.
fn text_of(v: ValueRef<'_>) -> String {
    String::from_utf8_lossy(&bytes_of(v)).into_owned()
}

fn event_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        device_id: text_of(r.get_ref(0)?),
        seq: r.get(1)?,
        hlc: Timestamp::from_i64(r.get(2)?),
        actor: text_of(r.get_ref(3)?),
        kind: text_of(r.get_ref(4)?),
        payload: bytes_of(r.get_ref(5)?),
        sig: bytes_of(r.get_ref(6)?),
    })
}

fn query_events(c: &Connection, sql: &str, args: &[Value]) -> Result<Vec<Event>> {
    let mut st = c.prepare(sql)?;
    let rows = st.query_map(rusqlite::params_from_iter(args.iter()), event_of)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn query_one(c: &Connection, table: &str, dev: &str, seq: i64) -> Result<Option<Event>> {
    Ok(c.query_row(
        &format!("SELECT {COLS} FROM {table} WHERE device_id = ?1 AND seq = ?2"),
        params![dev, seq],
        event_of,
    )
    .optional()?)
}

/// A byte column's bind value: empty binds SQL NULL, as Go's nil `[]byte`
/// does. A zero-length BLOB would slip past the `sig IS NULL` check.
fn blob(b: &[u8]) -> Option<&[u8]> {
    if b.is_empty() { None } else { Some(b) }
}

fn insert_row(c: &Connection, table: &str, ev: &Event) -> Result<()> {
    c.execute(
        &format!("INSERT INTO {table} ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            ev.device_id,
            ev.seq,
            ev.hlc.as_i64(),
            ev.actor,
            ev.kind,
            blob(&ev.payload),
            blob(&ev.sig)
        ],
    )?;
    Ok(())
}

fn max_seq(c: &Connection, table: &str, dev: &str) -> Result<i64> {
    Ok(c.query_row(
        &format!("SELECT COALESCE(MAX(seq), 0) FROM {table} WHERE device_id = ?1"),
        [dev],
        |r| r.get(0),
    )?)
}

/// The hlc of `(dev, seq)` in events; 0 when it is not held (Go ignores
/// ErrNoRows and keeps 0).
fn hlc_at(c: &Connection, dev: &str, seq: i64) -> Result<i64> {
    Ok(c.query_row(
        "SELECT hlc FROM events WHERE device_id = ?1 AND seq = ?2",
        params![dev, seq],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or(0))
}

fn same_row(a: &Event, b: &Event) -> bool {
    a.hlc == b.hlc
        && a.actor == b.actor
        && a.kind == b.kind
        && a.payload == b.payload
        && a.sig == b.sig
}

fn record_refused(c: &Connection, ev: &Event, reason: &str) -> Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO refused_events (device_id, seq, hlc, actor, kind, payload, sig, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            ev.device_id,
            ev.seq,
            ev.hlc.as_i64(),
            ev.actor,
            ev.kind,
            blob(&ev.payload),
            blob(&ev.sig),
            reason
        ],
    )?;
    Ok(())
}

fn record_fork(c: &Connection, ev: &Event) -> Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO forks (device_id, seq, other_hash, hlc, actor, kind, payload, sig, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            ev.device_id,
            ev.seq,
            row_hash(ev),
            ev.hlc.as_i64(),
            ev.actor,
            ev.kind,
            blob(&ev.payload),
            blob(&ev.sig),
            FORK_TWO_ROWS
        ],
    )?;
    Ok(())
}

/// The row held at `(dev, seq)`, in events or moved to refused_events.
fn held_row(c: &Connection, dev: &str, seq: i64) -> Result<Option<Event>> {
    match query_one(c, "events", dev, seq)? {
        Some(ev) => Ok(Some(ev)),
        None => query_one(c, "refused_events", dev, seq),
    }
}

/// Go `schemaVersion`: 0 without a schema_migrations table.
fn read_schema_version(c: &Connection) -> Result<i64> {
    let n: i64 = c.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |r| r.get(0),
    )?;
    if n == 0 {
        return Ok(0);
    }
    Ok(c.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// Go `backupV03` (store.md §3.1 step 4): copy a schema-v1 database that
/// holds rows to `path.v0.3` before migrating it, retaken on every start
/// that finds v1. VACUUM INTO writes a temporary file renamed into place.
fn backup_v03(c: &Connection, path: &Path) -> Result<()> {
    if read_schema_version(c)? != 1 {
        return Ok(());
    }
    let rows: i64 = c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
    if rows == 0 {
        return Ok(());
    }
    let dst = with_suffix(path, BACKUP_SUFFIX);
    let tmp = with_suffix(&dst, ".tmp");
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let fail = |e: &dyn std::fmt::Display| {
        let _ = std::fs::remove_file(&tmp);
        Error::other(format!(
            "store: back up the v0.3 database to {}: {e}",
            dst.display()
        ))
    };
    let tmp_s = tmp.to_string_lossy().into_owned();
    if let Err(e) = c.execute("VACUUM INTO ?1", [&tmp_s]) {
        return Err(fail(&e));
    }
    if let Err(e) = std::fs::rename(&tmp, &dst) {
        return Err(fail(&e));
    }
    Ok(())
}

/// Go `migrate`: one IMMEDIATE transaction for every missing migration.
fn migrate(c: &mut Connection, device_id: &str, signer: Option<&dyn Signer>) -> Result<()> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(SCHEMA_MIGRATIONS)?;
    let have: i64 = tx.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?;
    let want = schema_version();
    if have > want {
        return Err(Error::other(format!(
            "store: database schema v{have} is newer than this build (v{want}); update Irori"
        )));
    }
    for v in have..want {
        let step = match v {
            0 => tx.execute_batch(MIGRATION_V1).map_err(Error::from),
            1 => tx.execute_batch(MIGRATION_V2).map_err(Error::from),
            _ => back_sign_and_move_local(&tx, device_id, signer),
        };
        step.map_err(|e| e.context(format!("store: migration {}: ", v + 1)))?;
        tx.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [v + 1],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Go `backSignAndMoveLocal`, the one-time v0.4 migration (store.md §3.2).
fn back_sign_and_move_local(
    tx: &Transaction<'_>,
    device_id: &str,
    signer: Option<&dyn Signer>,
) -> Result<()> {
    let evs = query_events(
        tx,
        &format!("SELECT {COLS} FROM events WHERE device_id = ?1 ORDER BY seq"),
        &[Value::Text(device_id.into())],
    )?;
    if evs.is_empty() {
        return Ok(());
    }
    let Some(signer) = signer else {
        return Err(Error::other(
            "store: this log needs the v0.4 migration, which signs rows; open it with a Signer",
        ));
    };
    let mut local_seq = 0i64;
    for mut ev in evs {
        if is_local(&ev.kind) {
            local_seq += 1;
            tx.execute(
                "INSERT INTO local_events (device_id, seq, hlc, actor, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![ev.device_id, local_seq, ev.hlc.as_i64(), ev.actor, ev.kind, blob(&ev.payload)],
            )?;
            ev.kind = kinds::LOCAL_MOVED.into();
            ev.payload = b"{}".to_vec();
        }
        let sig = signer
            .sign_row(&signing_bytes(&ev))
            .map_err(|e| e.context(format!("sign {}/{}: ", ev.device_id, ev.seq)))?;
        tx.execute(
            "UPDATE events SET kind = ?1, payload = ?2, sig = ?3 WHERE device_id = ?4 AND seq = ?5",
            params![ev.kind, blob(&ev.payload), blob(&sig), ev.device_id, ev.seq],
        )?;
    }
    Ok(())
}

/// What insert_one did with a row.
enum Outcome {
    /// Stored: a fresh row.
    Stored,
    /// Already held (identical, or moved to refused_events).
    Held,
    /// A revoke past its writer's cut, kept in refused_events and folded.
    HonouredRevoke,
}

impl Store {
    /// Open (creating and migrating) the database at `path` for this
    /// device, unsigned and unverified.
    pub fn open(path: impl AsRef<Path>, device_id: &str, clock: Arc<hlc::Clock>) -> Result<Store> {
        Store::open_with(path, device_id, clock, OpenOptions::default())
    }

    /// Open with a Signer and Verifier (the Core's path), following Go's
    /// open sequence (store.md §3.1).
    pub fn open_with(
        path: impl AsRef<Path>,
        device_id: &str,
        clock: Arc<hlc::Clock>,
        o: OpenOptions,
    ) -> Result<Store> {
        let path = path.as_ref();
        if device_id.is_empty() {
            return Err(Error::other("store: empty device id"));
        }
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            let mut b = std::fs::DirBuilder::new();
            b.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
            b.create(dir)?;
        }
        let mut conn = connect(path)?;
        backup_v03(&conn, path)?;
        migrate(&mut conn, device_id, o.signer.as_deref())?;
        if o.signer.is_some() {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events WHERE device_id = ?1 AND sig IS NULL",
                [device_id],
                |r| r.get(0),
            )?;
            if n > 0 {
                return Err(Error::other(format!(
                    "store: {n} of this device's rows are unsigned; the log was changed outside Irori"
                )));
            }
        }
        let top: i64 = conn.query_row(
            "SELECT COALESCE(MAX(hlc), 0) FROM events WHERE hlc <= ?1",
            [MAX_REPLICATED_HLC.as_i64()],
            |r| r.get(0),
        )?;
        clock.observe(Timestamp::from_i64(top));
        let max_ahead = if o.max_ahead.is_zero() {
            DEFAULT_MAX_AHEAD
        } else {
            o.max_ahead
        };
        let now = o.now.unwrap_or_else(|| Arc::new(SystemTime::now));
        let inner = Arc::new(Inner {
            path: path.to_path_buf(),
            device_id: device_id.into(),
            clock,
            signer: o.signer,
            verifier: o.verifier,
            board: o.board.then(BoardRules::default),
            max_ahead,
            now,
            writer: Mutex::new(Writer {
                conn: Some(conn),
                hold_seq: 0,
            }),
            readers: Mutex::new(Vec::new()),
            verify_mu: Mutex::new(()),
            subs: Mutex::new(Subs {
                closed: false,
                next: 0,
                list: BTreeMap::new(),
            }),
        });
        let s = Store { inner };
        if s.inner.board.is_some() {
            s.inner.load_board()?;
        }
        if s.inner.verifier.is_some() {
            s.inner.load_verifier()?;
            let mut w = lock(&s.inner.writer);
            let conn = w.conn.as_mut().ok_or_else(closed)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            s.inner.apply_cuts(&tx)?;
            tx.commit()?;
        }
        Ok(s)
    }

    /// Close the database and end every subscription. Idempotent; later
    /// appends and inserts return `Closed`.
    pub fn close(&self) -> Result<()> {
        self.inner.close();
        Ok(())
    }

    /// The device this store appends as.
    pub fn device_id(&self) -> &str {
        &self.inner.device_id
    }

    /// A new row for this device with the next seq and a fresh HLC.
    /// `payload` is marshalled as Go's `encoding/json` would
    /// (`gocompat::json::marshal`; `&()` is `null`). Subscribers are
    /// notified after commit. A device-local kind is refused.
    pub fn append<P: serde::Serialize + ?Sized>(
        &self,
        actor: &str,
        kind: &str,
        payload: &P,
    ) -> Result<Event> {
        if kind.is_empty() {
            return Err(Error::other("store: empty kind"));
        }
        if is_local(kind) {
            return Err(Error::wrap(ErrorKind::LocalKind, kind));
        }
        let raw = gocompat::json::marshal(payload)
            .map_err(|e| Error::other(format!("store: marshal {kind} payload: {e}")))?;
        self.inner.append(actor, kind, raw)
    }

    /// A device-local row in `local_events`, with its own gapless seq,
    /// never signed, never delivered to subscribers.
    pub fn append_local<P: serde::Serialize + ?Sized>(
        &self,
        actor: &str,
        kind: &str,
        payload: &P,
    ) -> Result<Event> {
        if !is_local(kind) {
            return Err(Error::wrap(ErrorKind::NotLocal, kind));
        }
        let raw = gocompat::json::marshal(payload)
            .map_err(|e| Error::other(format!("store: marshal {kind} payload: {e}")))?;
        self.inner.append_local(actor, kind, raw)
    }

    /// Store one row replicated from another device (`insert_batch` of one).
    pub fn insert(&self, ev: &Event) -> Result<()> {
        self.insert_batch(std::slice::from_ref(ev)).1
    }

    /// Insert rows in order in one transaction. Stops at the first refused
    /// row, commits the rows before it, and reports how many were applied
    /// (already-held duplicates count).
    pub fn insert_batch(&self, evs: &[Event]) -> (usize, Result<()>) {
        self.inner.insert_batch(evs)
    }

    /// Block append (`Restoring`) until own rows up to `seq` come back; 0
    /// clears. Revokes and rotations are never held.
    pub fn hold_appends(&self, seq: i64) {
        lock(&self.inner.writer).hold_seq = seq;
    }

    /// Reset the Verifier and feed it every identity row again. No-op
    /// without a Verifier.
    pub fn reload_verifier(&self) {
        if self.inner.verifier.is_some() {
            self.inner.reset_verifier();
        }
    }

    /// The max seq held per device, capped at each other device's cut.
    pub fn vector(&self) -> Result<Vector> {
        self.inner.vector()
    }

    /// Rows the holder of `v` lacks, by `(device_id, seq)`, up to `limit`
    /// (<= 0: 1000; at most 10000).
    pub fn missing(&self, v: &Vector, limit: i64) -> Result<Vec<Event>> {
        self.inner.missing(v, limit)
    }

    /// Rows matching `f`, by `(hlc, device_id, seq)`.
    pub fn list(&self, f: &Filter) -> Result<Vec<Event>> {
        self.inner.list(f)
    }

    /// Rows moved out because they were past their device's cut (and
    /// honoured revokes), oldest first.
    pub fn refused(&self, limit: i64) -> Result<Vec<Event>> {
        let limit = if limit <= 0 {
            DEFAULT_LIST_LIMIT
        } else {
            limit
        };
        self.inner.read(|c| {
            query_events(
                c,
                &format!("SELECT {COLS} FROM refused_events ORDER BY hlc, device_id, seq LIMIT ?1"),
                &[Value::Integer(limit.min(MAX_LIST_LIMIT))],
            )
        })
    }

    /// The `refused_events` rows strictly after `after` in
    /// `(hlc, device_id, seq)` order, up to `limit` (at most 1000). Not in
    /// Go; `irori-dev verify` pages the whole table with it.
    pub fn refused_after(&self, after: &crate::Cursor, limit: i64) -> Result<Vec<Event>> {
        let limit = if limit <= 0 {
            DEFAULT_LIST_LIMIT
        } else {
            limit
        }
        .min(MAX_LIST_LIMIT);
        if after.hlc.0 > i64::MAX as u64 {
            return Ok(Vec::new());
        }
        let h = after.hlc.as_i64();
        self.inner.read(|c| {
            query_events(
                c,
                &format!(
                    "SELECT {COLS} FROM refused_events WHERE (hlc > ?1 OR (hlc = ?1 AND (device_id > ?2 OR (device_id = ?2 AND seq > ?3)))) ORDER BY hlc, device_id, seq LIMIT ?4"
                ),
                &[
                    Value::Integer(h),
                    Value::Text(after.device_id.clone()),
                    Value::Integer(after.seq),
                    Value::Integer(limit),
                ],
            )
        })
    }

    /// Every conflicting row insert recorded.
    pub fn forks(&self) -> Result<Vec<Fork>> {
        self.inner.read(|c| {
            let mut st = c.prepare(
                "SELECT device_id, seq, hlc, actor, kind, payload, sig, other_hash, reason FROM forks ORDER BY device_id, seq, seen_at",
            )?;
            let rows = st.query_map([], |r| {
                Ok(Fork {
                    other: event_of(r)?,
                    other_hash: text_of(r.get_ref(7)?),
                    reason: text_of(r.get_ref(8)?),
                    stored: Event::default(),
                })
            })?;
            let mut out = Vec::new();
            for f in rows {
                out.push(f?);
            }
            for f in &mut out {
                f.stored = query_one(c, "events", &f.other.device_id, f.other.seq)?
                    .unwrap_or_default();
            }
            Ok(out)
        })
    }

    /// Every row appended or freshly inserted after the call, until the
    /// subscription is cancelled or dropped or the store closes. A reader
    /// that falls 255 rows behind gets a zero Event (resync: re-list) and
    /// loses rows until it drains; writers never block.
    pub fn subscribe(&self) -> Subscription {
        let q = Arc::new(Queue::new(SUB_BUFFER));
        let mut s = lock(&self.inner.subs);
        if s.closed {
            q.close();
            return Subscription {
                q,
                id: u64::MAX,
                store: Weak::new(),
            };
        }
        let id = s.next;
        s.next += 1;
        s.list.insert(
            id,
            Sub {
                q: q.clone(),
                lagging: false,
            },
        );
        Subscription {
            q,
            id,
            store: Arc::downgrade(&self.inner),
        }
    }

    /// A store opened with `board: true`.
    pub fn is_board(&self) -> bool {
        self.inner.board.is_some()
    }

    /// What a board store refused; zero for the private log.
    pub fn board_refused(&self) -> BoardCounts {
        self.inner
            .board
            .as_ref()
            .map(BoardRules::counts)
            .unwrap_or_default()
    }

    /// Why `ev`'s references don't all resolve in this board log right now
    /// (kind `OutsideBoard`); the private log always resolves.
    pub fn unresolved(&self, ev: &Event) -> Result<()> {
        match &self.inner.board {
            Some(b) => b.resolves(&ev.kind, &ev.payload),
            None => Ok(()),
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.inner.close();
    }
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.inner.path)
            .field("device_id", &self.inner.device_id)
            .field("board", &self.inner.board.is_some())
            .finish()
    }
}

/// A subscription's stream of rows (Go's `<-chan Event`). A zero Event
/// ([`Event::is_resync`]) means "re-list". `None` from a receive means the
/// subscription ended.
pub struct Subscription {
    q: Arc<Queue<Event>>,
    id: u64,
    store: Weak<Inner>,
}

impl Subscription {
    /// A subscription fed by the caller instead of a store (tests: a
    /// backend that rewrites a store's stream, as Go's `wrapSub` does).
    /// A zero Event pushed on `q` is a resync.
    pub fn from_queue(q: Arc<Queue<Event>>) -> Subscription {
        Subscription {
            q,
            id: 0,
            store: Weak::new(),
        }
    }

    /// The next row, waiting on this thread.
    pub fn recv(&self) -> Option<Event> {
        self.q.recv()
    }

    /// The next row, waiting at most `timeout`; `Err(())` on timeout.
    #[allow(clippy::result_unit_err)]
    pub fn recv_timeout(&self, timeout: Duration) -> std::result::Result<Option<Event>, ()> {
        self.q.recv_timeout(timeout)
    }

    /// The next row if one is queued.
    pub fn try_recv(&self) -> TryRecv<Event> {
        self.q.try_recv()
    }

    /// The next row, from async code.
    pub async fn recv_async(&self) -> Option<Event> {
        self.q.recv_async().await
    }

    /// Rows queued (Go `len(ch)`).
    pub fn len(&self) -> usize {
        self.q.len()
    }

    /// Nothing queued.
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// End the subscription (Go: its context ends). Queued rows can still
    /// be received; then the stream ends.
    pub fn cancel(&self) {
        if let Some(inner) = self.store.upgrade() {
            let mut s = lock(&inner.subs);
            s.list.remove(&self.id);
        }
        self.q.close();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Inner {
    fn is_closed(&self) -> bool {
        lock(&self.subs).closed
    }

    fn close(&self) {
        {
            let mut s = lock(&self.subs);
            if s.closed {
                return;
            }
            s.closed = true;
            for sub in s.list.values() {
                sub.q.close();
            }
            s.list.clear();
        }
        lock(&self.writer).conn = None;
        lock(&self.readers).clear();
    }

    /// Run `f` on a read connection from the pool.
    fn read<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        if self.is_closed() {
            return Err(closed());
        }
        let pooled = lock(&self.readers).pop();
        let conn = match pooled {
            Some(c) => c,
            None => connect(&self.path)?,
        };
        let out = f(&conn);
        let mut pool = lock(&self.readers);
        if pool.len() < IDLE_READERS && !self.is_closed() {
            pool.push(conn);
        }
        out
    }

    fn identity_kind(&self, kind: &str) -> bool {
        if self.board.is_some() {
            kinds::is_board_identity_kind(kind)
        } else {
            kinds::is_person_kind(kind)
        }
    }

    fn identity_prefixes(&self) -> Vec<String> {
        if self.board.is_some() {
            vec!["board.".into(), "member.".into(), kinds::AGENT_CARD.into()]
        } else {
            vec!["person.".into()]
        }
    }

    /// Kinds honoured past their writer's cut.
    fn revoke_kinds(&self) -> &'static [&'static str] {
        if self.board.is_some() {
            &[
                kinds::MEMBER_DEVICE_REVOKED,
                kinds::BOARD_MEMBER_REMOVED,
                kinds::BOARD_MEMBER_LEFT,
            ]
        } else {
            &[kinds::PERSON_DEVICE_REVOKED]
        }
    }

    fn v_cut(&self, dev: &str) -> Option<i64> {
        let v = self.verifier.as_ref()?;
        let _g = lock(&self.verify_mu);
        v.cut(dev)
    }

    fn v_observe(&self, ev: &Event) {
        if let Some(v) = &self.verifier {
            let _g = lock(&self.verify_mu);
            v.observe(ev);
        }
    }

    fn v_verify_row(&self, ev: &Event) -> Result<()> {
        match &self.verifier {
            Some(v) => {
                let _g = lock(&self.verify_mu);
                v.verify_row(ev)
            }
            None => Ok(()),
        }
    }

    /// Go `verify`: a replicated row must be signed, and verify.
    fn verify(&self, ev: &Event) -> Result<()> {
        if ev.sig.is_empty() {
            return Err(Error::wrap(
                ErrorKind::Unsigned,
                format!("{}/{}", ev.device_id, ev.seq),
            ));
        }
        let r = self.v_verify_row(ev);
        if let (Some(b), Err(e)) = (&self.board, &r)
            && e.is(ErrorKind::BoardActor)
        {
            b.count_actor();
        }
        r
    }

    /// The highest HLC a replicated row may carry right now.
    fn ahead_bound(&self) -> Timestamp {
        let ms = hlc::unix_milli((self.now)() + self.max_ahead);
        Timestamp(Timestamp::from_unix_ms(ms).0 | 0xffff)
    }

    fn reset_verifier(&self) {
        if let Some(v) = &self.verifier {
            {
                let _g = lock(&self.verify_mu);
                v.reset();
            }
            // Go ignores the error, and does not honour cancellation.
            let _ = self.load_verifier();
        }
    }

    /// Go `loadVerifier`: fold every held identity row, then the revokes
    /// kept in refused_events.
    fn load_verifier(&self) -> Result<()> {
        let Some(v) = &self.verifier else {
            return Ok(());
        };
        let mut after = crate::Cursor::default();
        loop {
            let evs = self.list(&Filter {
                kinds: self.identity_prefixes(),
                after: after.clone(),
                limit: MAX_LIST_LIMIT,
                ..Filter::default()
            })?;
            {
                let _g = lock(&self.verify_mu);
                for ev in &evs {
                    v.observe(ev);
                }
            }
            if (evs.len() as i64) < MAX_LIST_LIMIT {
                break;
            }
            after = evs[evs.len() - 1].cursor();
        }
        let kinds = self.revoke_kinds();
        let marks = vec!["?"; kinds.len()].join(", ");
        let args: Vec<Value> = kinds.iter().map(|k| Value::Text((*k).into())).collect();
        let revs = self.read(|c| {
            query_events(
                c,
                &format!(
                    "SELECT {COLS} FROM refused_events WHERE kind IN ({marks}) ORDER BY hlc, device_id, seq"
                ),
                &args,
            )
        })?;
        let _g = lock(&self.verify_mu);
        for ev in &revs {
            v.observe(ev);
        }
        Ok(())
    }

    /// Go `loadBoard`: the closure index from every held row.
    fn load_board(&self) -> Result<()> {
        let Some(b) = &self.board else {
            return Ok(());
        };
        let mut after = crate::Cursor::default();
        loop {
            let evs = self.list(&Filter {
                after: after.clone(),
                limit: MAX_LIST_LIMIT,
                ..Filter::default()
            })?;
            for ev in &evs {
                b.define(&ev.kind, &ev.payload);
            }
            if (evs.len() as i64) < MAX_LIST_LIMIT {
                return Ok(());
            }
            after = evs[evs.len() - 1].cursor();
        }
    }

    /// Go `notify`: hand `ev` to every subscriber without blocking; the
    /// last slot of each queue is the resync signal's.
    fn notify(&self, ev: &Event) {
        let mut s = lock(&self.subs);
        for sub in s.list.values_mut() {
            let n = sub.q.len();
            if n < sub.q.cap() - 1 {
                sub.q.push(ev.clone());
                sub.lagging = false;
            } else if !sub.lagging {
                sub.q.push(Event::default());
                sub.lagging = true;
            }
        }
    }

    /// Go `notifyResync`: rows already folded were moved out.
    fn notify_resync(&self) {
        let mut s = lock(&self.subs);
        for sub in s.list.values_mut() {
            if !sub.lagging && sub.q.len() < sub.q.cap() {
                sub.q.push(Event::default());
                sub.lagging = true;
            }
        }
    }

    fn append(&self, actor: &str, kind: &str, raw: Vec<u8>) -> Result<Event> {
        if let Some(b) = &self.board {
            b.check_shape(actor, kind, &raw)?;
        }
        let mut w = lock(&self.writer);
        if self.is_closed() {
            return Err(closed());
        }
        if let Some(b) = &self.board {
            // Under the write lock, so what the row names can't be appended
            // and checked out of order.
            if let Err(e) = b.resolves(kind, &raw) {
                b.count_outside();
                return Err(e);
            }
        }
        let Writer { conn, hold_seq } = &mut *w;
        let conn = conn.as_mut().ok_or_else(closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let last = max_seq(&tx, "events", &self.device_id)?;
        if *hold_seq > 0 && !kinds::never_held(kind) {
            if last < *hold_seq {
                return Err(Error::new(
                    ErrorKind::Restoring,
                    format!(
                        "{} (hold {}, have {last})",
                        ErrorKind::Restoring.text(),
                        *hold_seq
                    ),
                ));
            }
            *hold_seq = 0;
        }
        let mut ev = Event {
            device_id: self.device_id.clone(),
            seq: last + 1,
            hlc: self.clock.now(),
            actor: actor.into(),
            kind: kind.into(),
            payload: raw,
            sig: Vec::new(),
        };
        if ev.hlc.0 > i64::MAX as u64 {
            return Err(Error::other(format!(
                "store: clock reached hlc {}, past int64",
                ev.hlc
            )));
        }
        if let Some(s) = &self.signer {
            ev.sig = s
                .sign_row(&signing_bytes(&ev))
                .map_err(|e| e.context(format!("store: sign {kind}: ")))?;
        }
        insert_row(&tx, "events", &ev)?;
        let observed = self.verifier.is_some() && self.identity_kind(kind);
        let mut resync = false;
        if observed {
            self.v_observe(&ev);
            match self.apply_cuts(&tx) {
                Ok(r) => resync = r,
                Err(e) => {
                    drop(tx);
                    self.reset_verifier();
                    return Err(e);
                }
            }
        }
        if let Err(e) = tx.commit() {
            if observed {
                self.reset_verifier();
            }
            return Err(e.into());
        }
        if let Some(b) = &self.board {
            b.define(kind, &ev.payload);
        }
        self.notify(&ev);
        if resync {
            self.notify_resync();
        }
        Ok(ev)
    }

    fn append_local(&self, actor: &str, kind: &str, raw: Vec<u8>) -> Result<Event> {
        let mut w = lock(&self.writer);
        if self.is_closed() {
            return Err(closed());
        }
        let conn = w.conn.as_mut().ok_or_else(closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let last = max_seq(&tx, "local_events", &self.device_id)?;
        let ev = Event {
            device_id: self.device_id.clone(),
            seq: last + 1,
            hlc: self.clock.now(),
            actor: actor.into(),
            kind: kind.into(),
            payload: raw,
            sig: Vec::new(),
        };
        insert_row(&tx, "local_events", &ev)?;
        tx.commit()?;
        Ok(ev)
    }

    /// Go `checkRow`: Insert's validation that needs no database, first
    /// match wins.
    fn check_row(&self, ev: &mut Event) -> Result<()> {
        let (dev, seq) = (&ev.device_id, ev.seq);
        if *dev == self.device_id && self.verifier.is_none() {
            return Err(Error::sentinel(ErrorKind::OwnDevice));
        }
        if dev.is_empty() {
            return Err(Error::other("store: replicated row has no device id"));
        }
        if seq < 1 {
            return Err(Error::other(format!(
                "store: replicated row {dev}/{seq} has seq < 1"
            )));
        }
        if ev.kind.is_empty() {
            return Err(Error::other(format!(
                "store: replicated row {dev}/{seq} has no kind"
            )));
        }
        if ev.hlc > MAX_REPLICATED_HLC {
            return Err(Error::other(format!(
                "store: replicated row {dev}/{seq} has hlc {} past the {} ceiling",
                ev.hlc, MAX_REPLICATED_HLC
            )));
        }
        if is_local(&ev.kind) {
            return Err(Error::wrap(
                ErrorKind::LocalKind,
                format!("replicated row {dev}/{seq} is {}", ev.kind),
            ));
        }
        if ev.payload.is_empty() {
            ev.payload = b"null".to_vec();
        }
        if !gocompat::json::valid(&ev.payload) {
            return Err(Error::other(format!(
                "store: replicated row {}/{} has a payload that is not valid JSON",
                ev.device_id, ev.seq
            )));
        }
        Ok(())
    }

    fn insert_batch(&self, evs: &[Event]) -> (usize, Result<()>) {
        if evs.is_empty() {
            return (0, Ok(()));
        }
        let mut w = lock(&self.writer);
        if self.is_closed() {
            return (0, Err(closed()));
        }
        let Some(conn) = w.conn.as_mut() else {
            return (0, Err(closed()));
        };
        let tx = match conn.transaction_with_behavior(TransactionBehavior::Immediate) {
            Ok(tx) => tx,
            Err(e) => return (0, Err(e.into())),
        };
        let mut applied = 0usize;
        let mut fresh: Vec<Event> = Vec::new();
        let mut observed = false;
        let mut resync = false;
        let mut row_err = None;
        for ev in evs {
            let mut ev = ev.clone();
            match self.insert_one(&tx, &mut ev) {
                Ok(Outcome::HonouredRevoke) => {
                    observed = true;
                    match self.apply_cuts(&tx) {
                        Ok(r) => resync |= r,
                        Err(e) => {
                            drop(tx);
                            self.reset_verifier();
                            return (0, Err(e));
                        }
                    }
                    applied += 1;
                }
                Ok(Outcome::Held) => applied += 1,
                Ok(Outcome::Stored) => {
                    applied += 1;
                    if self.verifier.is_some() && self.identity_kind(&ev.kind) {
                        self.v_observe(&ev);
                        observed = true;
                        match self.apply_cuts(&tx) {
                            Ok(r) => resync |= r,
                            Err(e) => {
                                drop(tx);
                                self.reset_verifier();
                                return (0, Err(e));
                            }
                        }
                    }
                    fresh.push(ev);
                }
                Err(e) => {
                    row_err = Some(e);
                    break;
                }
            }
        }
        if let Err(e) = tx.commit() {
            if observed {
                self.reset_verifier();
            }
            return (0, Err(e.into()));
        }
        if let Some(b) = &self.board {
            // Every row's ids first, so a subscriber checking closure on one
            // row of the batch sees the others.
            for ev in &fresh {
                b.define(&ev.kind, &ev.payload);
            }
        }
        for ev in &fresh {
            // Only a committed row moves the clock.
            if self.verifier.is_some() {
                self.clock.observe_bounded(ev.hlc, self.ahead_bound());
            } else {
                self.clock.observe(ev.hlc);
            }
            self.notify(ev);
        }
        if resync {
            self.notify_resync();
        }
        (applied, row_err.map_or(Ok(()), Err))
    }

    /// Go `insertOne`, in its exact check order (store.md §3.5, §7.7).
    fn insert_one(&self, tx: &Transaction<'_>, ev: &mut Event) -> Result<Outcome> {
        self.check_row(ev)?;
        if let Some(b) = &self.board {
            // The allow-list and actor shape hold for forwarded rows too;
            // closure doesn't.
            b.check_shape(&ev.actor, &ev.kind, &ev.payload)
                .map_err(|e| e.context(format!("{}/{}: ", ev.device_id, ev.seq)))?;
        }
        let last = max_seq(tx, "events", &ev.device_id)?;
        if ev.seq <= last {
            let Some(held) = query_one(tx, "events", &ev.device_id, ev.seq)? else {
                // Moved to refused_events: still held as far as sync goes.
                return Ok(Outcome::Held);
            };
            if same_row(&held, ev) {
                return Ok(Outcome::Held);
            }
            // Only a row the device really signed proves a fork.
            if self.verifier.is_some() {
                self.verify(ev)?;
            }
            record_fork(tx, ev)?;
            return Err(Error::wrap(
                ErrorKind::Fork,
                format!("{}/{}", ev.device_id, ev.seq),
            ));
        }
        if self.verifier.is_some() {
            // Before the gap check: a revoked device's later rows are
            // refused whether or not the rows between are held.
            if let Some(cut) = self.v_cut(&ev.device_id)
                && ev.seq > cut
            {
                if self.revoke_kinds().contains(&ev.kind.as_str()) && !ev.sig.is_empty() {
                    self.v_verify_row(ev)?;
                    if last > 0 {
                        let held = hlc_at(tx, &ev.device_id, last)?;
                        if ev.hlc.as_i64() <= held {
                            return Err(Error::wrap(
                                ErrorKind::HlcOrder,
                                format!("{}/{} hlc {} after {held}", ev.device_id, ev.seq, ev.hlc),
                            ));
                        }
                    }
                    if ev.hlc > self.ahead_bound() {
                        return Err(self.ahead_err(ev));
                    }
                    record_refused(tx, ev, REFUSED_REVOKE)?;
                    self.v_observe(ev);
                    return Ok(Outcome::HonouredRevoke);
                }
                return Err(Error::wrap(
                    ErrorKind::PastCut,
                    format!("{}/{} (cut {cut})", ev.device_id, ev.seq),
                ));
            }
        }
        if ev.seq > last + 1 {
            return Err(Error::wrap(
                ErrorKind::Gap,
                format!("{} holds seq {last}, got {}", ev.device_id, ev.seq),
            ));
        }
        if self.verifier.is_some() {
            self.verify(ev)?;
        }
        if last > 0 {
            let prev = hlc_at(tx, &ev.device_id, last)?;
            if ev.hlc.as_i64() <= prev {
                return Err(Error::wrap(
                    ErrorKind::HlcOrder,
                    format!("{}/{} hlc {} after {prev}", ev.device_id, ev.seq, ev.hlc),
                ));
            }
        }
        if self.verifier.is_some() && ev.hlc > self.ahead_bound() {
            return Err(self.ahead_err(ev));
        }
        insert_row(tx, "events", ev)?;
        if self.verifier.is_some() {
            self.check_cut_claims(tx, Some(ev))?;
        }
        Ok(Outcome::Stored)
    }

    fn ahead_err(&self, ev: &Event) -> Error {
        let at = gocompat::time::GoTime::unix_milli(ev.hlc.wall_ms()).utc();
        Error::wrap(
            ErrorKind::Ahead,
            format!(
                "{}/{} stamped {}",
                ev.device_id,
                ev.seq,
                at.format_rfc3339()
            ),
        )
    }

    /// Go `checkCutClaims`: a fork wherever an honoured revoke's cutHash
    /// names another row than the one held at its cut.
    fn check_cut_claims(&self, tx: &Transaction<'_>, only: Option<&Event>) -> Result<()> {
        let Some(v) = &self.verifier else {
            return Ok(());
        };
        let claims: Vec<CutClaim> = {
            let _g = lock(&self.verify_mu);
            v.cut_claims()
        };
        for c in claims {
            let held = match only {
                Some(o) => {
                    if o.device_id != c.device_id || o.seq != c.seq {
                        continue;
                    }
                    o.clone()
                }
                None => match held_row(tx, &c.device_id, c.seq)? {
                    Some(ev) => ev,
                    None => continue, // not held yet; checked when it arrives
                },
            };
            if row_hash(&held) == c.hash {
                continue;
            }
            tx.execute(
                "INSERT OR IGNORE INTO forks (device_id, seq, other_hash, hlc, actor, kind, reason) VALUES (?1, ?2, ?3, 0, '', '', ?4)",
                params![c.device_id, c.seq, c.hash, FORK_CUT_HASH],
            )?;
        }
        Ok(())
    }

    /// Go `applyCuts`: move every held row above its device's cut from
    /// events to refused_events. This device's own rows never move.
    fn apply_cuts(&self, tx: &Transaction<'_>) -> Result<bool> {
        let tops: Vec<(String, i64)> = {
            let mut st = tx.prepare("SELECT device_id, MAX(seq) FROM events GROUP BY device_id")?;
            let rows = st.query_map([], |r| Ok((text_of(r.get_ref(0)?), r.get::<_, i64>(1)?)))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };
        let mut moved = false;
        for (dev, max) in tops {
            if dev == self.device_id {
                continue;
            }
            let Some(cut) = self.v_cut(&dev) else {
                continue;
            };
            if max <= cut {
                continue;
            }
            tx.execute(
                "INSERT OR IGNORE INTO refused_events (device_id, seq, hlc, actor, kind, payload, sig, reason)
			 SELECT device_id, seq, hlc, actor, kind, payload, sig, ?1 FROM events WHERE device_id = ?2 AND seq > ?3",
                params![REFUSED_PAST_CUT, dev, cut],
            )?;
            tx.execute(
                "DELETE FROM events WHERE device_id = ?1 AND seq > ?2",
                params![dev, cut],
            )?;
            moved = true;
        }
        self.check_cut_claims(tx, None)?;
        Ok(moved)
    }

    fn vector(&self) -> Result<Vector> {
        let mut v = self.read(|c| {
            let mut st = c.prepare("SELECT device_id, MAX(seq) FROM events GROUP BY device_id")?;
            let rows = st.query_map([], |r| Ok((text_of(r.get_ref(0)?), r.get::<_, i64>(1)?)))?;
            let mut v = Vector::new();
            for r in rows {
                let (d, s) = r?;
                v.insert(d, s);
            }
            Ok(v)
        })?;
        // The cap also covers a revoke whose rows have not been moved yet.
        if let Some(ver) = &self.verifier {
            let _g = lock(&self.verify_mu);
            for (dev, top) in v.iter_mut() {
                if *dev == self.device_id {
                    continue;
                }
                if let Some(cut) = ver.cut(dev)
                    && *top > cut
                {
                    *top = cut;
                }
            }
        }
        Ok(v)
    }

    fn missing(&self, v: &Vector, limit: i64) -> Result<Vec<Event>> {
        let limit = if limit <= 0 {
            DEFAULT_MISSING_LIMIT
        } else {
            limit
        }
        .min(MAX_MISSING_LIMIT);
        let have = self.vector()?;
        // BTreeMap order is Go's sort.Strings (byte order).
        let devs: Vec<(&String, i64)> = have
            .iter()
            .filter(|(d, top)| **top > v.get(*d).copied().unwrap_or(0))
            .map(|(d, top)| (d, *top))
            .collect();
        self.read(|c| {
            let mut out = Vec::new();
            for (dev, top) in devs {
                if out.len() as i64 >= limit {
                    break;
                }
                let evs = query_events(
                    c,
                    &format!(
                        "SELECT {COLS} FROM events
			 WHERE device_id = ?1 AND seq > ?2 AND seq <= ?3 ORDER BY seq LIMIT ?4"
                    ),
                    &[
                        Value::Text(dev.clone()),
                        Value::Integer(v.get(dev).copied().unwrap_or(0)),
                        Value::Integer(top),
                        Value::Integer(limit - out.len() as i64),
                    ],
                )?;
                out.extend(evs);
            }
            Ok(out)
        })
    }

    fn list(&self, f: &Filter) -> Result<Vec<Event>> {
        let mut wh: Vec<String> = Vec::new();
        let mut args: Vec<Value> = Vec::new();
        if !f.kinds.is_empty() {
            let mut or = Vec::new();
            for k in &f.kinds {
                if let Some(stem) = k.strip_suffix('.') {
                    // A range, so the kind index applies: "agent." <= kind < "agent/".
                    or.push("(kind >= ? AND kind < ?)");
                    args.push(Value::Text(k.clone()));
                    args.push(Value::Text(format!("{stem}/")));
                } else {
                    or.push("kind = ?");
                    args.push(Value::Text(k.clone()));
                }
            }
            wh.push(format!("({})", or.join(" OR ")));
        }
        if !f.device_id.is_empty() {
            wh.push("device_id = ?".into());
            args.push(Value::Text(f.device_id.clone()));
        }
        if f.after_seq > 0 {
            wh.push("seq > ?".into());
            args.push(Value::Integer(f.after_seq));
        }
        if f.after_hlc.0 != 0 {
            if f.after_hlc.0 > i64::MAX as u64 {
                return Ok(Vec::new()); // nothing is stored past int64
            }
            wh.push("hlc > ?".into());
            args.push(Value::Integer(f.after_hlc.as_i64()));
        }
        if !f.after.is_zero() {
            if f.after.hlc.0 > i64::MAX as u64 {
                return Ok(Vec::new());
            }
            let h = f.after.hlc.as_i64();
            wh.push(
                "(hlc > ? OR (hlc = ? AND (device_id > ? OR (device_id = ? AND seq > ?))))".into(),
            );
            args.push(Value::Integer(h));
            args.push(Value::Integer(h));
            args.push(Value::Text(f.after.device_id.clone()));
            args.push(Value::Text(f.after.device_id.clone()));
            args.push(Value::Integer(f.after.seq));
        }
        let limit = if f.limit <= 0 {
            DEFAULT_LIST_LIMIT
        } else {
            f.limit
        }
        .min(MAX_LIST_LIMIT);
        let table = if f.local { "local_events" } else { "events" };
        let mut q = format!("SELECT {COLS} FROM {table}");
        if !wh.is_empty() {
            q.push_str(" WHERE ");
            q.push_str(&wh.join(" AND "));
        }
        // device_id, seq break HLC ties so the order is total and repeatable.
        if f.desc {
            q.push_str(" ORDER BY hlc DESC, device_id DESC, seq DESC");
        } else {
            q.push_str(" ORDER BY hlc, device_id, seq");
        }
        q.push_str(" LIMIT ?");
        args.push(Value::Integer(limit));
        self.read(|c| query_events(c, &q, &args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_bind_null() {
        assert_eq!(blob(b""), None);
        assert_eq!(blob(b"x"), Some(&b"x"[..]));
    }
}
