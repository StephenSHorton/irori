package core

import (
	"context"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// A watcher hears what changed on a card from Irori at its next turn,
// through the coord sweep; the person's answer reaches the raiser as a wake
// when the person chose it.
func TestCardNoticesRoute(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	co := r.core.Coord()
	task, err := co.CreateTask(ctx, r.keyA, coord.NewTask{Title: "ship"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := co.ClaimTask(ctx, r.keyA, task.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := co.Watch(ctx, r.keyB, task.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := co.SetSteps(ctx, r.keyA, task.ID, []coord.Step{{Text: "tag it"}}); err != nil {
		t.Fatal(err)
	}
	if _, _, err := co.Raise(ctx, r.keyA, task.ID, coord.NewBlocker{Question: "Ship now?", Options: []coord.Option{{Answer: "yes"}}}); err != nil {
		t.Fatal(err)
	}
	if err := r.core.sweepCoord(ctx); err != nil {
		t.Fatal(err)
	}
	ns := r.noticesTo(r.keyB)
	if len(ns) != 1 || ns[0].Mode != inbox.ModeNextTurn || ns[0].Message.From.Key != inbox.SystemKey ||
		!strings.Contains(ns[0].Message.Text, "raised a blocker: Ship now?") || !strings.Contains(ns[0].Message.Text, "set the steps") {
		t.Fatalf("watcher notices %+v", ns)
	}
	_, n, err := co.Answer(ctx, r.core.UserSender().Key, task.ID, "b1", coord.BlockerAnswer{Pick: 1, Wake: true})
	if err != nil {
		t.Fatal(err)
	}
	if err := r.core.RouteNotices(ctx, r.core.UserSender(), n); err != nil {
		t.Fatal(err)
	}
	var woke bool
	for _, d := range r.noticesTo(r.keyA) {
		woke = woke || d.Mode == inbox.ModeWake && d.Message.Ref == "task:"+task.ID && strings.Contains(d.Message.Text, "Answer: yes")
	}
	if !woke {
		t.Fatalf("raiser notices %+v", r.noticesTo(r.keyA))
	}
}

// RouteNotices sends each recipient its own delivery, next_turn unless the
// notice wakes, as a notice with its ref; an unreachable recipient is
// reported without stopping the rest.
func TestRouteNotices(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	ib := r.core.Inbox()
	cases := []struct {
		name string
		wake bool
		want inbox.DeliveryMode
	}{
		{"next turn", false, inbox.ModeNextTurn},
		{"wake", true, inbox.ModeWake},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			ref := "task:" + strings.ReplaceAll(tc.name, " ", "-")
			n := coord.Notice{Kind: coord.NoticeComment, Ref: ref, To: []string{"dev-x/not-here", r.keyB}, Text: "a comment on " + ref, Wake: tc.wake}
			err := r.core.RouteNotices(ctx, r.core.UserSender(), n)
			if err == nil || !strings.Contains(err.Error(), "dev-x/not-here") {
				t.Fatalf("err = %v, want the unreachable recipient named", err)
			}
			var found *inbox.Delivery
			for _, d := range ib.List(500) {
				if d.To == r.keyB && d.Message.Ref == ref {
					found = &d
					break
				}
			}
			if found == nil {
				t.Fatal("the reachable recipient got nothing")
			}
			if found.Mode != tc.want || !found.Message.Notice {
				t.Fatalf("delivery mode %q notice %v, want %q notice", found.Mode, found.Message.Notice, tc.want)
			}
		})
	}
}
