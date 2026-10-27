package inbox

import (
	"slices"
	"strings"
	"testing"
	"time"
)

func TestWakeBatchingOrderAndSize(t *testing.T) {
	h := newHarness(t)
	h.abc()
	var ids []string
	big := strings.Repeat("x", 6000)
	for i, from := range []Sender{claude(keyA, "alpha"), claude(keyC, "gamma"), claude(keyA, "alpha"), person()} {
		_, d := h.send(from, to("beta"), big+string(rune('a'+i)))
		ids = append(ids, d.ID)
		h.clk.advance(time.Second)
	}
	// Two fit in full; the rest is handed over too, named in the overflow
	// line for the inbox tool, and confirmed with the rest.
	w1 := h.deliverEngage(keyB, "t1")
	if len(w1.Prompt) > MaxWakeBytes || !slices.Equal(w1.DeliveryIDs, ids) || !slices.Equal(w1.Overflow, ids[2:]) {
		t.Fatalf("first wake %v overflow %v (%d bytes)", w1.DeliveryIDs, w1.Overflow, len(w1.Prompt))
	}
	if !strings.Contains(w1.Prompt, "4 messages") || !strings.Contains(w1.Prompt, "2 more waiting: read them with the irori inbox tool: "+ids[2]+", "+ids[3]+"\n") ||
		strings.Contains(w1.Prompt, `delivery="`+ids[2]+`"`) || strings.Contains(w1.Prompt, big+"c") {
		t.Fatalf("overflow not listed by id only:\n%s", w1.Prompt[len(w1.Prompt)-300:])
	}
	for _, id := range ids {
		h.wantState(id, Engaged)
	}
	h.noWake(keyB)

	// One message bigger than the cap still goes in full; nothing else fits,
	// not even as overflow, so the next one waits for the next wake.
	huge := strings.Repeat("\U0001F600", MaxText) // 32 KB of text
	_, d := h.send(claude(keyA, "alpha"), to("gamma"), huge)
	_, small := h.send(claude(keyA, "alpha"), to("gamma"), "small")
	w := h.deliverEngage(keyC, "t2")
	if len(w.DeliveryIDs) != 1 || w.DeliveryIDs[0] != d.ID || len(w.Overflow) != 0 {
		t.Fatalf("oversize wake %v", w.DeliveryIDs)
	}
	if w2 := h.wake(keyC); !slices.Equal(w2.DeliveryIDs, []string{small.ID}) || !strings.Contains(w2.Prompt, `trust="untrusted"`) {
		t.Fatalf("second wake %v", w2.DeliveryIDs)
	}
}

func TestOverflowCappedAndRebuilt(t *testing.T) {
	h := newHarness(t)
	h.abc()
	// Enough small messages that even their ids don't all fit: the overflow
	// line stops at the cap and the rest stays queued.
	big := strings.Repeat("y", 7000)
	var ids []string
	for i := range 3 + 600 {
		text := "s"
		if i < 3 {
			text = big
		}
		_, d := h.send(person(), to("beta"), text)
		ids = append(ids, d.ID)
	}
	w := h.wake(keyB)
	if len(w.Prompt) > MaxWakeBytes || len(w.Overflow) == 0 || len(w.DeliveryIDs) >= len(ids) {
		t.Fatalf("%d ids, %d overflow, %d bytes", len(w.DeliveryIDs), len(w.Overflow), len(w.Prompt))
	}
	// A reattach rebuilds the same prompt, overflow line included.
	ow, st := h.ib.OutstandingWake(keyB)
	if st != Queued || ow.Prompt != w.Prompt || !slices.Equal(ow.Overflow, w.Overflow) {
		t.Fatalf("rebuilt %s:\n%q\nwant\n%q", st, ow.Prompt[len(ow.Prompt)-200:], w.Prompt[len(w.Prompt)-200:])
	}
	h.reopen()
	if ow, _ := h.ib.OutstandingWake(keyB); ow == nil || ow.Prompt != w.Prompt {
		t.Fatal("rebuilt prompt differs after replay")
	}
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t"); err != nil {
		t.Fatal(err)
	}
	rest := ids[len(w.DeliveryIDs):]
	h.wantState(rest[0], Queued)
	w2 := h.wake(keyB)
	if w2.DeliveryIDs[0] != rest[0] {
		t.Fatalf("next wake starts at %s, want %s", w2.DeliveryIDs[0], rest[0])
	}
}

func TestNothingToWake(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.noWake(keyB)
	if _, err := h.ib.NextWake(bg, "", PathMod); err == nil {
		t.Fatal("empty agent accepted")
	}
	// Held deliveries never wake.
	h.ros.put(grokCard(keyG, "grok", ""))
	h.send(grok(keyG, "grok"), to("beta"), "held")
	h.noWake(keyB)
}

func TestReclaimUnconfirmedWake(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	w := h.wake(keyB)
	h.clk.advance(ReattachGrace)
	h.sweep() // not past the grace yet
	h.noWake(keyB)
	h.clk.advance(time.Second)
	h.sweep()
	got := h.wantState(d.ID, Queued)
	if got.WakeID != "" {
		t.Fatal("still in the old wake")
	}
	w2 := h.wake(keyB)
	if w2.ID == w.ID || w2.DeliveryIDs[0] != d.ID {
		t.Fatalf("re-wake %+v", w2)
	}
	// The first wake's late confirmation can't steal it from the second.
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d.ID, Queued); got.WakeID != w2.ID {
		t.Fatalf("wake %s", got.WakeID)
	}
	if err := h.ib.MarkDelivered(bg, keyB, w2.Token); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Delivered)
}

func TestLateConfirmAfterReclaimCounts(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	w := h.wake(keyB)
	h.clk.advance(ReattachGrace + time.Second)
	h.sweep()
	h.wantState(d.ID, Queued)
	// The prompt did go out after all: mark it delivered, don't send it twice.
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d.ID, Delivered); got.WakeID != w.ID {
		t.Fatal(got.WakeID)
	}
	h.noWake(keyB)
	// A turn start that never got a delivered confirmation engages directly.
	_, d2 := h.send(claude(keyC, "gamma"), to("alpha"), "q2")
	w2 := h.wake(keyA)
	if err := h.ib.MarkEngaged(bg, keyA, w2.Token, "t"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d2.ID, Engaged)
}

func TestStaleDeliveredWakeStopsGating(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.send(claude(keyA, "alpha"), to("beta"), "q")
	w := h.wake(keyB)
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	sub := h.ib.Subscribe(t.Context())
	h.send(claude(keyC, "gamma"), to("beta"), "q2")
	h.noWake(keyB)
	if n := count(drain(sub), WakeReady, keyB); n != 0 {
		t.Fatalf("%d WakeReady while gated", n)
	}
	h.clk.advance(UnengagedLimit)
	h.sweep() // the sweeper notices the gate opened with time
	if n := count(drain(sub), WakeReady, keyB); n != 1 {
		t.Fatalf("%d WakeReady after the gate went stale", n)
	}
	h.sweep()
	if n := count(drain(sub), WakeReady, keyB); n != 0 {
		t.Fatalf("%d repeated WakeReady", n)
	}
	if w2 := h.wake(keyB); len(w2.DeliveryIDs) != 1 {
		t.Fatalf("wake %v", w2.DeliveryIDs)
	}
}

func TestReminderWake(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	w := h.deliverEngage(keyB, "t1")
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	sub := h.ib.Subscribe(t.Context())
	h.clk.advance(QuietLimit)
	h.sweep()
	h.noWake(keyB) // not past the hour yet
	h.clk.advance(time.Second)
	h.sweep()
	if n := count(drain(sub), WakeReady, keyB); n != 1 {
		t.Fatalf("%d WakeReady", n)
	}
	r := h.wake(keyB)
	if !r.Reminder || !strings.HasPrefix(r.Token, "[irori-reminder:wake_") || strings.Count(r.Prompt, r.Token) != 1 {
		t.Fatalf("reminder %+v", r)
	}
	if strings.Contains(r.Prompt, "[irori-wake:") || !strings.Contains(r.Prompt, d.ID) {
		t.Fatal("reminder prompt")
	}
	if !h.state(d.ID).Reminded {
		t.Fatal("not marked reminded")
	}
	// Reminder tokens never engage anything.
	for _, f := range []func() error{
		func() error { return h.ib.MarkDelivered(bg, keyB, r.Token) },
		func() error { return h.ib.MarkEngaged(bg, keyB, r.Token, "t2") },
		func() error { return h.ib.TurnEnded(bg, keyB, r.Token, "t2") },
	} {
		if err := f(); err != nil {
			t.Fatal(err)
		}
	}
	h.wantState(d.ID, Engaged)
	if h.ib.RunningWake(keyB) != "" {
		t.Fatal("reminder set a running wake")
	}
	// Once only.
	h.clk.advance(QuietLimit * 3)
	h.sweep()
	h.noWake(keyB)

	// A deferral earns its own reminder later.
	if err := h.ib.Defer(bg, keyB, d.ID, 10, "after lunch"); err != nil {
		t.Fatal(err)
	}
	if h.state(d.ID).Reminded {
		t.Fatal("deferral kept the old reminder")
	}
}

func TestUnreachableIsNotReminded(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.send(claude(keyA, "alpha"), to("beta"), "q")
	h.deliverEngage(keyB, "t1")
	h.clk.advance(QuietLimit + time.Minute)
	h.sweep()
	h.noWake(keyB)
}

func TestDeferralDue(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	h.deliverEngage(keyB, "t1")
	if err := h.ib.Defer(bg, keyB, d.ID, 30, "later"); err != nil {
		t.Fatal(err)
	}
	sub := h.ib.Subscribe(t.Context())
	h.clk.advance(29 * time.Minute)
	h.sweep()
	h.wantState(d.ID, Deferred)
	h.clk.advance(time.Minute)
	h.sweep()
	h.wantState(d.ID, Queued)
	if n := count(drain(sub), WakeReady, keyB); n != 1 {
		t.Fatalf("%d WakeReady", n)
	}
	if w := h.wake(keyB); w.DeliveryIDs[0] != d.ID {
		t.Fatal(w.DeliveryIDs)
	}
}

func TestSubscribeEdges(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	sub := h.ib.Subscribe(t.Context())

	h.send(claude(keyA, "alpha"), to("beta"), "1")
	h.send(claude(keyC, "gamma"), to("beta"), "2")
	cs := drain(sub)
	if n := count(cs, WakeReady, keyB); n != 1 {
		t.Fatalf("%d WakeReady for two sends", n)
	}
	if count(cs, Updated, keyB) < 2 {
		t.Fatal("no Updated")
	}
	w := h.wake(keyB)
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	drain(sub)
	h.send(claude(keyA, "alpha"), to("beta"), "3") // gated: no WakeReady
	if n := count(drain(sub), WakeReady, keyB); n != 0 {
		t.Fatalf("%d WakeReady while gated", n)
	}
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t"); err != nil {
		t.Fatal(err)
	}
	if n := count(drain(sub), WakeReady, keyB); n != 1 {
		t.Fatalf("%d WakeReady after engaged", n)
	}

	_, hd := h.send(grok(keyG, "grok"), to("beta"), "held")
	cs = drain(sub)
	if count(cs, HeldNew, keyB) != 1 || cs[len(cs)-1].DeliveryID != hd.ID {
		t.Fatalf("held changes %+v", cs)
	}

	// Release of a held item makes something to submit again.
	h.wake(keyB) // takes "3"
	w3, _ := h.ib.OutstandingWake(keyB)
	if err := h.ib.MarkEngaged(bg, keyB, w3.Token, "t3"); err != nil {
		t.Fatal(err)
	}
	drain(sub)
	if err := h.ib.Release(bg, hd.ID, user); err != nil {
		t.Fatal(err)
	}
	if n := count(drain(sub), WakeReady, keyB); n != 1 {
		t.Fatalf("%d WakeReady after release", n)
	}

	// Subscribers end with their context.
	ctx, cancel := contextWithCancel(t)
	s2 := h.ib.Subscribe(ctx)
	cancel()
	select {
	case _, ok := <-s2:
		for ok {
			_, ok = <-s2
		}
	case <-time.After(time.Second):
		t.Fatal("not closed")
	}
}

func TestTurnEndedFreesGateForQueuedWork(t *testing.T) {
	// Work queued while a wake was delivered but before it engaged is offered
	// once the gate opens (engaged), and TurnEnded with nothing new is quiet.
	h := newHarness(t)
	h.abc()
	h.send(claude(keyA, "alpha"), to("beta"), "1")
	w := h.deliverEngage(keyB, "t1")
	sub := h.ib.Subscribe(t.Context())
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	if n := count(drain(sub), WakeReady, keyB); n != 0 {
		t.Fatalf("%d WakeReady with nothing queued", n)
	}
	wantErr(t, h.ib.TurnEnded(bg, keyB, "[irori-wake:nope]", "t1"), ErrNotFound)
	wantErr(t, h.ib.MarkEngaged(bg, keyA, w.Token, "t1"), ErrNotYours)
}
