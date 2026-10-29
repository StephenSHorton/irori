package coord

import (
	"testing"
	"time"
)

// sweepReturned runs the sweep and returns its returned notices.
func (e *env) sweepReturned() []Notice {
	e.t.Helper()
	ns, err := e.c.SweepNotices(ctx)
	if err != nil {
		e.t.Fatal(err)
	}
	var out []Notice
	for _, n := range ns {
		if n.Kind == NoticeReturned {
			out = append(out, n)
		}
	}
	return out
}

func (e *env) task(id string) Task {
	e.t.Helper()
	t, ok := e.c.GetTask(id)
	if !ok {
		e.t.Fatalf("no task %s", id)
	}
	return t
}

// A card whose agent went away goes back to open after HolderGrace, and
// the agent that made it hears (card #16).
func TestGoneHolderReturnsTheCardToItsCreator(t *testing.T) {
	e := newEnv(t)
	tk := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Fix the parser"}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, tk.ID))

	e.ros.set(alice, "")
	if got := e.sweepReturned(); len(got) != 0 { // first absent
		t.Fatalf("returned at once: %+v", got)
	}
	e.clk.add(HolderGrace - time.Second)
	e.sweepReturned()
	if got := e.task(tk.ID); got.State != Working || got.Holder != alice {
		t.Fatalf("moved before HolderGrace: %+v", got)
	}
	e.clk.add(2 * time.Second)
	got := e.sweepReturned()
	if len(got) != 1 || len(got[0].To) != 1 || got[0].To[0] != bob {
		t.Fatalf("notices %+v, want one to the creator", got)
	}
	after := e.task(tk.ID)
	if after.State != Submitted || len(after.Holders) != 0 || after.Returned == nil || after.Returned.Was != alice || after.Returned.Person || after.Returned.Row == "" {
		t.Fatalf("after: %+v (returned %+v)", after, after.Returned)
	}
	// Once only.
	e.clk.add(time.Minute)
	if again := e.sweepReturned(); len(again) != 0 {
		t.Fatalf("told again: %+v", again)
	}
	// Someone taking it up clears Returned.
	must[Task](t)(e.c.ClaimTask(ctx, carol, tk.ID))
	if r := e.task(tk.ID).Returned; r != nil {
		t.Fatalf("still returned after a claim: %+v", r)
	}
}

// With the creator gone too, or the person as creator, it comes back to
// the person (a notification), not to an agent.
func TestGoneHolderReturnsTheCardToThePerson(t *testing.T) {
	for _, creator := range []string{user, bob} {
		e := newEnv(t)
		tk := must[Task](t)(e.c.CreateTask(ctx, creator, NewTask{Title: "Ship docs"}))
		must[Task](t)(e.c.ClaimTask(ctx, alice, tk.ID))
		e.ros.set(alice, "")
		e.ros.set(bob, "gone")
		e.sweepReturned()
		e.clk.add(HolderGrace + time.Second)
		if got := e.sweepReturned(); len(got) != 0 {
			t.Fatalf("creator %s: an agent was told: %+v", creator, got)
		}
		if r := e.task(tk.ID).Returned; r == nil || !r.Person {
			t.Fatalf("creator %s: returned %+v, want the person", creator, r)
		}
	}
}

// A blocked card stays blocked (its questions still wait) but loses its
// gone holder; a shared card with someone still on it keeps going.
func TestGoneHolderOnBlockedAndSharedCards(t *testing.T) {
	e := newEnv(t)
	blocked := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Blocked one"}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, blocked.ID))
	must[Task](t)(e.c.Block(ctx, alice, blocked.ID, "which port?", nil))
	shared := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Shared one", Shared: true}))
	must[Task](t)(e.c.Join(ctx, alice, shared.ID, alice))
	must[Task](t)(e.c.Join(ctx, carol, shared.ID, carol))

	e.ros.set(alice, "")
	e.sweepReturned()
	e.clk.add(HolderGrace + time.Second)
	got := e.sweepReturned()
	if len(got) != 1 || got[0].TaskID != blocked.ID {
		t.Fatalf("notices %+v, want one for the blocked card only", got)
	}
	if b := e.task(blocked.ID); b.State != Blocked || len(b.Holders) != 0 {
		t.Fatalf("blocked card: %+v", b)
	}
	s := e.task(shared.ID)
	if s.State != Working || len(s.Holders) != 1 || s.Holders[0] != carol || s.Returned != nil {
		t.Fatalf("shared card: %+v", s)
	}
}

// Only the holder's own device acts: another device's agent, and the
// person, are never taken off.
func TestGoneHolderOnlyOnItsOwnDevice(t *testing.T) {
	e := newEnv(t)
	remote := "dev-z/zed-session"
	a := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Remote"}))
	if _, _, err := e.c.Assign(ctx, bob, a.ID, remote); err != nil {
		t.Fatal(err)
	}
	p := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Person's"}))
	must[Task](t)(e.c.ClaimTask(ctx, user, p.ID))
	e.sweepReturned()
	e.clk.add(HolderGrace + time.Hour)
	if got := e.sweepReturned(); len(got) != 0 {
		t.Fatalf("notices %+v", got)
	}
	if h := e.task(a.ID).Holders; len(h) != 1 || h[0] != remote {
		t.Fatalf("remote holder taken off: %v", h)
	}
	if h := e.task(p.ID).Holders; len(h) != 1 || h[0] != user {
		t.Fatalf("person taken off: %v", h)
	}
}
