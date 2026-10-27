// Package blob is capture's content-addressed store for patches and
// compaction summaries (docs/design/v0.3-capture.md §4). Blobs live outside
// the event log, at <dir>/<first 2 hex>/<sha256 hex>, gzip-compressed,
// written once (temp file in the same dir, then rename) and never changed.
// The id is the SHA-256 of the stored (possibly truncated) content, before
// compression.
//
// CONTRACT (implemented by the blob work package; keep these signatures):
package blob

import "errors"

// MaxSize is the largest blob kept. Put truncates longer data to MaxSize
// bytes (cut back to a UTF-8 boundary) and appends TruncatedMarker.
const MaxSize = 4 << 20

// TruncatedMarker ends a truncated blob.
const TruncatedMarker = "\n[irori: truncated]\n"

var (
	ErrNotFound = errors.New("blob: not found")
	ErrBadID    = errors.New("blob: id must be 64 lowercase hex characters")
)

// Store is safe for concurrent use, including two processes putting the same
// blob at once.
type Store struct {
	impl *store
}

// Open creates dir (0700) if needed.
func Open(dir string) (*Store, error) { return open(dir) }

// Put stores data and returns its id. Putting the same content again is a
// no-op returning the same id.
func (s *Store) Put(data []byte) (string, error) { return s.impl.put(data) }

// Get returns a blob's content, ErrBadID for a malformed id (never touching
// the filesystem with it), ErrNotFound when absent.
func (s *Store) Get(id string) ([]byte, error) { return s.impl.get(id) }

// Has reports whether a blob is stored.
func (s *Store) Has(id string) bool { return s.impl.has(id) }
