package core

import (
	"context"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// attachMod opens a mod stream that names its mod version.
func (r *rig) attachMod(session, version string) (*evStream, attach.Event) {
	r.t.Helper()
	s, code := r.open("session=" + session + "&mod=" + version)
	if code != 200 {
		r.t.Fatalf("attach %s: status %d", session, code)
	}
	hello := s.next(attach.TypeHello)
	if hello.V != attach.Version || hello.AttachSecret == "" {
		r.t.Fatalf("hello %+v", hello)
	}
	return s, hello
}

func TestNextTurnAppend(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	ib := r.core.Inbox()
	b1, _ := r.attachMod(sessB, attach.AppendSince)

	_, d, err := ib.SendWith(ctx, r.core.UserSender(), inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "look at the lexer when you're next up", inbox.SendOptions{Ref: "task:01LEX"})
	if err != nil {
		t.Fatal(err)
	}
	if d.Mode != inbox.ModeNextTurn {
		t.Fatalf("mode %q", d.Mode)
	}
	ev := b1.next(attach.TypeAppend)
	if ev.Mode != "next_turn" || !strings.HasPrefix(ev.Token, "[irori-append:") || !slices.Equal(ev.DeliveryIDs, []string{d.ID}) ||
		len(ev.MessageIDs) != 1 || ev.From != "you" || strings.Count(ev.Prompt, ev.Token) != 1 ||
		!strings.Contains(ev.Prompt, `mode="next_turn"`) || !strings.Contains(ev.Prompt, `ref="task:01LEX"`) {
		t.Fatalf("append %+v", ev)
	}
	b1.none(attach.TypeDeliver, 300*time.Millisecond)

	// A reload before delivered: the same append again, same token.
	b2, h2 := r.attachMod(sessB, "0.4.1")
	b1.ended(attach.TypeSuperseded)
	again := b2.next(attach.TypeAppend)
	if again.Token != ev.Token || again.Prompt != ev.Prompt {
		t.Fatalf("re-sent %+v", again)
	}
	r.control(h2.AttachSecret, "delivered", ev.Token, "")
	if s := r.state(d.ID); s != inbox.Delivered {
		t.Fatalf("after delivered: %s", s)
	}
	// After delivered a reload sends nothing.
	b3, h3 := r.attachMod(sessB, "0.4.1")
	b2.ended(attach.TypeSuperseded)
	b3.none(attach.TypeAppend, 500*time.Millisecond)

	// The next main-loop turn engages it; no running wake, turn-ended is a no-op.
	r.control(h3.AttachSecret, "engaged", ev.Token, "turn-p")
	if s := r.state(d.ID); s != inbox.Engaged {
		t.Fatalf("after engaged: %s", s)
	}
	if rw := ib.RunningWake(r.keyB); rw != "" {
		t.Fatalf("running wake %s", rw)
	}
	r.control(h3.AttachSecret, "turn-ended", ev.Token, "turn-p")

	// A second one goes out as soon as it is sent.
	_, d2, err := ib.SendWith(ctx, r.core.UserSender(), inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "and the parser", inbox.SendOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if ev2 := b3.next(attach.TypeAppend); ev2.Token == ev.Token || !slices.Equal(ev2.DeliveryIDs, []string{d2.ID}) {
		t.Fatalf("second append %+v", ev2)
	}
}

func TestNextTurnRidesWakeForOldMod(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	ib := r.core.Inbox()
	a, ha := r.attach(sessA) // mod=0.2.0-test: before appends
	_, d, err := ib.SendWith(ctx, r.core.UserSender(), inbox.Address{Kind: inbox.ToAgent, Value: "alpha"}, "fyi-3c1d", inbox.SendOptions{})
	if err != nil {
		t.Fatal(err)
	}
	a.none(attach.TypeAppend, 500*time.Millisecond)
	if s := r.state(d.ID); s != inbox.Queued {
		t.Fatalf("state %s", s)
	}
	// It rides the next wake.
	_, dw, err := ib.Send(ctx, r.sender(r.keyB, "beta"), inbox.Address{Kind: inbox.ToAgent, Value: "alpha"}, "wake-88aa", "")
	if err != nil {
		t.Fatal(err)
	}
	ev := a.next(attach.TypeDeliver)
	if ev.Mode != "" || !slices.Equal(ev.DeliveryIDs, []string{dw.ID, d.ID}) || !strings.Contains(ev.Prompt, "fyi-3c1d") || !strings.Contains(ev.Prompt, `mode="next_turn"`) {
		t.Fatalf("deliver %+v", ev)
	}
	r.control(ha.AttachSecret, "delivered", ev.Token, "")
	r.control(ha.AttachSecret, "engaged", ev.Token, "turn-1")
	if s := r.state(d.ID); s != inbox.Engaged {
		t.Fatalf("ride-along state %s", s)
	}
}
