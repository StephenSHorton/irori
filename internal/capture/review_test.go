package capture

import (
	"context"
	"errors"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
)

// Regression tests for the capture review findings.

func TestAmendKeepsExactLink(t *testing.T) {
	for _, tt := range []struct {
		name string
		redo func(t *testing.T, dir string) string
	}{
		{"amend", func(t *testing.T, dir string) string {
			gitCmd(t, dir, "commit", "--amend", "-q", "-m", "reworded")
			return head(t, dir)
		}},
		{"cherry-pick", func(t *testing.T, dir string) string {
			sha := head(t, dir)
			gitCmd(t, dir, "checkout", "-q", "-b", "other", "HEAD~1")
			writeFile(t, filepath.Join(dir, "bar.go"), "package bar\n")
			gitCmd(t, dir, "add", "bar.go")
			gitCmd(t, dir, "commit", "-q", "-m", "bar")
			gitCmd(t, dir, "cherry-pick", sha)
			return head(t, dir)
		}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			r := newRig(t, nil)
			dir := newRepo(t, "false")
			w := r.session(sidA, dir)
			first := editFlow(t, r, w, dir, "package foo\n\nfunc A() {}\n", "Bash")
			if cv, ok := r.c.Commit(first); !ok || cv.Link == nil || cv.Link.Files[0].Link != gitlink.LinkExact {
				t.Fatalf("first commit = %+v", cv)
			}
			again := tt.redo(t, dir)
			if again == first {
				t.Fatal("redo made no new commit")
			}
			r.repos()
			cv, ok := r.c.Commit(again)
			if !ok || cv.Link == nil {
				t.Fatalf("rewritten commit not linked: %+v", cv)
			}
			f := cv.Link.Files[0]
			if f.Path != "foo.go" || f.Link != gitlink.LinkExact || len(f.Edits) != 1 || len(cv.Link.Sessions) != 1 {
				t.Fatalf("rewritten link = %+v", cv.Link)
			}
		})
	}
}

func TestLinkedEditsNeverMakePartial(t *testing.T) {
	// A later commit of a different foo.go content must not pick up the
	// earlier, already committed edit as partial.
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	editFlow(t, r, w, dir, "package foo\n\nfunc A() {}\n", "Bash")
	writeFile(t, filepath.Join(dir, "foo.go"), "package foo\n\nfunc B() {}\n")
	gitCmd(t, dir, "commit", "-q", "-am", "by hand")
	sha := head(t, dir)
	r.repos()
	cv, ok := r.c.Commit(sha)
	if !ok || cv.Link == nil || cv.Link.Files[0].Link != gitlink.LinkUnattributed {
		t.Fatalf("hand commit = %+v", cv.Link)
	}
}

func TestCommitBeforeFirstRepoPass(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	now := time.Now()
	w.write(flat(
		[]obj{w.prompt("p1", "add foo", now)},
		w.writeTool("w1", filepath.Join(dir, "foo.go"), "package foo\n", nil, now.Add(time.Second)),
	)...)
	r.tail() // the root is first seen here; no repo pass yet
	gitCmd(t, dir, "add", "foo.go")
	gitCmd(t, dir, "commit", "-q", "-m", "add foo")
	sha := head(t, dir)
	r.repos()
	r.repos()
	cv, ok := r.c.Commit(sha)
	if !ok || cv.Link == nil || cv.Link.Files[0].Link != gitlink.LinkExact {
		t.Fatalf("commit after first sight = %+v, %v", cv, ok)
	}
}

// flakyGit fails the next left runs of one git subcommand.
type flakyGit struct {
	mu   sync.Mutex
	sub  string
	left int
}

func (g *flakyGit) Run(ctx context.Context, dir string, args ...string) (string, error) {
	g.mu.Lock()
	fail := g.left > 0 && slices.Contains(args, g.sub)
	if fail {
		g.left--
	}
	g.mu.Unlock()
	if fail {
		return "", errors.New("transient")
	}
	return gitlink.Exec.Run(ctx, dir, args...)
}

func TestTransientGitFailureRetries(t *testing.T) {
	for _, sub := range []string{"rev-list", "show"} {
		t.Run(sub, func(t *testing.T) {
			g := &flakyGit{sub: sub}
			r := newRig(t, func(o *Options) { o.Git = g })
			dir := newRepo(t, "false")
			r.roster.set(liveCard(sidA, dir, "busy"))
			r.repos()
			writeFile(t, filepath.Join(dir, "foo.go"), "package foo\n")
			gitCmd(t, dir, "add", "foo.go")
			gitCmd(t, dir, "commit", "-q", "-m", "add foo")
			sha := head(t, dir)
			g.mu.Lock()
			g.left = 2
			g.mu.Unlock()
			r.repos()
			r.repos()
			r.repos()
			if _, ok := r.c.Commit(sha); !ok {
				t.Fatalf("commit lost after a transient %s failure", sub)
			}
		})
	}
}

func TestStraddlingSecretNotStored(t *testing.T) {
	const exact = "irori-lockfile-token-0123456789abcdef"
	r := newRig(t, func(o *Options) { o.Scrub.SetExact(exact) })
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	now := time.Now()
	prompt := strings.Repeat("a", 1985) + " " + exact + " tail"
	cmd := "curl -s " + strings.Repeat("x", 140) + " -H " + exact
	w.write(flat(
		[]obj{w.prompt("p1", prompt, now)},
		w.shell("b1", "Bash", cmd, "", nil, now),
		[]obj{w.reply("ok", now), w.done(now)},
	)...)
	r.tail()
	tr := decode[TurnRow](t, r.rows(KindTurn)[0])
	if strings.Contains(tr.Prompt, exact[:13]) {
		t.Fatalf("prompt holds a token prefix: %q", tr.Prompt[len(tr.Prompt)-40:])
	}
	if len(tr.Tools) != 1 || strings.Contains(tr.Tools[0].Target, exact[:13]) {
		t.Fatalf("target holds a token prefix: %+v", tr.Tools)
	}
}

func TestCommitSubjectScrubbed(t *testing.T) {
	const exact = "irori-lockfile-token-0123456789abcdef"
	r := newRig(t, func(o *Options) { o.Scrub.SetExact(exact) })
	dir := newRepo(t, "false")
	r.roster.set(liveCard(sidA, dir, "busy"))
	r.repos()
	writeFile(t, filepath.Join(dir, "foo.go"), "package foo\n")
	gitCmd(t, dir, "add", "foo.go")
	gitCmd(t, dir, "commit", "-q", "-m", "use "+exact)
	r.repos()
	rows := r.rows(KindCommitSeen)
	if len(rows) == 0 {
		t.Fatal("no commit.seen")
	}
	for _, raw := range rows {
		if strings.Contains(string(raw), exact) {
			t.Fatalf("commit.seen holds a secret: %s", raw)
		}
	}
}

func TestStuckReflogLineSkipped(t *testing.T) {
	g := &flakyGit{sub: "show"}
	r := newRig(t, func(o *Options) { o.Git = g })
	dir := newRepo(t, "false")
	r.roster.set(liveCard(sidA, dir, "busy"))
	r.repos()
	gitCmd(t, dir, "commit", "-q", "--allow-empty", "-m", "bad")
	bad := head(t, dir)
	g.mu.Lock()
	g.left = 1 << 20
	g.mu.Unlock()
	for range maxReflogTries {
		r.repos()
	}
	g.mu.Lock()
	g.left = 0
	g.mu.Unlock()
	gitCmd(t, dir, "commit", "-q", "--allow-empty", "-m", "good")
	good := head(t, dir)
	r.repos()
	if _, ok := r.c.Commit(bad); ok {
		t.Fatal("stuck commit imported")
	}
	if _, ok := r.c.Commit(good); !ok {
		t.Fatal("a stuck line blocked the commits after it")
	}
}
