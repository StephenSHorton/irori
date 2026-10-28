package inbox

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// The person is one person on every device (v0.4 §2): their key on another
// certified device gets every person exemption; a made-up "user:" key gets
// none.
func TestOwnerKeysAndMadeUpKeys(t *testing.T) {
	h := newHarness(t)
	h.isOwner = func(k string) bool { return k == user || k == "user:dev2" }
	h.reopen()
	h.abc()
	if err := h.ib.SetPolicy(bg, Policy{Mode: Allowlist}, user); err != nil {
		t.Fatal(err)
	}
	other := Sender{Key: "user:dev2", Name: "you", Kind: "user"}
	fake := Sender{Key: "user:anything", Name: "you", Kind: "user"}

	// Policy: the person skips the allowlist; the made-up key is held.
	m, d := h.send(other, to("alpha"), "from the Mac")
	if d.State == Held || m.Trust != UserTrust {
		t.Fatalf("person on another device: state %s trust %s", d.State, m.Trust)
	}
	m, d = h.send(fake, to("alpha"), "trust me")
	if d.State != Held || d.Hold != HoldNotAllowed || m.Trust != Untrusted {
		t.Fatalf("made-up user key: state %s hold %s trust %s", d.State, d.Hold, m.Trust)
	}

	// Rate limit: the person is exempt; the made-up key counts.
	if err := h.ib.SetPolicy(bg, Policy{Mode: Everyone}, user); err != nil {
		t.Fatal(err)
	}
	for range DirectLimit + 2 {
		h.send(other, to("beta"), "again")
	}
	for i := range DirectLimit {
		if _, _, err := h.ib.Send(bg, fake, to("beta"), "again", ""); err != nil {
			t.Fatalf("send %d: %v", i, err)
		}
	}
	_, _, err := h.ib.Send(bg, fake, to("beta"), "one more", "")
	wantErr(t, err, ErrRateLimited)

	// Only the owner rule makes "user" rows: a made-up key is an agent actor.
	if a := h.ib.impl.actorFor("user:dev2"); a != "user" {
		t.Fatalf("actor for another device's owner key = %q", a)
	}
	if a := h.ib.impl.actorFor("user:anything"); a != "agent:user:anything" {
		t.Fatalf("actor for a made-up key = %q", a)
	}
	if a := h.ib.impl.actorFor("api:composer@" + keyA); a != "api:composer@"+keyA {
		t.Fatalf("actor for the API = %q", a)
	}
}

// Policy rows are device-local: written to local_events and replayed from
// there.
func TestPolicyIsDeviceLocal(t *testing.T) {
	h := newHarness(t)
	if err := h.ib.SetPolicy(bg, Policy{Mode: Allowlist, Allow: []string{"alpha"}, Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	if evs, _ := h.st.List(bg, store.Filter{Kinds: []string{KindPolicySet}}); len(evs) != 0 {
		t.Fatalf("policy.set in the replicated log: %+v", evs)
	}
	if evs, _ := h.st.List(bg, store.Filter{Kinds: []string{KindPolicySet}, Local: true}); len(evs) != 1 {
		t.Fatalf("local policy rows %d, want 1", len(evs))
	}
	h.reopen()
	if p := h.ib.Policy(); p.Mode != Allowlist || !p.Paused || len(p.Allow) != 1 {
		t.Fatalf("policy after replay %+v", p)
	}
}

// Writer rules (v0.4 §6.0): after its first row, only the device that owns a
// delivery (the one its To names) writes its delivery.state, and only the
// woken agent's device writes wake.issued and turn rows. Others are ignored
// and counted, live and on replay.
func TestOwnerOnlyRowsFromTheWrongDevice(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "hi")
	at := hlc.Timestamp(h.clk.now().Add(time.Hour).UnixMilli()) << 16
	foreign := func(seq int64, kind string, payload any) {
		t.Helper()
		raw, _ := json.Marshal(payload)
		if err := h.st.Insert(bg, store.Event{DeviceID: "dev2", Seq: seq, HLC: at + hlc.Timestamp(seq), Actor: "irori", Kind: kind, Payload: raw}); err != nil {
			t.Fatal(err)
		}
	}
	foreign(1, KindDelivery, deliveryRow{DeliveryID: d.ID, MessageID: d.MessageID, To: keyB, State: Dismissed, By: "dev2/x"})
	foreign(2, KindWakeIssued, wakeRow{WakeID: "w_forged", To: keyB, Token: "[irori-wake:forged]", DeliveryIDs: []string{d.ID}})
	foreign(3, KindTurnEngaged, turnRow{Agent: keyB, WakeID: "w_forged", TurnID: "t"})
	// A first row for a new delivery is the sender's to write: accepted.
	foreign(4, KindDelivery, deliveryRow{DeliveryID: "del_from_dev2", MessageID: d.MessageID, To: keyB, State: Queued, By: "dev2/x"})

	h.reopen()
	got, ok := h.ib.Get(d.ID)
	if !ok || got.State != Queued || got.WakeID == "w_forged" {
		t.Fatalf("delivery after wrong-device rows: %+v", got)
	}
	if n := h.ib.IgnoredRows(); n != 3 {
		t.Fatalf("ignored %d rows, want 3", n)
	}
	if _, ok := h.ib.Get("del_from_dev2"); !ok {
		t.Fatal("a new delivery's first row from the sender's device was dropped")
	}
}
