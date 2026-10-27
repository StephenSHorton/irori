package core

import (
	"context"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

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
