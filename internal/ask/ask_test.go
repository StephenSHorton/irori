package ask

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

const (
	agent1 = "dev1/aaaaaaaa-1111-4111-8111-111111111111"
	agent2 = "dev2/bbbbbbbb-2222-4222-8222-222222222222"
)

// rig is dev1's store and Book; other devices' rows are inserted into it.
type rig struct {
	t    *testing.T
	ctx  context.Context
	st   *store.Store
	book *Book
	seq  map[string]int64

	mu       sync.Mutex
	resolved []Ask
	revived  []string
}

// owners is dev1's person: user:dev1 and user:dev2. user:dev3 is a device
// of someone else's.
func owners(k string) bool { return k == "user:dev1" || k == "user:dev2" }

func newRig(t *testing.T, o Options) *rig {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	g := &rig{t: t, ctx: ctx, st: st, seq: map[string]int64{}}
	o.Store, o.Self = st, "dev1"
	if o.IsOwner == nil {
		o.IsOwner = owners
	}
	o.Revive = func(_ context.Context, target string) error {
		g.mu.Lock()
		defer g.mu.Unlock()
		g.revived = append(g.revived, target)
		return nil
	}
	g.book, err = Start(ctx, o)
	if err != nil {
		t.Fatal(err)
	}
	g.book.OnResolved(func(a Ask) {
		g.mu.Lock()
		defer g.mu.Unlock()
		g.resolved = append(g.resolved, a)
	})
	t.Cleanup(func() {
		cancel()
		<-g.book.Done()
		st.Close()
	})
	return g
}

// row inserts dev's next row with actor.
func (g *rig) row(dev, actor, kind string, payload any) {
	g.t.Helper()
	g.seq[dev]++
	raw, _ := json.Marshal(payload)
	at := hlc.Timestamp(time.Now().UnixMilli())<<16 + hlc.Timestamp(g.seq[dev])
	if err := g.st.Insert(g.ctx, store.Event{DeviceID: dev, Seq: g.seq[dev], HLC: at, Actor: actor, Kind: kind, Payload: raw}); err != nil {
		g.t.Fatal(err)
	}
}

func (g *rig) post(by string) Ask {
	g.t.Helper()
	a, err := g.book.Post(g.ctx, Ask{By: by, Name: "alpha", Text: "Ship it?", Actions: []string{"Yes", "No"}, Until: time.Now().Add(time.Hour)})
	if err != nil {
		g.t.Fatal(err)
	}
	return a
}

func (g *rig) resolvedOnce() []Ask {
	g.mu.Lock()
	defer g.mu.Unlock()
	return append([]Ask(nil), g.resolved...)
}

func eventually(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !ok() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting: %s", what)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func TestPostNeedsTheFields(t *testing.T) {
	g := newRig(t, Options{})
	for _, a := range []Ask{
		{Text: "x", Actions: []string{"Ok"}, Until: time.Now().Add(time.Hour)},
		{By: agent1, Actions: []string{"Ok"}, Until: time.Now().Add(time.Hour)},
		{By: agent1, Text: "x", Until: time.Now().Add(time.Hour)},
		{By: agent1, Text: "x", Actions: []string{"Ok"}},
	} {
		if _, err := g.book.Post(g.ctx, a); err == nil {
			t.Fatalf("Post(%+v) took an incomplete ask", a)
		}
	}
	if len(g.book.Open()) != 0 {
		t.Fatal("an incomplete ask was recorded")
	}
}

// The person answers on this device: the ask resolves with the choice,
// OnResolved runs, and it leaves Open. A second answer changes nothing.
func TestAnswerHereResolves(t *testing.T) {
	g := newRig(t, Options{})
	sub := g.book.Subscribe(g.ctx)
	a := g.post(agent1)
	if a.Device != "dev1" || a.Resolved {
		t.Fatalf("posted %+v", a)
	}
	if open := g.book.Open(); len(open) != 1 || open[0].ID != a.ID {
		t.Fatalf("open = %+v", open)
	}
	select {
	case id := <-sub:
		if id != a.ID {
			t.Fatalf("Subscribe sent %q", id)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("Subscribe sent nothing for the post")
	}
	if err := g.book.Answer(g.ctx, a.ID, "Yes", "user:dev1"); err != nil {
		t.Fatal(err)
	}
	got, _ := g.book.Get(a.ID)
	if !got.Resolved || !got.Answered || got.Choice != "Yes" || got.AnswerRef == "" {
		t.Fatalf("after the answer: %+v", got)
	}
	if r := g.resolvedOnce(); len(r) != 1 || r[0].Choice != "Yes" {
		t.Fatalf("OnResolved got %+v", r)
	}
	if err := g.book.Answer(g.ctx, a.ID, "No", "user:dev1"); err != nil {
		t.Fatal(err)
	}
	if got, _ := g.book.Get(a.ID); got.Choice != "Yes" {
		t.Fatalf("a second answer replaced the first: %+v", got)
	}
	if len(g.book.Open()) != 0 || len(g.resolvedOnce()) != 1 {
		t.Fatal("the ask resolved twice or stayed open")
	}
}

// Only the person's keys answer: a made-up user: key and another person's
// device are refused before anything is written.
func TestAnswerRefusesNonOwners(t *testing.T) {
	g := newRig(t, Options{})
	a := g.post(agent1)
	for _, by := range []string{"user:anything", "user:dev3", agent1, ""} {
		if err := g.book.Answer(g.ctx, a.ID, "Yes", by); !errors.Is(err, ErrNotPerson) {
			t.Fatalf("Answer by %q: err = %v", by, err)
		}
	}
	if err := g.book.Answer(g.ctx, "no-such-ask", "Yes", "user:dev1"); err == nil {
		t.Fatal("answered an ask that doesn't exist")
	}
	if got, _ := g.book.Get(a.ID); got.Resolved {
		t.Fatalf("resolved by a refused answer: %+v", got)
	}
}

// An answer from another device counts when it is the person's (actor
// "user" and an owner key); one with a made-up key, another person's key,
// or a non-user actor is ignored and counted against its device.
func TestRemoteAnswers(t *testing.T) {
	g := newRig(t, Options{})
	a := g.post(agent1)
	g.row("dev3", "user", KindAnswered, answeredRow{AskID: a.ID, Choice: "No", By: "user:dev3"})
	g.row("dev2", "user", KindAnswered, answeredRow{AskID: a.ID, Choice: "No", By: "user:anything"})
	g.row("dev2", "irori", KindAnswered, answeredRow{AskID: a.ID, Choice: "No", By: "user:dev2"})
	eventually(t, "three bad answers ignored", func() bool { return g.book.Ignored() == 3 })
	if by := g.book.IgnoredBy(); by["dev2"] != 2 || by["dev3"] != 1 {
		t.Fatalf("ignored by %v", by)
	}
	if got, _ := g.book.Get(a.ID); got.Resolved {
		t.Fatalf("a bad answer resolved it: %+v", got)
	}

	g.row("dev2", "user", KindAnswered, answeredRow{AskID: a.ID, Choice: "Yes", By: "user:dev2", AtDevice: "dev2"})
	eventually(t, "the person's answer from dev2 resolves it", func() bool {
		got, _ := g.book.Get(a.ID)
		return got.Resolved
	})
	got, _ := g.book.Get(a.ID)
	if got.Choice != "Yes" || got.AnswerRef != "dev2/3" {
		t.Fatalf("resolved %+v", got)
	}
}

// Only the asking device decides: another device's resolved row for this
// device's ask is ignored, and an agent asks only from its own device.
func TestWriterRules(t *testing.T) {
	g := newRig(t, Options{})
	a := g.post(agent1)
	g.row("dev2", "irori", KindResolved, resolvedRow{AskID: a.ID, Choice: "No", Answered: true})
	g.row("dev2", "irori", KindPosted, Ask{ID: "forged", By: agent1, Text: "x", Actions: []string{"Ok"}, Until: time.Now().Add(time.Hour)})
	eventually(t, "both rows ignored", func() bool { return g.book.Ignored() == 2 })
	if got, _ := g.book.Get(a.ID); got.Resolved {
		t.Fatalf("dev2 resolved dev1's ask: %+v", got)
	}
	if _, ok := g.book.Get("forged"); ok {
		t.Fatal("dev2 posted an ask for dev1's agent")
	}

	// dev2's own agent asks: the card shows here, and dev2's resolve clears it.
	g.row("dev2", "irori", KindPosted, Ask{ID: "theirs", By: agent2, Text: "y", Actions: []string{"Ok"}, Until: time.Now().Add(time.Hour)})
	eventually(t, "dev2's ask shows", func() bool { _, ok := g.book.Get("theirs"); return ok })
	if err := g.book.Answer(g.ctx, "theirs", "Ok", "user:dev1"); err != nil {
		t.Fatal(err)
	}
	if got, _ := g.book.Get("theirs"); got.Resolved {
		t.Fatal("this device resolved another device's ask")
	}
	g.row("dev2", "irori", KindResolved, resolvedRow{AskID: "theirs", Choice: "Ok", Answered: true})
	eventually(t, "dev2's resolve clears it", func() bool { got, _ := g.book.Get("theirs"); return got.Resolved })
	if len(g.resolvedOnce()) != 0 {
		t.Fatal("OnResolved ran for another device's ask")
	}
}

// An ask nobody answers resolves unanswered at until.
func TestUnansweredResolvesAtUntil(t *testing.T) {
	var mu sync.Mutex
	now := time.Now()
	clock := func() time.Time { mu.Lock(); defer mu.Unlock(); return now }
	g := newRig(t, Options{Now: clock})
	a, err := g.book.Post(g.ctx, Ask{By: agent1, Text: "x", Actions: []string{"Ok"}, Until: now.Add(time.Minute)})
	if err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	now = now.Add(2 * time.Minute)
	mu.Unlock()
	eventually(t, "the ask runs out", func() bool { got, _ := g.book.Get(a.ID); return got.Resolved })
	got, _ := g.book.Get(a.ID)
	if got.Answered || got.Choice != "" {
		t.Fatalf("ran out as %+v", got)
	}
	if r := g.resolvedOnce(); len(r) != 1 || r[0].Answered {
		t.Fatalf("OnResolved got %+v", r)
	}
}

// A revive asked for from another device posts the host's ask; only a
// Revive answer naming that target and delivery wakes it.
func TestReviveFromAnotherDevice(t *testing.T) {
	g := newRig(t, Options{})
	// Not the person: no ask.
	g.row("dev3", "user", KindReviveRequested, reviveRow{Target: agent1, DeliveryID: "d0", By: "user:dev3"})
	g.row("dev2", "irori", KindReviveRequested, reviveRow{Target: agent1, DeliveryID: "d0", By: "user:dev2"})
	// For a session on dev2: dev1 isn't its host.
	g.row("dev2", "user", KindReviveRequested, reviveRow{Target: agent2, DeliveryID: "d0", By: "user:dev2"})
	g.row("dev2", "user", KindReviveRequested, reviveRow{Target: agent1, DeliveryID: "d1", By: "user:dev2"})
	var ra Ask
	eventually(t, "the revive ask is posted", func() bool {
		open := g.book.Open()
		if len(open) == 1 {
			ra = open[0]
		}
		return len(open) == 1
	})
	if ra.ID != "revive-dev2-3" || ra.Target != agent1 || ra.DeliveryID != "d1" || ra.Device != "dev1" {
		t.Fatalf("revive ask %+v", ra)
	}
	// An answer from dev2 that names another delivery resolves the ask but wakes nothing.
	g.row("dev2", "user", KindAnswered, answeredRow{AskID: ra.ID, Choice: ChoiceRevive, Target: agent1, DeliveryID: "other", By: "user:dev2"})
	eventually(t, "the mismatched answer resolves it", func() bool { got, _ := g.book.Get(ra.ID); return got.Resolved })
	g.mu.Lock()
	n := len(g.revived)
	g.mu.Unlock()
	if n != 0 {
		t.Fatal("a mismatched answer revived the target")
	}

	g.row("dev2", "user", KindReviveRequested, reviveRow{Target: agent1, DeliveryID: "d2", By: "user:dev2"})
	var rb Ask
	eventually(t, "the second revive ask", func() bool {
		open := g.book.Open()
		if len(open) == 1 {
			rb = open[0]
		}
		return len(open) == 1
	})
	if err := g.book.Answer(g.ctx, rb.ID, ChoiceRevive, "user:dev1"); err != nil {
		t.Fatal(err)
	}
	g.mu.Lock()
	revived := append([]string(nil), g.revived...)
	g.mu.Unlock()
	if len(revived) != 1 || revived[0] != agent1 {
		t.Fatalf("revived %v", revived)
	}
	if len(g.resolvedOnce()) != 0 {
		t.Fatal("OnResolved ran for a revive ask")
	}
}

func TestRequestRevive(t *testing.T) {
	g := newRig(t, Options{})
	if err := g.book.RequestRevive(g.ctx, agent2, "d1", "user:anything"); !errors.Is(err, ErrNotPerson) {
		t.Fatalf("made-up key: err = %v", err)
	}
	if err := g.book.RequestRevive(g.ctx, agent1, "d1", "user:dev1"); err == nil {
		t.Fatal("asked another device to revive a session on this one")
	}
	if err := g.book.RequestRevive(g.ctx, agent2, "d1", "user:dev1"); err != nil {
		t.Fatal(err)
	}
	evs, err := g.st.List(g.ctx, store.Filter{Kinds: []string{KindReviveRequested}})
	if err != nil || len(evs) != 1 || evs[0].Actor != "user" {
		t.Fatalf("rows = %+v, %v", evs, err)
	}
	var r reviveRow
	json.Unmarshal(evs[0].Payload, &r)
	if r.Target != agent2 || r.DeliveryID != "d1" || r.By != "user:dev1" || r.AtDevice != "dev1" {
		t.Fatalf("row = %+v", r)
	}
}

// With no IsOwner, only this device's own user key is the person.
func TestDefaultOwnerIsThisDevice(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	b, err := Start(ctx, Options{Store: st, Self: "dev1"})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { cancel(); <-b.Done(); st.Close() }()
	a, err := b.Post(ctx, Ask{By: agent1, Text: "x", Actions: []string{"Ok"}, Until: time.Now().Add(time.Hour)})
	if err != nil {
		t.Fatal(err)
	}
	if err := b.Answer(ctx, a.ID, "Ok", "user:dev2"); !errors.Is(err, ErrNotPerson) {
		t.Fatalf("user:dev2 with no IsOwner: err = %v", err)
	}
	if err := b.Answer(ctx, a.ID, "Ok", "user:dev1"); err != nil {
		t.Fatal(err)
	}
}
