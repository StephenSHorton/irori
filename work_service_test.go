package main

import (
	"context"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// A coord change reaches work:changed without a poll, and a burst is
// coalesced into a few events.
func TestWorkPumpForwards(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev-a", hlc.New(time.Now))
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	c, err := coord.New(ctx, st, nil, coord.Options{})
	if err != nil {
		t.Fatal(err)
	}

	var emits atomic.Int32
	emitted := make(chan struct{}, 100)
	done := make(chan struct{})
	changes := c.Subscribe(ctx)
	go func() {
		defer close(done)
		pumpWork(ctx, changes, 100*time.Millisecond, func() {
			emits.Add(1)
			emitted <- struct{}{}
		})
	}()
	wait := func(what string) {
		t.Helper()
		select {
		case <-emitted:
		case <-time.After(2 * time.Second):
			t.Fatalf("no work:changed within 2s of %s", what)
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
	if n := emits.Load(); n > 3 {
		t.Fatalf("%d emits for 21 changes, want them coalesced", n)
	}

	cancel()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("pump did not stop on cancel")
	}
}
