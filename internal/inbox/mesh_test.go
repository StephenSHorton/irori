package inbox

import (
	"context"
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

// A reply is about what its message was about: a card ref (task: or
// comment:) carries over, in the message.sent row other devices fold.
func TestReplyInheritsRef(t *testing.T) {
	h := newHarness(t)
	h.abc()
	var replies []string
	for _, ref := range []string{"task:T1", "comment:C1"} {
		d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "about "+ref, SendOptions{Mode: ModeWake, Ref: ref})
		m, nd, err := h.ib.Reply(bg, claude(keyA, "alpha"), d.ID, "on it")
		if err != nil {
			t.Fatal(err)
		}
		if m.Ref != ref || nd.Message.Ref != ref || nd.To != keyC {
			t.Fatalf("reply to %s: ref %q / %q, to %s", ref, m.Ref, nd.Message.Ref, nd.To)
		}
		replies = append(replies, nd.ID)
	}
	evs, err := h.st.List(bg, store.Filter{Kinds: []string{KindMessageSent}})
	if err != nil {
		t.Fatal(err)
	}
	refs := map[string]string{}
	for _, ev := range evs {
		var m Message
		if err := json.Unmarshal(ev.Payload, &m); err != nil {
			t.Fatal(err)
		}
		if m.ReplyTo != "" {
			refs[m.Text+" "+m.Ref] = m.ReplyTo
		}
	}
	if len(refs) != 2 || refs["on it task:T1"] == "" || refs["on it comment:C1"] == "" {
		t.Fatalf("message.sent rows carry %v", refs)
	}
	h.reopen()
	for i, ref := range []string{"task:T1", "comment:C1"} {
		if got := h.state(replies[i]); got.Message.Ref != ref {
			t.Fatalf("after replay the reply's ref is %q, want %q", got.Message.Ref, ref)
		}
	}
	// No ref stays no ref.
	d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "plain", SendOptions{Mode: ModeWake})
	if m, _, err := h.ib.Reply(bg, claude(keyA, "alpha"), d.ID, "ok"); err != nil || m.Ref != "" {
		t.Fatalf("plain reply ref %q, %v", m.Ref, err)
	}
}

// The host lets another device's message about a card through its own
// policy: the sender recorded it held, so its text goes on the card from
// here (Options.OnPassed). One it holds goes once the person releases it.
func TestTheHostFillsTheCardWhenItLetsAMessageThrough(t *testing.T) {
	h := newHarness(t)
	var calls []string
	h.onPassed = func(_ context.Context, m Message, to string) error {
		calls = append(calls, m.ID+" to "+to)
		return nil
	}
	h.reopen()
	h.ros.put(card(keyB, "beta", "repo1", ""))
	if err := h.ib.SetPolicy(bg, Policy{Block: []string{"dev2/blocked"}}, user); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := contextWithCancel(t)
	defer cancel()
	go h.ib.Follow(ctx)
	f := &foreign{h: h}
	for _, from := range []string{keyR, "dev2/blocked"} {
		m := Message{ID: "msg_" + sessionPart(from)[:4], From: Sender{Key: from, Name: "remote", Kind: "claude"}, To: to(keyB), Text: "about T1", Ref: "task:T1", Trust: Untrusted, SentAt: h.clk.now()}
		f.row("agent:"+from, KindMessageSent, m)
		f.row("agent:"+from, KindDelivery, deliveryRow{DeliveryID: "dlv_" + m.ID, MessageID: m.ID, To: keyB, State: Queued, By: from})
	}
	eventually(t, "the host holds the blocked one", func() bool {
		d, ok := h.ib.Get("dlv_msg_bloc")
		return ok && d.State == Held
	})
	h.sweep()
	if len(calls) != 1 || calls[0] != "msg_rrrr to "+keyB {
		t.Fatalf("calls %v", calls)
	}
	if err := h.ib.Release(bg, "dlv_msg_bloc", user); err != nil {
		t.Fatal(err)
	}
	if len(calls) != 2 || calls[1] != "msg_bloc to "+keyB {
		t.Fatalf("calls %v", calls)
	}
}
