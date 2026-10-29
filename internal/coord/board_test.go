package coord

import (
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/search"
)

const dave = "dev-a/dave-session"

func pathRes(glob string) string { return "path:" + repoA + ":" + glob }

func mustN(t *testing.T) func(Task, Notice, error) (Task, Notice) {
	return func(task Task, n Notice, err error) (Task, Notice) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
		return task, n
	}
}

func shared(t *testing.T, e *env, by string) Task {
	t.Helper()
	return must[Task](t)(e.c.CreateTask(ctx, by, NewTask{Title: "shared", Shared: true}))
}

func decodeRow[T any](t *testing.T, raw json.RawMessage) T {
	t.Helper()
	var v T
	if err := json.Unmarshal(raw, &v); err != nil {
		t.Fatal(err)
	}
	return v
}

// ---- holders ----

func TestHoldersClaimAssign(t *testing.T) {
	type step struct {
		by, op, who string
		want        []string
	}
	cases := []struct {
		name   string
		shared bool
		steps  []step
	}{
		{"unshared: claim then assign replace", false, []step{
			{alice, "claim", "", []string{alice}},
			{alice, "assign", bob, []string{bob}},
			{user, "assign", carol, []string{carol}},
			{carol, "claim", "", []string{carol}},
		}},
		{"shared: claim and assign append, no duplicates", true, []step{
			{alice, "claim", "", []string{alice}},
			{bob, "claim", "", []string{alice, bob}},
			{alice, "assign", carol, []string{alice, bob, carol}},
			{bob, "claim", "", []string{alice, bob, carol}},
			{alice, "assign", bob, []string{alice, bob, carol}},
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t", Shared: tc.shared}))
			for i, s := range tc.steps {
				var got Task
				var err error
				if s.op == "claim" {
					got, err = e.c.ClaimTask(ctx, s.by, task.ID)
				} else {
					got, _, err = e.c.Assign(ctx, s.by, task.ID, s.who)
				}
				if err != nil {
					t.Fatalf("step %d: %v", i, err)
				}
				if !reflect.DeepEqual(got.Holders, s.want) || got.Holder != s.want[0] {
					t.Fatalf("step %d: holders %v holder %q, want %v", i, got.Holders, got.Holder, s.want)
				}
			}
		})
	}
}

func TestSharedClaimNotHeld(t *testing.T) {
	e := newEnv(t)
	task := shared(t, e, alice)
	must[Task](t)(e.c.ClaimTask(ctx, alice, task.ID))
	got := must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID)) // no ErrHeld: joins
	if got.State != Working || len(got.Holders) != 2 {
		t.Fatalf("got %+v", got)
	}
	// Unshared keeps v0.2's single writer.
	t2 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "u"}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, t2.ID))
	_, err := e.c.ClaimTask(ctx, bob, t2.ID)
	wantErr(t, err, ErrHeld)
}

func TestHolderFilterAndPermissionsUseHolders(t *testing.T) {
	e := newEnv(t)
	task := shared(t, e, user)
	must[Task](t)(e.c.ClaimTask(ctx, alice, task.ID))
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	if got := e.c.ListTasks(TaskFilter{Holder: bob}); len(got) != 1 || got[0].ID != task.ID {
		t.Fatalf("holder filter on the second holder = %+v", got)
	}
	// The second holder may post status; a stranger may not.
	must[Task](t)(e.c.SetTaskState(ctx, bob, task.ID, InputRequired, "?"))
	_, err := e.c.SetTaskState(ctx, carol, task.ID, Working, "")
	wantErr(t, err, ErrNotAllowed)
	must[Task](t)(e.c.Block(ctx, bob, task.ID, "waiting", nil))
	_, err = e.c.Block(ctx, carol, task.ID, "x", nil)
	wantErr(t, err, ErrNotAllowed)
}

func TestObserveTracksEveryHolder(t *testing.T) {
	e := newEnv(t)
	task := shared(t, e, alice)
	must[Task](t)(e.c.ClaimTask(ctx, alice, task.ID))
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	e.ros.set(bob, "")
	if err := e.c.Sweep(ctx); err != nil {
		t.Fatal(err)
	}
	e.c.impl.mu.Lock()
	_, tracked := e.c.impl.firstAbsent[bob]
	e.c.impl.mu.Unlock()
	if !tracked {
		t.Fatal("a second holder that left the roster isn't tracked")
	}
}

// ---- create ----

func TestCreateTaskBoardFields(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, carol, "port:5173", time.Hour, false))
	other := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "other"}))
	got := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{
		Title: "port 5173 is taken", Shared: true,
		Links:       []string{"https://linear.app/acme/issue/ABC-123/x", " ", "https://linear.app/acme/issue/ABC-123/x", "docs/a.md"},
		BlockReason: " a dead dev server holds it ", BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: " port:5173 "},
	}))
	if !got.Shared || got.State != Blocked || got.Reason != "a dead dev server holds it" ||
		got.BlockedBy == nil || *got.BlockedBy != (BlockedBy{Kind: BlockedOnLease, Ref: "port:5173"}) {
		t.Fatalf("created = %+v", got)
	}
	if len(got.Links) != 2 || got.Links[0].Label != "Linear ABC-123" || got.Links[1].Kind != LinkPath {
		t.Fatalf("links = %+v", got.Links)
	}
	rows := e.rows("task.")
	if n := len(rows); n != 3 || rows[1].Kind != KindTaskCreated || rows[2].Kind != KindTaskState {
		t.Fatalf("rows = %d", n)
	}
	st := decodeRow[taskStateRow](t, rows[2].Payload)
	if st.State != Blocked || st.Reason == "" || st.BlockedBy == nil {
		t.Fatalf("status row = %+v", st)
	}

	long := strings.Repeat("x", MaxReason+1)
	tooMany := make([]string, MaxLinks+1)
	for i := range tooMany {
		tooMany[i] = "docs/" + string(rune('a'+i)) + ".md"
	}
	for _, tc := range []struct {
		name string
		nt   NewTask
		want error
	}{
		{"blockedBy without reason", NewTask{Title: "x", BlockedBy: &BlockedBy{Kind: BlockedOnPerson}}, ErrInvalid},
		{"reason too long", NewTask{Title: "x", BlockReason: long}, ErrInvalid},
		{"bad link", NewTask{Title: "x", Links: []string{"ftp://x/y"}}, ErrInvalid},
		{"too many links", NewTask{Title: "x", Links: tooMany}, ErrInvalid},
		{"blocked on a missing task", NewTask{Title: "x", BlockReason: "r", BlockedBy: &BlockedBy{Kind: BlockedOnTask, Ref: "nope"}}, ErrNotFound},
		{"bad lease ref", NewTask{Title: "x", BlockReason: "r", BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: "Not A Resource"}}, ErrInvalid},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := e.c.CreateTask(ctx, alice, tc.nt)
			wantErr(t, err, tc.want)
		})
	}
	// Blocked on an existing task, no BlockedBy at all.
	b := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "y", BlockReason: "after other", BlockedBy: &BlockedBy{Kind: BlockedOnTask, Ref: other.ID}}))
	if b.State != Blocked {
		t.Fatalf("b = %+v", b)
	}
	c := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "z", BlockReason: "just blocked"}))
	if c.State != Blocked || c.BlockedBy != nil {
		t.Fatalf("c = %+v", c)
	}
}

// ---- assign ----

func TestAssignPermissionsAndNotice(t *testing.T) {
	cases := []struct {
		name     string
		holder   string // claims first, "" = none
		by, to   string
		wantErr  error
		wantTo   []string
		wantRows int // task.assigned rows
	}{
		{"creator", "", alice, bob, nil, []string{bob}, 1},
		{"stranger, no holder (decision 21)", "", carol, bob, nil, []string{bob}, 1},
		{"stranger assigns itself", "", carol, carol, nil, nil, 1},
		{"holder hands off", bob, bob, carol, nil, []string{carol}, 1},
		{"person", bob, user, carol, nil, []string{carol}, 1},
		{"stranger while held", bob, carol, dave, ErrNotAllowed, nil, 0},
		{"to a person: no recipients", "", alice, user, nil, nil, 1},
		{"to the holder: no-op", bob, alice, bob, nil, nil, 0},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "fix it", Objective: "the objective", DoneCriteria: "tests pass"}))
			if tc.holder != "" {
				must[Task](t)(e.c.ClaimTask(ctx, tc.holder, task.ID))
			}
			got, n, err := e.c.Assign(ctx, tc.by, task.ID, tc.to)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if rows := len(e.rows(KindTaskAssigned)); rows != tc.wantRows {
				t.Fatalf("rows = %d, want %d", rows, tc.wantRows)
			}
			if !reflect.DeepEqual(n.To, tc.wantTo) {
				t.Fatalf("notice to = %v, want %v", n.To, tc.wantTo)
			}
			if tc.wantRows == 0 {
				if n.Kind != "" || n.To != nil {
					t.Fatalf("no-op returned a notice %+v", n)
				}
				return
			}
			if got.Holder != tc.to || n.Kind != NoticeAssigned || n.Ref != "task:"+task.ID || n.By != tc.by || n.Wake || n.Title != "fix it" {
				t.Fatalf("task %+v notice %+v", got, n)
			}
			for _, s := range []string{"fix it", "task:" + task.ID, "the objective", "tests pass", "show task:" + task.ID} {
				if !strings.Contains(n.Text, s) {
					t.Fatalf("notice text %q lacks %q", n.Text, s)
				}
			}
		})
	}
}

func TestAssignNoticeCutsLongFields(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t", Objective: strings.Repeat("o", 2000), DoneCriteria: strings.Repeat("d", 700)}))
	_, n := mustN(t)(e.c.Assign(ctx, alice, task.ID, bob))
	if !strings.Contains(n.Text, strings.Repeat("o", maxNoticeField-1)+"…") || strings.Contains(n.Text, strings.Repeat("o", maxNoticeField)) || !strings.Contains(n.Text, strings.Repeat("d", maxNoticeField-1)+"…") {
		t.Fatalf("notice not cut: %d runes", len([]rune(n.Text)))
	}
}

func TestAssignTaskKeepsV02Rule(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	_, err := e.c.AssignTask(ctx, carol, task.ID, carol)
	wantErr(t, err, ErrNotAllowed)
	_, _, err = e.c.Assign(ctx, carol, task.ID, carol)
	if err != nil {
		t.Fatal(err)
	}
}

// ---- join, leave, share ----

func TestJoinLeaveShare(t *testing.T) {
	e := newEnv(t)
	sh := shared(t, e, alice)
	un := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "u"}))
	done := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "d", Shared: true}))
	must[Task](t)(e.c.SetTaskState(ctx, alice, done.ID, Completed, ""))

	steps := []struct {
		name    string
		do      func() (Task, error)
		wantErr error
		holders []string
		state   TaskState
	}{
		{"join moves submitted to working", func() (Task, error) { return e.c.Join(ctx, bob, sh.ID, bob) }, nil, []string{bob}, Working},
		{"second joins", func() (Task, error) { return e.c.Join(ctx, carol, sh.ID, carol) }, nil, []string{bob, carol}, Working},
		{"join again: no-op", func() (Task, error) { return e.c.Join(ctx, carol, sh.ID, carol) }, nil, []string{bob, carol}, Working},
		{"agent adds another: refused", func() (Task, error) { return e.c.Join(ctx, bob, sh.ID, dave) }, ErrNotAllowed, nil, ""},
		{"person adds someone", func() (Task, error) { return e.c.Join(ctx, user, sh.ID, dave) }, nil, []string{bob, carol, dave}, Working},
		{"join unshared: refused", func() (Task, error) { return e.c.Join(ctx, bob, un.ID, bob) }, ErrNotAllowed, nil, ""},
		{"join done: refused", func() (Task, error) { return e.c.Join(ctx, bob, done.ID, bob) }, ErrBadState, nil, ""},
		{"empty who", func() (Task, error) { return e.c.Join(ctx, bob, sh.ID, " ") }, ErrInvalid, nil, ""},
		{"unshare with three holders", func() (Task, error) { return e.c.SetShared(ctx, alice, sh.ID, false) }, ErrInvalid, nil, ""},
		{"leave someone else: refused", func() (Task, error) { return e.c.Leave(ctx, bob, sh.ID, carol) }, ErrNotAllowed, nil, ""},
		{"leave", func() (Task, error) { return e.c.Leave(ctx, bob, sh.ID, bob) }, nil, []string{carol, dave}, Working},
		{"leave not holding", func() (Task, error) { return e.c.Leave(ctx, bob, sh.ID, bob) }, ErrNotFound, nil, ""},
		{"person removes one", func() (Task, error) { return e.c.Leave(ctx, user, sh.ID, dave) }, nil, []string{carol}, Working},
		{"stranger can't share", func() (Task, error) { return e.c.SetShared(ctx, dave, sh.ID, false) }, ErrNotAllowed, nil, ""},
		{"holder unshares", func() (Task, error) { return e.c.SetShared(ctx, carol, sh.ID, false) }, nil, []string{carol}, Working},
		{"last leaves: state kept, no holder", func() (Task, error) { return e.c.Leave(ctx, carol, sh.ID, carol) }, nil, nil, Working},
		{"creator shares again", func() (Task, error) { return e.c.SetShared(ctx, alice, sh.ID, true) }, nil, nil, Working},
		{"share again: no-op", func() (Task, error) { return e.c.SetShared(ctx, alice, sh.ID, true) }, nil, nil, Working},
	}
	for _, s := range steps {
		e.clk.add(time.Second)
		got, err := s.do()
		if s.wantErr != nil {
			if !errors.Is(err, s.wantErr) {
				t.Fatalf("%s: err = %v, want %v", s.name, err, s.wantErr)
			}
			continue
		}
		if err != nil {
			t.Fatalf("%s: %v", s.name, err)
		}
		if !reflect.DeepEqual(got.Holders, s.holders) || got.State != s.state {
			t.Fatalf("%s: holders %v state %s", s.name, got.Holders, got.State)
		}
		if len(s.holders) > 0 && got.Holder != s.holders[0] || len(s.holders) == 0 && got.Holder != "" {
			t.Fatalf("%s: holder %q", s.name, got.Holder)
		}
	}
	if n := len(e.rows(KindTaskShared)); n != 2 {
		t.Fatalf("shared rows = %d", n)
	}
	if n := len(e.rows(KindTaskJoined)); n != 3 {
		t.Fatalf("joined rows = %d", n)
	}
}

// ---- block, unblock ----

func TestBlockValidation(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	other := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "o"}))
	must[Lease](t)(e.c.Acquire(ctx, dave, pathRes("src/**"), time.Hour, false))
	cases := []struct {
		name    string
		by      string
		reason  string
		bb      *BlockedBy
		wantErr error
		wantBB  *BlockedBy
	}{
		{"no reason", alice, "  ", nil, ErrInvalid, nil},
		{"reason too long", alice, strings.Repeat("é", MaxReason+1), nil, ErrInvalid, nil},
		{"reason at the cap", alice, strings.Repeat("é", MaxReason), nil, nil, nil},
		{"stranger", carol, "r", nil, ErrNotAllowed, nil},
		{"lease normalized", alice, "r", &BlockedBy{Kind: BlockedOnLease, Ref: "path:" + repoA + ":./src/"}, nil, &BlockedBy{Kind: BlockedOnLease, Ref: pathRes("src/**")}},
		{"lease bad", alice, "r", &BlockedBy{Kind: BlockedOnLease, Ref: "path:x"}, ErrInvalid, nil},
		{"lease empty", alice, "r", &BlockedBy{Kind: BlockedOnLease}, ErrInvalid, nil},
		{"lease nobody holds", alice, "r", &BlockedBy{Kind: BlockedOnLease, Ref: "port:9"}, ErrInvalid, nil},
		{"task", alice, "r", &BlockedBy{Kind: BlockedOnTask, Ref: other.ID}, nil, &BlockedBy{Kind: BlockedOnTask, Ref: other.ID}},
		{"task self", alice, "r", &BlockedBy{Kind: BlockedOnTask, Ref: task.ID}, ErrInvalid, nil},
		{"task missing", alice, "r", &BlockedBy{Kind: BlockedOnTask, Ref: "nope"}, ErrNotFound, nil},
		{"person anyone", user, "r", &BlockedBy{Kind: BlockedOnPerson}, nil, &BlockedBy{Kind: BlockedOnPerson}},
		{"person key", alice, "r", &BlockedBy{Kind: BlockedOnPerson, Ref: user}, nil, &BlockedBy{Kind: BlockedOnPerson, Ref: user}},
		{"person not a user key", alice, "r", &BlockedBy{Kind: BlockedOnPerson, Ref: bob}, ErrInvalid, nil},
		{"agent", alice, "r", &BlockedBy{Kind: BlockedOnAgent, Ref: bob}, nil, &BlockedBy{Kind: BlockedOnAgent, Ref: bob}},
		{"agent empty", alice, "r", &BlockedBy{Kind: BlockedOnAgent}, ErrInvalid, nil},
		{"external free text", alice, "r", &BlockedBy{Kind: BlockedOnExternal, Ref: "CI on #57"}, nil, &BlockedBy{Kind: BlockedOnExternal, Ref: "CI on #57"}},
		{"ref too long", alice, "r", &BlockedBy{Kind: BlockedOnExternal, Ref: strings.Repeat("x", MaxRef+1)}, ErrInvalid, nil},
		{"unknown kind", alice, "r", &BlockedBy{Kind: "weather"}, ErrInvalid, nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, err := e.c.Block(ctx, tc.by, task.ID, tc.reason, tc.bb)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if got.State != Blocked || got.Reason != strings.TrimSpace(tc.reason) || !sameBlocker(got.BlockedBy, tc.wantBB) {
				t.Fatalf("got %+v blockedBy %+v", got, got.BlockedBy)
			}
		})
	}
}

func TestBlockReplaceNoopAndLeave(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Working, "keep me"))
	_, err := e.c.SetTaskState(ctx, alice, task.ID, Blocked, "")
	wantErr(t, err, ErrBadState)

	got := must[Task](t)(e.c.Block(ctx, alice, task.ID, "one", nil))
	if got.Note != "keep me" {
		t.Fatalf("block dropped the note: %+v", got)
	}
	must[Task](t)(e.c.Block(ctx, alice, task.ID, "one", nil)) // no-op
	got = must[Task](t)(e.c.Block(ctx, alice, task.ID, "two", &BlockedBy{Kind: BlockedOnPerson}))
	if got.Reason != "two" || got.BlockedBy == nil {
		t.Fatalf("replace = %+v", got)
	}
	if n := len(e.rows(KindTaskState)); n != 3 {
		t.Fatalf("status rows = %d", n)
	}
	// Leaving Blocked through SetTaskState clears the blocker.
	got = must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Working, "answered"))
	if got.State != Working || got.Reason != "" || got.BlockedBy != nil {
		t.Fatalf("left blocked = %+v", got)
	}
	must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Completed, ""))
	_, err = e.c.Block(ctx, alice, task.ID, "late", nil)
	wantErr(t, err, ErrBadState)
}

func TestUnblock(t *testing.T) {
	cases := []struct {
		name      string
		shared    bool
		holders   []string
		by        string
		note      string
		wantState TaskState
		wantTo    []string
		wantErr   error
	}{
		{"no holders: submitted, creator hears", false, nil, user, "freed it", Submitted, []string{alice}, nil},
		{"no holders, creator unblocks: nobody", false, nil, alice, "", Submitted, nil, nil},
		{"holder: working, holder hears", false, []string{bob}, carol, "", Working, []string{bob}, nil},
		{"anyone, even a stranger", false, []string{bob}, dave, "", Working, []string{bob}, nil},
		{"shared: other holders hear", true, []string{bob, carol, user}, bob, "", Working, []string{carol}, nil},
		{"note too long", false, nil, user, strings.Repeat("n", MaxReason+1), "", nil, ErrInvalid},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t", Shared: tc.shared}))
			for _, h := range tc.holders {
				must[Task](t)(e.c.ClaimTask(ctx, h, task.ID))
			}
			must[Task](t)(e.c.Block(ctx, user, task.ID, "waiting on the db", &BlockedBy{Kind: BlockedOnExternal, Ref: "db"}))
			got, n, err := e.c.Unblock(ctx, tc.by, task.ID, tc.note)
			if tc.wantErr != nil {
				wantErr(t, err, tc.wantErr)
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if got.State != tc.wantState || got.Reason != "" || got.BlockedBy != nil {
				t.Fatalf("got %+v", got)
			}
			if !reflect.DeepEqual(n.To, tc.wantTo) || n.Kind != NoticeUnblocked || n.By != tc.by || n.Ref != "task:"+task.ID {
				t.Fatalf("notice %+v", n)
			}
			if !strings.Contains(n.Text, "waiting on the db") || !strings.Contains(n.Text, string(tc.wantState)) {
				t.Fatalf("text %q", n.Text)
			}
			rows := e.rows(KindBlockerClosed)
			if len(rows) != 1 {
				t.Fatalf("rows = %d", len(rows))
			}
			p := decodeRow[blockerClosedRow](t, rows[0].Payload)
			if p.Restored != tc.wantState || p.By != tc.by || p.Note != tc.note {
				t.Fatalf("row %+v", p)
			}
			_, _, err = e.c.Unblock(ctx, tc.by, task.ID, "")
			wantErr(t, err, ErrBadState) // not blocked any more
		})
	}
}

// ---- comments ----

func TestComments(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))

	for _, tc := range []struct {
		name string
		text string
		want error
	}{
		{"empty", " \n ", ErrInvalid},
		{"too long", strings.Repeat("c", MaxComment+1), ErrInvalid},
	} {
		_, _, err := e.c.AddComment(ctx, user, task.ID, tc.text, false)
		if !errors.Is(err, tc.want) {
			t.Fatalf("%s: %v", tc.name, err)
		}
	}
	_, _, err := e.c.AddComment(ctx, user, "nope", "hi", false)
	wantErr(t, err, ErrNotFound)

	c1, n1, err := e.c.AddComment(ctx, user, task.ID, " use postgres ", false)
	if err != nil {
		t.Fatal(err)
	}
	if c1.Text != "use postgres" || c1.By != user || c1.TaskID != task.ID || c1.Wake {
		t.Fatalf("comment %+v", c1)
	}
	if n1.Kind != NoticeComment || n1.Ref != "comment:"+c1.ID || !reflect.DeepEqual(n1.To, []string{bob}) || n1.Wake ||
		!strings.Contains(n1.Text, "use postgres") || !strings.Contains(n1.Text, "show task:"+task.ID) {
		t.Fatalf("notice %+v", n1)
	}
	e.clk.add(time.Second)
	_, n2, _ := e.c.AddComment(ctx, bob, task.ID, "ok", true)
	if n2.To != nil || !n2.Wake {
		t.Fatalf("the holder's own comment tells nobody, keeps wake: %+v", n2)
	}
	e.clk.add(time.Second)
	_, n3, _ := e.c.AddComment(ctx, alice, task.ID, strings.Repeat("é", MaxComment), true)
	if !n3.Wake || !reflect.DeepEqual(n3.To, []string{bob}) {
		t.Fatalf("n3 %+v", n3)
	}
	// Done tasks take comments too.
	must[Task](t)(e.c.SetTaskState(ctx, bob, task.ID, Completed, ""))
	e.clk.add(time.Second)
	c4, _, err := e.c.AddComment(ctx, user, task.ID, "thanks", false)
	if err != nil {
		t.Fatal(err)
	}
	cs := e.c.Comments(task.ID)
	if len(cs) != 4 || cs[0].ID != c1.ID || cs[3].ID != c4.ID {
		t.Fatalf("comments = %+v", cs)
	}
	got, _ := e.c.GetTask(task.ID)
	if got.Comments != 4 || !got.UpdatedAt.Equal(c4.At) {
		t.Fatalf("task = %+v", got)
	}
	if g, ok := e.c.GetComment(c1.ID); !ok || !reflect.DeepEqual(g, c1) {
		t.Fatalf("GetComment = %+v %v", g, ok)
	}
	if _, ok := e.c.GetComment("nope"); ok {
		t.Fatal("GetComment(nope)")
	}
	if e.c.Comments("nope") != nil {
		t.Fatal("Comments(nope) != nil")
	}
	if cs := e.c.Comments(must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "empty"})).ID); cs == nil || len(cs) != 0 {
		t.Fatalf("no comments = %#v", cs)
	}
}

// A message about a card goes on its thread as a comment keyed by the
// message id: once, with no notice and no activity line of its own.
func TestRecordMessage(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	acts := len(e.c.Activity(task.ID))
	rows := func() int { return len(e.rows(KindTaskComment)) }
	m := MessageEntry{TaskID: task.ID, MsgID: "msg_1", From: alice, To: bob, Text: " can you look? ", Files: 2}
	cm, err := e.c.RecordMessage(ctx, m)
	if err != nil {
		t.Fatal(err)
	}
	if cm.ID != "msg_1" || cm.MsgID != "msg_1" || cm.By != alice || cm.To != bob || cm.Text != "can you look?" ||
		cm.Device != "dev-a" || cm.HLC == 0 || cm.Seq == 0 || cm.Files != 2 || cm.Attachments != nil || cm.Gone || cm.Held || cm.Wake {
		t.Fatalf("recorded %+v", cm)
	}
	if ev := e.rows(KindTaskComment)[0]; ev.Actor != "agent:"+alice {
		t.Fatalf("actor %q", ev.Actor)
	}
	if n := len(e.c.Activity(task.ID)); n != acts {
		t.Fatalf("activity %d lines, want %d", n, acts)
	}
	// Again (a retry, the sweep): nothing written.
	if again, err := e.c.RecordMessage(ctx, m); err != nil || !reflect.DeepEqual(again, cm) || rows() != 1 {
		t.Fatalf("again: %+v %v, %d rows", again, err, rows())
	}
	// Gone: one row, as Irori, merged into the entry.
	g := m
	g.Gone, g.Writer, g.Text = true, "irori", "other text"
	for range 2 {
		got, err := e.c.RecordMessage(ctx, g)
		if err != nil || !got.Gone || got.Text != "can you look?" || got.HLC != cm.HLC || rows() != 2 {
			t.Fatalf("gone: %+v %v, %d rows", got, err, rows())
		}
	}
	if ev := e.rows(KindTaskComment)[1]; ev.Actor != "irori" {
		t.Fatalf("gone row actor %q", ev.Actor)
	}
	if cs := e.c.Comments(task.ID); len(cs) != 1 || !cs[0].Gone {
		t.Fatalf("comments %+v", cs)
	}
	// Long text is cut and points at the message.
	long, err := e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_2", From: alice, To: bob, Text: strings.Repeat("x", MaxComment+50)})
	if err != nil {
		t.Fatal(err)
	}
	if utf8.RuneCountInString(long.Text) != MaxComment || !strings.HasSuffix(long.Text, "… (full: msg:msg_2)") {
		t.Fatalf("cut to %d runes: …%s", utf8.RuneCountInString(long.Text), long.Text[len(long.Text)-40:])
	}
	// Held: no text.
	held, err := e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_3", From: alice, To: bob, Text: "secret", Held: true})
	if err != nil || held.Text != "" || !held.Held {
		t.Fatalf("held %+v %v", held, err)
	}
	// Let through later: a fill row (as Irori) brings the text, once.
	n := rows()
	f := MessageEntry{TaskID: task.ID, MsgID: "msg_3", From: alice, To: bob, Text: "secret", Fill: true, Writer: "irori"}
	for range 2 {
		got, err := e.c.RecordMessage(ctx, f)
		if err != nil || got.Text != "secret" || got.Held || got.HLC != held.HLC || rows() != n+1 {
			t.Fatalf("fill: %+v %v, %d rows", got, err, rows())
		}
	}
	if ev := e.rows(KindTaskComment)[n]; ev.Actor != "irori" {
		t.Fatalf("fill row actor %q", ev.Actor)
	}
	// A message already on the card with its text takes no fill.
	if _, err := e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_1", From: alice, To: bob, Text: "x", Fill: true, Writer: "irori"}); err != nil || rows() != n+1 {
		t.Fatalf("fill of a shown message: %v, %d rows", err, rows())
	}
	// Done cards take them; unknown ones don't.
	must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Completed, ""))
	if _, err := e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_4", From: bob, To: alice, Text: "late"}); err != nil {
		t.Fatal(err)
	}
	if got, _ := e.c.GetTask(task.ID); got.Comments != 4 {
		t.Fatalf("task %+v", got)
	}
	_, err = e.c.RecordMessage(ctx, MessageEntry{TaskID: "nope", MsgID: "msg_5", From: alice, To: bob, Text: "x"})
	wantErr(t, err, ErrNotFound)
	_, err = e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "01ABC", From: alice, To: bob, Text: "x"})
	wantErr(t, err, ErrInvalid)
	// Another device's sender is recorded on its own device; its gone row
	// may come from the recipient's (this one).
	_, err = e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_6", From: "dev-b/zed", To: alice, Text: "x"})
	wantErr(t, err, ErrNotAllowed)
	if _, err := e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_6", From: "dev-b/zed", To: alice, Text: "x", Gone: true, Writer: "irori"}); err != nil {
		t.Fatal(err)
	}
	// So may its fill row; not when neither party is on this device.
	if _, err := e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_7", From: "dev-b/zed", To: alice, Text: "x", Fill: true, Writer: "irori"}); err != nil {
		t.Fatal(err)
	}
	_, err = e.c.RecordMessage(ctx, MessageEntry{TaskID: task.ID, MsgID: "msg_8", From: "dev-b/zed", To: "dev-c/yan", Text: "x", Fill: true, Writer: "irori"})
	wantErr(t, err, ErrNotAllowed)
}

func TestCommentNoHoldersTellsCreator(t *testing.T) {
	for _, tc := range []struct {
		creator, by string
		want        []string
	}{
		{alice, user, []string{alice}},
		{alice, alice, nil},
		{user, bob, nil},
	} {
		e := newEnv(t)
		task := must[Task](t)(e.c.CreateTask(ctx, tc.creator, NewTask{Title: "t"}))
		_, n, err := e.c.AddComment(ctx, tc.by, task.ID, "x", false)
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(n.To, tc.want) {
			t.Fatalf("creator %s by %s: to %v, want %v", tc.creator, tc.by, n.To, tc.want)
		}
	}
}

// ---- links ----

func TestLinks(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	got := must[Task](t)(e.c.AddLink(ctx, carol, task.ID, "https://github.com/a/b/pull/57"))
	if len(got.Links) != 1 || got.Links[0].Label != "GitHub #57" {
		t.Fatalf("links %+v", got.Links)
	}
	must[Task](t)(e.c.AddLink(ctx, carol, task.ID, "https://github.com/a/b/pull/57")) // no-op
	if n := len(e.rows(KindTaskLinked)); n != 1 {
		t.Fatalf("rows = %d", n)
	}
	_, err := e.c.AddLink(ctx, carol, task.ID, "javascript:alert(1)://x")
	wantErr(t, err, ErrInvalid)
	_, err = e.c.AddLink(ctx, carol, "nope", "docs/a.md")
	wantErr(t, err, ErrNotFound)
	for i := 1; i < MaxLinks; i++ {
		must[Task](t)(e.c.AddLink(ctx, carol, task.ID, "docs/"+string(rune('a'+i))+".md"))
	}
	_, err = e.c.AddLink(ctx, carol, task.ID, "docs/full.md")
	wantErr(t, err, ErrInvalid)

	_, err = e.c.RemoveLink(ctx, carol, task.ID, "docs/b.md") // carol isn't creator/holder/person
	wantErr(t, err, ErrNotAllowed)
	_, err = e.c.RemoveLink(ctx, alice, task.ID, "docs/zz.md")
	wantErr(t, err, ErrNotFound)
	got = must[Task](t)(e.c.RemoveLink(ctx, alice, task.ID, `docs\b.md`)) // as typed on Windows
	if len(got.Links) != MaxLinks-1 || linkIndex(got.Links, "docs/b.md") >= 0 {
		t.Fatalf("after remove: %+v", got.Links)
	}
	// Links work on done tasks too.
	must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Completed, ""))
	got = must[Task](t)(e.c.AddLink(ctx, user, task.ID, "0123456789abcdef0123"))
	if got.Links[len(got.Links)-1].Label != "commit 0123456" {
		t.Fatalf("commit link %+v", got.Links)
	}
}

// ---- refusals ----

func TestRefusals(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, alice, pathRes("src/**"), time.Hour, false))
	e.clk.add(time.Second)
	_, err := e.c.Acquire(ctx, bob, pathRes("src/a.go"), 0, false)
	wantErr(t, err, ErrHeld)
	got := e.c.Refusals()
	want := Refusal{Resource: pathRes("src/a.go"), By: bob, HeldBy: alice, Held: pathRes("src/**"), Until: fixedNow.Add(time.Hour), At: fixedNow.Add(time.Second)}
	if len(got) != 1 || got[0] != want {
		t.Fatalf("refusals = %+v", got)
	}
	// Asking again while active writes nothing; a renewal keeps it active.
	must[Lease](t)(e.c.Acquire(ctx, alice, pathRes("src/**"), 2*time.Hour, false))
	_, _ = e.c.Acquire(ctx, bob, pathRes("src/a.go"), 0, false)
	if n := len(e.rows(KindLeaseRefused)); n != 1 {
		t.Fatalf("rows = %d", n)
	}
	// Another caller, another resource: rows of their own.
	e.clk.add(time.Second)
	_, _ = e.c.Acquire(ctx, carol, pathRes("src/a.go"), 0, false)
	e.clk.add(time.Second)
	_, _ = e.c.Acquire(ctx, bob, pathRes("src/b.go"), 0, false)
	if got := e.c.Refusals(); len(got) != 3 || got[0].Resource != pathRes("src/b.go") || got[1].By != carol {
		t.Fatalf("newest first = %+v", got)
	}
	// Released: none active; asking again after a re-acquire is a new row.
	if err := e.c.Release(ctx, alice, pathRes("src/**")); err != nil {
		t.Fatal(err)
	}
	if got := e.c.Refusals(); len(got) != 0 {
		t.Fatalf("after release = %+v", got)
	}
	e.clk.add(time.Second)
	must[Lease](t)(e.c.Acquire(ctx, alice, pathRes("src/**"), time.Hour, false))
	if got := e.c.Refusals(); len(got) != 0 {
		t.Fatalf("a new lease revived old refusals: %+v", got)
	}
	_, _ = e.c.Acquire(ctx, bob, pathRes("src/a.go"), 0, false)
	if n := len(e.rows(KindLeaseRefused)); n != 4 {
		t.Fatalf("rows = %d", n)
	}
	// A steal by the refused caller ends it.
	must[Lease](t)(e.c.Acquire(ctx, bob, pathRes("src/a.go"), 0, true))
	if got := e.c.Refusals(); len(got) != 0 {
		t.Fatalf("after steal = %+v", got)
	}
	// Expiry ends it.
	must[Lease](t)(e.c.Acquire(ctx, carol, "gpu/qwen", time.Minute, false))
	_, _ = e.c.Acquire(ctx, dave, "gpu/qwen", 0, false)
	if len(e.c.Refusals()) != 1 {
		t.Fatal("no gpu refusal")
	}
	e.clk.add(2 * time.Minute)
	if got := e.c.Refusals(); len(got) != 0 {
		t.Fatalf("after expiry = %+v", got)
	}
}

func TestRefusalOneRowPerHolder(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, alice, pathRes("src/a.go"), 0, false))
	must[Lease](t)(e.c.Acquire(ctx, alice, pathRes("src/b.go"), 0, false))
	must[Lease](t)(e.c.Acquire(ctx, carol, pathRes("src/c.go"), 0, false))
	_, err := e.c.Acquire(ctx, bob, pathRes("src/**"), 0, false)
	wantErr(t, err, ErrHeld)
	rows := e.rows(KindLeaseRefused)
	if len(rows) != 2 {
		t.Fatalf("rows = %d, want one per holder", len(rows))
	}
	if r := decodeRow[Refusal](t, rows[0].Payload); r.HeldBy != alice || r.Held != pathRes("src/a.go") {
		t.Fatalf("first = %+v", r)
	}
}

// ---- board ----

func TestBoardGroups(t *testing.T) {
	e := newEnv(t)
	// dev-b is another of the person's devices; "user:dev-z" is made up.
	e.owner = func(k string) bool { return k == user || k == "user:dev-b" }
	e.c = e.reopen()
	mk := func(title string, nt NewTask) Task {
		t.Helper()
		e.clk.add(time.Second)
		nt.Title = title
		return must[Task](t)(e.c.CreateTask(ctx, alice, nt))
	}
	open := mk("open", NewTask{})
	working := mk("working", NewTask{})
	must[Task](t)(e.c.ClaimTask(ctx, bob, working.ID))
	assigned := mk("assigned to bob", NewTask{})
	must[Task](t)(e.c.AssignTask(ctx, alice, assigned.ID, bob))
	mine := mk("assigned to user", NewTask{})
	must[Task](t)(e.c.AssignTask(ctx, alice, mine.ID, user))
	asks := mk("asks", NewTask{})
	must[Task](t)(e.c.ClaimTask(ctx, carol, asks.ID))
	must[Task](t)(e.c.SetTaskState(ctx, carol, asks.ID, InputRequired, "?"))
	onAnyone := mk("on anyone", NewTask{BlockReason: "r", BlockedBy: &BlockedBy{Kind: BlockedOnPerson}})
	onOther := mk("on the person at another device", NewTask{BlockReason: "r", BlockedBy: &BlockedBy{Kind: BlockedOnPerson, Ref: "user:dev-b"}})
	if _, err := e.c.CreateTask(ctx, alice, NewTask{Title: "made up", BlockReason: "r", BlockedBy: &BlockedBy{Kind: BlockedOnPerson, Ref: "user:dev-z"}}); !errors.Is(err, ErrInvalid) {
		t.Fatalf("a made-up user: key as a person blocker: err = %v", err)
	}
	onBob := mk("on bob", NewTask{BlockReason: "r", BlockedBy: &BlockedBy{Kind: BlockedOnAgent, Ref: bob}})
	ext := mk("external", NewTask{BlockReason: "ci"})
	var dones []Task
	for i := 0; i < 3; i++ {
		d := mk("done", NewTask{})
		must[Task](t)(e.c.SetTaskState(ctx, alice, d.ID, []TaskState{Completed, Failed, Canceled}[i], ""))
		dones = append(dones, d)
	}
	ids := func(cs []Card) []string {
		out := []string{}
		for _, c := range cs {
			out = append(out, c.ID)
		}
		return out
	}
	cases := []struct {
		name                                     string
		f                                        BoardFilter
		needs, blocked, working, openG, doneWant []string
		total                                    int
	}{
		{"the person", BoardFilter{For: user},
			[]string{onOther.ID, onAnyone.ID, asks.ID, mine.ID}, []string{ext.ID, onBob.ID},
			[]string{assigned.ID, working.ID}, []string{open.ID},
			[]string{dones[2].ID, dones[1].ID, dones[0].ID}, 3},
		{"whoever is at the app", BoardFilter{DoneLimit: 2},
			[]string{onOther.ID, onAnyone.ID, asks.ID}, []string{ext.ID, onBob.ID},
			[]string{mine.ID, assigned.ID, working.ID}, []string{open.ID},
			[]string{dones[2].ID, dones[1].ID}, 3},
		{"agent bob", BoardFilter{For: bob, DoneLimit: 1},
			[]string{onBob.ID, assigned.ID}, []string{ext.ID, onOther.ID, onAnyone.ID},
			[]string{asks.ID, mine.ID, working.ID}, []string{open.ID},
			[]string{dones[2].ID}, 3},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			v := e.c.Board(tc.f)
			for _, g := range []struct {
				name      string
				got, want []string
			}{
				{"needsYou", ids(v.NeedsYou), tc.needs},
				{"blocked", ids(v.Blocked), tc.blocked},
				{"working", ids(v.Working), tc.working},
				{"open", ids(v.Open), tc.openG},
				{"done", ids(v.Done), tc.doneWant},
			} {
				if !reflect.DeepEqual(g.got, g.want) {
					t.Errorf("%s = %v, want %v", g.name, g.got, g.want)
				}
			}
			if v.DoneTotal != tc.total {
				t.Errorf("doneTotal = %d", v.DoneTotal)
			}
		})
	}
	if v := e.c.Board(BoardFilter{RepoKey: repoA}); len(v.Open)+len(v.Working)+len(v.Done)+len(v.NeedsYou)+len(v.Blocked) != 0 || v.Leases == nil {
		t.Fatalf("repo filter = %+v", v)
	}
}

func TestBoardDoneCap(t *testing.T) {
	e := newEnv(t)
	for i := 0; i < DefaultDoneCard+5; i++ {
		e.clk.add(time.Second)
		d := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "d"}))
		must[Task](t)(e.c.SetTaskState(ctx, alice, d.ID, Completed, ""))
	}
	for _, tc := range []struct{ limit, want int }{{0, DefaultDoneCard}, {-1, DefaultDoneCard}, {3, 3}, {MaxDoneCard + 1, DefaultDoneCard + 5}} {
		v := e.c.Board(BoardFilter{DoneLimit: tc.limit})
		if len(v.Done) != tc.want || v.DoneTotal != DefaultDoneCard+5 {
			t.Fatalf("limit %d: %d cards, total %d", tc.limit, len(v.Done), v.DoneTotal)
		}
	}
}

func TestBoardCardsAndLeases(t *testing.T) {
	e := newEnv(t)
	sh := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "sh", Shared: true, RepoKey: repoA}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, sh.ID))
	must[Task](t)(e.c.ClaimTask(ctx, bob, sh.ID))
	must[Lease](t)(e.c.Acquire(ctx, alice, pathRes("src/**"), time.Hour, false))
	must[Lease](t)(e.c.Acquire(ctx, bob, pathRes("src/a.go"), 2*time.Hour, true)) // takes alice's src/**
	must[Lease](t)(e.c.Acquire(ctx, alice, pathRes("docs/**"), time.Hour, false))
	// Acquire never lets two holders overlap on one device; rows meeting
	// from two devices can. Plant bob's overlapping lease in the fold.
	e.c.impl.mu.Lock()
	res, _ := ParseResource(pathRes("docs/x.md"))
	e.c.impl.held[res.Name] = &leaseRec{Lease: Lease{Resource: res.Name, Holder: bob, Until: fixedNow.Add(time.Hour), AcquiredAt: fixedNow}, res: res}
	e.c.impl.mu.Unlock()

	e.clk.add(time.Second)
	blocked := must[Task](t)(e.c.CreateTask(ctx, carol, NewTask{Title: "needs src", RepoKey: repoA, BlockReason: "bob has it",
		BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: pathRes("src/a.go")}}))
	must[Lease](t)(e.c.Acquire(ctx, dave, "gpu/qwen", time.Hour, false))
	gone := must[Task](t)(e.c.CreateTask(ctx, carol, NewTask{Title: "needs gpu", BlockReason: "dave has it",
		BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: "gpu/qwen"}}))
	if err := e.c.Release(ctx, dave, "gpu/qwen"); err != nil {
		t.Fatal(err)
	}
	must[Lease](t)(e.c.Acquire(ctx, dave, "port:8080", 30*time.Minute, false))
	_, _ = e.c.Acquire(ctx, carol, "port:8080", 0, false)
	e.clk.add(time.Second)
	c1, _, _ := e.c.AddComment(ctx, user, sh.ID, "first", false)
	e.clk.add(time.Second)
	c2, _, _ := e.c.AddComment(ctx, user, sh.ID, "second", false)
	_ = c1

	v := e.c.Board(BoardFilter{For: user})
	var shCard, blockedCard, goneCard *Card
	for _, g := range [][]Card{v.Working, v.Blocked, v.NeedsYou, v.Open} {
		for i := range g {
			switch g[i].ID {
			case sh.ID:
				shCard = &g[i]
			case blocked.ID:
				blockedCard = &g[i]
			case gone.ID:
				goneCard = &g[i]
			}
		}
	}
	if shCard == nil || blockedCard == nil || goneCard == nil {
		t.Fatalf("cards missing: %+v", v)
	}
	if shCard.LastComment == nil || shCard.LastComment.ID != c2.ID {
		t.Fatalf("last comment = %+v", shCard.LastComment)
	}
	if !reflect.DeepEqual(shCard.Overlaps, []string{pathRes("docs/**") + " ~ " + pathRes("docs/x.md")}) {
		t.Fatalf("overlaps = %v", shCard.Overlaps)
	}
	if blockedCard.Resolved || !goneCard.Resolved {
		t.Fatalf("resolved: blocked %v gone %v", blockedCard.Resolved, goneCard.Resolved)
	}
	if len(v.Leases) != 2 {
		t.Fatalf("lease cards = %+v", v.Leases)
	}
	// Soonest expiry first: the port (30m), then bob's src/a.go (2h).
	if v.Leases[0].Resource != "port:8080" || len(v.Leases[0].Refused) != 1 || v.Leases[0].Refused[0].By != carol || v.Leases[0].Blocks != nil {
		t.Fatalf("port card = %+v", v.Leases[0])
	}
	if v.Leases[1].Resource != pathRes("src/a.go") || !reflect.DeepEqual(v.Leases[1].Blocks, []string{blocked.ID}) {
		t.Fatalf("src card = %+v", v.Leases[1])
	}
	// Filtered to another repo: the port refusal stays (no repo), the src card goes.
	v = e.c.Board(BoardFilter{RepoKey: repoB})
	if len(v.Leases) != 1 || v.Leases[0].Resource != "port:8080" {
		t.Fatalf("repoB leases = %+v", v.Leases)
	}
	// Released: the card goes.
	if err := e.c.Release(ctx, user, "port:8080"); err != nil {
		t.Fatal(err)
	}
	if v := e.c.Board(BoardFilter{}); len(v.Leases) != 1 {
		t.Fatalf("after release = %+v", v.Leases)
	}
}

// ---- sweep ----

func TestSweepNoticesClosesResolvedBlockers(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, bob, "gpu/qwen", time.Hour, false))
	onLease := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "needs gpu", BlockReason: "bob has it", BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: "gpu/qwen"}}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, onLease.ID))
	first := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "first"}))
	onTask := must[Task](t)(e.c.CreateTask(ctx, carol, NewTask{Title: "after first", BlockReason: "wait", BlockedBy: &BlockedBy{Kind: BlockedOnTask, Ref: first.ID}}))
	other := must[Task](t)(e.c.CreateTask(ctx, carol, NewTask{Title: "person", BlockReason: "ask", BlockedBy: &BlockedBy{Kind: BlockedOnPerson}}))

	ns, err := e.c.SweepNotices(ctx)
	if err != nil || len(ns) != 0 {
		t.Fatalf("nothing resolved yet: %+v %v", ns, err)
	}
	if err := e.c.Release(ctx, bob, "gpu/qwen"); err != nil {
		t.Fatal(err)
	}
	must[Task](t)(e.c.SetTaskState(ctx, user, first.ID, Completed, ""))
	ns, err = e.c.SweepNotices(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(ns) != 2 {
		t.Fatalf("notices = %+v", ns)
	}
	byTask := map[string]Notice{}
	for _, n := range ns {
		byTask[n.TaskID] = n
		if n.Kind != NoticeUnblocked || n.By != "irori" || n.Wake {
			t.Fatalf("notice %+v", n)
		}
	}
	if n := byTask[onLease.ID]; !reflect.DeepEqual(n.To, []string{alice}) || !strings.Contains(n.Text, "lease released") {
		t.Fatalf("lease notice %+v", n)
	}
	if n := byTask[onTask.ID]; !reflect.DeepEqual(n.To, []string{carol}) || !strings.Contains(n.Text, "task completed") {
		t.Fatalf("task notice %+v", n)
	}
	if got, _ := e.c.GetTask(onLease.ID); got.State != Working || got.Note != "lease released" {
		t.Fatalf("onLease = %+v", got)
	}
	if got, _ := e.c.GetTask(onTask.ID); got.State != Submitted {
		t.Fatalf("onTask = %+v", got)
	}
	if got, _ := e.c.GetTask(other.ID); got.State != Blocked {
		t.Fatalf("person blocker closed by itself: %+v", got)
	}
	for _, ev := range e.rows(KindBlockerClosed) {
		if ev.Actor != "irori" {
			t.Fatalf("actor = %s", ev.Actor)
		}
	}
	if ns, _ := e.c.SweepNotices(ctx); len(ns) != 0 {
		t.Fatalf("second sweep = %+v", ns)
	}
}

// An expired lease is not held either.
func TestSweepNoticesExpiredLease(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, bob, "port:3000", time.Minute, false))
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t", BlockReason: "r", BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: "port:3000"}}))
	e.clk.add(2 * time.Minute)
	if err := e.c.Sweep(ctx); err != nil { // Sweep closes it too, notices dropped
		t.Fatal(err)
	}
	if got, _ := e.c.GetTask(task.ID); got.State != Submitted {
		t.Fatalf("got %+v", got)
	}
}

// ---- search ----

func TestSearchDocs(t *testing.T) {
	e := newEnv(t)
	t1 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "one", Objective: "obj", DoneCriteria: "dc", RepoKey: repoA}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, t1.ID))
	e.clk.add(time.Second)
	t2 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "two", BlockReason: "stuck"}))
	e.clk.add(time.Second)
	c1, _, _ := e.c.AddComment(ctx, user, t1.ID, "from a person", false)
	e.clk.add(time.Second)
	c2, _, _ := e.c.AddComment(ctx, carol, t2.ID, "from an agent", false)

	docs, err := e.c.SearchDocs(ctx, time.Time{}, 0)
	if err != nil {
		t.Fatal(err)
	}
	refs := []string{}
	for _, d := range docs {
		refs = append(refs, d.Ref)
	}
	// t1's comment bumped its Updated, so t2 comes first.
	if len(docs) != 4 {
		t.Fatalf("docs = %v", refs)
	}
	byRef := map[string]search.Doc{}
	for _, d := range docs {
		byRef[d.Ref] = d
	}
	if d := byRef["task:"+t1.ID]; d.Kind != search.KindTask || d.Title != "one" || d.Body != "#1\nobj\ndc" || d.Agent != bob || d.RepoKey != repoA || !d.At.Equal(t1.CreatedAt) || !d.Updated.Equal(c1.At) {
		t.Fatalf("t1 doc %+v", d)
	}
	if d := byRef["task:"+t2.ID]; d.Body != "#2\nstuck" || d.Agent != "" {
		t.Fatalf("t2 doc %+v", d)
	}
	if d := byRef["comment:"+c1.ID]; d.Kind != search.KindComment || d.Title != "comment on one" || d.Body != "from a person" || d.Agent != "" || d.RepoKey != repoA {
		t.Fatalf("c1 doc %+v", d)
	}
	if d := byRef["comment:"+c2.ID]; d.Agent != carol {
		t.Fatalf("c2 doc %+v", d)
	}
	for i := 1; i < len(docs); i++ {
		if docs[i].Updated.Before(docs[i-1].Updated) {
			t.Fatalf("not oldest first: %v", refs)
		}
	}
	// since is inclusive; limit caps.
	docs, _ = e.c.SearchDocs(ctx, c2.At, 0)
	if len(docs) != 2 {
		t.Fatalf("since c2: %d docs", len(docs))
	}
	docs, _ = e.c.SearchDocs(ctx, time.Time{}, 2)
	if len(docs) != 2 || docs[0].Ref != "comment:"+c1.ID {
		t.Fatalf("limit 2: %+v", docs)
	}
}

func TestSearchShow(t *testing.T) {
	e := newEnv(t)
	dep := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "dep"}))
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{
		Title: "the card", Objective: "do the thing", DoneCriteria: "it works", RepoKey: repoA, Shared: true,
		Links: []string{"https://linear.app/acme/issue/ABC-9/x"},
	}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	must[Task](t)(e.c.Block(ctx, bob, task.ID, "after dep", &BlockedBy{Kind: BlockedOnTask, Ref: dep.ID}))
	var cids []string
	for i := 0; i < 13; i++ {
		e.clk.add(time.Second)
		text := "comment " + string(rune('a'+i))
		if i == 12 {
			text = strings.Repeat("long ", 200)
		}
		cm, _, err := e.c.AddComment(ctx, user, task.ID, text, false)
		if err != nil {
			t.Fatal(err)
		}
		cids = append(cids, cm.ID)
	}
	sh, err := e.c.SearchShow(ctx, " TASK:"+task.ID, search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if sh.Ref != "task:"+task.ID || sh.Kind != search.KindTask || sh.Title != "the card" || sh.More {
		t.Fatalf("shown %+v", sh)
	}
	for _, s := range []string{"state: blocked, shared", "holders: " + bob, "blocked: after dep (on task " + dep.ID + ")",
		"do the thing", "it works", "Linear ABC-9: https://linear.app/acme/issue/ABC-9/x", "comments (13, newest 10 shown",
		"comment d", "comment l", "show comment:" + cids[12]} {
		if !strings.Contains(sh.Text, s) {
			t.Fatalf("show lacks %q:\n%s", s, sh.Text)
		}
	}
	if strings.Contains(sh.Text, "comment c") {
		t.Fatal("an older comment was rendered")
	}
	wantRel := []string{"task:" + dep.ID, "comment:" + cids[2], "comment:" + cids[1], "comment:" + cids[0]}
	if !reflect.DeepEqual(sh.Related, wantRel) {
		t.Fatalf("related = %v, want %v", sh.Related, wantRel)
	}
	// Paging.
	p1, _ := e.c.SearchShow(ctx, "task:"+task.ID, search.ShowOptions{Limit: 100})
	if !p1.More || p1.Next != len(p1.Text) {
		t.Fatalf("page 1 %+v", p1)
	}
	p2, _ := e.c.SearchShow(ctx, "task:"+task.ID, search.ShowOptions{Offset: p1.Next, Limit: 100})
	if !strings.HasPrefix(sh.Text[p1.Next:], p2.Text) {
		t.Fatal("page 2 doesn't follow page 1")
	}

	cs, err := e.c.SearchShow(ctx, "comment:"+cids[0], search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if cs.Kind != search.KindComment || cs.Title != "comment on the card" || !strings.Contains(cs.Text, "comment a") ||
		!reflect.DeepEqual(cs.Related, []string{"task:" + task.ID}) {
		t.Fatalf("comment shown %+v", cs)
	}

	for _, tc := range []struct {
		ref  string
		want error
	}{
		{"task:nope", search.ErrNotFound},
		{"comment:nope", search.ErrNotFound},
		{"msg:x", search.ErrNotFound},
		{"nonsense", search.ErrBadRef},
	} {
		_, err := e.c.SearchShow(ctx, tc.ref, search.ShowOptions{})
		if !errors.Is(err, tc.want) {
			t.Fatalf("%s: %v", tc.ref, err)
		}
	}
}

// ---- replay ----

func TestBoardReplayEquality(t *testing.T) {
	e := newEnv(t)
	c := e.c
	step := func() { e.clk.add(time.Second) }
	sh := must[Task](t)(c.CreateTask(ctx, alice, NewTask{Title: "sh", Shared: true, Links: []string{"docs/a.md"}}))
	step()
	must[Task](t)(c.ClaimTask(ctx, alice, sh.ID))
	must[Task](t)(c.Join(ctx, bob, sh.ID, bob))
	must[Task](t)(c.Join(ctx, user, sh.ID, carol))
	must[Task](t)(c.Leave(ctx, alice, sh.ID, alice))
	step()
	must[Task](t)(c.Block(ctx, bob, sh.ID, "waiting", &BlockedBy{Kind: BlockedOnPerson}))
	mustN(t)(c.Unblock(ctx, user, sh.ID, "done waiting"))
	must[Task](t)(c.AddLink(ctx, carol, sh.ID, "https://github.com/a/b/issues/3"))
	must[Task](t)(c.RemoveLink(ctx, bob, sh.ID, "docs/a.md"))
	step()
	_, _, _ = c.AddComment(ctx, user, sh.ID, "hello", true)
	must[Lease](t)(c.Acquire(ctx, alice, "port:5173", time.Hour, false))
	un := must[Task](t)(c.CreateTask(ctx, bob, NewTask{Title: "un", BlockReason: "port", BlockedBy: &BlockedBy{Kind: BlockedOnLease, Ref: "port:5173"}}))
	mustN(t)(c.Assign(ctx, carol, un.ID, dave))
	must[Task](t)(c.SetShared(ctx, dave, un.ID, true))
	_, _ = c.Acquire(ctx, bob, "port:5173", 0, false)
	must[Lease](t)(c.Acquire(ctx, alice, "gpu/x", time.Hour, false))
	_, _ = c.Acquire(ctx, carol, "gpu/x", 0, false)
	_ = c.Release(ctx, alice, "gpu/x")
	step()

	c2 := e.reopen()
	if a, b := c.ListTasks(TaskFilter{}), c2.ListTasks(TaskFilter{}); !reflect.DeepEqual(a, b) {
		t.Fatalf("tasks differ:\n%+v\n%+v", a, b)
	}
	if a, b := c.Comments(sh.ID), c2.Comments(sh.ID); !reflect.DeepEqual(a, b) || len(a) != 1 {
		t.Fatalf("comments differ:\n%+v\n%+v", a, b)
	}
	if a, b := c.Refusals(), c2.Refusals(); !reflect.DeepEqual(a, b) || len(a) != 1 {
		t.Fatalf("refusals differ:\n%+v\n%+v", a, b)
	}
	if a, b := c.Board(BoardFilter{For: user}), c2.Board(BoardFilter{For: user}); !reflect.DeepEqual(a, b) {
		t.Fatalf("boards differ:\n%+v\n%+v", a, b)
	}
	got, _ := c2.GetTask(sh.ID)
	if !reflect.DeepEqual(got.Holders, []string{bob, carol}) || got.State != Working || got.Note != "done waiting" ||
		len(got.Links) != 1 || got.Links[0].Label != "GitHub #3" || got.Comments != 1 {
		t.Fatalf("replayed sh = %+v", got)
	}
}

func TestBoardReplayTolerates(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	for _, row := range []struct {
		kind    string
		payload any
	}{
		{KindTaskJoined, map[string]any{"taskId": task.ID}},               // no who
		{KindTaskJoined, map[string]any{"taskId": "missing", "who": bob}}, // no task
		{KindTaskLinked, map[string]any{"taskId": task.ID, "link": map[string]any{}}},
		{KindTaskComment, map[string]any{"taskId": "missing", "commentId": "c1", "text": "x"}},
		{KindBlockerClosed, map[string]any{"taskId": task.ID, "restored": "working"}}, // not blocked
		{KindLeaseRefused, map[string]any{"resource": "gpu/x"}},                       // no by
		{KindTaskShared, "nope"},
	} {
		if _, err := e.st.Append(ctx, "irori", row.kind, row.payload); err != nil {
			t.Fatal(err)
		}
	}
	c2 := e.reopen()
	got, ok := c2.GetTask(task.ID)
	if !ok || got.State != Submitted || got.Holders != nil || got.Links != nil {
		t.Fatalf("got %+v", got)
	}
	if c2.Comments("missing") != nil {
		t.Fatal("comment on a missing task kept")
	}
}

// The person's comment from the app reaches, for each kind of card: the
// holders that work it, then those still assigned, asked to answer; with
// none of them around, the creator; the watchers, told who answers and
// woken only when nobody else heard. With nobody around it is written
// unheard and tells nobody.
func TestPersonCommentHearers(t *testing.T) {
	gone := map[string]bool{}
	around := func(k string) bool { return !gone[k] }
	for _, tc := range []struct {
		name         string
		setup        func(e *env) Task
		gone         []string
		answer       []string
		watch        []string
		watchWake    bool
		watchAnswers string // a line the watchers' notice says
	}{
		{"held", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
			must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
			return must[Task](t)(e.c.Watch(ctx, carol, task.ID))
		}, nil, []string{bob}, []string{carol}, false, bob + " answers it"},
		{"assigned", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "t"}))
			task, _ = mustN(t)(e.c.Assign(ctx, user, task.ID, bob))
			return task
		}, nil, []string{bob}, nil, false, ""},
		{"held and assigned: workers first", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "t", Shared: true}))
			mustN(t)(e.c.Assign(ctx, user, task.ID, carol))
			return must[Task](t)(e.c.Join(ctx, bob, task.ID, bob))
		}, nil, []string{bob, carol}, nil, false, ""},
		{"watched only", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "t"}))
			return must[Task](t)(e.c.Watch(ctx, carol, task.ID))
		}, nil, nil, []string{carol}, true, "Nobody else is on it"},
		{"nobody holds: the creator", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
			return must[Task](t)(e.c.Watch(ctx, carol, task.ID))
		}, nil, []string{alice}, []string{carol}, false, alice + " answers it"},
		{"the holder has gone: the creator", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
			return must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
		}, []string{bob}, []string{alice}, nil, false, ""},
		{"a creating watcher answers once", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
			return must[Task](t)(e.c.Watch(ctx, alice, task.ID))
		}, nil, []string{alice}, nil, false, ""},
		{"nobody reachable", func(e *env) Task {
			task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
			must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
			return must[Task](t)(e.c.Watch(ctx, carol, task.ID))
		}, []string{alice, bob, carol}, nil, nil, false, ""},
		{"the person's own card", func(e *env) Task {
			return must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "t"}))
		}, nil, nil, nil, false, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			e := newEnv(t)
			task := tc.setup(e)
			clear(gone)
			for _, g := range tc.gone {
				gone[g] = true
			}
			cm, ns, err := e.c.PersonComment(ctx, user, task.ID, "which tag?", true, around)
			if err != nil {
				t.Fatal(err)
			}
			unheard := tc.answer == nil && tc.watch == nil
			if cm.Unheard != unheard || cm.By != user || cm.Text != "which tag?" {
				t.Fatalf("comment %+v, want unheard %v", cm, unheard)
			}
			var answer, watch *Notice
			for i := range ns {
				n := &ns[i]
				if n.Kind != NoticeComment || n.Ref != "comment:"+cm.ID || !strings.Contains(n.Text, "which tag?") {
					t.Fatalf("notice %+v", n)
				}
				if n.Watching {
					watch = n
				} else {
					answer = n
				}
			}
			if len(ns) != btoi(answer != nil)+btoi(watch != nil) {
				t.Fatalf("notices %+v", ns)
			}
			if tc.answer == nil != (answer == nil) || answer != nil && (!reflect.DeepEqual(answer.To, tc.answer) || !answer.Wake ||
				!strings.Contains(answer.Text, "Answer it on the card with board_comment")) {
				t.Fatalf("answer notice %+v, want to %v", answer, tc.answer)
			}
			if tc.watch == nil != (watch == nil) || watch != nil && (!reflect.DeepEqual(watch.To, tc.watch) || watch.Wake != tc.watchWake ||
				!strings.Contains(watch.Text, tc.watchAnswers)) {
				t.Fatalf("watch notice %+v, want to %v wake %v", watch, tc.watch, tc.watchWake)
			}
			// Unheard is in the row: a replay (or another device) folds it.
			if g, _ := e.reopen().GetComment(cm.ID); g.Unheard != unheard {
				t.Fatalf("replayed unheard = %v", g.Unheard)
			}
		})
	}
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	_, _, err := e.c.PersonComment(ctx, bob, task.ID, "me too", false, nil)
	wantErr(t, err, ErrInvalid)
	_, _, err = e.c.PersonComment(ctx, user, task.ID, " ", false, nil)
	wantErr(t, err, ErrInvalid)
}

func btoi(b bool) int {
	if b {
		return 1
	}
	return 0
}
