package inbox

import (
	"encoding/json"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

func (h *harness) sendWith(from Sender, addr Address, text string, o SendOptions) Delivery {
	h.t.Helper()
	_, d, err := h.ib.SendWith(bg, from, addr, text, o)
	if err != nil {
		h.t.Fatalf("send %q: %v", text, err)
	}
	return d
}

func (h *harness) appendFor(agent string) *Wake {
	h.t.Helper()
	w, err := h.ib.NextAppend(bg, agent)
	if err != nil {
		h.t.Fatal(err)
	}
	if w == nil {
		h.t.Fatalf("no append for %s", agent)
	}
	return w
}

func (h *harness) noAppend(agent string) {
	h.t.Helper()
	w, err := h.ib.NextAppend(bg, agent)
	if err != nil {
		h.t.Fatal(err)
	}
	if w != nil {
		h.t.Fatalf("unexpected append for %s: %v", agent, w.DeliveryIDs)
	}
}

func TestSendWithModes(t *testing.T) {
	tests := []struct {
		name string
		from Sender
		mode DeliveryMode
		want DeliveryMode
	}{
		{"a person defaults to next_turn", person(), "", ModeNextTurn},
		{"Irori defaults to next_turn", SystemSender(), "", ModeNextTurn},
		{"an agent defaults to wake", claude(keyA, "alpha"), "", ModeWake},
		{"a grok member defaults to wake", grok(keyG, "grok"), "", ModeWake},
		{"a person may choose wake", person(), ModeWake, ModeWake},
		{"an agent may choose next_turn", claude(keyA, "alpha"), ModeNextTurn, ModeNextTurn},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			h := newHarness(t)
			h.abc()
			h.ros.put(grokCard(keyG, "grok", ""))
			d := h.sendWith(tc.from, to(keyC), "hi", SendOptions{Mode: tc.mode})
			if d.Mode != tc.want {
				t.Fatalf("mode %q, want %q", d.Mode, tc.want)
			}
			h.reopen()
			if got := h.state(d.ID).Mode; got != tc.want {
				t.Fatalf("after replay mode %q, want %q", got, tc.want)
			}
		})
	}

	t.Run("an unknown mode is refused", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		if _, _, err := h.ib.SendWith(bg, person(), to("beta"), "hi", SendOptions{Mode: "later"}); err == nil {
			t.Fatal("accepted")
		}
	})

	t.Run("Send is SendWith with wake, even from a person", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		m1, d1, err := h.ib.Send(bg, person(), to("beta"), "a", "why")
		if err != nil {
			t.Fatal(err)
		}
		m2, d2, err := h.ib.SendWith(bg, person(), to("beta"), "a", SendOptions{Reason: "why", Mode: ModeWake})
		if err != nil {
			t.Fatal(err)
		}
		if d1.Mode != ModeWake || d2.Mode != d1.Mode || m1.Reason != m2.Reason || m1.Trust != m2.Trust || d1.State != d2.State {
			t.Fatalf("%+v vs %+v", d1, d2)
		}
	})

	t.Run("ref and notice are kept and fenced", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		d := h.sendWith(claude(keyA, "alpha"), to("beta"), "card moved", SendOptions{Ref: " task:01ABC ", Notice: true, Mode: ModeWake})
		if d.Message.Ref != "task:01ABC" || !d.Message.Notice {
			t.Fatalf("message %+v", d.Message)
		}
		w := h.wake(keyB)
		for _, want := range []string{`ref="task:01ABC"`, `notice="true"`, "needs no reply", "show tool"} {
			if !strings.Contains(w.Prompt, want) {
				t.Fatalf("prompt lacks %q:\n%s", want, w.Prompt)
			}
		}
		long := h.sendWith(claude(keyA, "alpha"), to("beta"), "x", SendOptions{Ref: "task:" + strings.Repeat("z", 400)})
		if n := len([]rune(long.Message.Ref)); n != MaxRef {
			t.Fatalf("ref %d runes", n)
		}
	})
}

func TestSystemSender(t *testing.T) {
	h := newHarness(t)
	h.abc()
	sys := SystemSender()
	// No rate limit between Irori and one agent.
	for i := range DirectLimit + 3 {
		d := h.sendWith(sys, to("beta"), "notice "+string(rune('a'+i)), SendOptions{Notice: true})
		if d.State != Queued || d.Message.Trust != Untrusted || d.Mode != ModeNextTurn {
			t.Fatalf("notice %d: %s %s %s", i, d.State, d.Message.Trust, d.Mode)
		}
	}
	// It is no agent: not addressable, and nothing answers its notices.
	_, _, err := h.ib.Send(bg, claude(keyA, "alpha"), to("irori"), "hi", "")
	wantErr(t, err, ErrUnknownAgent)
	_, _, err = h.ib.Send(bg, claude(keyA, "alpha"), to(SystemKey), "hi", "")
	wantErr(t, err, ErrUnknownAgent)
	if a := h.ib.Aliases(SystemKey); len(a) != 0 {
		t.Fatalf("aliases %v", a)
	}
	d := h.ib.For(keyB, false, 1)[0]
	_, _, err = h.ib.Reply(bg, claude(keyB, "beta"), d.ID, "thanks")
	wantErr(t, err, ErrBadState)
	h.reopen()
	_, _, err = h.ib.Send(bg, claude(keyA, "alpha"), to("irori"), "hi", "")
	wantErr(t, err, ErrUnknownAgent)
}

func TestAppendFlow(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyB, PathMod, true)
	sub := h.ib.Subscribe(t.Context())
	d1 := h.sendWith(person(), to("beta"), "read this when you can", SendOptions{})
	// next_turn never wakes on the mod path.
	h.noWake(keyB)
	cs := drain(sub)
	if count(cs, WakeReady, keyB) != 0 || count(cs, AppendReady, keyB) != 1 {
		t.Fatalf("changes %+v", cs)
	}

	w := h.appendFor(keyB)
	if !strings.HasPrefix(w.Token, appendTokenPrefix) || w.Mode != ModeNextTurn || w.Reminder ||
		!slices.Equal(w.DeliveryIDs, []string{d1.ID}) || len(w.Overflow) != 0 {
		t.Fatalf("append %+v", w)
	}
	if strings.Count(w.Prompt, w.Token) != 1 || !strings.HasPrefix(w.Prompt, w.Token) ||
		!strings.Contains(w.Prompt, `mode="next_turn"`) || !strings.Contains(w.Prompt, "No turn was started") || !strings.Contains(w.Prompt, `trust="user"`) {
		t.Fatalf("prompt:\n%s", w.Prompt)
	}
	// While it is unconfirmed: no second append, and it never gates a wake
	// (nor shows up as one to re-send).
	d2 := h.sendWith(person(), to("beta"), "and this", SendOptions{})
	h.noAppend(keyB)
	if cs := drain(sub); count(cs, AppendReady, keyB) != 0 {
		t.Fatalf("AppendReady while one is pending: %+v", cs)
	}
	if ow, _ := h.ib.OutstandingWake(keyB); ow != nil {
		t.Fatalf("append offered as a wake: %+v", ow)
	}
	oa := h.ib.OutstandingAppend(keyB)
	if oa == nil || oa.Token != w.Token || oa.Prompt != w.Prompt || oa.Mode != ModeNextTurn {
		t.Fatalf("outstanding %+v", oa)
	}
	h.reopen()
	sub = h.ib.Subscribe(t.Context())
	h.ib.SetReachable(keyB, PathMod, true)
	if oa := h.ib.OutstandingAppend(keyB); oa == nil || oa.Token != w.Token || oa.Prompt != w.Prompt {
		t.Fatal("outstanding append lost in replay")
	}
	_, dw := h.send(claude(keyA, "alpha"), to("beta"), "wake up")
	ww := h.wake(keyB)
	if !slices.Equal(ww.DeliveryIDs, []string{dw.ID, d2.ID}) {
		t.Fatalf("wake %v, want the wake item then the queued next_turn one", ww.DeliveryIDs)
	}
	drain(sub)

	// Delivered: the append stops being outstanding.
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d1.ID, Delivered); got.Via != string(PathMod) || got.WakeID != w.ID {
		t.Fatalf("delivered %+v", got)
	}
	if h.ib.OutstandingAppend(keyB) != nil {
		t.Fatal("still outstanding after delivered")
	}
	// The next turn starts: engaged, with no running wake.
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "turn-p"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d1.ID, Engaged)
	if rw := h.ib.RunningWake(keyB); rw != "" {
		t.Fatalf("running wake %s after an append", rw)
	}
	m, _, err := h.ib.Send(bg, claude(keyB, "beta"), to("gamma"), "from that turn", "")
	if err != nil {
		t.Fatal(err)
	}
	if m.Hop != 0 || m.ParentID != "" {
		t.Fatalf("hop %d parent %q", m.Hop, m.ParentID)
	}
	if err := h.ib.TurnEnded(bg, keyB, w.Token, "turn-p"); err != nil {
		t.Fatal(err)
	}
	// The person can reply to it like any other.
	if _, _, err := h.ib.Reply(bg, claude(keyB, "beta"), d1.ID, "on it"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d1.ID, Answered)
}

func TestAppendAfterDeliveredAndEdges(t *testing.T) {
	h := newHarness(t)
	h.abc()
	sub := h.ib.Subscribe(t.Context())
	h.sendWith(person(), to("beta"), "one", SendOptions{})
	w := h.appendFor(keyB)
	d2 := h.sendWith(person(), to("beta"), "two", SendOptions{})
	drain(sub)
	// Confirming the first makes the next ready at once.
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if n := count(drain(sub), AppendReady, keyB); n != 1 {
		t.Fatalf("%d AppendReady after delivered", n)
	}
	w2 := h.appendFor(keyB)
	if w2.Token == w.Token || !slices.Equal(w2.DeliveryIDs, []string{d2.ID}) {
		t.Fatalf("second append %+v", w2)
	}
	// Engaging straight from queued works too (a lost delivered report).
	if err := h.ib.MarkEngaged(bg, keyB, w2.Token, "t"); err != nil {
		t.Fatal(err)
	}
	h.wantState(d2.ID, Engaged)
	h.noAppend(keyB)
	if _, err := h.ib.NextAppend(bg, ""); err == nil {
		t.Fatal("empty agent accepted")
	}
	// Someone else's token is refused.
	wantErr(t, h.ib.MarkEngaged(bg, keyA, w2.Token, "t"), ErrNotYours)
}

func TestAppendPausedAndHeld(t *testing.T) {
	h := newHarness(t)
	h.abc()
	if err := h.ib.SetPolicy(bg, Policy{Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	d := h.sendWith(person(), to("beta"), "later", SendOptions{})
	if d.State != Held || d.Hold != HoldPaused {
		t.Fatalf("%s %s", d.State, d.Hold)
	}
	h.noAppend(keyB)
	sub := h.ib.Subscribe(t.Context())
	if err := h.ib.SetPolicy(bg, Policy{}, user); err != nil {
		t.Fatal(err)
	}
	if n := count(drain(sub), AppendReady, keyB); n != 1 {
		t.Fatalf("%d AppendReady after unpause", n)
	}
	if w := h.appendFor(keyB); !slices.Equal(w.DeliveryIDs, []string{d.ID}) {
		t.Fatal(w.DeliveryIDs)
	}
	// Pause with an item queued: no append until resumed.
	h.sendWith(person(), to("gamma"), "x", SendOptions{})
	if err := h.ib.SetPolicy(bg, Policy{Paused: true}, user); err != nil {
		t.Fatal(err)
	}
	h.noAppend(keyC)
}

func TestNextTurnOnChannel(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	h.ib.SetReachable(keyG, PathChannel, true)
	d := h.sendWith(person(), to("grok"), "fyi", SendOptions{})
	w, err := h.ib.NextWake(bg, keyG, PathChannel)
	if err != nil || w == nil {
		t.Fatalf("no push: %v", err)
	}
	if w.Mode != ModeNextTurn || !strings.HasPrefix(w.Token, wakeTokenPrefix) || !slices.Equal(w.DeliveryIDs, []string{d.ID}) {
		t.Fatalf("push %+v", w)
	}
	if err := h.ib.Ack(bg, keyG, d.ID); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Engaged)
	// A wake item makes a plain wake carrying both.
	n := h.sendWith(person(), to("grok"), "fyi 2", SendOptions{})
	_, wd := h.send(claude(keyA, "alpha"), to("grok"), "now")
	w2, _ := h.ib.NextWake(bg, keyG, PathChannel)
	if w2 == nil || w2.Mode != ModeWake || !slices.Equal(w2.DeliveryIDs, []string{wd.ID, n.ID}) {
		t.Fatalf("mixed push %+v", w2)
	}
	if !strings.Contains(w2.Prompt, "came along with this one") {
		t.Fatal(w2.Prompt)
	}
}

func TestNoticeClosesOnEngage(t *testing.T) {
	notice := SendOptions{Notice: true, Ref: "task:01X"}
	t.Run("append", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		d := h.sendWith(SystemSender(), to("beta"), "blocker closed", notice)
		plain := h.sendWith(person(), to("beta"), "also", SendOptions{})
		w := h.appendFor(keyB)
		if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
			t.Fatal(err)
		}
		h.wantState(d.ID, Delivered)
		if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t"); err != nil {
			t.Fatal(err)
		}
		if got := h.wantState(d.ID, Skipped); got.Note != "notice" {
			t.Fatalf("note %q", got.Note)
		}
		h.wantState(plain.ID, Engaged)
	})
	t.Run("wake", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		o := notice
		o.Mode = ModeWake
		d := h.sendWith(claude(keyA, "alpha"), to("beta"), "assigned you", o)
		h.deliverEngage(keyB, "t")
		if got := h.wantState(d.ID, Skipped); got.Note != "notice" {
			t.Fatalf("note %q", got.Note)
		}
		if h.ib.RunningWake(keyB) == "" {
			t.Fatal("a woken turn still runs")
		}
	})
	t.Run("ack", func(t *testing.T) {
		h := newHarness(t)
		h.abc()
		h.ros.put(grokCard(keyG, "grok", ""))
		d := h.sendWith(SystemSender(), to("grok"), "blocker closed", notice)
		if err := h.ib.Ack(bg, keyG, d.ID); err != nil {
			t.Fatal(err)
		}
		if got := h.wantState(d.ID, Skipped); got.Note != "notice" {
			t.Fatalf("note %q", got.Note)
		}
		if open := h.ib.For(keyG, false, 0); len(open) != 0 {
			t.Fatalf("still open: %+v", open)
		}
	})
}

func TestNextTurnSweep(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyA, PathMod, true)
	h.ib.SetReachable(keyB, PathMod, true)
	// Repo work for the front desk (alpha, first reachable), sent next_turn.
	d := h.sendWith(person(), toRepo("repo1"), "when you get to it", SendOptions{})
	if d.To != keyA {
		t.Fatalf("desk %s", d.To)
	}
	w := h.appendFor(keyA)
	if err := h.ib.MarkDelivered(bg, keyA, w.Token); err != nil {
		t.Fatal(err)
	}
	// Hours pass with no turn: it is neither moved nor reminded.
	for range 5 {
		h.clk.advance(QuietLimit)
		h.sweep()
	}
	if got := h.wantState(d.ID, Delivered); got.To != keyA || got.Reminded {
		t.Fatalf("%+v", got)
	}
	h.noWake(keyA)
	// Its holder going for good still moves it, still next_turn.
	h.ros.remove(keyA)
	h.sweep()
	h.clk.advance(GoneGrace + time.Second)
	h.sweep()
	h.wantState(d.ID, Moved)
	open := h.ib.For(keyB, false, 0)
	if len(open) != 1 || open[0].Mode != ModeNextTurn || open[0].MessageID != d.MessageID {
		t.Fatalf("moved to %+v", open)
	}
}

func TestAppendReclaim(t *testing.T) {
	h := newHarness(t)
	h.abc()
	d := h.sendWith(person(), to("beta"), "q", SendOptions{})
	w := h.appendFor(keyB)
	h.clk.advance(ReattachGrace + time.Second)
	h.sweep()
	if got := h.wantState(d.ID, Queued); got.WakeID != "" {
		t.Fatal("still in the old append")
	}
	if h.ib.OutstandingAppend(keyB) != nil {
		t.Fatal("reclaimed append still outstanding")
	}
	w2 := h.appendFor(keyB)
	if w2.Token == w.Token || w2.DeliveryIDs[0] != d.ID {
		t.Fatalf("re-append %+v", w2)
	}
	// The old one's late confirmation can't take it from the new one...
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if got := h.wantState(d.ID, Queued); got.WakeID != w2.ID {
		t.Fatal(got.WakeID)
	}
	if err := h.ib.MarkDelivered(bg, keyB, w2.Token); err != nil {
		t.Fatal(err)
	}
	h.wantState(d.ID, Delivered)

	// ...but a late confirmation with nothing re-issued still counts.
	d3 := h.sendWith(person(), to("gamma"), "q3", SendOptions{})
	w3 := h.appendFor(keyC)
	h.clk.advance(ReattachGrace + time.Second)
	h.sweep()
	if err := h.ib.MarkDelivered(bg, keyC, w3.Token); err != nil {
		t.Fatal(err)
	}
	h.wantState(d3.ID, Delivered)
	h.noAppend(keyC)
}

func TestAppendOverflow(t *testing.T) {
	h := newHarness(t)
	h.abc()
	big := strings.Repeat("n", 7000)
	var ids []string
	for range 4 {
		ids = append(ids, h.sendWith(person(), to("beta"), big, SendOptions{}).ID)
	}
	w := h.appendFor(keyB)
	if !slices.Equal(w.DeliveryIDs, ids) || !slices.Equal(w.Overflow, ids[2:]) || len(w.Prompt) > MaxWakeBytes {
		t.Fatalf("append %v overflow %v (%d bytes)", w.DeliveryIDs, w.Overflow, len(w.Prompt))
	}
	if oa := h.ib.OutstandingAppend(keyB); oa.Prompt != w.Prompt || !slices.Equal(oa.Overflow, w.Overflow) {
		t.Fatal("rebuilt append differs")
	}
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t"); err != nil {
		t.Fatal(err)
	}
	for _, id := range ids {
		h.wantState(id, Engaged)
	}
}

func TestWakeIssuedRowFields(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.sendWith(person(), to("beta"), "x", SendOptions{})
	w := h.appendFor(keyB)
	_, d := h.send(claude(keyA, "alpha"), to("gamma"), "y")
	ww := h.wake(keyC)
	evs, err := h.st.List(bg, store.Filter{Kinds: []string{KindWakeIssued, KindDelivery}})
	if err != nil {
		t.Fatal(err)
	}
	modes := map[string]string{}
	var dmode string
	for _, ev := range evs {
		var r struct {
			WakeID, DeliveryID, Mode string
		}
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			t.Fatal(err)
		}
		if ev.Kind == KindWakeIssued {
			modes[r.WakeID] = r.Mode
		} else if r.DeliveryID == d.ID {
			dmode = r.Mode
		}
	}
	if modes[w.ID] != "next_turn" || modes[ww.ID] != "" || dmode != "wake" {
		t.Fatalf("modes %v delivery %q", modes, dmode)
	}
}

// A channel push of next_turn items alone must not hold back a wake that
// arrives after it: the agent was told it need not act on it now.
func TestChannelNextTurnPushDoesNotGate(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	h.ib.SetReachable(keyG, PathChannel, true)
	fyi := h.sendWith(person(), to("grok"), "fyi", SendOptions{})
	w, err := h.ib.NextWake(bg, keyG, PathChannel)
	if err != nil || w == nil || w.Mode != ModeNextTurn {
		t.Fatalf("push %+v: %v", w, err)
	}
	if ow, _ := h.ib.OutstandingWake(keyG); ow != nil {
		t.Fatalf("next_turn push gates: %+v", ow)
	}
	_, urgent := h.send(claude(keyA, "alpha"), to("grok"), "urgent")
	w2, err := h.ib.NextWake(bg, keyG, PathChannel)
	if err != nil || w2 == nil {
		t.Fatalf("urgent wake held back: %v", err)
	}
	if w2.Mode != ModeWake || !slices.Equal(w2.DeliveryIDs, []string{urgent.ID}) {
		t.Fatalf("wake %+v", w2)
	}
	// The pushed next_turn item stays with its push, not pushed twice.
	if got := h.state(fyi.ID); got.WakeID != w.ID {
		t.Fatalf("fyi moved to %q", got.WakeID)
	}
	// And a real wake still gates the next one.
	h.send(claude(keyA, "alpha"), to("grok"), "urgent 2")
	if w3, _ := h.ib.NextWake(bg, keyG, PathChannel); w3 != nil {
		t.Fatalf("wake not gated: %+v", w3)
	}
}

// An overflowed notice never reached a prompt, so engaging the append must
// leave it open for the inbox tool the prompt points to.
func TestOverflowNoticeStaysOpen(t *testing.T) {
	h := newHarness(t)
	h.abc()
	big := strings.Repeat("n", 7000)
	notice := SendOptions{Notice: true, Ref: "task:01X"}
	var ids []string
	for range 3 {
		ids = append(ids, h.sendWith(SystemSender(), to("beta"), big, notice).ID)
	}
	w := h.appendFor(keyB)
	if !slices.Equal(w.Overflow, ids[2:]) {
		t.Fatalf("overflow %v", w.Overflow)
	}
	if err := h.ib.MarkDelivered(bg, keyB, w.Token); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.MarkEngaged(bg, keyB, w.Token, "t"); err != nil {
		t.Fatal(err)
	}
	for _, id := range ids[:2] {
		h.wantState(id, Skipped)
	}
	h.wantState(ids[2], Engaged)
	open := h.ib.For(keyB, false, 0)
	if len(open) != 1 || open[0].ID != ids[2] {
		t.Fatalf("open %+v", open)
	}
	// The agent can still close it itself.
	if err := h.ib.Skip(bg, keyB, ids[2], ""); err != nil {
		t.Fatal(err)
	}
}
