package main

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/scrub"
	"github.com/StephenSHorton/irori/internal/store"
)

type staticRoster struct {
	mu    sync.Mutex
	cards []presence.AgentCard
}

func (r *staticRoster) Agents() []presence.AgentCard {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]presence.AgentCard(nil), r.cards...)
}

// newTestCapture is a CaptureService over a real capture service with an
// empty ~/.claude and nothing running: rows come only from the calls made.
func newTestCapture(t *testing.T, roster *staticRoster, mods map[string]bool) *CaptureService {
	t.Helper()
	ctx := context.Background()
	data := t.TempDir()
	st, err := store.Open(ctx, filepath.Join(data, "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	c, err := capture.New(ctx, st, roster, capture.Options{
		ClaudeDir: t.TempDir(), DataDir: data, DeviceID: "dev1", RepoKey: repo.New().Key, Scrub: scrub.New(),
	})
	if err != nil {
		t.Fatal(err)
	}
	return &CaptureService{
		cap:    c,
		agents: roster.Agents,
		user:   func() inbox.Sender { return inbox.Sender{Key: "user:dev1", Name: "you", Kind: "user"} },
		reach:  func(agent string) (bool, bool) { return mods[agent], false },
	}
}

// The GUI records decisions as the person, with trust "user", and finds the
// worktree for a repo-relative path from a live agent in that repo.
func TestCaptureRecordDecisionAsPerson(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not installed")
	}
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"init", "-q"}, {"config", "user.email", "t@example.invalid"}, {"config", "user.name", "t"}} {
		if out, err := exec.Command("git", append([]string{"-C", root}, args...)...).CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v %s", args, err, out)
		}
	}
	if err := os.WriteFile(filepath.Join(root, "main.go"), []byte("package main\n\nfunc main() {}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"add", "main.go"}, {"commit", "-q", "-m", "first"}} {
		if out, err := exec.Command("git", append([]string{"-C", root}, args...)...).CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v %s", args, err, out)
		}
	}
	rk := repo.New().Key(root)
	if rk == "" {
		t.Fatal("no repo key for the temp repo")
	}
	roster := &staticRoster{cards: []presence.AgentCard{{Key: "dev1/s1", Agent: "claude", Cwd: root, RepoKey: rk}}}
	s := newTestCapture(t, roster, nil)

	d, err := s.RecordDecision("  Keep main tiny  ", "Sample reason.", "main.go", "3", rk, []string{" ", ""}, "", "turn-1")
	if err != nil {
		t.Fatal(err)
	}
	if d.By.Kind != "user" || d.By.Key != "user:dev1" || d.Trust != "user" {
		t.Fatalf("by = %+v trust = %q, want the person with trust user", d.By, d.Trust)
	}
	if !strings.HasPrefix(d.ID, "d-") || d.Statement != "Keep main tiny" {
		t.Fatalf("decision = %+v", d)
	}
	if d.Anchor == nil || d.Anchor.Path != "main.go" || d.Anchor.RepoKey != rk || d.Anchor.Start != 3 {
		t.Fatalf("anchor = %+v", d.Anchor)
	}
	if d.Source == nil || d.Source.TurnID != "turn-1" || len(d.Supersedes) != 0 {
		t.Fatalf("source = %+v supersedes = %v", d.Source, d.Supersedes)
	}

	list := s.Decisions(rk, "", "", false)
	if len(list) != 1 || list[0].ID != d.ID || list[0].Status == nil {
		t.Fatalf("decisions = %+v", list)
	}
	if list[0].Status.State != "ok" {
		t.Fatalf("anchor status = %q, want ok", list[0].Status.State)
	}

	// The person may supersede it; the old one leaves the default list.
	d2, err := s.RecordDecision("Main may grow", "", "", "", "", []string{d.ID}, "", "")
	if err != nil {
		t.Fatal(err)
	}
	if got := s.Decisions("", "", "", false); len(got) != 1 || got[0].ID != d2.ID {
		t.Fatalf("default list = %+v, want only the superseding decision", got)
	}
	if got := s.Decisions("", "", "", true); len(got) != 2 {
		t.Fatalf("all = %d decisions, want 2", len(got))
	}

	// A relative path with no repo to resolve it in says what to do.
	if _, err := s.RecordDecision("x", "", "main.go", "", "", nil, "", ""); !errors.Is(err, capture.ErrNoRepo) {
		t.Fatalf("err = %v, want ErrNoRepo", err)
	}
	if _, err := s.Why("main.go", 0, ""); !errors.Is(err, capture.ErrNoRepo) {
		t.Fatalf("why err = %v, want ErrNoRepo", err)
	}
	w, err := s.Why("main.go", 3, rk)
	if err != nil {
		t.Fatal(err)
	}
	if w.Path != "main.go" || w.Edits == nil || len(w.Decisions) != 0 {
		// d is superseded, so only d2 could show, and it is unanchored.
		t.Fatalf("why = %+v", w)
	}
}

// A live agent working in a subdirectory still resolves the person's
// repo-relative paths against the worktree root, not its cwd: the Why box
// and the decision form take paths in the repo, like edit and commit rows.
func TestCaptureRepoPathsFromAgentInSubdir(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not installed")
	}
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	sub := filepath.Join(root, "sub")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	for _, f := range []string{filepath.Join(root, "README.md"), filepath.Join(sub, "README.md")} {
		if err := os.WriteFile(f, []byte("# sample\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	for _, args := range [][]string{
		{"init", "-q"}, {"config", "user.email", "t@example.invalid"}, {"config", "user.name", "t"},
		{"add", "."}, {"commit", "-q", "-m", "first"},
	} {
		if out, err := exec.Command("git", append([]string{"-C", root}, args...)...).CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v %s", args, err, out)
		}
	}
	rk := repo.New().Key(root)
	if rk == "" {
		t.Fatal("no repo key for the temp repo")
	}
	roster := &staticRoster{cards: []presence.AgentCard{{Key: "dev1/s1", Agent: "claude", Cwd: sub, RepoKey: rk}}}
	s := newTestCapture(t, roster, nil)

	w, err := s.Why("README.md", 1, rk)
	if err != nil {
		t.Fatal(err)
	}
	if w.Path != "README.md" {
		t.Fatalf("why path = %q, want README.md at the repo root", w.Path)
	}
	d, err := s.RecordDecision("Keep the readme short", "", "README.md", "1", rk, nil, "", "")
	if err != nil {
		t.Fatal(err)
	}
	if d.Anchor == nil || d.Anchor.Path != "README.md" {
		t.Fatalf("anchor = %+v, want README.md at the repo root", d.Anchor)
	}
	// A path given from the subdirectory's own view still works by spelling
	// it out from the root.
	if w, err := s.Why("sub/README.md", 0, rk); err != nil || w.Path != "sub/README.md" {
		t.Fatalf("why sub = %+v, %v", w, err)
	}
}

func TestCaptureTimelineAndLookups(t *testing.T) {
	roster := &staticRoster{cards: []presence.AgentCard{
		{Key: "dev1/a", Agent: "claude"},
		{Key: "dev1/b", Agent: "claude"},
		{Key: "dev1/member", Agent: "grok"},
	}}
	s := newTestCapture(t, roster, map[string]bool{"dev1/a": true})

	if _, err := s.Timeline("dev1/a", "yesterday", 0, false); err == nil {
		t.Fatal("a bad before was accepted")
	}
	tl, err := s.Timeline("dev1/a", time.Now().UTC().Format(time.RFC3339Nano), 0, false)
	if err != nil {
		t.Fatal(err)
	}
	if tl.Key != "dev1/a" || tl.Entries == nil || tl.More {
		t.Fatalf("timeline = %+v", tl)
	}
	if s.Commit("abcdef1") != nil {
		t.Fatal("found a commit in an empty log")
	}
	if c := s.Commits("", 0); c == nil || len(c) != 0 {
		t.Fatalf("commits = %v, want empty non-nil", c)
	}
	if _, err := s.Patch("not-a-blob"); err == nil {
		t.Fatal("a bad blob id was accepted")
	}

	want := map[string]string{"dev1/a": FidelityMod, "dev1/b": FidelityPassive, "dev1/member": FidelityMCP}
	got := s.Fidelity()
	for k, v := range want {
		if got[k] != v {
			t.Fatalf("fidelity[%s] = %q, want %q (all: %v)", k, got[k], v, got)
		}
	}
}

// A burst of capture changes becomes one capture:changed with each kind,
// key and repo once.
func TestCapturePumpCoalesces(t *testing.T) {
	s := &CaptureService{}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	changes := make(chan capture.Change, 8)
	got := make(chan CaptureChange, 4)
	changes <- capture.Change{Kind: capture.ChangedOpenTurn, Key: "dev1/a"}
	changes <- capture.Change{Kind: capture.ChangedOpenTurn, Key: "dev1/a"}
	changes <- capture.Change{Kind: capture.ChangedCommit, RepoKey: "root:abc"}
	changes <- capture.Change{Kind: capture.ChangedEdit, Key: "dev1/b", RepoKey: "root:abc"}
	go s.run(ctx, changes, func(c CaptureChange) { got <- c })

	select {
	case c := <-got:
		if strings.Join(c.Kinds, ",") != "open,commit,edit" || strings.Join(c.Keys, ",") != "dev1/a,dev1/b" ||
			strings.Join(c.RepoKeys, ",") != "root:abc" {
			t.Fatalf("change = %+v", c)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no capture:changed")
	}
	select {
	case c := <-got:
		t.Fatalf("second event %+v for one burst", c)
	case <-time.After(2 * captureEmitEvery):
	}
}
