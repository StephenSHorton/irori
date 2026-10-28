package api

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/store"
)

// Everything here is synthetic: a fresh temp repo and hand-built capture
// rows appended to a temp store, which capture.New replays (no tailing).

const capKey = "dev1/s1"

type noRoster struct{}

func (noRoster) Agents() []presence.AgentCard { return nil }

// capFixture is a capture service over seeded rows.
type capFixture struct {
	c       *capture.Capture
	port    int
	client  *Client
	root    string // the temp repo
	repoKey string
	head    string
	patch   string // blob id of the seeded patch
	patchTx string
	t0      time.Time
	decID   string
}

func canonDir(t *testing.T, dir string) string {
	t.Helper()
	d, err := filepath.EvalSymlinks(dir)
	if err != nil {
		t.Fatal(err)
	}
	return d
}

func git(t *testing.T, dir string, args ...string) string {
	t.Helper()
	full := append([]string{"-c", "user.name=Test", "-c", "user.email=test@example.com",
		"-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"}, args...)
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

func newCapFixture(t *testing.T) *capFixture {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not on PATH")
	}
	ctx := context.Background()
	f := &capFixture{root: canonDir(t, t.TempDir()), t0: time.Now().Add(-time.Hour).UTC().Truncate(time.Second)}
	git(t, f.root, "init", "-q")
	git(t, f.root, "config", "core.autocrlf", "false")
	if err := os.WriteFile(filepath.Join(f.root, "a.txt"), []byte("one\ntwo\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	git(t, f.root, "add", "a.txt")
	git(t, f.root, "commit", "-q", "-m", "add a")
	f.head = git(t, f.root, "rev-parse", "HEAD")
	f.repoKey = repo.New().Key(f.root)

	data := canonDir(t, t.TempDir())
	bs, err := blob.Open(filepath.Join(data, "blobs"))
	if err != nil {
		t.Fatal(err)
	}
	f.patchTx = "--- a/a.txt\n+++ b/a.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n"
	if f.patch, err = bs.Put([]byte(f.patchTx)); err != nil {
		t.Fatal(err)
	}

	st, err := store.Open(ctx, filepath.Join(data, "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	at := func(d time.Duration) time.Time { return f.t0.Add(d) }
	turn := func(id, turnID, agent, prompt string, start time.Duration) capture.TurnRow {
		return capture.TurnRow{ID: id, Key: capKey, RepoKey: f.repoKey, Root: f.root, TurnID: turnID,
			Turn: transcript.Turn{SessionID: "s1", AgentID: agent, Origin: "human", Prompt: prompt,
				StartedAt: at(start), EndedAt: at(start + time.Minute), EndReason: "turn_duration"}}
	}
	rows := []struct {
		kind    string
		payload any
	}{
		{capture.KindRepoSeen, map[string]any{"repoKey": f.repoKey, "root": f.root, "at": at(0)}},
		{capture.KindTurn, turn("t-1", "p1", "", "add a file", 0)},
		{capture.KindTurn, turn("t-2", "p2", "a1", "subagent work", 5*time.Minute)},
		{capture.KindTurn, turn("t-3", "p3", "", "tidy up", 10*time.Minute)},
		{capture.KindEdit, capture.EditRow{ID: "e-1", Key: capKey, TurnID: "p1", ToolUseID: "tu1", Tool: "Write",
			RepoKey: f.repoKey, Root: f.root, Path: "a.txt", Create: true, Patch: f.patch, Adds: 2,
			Fidelity: transcript.Exact, At: at(30 * time.Second)}},
		{capture.KindCommitSeen, capture.CommitRow{RepoKey: f.repoKey, Root: f.root, Op: "commit", Branch: "main",
			Commit: gitlink.Commit{SHA: f.head, At: at(2 * time.Minute), Author: "Test", Subject: "add a",
				Files: []gitlink.FileChange{{Path: "a.txt", Status: "A"}}}}},
		{capture.KindCommitLink, capture.CommitLink{RepoKey: f.repoKey, SHA: f.head,
			Committer: &capture.Committer{Key: capKey, TurnID: "p1"},
			Files:     []gitlink.FileLink{{Path: "a.txt", Link: "exact", Edits: []string{"e-1"}}},
			Sessions:  []string{capKey}, Turns: []string{"p1"}, At: at(2 * time.Minute)}},
		{capture.KindPR, ledger.Entry{Key: ledger.Key("owner/name", 7), Repo: "owner/name", Number: 7,
			URL: "https://github.com/owner/name/pull/7", Session: capKey, Role: ledger.Author,
			Source: "pr-link", At: at(3 * time.Minute)}},
	}
	for _, r := range rows {
		if _, err := st.Append(ctx, "irori", r.kind, r.payload); err != nil {
			t.Fatal(err)
		}
	}
	f.c, err = capture.New(ctx, st, noRoster{}, capture.Options{
		ClaudeDir: canonDir(t, t.TempDir()), DataDir: data, DeviceID: "dev1", RepoKey: repo.New().Key,
	})
	if err != nil {
		t.Fatal(err)
	}
	d, err := f.c.RecordDecision(ctx, capture.DecisionInput{
		By: decision.Party{Key: capKey, Kind: "claude"}, Statement: "keep a.txt two lines",
		Why: "the reader expects it", Path: "a.txt", Lines: "1", Cwd: f.root,
	})
	if err != nil {
		t.Fatal(err)
	}
	f.decID = d.ID
	f.port = startWith(t, &fakeBackend{cap: f.c})
	f.client = NewClient(discovery.Lockfile{Port: f.port, Token: testToken})
	return f
}

// getJSON GETs path with the token and decodes a 200 body into out.
func getJSON(t *testing.T, port int, path string, out any) {
	t.Helper()
	resp := do(t, port, "GET", path, bearer(testToken))
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("GET %s: status %d: %s", path, resp.StatusCode, body)
	}
	if ct := resp.Header.Get("Content-Type"); ct != "application/json" {
		t.Fatalf("GET %s: content-type %q", path, ct)
	}
	if err := json.Unmarshal(body, out); err != nil {
		t.Fatalf("GET %s: %v: %s", path, err, body)
	}
}

// wantStatus GETs path with the token and checks a JSON error status.
func wantStatus(t *testing.T, port int, path string, code int) {
	t.Helper()
	// As the person: these check routes and parameters, not viewers.
	resp := do(t, port, "GET", path, func(r *http.Request) {
		bearer(testToken)(r)
		r.Header.Set(HeaderPerson, personOf(port))
	})
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != code {
		t.Fatalf("GET %s: status %d, want %d: %s", path, resp.StatusCode, code, body)
	}
	var eb errorBody
	if err := json.Unmarshal(body, &eb); err != nil || eb.Error == "" {
		t.Fatalf("GET %s: want a JSON error body, got %s", path, body)
	}
}

var v03Paths = []string{
	"/v1/timeline?agent=" + capKey, "/v1/edits", "/v1/commits", "/v1/commits/abcdef0",
	"/v1/blobs/" + strings.Repeat("a", 64), "/v1/decisions", "/v1/prs", "/v1/capture/status",
	"/v1/why?path=a.txt",
}

func TestV03RoutesWithoutCapture503(t *testing.T) {
	_, port := start(t)
	for _, p := range v03Paths {
		wantStatus(t, port, p, http.StatusServiceUnavailable)
	}
}

func TestV03RoutesKeepV01Rules(t *testing.T) {
	f := newCapFixture(t)
	for _, p := range v03Paths {
		if resp := do(t, f.port, "GET", p, nil); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("GET %s without token: %d", p, resp.StatusCode)
		}
		if resp := do(t, f.port, "GET", p, bearer("wrong")); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("GET %s with a wrong token: %d", p, resp.StatusCode)
		}
		if resp := do(t, f.port, "GET", p, func(r *http.Request) {
			bearer(testToken)(r)
			r.Header.Set("Origin", "http://evil.example")
		}); resp.StatusCode != http.StatusForbidden {
			t.Errorf("GET %s with Origin: %d", p, resp.StatusCode)
		}
		if resp := do(t, f.port, "GET", p, func(r *http.Request) {
			bearer(testToken)(r)
			r.Host = "evil.example"
		}); resp.StatusCode != http.StatusForbidden {
			t.Errorf("GET %s with a foreign Host: %d", p, resp.StatusCode)
		}
		if resp := do(t, f.port, "POST", p, bearer(testToken)); resp.StatusCode != http.StatusMethodNotAllowed {
			t.Errorf("POST %s: %d", p, resp.StatusCode)
		}
	}
}

func TestV03BadParams400(t *testing.T) {
	f := newCapFixture(t)
	for _, p := range []string{
		"/v1/timeline",
		"/v1/timeline?agent=",
		"/v1/timeline?agent=" + capKey + "&before=yesterday",
		"/v1/timeline?agent=" + capKey + "&limit=0",
		"/v1/timeline?agent=" + capKey + "&limit=x",
		"/v1/edits?limit=-1",
		"/v1/commits?limit=abc",
		"/v1/commits/abc12",                      // too short
		"/v1/commits/zzzzzzz",                    // not hex
		"/v1/commits/" + strings.Repeat("a", 65), // too long
		"/v1/blobs/nothex",
		"/v1/blobs/" + strings.Repeat("A", 64), // ids are lowercase
		"/v1/blobs/" + strings.Repeat("a", 63),
		"/v1/blobs/..%2f..%2firori.db",
		"/v1/decisions?limit=0",
		"/v1/why",
		"/v1/why?path=a.txt&line=0",
		"/v1/why?path=a.txt&line=one",
	} {
		wantStatus(t, f.port, p, http.StatusBadRequest)
	}
}

func TestV03Timeline(t *testing.T) {
	f := newCapFixture(t)
	var tl capture.Timeline
	getJSON(t, f.port, "/v1/timeline?agent="+url.QueryEscape(capKey), &tl)
	if tl.Key != capKey || tl.More {
		t.Fatalf("timeline %q more=%v", tl.Key, tl.More)
	}
	var kinds []string
	for _, e := range tl.Entries {
		kinds = append(kinds, string(e.Kind))
	}
	// Newest first: t-3 (10m), pr (3m), commit (2m), t-1 (0); the subagent turn is hidden.
	if got := strings.Join(kinds, ","); got != "turn,pr,commit,turn" {
		t.Fatalf("kinds %s", got)
	}
	last := tl.Entries[3].Turn
	if last == nil || last.TurnID != "p1" || len(last.Edits) != 1 || last.Edits[0].Patch != f.patch {
		t.Fatalf("oldest turn %+v", last)
	}
	if tl.Entries[2].Commit == nil || tl.Entries[2].Commit.SHA != f.head || tl.Entries[2].Commit.Link == nil {
		t.Fatalf("commit entry %+v", tl.Entries[2].Commit)
	}

	getJSON(t, f.port, "/v1/timeline?subagents=1&agent="+url.QueryEscape(capKey), &tl)
	turns := 0
	for _, e := range tl.Entries {
		if e.Kind == capture.EntryTurn {
			turns++
		}
	}
	if turns != 3 {
		t.Fatalf("with subagents: %d turns", turns)
	}

	// Paging: one turn per page.
	getJSON(t, f.port, "/v1/timeline?limit=1&agent="+url.QueryEscape(capKey), &tl)
	if !tl.More || len(tl.Entries) != 3 || tl.Entries[0].Turn.TurnID != "p3" {
		t.Fatalf("page 1: more=%v %d entries", tl.More, len(tl.Entries))
	}
	before := tl.Entries[len(tl.Entries)-1].At.Format(time.RFC3339Nano)
	getJSON(t, f.port, "/v1/timeline?limit=1&agent="+url.QueryEscape(capKey)+"&before="+url.QueryEscape(before), &tl)
	if tl.More || len(tl.Entries) != 1 || tl.Entries[0].Turn.TurnID != "p1" {
		t.Fatalf("page 2: more=%v %+v", tl.More, tl.Entries)
	}

	// An unknown agent is an empty page, never null.
	resp := do(t, f.port, "GET", "/v1/timeline?agent=dev1/nobody", bearer(testToken))
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != 200 || !strings.Contains(string(body), `"entries":[]`) {
		t.Fatalf("unknown agent: %d %s", resp.StatusCode, body)
	}

	// Through the client, before is sent as RFC 3339.
	ctl, err := f.client.Timeline(context.Background(), capture.TimelineQuery{Key: capKey, Limit: 1, Before: f.t0.Add(time.Minute)})
	if err != nil || len(ctl.Entries) != 1 || ctl.Entries[0].Turn.TurnID != "p1" {
		t.Fatalf("client timeline: %v %+v", err, ctl)
	}
}

func TestV03Edits(t *testing.T) {
	f := newCapFixture(t)
	var es []capture.EditRow
	getJSON(t, f.port, "/v1/edits?repo="+url.QueryEscape(f.repoKey)+"&path=a.txt&agent="+url.QueryEscape(capKey)+"&limit=5", &es)
	if len(es) != 1 || es[0].ID != "e-1" {
		t.Fatalf("edits %+v", es)
	}
	for _, p := range []string{"/v1/edits?path=b.txt", "/v1/edits?agent=dev1/other", "/v1/edits?repo=nope"} {
		resp := do(t, f.port, "GET", p, bearer(testToken))
		body, _ := io.ReadAll(resp.Body)
		if strings.TrimSpace(string(body)) != "[]" {
			t.Fatalf("GET %s: %s", p, body)
		}
	}
	ces, err := f.client.Edits(context.Background(), capture.EditQuery{Path: "a.txt"})
	if err != nil || len(ces) != 1 {
		t.Fatalf("client edits: %v %+v", err, ces)
	}
}

func TestV03Commits(t *testing.T) {
	f := newCapFixture(t)
	var cs []capture.CommitView
	getJSON(t, f.port, "/v1/commits?repo="+url.QueryEscape(f.repoKey), &cs)
	if len(cs) != 1 || cs[0].SHA != f.head || cs[0].Link == nil || cs[0].Link.Committer.TurnID != "p1" {
		t.Fatalf("commits %+v", cs)
	}
	getJSON(t, f.port, "/v1/commits", &cs) // all repos
	if len(cs) != 1 {
		t.Fatalf("all commits %d", len(cs))
	}
	var cv capture.CommitView
	getJSON(t, f.port, "/v1/commits/"+f.head, &cv)
	if cv.SHA != f.head {
		t.Fatalf("commit %+v", cv)
	}
	getJSON(t, f.port, "/v1/commits/"+strings.ToUpper(f.head[:7]), &cv) // a prefix, any case
	if cv.SHA != f.head {
		t.Fatalf("commit by prefix %+v", cv)
	}
	miss := "0000000"
	if strings.HasPrefix(f.head, miss) {
		miss = "fffffff"
	}
	wantStatus(t, f.port, "/v1/commits/"+miss, http.StatusNotFound)

	ccv, err := f.client.Commit(context.Background(), f.head[:10])
	if err != nil || ccv.SHA != f.head {
		t.Fatalf("client commit: %v %+v", err, ccv)
	}
	var se *StatusError
	if _, err := f.client.Commit(context.Background(), miss); !errors.As(err, &se) || se.Code != 404 {
		t.Fatalf("client commit miss: %v", err)
	}
	ccs, err := f.client.Commits(context.Background(), "", 10)
	if err != nil || len(ccs) != 1 {
		t.Fatalf("client commits: %v %+v", err, ccs)
	}
}

func TestV03Blobs(t *testing.T) {
	f := newCapFixture(t)
	resp := do(t, f.port, "GET", "/v1/blobs/"+f.patch, bearer(testToken))
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != 200 || string(body) != f.patchTx {
		t.Fatalf("blob: %d %q", resp.StatusCode, body)
	}
	for k, want := range map[string]string{
		"Content-Type":           "text/plain; charset=utf-8",
		"X-Content-Type-Options": "nosniff",
		"Cache-Control":          "no-store",
	} {
		if got := resp.Header.Get(k); got != want {
			t.Errorf("%s = %q, want %q", k, got, want)
		}
	}
	absent := strings.Repeat("0", 64)
	wantStatus(t, f.port, "/v1/blobs/"+absent, http.StatusNotFound)

	got, err := f.client.Blob(context.Background(), f.patch)
	if err != nil || string(got) != f.patchTx {
		t.Fatalf("client blob: %v %q", err, got)
	}
	var se *StatusError
	if _, err := f.client.Blob(context.Background(), absent); !errors.As(err, &se) || se.Code != 404 {
		t.Fatalf("client blob absent: %v", err)
	}
	if _, err := f.client.Blob(context.Background(), "bad"); !errors.As(err, &se) || se.Code != 400 {
		t.Fatalf("client blob bad id: %v", err)
	}
}

func TestV03Decisions(t *testing.T) {
	f := newCapFixture(t)
	var ds []capture.DecisionView
	getJSON(t, f.port, "/v1/decisions?repo="+url.QueryEscape(f.repoKey)+"&path=a.txt&all=1", &ds)
	if len(ds) != 1 || ds[0].ID != f.decID || ds[0].Status == nil || ds[0].Status.State != decision.OK {
		t.Fatalf("decisions %+v", ds)
	}
	if !strings.HasPrefix(ds[0].ID, "d-") || ds[0].Trust != "untrusted" {
		t.Fatalf("decision id %q trust %q", ds[0].ID, ds[0].Trust)
	}
	getJSON(t, f.port, "/v1/decisions?query=TWO+LINES", &ds)
	if len(ds) != 1 {
		t.Fatalf("query: %d", len(ds))
	}
	resp := do(t, f.port, "GET", "/v1/decisions?query=nothing-matches", bearer(testToken))
	body, _ := io.ReadAll(resp.Body)
	if strings.TrimSpace(string(body)) != "[]" {
		t.Fatalf("no match: %s", body)
	}
	cds, err := f.client.Decisions(context.Background(), decision.Filter{Path: "a.txt", All: true})
	if err != nil || len(cds) != 1 {
		t.Fatalf("client decisions: %v %+v", err, cds)
	}
}

func TestV03PRsAndStatus(t *testing.T) {
	f := newCapFixture(t)
	var prs []ledger.Entry
	getJSON(t, f.port, "/v1/prs?repo=owner/name", &prs)
	if len(prs) != 1 || prs[0].Number != 7 || prs[0].Session != capKey {
		t.Fatalf("prs %+v", prs)
	}
	resp := do(t, f.port, "GET", "/v1/prs?repo=other/repo", bearer(testToken))
	body, _ := io.ReadAll(resp.Body)
	if strings.TrimSpace(string(body)) != "[]" {
		t.Fatalf("other repo: %s", body)
	}
	cprs, err := f.client.PRs(context.Background(), "")
	if err != nil || len(cprs) != 1 {
		t.Fatalf("client prs: %v %+v", err, cprs)
	}

	var st capture.Status
	getJSON(t, f.port, "/v1/capture/status", &st)
	if _, err := f.client.CaptureStatus(context.Background()); err != nil {
		t.Fatal(err)
	}
}

func TestV03Why(t *testing.T) {
	f := newCapFixture(t)
	var res capture.WhyResult
	getJSON(t, f.port, "/v1/why?path=a.txt&line=1&cwd="+url.QueryEscape(f.root), &res)
	if res.Path != "a.txt" || res.RepoKey != f.repoKey || res.BlameSHA != f.head {
		t.Fatalf("why %+v", res)
	}
	if res.Commit == nil || res.Commit.SHA != f.head || len(res.Edits) != 1 || res.Edits[0].ID != "e-1" {
		t.Fatalf("why commit/edits %+v", res)
	}
	if len(res.Turns) != 1 || res.Turns[0].TurnID != "p1" || len(res.Decisions) != 1 {
		t.Fatalf("why turns/decisions %+v", res)
	}

	// An absolute path needs no cwd; the client sends the same query.
	cres, err := f.client.Why(context.Background(), capture.WhyQuery{Path: filepath.Join(f.root, "a.txt"), Line: 1})
	if err != nil || cres.BlameSHA != f.head {
		t.Fatalf("client why: %v %+v", err, cres)
	}

	// A relative path with no repo to resolve it against is 404 (ErrNoRepo).
	wantStatus(t, f.port, "/v1/why?path=a.txt", http.StatusNotFound)
	notRepo := canonDir(t, t.TempDir())
	wantStatus(t, f.port, "/v1/why?path=a.txt&cwd="+url.QueryEscape(notRepo), http.StatusNotFound)
	var se *StatusError
	if _, err := f.client.Why(context.Background(), capture.WhyQuery{Path: "a.txt", Cwd: notRepo}); !errors.As(err, &se) || se.Code != 404 {
		t.Fatalf("client why no repo: %v", err)
	}
}

// GET /v1/why is a read: a git repo capture has never seen is 404, and the
// query does not add it to the known repos (no repo.seen, no reflog watch).
func TestV03WhyUnknownRepoIsReadOnly(t *testing.T) {
	f := newCapFixture(t)
	before := f.c.Repos()

	other := canonDir(t, t.TempDir())
	git(t, other, "init", "-q")
	git(t, other, "config", "core.autocrlf", "false")
	if err := os.WriteFile(filepath.Join(other, "b.txt"), []byte("b\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	git(t, other, "add", "b.txt")
	git(t, other, "commit", "-q", "-m", "add b")
	wantStatus(t, f.port, "/v1/why?line=1&path="+url.QueryEscape(filepath.Join(other, "b.txt")), http.StatusNotFound)
	wantStatus(t, f.port, "/v1/why?path=b.txt&cwd="+url.QueryEscape(other), http.StatusNotFound)

	// A repo with no commits (no repo key) is not a known repo either.
	empty := canonDir(t, t.TempDir())
	git(t, empty, "init", "-q")
	wantStatus(t, f.port, "/v1/why?path=c.txt&cwd="+url.QueryEscape(empty), http.StatusNotFound)

	if after := f.c.Repos(); len(after) != len(before) {
		t.Fatalf("why added repos: before %v after %v", before, after)
	}
}
