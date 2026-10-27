package coord

import (
	"reflect"
	"strings"
	"testing"
	"time"
)

// Regression tests for the coord-board review.

func boardHas(cs []Card, id string) bool {
	for _, c := range cs {
		if c.ID == id {
			return true
		}
	}
	return false
}

// An assignee that hasn't claimed isn't working the task: closing a blocker
// (Unblock or the sweeper) leaves it Submitted, still in the assignee's
// NeedsYou.
func TestUnblockKeepsUnclaimedAssigneeSubmitted(t *testing.T) {
	cases := []struct {
		name  string
		close func(e *env, id string)
		claim bool // the assignee claims before the block
		want  TaskState
	}{
		{"unblock, never claimed", func(e *env, id string) { mustN(t)(e.c.Unblock(ctx, carol, id, "")) }, false, Submitted},
		{"sweeper, never claimed", func(e *env, id string) {
			if err := e.c.Release(ctx, carol, "port:5173"); err != nil {
				t.Fatal(err)
			}
			if _, err := e.c.SweepNotices(ctx); err != nil {
				t.Fatal(err)
			}
		}, false, Submitted},
		{"unblock, claimed", func(e *env, id string) { mustN(t)(e.c.Unblock(ctx, carol, id, "")) }, true, Working},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			must[Lease](t)(e.c.Acquire(ctx, carol, "port:5173", time.Hour, false))
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
			mustN(t)(e.c.Assign(ctx, alice, task.ID, bob))
			if tc.claim {
				must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
			}
			must[Task](t)(e.c.Block(ctx, alice, task.ID, "wait", &BlockedBy{Kind: BlockedOnLease, Ref: "port:5173"}))
			tc.close(e, task.ID)
			for _, c := range []*Coord{e.c, e.reopen()} {
				got, _ := c.GetTask(task.ID)
				if got.State != tc.want || !reflect.DeepEqual(got.Holders, []string{bob}) {
					t.Fatalf("state %s holders %v, want %s [bob]", got.State, got.Holders, tc.want)
				}
				v := c.Board(BoardFilter{For: bob})
				if boardHas(v.NeedsYou, task.ID) == tc.claim {
					t.Fatalf("needsYou for bob = %v (claimed %v)", v.NeedsYou, tc.claim)
				}
			}
		})
	}
}

// On a shared task someone already works, an assignee is still only
// assigned until it claims or joins: the task is in its NeedsYou, Assigned
// says so, and its claim (or join) writes the row.
func TestSharedAssigneeNotEngagedUntilItClaims(t *testing.T) {
	for _, op := range []string{"claim", "join"} {
		t.Run(op, func(t *testing.T) {
			e := newEnv(t)
			task := shared(t, e, alice)
			must[Task](t)(e.c.ClaimTask(ctx, alice, task.ID))
			got, n := mustN(t)(e.c.Assign(ctx, alice, task.ID, bob))
			if !reflect.DeepEqual(n.To, []string{bob}) || !reflect.DeepEqual(got.Assigned, []string{bob}) {
				t.Fatalf("assign: notice to %v, assigned %v", n.To, got.Assigned)
			}
			v := e.c.Board(BoardFilter{For: bob})
			if !boardHas(v.NeedsYou, task.ID) || boardHas(v.Working, task.ID) {
				t.Fatalf("bob's board: needsYou %d working %d", len(v.NeedsYou), len(v.Working))
			}
			rows := KindTaskClaimed
			if op == "claim" {
				got = must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
			} else {
				rows = KindTaskJoined
				got = must[Task](t)(e.c.Join(ctx, bob, task.ID, bob))
			}
			wantRows := 1
			if op == "claim" {
				wantRows = 2 // alice's and bob's
			}
			if n := len(e.rows(rows)); n != wantRows {
				t.Fatalf("%s rows = %d, want %d", rows, n, wantRows)
			}
			if got.Assigned != nil || !reflect.DeepEqual(got.Holders, []string{alice, bob}) {
				t.Fatalf("after %s: assigned %v holders %v", op, got.Assigned, got.Holders)
			}
			if v := e.c.Board(BoardFilter{For: bob}); boardHas(v.NeedsYou, task.ID) || !boardHas(v.Working, task.ID) {
				t.Fatalf("after %s: bob's board needsYou %d working %d", op, len(v.NeedsYou), len(v.Working))
			}
			// Claiming again is still a no-op.
			must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
			if n := len(e.rows(rows)); n != wantRows {
				t.Fatalf("repeat: %s rows = %d", rows, n)
			}
			r, _ := e.reopen().GetTask(task.ID)
			if !reflect.DeepEqual(r, got) {
				t.Fatalf("replay:\n%+v\n%+v", r, got)
			}
		})
	}
}

// Assigned follows the holders: a handoff on an unshared task replaces it,
// leaving drops the assignee, and the assignee posting status takes it up.
func TestAssignedFollowsHolders(t *testing.T) {
	e := newEnv(t)
	un := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "u"}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, un.ID))
	got, _ := mustN(t)(e.c.Assign(ctx, alice, un.ID, bob))
	if !reflect.DeepEqual(got.Assigned, []string{bob}) || got.State != Working {
		t.Fatalf("handoff: %+v", got)
	}
	got, _ = mustN(t)(e.c.Assign(ctx, bob, un.ID, carol))
	if !reflect.DeepEqual(got.Assigned, []string{carol}) {
		t.Fatalf("second handoff: assigned %v", got.Assigned)
	}
	got = must[Task](t)(e.c.SetTaskState(ctx, carol, un.ID, InputRequired, "?"))
	if got.Assigned != nil {
		t.Fatalf("status by the assignee: assigned %v", got.Assigned)
	}

	sh := shared(t, e, alice)
	mustN(t)(e.c.Assign(ctx, alice, sh.ID, bob))
	mustN(t)(e.c.Assign(ctx, alice, sh.ID, carol))
	got = must[Task](t)(e.c.Leave(ctx, bob, sh.ID, bob))
	if !reflect.DeepEqual(got.Assigned, []string{carol}) || !reflect.DeepEqual(got.Holders, []string{carol}) {
		t.Fatalf("leave: assigned %v holders %v", got.Assigned, got.Holders)
	}
}

// Irori's own key, and keys that aren't one line of bounded length, can't
// be assigned or join.
func TestAssignAndJoinCheckKeys(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	sh := shared(t, e, alice)
	for _, k := range []string{sweeper, "a\nb", "a\tb", strings.Repeat("k", MaxKey+1)} {
		_, _, err := e.c.Assign(ctx, alice, task.ID, k)
		wantErr(t, err, ErrInvalid)
		_, err = e.c.AssignTask(ctx, alice, task.ID, k)
		wantErr(t, err, ErrInvalid)
		_, err = e.c.Join(ctx, user, sh.ID, k)
		wantErr(t, err, ErrInvalid)
	}
	if n := len(e.rows(KindTaskAssigned)) + len(e.rows(KindTaskJoined)); n != 0 {
		t.Fatalf("rows written: %d", n)
	}
	mustN(t)(e.c.Assign(ctx, alice, task.ID, strings.Repeat("k", MaxKey)))
}

// A task that is already done can't block: the sweeper would close the
// blocker at once.
func TestBlockOnTerminalTaskRefused(t *testing.T) {
	e := newEnv(t)
	a := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "a"}))
	b := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "b"}))
	must[Task](t)(e.c.SetTaskState(ctx, alice, b.ID, Completed, ""))
	_, err := e.c.Block(ctx, alice, a.ID, "waiting", &BlockedBy{Kind: BlockedOnTask, Ref: b.ID})
	wantErr(t, err, ErrInvalid)
	_, err = e.c.CreateTask(ctx, alice, NewTask{Title: "c", BlockReason: "waiting", BlockedBy: &BlockedBy{Kind: BlockedOnTask, Ref: b.ID}})
	wantErr(t, err, ErrInvalid)
	ns, err := e.c.SweepNotices(ctx)
	if err != nil || len(ns) != 0 {
		t.Fatalf("sweep: %v %v", ns, err)
	}
}
