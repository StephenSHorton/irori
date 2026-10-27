package main

import (
	"context"
	"fmt"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// A session first seen before the newest 1000 agent.seen rows must still
// have its card, or Gone drops it and Revive refuses it.
func TestCardIndexReachesPastNewestThousand(t *testing.T) {
	ctx := context.Background()
	const dev = "dev1"
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), dev, hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	seen := func(key, name string) {
		t.Helper()
		if _, err := st.Append(ctx, "irori", store.KindAgentSeen, presence.AgentCard{Key: key, Name: name, Agent: "claude"}); err != nil {
			t.Fatal(err)
		}
	}
	seen(dev+"/old", "first")
	seen(dev+"/old", "second") // the newer card for the same key wins
	for i := range store.MaxListLimit + 5 {
		seen(fmt.Sprintf("%s/s%04d", dev, i), "")
	}

	var x cardIndex
	cards, err := x.update(ctx, st.List, dev)
	if err != nil {
		t.Fatal(err)
	}
	c, ok := cards[dev+"/old"]
	if !ok {
		t.Fatalf("old session missing from %d cards", len(cards))
	}
	if c.Name != "second" {
		t.Fatalf("old session card = %q, want the newest (second)", c.Name)
	}
	if want := store.MaxListLimit + 6; len(cards) != want {
		t.Fatalf("got %d cards, want %d", len(cards), want)
	}

	// A later update picks up only what was appended since.
	seen(dev+"/new", "")
	seen(dev+"/old", "third")
	cards, err = x.update(ctx, st.List, dev)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cards[dev+"/new"]; !ok {
		t.Fatal("new session missing after second update")
	}
	if cards[dev+"/old"].Name != "third" {
		t.Fatalf("old session card = %q after update, want third", cards[dev+"/old"].Name)
	}
}

// A second Revive of the same session inside the window is refused, so a
// re-shown button or stale Recently gone row can't start it twice.
func TestReviveGuardRefusesRepeatWithinWindow(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	g := reviveGuard{now: func() time.Time { return now }}
	if _, ok := g.claim("abcd1234"); !ok {
		t.Fatal("first claim refused")
	}
	now = now.Add(5 * time.Second)
	if ago, ok := g.claim("abcd1234"); ok || ago != 5*time.Second {
		t.Fatalf("second claim = (%v, %v), want refused 5s ago", ago, ok)
	}
	if _, ok := g.claim("ffff0000"); !ok {
		t.Fatal("another session's claim refused")
	}
	if !g.recent("abcd1234", now.Add(-time.Minute)) {
		t.Fatal("recent = false for a session revived after it left")
	}
	if g.recent("abcd1234", now) {
		t.Fatal("recent = true for a session that left again after the revive")
	}
	now = now.Add(reviveWindow)
	if _, ok := g.claim("abcd1234"); !ok {
		t.Fatal("claim refused after the window passed")
	}
	g.release("abcd1234")
	if _, ok := g.claim("abcd1234"); !ok {
		t.Fatal("claim refused after release")
	}
}
