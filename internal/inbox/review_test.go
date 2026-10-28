package inbox

import (
	"math"
	"testing"
	"time"
)

// Regression tests for the wave-1 review findings.

// The allow-list matches a sender's key or its card name, never the name it
// chose to send under.
func TestAllowListIgnoresSelfChosenName(t *testing.T) {
	// A Claude session on a device not certified to the person: not one of
	// the person's own agents, so the bypass hold still applies (decision 34).
	const keyN = "devX/nnnnnnnn-5555-4555-8555-555555555555"
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""), card(keyN, "normal", "", "default"))
	if err := h.ib.SetPolicy(bg, Policy{Allow: []string{"gamma"}}, user); err != nil {
		t.Fatal(err)
	}
	for _, from := range []Sender{grok(keyG, "gamma"), claude(keyN, "gamma")} {
		if _, d := h.send(from, to("alpha"), "spoof"); d.State != Held || d.Hold != HoldBypassReceiver {
			t.Fatalf("%s as gamma: state %s hold %q, want held bypass_receiver", from.Key, d.State, d.Hold)
		}
	}
	// An alias recorded from an earlier send doesn't count either.
	if _, d := h.send(grok(keyG, "gamma"), to("alpha"), "again"); d.Hold != HoldBypassReceiver {
		t.Fatalf("alias let it through: hold %q", d.Hold)
	}
	// The card name still allows, whatever name the sender uses.
	if err := h.ib.SetPolicy(bg, Policy{Mode: Allowlist, Allow: []string{"GROK"}}, user); err != nil {
		t.Fatal(err)
	}
	if _, d := h.send(grok(keyG, "someone"), to("alpha"), "ok"); d.State != Queued {
		t.Fatalf("card name not allowed: %s %q", d.State, d.Hold)
	}
	if _, d := h.send(claude(keyN, "grok"), to("alpha"), "spoof"); d.Hold != HoldNotAllowed {
		t.Fatalf("allowlist spoof: hold %q", d.Hold)
	}
	// Block still matches the names a sender used (stricter is fine).
	if err := h.ib.SetPolicy(bg, Policy{Block: []string{"gamma"}}, user); err != nil {
		t.Fatal(err)
	}
	if _, d := h.send(grok(keyG, "grok"), to("beta"), "x"); d.Hold != HoldBlocked {
		t.Fatalf("block by alias: hold %q", d.Hold)
	}
}

// A name another agent sent under never takes messages from the agent whose
// card carries that name, live or gone.
func TestAliasCannotHijackCardName(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	h.send(grok(keyG, "beta"), to("alpha"), "I'm beta now")
	if _, d := h.send(claude(keyA, "alpha"), to("beta"), "live"); d.To != keyB {
		t.Fatalf("live beta: routed to %s", d.To)
	}
	h.ros.remove(keyB)
	if _, d := h.send(claude(keyA, "alpha"), to("beta"), "gone"); d.To != keyB || d.State != Queued {
		t.Fatalf("gone beta: routed to %s (%s)", d.To, d.State)
	}
	// The alias still reaches its owner when no card has the name.
	h.send(grok(keyG, "zeta"), to("alpha"), "x")
	if _, d := h.send(claude(keyA, "alpha"), to("zeta"), "y"); d.To != keyG {
		t.Fatalf("alias routed to %s", d.To)
	}
}

// The recipient never sees a held (or dismissed) message's text through For.
func TestHeldNotShownToRecipient(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	_, held := h.send(grok(keyG, "grok"), to("alpha"), "SECRET held text")
	_, dis := h.send(grok(keyG, "grok"), to("alpha"), "SECRET dismissed text")
	if err := h.ib.Dismiss(bg, dis.ID, user); err != nil {
		t.Fatal(err)
	}
	_, ok := h.send(claude(keyB, "beta"), to("alpha"), "fine")
	for _, all := range []bool{false, true} {
		got := h.ib.For(keyA, all, 0)
		if len(got) != 1 || got[0].ID != ok.ID {
			t.Fatalf("For(all=%v) = %+v", all, got)
		}
	}
	// The GUI still sees both.
	seen := map[string]bool{}
	for _, d := range h.ib.List(0) {
		seen[d.ID] = true
	}
	if !seen[held.ID] || !seen[dis.ID] {
		t.Fatal("List lost held deliveries")
	}
	// Released, it shows up.
	if err := h.ib.Release(bg, held.ID, user); err != nil {
		t.Fatal(err)
	}
	if got := h.ib.For(keyA, false, 0); len(got) != 2 {
		t.Fatalf("released not listed: %+v", got)
	}
}

// Repo work never lands on its own sender: not on first assignment by the
// sweeper, not on Release, not on a move.
func TestRepoWorkNeverAssignedToSender(t *testing.T) {
	t.Run("assign and release", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		h.ib.SetReachable(keyA, PathMod, true)
		_, d1 := h.send(claude(keyA, "alpha"), toRepo("repo1"), "one")
		_, d2 := h.send(claude(keyA, "alpha"), toRepo("repo1"), "two")
		if d1.Hold != HoldUnowned {
			t.Fatalf("hold %q", d1.Hold)
		}
		h.sweep()
		if got := h.wantState(d1.ID, Held); got.To != "" {
			t.Fatalf("sweep assigned to %s", got.To)
		}
		wantErr(t, h.ib.Release(bg, d2.ID, user), ErrBadState)
		if got := h.state(d2.ID); got.To != "" {
			t.Fatalf("release assigned to %s", got.To)
		}
		h.ib.SetReachable(keyB, PathMod, true)
		if err := h.ib.Release(bg, d2.ID, user); err != nil {
			t.Fatal(err)
		}
		if got := h.wantState(d2.ID, Queued); got.To != keyB {
			t.Fatalf("released to %s", got.To)
		}
		h.sweep()
		if got := h.wantState(d1.ID, Queued); got.To != keyB {
			t.Fatalf("assigned to %s", got.To)
		}
	})
	t.Run("move when the holder goes", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		h.ib.SetReachable(keyA, PathMod, true)
		h.clk.advance(time.Second)
		h.ib.SetReachable(keyB, PathMod, true)
		_, d := h.send(claude(keyA, "alpha"), toRepo("repo1"), "work")
		if d.To != keyB {
			t.Fatalf("desk %s", d.To)
		}
		h.ros.remove(keyB)
		h.sweep()
		h.clk.advance(GoneGrace + time.Second)
		h.sweep()
		old := h.wantState(d.ID, Moved)
		if old.MovedTo != "" {
			t.Fatalf("moved to %s", old.MovedTo)
		}
		for _, x := range h.ib.List(0) {
			if x.MessageID == d.MessageID && x.ID != d.ID && (x.To != "" || x.Hold != HoldUnowned) {
				t.Fatalf("new delivery to %s (%s %q)", x.To, x.State, x.Hold)
			}
		}
	})
	t.Run("move when the holder doesn't start", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		h.ib.SetReachable(keyA, PathMod, true)
		h.clk.advance(time.Second)
		h.ib.SetReachable(keyB, PathMod, true)
		_, d := h.send(claude(keyA, "alpha"), toRepo("repo1"), "work")
		h.clk.advance(UnengagedLimit + time.Second)
		h.sweep()
		if got := h.wantState(d.ID, Queued); got.To != keyB {
			t.Fatalf("moved to %s", got.To)
		}
	})
}

// Agent work that was delivered but never engaged gets one reminder.
func TestDeliveredNeverEngagedIsReminded(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	w := h.wake(keyB)
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	h.clk.advance(UnengagedLimit)
	h.sweep()
	h.noWake(keyB)
	h.clk.advance(time.Second)
	h.sweep()
	r := h.wake(keyB)
	if !r.Reminder || len(r.DeliveryIDs) != 1 || r.DeliveryIDs[0] != d.ID {
		t.Fatalf("reminder %+v", r)
	}
	h.wantState(d.ID, Delivered)
	// Once.
	for range 48 {
		h.clk.advance(30 * time.Minute)
		h.sweep()
		h.noWake(keyB)
	}
}

func TestDeferClampsHugeMinutes(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	for _, m := range []int{math.MaxInt, 153722867280913, int(MaxDefer/time.Minute) + 1} {
		if err := h.ib.Defer(bg, keyB, d.ID, m, ""); err != nil {
			t.Fatal(err)
		}
		if got := h.state(d.ID); !got.Until.Equal(h.clk.now().Add(MaxDefer)) {
			t.Fatalf("minutes %d: until %v, want %v", m, got.Until, h.clk.now().Add(MaxDefer))
		}
	}
}

// A card rename is in the log, so a gone agent's new name resolves the same
// before and after a restart.
func TestRenameSurvivesReplay(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.send(claude(keyB, "beta"), to("alpha"), "x")
	h.ros.put(card(keyA, "alpha2", "repo1", BypassMode))
	h.send(claude(keyB, "beta"), to("alpha2"), "y")
	h.ros.remove(keyA)
	check := func(when string) {
		t.Helper()
		_, d, err := h.ib.Send(bg, claude(keyB, "beta"), to("alpha2"), "z", "")
		if err != nil || d.To != keyA {
			t.Fatalf("%s: %v %s", when, err, d.To)
		}
		_, _, err = h.ib.Send(bg, claude(keyB, "beta"), to("alpha"), "z", "")
		wantErr(t, err, ErrUnknownAgent)
	}
	check("before restart")
	h.reopen()
	check("after restart")
}
