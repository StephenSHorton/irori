package home

import (
	"context"
	"errors"
	"sync"
	"testing"

	"github.com/StephenSHorton/irori/internal/awake"
)

// fakeAwake counts holds taken and released, and fails while fail is set.
type fakeAwake struct {
	mu       sync.Mutex
	fail     error
	acquired int
	released int
}

type fakeHold struct {
	f    *fakeAwake
	once sync.Once
}

func (f *fakeAwake) Acquire() (awake.Hold, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.fail != nil {
		return nil, f.fail
	}
	f.acquired++
	return &fakeHold{f: f}, nil
}

func (h *fakeHold) Release() {
	h.once.Do(func() {
		h.f.mu.Lock()
		h.f.released++
		h.f.mu.Unlock()
	})
}

func (f *fakeAwake) counts() (acquired, released int) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.acquired, f.released
}

// The hold is taken when the server starts serving, once however often it
// re-checks, released when the setting goes off and taken again when it
// comes back on, and released when the server stops.
func TestKeepAwakeWhileServing(t *testing.T) {
	n := newNode(t, nil)
	ctx := context.Background()
	want := func(what string, acquired, released int) {
		t.Helper()
		if a, r := n.awake.counts(); a != acquired || r != released {
			t.Fatalf("%s: acquired %d released %d, want %d and %d", what, a, r, acquired, released)
		}
	}
	if !n.srv.Settings().AwakeOn() {
		t.Fatal("keep awake is off by default")
	}
	n.srv.reconcile(ctx)
	want("before the server is switched on", 0, 0)
	if err := n.srv.Set(true, false, ""); err != nil {
		t.Fatal(err)
	}
	n.srv.reconcile(ctx)
	if !n.srv.Status().Running {
		t.Fatal("not running")
	}
	want("started", 1, 0)
	if st := n.srv.Status(); !st.Awake || st.AwakeLine != "Keeping this PC awake" {
		t.Fatalf("status %+v", st)
	}
	n.srv.reconcile(ctx)
	n.srv.reconcile(ctx)
	want("re-checks", 1, 0)

	if err := n.srv.SetAwake(false); err != nil {
		t.Fatal(err)
	}
	n.srv.reconcile(ctx)
	want("setting off", 1, 1)
	if st := n.srv.Status(); !st.Running || st.Awake || st.AwakeLine != "" {
		t.Fatalf("status with the setting off %+v", st)
	}
	if set, err := LoadSettings(n.dir); err != nil || set.AwakeOn() {
		t.Fatalf("saved setting %+v %v", set, err)
	}

	if err := n.srv.SetAwake(true); err != nil {
		t.Fatal(err)
	}
	n.srv.reconcile(ctx)
	want("setting back on", 2, 1)

	n.srv.stop()
	want("stopped", 2, 2)
	n.srv.stop()
	want("stopped twice", 2, 2)
}

// Switching the server off releases the hold through the Run loop.
func TestKeepAwakeReleasedWhenSwitchedOff(t *testing.T) {
	n := newNode(t, nil)
	n.run()
	if err := n.srv.Set(true, false, ""); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the hold", func() bool { a, _ := n.awake.counts(); return a == 1 })
	if err := n.srv.Set(false, false, ""); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the release", func() bool { _, r := n.awake.counts(); return r == 1 })
	if a, _ := n.awake.counts(); a != 1 {
		t.Fatalf("acquired %d times", a)
	}
}

// A hold that can't be taken leaves the server serving, says why, and is
// taken at a later check once it can be.
func TestKeepAwakeFailureKeepsServing(t *testing.T) {
	n := newNode(t, nil)
	n.awake.fail = errors.New("no power API")
	ctx := context.Background()
	if err := n.srv.Set(true, false, ""); err != nil {
		t.Fatal(err)
	}
	n.srv.reconcile(ctx)
	st := n.srv.Status()
	if !st.Running || st.AwakeLine != "Can't keep this PC awake: no power API" {
		t.Fatalf("status %+v", st)
	}
	n.awake.mu.Lock()
	n.awake.fail = nil
	n.awake.mu.Unlock()
	n.srv.reconcile(ctx)
	if a, _ := n.awake.counts(); a != 1 || n.srv.Status().AwakeLine != "Keeping this PC awake" {
		t.Fatalf("acquired %d, status %+v", a, n.srv.Status())
	}
	n.srv.stop()
	if _, r := n.awake.counts(); r != 1 {
		t.Fatalf("released %d", r)
	}
}
