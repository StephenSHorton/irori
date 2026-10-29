package coord

import (
	"encoding/json"
	"errors"
	"slices"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/search"
	"github.com/StephenSHorton/irori/internal/store"
)

func get(t *testing.T, c *Coord, id string) Task {
	t.Helper()
	x, ok := c.GetTask(id)
	if !ok {
		t.Fatalf("no task %s", id)
	}
	return x
}

// A split moves every open step to a subtask, keeps the done ones on the
// parent, and the parent's progress and state follow its subtasks.
func TestSplitAndRollup(t *testing.T) {
	e := newEnv(t)
	big := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "big", Steps: []Step{
		{Text: "design"}, {Text: "backend"}, {Text: "tools"}, {Text: "window"},
	}}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, big.ID))
	must[Task](t)(e.c.SetStep(ctx, alice, big.ID, "s1", StepUpdate{Done: ptr(true)}))

	// An open step left out is refused, with its id named.
	_, _, err := e.c.Split(ctx, alice, big.ID, []NewSubtask{{Title: "a", Steps: []string{"s2"}}}, nil)
	if err == nil || !strings.Contains(err.Error(), "s3") || !strings.Contains(err.Error(), "s4") {
		t.Fatalf("unplaced steps: %v", err)
	}
	// A done step can't move, and a step can't go to two subtasks.
	if _, _, err := e.c.Split(ctx, alice, big.ID, []NewSubtask{{Title: "a", Steps: []string{"s1", "s2", "s3", "s4"}}}, nil); err == nil {
		t.Fatal("a done step moved")
	}
	if _, _, err := e.c.Split(ctx, alice, big.ID, []NewSubtask{{Title: "a", Steps: []string{"s2", "s3"}}, {Title: "b", Steps: []string{"s3", "s4"}}}, nil); err == nil {
		t.Fatal("a step went to two subtasks")
	}
	// Someone who doesn't manage it can't split it.
	if _, _, err := e.c.Split(ctx, bob, big.ID, []NewSubtask{{Title: "a", Steps: []string{"s2", "s3", "s4"}}}, nil); !errors.Is(err, ErrNotAllowed) {
		t.Fatalf("bob split alice's card: %v", err)
	}

	p, kids, err := e.c.Split(ctx, alice, big.ID, []NewSubtask{
		{Title: "backend", Steps: []string{"s2", "extra test"}},
		{Title: "tools", Steps: []string{"s3"}},
		{Title: "window", Steps: []string{"s4"}},
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(kids) != 3 || len(p.Subtasks) != 3 || p.Subtasks[0] != kids[0].ID {
		t.Fatalf("subtasks %v, made %d", p.Subtasks, len(kids))
	}
	if len(p.Steps) != 1 || !p.Steps[0].Done || p.Steps[0].Text != "design" {
		t.Fatalf("parent steps %+v", p.Steps)
	}
	if k := kids[0]; k.Parent != big.ID || len(k.Steps) != 2 || k.Steps[0].Text != "backend" || k.Steps[1].Text != "extra test" {
		t.Fatalf("subtask %+v", k)
	}
	if r := p.Rollup; r == nil || r.State != Submitted || r.Done != 0 || r.Total != 3 {
		t.Fatalf("rollup after split %+v", r)
	}

	// Two agents take subtasks: the parent works.
	must[Task](t)(e.c.ClaimTask(ctx, bob, kids[0].ID))
	must[Task](t)(e.c.ClaimTask(ctx, carol, kids[1].ID))
	if r := get(t, e.c, big.ID).Rollup; r.State != Working {
		t.Fatalf("rollup with two working %+v", r)
	}
	// A blocked subtask shows the parent blocked, naming it.
	if _, _, err := e.c.Raise(ctx, bob, kids[0].ID, NewBlocker{Question: "which db?"}); err != nil {
		t.Fatal(err)
	}
	if r := get(t, e.c, big.ID).Rollup; r.State != Blocked || r.Blocked != kids[0].ID || r.BlockedQuestion != "which db?" {
		t.Fatalf("rollup blocked %+v", r)
	}
	if v := e.c.Board(BoardFilter{}); !slices.ContainsFunc(v.Blocked, func(c Card) bool { return c.ID == big.ID }) &&
		!slices.ContainsFunc(v.NeedsYou, func(c Card) bool { return c.ID == big.ID }) {
		t.Fatal("the board doesn't show the parent blocked")
	}

	// Its steps and derived states can't be set on the parent.
	if _, err := e.c.SetSteps(ctx, alice, big.ID, []Step{{Text: "x"}}); !errors.Is(err, ErrSubtask) {
		t.Fatalf("steps on a parent: %v", err)
	}
	if _, err := e.c.SetTaskState(ctx, alice, big.ID, InputRequired, ""); !errors.Is(err, ErrSubtask) {
		t.Fatalf("input_required on a parent: %v", err)
	}
	if _, err := e.c.SetTaskState(ctx, alice, big.ID, Completed, ""); !errors.Is(err, ErrSubtask) ||
		!strings.Contains(err.Error(), kids[0].ShortRef()) {
		t.Fatalf("completing a parent with open subtasks: %v", err)
	}

	// Done/total counts completed subtasks; canceled ones leave the total.
	if _, err := e.c.SetTaskState(ctx, bob, kids[0].ID, Completed, ""); err != nil {
		t.Fatal(err)
	}
	must[Task](t)(e.c.ClaimTask(ctx, alice, kids[2].ID))
	if _, err := e.c.SetTaskState(ctx, alice, kids[2].ID, Canceled, "not needed"); err != nil {
		t.Fatal(err)
	}
	if r := get(t, e.c, big.ID).Rollup; r.Done != 1 || r.Total != 2 || r.State != Working {
		t.Fatalf("rollup 1 of 2 %+v", r)
	}
	if _, err := e.c.SetTaskState(ctx, carol, kids[1].ID, Completed, ""); err != nil {
		t.Fatal(err)
	}
	pp := get(t, e.c, big.ID)
	if pp.Rollup.State != Completed || EffectiveState(pp) != Completed || pp.State == Completed {
		t.Fatalf("all ended: rollup %+v, own %s", pp.Rollup, pp.State)
	}
	if v := e.c.Board(BoardFilter{}); !slices.ContainsFunc(v.Done, func(c Card) bool { return c.ID == big.ID }) {
		t.Fatal("a parent whose subtasks all ended isn't in Done")
	}

	// A replay folds the same.
	c2 := e.reopen()
	if r := get(t, c2, big.ID).Rollup; r == nil || r.State != Completed || r.Done != 2 || r.Total != 2 {
		t.Fatalf("after replay %+v", r)
	}
}

func ptr[T any](v T) *T { return &v }

// One level: a subtask can't take subtasks, a parent can't become one, and
// board_create parent on a subtask is refused with its parent named.
func TestSubtasksAreOneLevel(t *testing.T) {
	e := newEnv(t)
	p := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "parent"}))
	k := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "kid", Parent: p.ShortRef()}))
	if k.Parent != p.ID || !slices.Equal(get(t, e.c, p.ID).Subtasks, []string{k.ID}) {
		t.Fatalf("board_create parent: %+v", k)
	}
	if _, err := e.c.CreateTask(ctx, alice, NewTask{Title: "grandkid", Parent: k.ID}); !errors.Is(err, ErrSubtask) ||
		!strings.Contains(err.Error(), p.ShortRef()) {
		t.Fatalf("a subtask of a subtask: %v", err)
	}
	other := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "other"}))
	if _, _, err := e.c.Split(ctx, alice, other.ID, nil, []string{p.ID}); !errors.Is(err, ErrSubtask) {
		t.Fatalf("adopting a parent: %v", err)
	}
	if _, _, err := e.c.Split(ctx, alice, k.ID, []NewSubtask{{Title: "x"}}, nil); !errors.Is(err, ErrSubtask) {
		t.Fatalf("splitting a subtask: %v", err)
	}
	// Adopt moves an existing card under the parent.
	_, _, err := e.c.Split(ctx, alice, p.ID, nil, []string{other.ShortRef()})
	if err != nil {
		t.Fatal(err)
	}
	if got := get(t, e.c, p.ID).Subtasks; !slices.Equal(got, []string{k.ID, other.ID}) {
		t.Fatalf("after adopt %v", got)
	}
}

// Canceling a parent cancels its subtasks still in play.
func TestCancelParentCascades(t *testing.T) {
	e := newEnv(t)
	p := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "parent"}))
	a := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "a", Parent: p.ID}))
	b := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "b", Parent: p.ID}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, a.ID))
	if _, err := e.c.SetTaskState(ctx, bob, a.ID, Completed, ""); err != nil {
		t.Fatal(err)
	}
	if _, err := e.c.SetTaskState(ctx, alice, p.ID, Canceled, "dropped"); err != nil {
		t.Fatal(err)
	}
	if s := get(t, e.c, a.ID).State; s != Completed {
		t.Fatalf("a done subtask became %s", s)
	}
	if s := get(t, e.c, b.ID).State; s != Canceled {
		t.Fatalf("an open subtask is %s", s)
	}
	if s := EffectiveState(get(t, e.c, p.ID)); s != Canceled {
		t.Fatalf("the parent is %s", s)
	}
}

// parentRow is a task.parent row as another device wrote it.
func parentRow(dev string, seq int64, at hlc.Timestamp, child, parent string) store.Event {
	raw, _ := json.Marshal(taskParentRow{TaskID: child, Parent: parent, By: dev + "/s", At: fixedNow})
	return store.Event{DeviceID: dev, Seq: seq, HLC: at, Actor: "agent:" + dev + "/s", Kind: KindTaskParent, Payload: raw}
}

// Two devices apart each make half of a two-level link: B under A on one,
// C under B on the other. Every arrival order settles the same way: the
// earlier row (B under A) stands, and C under B is inert, since B is a
// subtask there. Replay agrees.
func TestTwoLevelLinkSettlesInAnyOrder(t *testing.T) {
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	rows := []store.Event{
		createdRow("dev-b", 1, base+1, "A", 1),
		createdRow("dev-b", 2, base+2, "B", 2),
		createdRow("dev-b", 3, base+3, "C", 3),
		parentRow("dev-b", 4, base+10, "B", "A"),
		parentRow("dev-c", 1, base+20, "C", "B"),
	}
	check := func(name string, c *Coord) {
		t.Helper()
		a, b, cc := get(t, c, "A"), get(t, c, "B"), get(t, c, "C")
		if b.Parent != "A" || cc.Parent != "" || !slices.Equal(a.Subtasks, []string{"B"}) || len(b.Subtasks) != 0 {
			t.Fatalf("%s: A %v, B parent %q kids %v, C parent %q", name, a.Subtasks, b.Parent, b.Subtasks, cc.Parent)
		}
	}
	for name, order := range map[string][]int{"in order": {0, 1, 2, 3, 4}, "c first": {4, 0, 1, 2, 3}, "late link": {0, 1, 2, 4, 3}} {
		e := newEnv(t)
		for _, i := range order {
			e.c.impl.applyRemote(ctx, rows[i])
		}
		check(name, e.c)
	}
	e := newEnv(t)
	for _, ev := range rows {
		if err := e.st.Insert(ctx, ev); err != nil {
			t.Fatal(err)
		}
	}
	check("replay", e.reopen())

	// The other way round (C under B first), B under A is the inert one.
	rows[3], rows[4] = parentRow("dev-b", 4, base+20, "B", "A"), parentRow("dev-c", 1, base+10, "C", "B")
	e = newEnv(t)
	for _, ev := range rows {
		e.c.impl.applyRemote(ctx, ev)
	}
	if b, cc := get(t, e.c, "B"), get(t, e.c, "C"); b.Parent != "" || cc.Parent != "B" {
		t.Fatalf("C under B first: B parent %q, C parent %q", b.Parent, cc.Parent)
	}
}

// A subtask's change reaches its parent's holder in the watch digest.
func TestSubtaskChangesReachTheParentsHolder(t *testing.T) {
	e := newEnv(t)
	p := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "parent"}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, p.ID))
	k := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "kid", Parent: p.ID}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, k.ID))
	ns, err := e.c.SweepNotices(ctx)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, n := range ns {
		if slices.Contains(n.To, alice) && strings.Contains(n.Text, "(part of "+p.ShortRef()+")") {
			found = true
		}
	}
	if !found {
		t.Fatalf("the parent's holder heard nothing: %+v", ns)
	}
}

// An open ask is neither a parent nor a subtask; closed as a card with a
// parent, it becomes a subtask in the same call.
func TestAsksAndSubtasks(t *testing.T) {
	e := newEnv(t)
	p := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "parent"}))
	ask, _ := postAsk(t, e, "can someone look at the login flow?")
	if _, err := e.c.CreateTask(ctx, alice, NewTask{Title: "kid", Parent: ask.ID}); !errors.Is(err, ErrSubtask) {
		t.Fatalf("a subtask of an open ask: %v", err)
	}
	if _, _, err := e.c.Split(ctx, alice, p.ID, nil, []string{ask.ID}); !errors.Is(err, ErrSubtask) {
		t.Fatalf("adopting an open ask: %v", err)
	}
	if _, _, err := e.c.Split(ctx, user, ask.ID, []NewSubtask{{Title: "x"}}, nil); !errors.Is(err, ErrSubtask) {
		t.Fatalf("splitting an open ask: %v", err)
	}
	// An older device's task.parent on the open ask stays inert.
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	e.remote("dev-b", 1, base+1, "agent:dev-b/s", KindTaskParent, taskParentRow{TaskID: ask.ID, Parent: p.ID, By: "dev-b/s", At: fixedNow})
	if x := get(t, e.c, ask.ID); x.Parent != "" {
		t.Fatalf("an open ask got a parent from an older device: %q", x.Parent)
	}

	must[ClaimResult](t)(e.c.AskClaim(ctx, bob, ask.ID, 0))
	if _, err := e.c.AskClose(ctx, bob, ask.ID, AskOutcome{Kind: OutcomeNone, Note: "x", Parent: p.ID}); err == nil {
		t.Fatal("parent with a none outcome")
	}
	if _, err := e.c.AskClose(ctx, bob, ask.ID, AskOutcome{Kind: OutcomeCard, Note: "it's a card", Title: "Fix login", Parent: p.ShortRef()}); err != nil {
		t.Fatal(err)
	}
	if x := get(t, e.c, ask.ID); x.Parent != p.ID || !slices.Contains(get(t, e.c, p.ID).Subtasks, ask.ID) {
		t.Fatalf("after ask_close card with parent: parent %q, kids %v", x.Parent, get(t, e.c, p.ID).Subtasks)
	}
	if x := get(t, e.reopen(), ask.ID); x.Parent != p.ID {
		t.Fatalf("after replay the closed ask's parent is %q", x.Parent)
	}
}

// show names a subtask's parent and lists a parent's subtasks.
func TestShowSubtasks(t *testing.T) {
	e := newEnv(t)
	p := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "parent"}))
	k := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "kid", Parent: p.ID}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, k.ID))
	sh, err := e.c.SearchShow(ctx, "task:"+p.ID, search.ShowOptions{})
	if err != nil || !strings.Contains(sh.Text, "subtasks: \n- "+k.ShortRef()+" [working] \"kid\": "+bob) ||
		!strings.Contains(sh.Text, "from its subtasks: 0/1 done") {
		t.Fatalf("show parent: %v\n%s", err, sh.Text)
	}
	sh, err = e.c.SearchShow(ctx, "task:"+k.ID, search.ShowOptions{})
	if err != nil || !strings.Contains(sh.Text, "part of: "+p.ShortRef()+" \"parent\"") {
		t.Fatalf("show subtask: %v\n%s", err, sh.Text)
	}
}

// A rejected re-parent row is inert: the card keeps the parent it had. B
// is under A; a later row puts B under C, which is itself a subtask of D,
// so it would make two levels. In any order, and on replay, B stays
// under A.
func TestRejectedReparentKeepsTheOldParent(t *testing.T) {
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	rows := []store.Event{
		createdRow("dev-b", 1, base+1, "A", 1),
		createdRow("dev-b", 2, base+2, "B", 2),
		createdRow("dev-b", 3, base+3, "C", 3),
		createdRow("dev-b", 4, base+4, "D", 4),
		parentRow("dev-b", 5, base+10, "B", "A"),
		parentRow("dev-b", 6, base+11, "C", "D"),
		parentRow("dev-c", 1, base+20, "B", "C"),
	}
	check := func(name string, c *Coord) {
		t.Helper()
		if b := get(t, c, "B"); b.Parent != "A" {
			t.Fatalf("%s: B's parent is %q, want A", name, b.Parent)
		}
		if a := get(t, c, "A"); !slices.Equal(a.Subtasks, []string{"B"}) {
			t.Fatalf("%s: A's subtasks %v", name, a.Subtasks)
		}
	}
	for name, order := range map[string][]int{"in order": {0, 1, 2, 3, 4, 5, 6}, "reparent first": {6, 0, 1, 2, 3, 4, 5}} {
		e := newEnv(t)
		for _, i := range order {
			e.c.impl.applyRemote(ctx, rows[i])
		}
		check(name, e.c)
	}
	e := newEnv(t)
	for _, ev := range rows {
		if err := e.st.Insert(ctx, ev); err != nil {
			t.Fatal(err)
		}
	}
	check("replay", e.reopen())
}

// Adopting a card tells the parent's holder, and the parent's steps line
// in show says they came before the split.
func TestAdoptionReachesTheParentsHolder(t *testing.T) {
	e := newEnv(t)
	// alice made the card and splits it; carol holds it.
	p := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "parent", Steps: []Step{{Text: "plan"}, {Text: "build"}}}))
	must[Task](t)(e.c.ClaimTask(ctx, carol, p.ID))
	must[Task](t)(e.c.SetStep(ctx, carol, p.ID, "s1", StepUpdate{Done: ptr(true)}))
	other := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "related"}))
	if _, err := e.c.SweepNotices(ctx); err != nil { // clear what came before
		t.Fatal(err)
	}
	if _, _, err := e.c.Split(ctx, alice, p.ID, []NewSubtask{{Title: "build it", Steps: []string{"s2"}}}, []string{other.ID}); err != nil {
		t.Fatal(err)
	}
	ns, err := e.c.SweepNotices(ctx)
	if err != nil {
		t.Fatal(err)
	}
	adopted, made, parentLine := false, false, false
	for _, n := range ns {
		if !slices.Contains(n.To, carol) {
			continue
		}
		adopted = adopted || strings.Contains(n.Text, "made it part of "+p.ShortRef())
		made = made || strings.Contains(n.Text, "put the card up as a subtask of "+p.ShortRef())
		if n.TaskID == p.ID {
			parentLine = strings.Contains(n.Text, "subtasks 0/2 done") && !strings.Contains(n.Text, "steps 1/1")
		}
	}
	if !adopted || !made || !parentLine {
		t.Fatalf("the parent's holder didn't hear the adoption: %+v", ns)
	}
	sh, err := e.c.SearchShow(ctx, "task:"+p.ID, search.ShowOptions{})
	if err != nil || !strings.Contains(sh.Text, "steps done before the split:") || strings.Contains(sh.Text, "steps (1/1") {
		t.Fatalf("show parent steps: %v\n%s", err, sh.Text)
	}
}
