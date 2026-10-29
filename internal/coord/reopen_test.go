package coord

import (
	"reflect"
	"strings"
	"testing"
	"time"
)

// Closed cards (card #28): an answered ask completes, a finished card can
// be reopened, and the person's comment on one someone holds reopens it.

func TestAnsweredAskCompletes(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "what port does the server use?")
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, tk.ID, time.Second))
	got := must[Task](t)(e.c.AskClose(ctx, alice, tk.ID, AskOutcome{Kind: OutcomeAnswered, Note: "8080, said on the card"}))
	if got.State != Completed || got.Ask.Open() || got.Ask.Outcome == nil || got.Ask.Outcome.Kind != OutcomeAnswered ||
		got.Ask.Outcome.Note != "8080, said on the card" {
		t.Fatalf("answered outcome %+v", got)
	}
	if !reflect.DeepEqual(got.Holders, []string{alice}) {
		t.Fatalf("the taker should still hold it: %v", got.Holders)
	}
	b, _ := e.reopen().GetTask(tk.ID)
	if b.State != Completed || !reflect.DeepEqual(b.Ask, got.Ask) {
		t.Fatalf("replay differs:\n%+v\n%+v", got, b)
	}
	wantErr(t, errOnly(e.c.AskClose(ctx, alice, tk.ID, AskOutcome{Kind: "maybe", Note: "x"})), ErrBadState) // not an open ask now
	other, _ := postAsk(t, e, "x")
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, other.ID, time.Second))
	wantErr(t, errOnly(e.c.AskClose(ctx, alice, other.ID, AskOutcome{Kind: "maybe", Note: "x"})), ErrInvalid)
}

func TestReopenKeepsHolders(t *testing.T) {
	e := newEnv(t)
	tk := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "Fix the build"}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, tk.ID))
	wantErr(t, errOnly(e.c.Reopen(ctx, user, tk.ID)), ErrBadState) // still open
	must[Task](t)(e.c.SetTaskState(ctx, alice, tk.ID, Completed, "done"))
	wantErr(t, errOnly(e.c.Reopen(ctx, bob, tk.ID)), ErrNotAllowed)
	got := must[Task](t)(e.c.Reopen(ctx, user, tk.ID))
	if got.State != Working || got.Note != ReopenNote || !reflect.DeepEqual(got.Holders, []string{alice}) {
		t.Fatalf("reopened %+v", got)
	}
	if acts := e.c.Activity(tk.ID); acts[0].What != "reopened it" || acts[0].By != user {
		t.Fatalf("history line %+v", acts[0])
	}
	// Nobody holds it: it comes back open.
	free := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "Unheld"}))
	must[Task](t)(e.c.SetTaskState(ctx, user, free.ID, Canceled, ""))
	if got := must[Task](t)(e.c.Reopen(ctx, user, free.ID)); got.State != Submitted {
		t.Fatalf("unheld reopened %+v", got)
	}
}

func TestReopenSubtasksAndParents(t *testing.T) {
	e := newEnv(t)
	p := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "Parent"}))
	k := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "Child", Parent: p.ID}))
	must[Task](t)(e.c.SetTaskState(ctx, user, k.ID, Completed, ""))
	must[Task](t)(e.c.SetTaskState(ctx, user, p.ID, Completed, ""))
	// The parent's state follows its subtasks, which have all ended.
	wantErr(t, errOnly(e.c.Reopen(ctx, user, p.ID)), ErrSubtask)
	// Reopening the subtask brings the parent back with it.
	must[Task](t)(e.c.Reopen(ctx, user, k.ID))
	pt, _ := e.c.GetTask(p.ID)
	kt, _ := e.c.GetTask(k.ID)
	if IsTerminal(pt.State) || IsTerminal(EffectiveState(pt)) || IsTerminal(kt.State) {
		t.Fatalf("parent %s (%s), child %s", pt.State, EffectiveState(pt), kt.State)
	}
}

func TestPersonCommentReopensAFinishedCard(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "what port does the server use?")
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, tk.ID, time.Second))
	must[Task](t)(e.c.AskClose(ctx, alice, tk.ID, AskOutcome{Kind: OutcomeAnswered, Note: "8080"}))
	_, ns, err := e.c.PersonComment(ctx, user, tk.ID, "and in dev?", false, nil)
	if err != nil || len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{alice}) || !strings.Contains(ns[0].Text, "open again") {
		t.Fatalf("%+v %v", ns, err)
	}
	got, _ := e.c.GetTask(tk.ID)
	if got.State != Working || got.Note != ReopenCommentNote || !reflect.DeepEqual(got.Holders, []string{alice}) {
		t.Fatalf("after the comment %+v", got)
	}
	// The reopen line comes before the comment.
	acts := e.c.Activity(tk.ID)
	if len(acts) < 2 || acts[1].What != "reopened it" {
		t.Fatalf("activity %+v", acts)
	}
	// A finished card nobody holds stays finished; the comment stays on it.
	free := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "Unheld"}))
	must[Task](t)(e.c.SetTaskState(ctx, user, free.ID, Completed, ""))
	must[Comment](t)(func() (Comment, error) {
		cm, _, err := e.c.PersonComment(ctx, user, free.ID, "fine", false, nil)
		return cm, err
	}())
	if ft, _ := e.c.GetTask(free.ID); ft.State != Completed {
		t.Fatalf("unheld card %+v", ft)
	}
}

func TestWatchAFinishedCard(t *testing.T) {
	e := newEnv(t)
	tk := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "Done thing"}))
	must[Task](t)(e.c.SetTaskState(ctx, user, tk.ID, Completed, ""))
	got := must[Task](t)(e.c.Watch(ctx, bob, tk.ID))
	if !reflect.DeepEqual(got.Watchers, []string{bob}) {
		t.Fatalf("watchers %v", got.Watchers)
	}
}
