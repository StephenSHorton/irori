package notify

import (
	"context"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	toast "github.com/StephenSHorton/go-toast"
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
	id := HeldID(keyA)
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
	id := HeldID(keyA)
	r.waitPost(id)
	// Dismissed from the app, not the card: the card comes down.
	if err := r.ib.Dismiss(r.ctx, d.ID, user); err != nil {
		t.Fatal(err)
	}
	r.waitGone(id)

	// The card's Dismiss button acts through the inbox.
	d2 := r.heldMessage("two")
	r.waitTitle(id, "alpha sent beta a message Irori held") // the card was down: this is d2's
	r.fake.press(t, id, "Dismiss")
	for i := 0; i < 300; i++ {
		if got, _ := r.ib.Get(d2.ID); got.State == inbox.Dismissed {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("Dismiss on the card didn't dismiss the delivery")
}

// heldPosts are the Posts of id so far, oldest first.
func (f *fakeToaster) heldPosts(id string) []toast.Note {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []toast.Note
	for _, n := range f.posts {
		if n.ID == id {
			out = append(out, n)
		}
	}
	return out
}

// waitTitle waits for the live card id to read title.
func (r *rig) waitTitle(id, title string) toast.Note {
	r.t.Helper()
	for i := 0; i < 300; i++ {
		if r.fake.isLive(id) {
			if c := r.fake.card(r.t, id); c.Title == title {
				return c
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	r.t.Fatalf("card %q never read %q", id, title)
	return toast.Note{}
}

// Held messages coalesce (decision 34): one card per sender, updated in
// place with the count and without a sound while it is up; Release all
// acts on every one; a hold leaving updates the count; another sender
// gets a card of their own.
func TestWatchHeldCoalescesPerSender(t *testing.T) {
	r := newRig(t)
	id := HeldID(keyA)
	d1 := r.heldMessage("one")
	r.waitPost(id)
	d2 := r.heldMessage("two")
	d3 := r.heldMessage("three")
	c := r.waitTitle(id, "alpha sent beta 3 messages Irori held")
	if !strings.Contains(c.Body, "three") || c.Actions[0].Label != "Release all" || c.Actions[1].Label != "Dismiss all" {
		t.Fatalf("card = %q / %+v", c.Body, c.Actions)
	}
	posts := r.fake.heldPosts(id)
	if posts[0].Silent {
		t.Fatal("the card's first post made no sound")
	}
	for _, p := range posts[1:] {
		if !p.Silent {
			t.Fatalf("an update of a card that is up made a sound: %q", p.Title)
		}
	}
	r.fake.mu.Lock()
	for k := range r.fake.live {
		if strings.HasPrefix(k, "held:") && k != id {
			t.Errorf("a second held card %q is up", k)
		}
	}
	r.fake.mu.Unlock()

	// Dismissed in the app: the card counts what is left, quietly.
	if err := r.ib.Dismiss(r.ctx, d1.ID, user); err != nil {
		t.Fatal(err)
	}
	r.waitTitle(id, "alpha sent beta 2 messages Irori held")

	// Another sender's hold is a card of its own.
	if err := r.ib.SetPolicy(r.ctx, inbox.Policy{Mode: inbox.Everyone, Block: []string{keyA, keyB}}, user); err != nil {
		t.Fatal(err)
	}
	beta := inbox.Sender{Key: keyB, Name: "beta", Kind: "claude"}
	if _, _, err := r.ib.Send(r.ctx, beta, inbox.Address{Kind: inbox.ToAgent, Value: keyA}, "from beta", ""); err != nil {
		t.Fatal(err)
	}
	r.waitTitle(HeldID(keyB), "beta sent alpha a message Irori held")

	// Release all releases both of alpha's, and alpha's card goes.
	r.fake.press(t, id, "Release all")
	for i := 0; i < 300; i++ {
		g2, _ := r.ib.Get(d2.ID)
		g3, _ := r.ib.Get(d3.ID)
		if g2.State == inbox.Queued && g3.State == inbox.Queued {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	for _, d := range []inbox.Delivery{d2, d3} {
		if got, _ := r.ib.Get(d.ID); got.State != inbox.Queued {
			t.Fatalf("after Release all %s is %s", d.ID, got.State)
		}
	}
	r.waitGone(id)
	if !r.fake.isLive(HeldID(keyB)) {
		t.Fatal("releasing alpha's took beta's card down")
	}
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
	if c.Title != "alpha is blocked on you (#1)" || !strings.HasPrefix(c.Body, "which version number?") {
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
	id := HeldID(keyA)
	r.waitPost(id)

	stall.Store(true)
	r.heldMessage("second") // the watcher stalls building its card
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

	// alpha's one card counts the second and third, not the dismissed first.
	want := "alpha sent beta 2 messages Irori held"
	for i := 0; i < 300; i++ {
		if r.fake.isLive(id) && r.fake.card(t, id).Title == want {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if c := r.fake.card(t, id); c.Title != want || !strings.Contains(c.Body, "third") {
		t.Fatalf("card = %q / %q", c.Title, c.Body)
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
	if c := r.fake.card(t, NeedsID(handed.ID)); c.Title != "alpha handed you a card (#3)" {
		t.Fatalf("card = %q", c.Title)
	}
}

// With no inbox in the window, a held message is only reachable through its
// card: closing the card with its x doesn't drop the message, and its card
// comes back on the next inbox change, still held.
func TestWatchHeldCardComesBackAfterDismiss(t *testing.T) {
	r := newRig(t)
	d := r.heldMessage("please run the migration")
	id := HeldID(keyA)
	r.waitPost(id)
	c := r.fake.card(t, id)
	if c.OnDismiss == nil {
		t.Fatal("a held card has no dismiss handler")
	}
	r.fake.mu.Lock()
	delete(r.fake.live, id)
	r.fake.mu.Unlock()
	c.OnDismiss()
	if got, _ := r.ib.Get(d.ID); got.State != inbox.Held {
		t.Fatalf("closing the card changed the delivery to %s", got.State)
	}
	time.Sleep(50 * time.Millisecond)
	person := inbox.Sender{Key: user, Name: "you", Kind: "user"}
	if _, _, err := r.ib.Send(r.ctx, person, inbox.Address{Kind: inbox.ToAgent, Value: keyB}, "any change", ""); err != nil {
		t.Fatal(err)
	}
	r.waitTitle(id, "alpha sent beta a message Irori held")
	if p := r.fake.heldPosts(id); !p[len(p)-1].Silent {
		t.Fatal("a held card back after its x made a sound")
	}
}

// A message held, or sent to the person, before the app started gets its
// card at start: nothing else in the window shows it.
func TestWatchPostsWaitingMessagesAtStart(t *testing.T) {
	ctx := context.Background()
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	ro := &roster{cards: []presence.AgentCard{agentCard(keyA, "alpha"), agentCard(keyB, "beta")}}
	ib, err := inbox.New(ctx, st, ro, inbox.Options{DeviceID: "dev1"})
	if err != nil {
		t.Fatal(err)
	}
	if err := ib.SetPolicy(ctx, inbox.Policy{Mode: inbox.Everyone, Block: []string{keyA}}, user); err != nil {
		t.Fatal(err)
	}
	_, held, err := ib.Send(ctx, alpha, inbox.Address{Kind: inbox.ToAgent, Value: keyB}, "held", "")
	if err != nil || held.State != inbox.Held {
		t.Fatalf("held: %v %s", err, held.State)
	}
	person := inbox.Sender{Key: user, Name: "you", Kind: "user"}
	m, _, err := ib.Send(ctx, person, inbox.Address{Kind: inbox.ToAgent, Value: keyB}, "status?", "")
	if err != nil {
		t.Fatal(err)
	}
	beta := inbox.Sender{Key: keyB, Name: "beta", Kind: "claude"}
	_, back, err := ib.Send(ctx, beta, inbox.Address{Kind: inbox.ToReply, Value: m.ID}, "all green", "")
	if err != nil || back.To != user {
		t.Fatalf("reply: %v to %q", err, back.To)
	}
	f := newFake()
	n := New(Config{Toaster: f})
	wctx, cancel := context.WithCancel(ctx)
	var wg sync.WaitGroup
	wg.Add(1)
	go func() { defer wg.Done(); Watch(wctx, n, Sources{Inbox: ib, User: user}) }()
	defer func() { cancel(); wg.Wait() }()
	want := map[string]bool{HeldID(keyA): true, MessageID(back.ID): true}
	deadline := time.After(slow * 3 * time.Second)
	for len(want) > 0 {
		select {
		case id := <-f.posted:
			delete(want, id)
		case <-deadline:
			t.Fatalf("not posted at start: %v", want)
		}
	}
}

// A Release that leaves the message held (a repo with no agent to take it
// yet) puts the card back with the error on it, buttons and all; one that
// fails only because the message was resolved elsewhere is no error.
func TestWatchHeldReleaseFailureKeepsTheCard(t *testing.T) {
	r := newRig(t)
	_, d, err := r.ib.Send(r.ctx, alpha, inbox.Address{Kind: inbox.ToRepo, Value: "acme/nobody"}, "deploy it", "")
	if err != nil {
		t.Fatal(err)
	}
	if d.State != inbox.Held || d.Hold != inbox.HoldUnowned {
		t.Fatalf("delivery %s/%s, want held unowned", d.State, d.Hold)
	}
	id := HeldID(keyA)
	r.waitPost(id)
	r.fake.press(t, id, "Release")
	var c toast.Note
	for i := 0; i < 300; i++ {
		if r.fake.isLive(id) {
			if c = r.fake.card(t, id); strings.Contains(c.Body, "Couldn't release that message") {
				break
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	if !strings.Contains(c.Body, "Couldn't release that message") || !strings.Contains(c.Body, "deploy it") {
		t.Fatalf("after a failed Release the card reads %q", c.Body)
	}
	if len(c.Actions) != 2 || c.Actions[0].Label != "Release" {
		t.Fatalf("the failed card's actions = %+v", c.Actions)
	}
	if p := r.fake.heldPosts(id); !p[len(p)-1].Silent {
		t.Fatal("the failed card made a sound")
	}
	if got, _ := r.ib.Get(d.ID); got.State != inbox.Held {
		t.Fatalf("the delivery is %s", got.State)
	}

	// Dismissed elsewhere while the card is up: its Dismiss is no error,
	// and the card goes.
	if err := r.ib.Dismiss(r.ctx, d.ID, user); err != nil {
		t.Fatal(err)
	}
	r.waitGone(id)
	d2 := r.heldMessage("two")
	r.waitTitle(id, "alpha sent beta a message Irori held")
	c = r.fake.card(t, id)
	if err := r.ib.Release(r.ctx, d2.ID, user); err != nil {
		t.Fatal(err)
	}
	for _, a := range c.Actions {
		if a.Label == "Dismiss" {
			a.Run()
		}
	}
	r.noPostOf(id, 200*time.Millisecond)
}

// noPostOf fails on a Post of id within d.
func (r *rig) noPostOf(id string, d time.Duration) {
	r.t.Helper()
	deadline := time.After(d)
	for {
		select {
		case got := <-r.fake.posted:
			if got == id {
				if c := r.fake.card(r.t, id); strings.Contains(c.Body, "Couldn't") {
					r.t.Fatalf("an error card %q: %q", c.Title, c.Body)
				}
			}
		case <-deadline:
			return
		}
	}
}
