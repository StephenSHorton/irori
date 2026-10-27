package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/StephenSHorton/irori/internal/hlc"

	_ "modernc.org/sqlite" // registers the "sqlite" driver
)

// HLC storage: SQLite integers are int64 and hlc.Timestamp is uint64. Rows store
// int64(ts), which keeps ordering as long as the top bit is clear. The wall part
// is milliseconds<<16, so the top bit stays clear until 2^47 ms after 1970, a few
// thousand years out. Insert refuses anything larger rather than wrap it.

// Errors callers may want to match.
var (
	// ErrOwnDevice: Insert was handed a row carrying this device's id.
	ErrOwnDevice = errors.New("store: refusing to insert a row for this device's own id; use Append")
	// ErrGap: Insert was handed a row past the next expected seq for its device,
	// which would leave a hole that Vector could not describe.
	ErrGap = errors.New("store: replicated row leaves a seq gap")
	// ErrClosed: the store was closed.
	ErrClosed = errors.New("store: closed")
)

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

// migrations[i] takes the schema from version i to i+1. Append only.
var migrations = []string{
	`CREATE TABLE events (
		device_id TEXT    NOT NULL,
		seq       INTEGER NOT NULL,
		hlc       INTEGER NOT NULL,
		actor     TEXT    NOT NULL,
		kind      TEXT    NOT NULL,
		payload   BLOB,
		PRIMARY KEY (device_id, seq)
	) WITHOUT ROWID;
	CREATE INDEX events_hlc ON events (hlc);
	CREATE INDEX events_kind ON events (kind, hlc);`,
}

type db struct {
	sql      *sql.DB
	deviceID string
	clock    *hlc.Clock

	// writeMu serializes every write and its notification, so subscribers see
	// rows in commit order and own seqs follow own HLCs.
	writeMu sync.Mutex

	subMu  sync.Mutex
	subs   map[*subscriber]struct{}
	closed bool
}

type subscriber struct {
	ch      chan Event
	lagging bool // a resync signal is queued and not yet followed by a delivery
}

func open(ctx context.Context, path, deviceID string, clock *hlc.Clock) (*Store, error) {
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
	if err := migrate(ctx, sdb); err != nil {
		sdb.Close()
		return nil, err
	}
	return &Store{impl: &db{sql: sdb, deviceID: deviceID, clock: clock, subs: map[*subscriber]struct{}{}}}, nil
}

// SchemaVersion is the schema version this build writes.
func SchemaVersion() int { return len(migrations) }

func migrate(ctx context.Context, sdb *sql.DB) error {
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
		if _, err := tx.ExecContext(ctx, migrations[v]); err != nil {
			return fmt.Errorf("store: migration %d: %w", v+1, err)
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO schema_migrations (version) VALUES (?)`, v+1); err != nil {
			return err
		}
	}
	return tx.Commit()
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
	d.subMu.Unlock()
	return d.sql.Close()
}

func (s *Store) append(ctx context.Context, actor, kind string, payload any) (Event, error) {
	d := s.impl
	if kind == "" {
		return Event{}, errors.New("store: empty kind")
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
	ev := Event{DeviceID: d.deviceID, Seq: last + 1, HLC: d.clock.Now(), Actor: actor, Kind: kind, Payload: raw}
	if uint64(ev.HLC) > math.MaxInt64 {
		return Event{}, fmt.Errorf("store: clock reached hlc %d, past int64", ev.HLC)
	}
	if err := insertRow(ctx, tx, ev); err != nil {
		return Event{}, err
	}
	if err := tx.Commit(); err != nil {
		return Event{}, err
	}
	d.notify(ev)
	return ev, nil
}

func (s *Store) insert(ctx context.Context, ev Event) error {
	d := s.impl
	switch {
	case ev.DeviceID == d.deviceID:
		return ErrOwnDevice
	case ev.DeviceID == "":
		return errors.New("store: replicated row has no device id")
	case ev.Seq < 1:
		return fmt.Errorf("store: replicated row %s/%d has seq < 1", ev.DeviceID, ev.Seq)
	case ev.Kind == "":
		return fmt.Errorf("store: replicated row %s/%d has no kind", ev.DeviceID, ev.Seq)
	case uint64(ev.HLC) > math.MaxInt64:
		return fmt.Errorf("store: replicated row %s/%d has hlc %d past int64", ev.DeviceID, ev.Seq, ev.HLC)
	}
	if len(ev.Payload) == 0 {
		ev.Payload = json.RawMessage("null")
	}
	d.clock.Observe(ev.HLC)

	d.writeMu.Lock()
	defer d.writeMu.Unlock()
	if d.isClosed() {
		return ErrClosed
	}
	tx, err := d.sql.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var last int64
	if err := tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(seq), 0) FROM events WHERE device_id = ?`, ev.DeviceID).Scan(&last); err != nil {
		return err
	}
	switch {
	case ev.Seq <= last:
		return nil // already held; rows never change
	case ev.Seq > last+1:
		return fmt.Errorf("%w: %s holds seq %d, got %d", ErrGap, ev.DeviceID, last, ev.Seq)
	}
	if err := insertRow(ctx, tx, ev); err != nil {
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	d.notify(ev)
	return nil
}

func (d *db) isClosed() bool {
	d.subMu.Lock()
	defer d.subMu.Unlock()
	return d.closed
}

func insertRow(ctx context.Context, tx *sql.Tx, ev Event) error {
	_, err := tx.ExecContext(ctx,
		`INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES (?, ?, ?, ?, ?, ?)`,
		ev.DeviceID, ev.Seq, int64(ev.HLC), ev.Actor, ev.Kind, []byte(ev.Payload))
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
	return v, rows.Err()
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
			`SELECT device_id, seq, hlc, actor, kind, payload FROM events
			 WHERE device_id = ? AND seq > ? ORDER BY seq LIMIT ?`,
			dev, v[dev], limit-len(out))
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
	if f.AfterHLC != 0 {
		if uint64(f.AfterHLC) > math.MaxInt64 {
			return nil, nil // nothing is stored past int64
		}
		where = append(where, "hlc > ?")
		args = append(args, int64(f.AfterHLC))
	}
	limit := f.Limit
	if limit <= 0 {
		limit = DefaultListLimit
	}
	limit = min(limit, MaxListLimit)

	q := `SELECT device_id, seq, hlc, actor, kind, payload FROM events`
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

func (s *Store) query(ctx context.Context, q string, args ...any) ([]Event, error) {
	rows, err := s.impl.sql.QueryContext(ctx, q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []Event
	for rows.Next() {
		var ev Event
		var h int64
		var payload []byte
		if err := rows.Scan(&ev.DeviceID, &ev.Seq, &h, &ev.Actor, &ev.Kind, &payload); err != nil {
			return nil, err
		}
		ev.HLC = hlc.Timestamp(h)
		ev.Payload = json.RawMessage(payload)
		out = append(out, ev)
	}
	return out, rows.Err()
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
		<-ctx.Done()
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
