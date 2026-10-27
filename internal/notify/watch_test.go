package notify

import (
	"context"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

const (
	keyA = "dev1/aaaaaaaa-1111-4111-8111-111111111111"
	keyB = "dev1/bbbbbbbb-2222-4222-8222-222222222222"
	user = "user:dev1"
)

type roster struct{ cards []presence.AgentCard }

func (r *roster) Agents() []presence.AgentCard { return r.cards }

func agentCard(key, name string) presence.AgentCard {
	_, sess, _ := strings.Cut(key, "/")
	return presence.AgentCard{Key: key, DeviceID: "dev1", SessionID: sess, Agent: "claude", Name: name, Status: "idle"}
}

type rig struct {
	t      *testing.T
	ib     *inbox.Inbox
	co     *coord.Coord
	fake   *fakeToaster
	n      *Notifier
	opened chan Target
	ctx    context.Context
	st     *store.Store
}

func newRig(t *testing.T) *rig { return newRigNamed(t, nil) }

// newRigNamed is newRig with a Name func wrapped around the usual names
// (wrap may block, to stall the watcher).
func newRigNamed(t *testing.T, wrap func(key, name string) string) *rig {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	r := &roster{cards: []presence.AgentCard{agentCard(keyA, "alpha"), agentCard(keyB, "beta")}}
	ib, err := inbox.New(ctx, st, r, inbox.Options{DeviceID: "dev1"})
	if err != nil {
		t.Fatal(err)
	}
	co, err := coord.New(ctx, st, r, coord.Options{})
	if err != nil {
		t.Fatal(err)
	}
	f := newFake()
	n := New(Config{Toaster: f})
	opened := make(chan Target, 8)
	n.SetOpener(func(tg Target) { opened <- tg })
	names := map[string]string{keyA: "alpha", keyB: "beta"}
	done := make(chan struct{})
	go func() {
		defer close(done)
		name := func(k string) string { return names[k] }
		if wrap != nil {
			name = func(k string) string { return wrap(k, names[k]) }
		}
		Watch(ctx, n, Sources{Inbox: ib, Board: co, User: user, Name: name})
	}()
	t.Cleanup(func() {
		cancel()
		<-done
		st.Close()
	})
	// Watch subscribes before it reads; give it a moment so the first
	// change of a test isn't sent before anyone listens.
	time.Sleep(50 * time.Millisecond)
	return &rig{t: t, ib: ib, co: co, fake: f, n: n, opened: opened, ctx: ctx, st: st}
}

// waitPost waits for a Post of id.
func (r *rig) waitPost(id string) {
	r.t.Helper()
	deadline := time.After(slow * 3 * time.Second)
	for {
		select {
		case got := <-r.fake.posted:
			if got == id {
				return
			}
		case <-deadline:
			r.t.Fatalf("no card %q posted", id)
		}
	}
}

// waitGone waits for id to leave the screen.
func (r *rig) waitGone(id string) {
	r.t.Helper()
	for i := 0; i < 300; i++ {
		if !r.fake.isLive(id) {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	r.t.Fatalf("card %q is still up", id)
}

func (r *rig) noPost(within time.Duration) {
	r.t.Helper()
	select {
	case id := <-r.fake.posted:
		r.t.Fatalf("unexpected card %q", id)
	case <-time.After(within):
	}
}

var alpha = inbox.Sender{Key: keyA, Name: "alpha", Kind: "claude"}

func (r *rig) heldMessage(text string) inbox.Delivery {
	r.t.Helper()
	if err := r.ib.SetPolicy(r.ctx, inbox.Policy{Mode: inbox.Everyone, Block: []string{keyA}}, user); err != nil {
		r.t.Fatal(err)
	}
	_, d, err := r.ib.Send(r.ctx, alpha, inbox.Address{Kind: inbox.ToAgent, Value: keyB}, text, "")
	if err != nil {
		r.t.Fatal(err)
	}
	if d.State != inbox.Held {
		r.t.Fatalf("delivery state %s, want held", d.State)
	}
	return d
}

func TestWatchHeldReleaseActsAsPerson(t *testing.T) {
	r := newRig(t)
	d := r.heldMessage("please run the migration")
	id := HeldID(d.ID)
	r.waitPost(id)
	c := r.fake.card(t, id)
	if c.Title != "alpha sent beta a message Irori held" || !strings.Contains(c.Body, "block list") || !strings.Contains(c.Body, "migration") {
		t.Fatalf("card = %q / %q", c.Title, c.Body)
	}
	if len(c.Actions) != 2 || c.Actions[0].Label != "Release" || c.Actions[1].Label != "Dismiss" {
		t.Fatalf("actions = %+v", c.Actions)
	}
	r.fake.press(t, id, "Release")
	for i := 0; i < 300; i++ {
		if got, _ := r.ib.Get(d.ID); got.State == inbox.Queued {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if got, _ := r.ib.Get(d.ID); got.State != inbox.Queued {
		t.Fatalf("after Release the delivery is %s", got.State)
	}
	// The release row (the newest) is the person's.
	evs, err := r.st.List(r.ctx, store.Filter{Limit: 1, Desc: true})
	if err != nil || len(evs) != 1 {
		t.Fatal(err)
	}
	if evs[0].Actor != "user" || !strings.Contains(string(evs[0].Payload), d.ID) {
		t.Fatalf("release row %s by %q: %s", evs[0].Kind, evs[0].Actor, evs[0].Payload)
	}
}

func TestWatchHeldClearedWhenResolvedElsewhere(t *testing.T) {
	r := newRig(t)
	d := r.heldMessage("one")
	id := HeldID(d.ID)
	r.waitPost(id)
	// Dismissed from the app, not the card: the card comes down.
	if err := r.ib.Dismiss(r.ctx, d.ID, user); err != nil {
		t.Fatal(err)
	}
	r.waitGone(id)

	// The card's Dismiss button acts through the inbox.
	d2 := r.heldMessage("two")
	r.waitPost(HeldID(d2.ID))
	r.fake.press(t, HeldID(d2.ID), "Dismiss")
	for i := 0; i < 300; i++ {
		if got, _ := r.ib.Get(d2.ID); got.State == inbox.Dismissed {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("Dismiss on the card didn't dismiss the delivery")
}

func TestWatchPausedHoldsStayQuiet(t *testing.T) {
	r := newRig(t)
	if err := r.ib.SetPolicy(r.ctx, inbox.Policy{Mode: inbox.Everyone, Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	_, d, err := r.ib.Send(r.ctx, alpha, inbox.Address{Kind: inbox.ToAgent, Value: keyB}, "x", "")
	if err != nil || d.Hold != inbox.HoldPaused {
		t.Fatalf("send: %v %s", err, d.Hold)
	}
	r.noPost(150 * time.Millisecond)
}

func TestWatchMessageToPerson(t *testing.T) {
	r := newRig(t)
	person := inbox.Sender{Key: user, Name: "you", Kind: "user"}
	m, _, err := r.ib.Send(r.ctx, person, inbox.Address{Kind: inbox.ToAgent, Value: keyA}, "how is it going?", "")
	if err != nil {
		t.Fatal(err)
	}
	_, back, err := r.ib.Send(r.ctx, alpha, inbox.Address{Kind: inbox.ToReply, Value: m.ID}, "done, tests pass", "")
	if err != nil {
		t.Fatal(err)
	}
	if back.To != user {
		t.Fatalf("reply went to %q", back.To)
	}
	id := MessageID(back.ID)
	r.waitPost(id)
	c := r.fake.card(t, id)
	if c.Title != "alpha sent you a message" || c.Body != "done, tests pass" || len(c.Actions) != 1 || c.Actions[0].Label != "Open" {
		t.Fatalf("card = %+v", c)
	}
	c.OnClick()
	select {
	case tg := <-r.opened:
		if tg.Tab != TabInbox || tg.Ref != back.ID {
			t.Fatalf("opened %+v", tg)
		}
	case <-time.After(time.Second):
		t.Fatal("body click didn't open the app")
	}
}

func TestWatchBoardNeedsYou(t *testing.T) {
	r := newRig(t)
	task, err := r.co.CreateTask(r.ctx, keyA, coord.NewTask{Title: "ship the release"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.co.ClaimTask(r.ctx, keyA, task.ID); err != nil {
		t.Fatal(err)
	}
	r.noPost(100 * time.Millisecond) // working, not on the person
	if _, err := r.co.Block(r.ctx, keyA, task.ID, "which version number?", &coord.BlockedBy{Kind: coord.BlockedOnPerson}); err != nil {
		t.Fatal(err)
	}
	id := NeedsID(task.ID)
	r.waitPost(id)
	c := r.fake.card(t, id)
	if c.Title != "alpha is blocked on you" || !strings.HasPrefix(c.Body, "which version number?") {
		t.Fatalf("card = %q / %q", c.Title, c.Body)
	}
	if len(c.Actions) != 2 || c.Actions[0].Label != "Answer" || c.Actions[1].Label != "Later" {
		t.Fatalf("actions = %+v", c.Actions)
	}
	r.fake.press(t, id, "Answer")
	select {
	case tg := <-r.opened:
		if tg.Tab != TabBoard || tg.Ref != "task:"+task.ID {
			t.Fatalf("opened %+v", tg)
		}
	case <-time.After(time.Second):
		t.Fatal("Answer didn't open the Board")
	}
	// A comment while it is still blocked doesn't pop it up again.
	if _, _, err := r.co.AddComment(r.ctx, keyB, task.ID, "I think 0.4", false); err != nil {
		t.Fatal(err)
	}
	r.noPost(100 * time.Millisecond)

	// Unblocked: the card (posted again here to see it go) is cleared.
	if _, err := r.co.Block(r.ctx, keyA, task.ID, "still need the number", &coord.BlockedBy{Kind: coord.BlockedOnPerson}); err != nil {
		t.Fatal(err)
	}
	r.noPost(80 * time.Millisecond) // a new reason while still in the lane: no repost
	r.n.Post(Note{ID: id, Title: "up"})
	<-r.fake.posted
	if _, _, err := r.co.Unblock(r.ctx, user, task.ID, "0.4"); err != nil {
		t.Fatal(err)
	}
	r.waitGone(id)

	// Blocked on the person again later: a new card.
	if _, err := r.co.Block(r.ctx, keyA, task.ID, "and the date?", &coord.BlockedBy{Kind: coord.BlockedOnPerson}); err != nil {
		t.Fatal(err)
	}
	r.waitPost(id)
}

func TestWatchSeedsQuietly(t *testing.T) {
	// A card already waiting when the app starts doesn't pop up.
	ctx := context.Background()
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	r := &roster{cards: []presence.AgentCard{agentCard(keyA, "alpha")}}
	co, err := coord.New(ctx, st, r, coord.Options{})
	if err != nil {
		t.Fatal(err)
	}
	task, _ := co.CreateTask(ctx, keyA, coord.NewTask{Title: "t"})
	if _, err := co.Block(ctx, keyA, task.ID, "r", &coord.BlockedBy{Kind: coord.BlockedOnPerson}); err != nil {
		t.Fatal(err)
	}
	f := newFake()
	n := New(Config{Toaster: f})
	wctx, cancel := context.WithCancel(ctx)
	var wg sync.WaitGroup
	wg.Add(1)
	go func() { defer wg.Done(); Watch(wctx, n, Sources{Board: co, User: user}) }()
	time.Sleep(100 * time.Millisecond)
	cancel()
	wg.Wait()
	if f.postCount() != 0 {
		t.Fatal("a card that was already waiting popped up at start")
	}
}

func TestWatchHeadlessReturns(t *testing.T) {
	done := make(chan struct{})
	go func() {
		Watch(context.Background(), New(Config{Headless: true}), Sources{})
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("Watch on a headless notifier should return at once")
	}
}

// The inbox drops changes for a reader whose buffer is full and never says
// so; the watcher re-reads, so a release or dismiss it didn't hear still
// takes the card down, and a hold it didn't hear still gets one.
func TestWatchCatchesUpAfterDroppedChanges(t *testing.T) {
	var stall atomic.Bool
	release := make(chan struct{})
	stalled := make(chan struct{}, 1)
	r := newRigNamed(t, func(key, name string) string {
		if stall.CompareAndSwap(true, false) {
			stalled <- struct{}{}
			<-release
		}
		return name
	})
	d := r.heldMessage("first")
	r.waitPost(HeldID(d.ID))

	stall.Store(true)
	d2 := r.heldMessage("second") // the watcher stalls building its card
	select {
	case <-stalled:
	case <-time.After(slow * 3 * time.Second):
		t.Fatal("the watcher never stalled")
	}
	for i := 0; i < 80; i++ { // more changes than the 64-slot buffer
		if err := r.ib.SetPolicy(r.ctx, inbox.Policy{Mode: inbox.Everyone, Block: []string{keyA}}, user); err != nil {
			t.Fatal(err)
		}
	}
	if err := r.ib.Dismiss(r.ctx, d.ID, user); err != nil {
		t.Fatal(err)
	}
	_, d3, err := r.ib.Send(r.ctx, alpha, inbox.Address{Kind: inbox.ToAgent, Value: keyB}, "third", "")
	if err != nil || d3.State != inbox.Held {
		t.Fatalf("send: %v %s", err, d3.State)
	}
	close(release)

	r.waitGone(HeldID(d.ID))
	for i := 0; i < 300 && !r.fake.isLive(HeldID(d3.ID)); i++ {
		time.Sleep(10 * time.Millisecond)
	}
	if !r.fake.isLive(HeldID(d2.ID)) || !r.fake.isLive(HeldID(d3.ID)) {
		t.Fatalf("held cards: second %v, third %v; want both up", r.fake.isLive(HeldID(d2.ID)), r.fake.isLive(HeldID(d3.ID)))
	}
}

// What the person did on the Board themselves doesn't pop a card at them.
func TestWatchBoardIgnoresThePersonsOwnMoves(t *testing.T) {
	r := newRig(t)
	// (a) The person takes a card an agent made.
	task, err := r.co.CreateTask(r.ctx, keyA, coord.NewTask{Title: "agent work"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.co.ClaimTask(r.ctx, user, task.ID); err != nil {
		t.Fatal(err)
	}
	// (b) The person's own card, blocked on them by them.
	own, err := r.co.CreateTask(r.ctx, user, coord.NewTask{Title: "renew the cert"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.co.Block(r.ctx, user, own.ID, "waiting on vendor", &coord.BlockedBy{Kind: coord.BlockedOnPerson}); err != nil {
		t.Fatal(err)
	}
	if v := r.co.Board(coord.BoardFilter{For: user}); len(v.NeedsYou) < 2 {
		t.Fatalf("both cards should be in Needs you; got %d", len(v.NeedsYou))
	}
	r.noPost(150 * time.Millisecond)

	// An agent handing the person a card still pops one.
	handed, err := r.co.CreateTask(r.ctx, keyA, coord.NewTask{Title: "review the PR"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.co.AssignTask(r.ctx, keyA, handed.ID, user); err != nil {
		t.Fatal(err)
	}
	r.waitPost(NeedsID(handed.ID))
	if c := r.fake.card(t, NeedsID(handed.ID)); c.Title != "alpha handed you a card" {
		t.Fatalf("card = %q", c.Title)
	}
}
