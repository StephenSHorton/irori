package inbox

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// Another device's agent, as the mesh roster shows it.
const keyR = "dev2/rrrrrrrr-9999-4999-8999-999999999999"

// foreign inserts a row written by dev2, as the mesh would.
type foreign struct {
	h   *harness
	seq int64
}

func (f *foreign) row(actor, kind string, payload any) {
	f.h.t.Helper()
	f.seq++
	raw, _ := json.Marshal(payload)
	at := hlc.Timestamp(f.h.clk.now().UnixMilli())<<16 + hlc.Timestamp(f.seq)
	if err := f.h.st.Insert(bg, store.Event{DeviceID: "dev2", Seq: f.seq, HLC: at, Actor: actor, Kind: kind, Payload: raw}); err != nil {
		f.h.t.Fatal(err)
	}
}

func eventually(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
}

// A message to another device's agent is queued with no hold decision, is
// never woken here, and is never swept as gone: its host does all three
// (v0.4 §6.3).
func TestARemoteRecipientIsTheHostsToWakeAndSweep(t *testing.T) {
	h := newHarness(t)
	h.abc()
	if err := h.ib.SetPolicy(bg, Policy{Paused: true, Block: []string{keyA}}, user); err != nil {
		t.Fatal(err)
	}
	h.ros.put(card(keyR, "remote", "repo1", ""))
	_, d := h.send(claude(keyA, "alpha"), to(keyR), "to the mac")
	if d.State != Queued || d.Hold != "" {
		t.Fatalf("this device's policy decided a remote recipient's hold: %s %s", d.State, d.Hold)
	}
	if h.ib.Owned(d.ID) {
		t.Fatal("this device owns a delivery to another device's agent")
	}
	if err := h.ib.SetPolicy(bg, Policy{}, user); err != nil {
		t.Fatal(err)
	}
	h.noWake(keyR)
	if w, _ := h.ib.NextAppend(bg, keyR); w != nil {
		t.Fatal("an append for another device's agent")
	}
	// Gone from this device's view for far longer than the grace: frozen.
	h.ros.remove(keyR)
	for range 3 {
		h.clk.advance(GoneGrace + UnengagedLimit)
		h.sweep()
	}
	if got := h.wantState(d.ID, Queued); got.To != keyR {
		t.Fatalf("a remote recipient's delivery moved to %q", got.To)
	}
}

// Repo work with no owner yet is assigned only by the device that wrote
// it, even when this device has a desk for that repo.
func TestRepoWorkIsAssignedOnlyByItsWriter(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyA, PathMod, true)
	f := &foreign{h: h}
	m := Message{ID: "msg_remote", From: Sender{Key: keyR, Name: "remote", Kind: "claude"}, To: toRepo("repo1"), Text: "anyone?", Trust: Untrusted, SentAt: h.clk.now()}
	f.row("agent:"+keyR, KindMessageSent, m)
	f.row("agent:"+keyR, KindDelivery, deliveryRow{DeliveryID: "dlv_remote", MessageID: m.ID, RepoKey: "repo1", State: Held, Hold: HoldUnowned, By: keyR})
	h.reopen()
	h.ib.SetReachable(keyA, PathMod, true)
	h.sweep()
	if got := h.wantState("dlv_remote", Held); got.To != "" {
		t.Fatalf("this device assigned another device's repo work to %s", got.To)
	}
	// Its own unowned work it does assign: repo2 has no desk until gamma
	// attaches.
	_, d := h.send(claude(keyA, "alpha"), toRepo("repo2"), "local work")
	if d.To != "" {
		t.Fatalf("sent to %q with no desk", d.To)
	}
	h.ib.SetReachable(keyC, PathMod, true)
	h.sweep()
	if got := h.state(d.ID); got.To == "" {
		t.Fatal("this device didn't assign its own repo work")
	}
}

// The host decides holds with its own policy when it folds another
// device's queued row (Follow), and applies the first of two requests.
func TestTheHostHoldsAndAppliesTheFirstRequest(t *testing.T) {
	h := newHarness(t)
	h.isOwner = func(k string) bool { return k == user || k == "user:dev2" }
	h.reopen()
	h.ros.put(card(keyB, "beta", "repo1", "")) // not in bypass: the pause decides
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
		return ok && d.State == Held && d.Hold == HoldPaused
	})
	// An agent's request is ignored; the person's first one wins.
	f.row("agent:"+keyR, KindDecided, decidedRow{DeliveryID: "dlv_r1", Choice: ChoiceRelease, By: keyR})
	f.row("user", KindDecided, decidedRow{DeliveryID: "dlv_r1", Choice: ChoiceDismiss, By: "user:dev2", AtDevice: "dev2"})
	f.row("user", KindDecided, decidedRow{DeliveryID: "dlv_r1", Choice: ChoiceRelease, By: "user:dev2", AtDevice: "dev2"})
	eventually(t, "the first request applies", func() bool {
		d, _ := h.ib.Get("dlv_r1")
		return d.State == Dismissed
	})
	time.Sleep(50 * time.Millisecond)
	h.wantState("dlv_r1", Dismissed)
	if n := h.ib.IgnoredRows(); n != 1 {
		t.Fatalf("ignored %d requests, want the agent's one", n)
	}
	// After a restart nothing is decided twice.
	cancel()
	h.reopen()
	ctx2, cancel2 := contextWithCancel(t)
	defer cancel2()
	go h.ib.Follow(ctx2)
	time.Sleep(50 * time.Millisecond)
	h.wantState("dlv_r1", Dismissed)
}

// Decision 34 on the host: a non-bypass (or unknown-mode) Claude agent on
// another of the person's certified devices reaches a bypass session here
// without a hold. A Grok member, an agent on a revoked device and one on a
// device never certified are still held; the block list and Pause all
// still win.
func TestTheHostLetsTheOwnersOtherDeviceAgentsThroughBypass(t *testing.T) {
	const keyRev = "dev3/dddddddd-aaaa-4aaa-8aaa-aaaaaaaaaaaa" // dev3 was removed
	const keyUnk = "devX/eeeeeeee-bbbb-4bbb-8bbb-bbbbbbbbbbbb" // never certified
	const keyRG = "dev2/grok-remote"
	h := newHarness(t)
	h.isOwner = func(k string) bool { return k == user || k == "user:dev2" || k == "user:dev3" }
	h.revoked = func(dev string) bool { return dev == "dev3" }
	h.reopen()
	h.abc() // alpha runs in bypass
	h.ros.put(card(keyR, "mac", "", ""), card(keyRev, "old", "", "default"), card(keyUnk, "stranger", "", "default"), grokCard(keyRG, "grok", ""))
	ctx, cancel := contextWithCancel(t)
	defer cancel()
	go h.ib.Follow(ctx)
	f := &foreign{h: h}
	send := func(id string, from Sender) {
		m := Message{ID: "msg_" + id, From: from, To: to(keyA), Text: "hi from " + id, Trust: Untrusted, SentAt: h.clk.now()}
		f.row("agent:"+from.Key, KindMessageSent, m)
		f.row("agent:"+from.Key, KindDelivery, deliveryRow{DeliveryID: "dlv_" + id, MessageID: m.ID, To: keyA, State: Queued, By: from.Key})
	}
	send("own", claude(keyR, "mac"))
	send("grok", grok(keyRG, "grok"))
	send("revoked", claude(keyRev, "old"))
	send("unknown", claude(keyUnk, "stranger"))
	for _, id := range []string{"dlv_grok", "dlv_revoked", "dlv_unknown"} {
		eventually(t, id+" held for bypass", func() bool {
			d, ok := h.ib.Get(id)
			return ok && d.State == Held && d.Hold == HoldBypassReceiver
		})
	}
	if d, _ := h.ib.Get("dlv_own"); d.State == Held || d.Hold != "" {
		t.Fatalf("own agent on another device: %s %q", d.State, d.Hold)
	}

	if err := h.ib.SetPolicy(bg, Policy{Block: []string{"mac"}}, user); err != nil {
		t.Fatal(err)
	}
	send("blocked", claude(keyR, "mac"))
	eventually(t, "the block list wins", func() bool {
		d, ok := h.ib.Get("dlv_blocked")
		return ok && d.State == Held && d.Hold == HoldBlocked
	})
	if err := h.ib.SetPolicy(bg, Policy{Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	send("paused", claude(keyR, "mac"))
	eventually(t, "Pause all wins", func() bool {
		d, ok := h.ib.Get("dlv_paused")
		return ok && d.State == Held && d.Hold == HoldPaused
	})
}
