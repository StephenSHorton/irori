package coord

import (
	"reflect"
	"strings"
	"testing"
	"time"
)

func bp(b bool) *bool { return &b }

func stepIDs(ss []Step) []string {
	var out []string
	for _, s := range ss {
		out = append(out, s.ID)
	}
	return out
}

// claimed is a task alice created and holds.
func claimed(t *testing.T, e *env) Task {
	t.Helper()
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "ship it"}))
	return must[Task](t)(e.c.ClaimTask(ctx, alice, task.ID))
}

// ---- steps ----

func TestSetSteps(t *testing.T) {
	long := strings.Repeat("x", MaxStepText+1)
	many := make([]Step, MaxSteps+1)
	for i := range many {
		many[i] = Step{Text: "s"}
	}
	cases := []struct {
		name    string
		by      string
		steps   []Step
		wantErr error
		wantIDs []string
	}{
		{"holder sets three", alice, []Step{{Text: "a", Done: true}, {Text: "b", Doing: true}, {Text: "c"}}, nil, []string{"s1", "s2", "s3"}},
		{"person may", user, []Step{{Text: "a"}}, nil, []string{"s1"}},
		{"creator may", alice, []Step{{Text: "a"}}, nil, []string{"s1"}},
		{"stranger may not", carol, []Step{{Text: "a"}}, ErrNotAllowed, nil},
		{"empty text", alice, []Step{{Text: "  "}}, ErrInvalid, nil},
		{"too long", alice, []Step{{Text: long}}, ErrInvalid, nil},
		{"two lines", alice, []Step{{Text: "a\nb"}}, ErrInvalid, nil},
		{"too many", alice, many, ErrInvalid, nil},
		{"two doing", alice, []Step{{Text: "a", Doing: true}, {Text: "b", Doing: true}}, ErrInvalid, nil},
		{"done and doing", alice, []Step{{Text: "a", Done: true, Doing: true}}, ErrInvalid, nil},
		{"empty list clears nothing: no-op", alice, nil, nil, nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			got, err := e.c.SetSteps(ctx, tc.by, task.ID, tc.steps)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				if n := len(e.rows(KindTaskSteps)); n != 0 {
					t.Fatalf("a refused call wrote %d rows", n)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(stepIDs(got.Steps), tc.wantIDs) {
				t.Fatalf("ids %v, want %v", stepIDs(got.Steps), tc.wantIDs)
			}
		})
	}
}

func TestSetStepsKeepsIDs(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	must[Task](t)(e.c.SetSteps(ctx, alice, task.ID, []Step{{Text: "a"}, {Text: "b"}, {Text: "c"}}))
	// Kept by id (renamed), kept by text (reordered), dropped, and new.
	got := must[Task](t)(e.c.SetSteps(ctx, alice, task.ID, []Step{{Text: "c"}, {ID: "s1", Text: "a, renamed", Done: true}, {Text: "d"}}))
	want := []Step{{ID: "s3", Text: "c"}, {ID: "s1", Text: "a, renamed", Done: true}, {ID: "s4", Text: "d"}}
	if !reflect.DeepEqual(got.Steps, want) {
		t.Fatalf("steps %+v, want %+v", got.Steps, want)
	}
	// The same list again writes nothing; a dropped id is never minted again.
	must[Task](t)(e.c.SetSteps(ctx, alice, task.ID, want))
	if n := len(e.rows(KindTaskSteps)); n != 2 {
		t.Fatalf("steps rows = %d, want 2", n)
	}
	got = must[Task](t)(e.c.SetSteps(ctx, alice, task.ID, append(want, Step{Text: "e"})))
	if id := got.Steps[3].ID; id != "s5" {
		t.Fatalf("new id %q, want s5 (s2 was dropped, never reused)", id)
	}
	if d, n := got.Progress(); d != 1 || n != 4 {
		t.Fatalf("progress %d/%d", d, n)
	}
	if d, n := (Task{}).Progress(); d != 0 || n != 0 {
		t.Fatalf("no steps: %d/%d", d, n)
	}
}

func TestSetStep(t *testing.T) {
	cases := []struct {
		name     string
		by, step string
		u        StepUpdate
		wantErr  error
		want     []Step // after; nil = unchanged
	}{
		{"tick", alice, "s1", StepUpdate{Done: bp(true)}, nil,
			[]Step{{ID: "s1", Text: "a", Done: true}, {ID: "s2", Text: "b", Doing: true}, {ID: "s3", Text: "c"}}},
		{"mark current moves doing", alice, "s3", StepUpdate{Doing: bp(true)}, nil,
			[]Step{{ID: "s1", Text: "a"}, {ID: "s2", Text: "b"}, {ID: "s3", Text: "c", Doing: true}}},
		{"done clears doing", alice, "s2", StepUpdate{Done: bp(true)}, nil,
			[]Step{{ID: "s1", Text: "a"}, {ID: "s2", Text: "b", Done: true}, {ID: "s3", Text: "c"}}},
		{"untick doing step stops it", alice, "s2", StepUpdate{Doing: bp(false)}, nil,
			[]Step{{ID: "s1", Text: "a"}, {ID: "s2", Text: "b"}, {ID: "s3", Text: "c"}}},
		{"no change is a no-op", alice, "s1", StepUpdate{Done: bp(false)}, nil, nil},
		{"person may", user, "s1", StepUpdate{Done: bp(true)}, nil,
			[]Step{{ID: "s1", Text: "a", Done: true}, {ID: "s2", Text: "b", Doing: true}, {ID: "s3", Text: "c"}}},
		{"stranger may not", carol, "s1", StepUpdate{Done: bp(true)}, ErrNotAllowed, nil},
		{"unknown step", alice, "s9", StepUpdate{Done: bp(true)}, ErrNotFound, nil},
		{"nothing given", alice, "s1", StepUpdate{}, ErrInvalid, nil},
		{"done and doing", alice, "s1", StepUpdate{Done: bp(true), Doing: bp(true)}, ErrInvalid, nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			before := must[Task](t)(e.c.SetSteps(ctx, alice, task.ID, []Step{{Text: "a"}, {Text: "b", Doing: true}, {Text: "c"}}))
			got, err := e.c.SetStep(ctx, tc.by, task.ID, tc.step, tc.u)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			want, rows := tc.want, 1
			if want == nil {
				want, rows = before.Steps, 0
			}
			if !reflect.DeepEqual(got.Steps, want) {
				t.Fatalf("steps %+v, want %+v", got.Steps, want)
			}
			if n := len(e.rows(KindTaskStep)); n != rows {
				t.Fatalf("step rows = %d, want %d", n, rows)
			}
		})
	}
}

// ---- blockers ----

func TestRaiseValidation(t *testing.T) {
	tooMany := make([]Option, MaxOptions+1)
	for i := range tooMany {
		tooMany[i] = Option{Answer: strings.Repeat("a", i+1)}
	}
	cases := []struct {
		name    string
		by      string
		nb      NewBlocker
		wantErr error
	}{
		{"holder with options", alice, NewBlocker{Question: "Ship today?", Context: "CI passes.", Options: []Option{{Answer: "Yes", Why: "it's ready"}, {Answer: "No"}}}, nil},
		{"person", user, NewBlocker{Question: "q"}, nil},
		{"stranger", carol, NewBlocker{Question: "q"}, ErrNotAllowed},
		{"no question", alice, NewBlocker{Question: " "}, ErrInvalid},
		{"question too long", alice, NewBlocker{Question: strings.Repeat("q", MaxQuestion+1)}, ErrInvalid},
		{"question at the cap", alice, NewBlocker{Question: strings.Repeat("é", MaxQuestion)}, nil},
		{"context too long", alice, NewBlocker{Question: "q", Context: strings.Repeat("c", MaxContext+1)}, ErrInvalid},
		{"too many options", alice, NewBlocker{Question: "q", Options: tooMany}, ErrInvalid},
		{"empty option", alice, NewBlocker{Question: "q", Options: []Option{{Answer: " "}}}, ErrInvalid},
		{"answer too long", alice, NewBlocker{Question: "q", Options: []Option{{Answer: strings.Repeat("a", MaxAnswer+1)}}}, ErrInvalid},
		{"why too long", alice, NewBlocker{Question: "q", Options: []Option{{Answer: "a", Why: strings.Repeat("w", MaxWhy+1)}}}, ErrInvalid},
		{"bad blocked-by", alice, NewBlocker{Question: "q", BlockedBy: &BlockedBy{Kind: "weather"}}, ErrInvalid},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			got, b, err := e.c.Raise(ctx, tc.by, task.ID, tc.nb)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				if g, _ := e.c.GetTask(task.ID); g.State != Working || len(g.Blockers) != 0 {
					t.Fatalf("a refused raise changed the task: %+v", g)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			q := strings.TrimSpace(tc.nb.Question)
			if got.State != Blocked || got.Reason != q || b.ID != "b1" || b.Question != q || b.RaisedBy != tc.by || !b.Open() {
				t.Fatalf("task %+v item %+v", got, b)
			}
			for _, o := range b.Options {
				if o.By != tc.by || o.At.IsZero() {
					t.Fatalf("option %+v", o)
				}
			}
			if len(b.Options) != len(tc.nb.Options) {
				t.Fatalf("options %+v", b.Options)
			}
		})
	}
}

func TestRaiseAddsDedupesAndCaps(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	must[Lease](t)(e.c.Acquire(ctx, bob, "gpu/qwen", time.Hour, false))
	_, b1, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "one", BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: "gpu/qwen"}})
	if err != nil {
		t.Fatal(err)
	}
	_, again, _ := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "one"})
	if again.ID != b1.ID {
		t.Fatalf("same question raised a new item %+v", again)
	}
	got, b2, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "two"})
	if err != nil {
		t.Fatal(err)
	}
	// A second item without a BlockedBy keeps the task's.
	if b2.ID != "b2" || got.Reason != "one" || got.BlockedBy == nil || got.BlockedBy.Kind != BlockedOnLease || len(got.OpenBlockers()) != 2 {
		t.Fatalf("got %+v", got)
	}
	for i := 3; i <= MaxOpenBlockers; i++ {
		must[Task](t)(func() (Task, error) {
			t, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: strings.Repeat("q", i)})
			return t, err
		}())
	}
	_, _, err = e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "one too many"})
	wantErr(t, err, ErrInvalid)
	must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Completed, ""))
	_, _, err = e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "late"})
	wantErr(t, err, ErrBadState)
}

func TestRecommend(t *testing.T) {
	cases := []struct {
		name    string
		by      string
		setup   func(e *env, id string)
		blocker string
		answer  string
		wantErr error
		wantN   int // options after
	}{
		{"holder", alice, nil, "b1", "Wait", nil, 2},
		{"watcher", bob, func(e *env, id string) { must[Task](e.t)(e.c.Watch(ctx, bob, id)) }, "b1", "Wait", nil, 2},
		{"person", user, nil, "b1", "Wait", nil, 2},
		{"stranger must watch first", carol, nil, "b1", "Wait", ErrNotAllowed, 1},
		{"same answer is a no-op", alice, nil, "b1", "ship now", nil, 1},
		{"unknown blocker", alice, nil, "b9", "Wait", ErrNotFound, 1},
		{"empty answer", alice, nil, "b1", " ", ErrInvalid, 1},
		{"resolved blocker", alice, func(e *env, id string) {
			mustN(e.t)(e.c.Answer(ctx, user, id, "b1", BlockerAnswer{Pick: 1}))
		}, "b1", "Wait", ErrBadState, 1},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			must[Task](t)(func() (Task, error) {
				t, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "Ship?", Options: []Option{{Answer: "Ship now"}}})
				return t, err
			}())
			if tc.setup != nil {
				tc.setup(e, task.ID)
			}
			_, err := e.c.Recommend(ctx, tc.by, task.ID, tc.blocker, tc.answer, "because")
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
			} else if err != nil {
				t.Fatal(err)
			}
			got, _ := e.c.GetTask(task.ID)
			if n := len(got.Blockers[0].Options); n != tc.wantN {
				t.Fatalf("options = %d, want %d", n, tc.wantN)
			}
			if tc.wantErr == nil && tc.wantN == 2 {
				if o := got.Blockers[0].Options[1]; o.By != tc.by || o.Answer != tc.answer || o.Why != "because" {
					t.Fatalf("option %+v", o)
				}
			}
		})
	}
}

func TestRecommendCap(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	opts := make([]Option, MaxOptions)
	for i := range opts {
		opts[i] = Option{Answer: strings.Repeat("o", i+1)}
	}
	if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "q", Options: opts}); err != nil {
		t.Fatal(err)
	}
	_, err := e.c.Recommend(ctx, alice, task.ID, "b1", "another", "")
	wantErr(t, err, ErrInvalid)
}

func TestAnswer(t *testing.T) {
	cases := []struct {
		name      string
		by        string
		blocker   string
		a         BlockerAnswer
		wantErr   error
		wantState TaskState
		wantAns   string
		wantOpt   int
		wantTo    []string
	}{
		{"person picks an option, last item", user, "b2", BlockerAnswer{Pick: 2, Wake: true}, nil, Working, "TAP-Windows", 1, []string{bob}},
		{"own words", user, "b2", BlockerAnswer{Text: " use both "}, nil, Working, "use both", -1, []string{bob}},
		{"an agent may (decision 19)", carol, "b2", BlockerAnswer{Pick: 1}, nil, Working, "Wintun", 0, []string{bob}},
		{"the raiser itself: holders hear", bob, "b2", BlockerAnswer{Pick: 1}, nil, Working, "Wintun", 0, []string{alice}},
		{"pick out of range", user, "b2", BlockerAnswer{Pick: 3}, ErrInvalid, "", "", 0, nil},
		{"negative pick", user, "b2", BlockerAnswer{Pick: -1}, ErrInvalid, "", "", 0, nil},
		{"no text", user, "b2", BlockerAnswer{Text: "  "}, ErrInvalid, "", "", 0, nil},
		{"text too long", user, "b2", BlockerAnswer{Text: strings.Repeat("x", MaxOwnAnswer+1)}, ErrInvalid, "", "", 0, nil},
		{"unknown item", user, "b7", BlockerAnswer{Pick: 1}, ErrNotFound, "", "", 0, nil},
		{"already resolved", user, "b1", BlockerAnswer{Pick: 1}, ErrBadState, "", "", 0, nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			must[Task](t)(e.c.Watch(ctx, bob, task.ID))
			if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "first?", Options: []Option{{Answer: "yes"}}}); err != nil {
				t.Fatal(err)
			}
			// bob, a watcher, may recommend but not raise; he raises once he holds it.
			task = must[Task](t)(e.c.SetShared(ctx, alice, task.ID, true))
			must[Task](t)(e.c.Join(ctx, bob, task.ID, bob))
			if _, _, err := e.c.Raise(ctx, bob, task.ID, NewBlocker{Question: "Which adapter?", Context: "Radmin isn't open source.",
				Options: []Option{{Answer: "Wintun", Why: "signed, modern"}, {Answer: "TAP-Windows", Why: "broadcasts just work"}}}); err != nil {
				t.Fatal(err)
			}
			// Resolve b1 first: the task stays blocked on b2.
			mid, n1 := mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Pick: 1}))
			if mid.State != Blocked || mid.Reason != "Which adapter?" || len(mid.OpenBlockers()) != 1 ||
				!reflect.DeepEqual(n1.To, []string{alice}) || !strings.Contains(n1.Text, "1 more blockers are still open") {
				t.Fatalf("after b1: %+v notice %+v", mid, n1)
			}
			got, n, err := e.c.Answer(ctx, tc.by, task.ID, tc.blocker, tc.a)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				if g, _ := e.c.GetTask(task.ID); g.State != Blocked {
					t.Fatalf("a refused answer left blocked: %+v", g)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			b := got.Blockers[1]
			if got.State != tc.wantState || got.Reason != "" || got.BlockedBy != nil || b.Open() ||
				b.Resolved.Answer != tc.wantAns || b.Resolved.Option != tc.wantOpt || b.Resolved.By != tc.by || b.Resolved.Closed {
				t.Fatalf("got %+v item %+v resolved %+v", got, b, b.Resolved)
			}
			if n.Kind != NoticeAnswered || !reflect.DeepEqual(n.To, tc.wantTo) || n.Wake != tc.a.Wake || n.Ref != "task:"+task.ID ||
				!strings.Contains(n.Text, "Which adapter?") || !strings.Contains(n.Text, tc.wantAns) || !strings.Contains(n.Text, "working again") {
				t.Fatalf("notice %+v", n)
			}
			if tc.a.Pick > 0 && !strings.Contains(n.Text, "recommended") {
				t.Fatalf("a picked option should say who recommended it: %q", n.Text)
			}
			rows := e.rows(KindBlockerResolved)
			if p := decodeRow[blockerResolvedRow](t, rows[len(rows)-1].Payload); p.Restored != Working || p.Option != tc.wantOpt {
				t.Fatalf("row %+v", p)
			}
		})
	}
}

func TestAnswerUnclaimedRestoresSubmitted(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "nobody on it"}))
	if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "q"}); err != nil {
		t.Fatal(err)
	}
	got, n := mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "go"}))
	// The raiser, alice, hears even though she doesn't hold it.
	if got.State != Submitted || !reflect.DeepEqual(n.To, []string{alice}) {
		t.Fatalf("got %+v notice %+v", got, n)
	}
}

func TestLeavingBlockedClosesItems(t *testing.T) {
	cases := []struct {
		name  string
		leave func(e *env, id string) Task
		by    string
		note  string
	}{
		{"unblock resolves all with the note", func(e *env, id string) Task {
			got, _ := mustN(e.t)(e.c.Unblock(ctx, carol, id, "sorted it"))
			return got
		}, carol, "sorted it"},
		{"a state change closes them", func(e *env, id string) Task {
			return must[Task](e.t)(e.c.SetTaskState(ctx, alice, id, Working, "moving on"))
		}, alice, "moving on"},
		{"the sweeper, when the lease goes", func(e *env, id string) Task {
			if err := e.c.Release(ctx, bob, "gpu/qwen"); err != nil {
				e.t.Fatal(err)
			}
			if _, err := e.c.SweepNotices(ctx); err != nil {
				e.t.Fatal(err)
			}
			got, _ := e.c.GetTask(id)
			return got
		}, sweeper, "lease released"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			must[Lease](t)(e.c.Acquire(ctx, bob, "gpu/qwen", time.Hour, false))
			if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "one", BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: "gpu/qwen"}}); err != nil {
				t.Fatal(err)
			}
			if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "two"}); err != nil {
				t.Fatal(err)
			}
			got := tc.leave(e, task.ID)
			if got.State != Working || got.Reason != "" || len(got.OpenBlockers()) != 0 || len(got.Blockers) != 2 {
				t.Fatalf("got %+v", got)
			}
			for _, b := range got.Blockers {
				if r := b.Resolved; r == nil || !r.Closed || r.By != tc.by || r.Answer != tc.note || r.Option != -1 {
					t.Fatalf("item %+v resolved %+v", b, b.Resolved)
				}
			}
		})
	}
}

func TestBlockKeepsOneReasonItem(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	must[Task](t)(e.c.Block(ctx, alice, task.ID, "waiting on the db", nil))
	if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "Which db?", Options: []Option{{Answer: "postgres"}}}); err != nil {
		t.Fatal(err)
	}
	got := must[Task](t)(e.c.Block(ctx, alice, task.ID, "still waiting on the db", nil))
	if len(got.Blockers) != 2 || got.Blockers[0].ID != "b1" || got.Blockers[0].Question != "still waiting on the db" ||
		got.Blockers[1].Question != "Which db?" || got.Reason != "still waiting on the db" {
		t.Fatalf("got %+v", got.Blockers)
	}
	// A reason longer than a question goes whole into the context.
	long := strings.Repeat("r", MaxQuestion+50)
	got = must[Task](t)(e.c.Block(ctx, alice, task.ID, long, nil))
	if b := got.Blockers[0]; utf8Len(b.Question) != MaxQuestion || b.Context != long || got.Reason != long {
		t.Fatalf("long reason item %+v", b)
	}
	// NewTask.BlockReason makes the same kind of item.
	nt := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "port", BlockReason: "a dead dev server has it"}))
	if len(nt.Blockers) != 1 || nt.Blockers[0].ID != "b1" || nt.Blockers[0].Question != "a dead dev server has it" {
		t.Fatalf("created blocked: %+v", nt.Blockers)
	}
}

func utf8Len(s string) int { return len([]rune(s)) }

func TestResolvedItemsAreTrimmed(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	for i := 0; i < KeepResolved+3; i++ {
		_, b, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "q" + strings.Repeat("x", i)})
		if err != nil {
			t.Fatal(err)
		}
		mustN(t)(e.c.Answer(ctx, user, task.ID, b.ID, BlockerAnswer{Text: "ok"}))
	}
	got, _ := e.c.GetTask(task.ID)
	if len(got.Blockers) != KeepResolved || got.Blockers[0].ID != "b4" {
		t.Fatalf("kept %d, first %s", len(got.Blockers), got.Blockers[0].ID)
	}
}

// ---- watchers ----

func TestWatchRules(t *testing.T) {
	cases := []struct {
		name    string
		by      string
		done    bool
		wantErr error
	}{
		{"an agent", bob, false, nil},
		{"a person can't", user, false, ErrInvalid},
		{"irori can't", sweeper, false, ErrInvalid},
		{"a done card can't be watched", bob, true, ErrBadState},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			if tc.done {
				must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Completed, ""))
			}
			got, err := e.c.Watch(ctx, tc.by, task.ID)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			must[Task](t)(e.c.Watch(ctx, tc.by, task.ID)) // twice: no-op
			if !reflect.DeepEqual(got.Watchers, []string{tc.by}) || len(e.rows(KindTaskWatched)) != 1 {
				t.Fatalf("watchers %v", got.Watchers)
			}
			got = must[Task](t)(e.c.Unwatch(ctx, tc.by, task.ID))
			must[Task](t)(e.c.Unwatch(ctx, tc.by, task.ID)) // not watching: no-op
			if len(got.Watchers) != 0 || len(e.rows(KindTaskWatched)) != 2 {
				t.Fatalf("after unwatch %v", got.Watchers)
			}
		})
	}
}

func TestWatchNotices(t *testing.T) {
	cases := []struct {
		name      string
		do        func(e *env, id string)
		wantTo    []string
		wantLines []string // substrings
	}{
		{"steps and a tick", func(e *env, id string) {
			must[Task](e.t)(e.c.SetSteps(ctx, alice, id, []Step{{Text: "build"}, {Text: "tag"}}))
			must[Task](e.t)(e.c.SetStep(ctx, alice, id, "s1", StepUpdate{Done: bp(true)}))
		}, []string{bob}, []string{alice + " set the steps (0/2 done)", alice + " ticked “build” (1/2)", "steps 1/2"}},
		{"a blocker raised and answered", func(e *env, id string) {
			if _, _, err := e.c.Raise(ctx, alice, id, NewBlocker{Question: "Ship?"}); err != nil {
				e.t.Fatal(err)
			}
			mustN(e.t)(e.c.Answer(ctx, user, id, "b1", BlockerAnswer{Text: "yes"}))
		}, []string{bob}, []string{alice + " raised a blocker: Ship?", user + " answered “Ship?”: yes"}},
		{"a state change", func(e *env, id string) {
			must[Task](e.t)(e.c.SetTaskState(ctx, alice, id, Completed, ""))
		}, []string{bob}, []string{alice + " set it to completed", "Now: completed"}},
		{"a PR link, not other links", func(e *env, id string) {
			must[Task](e.t)(e.c.AddLink(ctx, alice, id, "https://linear.app/x/issue/ABC-1"))
			must[Task](e.t)(e.c.AddLink(ctx, alice, id, "https://github.com/acme/app/pull/57"))
		}, []string{bob}, []string{alice + " linked GitHub #57"}},
		{"a recommendation tells the raiser too", func(e *env, id string) {
			if _, _, err := e.c.Raise(ctx, alice, id, NewBlocker{Question: "Ship?"}); err != nil {
				e.t.Fatal(err)
			}
			if _, err := e.c.SweepNotices(ctx); err != nil { // drain the raise
				e.t.Fatal(err)
			}
			must[Task](e.t)(e.c.Recommend(ctx, bob, id, "b1", "Wait", "the temple is untested"))
		}, []string{alice}, []string{bob + " recommended “Wait” for “Ship?”"}},
		{"comments, joins and watches don't count", func(e *env, id string) {
			mustN(e.t)(func() (Task, Notice, error) {
				_, n, err := e.c.AddComment(ctx, alice, id, "hi", false)
				return Task{}, n, err
			}())
			must[Task](e.t)(e.c.Watch(ctx, carol, id))
		}, nil, nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := claimed(t, e)
			must[Task](t)(e.c.Watch(ctx, bob, task.ID))
			tc.do(e, task.ID)
			ns, err := e.c.SweepNotices(ctx)
			if err != nil {
				t.Fatal(err)
			}
			var watched []Notice
			for _, n := range ns {
				if n.Kind == NoticeWatched {
					watched = append(watched, n)
				}
			}
			if tc.wantTo == nil {
				if len(watched) != 0 {
					t.Fatalf("notices %+v", watched)
				}
				return
			}
			if len(watched) != 1 || !reflect.DeepEqual(watched[0].To, tc.wantTo) || watched[0].By != sweeper || watched[0].Wake ||
				watched[0].Ref != "task:"+task.ID {
				t.Fatalf("notices %+v", watched)
			}
			for _, l := range tc.wantLines {
				if !strings.Contains(watched[0].Text, l) {
					t.Fatalf("text %q lacks %q", watched[0].Text, l)
				}
			}
			if strings.Contains(watched[0].Text, "linear") || strings.Contains(watched[0].Text, "Linear") {
				t.Fatalf("a non-PR link reached the watcher: %q", watched[0].Text)
			}
			// Drained: the next sweep has nothing.
			if ns, _ := e.c.SweepNotices(ctx); len(ns) != 0 {
				t.Fatalf("second sweep %+v", ns)
			}
		})
	}
}

func TestWatchNoticeSkipsOwnChanges(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	must[Task](t)(e.c.Watch(ctx, alice, task.ID)) // the holder watches its own card
	must[Task](t)(e.c.Watch(ctx, bob, task.ID))
	must[Task](t)(e.c.SetSteps(ctx, alice, task.ID, []Step{{Text: "a"}}))
	ns := must[[]Notice](t)(e.c.SweepNotices(ctx))
	if len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{bob}) {
		t.Fatalf("notices %+v", ns)
	}
	// Two watchers reading the same lines share one notice.
	must[Task](t)(e.c.Watch(ctx, carol, task.ID))
	must[Task](t)(e.c.SetStep(ctx, user, task.ID, "s1", StepUpdate{Done: bp(true)}))
	ns = must[[]Notice](t)(e.c.SweepNotices(ctx))
	if len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{alice, bob, carol}) {
		t.Fatalf("notices %+v", ns)
	}
	// Unwatching stops them; a replay queues none.
	must[Task](t)(e.c.Unwatch(ctx, bob, task.ID))
	must[Task](t)(e.c.SetStep(ctx, alice, task.ID, "s1", StepUpdate{Done: bp(false)}))
	c2 := e.reopen()
	if ns := must[[]Notice](t)(c2.SweepNotices(ctx)); len(ns) != 0 {
		t.Fatalf("replay queued %+v", ns)
	}
	ns = must[[]Notice](t)(e.c.SweepNotices(ctx))
	if len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{carol}) {
		t.Fatalf("after unwatch %+v", ns)
	}
}

// ---- replay ----

func TestCardsReplay(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	must[Task](t)(e.c.Watch(ctx, bob, task.ID))
	must[Task](t)(e.c.SetSteps(ctx, alice, task.ID, []Step{{Text: "a", Done: true}, {Text: "b", Doing: true}}))
	must[Task](t)(e.c.SetStep(ctx, alice, task.ID, "s2", StepUpdate{Done: bp(true)}))
	must[Task](t)(e.c.Block(ctx, alice, task.ID, "legacy reason", nil))
	if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "Ship?", Context: "ctx", Options: []Option{{Answer: "yes", Why: "ready"}}}); err != nil {
		t.Fatal(err)
	}
	must[Task](t)(e.c.Recommend(ctx, bob, task.ID, "b2", "no", "wait"))
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "fine"}))
	e.clk.add(time.Minute)
	must[Task](t)(e.c.AddLink(ctx, alice, task.ID, "https://github.com/acme/app/pull/57"))
	want, _ := e.c.GetTask(task.ID)
	wantAct := e.c.Activity(task.ID)

	c2 := e.reopen()
	got, _ := c2.GetTask(task.ID)
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("replayed\n%+v\nwant\n%+v", got, want)
	}
	if a := c2.Activity(task.ID); !reflect.DeepEqual(a, wantAct) {
		t.Fatalf("activity\n%+v\nwant\n%+v", a, wantAct)
	}
	if len(wantAct) == 0 || wantAct[0].What != "linked GitHub #57" || wantAct[len(wantAct)-1].What != "put the card up" {
		t.Fatalf("activity newest first: %+v", wantAct)
	}
	// Ids keep counting after a replay.
	_, b, err := c2.Raise(ctx, alice, task.ID, NewBlocker{Question: "third"})
	if err != nil || b.ID != "b3" {
		t.Fatalf("raise after replay: %+v %v", b, err)
	}
}

func TestLegacyBlockedRowsFold(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	// Rows written before blocker items existed: reason only.
	for _, reason := range []string{"old reason", "replaced reason"} {
		if _, err := e.st.Append(ctx, "agent:"+alice, KindTaskState, map[string]any{
			"taskId": task.ID, "state": "blocked", "reason": reason, "by": alice, "at": fixedNow,
		}); err != nil {
			t.Fatal(err)
		}
	}
	c2 := e.reopen()
	got, _ := c2.GetTask(task.ID)
	if got.State != Blocked || got.Reason != "replaced reason" || len(got.Blockers) != 1 ||
		got.Blockers[0].ID != "b1" || got.Blockers[0].Question != "replaced reason" || got.Blockers[0].RaisedBy != alice {
		t.Fatalf("got %+v", got)
	}
	// It answers like any item.
	got, _ = mustN(t)(c2.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "done"}))
	if got.State != Working {
		t.Fatalf("after answer %+v", got)
	}
}

func TestActivityCap(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	for i := 0; i < MaxActivity+5; i++ {
		mustN(t)(func() (Task, Notice, error) {
			_, n, err := e.c.AddComment(ctx, alice, task.ID, "c", false)
			return Task{}, n, err
		}())
	}
	if a := e.c.Activity(task.ID); len(a) != MaxActivity {
		t.Fatalf("activity %d", len(a))
	}
	if e.c.Activity("nope") != nil {
		t.Fatal("unknown task has activity")
	}
}

func TestPRLink(t *testing.T) {
	cases := []struct {
		in   string
		repo string
		n    int
		ok   bool
	}{
		{"https://github.com/acme/app/pull/57", "acme/app", 57, true},
		{"https://www.github.com/acme/app/pull/57/files", "acme/app", 57, true},
		{"https://github.com/acme/app/issues/57", "", 0, false},
		{"https://evilgithub.com/acme/app/pull/57", "", 0, false},
		{"src/pull/57", "", 0, false},
	}
	for _, tc := range cases {
		l, err := ParseLink(tc.in)
		if err != nil {
			t.Fatal(err)
		}
		repo, n, ok := PRLink(l)
		if repo != tc.repo || n != tc.n || ok != tc.ok {
			t.Errorf("%s: %q %d %v", tc.in, repo, n, ok)
		}
	}
}
