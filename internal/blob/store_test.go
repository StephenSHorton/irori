package blob

import (
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"unicode/utf8"
)

func newStore(t *testing.T) (*Store, string) {
	t.Helper()
	dir := filepath.Join(t.TempDir(), "blobs")
	s, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	return s, dir
}

func sha(b []byte) string {
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}

func TestOpenCreatesDir(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "a", "b", "blobs")
	if _, err := Open(dir); err != nil {
		t.Fatal(err)
	}
	if fi, err := os.Stat(dir); err != nil || !fi.IsDir() {
		t.Fatalf("dir not created: %v", err)
	}
	if _, err := Open(dir); err != nil {
		t.Fatalf("reopen: %v", err)
	}
	if _, err := Open(""); err == nil {
		t.Fatal("empty dir accepted")
	}
}

func TestPutGetRoundTrip(t *testing.T) {
	s, dir := newStore(t)
	tests := []struct {
		name string
		data []byte
	}{
		{"empty", nil},
		{"text", []byte("--- a/x.go\n+++ b/x.go\n@@ -1 +1 @@\n-a\n+b\n")},
		{"crlf", []byte("line one\r\nline two\r\n")},
		{"binary", []byte{0, 1, 2, 0xff, 0xfe, 0x80}},
		{"unicode", []byte("囲炉裏 summary ✓\n")},
		{"exactly max", bytes.Repeat([]byte("x"), MaxSize)},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			id, err := s.Put(tt.data)
			if err != nil {
				t.Fatal(err)
			}
			if id != sha(tt.data) {
				t.Fatalf("id %s, want sha256 of content %s", id, sha(tt.data))
			}
			if !s.Has(id) {
				t.Fatal("Has = false after Put")
			}
			if _, err := os.Stat(filepath.Join(dir, id[:2], id)); err != nil {
				t.Fatalf("not at <dir>/<aa>/<id>: %v", err)
			}
			got, err := s.Get(id)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(got, tt.data) {
				t.Fatalf("got %d bytes back, want %d", len(got), len(tt.data))
			}
		})
	}
}

func TestStoredGzipped(t *testing.T) {
	s, dir := newStore(t)
	data := []byte(strings.Repeat("hello blob\n", 1000))
	id, err := s.Put(data)
	if err != nil {
		t.Fatal(err)
	}
	f, err := os.Open(filepath.Join(dir, id[:2], id))
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	zr, err := gzip.NewReader(f)
	if err != nil {
		t.Fatalf("file is not gzip: %v", err)
	}
	var buf bytes.Buffer
	if _, err := buf.ReadFrom(zr); err != nil || !bytes.Equal(buf.Bytes(), data) {
		t.Fatalf("gunzipped content differs: %v", err)
	}
}

func TestPutIdempotent(t *testing.T) {
	s, dir := newStore(t)
	data := []byte("same content")
	a, err := s.Put(data)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, a[:2], a)
	before, _ := os.Stat(path)
	b, err := s.Put(data)
	if err != nil || a != b {
		t.Fatalf("second put %s, %v; want %s", b, err, a)
	}
	after, _ := os.Stat(path)
	if !after.ModTime().Equal(before.ModTime()) {
		t.Fatal("second put rewrote the file")
	}
	assertNoTemps(t, dir)
}

func TestDistinctContentDistinctIDs(t *testing.T) {
	s, _ := newStore(t)
	a, _ := s.Put([]byte("a\n"))
	b, _ := s.Put([]byte("a\r\n"))
	if a == b {
		t.Fatal("LF and CRLF content got the same id")
	}
}

func TestTruncation(t *testing.T) {
	ascii := bytes.Repeat([]byte("a"), MaxSize+100)
	// A 3-byte rune straddling MaxSize: its first byte at MaxSize-1.
	straddle := append(bytes.Repeat([]byte("a"), MaxSize-1), []byte("€€€")...)
	// A 4-byte rune ending exactly at MaxSize: no cut-back needed.
	exact := append(bytes.Repeat([]byte("a"), MaxSize-4), []byte("😀tail")...)
	// Continuation bytes only (not UTF-8): cut at MaxSize as is.
	junk := bytes.Repeat([]byte{0x80}, MaxSize+10)
	tests := []struct {
		name    string
		data    []byte
		wantCut int
	}{
		{"one over", bytes.Repeat([]byte("b"), MaxSize+1), MaxSize},
		{"ascii", ascii, MaxSize},
		{"rune straddles", straddle, MaxSize - 1},
		{"rune ends at max", exact, MaxSize},
		{"invalid utf8", junk, MaxSize},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			s, _ := newStore(t)
			id, err := s.Put(tt.data)
			if err != nil {
				t.Fatal(err)
			}
			got, err := s.Get(id)
			if err != nil {
				t.Fatal(err)
			}
			want := append(append([]byte{}, tt.data[:tt.wantCut]...), TruncatedMarker...)
			if !bytes.Equal(got, want) {
				t.Fatalf("got %d bytes, want %d (cut %d + marker)", len(got), len(want), tt.wantCut)
			}
			if id != sha(want) {
				t.Fatal("id is not the sha256 of the truncated content")
			}
			if tt.name != "invalid utf8" && !utf8.Valid(got) {
				t.Fatal("truncated text is not valid UTF-8")
			}
		})
	}
}

func TestTruncatedPutsMatch(t *testing.T) {
	// Two inputs differing only past MaxSize store the same blob.
	s, _ := newStore(t)
	base := bytes.Repeat([]byte("z"), MaxSize)
	a, _ := s.Put(append(append([]byte{}, base...), "one"...))
	b, _ := s.Put(append(append([]byte{}, base...), "two"...))
	if a != b {
		t.Fatal("inputs equal up to MaxSize got different ids")
	}
}

func TestGetBadID(t *testing.T) {
	s, dir := newStore(t)
	valid, _ := s.Put([]byte("x"))
	// A file named like a traversal must not be reachable.
	if err := os.WriteFile(filepath.Join(dir, "evil"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name string
		id   string
	}{
		{"empty", ""},
		{"short", valid[:63]},
		{"long", valid + "0"},
		{"uppercase", strings.ToUpper(valid)},
		{"non-hex", strings.Repeat("g", 64)},
		{"traversal slash", "../" + valid[3:]},
		{"traversal backslash", `..\` + valid[3:]},
		{"absolute windows", `C:\` + valid[3:]},
		{"dots", strings.Repeat(".", 64)},
		{"space", " " + valid[1:]},
		{"nul", "\x00" + valid[1:]},
		{"sha1 length", valid[:40]},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := s.Get(tt.id); !errors.Is(err, ErrBadID) {
				t.Fatalf("Get(%q) err %v, want ErrBadID", tt.id, err)
			}
			if s.Has(tt.id) {
				t.Fatalf("Has(%q) = true", tt.id)
			}
		})
	}
}

func TestGetNotFound(t *testing.T) {
	s, _ := newStore(t)
	id := sha([]byte("never stored"))
	if _, err := s.Get(id); !errors.Is(err, ErrNotFound) {
		t.Fatalf("err %v, want ErrNotFound", err)
	}
	if s.Has(id) {
		t.Fatal("Has = true for absent blob")
	}
}

func TestHasIgnoresDirectory(t *testing.T) {
	s, dir := newStore(t)
	id := sha([]byte("dir squatter"))
	if err := os.MkdirAll(filepath.Join(dir, id[:2], id), 0o700); err != nil {
		t.Fatal(err)
	}
	if s.Has(id) {
		t.Fatal("Has = true for a directory")
	}
}

func TestCorruptBlob(t *testing.T) {
	gz := func(b []byte) []byte {
		var buf bytes.Buffer
		zw := gzip.NewWriter(&buf)
		zw.Write(b)
		zw.Close()
		return buf.Bytes()
	}
	good := gz([]byte("original content"))
	tests := []struct {
		name string
		raw  []byte
	}{
		{"empty file", nil},
		{"not gzip", []byte("plain text, not gzip")},
		{"truncated gzip", good[:len(good)/2]},
		{"bad crc", append(append([]byte{}, good[:len(good)-8]...), 0, 0, 0, 0, 0, 0, 0, 0)},
		{"wrong content", gz([]byte("different content"))},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			s, dir := newStore(t)
			id := sha([]byte("original content"))
			if err := os.MkdirAll(filepath.Join(dir, id[:2]), 0o700); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(dir, id[:2], id), tt.raw, 0o600); err != nil {
				t.Fatal(err)
			}
			got, err := s.Get(id)
			if err == nil {
				t.Fatalf("corrupt blob read back %q with no error", got)
			}
			if errors.Is(err, ErrNotFound) || errors.Is(err, ErrBadID) {
				t.Fatalf("err %v, want a corruption error", err)
			}
		})
	}
}

func TestConcurrentPutSameContent(t *testing.T) {
	s, dir := newStore(t)
	data := []byte(strings.Repeat("shared patch\r\n", 5000))
	want := sha(data)
	const n = 64
	var wg sync.WaitGroup
	errs := make(chan error, n)
	start := make(chan struct{})
	for range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			<-start
			id, err := s.Put(data)
			if err == nil && id != want {
				err = errors.New("wrong id " + id)
			}
			errs <- err
		}()
	}
	close(start)
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	got, err := s.Get(want)
	if err != nil || !bytes.Equal(got, data) {
		t.Fatalf("get after concurrent puts: %v", err)
	}
	assertNoTemps(t, dir)
}

func TestConcurrentPutAndGet(t *testing.T) {
	// Readers holding the final file open while writers race to place it.
	s, _ := newStore(t)
	data := []byte("read while writing")
	id := sha(data)
	var wg sync.WaitGroup
	errs := make(chan error, 64)
	for i := range 64 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if i%2 == 0 {
				_, err := s.Put(data)
				errs <- err
				return
			}
			got, err := s.Get(id)
			if errors.Is(err, ErrNotFound) {
				err = nil
			} else if err == nil && !bytes.Equal(got, data) {
				err = errors.New("short read")
			}
			errs <- err
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
}

func TestConcurrentStores(t *testing.T) {
	// Two Stores on one dir stand in for two processes.
	dir := filepath.Join(t.TempDir(), "blobs")
	a, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	b, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	data := []byte("two processes")
	var wg sync.WaitGroup
	errs := make(chan error, 32)
	for i := range 32 {
		st := a
		if i%2 == 1 {
			st = b
		}
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := st.Put(data)
			errs <- err
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	if got, err := b.Get(sha(data)); err != nil || !bytes.Equal(got, data) {
		t.Fatalf("get: %v", err)
	}
	assertNoTemps(t, dir)
}

func TestPutWhileFinalHeldOpen(t *testing.T) {
	// On Windows an open reader can make a rename onto the final file fail;
	// an existing final file with the same id is success.
	s, dir := newStore(t)
	data := []byte("held open")
	id, err := s.Put(data)
	if err != nil {
		t.Fatal(err)
	}
	f, err := os.Open(filepath.Join(dir, id[:2], id))
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	impl := s.impl
	tmp := filepath.Join(dir, id[:2], id+".tmp-test")
	raw, err := os.ReadFile(filepath.Join(dir, id[:2], id))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := impl.place(tmp, filepath.Join(dir, id[:2], id)); err != nil {
		t.Fatalf("place onto an open existing blob: %v", err)
	}
	os.Remove(tmp)
	if got, err := s.Get(id); err != nil || !bytes.Equal(got, data) {
		t.Fatalf("get: %v", err)
	}
}

func TestPutFailsWhenDirIsFile(t *testing.T) {
	s, dir := newStore(t)
	data := []byte("blocked")
	id := sha(data)
	// A file where the <aa> subdirectory should be.
	if err := os.WriteFile(filepath.Join(dir, id[:2]), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Put(data); err == nil {
		t.Fatal("put succeeded with no place to write")
	}
}

func TestValidID(t *testing.T) {
	tests := []struct {
		id   string
		want bool
	}{
		{strings.Repeat("0", 64), true},
		{strings.Repeat("f", 64), true},
		{"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", true},
		{strings.Repeat("F", 64), false},
		{strings.Repeat("0", 63) + "A", false},
		{strings.Repeat("0", 63) + "/", false},
		{strings.Repeat("0", 65), false},
		{"", false},
	}
	for _, tt := range tests {
		if got := validID(tt.id); got != tt.want {
			t.Errorf("validID(%q) = %v, want %v", tt.id, got, tt.want)
		}
	}
}

func assertNoTemps(t *testing.T, dir string) {
	t.Helper()
	filepath.WalkDir(dir, func(path string, d os.DirEntry, err error) error {
		if err == nil && strings.Contains(d.Name(), ".tmp-") {
			t.Errorf("temp file left behind: %s", path)
		}
		return nil
	})
}
