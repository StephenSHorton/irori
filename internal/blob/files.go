package blob

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"hash"
	"io"
	"io/fs"
	"mime"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// Files is the attachment store (docs/design/v0.4-devices.md §6.6,
// decision 28): files on messages and Board cards, at
// <dir>/<first 2 hex>/<sha256 hex>, stored as they are (no compression, no
// 4 MiB cap, nothing scrubbed: it's the person's file), written once and
// never changed. The id is the SHA-256 of the file's bytes. Partial fetches
// wait in <dir>/partial/<sha256> until their length and hash check out.
type Files struct {
	dir string
	max int64
}

// MaxFileSize caps one attachment between the person's own devices
// (decision D6). A var so tests can lower it.
var MaxFileSize int64 = 256 << 20

// FileRef is an attachment as a message or a comment carries it (§6.6).
// Device is where it was attached; Name comes from that device and is only
// a hint until SafeName has reduced it.
type FileRef struct {
	SHA256 string `json:"sha256"`
	Name   string `json:"name"`
	Size   int64  `json:"size"`
	Mime   string `json:"mime,omitempty"`
	Device string `json:"device"`
}

var (
	ErrTooLarge = errors.New("blob: the file is larger than the attachment cap")
	ErrNotFile  = errors.New("blob: not a regular file")
	ErrMismatch = errors.New("blob: the bytes don't match the attachment's length and hash")
)

// OpenFiles creates dir (0700) if needed.
func OpenFiles(dir string) (*Files, error) {
	if dir == "" {
		return nil, errors.New("blob: empty dir")
	}
	if err := os.MkdirAll(filepath.Join(dir, "partial"), 0o700); err != nil {
		return nil, err
	}
	return &Files{dir: dir, max: MaxFileSize}, nil
}

// Max is the size cap this store keeps.
func (f *Files) Max() int64 { return f.max }

func (f *Files) path(id string) string { return filepath.Join(f.dir, id[:2], id) }

// Has reports whether an attachment is stored.
func (f *Files) Has(id string) bool { return validID(id) && regular(f.path(id)) }

// Size is a stored attachment's size.
func (f *Files) Size(id string) (int64, error) {
	if !validID(id) {
		return 0, ErrBadID
	}
	fi, err := os.Stat(f.path(id))
	if errors.Is(err, fs.ErrNotExist) || err == nil && !fi.Mode().IsRegular() {
		return 0, ErrNotFound
	}
	if err != nil {
		return 0, err
	}
	return fi.Size(), nil
}

// Open opens a stored attachment for reading (ErrBadID, ErrNotFound).
func (f *Files) Open(id string) (*os.File, error) {
	if !validID(id) {
		return nil, ErrBadID
	}
	fh, err := openRetry(f.path(id))
	if errors.Is(err, fs.ErrNotExist) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	if fi, err := fh.Stat(); err != nil || !fi.Mode().IsRegular() {
		fh.Close()
		return nil, ErrNotFound
	}
	return fh, nil
}

// PutFile copies the regular file at path into the store and returns its
// ref (Device unset). Larger than the cap is ErrTooLarge, before and while
// copying, so a file that grows meanwhile is still refused.
func (f *Files) PutFile(path string) (FileRef, error) {
	src, err := os.Open(path)
	if err != nil {
		return FileRef{}, err
	}
	defer src.Close()
	fi, err := src.Stat()
	if err != nil {
		return FileRef{}, err
	}
	if !fi.Mode().IsRegular() {
		return FileRef{}, ErrNotFile
	}
	if fi.Size() > f.max {
		return FileRef{}, fmt.Errorf("%w (%d bytes, at most %d)", ErrTooLarge, fi.Size(), f.max)
	}
	tmp, err := os.CreateTemp(filepath.Join(f.dir, "partial"), "put-*")
	if err != nil {
		return FileRef{}, err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op once renamed
	h := sha256.New()
	head := &headBuf{}
	n, err := io.Copy(io.MultiWriter(tmp, h, head), io.LimitReader(src, f.max+1))
	if err == nil && n > f.max {
		err = fmt.Errorf("%w (at most %d bytes)", ErrTooLarge, f.max)
	}
	if err == nil {
		err = tmp.Sync()
	}
	if cerr := tmp.Close(); err == nil {
		err = cerr
	}
	if err != nil {
		return FileRef{}, err
	}
	id := hex.EncodeToString(h.Sum(nil))
	if err := f.placeFile(tmpName, id); err != nil {
		return FileRef{}, err
	}
	return FileRef{SHA256: id, Name: filepath.Base(path), Size: n, Mime: mimeOf(path, head.b)}, nil
}

func (f *Files) placeFile(tmp, id string) error {
	if err := os.MkdirAll(filepath.Join(f.dir, id[:2]), 0o700); err != nil {
		return err
	}
	s := &store{dir: f.dir}
	return s.place(tmp, f.path(id))
}

// Partial opens the partial file of a fetch of id for appending and
// returns how many bytes it already holds, so a fetch resumes by offset.
func (f *Files) Partial(id string) (*os.File, int64, error) {
	if !validID(id) {
		return nil, 0, ErrBadID
	}
	fh, err := os.OpenFile(f.partialPath(id), os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, 0, err
	}
	off, err := fh.Seek(0, io.SeekEnd)
	if err != nil {
		fh.Close()
		return nil, 0, err
	}
	return fh, off, nil
}

func (f *Files) partialPath(id string) string { return filepath.Join(f.dir, "partial", id) }

// Finish checks a fetch's partial file against the ref's length and hash
// and moves it into the store. A mismatch (a stream cut by a dying
// connection can look like a clean EOF) leaves a short file to resume and
// drops a wrong or overlong one.
func (f *Files) Finish(id string, size int64) error {
	p := f.partialPath(id)
	fi, err := os.Stat(p)
	if err != nil {
		return err
	}
	if fi.Size() < size {
		return fmt.Errorf("%w: %d of %d bytes", ErrMismatch, fi.Size(), size)
	}
	fh, err := os.Open(p)
	if err != nil {
		return err
	}
	h := sha256.New()
	_, err = io.Copy(h, fh)
	fh.Close()
	if err != nil {
		return err
	}
	if fi.Size() != size || hex.EncodeToString(h.Sum(nil)) != id {
		os.Remove(p)
		return ErrMismatch
	}
	return f.placeFile(p, id)
}

// DropPartial removes a fetch's partial file.
func (f *Files) DropPartial(id string) {
	if validID(id) {
		os.Remove(f.partialPath(id))
	}
}

// CopyOut writes a stored attachment to dst, which must not exist yet: a
// row never overwrites a file (§6.6). The copy is checked against id.
func (f *Files) CopyOut(id, dst string) error {
	src, err := f.Open(id)
	if err != nil {
		return err
	}
	defer src.Close()
	if err := os.MkdirAll(filepath.Dir(dst), 0o700); err != nil {
		return err
	}
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	h := sha256.New()
	_, err = io.Copy(io.MultiWriter(out, h), src)
	if cerr := out.Close(); err == nil {
		err = cerr
	}
	if err == nil && hex.EncodeToString(h.Sum(nil)) != id {
		err = ErrMismatch
	}
	if err != nil {
		os.Remove(dst)
	}
	return err
}

// SameContent reports whether the file at path already holds id's bytes.
func SameContent(path, id string) bool {
	fh, err := os.Open(path)
	if err != nil {
		return false
	}
	defer fh.Close()
	var h hash.Hash = sha256.New()
	if _, err := io.Copy(h, fh); err != nil {
		return false
	}
	return hex.EncodeToString(h.Sum(nil)) == id
}

// ValidID reports whether id is 64 lowercase hex characters.
func ValidID(id string) bool { return validID(id) }

// headBuf keeps the first 512 bytes written, for content sniffing.
type headBuf struct{ b []byte }

func (h *headBuf) Write(p []byte) (int, error) {
	if room := 512 - len(h.b); room > 0 {
		h.b = append(h.b, p[:min(room, len(p))]...)
	}
	return len(p), nil
}

// mimeOf is the type by extension, else by content.
func mimeOf(path string, head []byte) string {
	if t := mime.TypeByExtension(strings.ToLower(filepath.Ext(path))); t != "" {
		return t
	}
	return http.DetectContentType(head)
}
