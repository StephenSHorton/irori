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

// TestUnconfirmedWakeReachesNextTurn replays card #23 through a real attach
// stream: the person's wake-mode notice to an idle, attached session whose
// mod never confirms a deliver (its host refused prompt.submit). The Core
// used to reclaim it after ReattachGrace and push a new token forever; now,
// after inbox.MaxWakeTries, the stream sends it as an append, which the mod
// confirms and the next turn engages.
func TestUnconfirmedWakeReachesNextTurn(t *testing.T) {
	const grace = 300 * time.Millisecond
	r := newRigWith(t, func(o *Options) { o.ReattachGrace = grace })
	ctx := context.Background()
	ib := r.core.Inbox()
	b, hb := r.attachMod(sessB, attach.AppendSince)

	_, d, err := ib.SendWith(ctx, r.core.UserSender(), inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "done",
		inbox.SendOptions{Mode: inbox.ModeWake, Ref: "task:01M62BP2MKFWJ4FK189419DNRV", Notice: true, Reason: "answered"})
	if err != nil {
		t.Fatal(err)
	}
	tokens := map[string]bool{}
	for i := range inbox.MaxWakeTries {
		ev := b.next(attach.TypeDeliver)
		if !slices.Equal(ev.DeliveryIDs, []string{d.ID}) || tokens[ev.Token] {
			t.Fatalf("try %d: deliver %+v", i, ev)
		}
		tokens[ev.Token] = true
		// The mod drops it and says nothing; the sweeper reclaims it.
		time.Sleep(grace + 50*time.Millisecond)
		if err := ib.Sweep(ctx); err != nil {
			t.Fatal(err)
		}
	}
	ev := b.next(attach.TypeAppend)
	if !strings.HasPrefix(ev.Token, "[irori-append:") || !slices.Equal(ev.DeliveryIDs, []string{d.ID}) || !strings.Contains(ev.Prompt, `reason="answered"`) {
		t.Fatalf("append %+v", ev)
	}
	b.none(attach.TypeDeliver, 300*time.Millisecond)
	r.control(hb.AttachSecret, "delivered", ev.Token, "")
	if s := r.state(d.ID); s != inbox.Delivered {
		t.Fatalf("after delivered: %s", s)
	}
	r.control(hb.AttachSecret, "engaged", ev.Token, "turn-human")
	if s := r.state(d.ID); s != inbox.Skipped {
		t.Fatalf("after engaged: %s", s)
	}
}
