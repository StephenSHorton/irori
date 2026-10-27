package blob

import (
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"time"
	"unicode/utf8"
)

type store struct {
	dir string
}

func open(dir string) (*Store, error) {
	if dir == "" {
		return nil, errors.New("blob: empty dir")
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	return &Store{impl: &store{dir: dir}}, nil
}

// truncate cuts data longer than MaxSize back to a UTF-8 boundary at or
// below MaxSize and appends TruncatedMarker.
func truncate(data []byte) []byte {
	if len(data) <= MaxSize {
		return data
	}
	cut := MaxSize
	// Step back over at most UTFMax-1 continuation bytes so a rune is not
	// split; binary data with longer runs is cut at MaxSize as is.
	for i := 0; i < utf8.UTFMax-1 && cut > 0 && !utf8.RuneStart(data[cut]); i++ {
		cut--
	}
	if !utf8.RuneStart(data[cut]) {
		cut = MaxSize
	}
	out := make([]byte, 0, cut+len(TruncatedMarker))
	out = append(out, data[:cut]...)
	return append(out, TruncatedMarker...)
}

// validID reports whether id is 64 lowercase hex characters.
func validID(id string) bool {
	if len(id) != sha256.Size*2 {
		return false
	}
	for i := 0; i < len(id); i++ {
		c := id[i]
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return false
		}
	}
	return true
}

func (s *store) path(id string) string {
	return filepath.Join(s.dir, id[:2], id)
}

func (s *store) put(data []byte) (string, error) {
	data = truncate(data)
	sum := sha256.Sum256(data)
	id := hex.EncodeToString(sum[:])
	if s.has(id) {
		return id, nil
	}
	sub := filepath.Join(s.dir, id[:2])
	if err := os.MkdirAll(sub, 0o700); err != nil {
		return "", err
	}
	tmp, err := os.CreateTemp(sub, id+".tmp-*")
	if err != nil {
		return "", err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op once renamed; drops a losing temp
	zw := gzip.NewWriter(tmp)
	_, err = zw.Write(data)
	if err == nil {
		err = zw.Close()
	}
	if err == nil {
		err = tmp.Sync()
	}
	if cerr := tmp.Close(); err == nil {
		err = cerr
	}
	if err != nil {
		return "", err
	}
	if err := s.place(tmpName, s.path(id)); err != nil {
		return "", err
	}
	return id, nil
}

// place renames tmp onto final. Blobs are content-addressed, so a final file
// that already exists (another goroutine or process won the race, and on
// Windows may still hold it open) holds the same content and counts as
// success. It is checked before every rename so a live blob is never
// replaced: on Windows a replace makes concurrent opens of it fail. Other
// failures are retried briefly, as in discovery's renameRetry.
func (s *store) place(tmp, final string) error {
	var err error
	for wait := 5 * time.Millisecond; ; wait *= 2 {
		if regular(final) {
			return nil
		}
		if err = os.Rename(tmp, final); err == nil || regular(final) {
			return nil
		}
		if wait >= time.Second {
			return err
		}
		time.Sleep(wait)
	}
}

// openRetry opens path, retrying briefly while another writer's rename holds
// it (a Windows sharing violation; see transientOpen).
func openRetry(path string) (*os.File, error) {
	for wait := time.Millisecond; ; wait *= 2 {
		f, err := os.Open(path)
		if err == nil || !transientOpen(err) || wait >= 500*time.Millisecond {
			return f, err
		}
		time.Sleep(wait)
	}
}

func regular(path string) bool {
	fi, err := os.Stat(path)
	return err == nil && fi.Mode().IsRegular()
}

func (s *store) get(id string) ([]byte, error) {
	if !validID(id) {
		return nil, ErrBadID
	}
	f, err := openRetry(s.path(id))
	if errors.Is(err, fs.ErrNotExist) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	defer f.Close()
	// Agree with has: only a regular file is a stored blob.
	if fi, err := f.Stat(); err != nil {
		return nil, err
	} else if !fi.Mode().IsRegular() {
		return nil, ErrNotFound
	}
	zr, err := gzip.NewReader(f)
	if err != nil {
		return nil, fmt.Errorf("blob: %s is corrupt: %w", id, err)
	}
	defer zr.Close()
	// Nothing Put writes is larger than this; a bigger stream is corrupt.
	limit := int64(MaxSize + len(TruncatedMarker))
	var buf bytes.Buffer
	n, err := io.Copy(&buf, io.LimitReader(zr, limit+1))
	if err != nil {
		return nil, fmt.Errorf("blob: %s is corrupt: %w", id, err)
	}
	if n > limit {
		return nil, fmt.Errorf("blob: %s is corrupt: larger than %d bytes", id, limit)
	}
	sum := sha256.Sum256(buf.Bytes())
	if hex.EncodeToString(sum[:]) != id {
		return nil, fmt.Errorf("blob: %s is corrupt: content hash differs", id)
	}
	return buf.Bytes(), nil
}

func (s *store) has(id string) bool {
	return validID(id) && regular(s.path(id))
}
