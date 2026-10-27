package inbox

import (
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
)

// Synthetic PR fixtures: repo Acme/App, PR 7.
const (
	prKey   = "pr:acme/app#7"
	prRepo  = "remote:github.com/acme/app"
	keyD    = "dev1/dddddddd-4444-4444-8444-444444444444" // the repo's front desk
	keyS    = "dev1/ssssssss-5555-4555-8555-555555555555" // a sender elsewhere
	keyGone = "dev1/eeeeeeee-6666-4666-8666-666666666666" // an owner never seen here
)

func toPR(v string) Address { return Address{Kind: ToPR, Value: v} }

// prHarness has owners for prKey and a roster with the sender s; withDesk
// adds a reachable front desk d in the PR's repo.
func prHarness(t *testing.T, owners []string, withDesk bool) *harness {
	t.Helper()
	h := &harness{
		t: t, dir: t.TempDir(),
		ros: &fakeRoster{cards: map[string]presence.AgentCard{}},
		clk: &fakeClock{t: time.Date(2026, 10, 26, 9, 0, 0, 0, time.UTC)},
	}
	if owners != nil {
		h.owners = func(k string) []string {
			if k == prKey {
				return owners
			}
			return nil
		}
	}
	h.open()
	h.ros.put(card(keyS, "sigma", "repo9", BypassMode))
	if withDesk {
		h.ros.put(card(keyD, "delta", prRepo, BypassMode))
		h.ib.SetReachable(keyD, PathMod, true)
	}
	return h
}

func TestNormalizePR(t *testing.T) {
	tests := []struct{ in, want string }{
		{"pr:acme/app#7", prKey},
		{" PR:Acme/App#7 ", prKey},
		{"Acme/App#7", prKey},
		{"https://github.com/Acme/App/pull/7", prKey},
		{"github.com/acme/app/pull/7/files", prKey},
		{"http://www.github.com/acme/app/pull/7?w=1", prKey},
	}
	for _, tc := range tests {
		got, err := NormalizePR(tc.in)
		if err != nil || got != tc.want {
			t.Errorf("NormalizePR(%q) = %q, %v; want %q", tc.in, got, err, tc.want)
		}
	}
	for _, bad := range []string{"", " ", "7", "#7", "app#7", "pr:acme/app", "pr:acme/app#0", "acme/app#x",
		"https://github.com/acme/app/issues/7", "https://gitlab.com/acme/app/pull/7", "pr:../app#7"} {
		_, err := NormalizePR(bad)
		if !errors.Is(err, ErrBadPR) {
			t.Errorf("NormalizePR(%q) err = %v, want ErrBadPR", bad, err)
		}
	}
}

func TestPRRouting(t *testing.T) {
	tests := []struct {
		name     string
		owners   []string // best first, as the ledger ranks them
		live     []string // owners in the roster
		goneCard []string // owners whose card says gone
		desk     bool
		from     string
		wantTo   string
		wantRepo string
		wantNote []string // substrings; nil = no note
	}{
		{name: "live author", owners: []string{keyA, keyB}, live: []string{keyA, keyB}, desk: true, wantTo: keyA},
		{name: "author gone, toucher live", owners: []string{keyA, keyB}, live: []string{keyB}, desk: true, wantTo: keyB},
		{name: "order is the ledger's: reviewer ranked first wins", owners: []string{keyC, keyB}, live: []string{keyB, keyC}, wantTo: keyC},
		{name: "a gone card isn't live", owners: []string{keyA, keyB}, live: []string{keyB}, goneCard: []string{keyA}, wantTo: keyB},
		{name: "owner is not reachable but live: still the owner", owners: []string{keyB}, live: []string{keyB}, desk: true, wantTo: keyB},
		{name: "all owners gone: desk, note names the author", owners: []string{keyA, keyB}, desk: true,
			wantTo: keyD, wantRepo: prRepo, wantNote: []string{"alpha (" + keyA + ")", prKey}},
		{name: "owner never seen: note has its key", owners: []string{keyGone}, desk: true,
			wantTo: keyD, wantRepo: prRepo, wantNote: []string{keyGone}},
		{name: "no owners: desk, no note", owners: []string{}, desk: true, wantTo: keyD, wantRepo: prRepo},
		{name: "no ledger (nil owners): desk", desk: true, wantTo: keyD, wantRepo: prRepo},
		{name: "sender owns it: passed over", owners: []string{keyA, keyB}, live: []string{keyA, keyB}, from: keyA, wantTo: keyB},
		{name: "sender is the only owner and the desk: unowned", owners: []string{keyD}, desk: true, from: keyD, wantRepo: prRepo},
		{name: "no desk: unowned with a note", owners: []string{keyA}, wantRepo: prRepo, wantNote: []string{"alpha"}},
	}
	names := map[string]string{keyA: "alpha", keyB: "beta", keyC: "gamma"}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			h := prHarness(t, tc.owners, tc.desk) // nil owners = no ledger
			// Every named owner was seen once, so its name is known.
			for k, n := range names {
				h.ros.put(card(k, n, "repo1", BypassMode))
			}
			h.send(claude(keyS, "sigma"), to("alpha"), "hello") // records names
			h.ros.remove(keyA, keyB, keyC)
			for _, k := range tc.live {
				h.ros.put(card(k, names[k], "repo1", BypassMode))
			}
			for _, k := range tc.goneCard {
				c := card(k, names[k], "repo1", BypassMode)
				c.Status = "gone"
				h.ros.put(c)
			}
			from := claude(keyS, "sigma")
			if tc.from != "" {
				from = claude(tc.from, "owner")
			}
			m, d, err := h.ib.Send(bg, from, toPR("https://github.com/Acme/App/pull/7"), "is this ready?", "")
			if err != nil {
				t.Fatal(err)
			}
			if m.To.Kind != ToPR || m.To.Value != prKey {
				t.Fatalf("message to %+v, want normalized %s", m.To, prKey)
			}
			if d.To != tc.wantTo || d.RepoKey != tc.wantRepo {
				t.Fatalf("routed to %q repo %q, want %q repo %q", d.To, d.RepoKey, tc.wantTo, tc.wantRepo)
			}
			wantState, wantHold := Queued, HoldReason("")
			if tc.wantTo == "" {
				wantState, wantHold = Held, HoldUnowned
			}
			if d.State != wantState || d.Hold != wantHold {
				t.Fatalf("state %s hold %q", d.State, d.Hold)
			}
			if tc.wantNote == nil && d.Note != "" {
				t.Fatalf("unexpected note %q", d.Note)
			}
			for _, s := range tc.wantNote {
				if !strings.Contains(d.Note, s) {
					t.Fatalf("note %q lacks %q", d.Note, s)
				}
			}
		})
	}
}

func TestPRInvalidTargets(t *testing.T) {
	h := prHarness(t, nil, true)
	for _, bad := range []string{"", "7", "acme#7", "pr:acme", "https://github.com/acme/app/issues/7"} {
		_, _, err := h.ib.Send(bg, claude(keyS, "sigma"), toPR(bad), "x", "")
		wantErr(t, err, ErrBadPR)
		if bad != "" && !strings.Contains(err.Error(), "pr:<owner>/<name>#<number>") {
			t.Errorf("error doesn't say the forms: %v", err)
		}
	}
	if len(h.ib.List(0)) != 0 {
		t.Fatal("a bad PR send recorded a delivery")
	}
}

// Unowned PR work waits, survives a restart without the ledger, and goes to
// the first desk that appears, keeping its note.
func TestPRUnownedThenDesk(t *testing.T) {
	h := prHarness(t, []string{keyGone}, false)
	_, u := h.send(claude(keyS, "sigma"), toPR("acme/app#7"), "who has this?")
	if u.To != "" || u.State != Held || u.Hold != HoldUnowned || u.RepoKey != prRepo || !strings.Contains(u.Note, keyGone) {
		t.Fatalf("unowned %+v", u)
	}
	wantErr(t, h.ib.Release(bg, u.ID, user), ErrBadState)

	// Replay needs no ledger: the routing is in the rows.
	h.owners = nil
	h.reopen()
	got := h.wantState(u.ID, Held)
	if got.RepoKey != prRepo || got.Note != u.Note || got.Message.To.Value != prKey {
		t.Fatalf("after replay %+v", got)
	}

	h.ros.put(card(keyD, "delta", prRepo, BypassMode))
	h.ib.SetReachable(keyD, PathMod, true)
	h.sweep()
	got = h.wantState(u.ID, Queued)
	if got.To != keyD || got.Note != u.Note {
		t.Fatalf("assigned %+v", got)
	}
	w := h.wake(keyD)
	if !strings.Contains(w.Prompt, "who has this?") {
		t.Fatalf("wake lacks the text: %s", w.Prompt)
	}
}

// Desk-routed PR work whose desk goes moves on like repo work and keeps the
// note naming its owner.
func TestPRDeskGoneKeepsNote(t *testing.T) {
	h := prHarness(t, []string{keyGone}, true)
	_, d := h.send(claude(keyS, "sigma"), toPR(prKey), "who has this?")
	if d.To != keyD || d.Note == "" {
		t.Fatalf("routed %+v", d)
	}
	h.ros.remove(keyD)
	h.sweep()
	h.clk.advance(GoneGrace + time.Second)
	h.sweep()
	moved := h.wantState(d.ID, Moved)
	var next Delivery
	for _, x := range h.ib.List(0) {
		if x.MessageID == d.MessageID && x.ID != d.ID {
			next = x
		}
	}
	if moved.MovedTo != "" || next.State != Held || next.Hold != HoldUnowned || next.Note != d.Note || next.RepoKey != prRepo {
		t.Fatalf("moved %+v, next %+v", moved, next)
	}
}

// Owner-routed work is the owner's, like an agent-addressed message: the
// sweeper doesn't move it to the desk when it sits unengaged.
func TestPROwnerRoutedStaysPut(t *testing.T) {
	h := prHarness(t, []string{keyA}, true)
	h.ros.put(card(keyA, "alpha", prRepo, BypassMode))
	_, d := h.send(claude(keyS, "sigma"), toPR("pr:acme/app#7"), "rebase please")
	if d.To != keyA || d.RepoKey != "" {
		t.Fatalf("routed %+v", d)
	}
	h.clk.advance(UnengagedLimit + GoneGrace + time.Minute)
	h.sweep()
	h.sweep()
	if got := h.wantState(d.ID, Queued); got.To != keyA {
		t.Fatalf("moved to %s", got.To)
	}
}

// Policy and the rate limit apply to PR sends as to any other.
func TestPRPolicyAndRate(t *testing.T) {
	h := prHarness(t, []string{keyA}, true)
	h.ros.put(card(keyA, "alpha", prRepo, BypassMode))
	from := claude(keyS, "sigma")
	for i := range DirectLimit {
		if _, _, err := h.ib.Send(bg, from, toPR(prKey), "ping", ""); err != nil {
			t.Fatalf("send %d: %v", i, err)
		}
	}
	_, _, err := h.ib.Send(bg, from, toPR(prKey), "ping", "")
	wantErr(t, err, ErrRateLimited)

	if err := h.ib.SetPolicy(bg, Policy{Mode: Everyone, Block: []string{"sigma"}}, user); err != nil {
		t.Fatal(err)
	}
	_, d, err := h.ib.Send(bg, person(), toPR(prKey), "from the GUI", "")
	if err != nil || d.State != Queued || d.To != keyA {
		t.Fatalf("person send %+v %v", d, err)
	}
	h2 := prHarness(t, nil, true)
	if err := h2.ib.SetPolicy(bg, Policy{Mode: Everyone, Block: []string{"sigma"}}, user); err != nil {
		t.Fatal(err)
	}
	_, d, err = h2.ib.Send(bg, from, toPR(prKey), "blocked?", "")
	if err != nil || d.State != Held || d.Hold != HoldBlocked || d.To != keyD {
		t.Fatalf("blocked send %+v %v", d, err)
	}
}
