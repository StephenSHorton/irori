package tools

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/scrub"
)

// Everything here is synthetic: capture rows are seeded into the log before
// the capture service replays it, and the repo is a fresh temp repo.

func canonDir(t *testing.T, dir string) string {
	t.Helper()
	d, err := filepath.EvalSymlinks(dir)
	if err != nil {
		t.Fatal(err)
	}
	return d
}

func gitRun(t *testing.T, dir string, args ...string) string {
	t.Helper()
	full := append([]string{"-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false",
		"-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"}, args...)
	cmd := exec.Command("git", full...)
	cmd.Dir = dir
	var env []string
	for _, kv := range os.Environ() {
		if !strings.HasPrefix(strings.ToUpper(kv), "GIT_") {
			env = append(env, kv)
		}
	}
	cmd.Env = env
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
	return strings.TrimSpace(string(out))
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

const fooGo = "package foo\n\nfunc A() {}\n\nfunc B() {}\n"

// capRig is a harness with capture on over a temp repo that alpha works in.
type capRig struct {
	*harness
	dir     string // the repo, alpha's cwd
	repoKey string
	sha     string // the commit that added foo.go
	editID  string
}

// withCapture makes a temp repo with foo.go committed, points alpha's card
// at it, seeds a turn, its edit and the linked commit (plus whatever seed
// adds), then starts capture over the harness's store.
func withCapture(t *testing.T, seed func(r *capRig)) *capRig {
	t.Helper()
	h := newHarness(t)
	dir := canonDir(t, t.TempDir())
	gitRun(t, dir, "init", "-q")
	writeFile(t, filepath.Join(dir, "foo.go"), fooGo)
	gitRun(t, dir, "add", "foo.go")
	gitRun(t, dir, "commit", "-q", "-m", "add foo")
	r := &capRig{harness: h, dir: dir, repoKey: repo.New().Key(dir), sha: gitRun(t, dir, "rev-parse", "HEAD"), editID: "e-foo1"}
	a := card(keyA, "alpha", r.repoKey, inbox.BypassMode)
	a.Cwd = dir
	h.b.put(a)

	t0 := h.clk.now().Add(-2 * time.Hour)
	r.turn(keyA, "p1", "add foo please", "added foo.go", t0, "foo.go")
	r.append(capture.KindEdit, capture.EditRow{ID: r.editID, Key: keyA, TurnID: "p1", ToolUseID: "w1", Tool: "Write",
		RepoKey: r.repoKey, Root: dir, Path: "foo.go", Create: true, Adds: 5, Fidelity: transcript.Exact, At: t0.Add(time.Second)})
	r.append(capture.KindCommitSeen, capture.CommitRow{RepoKey: r.repoKey, Root: dir, Op: "commit", Branch: "main",
		Commit: gitlink.Commit{SHA: r.sha, At: t0.Add(time.Minute), Author: "Test", Subject: "add foo",
			Files: []gitlink.FileChange{{Path: "foo.go"}}}})
	r.append(capture.KindCommitLink, capture.CommitLink{RepoKey: r.repoKey, SHA: r.sha,
		Committer: &capture.Committer{Key: keyA, TurnID: "p1"},
		Files:     []gitlink.FileLink{{Path: "foo.go", Link: gitlink.LinkExact, Edits: []string{r.editID}}},
		Sessions:  []string{keyA}, Turns: []string{"p1"}, At: t0.Add(time.Minute)})
	if seed != nil {
		seed(r)
	}
	cp, err := capture.New(bg, h.b.st, h.b.roster, capture.Options{
		ClaudeDir: t.TempDir(), DataDir: t.TempDir(), DeviceID: "dev1", Now: h.clk.now,
		RepoKey: repo.New().Key, Scrub: scrub.New(),
	})
	if err != nil {
		t.Fatal(err)
	}
	cp.AddSource(h.b.co)
	cp.AddSource(h.b.ib)
	h.b.cp = cp
	return r
}

func (r *capRig) append(kind string, payload any) {
	r.t.Helper()
	if _, err := r.b.st.Append(bg, "irori", kind, payload); err != nil {
		r.t.Fatal(err)
	}
}

func (r *capRig) turn(key, id, prompt, reply string, at time.Time, edits ...string) {
	r.t.Helper()
	_, sess, _ := strings.Cut(key, "/")
	r.append(capture.KindTurn, capture.TurnRow{ID: "t-" + key + id, Key: key, RepoKey: r.repoKey, Root: r.dir, TurnID: id,
		Turn: transcript.Turn{SessionID: sess, Origin: "human", Prompt: prompt, Reply: reply, Cwd: r.dir,
			StartedAt: at, EndedAt: at.Add(30 * time.Second), EndReason: "turn_duration", EditPaths: edits,
			Tools: []transcript.ToolCall{{ID: id + "a", Name: "Edit"}, {ID: id + "b", Name: "Edit"}, {ID: id + "c", Name: "Bash"}}}})
}

// grokNoCwd is the Grok card as a real member has it: no working directory.
func (r *capRig) grokNoCwd() {
	g := card(keyG, "grokker", "", "")
	g.Agent, g.Cwd = "grok", ""
	r.b.put(g)
}

// moreBefore pulls the before= value out of a (more: ...) hint.
func moreBefore(t *testing.T, out string) string {
	t.Helper()
	_, rest, ok := strings.Cut(out, "(more: before=")
	if !ok {
		t.Fatalf("no more hint in %q", out)
	}
	v, _, _ := strings.Cut(rest, ";")
	return v
}

func TestCaptureOff(t *testing.T) {
	h := newHarness(t)
	for name, args := range map[string]any{
		"timeline": nil,
		"why":      map[string]any{"path": "foo.go"},
	} {
		h.fails(callA, name, args, "capture is off")
	}
}

func TestTimeline(t *testing.T) {
	r := withCapture(t, func(r *capRig) {
		r.append(capture.KindPR, ledger.Entry{Key: ledger.Key("acme/app", 7), Repo: "acme/app", Number: 7,
			URL: "https://github.com/acme/app/pull/7", Session: keyA, Role: ledger.Author, Source: "pr-link",
			At: r.clk.now().Add(-90 * time.Minute)})
	})
	r.sendTo(callB, "alpha", "hello alpha from beta")
	r.clk.advance(time.Minute)
	r.sendTo(callA, "beta", "thanks beta")

	out := r.ok(callA, "timeline", nil)
	has(t, out, "Timeline of alpha ("+keyA+"), you,", "untrusted data",
		"turn p1 (origin human), tools: Edit 2, Bash 1", `prompt: "add foo please"`, `reply: "added foo.go"`,
		"edited: ", "foo.go", "commit "+r.sha[:7]+` "add foo" on main, committed by alpha`, "files: foo.go [exact]",
		"PR acme/app#7 author (pr-link) https://github.com/acme/app/pull/7",
		"message from beta ("+keyB+")", `text: "hello alpha from beta"`,
		"message sent to beta ("+keyB+")", `text: "thanks beta"`)
	hasNot(t, out, "(more:")
	// Newest first: the reply message, then the first, then the PR, commit, turn.
	order := []string{"thanks beta", "hello alpha", "PR acme/app#7", "commit " + r.sha[:7], "turn p1"}
	last := -1
	for _, w := range order {
		i := strings.Index(out, w)
		if i < last {
			t.Fatalf("%q out of order:\n%s", w, out)
		}
		last = i
	}

	// Another agent's timeline: by name, key and key prefix; message text
	// only for the messages' parties.
	for _, who := range []string{"alpha", keyA, "aaaaaaaa-1111", "dev1/aaaaaaaa"} {
		out := r.ok(callC, "timeline", map[string]any{"agent": who})
		has(t, out, "Timeline of alpha ("+keyA+")", "turn p1", "[left out: only its sender and recipient see it]")
		hasNot(t, out, "hello alpha", "thanks beta", "), you,")
	}
	has(t, r.ok(callB, "timeline", map[string]any{"agent": "alpha"}), "hello alpha from beta", "thanks beta")
	// beta has no turns but its messages show.
	has(t, r.ok(callB, "timeline", nil), "Timeline of beta", "message sent to alpha", "hello alpha")
}

func TestTimelineHeldText(t *testing.T) {
	r := withCapture(t, nil)
	const secret = "grok held secret"
	out := r.ok(grokCaller(), "send", map[string]any{"to": "alpha", "text": secret})
	has(t, out, "held")
	for _, c := range []Caller{callA, callB} {
		hasNot(t, r.ok(c, "timeline", map[string]any{"agent": "alpha"}), secret, "grokker")
	}
	// Its sender sees its own message.
	has(t, r.ok(grokCaller(), "timeline", nil), "message sent to alpha", secret)
}

func TestTimelinePaging(t *testing.T) {
	r := withCapture(t, func(r *capRig) {
		base := r.clk.now().Add(-time.Hour)
		for i := 2; i <= 4; i++ {
			r.turn(keyA, fmt.Sprintf("p%d", i), fmt.Sprintf("prompt %d", i), "", base.Add(time.Duration(i)*time.Minute))
		}
	})
	out := r.ok(callA, "timeline", map[string]any{"limit": 2})
	has(t, out, "turn p4", "turn p3", "(more: before=")
	hasNot(t, out, "turn p2", "turn p1")
	before := moreBefore(t, out)
	out = r.ok(callA, "timeline", map[string]any{"limit": 2, "before": before})
	has(t, out, "turn p2", "turn p1", "commit "+r.sha[:7])
	hasNot(t, out, "turn p3", "turn p4", "(more:")
}

func TestTimelineCapped(t *testing.T) {
	r := withCapture(t, func(r *capRig) {
		base := r.clk.now().Add(-time.Hour)
		long := strings.Repeat("a long prompt line ", 40)
		for i := range 50 {
			r.turn(keyA, fmt.Sprintf("q%02d", i), long, long, base.Add(time.Duration(i)*time.Second))
		}
	})
	out := r.ok(callA, "timeline", map[string]any{"limit": 50})
	if len(out) > MaxResult {
		t.Fatalf("%d bytes", len(out))
	}
	has(t, out, "turn q49", "(more: before=")
	// Paging by the hints reaches every turn once, each page under the cap.
	seen := map[string]int{}
	for page := 0; ; page++ {
		if page > 20 {
			t.Fatal("paging doesn't end")
		}
		hasNot(t, out, "[truncated")
		for _, l := range strings.Split(out, "\n") {
			if _, rest, ok := strings.Cut(l, " turn "); ok {
				id, _, _ := strings.Cut(rest, " ")
				seen[id]++
			}
		}
		if !strings.Contains(out, "(more: before=") {
			break
		}
		out = r.ok(callA, "timeline", map[string]any{"limit": 50, "before": moreBefore(t, out)})
	}
	if len(seen) != 51 {
		t.Fatalf("saw %d turns: %v", len(seen), seen)
	}
	for id, n := range seen {
		if n != 1 {
			t.Errorf("turn %s shown %d times", id, n)
		}
	}
}

func TestTimelineErrors(t *testing.T) {
	r := withCapture(t, nil)
	r.b.put(card("dev1/aaaaaaaa-9999-4999-8999-999999999999", "alpha2", "", ""))
	r.fails(callA, "timeline", map[string]any{"agent": "nobody-here"}, "no agent called", "agents tool")
	r.fails(callA, "timeline", map[string]any{"agent": "aaaaaaaa"}, "matches 2 agents", "use a key")
	r.fails(callA, "timeline", map[string]any{"before": "yesterday"}, "RFC 3339")
	r.fails(callA, "timeline", map[string]any{"limit": "ten"}, "limit should be a whole number")
	r.fails(callA, "timeline", map[string]any{"who": "alpha"}, "unknown argument")
	r.fails(Caller{}, "timeline", nil, "join first")
	// A gone agent's key still works.
	has(t, r.ok(callA, "timeline", map[string]any{"agent": "dev1/zzzzzzzz-gone"}), "nothing recorded yet")
}

func TestWhy(t *testing.T) {
	r := withCapture(t, nil)
	// A committed line: blame -> commit -> session and turn.
	out := r.ok(callA, "why", map[string]any{"path": "foo.go", "line": 3})
	has(t, out, "Why foo.go:3 in "+r.repoKey, "Last changed by commit "+r.sha[:7]+` "add foo", by Test`,
		"Sessions behind it: alpha ("+keyA+")", "Committed by alpha ("+keyA+") in turn p1",
		"Turns:", `prompt: "add foo please"`, "Edits, newest first:", "Write +5/-0 (exact) by alpha")
	hasNot(t, out, "ecision")
	// Absolute path, from Grok with no cwd.
	r.grokNoCwd()
	has(t, r.ok(grokCaller(), "why", map[string]any{"path": filepath.Join(r.dir, "foo.go")}), "Why foo.go in", "No line given")
	r.fails(grokCaller(), "why", map[string]any{"path": "foo.go"}, "no working directory for you", "repo")
	has(t, r.ok(grokCaller(), "why", map[string]any{"path": "foo.go", "repo": r.repoKey}), "Why foo.go in "+r.repoKey)

	// An uncommitted line: the edits since the last commit.
	writeFile(t, filepath.Join(r.dir, "foo.go"), fooGo+"\nfunc C() {}\n")
	out = r.ok(callA, "why", map[string]any{"path": "foo.go", "line": 7})
	has(t, out, "isn't committed yet")

	// Decision records left the agent tools (decision 27): one already in
	// the log, anchored right there, is not shown.
	if _, err := r.b.cp.RecordDecision(bg, capture.DecisionInput{Statement: "keep A tiny", Path: "foo.go", Lines: "3", Cwd: r.dir}); err != nil {
		t.Fatal(err)
	}
	out = r.ok(callB, "why", map[string]any{"path": filepath.Join(r.dir, "foo.go"), "line": 3})
	has(t, out, "Last changed by commit "+r.sha[:7])
	hasNot(t, out, "ecision", "keep A tiny")

	r.fails(callA, "why", nil, "give path")
	r.fails(callA, "why", map[string]any{"path": "foo.go", "line": -1}, "1-based")
	outside := canonDir(t, t.TempDir())
	writeFile(t, filepath.Join(outside, "x.txt"), "x\n")
	r.fails(callA, "why", map[string]any{"path": filepath.Join(outside, "x.txt")}, "not inside a known git repo", "absolute path")
}

// why stays under the result cap, without the generic clip, when the turns
// and edits behind a line are big enough to fill it.
func TestWhyCapsBigTurns(t *testing.T) {
	big := strings.Repeat("漢", 300) // 3 bytes a rune, past the excerpt cap
	r := withCapture(t, func(r *capRig) {
		t0 := r.clk.now().Add(-time.Hour)
		edits := []string{r.editID}
		turns := []string{"p1"}
		for i := 2; i <= 10; i++ {
			id, eid := fmt.Sprintf("p%d", i), fmt.Sprintf("e-foo%d", i)
			at := t0.Add(time.Duration(i) * time.Minute)
			r.turn(keyA, id, big, big, at, "foo.go")
			r.append(capture.KindEdit, capture.EditRow{ID: eid, Key: keyA, TurnID: id, ToolUseID: "w" + id, Tool: "Edit",
				RepoKey: r.repoKey, Root: r.dir, Path: "foo.go", Adds: 1, Fidelity: transcript.Exact, At: at.Add(time.Second)})
			edits = append(edits, eid)
			turns = append(turns, id)
		}
		r.append(capture.KindCommitLink, capture.CommitLink{RepoKey: r.repoKey, SHA: r.sha,
			Committer: &capture.Committer{Key: keyA, TurnID: "p1"},
			Files:     []gitlink.FileLink{{Path: "foo.go", Link: gitlink.LinkExact, Edits: edits}},
			Sessions:  []string{keyA}, Turns: turns, At: t0.Add(11 * time.Minute)})
	})
	out := r.ok(callA, "why", map[string]any{"path": "foo.go", "line": 3})
	has(t, out, "Turns:", "not shown")
	hasNot(t, out, "[truncated:")
	if len(out) > MaxResult {
		t.Fatalf("why is %d bytes, over %d", len(out), MaxResult)
	}
}
