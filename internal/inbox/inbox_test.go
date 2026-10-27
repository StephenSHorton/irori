package inbox

import (
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"
)

func TestSendWakeEngageReply(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)

	m, d := h.send(claude(keyA, "Front Desk"), to("beta"), "  can you review the parser?  ")
	if m.Text != "can you review the parser?" || m.Trust != Untrusted || m.Hop != 0 || m.From.Key != keyA {
		t.Fatalf("message %+v", m)
	}
	if !strings.HasPrefix(m.ID, "msg_") || !strings.HasPrefix(d.ID, "dlv_") {
		t.Fatalf("ids %s %s", m.ID, d.ID)
	}
	if d.To != keyB || d.State != Queued || d.Message.ID != m.ID {
		t.Fatalf("delivery %+v", d)
	}

	w := h.wake(keyB)
	if !strings.HasPrefix(w.ID, "wake_") || w.Token != "[irori-wake:"+w.ID+"]" || w.Reminder {
		t.Fatalf("wake %+v", w)
	}
	if strings.Count(w.Prompt, w.Token) != 1 || len(w.DeliveryIDs) != 1 || w.DeliveryIDs[0] != d.ID {
		t.Fatalf("wake prompt/ids: %+v", w)
	}
	h.noWake(keyB) // issued, not yet delivered: gated
	if ow, st := h.ib.OutstandingWake(keyB); ow == nil || ow.ID != w.ID || st != Queued || ow.Prompt != w.Prompt {
		t.Fatalf("outstanding %+v %s", ow, st)
	}

	for range 2 { // idempotent
		if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
			t.Fatal(err)
		}
	}
	got := h.wantState(d.ID, Delivered)
	if got.Via != "mod" || got.WakeID != w.ID {
		t.Fatalf("delivered %+v", got)
	}
	h.noWake(keyB) // delivered, not engaged: still gated
	wantErr(t, h.ib.MarkDelivered(bg, keyA, w.Token), ErrNotYours)
	wantErr(t, h.ib.MarkDelivered(bg, keyB, "[irori-wake:nope]"), ErrNotFound)

	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "turn-1"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Engaged)
	if h.ib.RunningWake(keyB) != w.ID {
		t.Fatal("running wake not set")
	}
	if ow, _ := h.ib.OutstandingWake(keyB); ow != nil {
		t.Fatal("engaged wake still outstanding")
	}

	// The reply goes back to the sender, from inside the woken turn.
	rm, rd, err := h.ib.Reply(bg, claude(keyB, "beta"), d.ID, "looks fine")
	if err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Answered)
	if rd.To != keyA || rm.ReplyTo != m.ID || rm.To.Kind != ToReply || rm.Hop != 1 || rm.ParentID != m.ID {
		t.Fatalf("reply %+v / %+v", rm, rd)
	}
	// A turn ending without a reply leaves it engaged; here it was answered.
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "turn-1"); err != nil {
		t.Fatal(err)
	}
	if h.ib.RunningWake(keyB) != "" {
		t.Fatal("running wake not cleared")
	}
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "turn-1"); err != nil { // again: no-op
		t.Fatal(err)
	}
	// "Front Desk" is now an alias of a.
	if _, d2 := h.send(claude(keyC, "gamma"), to("front desk"), "hi"); d2.To != keyA {
		t.Fatalf("alias routed to %s", d2.To)
	}
}

func TestTurnEndWithoutReplyStaysEngaged(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to(keyB), "fyi")
	w := h.deliverEngage(keyB, "t1")
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Engaged)
	// MarkEngaged on an engaged wake again is a no-op.
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Engaged)
}

func TestRouting(t *testing.T) {
	h := newHarness(t)
	h.abc()
	// A second "beta" lives in another repo; "delta" is a known agent that has gone.
	keyB2 := "dev1/bbbbbbbb-9999-4999-8999-999999999999"
	keyD := "dev1/dddddddd-4444-4444-8444-444444444444"
	h.ros.put(card(keyD, "delta", "", BypassMode))
	h.send(claude(keyD, "delta"), to("alpha"), "hello") // delta is now known
	h.ros.remove(keyD)

	tests := []struct {
		name, to string
		want     string
		err      error
	}{
		{"exact key", keyB, keyB, nil},
		{"exact key any case", strings.ToUpper(keyB), keyB, nil},
		{"session prefix", "cccccccc-33", keyC, nil},
		{"key prefix", "dev1/ccccc", keyC, nil},
		{"short prefix is not a prefix", "ccccccc", "", ErrUnknownAgent},
		{"name, any case", "  GAMMA ", keyC, nil},
		{"self by name", "alpha", "", ErrSelf},
		{"self by key", keyA, "", ErrSelf},
		{"unknown", "omega", "", ErrUnknownAgent},
		{"empty", " ", "", ErrUnknownAgent},
		{"gone agent by exact key is queued for it", keyD, keyD, nil},
		{"gone agent by its name", "delta", keyD, nil},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			_, d, err := h.ib.Send(bg, claude(keyA, "alpha"), to(tc.to), "x", "")
			if tc.err != nil {
				wantErr(t, err, tc.err)
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if d.To != tc.want || d.State != Queued {
				t.Fatalf("routed to %s (%s), want %s", d.To, d.State, tc.want)
			}
		})
	}

	t.Run("name matching two live agents names both", func(t *testing.T) {
		h.ros.put(card(keyB2, "Beta", "repo3", BypassMode))
		defer h.ros.remove(keyB2)
		_, _, err := h.ib.Send(bg, claude(keyC, "gamma"), to("beta"), "x", "")
		wantErr(t, err, ErrAmbiguous)
		if !strings.Contains(err.Error(), keyB) || !strings.Contains(err.Error(), keyB2) {
			t.Fatalf("error doesn't name both: %v", err)
		}
		// The prefix shared by both is ambiguous too; the full key works.
		_, _, err = h.ib.Send(bg, claude(keyC, "gamma"), to("bbbbbbbb"), "x", "")
		wantErr(t, err, ErrAmbiguous)
		if _, d, err := h.ib.Send(bg, claude(keyC, "gamma"), to(keyB2), "x", ""); err != nil || d.To != keyB2 {
			t.Fatalf("%v %s", err, d.To)
		}
	})

	t.Run("live beats gone for a name", func(t *testing.T) {
		h.ros.put(card(keyC+"x", "delta", "", BypassMode))
		defer h.ros.remove(keyC + "x")
		_, d, err := h.ib.Send(bg, claude(keyA, "alpha"), to("delta"), "x", "")
		if err != nil || d.To != keyC+"x" {
			t.Fatalf("%v %s", err, d.To)
		}
	})

	t.Run("key prefix beats a name", func(t *testing.T) {
		h.ros.put(card(keyC+"y", "aaaaaaaa", "", BypassMode)) // named like a's session prefix
		defer h.ros.remove(keyC + "y")
		_, d, err := h.ib.Send(bg, claude(keyB, "beta"), to("aaaaaaaa"), "x", "")
		if err != nil || d.To != keyA {
			t.Fatalf("%v %s", err, d.To)
		}
	})

	t.Run("reply address", func(t *testing.T) {
		m, _ := h.send(claude(keyA, "alpha"), to("beta"), "q")
		_, d, err := h.ib.Send(bg, claude(keyB, "beta"), Address{Kind: ToReply, Value: m.ID}, "a", "")
		if err != nil || d.To != keyA {
			t.Fatalf("%v %s", err, d.To)
		}
		_, _, err = h.ib.Send(bg, claude(keyA, "alpha"), Address{Kind: ToReply, Value: m.ID}, "a", "")
		wantErr(t, err, ErrSelf)
		_, _, err = h.ib.Send(bg, claude(keyB, "beta"), Address{Kind: ToReply, Value: "msg_nope"}, "a", "")
		wantErr(t, err, ErrNotFound)
	})

	t.Run("bad address kind and no sender", func(t *testing.T) {
		if _, _, err := h.ib.Send(bg, claude(keyA, "alpha"), Address{Kind: "pr", Value: "1"}, "x", ""); err == nil {
			t.Fatal("want error")
		}
		if _, _, err := h.ib.Send(bg, Sender{}, to("beta"), "x", ""); err == nil {
			t.Fatal("want error")
		}
	})
}

func TestAliases(t *testing.T) {
	h := newHarness(t)
	h.abc()
	long := strings.Repeat("n", MaxAliasLen+1)
	h.send(claude(keyA, long), to("beta"), "x")
	if a := h.ib.Aliases(keyA); len(a) != 0 {
		t.Fatalf("over-long alias recorded: %v", a)
	}
	for i := range MaxAliases + 2 {
		h.send(claude(keyA, fmt.Sprintf("Name%02d", i)), to("beta"), "x")
		h.clk.advance(DirectWindow) // stay under the rate limit
	}
	a := h.ib.Aliases(keyA)
	if len(a) != MaxAliases || a[0] != "name02" || a[len(a)-1] != "name11" {
		t.Fatalf("aliases %v", a)
	}
	// Reusing an old name makes it newest; the oldest still falls off later.
	h.send(claude(keyA, "NAME02"), to("beta"), "x")
	if a := h.ib.Aliases(keyA); a[len(a)-1] != "name02" || len(a) != MaxAliases {
		t.Fatalf("aliases %v", a)
	}
	_, _, err := h.ib.Send(bg, claude(keyB, "beta"), to("name00"), "x", "")
	wantErr(t, err, ErrUnknownAgent)
	if _, d := h.send(claude(keyB, "beta"), to("name05"), "x"); d.To != keyA {
		t.Fatalf("alias routed to %s", d.To)
	}
	// People aren't aliased.
	h.send(person(), to("beta"), "x")
	if a := h.ib.Aliases(user); len(a) != 0 {
		t.Fatalf("user aliased: %v", a)
	}
}

func TestTextLimits(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, _, err := h.ib.Send(bg, claude(keyA, "alpha"), to("beta"), " \n\t ", "")
	wantErr(t, err, ErrEmpty)
	_, _, err = h.ib.Send(bg, claude(keyA, "alpha"), to("beta"), strings.Repeat("é", MaxText+1), "")
	wantErr(t, err, ErrTooLong)
	if _, d := h.send(claude(keyA, "alpha"), to("beta"), "  "+strings.Repeat("é", MaxText)+"  "); d.State != Queued {
		t.Fatal(d.State)
	}
}

func TestRateLimit(t *testing.T) {
	h := newHarness(t)
	h.abc()
	for i := range DirectLimit { // alternating directions count together
		from, dst := claude(keyA, "alpha"), "beta"
		if i%2 == 1 {
			from, dst = claude(keyB, "beta"), "alpha"
		}
		h.send(from, to(dst), "ping")
		h.clk.advance(time.Second)
	}
	_, _, err := h.ib.Send(bg, claude(keyB, "beta"), to("alpha"), "ping", "")
	wantErr(t, err, ErrRateLimited)
	// Other pairs and the person are unaffected.
	h.send(claude(keyA, "alpha"), to("gamma"), "ok")
	for range DirectLimit + 2 {
		h.send(person(), to("alpha"), "from the GUI")
	}
	// Still refused just inside the window; allowed once the first send leaves it.
	h.clk.advance(DirectWindow - 8*time.Second - time.Millisecond)
	_, _, err = h.ib.Send(bg, claude(keyA, "alpha"), to("beta"), "ping", "")
	wantErr(t, err, ErrRateLimited)
	h.clk.advance(time.Millisecond)
	h.send(claude(keyA, "alpha"), to("beta"), "allowed again")

	// The window is rebuilt from the log after a restart: 8 in it again.
	h.reopen()
	_, _, err = h.ib.Send(bg, claude(keyA, "alpha"), to("beta"), "p", "")
	wantErr(t, err, ErrRateLimited)
}

func TestHops(t *testing.T) {
	h := newHarness(t)
	h.abc()
	m0, _ := h.send(claude(keyA, "alpha"), to("beta"), "start")
	w := h.deliverEngage(keyB, "t1")
	m1, _ := h.send(claude(keyB, "beta"), to("gamma"), "pass it on")
	if m1.Hop != 1 || m1.ParentID != m0.ID {
		t.Fatalf("hop %d parent %s", m1.Hop, m1.ParentID)
	}
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "t1"); err != nil {
		t.Fatal(err)
	}
	if m, _ := h.send(claude(keyB, "beta"), to("gamma"), "fresh"); m.Hop != 0 || m.ParentID != "" {
		t.Fatalf("after turn end: hop %d parent %s", m.Hop, m.ParentID)
	}

	// A chain past MaxHop is held.
	h2 := newHarness(t)
	h2.abc()
	agents := []Sender{claude(keyA, "alpha"), claude(keyB, "beta"), claude(keyC, "gamma")}
	_, d := h2.send(agents[0], to(keyB), "hop 0")
	for i := 1; ; i++ {
		cur := agents[i%3]
		if d.State == Held {
			if d.Hold != HoldHopLimit || d.Message.Hop != MaxHop+1 {
				t.Fatalf("held %s at hop %d", d.Hold, d.Message.Hop)
			}
			break
		}
		w := h2.deliverEngage(cur.Key, fmt.Sprint("t", i))
		next := agents[(i+1)%3]
		_, d = h2.send(cur, to(next.Key), fmt.Sprint("hop ", i))
		if d.Message.Hop != i {
			t.Fatalf("hop %d, want %d", d.Message.Hop, i)
		}
		if err := h2.ib.TurnEnded(bg, cur.Key, w.Token, fmt.Sprint("t", i)); err != nil {
			t.Fatal(err)
		}
		if i > MaxHop+2 {
			t.Fatal("never held")
		}
	}
}

func TestReplyHopWithoutRunningTurn(t *testing.T) {
	// A channel-path recipient acks rather than running a woken turn; its
	// reply still continues the chain from the message it answers.
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	m, d := h.send(claude(keyA, "alpha"), to("grok"), "q")
	if err := h.ib.Ack(bg, keyG, d.ID); err != nil {
		t.Fatal(err)
	}
	rm, rd, err := h.ib.Reply(bg, grok(keyG, "grok"), d.ID, "a")
	if err != nil {
		t.Fatal(err)
	}
	if rm.Hop != m.Hop+1 || rm.ParentID != m.ID {
		t.Fatalf("hop %d parent %s", rm.Hop, rm.ParentID)
	}
	// alpha runs in bypass and grok doesn't: the reply is held for the person.
	if rd.State != Held || rd.Hold != HoldBypassReceiver {
		t.Fatalf("%s %s", rd.State, rd.Hold)
	}
}

func TestReplyErrors(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	_, _, err := h.ib.Reply(bg, claude(keyB, "beta"), d.ID, "too early") // still queued
	wantErr(t, err, ErrBadState)
	_, _, err = h.ib.Reply(bg, claude(keyC, "gamma"), d.ID, "not mine")
	wantErr(t, err, ErrNotYours)
	_, _, err = h.ib.Reply(bg, claude(keyB, "beta"), "dlv_nope", "x")
	wantErr(t, err, ErrNotFound)
	h.deliverEngage(keyB, "t")
	_, _, err = h.ib.Reply(bg, claude(keyB, "beta"), d.ID, "")
	wantErr(t, err, ErrEmpty)
	h.wantState(d.ID, Engaged) // a refused reply doesn't answer it
	if _, _, err := h.ib.Reply(bg, claude(keyB, "beta"), d.ID, "ok"); err != nil {
		t.Fatal(err)
	}
	_, _, err = h.ib.Reply(bg, claude(keyB, "beta"), d.ID, "again")
	wantErr(t, err, ErrBadState)

	// Deferred deliveries can be answered.
	_, d2 := h.send(claude(keyC, "gamma"), to("beta"), "later?")
	if err := h.ib.Defer(bg, keyB, d2.ID, 30, "busy"); err != nil {
		t.Fatal(err)
	}
	if _, _, err := h.ib.Reply(bg, claude(keyB, "beta"), d2.ID, "now"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d2.ID, Answered)
}

func TestAckDeferSkip(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d := h.send(claude(keyA, "alpha"), to("beta"), "q")
	wantErr(t, h.ib.Ack(bg, keyC, d.ID), ErrNotYours)
	wantErr(t, h.ib.Ack(bg, keyB, "dlv_x"), ErrNotFound)
	if err := h.ib.Ack(bg, keyB, d.ID); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d.ID, Engaged); got.Via != "channel" {
		t.Fatalf("via %q", got.Via)
	}
	if err := h.ib.Ack(bg, keyB, d.ID); err != nil { // idempotent
		t.Fatal(err)
	}

	// Defer clamps to 12 h, needs a positive time, and only the recipient may.
	if err := h.ib.Defer(bg, keyB, d.ID, 0, ""); err == nil {
		t.Fatal("zero minutes accepted")
	}
	wantErr(t, h.ib.Defer(bg, keyA, d.ID, 5, ""), ErrNotYours)
	if err := h.ib.Defer(bg, keyB, d.ID, 100000, strings.Repeat("n", 900)); err != nil {
		t.Fatal(err)
	}
	got := h.wantState(d.ID, Deferred)
	if !got.Until.Equal(h.clk.now().Add(MaxDefer)) || len([]rune(got.Note)) != MaxNote {
		t.Fatalf("until %v note %d", got.Until, len(got.Note))
	}

	// Skip closes it; skipping again is a no-op; nothing else can follow.
	if err := h.ib.Skip(bg, keyB, d.ID, "no reply needed"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Skipped)
	if err := h.ib.Skip(bg, keyB, d.ID, ""); err != nil {
		t.Fatal(err)
	}
	wantErr(t, h.ib.Defer(bg, keyB, d.ID, 5, ""), ErrBadState)
	wantErr(t, h.ib.Ack(bg, keyB, d.ID), ErrBadState)
	wantErr(t, h.ib.Release(bg, d.ID, user), ErrBadState)
	wantErr(t, h.ib.Dismiss(bg, d.ID, user), ErrBadState)

	// Held deliveries can't be acked, deferred or skipped by the agent.
	h.ros.put(grokCard(keyG, "grok", ""))
	_, hd := h.send(grok(keyG, "grok"), to("beta"), "held")
	if hd.State != Held {
		t.Fatal(hd.State)
	}
	wantErr(t, h.ib.Ack(bg, keyB, hd.ID), ErrBadState)
	wantErr(t, h.ib.Defer(bg, keyB, hd.ID, 5, ""), ErrBadState)
	wantErr(t, h.ib.Skip(bg, keyB, hd.ID, ""), ErrBadState)

	// MarkRead: queued becomes delivered, then nothing.
	_, d3 := h.send(claude(keyA, "alpha"), to("gamma"), "r")
	if err := h.ib.MarkRead(bg, keyC, d3.ID); err != nil {
		t.Fatal(err)
	}
	h.wantState(d3.ID, Delivered)
	if err := h.ib.MarkRead(bg, keyC, d3.ID); err != nil {
		t.Fatal(err)
	}
	wantErr(t, h.ib.MarkRead(bg, keyA, d3.ID), ErrNotYours)
	if err := h.ib.Ack(bg, keyC, d3.ID); err != nil {
		t.Fatal(err)
	}
	h.wantState(d3.ID, Engaged)
}

func TestForAndList(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, d1 := h.send(claude(keyA, "alpha"), to("beta"), "1")
	_, d2 := h.send(claude(keyC, "gamma"), to("beta"), "2")
	_, d3 := h.send(claude(keyB, "beta"), to("alpha"), "3")
	if err := h.ib.Ack(bg, keyB, d1.ID); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.Skip(bg, keyB, d1.ID, ""); err != nil {
		t.Fatal(err)
	}
	open := h.ib.For(keyB, false, 0)
	if len(open) != 1 || open[0].ID != d2.ID || open[0].Message.Text != "2" {
		t.Fatalf("open %+v", open)
	}
	if all := h.ib.For(keyB, true, 0); len(all) != 2 || all[0].ID != d2.ID || all[1].ID != d1.ID {
		t.Fatalf("all %+v", all)
	}
	if l := h.ib.List(2); len(l) != 2 || l[0].ID != d3.ID || l[1].ID != d2.ID {
		t.Fatalf("list %+v", l)
	}
	if _, ok := h.ib.Get("dlv_nope"); ok {
		t.Fatal("found nothing")
	}
	if m, ok := h.ib.Message(d3.MessageID); !ok || m.Text != "3" {
		t.Fatal(m)
	}
}

func TestIDsSortAndDiffer(t *testing.T) {
	var g idGen
	at := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
	prev := ""
	seen := map[string]bool{}
	for i := range 5000 {
		id := g.next(at.Add(time.Duration(i/1000) * time.Millisecond))
		if len(id) != 26 || id <= prev || seen[id] {
			t.Fatalf("id %d %q after %q", i, id, prev)
		}
		seen[id] = true
		prev = id
	}
	// A clock that steps back still yields increasing ids.
	if id := g.next(at); id <= prev {
		t.Fatalf("%q <= %q", id, prev)
	}
	var g2 idGen
	if a, b := g2.next(at), g2.next(at.Add(time.Millisecond)); a[:10] == b[:10] {
		t.Fatal("time part didn't move")
	}
}

func TestConcurrentUse(t *testing.T) {
	h := newHarness(t)
	h.abc()
	sub := h.ib.Subscribe(t.Context())
	go func() {
		for range sub {
		}
	}()
	done := make(chan error, 4)
	for w := range 4 {
		go func() {
			for i := range 25 {
				if _, _, err := h.ib.Send(bg, person(), to("beta"), fmt.Sprint(w, i), ""); err != nil {
					done <- err
					return
				}
				if _, err := h.ib.NextWake(bg, keyB, PathMod); err != nil {
					done <- err
					return
				}
				_ = h.ib.List(5)
				_ = h.ib.Sweep(bg)
			}
			done <- nil
		}()
	}
	for range 4 {
		if err := <-done; err != nil {
			t.Fatal(err)
		}
	}
	if n := len(h.ib.For(keyB, true, 1000)); n != 100 {
		t.Fatalf("%d deliveries", n)
	}
}

func TestErrorsWrap(t *testing.T) {
	h := newHarness(t)
	h.abc()
	_, _, err := h.ib.Send(bg, claude(keyA, "alpha"), to("nobody"), "x", "")
	if !errors.Is(err, ErrUnknownAgent) || !strings.Contains(err.Error(), "beta") {
		t.Fatalf("%v", err)
	}
}
