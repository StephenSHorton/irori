package capture

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/search"
)

// All rows here are synthetic and go straight into the fold through emit,
// the same path the tailer and the repo watcher use.

// t0 is recent: the ledger only shows entries younger than ledger.Keep.
var t0 = time.Now().UTC().Truncate(time.Second).Add(-2 * time.Hour)

func at(min int) time.Time { return t0.Add(time.Duration(min) * time.Minute) }

const (
	keyA  = device + "/sess-a"
	keyB  = device + "/sess-b"
	repoX = "remote:github.com/acme/widgets"
	repoY = "remote:github.com/acme/other"
)

func (r *rig) emitRow(kind string, payload any) {
	r.t.Helper()
	if err := r.c.impl.emit(r.ctx, "irori", kind, payload); err != nil {
		r.t.Fatal(err)
	}
}

func (r *rig) turn(key, repoKey, turnID, prompt, reply string, started time.Time) TurnRow {
	r.t.Helper()
	row := TurnRow{
		ID: turnRowID(key, "", turnID), Key: key, RepoKey: repoKey, TurnID: turnID,
		Turn: transcript.Turn{ID: turnID, Prompt: prompt, Reply: reply, StartedAt: started, EndedAt: started.Add(time.Minute), Origin: "human"},
	}
	r.emitRow(KindTurn, row)
	return row
}

func (r *rig) edit(key, repoKey, turnID, path string, when time.Time, patch string) EditRow {
	r.t.Helper()
	row := EditRow{
		ID: editRowID(key, "tu-"+path, path), Key: key, TurnID: turnID, ToolUseID: "tu-" + path,
		Tool: "Edit", RepoKey: repoKey, Path: path, Adds: 3, Dels: 1, Fidelity: transcript.Exact, At: when,
	}
	if patch != "" {
		id, err := r.c.impl.blobs.Put([]byte(patch))
		if err != nil {
			r.t.Fatal(err)
		}
		row.Patch = id
	}
	r.emitRow(KindEdit, row)
	return row
}

func (r *rig) commitRow(repoKey, sha, subject string, when time.Time, paths ...string) {
	r.t.Helper()
	files := make([]gitlink.FileChange, len(paths))
	for i, p := range paths {
		files[i] = gitlink.FileChange{Path: p, Status: "M"}
	}
	r.emitRow(KindCommitSeen, CommitRow{RepoKey: repoKey, Op: "commit", Branch: "main",
		Commit: gitlink.Commit{SHA: sha, At: when, Author: "Tester", Subject: subject, Files: files}})
}

func (r *rig) link(repoKey, sha string, committer *Committer, e EditRow) {
	r.t.Helper()
	r.emitRow(KindCommitLink, CommitLink{RepoKey: repoKey, SHA: sha, Committer: committer,
		Files:    []gitlink.FileLink{{Path: e.Path, Link: gitlink.LinkExact, Edits: []string{e.ID}}},
		Sessions: []string{e.Key}, Turns: []string{e.TurnID}, At: e.At})
}

func (r *rig) pr(repo string, n int, session string, role ledger.Role, when time.Time) {
	r.t.Helper()
	r.emitRow(KindPR, ledger.Entry{Key: ledger.Key(repo, n), Repo: repo, Number: n, Session: session,
		Role: role, Source: "gh", At: when, URL: fmt.Sprintf("https://github.com/%s/pull/%d", repo, n)})
}

func (r *rig) search(q SearchQuery) []Hit {
	r.t.Helper()
	hits, err := r.c.Search(r.ctx, q)
	if err != nil {
		r.t.Fatal(err)
	}
	return hits
}

func refs(hits []Hit) []string {
	out := make([]string, len(hits))
	for i, h := range hits {
		out[i] = h.Ref
	}
	return out
}

func TestMergeOwner(t *testing.T) {
	e := func(session string, role ledger.Role, min int, url string) ledger.Entry {
		return ledger.Entry{Key: "pr:a/b#1", Session: session, Role: role, At: at(min), URL: url}
	}
	es := mergeOwner(nil, e(keyA, ledger.Author, 5, ""))
	es = mergeOwner(es, e(keyA, ledger.Author, 2, "u1")) // earlier, first URL
	es = mergeOwner(es, e(keyA, ledger.Author, 9, "u2")) // later, URL kept
	es = mergeOwner(es, e(keyA, ledger.Reviewer, 1, "")) // another role
	if len(es) != 2 || !es[0].At.Equal(at(2)) || es[0].URL != "u1" || es[1].Role != ledger.Reviewer {
		t.Fatalf("merged %+v", es)
	}
}

func TestMatchExpr(t *testing.T) {
	tests := []struct {
		text, want string
		hasText    bool
	}{
		{"", "", false},
		{"   \t\n", "", false},
		{"lease", `"lease"*`, true},
		{"lease refused", `"lease" AND "refused"*`, true},
		{`say "hi"`, `"say" AND """hi"""*`, true},
		{"a-b OR c", `"a-b" AND "OR" AND "c"*`, true},
		{"NEAR( x", `"NEAR(" AND "x"*`, true},
		{"- * ( )", "", true}, // nothing the tokenizer keeps
		{"日本 語", `"日本" AND "語"*`, true},
	}
	for _, tt := range tests {
		got, has := matchExpr(tt.text)
		if got != tt.want || has != tt.hasText {
			t.Errorf("matchExpr(%q) = %q, %v; want %q, %v", tt.text, got, has, tt.want, tt.hasText)
		}
	}
}

func TestSearchCaptureKinds(t *testing.T) {
	r := newRig(t, nil)
	tr := r.turn(keyA, repoX, "p1", "Fix the lease refusal\nsecond line", "Renamed the gizmo handler.", at(0))
	e := r.edit(keyA, repoX, "p1", "internal/lease/lease.go", at(1), "")
	sha := "abcdef1234567890abcdef1234567890abcdef12"
	r.commitRow(repoX, sha, "Teach leases to refuse politely", at(2), "internal/lease/lease.go", "docs/zebra.md")
	r.pr("Acme/Widgets", 12, keyB, ledger.Reviewer, at(3))
	r.pr("Acme/Widgets", 12, keyA, ledger.Author, at(4))
	r.emitRow(KindDecision, decision.Decision{ID: "d-1", By: decision.Party{Key: keyA}, Statement: "Prefer quokka caches", Why: "they are friendly", Trust: "untrusted", At: at(5)})

	tests := []struct {
		name  string
		q     SearchQuery
		ref   string
		title string
		agent string
	}{
		{"turn by prompt", SearchQuery{Text: "refusal"}, "turn:" + tr.ID, "Fix the lease refusal", keyA},
		{"turn by reply", SearchQuery{Text: "gizmo"}, "turn:" + tr.ID, "Fix the lease refusal", keyA},
		{"edit by path", SearchQuery{Text: "lease.go", Kinds: []search.Kind{search.KindEdit}}, "edit:" + e.ID, "Edit internal/lease/lease.go +3 -1", keyA},
		{"commit by subject", SearchQuery{Text: "politely"}, "commit:" + sha, "Teach leases to refuse politely", ""},
		{"commit by file path", SearchQuery{Text: "zebra"}, "commit:" + sha, "Teach leases to refuse politely", ""},
		{"pr best role", SearchQuery{Kinds: []search.Kind{search.KindPR}}, "pr:acme/widgets#12", "PR Acme/Widgets#12 (author)", keyA},
		{"decision by why", SearchQuery{Text: "friendly"}, "decision:d-1", "Prefer quokka caches", keyA},
		{"prefix on last word", SearchQuery{Text: "quok"}, "decision:d-1", "Prefer quokka caches", keyA},
		{"porter stemming", SearchQuery{Text: "refusing", Kinds: []search.Kind{search.KindCommit}}, "commit:" + sha, "Teach leases to refuse politely", ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			hits := r.search(tt.q)
			if len(hits) == 0 || hits[0].Ref != tt.ref {
				t.Fatalf("hits %v, want %s first", refs(hits), tt.ref)
			}
			h := hits[0]
			if h.Title != tt.title || h.Agent != tt.agent {
				t.Fatalf("hit %+v, want title %q agent %q", h, tt.title, tt.agent)
			}
			kind, _, _ := search.ParseRef(tt.ref)
			if h.Kind != kind || h.At.IsZero() {
				t.Fatalf("hit %+v", h)
			}
		})
	}

	// The link fills the commit's agent.
	r.link(repoX, sha, &Committer{Key: keyB, TurnID: "x"}, e)
	if hits := r.search(SearchQuery{Text: "politely"}); len(hits) != 1 || hits[0].Agent != keyB {
		t.Fatalf("after link: %+v", hits)
	}
	// Replay rebuilds the same index.
	r.restart()
	if hits := r.search(SearchQuery{Text: "politely"}); len(hits) != 1 || hits[0].Agent != keyB {
		t.Fatalf("after restart: %+v", hits)
	}
	if n := r.c.impl.idx.size(); n != 5 {
		t.Fatalf("index holds %d docs after replay, want 5", n)
	}
}

func TestSearchRankingAndFilters(t *testing.T) {
	r := newRig(t, nil)
	inBody := r.turn(keyA, repoX, "p1", "Unrelated start", "then the walrus showed up", at(10))
	inTitle := r.turn(keyB, repoY, "p2", "Walrus migration plan", "nothing else", at(0))
	newer := r.turn(keyB, repoX, "p3", "Walrus migration plan", "nothing else", at(5))
	e := r.edit(keyA, repoX, "p1", "walrus/tusk.go", at(11), "")

	tests := []struct {
		name string
		q    SearchQuery
		want []string
	}{
		{"title beats body, then newest", SearchQuery{Text: "walrus", Kinds: []search.Kind{search.KindTurn}},
			[]string{"turn:" + newer.ID, "turn:" + inTitle.ID, "turn:" + inBody.ID}},
		{"kinds", SearchQuery{Text: "walrus", Kinds: []search.Kind{search.KindEdit}}, []string{"edit:" + e.ID}},
		{"repo", SearchQuery{Text: "walrus", RepoKey: repoY}, []string{"turn:" + inTitle.ID}},
		{"agent", SearchQuery{Text: "walrus", Agent: keyA, Kinds: []search.Kind{search.KindTurn}}, []string{"turn:" + inBody.ID}},
		{"before", SearchQuery{Text: "walrus", Before: at(5)}, []string{"turn:" + inTitle.ID}},
		{"empty text lists newest", SearchQuery{}, []string{"edit:" + e.ID, "turn:" + inBody.ID, "turn:" + newer.ID, "turn:" + inTitle.ID}},
		{"empty text with limit", SearchQuery{Limit: 2}, []string{"edit:" + e.ID, "turn:" + inBody.ID}},
		{"all words must match", SearchQuery{Text: "walrus nothing"}, []string{"turn:" + newer.ID, "turn:" + inTitle.ID}},
		{"no match", SearchQuery{Text: "narwhal"}, []string{}},
		{"punctuation only", SearchQuery{Text: "* -"}, []string{}},
		{"quote in text", SearchQuery{Text: `"walrus"`, RepoKey: repoY}, []string{"turn:" + inTitle.ID}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := refs(r.search(tt.q))
			if !slices.Equal(got, tt.want) {
				t.Fatalf("got %v\nwant %v", got, tt.want)
			}
		})
	}
}

func TestSearchLimits(t *testing.T) {
	r := newRig(t, nil)
	for i := range 60 {
		r.turn(keyA, repoX, fmt.Sprintf("p%02d", i), fmt.Sprintf("otter number %d", i), "", at(i))
	}
	tests := []struct {
		limit, want int
	}{
		{0, search.DefaultLimit},
		{-3, search.DefaultLimit},
		{7, 7},
		{search.MaxLimit, search.MaxLimit},
		{search.MaxLimit + 1, search.MaxLimit + 1}, // the "more" probe
		{500, search.MaxLimit + 1},
	}
	for _, tt := range tests {
		for _, text := range []string{"", "otter"} {
			if got := len(r.search(SearchQuery{Text: text, Limit: tt.limit})); got != tt.want {
				t.Errorf("limit %d text %q: %d hits, want %d", tt.limit, text, got, tt.want)
			}
		}
	}
}

func TestSearchSnippet(t *testing.T) {
	r := newRig(t, nil)
	long := strings.Repeat("filler words go here ", 40)
	r.turn(keyA, repoX, "p1", "Short title", long+"\n\nthe capybara\n line two "+long, at(0))
	r.turn(keyA, repoX, "p2", "Empty-text listing", "first line\nsecond line "+long, at(1))

	h := r.search(SearchQuery{Text: "capybara"})
	if len(h) != 1 {
		t.Fatalf("hits %v", refs(h))
	}
	s := h[0].Snippet
	if !strings.Contains(s, "[capybara]") || strings.ContainsAny(s, "\r\n") || utf8.RuneCountInString(s) > search.MaxSnippet {
		t.Fatalf("snippet %q", s)
	}
	h = r.search(SearchQuery{Limit: 1})
	s = h[0].Snippet
	if !strings.HasPrefix(s, "Empty-text listing first line second line") || strings.ContainsAny(s, "\r\n") ||
		utf8.RuneCountInString(s) > search.MaxSnippet || !strings.HasSuffix(s, "…") {
		t.Fatalf("listing snippet %q", s)
	}
	// Titles are one line and capped.
	r.turn(keyA, repoX, "p3", strings.Repeat("tapir ", 50), "", at(2))
	h = r.search(SearchQuery{Text: "tapir"})
	if utf8.RuneCountInString(h[0].Title) > search.MaxTitle {
		t.Fatalf("title %d runes", utf8.RuneCountInString(h[0].Title))
	}
}

// fakeSource is a settable search.Source.
type fakeSource struct {
	mu    sync.Mutex
	kinds []search.Kind
	docs  []search.Doc
	calls []time.Time // since of each SearchDocs
	err   error
	shown search.Shown
}

func (f *fakeSource) SearchKinds() []search.Kind { return f.kinds }

func (f *fakeSource) SearchDocs(ctx context.Context, since time.Time, limit int) ([]search.Doc, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, since)
	if f.err != nil {
		return nil, f.err
	}
	var out []search.Doc
	for _, d := range f.docs {
		u := d.Updated
		if u.IsZero() {
			u = d.At
		}
		if !u.Before(since) {
			out = append(out, d)
		}
	}
	slices.SortStableFunc(out, func(a, b search.Doc) int { return a.Updated.Compare(b.Updated) })
	return out[:min(len(out), limit)], nil
}

func (f *fakeSource) SearchShow(ctx context.Context, ref string, o search.ShowOptions) (search.Shown, error) {
	if ref != f.shown.Ref {
		return search.Shown{}, search.ErrNotFound
	}
	return f.shown, nil
}

func (f *fakeSource) set(docs ...search.Doc) {
	f.mu.Lock()
	f.docs = docs
	f.calls = nil
	f.mu.Unlock()
}

func (f *fakeSource) sinces() []time.Time {
	f.mu.Lock()
	defer f.mu.Unlock()
	return slices.Clone(f.calls)
}

func taskDoc(id, title string, upd time.Time) search.Doc {
	return search.Doc{Ref: "task:" + id, At: upd, Updated: upd, Title: title, Body: "body of " + title, RepoKey: repoX}
}

func TestSearchPullsSources(t *testing.T) {
	r := newRig(t, nil)
	src := &fakeSource{kinds: []search.Kind{search.KindTask, search.KindComment}}
	r.c.AddSource(src)

	src.set(taskDoc("1", "Paint the shed", at(1)), taskDoc("2", "Oil the gate", at(2)))
	if got := refs(r.search(SearchQuery{Text: "shed"})); !slices.Equal(got, []string{"task:1"}) {
		t.Fatalf("first pull: %v", got)
	}
	if s := src.sinces(); len(s) != 1 || !s[0].IsZero() {
		t.Fatalf("first pull since %v, want one zero", s)
	}
	h := r.search(SearchQuery{Text: "gate"})
	if len(h) != 1 || h[0].Kind != search.KindTask || h[0].RepoKey != repoX {
		t.Fatalf("hit %+v", h)
	}

	// A changed doc comes back with a newer Updated and replaces the old.
	src.set(taskDoc("1", "Paint the barn", at(3)), taskDoc("2", "Oil the gate", at(2)))
	if got := refs(r.search(SearchQuery{Text: "barn"})); !slices.Equal(got, []string{"task:1"}) {
		t.Fatalf("after change: %v", got)
	}
	if s := src.sinces(); len(s) != 1 || !s[0].Equal(at(2)) {
		t.Fatalf("second pull since %v, want [%v]", s, at(2))
	}
	if got := r.search(SearchQuery{Text: "shed"}); len(got) != 0 {
		t.Fatalf("old text still found: %v", refs(got))
	}
	// The boundary doc is re-read and upserted, never duplicated.
	if got := refs(r.search(SearchQuery{Kinds: []search.Kind{search.KindTask}})); !slices.Equal(got, []string{"task:1", "task:2"}) {
		t.Fatalf("listing %v", got)
	}

	// Docs of kinds the source doesn't own, and bad refs, are skipped.
	src.set(search.Doc{Ref: "turn:t-forged", Title: "forged", At: at(9)}, search.Doc{Ref: "nope", Title: "forged", At: at(9)})
	if got := r.search(SearchQuery{Text: "forged"}); len(got) != 0 {
		t.Fatalf("foreign kinds indexed: %v", refs(got))
	}

	// A failing source doesn't fail the search.
	src.mu.Lock()
	src.err = errors.New("boom")
	src.mu.Unlock()
	if got := refs(r.search(SearchQuery{Text: "barn"})); !slices.Equal(got, []string{"task:1"}) {
		t.Fatalf("with failing source: %v", got)
	}
	if !strings.Contains(r.c.Status().LastError, "boom") {
		t.Fatalf("status %q", r.c.Status().LastError)
	}
}

func TestSearchPullPaging(t *testing.T) {
	tests := []struct {
		name      string
		n         int
		same      bool // every doc has the same Updated
		wantCalls int
		wantDocs  int
	}{
		{"short page", 3, false, 1, 3},
		{"pages until short", 1200, false, 3, 1200},
		{"exact page then empty-progress", pullPage, false, 2, pullPage},
		{"no progress stops", pullPage + 100, true, 2, pullPage},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := newRig(t, nil)
			src := &fakeSource{kinds: []search.Kind{search.KindMsg}}
			r.c.AddSource(src)
			docs := make([]search.Doc, tt.n)
			for i := range docs {
				u := t0.Add(time.Duration(i) * time.Second)
				if tt.same {
					u = t0
				}
				docs[i] = search.Doc{Ref: fmt.Sprintf("msg:m%05d", i), At: u, Updated: u, Title: "hello"}
			}
			src.set(docs...)
			r.search(SearchQuery{})
			if got := len(src.sinces()); got != tt.wantCalls {
				t.Fatalf("%d SearchDocs calls, want %d", got, tt.wantCalls)
			}
			if got := r.c.impl.idx.size(); got != tt.wantDocs {
				t.Fatalf("%d docs indexed, want %d", got, tt.wantDocs)
			}
		})
	}
}

func TestAddSourcePanics(t *testing.T) {
	tests := []struct {
		name  string
		first []search.Kind
		next  []search.Kind
	}{
		{"capture's turn", nil, []search.Kind{search.KindTurn}},
		{"capture's agent", nil, []search.Kind{search.KindAgent}},
		{"capture's pr", nil, []search.Kind{search.KindMsg, search.KindPR}},
		{"duplicate", []search.Kind{search.KindTask}, []search.Kind{search.KindComment, search.KindTask}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := newRig(t, nil)
			if tt.first != nil {
				r.c.AddSource(&fakeSource{kinds: tt.first})
			}
			defer func() {
				if recover() == nil {
					t.Fatal("no panic")
				}
				// Nothing of the refused source was registered.
				if s := r.c.impl.srcs.byKind[search.KindMsg]; s != nil {
					t.Fatal("refused source half-registered")
				}
			}()
			r.c.AddSource(&fakeSource{kinds: tt.next})
		})
	}
}

func TestShow(t *testing.T) {
	r := newRig(t, nil)
	tr := r.turn(keyA, repoX, "p1", "Fix the lease refusal", "Done: leases refuse.", at(0))
	e := r.edit(keyA, repoX, "p1", "internal/lease/lease.go", at(1), "@@ -1 +1 @@\n-old\n+new\n")
	plain := r.edit(keyA, repoX, "p1", "README.md", at(1), "")
	sha := "abcdef1234567890abcdef1234567890abcdef12"
	r.commitRow(repoX, sha, "Teach leases to refuse", at(2), "internal/lease/lease.go")
	r.link(repoX, sha, &Committer{Key: keyA, TurnID: "p1"}, e)
	r.pr("Acme/Widgets", 12, keyA, ledger.Author, at(3))
	r.pr("Acme/Widgets", 12, keyB, ledger.Reviewer, at(4))
	r.emitRow(KindDecision, decision.Decision{ID: "d-1", By: decision.Party{Key: keyA, Name: "alice"}, Statement: "Prefer quokka caches", Why: "they are friendly", Trust: "untrusted", At: at(5)})
	r.emitRow(KindDecision, decision.Decision{ID: "d-2", By: decision.Party{Key: keyA}, Statement: "Prefer wombat caches", Supersedes: []string{"d-1"}, Trust: "untrusted", At: at(6)})
	r.roster.set(presence.AgentCard{Key: keyA, DeviceID: device, SessionID: "sess-a", Name: "alice", Status: "idle", RepoKey: repoX})
	src := &fakeSource{kinds: []search.Kind{search.KindTask}, shown: search.Shown{Ref: "task:9", Kind: search.KindTask, Title: "from source"}}
	r.c.AddSource(src)

	tests := []struct {
		name    string
		ref     string
		wantRef string
		title   string
		text    []string // substrings
		related []string // exact
	}{
		{"turn", "turn:" + tr.ID, "turn:" + tr.ID, "Fix the lease refusal",
			[]string{"prompt:\nFix the lease refusal", "reply:\nDone: leases refuse.", "edit:" + e.ID + " Edit internal/lease/lease.go +3 -1", "commit:" + sha + " Teach leases to refuse"},
			[]string{"edit:" + e.ID, "edit:" + plain.ID, "commit:" + sha, "agent:" + keyA}},
		{"edit with patch", "edit:" + e.ID, "edit:" + e.ID, "Edit internal/lease/lease.go +3 -1",
			[]string{"fidelity exact", "turn: turn:" + tr.ID, "committed in: commit:" + sha, "patch:\n@@ -1 +1 @@\n-old\n+new"},
			[]string{"turn:" + tr.ID, "commit:" + sha}},
		{"edit without patch", "edit:" + plain.ID, "edit:" + plain.ID, "Edit README.md +3 -1",
			[]string{"(no patch stored)"}, []string{"turn:" + tr.ID}},
		{"commit by prefix", "commit:ABCDEF1", "commit:" + sha, "Teach leases to refuse",
			[]string{"author: Tester", "branch: main", "- M internal/lease/lease.go (exact)", "committed by: " + keyA},
			[]string{"turn:" + tr.ID}},
		{"pr", "pr:Acme/Widgets#12", "pr:acme/widgets#12", "PR Acme/Widgets#12",
			[]string{"url: https://github.com/Acme/Widgets/pull/12", "agent:" + keyA + " author via gh", "agent:" + keyB + " reviewer via gh"},
			[]string{"agent:" + keyA, "agent:" + keyB}},
		{"decision", "decision:d-1", "decision:d-1", "Prefer quokka caches",
			[]string{"by alice (untrusted)", "why:\nthey are friendly", "superseded by: decision:d-2"},
			[]string{"decision:d-2"}},
		{"agent", "agent:" + keyA, "agent:" + keyA, "alice",
			[]string{"agent alice (" + keyA + "), idle", "turn:" + tr.ID + " Fix the lease refusal", "commit:" + sha + " Teach leases", "pr:acme/widgets#12 author"},
			[]string{"pr:acme/widgets#12", "commit:" + sha, "turn:" + tr.ID}}, // newest first
		{"source kind", "task:9", "task:9", "from source", nil, nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			sh, err := r.c.Show(r.ctx, tt.ref, ShowOptions{})
			if err != nil {
				t.Fatal(err)
			}
			if sh.Ref != tt.wantRef || sh.Title != tt.title || sh.More {
				t.Fatalf("shown %+v", sh)
			}
			for _, s := range tt.text {
				if !strings.Contains(sh.Text, s) {
					t.Errorf("text lacks %q:\n%s", s, sh.Text)
				}
			}
			if !slices.Equal(sh.Related, tt.related) {
				t.Errorf("related %v, want %v", sh.Related, tt.related)
			}
		})
	}

	errTests := []struct {
		ref  string
		want error
	}{
		{"", search.ErrBadRef},
		{"turn", search.ErrBadRef},
		{"bogus:1", search.ErrBadRef},
		{"commit:abc", search.ErrBadRef},
		{"turn:t-missing", search.ErrNotFound},
		{"edit:e-missing", search.ErrNotFound},
		{"commit:1234567", search.ErrNotFound},
		{"pr:acme/widgets#99", search.ErrNotFound},
		{"decision:nope", search.ErrNotFound},
		{"agent:dev1/nobody", search.ErrNotFound},
		{"task:other", search.ErrNotFound}, // the source has no such ref
		{"msg:m1", search.ErrNotFound},     // no source owns msg here
		{"comment:c1", search.ErrNotFound}, // nor comment
	}
	for _, tt := range errTests {
		if _, err := r.c.Show(r.ctx, tt.ref, ShowOptions{}); !errors.Is(err, tt.want) {
			t.Errorf("Show(%q) = %v, want %v", tt.ref, err, tt.want)
		}
	}
}

func TestShowPaging(t *testing.T) {
	r := newRig(t, nil)
	reply := strings.Repeat("ünïcode text, ", 1000) // well past MaxShow
	tr := r.turn(keyA, repoX, "p1", "Long one", reply, at(0))
	ref := "turn:" + tr.ID

	sh, err := r.c.Show(r.ctx, ref, ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if !sh.More || len(sh.Text) > search.DefaultShow || sh.Next != len(sh.Text) || !utf8.ValidString(sh.Text) {
		t.Fatalf("first page: more %v next %d len %d", sh.More, sh.Next, len(sh.Text))
	}
	big, _ := r.c.Show(r.ctx, ref, ShowOptions{Limit: 1 << 20})
	if len(big.Text) > search.MaxShow {
		t.Fatalf("limit not capped: %d", len(big.Text))
	}
	// Walking Next reassembles the whole rendering.
	var all strings.Builder
	for o := 0; ; {
		p, err := r.c.Show(r.ctx, ref, ShowOptions{Offset: o, Limit: 1000})
		if err != nil {
			t.Fatal(err)
		}
		if !utf8.ValidString(p.Text) {
			t.Fatalf("page at %d cuts a rune", o)
		}
		all.WriteString(p.Text)
		if !p.More {
			break
		}
		o = p.Next
	}
	if !strings.Contains(all.String(), strings.TrimSpace(reply)) {
		t.Fatal("pages don't reassemble the reply")
	}
}

func TestShowRelatedCap(t *testing.T) {
	r := newRig(t, nil)
	tr := r.turn(keyA, repoX, "p1", "Many edits", "", at(0))
	for i := range 20 {
		r.edit(keyA, repoX, "p1", fmt.Sprintf("f%02d.go", i), at(1), "")
	}
	sh, err := r.c.Show(r.ctx, "turn:"+tr.ID, ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if len(sh.Related) != search.MaxRelated {
		t.Fatalf("%d related, want %d", len(sh.Related), search.MaxRelated)
	}
	if strings.Count(sh.Text, "edit:e-") != 20 {
		t.Fatalf("text should list every edit:\n%s", sh.Text)
	}
}

func TestShowOpenTurn(t *testing.T) {
	r := newRig(t, nil)
	r.c.impl.mu.Lock()
	r.c.impl.open[openKey(keyA, "")] = &transcript.Turn{ID: "p9", Prompt: "Still going", StartedAt: at(0)}
	r.c.impl.mu.Unlock()
	sh, err := r.c.Show(r.ctx, "turn:"+turnRowID(keyA, "", "p9"), ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(sh.Text, ", open;") || !strings.Contains(sh.Text, "Still going") {
		t.Fatalf("open turn:\n%s", sh.Text)
	}
}

func TestOverview(t *testing.T) {
	r := newRig(t, nil)
	card := func(sid, name, status, repoKey string) presence.AgentCard {
		return presence.AgentCard{Key: device + "/" + sid, DeviceID: device, SessionID: sid, Name: name, Status: status, RepoKey: repoKey}
	}
	cards := []presence.AgentCard{
		card("sess-z", "zed", "idle", repoY),
		card("sess-b", "Bob", "busy", repoX),
		card("sess-a", "alice", "busy", repoX),
		card("sess-me", "me", "busy", repoX),
		card("sess-c", "carol", "idle", repoX),
		card("sess-gone", "ghost", "gone", repoX),
	}
	r.roster.set(cards...)
	me := device + "/sess-me"
	bob := device + "/sess-b"
	alice := device + "/sess-a"
	carol := device + "/sess-c"
	zed := device + "/sess-z"

	// alice: open turn (wins over everything). bob: a current task (wins
	// over his last reply). carol: only replies, the newest wins. zed: nothing.
	r.c.impl.mu.Lock()
	r.c.impl.open[openKey(alice, "")] = &transcript.Turn{ID: "pa", Prompt: "Refactor the ferret\nmore", StartedAt: at(50)}
	r.c.impl.mu.Unlock()
	r.turn(alice, repoX, "a1", "old", "alice old reply", at(1))
	r.turn(bob, repoX, "b1", "q", "bob reply", at(2))
	r.turn(carol, repoX, "c1", "q", "carol older reply", at(3))
	newest := r.turn(carol, repoX, "c2", "q", strings.Repeat("carol newest reply ", 20), at(4))
	r.turn(carol, repoX, "c3", "q", "", at(5)) // no reply: skipped

	for i := range 7 {
		r.commitRow(repoX, fmt.Sprintf("%040x", i+1), fmt.Sprintf("commit %d", i), at(10+i))
	}
	r.commitRow(repoY, fmt.Sprintf("%040x", 99), "other repo", at(30))
	for i := range 7 {
		r.pr("acme/widgets", 100+i, bob, ledger.Author, at(20))
		r.pr("acme/widgets", 100+i, alice, ledger.Reviewer, at(21)) // same PR twice: listed once
	}
	r.pr("acme/other", 5, bob, ledger.Author, at(20))

	long := strings.Repeat("x", 300)
	in := OverviewInput{
		InboxOpen: 3, InboxHeld: 1,
		Inbox:      []Line{{Ref: "msg:m1", Text: "hello\nworld"}},
		Blockers:   []Line{{Ref: "task:b1", Text: long}},
		Tasks:      []Line{{Ref: "task:1"}, {Ref: "task:2"}, {Ref: "task:3"}, {Ref: "task:4"}, {Ref: "task:5"}, {Ref: "task:6"}},
		MoreTasks:  2,
		Current:    map[string]Line{bob: {Ref: "task:7", Text: "Ship the badger"}, alice: {Ref: "task:8", Text: "not shown"}},
		RepoKey:    repoX,
		BoardCount: 9,
	}
	ov := r.c.Overview(r.ctx, me, in)

	var order []string
	for _, a := range ov.Agents {
		order = append(order, a.Name)
	}
	if want := []string{"me", "alice", "Bob", "carol", "zed"}; !slices.Equal(order, want) {
		t.Fatalf("agent order %v, want %v", order, want)
	}
	doing := map[string]Line{}
	for _, a := range ov.Agents {
		doing[a.Key] = a.Doing
	}
	tests := []struct {
		key, ref, text string
	}{
		{alice, "agent:" + alice, "Refactor the ferret"},
		{bob, "task:7", "Ship the badger"},
		{carol, "turn:" + newest.ID, clipRunes(strings.TrimSpace(strings.Repeat("carol newest reply ", 20)), OverviewLineMax)},
		{zed, "agent:" + zed, ""},
		{me, "agent:" + me, ""},
	}
	for _, tt := range tests {
		d := doing[tt.key]
		if d.Ref != tt.ref || d.Text != tt.text {
			t.Errorf("%s doing %+v, want ref %q text %q", tt.key, d, tt.ref, tt.text)
		}
		if utf8.RuneCountInString(d.Text) > OverviewLineMax {
			t.Errorf("%s doing too long", tt.key)
		}
	}
	if len(ov.Commits) != OverviewLines || ov.Commits[0].Ref != "commit:"+fmt.Sprintf("%040x", 7) || ov.Commits[0].Text != "commit 6" {
		t.Fatalf("commits %+v", ov.Commits)
	}
	if len(ov.PRs) != OverviewLines {
		t.Fatalf("prs %+v", ov.PRs)
	}
	seen := map[string]bool{}
	for _, p := range ov.PRs {
		if seen[p.Ref] || !strings.HasPrefix(p.Ref, "pr:acme/widgets#") {
			t.Fatalf("prs %+v", ov.PRs)
		}
		seen[p.Ref] = true
	}
	if ov.PRs[0].Ref != "pr:acme/widgets#106" || ov.PRs[0].Text != "PR acme/widgets#106 author Bob" {
		t.Fatalf("first pr %+v", ov.PRs[0])
	}
	if ov.InboxOpen != 3 || ov.InboxHeld != 1 || ov.Inbox[0].Text != "hello" {
		t.Fatalf("inbox %+v", ov)
	}
	if utf8.RuneCountInString(ov.Blockers[0].Text) != OverviewLineMax {
		t.Fatalf("blocker not clipped: %d", utf8.RuneCountInString(ov.Blockers[0].Text))
	}
	if len(ov.Tasks) != OverviewLines || ov.MoreTasks != 3 {
		t.Fatalf("tasks %d more %d", len(ov.Tasks), ov.MoreTasks)
	}
	if ov.For != me || ov.RepoKey != repoX || !strings.Contains(ov.Hint, "show <ref>") || !strings.Contains(ov.Hint, "(9 open)") {
		t.Fatalf("overview %+v", ov)
	}
}

func TestOverviewCapsAndRepoFallback(t *testing.T) {
	r := newRig(t, nil)
	var cards []presence.AgentCard
	for i := range 15 {
		rk := repoY
		if i%2 == 0 {
			rk = repoX
		}
		cards = append(cards, presence.AgentCard{Key: fmt.Sprintf("%s/s%02d", device, i), SessionID: fmt.Sprintf("s%02d", i),
			Name: fmt.Sprintf("agent %02d", i), Status: "idle", RepoKey: rk})
	}
	r.roster.set(cards...)
	me := device + "/s07" // in repoY
	r.commitRow(repoY, fmt.Sprintf("%040x", 1), "in y", at(0))
	r.commitRow(repoX, fmt.Sprintf("%040x", 2), "in x", at(1))

	ov := r.c.Overview(r.ctx, me, OverviewInput{})
	if len(ov.Agents) != OverviewAgents || ov.MoreAgents != 3 {
		t.Fatalf("%d agents, %d more", len(ov.Agents), ov.MoreAgents)
	}
	if ov.Agents[0].Key != me || ov.RepoKey != repoY {
		t.Fatalf("first %s repo %s", ov.Agents[0].Key, ov.RepoKey)
	}
	for i, a := range ov.Agents[1:7] { // the other six in repoY come next
		if a.RepoKey != repoY {
			t.Fatalf("agent %d %s in %s", i+1, a.Key, a.RepoKey)
		}
	}
	if len(ov.Commits) != 1 || ov.Commits[0].Text != "in y" {
		t.Fatalf("commits %+v", ov.Commits)
	}
	if !strings.Contains(ov.Hint, "3 more agents") {
		t.Fatalf("hint %q", ov.Hint)
	}

	// A repo that isn't on GitHub never lists every repo's PRs.
	r.pr("acme/widgets", 1, me, ledger.Author, at(0))
	ov = r.c.Overview(r.ctx, me, OverviewInput{RepoKey: "path:C:/somewhere"})
	if len(ov.PRs) != 0 {
		t.Fatalf("prs for a non-GitHub repo: %+v", ov.PRs)
	}
	// No repo at all: no commits or PRs.
	ov = r.c.Overview(r.ctx, device+"/not-live", OverviewInput{})
	if ov.RepoKey != "" || len(ov.Commits) != 0 || len(ov.PRs) != 0 {
		t.Fatalf("no-repo overview %+v", ov)
	}
}

// Offset pages ranked and plain results alike: a newer, weaker match and a
// doc sharing the last hit's At are both reached on the next page.
func TestSearchOffsetPaging(t *testing.T) {
	r := newRig(t, nil)
	r.turn(keyA, repoX, "p1", "wombat wombat", "", at(0))              // strong title match, older
	r.turn(keyA, repoX, "p2", "feeding time", "a wombat came", at(10)) // weak body match, newer
	r.turn(keyA, repoX, "p3", "same minute", "", at(10))
	tests := []struct {
		name string
		text string
	}{
		{"ranked", "wombat"},
		{"plain", ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			all := refs(r.search(SearchQuery{Text: tt.text}))
			var paged []string
			for off := 0; off <= len(all); off++ {
				paged = append(paged, refs(r.search(SearchQuery{Text: tt.text, Limit: 1, Offset: off}))...)
			}
			if !slices.Equal(paged, all) {
				t.Fatalf("paged %v, want %v", paged, all)
			}
		})
	}
	if got := len(r.search(SearchQuery{Offset: -4})); got != 3 {
		t.Fatalf("negative offset: %d hits", got)
	}
}

// A doc with Readers is found only by the person ("") and the agents it
// names; docs without Readers are found by everyone.
func TestSearchViewer(t *testing.T) {
	r := newRig(t, nil)
	src := &fakeSource{kinds: []search.Kind{search.KindMsg}}
	r.c.AddSource(src)
	src.set(
		search.Doc{Ref: "msg:1", At: at(1), Title: "alpha -> beta", Body: "secret pangolin plan", Readers: []string{keyA, keyB}},
		search.Doc{Ref: "msg:2", At: at(2), Title: "note", Body: "public pangolin note"},
	)
	tests := []struct {
		viewer string
		want   []string
	}{
		{"", []string{"msg:1", "msg:2"}},
		{keyA, []string{"msg:1", "msg:2"}},
		{keyB, []string{"msg:1", "msg:2"}},
		{device + "/sess-c", []string{"msg:2"}},
		{keyA[:len(keyA)-1], []string{"msg:2"}}, // a prefix of a reader is not that reader
	}
	for _, tt := range tests {
		got := refs(r.search(SearchQuery{Text: "pangolin", Viewer: tt.viewer}))
		slices.Sort(got)
		if !slices.Equal(got, tt.want) {
			t.Errorf("viewer %q: %v, want %v", tt.viewer, got, tt.want)
		}
		if got := refs(r.search(SearchQuery{Viewer: tt.viewer})); len(got) != len(tt.want) {
			t.Errorf("viewer %q listing: %v", tt.viewer, got)
		}
	}
}

// A wake's prompt carries the messages it delivered. The turn it starts is
// the recipient's, but the message text stays with the message: no agent
// finds it through turns, and only the recipient and the person see it when
// they show the turn (design/board.md: no other agent finds or shows a
// message).
func TestTurnFenceKeepsMessagesPrivate(t *testing.T) {
	r := newRig(t, nil)
	prompt := inbox.FencePrompt("[irori-wake:tok1]", false, []inbox.Delivery{{
		ID: "d-1", MessageID: "m-1", To: keyA, Mode: inbox.ModeWake,
		Message: inbox.Message{ID: "m-1", From: inbox.Sender{Key: "user:" + device, Name: "Stephen", Kind: "user"},
			Text: "secret mango for alpha only", Trust: inbox.UserTrust},
	}})
	row := r.turn(keyA, repoX, "t-fence", prompt, "on it, peeling now", at(1))
	ref := search.Ref(search.KindTurn, row.ID)

	for _, viewer := range []string{"", keyA, keyB} {
		if got := refs(r.search(SearchQuery{Text: "mango", Kinds: []search.Kind{search.KindTurn}, Viewer: viewer})); len(got) != 0 {
			t.Errorf("viewer %q finds the message text through turns: %v", viewer, got)
		}
		for _, h := range r.search(SearchQuery{Kinds: []search.Kind{search.KindTurn}, Viewer: viewer}) {
			if strings.Contains(h.Snippet+h.Title, "mango") {
				t.Errorf("viewer %q: hit %s shows the message text: %q", viewer, h.Ref, h.Snippet)
			}
		}
	}
	// The reply is the agent's own work: still found by everyone.
	if got := refs(r.search(SearchQuery{Text: "peeling", Viewer: keyB})); !slices.Contains(got, ref) {
		t.Errorf("reply search for another agent: %v, want %s", got, ref)
	}

	tests := []struct {
		viewer string
		sees   bool
	}{
		{"", true},    // the person
		{keyA, true},  // the recipient, whose turn it is
		{keyB, false}, // anyone else
	}
	for _, tt := range tests {
		sh, err := r.c.Show(r.ctx, ref, ShowOptions{Viewer: tt.viewer, Limit: search.MaxShow})
		if err != nil {
			t.Fatalf("viewer %q: show %s: %v", tt.viewer, ref, err)
		}
		if got := strings.Contains(sh.Text, "secret mango"); got != tt.sees {
			t.Errorf("viewer %q sees the message text: %v, want %v\n%s", tt.viewer, got, tt.sees, sh.Text)
		}
		if !strings.Contains(sh.Text, `delivery="d-1"`) || !strings.Contains(sh.Text, "peeling now") {
			t.Errorf("viewer %q: the turn lost its fence header or reply:\n%s", tt.viewer, sh.Text)
		}
		ag, err := r.c.Show(r.ctx, search.Ref(search.KindAgent, keyA), ShowOptions{Viewer: tt.viewer, Limit: search.MaxShow})
		if err != nil {
			t.Fatalf("viewer %q: show agent: %v", tt.viewer, err)
		}
		if strings.Contains(ag.Text, "mango") {
			t.Errorf("viewer %q: agent timeline shows the message text:\n%s", tt.viewer, ag.Text)
		}
	}
}
