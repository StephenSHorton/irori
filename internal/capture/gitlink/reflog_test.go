package gitlink

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"
)

const (
	zero40 = "0000000000000000000000000000000000000000"
	shaA   = "1111111111111111111111111111111111111111"
	shaB   = "2222222222222222222222222222222222222222"
	shaC   = "abcdefabcdefabcdefabcdefabcdefabcdefabcd"
)

func TestParseReflog(t *testing.T) {
	zero64 := strings.Repeat("0", 64)
	sha256 := strings.Repeat("ab", 32)
	tests := []struct {
		name     string
		data     string
		want     []ReflogEntry
		consumed int // -1 means len(data)
	}{
		{
			name: "commit line",
			data: shaA + " " + shaB + " Jo Coder <jo@example.invalid> 1700000000 +0130\tcommit: add things\n",
			want: []ReflogEntry{{Old: shaA, New: shaB, Name: "Jo Coder", Email: "jo@example.invalid",
				At: time.Unix(1700000000, 0), Message: "commit: add things"}},
			consumed: -1,
		},
		{
			name: "initial commit, negative zone",
			data: zero40 + " " + shaA + " A <a@x> 1700000001 -0700\tcommit (initial): first\n",
			want: []ReflogEntry{{Old: zero40, New: shaA, Name: "A", Email: "a@x",
				At: time.Unix(1700000001, 0), Message: "commit (initial): first"}},
			consumed: -1,
		},
		{
			name: "every op kind",
			data: shaA + " " + shaB + " A <a@x> 1 +0000\tmerge feat: Fast-forward\n" +
				shaB + " " + shaC + " A <a@x> 2 +0000\tpull: Fast-forward\n" +
				shaC + " " + shaA + " A <a@x> 3 +0000\trebase (finish): returning to refs/heads/main\n" +
				shaA + " " + shaB + " A <a@x> 4 +0000\tcherry-pick: fix\n" +
				shaB + " " + shaC + " A <a@x> 5 +0000\trevert: Revert \"x\"\n" +
				shaC + " " + shaA + " A <a@x> 6 +0000\tcheckout: moving from main to feat\n" +
				shaA + " " + shaB + " A <a@x> 7 +0000\treset: moving to HEAD~1\n" +
				shaB + " " + shaC + " A <a@x> 8 +0000\tcommit (amend): fix\n" +
				shaC + " " + shaA + " A <a@x> 9 +0000\tcommit (merge): Merge branch 'x'\n",
			want: []ReflogEntry{
				{Old: shaA, New: shaB, Name: "A", Email: "a@x", At: time.Unix(1, 0), Message: "merge feat: Fast-forward"},
				{Old: shaB, New: shaC, Name: "A", Email: "a@x", At: time.Unix(2, 0), Message: "pull: Fast-forward"},
				{Old: shaC, New: shaA, Name: "A", Email: "a@x", At: time.Unix(3, 0), Message: "rebase (finish): returning to refs/heads/main"},
				{Old: shaA, New: shaB, Name: "A", Email: "a@x", At: time.Unix(4, 0), Message: "cherry-pick: fix"},
				{Old: shaB, New: shaC, Name: "A", Email: "a@x", At: time.Unix(5, 0), Message: "revert: Revert \"x\""},
				{Old: shaC, New: shaA, Name: "A", Email: "a@x", At: time.Unix(6, 0), Message: "checkout: moving from main to feat"},
				{Old: shaA, New: shaB, Name: "A", Email: "a@x", At: time.Unix(7, 0), Message: "reset: moving to HEAD~1"},
				{Old: shaB, New: shaC, Name: "A", Email: "a@x", At: time.Unix(8, 0), Message: "commit (amend): fix"},
				{Old: shaC, New: shaA, Name: "A", Email: "a@x", At: time.Unix(9, 0), Message: "commit (merge): Merge branch 'x'"},
			},
			consumed: -1,
		},
		{
			name: "CRLF line, empty message with no tab, empty name",
			data: shaA + " " + shaB + " A <a@x> 10 +0000\r\n" +
				shaB + " " + shaC + "  <b@x> 11 +0000\tcommit: x\r\n",
			want: []ReflogEntry{
				{Old: shaA, New: shaB, Name: "A", Email: "a@x", At: time.Unix(10, 0)},
				{Old: shaB, New: shaC, Name: "", Email: "b@x", At: time.Unix(11, 0), Message: "commit: x"},
			},
			consumed: -1,
		},
		{
			name:     "message keeps later tabs, uppercase ids lowered",
			data:     strings.ToUpper(shaC) + " " + shaA + " A <a@x> 12 +0000\tcommit: a\tb\n",
			want:     []ReflogEntry{{Old: shaC, New: shaA, Name: "A", Email: "a@x", At: time.Unix(12, 0), Message: "commit: a\tb"}},
			consumed: -1,
		},
		{
			name:     "sha256 ids",
			data:     zero64 + " " + sha256 + " A <a@x> 13 +0000\tcommit (initial): x\n",
			want:     []ReflogEntry{{Old: zero64, New: sha256, Name: "A", Email: "a@x", At: time.Unix(13, 0), Message: "commit (initial): x"}},
			consumed: -1,
		},
		{
			name: "malformed lines skipped, good ones kept",
			data: "garbage\n" +
				"\n" +
				shaA[:7] + " " + shaB + " A <a@x> 1 +0000\tshort old\n" +
				shaA + " " + shaB[:39] + " A <a@x> 1 +0000\tshort new\n" +
				shaA + " " + strings.Repeat("g", 40) + " A <a@x> 1 +0000\tnot hex\n" +
				shaA + " " + strings.Repeat("ab", 32) + " A <a@x> 1 +0000\tmixed lengths\n" +
				shaA + " " + shaB + " A a@x 1 +0000\tno brackets\n" +
				shaA + " " + shaB + " A <a@x> soon +0000\tbad time\n" +
				shaA + " " + shaB + " A <a@x> 1 0000\tbad zone\n" +
				shaA + " " + shaB + " A <a@x> 1 +0099\tbad minutes\n" +
				shaA + " " + shaB + " A <a@x> 1\tno zone\n" +
				shaA + " " + shaB + " A <a@x> 14 +0000\tgood\n",
			want:     []ReflogEntry{{Old: shaA, New: shaB, Name: "A", Email: "a@x", At: time.Unix(14, 0), Message: "good"}},
			consumed: -1,
		},
		{
			name: "trailing partial line left for next time",
			data: shaA + " " + shaB + " A <a@x> 15 +0000\tcommit: one\n" +
				shaB + " " + shaC + " A <a@x> 16 +00",
			want:     []ReflogEntry{{Old: shaA, New: shaB, Name: "A", Email: "a@x", At: time.Unix(15, 0), Message: "commit: one"}},
			consumed: len(shaA + " " + shaB + " A <a@x> 15 +0000\tcommit: one\n"),
		},
		{name: "only a partial line", data: shaA + " " + shaB, consumed: 0},
		{name: "empty", data: "", consumed: 0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, n := ParseReflog([]byte(tt.data))
			want := tt.consumed
			if want < 0 {
				want = len(tt.data)
			}
			if n != want {
				t.Fatalf("consumed %d, want %d", n, want)
			}
			if len(got) != len(tt.want) {
				t.Fatalf("got %d entries, want %d: %+v", len(got), len(tt.want), got)
			}
			for i := range got {
				g, w := got[i], tt.want[i]
				if g.Old != w.Old || g.New != w.New || g.Name != w.Name || g.Email != w.Email ||
					g.Message != w.Message || !g.At.Equal(w.At) {
					t.Errorf("entry %d = %+v, want %+v", i, g, w)
				}
			}
		})
	}
}

func TestParseReflogZone(t *testing.T) {
	got, _ := ParseReflog([]byte(shaA + " " + shaB + " A <a@x> 1700000000 -0730\tx\n"))
	if len(got) != 1 {
		t.Fatal("no entry")
	}
	if _, off := got[0].At.Zone(); off != -(7*3600 + 30*60) {
		t.Fatalf("zone offset %d", off)
	}
}

func TestZeroSHA(t *testing.T) {
	tests := []struct {
		in   string
		want bool
	}{
		{zero40, true},
		{strings.Repeat("0", 64), true},
		{"0", true},
		{"", false},
		{shaA, false},
		{"000000000000000000000000000000000000000a", false},
	}
	for _, tt := range tests {
		if got := ZeroSHA(tt.in); got != tt.want {
			t.Errorf("ZeroSHA(%q) = %v", tt.in, got)
		}
	}
}

// samePath compares two filesystem paths after cleaning, ignoring case on
// Windows.
func samePath(a, b string) bool {
	a, b = filepath.Clean(a), filepath.Clean(b)
	if runtime.GOOS == "windows" {
		return strings.EqualFold(a, b)
	}
	return a == b
}

func TestGitDir(t *testing.T) {
	r := newRepo(t, "false")
	r.write("a.txt", "a\n")
	r.commit("one", 1700000000)

	got, err := GitDir(r.root)
	if err != nil || !samePath(got, filepath.Join(r.root, ".git")) {
		t.Fatalf("main GitDir = %q, %v", got, err)
	}

	wt := filepath.Join(filepath.Dir(r.root), "linked")
	r.git("worktree", "add", "-q", "-b", "feat", wt)
	got, err = GitDir(wt)
	want := gitIn(t, wt, nil, "", "rev-parse", "--absolute-git-dir")
	if err != nil || !samePath(got, filepath.FromSlash(want)) {
		t.Fatalf("linked GitDir = %q, %v; git says %q", got, err, want)
	}
	if _, err := os.Stat(filepath.Join(got, "logs", "HEAD")); err != nil {
		t.Fatalf("linked worktree logs/HEAD: %v", err)
	}

	synth := func(content string) string {
		d := t.TempDir()
		if err := os.WriteFile(filepath.Join(d, ".git"), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
		return d
	}

	d := synth("gitdir: ../main/.git/worktrees/x\n")
	got, err = GitDir(d)
	if err != nil || !samePath(got, filepath.Join(filepath.Dir(d), "main", ".git", "worktrees", "x")) {
		t.Fatalf("relative GitDir = %q, %v", got, err)
	}

	abs := filepath.Join(t.TempDir(), "elsewhere", "worktrees", "y")
	d = synth("\xef\xbb\xbfgitdir: " + filepath.ToSlash(abs) + "\r\n")
	got, err = GitDir(d)
	if err != nil || !samePath(got, abs) {
		t.Fatalf("BOM+CRLF GitDir = %q, %v, want %q", got, err, abs)
	}

	for _, bad := range []string{"", "nothing here\n", "gitdir:   \n"} {
		if got, err := GitDir(synth(bad)); err == nil {
			t.Errorf("GitDir with .git %q = %q, want error", bad, got)
		}
	}
	if _, err := GitDir(t.TempDir()); err == nil {
		t.Error("GitDir without .git: want error")
	}
}

// TestReflogMovements runs the §5 watcher's steps on a real repo: initial
// commit, commit, a fast-forward from a removed linked worktree, and
// checkouts to known commits.
func TestReflogMovements(t *testing.T) {
	r := newRepo(t, "false")
	seen := seenSet{}
	c := ctx(t)
	take := func(e ReflogEntry) []string {
		t.Helper()
		shas, skipped, err := NewCommits(c, Exec, r.root, e, seen.has)
		if err != nil || skipped != 0 {
			t.Fatalf("NewCommits(%s): %v, skipped %d", e.Message, err, skipped)
		}
		for _, s := range shas {
			seen[s] = true
		}
		return shas
	}
	eq := func(got []string, want ...string) {
		t.Helper()
		if strings.Join(got, ",") != strings.Join(want, ",") {
			t.Fatalf("got %v, want %v", got, want)
		}
	}

	r.write("a.txt", "a\n")
	c1 := r.commit("first", 1700000000)
	log := r.reflog()
	if len(log) != 1 || !ZeroSHA(log[0].Old) || log[0].New != c1 ||
		!strings.HasPrefix(log[0].Message, "commit (initial): first") {
		t.Fatalf("initial reflog %+v", log)
	}
	if log[0].Name != "Test Committer" || log[0].Email != "committer@example.invalid" {
		t.Fatalf("identity %q <%q>", log[0].Name, log[0].Email)
	}
	eq(take(log[0]), c1)

	r.write("a.txt", "a2\n")
	c2 := r.commit("second", 1700000100)
	log = r.reflog()
	if len(log) != 2 || log[1].Old != c1 || log[1].New != c2 || log[1].Message != "commit: second" {
		t.Fatalf("commit reflog %+v", log)
	}
	eq(take(log[1]), c2)
	// The same movement again adds nothing.
	eq(take(log[1]))

	// Two commits in a linked worktree, which is then removed with its
	// reflog; main fast-forwards over them.
	wt := filepath.Join(filepath.Dir(r.root), "wt")
	r.git("worktree", "add", "-q", "-b", "feat", wt)
	if err := os.WriteFile(filepath.Join(wt, "b.txt"), []byte("b\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	f1 := commitIn(t, wt, "feat one", 1700000200)
	if err := os.WriteFile(filepath.Join(wt, "b.txt"), []byte("b2\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	f2 := commitIn(t, wt, "feat two", 1700000300)
	r.git("worktree", "remove", wt)
	if _, err := os.Stat(wt); !os.IsNotExist(err) {
		t.Fatalf("worktree still there: %v", err)
	}
	r.git("merge", "-q", "--ff-only", "feat")
	log = r.reflog()
	ff := log[len(log)-1]
	if ff.Old != c2 || ff.New != f2 || !strings.Contains(ff.Message, "Fast-forward") {
		t.Fatalf("ff reflog %+v", ff)
	}
	eq(take(ff), f1, f2)

	// Checkouts to known commits add nothing, in both directions.
	r.git("checkout", "-q", c1)
	r.git("checkout", "-q", "main")
	log = r.reflog()
	for _, e := range log[len(log)-2:] {
		if !strings.HasPrefix(e.Message, "checkout:") {
			t.Fatalf("expected checkout lines, got %+v", e)
		}
		eq(take(e))
	}

	// A reset back then a commit: only the new commit.
	r.git("reset", "-q", "--hard", c2)
	r.write("a.txt", "a3\n")
	c3 := r.commit("third", 1700000400)
	log = r.reflog()
	for _, e := range log[len(log)-2:] {
		take(e)
	}
	if !seen[c3] || len(seen) != 5 {
		t.Fatalf("seen = %v", seen)
	}
}

func TestReflogPartialLineOnDisk(t *testing.T) {
	r := newRepo(t, "false")
	r.write("a.txt", "a\n")
	r.commit("one", 1700000000)
	dir, _ := GitDir(r.root)
	data, err := os.ReadFile(filepath.Join(dir, "logs", "HEAD"))
	if err != nil {
		t.Fatal(err)
	}
	half := append(append([]byte{}, data...), []byte(shaA+" "+shaB+" A <a@")...)
	got, n := ParseReflog(half)
	if len(got) != 1 || n != len(data) {
		t.Fatalf("got %d entries, consumed %d of %d", len(got), n, len(data))
	}
}

// fastImport makes n linear commits on main in one git process and returns
// their shas oldest first.
func fastImport(r *testRepo, n int) []string {
	var b strings.Builder
	for i := 1; i <= n; i++ {
		msg := "c" + strconv.Itoa(i)
		body := strconv.Itoa(i) + "\n"
		b.WriteString("commit refs/heads/main\n")
		b.WriteString("committer T <t@example.invalid> " + strconv.Itoa(1700000000+i) + " +0000\n")
		b.WriteString("data " + strconv.Itoa(len(msg)) + "\n" + msg + "\n")
		b.WriteString("M 100644 inline f.txt\n")
		b.WriteString("data " + strconv.Itoa(len(body)) + "\n" + body + "\n")
	}
	gitIn(r.t, r.root, nil, b.String(), "fast-import", "--quiet")
	return strings.Fields(r.git("rev-list", "--reverse", "main"))
}

func TestNewCommitsMaxWalk(t *testing.T) {
	r := newRepo(t, "false")
	all := fastImport(r, MaxWalk+5)
	tip := all[len(all)-1]

	shas, skipped, err := NewCommits(ctx(t), Exec, r.root, ReflogEntry{Old: zero40, New: tip}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(shas) != MaxWalk || skipped != 5 {
		t.Fatalf("got %d, skipped %d", len(shas), skipped)
	}
	// The newest MaxWalk, oldest first.
	if shas[0] != all[5] || shas[len(shas)-1] != tip {
		t.Fatalf("walk %s..%s, want %s..%s", shas[0], shas[len(shas)-1], all[5], tip)
	}

	// A range: <old>..<new> holds MaxWalk+2 commits.
	shas, skipped, err = NewCommits(ctx(t), Exec, r.root, ReflogEntry{Old: all[2], New: tip}, nil)
	if err != nil || len(shas) != MaxWalk || skipped != 2 {
		t.Fatalf("range: %d, skipped %d, %v", len(shas), skipped, err)
	}

	// Seen commits are dropped but still count toward the cap.
	seen := seenSet{all[10]: true, all[11]: true}
	shas, skipped, err = NewCommits(ctx(t), Exec, r.root, ReflogEntry{Old: zero40, New: tip}, seen.has)
	if err != nil || len(shas) != MaxWalk-2 || skipped != 5 {
		t.Fatalf("seen: %d, skipped %d, %v", len(shas), skipped, err)
	}

	// Under the cap: no skipped count.
	shas, skipped, err = NewCommits(ctx(t), Exec, r.root, ReflogEntry{Old: all[len(all)-4], New: tip}, nil)
	if err != nil || len(shas) != 3 || skipped != 0 {
		t.Fatalf("small range: %v, skipped %d, %v", shas, skipped, err)
	}
}

// recordGit is a fake Git that answers by args and records each call.
type recordGit struct {
	answers map[string]string // joined args -> stdout; missing is an error
	calls   []string
}

func (g *recordGit) Run(_ context.Context, dir string, args ...string) (string, error) {
	k := strings.Join(args, " ")
	g.calls = append(g.calls, k)
	if out, ok := g.answers[k]; ok {
		return out, nil
	}
	return "", errors.New("exit status 128")
}

func TestNewCommitsFake(t *testing.T) {
	maxc := "--max-count=" + strconv.Itoa(MaxWalk)

	// New already seen: no git at all.
	g := &recordGit{}
	shas, skipped, err := NewCommits(context.Background(), g, "r", ReflogEntry{Old: shaA, New: shaB},
		func(s string) bool { return s == shaB })
	if err != nil || shas != nil || skipped != 0 || len(g.calls) != 0 {
		t.Fatalf("seen new: %v %d %v calls %v", shas, skipped, err, g.calls)
	}

	// A zero New (never written for HEAD, but harmless): nothing.
	if shas, _, err := NewCommits(context.Background(), g, "r", ReflogEntry{Old: shaA, New: zero40}, nil); err != nil || shas != nil || len(g.calls) != 0 {
		t.Fatalf("zero new: %v %v", shas, err)
	}

	// Malformed ids never reach git.
	for _, e := range []ReflogEntry{{Old: shaA, New: "--output=x"}, {Old: "HEAD~1", New: shaB}, {New: "abc"}} {
		if _, _, err := NewCommits(context.Background(), g, "r", e, nil); err == nil {
			t.Errorf("entry %+v: want error", e)
		}
	}
	if len(g.calls) != 0 {
		t.Fatalf("calls %v", g.calls)
	}

	// Old object gone: falls back to walking New alone.
	g = &recordGit{answers: map[string]string{
		"rev-list --reverse " + maxc + " " + shaB + " --": shaA + "\n" + shaB + "\n",
	}}
	shas, _, err = NewCommits(context.Background(), g, "r", ReflogEntry{Old: shaC, New: shaB},
		func(s string) bool { return s == shaA })
	if err != nil || strings.Join(shas, ",") != shaB || len(g.calls) != 2 {
		t.Fatalf("fallback: %v %v calls %v", shas, err, g.calls)
	}

	// Both walks fail: the error comes back.
	g = &recordGit{}
	if _, _, err := NewCommits(context.Background(), g, "r", ReflogEntry{Old: shaC, New: shaB}, nil); err == nil {
		t.Fatal("want error")
	}
}
