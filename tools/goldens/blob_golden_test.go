// Golden exporter for crates/blob (capture.md §0 I4, §2.6, §3, §7.15, §7.19).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/blob with `go test -overlay` (package blob, so it reaches
// neverLeaves, truncate and humanSize). It only runs when IRORI_GOLDEN_OUT
// is set, and writes $IRORI_GOLDEN_OUT/blob/:
//
//	blob.json     SafeName, neverLeaves, Within, Describe, the sniff table,
//	              truncation ids, and
//	              CheckAttachPath errors in a scratch tree ($ROOT for its path)
//	store/        a Store Go wrote (gzip blobs the Rust Store must read)
package blob

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type goldSafe struct {
	Name string `json:"name"`
	SHA  string `json:"sha"`
	Out  string `json:"out"`
}

type goldStr struct {
	In  string `json:"in"`
	Out string `json:"out"`
}

type goldWithin struct {
	P    string `json:"p"`
	Base string `json:"base"`
	Want bool   `json:"want"`
}

type goldSniff struct {
	Head []byte `json:"head"`
	Out  string `json:"out"`
}

type goldAttach struct {
	Path string `json:"path"`
	Cwd  string `json:"cwd"`
	Out  string `json:"out"`
	Err  string `json:"err"`
}

type goldStored struct {
	ID   string `json:"id"`
	Data []byte `json:"data"`
}

func TestGoldenBlob(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	out := filepath.Join(dir, "blob")
	if err := os.RemoveAll(filepath.Join(out, "store")); err != nil {
		t.Fatal(err)
	}
	const id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

	var safe []goldSafe
	for _, n := range []string{
		"report.pdf", "../../etc/passwd", `..\..\Windows\win.ini`, "C:evil.txt", "1:x.txt", "é:x.txt",
		"file.txt:Zone.Identifier", "..", "", "...   ", "/", "CON", "con.txt", "Nul.tar.gz", "COM1", "lpt9.log",
		"COM10", "COM¹", "lpt³.txt", "COM⁴", "CONIN$", "conout$.log", "AUX .txt", "aux", "console.txt",
		"trail. . ", "a\x00b\nc.txt", `what?<>|*".txt`, ".hidden", " . x . ", "invoice\u202egpj.exe",
		"a\u2066b\u2069\u200b\ufeff.png", "\u0085x\u00a0", "x\u009f", "\ufffd.txt", "日本語.txt",
		"\u3000name\u3000", strings.Repeat("a", 300) + ".txt", strings.Repeat("é", 200) + ".verylongextension1",
		strings.Repeat("b", 119) + ".md", strings.Repeat("日", 130), "ǅ.txt", "ß", "ﬃ.txt",
	} {
		safe = append(safe, goldSafe{n, id, SafeName(n, id)})
	}
	safe = append(safe, goldSafe{"", "short", SafeName("", "short")})

	var never []goldStr
	for _, p := range []string{
		`C:\work\report.md`, `C:\work\.env.local`, `C:\Users\me\.ssh\config`, `C:\Users\me\.ssh`,
		`C:\Users\me\.SSH.\config`, `/home/me/.gnupg/x`, `/home/me/.aws/credentials`, `C:\x\.git-credentials`,
		`C:\x\Irori.json`, `C:\x\device.key`, `C:\x\person.json.`, `C:\x\login.keychain-db`, `C:\x\a.kdbx`,
		`C:\x\a.PPK`, `C:\x\a.gpg `, `C:\Users\me\AppData\Roaming\Microsoft\Credentials\x`,
		`C:\Users\me\AppData\Roaming\Microsoft\Protect\S-1\x`, `C:\microsoft\other`, `/Library/Keychains/x`,
		`C:\keychains`, `C:\x\.docker\config.json`, `C:\x\.password-store\a`, `C:\x\.kube\config`,
		`C:\x\.azure\x`, `C:\x\id_rsa`, "", `C:\`, `\\srv\share\.ssh\x`, `C:\x\İrori.json`,
	} {
		never = append(never, goldStr{p, neverLeaves(p)})
	}

	var within []goldWithin
	for _, c := range [][2]string{
		{`C:\a\b`, `C:\a`}, {`C:\a`, `C:\a`}, {`C:\A\b`, `c:\a`}, {`C:\ab`, `C:\a`}, {`C:\a\..b`, `C:\a`},
		{`C:\a\..\b`, `C:\a`}, {`D:\a`, `C:\a`}, {`C:\a\b\..`, `C:\a`}, {`C:\a`, `C:\a\b`}, {`\\s\h\x`, `\\S\H`},
		{`C:\İ\x`, `C:\i`}, {`C:\K\x`, "C:\\\u212a"},
	} {
		within = append(within, goldWithin{c[0], c[1], Within(c[0], c[1])})
	}

	var describe []goldStr
	for _, r := range []FileRef{
		{SHA256: id, Name: "notes.txt", Size: 68000, Mime: "text/plain; charset=utf-8", Device: "0123456789abcdef"},
		{SHA256: id, Name: "a.bin", Size: 5, Device: "dev"},
		{SHA256: id, Name: "big.iso", Size: 3 << 30, Mime: "application/x-iso9660-image"},
		{SHA256: id, Name: "x", Size: 1023, Mime: "bad\nmime", Device: "dev/../x"},
		{SHA256: id, Name: "x", Size: 1024, Mime: strings.Repeat("a", 101)},
		{SHA256: id, Name: "x", Size: 1<<20 - 1},
		{SHA256: id, Name: "x", Size: 1 << 20},
		{SHA256: id, Name: "x", Size: 1572864},
		{SHA256: id, Name: "x", Size: 1153434},
		{SHA256: id, Name: "x", Size: 0, Device: "é"},
		{SHA256: id, Name: "CON", Size: -1, Device: "abc_-def-ghij"},
	} {
		b, _ := json.Marshal(r)
		describe = append(describe, goldStr{string(b), r.Describe()})
	}

	var sniff []goldSniff
	for _, h := range [][]byte{
		nil, []byte("hello\n"), []byte("  <!DOCTYPE html><p>"), []byte("<html>"), []byte("<HTMLx"), []byte("\t<p>"),
		[]byte("<?xml version"), []byte("%PDF-1.7"), []byte("%!PS-Adobe-3"), {0xFE, 0xFF, 1, 2}, {0xFF, 0xFE, 1, 2},
		{0xEF, 0xBB, 0xBF, 'x'}, {0, 0, 1, 0}, []byte("BM...."), []byte("GIF89a"), []byte("RIFF\x00\x00\x00\x00WEBPVP8 "),
		[]byte("\x89PNG\r\n\x1a\n...."), {0xFF, 0xD8, 0xFF, 0xE0}, []byte("FORM\x01\x02\x03\x04AIFF"), []byte("ID3\x04"),
		[]byte("OggS\x00\x02"), []byte("MThd\x00\x00\x00\x06"), []byte("RIFF\x00\x00\x00\x00AVI LIST"),
		[]byte("RIFF\x00\x00\x00\x00WAVEfmt "), []byte("\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp41isom"),
		[]byte("\x00\x00\x00\x14ftypisom\x00\x00\x00\x00mp4x"), []byte("\x1A\x45\xDF\xA3"), []byte("\x00\x01\x00\x00"),
		[]byte("OTTO"), []byte("wOF2"), []byte("\x1F\x8B\x08\x00"), []byte("PK\x03\x04"), []byte("Rar!\x1A\x07\x01\x00"),
		[]byte("\x00asm\x01"), []byte("text\x1bescape"), []byte("bin\x00ary"), []byte("tab\tand\fff\r\n"),
		append(bytes.Repeat([]byte{0}, 34), 'L', 'P'), bytes.Repeat([]byte("a"), 600), append(bytes.Repeat([]byte("a"), 520), 0),
	} {
		sniff = append(sniff, goldSniff{h, http.DetectContentType(h)})
	}

	// Truncation ids: the id of a long input is the sha of its cut.
	var cuts []goldStr
	for _, d := range [][]byte{
		bytes.Repeat([]byte("a"), MaxSize+1),
		append(bytes.Repeat([]byte("a"), MaxSize-1), []byte("€€€")...),
		append(bytes.Repeat([]byte("a"), MaxSize-2), []byte("😀😀")...),
		bytes.Repeat([]byte{0x80}, MaxSize+10),
		append(bytes.Repeat([]byte("a"), MaxSize-3), []byte{0xF0, 0x80, 0x80, 0x80, 0x80}...),
	} {
		cut := truncate(d)
		cuts = append(cuts, goldStr{"len" + itoa(len(d)) + "tail" + string(hexTail(d)), sha256Hex(cut)})
	}

	// CheckAttachPath errors in a scratch tree.
	root := t.TempDir()
	data := filepath.Join(root, "irori")
	work := filepath.Join(root, "work")
	for _, d := range []string{data, filepath.Join(data, ReceivedDir, "s1"), work} {
		os.MkdirAll(d, 0o700)
	}
	os.WriteFile(filepath.Join(work, "report.md"), []byte("x"), 0o600)
	os.WriteFile(filepath.Join(data, "irori.db"), []byte("x"), 0o600)
	os.WriteFile(filepath.Join(data, ReceivedDir, "s1", "got.txt"), []byte("x"), 0o600)
	var attach []goldAttach
	rel := func(s string) string { return strings.ReplaceAll(s, root, "$ROOT") }
	for _, c := range [][2]string{
		{"$ROOT/work/report.md", ""}, {"report.md", "$ROOT/work"}, {"report.md", ""}, {"report.md", "relative"},
		{"  ", ""}, {"$ROOT/work", ""}, {"$ROOT/work/missing.txt", ""}, {"$ROOT/nodir/missing.txt", ""},
		{"$ROOT/irori/irori.db", ""}, {"$ROOT/irori/received/s1/got.txt", ""}, {"$ROOT/work/.env", ""},
		{"$ROOT/work/../irori/irori.db", ""}, {"$ROOT/irori", ""},
	} {
		p := strings.ReplaceAll(c[0], "$ROOT", root)
		cwd := strings.ReplaceAll(c[1], "$ROOT", root)
		p, cwd = filepath.FromSlash(p), filepath.FromSlash(cwd)
		if strings.TrimSpace(c[0]) == "" {
			p = c[0]
		}
		got, err := CheckAttachPath(p, cwd, data)
		g := goldAttach{Path: c[0], Cwd: c[1], Out: rel(got)}
		if err != nil {
			g.Err = rel(err.Error())
		}
		attach = append(attach, g)
	}

	// A store Go wrote, for the Rust reader.
	st, err := Open(filepath.Join(out, "store"))
	if err != nil {
		t.Fatal(err)
	}
	var stored []goldStored
	for _, d := range [][]byte{nil, []byte("--- a/x\n+++ b/x\n"), bytes.Repeat([]byte("patch line\r\n"), 3000), {0, 1, 0xff}} {
		id, err := st.Put(d)
		if err != nil {
			t.Fatal(err)
		}
		stored = append(stored, goldStored{id, d})
	}

	v := map[string]any{
		"safeName": safe, "neverLeaves": never, "within": within, "describe": describe,
		"sniff": sniff, "truncate": cuts, "checkAttachPath": attach, "stored": stored,
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(out, "blob.json"), buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

func itoa(n int) string {
	b, _ := json.Marshal(n)
	return string(b)
}

func hexTail(d []byte) []byte {
	const hx = "0123456789abcdef"
	var out []byte
	for _, c := range d[len(d)-6:] {
		out = append(out, hx[c>>4], hx[c&15])
	}
	return out
}

func sha256Hex(b []byte) string {
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}
