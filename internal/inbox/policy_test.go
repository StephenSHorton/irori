package inbox

import (
	"testing"
)

func TestPolicyHolds(t *testing.T) {
	const keyN = "dev1/nnnnnnnn-5555-4555-8555-555555555555" // a default-mode session
	const keyU = "dev1/uuuuuuuu-6666-4666-8666-666666666666" // permission mode unknown
	const keyX = "devX/xxxxxxxx-7777-4777-8777-777777777777" // a device not certified to the person
	tests := []struct {
		name   string
		policy Policy
		from   Sender
		to     string
		want   HoldReason
	}{
		{"bypass to bypass flows", Policy{}, claude(keyA, "alpha"), keyB, ""},
		{"grok to a bypass receiver is held", Policy{}, grok(keyG, "grok"), keyB, HoldBypassReceiver},
		// Decision 34: the person's own Claude agents skip the bypass hold
		// whatever their mode; a Claude key on an uncertified device doesn't.
		{"own default-mode sender to bypass flows", Policy{}, claude(keyN, "normal"), keyB, ""},
		{"own unknown-mode sender to bypass flows", Policy{}, claude(keyU, "unknown"), keyB, ""},
		{"unknown device's sender to bypass is held", Policy{}, claude(keyX, "stranger"), keyB, HoldBypassReceiver},
		{"grok under a claude key's device is held", Policy{}, grok("dev1/grok-other", "grok2"), keyB, HoldBypassReceiver},
		{"a grok card sending as claude is held", Policy{}, claude("dev1/grok-other", "grok2"), keyB, HoldBypassReceiver},
		{"own sender still blocked", Policy{Block: []string{"normal"}}, claude(keyN, "normal"), keyB, HoldBlocked},
		{"own sender still paused", Policy{Paused: true}, claude(keyN, "normal"), keyB, HoldPaused},
		{"own sender still needs the allowlist", Policy{Mode: Allowlist}, claude(keyN, "normal"), keyB, HoldNotAllowed},
		{"to an unknown-mode receiver flows", Policy{}, grok(keyG, "grok"), keyU, ""},
		{"to grok flows", Policy{}, claude(keyN, "normal"), keyG, ""},
		{"allow-listed grok reaches bypass", Policy{Allow: []string{"GROK"}}, grok(keyG, "grok"), keyB, ""},
		{"the person reaches bypass", Policy{}, person(), keyB, ""},
		{"blocked by key", Policy{Block: []string{keyA}}, claude(keyA, "alpha"), keyB, HoldBlocked},
		{"blocked by name, any case", Policy{Block: []string{"Alpha"}}, claude(keyA, "alpha"), keyB, HoldBlocked},
		{"block beats allow", Policy{Block: []string{"alpha"}, Allow: []string{"alpha"}}, claude(keyA, "alpha"), keyB, HoldBlocked},
		{"the person can't be blocked", Policy{Block: []string{user, "stephen"}}, person(), keyB, ""},
		{"allowlist without the sender", Policy{Mode: Allowlist}, claude(keyA, "alpha"), keyB, HoldNotAllowed},
		{"allowlist with the sender", Policy{Mode: Allowlist, Allow: []string{keyA}}, claude(keyA, "alpha"), keyB, ""},
		{"allowlist and the person", Policy{Mode: Allowlist}, person(), keyB, ""},
		{"paused holds agents", Policy{Paused: true}, claude(keyA, "alpha"), keyB, HoldPaused},
		{"paused holds the person too", Policy{Paused: true}, person(), keyB, HoldPaused},
		{"blocked while paused stays blocked", Policy{Paused: true, Block: []string{"alpha"}}, claude(keyA, "alpha"), keyB, HoldBlocked},
		{"Irori reaches bypass", Policy{}, SystemSender(), keyB, ""},
		{"Irori passes allowlist mode", Policy{Mode: Allowlist}, SystemSender(), keyB, ""},
		{"Irori blocked by key", Policy{Block: []string{SystemKey}}, SystemSender(), keyB, HoldBlocked},
		{"Irori blocked by name", Policy{Block: []string{"IRORI"}}, SystemSender(), keyB, HoldBlocked},
		{"paused holds Irori", Policy{Paused: true}, SystemSender(), keyB, HoldPaused},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			h := newHarness(t)
			h.abc()
			h.ros.put(grokCard(keyG, "grok", ""), card(keyN, "normal", "", "default"), card(keyU, "unknown", "", ""),
				card(keyX, "stranger", "", "default"), grokCard("dev1/grok-other", "grok2", ""))
			if err := h.ib.SetPolicy(bg, tc.policy, user); err != nil {
				t.Fatal(err)
			}
			_, d, err := h.ib.Send(bg, tc.from, to(tc.to), "hi", "")
			if err != nil {
				t.Fatal(err)
			}
			if d.Hold != tc.want || (tc.want == "") != (d.State == Queued) || (tc.want != "") != (d.State == Held) {
				t.Fatalf("state %s hold %q, want hold %q", d.State, d.Hold, tc.want)
			}
		})
	}
}

func TestPolicyAliasBlockAndValidation(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.send(claude(keyA, "Old Name"), to("beta"), "x") // alias "old name"
	if err := h.ib.SetPolicy(bg, Policy{Block: []string{" old name ", "", "old name"}}, user); err != nil {
		t.Fatal(err)
	}
	if p := h.ib.Policy(); len(p.Block) != 1 || p.Mode != Everyone || p.Allow == nil {
		t.Fatalf("policy %+v", p)
	}
	if _, d := h.send(claude(keyA, "new name"), to("beta"), "x"); d.Hold != HoldBlocked {
		t.Fatalf("hold %q", d.Hold)
	}
	if err := h.ib.SetPolicy(bg, Policy{Mode: "sometimes"}, user); err == nil {
		t.Fatal("bad mode accepted")
	}
	// Policy returns a copy.
	p := h.ib.Policy()
	p.Block[0] = "changed"
	if h.ib.Policy().Block[0] != "old name" {
		t.Fatal("policy aliased")
	}
}

func TestReleaseDismissAndUnpause(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))

	// Release bypasses the policy; Dismiss closes; both only from held.
	_, d1 := h.send(grok(keyG, "grok"), to("beta"), "held 1")
	_, d2 := h.send(grok(keyG, "grok"), to("beta"), "held 2")
	if err := h.ib.Release(bg, d1.ID, user); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d1.ID, Queued); got.Hold != "" {
		t.Fatalf("hold %q kept", got.Hold)
	}
	wantErr(t, h.ib.Release(bg, d1.ID, user), ErrBadState)
	wantErr(t, h.ib.Release(bg, "dlv_nope", user), ErrNotFound)
	if err := h.ib.Dismiss(bg, d2.ID, user); err != nil {
		t.Fatal(err)
	}
	h.wantState(d2.ID, Dismissed)
	if err := h.ib.Dismiss(bg, d2.ID, user); err != nil { // idempotent
		t.Fatal(err)
	}
	wantErr(t, h.ib.Dismiss(bg, d1.ID, user), ErrBadState)
	wantErr(t, h.ib.Dismiss(bg, "dlv_nope", user), ErrNotFound)

	// Pause: everything new is held and nothing wakes; unpause re-runs the
	// rest of the policy over paused items.
	if err := h.ib.SetPolicy(bg, Policy{Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	h.noWake(keyB) // d1 is queued but paused
	_, p1 := h.send(claude(keyA, "alpha"), to("beta"), "paused 1")
	_, p2 := h.send(claude(keyC, "gamma"), to("beta"), "paused 2")
	if p1.Hold != HoldPaused || p2.Hold != HoldPaused {
		t.Fatal(p1.Hold, p2.Hold)
	}
	if err := h.ib.SetPolicy(bg, Policy{Block: []string{"gamma"}}, user); err != nil {
		t.Fatal(err)
	}
	h.wantState(p1.ID, Queued)
	if got := h.wantState(p2.ID, Held); got.Hold != HoldBlocked {
		t.Fatalf("hold %q", got.Hold)
	}
	w := h.wake(keyB)
	if len(w.DeliveryIDs) != 2 || w.DeliveryIDs[0] != d1.ID || w.DeliveryIDs[1] != p1.ID {
		t.Fatalf("wake %v", w.DeliveryIDs)
	}
}
