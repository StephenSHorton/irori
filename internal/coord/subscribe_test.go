package coord

import (
	"context"
	"testing"
	"time"
)

// next reads one change or fails after a couple of seconds.
func next(t *testing.T, ch <-chan Change) Change {
	t.Helper()
	select {
	case x, ok := <-ch:
		if !ok {
			t.Fatal("subscription closed")
		}
		return x
	case <-time.After(2 * time.Second):
		t.Fatal("no change within 2s")
	}
	return Change{}
}

// quiet asserts nothing else arrived.
func quiet(t *testing.T, ch <-chan Change) {
	t.Helper()
	select {
	case x := <-ch:
		t.Fatalf("unexpected change %+v", x)
	case <-time.After(50 * time.Millisecond):
	}
}

func subscribe(t *testing.T, c *Coord) <-chan Change {
	t.Helper()
	sctx, cancel := context.WithCancel(ctx)
	t.Cleanup(cancel)
	return c.Subscribe(sctx)
}

func TestSubscribeMutations(t *testing.T) {
	e := newEnv(t)
	ch := subscribe(t, e.c)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	if got := next(t, ch); got != (Change{Kind: TaskChanged, TaskID: task.ID}) {
		t.Fatalf("create: %+v", got)
	}

	steps := []struct {
		name string
		do   func() error
		want []Change
	}{
		{"claim", func() error { _, err := e.c.ClaimTask(ctx, alice, task.ID); return err },
			[]Change{{Kind: TaskChanged, TaskID: task.ID, Holder: alice}}},
		{"assign", func() error { _, err := e.c.AssignTask(ctx, alice, task.ID, bob); return err },
			[]Change{{Kind: TaskChanged, TaskID: task.ID, Holder: bob}}},
		{"status", func() error { _, err := e.c.SetTaskState(ctx, bob, task.ID, InputRequired, "q"); return err },
			[]Change{{Kind: TaskChanged, TaskID: task.ID, Holder: bob}}},
		{"no-op status", func() error { _, err := e.c.SetTaskState(ctx, bob, task.ID, InputRequired, "q"); return err },
			nil},
		{"acquire", func() error { _, err := e.c.Acquire(ctx, alice, "gpu/qwen", 0, false); return err },
			[]Change{{Kind: LeaseChanged, Resource: "gpu/qwen", Holder: alice}}},
		{"renew", func() error { _, err := e.c.Acquire(ctx, alice, "gpu/qwen", time.Hour, false); return err },
			[]Change{{Kind: LeaseChanged, Resource: "gpu/qwen", Holder: alice}}},
		{"refused", func() error { _, _ = e.c.Acquire(ctx, bob, "gpu/qwen", 0, false); return nil },
			nil},
		{"steal", func() error { _, err := e.c.Acquire(ctx, bob, "gpu/qwen", 0, true); return err },
			[]Change{
				{Kind: LeaseChanged, Resource: "gpu/qwen", Holder: alice, Reason: ReasonStolen},
				{Kind: LeaseChanged, Resource: "gpu/qwen", Holder: bob},
			}},
		{"release", func() error { return e.c.Release(ctx, bob, "gpu/qwen") },
			[]Change{{Kind: LeaseChanged, Resource: "gpu/qwen", Holder: bob, Reason: ReasonReleased}}},
	}
	for _, s := range steps {
		if err := s.do(); err != nil {
			t.Fatalf("%s: %v", s.name, err)
		}
		for _, w := range s.want {
			if got := next(t, ch); got != w {
				t.Fatalf("%s: got %+v, want %+v", s.name, got, w)
			}
		}
		quiet(t, ch)
	}
}

func TestSubscribeSweepExpiryAndGone(t *testing.T) {
	e := newEnv(t)
	ch := subscribe(t, e.c)
	must[Lease](t)(e.c.Acquire(ctx, alice, "gpu/a", time.Minute, false))
	must[Lease](t)(e.c.Acquire(ctx, bob, "gpu/b", time.Hour, false))
	next(t, ch)
	next(t, ch)

	e.clk.add(2 * time.Minute)
	if err := e.c.Sweep(ctx); err != nil {
		t.Fatal(err)
	}
	if got := next(t, ch); got != (Change{Kind: LeaseExpired, Resource: "gpu/a", Holder: alice}) {
		t.Fatalf("expiry: %+v", got)
	}

	e.ros.set(bob, "")
	if err := e.c.Sweep(ctx); err != nil { // starts bob's grace clock
		t.Fatal(err)
	}
	quiet(t, ch)
	e.clk.add(GoneGrace)
	if err := e.c.Sweep(ctx); err != nil {
		t.Fatal(err)
	}
	if got := next(t, ch); got != (Change{Kind: LeaseChanged, Resource: "gpu/b", Holder: bob, Reason: ReasonHolderGone}) {
		t.Fatalf("auto-release: %+v", got)
	}
}

// A lease running out reaches subscribers by itself: nobody calls Sweep or
// Leases, the expiry timer does.
func TestSubscribeExpiryTimer(t *testing.T) {
	e := newEnv(t)
	c, err := New(ctx, e.st, e.ros, Options{Now: time.Now})
	if err != nil {
		t.Fatal(err)
	}
	ch := subscribe(t, c)
	must[Lease](t)(c.Acquire(ctx, alice, "gpu/long", time.Hour, false))
	must[Lease](t)(c.Acquire(ctx, alice, "gpu/short", 100*time.Millisecond, false))
	next(t, ch)
	next(t, ch)
	if got := next(t, ch); got != (Change{Kind: LeaseExpired, Resource: "gpu/short", Holder: alice}) {
		t.Fatalf("got %+v, want gpu/short expired", got)
	}
	c.impl.mu.Lock()
	at := c.impl.expiryAt
	c.impl.mu.Unlock()
	if at.Before(time.Now().Add(50 * time.Minute)) {
		t.Fatalf("timer not re-armed for gpu/long: %v", at)
	}
}

// A reader that falls behind gets a Resync before the next change.
func TestSubscribeSlowReaderResync(t *testing.T) {
	e := newEnv(t)
	ch := subscribe(t, e.c)
	for i := 0; i < subBuffer+5; i++ {
		must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	}
	for i := 0; i < subBuffer; i++ {
		if got := next(t, ch); got.Kind != TaskChanged {
			t.Fatalf("change %d = %+v", i, got)
		}
	}
	quiet(t, ch)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "after"}))
	if got := next(t, ch); got.Kind != Resync {
		t.Fatalf("got %+v, want resync", got)
	}
	if got := next(t, ch); got.TaskID != task.ID {
		t.Fatalf("got %+v, want task %s", got, task.ID)
	}
}

func TestSubscribeStopsOnCancel(t *testing.T) {
	e := newEnv(t)
	sctx, cancel := context.WithCancel(ctx)
	ch := e.c.Subscribe(sctx)
	must[Lease](t)(e.c.Acquire(ctx, alice, "gpu/qwen", time.Hour, false))
	next(t, ch)
	cancel()
	deadline := time.After(2 * time.Second)
	for {
		select {
		case _, ok := <-ch:
			if ok {
				continue
			}
			e.c.impl.mu.Lock()
			n, at := len(e.c.impl.subs), e.c.impl.expiryAt
			e.c.impl.mu.Unlock()
			if n != 0 || !at.IsZero() {
				t.Fatalf("after cancel: %d subscribers, timer at %v", n, at)
			}
			return
		case <-deadline:
			t.Fatal("channel not closed after cancel")
		}
	}
}
