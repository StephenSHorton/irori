package inbox

import (
	"slices"
	"strings"
	"testing"
	"time"
)

// Card #23: a wake the session's mod never confirms (the host refused
// prompt.submit, or the prompt was lost on the way) used to be reclaimed after
// ReattachGrace and issued again under a new token, forever: the Mac rows show
// 71 wake.issued rows for dlv_01M62R3ZJVF8XHH11JRXX0A5K9 between 20:34 and
// 23:24 with no delivered row in between, while the session sat idle.

// reclaimAll lets ReattachGrace pass and sweeps, so every unconfirmed wake
// goes back to Queued.
func (h *harness) reclaimAll() {
	h.t.Helper()
	h.clk.advance(ReattachGrace + time.Second)
	h.sweep()
}

func TestUnconfirmedWakeFallsBackToAppend(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	// The person's blocker answer, as the Board sends it: a wake-mode notice.
	d := h.sendWith(person(), to("beta"), "done", SendOptions{Mode: ModeWake, Ref: "task:01M62BP2MKFWJ4FK189419DNRV", Notice: true, Reason: "answered"})

	tokens := map[string]bool{}
	for i := range MaxWakeTries {
		w := h.wake(keyB)
		if !slices.Equal(w.DeliveryIDs, []string{d.ID}) || tokens[w.Token] {
			t.Fatalf("try %d: wake %+v", i, w)
		}
		tokens[w.Token] = true
		h.noAppend(keyB) // a wake item is not an append while it still has tries
		if i == 1 {
			h.reopen() // the count is folded from wake.issued rows, so replay keeps it
			h.ib.SetReachable(keyB, PathMod, true)
		}
		// The mod never confirms it.
		h.reclaimAll()
		h.wantState(d.ID, Queued)
	}

	// No fourth wake under yet another token: it goes out as an append, which
	// needs no idle prompt, and is read at the session's next turn.
	h.noWake(keyB)
	if ow, _ := h.ib.OutstandingWake(keyB); ow != nil {
		t.Fatalf("outstanding wake %+v", ow)
	}
	a := h.appendFor(keyB)
	if !strings.HasPrefix(a.Token, appendTokenPrefix) || a.Mode != ModeNextTurn || !slices.Equal(a.DeliveryIDs, []string{d.ID}) {
		t.Fatalf("append %+v", a)
	}
	if !strings.Contains(a.Prompt, `reason="answered"`) || !strings.Contains(a.Prompt, "No turn was started") {
		t.Fatalf("prompt:\n%s", a.Prompt)
	}
	if err := h.ib.MarkDelivered(bg, keyB, a.Token); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d.ID, Delivered); got.WakeID != a.ID || got.Via != string(PathMod) {
		t.Fatalf("delivered %+v", got)
	}
	// The next turn, whatever starts it, reads it: a notice then closes.
	if err := h.ib.MarkEngaged(bg, keyB, a.Token, "turn-human"); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d.ID, Skipped); got.Note != "notice" {
		t.Fatalf("engaged %+v", got)
	}
}

func TestAppendedWakeEngagesWithoutAHop(t *testing.T) {
	// The mod put a wake in as an append (its host refused the submit) and the
	// person's next turn read it. That wake did not start the turn, so a
	// message sent from it starts a fresh chain (hop 0), not one hop deeper.
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	m0, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	w := h.wake(keyB)
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.MarkAppendedEngaged(bg, keyB, w.Token, "turn-human"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Engaged)
	if id := h.ib.RunningWake(keyB); id != "" {
		t.Fatalf("running wake %s", id)
	}
	if m, _ := h.send(claude(keyB, "beta"), to("gamma"), "from the person's turn"); m.Hop != 0 || m.ParentID != "" {
		t.Fatalf("hop %d parent %s (wake message %s)", m.Hop, m.ParentID, m0.ID)
	}
	// The same wake engaged as a real wake does continue the chain.
	_, d2 := h.send(claude(keyA, "alpha"), to("beta"), "q2")
	w2 := h.deliverEngage(keyB, "t2")
	if !slices.Contains(w2.DeliveryIDs, d2.ID) {
		t.Fatalf("wake %v", w2.DeliveryIDs)
	}
	if m, _ := h.send(claude(keyB, "beta"), to("gamma"), "from the woken turn"); m.Hop != 1 {
		t.Fatalf("woken turn hop %d", m.Hop)
	}
}

func TestAppendedWakeDoesNotGateTheNext(t *testing.T) {
	// A wake the mod appended leaves no prompt in the host's queue, so it
	// must not hold the session's next wake back for UnengagedLimit.
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	_, d1 := h.send(claude(keyA, "alpha"), to("beta"), "refused")
	w1 := h.wake(keyB)
	if err := h.ib.MarkAppendedDelivered(bg, keyB, w1.Token); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d1.ID, Delivered); got.WakeID != w1.ID {
		t.Fatalf("delivered %+v", got)
	}
	if ow, _ := h.ib.OutstandingWake(keyB); ow != nil {
		t.Fatalf("outstanding %+v", ow)
	}
	h.reopen() // the flag is folded from the delivery row
	h.ib.SetReachable(keyB, PathMod, true)
	_, d2 := h.send(claude(keyC, "gamma"), to("beta"), "next")
	if w2 := h.wake(keyB); !slices.Equal(w2.DeliveryIDs, []string{d2.ID}) {
		t.Fatalf("wake %v", w2.DeliveryIDs)
	}

	// Reported delivered while the submit was pending (so it gates), then
	// appended after a later drop: the second report lifts the gate.
	h2 := newHarness(t)
	h2.abc()
	h2.ib.SetReachable(keyB, PathMod, true)
	h2.send(claude(keyA, "alpha"), to("beta"), "queued, then dropped")
	w := h2.wake(keyB)
	if err := h2.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if ow, st := h2.ib.OutstandingWake(keyB); ow == nil || st != Delivered {
		t.Fatalf("pending submit should gate: %+v %s", ow, st)
	}
	if err := h2.ib.MarkAppendedDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if ow, _ := h2.ib.OutstandingWake(keyB); ow != nil {
		t.Fatalf("still gated by %+v", ow)
	}
	// Engaging it clears the flag with the Delivered state.
	if err := h2.ib.MarkAppendedEngaged(bg, keyB, w.Token, "turn-human"); err != nil {
		t.Fatal(err)
	}
	if got := h2.wantState(w.DeliveryIDs[0], Engaged); got.WakeID != w.ID {
		t.Fatalf("engaged %+v", got)
	}
}

func TestUntakenItemRidesTheNextWake(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	_, stuck := h.send(claude(keyA, "alpha"), to("beta"), "first")
	for range MaxWakeTries {
		h.wake(keyB)
		h.reclaimAll()
	}
	// An append for it goes out, but the mod drops that too (reclaimed).
	h.appendFor(keyB)
	h.reclaimAll()
	h.noWake(keyB)

	// The session takes wakes again (say, after a reload): a new wake item
	// wakes it, and the untaken one rides along instead of waiting.
	_, fresh := h.send(claude(keyC, "gamma"), to("beta"), "second")
	w := h.wake(keyB)
	if !slices.Equal(w.DeliveryIDs, []string{fresh.ID, stuck.ID}) {
		t.Fatalf("wake %v, want the new item then the untaken one", w.DeliveryIDs)
	}
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	h.wantState(stuck.ID, Delivered)
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	h.wantState(stuck.ID, Engaged)
	h.wantState(fresh.ID, Engaged)
}

func TestWakeTriesRestartWhenRequeued(t *testing.T) {
	// A delivery that once went through (deferred, then due again) gets its
	// full tries again: the count is for unconfirmed wakes in a row.
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	for range MaxWakeTries - 1 {
		h.wake(keyB)
		h.reclaimAll()
	}
	w := h.deliverEngage(keyB, "t1")
	if err := h.ib.Defer(bg, keyB, d.ID, 1, ""); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	h.clk.advance(time.Minute + time.Second)
	h.sweep()
	h.wantState(d.ID, Queued)
	for range MaxWakeTries {
		h.wake(keyB)
		h.reclaimAll()
	}
	h.noWake(keyB)
	h.appendFor(keyB)
}

func TestChannelPushesKeepRepeating(t *testing.T) {
	// A Grok member acks a push with a tool call; its unacked pushes are
	// re-pushed by design, never turned into appends (it has none).
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	h.ib.SetReachable(keyG, PathChannel, true)
	_, d := h.send(claude(keyA, "alpha"), to("grok"), "q")
	for range MaxWakeTries + 1 {
		w, err := h.ib.NextWake(bg, keyG, PathChannel)
		if err != nil || w == nil || w.DeliveryIDs[0] != d.ID {
			t.Fatalf("push %+v %v", w, err)
		}
		h.reclaimAll()
	}
}

func TestWakeLandingMidTurn(t *testing.T) {
	// A wake issued while the session runs a turn: the mod reports it
	// delivered once the submit is queued, and it engages at its own turn.
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	h.send(claude(keyA, "alpha"), to("beta"), "first")
	w1 := h.deliverEngage(keyB, "t1")
	if h.ib.RunningWake(keyB) != w1.ID {
		t.Fatal("no running turn")
	}
	_, d := h.send(claude(keyC, "gamma"), to("beta"), "while you work")
	w2 := h.wake(keyB) // mid-turn: a running turn does not hold a wake back
	if !slices.Equal(w2.DeliveryIDs, []string{d.ID}) {
		t.Fatalf("wake %v", w2.DeliveryIDs)
	}
	if err := h.ib.MarkDelivered(bg, keyB, w2.Token); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Delivered)
	if err := h.ib.TurnEnded(bg, keyB, w1.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.MarkEngaged(bg, keyB, w2.Token, "t2"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Engaged)

	// The same, but the host drops the mid-turn submit every time: once its
	// tries are spent it goes as an append, and the next turn reads it.
	_, d2 := h.send(claude(keyC, "gamma"), to("beta"), "dropped while you work")
	for range MaxWakeTries {
		h.wake(keyB)
		h.reclaimAll()
	}
	h.noWake(keyB)
	a := h.appendFor(keyB)
	if err := h.ib.MarkDelivered(bg, keyB, a.Token); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.TurnEnded(bg, keyB, w2.Token, "t2"); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.MarkEngaged(bg, keyB, a.Token, "t3"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d2.ID, Engaged)
}
