package core

import (
	"context"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/store"
)

// An ask posted on the Mac (board.md "Asks"): every free agent hears it;
// with the PC and the Mac apart, both agents claim it. The Mac, which
// hands it out, grants its own agent at once; the PC's claim waits. Once
// they heal, every device agrees on the Mac's agent, the PC's agent hears
// who took it, and its queued ask notice is skipped as stale.
func TestAPartitionedAskHasOneTaker(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	wait(t, "the mac sees the pc's session", func() bool { return hasCard(mac, pc.agent(), "connected") })
	ask, n, err := mac.c.Coord().PostAsk(ctx, mac.user(), coord.NewAsk{Text: "Why is the Mac build slow?"}, mac.c.MeshAgents())
	if err != nil {
		t.Fatal(err)
	}
	if !n.Person || len(n.To) != 2 {
		t.Fatalf("the ask told %v", n.To)
	}
	if err := mac.c.RouteNotices(ctx, mac.c.UserSender(), n); err != nil {
		t.Fatal(err)
	}
	notice := func() *inbox.Delivery {
		for _, d := range pc.c.Inbox().For(pc.agent(), true, 0) {
			if d.Message.Ref == "task:"+ask.ID && d.Message.Reason == string(coord.NoticeAsk) {
				return &d
			}
		}
		return nil
	}
	wait(t, "the pc has the ask and its notice", func() bool {
		_, ok := pc.c.Coord().GetTask(ask.ID)
		return ok && notice() != nil && notice().State == inbox.Queued
	})

	net.Isolate(mac.id())
	r, err := mac.c.Coord().AskClaim(ctx, mac.agent(), ask.ID, time.Second)
	if err != nil || !r.Mine {
		t.Fatalf("the mac's claim: %+v %v", r, err)
	}
	r, err = pc.c.Coord().AskClaim(ctx, pc.agent(), ask.ID, 200*time.Millisecond)
	if err != nil || !r.Pending || r.Mine {
		t.Fatalf("the pc's claim while apart: %+v %v", r, err)
	}
	if x, _ := pc.c.Coord().GetTask(ask.ID); len(x.Ask.Waiting) != 1 || x.Ask.Taker != "" {
		t.Fatalf("the pc's view while apart: %+v", x.Ask)
	}

	net.HealAll()
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "one taker on "+h.name, func() bool {
			x, _ := h.c.Coord().GetTask(ask.ID)
			return x.Ask != nil && x.Ask.Taker == mac.agent() && len(x.Ask.Waiting) == 0
		})
	}
	wait(t, "the three converge", synced(pc, mac, nd))
	// The PC's sweep tells its agent who took it.
	wait(t, "the pc's agent hears who took it", func() bool {
		for _, d := range pc.c.Inbox().For(pc.agent(), true, 0) {
			if d.Message.Reason == string(coord.NoticeTaken) && strings.Contains(d.Message.Text, mac.agent()+" took") {
				return true
			}
		}
		return false
	})
	// Its ask notice no longer reaches it; who took it does.
	w, err := pc.c.Inbox().NextAppend(ctx, pc.agent())
	if err != nil {
		t.Fatal(err)
	}
	if w == nil || strings.Contains(w.Prompt, "asks the room") || !strings.Contains(w.Prompt, mac.agent()+" took") {
		t.Fatalf("the pc's append: %+v", w)
	}
	if d := notice(); d == nil || d.State != inbox.Skipped || !strings.Contains(d.Note, "taken by") {
		t.Fatalf("the stale ask notice: %+v", d)
	}
	// Nothing ever wrote a second taker.
	evs := 0
	for _, x := range mustList(t, mac, coord.KindTakeState) {
		if strings.Contains(string(x.Payload), `"reason":"taken"`) {
			evs++
		}
	}
	if evs != 1 {
		t.Fatalf("%d taken rows", evs)
	}
}

func mustList(t *testing.T, h *hcore, kind string) []store.Event {
	t.Helper()
	evs, err := h.c.store.List(context.Background(), store.Filter{Kinds: []string{kind}, Limit: store.MaxListLimit})
	if err != nil {
		t.Fatal(err)
	}
	return evs
}

// Pause all holds the ask for this device's agents, and a held notice
// doesn't count as heard: they are left out of who is told (the PC's
// agent, whose host decides for itself, is not). The person's comment to
// the room goes stale once someone takes the ask, like the announcement.
func TestAskUnderPauseAllAndTheRoomsComments(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, _ := meshTrio(t)
	wait(t, "the mac sees the pc's session", func() bool { return hasCard(mac, pc.agent(), "connected") })
	if err := mac.c.Inbox().SetPolicy(ctx, inbox.Policy{Paused: true}, mac.user()); err != nil {
		t.Fatal(err)
	}
	keys := func() (out []string) {
		for _, a := range mac.c.AskAgents() {
			out = append(out, a.Key)
		}
		return out
	}
	if got := keys(); slices.Contains(got, mac.agent()) || !slices.Contains(got, pc.agent()) {
		t.Fatalf("told under Pause all: %v", got)
	}
	if err := mac.c.Inbox().SetPolicy(ctx, inbox.Policy{}, mac.user()); err != nil {
		t.Fatal(err)
	}
	if got := keys(); !slices.Contains(got, mac.agent()) {
		t.Fatalf("after the pause: %v", got)
	}

	ask, _, err := mac.c.Coord().PostAsk(ctx, mac.user(), coord.NewAsk{Text: "who has the release?"}, mac.c.AskAgents())
	if err != nil {
		t.Fatal(err)
	}
	_, ns, err := mac.c.Coord().PersonComment(ctx, mac.user(), ask.ID, "also the notes", false, nil)
	if err != nil || len(ns) != 1 {
		t.Fatalf("%+v %v", ns, err)
	}
	room := inbox.Delivery{To: pc.agent(), Message: inbox.Message{Ref: ns[0].Ref, Reason: string(ns[0].Kind)}}
	if mac.c.staleNotice(room) != "" {
		t.Fatal("stale before anyone took it")
	}
	if r, err := mac.c.Coord().AskClaim(ctx, mac.agent(), ask.ID, time.Second); err != nil || !r.Mine {
		t.Fatalf("claim %+v %v", r, err)
	}
	if note := mac.c.staleNotice(room); !strings.Contains(note, "taken by") {
		t.Fatalf("the room's comment after the take: %q", note)
	}
}
