// Package store is Irori's per-device event log: one SQLite database
// (modernc.org/sqlite, pure Go, WAL) holding append-only rows
// (device_id, seq, hlc, actor, kind, payload).
//
// Rows never change once written, so replicas never conflict: devices swap
// {device_id: max seq} vectors and send each other the rows the other lacks
// (v0.4). Views (the roster, timelines) are rebuilt from the log.
//
// CONTRACT (implemented by the store work package; keep these signatures):
package store

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/StephenSHorton/irori/internal/hlc"
)

// ErrUnimplemented is returned by skeleton stubs.
var ErrUnimplemented = errors.New("store: not implemented yet")

// Event is one log row. (DeviceID, Seq) is the primary key.
type Event struct {
	DeviceID string          `json:"deviceId"`
	Seq      int64           `json:"seq"` // 1-based, gapless per device
	HLC      hlc.Timestamp   `json:"hlc"`
	Actor    string          `json:"actor"` // who caused it: "irori", "agent:<device>/<sessionId>", "user"
	Kind     string          `json:"kind"`  // dotted, e.g. "agent.seen"; see kinds.go
	Payload  json.RawMessage `json:"payload"`
}

// Vector maps device id to the highest seq held for it.
type Vector map[string]int64

// Filter narrows List. Zero values mean "any".
type Filter struct {
	Kinds    []string // exact kinds, or prefixes ending in "." ("agent.")
	DeviceID string
	AfterHLC hlc.Timestamp // strictly after
	AfterSeq int64         // seq strictly after; meant with DeviceID (one device's rows)
	Limit    int           // default 200, max 1000
	Desc     bool          // newest first
}

// Store is the event log. Safe for concurrent use.
type Store struct {
	impl *db
}

// Open opens (creating and migrating) the database at path for this device.
// The clock stamps rows this device appends.
func Open(ctx context.Context, path, deviceID string, clock *hlc.Clock) (*Store, error) {
	return open(ctx, path, deviceID, clock)
}

// Close closes the database.
func (s *Store) Close() error { return s.close() }

// Append writes a new row for this device with the next seq and a fresh HLC.
// payload is marshalled with encoding/json. Subscribers are notified after commit.
func (s *Store) Append(ctx context.Context, actor, kind string, payload any) (Event, error) {
	return s.append(ctx, actor, kind, payload)
}

// Insert stores a row replicated from another device. It is idempotent on
// (DeviceID, Seq), observes ev.HLC on the clock once the row commits, and
// refuses this device's own id, HLCs past MaxReplicatedHLC and invalid JSON.
func (s *Store) Insert(ctx context.Context, ev Event) error { return s.insert(ctx, ev) }

// Vector reports the max seq held per device.
func (s *Store) Vector(ctx context.Context) (Vector, error) { return s.vector(ctx) }

// Missing returns rows the holder of v lacks, ordered by (device_id, seq), up to limit.
func (s *Store) Missing(ctx context.Context, v Vector, limit int) ([]Event, error) {
	return s.missing(ctx, v, limit)
}

// List returns rows matching f, ordered by HLC.
func (s *Store) List(ctx context.Context, f Filter) ([]Event, error) { return s.list(ctx, f) }

// Subscribe delivers every row appended or inserted after the call until ctx ends.
// A slow subscriber drops rows rather than blocking writers; it then gets a
// zero Event (Seq == 0) as a "resync" signal.
func (s *Store) Subscribe(ctx context.Context) <-chan Event { return s.subscribe(ctx) }
