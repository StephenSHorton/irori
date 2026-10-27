package api

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/search"
	"github.com/StephenSHorton/irori/internal/store"
)

// The Board and browsing routes over a real inbox, coord and capture on one
// temp store. Two agents: the attached Claude session dev1/s1 (boardRepo) and
// the Grok member dev1/grok-1.

const (
	boardRepo    = "remote:github.com/owner/board"
	claudeKey    = "dev1/s1"
	grokKey      = "dev1/grok-1"
	memberSecret = "member-secret-1"
	personKey    = "user:dev1"
)

// boardBackend is fakeBackend with the services behind the Board routes.
type boardBackend struct {
	*fakeBackend
	ib  *inbox.Inbox
	co  *coord.Coord
	cap *capture.Capture
}

func (b *boardBackend) Agents() []presence.AgentCard {
	return []presence.AgentCard{
		{Key: claudeKey, DeviceID: "dev1", Agent: "claude", SessionID: "s1", Name: "alpha", Status: "busy", RepoKey: boardRepo},
		{Key: grokKey, DeviceID: "dev1", Agent: "grok", SessionID: "grok-1", Name: "gamma", Status: "idle", RepoKey: boardRepo},
	}
}
func (b *boardBackend) Inbox() *inbox.Inbox       { return b.ib }
func (b *boardBackend) Coord() *coord.Coord       { return b.co }
func (b *boardBackend) Capture() *capture.Capture { return b.cap }
func (b *boardBackend) MemberBySecret(s string) (Member, bool) {
	if s == memberSecret {
		return Member{Key: grokKey, Secret: s, Name: "gamma"}, true
	}
	return Member{}, false
}

type boardFixture struct {
	b      *boardBackend
	port   int
	client *Client
	attach string // dev1/s1's attach secret
	// Seeded rows.
	assigned, blocked, open coord.Task
	comment                 coord.Comment
	msgToGrok, msgToClaude  string // message ids, the person to each agent
}

func newBoardFixture(t *testing.T) *boardFixture {
	t.Helper()
	ctx := context.Background()
	data := canonDir(t, t.TempDir())
	st, err := store.Open(ctx, filepath.Join(data, "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	b := &boardBackend{fakeBackend: &fakeBackend{}}
	if b.ib, err = inbox.New(ctx, st, b, inbox.Options{DeviceID: "dev1"}); err != nil {
		t.Fatal(err)
	}
	if b.co, err = coord.New(ctx, st, b, coord.Options{}); err != nil {
		t.Fatal(err)
	}
	if b.cap, err = capture.New(ctx, st, b, capture.Options{
		ClaudeDir: canonDir(t, t.TempDir()), DataDir: data, DeviceID: "dev1", RepoKey: repo.New().Key,
	}); err != nil {
		t.Fatal(err)
	}
	b.cap.AddSource(b.co)
	b.cap.AddSource(b.ib)

	f := &boardFixture{b: b}
	must := func(err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}
	f.assigned, err = b.co.CreateTask(ctx, personKey, coord.NewTask{Title: "wire the board routes", Objective: "loopback reads", RepoKey: boardRepo})
	must(err)
	f.assigned, _, err = b.co.Assign(ctx, personKey, f.assigned.ID, claudeKey)
	must(err)
	f.comment, _, err = b.co.AddComment(ctx, personKey, f.assigned.ID, "use the pineapple fixture", false)
	must(err)
	f.blocked, err = b.co.CreateTask(ctx, personKey, coord.NewTask{Title: "port 5173 is taken", RepoKey: boardRepo, BlockReason: "dev server left running"})
	must(err)
	f.open, err = b.co.CreateTask(ctx, personKey, coord.NewTask{Title: "tidy the docs", RepoKey: boardRepo})
	must(err)

	person := inbox.Sender{Key: personKey, Name: "Stephen", Kind: "user"}
	m, _, err := b.ib.SendWith(ctx, person, inbox.Address{Kind: inbox.ToAgent, Value: grokKey}, "secret mango for gamma only", inbox.SendOptions{})
	must(err)
	f.msgToGrok = m.ID
	m, _, err = b.ib.SendWith(ctx, person, inbox.Address{Kind: inbox.ToAgent, Value: claudeKey}, "kiwi note for alpha", inbox.SendOptions{})
	must(err)
	f.msgToClaude = m.ID

	f.port = startWith(t, b)
	f.client = NewClient(discovery.Lockfile{Port: f.port, Token: testToken})
	f.attach = f.openAttach(t)
	return f
}

// openAttach opens dev1/s1's attach stream for the test's life and returns
// its secret from the hello line.
func (f *boardFixture) openAttach(t *testing.T) string {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	body, err := f.client.OpenStream(ctx, url.Values{"session": {"s1"}, "mod": {"0.4.0"}})
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	t.Cleanup(func() { cancel(); body.Close() })
	line, err := bufio.NewReader(body).ReadBytes('\n')
	if err != nil {
		t.Fatal(err)
	}
	var hello attach.Event
	if err := json.Unmarshal(line, &hello); err != nil || hello.AttachSecret == "" {
		t.Fatalf("hello: %v: %s", err, line)
	}
	go io.Copy(io.Discard, body) // keep the stream drained
	return hello.AttachSecret
}

var (
	asPerson = Identity{}
	asMember = Identity{Member: memberSecret}
)

func (f *boardFixture) asClaude() Identity { return Identity{Attach: f.attach} }

// get sends a GET with the token and the given identity headers.
func (f *boardFixture) get(t *testing.T, path string, hdr map[string]string) (int, []byte) {
	t.Helper()
	resp := do(t, f.port, "GET", path, func(r *http.Request) {
		r.Header.Set("Authorization", "Bearer "+testToken)
		for k, v := range hdr {
			r.Header.Set(k, v)
		}
	})
	body, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, body
}

func statusOf(err error) int {
	var se *StatusError
	if errors.As(err, &se) {
		return se.Code
	}
	if err != nil {
		return -1
	}
	return http.StatusOK
}

var boardPaths = []string{
	"/v1/board", "/v1/tasks/x", "/v1/search?q=a", "/v1/show?ref=task:x", "/v1/overview?agent=" + claudeKey,
}

func TestBoardRoutesKeepV01Rules(t *testing.T) {
	_, port := start(t)
	for _, p := range boardPaths {
		if resp := do(t, port, "GET", p, nil); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s without token: %d, want 401", p, resp.StatusCode)
		}
		if resp := do(t, port, "GET", p, bearer("wrong")); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s wrong token: %d, want 401", p, resp.StatusCode)
		}
		resp := do(t, port, "POST", p, bearer(testToken))
		if resp.StatusCode != http.StatusMethodNotAllowed || resp.Header.Get("Allow") != "GET" {
			t.Errorf("POST %s: %d Allow %q, want 405 GET", p, resp.StatusCode, resp.Header.Get("Allow"))
		}
		resp = do(t, port, "GET", p, func(r *http.Request) {
			bearer(testToken)(r)
			r.Header.Set("Origin", "https://example.com")
		})
		if resp.StatusCode != http.StatusForbidden {
			t.Errorf("%s with Origin: %d, want 403", p, resp.StatusCode)
		}
	}
}

func TestBoardRoutesWithoutServices503(t *testing.T) {
	_, port := start(t) // fakeBackend: no coord, inbox or capture
	for _, p := range boardPaths {
		wantStatus(t, port, p, http.StatusServiceUnavailable)
	}
}

func TestBoardViewerHeaders(t *testing.T) {
	f := newBoardFixture(t)
	tests := []struct {
		name string
		hdr  map[string]string
		want int
	}{
		{"person", nil, http.StatusOK},
		{"attach", map[string]string{attach.HeaderAttach: f.attach}, http.StatusOK},
		{"member", map[string]string{attach.HeaderMember: memberSecret}, http.StatusOK},
		{"unknown attach", map[string]string{attach.HeaderAttach: "stale"}, http.StatusForbidden},
		{"unknown member", map[string]string{attach.HeaderMember: "stale"}, http.StatusForbidden},
		{"both", map[string]string{attach.HeaderAttach: f.attach, attach.HeaderMember: memberSecret}, http.StatusForbidden},
		// A header that is sent is an identity claim even when empty: never
		// the person's view.
		{"empty attach", map[string]string{attach.HeaderAttach: ""}, http.StatusForbidden},
		{"blank member", map[string]string{attach.HeaderMember: " "}, http.StatusForbidden},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			for _, p := range []string{"/v1/board", "/v1/tasks/" + f.open.ID, "/v1/search?q=mango", "/v1/show?ref=task:" + f.open.ID, "/v1/overview"} {
				if tc.hdr == nil && p == "/v1/overview" {
					p += "?agent=" + claudeKey
				}
				code, body := f.get(t, p, tc.hdr)
				if code != tc.want {
					t.Errorf("%s: %d, want %d: %s", p, code, tc.want, body)
				}
			}
		})
	}
}

func TestBoardBadParams400(t *testing.T) {
	f := newBoardFixture(t)
	for _, p := range []string{
		"/v1/board?done=0", "/v1/board?done=x",
		"/v1/search?kinds=bogus", "/v1/search?kinds=task,agent", "/v1/search?offset=-1", "/v1/search?offset=x",
		"/v1/search?limit=0", "/v1/search?before=yesterday",
		"/v1/show", "/v1/show?ref=nope", "/v1/show?ref=task:x&offset=-2", "/v1/show?ref=task:x&limit=0",
		"/v1/overview",
	} {
		wantStatus(t, f.port, p, http.StatusBadRequest)
	}
	for _, p := range []string{"/v1/tasks/nope", "/v1/tasks/", "/v1/tasks/a/b", "/v1/show?ref=task:nope"} {
		wantStatus(t, f.port, p, http.StatusNotFound)
	}
}

func cardIDs(cs []coord.Card) []string {
	out := []string{}
	for _, c := range cs {
		out = append(out, c.ID)
	}
	return out
}

func TestBoardView(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	tests := []struct {
		name               string
		id                 Identity
		needsYou, working  []string
		blocked, openCards []string
	}{
		// The person: an assigned agent's task is someone's work; a blocker
		// on no one in particular isn't the person's either.
		{"person", asPerson, []string{}, []string{f.assigned.ID}, []string{f.blocked.ID}, []string{f.open.ID}},
		// The assignee sees its assignment in NeedsYou.
		{"assignee", f.asClaude(), []string{f.assigned.ID}, []string{}, []string{f.blocked.ID}, []string{f.open.ID}},
		{"other agent", asMember, []string{}, []string{f.assigned.ID}, []string{f.blocked.ID}, []string{f.open.ID}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			v, err := f.client.Board(ctx, tc.id, coord.BoardFilter{RepoKey: boardRepo})
			if err != nil {
				t.Fatal(err)
			}
			for _, g := range []struct {
				name      string
				got, want []string
			}{
				{"needsYou", cardIDs(v.NeedsYou), tc.needsYou},
				{"working", cardIDs(v.Working), tc.working},
				{"blocked", cardIDs(v.Blocked), tc.blocked},
				{"open", cardIDs(v.Open), tc.openCards},
			} {
				if !slices.Equal(g.got, g.want) {
					t.Errorf("%s = %v, want %v", g.name, g.got, g.want)
				}
			}
		})
	}

	// Other repo: nothing. Lists are [] in the JSON, never null.
	code, body := f.get(t, "/v1/board?repo=elsewhere", nil)
	if code != http.StatusOK {
		t.Fatalf("board: %d %s", code, body)
	}
	for _, k := range []string{`"needsYou":[]`, `"blocked":[]`, `"working":[]`, `"open":[]`, `"done":[]`, `"leases":[]`} {
		if !strings.Contains(string(body), k) {
			t.Errorf("empty board missing %s: %s", k, body)
		}
	}

	// Done is capped by done=, DoneTotal counts them all.
	for _, tk := range []coord.Task{f.open, f.assigned} {
		if _, err := f.b.co.SetTaskState(ctx, personKey, tk.ID, coord.Completed, "done"); err != nil {
			t.Fatal(err)
		}
	}
	v, err := f.client.Board(ctx, asPerson, coord.BoardFilter{DoneLimit: 1})
	if err != nil {
		t.Fatal(err)
	}
	if len(v.Done) != 1 || v.DoneTotal != 2 {
		t.Errorf("done = %v (total %d), want 1 of 2", cardIDs(v.Done), v.DoneTotal)
	}
}

func TestBoardTask(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	for _, id := range []Identity{asPerson, f.asClaude(), asMember} {
		d, err := f.client.Task(ctx, id, f.assigned.ID)
		if err != nil {
			t.Fatal(err)
		}
		if d.Task.ID != f.assigned.ID || d.Task.Title != "wire the board routes" || len(d.Comments) != 1 || d.Comments[0].ID != f.comment.ID {
			t.Errorf("task detail = %+v", d)
		}
	}
	code, body := f.get(t, "/v1/tasks/"+f.open.ID, nil)
	if code != http.StatusOK || !strings.Contains(string(body), `"comments":[]`) {
		t.Errorf("no comments should be []: %d %s", code, body)
	}
	if _, err := f.client.Task(ctx, asPerson, "nope"); statusOf(err) != http.StatusNotFound {
		t.Errorf("unknown task: %v, want 404", err)
	}
}

func hitRefs(hs []search.Hit) []string {
	out := []string{}
	for _, h := range hs {
		out = append(out, h.Ref)
	}
	return out
}

func TestBoardSearchViewerScoping(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	mango := search.Ref(search.KindMsg, f.msgToGrok)
	tests := []struct {
		name string
		id   Identity
		want []string
	}{
		{"person sees every message", asPerson, []string{mango}},
		{"recipient", asMember, []string{mango}},
		{"another agent", f.asClaude(), []string{}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			res, err := f.client.Search(ctx, tc.id, search.Query{Text: "mango"})
			if err != nil {
				t.Fatal(err)
			}
			if got := hitRefs(res.Hits); !slices.Equal(got, tc.want) || res.More {
				t.Errorf("hits = %v more %v, want %v", got, res.More, tc.want)
			}
		})
	}
}

func TestBoardSearchFiltersAndPaging(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	task := func(tk coord.Task) string { return search.Ref(search.KindTask, tk.ID) }

	res, err := f.client.Search(ctx, asPerson, search.Query{Text: "pineapple"})
	if err != nil {
		t.Fatal(err)
	}
	if got := hitRefs(res.Hits); !slices.Equal(got, []string{search.Ref(search.KindComment, f.comment.ID)}) {
		t.Errorf("comment search = %v", got)
	}

	// kinds: repeated and comma-separated both narrow.
	for _, p := range []string{"/v1/search?kinds=task", "/v1/search?kinds=task,comment&kinds=task", "/v1/search?kinds=TASK"} {
		var r SearchResult
		getJSON(t, f.port, p, &r)
		for _, h := range r.Hits {
			if h.Kind != search.KindTask && !(strings.Contains(p, "comment") && h.Kind == search.KindComment) {
				t.Errorf("%s: hit %s of kind %s", p, h.Ref, h.Kind)
			}
		}
		if len(r.Hits) < 3 {
			t.Errorf("%s: %d hits, want the 3 tasks", p, len(r.Hits))
		}
	}

	// Paging: limit 2 over the three tasks, then the rest at next.
	q := search.Query{Kinds: []search.Kind{search.KindTask}, Limit: 2}
	p1, err := f.client.Search(ctx, asPerson, q)
	if err != nil {
		t.Fatal(err)
	}
	if len(p1.Hits) != 2 || !p1.More || p1.Next != 2 {
		t.Fatalf("page 1 = %v more %v next %d", hitRefs(p1.Hits), p1.More, p1.Next)
	}
	q.Offset = p1.Next
	p2, err := f.client.Search(ctx, asPerson, q)
	if err != nil {
		t.Fatal(err)
	}
	if len(p2.Hits) != 1 || p2.More || p2.Next != 0 {
		t.Fatalf("page 2 = %v more %v next %d", hitRefs(p2.Hits), p2.More, p2.Next)
	}
	all := append(hitRefs(p1.Hits), hitRefs(p2.Hits)...)
	slices.Sort(all)
	want := []string{task(f.assigned), task(f.blocked), task(f.open)}
	slices.Sort(want)
	if !slices.Equal(all, want) {
		t.Errorf("pages = %v, want %v", all, want)
	}

	// repo and before filter.
	if r, err := f.client.Search(ctx, asPerson, search.Query{Kinds: []search.Kind{search.KindTask}, RepoKey: "elsewhere"}); err != nil || len(r.Hits) != 0 {
		t.Errorf("other repo = %v, %v", r.Hits, err)
	}
	if r, err := f.client.Search(ctx, asPerson, search.Query{Before: time.Now().Add(-time.Hour)}); err != nil || len(r.Hits) != 0 {
		t.Errorf("before an hour ago = %v, %v", r.Hits, err)
	}
	// limit is capped, not refused.
	var r SearchResult
	getJSON(t, f.port, "/v1/search?limit=100000", &r)
}

func TestBoardShowViewerScoping(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	mango, kiwi := search.Ref(search.KindMsg, f.msgToGrok), search.Ref(search.KindMsg, f.msgToClaude)
	tests := []struct {
		name string
		id   Identity
		ref  string
		want int
	}{
		{"person reads gamma's", asPerson, mango, http.StatusOK},
		{"gamma reads its own", asMember, mango, http.StatusOK},
		{"alpha can't read gamma's", f.asClaude(), mango, http.StatusNotFound},
		{"alpha reads its own", f.asClaude(), kiwi, http.StatusOK},
		{"gamma can't read alpha's", asMember, kiwi, http.StatusNotFound},
		{"tasks have no readers", asMember, search.Ref(search.KindTask, f.assigned.ID), http.StatusOK},
		{"unknown", asPerson, "msg:nope", http.StatusNotFound},
		{"bad ref", asPerson, "nope", http.StatusBadRequest},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			sh, err := f.client.Show(ctx, tc.id, tc.ref, search.ShowOptions{})
			if got := statusOf(err); got != tc.want {
				t.Fatalf("status %d (%v), want %d", got, err, tc.want)
			}
			if tc.want == http.StatusOK && (sh.Ref != tc.ref || sh.Text == "") {
				t.Errorf("shown = %+v", sh)
			}
		})
	}
	// The secret never leaks in a 404's body.
	code, body := f.get(t, "/v1/show?ref="+mango, map[string]string{attach.HeaderAttach: f.attach})
	if code != http.StatusNotFound || strings.Contains(string(body), "mango") {
		t.Errorf("hidden show: %d %s", code, body)
	}
}

func TestBoardShowPaging(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	ref := search.Ref(search.KindTask, f.assigned.ID)
	full, err := f.client.Show(ctx, asPerson, ref, search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	p1, err := f.client.Show(ctx, asPerson, ref, search.ShowOptions{Limit: 10})
	if err != nil {
		t.Fatal(err)
	}
	if !p1.More || p1.Next == 0 || len(p1.Text) > 10 {
		t.Fatalf("page 1 = %+v", p1)
	}
	p2, err := f.client.Show(ctx, asPerson, ref, search.ShowOptions{Offset: p1.Next, Limit: search.MaxShow})
	if err != nil {
		t.Fatal(err)
	}
	if p1.Text+p2.Text != full.Text || p2.More {
		t.Errorf("pages don't rebuild the text: %q + %q vs %q", p1.Text, p2.Text, full.Text)
	}
}

func TestBoardOverview(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	lineRefs := func(ls []capture.Line) []string {
		out := []string{}
		for _, l := range ls {
			out = append(out, l.Ref)
		}
		return out
	}

	// The attached agent's own overview, with or without agent=.
	for _, agent := range []string{"", claudeKey} {
		ov, err := f.client.Overview(ctx, f.asClaude(), agent)
		if err != nil {
			t.Fatal(err)
		}
		if ov.For != claudeKey || ov.RepoKey != boardRepo || ov.InboxOpen != 1 {
			t.Errorf("overview = for %q repo %q open %d", ov.For, ov.RepoKey, ov.InboxOpen)
		}
		if got := lineRefs(ov.Inbox); !slices.Equal(got, []string{search.Ref(search.KindMsg, f.msgToClaude)}) {
			t.Errorf("inbox lines = %v", got)
		}
		if got := lineRefs(ov.Blockers); !slices.Equal(got, []string{search.Ref(search.KindTask, f.blocked.ID)}) {
			t.Errorf("blockers = %v", got)
		}
		// Its own task first.
		if got := lineRefs(ov.Tasks); len(got) != 2 || got[0] != search.Ref(search.KindTask, f.assigned.ID) {
			t.Errorf("tasks = %v", got)
		}
		if len(ov.Agents) != 2 || ov.Agents[0].Key != claudeKey || ov.Hint == "" {
			t.Errorf("agents = %+v hint %q", ov.Agents, ov.Hint)
		}
	}

	// The person picks the agent; gamma's inbox is gamma's.
	ov, err := f.client.Overview(ctx, asPerson, grokKey)
	if err != nil {
		t.Fatal(err)
	}
	if ov.For != grokKey || !slices.Equal(lineRefs(ov.Inbox), []string{search.Ref(search.KindMsg, f.msgToGrok)}) {
		t.Errorf("person's view of gamma = for %q inbox %v", ov.For, lineRefs(ov.Inbox))
	}

	// An agent can't read another's overview (it carries that inbox).
	if _, err := f.client.Overview(ctx, f.asClaude(), grokKey); statusOf(err) != http.StatusForbidden {
		t.Errorf("alpha reading gamma's overview: %v, want 403", err)
	}
	if _, err := f.client.Overview(ctx, asPerson, ""); statusOf(err) != http.StatusBadRequest {
		t.Errorf("person without agent: %v, want 400", err)
	}
}

// The person's board is the GUI's: For is "user:<device>", so a task the
// person holds and one blocked on them are in their NeedsYou; one blocked
// on another person is not.
func TestBoardPersonNeedsYou(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	mine, err := f.b.co.CreateTask(ctx, claudeKey, coord.NewTask{Title: "review this", RepoKey: boardRepo})
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err = f.b.co.Assign(ctx, claudeKey, mine.ID, personKey); err != nil {
		t.Fatal(err)
	}
	onMe, err := f.b.co.CreateTask(ctx, claudeKey, coord.NewTask{Title: "need a key", RepoKey: boardRepo,
		BlockReason: "an API key", BlockedBy: &coord.BlockedBy{Kind: coord.BlockedOnPerson, Ref: personKey}})
	if err != nil {
		t.Fatal(err)
	}
	other, err := f.b.co.CreateTask(ctx, claudeKey, coord.NewTask{Title: "ask a teammate", RepoKey: boardRepo,
		BlockReason: "their call", BlockedBy: &coord.BlockedBy{Kind: coord.BlockedOnPerson, Ref: "user:dev2"}})
	if err != nil {
		t.Fatal(err)
	}
	v, err := f.client.Board(ctx, asPerson, coord.BoardFilter{RepoKey: boardRepo})
	if err != nil {
		t.Fatal(err)
	}
	needs := cardIDs(v.NeedsYou)
	if !slices.Contains(needs, mine.ID) || !slices.Contains(needs, onMe.ID) || slices.Contains(needs, other.ID) {
		t.Errorf("person's NeedsYou = %v, want %s and %s but not %s", needs, mine.ID, onMe.ID, other.ID)
	}
	if !slices.Contains(cardIDs(v.Blocked), other.ID) {
		t.Errorf("blocked = %v, want %s", cardIDs(v.Blocked), other.ID)
	}
}
