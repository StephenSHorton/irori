package coord

import (
	"encoding/json"
	"errors"
	"slices"
	"sort"
	"testing"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// pinnedOrder lists the pinned cards' ids, by rank.
func pinnedOrder(t *testing.T, c *Coord) []string {
	t.Helper()
	var ps []Task
	for _, x := range c.ListTasks(TaskFilter{}) {
		if x.Pinned {
			ps = append(ps, x)
		}
	}
	sort.Slice(ps, func(i, j int) bool { return ps[i].PinRank < ps[j].PinRank })
	var out []string
	for _, x := range ps {
		out = append(out, x.ID)
	}
	return out
}

// The person pins cards (each new one last), moves one before another,
// unpins one, and every device's replay agrees. Agents can't pin.
func TestPins(t *testing.T) {
	e := newEnv(t)
	a := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "a"}))
	b := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "b"}))
	c := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "c"}))
	if _, err := e.c.Pin(ctx, alice, a.ID, true); !errors.Is(err, ErrNotAllowed) {
		t.Fatalf("an agent pinned a card: %v", err)
	}
	for _, x := range []Task{a, b, c} {
		if _, err := e.c.Pin(ctx, user, x.ID, true); err != nil {
			t.Fatal(err)
		}
	}
	if got := pinnedOrder(t, e.c); !slices.Equal(got, []string{a.ID, b.ID, c.ID}) {
		t.Fatalf("pinned order %v", got)
	}
	// c before a (to the top), then b before c (between nothing and c).
	if _, err := e.c.MovePin(ctx, user, c.ID, a.ID); err != nil {
		t.Fatal(err)
	}
	if got := pinnedOrder(t, e.c); !slices.Equal(got, []string{c.ID, a.ID, b.ID}) {
		t.Fatalf("after moving c to the top: %v", got)
	}
	if _, err := e.c.MovePin(ctx, user, b.ID, a.ID); err != nil {
		t.Fatal(err)
	}
	if got := pinnedOrder(t, e.c); !slices.Equal(got, []string{c.ID, b.ID, a.ID}) {
		t.Fatalf("after moving b before a: %v", got)
	}
	if _, err := e.c.MovePin(ctx, user, a.ID, "no-such-card"); err == nil {
		t.Fatal("moved before a card that isn't pinned")
	}
	if _, err := e.c.Pin(ctx, user, b.ID, false); err != nil {
		t.Fatal(err)
	}
	if x, _ := e.c.GetTask(b.ID); x.Pinned || x.PinRank != 0 {
		t.Fatalf("unpinned card %+v", x)
	}
	want := []string{c.ID, a.ID}
	if got := pinnedOrder(t, e.c); !slices.Equal(got, want) {
		t.Fatalf("after unpinning b: %v", got)
	}
	if got := pinnedOrder(t, e.reopen()); !slices.Equal(got, want) {
		t.Fatalf("replay: %v", got)
	}
}

// pinRow is a task.pinned row another device wrote.
func pinRow(dev string, seq int64, at hlc.Timestamp, actor, by, id string, pinned bool, rank float64) store.Event {
	raw, _ := json.Marshal(taskPinnedRow{TaskID: id, Pinned: pinned, Rank: rank, By: by, At: fixedNow})
	return store.Event{DeviceID: dev, Seq: seq, HLC: at, Actor: actor, Kind: KindTaskPinned, Payload: raw}
}

// A pin from the person's other device counts, even when it arrives before
// its card; an agent's row claiming to pin is ignored; for one card the
// latest row stands whatever order they arrive in.
func TestPinsAcrossDevices(t *testing.T) {
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	rows := []store.Event{
		pinRow("dev-b", 1, base+5, "user", "user:dev-b", "A", true, 1),
		createdRow("dev-b", 2, base+6, "A", 1),
		pinRow("dev-b", 3, base+7, "agent:dev-b/s", "dev-b/s", "A", false, 0), // not the person's
		pinRow("dev-b", 4, base+20, "user", "user:dev-b", "A", false, 0),
		pinRow("dev-c", 1, base+10, "user", "user:dev-c", "A", true, 3), // earlier than the unpin
	}
	for name, order := range map[string][]int{"in order": {0, 1, 2, 3, 4}, "dev-c first": {4, 0, 1, 2, 3}, "dev-c between": {0, 1, 4, 2, 3}} {
		e := newEnv(t)
		e.owner = func(k string) bool { return k == "user:dev-a" || k == "user:dev-b" || k == "user:dev-c" }
		e.c = e.reopen()
		for _, i := range order {
			e.c.impl.applyRemote(ctx, rows[i])
		}
		if x, _ := e.c.GetTask("A"); x.Pinned {
			t.Fatalf("%s: the latest row unpinned A, but it reads pinned", name)
		}
	}
	// Without the unpin, the person's pin from dev-b stands and the agent's row is ignored.
	e := newEnv(t)
	e.owner = func(k string) bool { return k == "user:dev-a" || k == "user:dev-b" }
	e.c = e.reopen()
	for _, i := range []int{0, 1, 2} {
		e.c.impl.applyRemote(ctx, rows[i])
	}
	if x, _ := e.c.GetTask("A"); !x.Pinned || x.PinRank != 1 {
		t.Fatalf("pin from the other device: %+v", x)
	}
}
