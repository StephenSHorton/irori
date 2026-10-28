package inbox

import (
	"context"
	"errors"
	"slices"
	"strings"
	"testing"
	"time"
)

// Planner tables ported from tsukumo's planSweep/planReminders tests, recast
// as Irori deliveries (design §4).
func TestPlanSweep(t *testing.T) {
	t0 := time.Date(2026, 10, 26, 12, 0, 0, 0, time.UTC)
	min := func(n int) time.Time { return t0.Add(-time.Duration(n) * time.Minute) }
	env := func(gone map[string]time.Duration, reach map[string]bool, desks map[string][]string) sweepEnv {
		return sweepEnv{
			Now: t0, Unengaged: UnengagedLimit, Gone: GoneGrace, Quiet: QuietLimit, Grace: ReattachGrace,
			GoneFor:   func(a string) time.Duration { return gone[a] },
			EndedFor:  func(string) time.Duration { return 0 },
			Self:      "dev1",
			Reachable: func(a string) bool { return reach[a] },
			Desk: func(repo string, exclude ...string) string {
				for _, k := range desks[repo] {
					if !slices.Contains(exclude, k) {
						return k
					}
				}
				return ""
			},
		}
	}
	// ended is env with a's session out of presence for d (and, as a
	// stretch that long implies, gone for the gone-move too).
	ended := func(d time.Duration, reach map[string]bool, desks map[string][]string) sweepEnv {
		e := env(map[string]time.Duration{"a": d}, reach, desks)
		e.EndedFor = func(a string) time.Duration {
			if a == "a" {
				return d
			}
			return 0
		}
		return e
	}
	around := map[string]bool{"a": true, "b": true}
	twoDesks := map[string][]string{"r": {"a", "b"}}
	type want struct {
		kind actionKind
		to   string
	}
	none := []want(nil)
	tests := []struct {
		name string
		item sweepItem
		env  sweepEnv
		want []want
	}{
		{"a holder gone 3 minutes loses repo work, whatever it is",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Engaged, EngagedOnce: true, AssignedAt: min(5), UpdatedAt: min(5)},
			env(map[string]time.Duration{"a": GoneGrace + time.Second}, around, map[string][]string{"r": {"b"}}),
			[]want{{actMove, "b"}}},
		{"an owner gone for good loses its PR work to the PR's repo",
			sweepItem{ID: "1", To: "a", GoneRepo: "r", State: Queued, AssignedAt: min(5), UpdatedAt: min(5)},
			env(map[string]time.Duration{"a": GoneGrace + time.Second}, around, map[string][]string{"r": {"b"}}),
			[]want{{actMove, "b"}}},
		{"an owner around keeps its unengaged PR work",
			sweepItem{ID: "1", To: "a", GoneRepo: "r", State: Queued, AssignedAt: min(30), UpdatedAt: min(30)},
			env(nil, around, map[string][]string{"r": {"b"}}),
			nil},
		{"even moved and deferred repo work leaves a gone holder",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Deferred, Until: t0.Add(time.Hour), MovedFrom: true, EngagedOnce: true},
			env(map[string]time.Duration{"a": GoneGrace + time.Second}, around, map[string][]string{"r": {"b"}}),
			[]want{{actMove, "b"}}},
		{"gone with no desk left: unowned",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Queued, AssignedAt: min(1)},
			env(map[string]time.Duration{"a": GoneGrace + time.Second}, nil, nil),
			[]want{{actMove, ""}}},
		{"a restart blip doesn't move it",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Engaged, EngagedOnce: true, UpdatedAt: min(5)},
			env(map[string]time.Duration{"a": 2 * time.Minute}, around, twoDesks),
			none},
		{"agent work stays with a gone agent",
			sweepItem{ID: "1", To: "a", State: Queued, AssignedAt: min(60)},
			env(map[string]time.Duration{"a": time.Hour}, nil, twoDesks),
			none},
		{"new work the holder hasn't started on moves to the next desk after 10 minutes",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Delivered, AssignedAt: min(11), UpdatedAt: min(11)},
			env(nil, around, twoDesks), []want{{actMove, "b"}}},
		{"queued, never even delivered, moves too",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Queued, AssignedAt: min(11), UpdatedAt: min(11)},
			env(nil, around, twoDesks), []want{{actMove, "b"}}},
		{"not before 10 minutes",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Delivered, AssignedAt: min(10), UpdatedAt: min(10)},
			env(nil, around, twoDesks), none},
		{"work that already moved once isn't moved again; its holder is reminded once",
			sweepItem{ID: "1", To: "b", Repo: "r", State: Delivered, MovedFrom: true, AssignedAt: min(11), UpdatedAt: min(11)},
			env(nil, around, twoDesks), []want{{actRemind, "b"}}},
		{"...and only once",
			sweepItem{ID: "1", To: "b", Repo: "r", State: Delivered, MovedFrom: true, Reminded: true, AssignedAt: min(30)},
			env(nil, around, twoDesks), none},
		{"a lone front desk that hasn't started is reminded (nobody to hand it to)",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Delivered, AssignedAt: min(11), UpdatedAt: min(11)},
			env(nil, around, map[string][]string{"r": {"a"}}), []want{{actRemind, "a"}}},
		{"a lone desk with it still queued: nothing to remind about",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Queued, AssignedAt: min(11)},
			env(nil, around, map[string][]string{"r": {"a"}}), none},
		{"a holder that started on it keeps it, however long it takes",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Delivered, EngagedOnce: true, AssignedAt: min(30), UpdatedAt: min(30)},
			env(nil, around, twoDesks), none},
		{"an hour engaged with no reply and no deferral: remind the holder",
			sweepItem{ID: "1", To: "a", State: Engaged, EngagedOnce: true, UpdatedAt: min(61)},
			env(nil, around, nil), []want{{actRemind, "a"}}},
		{"once",
			sweepItem{ID: "1", To: "a", State: Engaged, Reminded: true, UpdatedAt: min(200)},
			env(nil, around, nil), none},
		{"not before the hour",
			sweepItem{ID: "1", To: "a", State: Engaged, UpdatedAt: min(60)},
			env(nil, around, nil), none},
		{"only holders that can be reached are reminded",
			sweepItem{ID: "1", To: "c", State: Engaged, UpdatedAt: min(61)},
			env(nil, around, nil), none},
		{"a deferral that's due comes back to the queue",
			sweepItem{ID: "1", To: "a", State: Deferred, Until: t0, UpdatedAt: min(30)},
			env(nil, around, nil), []want{{actRequeue, ""}}},
		{"a deferral holds until its time, past the hour",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Deferred, EngagedOnce: true, Until: t0.Add(time.Minute), AssignedAt: min(600), UpdatedAt: min(600)},
			env(nil, around, twoDesks), none},
		{"a wake never confirmed within the grace is reclaimed",
			sweepItem{ID: "1", To: "a", State: Queued, InFlight: true, IssuedAt: t0.Add(-ReattachGrace - time.Second)},
			env(nil, around, nil), []want{{actReclaim, ""}}},
		{"not within the grace",
			sweepItem{ID: "1", To: "a", State: Queued, InFlight: true, IssuedAt: t0.Add(-ReattachGrace)},
			env(nil, around, nil), none},
		{"unowned repo work goes to a desk once there is one",
			sweepItem{ID: "1", Repo: "r", State: Held, Hold: HoldUnowned},
			env(nil, around, twoDesks), []want{{actAssign, "a"}}},
		{"unowned with no desk waits",
			sweepItem{ID: "1", Repo: "r", State: Held, Hold: HoldUnowned},
			env(nil, nil, nil), none},
		{"held by policy is a person's call",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Held, Hold: HoldBlocked, AssignedAt: min(100)},
			env(map[string]time.Duration{"a": time.Hour}, around, twoDesks), none},
		{"answered is left alone",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Answered, AssignedAt: min(100)},
			env(map[string]time.Duration{"a": time.Hour}, around, twoDesks), none},
	}
	// next_turn waits for the next turn however long it takes: no unengaged
	// move and no reminder; the gone-move and the reclaim still apply.
	tests = append(tests, []struct {
		name string
		item sweepItem
		env  sweepEnv
		want []want
	}{
		{"next_turn repo work is never moved for being unengaged",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Queued, NextTurn: true, AssignedAt: min(600), UpdatedAt: min(600)},
			env(nil, around, twoDesks), none},
		{"next_turn delivered and never engaged gets no reminder",
			sweepItem{ID: "1", To: "a", State: Delivered, NextTurn: true, AssignedAt: min(600), UpdatedAt: min(600)},
			env(nil, around, nil), none},
		{"next_turn delivered repo work gets no reminder either",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Delivered, NextTurn: true, MovedFrom: true, AssignedAt: min(600), UpdatedAt: min(600)},
			env(nil, around, twoDesks), none},
		{"next_turn engaged and quiet for hours gets no reminder",
			sweepItem{ID: "1", To: "a", State: Engaged, NextTurn: true, EngagedOnce: true, UpdatedAt: min(600)},
			env(nil, around, nil), none},
		{"next_turn repo work still leaves a gone holder",
			sweepItem{ID: "1", To: "a", Repo: "r", State: Delivered, NextTurn: true, AssignedAt: min(5), UpdatedAt: min(5)},
			env(map[string]time.Duration{"a": GoneGrace + time.Second}, around, map[string][]string{"r": {"b"}}),
			[]want{{actMove, "b"}}},
		{"an unconfirmed append is reclaimed",
			sweepItem{ID: "1", To: "a", State: Queued, NextTurn: true, InFlight: true, IssuedAt: min(3), AssignedAt: min(3)},
			env(nil, around, nil), []want{{actReclaim, ""}}},
		{"a due next_turn deferral is re-queued",
			sweepItem{ID: "1", To: "a", State: Deferred, NextTurn: true, Until: min(1)},
			env(nil, around, nil), []want{{actRequeue, ""}}},
	}...)
	// Agent work whose recipient session ended goes on its card, or from an
	// agent to the person; presence alone decides, not the wake path.
	past := GoneGrace + time.Second
	tests = append(tests, []struct {
		name string
		item sweepItem
		env  sweepEnv
		want []want
	}{
		{"ended, about a task: on the card",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "task:T1", State: Engaged, EngagedOnce: true, UpdatedAt: min(5)},
			ended(past, nil, nil), []want{{actGoneCard, ""}}},
		{"ended, about a comment: on its card",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "comment:C1", State: Queued, AssignedAt: min(5)},
			ended(past, nil, nil), []want{{actGoneCard, ""}}},
		{"ended, the person's message about a card: on the card too",
			sweepItem{ID: "1", To: "a", From: user, FromPerson: true, Ref: "task:T1", State: Queued, AssignedAt: min(5)},
			ended(past, nil, nil), []want{{actGoneCard, ""}}},
		{"ended, no card, from an agent: to the person",
			sweepItem{ID: "1", To: "a", From: "c", State: Delivered, AssignedAt: min(5), UpdatedAt: min(5)},
			ended(past, nil, nil), []want{{actGonePerson, "user:dev1"}}},
		{"ended, a ref that is no card, from an agent: to the person",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "pr:o/r#1", State: Queued, AssignedAt: min(5)},
			ended(past, nil, nil), []want{{actGonePerson, "user:dev1"}}},
		{"ended, no card, from the person: stays",
			sweepItem{ID: "1", To: "a", From: user, FromPerson: true, State: Queued, AssignedAt: min(5)},
			ended(past, nil, nil), none},
		{"live but unreachable: stays",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "task:T1", State: Queued, AssignedAt: min(60)},
			env(map[string]time.Duration{"a": time.Hour}, nil, nil), none},
		{"ended for less than the grace: stays",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "task:T1", State: Queued, AssignedAt: min(5)},
			ended(GoneGrace, nil, nil), none},
		{"a notice stays",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "task:T1", Notice: true, State: Queued, NextTurn: true, AssignedAt: min(5)},
			ended(past, nil, nil), none},
		{"Irori's own stays",
			sweepItem{ID: "1", To: "a", From: SystemKey, System: true, State: Queued, AssignedAt: min(5)},
			ended(past, nil, nil), none},
		{"held is a person's call",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "task:T1", State: Held, Hold: HoldBlocked, AssignedAt: min(5)},
			ended(past, nil, nil), none},
		{"a next_turn delivery still hands off",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "task:T1", State: Queued, NextTurn: true, AssignedAt: min(5)},
			ended(past, nil, nil), []want{{actGoneCard, ""}}},
		{"an in-flight delivery hands off rather than being reclaimed",
			sweepItem{ID: "1", To: "a", From: "c", State: Queued, InFlight: true, IssuedAt: min(5), AssignedAt: min(5)},
			ended(past, nil, nil), []want{{actGonePerson, "user:dev1"}}},
		{"a deferred delivery hands off",
			sweepItem{ID: "1", To: "a", From: "c", Ref: "task:T1", State: Deferred, Until: t0.Add(time.Hour), EngagedOnce: true},
			ended(past, nil, nil), []want{{actGoneCard, ""}}},
		{"repo work keeps its desk move",
			sweepItem{ID: "1", To: "a", From: "c", Repo: "r", Ref: "task:T1", State: Engaged, EngagedOnce: true},
			ended(past, nil, map[string][]string{"r": {"b"}}), []want{{actMove, "b"}}},
		{"PR work keeps its repo move",
			sweepItem{ID: "1", To: "a", From: "c", GoneRepo: "r", State: Queued, AssignedAt: min(5)},
			ended(past, nil, map[string][]string{"r": {"b"}}), []want{{actMove, "b"}}},
	}...)
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := planSweep([]sweepItem{tc.item}, tc.env)
			if len(got) != len(tc.want) {
				t.Fatalf("got %+v, want %+v", got, tc.want)
			}
			for i, w := range tc.want {
				if got[i].Kind != w.kind || got[i].To != w.to || got[i].DeliveryID != tc.item.ID {
					t.Fatalf("got %+v, want %+v", got[i], w)
				}
			}
		})
	}

	t.Run("paused: no reminders", func(t *testing.T) {
		e := env(nil, around, nil)
		e.Paused = true
		if got := planSweep([]sweepItem{{ID: "1", To: "a", State: Engaged, UpdatedAt: min(61)}}, e); len(got) != 0 {
			t.Fatal(got)
		}
	})
}

func TestRepoFrontDesk(t *testing.T) {
	h := newHarness(t)
	h.abc() // a and b in repo1, c in repo2
	if h.ib.FrontDesk("repo1") != "" {
		t.Fatal("desk with nobody reachable")
	}
	// Nobody reachable: held unowned, To "".
	_, u := h.send(claude(keyC, "gamma"), toRepo("repo1"), "anyone?")
	if u.To != "" || u.State != Held || u.Hold != HoldUnowned || u.RepoKey != "repo1" {
		t.Fatalf("unowned %+v", u)
	}
	wantErr(t, h.ib.Release(bg, u.ID, user), ErrBadState) // no desk to release to

	h.ib.SetReachable(keyB, PathMod, true)
	h.clk.advance(time.Second)
	h.ib.SetReachable(keyA, PathMod, true)
	if h.ib.FrontDesk("repo1") != keyB {
		t.Fatal("first reachable isn't the desk")
	}
	h.sweep()
	if got := h.wantState(u.ID, Queued); got.To != keyB {
		t.Fatalf("assigned to %s", got.To)
	}
	_, d := h.send(claude(keyC, "gamma"), toRepo("repo1"), "review please")
	if d.To != keyB || d.RepoKey != "repo1" {
		t.Fatalf("repo send to %s", d.To)
	}
	// The desk sending to its own repo goes to the next agent.
	if _, d := h.send(claude(keyB, "beta"), toRepo("repo1"), "help"); d.To != keyA {
		t.Fatalf("self-desk send to %s", d.To)
	}
	// A brief drop keeps b's place; a long one hands the desk to a.
	h.ib.SetReachable(keyB, PathMod, false)
	h.ib.SetReachable(keyB, PathMod, true)
	if h.ib.FrontDesk("repo1") != keyB {
		t.Fatal("reattach lost the desk")
	}
	h.ib.SetReachable(keyB, PathMod, false)
	h.clk.advance(ReattachGrace - time.Second)
	if h.ib.FrontDesk("repo1") != keyB {
		t.Fatal("desk lost within the grace")
	}
	h.clk.advance(time.Second)
	if h.ib.FrontDesk("repo1") != keyA {
		t.Fatal("desk not handed on")
	}
	h.ib.SetReachable(keyB, PathMod, true) // back, but now behind a
	if h.ib.FrontDesk("repo1") != keyA {
		t.Fatal("returning agent jumped the queue")
	}
	// Channel-path agents count as reachable too.
	h.ros.put(grokCard(keyG, "grok", "repo2"))
	h.ib.SetReachable(keyG, PathChannel, true)
	if h.ib.FrontDesk("repo2") != keyG {
		t.Fatal("grok desk")
	}
	_, _, err := h.ib.Send(bg, claude(keyA, "alpha"), toRepo(" "), "x", "")
	if err == nil {
		t.Fatal("empty repo accepted")
	}
}

func TestSweepMovesUnengagedRepoWorkOnce(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyA, PathMod, true)
	h.clk.advance(time.Second)
	h.ib.SetReachable(keyB, PathMod, true)
	_, d := h.send(claude(keyC, "gamma"), toRepo("repo1"), "review")
	if d.To != keyA {
		t.Fatal(d.To)
	}
	w := h.wake(keyA)
	if err := h.ib.MarkDelivered(bg, keyA, w.Token); err != nil {
		t.Fatal(err)
	}
	sub := h.ib.Subscribe(t.Context())
	h.clk.advance(UnengagedLimit + time.Second)
	h.sweep()
	old := h.wantState(d.ID, Moved)
	if old.MovedTo != keyB {
		t.Fatalf("moved to %q", old.MovedTo)
	}
	cs := drain(sub)
	if count(cs, MovedAway, keyA) != 1 || count(cs, WakeReady, keyB) != 1 {
		t.Fatalf("changes %+v", cs)
	}
	open := h.ib.For(keyB, false, 0)
	if len(open) != 1 || open[0].MessageID != d.MessageID || open[0].State != Queued || open[0].RepoKey != "repo1" {
		t.Fatalf("new delivery %+v", open)
	}
	nd := open[0]
	// a can't act on what moved away.
	_, _, err := h.ib.Reply(bg, claude(keyA, "alpha"), d.ID, "mine?")
	wantErr(t, err, ErrBadState)
	h.noWake(keyA) // the moved wake no longer gates a

	// b doesn't start either: it is not moved again, b is reminded once.
	w2 := h.wake(keyB)
	if err := h.ib.MarkDelivered(bg, keyB, w2.Token); err != nil {
		t.Fatal(err)
	}
	h.clk.advance(UnengagedLimit + time.Second)
	h.sweep()
	h.wantState(nd.ID, Delivered)
	r := h.wake(keyB) // the stale delivered wake stopped gating; a reminder is due
	if !r.Reminder || r.DeliveryIDs[0] != nd.ID {
		t.Fatalf("reminder %+v", r)
	}
	h.clk.advance(UnengagedLimit)
	h.sweep()
	h.noWake(keyB)
	h.wantState(nd.ID, Delivered)
}

func TestSweepGoneHolder(t *testing.T) {
	h := newHarness(t)
	var onCard []string
	h.onGone = func(_ context.Context, m Message, to string) (string, error) {
		onCard = append(onCard, m.ID)
		return "x", nil
	}
	h.reopen()
	h.abc()
	h.ib.SetReachable(keyA, PathMod, true)
	h.clk.advance(time.Second)
	h.ib.SetReachable(keyB, PathMod, true)
	_, repoWork := h.send(claude(keyC, "gamma"), toRepo("repo1"), "repo work")
	_, agentWork := h.send(claude(keyC, "gamma"), to("alpha"), "agent work")
	cardWork := h.sendWith(claude(keyC, "gamma"), to("alpha"), "card work", SendOptions{Mode: ModeWake, Ref: "task:x"})
	h.deliverEngage(keyA, "t1")
	h.wantState(repoWork.ID, Engaged)

	// a's session ends.
	h.ros.remove(keyA)
	h.ib.SetReachable(keyA, PathMod, false)
	h.sweep() // first sighting starts the debounce
	h.clk.advance(GoneGrace)
	h.sweep()
	h.wantState(repoWork.ID, Engaged) // not yet
	h.clk.advance(time.Second)
	h.sweep()
	if got := h.wantState(repoWork.ID, Moved); got.MovedTo != keyB {
		t.Fatalf("moved to %s", got.MovedTo)
	}
	// a's session ended: its agent work goes to the person, marked, and
	// the work about a card goes on the card.
	if got := h.wantState(agentWork.ID, Moved); got.MovedTo != user || got.Note != NoteRecipientGone {
		t.Fatalf("agent work %+v", got)
	}
	mine := h.ib.For(user, false, 0)
	if len(mine) != 1 || mine[0].MessageID != agentWork.MessageID || !strings.HasPrefix(mine[0].For, "recipient gone: alpha") {
		t.Fatalf("the person's %+v", mine)
	}
	if got := h.wantState(cardWork.ID, Skipped); got.Note != "recipient gone; on task:x" || len(onCard) != 1 || onCard[0] != cardWork.MessageID {
		t.Fatalf("card work %+v (on card %v)", got, onCard)
	}
	if open := h.ib.For(keyB, false, 0); len(open) != 1 || open[0].MessageID != repoWork.MessageID {
		t.Fatalf("b's work %+v", open)
	}

	// b goes too, with nobody left: the work waits unowned, then a new desk gets it.
	h.ros.remove(keyB)
	h.sweep()
	h.clk.advance(GoneGrace + time.Second)
	h.sweep()
	var unowned Delivery
	for _, d := range h.ib.List(0) {
		if d.MessageID == repoWork.MessageID && d.State.Open() {
			unowned = d
		}
	}
	if unowned.To != "" || unowned.Hold != HoldUnowned {
		t.Fatalf("unowned %+v", unowned)
	}
	const keyE = "dev1/eeeeeeee-7777-4777-8777-777777777777"
	h.ros.put(card(keyE, "epsilon", "repo1", BypassMode))
	h.ib.SetReachable(keyE, PathMod, true)
	h.sweep()
	if got := h.wantState(unowned.ID, Queued); got.To != keyE {
		t.Fatalf("assigned to %s", got.To)
	}
}

// goneHook is an Options.OnRecipientGone that records what it put on a
// card, checking it runs with the inbox lock released.
type goneHook struct {
	t     *testing.T
	h     *harness
	calls []Message
	tos   []string
	err   error // what the hook returns ("T1" when nil)
}

func (g *goneHook) hook(_ context.Context, m Message, to string) (string, error) {
	// Called with b.mu released: the inbox can be read from inside it.
	if !g.h.ib.impl.mu.TryLock() {
		g.t.Error("hook called with the inbox lock held")
	} else {
		g.h.ib.impl.mu.Unlock()
	}
	g.h.ib.List(0)
	g.calls, g.tos = append(g.calls, m), append(g.tos, to)
	if g.err != nil {
		return "", g.err
	}
	return "T1", nil
}

// newGoneHarness is a, b, c in presence, with a goneHook.
func newGoneHarness(t *testing.T) (*harness, *goneHook) {
	h := newHarness(t)
	g := &goneHook{t: t, h: h}
	h.onGone = g.hook
	h.reopen()
	h.abc()
	return h, g
}

// endA removes a's session and sweeps until it is past GoneGrace.
func (h *harness) endA() {
	h.t.Helper()
	h.ros.remove(keyA)
	h.sweep() // first sighting starts the clock
	h.clk.advance(GoneGrace + time.Second)
	h.sweep()
}

func TestSweepGoneToCard(t *testing.T) {
	h, g := newGoneHarness(t)
	d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "about T1", SendOptions{Mode: ModeWake, Ref: "task:T1"})
	h.sweep()
	if len(g.calls) != 0 {
		t.Fatal("handed off while a is around")
	}
	h.endA()
	if len(g.calls) != 1 || g.calls[0].ID != d.MessageID || g.calls[0].Ref != "task:T1" || g.tos[0] != keyA {
		t.Fatalf("hook calls %+v to %v", g.calls, g.tos)
	}
	if got := h.wantState(d.ID, Skipped); got.Note != "recipient gone; on task:T1" {
		t.Fatalf("note %q", got.Note)
	}
	if n := len(h.ib.List(0)); n != 1 {
		t.Fatalf("%d deliveries, want the one", n)
	}
	// Nothing left for a: the next sweep is a no-op, and a's clock is gone.
	h.clk.advance(time.Minute)
	h.sweep()
	if len(g.calls) != 1 || len(h.ib.List(0)) != 1 {
		t.Fatalf("second sweep: %d calls, %d deliveries", len(g.calls), len(h.ib.List(0)))
	}
	if h.ib.EndedFor(keyA) != 0 {
		t.Fatal("ended clock kept with no open work")
	}
}

func TestSweepGoneToPerson(t *testing.T) {
	t.Run("no card", func(t *testing.T) {
		h, g := newGoneHarness(t)
		d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "no card", SendOptions{Mode: ModeWake})
		mine := h.sendWith(person(), to("alpha"), "the person's own", SendOptions{})
		h.endA()
		if len(g.calls) != 0 {
			t.Fatal("hook called for a message with no card")
		}
		old := h.wantState(d.ID, Moved)
		if old.MovedTo != user || old.Note != NoteRecipientGone {
			t.Fatalf("old %+v", old)
		}
		got := h.ib.For(user, false, 0)
		if len(got) != 1 || got[0].MessageID != d.MessageID || got[0].State != Queued ||
			got[0].For != "recipient gone: alpha ("+keyA+")" {
			t.Fatalf("the person's %+v", got)
		}
		h.wantState(mine.ID, Queued) // the person's own message stays for a
	})
	t.Run("the card isn't here", func(t *testing.T) {
		h, g := newGoneHarness(t)
		g.err = ErrNoCard
		d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "about a card", SendOptions{Mode: ModeWake, Ref: "comment:C1"})
		h.endA()
		if len(g.calls) != 1 {
			t.Fatalf("hook calls %d", len(g.calls))
		}
		if old := h.wantState(d.ID, Moved); old.MovedTo != user || old.Note != NoteRecipientGone {
			t.Fatalf("old %+v", old)
		}
		if got := h.ib.For(user, false, 0); len(got) != 1 || !strings.HasPrefix(got[0].For, "recipient gone: ") {
			t.Fatalf("the person's %+v", got)
		}
	})
	t.Run("another error leaves it for the next sweep", func(t *testing.T) {
		h, g := newGoneHarness(t)
		g.err = errors.New("restoring")
		d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "about T1", SendOptions{Mode: ModeWake, Ref: "task:T1"})
		h.endA()
		h.wantState(d.ID, Queued)
		if len(h.ib.For(user, false, 0)) != 0 {
			t.Fatal("moved to the person on a hook error")
		}
		g.err = nil
		h.sweep()
		if len(g.calls) != 2 {
			t.Fatalf("hook calls %d, want a retry", len(g.calls))
		}
		h.wantState(d.ID, Skipped)
	})
	t.Run("no hook: card work goes to the person", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "about T1", SendOptions{Mode: ModeWake, Ref: "task:T1"})
		h.endA()
		if old := h.wantState(d.ID, Moved); old.MovedTo != user {
			t.Fatalf("old %+v", old)
		}
	})
}

func TestEndedIgnoresReachability(t *testing.T) {
	h, g := newGoneHarness(t)
	// a is in presence with no wake path (no mod attached) for 10 minutes:
	// gone for repo work, but not ended, so its mail waits.
	d := h.sendWith(claude(keyC, "gamma"), to("alpha"), "about T1", SendOptions{Mode: ModeWake, Ref: "task:T1"})
	n := h.sendWith(claude(keyC, "gamma"), to("alpha"), "no card", SendOptions{Mode: ModeWake})
	for range 10 {
		h.sweep()
		h.clk.advance(time.Minute)
	}
	h.sweep()
	if len(g.calls) != 0 || h.ib.EndedFor(keyA) != 0 {
		t.Fatalf("handed off a live session's mail (%d calls, ended %v)", len(g.calls), h.ib.EndedFor(keyA))
	}
	h.wantState(d.ID, Queued)
	h.wantState(n.ID, Queued)

	// a leaves and comes back within the grace: its clock starts over.
	h.ros.remove(keyA)
	h.sweep()
	h.clk.advance(2 * time.Minute)
	h.sweep()
	h.abc()
	h.sweep()
	if h.ib.EndedFor(keyA) != 0 {
		t.Fatal("ended clock kept after a came back")
	}
	h.ros.remove(keyA)
	h.sweep()
	h.clk.advance(2 * time.Minute)
	h.sweep()
	if len(g.calls) != 0 {
		t.Fatal("handed off before a was gone for the grace")
	}
	h.wantState(d.ID, Queued)
	h.wantState(n.ID, Queued)
}

func TestSweepAttachLostCountsAsGone(t *testing.T) {
	// Presence still shows the session but its attach is gone: repo work moves.
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyA, PathMod, true)
	h.clk.advance(time.Second)
	h.ib.SetReachable(keyB, PathMod, true)
	_, d := h.send(claude(keyC, "gamma"), toRepo("repo1"), "x")
	h.deliverEngage(keyA, "t")
	h.ib.SetReachable(keyA, PathMod, false)
	h.clk.advance(ReattachGrace)
	h.sweep()
	h.clk.advance(GoneGrace + time.Second)
	h.sweep()
	if got := h.wantState(d.ID, Moved); got.MovedTo != keyB {
		t.Fatal(got.MovedTo)
	}
}

func TestSweepAssignsHeldPolicyOnArrival(t *testing.T) {
	// Unowned work is checked against the desk's policy when it gets one.
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	_, d := h.send(grok(keyG, "grok"), toRepo("repo1"), "from grok")
	if d.Hold != HoldUnowned {
		t.Fatal(d.Hold)
	}
	h.ib.SetReachable(keyA, PathMod, true)
	h.sweep()
	got := h.wantState(d.ID, Held)
	if got.Hold != HoldBypassReceiver || got.To != keyA {
		t.Fatalf("%+v", got)
	}
}
