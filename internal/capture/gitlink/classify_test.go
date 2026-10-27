package gitlink

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/gitsha"
)

const (
	blobX = "1234567890123456789012345678901234567890"
	blobY = "9876543210987654321098765432109876543210"
	blobZ = "abababababababababababababababababababab"
)

func at(s int) time.Time { return time.Unix(1700000000+int64(s), 0) }

func linkString(ls []FileLink) string {
	var parts []string
	for _, l := range ls {
		parts = append(parts, l.Path+"="+l.Link+"["+strings.Join(l.Edits, ",")+"]")
	}
	return strings.Join(parts, " ")
}

func TestClassify(t *testing.T) {
	const rootA = `C:\w\main`
	const rootB = `C:\w\agent`
	mod := func(path, blob string) FileChange { return FileChange{Path: path, Blob: blob, Status: "M"} }
	tests := []struct {
		name  string
		files []FileChange
		root  string
		cands []EditRef
		want  string
	}{
		{
			name:  "exact by raw blob links the run",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "e1", Root: rootA, Path: "a.go", AfterBlob: blobY, At: at(1)},
				{ID: "e2", Root: rootA, Path: "a.go", AfterBlob: blobX, At: at(2)},
				{ID: "e3", Root: rootA, Path: "b.go", AfterBlob: blobX, At: at(1)},
			},
			want: "a.go=exact[e1,e2]",
		},
		{
			name:  "exact by LF blob",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{{ID: "e1", Root: rootA, Path: "a.go", AfterBlob: blobY, AfterBlobLF: blobX, At: at(1)}},
			want:  "a.go=exact[e1]",
		},
		{
			name:  "exact ignores hex case",
			files: []FileChange{mod("a.go", "abcdefabcdefabcdefabcdefabcdefabcdefabcd")},
			root:  rootA,
			cands: []EditRef{{ID: "e1", Root: rootA, Path: "a.go", AfterBlob: "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD", At: at(1)}},
			want:  "a.go=exact[e1]",
		},
		{
			name:  "exact newest match wins, later edits excluded, heuristic earlier ones included",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "e4", Root: rootA, Path: "a.go", AfterBlob: blobZ, At: at(9)},
				{ID: "e2", Root: rootA, Path: "a.go", AfterBlob: blobX, At: at(5)},
				{ID: "h1", Root: rootA, Path: "a.go", Heuristic: true, At: at(3)},
				{ID: "e1", Root: rootA, Path: "a.go", AfterBlob: blobX, At: at(1)},
			},
			want: "a.go=exact[e1,h1,e2]",
		},
		{
			name:  "exact from another root (deleted agent worktree) links that root's run only",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "a1", Root: rootA, Path: "a.go", AfterBlob: blobY, At: at(1)},
				{ID: "b1", Root: rootB, Path: "a.go", AfterBlob: blobZ, At: at(2)},
				{ID: "b2", Root: rootB, Path: "a.go", AfterBlob: blobX, At: at(3)},
			},
			want: "a.go=exact[b1,b2]",
		},
		{
			name:  "exact run root compares slash style and case",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "b1", Root: "c:/W/agent/", Path: "a.go", AfterBlob: blobZ, At: at(2)},
				{ID: "b2", Root: rootB, Path: "a.go", AfterBlob: blobX, At: at(3)},
			},
			want: "a.go=exact[b1,b2]",
		},
		{
			name:  "a heuristic edit never matches by blob",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{{ID: "h1", Root: rootB, Path: "a.go", AfterBlob: blobX, Heuristic: true, At: at(1)}},
			want:  "a.go=unattributed[]",
		},
		{
			name:  "partial: own root, no blob match",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "a2", Root: rootA, Path: "a.go", AfterBlob: blobZ, At: at(4)},
				{ID: "a1", Root: rootA, Path: "a.go", At: at(1)},
				{ID: "h1", Root: rootA, Path: "a.go", Heuristic: true, At: at(2)},
			},
			want: "a.go=partial[a1,h1,a2]",
		},
		{
			name:  "partial only from the commit's own root",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{{ID: "b1", Root: rootB, Path: "a.go", AfterBlob: blobZ, At: at(1)}},
			want:  "a.go=unattributed[]",
		},
		{
			name:  "heuristic: only bash edits from own root",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "h2", Root: rootA, Path: "a.go", Heuristic: true, At: at(2)},
				{ID: "h1", Root: rootA, Path: "a.go", Heuristic: true, At: at(1)},
				{ID: "b1", Root: rootB, Path: "a.go", At: at(1)},
			},
			want: "a.go=heuristic[h1,h2]",
		},
		{
			name:  "paths compare exactly",
			files: []FileChange{mod("A.go", blobX), mod("dir/a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "e1", Root: rootA, Path: "a.go", AfterBlob: blobX, At: at(1)},
				{ID: "e2", Root: rootA, Path: `dir\a.go`, AfterBlob: blobX, At: at(1)},
			},
			want: "A.go=unattributed[] dir/a.go=unattributed[]",
		},
		{
			name:  "deletion links by root, never by blob",
			files: []FileChange{{Path: "a.go", Blob: zero40, Status: "D"}},
			root:  rootA,
			cands: []EditRef{
				{ID: "b1", Root: rootB, Path: "a.go", AfterBlob: zero40, At: at(1)},
				{ID: "a1", Root: rootA, Path: "a.go", AfterBlob: blobY, At: at(2)},
			},
			want: "a.go=partial[a1]",
		},
		{
			name:  "deletion with no own-root edits",
			files: []FileChange{{Path: "a.go", Blob: zero40, Status: "D"}},
			root:  rootA,
			cands: []EditRef{{ID: "b1", Root: rootB, Path: "a.go", AfterBlob: zero40, At: at(1)}},
			want:  "a.go=unattributed[]",
		},
		{
			name:  "an edit with no after blob never matches an empty commit blob",
			files: []FileChange{{Path: "a.go", Blob: "", Status: "M"}},
			root:  rootA,
			cands: []EditRef{{ID: "b1", Root: rootB, Path: "a.go", At: at(1)}},
			want:  "a.go=unattributed[]",
		},
		{
			name:  "equal times keep candidate order",
			files: []FileChange{mod("a.go", blobX)},
			root:  rootA,
			cands: []EditRef{
				{ID: "x", Root: rootA, Path: "a.go", At: at(1)},
				{ID: "y", Root: rootA, Path: "a.go", AfterBlob: blobX, At: at(1)},
			},
			want: "a.go=exact[x,y]",
		},
		{
			name:  "no candidates",
			files: []FileChange{mod("a.go", blobX), mod("b.go", blobY)},
			root:  rootA,
			want:  "a.go=unattributed[] b.go=unattributed[]",
		},
		{name: "no files", root: rootA, want: ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := Classify(Commit{Files: tt.files}, tt.root, tt.cands)
			if len(got) != len(tt.files) {
				t.Fatalf("%d links for %d files", len(got), len(tt.files))
			}
			if s := linkString(got); s != tt.want {
				t.Fatalf("got  %s\nwant %s", s, tt.want)
			}
		})
	}
}

// TestClassifyTwoRoots: two agents edit the same path in two worktrees and
// each commits there. Each commit credits only its own agent.
func TestClassifyTwoRoots(t *testing.T) {
	r := newRepo(t, "false")
	r.write("foo.go", "package foo\n")
	r.commit("base", 1700000000)
	wtA := filepath.Join(filepath.Dir(r.root), "wtA")
	wtB := filepath.Join(filepath.Dir(r.root), "wtB")
	r.git("worktree", "add", "-q", "-b", "a", wtA)
	r.git("worktree", "add", "-q", "-b", "b", wtB)

	contentA := "package foo\n\n// A\n"
	contentB := "package foo\n\n// B\n"
	// A's edit matches its commit; B's last edit was rewritten by a
	// formatter before B committed.
	cands := []EditRef{
		{ID: "a1", Root: wtA, Path: "foo.go", AfterBlob: gitsha.Blob([]byte(contentA)), At: at(1)},
		{ID: "b1", Root: wtB, Path: "foo.go", AfterBlob: gitsha.Blob([]byte(contentB + " ")), At: at(2)},
	}
	if err := os.WriteFile(filepath.Join(wtA, "foo.go"), []byte(contentA), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(wtB, "foo.go"), []byte(contentB), 0o644); err != nil {
		t.Fatal(err)
	}
	shaA := commitIn(t, wtA, "A", 1700000010)
	shaB := commitIn(t, wtB, "B", 1700000020)

	ca, err := Describe(ctx(t), Exec, wtA, shaA)
	if err != nil {
		t.Fatal(err)
	}
	cb, err := Describe(ctx(t), Exec, wtB, shaB)
	if err != nil {
		t.Fatal(err)
	}
	if s := linkString(Classify(ca, wtA, cands)); s != "foo.go=exact[a1]" {
		t.Fatalf("A: %s", s)
	}
	if s := linkString(Classify(cb, wtB, cands)); s != "foo.go=partial[b1]" {
		t.Fatalf("B: %s", s)
	}
	// Committed from main (say after merging A), A's blob still links
	// exactly to A.
	if s := linkString(Classify(ca, r.root, cands)); s != "foo.go=exact[a1]" {
		t.Fatalf("A from main: %s", s)
	}
}

// TestClassifyRealBlobs links commits made by real git to edits hashed with
// gitsha, with and without core.autocrlf.
func TestClassifyRealBlobs(t *testing.T) {
	tests := []struct {
		name     string
		autocrlf string
		content  string
		useRaw   bool // edit carries only the raw hash
		want     string
	}{
		{name: "raw blob, LF content", autocrlf: "false", content: "one\ntwo\n", useRaw: true, want: "exact"},
		{name: "raw blob, CRLF kept", autocrlf: "false", content: "one\r\ntwo\r\n", useRaw: true, want: "exact"},
		{name: "CRLF committed as LF via autocrlf, LF hash", autocrlf: "true", content: "one\r\ntwo\r\n", want: "exact"},
		{name: "CRLF committed as LF, raw hash alone misses", autocrlf: "true", content: "one\r\ntwo\r\n", useRaw: true, want: "partial"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := newRepo(t, tt.autocrlf)
			r.write("keep.txt", "k\n")
			r.commit("base", 1700000000)

			raw, lf := gitsha.Pair([]byte(tt.content))
			e := EditRef{ID: "e1", Root: r.root, Path: "src/f.txt", AfterBlob: raw, At: at(1)}
			if !tt.useRaw {
				e.AfterBlobLF = lf
			}
			r.write("src/f.txt", tt.content)
			r.git("rm", "-q", "keep.txt")
			r.write("other.txt", "unrelated\n")
			sha := r.commit("work", 1700000100)

			c, err := Describe(ctx(t), Exec, r.root, sha)
			if err != nil {
				t.Fatal(err)
			}
			got := linkString(Classify(c, r.root, []EditRef{e,
				{ID: "d1", Root: r.root, Path: "keep.txt", AfterBlob: gitsha.Blob([]byte("k\n")), At: at(2)}}))
			want := "keep.txt=partial[d1] other.txt=unattributed[] src/f.txt=" + tt.want + "[e1]"
			if got != want {
				t.Fatalf("got  %s\nwant %s", got, want)
			}
		})
	}
}

// TestClassifyFastForwardFromRemovedWorktree is the v0.2 flow: an agent
// commits in a worktree-wf_* worktree, main fast-forwards, the worktree is
// removed. The commit seen in main links to the agent's edits by blob.
func TestClassifyFastForwardFromRemovedWorktree(t *testing.T) {
	r := newRepo(t, "true")
	r.write("a.txt", "a\n")
	r.commit("base", 1700000000)
	wt := filepath.Join(filepath.Dir(r.root), "worktree-wf_1")
	r.git("worktree", "add", "-q", "-b", "wf", wt)
	content := "line one\r\nline two\r\n"
	raw, lf := gitsha.Pair([]byte(content))
	if err := os.WriteFile(filepath.Join(wt, "a.txt"), []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	sha := commitIn(t, wt, "agent work", 1700000100)
	r.git("worktree", "remove", wt)
	r.git("merge", "-q", "--ff-only", "wf")

	log := r.reflog()
	shas, _, err := NewCommits(ctx(t), Exec, r.root, log[len(log)-1], nil)
	if err != nil || strings.Join(shas, ",") != sha {
		t.Fatalf("NewCommits = %v, %v", shas, err)
	}
	c, err := Describe(ctx(t), Exec, r.root, sha)
	if err != nil {
		t.Fatal(err)
	}
	cands := []EditRef{{ID: "e1", Root: wt, Path: "a.txt", AfterBlob: raw, AfterBlobLF: lf, At: at(1)}}
	if s := linkString(Classify(c, r.root, cands)); s != "a.txt=exact[e1]" {
		t.Fatalf("got %s", s)
	}
}

func TestCommitter(t *testing.T) {
	c := Commit{At: at(100)}
	tests := []struct {
		name string
		root string
		cmds []CommitCmd
		want string // TurnID, "" for nil
	}{
		{
			name: "closest wins",
			root: `C:\a\b`,
			cmds: []CommitCmd{
				{TurnID: "far", Cwd: `C:\a\b`, At: at(92)},
				{TurnID: "near", Cwd: `C:\a\b`, At: at(101)},
			},
			want: "near",
		},
		{
			name: "tie on distance: latest wins",
			root: `C:\a\b`,
			cmds: []CommitCmd{
				{TurnID: "after", Cwd: `C:\a\b`, At: at(103)},
				{TurnID: "before", Cwd: `C:\a\b`, At: at(97)},
			},
			want: "after",
		},
		{
			name: "window is inclusive at 10s",
			root: `C:\a\b`,
			cmds: []CommitCmd{{TurnID: "edge", Cwd: `C:\a\b`, At: at(90)}},
			want: "edge",
		},
		{
			name: "outside the window",
			root: `C:\a\b`,
			cmds: []CommitCmd{
				{TurnID: "early", Cwd: `C:\a\b`, At: at(89)},
				{TurnID: "late", Cwd: `C:\a\b`, At: at(100).Add(CommitterWindow + time.Millisecond)},
			},
		},
		{
			name: "case and slash insensitive on Windows paths, subdir counts",
			root: `C:\a\b`,
			cmds: []CommitCmd{{TurnID: "sub", Cwd: "c:/A/b/c", At: at(100)}},
			want: "sub",
		},
		{
			name: "Git Bash style cwd",
			root: `C:\a\b`,
			cmds: []CommitCmd{{TurnID: "bash", Cwd: "/c/a/b/c", At: at(100)}},
			want: "bash",
		},
		{
			name: "sibling with the root as a name prefix is not under it",
			root: `C:\a\b`,
			cmds: []CommitCmd{{TurnID: "sib", Cwd: `C:\a\bc`, At: at(100)}},
		},
		{
			name: "other root ignored even when closer",
			root: `C:\a\b`,
			cmds: []CommitCmd{
				{TurnID: "other", Cwd: `C:\a\x`, At: at(100)},
				{TurnID: "mine", Cwd: `C:\a\b\`, At: at(105)},
			},
			want: "mine",
		},
		{
			name: "POSIX paths are case-sensitive",
			root: "/home/u/repo",
			cmds: []CommitCmd{
				{TurnID: "upper", Cwd: "/home/u/Repo", At: at(100)},
				{TurnID: "ok", Cwd: "/home/u/repo/sub", At: at(102)},
			},
			want: "ok",
		},
		{
			name: "empty cwd never matches",
			root: `C:\a\b`,
			cmds: []CommitCmd{{TurnID: "none", Cwd: "", At: at(100)}},
		},
		{
			name: "empty root matches nothing",
			root: "",
			cmds: []CommitCmd{{TurnID: "x", Cwd: `C:\a`, At: at(100)}},
		},
		{name: "no commands", root: `C:\a\b`},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := Committer(c, tt.root, tt.cmds)
			id := ""
			if got != nil {
				id = got.TurnID
			}
			if id != tt.want {
				t.Fatalf("got %q, want %q", id, tt.want)
			}
		})
	}
}

func TestUnderRoot(t *testing.T) {
	tests := []struct {
		dir, root string
		want      bool
	}{
		{`C:\a\b`, `C:\a\b`, true},
		{`c:/a/b/c`, `C:\a\b`, true},
		{`C:\A\B\C\`, `c:/a/b/`, true},
		{`C:\a\bc`, `C:\a\b`, false},
		{`C:\a`, `C:\a\b`, false},
		{`D:\a\b`, `C:\a\b`, false},
		{`C:\x`, `C:\`, true},
		{`c:/`, `C:\`, true},
		{`/c/a/b`, `C:\a\b`, true},
		{`/d/a/b`, `C:\a\b`, false},
		{`\\host\share\repo\x`, `\\HOST\share\repo`, true},
		{"/home/u/repo", "/home/u/repo", true},
		{"/home/u/repo/x", "/home/u/repo/", true},
		{"/home/u/Repo", "/home/u/repo", false},
		{"/x", "/", true},
		{"", `C:\a`, false},
	}
	for _, tt := range tests {
		if got := underRoot(tt.dir, tt.root); got != tt.want {
			t.Errorf("underRoot(%q, %q) = %v", tt.dir, tt.root, got)
		}
	}
}
