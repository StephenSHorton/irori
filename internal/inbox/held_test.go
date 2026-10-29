package inbox

import (
	"context"
	"encoding/json"
	"errors"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// Release undoes a Dismiss: the dismissed delivery is queued with its
// message intact, its text goes on its card, and the hold it was kept for
// stays readable (HeldFor) with who decided and when.
func TestReleaseUndoesADismiss(t *testing.T) {
	h := newHarness(t)
	var mu sync.Mutex
	var calls []string
	h.onPassed = func(_ context.Context, m Message, to string) error {
		mu.Lock()
		calls = append(calls, m.ID+" to "+to)
		mu.Unlock()
		return nil
	}
	h.reopen()
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	m, d, err := h.ib.SendWith(bg, grok(keyG, "grok"), to("beta"), "about T1", SendOptions{Ref: "task:T1"})
	if err != nil {
		t.Fatal(err)
	}
	if d.State != Held || d.Hold != HoldBypassReceiver || d.HeldFor != HoldBypassReceiver {
		t.Fatalf("sent %+v", d)
	}
	h.clk.advance(time.Minute)
	if err := h.ib.Dismiss(bg, d.ID, user); err != nil {
		t.Fatal(err)
	}
	got := h.wantState(d.ID, Dismissed)
	if got.DecidedBy != user || !got.DecidedAt.Equal(h.clk.now()) || got.HeldFor != HoldBypassReceiver {
		t.Fatalf("dismissed %+v", got)
	}
	if len(h.ib.For(keyB, true, 0)) != 0 {
		t.Fatal("the recipient sees a dismissed message")
	}
	h.clk.advance(time.Minute)
	if err := h.ib.Release(bg, d.ID, user); err != nil {
		t.Fatal(err)
	}
	got = h.wantState(d.ID, Queued)
	if got.Hold != "" || got.HeldFor != HoldBypassReceiver || got.DecidedBy != user || !got.DecidedAt.Equal(h.clk.now()) {
		t.Fatalf("released %+v", got)
	}
	if got.Message.ID != m.ID || got.Message.Text != "about T1" {
		t.Fatalf("message %+v", got.Message)
	}
	mu.Lock()
	if len(calls) != 1 || calls[0] != m.ID+" to "+keyB {
		t.Fatalf("card fills %v", calls)
	}
	mu.Unlock()
	if w := h.wake(keyB); len(w.DeliveryIDs) != 1 || w.DeliveryIDs[0] != d.ID {
		t.Fatalf("wake %v", w.DeliveryIDs)
	}
	// Released twice is no undo: it isn't held or dismissed any more.
	wantErr(t, h.ib.Release(bg, d.ID, user), ErrBadState)

	// The fields are folded, so a restart reads them the same.
	h.reopen()
	if got := h.state(d.ID); got.HeldFor != HoldBypassReceiver || got.DecidedBy != user {
		t.Fatalf("after replay %+v", got)
	}
}

// HeldHistory lists what a hold once kept and isn't held any more, the
// most recently decided first; what was never held, or still is, isn't in
// it.
func TestHeldHistory(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	_, never := h.send(claude(keyA, "alpha"), to("beta"), "never held")
	_, d1 := h.send(grok(keyG, "grok"), to("beta"), "released")
	_, d2 := h.send(grok(keyG, "grok"), to("beta"), "dismissed")
	_, d3 := h.send(grok(keyG, "grok"), to("beta"), "still held")
	if never.State != Queued {
		t.Fatalf("never %+v", never)
	}
	h.clk.advance(time.Minute)
	if err := h.ib.Dismiss(bg, d2.ID, user); err != nil {
		t.Fatal(err)
	}
	h.clk.advance(time.Minute)
	if err := h.ib.Release(bg, d1.ID, user); err != nil {
		t.Fatal(err)
	}
	hist := h.ib.HeldHistory(0)
	if len(hist) != 2 || hist[0].ID != d1.ID || hist[1].ID != d2.ID {
		t.Fatalf("history %+v", hist)
	}
	if hist[0].State != Queued || hist[1].State != Dismissed || hist[1].Message.Text != "dismissed" {
		t.Fatalf("history %+v", hist)
	}
	if got := h.ib.HeldHistory(1); len(got) != 1 || got[0].ID != d1.ID {
		t.Fatalf("limit 1: %+v", got)
	}
	h.wantState(d3.ID, Held)
	if ds := h.ib.OfMessage(d3.MessageID); len(ds) != 1 || ds[0].ID != d3.ID {
		t.Fatalf("of message %+v", ds)
	}
}

// Across devices: a Release asked by a device that saw the delivery
// dismissed is the person's undo and applies on the owner, though the
// Dismiss was its delivery's first request, and though the asking device's
// clock runs behind the owner's.
func TestARemoteReleaseUndoesADismiss(t *testing.T) {
	h := newHarness(t)
	h.isOwner = func(k string) bool { return k == user || k == "user:dev2" }
	h.reopen()
	h.ros.put(card(keyB, "beta", "repo1", ""))
	if err := h.ib.SetPolicy(bg, Policy{Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := contextWithCancel(t)
	defer cancel()
	go h.ib.Follow(ctx)
	f := &foreign{h: h}
	m := Message{ID: "msg_r1", From: Sender{Key: keyR, Name: "remote", Kind: "claude"}, To: to(keyB), Text: "wake up", Trust: Untrusted, SentAt: h.clk.now()}
	f.row("agent:"+keyR, KindMessageSent, m)
	f.row("agent:"+keyR, KindDelivery, deliveryRow{DeliveryID: "dlv_r1", MessageID: m.ID, To: keyB, State: Queued, By: keyR})
	eventually(t, "the host holds it", func() bool {
		d, ok := h.ib.Get("dlv_r1")
		return ok && d.State == Held
	})
	h.clk.advance(time.Minute)
	f.row("user", KindDecided, decidedRow{DeliveryID: "dlv_r1", Choice: ChoiceDismiss, By: "user:dev2", AtDevice: "dev2", At: h.clk.now()})
	eventually(t, "the dismiss applies", func() bool {
		d, _ := h.ib.Get("dlv_r1")
		return d.State == Dismissed
	})
	behind := h.clk.now().Add(-time.Hour)
	f.row("user", KindDecided, decidedRow{DeliveryID: "dlv_r1", Choice: ChoiceRelease, By: "user:dev2", AtDevice: "dev2", At: behind, Undo: true})
	eventually(t, "the undo applies", func() bool {
		d, _ := h.ib.Get("dlv_r1")
		return d.State == Queued
	})
	if d, _ := h.ib.Get("dlv_r1"); d.DecidedBy != "user:dev2" || d.HeldFor != HoldPaused || d.Message.Text != "wake up" {
		t.Fatalf("released %+v", d)
	}
	// After a restart nothing is decided twice.
	cancel()
	h.reopen()
	ctx2, cancel2 := contextWithCancel(t)
	defer cancel2()
	go h.ib.Follow(ctx2)
	time.Sleep(50 * time.Millisecond)
	h.wantState("dlv_r1", Queued)
}

// A Release asked while its device still saw the delivery held lost the
// race to the Dismiss: it stays dismissed, though the asking device's clock
// runs ahead of the owner's.
func TestARemoteReleaseFromBeforeTheDismissLoses(t *testing.T) {
	h := newHarness(t)
	h.isOwner = func(k string) bool { return k == user || k == "user:dev2" }
	h.reopen()
	h.ros.put(card(keyB, "beta", "repo1", ""))
	if err := h.ib.SetPolicy(bg, Policy{Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := contextWithCancel(t)
	defer cancel()
	go h.ib.Follow(ctx)
	f := &foreign{h: h}
	m := Message{ID: "msg_r1", From: Sender{Key: keyR, Name: "remote", Kind: "claude"}, To: to(keyB), Text: "wake up", Trust: Untrusted, SentAt: h.clk.now()}
	f.row("agent:"+keyR, KindMessageSent, m)
	f.row("agent:"+keyR, KindDelivery, deliveryRow{DeliveryID: "dlv_r1", MessageID: m.ID, To: keyB, State: Queued, By: keyR})
	eventually(t, "the host holds it", func() bool {
		d, ok := h.ib.Get("dlv_r1")
		return ok && d.State == Held
	})
	if err := h.ib.Dismiss(bg, "dlv_r1", user); err != nil {
		t.Fatal(err)
	}
	ahead := h.clk.now().Add(time.Hour)
	f.row("user", KindDecided, decidedRow{DeliveryID: "dlv_r1", Choice: ChoiceRelease, By: "user:dev2", AtDevice: "dev2", At: ahead})
	time.Sleep(50 * time.Millisecond)
	h.wantState("dlv_r1", Dismissed)
}

// The asking device marks a Release an undo by what it sees: a dismissed
// delivery another device owns asks with undo, a held one without.
func TestARemoteReleaseIsAnUndoOnlyWhenSeenDismissed(t *testing.T) {
	h := newHarness(t)
	f := &foreign{h: h}
	m := Message{ID: "msg_r2", From: Sender{Key: keyA, Name: "alpha", Kind: "claude"}, To: to(keyR), Text: "over there", Trust: Untrusted, SentAt: h.clk.now()}
	f.row("agent:"+keyA, KindMessageSent, m)
	f.row("agent:"+keyR, KindDelivery, deliveryRow{DeliveryID: "dlv_h", MessageID: m.ID, To: keyR, State: Held, Hold: HoldPaused, By: "irori"})
	f.row("agent:"+keyR, KindDelivery, deliveryRow{DeliveryID: "dlv_d", MessageID: m.ID, To: keyR, State: Dismissed, Hold: HoldPaused, By: "user:dev2"})
	h.reopen()
	for _, id := range []string{"dlv_h", "dlv_d"} {
		if h.ib.Owned(id) {
			t.Fatalf("this device owns %s", id)
		}
		if err := h.ib.Release(bg, id, user); err != nil {
			t.Fatal(err)
		}
	}
	evs, err := h.st.List(bg, store.Filter{Kinds: []string{KindDecided}})
	if err != nil {
		t.Fatal(err)
	}
	undo := map[string]bool{}
	for _, e := range evs {
		var r decidedRow
		if err := json.Unmarshal(e.Payload, &r); err != nil {
			t.Fatal(err)
		}
		undo[r.DeliveryID] = r.Undo
	}
	if len(undo) != 2 || undo["dlv_h"] || !undo["dlv_d"] {
		t.Fatalf("undo marks %v, want dlv_d only", undo)
	}
}

// Undoing the Dismiss of repo work no agent has taken goes to the front
// desk; with no agent in that repo yet it says so and stays dismissed.
func TestReleasingDismissedRepoWorkWithNoDeskSaysSo(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), toRepo("repo2"), "anyone?")
	if d.State != Held || d.Hold != HoldUnowned || d.To != "" {
		t.Fatalf("sent %s %s to %q", d.State, d.Hold, d.To)
	}
	if err := h.ib.Dismiss(bg, d.ID, user); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.Release(bg, d.ID, user); !errors.Is(err, ErrBadState) {
		t.Fatalf("release with no desk: %v", err)
	}
	h.wantState(d.ID, Dismissed)
}
