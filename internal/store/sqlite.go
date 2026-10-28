package store

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"

	_ "modernc.org/sqlite" // registers the "sqlite" driver
)

// HLC storage: SQLite integers are int64 and hlc.Timestamp is uint64. Rows store
// int64(ts), which keeps ordering as long as the top bit is clear. The wall part
// is milliseconds<<16, so the top bit stays clear until 2^47 ms after 1970, a few
// thousand years out. Insert refuses anything larger rather than wrap it.
//
// Replicated rows must also sit at or below MaxReplicatedHLC. Observing a row
// near MaxInt64 would leave the clock no room, and every later Append would
// fail; the ceiling leaves 2^62 readings of headroom past the highest row any
// peer can hand us.

// Errors callers may want to match.
var (
	// ErrOwnDevice: Insert was handed a row carrying this device's id, and
	// the store has no Verifier to check it came from this device's key.
	ErrOwnDevice = errors.New("store: refusing to insert a row for this device's own id; use Append")
	// ErrGap: Insert was handed a row past the next expected seq for its device,
	// which would leave a hole that Vector could not describe.
	ErrGap = errors.New("store: replicated row leaves a seq gap")
	// ErrClosed: the store was closed.
	ErrClosed = errors.New("store: closed")
)

// MaxReplicatedHLC is the highest HLC Insert accepts: a wall part of 2^46 ms,
// about the year 4199.
const MaxReplicatedHLC hlc.Timestamp = 1 << 62

const (
	// DefaultListLimit and MaxListLimit bound Filter.Limit.
	DefaultListLimit = 200
	MaxListLimit     = 1000
	// DefaultMissingLimit is used when Missing gets limit <= 0; MaxMissingLimit caps it.
	DefaultMissingLimit = 1000
	MaxMissingLimit     = 10000

	// subBuffer is each subscriber's channel capacity; the last slot is kept
	// for the resync signal.
	subBuffer = 256
)

// BackupSuffix names the copy of a v0.3 database taken before the v0.4
// migration (§5.4): v0.3.x refuses the migrated schema, so this is the way
// back.
const BackupSuffix = ".v0.3"

// migration takes the schema up one version: SQL, or Go code for steps that
// need the Signer (§5.7 item 6). Append only.
type migration struct {
	sql string
	run func(ctx context.Context, tx *sql.Tx, env migrateEnv) error
}

type migrateEnv struct {
	deviceID string
	signer   Signer
}

var migrations = []migration{
	// v1 (v0.1-v0.3): the log.
	{sql: `CREATE TABLE events (
		device_id TEXT    NOT NULL,
		seq       INTEGER NOT NULL,
		hlc       INTEGER NOT NULL,
		actor     TEXT    NOT NULL,
		kind      TEXT    NOT NULL,
		payload   BLOB,
		PRIMARY KEY (device_id, seq)
	) WITHOUT ROWID;
	CREATE INDEX events_hlc ON events (hlc);
	CREATE INDEX events_kind ON events (kind, hlc);`},
	// v2 (v0.4): signatures, device-local rows, refused rows and forks.
	{sql: `ALTER TABLE events ADD COLUMN sig BLOB;
	CREATE TABLE local_events (
		device_id TEXT    NOT NULL,
		seq       INTEGER NOT NULL,
		hlc       INTEGER NOT NULL,
		actor     TEXT    NOT NULL,
		kind      TEXT    NOT NULL,
		payload   BLOB,
		sig       BLOB,
		PRIMARY KEY (device_id, seq)
	) WITHOUT ROWID;
	CREATE INDEX local_events_kind ON local_events (kind, hlc);
	CREATE TABLE refused_events (
		device_id  TEXT    NOT NULL,
		seq        INTEGER NOT NULL,
		hlc        INTEGER NOT NULL,
		actor      TEXT    NOT NULL,
		kind       TEXT    NOT NULL,
		payload    BLOB,
		sig        BLOB,
		reason     TEXT    NOT NULL,
		refused_at INTEGER NOT NULL DEFAULT (unixepoch()),
		PRIMARY KEY (device_id, seq)
	) WITHOUT ROWID;
	CREATE TABLE forks (
		device_id  TEXT    NOT NULL,
		seq        INTEGER NOT NULL,
		other_hash TEXT    NOT NULL,
		hlc        INTEGER NOT NULL,
		actor      TEXT    NOT NULL,
		kind       TEXT    NOT NULL,
		payload    BLOB,
		sig        BLOB,
		reason     TEXT    NOT NULL DEFAULT 'two rows',
		seen_at    INTEGER NOT NULL DEFAULT (unixepoch()),
		PRIMARY KEY (device_id, seq, other_hash)
	) WITHOUT ROWID;`},
	// v3 (v0.4): the one-time migration (§5.4).
	{run: backSignAndMoveLocal},
}

type db struct {
	sql      *sql.DB
	deviceID string
	clock    *hlc.Clock
	signer   Signer
	verifier Verifier
	maxAhead time.Duration
	now      func() time.Time

	// writeMu serializes every write and its notification, so subscribers see
	// rows in commit order and own seqs follow own HLCs.
	writeMu  sync.Mutex
	holdSeq  int64 // HoldAppends: Append waits until own rows reach this seq
	verifyMu sync.Mutex

	subMu  sync.Mutex
	subs   map[*subscriber]struct{}
	closed bool
	done   chan struct{} // closed by close(), ends subscriber goroutines
}

type subscriber struct {
	ch      chan Event
	lagging bool // a resync signal is queued and not yet followed by a delivery
}

func open(ctx context.Context, path, deviceID string, clock *hlc.Clock, o OpenOptions) (*Store, error) {
	if deviceID == "" {
		return nil, errors.New("store: empty device id")
	}
	if clock == nil {
		return nil, errors.New("store: nil clock")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return nil, err
	}
	// A plain path keeps its query for the driver: WAL, wait on locks held by
	// another process, and BEGIN IMMEDIATE so a write transaction takes the
	// write lock up front instead of failing to upgrade later.
	dsn := path + "?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)&_txlock=immediate"
	sdb, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	sdb.SetMaxOpenConns(8)
	if err := backupV03(ctx, sdb, path); err != nil {
		sdb.Close()
		return nil, err
	}
	if err := migrate(ctx, sdb, migrateEnv{deviceID: deviceID, signer: o.Signer}); err != nil {
		sdb.Close()
		return nil, err
	}
	if o.Signer != nil {
		// Every own row is signed by now; one that isn't means something
		// wrote the log behind the store's back.
		var n int
		if err := sdb.QueryRowContext(ctx, `SELECT COUNT(*) FROM events WHERE device_id = ? AND sig IS NULL`, deviceID).Scan(&n); err != nil {
			sdb.Close()
			return nil, err
		}
		if n > 0 {
			sdb.Close()
			return nil, fmt.Errorf("store: %d of this device's rows are unsigned; the log was changed outside Irori", n)
		}
	}
	// Seed the clock with the newest row held, so own rows stamped after a
	// restart still sort after everything this device already has.
	var top int64
	if err := sdb.QueryRowContext(ctx, `SELECT COALESCE(MAX(hlc), 0) FROM events WHERE hlc <= ?`, int64(MaxReplicatedHLC)).Scan(&top); err != nil {
		sdb.Close()
		return nil, err
	}
	clock.Observe(hlc.Timestamp(top))
	if o.MaxAhead <= 0 {
		o.MaxAhead = DefaultMaxAhead
	}
	if o.Now == nil {
		o.Now = time.Now
	}
	s := &Store{impl: &db{
		sql: sdb, deviceID: deviceID, clock: clock, signer: o.Signer, verifier: o.Verifier,
		maxAhead: o.MaxAhead, now: o.Now, subs: map[*subscriber]struct{}{}, done: make(chan struct{}),
	}}
	if o.Verifier != nil {
		if err := s.impl.loadVerifier(ctx); err != nil {
			sdb.Close()
			return nil, err
		}
		tx, err := sdb.BeginTx(ctx, nil)
		if err != nil {
			sdb.Close()
			return nil, err
		}
		if _, err := s.impl.applyCuts(ctx, tx); err != nil {
			tx.Rollback()
			sdb.Close()
			return nil, err
		}
		if err := tx.Commit(); err != nil {
			sdb.Close()
			return nil, err
		}
	}
	return s, nil
}

// SchemaVersion is the schema version this build writes.
func SchemaVersion() int { return len(migrations) }

func schemaVersion(ctx context.Context, q interface {
	QueryRowContext(context.Context, string, ...any) *sql.Row
}) (int, error) {
	var n int
	if err := q.QueryRowContext(ctx, `SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'`).Scan(&n); err != nil || n == 0 {
		return 0, err
	}
	var have int
	err := q.QueryRowContext(ctx, `SELECT COALESCE(MAX(version), 0) FROM schema_migrations`).Scan(&have)
	return have, err
}

// backupV03 copies a v0.3 database (schema v1 holding rows) to
// path+BackupSuffix before it is migrated. VACUUM INTO writes one consistent
// file including rows still in the WAL, which a file copy would miss. The
// copy is retaken on every start that finds schema v1, replacing an older
// one: the migration is one transaction, so a failed run leaves the v0.3
// rows untouched, and a database rolled back to v0.3 and used for a while
// must not be paired with the backup from before that. VACUUM INTO writes
// a temporary file that is renamed into place, so a copy that dies partway
// never stands as the backup.
func backupV03(ctx context.Context, sdb *sql.DB, path string) error {
	have, err := schemaVersion(ctx, sdb)
	if err != nil || have != 1 {
		return err
	}
	var rows int
	if err := sdb.QueryRowContext(ctx, `SELECT COUNT(*) FROM events`).Scan(&rows); err != nil || rows == 0 {
		return err
	}
	dst := path + BackupSuffix
	tmp := dst + ".tmp"
	if err := os.Remove(tmp); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	if _, err := sdb.ExecContext(ctx, `VACUUM INTO ?`, tmp); err != nil {
		os.Remove(tmp)
		return fmt.Errorf("store: back up the v0.3 database to %s: %w", dst, err)
	}
	if err := os.Rename(tmp, dst); err != nil {
		os.Remove(tmp)
		return fmt.Errorf("store: back up the v0.3 database to %s: %w", dst, err)
	}
	return nil
}

func migrate(ctx context.Context, sdb *sql.DB, env migrateEnv) error {
	tx, err := sdb.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS schema_migrations (
		version    INTEGER PRIMARY KEY,
		applied_at INTEGER NOT NULL DEFAULT (unixepoch())
	)`); err != nil {
		return err
	}
	var have int
	if err := tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(version), 0) FROM schema_migrations`).Scan(&have); err != nil {
		return err
	}
	if have > len(migrations) {
		return fmt.Errorf("store: database schema v%d is newer than this build (v%d); update Irori", have, len(migrations))
	}
	for v := have; v < len(migrations); v++ {
		m := migrations[v]
		if m.sql != "" {
			if _, err := tx.ExecContext(ctx, m.sql); err != nil {
				return fmt.Errorf("store: migration %d: %w", v+1, err)
			}
		}
		if m.run != nil {
			if err := m.run(ctx, tx, env); err != nil {
				return fmt.Errorf("store: migration %d: %w", v+1, err)
			}
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO schema_migrations (version) VALUES (?)`, v+1); err != nil {
			return err
		}
	}
	return tx.Commit()
}

// backSignAndMoveLocal is the one-time v0.4 migration (§5.4): every row this
// device wrote is signed, and each device-local row moves to local_events,
// leaving its seq in events as a local.moved placeholder so seqs stay
// gapless and no policy history ever replicates. It is safe only because no
// row has left the device yet: v0.1-v0.3 had no transport.
func backSignAndMoveLocal(ctx context.Context, tx *sql.Tx, env migrateEnv) error {
	rows, err := tx.QueryContext(ctx,
		`SELECT device_id, seq, hlc, actor, kind, payload, sig FROM events WHERE device_id = ? ORDER BY seq`, env.deviceID)
	if err != nil {
		return err
	}
	evs, err := scanEvents(rows)
	if err != nil {
		return err
	}
	if len(evs) == 0 {
		return nil
	}
	if env.signer == nil {
		return errors.New("store: this log needs the v0.4 migration, which signs rows; open it with a Signer")
	}
	var localSeq int64
	for _, ev := range evs {
		if IsLocal(ev.Kind) {
			localSeq++
			if _, err := tx.ExecContext(ctx,
				`INSERT INTO local_events (device_id, seq, hlc, actor, kind, payload) VALUES (?, ?, ?, ?, ?, ?)`,
				ev.DeviceID, localSeq, int64(ev.HLC), ev.Actor, ev.Kind, []byte(ev.Payload)); err != nil {
				return err
			}
			ev.Kind, ev.Payload = KindLocalMoved, json.RawMessage(`{}`)
		}
		sig, err := env.signer.SignRow(SigningBytes(ev))
		if err != nil {
			return fmt.Errorf("sign %s/%d: %w", ev.DeviceID, ev.Seq, err)
		}
		if _, err := tx.ExecContext(ctx, `UPDATE events SET kind = ?, payload = ?, sig = ? WHERE device_id = ? AND seq = ?`,
			ev.Kind, []byte(ev.Payload), sig, ev.DeviceID, ev.Seq); err != nil {
			return err
		}
	}
	return nil
}

func (s *Store) close() error {
	d := s.impl
	d.subMu.Lock()
	if d.closed {
		d.subMu.Unlock()
		return nil
	}
	d.closed = true
	for sub := range d.subs {
		close(sub.ch)
	}
	d.subs = nil
	close(d.done)
	d.subMu.Unlock()
	return d.sql.Close()
}

func (s *Store) holdAppends(seq int64) {
	d := s.impl
	d.writeMu.Lock()
	defer d.writeMu.Unlock()
	d.holdSeq = seq
}

func (s *Store) append(ctx context.Context, actor, kind string, payload any) (Event, error) {
	d := s.impl
	if kind == "" {
		return Event{}, errors.New("store: empty kind")
	}
	if IsLocal(kind) {
		return Event{}, fmt.Errorf("%w: %s", ErrLocalKind, kind)
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		return Event{}, fmt.Errorf("store: marshal %s payload: %w", kind, err)
	}

	d.writeMu.Lock()
	defer d.writeMu.Unlock()
	if d.isClosed() {
		return Event{}, ErrClosed
	}
	tx, err := d.sql.BeginTx(ctx, nil)
	if err != nil {
		return Event{}, err
	}
	defer tx.Rollback()
	var last int64
	if err := tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(seq), 0) FROM events WHERE device_id = ?`, d.deviceID).Scan(&last); err != nil {
		return Event{}, err
	}
	if d.holdSeq > 0 {
		if last < d.holdSeq {
			return Event{}, fmt.Errorf("%w (hold %d, have %d)", ErrRestoring, d.holdSeq, last)
		}
		d.holdSeq = 0
	}
	ev := Event{DeviceID: d.deviceID, Seq: last + 1, HLC: d.clock.Now(), Actor: actor, Kind: kind, Payload: raw}
	if uint64(ev.HLC) > math.MaxInt64 {
		return Event{}, fmt.Errorf("store: clock reached hlc %d, past int64", ev.HLC)
	}
	if d.signer != nil {
		if ev.Sig, err = d.signer.SignRow(SigningBytes(ev)); err != nil {
			return Event{}, fmt.Errorf("store: sign %s: %w", kind, err)
		}
	}
	if err := insertRow(ctx, tx, "events", ev); err != nil {
		return Event{}, err
	}
	resync := false
	if d.verifier != nil && IsPersonKind(kind) {
		d.verifyMu.Lock()
		d.verifier.Observe(ev)
		d.verifyMu.Unlock()
		if resync, err = d.applyCuts(ctx, tx); err != nil {
			d.resetVerifier(ctx)
			return Event{}, err
		}
	}
	if err := tx.Commit(); err != nil {
		if d.verifier != nil && IsPersonKind(kind) {
			d.resetVerifier(ctx)
		}
		return Event{}, err
	}
	d.notify(ev)
	if resync {
		d.notifyResync()
	}
	return ev, nil
}

func (s *Store) appendLocal(ctx context.Context, actor, kind string, payload any) (Event, error) {
	d := s.impl
	if !IsLocal(kind) {
		return Event{}, fmt.Errorf("%w: %s", ErrNotLocal, kind)
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		return Event{}, fmt.Errorf("store: marshal %s payload: %w", kind, err)
	}
	d.writeMu.Lock()
	defer d.writeMu.Unlock()
	if d.isClosed() {
		return Event{}, ErrClosed
	}
	tx, err := d.sql.BeginTx(ctx, nil)
	if err != nil {
		return Event{}, err
	}
	defer tx.Rollback()
	var last int64
	if err := tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(seq), 0) FROM local_events WHERE device_id = ?`, d.deviceID).Scan(&last); err != nil {
		return Event{}, err
	}
	ev := Event{DeviceID: d.deviceID, Seq: last + 1, HLC: d.clock.Now(), Actor: actor, Kind: kind, Payload: raw}
	if err := insertRow(ctx, tx, "local_events", ev); err != nil {
		return Event{}, err
	}
	return ev, tx.Commit()
}

// checkRow is the part of Insert's validation that needs no database.
func (d *db) checkRow(ev *Event) error {
	switch {
	case ev.DeviceID == d.deviceID && d.verifier == nil:
		return ErrOwnDevice
	case ev.DeviceID == "":
		return errors.New("store: replicated row has no device id")
	case ev.Seq < 1:
		return fmt.Errorf("store: replicated row %s/%d has seq < 1", ev.DeviceID, ev.Seq)
	case ev.Kind == "":
		return fmt.Errorf("store: replicated row %s/%d has no kind", ev.DeviceID, ev.Seq)
	case ev.HLC > MaxReplicatedHLC:
		return fmt.Errorf("store: replicated row %s/%d has hlc %d past the %d ceiling", ev.DeviceID, ev.Seq, ev.HLC, MaxReplicatedHLC)
	case IsLocal(ev.Kind):
		return fmt.Errorf("%w: replicated row %s/%d is %s", ErrLocalKind, ev.DeviceID, ev.Seq, ev.Kind)
	}
	if len(ev.Payload) == 0 {
		ev.Payload = json.RawMessage("null")
	}
	if !json.Valid(ev.Payload) {
		return fmt.Errorf("store: replicated row %s/%d has a payload that is not valid JSON", ev.DeviceID, ev.Seq)
	}
	return nil
}

func (s *Store) insert(ctx context.Context, ev Event) error {
	_, err := s.insertBatch(ctx, []Event{ev})
	return err
}

func (s *Store) insertBatch(ctx context.Context, evs []Event) (int, error) {
	d := s.impl
	if len(evs) == 0 {
		return 0, nil
	}
	d.writeMu.Lock()
	defer d.writeMu.Unlock()
	if d.isClosed() {
		return 0, ErrClosed
	}
	tx, err := d.sql.BeginTx(ctx, nil)
	if err != nil {
		return 0, err
	}
	defer tx.Rollback()

	var (
		applied  int
		fresh    []Event
		observed bool
		resync   bool
		rowErr   error
	)
	for i := range evs {
		ev := evs[i]
		stored, err := d.insertOne(ctx, tx, &ev)
		if errors.Is(err, errHonouredRevoke) {
			// Kept in refused_events and folded; its cut may move rows out.
			observed = true
			r, err := d.applyCuts(ctx, tx)
			if err != nil {
				d.resetVerifier(ctx)
				return 0, err
			}
			resync = resync || r
			applied++
			continue
		}
		if err != nil {
			rowErr = err
			break
		}
		applied++
		if !stored {
			continue
		}
		fresh = append(fresh, ev)
		if d.verifier != nil && IsPersonKind(ev.Kind) {
			d.verifyMu.Lock()
			d.verifier.Observe(ev)
			d.verifyMu.Unlock()
			observed = true
			r, err := d.applyCuts(ctx, tx)
			if err != nil {
				d.resetVerifier(ctx)
				return 0, err
			}
			resync = resync || r
		}
	}
	if err := tx.Commit(); err != nil {
		if observed {
			d.resetVerifier(ctx)
		}
		return 0, err
	}
	for _, ev := range fresh {
		// Only a committed row moves the clock; refused or re-delivered copies do not.
		if d.verifier != nil {
			d.clock.ObserveBounded(ev.HLC, d.aheadBound())
		} else {
			d.clock.Observe(ev.HLC)
		}
		d.notify(ev)
	}
	if resync {
		d.notifyResync()
	}
	return applied, rowErr
}

// aheadBound is the highest HLC a replicated row may carry right now.
func (d *db) aheadBound() hlc.Timestamp {
	return hlc.Timestamp(d.now().Add(d.maxAhead).UnixMilli())<<16 | 0xffff
}

// insertOne validates and stores one replicated row inside tx. stored is
// false for an identical copy of a row already held. A fork is recorded in
// tx before ErrFork is returned, so the caller's commit keeps it.
func (d *db) insertOne(ctx context.Context, tx *sql.Tx, ev *Event) (stored bool, err error) {
	if err := d.checkRow(ev); err != nil {
		return false, err
	}
	var last, prevHLC int64
	if err := tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(seq), 0) FROM events WHERE device_id = ?`, ev.DeviceID).Scan(&last); err != nil {
		return false, err
	}
	if ev.Seq <= last {
		held, err := scanOne(tx.QueryRowContext(ctx,
			`SELECT device_id, seq, hlc, actor, kind, payload, sig FROM events WHERE device_id = ? AND seq = ?`, ev.DeviceID, ev.Seq))
		if errors.Is(err, sql.ErrNoRows) {
			return false, nil // moved to refused_events; still held as far as sync goes
		}
		if err != nil {
			return false, err
		}
		if sameRow(held, *ev) {
			return false, nil
		}
		// Only a row the device really signed proves a fork (§5.2): a junk
		// or unsigned copy from a peer or relay is refused, not recorded.
		if d.verifier != nil {
			if err := d.verify(*ev); err != nil {
				return false, err
			}
		}
		if err := recordFork(ctx, tx, *ev); err != nil {
			return false, err
		}
		return false, fmt.Errorf("%w: %s/%d", ErrFork, ev.DeviceID, ev.Seq)
	}
	if d.verifier != nil {
		// Before the gap check: a revoked device's later rows are refused
		// whether or not the rows between are held.
		if cut, ok := d.verifier.Cut(ev.DeviceID); ok && ev.Seq > cut {
			if ev.Kind == KindPersonDeviceRevoked && len(ev.Sig) > 0 {
				// A revoke is honoured if its writer was ever certified, even
				// past the writer's own cut (§3: a mutual revoke removes
				// both, in either arrival order). It can't join the gapless
				// log, so it is kept in refused_events, which the Verifier
				// also folds at Open.
				d.verifyMu.Lock()
				err := d.verifier.VerifyRow(*ev)
				d.verifyMu.Unlock()
				if err != nil {
					return false, err
				}
				if err := recordRefused(ctx, tx, *ev, refusedRevoke); err != nil {
					return false, err
				}
				d.verifyMu.Lock()
				d.verifier.Observe(*ev)
				d.verifyMu.Unlock()
				return false, errHonouredRevoke
			}
			return false, fmt.Errorf("%w: %s/%d (cut %d)", ErrPastCut, ev.DeviceID, ev.Seq, cut)
		}
	}
	if ev.Seq > last+1 {
		return false, fmt.Errorf("%w: %s holds seq %d, got %d", ErrGap, ev.DeviceID, last, ev.Seq)
	}
	if d.verifier != nil {
		if err := d.verify(*ev); err != nil {
			return false, err
		}
	}
	if last > 0 {
		if err := tx.QueryRowContext(ctx, `SELECT hlc FROM events WHERE device_id = ? AND seq = ?`, ev.DeviceID, last).Scan(&prevHLC); err != nil && !errors.Is(err, sql.ErrNoRows) {
			return false, err
		}
		if int64(ev.HLC) <= prevHLC {
			return false, fmt.Errorf("%w: %s/%d hlc %d after %d", ErrHLCOrder, ev.DeviceID, ev.Seq, ev.HLC, prevHLC)
		}
	}
	if d.verifier != nil && ev.HLC > d.aheadBound() {
		return false, fmt.Errorf("%w: %s/%d stamped %s", ErrAhead, ev.DeviceID, ev.Seq, ev.HLC.Wall().UTC().Format(time.RFC3339))
	}
	if err := insertRow(ctx, tx, "events", *ev); err != nil {
		return false, err
	}
	if d.verifier != nil {
		if err := d.checkCutClaims(ctx, tx, ev); err != nil {
			return false, err
		}
	}
	return true, nil
}

// verify checks a replicated row's signature: it must have one, and it
// must verify against its device's cert.
func (d *db) verify(ev Event) error {
	if len(ev.Sig) == 0 {
		return fmt.Errorf("%w: %s/%d", ErrUnsigned, ev.DeviceID, ev.Seq)
	}
	d.verifyMu.Lock()
	defer d.verifyMu.Unlock()
	return d.verifier.VerifyRow(ev)
}

// checkCutClaims records a fork wherever an honoured revoke's cutHash names
// another row than the one held at its cut (§5.5). With only, it checks
// that one row; otherwise every claim against the rows held in events or
// refused_events.
func (d *db) checkCutClaims(ctx context.Context, tx *sql.Tx, only *Event) error {
	d.verifyMu.Lock()
	claims := d.verifier.CutClaims()
	d.verifyMu.Unlock()
	for _, c := range claims {
		var held Event
		switch {
		case only != nil:
			if only.DeviceID != c.DeviceID || only.Seq != c.Seq {
				continue
			}
			held = *only
		default:
			ev, err := heldRow(ctx, tx, c.DeviceID, c.Seq)
			if errors.Is(err, sql.ErrNoRows) {
				continue // not held yet; checked when it arrives
			}
			if err != nil {
				return err
			}
			held = ev
		}
		if RowHash(held) == c.Hash {
			continue
		}
		if _, err := tx.ExecContext(ctx,
			`INSERT OR IGNORE INTO forks (device_id, seq, other_hash, hlc, actor, kind, reason) VALUES (?, ?, ?, 0, '', '', ?)`,
			c.DeviceID, c.Seq, c.Hash, ForkCutHash); err != nil {
			return err
		}
	}
	return nil
}

// heldRow is the row held at (dev, seq), in events or moved to refused_events.
func heldRow(ctx context.Context, tx *sql.Tx, dev string, seq int64) (Event, error) {
	ev, err := scanOne(tx.QueryRowContext(ctx,
		`SELECT device_id, seq, hlc, actor, kind, payload, sig FROM events WHERE device_id = ? AND seq = ?`, dev, seq))
	if !errors.Is(err, sql.ErrNoRows) {
		return ev, err
	}
	return scanOne(tx.QueryRowContext(ctx,
		`SELECT device_id, seq, hlc, actor, kind, payload, sig FROM refused_events WHERE device_id = ? AND seq = ?`, dev, seq))
}

func sameRow(a, b Event) bool {
	return a.HLC == b.HLC && a.Actor == b.Actor && a.Kind == b.Kind &&
		bytes.Equal(a.Payload, b.Payload) && bytes.Equal(a.Sig, b.Sig)
}

// errHonouredRevoke: insertOne kept a revoke past its writer's cut in
// refused_events and folded it (not an error for the caller).
var errHonouredRevoke = errors.New("store: revoke past cut honoured")

// Reasons in refused_events.
const (
	refusedPastCut = "past cut"
	refusedRevoke  = "revoke past cut"
)

func recordRefused(ctx context.Context, tx *sql.Tx, ev Event, reason string) error {
	_, err := tx.ExecContext(ctx,
		`INSERT OR IGNORE INTO refused_events (device_id, seq, hlc, actor, kind, payload, sig, reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
		ev.DeviceID, ev.Seq, int64(ev.HLC), ev.Actor, ev.Kind, []byte(ev.Payload), ev.Sig, reason)
	return err
}

func recordFork(ctx context.Context, tx *sql.Tx, ev Event) error {
	_, err := tx.ExecContext(ctx,
		`INSERT OR IGNORE INTO forks (device_id, seq, other_hash, hlc, actor, kind, payload, sig, reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		ev.DeviceID, ev.Seq, RowHash(ev), int64(ev.HLC), ev.Actor, ev.Kind, []byte(ev.Payload), ev.Sig, ForkTwoRows)
	return err
}

// applyCuts moves every held row above its device's effective cut from
// events to refused_events (§5.5). It reports whether any row moved.
// This device's own rows are never moved.
func (d *db) applyCuts(ctx context.Context, tx *sql.Tx) (bool, error) {
	rows, err := tx.QueryContext(ctx, `SELECT device_id, MAX(seq) FROM events GROUP BY device_id`)
	if err != nil {
		return false, err
	}
	type top struct {
		dev string
		max int64
	}
	var tops []top
	for rows.Next() {
		var t top
		if err := rows.Scan(&t.dev, &t.max); err != nil {
			rows.Close()
			return false, err
		}
		tops = append(tops, t)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return false, err
	}
	moved := false
	for _, t := range tops {
		if t.dev == d.deviceID {
			continue
		}
		d.verifyMu.Lock()
		cut, ok := d.verifier.Cut(t.dev)
		d.verifyMu.Unlock()
		if !ok || t.max <= cut {
			continue
		}
		if _, err := tx.ExecContext(ctx,
			`INSERT OR IGNORE INTO refused_events (device_id, seq, hlc, actor, kind, payload, sig, reason)
			 SELECT device_id, seq, hlc, actor, kind, payload, sig, ? FROM events WHERE device_id = ? AND seq > ?`, refusedPastCut,
			t.dev, cut); err != nil {
			return false, err
		}
		if _, err := tx.ExecContext(ctx, `DELETE FROM events WHERE device_id = ? AND seq > ?`, t.dev, cut); err != nil {
			return false, err
		}
		moved = true
	}
	return moved, d.checkCutClaims(ctx, tx, nil)
}

// loadVerifier folds every held person.* row into the Verifier.
func (d *db) loadVerifier(ctx context.Context) error {
	var after Cursor
	for {
		evs, err := (&Store{impl: d}).list(ctx, Filter{Kinds: []string{"person."}, After: after, Limit: MaxListLimit})
		if err != nil {
			return err
		}
		d.verifyMu.Lock()
		for _, ev := range evs {
			d.verifier.Observe(ev)
		}
		d.verifyMu.Unlock()
		if len(evs) < MaxListLimit {
			break
		}
		after = evs[len(evs)-1].Cursor()
	}
	// Revokes in refused_events still count: one honoured past its writer's
	// cut, and one moved there when its writer was revoked in turn (a
	// revoke counts if its writer was ever certified, §5.5).
	revs, err := (&Store{impl: d}).query(ctx, `SELECT device_id, seq, hlc, actor, kind, payload, sig FROM refused_events
		WHERE kind = ? ORDER BY hlc, device_id, seq`, KindPersonDeviceRevoked)
	if err != nil {
		return err
	}
	d.verifyMu.Lock()
	for _, ev := range revs {
		d.verifier.Observe(ev)
	}
	d.verifyMu.Unlock()
	return nil
}

// resetVerifier rebuilds the Verifier from committed rows after a write that
// observed rows failed to commit.
func (d *db) resetVerifier(ctx context.Context) {
	d.verifyMu.Lock()
	d.verifier.Reset()
	d.verifyMu.Unlock()
	_ = d.loadVerifier(ctx)
}

func (d *db) isClosed() bool {
	d.subMu.Lock()
	defer d.subMu.Unlock()
	return d.closed
}

func insertRow(ctx context.Context, tx *sql.Tx, table string, ev Event) error {
	_, err := tx.ExecContext(ctx,
		`INSERT INTO `+table+` (device_id, seq, hlc, actor, kind, payload, sig) VALUES (?, ?, ?, ?, ?, ?, ?)`,
		ev.DeviceID, ev.Seq, int64(ev.HLC), ev.Actor, ev.Kind, []byte(ev.Payload), ev.Sig)
	return err
}

func (s *Store) vector(ctx context.Context) (Vector, error) {
	rows, err := s.impl.sql.QueryContext(ctx, `SELECT device_id, MAX(seq) FROM events GROUP BY device_id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	v := Vector{}
	for rows.Next() {
		var dev string
		var seq int64
		if err := rows.Scan(&dev, &seq); err != nil {
			return nil, err
		}
		v[dev] = seq
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	// Rows past a cut are moved out, so max <= cut already; the cap also
	// covers a revoke whose rows have not been moved yet.
	if d := s.impl; d.verifier != nil {
		d.verifyMu.Lock()
		for dev, top := range v {
			if cut, ok := d.verifier.Cut(dev); ok && dev != d.deviceID && top > cut {
				v[dev] = cut
			}
		}
		d.verifyMu.Unlock()
	}
	return v, nil
}

func (s *Store) missing(ctx context.Context, v Vector, limit int) ([]Event, error) {
	if limit <= 0 {
		limit = DefaultMissingLimit
	}
	limit = min(limit, MaxMissingLimit)
	have, err := s.vector(ctx)
	if err != nil {
		return nil, err
	}
	devs := make([]string, 0, len(have))
	for dev, top := range have {
		if top > v[dev] {
			devs = append(devs, dev)
		}
	}
	sort.Strings(devs)
	var out []Event
	for _, dev := range devs {
		if len(out) >= limit {
			break
		}
		evs, err := s.query(ctx,
			`SELECT device_id, seq, hlc, actor, kind, payload, sig FROM events
			 WHERE device_id = ? AND seq > ? AND seq <= ? ORDER BY seq LIMIT ?`,
			dev, v[dev], have[dev], limit-len(out))
		if err != nil {
			return nil, err
		}
		out = append(out, evs...)
	}
	return out, nil
}

func (s *Store) list(ctx context.Context, f Filter) ([]Event, error) {
	var where []string
	var args []any
	if len(f.Kinds) > 0 {
		var or []string
		for _, k := range f.Kinds {
			if strings.HasSuffix(k, ".") {
				// Prefix as a range so the kind index applies: "agent." <= kind < "agent/".
				or = append(or, "(kind >= ? AND kind < ?)")
				args = append(args, k, k[:len(k)-1]+"/")
			} else {
				or = append(or, "kind = ?")
				args = append(args, k)
			}
		}
		where = append(where, "("+strings.Join(or, " OR ")+")")
	}
	if f.DeviceID != "" {
		where = append(where, "device_id = ?")
		args = append(args, f.DeviceID)
	}
	if f.AfterSeq > 0 {
		where = append(where, "seq > ?")
		args = append(args, f.AfterSeq)
	}
	if f.AfterHLC != 0 {
		if uint64(f.AfterHLC) > math.MaxInt64 {
			return nil, nil // nothing is stored past int64
		}
		where = append(where, "hlc > ?")
		args = append(args, int64(f.AfterHLC))
	}
	if !f.After.IsZero() {
		if uint64(f.After.HLC) > math.MaxInt64 {
			return nil, nil
		}
		h := int64(f.After.HLC)
		where = append(where, "(hlc > ? OR (hlc = ? AND (device_id > ? OR (device_id = ? AND seq > ?))))")
		args = append(args, h, h, f.After.DeviceID, f.After.DeviceID, f.After.Seq)
	}
	limit := f.Limit
	if limit <= 0 {
		limit = DefaultListLimit
	}
	limit = min(limit, MaxListLimit)

	table := "events"
	if f.Local {
		table = "local_events"
	}
	q := `SELECT device_id, seq, hlc, actor, kind, payload, sig FROM ` + table
	if len(where) > 0 {
		q += " WHERE " + strings.Join(where, " AND ")
	}
	// device_id, seq break HLC ties so the order is total and repeatable.
	if f.Desc {
		q += " ORDER BY hlc DESC, device_id DESC, seq DESC"
	} else {
		q += " ORDER BY hlc, device_id, seq"
	}
	q += " LIMIT ?"
	args = append(args, limit)
	return s.query(ctx, q, args...)
}

func (s *Store) refused(ctx context.Context, limit int) ([]Event, error) {
	if limit <= 0 {
		limit = DefaultListLimit
	}
	return s.query(ctx, `SELECT device_id, seq, hlc, actor, kind, payload, sig FROM refused_events
		ORDER BY hlc, device_id, seq LIMIT ?`, min(limit, MaxListLimit))
}

func (s *Store) forks(ctx context.Context) ([]Fork, error) {
	rows, err := s.impl.sql.QueryContext(ctx, `SELECT device_id, seq, hlc, actor, kind, payload, sig, other_hash, reason FROM forks ORDER BY device_id, seq, seen_at`)
	if err != nil {
		return nil, err
	}
	var out []Fork
	for rows.Next() {
		var f Fork
		var h int64
		var payload, sig []byte
		if err := rows.Scan(&f.Other.DeviceID, &f.Other.Seq, &h, &f.Other.Actor, &f.Other.Kind, &payload, &sig, &f.OtherHash, &f.Reason); err != nil {
			rows.Close()
			return nil, err
		}
		f.Other.HLC = hlc.Timestamp(h)
		if len(payload) > 0 {
			f.Other.Payload = json.RawMessage(payload)
		}
		if len(sig) > 0 {
			f.Other.Sig = sig
		}
		out = append(out, f)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}
	for i := range out {
		held, err := scanOne(s.impl.sql.QueryRowContext(ctx,
			`SELECT device_id, seq, hlc, actor, kind, payload, sig FROM events WHERE device_id = ? AND seq = ?`, out[i].Other.DeviceID, out[i].Other.Seq))
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return nil, err
		}
		out[i].Stored = held
	}
	return out, nil
}

func (s *Store) query(ctx context.Context, q string, args ...any) ([]Event, error) {
	rows, err := s.impl.sql.QueryContext(ctx, q, args...)
	if err != nil {
		return nil, err
	}
	return scanEvents(rows)
}

func scanEvents(rows *sql.Rows) ([]Event, error) {
	defer rows.Close()
	var out []Event
	for rows.Next() {
		var ev Event
		var h int64
		var payload, sig []byte
		if err := rows.Scan(&ev.DeviceID, &ev.Seq, &h, &ev.Actor, &ev.Kind, &payload, &sig); err != nil {
			return nil, err
		}
		ev.HLC = hlc.Timestamp(h)
		ev.Payload = json.RawMessage(payload)
		if len(sig) > 0 {
			ev.Sig = sig
		}
		out = append(out, ev)
	}
	return out, rows.Err()
}

func scanOne(row *sql.Row) (Event, error) {
	var ev Event
	var h int64
	var payload, sig []byte
	if err := row.Scan(&ev.DeviceID, &ev.Seq, &h, &ev.Actor, &ev.Kind, &payload, &sig); err != nil {
		return Event{}, err
	}
	ev.HLC = hlc.Timestamp(h)
	ev.Payload = json.RawMessage(payload)
	if len(sig) > 0 {
		ev.Sig = sig
	}
	return ev, nil
}

func (s *Store) subscribe(ctx context.Context) <-chan Event {
	d := s.impl
	sub := &subscriber{ch: make(chan Event, subBuffer)}
	d.subMu.Lock()
	if d.closed {
		d.subMu.Unlock()
		close(sub.ch)
		return sub.ch
	}
	d.subs[sub] = struct{}{}
	d.subMu.Unlock()
	go func() {
		select {
		case <-ctx.Done():
		case <-d.done:
		}
		d.subMu.Lock()
		defer d.subMu.Unlock()
		if _, ok := d.subs[sub]; ok {
			delete(d.subs, sub)
			close(sub.ch)
		}
	}()
	return sub.ch
}

// notify hands ev to every subscriber without blocking. Only notify sends, and
// it holds subMu, so len(ch) can only shrink underneath it. The last slot of
// each buffer is reserved for the resync signal: when a subscriber's buffer
// reaches it, the zero Event goes in and later rows are dropped until the
// reader drains the buffer and has a free slot again.
func (d *db) notify(ev Event) {
	d.subMu.Lock()
	defer d.subMu.Unlock()
	for sub := range d.subs {
		switch n := len(sub.ch); {
		case n < cap(sub.ch)-1:
			sub.ch <- ev
			sub.lagging = false
		case !sub.lagging:
			sub.ch <- Event{} // resync: the reader should re-List
			sub.lagging = true
		}
	}
}

// notifyResync tells every subscriber to rebuild from the log: rows it has
// already folded were moved out (§5.5).
func (d *db) notifyResync() {
	d.subMu.Lock()
	defer d.subMu.Unlock()
	for sub := range d.subs {
		if !sub.lagging && len(sub.ch) < cap(sub.ch) {
			sub.ch <- Event{}
			sub.lagging = true
		}
	}
}
