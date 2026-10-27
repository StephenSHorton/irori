package main

import (
	"context"
	"errors"
	"path/filepath"
	"slices"
	"sync/atomic"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/store"
)

func testCoord(t *testing.T) *coord.Coord {
	t.Helper()
	ctx := context.Background()
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev-a", hlc.New(time.Now))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	c, err := coord.New(ctx, st, nil, coord.Options{})
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// routed is one notice the service handed to the router.
type routed struct {
	from inbox.Sender
	n    coord.Notice
}

// testBoard is a BoardService over a fresh coord whose router records what
// it is given and fails with fail.
func testBoard(t *testing.T, fail error) (*BoardService, *coord.Coord, *[]routed) {
	t.Helper()
	c := testCoord(t)
	var got []routed
	b := &BoardService{
		coord: c,
		user:  inbox.Sender{Key: "user:dev-a", Name: "you", Kind: "user"},
		route: func(_ context.Context, from inbox.Sender, ns ...coord.Notice) error {
			for _, n := range ns {
				got = append(got, routed{from, n})
			}
			return fail
		},
	}
	return b, c, &got
}

const agentA, agentB = "dev-a/agent-a", "dev-a/agent-b"

// heldTask is a task agentA created and claimed, so comments reach it.
func heldTask(t *testing.T, c *coord.Coord) coord.Task {
	t.Helper()
	ctx := context.Background()
	tk, err := c.CreateTask(ctx, agentA, coord.NewTask{Title: "ship it"})
	if err != nil {
		t.Fatal(err)
	}
	if tk, err = c.ClaimTask(ctx, agentA, tk.ID); err != nil {
		t.Fatal(err)
	}
	return tk
}

// A person's comment reaches the holder at its next turn by default, wakes
// it only when asked, and with "none" stays on the card (decision 18).
func TestBoardCommentTell(t *testing.T) {
	cases := []struct {
		tell     string
		wantMode string // "" = nothing routed
		wantWake bool
	}{
		{"", "next_turn", false},
		{"next_turn", "next_turn", false},
		{"wake", "wake", true},
		{"none", "", false},
	}
	for _, tc := range cases {
		t.Run("tell="+tc.tell, func(t *testing.T) {
			b, c, got := testBoard(t, nil)
			tk := heldTask(t, c)
			res, err := b.Comment(tk.ID, "use the date from the tag", tc.tell)
			if err != nil {
				t.Fatal(err)
			}
			if res.Comment.Wake != tc.wantWake || res.Comment.By != "user:dev-a" {
				t.Fatalf("comment = %+v", res.Comment)
			}
			if res.Told.Mode != tc.wantMode {
				t.Fatalf("told mode = %q, want %q", res.Told.Mode, tc.wantMode)
			}
			if tc.wantMode == "" {
				if len(*got) != 0 || len(res.Told.To) != 0 {
					t.Fatalf("routed %+v / told %+v, want nothing", *got, res.Told)
				}
			} else {
				if len(*got) != 1 {
					t.Fatalf("routed %d notices, want 1", len(*got))
				}
				r := (*got)[0]
				if r.from.Key != "user:dev-a" || !slices.Equal(r.n.To, []string{agentA}) || r.n.Wake != tc.wantWake || r.n.Kind != coord.NoticeComment {
					t.Fatalf("routed %+v", r)
				}
				if !slices.Equal(res.Told.To, []string{agentA}) {
					t.Fatalf("told = %+v", res.Told)
				}
			}
			d, err := b.Task(tk.ID)
			if err != nil || len(d.Comments) != 1 || d.Comments[0].Text != "use the date from the tag" {
				t.Fatalf("detail = %+v, %v", d, err)
			}
		})
	}
}

func TestBoardCommentBadTell(t *testing.T) {
	b, c, got := testBoard(t, nil)
	tk := heldTask(t, c)
	if _, err := b.Comment(tk.ID, "hi", "shout"); err == nil {
		t.Fatal("unknown tell accepted")
	}
	if d, _ := b.Task(tk.ID); len(d.Comments) != 0 || len(*got) != 0 {
		t.Fatal("a refused comment was written or routed")
	}
}

// A comment on a card only the person is on tells nobody.
func TestBoardCommentNobody(t *testing.T) {
	b, _, got := testBoard(t, nil)
	mine, err := b.Create(NewCard{Title: "mine"})
	if err != nil {
		t.Fatal(err)
	}
	res, err := b.Comment(mine.Task.ID, "note to self", "wake")
	if err != nil || res.Told.Mode != "" || len(*got) != 0 {
		t.Fatalf("res %+v, err %v, routed %+v: want nobody told", res, err, *got)
	}
}

// A router failure (a session that has gone) leaves the comment written
// and shows in Told.
func TestBoardCommentRouteError(t *testing.T) {
	b, c, _ := testBoard(t, errors.New("agent gone"))
	tk := heldTask(t, c)
	res, err := b.Comment(tk.ID, "still there?", "")
	if err != nil {
		t.Fatalf("route failure failed the comment: %v", err)
	}
	if res.Told.Error == "" {
		t.Fatal("route failure not reported in Told")
	}
	if d, _ := b.Task(tk.ID); len(d.Comments) != 1 {
		t.Fatal("comment missing after a route failure")
	}
}

// The person's other actions: create (task, assigned task, blocker), assign,
// block/unblock, links, done, and the lanes they land in.
func TestBoardActions(t *testing.T) {
	b, _, got := testBoard(t, nil)
	in := func(cs []coord.Card, id string) bool {
		return slices.ContainsFunc(cs, func(c coord.Card) bool { return c.ID == id })
	}

	open, err := b.Create(NewCard{Title: "pick me up", Shared: true, Links: []string{"https://github.com/o/r/issues/57", " "}})
	if err != nil {
		t.Fatal(err)
	}
	if len(open.Task.Links) != 1 || open.Task.Links[0].Label != "GitHub #57" || !open.Task.Shared {
		t.Fatalf("open task = %+v", open.Task)
	}

	assigned, err := b.Create(NewCard{Title: "for agent a", AssignTo: agentA})
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(assigned.Told.To, []string{agentA}) || assigned.Told.Mode != "next_turn" {
		t.Fatalf("assign told = %+v", assigned.Told)
	}
	if n := (*got)[len(*got)-1].n; n.Kind != coord.NoticeAssigned || n.Wake {
		t.Fatalf("assign notice = %+v", n)
	}

	blocker, err := b.Create(NewCard{Title: "port 8799 held", BlockReason: "a dead dev server holds it", BlockedOn: coord.BlockedOnAgent, BlockedRef: agentB})
	if err != nil {
		t.Fatal(err)
	}
	if blocker.Task.State != coord.Blocked || blocker.Task.BlockedBy == nil || blocker.Task.BlockedBy.Ref != agentB {
		t.Fatalf("blocker = %+v", blocker.Task)
	}

	v := b.Board("", 0)
	if !in(v.Open, open.Task.ID) || !in(v.Working, assigned.Task.ID) || !in(v.Blocked, blocker.Task.ID) {
		t.Fatalf("lanes = %+v", v)
	}

	// Assign the open one to agent b, then block it on the person and
	// unblock it.
	if r, err := b.Assign(open.Task.ID, agentB); err != nil || !slices.Equal(r.Told.To, []string{agentB}) {
		t.Fatalf("assign: %+v, %v", r, err)
	}
	if tk, err := b.Block(open.Task.ID, "needs your call on the name", coord.BlockedOnPerson, ""); err != nil || tk.State != coord.Blocked {
		t.Fatalf("block: %+v, %v", tk, err)
	}
	if v := b.Board("", 0); !in(v.NeedsYou, open.Task.ID) {
		t.Fatalf("a task blocked on the person is not in Needs you: %+v", v.NeedsYou)
	}
	if r, err := b.Unblock(open.Task.ID, "Replay"); err != nil || r.Task.State == coord.Blocked {
		t.Fatalf("unblock: %+v, %v", r, err)
	}

	if tk, err := b.AddLink(open.Task.ID, "docs/mp-test.md"); err != nil || len(tk.Links) != 2 {
		t.Fatalf("add link: %+v, %v", tk.Links, err)
	}
	if tk, err := b.RemoveLink(open.Task.ID, "https://github.com/o/r/issues/57"); err != nil || len(tk.Links) != 1 {
		t.Fatalf("remove link: %+v, %v", tk.Links, err)
	}

	if _, err := b.SetDone(open.Task.ID, false); err != nil {
		t.Fatal(err)
	}
	if _, err := b.SetDone(assigned.Task.ID, true); err != nil {
		t.Fatal(err)
	}
	v = b.Board("", 0)
	if v.DoneTotal != 2 || !in(v.Done, open.Task.ID) || !in(v.Done, assigned.Task.ID) {
		t.Fatalf("done = %d %+v", v.DoneTotal, v.Done)
	}
	if v.NeedsYou == nil || v.Leases == nil || v.Open == nil {
		t.Fatal("Board returned nil lanes; the webview wants []")
	}
}

// A coord change reaches board:changed without a poll, and a burst is
// coalesced into a few events.
func TestBoardPumpForwards(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c := testCoord(t)

	var emits atomic.Int32
	emitted := make(chan struct{}, 100)
	done := make(chan struct{})
	changes := c.Subscribe(ctx)
	go func() {
		defer close(done)
		pumpBoard(ctx, changes, 100*time.Millisecond, func() {
			emits.Add(1)
			emitted <- struct{}{}
		})
	}()
	wait := func(what string) {
		t.Helper()
		select {
		case <-emitted:
		case <-time.After(2 * time.Second):
			t.Fatalf("no board:changed within 2s of %s", what)
		}
	}

	if _, err := c.Acquire(ctx, "dev-a/agent", "gpu/qwen", 0, false); err != nil {
		t.Fatal(err)
	}
	wait("an agent's acquire")

	for i := 0; i < 20; i++ {
		if _, err := c.CreateTask(ctx, "dev-a/agent", coord.NewTask{Title: "t"}); err != nil {
			t.Fatal(err)
		}
	}
	wait("a burst of creates")
	time.Sleep(300 * time.Millisecond) // let any trailing emit land
	if n := emits.Load(); n > 10 {
		t.Fatalf("%d emits for 21 changes, want them coalesced", n)
	}

	cancel()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("pump did not stop on cancel")
	}
}
