package inbox

import (
	"fmt"
	"reflect"
	"testing"
	"time"
)

// snapshot is everything observable about an inbox, for replay equality.
type snapshot struct {
	List        []Delivery
	For         map[string][]Delivery
	ForAll      map[string][]Delivery
	Policy      Policy
	Outstanding map[string]*Wake
	OutState    map[string]State
	Running     map[string]string
	Aliases     map[string][]string
}

func snap(ib *Inbox, agents ...string) snapshot {
	s := snapshot{
		List: ib.List(10000), For: map[string][]Delivery{}, ForAll: map[string][]Delivery{},
		Policy: ib.Policy(), Outstanding: map[string]*Wake{}, OutState: map[string]State{},
		Running: map[string]string{}, Aliases: map[string][]string{},
	}
	for _, a := range agents {
		s.For[a] = ib.For(a, false, 10000)
		s.ForAll[a] = ib.For(a, true, 10000)
		s.Outstanding[a], s.OutState[a] = ib.OutstandingWake(a)
		s.Running[a] = ib.RunningWake(a)
		s.Aliases[a] = ib.Aliases(a)
	}
	return s
}

func TestReplayReproducesState(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", "repo1"))
	h.ib.SetReachable(keyA, PathMod, true)
	h.clk.advance(time.Second)
	h.ib.SetReachable(keyB, PathMod, true)

	// A bit of everything.
	_, d1 := h.send(claude(keyA, "Front Desk"), to("beta"), "one")
	h.send(claude(keyC, "gamma"), to("beta"), "two")
	w := h.deliverEngage(keyB, "t1")                     // b engaged, turn running
	h.send(claude(keyB, "beta"), to("gamma"), "hop one") // hop 1 from inside the turn
	if _, _, err := h.ib.Reply(bg, claude(keyB, "beta"), d1.ID, "done"); err != nil {
		t.Fatal(err)
	}
	_, held := h.send(grok(keyG, "grok"), to("alpha"), "bypass hold")
	_, dismissed := h.send(grok(keyG, "grok"), to("beta"), "dismiss me")
	if err := h.ib.Dismiss(bg, dismissed.ID, user); err != nil {
		t.Fatal(err)
	}
	_, repo := h.send(claude(keyC, "gamma"), toRepo("repo1"), "repo work")
	h.wake(keyA) // issued, never confirmed: outstanding Queued
	_, unowned := h.send(claude(keyA, "alpha"), toRepo("repo9"), "nobody here")
	_, deferred := h.send(person(), to("gamma"), "from the GUI")
	if err := h.ib.Ack(bg, keyC, deferred.ID); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.Defer(bg, keyC, deferred.ID, 45, "after the build"); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.SetPolicy(bg, Policy{Mode: Everyone, Block: []string{"spam"}, Allow: []string{"grok"}}, user); err != nil {
		t.Fatal(err)
	}
	h.clk.advance(UnengagedLimit + time.Second)
	h.sweep() // repo work moves a -> b; reclaims a's wake first? (one action each)
	h.clk.advance(time.Minute)
	h.sweep()
	wc := h.wake(keyC) // gamma's queued work: delivered, not engaged
	if err := h.ib.MarkDelivered(bg, keyC, wc.Token); err != nil {
		t.Fatal(err)
	}
	h.wake(keyB) // the moved repo work: issued, not confirmed

	agents := []string{keyA, keyB, keyC, keyG, user}
	before := snap(h.ib, agents...)
	states := map[State]bool{}
	for _, d := range before.List {
		states[d.State] = true
	}
	for _, s := range []State{Queued, Held, Delivered, Engaged, Answered, Deferred, Dismissed, Moved} {
		if !states[s] {
			t.Fatalf("scenario has no %s delivery: %v", s, states)
		}
	}
	if before.OutState[keyB] != Queued || before.OutState[keyC] != Delivered {
		t.Fatalf("outstanding states %v", before.OutState)
	}
	if before.Running[keyB] == "" {
		t.Fatal("scenario has no running turn")
	}
	h.reopen()
	after := snap(h.ib, agents...)
	if !reflect.DeepEqual(before, after) {
		for i := range before.List {
			if i < len(after.List) && !reflect.DeepEqual(before.List[i], after.List[i]) {
				t.Errorf("delivery %d:\nbefore %+v\nafter  %+v", i, before.List[i], after.List[i])
			}
		}
		t.Fatalf("replay differs:\nbefore %+v\nafter  %+v", before, after)
	}
	for _, id := range []string{held.ID, unowned.ID, repo.ID} {
		if _, ok := h.ib.Get(id); !ok {
			t.Fatalf("lost %s", id)
		}
	}

	// Behaviour carries on from the replayed state: the wake gate, the
	// running turn's hop, idempotent tokens.
	if m, _ := h.send(claude(keyB, "beta"), to("gamma"), "hop again"); m.Hop != 1 {
		t.Fatalf("hop %d after replay", m.Hop)
	}
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	if h.ib.RunningWake(keyB) != "" {
		t.Fatal("turn didn't end")
	}
}

func TestReplayPagesPastListCap(t *testing.T) {
	h := newHarness(t)
	h.abc()
	const n = 600 // 1200+ rows: more than one List page
	for i := range n {
		from := claude(keyA, "alpha")
		if i%2 == 1 {
			from = person()
		}
		h.send(from, to("beta"), fmt.Sprint("m", i))
		if i%7 == 0 {
			h.clk.advance(DirectWindow) // keep a and b under the rate limit
		}
	}
	before := h.ib.For(keyB, true, 10000)
	h.reopen()
	after := h.ib.For(keyB, true, 10000)
	if len(after) != n || !reflect.DeepEqual(before, after) {
		t.Fatalf("replayed %d of %d", len(after), n)
	}
}
