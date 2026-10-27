package decision

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/gitsha"
)

func TestValidate(t *testing.T) {
	cases := []struct {
		name, stmt, why string
		want            error
		wantStmt        string
	}{
		{"plain", "use sqlite", "", nil, "use sqlite"},
		{"trims", "  use sqlite \r\n", "  because \t", nil, "use sqlite"},
		{"empty", "", "", ErrEmpty, ""},
		{"blank", " \t\n ", "why", ErrEmpty, ""},
		{"stmt at max runes", strings.Repeat("é", MaxStatement), "", nil, strings.Repeat("é", MaxStatement)},
		{"stmt over max", strings.Repeat("a", MaxStatement+1), "", ErrTooLong, strings.Repeat("a", MaxStatement+1)},
		{"stmt over max multibyte", strings.Repeat("界", MaxStatement+1), "", ErrTooLong, strings.Repeat("界", MaxStatement+1)},
		{"stmt max after trim", "  " + strings.Repeat("a", MaxStatement) + "  ", "", nil, strings.Repeat("a", MaxStatement)},
		{"why at max runes", "s", strings.Repeat("界", MaxWhy), nil, "s"},
		{"why over max", "s", strings.Repeat("b", MaxWhy+1), ErrTooLong, "s"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			d := Decision{Statement: c.stmt, Why: c.why}
			if err := Validate(&d); !errors.Is(err, c.want) {
				t.Fatalf("err = %v, want %v", err, c.want)
			}
			if c.wantStmt != "" && d.Statement != c.wantStmt {
				t.Fatalf("statement = %q", d.Statement)
			}
			if d.Why != strings.TrimSpace(c.why) {
				t.Fatalf("why = %q", d.Why)
			}
		})
	}
}

func TestParseLines(t *testing.T) {
	cases := []struct {
		in         string
		start, end int
		bad        bool
	}{
		{"", 0, 0, false},
		{"  ", 0, 0, false},
		{"40", 40, 40, false},
		{" 40 ", 40, 40, false},
		{"40-52", 40, 52, false},
		{"40 - 52", 40, 52, false},
		{"40 -52", 40, 52, false},
		{"40- 52", 40, 52, false},
		{"1-1", 1, 1, false},
		{"007", 7, 7, false},
		{"0", 0, 0, true},
		{"0-3", 0, 0, true},
		{"40-39", 0, 0, true},
		{"+40", 0, 0, true},
		{"-40", 0, 0, true},
		{"40-", 0, 0, true},
		{"-", 0, 0, true},
		{"a", 0, 0, true},
		{"4a", 0, 0, true},
		{"1-2-3", 0, 0, true},
		{"1.5", 0, 0, true},
		{"1,2", 0, 0, true},
		{"4 0", 0, 0, true},
		{"99999999999999999999999", 0, 0, true},
	}
	for _, c := range cases {
		s, e, err := ParseLines(c.in)
		if c.bad {
			if !errors.Is(err, ErrBadLines) {
				t.Errorf("ParseLines(%q) err = %v, want ErrBadLines", c.in, err)
			}
			continue
		}
		if err != nil || s != c.start || e != c.end {
			t.Errorf("ParseLines(%q) = %d,%d,%v want %d,%d", c.in, s, e, err, c.start, c.end)
		}
	}
}

// writeFile writes rel (slash form) under root.
func writeFile(t *testing.T, root, rel, content string) string {
	t.Helper()
	full := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(full, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	return full
}

const tenLines = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n"

func TestMakeAnchorPaths(t *testing.T) {
	root := t.TempDir()
	writeFile(t, root, "src/app/main.go", tenLines)
	writeFile(t, root, "top.txt", tenLines)
	outside := writeFile(t, t.TempDir(), "other.go", tenLines)
	abs := filepath.Join(root, "src", "app", "main.go")

	cases := []struct {
		name, path, want string
		err              error
	}{
		{"relative", "src/app/main.go", "src/app/main.go", nil},
		{"backslashes", `src\app\main.go`, "src/app/main.go", nil},
		{"dot prefix", "./src/app/main.go", "src/app/main.go", nil},
		{"doubled slashes", "src//app/./main.go", "src/app/main.go", nil},
		{"spaces trimmed", "  top.txt ", "top.txt", nil},
		{"absolute native", abs, "src/app/main.go", nil},
		{"absolute forward", filepath.ToSlash(abs), "src/app/main.go", nil},
		{"dotdot escape", "../x.go", "", ErrBadPath},
		{"dotdot inside", "src/../top.txt", "", ErrBadPath},
		{"dotdot backslash", `src\..\..\x`, "", ErrBadPath},
		{"dotdot absolute", filepath.Join(root, "src") + string(filepath.Separator) + ".." + string(filepath.Separator) + "top.txt", "", ErrBadPath},
		{"absolute outside", outside, "", ErrBadPath},
		{"root itself", root, "", ErrBadPath},
		{"sibling prefix", root + "2" + string(filepath.Separator) + "top.txt", "", ErrBadPath},
		{"empty", "", "", ErrBadPath},
		{"dot", ".", "", ErrBadPath},
		{"missing", "src/nope.go", "", ErrNoFile},
		{"directory", "src/app", "", ErrNoFile},
	}
	if runtime.GOOS == "windows" {
		cases = append(cases,
			struct {
				name, path, want string
				err              error
			}{"windows case-insensitive", strings.ToUpper(abs[:1]) + strings.ToUpper(abs[1:len(root)]) + abs[len(root):], "src/app/main.go", nil},
			struct {
				name, path, want string
				err              error
			}{"windows lower drive", strings.ToLower(abs[:1]) + abs[1:], "src/app/main.go", nil},
			struct {
				name, path, want string
				err              error
			}{"windows rooted no volume", `\src\app\main.go`, "", ErrBadPath},
			struct {
				name, path, want string
				err              error
			}{"windows volume relative", "C:top.txt", "", ErrBadPath},
		)
	} else {
		cases = append(cases, struct {
			name, path, want string
			err              error
		}{"rooted outside", "/etc/hosts", "", ErrBadPath})
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			a, err := MakeAnchor(root, c.path, 0, 0, "rk", "head")
			if !errors.Is(err, c.err) {
				t.Fatalf("err = %v, want %v", err, c.err)
			}
			if err == nil && a.Path != c.want {
				t.Fatalf("path = %q, want %q", a.Path, c.want)
			}
		})
	}
	if _, err := MakeAnchor("", "top.txt", 0, 0, "", ""); !errors.Is(err, ErrBadPath) {
		t.Fatalf("empty root err = %v", err)
	}
}

func TestMakeAnchorWholeFile(t *testing.T) {
	root := t.TempDir()
	writeFile(t, root, "a.txt", "hello\n")
	writeFile(t, root, "empty.txt", "")
	a, err := MakeAnchor(root, "a.txt", 0, 0, "repo1", "abc123")
	if err != nil {
		t.Fatal(err)
	}
	want := Anchor{RepoKey: "repo1", Root: filepath.Clean(root), Path: "a.txt", SHA: "abc123",
		Hash: "ce013625030ba8dba906f756967f9e9ca394464a"}
	if fmt.Sprint(a) != fmt.Sprint(want) {
		t.Fatalf("anchor = %+v\nwant %+v", a, want)
	}
	e, err := MakeAnchor(root, "empty.txt", 0, 0, "", "")
	if err != nil || e.Hash != "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391" {
		t.Fatalf("empty: %+v %v", e, err)
	}
	if _, err := MakeAnchor(root, "empty.txt", 1, 1, "", ""); !errors.Is(err, ErrRange) {
		t.Fatalf("lines in empty file err = %v", err)
	}
}

func TestMakeAnchorLines(t *testing.T) {
	root := t.TempDir()
	writeFile(t, root, "f.txt", tenLines)
	writeFile(t, root, "nonl.txt", "a\nb\nc")
	h := func(ls ...string) string { return hashLines(ls) }
	cases := []struct {
		name       string
		file       string
		start, end int
		hash       string
		ctx        []string
		err        error
	}{
		{"middle", "f.txt", 4, 5, h("l4", "l5"), []string{"l2", "l3", "l6", "l7"}, nil},
		{"single", "f.txt", 5, 5, h("l5"), []string{"l3", "l4", "l6", "l7"}, nil},
		{"first line", "f.txt", 1, 1, h("l1"), []string{"l2", "l3"}, nil},
		{"second line", "f.txt", 2, 3, h("l2", "l3"), []string{"l1", "l4", "l5"}, nil},
		{"last line", "f.txt", 10, 10, h("l10"), []string{"l8", "l9"}, nil},
		{"whole range", "f.txt", 1, 10, h("l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8", "l9", "l10"), nil, nil},
		{"no final newline", "nonl.txt", 3, 3, h("c"), []string{"a", "b"}, nil},
		{"past end", "f.txt", 10, 11, "", nil, ErrRange},
		{"start past end", "f.txt", 11, 11, "", nil, ErrRange},
		{"past end no newline", "nonl.txt", 4, 4, "", nil, ErrRange},
		{"end before start", "f.txt", 5, 4, "", nil, ErrBadLines},
		{"end without start", "f.txt", 0, 4, "", nil, ErrBadLines},
		{"negative", "f.txt", -1, 2, "", nil, ErrBadLines},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			a, err := MakeAnchor(root, c.file, c.start, c.end, "rk", "")
			if !errors.Is(err, c.err) {
				t.Fatalf("err = %v, want %v", err, c.err)
			}
			if err != nil {
				return
			}
			if a.Start != c.start || a.End != c.end || a.Hash != c.hash || !slices.Equal(a.Context, c.ctx) {
				t.Fatalf("anchor = %+v", a)
			}
		})
	}
}

func TestLineHashKnownValue(t *testing.T) {
	// sha256("a\nb"), with trailing whitespace and \r normalised away.
	root := t.TempDir()
	writeFile(t, root, "x.txt", "a  \r\nb\t\r\n")
	a, err := MakeAnchor(root, "x.txt", 1, 2, "", "")
	if err != nil {
		t.Fatal(err)
	}
	// printf 'a\nb' | sha256sum
	if a.Hash != "7e18f737311b2dc3b2f269dd78396b0351f14fb66efa879f768cb23181883c78" {
		t.Fatalf("hash %s", a.Hash)
	}
}

func TestCRLFMatchesLF(t *testing.T) {
	root := t.TempDir()
	lf := "package x\n\nfunc A() {   \n\treturn\n}\n\nfunc B() {}\n"
	crlf := strings.ReplaceAll(lf, "\n", "\r\n")
	writeFile(t, root, "lf.go", lf)
	writeFile(t, root, "crlf.go", crlf)

	for _, r := range [][2]int{{0, 0}, {3, 5}, {1, 1}, {7, 7}} {
		a, err := MakeAnchor(root, "lf.go", r[0], r[1], "", "")
		if err != nil {
			t.Fatal(err)
		}
		b, err := MakeAnchor(root, "crlf.go", r[0], r[1], "", "")
		if err != nil {
			t.Fatal(err)
		}
		if a.Hash != b.Hash || !slices.Equal(a.Context, b.Context) {
			t.Fatalf("lines %v: lf %+v crlf %+v", r, a, b)
		}
		if r[0] == 0 && a.Hash != gitsha.Blob([]byte(lf)) {
			t.Fatalf("whole-file hash %s is not the LF blob", a.Hash)
		}
		// Each anchor checks OK against the other line-ending form.
		b.Path = "lf.go"
		if st := Check(b, root); st.State != OK {
			t.Fatalf("crlf anchor on lf file: %+v", st)
		}
	}
}

func TestCheck(t *testing.T) {
	orig := "a\nb\nc\nd\ne\nf\n"
	mk := func(t *testing.T, content string, start, end int) (Anchor, string) {
		root := t.TempDir()
		writeFile(t, root, "f.txt", content)
		a, err := MakeAnchor(root, "f.txt", start, end, "rk", "")
		if err != nil {
			t.Fatal(err)
		}
		return a, root
	}
	cases := []struct {
		name       string
		start, end int
		after      string // "" removes the file
		want       Status
	}{
		{"unchanged", 3, 4, orig, Status{State: OK, Start: 3, End: 4}},
		{"whitespace only", 3, 4, "a\nb\nc   \r\nd\t\ne\nf\n", Status{State: OK, Start: 3, End: 4}},
		{"moved down", 3, 4, "x\ny\na\nb\nc\nd\ne\nf\n", Status{State: Moved, Start: 5, End: 6}},
		{"moved up", 3, 4, "c\nd\ne\nf\n", Status{State: Moved, Start: 1, End: 2}},
		{"edited", 3, 4, "a\nb\nC\nd\ne\nf\n", Status{State: Changed}},
		{"truncated", 5, 6, "a\nb\n", Status{State: Changed}},
		{"emptied", 1, 1, "", Status{State: Changed}},
		{"whole ok", 0, 0, orig, Status{State: OK}},
		{"whole ok crlf", 0, 0, strings.ReplaceAll(orig, "\n", "\r\n"), Status{State: OK}},
		{"whole changed", 0, 0, orig + "g\n", Status{State: Changed}},
		{"whole trailing space changed", 0, 0, "a \nb\nc\nd\ne\nf\n", Status{State: Changed}},
		// Equidistant copies at 1-2 and 5-6 around the old 3-4: earlier wins.
		{"tie earlier", 3, 4, "c\nd\nx\ny\nc\nd\n", Status{State: Moved, Start: 1, End: 2}},
		// Nearest wins over earlier: copies at 1 and 5 around old 4.
		{"nearest", 4, 4, "d\nx\nx\nx\nd\nx\n", Status{State: Moved, Start: 5, End: 5}},
		// A match in place wins over a copy elsewhere.
		{"in place despite copy", 3, 4, "c\nd\nc\nd\n", Status{State: OK, Start: 3, End: 4}},
		{"only later copy", 1, 2, "x\nx\nx\nx\nx\nx\na\nb\n", Status{State: Moved, Start: 7, End: 8}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			a, root := mk(t, orig, c.start, c.end)
			writeFile(t, root, "f.txt", c.after)
			if got := Check(a, root); got != c.want {
				t.Fatalf("Check = %+v, want %+v", got, c.want)
			}
		})
	}

	t.Run("gone", func(t *testing.T) {
		a, root := mk(t, orig, 2, 2)
		os.Remove(filepath.Join(root, "f.txt"))
		if got := Check(a, root); got.State != Gone {
			t.Fatalf("%+v", got)
		}
	})
	t.Run("directory is gone", func(t *testing.T) {
		a, root := mk(t, orig, 2, 2)
		os.Remove(filepath.Join(root, "f.txt"))
		os.Mkdir(filepath.Join(root, "f.txt"), 0o755)
		if got := Check(a, root); got.State != Gone {
			t.Fatalf("%+v", got)
		}
	})
	t.Run("other root", func(t *testing.T) {
		a, _ := mk(t, orig, 2, 3)
		other := t.TempDir()
		writeFile(t, other, "f.txt", "z\n"+orig)
		if got := Check(a, other); got != (Status{State: Moved, Start: 3, End: 4}) {
			t.Fatalf("%+v", got)
		}
	})
	t.Run("too big", func(t *testing.T) {
		a, root := mk(t, orig, 0, 0)
		old := maxCheckSize
		maxCheckSize = int64(len(orig)) - 1
		defer func() { maxCheckSize = old }()
		if got := Check(a, root); got.State != Changed {
			t.Fatalf("%+v", got)
		}
	})
	t.Run("eight MiB limit", func(t *testing.T) {
		root := t.TempDir()
		big := writeFile(t, root, "big.bin", "")
		f, err := os.OpenFile(big, os.O_WRONLY, 0)
		if err != nil {
			t.Fatal(err)
		}
		f.Truncate(8<<20 + 1)
		f.Close()
		a := Anchor{Path: "big.bin", Hash: gitsha.Blob(make([]byte, 8<<20+1))}
		if got := Check(a, root); got.State != Changed {
			t.Fatalf("%+v", got)
		}
	})

	unknown := []struct {
		name string
		a    Anchor
		root string
	}{
		{"no root", Anchor{Path: "f.txt", Hash: "x"}, ""},
		{"no hash", Anchor{Path: "f.txt"}, "."},
		{"bad range", Anchor{Path: "f.txt", Hash: "x", Start: 5, End: 4}, "."},
		{"negative", Anchor{Path: "f.txt", Hash: "x", Start: -1}, "."},
		{"escape", Anchor{Path: "../f.txt", Hash: "x"}, "."},
		{"absolute", Anchor{Path: filepath.ToSlash(filepath.Join(t.TempDir(), "f.txt")), Hash: "x"}, "."},
		{"unclean", Anchor{Path: "./f.txt", Hash: "x"}, "."},
		{"backslash", Anchor{Path: `a\f.txt`, Hash: "x"}, "."},
	}
	for _, c := range unknown {
		t.Run("unknown "+c.name, func(t *testing.T) {
			if got := Check(c.a, c.root); got.State != Unknown {
				t.Fatalf("%+v", got)
			}
		})
	}
}

var t0 = time.Date(2026, 10, 27, 12, 0, 0, 0, time.UTC)

func dec(id, key, kind string, min int, sup ...string) Decision {
	return Decision{ID: id, By: Party{Key: key, Kind: kind}, Statement: "stmt " + id,
		Supersedes: sup, Trust: "untrusted", At: t0.Add(time.Duration(min) * time.Minute)}
}

func ids(ls []Listed) []string {
	var out []string
	for _, l := range ls {
		out = append(out, l.ID)
	}
	return out
}

func TestViewAddGet(t *testing.T) {
	v := NewView()
	if _, ok := v.Get("a"); ok {
		t.Fatal("empty view has a")
	}
	d := dec("a", "k1", "claude", 0)
	d.Anchor = &Anchor{Path: "x.go", Context: []string{"c1"}}
	d.Source = &Source{TurnID: "t1"}
	v.Add(d)
	// Idempotent: a repeat id is ignored, the first wins.
	dup := dec("a", "k2", "claude", 5)
	dup.Statement = "other"
	v.Add(dup)
	v.Add(Decision{Statement: "no id"})

	got, ok := v.Get("a")
	if !ok || got.Statement != "stmt a" || got.By.Key != "k1" {
		t.Fatalf("got %+v", got)
	}
	if n := len(v.List(Filter{All: true})); n != 1 {
		t.Fatalf("list has %d", n)
	}
	// Mutating what went in or came out does not reach the view.
	d.Anchor.Context[0] = "changed"
	d.Source.TurnID = "changed"
	got.Anchor.Path = "changed"
	again, _ := v.Get("a")
	if again.Anchor.Path != "x.go" || again.Anchor.Context[0] != "c1" || again.Source.TurnID != "t1" {
		t.Fatalf("view aliased caller data: %+v", again)
	}
}

func TestViewSupersede(t *testing.T) {
	v := NewView()
	v.Add(dec("a", "k1", "claude", 0))
	v.Add(dec("b", "k1", "claude", 1, "a"))
	// c supersedes d, which arrives later.
	v.Add(dec("c", "k2", "grok", 2, "d", "c", ""))
	if got := ids(v.List(Filter{})); !slices.Equal(got, []string{"c", "b"}) {
		t.Fatalf("before d: %v", got)
	}
	v.Add(dec("d", "k2", "grok", 3))
	if got := ids(v.List(Filter{})); !slices.Equal(got, []string{"c", "b"}) {
		t.Fatalf("after d: %v", got)
	}
	// A second superseder; repeated ids in one decision count once.
	v.Add(dec("e", "u", "user", 4, "a", "a"))
	all := v.List(Filter{All: true})
	if got := ids(all); !slices.Equal(got, []string{"e", "d", "c", "b", "a"}) {
		t.Fatalf("all: %v", got)
	}
	by := map[string][]string{}
	for _, l := range all {
		by[l.ID] = l.SupersededBy
	}
	if !slices.Equal(by["a"], []string{"b", "e"}) || !slices.Equal(by["d"], []string{"c"}) || by["c"] != nil || by["b"] != nil {
		t.Fatalf("supersededBy: %v", by)
	}
}

func TestCanSupersede(t *testing.T) {
	v := NewView()
	v.Add(dec("a", "k1", "claude", 0))
	v.Add(dec("noKey", "", "claude", 0))
	cases := []struct {
		name   string
		by     Party
		target string
		want   error
	}{
		{"author", Party{Key: "k1", Kind: "claude"}, "a", nil},
		{"author other kind", Party{Key: "k1", Kind: "grok"}, "a", nil},
		{"other agent", Party{Key: "k2", Kind: "claude"}, "a", ErrSupersede},
		{"user", Party{Key: "someone", Kind: "user"}, "a", nil},
		{"user no key", Party{Kind: "user"}, "a", nil},
		{"empty keys do not match", Party{Kind: "claude"}, "noKey", ErrSupersede},
		{"unknown target", Party{Key: "k1"}, "zzz", ErrNoDecision},
		{"unknown target user", Party{Kind: "user"}, "zzz", ErrNoDecision},
	}
	for _, c := range cases {
		if err := v.CanSupersede(c.by, c.target); !errors.Is(err, c.want) {
			t.Errorf("%s: err = %v, want %v", c.name, err, c.want)
		}
	}
}

func TestListFilters(t *testing.T) {
	v := NewView()
	add := func(id string, min int, rk, path, stmt, why string) {
		d := dec(id, "k", "claude", min)
		d.Statement, d.Why = stmt, why
		if path != "" {
			d.Anchor = &Anchor{RepoKey: rk, Path: path}
		}
		v.Add(d)
	}
	add("1", 1, "r1", "src/a.go", "Use SQLite", "")
	add("2", 2, "r1", "src/b.go", "retry twice", "the network is FLAKY")
	add("3", 3, "r2", "src/a.go", "no globals", "")
	add("4", 4, "", "", "unanchored sqlite note", "")
	add("5", 4, "r1", "src/a.go", "same time, added later", "")

	cases := []struct {
		name string
		f    Filter
		want []string
	}{
		{"all newest first", Filter{}, []string{"5", "4", "3", "2", "1"}},
		{"repo", Filter{RepoKey: "r1"}, []string{"5", "2", "1"}},
		{"path", Filter{Path: "src/a.go"}, []string{"5", "3", "1"}},
		{"path backslash", Filter{Path: `src\a.go`}, []string{"5", "3", "1"}},
		{"path exact", Filter{Path: "src/a"}, nil},
		{"repo and path", Filter{RepoKey: "r2", Path: "src/a.go"}, []string{"3"}},
		{"query statement", Filter{Query: "sqlite"}, []string{"4", "1"}},
		{"query why", Filter{Query: "flaky"}, []string{"2"}},
		{"query trimmed", Filter{Query: "  Flaky "}, []string{"2"}},
		{"query none", Filter{Query: "zebra"}, nil},
		{"limit", Filter{Limit: 2}, []string{"5", "4"}},
	}
	for _, c := range cases {
		if got := ids(v.List(c.f)); !slices.Equal(got, c.want) {
			t.Errorf("%s: %v, want %v", c.name, got, c.want)
		}
	}
}

func TestListLimit(t *testing.T) {
	v := NewView()
	for i := range 600 {
		v.Add(dec(fmt.Sprint(i), "k", "claude", i))
	}
	cases := []struct{ limit, want int }{{0, 50}, {-3, 50}, {1, 1}, {50, 50}, {499, 499}, {500, 500}, {501, 500}, {10000, 500}}
	for _, c := range cases {
		got := v.List(Filter{Limit: c.limit})
		if len(got) != c.want {
			t.Errorf("limit %d: got %d, want %d", c.limit, len(got), c.want)
		}
		if got[0].ID != "599" {
			t.Errorf("limit %d: first %s", c.limit, got[0].ID)
		}
	}
}

func TestViewConcurrent(t *testing.T) {
	v := NewView()
	var wg sync.WaitGroup
	for i := range 20 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := range 50 {
				id := fmt.Sprintf("%d-%d", i, j)
				v.Add(dec(id, "k", "claude", j, fmt.Sprintf("%d-%d", i, j-1)))
				v.Get(id)
				v.List(Filter{Query: "stmt"})
				v.CanSupersede(Party{Key: "k"}, id)
			}
		}()
	}
	wg.Wait()
	if n := len(v.List(Filter{All: true, Limit: 500})); n != 500 {
		t.Fatalf("all = %d", n)
	}
	// Each chain's last link is the only one not superseded.
	if n := len(v.List(Filter{Limit: 500})); n != 20 {
		t.Fatalf("live = %d", n)
	}
}
