package core

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

// noticesTo lists the Board notices delivered to key.
func (r *rig) noticesTo(key string) []inbox.Delivery {
	var out []inbox.Delivery
	for _, d := range r.core.Inbox().For(key, true, 0) {
		if d.Message.Notice {
			out = append(out, d)
		}
	}
	return out
}

// A blocker that resolves by itself (the lease is released) is closed by
// the coord sweep, and its holder hears from Irori at its next turn.
func TestSweepRoutesNotices(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	co := r.core.Coord()
	if _, err := co.Acquire(ctx, r.keyB, "gpu/qwen", 0, false); err != nil {
		t.Fatal(err)
	}
	task, err := co.CreateTask(ctx, r.keyA, coord.NewTask{Title: "train"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := co.ClaimTask(ctx, r.keyA, task.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := co.Block(ctx, r.keyA, task.ID, "gpu busy", &coord.BlockedBy{Kind: coord.BlockedOnLease, Ref: "gpu/qwen"}); err != nil {
		t.Fatal(err)
	}
	if err := co.Release(ctx, r.keyB, "gpu/qwen"); err != nil {
		t.Fatal(err)
	}
	if err := r.core.sweepCoord(ctx); err != nil {
		t.Fatal(err)
	}
	if got, _ := co.GetTask(task.ID); got.State == coord.Blocked {
		t.Fatalf("still blocked: %+v", got)
	}
	ns := r.noticesTo(r.keyA)
	if len(ns) != 1 {
		t.Fatalf("notices %+v", ns)
	}
	d := ns[0]
	if d.Mode != inbox.ModeNextTurn || d.Message.From.Key != inbox.SystemKey || d.Message.Ref != "task:"+task.ID {
		t.Fatalf("notice %+v", d)
	}
	// A second sweep has nothing more to say.
	if err := r.core.sweepCoord(ctx); err != nil {
		t.Fatal(err)
	}
	if n := len(r.noticesTo(r.keyA)); n != 1 {
		t.Fatalf("%d notices after a second sweep", n)
	}
}

// RouteNotice is what the GUI calls: a person's comment defaults to the
// next turn, and the Wake toggle makes it a wake.
func TestRouteNoticeFromThePerson(t *testing.T) {
	tests := []struct {
		name string
		wake bool
		want inbox.DeliveryMode
	}{
		{"next turn", false, inbox.ModeNextTurn},
		{"wake", true, inbox.ModeWake},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := newRig(t)
			ctx := context.Background()
			co := r.core.Coord()
			task, err := co.CreateTask(ctx, r.keyA, coord.NewTask{Title: "docs"})
			if err != nil {
				t.Fatal(err)
			}
			if _, err := co.ClaimTask(ctx, r.keyB, task.ID); err != nil {
				t.Fatal(err)
			}
			cm, n, err := co.AddComment(ctx, r.core.UserSender().Key, task.ID, "use the new heading style", tt.wake)
			if err != nil {
				t.Fatal(err)
			}
			rs := r.core.RouteNotice(ctx, r.core.UserSender(), n)
			if err := tools.NoticeErr(rs); err != nil || len(rs) != 1 {
				t.Fatalf("routed %+v: %v", rs, err)
			}
			d := rs[0].Delivery
			if rs[0].To != r.keyB || d.Mode != tt.want || d.Message.Ref != "comment:"+cm.ID || !d.Message.Notice || d.Message.Trust != inbox.UserTrust {
				t.Fatalf("delivery %+v", d)
			}
		})
	}
}

// The board tools served by the Core route their notices through the inbox.
func TestBoardToolsThroughCore(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	a := tools.Caller{Sender: r.sender(r.keyA, "alpha"), Path: inbox.PathMod}
	call := func(name string, args map[string]any) string {
		t.Helper()
		raw, _ := json.Marshal(args)
		out, err := r.core.Tools().Call(ctx, a, name, raw)
		if err != nil || strings.HasPrefix(out, "Error: ") {
			t.Fatalf("%s: %q, %v", name, out, err)
		}
		return out
	}
	out := call("board_create", map[string]any{"title": "hand-off", "assign_to": r.keyB})
	if !strings.Contains(out, "Told at their next turn") {
		t.Fatalf("create: %s", out)
	}
	ns := r.noticesTo(r.keyB)
	if len(ns) != 1 || ns[0].Mode != inbox.ModeNextTurn || ns[0].Message.From.Key != r.keyA {
		t.Fatalf("notices %+v", ns)
	}
	if out := call("search", map[string]any{"query": "hand-off", "kinds": []string{"task"}}); !strings.Contains(out, "task:") {
		t.Fatalf("search: %s", out)
	}
}
