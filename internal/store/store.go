// Package store is Irori's per-device event log: one SQLite database
// (modernc.org/sqlite, pure Go, WAL) holding append-only rows
// (device_id, seq, hlc, actor, kind, payload, sig).
//
// Rows never change once written, so replicas never conflict: devices swap
// {device_id: max seq} vectors and send each other the rows the other lacks
// (v0.4). Views (the roster, timelines) are rebuilt from the log.
//
// v0.4 (design/v0.4-devices.md §5): every row this device appends is signed
// by the device key (a Signer), and every replicated row is verified before
// it is stored (a Verifier: the cert chain to the pinned root, signatures and
// revocation cuts). Kinds that never leave the device live in a second table,
// local_events (AppendLocal; List with Filter.Local). The store itself holds
// no keys and does no ed25519: it takes both interfaces at Open.
//
// CONTRACT (implemented by the store work package; keep these signatures):
package store

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"time"

	"crypto/sha256"

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
	// Sig is the writing device's ed25519 signature over SigningBytes. It is
	// kept out of JSON so /v1/events output is unchanged.
	Sig []byte `json:"-"`
}

// Cursor is a position in the log's total (hlc, device_id, seq) order.
func (ev Event) Cursor() Cursor { return Cursor{HLC: ev.HLC, DeviceID: ev.DeviceID, Seq: ev.Seq} }

// Cursor pages List without dropping rows that two devices wrote at the same
// HLC (AfterHLC alone would skip them at a page boundary).
type Cursor struct {
	HLC      hlc.Timestamp
	DeviceID string
	Seq      int64
}

// IsZero reports an unset cursor.
func (c Cursor) IsZero() bool { return c == Cursor{} }

// Vector maps device id to the highest seq held for it.
type Vector map[string]int64

// Filter narrows List. Zero values mean "any".
type Filter struct {
	Kinds    []string // exact kinds, or prefixes ending in "." ("agent.")
	DeviceID string
	AfterHLC hlc.Timestamp // strictly after
	// After returns rows strictly after this position in (hlc, device_id,
	// seq) order. Use it, not AfterHLC, to page.
	After    Cursor
	AfterSeq int64 // seq strictly after; meant with DeviceID (one device's rows)
	Limit    int   // default 200, max 1000
	Desc     bool  // newest first
	// Local reads local_events (device-local kinds, §5.1) instead of events.
	Local bool
}

// Signer signs this device's rows. msg is SigningBytes(ev).
type Signer interface {
	SignRow(msg []byte) ([]byte, error)
}

// Verifier checks rows from other devices (or this device's own rows coming
// back from a peer) and keeps the cert fold current. internal/identity's
// Chain implements it.
type Verifier interface {
	// VerifyRow reports whether ev is signed by a device certified to this
	// device's person. It returns an error wrapping ErrNoCert when the
	// device has no cert yet, ErrBadSig when the signature fails.
	VerifyRow(ev Event) error
	// Cut is the effective revocation cut of deviceID: rows above it are
	// refused. ok is false for a device that is not revoked.
	Cut(deviceID string) (cut int64, ok bool)
	// Observe folds one committed person.* row. The store calls it for every
	// such row it holds at Open, then for each one appended or inserted.
	Observe(ev Event)
	// Reset forgets everything observed (the store replays after a failed
	// batch commit).
	Reset()
}

// OpenOptions configure OpenWith. Zero values give an unsigned, unverified
// store (tests and tools that read a log).
type OpenOptions struct {
	// Signer signs every appended row and back-signs a v0.3 log during the
	// one-time migration. With a Signer, an unsigned own row found at Open
	// is an error, not something to fix silently.
	Signer Signer
	// Verifier checks replicated rows. With a Verifier, Insert also holds
	// rows stamped more than MaxAhead past local wall time (ErrAhead) and
	// accepts this device's own rows back when they verify.
	Verifier Verifier
	// MaxAhead bounds how far past wall time a replicated row's HLC may be
	// (default 2 minutes; only applied with a Verifier).
	MaxAhead time.Duration
	// Now is the wall clock for MaxAhead (time.Now when nil).
	Now func() time.Time
}

// DefaultMaxAhead is OpenOptions.MaxAhead's default (§5.6).
const DefaultMaxAhead = 2 * time.Minute

// Errors Insert and Append report. Match them with errors.Is.
var (
	// ErrLocalKind: a device-local kind (IsLocal) given to Append or Insert.
	ErrLocalKind = errors.New("store: device-local kind; use AppendLocal")
	// ErrNotLocal: AppendLocal given a kind that replicates.
	ErrNotLocal = errors.New("store: kind replicates; use Append")
	// ErrNoCert: the row's device has no cert yet. Hold the row and retry
	// once a cert arrives; it is not a forgery.
	ErrNoCert = errors.New("store: row's device is not certified")
	// ErrBadSig: the signature does not verify against the device's cert.
	ErrBadSig = errors.New("store: row signature does not verify")
	// ErrUnsigned: a replicated row with no signature.
	ErrUnsigned = errors.New("store: row is unsigned")
	// ErrPastCut: the row is above its device's revocation cut.
	ErrPastCut = errors.New("store: row is past its device's revocation cut")
	// ErrFork: a row for a (device, seq) already held with different signed
	// bytes. Both rows are kept (Forks); stop that device's stream.
	ErrFork = errors.New("store: device signed two different rows for one seq")
	// ErrHLCOrder: the row's HLC is not above its device's previous row's.
	// Treated like a fork: stop that device's stream.
	ErrHLCOrder = errors.New("store: row's hlc does not follow its device's previous row")
	// ErrAhead: the row is stamped too far past local wall time. It is not
	// stored yet; retry once wall time catches up.
	ErrAhead = errors.New("store: row is stamped too far in the future")
	// ErrRestoring: Append is blocked until this device's own rows come back
	// from a peer (HoldAppends).
	ErrRestoring = errors.New("store: restoring this device's rows; appends wait")
)

// Fork is one row that conflicts with a stored row at the same (device, seq).
type Fork struct {
	Stored Event `json:"stored"`
	Other  Event `json:"other"`
}

// Store is the event log. Safe for concurrent use.
type Store struct {
	impl *db
}

// Open opens (creating and migrating) the database at path for this device,
// unsigned and unverified. The clock stamps rows this device appends.
func Open(ctx context.Context, path, deviceID string, clock *hlc.Clock) (*Store, error) {
	return open(ctx, path, deviceID, clock, OpenOptions{})
}

// OpenWith is Open with a Signer and Verifier (the Core's path). A v0.3
// database (schema v1) is first copied to path+".v0.3", then migrated: own
// rows back-signed and device-local kinds moved to local_events (§5.4).
func OpenWith(ctx context.Context, path, deviceID string, clock *hlc.Clock, o OpenOptions) (*Store, error) {
	return open(ctx, path, deviceID, clock, o)
}

// Close closes the database.
func (s *Store) Close() error { return s.close() }

// DeviceID is the device this store appends as.
func (s *Store) DeviceID() string { return s.impl.deviceID }

// Append writes a new row for this device with the next seq and a fresh HLC.
// payload is marshalled with encoding/json. Subscribers are notified after
// commit. A device-local kind is refused (ErrLocalKind).
func (s *Store) Append(ctx context.Context, actor, kind string, payload any) (Event, error) {
	return s.append(ctx, actor, kind, payload)
}

// AppendLocal writes a device-local row (IsLocal) to local_events, with its
// own gapless seq. Vector, Missing and Insert never see these rows, and
// Subscribe does not deliver them.
func (s *Store) AppendLocal(ctx context.Context, actor, kind string, payload any) (Event, error) {
	return s.appendLocal(ctx, actor, kind, payload)
}

// Insert stores a row replicated from another device. It is idempotent on
// (DeviceID, Seq) for identical signed bytes, observes ev.HLC on the clock
// once the row commits, and refuses HLCs past MaxReplicatedHLC, invalid
// JSON, local kinds, a seq gap, an HLC behind the device's previous row and
// (ErrFork) a different row at a held seq. With a Verifier it also refuses
// unsigned, uncertified and past-cut rows and holds far-future ones; without
// one, this device's own id is refused (ErrOwnDevice).
func (s *Store) Insert(ctx context.Context, ev Event) error { return s.insert(ctx, ev) }

// InsertBatch inserts evs in order in one transaction. It stops at the first
// row that is refused, commits the rows before it, and reports how many
// were applied (already-held duplicates count as applied).
func (s *Store) InsertBatch(ctx context.Context, evs []Event) (int, error) {
	return s.insertBatch(ctx, evs)
}

// HoldAppends blocks Append (ErrRestoring) until this device holds its own
// rows up to seq (they come back from a peer through Insert). seq <= the
// held max clears the hold.
func (s *Store) HoldAppends(seq int64) { s.holdAppends(seq) }

// Vector reports the max seq held per device.
func (s *Store) Vector(ctx context.Context) (Vector, error) { return s.vector(ctx) }

// Missing returns rows the holder of v lacks, ordered by (device_id, seq), up to limit.
func (s *Store) Missing(ctx context.Context, v Vector, limit int) ([]Event, error) {
	return s.missing(ctx, v, limit)
}

// List returns rows matching f, ordered by HLC.
func (s *Store) List(ctx context.Context, f Filter) ([]Event, error) { return s.list(ctx, f) }

// Refused lists rows moved out of the log because they were past their
// device's revocation cut (§5.5), oldest first.
func (s *Store) Refused(ctx context.Context, limit int) ([]Event, error) {
	return s.refused(ctx, limit)
}

// Forks lists every conflicting row recorded by Insert (§5.2).
func (s *Store) Forks(ctx context.Context) ([]Fork, error) { return s.forks(ctx) }

// Subscribe delivers every row appended or inserted after the call until ctx ends.
// A slow subscriber drops rows rather than blocking writers; it then gets a
// zero Event (Seq == 0) as a "resync" signal. Rows moved to refused_events
// also send the resync signal, so folds rebuild without them.
func (s *Store) Subscribe(ctx context.Context) <-chan Event { return s.subscribe(ctx) }

// SigningBytes is what a row's signature covers (§5.3):
//
//	"irori/row/1" || lp(device_id) || u64(seq) || u64(hlc) || lp(actor) || lp(kind) || sha256(payload)
//
// where lp is a 4-byte big-endian length then the bytes, and payload is the
// exact stored bytes. The actor is covered so a forwarder can't turn an
// agent's row into a "user" row.
func SigningBytes(ev Event) []byte {
	sum := sha256.Sum256(ev.Payload)
	out := make([]byte, 0, 64+len(ev.DeviceID)+len(ev.Actor)+len(ev.Kind))
	out = append(out, "irori/row/1"...)
	lp := func(s string) {
		out = binary.BigEndian.AppendUint32(out, uint32(len(s)))
		out = append(out, s...)
	}
	lp(ev.DeviceID)
	out = binary.BigEndian.AppendUint64(out, uint64(ev.Seq))
	out = binary.BigEndian.AppendUint64(out, uint64(ev.HLC))
	lp(ev.Actor)
	lp(ev.Kind)
	return append(out, sum[:]...)
}
