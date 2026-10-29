package inbox

import (
	"sync"
	"testing"
)

// A queued or held notice about an ask someone else took is closed as
// Skipped before a wake or append carries it, and by the sweep (board.md
// "Asks": once taken, the others stop hearing).
func TestStaleNoticesAreSkipped(t *testing.T) {
	h := newHarness(t)
	var mu sync.Mutex
	taken := map[string]string{} // ref -> taker
	h.stale = func(d Delivery) string {
		mu.Lock()
		defer mu.Unlock()
		if k := taken[d.Message.Ref]; k != "" && k != d.To {
			return "taken by " + k
		}
		return ""
	}
	h.open()
	h.abc()
	ask := SendOptions{Notice: true, Ref: "task:01ASK", Mode: ModeNextTurn}
	toA := h.sendWith(person(), to("alpha"), "the person asks the room", ask)
	toB := h.sendWith(person(), to("beta"), "the person asks the room", ask)
	toC := h.sendWith(person(), to("gamma"), "the person asks the room", SendOptions{Notice: true, Ref: "task:01ASK", Mode: ModeWake})
	plain := h.sendWith(person(), to("beta"), "a plain message", SendOptions{Ref: "task:01ASK"})
	mu.Lock()
	taken["task:01ASK"] = keyA
	mu.Unlock()

	if w := h.appendFor(keyB); len(w.DeliveryIDs) != 1 || w.DeliveryIDs[0] != plain.ID {
		t.Fatalf("beta's append carried %v", w.DeliveryIDs)
	}
	if d := h.wantState(toB.ID, Skipped); d.Note != "taken by "+keyA {
		t.Fatalf("note %q", d.Note)
	}
	h.noWake(keyC)
	h.wantState(toC.ID, Skipped)
	// The taker still hears it.
	if w := h.appendFor(keyA); len(w.DeliveryIDs) != 1 || w.DeliveryIDs[0] != toA.ID {
		t.Fatalf("alpha's append %v", w.DeliveryIDs)
	}

	// A queued one the sweep finds.
	toB2 := h.sendWith(person(), to("beta"), "again", ask)
	if err := h.ib.Sweep(bg); err != nil {
		t.Fatal(err)
	}
	h.wantState(toB2.ID, Skipped)
	h.reopen()
	h.wantState(toB.ID, Skipped)
}
