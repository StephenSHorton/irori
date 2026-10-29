package blob

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func newFiles(t *testing.T) *Files {
	t.Helper()
	f, err := OpenFiles(filepath.Join(t.TempDir(), "files"))
	if err != nil {
		t.Fatal(err)
	}
	return f
}

func TestFilesPutResumeAndFinish(t *testing.T) {
	f := newFiles(t)
	src := filepath.Join(t.TempDir(), "notes.txt")
	data := bytes.Repeat([]byte("irori attachment\n"), 4000)
	if err := os.WriteFile(src, data, 0o600); err != nil {
		t.Fatal(err)
	}
	ref, err := f.PutFile(src)
	if err != nil {
		t.Fatal(err)
	}
	if ref.SHA256 != sha(data) || ref.Size != int64(len(data)) || ref.Name != "notes.txt" || !strings.HasPrefix(ref.Mime, "text/plain") {
		t.Fatalf("ref = %+v", ref)
	}
	// Stored as is, uncompressed.
	if got, err := os.ReadFile(f.path(ref.SHA256)); err != nil || !bytes.Equal(got, data) {
		t.Fatalf("stored bytes differ: %v", err)
	}
	if again, err := f.PutFile(src); err != nil || again.SHA256 != ref.SHA256 {
		t.Fatalf("second put = %+v, %v", again, err)
	}

	// A second store fetches it in two pieces, resuming by offset.
	g := newFiles(t)
	p, off, err := g.Partial(ref.SHA256)
	if err != nil || off != 0 {
		t.Fatalf("partial = %d, %v", off, err)
	}
	p.Write(data[:1000])
	p.Close()
	if err := g.Finish(ref.SHA256, ref.Size); !errors.Is(err, ErrMismatch) {
		t.Fatalf("short finish = %v", err)
	}
	p, off, err = g.Partial(ref.SHA256)
	if err != nil || off != 1000 {
		t.Fatalf("resume at %d, %v", off, err)
	}
	p.Write(data[1000:])
	p.Close()
	if err := g.Finish(ref.SHA256, ref.Size); err != nil {
		t.Fatal(err)
	}
	if !g.Has(ref.SHA256) {
		t.Fatal("finished fetch not stored")
	}

	// A wrong body is dropped, not kept to resume.
	h := newFiles(t)
	p, _, _ = h.Partial(ref.SHA256)
	p.Write(bytes.Repeat([]byte("x"), len(data)))
	p.Close()
	if err := h.Finish(ref.SHA256, ref.Size); !errors.Is(err, ErrMismatch) {
		t.Fatalf("wrong finish = %v", err)
	}
	p, off, _ = h.Partial(ref.SHA256)
	p.Close()
	if off != 0 {
		t.Fatalf("wrong partial kept: %d bytes", off)
	}

	// CopyOut never overwrites.
	dst := filepath.Join(t.TempDir(), "out", "notes.txt")
	if err := g.CopyOut(ref.SHA256, dst); err != nil {
		t.Fatal(err)
	}
	if !SameContent(dst, ref.SHA256) {
		t.Fatal("copy differs")
	}
	if err := g.CopyOut(ref.SHA256, dst); err == nil {
		t.Fatal("CopyOut overwrote a file")
	}
}

func TestFilesCap(t *testing.T) {
	old := MaxFileSize
	MaxFileSize = 1 << 10
	defer func() { MaxFileSize = old }()
	f := newFiles(t)
	src := filepath.Join(t.TempDir(), "big.bin")
	os.WriteFile(src, make([]byte, 1<<10+1), 0o600)
	if _, err := f.PutFile(src); !errors.Is(err, ErrTooLarge) {
		t.Fatalf("over the cap = %v", err)
	}
	os.WriteFile(src, make([]byte, 1<<10), 0o600)
	if _, err := f.PutFile(src); err != nil {
		t.Fatalf("at the cap = %v", err)
	}
}

func TestCheckAttachPath(t *testing.T) {
	root := t.TempDir()
	data := filepath.Join(root, "irori")
	work := filepath.Join(root, "work")
	for _, d := range []string{data, filepath.Join(data, ReceivedDir, "s1"), work, filepath.Join(root, ".ssh")} {
		os.MkdirAll(d, 0o700)
	}
	write := func(p string) string {
		os.WriteFile(p, []byte("x"), 0o600)
		return p
	}
	ok := write(filepath.Join(work, "report.md"))
	write(filepath.Join(data, "irori.db"))
	recv := write(filepath.Join(data, ReceivedDir, "s1", "got.txt"))
	write(filepath.Join(work, ".env.local"))
	write(filepath.Join(work, "server.pem"))
	write(filepath.Join(root, ".ssh", "config"))
	write(filepath.Join(work, ".git-credentials"))

	if got, err := CheckAttachPath(ok, "", data); err != nil || !SameFile(got, ok) {
		t.Fatalf("plain file = %q, %v", got, err)
	}
	if got, err := CheckAttachPath("report.md", work, data); err != nil || !SameFile(got, ok) {
		t.Fatalf("relative to cwd = %q, %v", got, err)
	}
	if _, err := CheckAttachPath("report.md", "", data); err == nil {
		t.Fatal("relative with no cwd accepted")
	}
	if _, err := CheckAttachPath(recv, "", data); err != nil {
		t.Fatalf("a received file can be passed on: %v", err)
	}
	if _, err := CheckAttachPath(work, "", data); !errors.Is(err, ErrNotFile) {
		t.Fatalf("directory = %v", err)
	}
	for _, p := range []string{
		filepath.Join(data, "irori.db"),
		filepath.Join(data, "..", "irori", "irori.db"),
		filepath.Join(work, ".env.local"),
		filepath.Join(work, "server.pem"),
		filepath.Join(root, ".ssh", "config"),
		filepath.Join(work, ".git-credentials"),
	} {
		if _, err := CheckAttachPath(p, "", data); !errors.Is(err, ErrNeverLeaves) {
			t.Errorf("%s = %v, want refused", p, err)
		}
	}
	if runtime.GOOS == "windows" {
		upper := strings.ToUpper(filepath.Join(data, "irori.db"))
		if _, err := CheckAttachPath(upper, "", data); !errors.Is(err, ErrNeverLeaves) {
			t.Errorf("data dir in another case = %v", err)
		}
	}

	// A symlink into the data dir is refused by where it resolves to.
	link := filepath.Join(work, "innocent.txt")
	if err := os.Symlink(filepath.Join(data, "irori.db"), link); err != nil {
		t.Logf("no symlinks here (%v); the link cases are skipped", err)
		return
	}
	if _, err := CheckAttachPath(link, "", data); !errors.Is(err, ErrNeverLeaves) {
		t.Fatalf("symlink into the data dir = %v", err)
	}
	dirLink := filepath.Join(work, "stuff")
	if err := os.Symlink(data, dirLink); err == nil {
		if _, err := CheckAttachPath(filepath.Join(dirLink, "irori.db"), "", data); !errors.Is(err, ErrNeverLeaves) {
			t.Fatalf("through a linked folder = %v", err)
		}
	}
	keyLink := filepath.Join(work, "notes2.txt")
	if err := os.Symlink(filepath.Join(work, "server.pem"), keyLink); err == nil {
		if _, err := CheckAttachPath(keyLink, "", data); !errors.Is(err, ErrNeverLeaves) {
			t.Fatalf("link to a key = %v", err)
		}
	}
}

// SameFile compares two paths of one file.
func SameFile(a, b string) bool {
	x, err1 := os.Stat(a)
	y, err2 := os.Stat(b)
	return err1 == nil && err2 == nil && os.SameFile(x, y)
}

func TestSafeName(t *testing.T) {
	const id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
	cases := []struct{ in, want string }{
		{"report.pdf", "report.pdf"},
		{"../../etc/passwd", "passwd"},
		{`..\..\Windows\win.ini`, "win.ini"},
		{"a/b/c.txt", "c.txt"},
		{`C:evil.txt`, "evil.txt"},
		{`C:\Users\x\evil.txt`, "evil.txt"},
		{"file.txt:Zone.Identifier", "file.txt"},
		{"..", "attachment-0123456789ab"},
		{"", "attachment-0123456789ab"},
		{"...   ", "attachment-0123456789ab"},
		{"/", "attachment-0123456789ab"},
		{"CON", "_CON"},
		{"con.txt", "_con.txt"},
		{"Nul.tar.gz", "_Nul.tar.gz"},
		{"COM1", "_COM1"},
		{"lpt9.log", "_lpt9.log"},
		{"COM10", "COM10"},
		{"console.txt", "console.txt"},
		{"trail. . ", "trail"},
		{"a\x00b\nc.txt", "a_b_c.txt"},
		{`what?<>|*".txt`, "what______.txt"},
		{".hidden", "hidden"},
		// A right-to-left override would show invoiceexe.jpg in Explorer.
		{"invoice\u202egpj.exe", "invoice_gpj.exe"},
		{"a\u2066b\u2069\u200b\ufeff.png", "a_b___.png"},
	}
	for _, c := range cases {
		if got := SafeName(c.in, id); got != c.want {
			t.Errorf("SafeName(%q) = %q, want %q", c.in, got, c.want)
		}
	}
	long := strings.Repeat("a", 300) + ".txt"
	if got := SafeName(long, id); len([]rune(got)) != maxName || !strings.HasSuffix(got, ".txt") {
		t.Errorf("long name = %q", got)
	}
}
