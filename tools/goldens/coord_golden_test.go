// Golden exporter for crates/coord's Board fold (coord.md §6, architecture.md
// §6.2): row-dump replay goldens.
//
// Injected into I:internal/coord by tools/goldens/export.sh with
// `go test -overlay`; runs only when IRORI_GOLDEN_OUT is set. Each scenario
// drives the Go Board through its public calls on the coord_test.go harness
// (fixed clock, dev-a, alice/bob/carol, user:dev-a), plus rows inserted as
// other devices. Then it reopens the coord over the same store, so what is
// dumped is a replay in log order, and records:
//
//   - every task./lease./blocker./take. row with its exact payload bytes
//     (base64), in log order;
//   - the clock's time at the end (the replay's now);
//   - the views as Go marshals them: ListTasks, Leases, Refusals, Board for
//     nobody and for alice, each task's Activity and Comments.
//
// crates/coord/tests/golden.rs folds the same rows with Coord::from_rows and
// compares its views' bytes with these.
package coord

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

type goldenRow struct {
	DeviceID string `json:"deviceId"`
	Seq      int64  `json:"seq"`
	HLC      uint64 `json:"hlc"`
	Actor    string `json:"actor"`
	Kind     string `json:"kind"`
	Payload  []byte `json:"payload"`
}

type goldenCase struct {
	Name     string            `json:"name"`
	Device   string            `json:"device"`
	Now      time.Time         `json:"now"`
	Board    bool              `json:"board,omitempty"`
	Person   string            `json:"person,omitempty"`
	Owners   []string          `json:"owners,omitempty"`
	Rows     []goldenRow       `json:"rows"`
	Tasks    string            `json:"tasks"`
	Leases   string            `json:"leases"`
	Refusals string            `json:"refusals"`
	BoardAll string            `json:"boardAll"`
	BoardFor string            `json:"boardFor"`
	Activity map[string]string `json:"activity"`
	Comments map[string]string `json:"comments"`
}

func goldenJSON(t *testing.T, v any) string {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

// goldenInsert inserts a row as another device, stamped by the env's clock.
type goldenPeer struct {
	e   *env
	dev string
	seq int64
	clk *hlc.Clock
}

func (e *env) peer(dev string) *goldenPeer {
	return &goldenPeer{e: e, dev: dev, clk: hlc.New(e.clk.now)}
}

func (p *goldenPeer) row(actor, kind string, payload any) {
	p.e.t.Helper()
	p.seq++
	raw, err := json.Marshal(payload)
	if err != nil {
		p.e.t.Fatal(err)
	}
	// After every row this store holds, so a peer row folds where it lands.
	last, err := p.e.st.List(ctx, store.Filter{Desc: true, Limit: 1})
	if err != nil {
		p.e.t.Fatal(err)
	}
	if len(last) > 0 {
		p.clk.Observe(last[0].HLC)
	}
	ev := store.Event{DeviceID: p.dev, Seq: p.seq, HLC: p.clk.Now(), Actor: actor, Kind: kind, Payload: raw}
	if err := p.e.st.Insert(ctx, ev); err != nil {
		p.e.t.Fatal(err)
	}
}

func goldenCapture(t *testing.T, name string, e *env, o Options, owners []string) goldenCase {
	t.Helper()
	o.Now = e.clk.now
	c, err := New(ctx, e.st, nil, o)
	if err != nil {
		t.Fatal(err)
	}
	gc := goldenCase{Name: name, Device: e.st.DeviceID(), Now: e.clk.now(), Board: o.Board, Person: o.Person, Owners: owners,
		Activity: map[string]string{}, Comments: map[string]string{}}
	f := store.Filter{Kinds: followKinds, Limit: store.MaxListLimit}
	for {
		evs, err := e.st.List(ctx, f)
		if err != nil {
			t.Fatal(err)
		}
		for _, ev := range evs {
			gc.Rows = append(gc.Rows, goldenRow{DeviceID: ev.DeviceID, Seq: ev.Seq, HLC: uint64(ev.HLC), Actor: ev.Actor, Kind: ev.Kind, Payload: ev.Payload})
		}
		if len(evs) < f.Limit {
			break
		}
		f.After = evs[len(evs)-1].Cursor()
	}
	tasks := c.ListTasks(TaskFilter{})
	gc.Tasks = goldenJSON(t, tasks)
	gc.Leases = goldenJSON(t, c.Leases())
	gc.Refusals = goldenJSON(t, c.Refusals())
	gc.BoardAll = goldenJSON(t, c.Board(BoardFilter{}))
	gc.BoardFor = goldenJSON(t, c.Board(BoardFilter{For: alice}))
	for _, tk := range tasks {
		gc.Activity[tk.ID] = goldenJSON(t, c.Activity(tk.ID))
		gc.Comments[tk.ID] = goldenJSON(t, c.Comments(tk.ID))
	}
	return gc
}

// goldT is the running exporter, for the helpers below (Go passes a
// call's results on only when they are all the arguments).
var goldT *testing.T

func gm[T any](v T, err error) T {
	goldT.Helper()
	if err != nil {
		goldT.Fatal(err)
	}
	return v
}

func gm2[T, U any](v T, _ U, err error) T {
	goldT.Helper()
	if err != nil {
		goldT.Fatal(err)
	}
	return v
}

func gok(err error) {
	goldT.Helper()
	if err != nil {
		goldT.Fatal(err)
	}
}

func TestGoldenCoordReplay(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	goldT = t
	var cases []goldenCase
	tick := func(e *env) { e.clk.add(1500 * time.Millisecond) }

	// tasks: create, claim, assign, join, states, blocked at birth, reopen.
	{
		e := newEnv(t)
		t1 := gm(e.c.CreateTask(ctx, alice, NewTask{Title: " Fix the <login> & flow ", Objective: "Users   can't log in", DoneCriteria: "green CI",
			RepoKey: repoA, Scope: []string{"./src/", "docs/*.md"}, Links: []string{"https://github.com/a/b/issues/7", "README.md"}}))
		tick(e)
		t2 := gm(e.c.CreateTask(ctx, user, NewTask{Title: "Shared work", Shared: true, Steps: []Step{{Text: "one"}, {Text: "two", Doing: true}}}))
		tick(e)
		gm(e.c.ClaimTask(ctx, alice, t1.ID))
		gm2(e.c.Assign(ctx, user, t2.ID, bob))
		gm(e.c.Join(ctx, carol, t2.ID, carol))
		tick(e)
		gm(e.c.SetTaskState(ctx, alice, t1.ID, Completed, "shipped it"))
		gm(e.c.CreateTask(ctx, bob, NewTask{Title: "Port 8080 stuck", BlockReason: "a dead dev server holds it", BlockedBy: &BlockedBy{Kind: BlockedOnExternal, Ref: "pid 4242"}}))
		tick(e)
		gm2(e.c.Reopen(ctx, user, t1.ID))
		gm(e.c.Leave(ctx, carol, t2.ID, carol))
		gm(e.c.SetShared(ctx, user, t2.ID, false))
		gm(e.c.AssignTask(ctx, user, t1.ID, "user:dev-a"))
		cases = append(cases, goldenCapture(t, "tasks", e, Options{}, nil))
	}

	// cards: steps, blockers with options, answers, replies, watch, links.
	{
		e := newEnv(t)
		tk := gm(e.c.CreateTask(ctx, alice, NewTask{Title: "Release v0.6"}))
		gm(e.c.ClaimTask(ctx, alice, tk.ID))
		tk = gm(e.c.SetSteps(ctx, alice, tk.ID, []Step{{Text: "Tag it"}, {Text: "Write notes", Doing: true}, {Text: "Announce"}}))
		done := true
		gm(e.c.SetStep(ctx, alice, tk.ID, tk.Steps[0].ID, StepUpdate{Done: &done}))
		gm(e.c.Watch(ctx, bob, tk.ID))
		tick(e)
		_, b1, err := e.c.Raise(ctx, alice, tk.ID, NewBlocker{Question: "Which channel?", Context: "We can post in two places.",
			Options: []Option{{Answer: "Discord", Why: "most people", Recommended: true}, {Answer: "Blog"}}, Links: []string{"https://example.com/plan"},
			Files: []blob.FileRef{{SHA256: "ab12", Name: "plan.txt", Size: 12, Device: "dev-a"}}})
		gok(err)
		gm(e.c.Recommend(ctx, bob, tk.ID, b1.ID, "Both", "reach"))
		tick(e)
		gm2(e.c.Answer(ctx, user, tk.ID, b1.ID, BlockerAnswer{Pick: 1}))
		gm2(e.c.CommentOn(ctx, alice, tk.ID, CommentInput{Text: "Discord alone?", BlockerID: b1.ID, Missing: []string{"the channel name"}}))
		gm2(e.c.Answer(ctx, user, tk.ID, b1.ID, BlockerAnswer{Text: "Yes, #general"}))
		tick(e)
		gm2(e.c.Answer(ctx, alice, tk.ID, b1.ID, BlockerAnswer{Resolve: true, Text: "posting there now"}))
		gm(e.c.Block(ctx, alice, tk.ID, "waiting on review", &BlockedBy{Kind: BlockedOnAgent, Ref: carol}))
		gm(e.c.Block(ctx, alice, tk.ID, "waiting on a second review", &BlockedBy{Kind: BlockedOnAgent, Ref: carol}))
		tick(e)
		gm2(e.c.Unblock(ctx, carol, tk.ID, "reviewed"))
		gm(e.c.AddLink(ctx, bob, tk.ID, "https://github.com/a/b/pull/12"))
		gm(e.c.AddLink(ctx, bob, tk.ID, "ABCDEF1234"))
		gm(e.c.RemoveLink(ctx, alice, tk.ID, "abcdef1234"))
		gm(e.c.Unwatch(ctx, bob, tk.ID))
		t2 := gm(e.c.CreateTask(ctx, bob, NewTask{Title: "Two questions"}))
		_, q1, err := e.c.Raise(ctx, bob, t2.ID, NewBlocker{Question: "A?"})
		gok(err)
		_, _, err = e.c.Raise(ctx, bob, t2.ID, NewBlocker{Question: "B?", BlockedBy: &BlockedBy{Kind: BlockedOnTask, Ref: tk.ID}})
		gok(err)
		gm2(e.c.Answer(ctx, bob, t2.ID, q1.ID, BlockerAnswer{Resolve: true, Text: "settled"}))
		cases = append(cases, goldenCapture(t, "cards", e, Options{}, nil))
	}

	// leases: acquire, refuse, renew, steal, release, a conflict and a
	// person's steal from another device.
	{
		e := newEnv(t)
		gm(e.c.Acquire(ctx, alice, "path:"+repoA+":src/", time.Hour, false))
		if _, err := e.c.Acquire(ctx, bob, "path:"+repoA+":src/a.go", 0, false); err == nil {
			t.Fatal("bob's overlapping lease was granted")
		}
		if _, err := e.c.Acquire(ctx, bob, "path:"+repoA+":src/a.go", 0, false); err == nil {
			t.Fatal("bob's second try was granted")
		}
		gm(e.c.Acquire(ctx, alice, "port:8080", 0, false))
		tick(e)
		gm(e.c.Acquire(ctx, alice, "port:8080", 2*time.Hour, false))
		gm(e.c.Acquire(ctx, carol, "gpu/qwen", 0, false))
		tk := gm(e.c.CreateTask(ctx, bob, NewTask{Title: "Needs the GPU"}))
		gm(e.c.Block(ctx, bob, tk.ID, "the GPU is busy", &BlockedBy{Kind: BlockedOnLease, Ref: "gpu/qwen"}))
		gm(e.c.Acquire(ctx, bob, "port:8080", 0, true))
		gok(e.c.Release(ctx, carol, "gpu/qwen"))
		gm(e.c.Acquire(ctx, carol, "path:"+repoB+":docs/**", 0, false))
		p := e.peer("dev-b")
		until := e.clk.now().Add(time.Hour)
		p.row("agent:dev-b/dan", KindLeaseAcq, leaseAcquiredRow{Lease: Lease{Resource: "named/one", Holder: "dev-b/dan", Until: until, AcquiredAt: e.clk.now()}})
		p.row("agent:dev-b/dan", KindLeaseAcq, leaseAcquiredRow{Lease: Lease{Resource: "port:9000", Holder: "dev-b/dan", Until: until, AcquiredAt: e.clk.now()}})
		p.row("agent:dev-b/dan", KindLeaseRel, leaseReleasedRow{Resource: "named/one", Holder: "dev-b/dan", Reason: ReasonReleased, By: "dev-b/dan", At: e.clk.now()})
		p.row("user", KindLeaseStolen, leaseStolenRow{Resource: "port:9000", Holder: "dev-b/dan", AcquiredAt: e.clk.now(), By: "user:dev-a", At: e.clk.now()})
		p.row("agent:dev-b/eve", KindLeaseAcq, leaseAcquiredRow{Lease: Lease{Resource: "port:8080", Holder: "dev-b/eve", Until: until, AcquiredAt: e.clk.now()}})
		p.row("agent:dev-b/dan", KindLeaseAcq, leaseAcquiredRow{Lease: Lease{Resource: "path:" + repoA + ":src/x.go", Holder: "dev-a/alice-session", Until: until, AcquiredAt: e.clk.now()}})
		cases = append(cases, goldenCapture(t, "leases", e, Options{}, nil))
	}

	// comments: plain, with files, the person's, mirrored messages.
	{
		e := newEnv(t)
		tk := gm(e.c.CreateTask(ctx, alice, NewTask{Title: "Talk it over"}))
		gm(e.c.ClaimTask(ctx, alice, tk.ID))
		gm2(e.c.AddComment(ctx, bob, tk.ID, "Looks good <3", true))
		gm2(e.c.AddCommentWith(ctx, carol, tk.ID, "Here are the logs", false, []blob.FileRef{{SHA256: "cd34", Name: "log.txt", Size: 99, Mime: "text/plain", Device: "dev-a"}}))
		tick(e)
		gm2(e.c.PersonComment(ctx, user, tk.ID, "Please hurry", false, nil))
		gm(e.c.RecordMessage(ctx, MessageEntry{TaskID: tk.ID, MsgID: "msg_1", From: alice, To: bob, Text: "can you look?", Files: 2}))
		gm(e.c.RecordMessage(ctx, MessageEntry{TaskID: tk.ID, MsgID: "msg_2", From: bob, To: carol, Text: "secret", Held: true}))
		gm(e.c.RecordMessage(ctx, MessageEntry{TaskID: tk.ID, MsgID: "msg_2", From: bob, To: carol, Text: "secret", Fill: true, Writer: "irori"}))
		gm(e.c.RecordMessage(ctx, MessageEntry{TaskID: tk.ID, MsgID: "msg_1", From: alice, To: bob, Text: "can you look?", Gone: true, Writer: "irori"}))
		gm(e.c.SetTaskState(ctx, alice, tk.ID, Completed, ""))
		tick(e)
		gm2(e.c.PersonComment(ctx, user, tk.ID, "One more thing", true, nil))
		cases = append(cases, goldenCapture(t, "comments", e, Options{}, nil))
	}

	// shorts: numbers claimed apart on two devices settle into letters.
	{
		e := newEnv(t)
		a1 := gm(e.c.CreateTask(ctx, alice, NewTask{Title: "first here"}))
		gm(e.c.CreateTask(ctx, alice, NewTask{Title: "second here"}))
		p := e.peer("dev-b")
		p.row("agent:dev-b/dan", KindTaskCreated, taskCreatedRow{TaskID: "01KB0000000000000000000001", Num: 1, Title: "first there", By: "dev-b/dan", At: e.clk.now()})
		p.row("agent:dev-b/dan", KindTaskCreated, taskCreatedRow{TaskID: "01KB0000000000000000000002", Title: "no number", By: "dev-b/dan", At: e.clk.now()})
		p.row("agent:dev-b/dan", KindTaskCreated, taskCreatedRow{TaskID: "01KB0000000000000000000003", Num: 3, Title: "third there", By: "dev-b/dan", At: e.clk.now()})
		p.row("agent:dev-b/dan", KindTaskCreated, taskCreatedRow{TaskID: "01KB0000000000000000000004", Num: 1, Title: "also first", By: "dev-b/dan", At: e.clk.now()})
		gm(e.c.CreateTask(ctx, alice, NewTask{Title: "after the sync"}))
		gm2(e.c.AddComment(ctx, bob, a1.ID, "which one is #1?", false))
		cases = append(cases, goldenCapture(t, "shorts", e, Options{}, nil))
	}

	// subtasks: split moves steps, adoption, rollup, a cascade.
	{
		e := newEnv(t)
		p := gm(e.c.CreateTask(ctx, alice, NewTask{Title: "Big job", Steps: []Step{{Text: "plan", Done: true}, {Text: "build"}, {Text: "ship"}}}))
		gm(e.c.ClaimTask(ctx, alice, p.ID))
		other := gm(e.c.CreateTask(ctx, bob, NewTask{Title: "Loose card"}))
		_, subs, err := e.c.Split(ctx, alice, p.ID, []NewSubtask{{Title: "Build it", Steps: []string{p.Steps[1].ID, "test"}}, {Title: "Ship it", Steps: []string{p.Steps[2].ID}}}, []string{other.ID})
		gok(err)
		gm(e.c.ClaimTask(ctx, bob, subs[0].ID))
		gm(e.c.SetTaskState(ctx, bob, subs[0].ID, Completed, "built"))
		gm(e.c.Block(ctx, user, subs[1].ID, "needs a release window", nil))
		gm(e.c.CreateTask(ctx, carol, NewTask{Title: "Docs too", Parent: p.ShortRef()}))
		p2 := gm(e.c.CreateTask(ctx, user, NewTask{Title: "Small job"}))
		k2 := gm(e.c.CreateTask(ctx, user, NewTask{Title: "Small part", Parent: p2.ID}))
		gm(e.c.SetTaskState(ctx, user, p2.ID, Canceled, "not needed"))
		_ = k2
		cases = append(cases, goldenCapture(t, "subtasks", e, Options{}, nil))
	}

	// pins: pin, move, unpin, and a pin row from another device's person.
	{
		e := newEnv(t)
		var ids []string
		for _, title := range []string{"a", "b", "c", "d"} {
			ids = append(ids, gm(e.c.CreateTask(ctx, alice, NewTask{Title: title})).ID)
		}
		gm(e.c.Pin(ctx, user, ids[0], true))
		gm(e.c.Pin(ctx, user, ids[1], true))
		gm(e.c.Pin(ctx, user, ids[2], true))
		gm(e.c.MovePin(ctx, user, ids[2], ids[0]))
		gm(e.c.MovePin(ctx, user, ids[1], ids[2]))
		gm(e.c.Pin(ctx, user, ids[0], false))
		p := e.peer("dev-b")
		p.row("user", KindTaskPinned, taskPinnedRow{TaskID: ids[3], Pinned: true, Rank: 7.5, By: "user:dev-b", At: e.clk.now()})
		p.row("agent:dev-b/dan", KindTaskPinned, taskPinnedRow{TaskID: ids[0], Pinned: true, Rank: 1, By: "dev-b/dan", At: e.clk.now()})
		cases = append(cases, goldenCapture(t, "pins", e, Options{IsOwner: func(k string) bool { return k == user || k == "user:dev-b" }}, []string{user, "user:dev-b"}))
	}

	// asks: post to the room, claim, close as a card, as an edit, answered,
	// none, and one left unclaimed.
	{
		e := newEnv(t)
		e.ros.set(carol, "busy")
		agents := e.ros.Agents()
		a1 := gm2(e.c.PostAsk(ctx, user, NewAsk{Text: "Can someone look at the flaky test in CI? It fails one run in five on Windows only.", Wake: true}, agents))
		gm(e.c.AskClaim(ctx, alice, a1.ID, time.Second))
		gm2(e.c.AddComment(ctx, alice, a1.ID, "On it", false))
		gm(e.c.AskClose(ctx, alice, a1.ID, AskOutcome{Kind: OutcomeCard, Note: "it is a race", Title: "Fix the flaky test", Steps: []Step{{Text: "find the race"}}}))
		target := gm(e.c.CreateTask(ctx, bob, NewTask{Title: "Docs page", Objective: "old text"}))
		a2 := gm2(e.c.PostAsk(ctx, user, NewAsk{Text: "Add the install steps to the docs page", Files: []blob.FileRef{{SHA256: "ef56", Name: "shot.png", Size: 5, Device: "dev-a"}}}, agents))
		gm(e.c.AskClaim(ctx, bob, a2.ID, time.Second))
		gm(e.c.AskClose(ctx, bob, a2.ID, AskOutcome{Kind: OutcomeEdit, Note: "added them", Task: target.ID, Objective: "install steps"}))
		a3 := gm2(e.c.PostAsk(ctx, user, NewAsk{Text: "What version are we on?"}, agents))
		gm(e.c.AskClaim(ctx, alice, a3.ID, time.Second))
		gm(e.c.AskClose(ctx, alice, a3.ID, AskOutcome{Kind: OutcomeAnswered, Note: "v0.5.0"}))
		a4 := gm2(e.c.PostAsk(ctx, user, NewAsk{Text: "Never mind this one", RepoKey: repoA}, agents))
		gm(e.c.AskClose(ctx, user, a4.ID, AskOutcome{Kind: OutcomeNone}))
		gm2(e.c.PostAsk(ctx, user, NewAsk{Text: "Anyone free?"}, nil))
		cases = append(cases, goldenCapture(t, "asks", e, Options{}, nil))
	}

	// board: a shared board's log, with person: and agent: keys.
	{
		e := newEnv(t)
		const me, other, card = "person:p1", "person:p2", "agent:c1"
		owners := func(k string) bool { return k == me || k == other }
		o := Options{Now: e.clk.now, Board: true, Person: me, IsOwner: owners, Tag: func() string { return "mlg" }}
		c, err := New(ctx, e.st, nil, o)
		gok(err)
		t1 := gm(c.CreateTask(ctx, me, NewTask{Title: "Board card"}))
		gm(c.ClaimTask(ctx, card, t1.ID))
		gm2(c.AddComment(ctx, me, t1.ID, "from the person", false))
		_, _, err = c.Raise(ctx, card, t1.ID, NewBlocker{Question: "Which build?"})
		gok(err)
		t2 := gm(c.CreateTask(ctx, card, NewTask{Title: "Agent card"}))
		gm(c.Withdraw(ctx, me, t2.ID, ""))
		gm(c.CreateTask(ctx, other, NewTask{Title: "Other person's card"}))
		cases = append(cases, goldenCapture(t, "board", e, Options{Board: true, Person: me, IsOwner: owners, Tag: func() string { return "mlg" }}, []string{me, other}))
	}

	path := filepath.Join(dir, "coord", "replay.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(cases); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

var _ = presence.AgentCard{}

type goldenNotice struct {
	Step string `json:"step"`
	Kind string `json:"kind"`
	To   string `json:"to"`
	Wake bool   `json:"wake"`
	Text string `json:"text"`
}

// TestGoldenCoordNotices records the notice texts (coord.md §3.18) a fixed
// script of calls returns. Ids are random, so crates/coord/tests/golden.rs
// runs the same script and compares the texts with every 26-character id
// replaced on both sides; notices are compared as sorted sets per step.
func TestGoldenCoordNotices(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	goldT = t
	var out []goldenNotice
	add := func(step string, ns ...Notice) {
		for _, n := range ns {
			if n.Kind == "" {
				continue
			}
			out = append(out, goldenNotice{Step: step, Kind: string(n.Kind), To: strings.Join(n.To, ","), Wake: n.Wake, Text: n.Text})
		}
	}
	e := newEnv(t)
	t1 := gm(e.c.CreateTask(ctx, alice, NewTask{Title: "Fix \"it\" <now>", Objective: "obj & more", DoneCriteria: "dc"}))
	_, n := gm2pair(e.c.Assign(ctx, alice, t1.ID, bob))
	add("assign", n)
	gm(e.c.ClaimTask(ctx, bob, t1.ID))
	gm(e.c.Block(ctx, bob, t1.ID, "the db", nil))
	_, n = gm2pair(e.c.Unblock(ctx, user, t1.ID, "back"))
	add("unblock", n)
	_, n, err := e.c.AddComment(ctx, carol, t1.ID, "hi <b>", false)
	gok(err)
	add("comment", n)
	_, _, err = e.c.Raise(ctx, bob, t1.ID, NewBlocker{Question: "Which?", Options: []Option{{Answer: "A", Recommended: true}, {Answer: "B"}}})
	gok(err)
	_, n = gm2pair(e.c.Answer(ctx, user, t1.ID, "b2", BlockerAnswer{Pick: 1}))
	add("answer", n)
	_, n = gm2pair(e.c.Answer(ctx, bob, t1.ID, "b2", BlockerAnswer{Resolve: true, Text: "ok"}))
	add("resolve", n)
	gm(e.c.SetTaskState(ctx, bob, t1.ID, Completed, ""))
	_, n = gm2pair(e.c.Reopen(ctx, user, t1.ID))
	add("reopen", n)
	_, ns, err := e.c.PersonComment(ctx, user, t1.ID, "more?", true, nil)
	gok(err)
	add("person comment", ns...)
	gm(e.c.Watch(ctx, carol, t1.ID))
	gm(e.c.SetSteps(ctx, bob, t1.ID, []Step{{Text: "x"}}))
	add("watch digest", gm(e.c.SweepNotices(ctx))...)
	ask, n, err := e.c.PostAsk(ctx, user, NewAsk{Text: "Look at this\nplease", Wake: true}, e.ros.Agents())
	gok(err)
	add("ask", n)
	gm(e.c.AskClaim(ctx, alice, ask.ID, time.Second))
	e.ros.set(alice, "")
	add("ask sweep 1", gm(e.c.SweepAsks(ctx, e.ros.Agents()))...)
	e.clk.add(GoneGrace + time.Second)
	add("ask sweep 2", gm(e.c.SweepAsks(ctx, e.ros.Agents()))...)
	t3 := gm(e.c.CreateTask(ctx, bob, NewTask{Title: "Left behind"}))
	gm(e.c.ClaimTask(ctx, carol, t3.ID))
	e.ros.set(carol, "")
	add("returned 1", gm(e.c.SweepNotices(ctx))...)
	e.clk.add(HolderGrace + time.Second)
	add("returned 2", gm(e.c.SweepNotices(ctx))...)
	path := filepath.Join(dir, "coord", "notices.json")
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(out); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

func gm2pair[T, U any](v T, u U, err error) (T, U) {
	goldT.Helper()
	if err != nil {
		goldT.Fatal(err)
	}
	return v, u
}
