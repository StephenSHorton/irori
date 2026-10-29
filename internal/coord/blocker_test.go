package coord

import (
	"reflect"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

// Blocker threads: the person's answer is a blocker comment sent to the
// raiser, an agent's reply sends it back, and only a resolve (with a note
// once the person answered) closes the item.

// raisedOn is a task alice holds with blocker b1 (two options) raised by
// alice.
func raisedOn(t *testing.T, e *env) Task {
	t.Helper()
	task := claimed(t, e)
	if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "Which adapter?",
		Options: []Option{{Answer: "Wintun"}, {Answer: "TAP-Windows"}}}); err != nil {
		t.Fatal(err)
	}
	return task
}

func blockerOf(t *testing.T, c *Coord, id, bid string) (Task, Blocker) {
	t.Helper()
	got, ok := c.GetTask(id)
	if !ok {
		t.Fatalf("no task %s", id)
	}
	for _, b := range got.Blockers {
		if b.ID == bid {
			return got, b
		}
	}
	t.Fatalf("task %s has no blocker %s: %+v", id, bid, got.Blockers)
	return Task{}, Blocker{}
}

func reply(t *testing.T, c *Coord, by, id, bid, text string, missing ...string) {
	t.Helper()
	if _, n, err := c.CommentOn(ctx, by, id, CommentInput{Text: text, BlockerID: bid, Missing: missing}); err != nil {
		t.Fatal(err)
	} else if len(n.To) != 0 || n.Text != "" {
		t.Fatalf("a blocker reply routed a notice: %+v", n)
	}
}

func TestBlockerStates(t *testing.T) {
	e := newEnv(t)
	task := raisedOn(t, e)
	answer := func(a BlockerAnswer) func() {
		return func() { mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", a)) }
	}
	steps := []struct {
		name    string
		do      func()
		status  BlockerStatus
		sentTo  string
		missing []string
		last    string // the newest comment's text
	}{
		{"raised", func() {}, BlockerOpen, "", nil, ""},
		{"an agent adds context", func() { reply(t, e.c, carol, task.ID, "b1", "Radmin isn't open source.", "ignored before an answer") }, BlockerOpen, "", nil, "Radmin isn't open source."},
		{"the person asks back", answer(BlockerAnswer{Text: "which is signed?"}), BlockerAnswered, alice, nil, "which is signed?"},
		{"the person picks", answer(BlockerAnswer{Pick: 2}), BlockerAnswered, alice, nil, "TAP-Windows"},
		{"the raiser replies", func() { reply(t, e.c, alice, task.ID, "b1", "TAP needs a port.", "which port") }, BlockerReplied, "", []string{"which port"}, "TAP needs a port."},
		{"the person again", answer(BlockerAnswer{Text: "4242"}), BlockerAnswered, alice, nil, "4242"},
	}
	for _, s := range steps {
		s.do()
		got, b := blockerOf(t, e.c, task.ID, "b1")
		if b.Status != s.status || b.SentTo != s.sentTo || !reflect.DeepEqual(b.Missing, s.missing) || !b.Open() || got.State != Blocked {
			t.Fatalf("%s: task %s blocker %+v", s.name, got.State, b)
		}
		if cs := e.c.Comments(task.ID); s.last != "" && (len(cs) == 0 || cs[len(cs)-1].Text != s.last || cs[len(cs)-1].BlockerID != "b1") {
			t.Fatalf("%s: comments %+v", s.name, cs)
		}
	}
	cs := e.c.Comments(task.ID)
	if pick := cs[2]; pick.Pick != 2 || pick.To != alice || pick.By != user || !pick.person {
		t.Fatalf("picked answer %+v", pick)
	}
	if acts := e.c.Activity(task.ID); acts[0].What != "answered blocker b1" || acts[0].BlockerID != "b1" || acts[1].What != "replied on blocker b1" {
		t.Fatalf("activity %+v", acts[:2])
	}
	sh, err := e.c.SearchShow(ctx, "task:"+task.ID, search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{"(answered, waiting on " + alice, user + " on b1: TAP-Windows (option 2)", alice + " on b1: TAP needs a port. [missing: which port]"} {
		if !strings.Contains(sh.Text, s) {
			t.Fatalf("show lacks %q:\n%s", s, sh.Text)
		}
	}

	// The raiser resolves with a note: the person's latest answer stands.
	got, n := mustN(t)(e.c.Answer(ctx, alice, task.ID, "b1", BlockerAnswer{Text: "using port 4242", Resolve: true}))
	b := got.Blockers[0]
	if r := b.Resolved; r == nil || r.Answer != "4242" || r.Note != "using port 4242" || r.Option != -1 || r.By != alice || r.Closed || got.State != Working {
		t.Fatalf("resolved: task %+v item %+v", got, b.Resolved)
	}
	if !strings.Contains(n.Text, "resolved blocker b1") || !strings.Contains(n.Text, "using port 4242") || !strings.Contains(n.Text, "working again") {
		t.Fatalf("notice %+v", n)
	}
	if acts := e.c.Activity(task.ID); acts[0].What != "resolved “Which adapter?”: using port 4242" {
		t.Fatalf("activity %+v", acts[0])
	}
	if a, b := e.c.ListTasks(TaskFilter{}), e.reopen().ListTasks(TaskFilter{}); !reflect.DeepEqual(a, b) {
		t.Fatalf("replay differs:\n%+v\n%+v", a, b)
	}
	// A picked answer resolves to its option.
	e2 := newEnv(t)
	t2 := raisedOn(t, e2)
	mustN(t)(e2.c.Answer(ctx, user, t2.ID, "b1", BlockerAnswer{Pick: 1}))
	got, _ = mustN(t)(e2.c.Answer(ctx, alice, t2.ID, "b1", BlockerAnswer{Text: "on it", Resolve: true}))
	if r := got.Blockers[0].Resolved; r == nil || r.Answer != "Wintun" || r.Option != 0 || r.Note != "on it" {
		t.Fatalf("resolved %+v", r)
	}
}

// Only the signed actor "user" with an owner key makes a person's answer,
// and an agent's reply counts only from the device its key names.
func TestBlockerSpoofIgnored(t *testing.T) {
	e := newEnv(t)
	other := "user:dev-b"
	e.owner = func(k string) bool { return k == user || k == other }
	e.c = e.reopen()
	task := raisedOn(t, e)
	at := hlcAt(fixedNow.Add(time.Minute))
	row := func(id, by, text string) taskCommentRow {
		return taskCommentRow{TaskID: task.ID, CommentID: id, By: by, Text: text, BlockerID: "b1", At: fixedNow}
	}
	check := func(c *Coord, want BlockerStatus, comments, ignored int) {
		t.Helper()
		if _, b := blockerOf(t, c, task.ID, "b1"); b.Status != want {
			t.Fatalf("status %q, want %q: %+v", b.Status, want, b)
		}
		if n := len(c.Comments(task.ID)); n != comments {
			t.Fatalf("%d comments, want %d", n, comments)
		}
		if n := c.IgnoredRows(); n != ignored {
			t.Fatalf("ignored %d rows, want %d", n, ignored)
		}
	}
	// An agent on dev-b writing as the person's dev-b key: a comment, not
	// the person's answer.
	e.remote("dev-b", 1, at+1, "agent:dev-b/x", KindTaskComment, row("c1", other, "as the person"))
	check(e.c, BlockerOpen, 1, 0)
	// The person on dev-b answers.
	e.remote("dev-b", 2, at+2, "user", KindTaskComment, row("c2", other, "yes"))
	check(e.c, BlockerAnswered, 2, 0)
	// dev-b writes a reply in alice's name (alice is on dev-a): ignored.
	e.remote("dev-b", 3, at+3, "agent:dev-b/x", KindTaskComment, row("c3", alice, "spoofed reply"))
	check(e.c, BlockerAnswered, 2, 1)
	check(e.reopen(), BlockerAnswered, 2, 1)
}

// A blocker comment can reach a device before the row raising its item;
// the raise picks it up, live and on replay.
func TestBlockerCommentBeforeRaise(t *testing.T) {
	e := newEnv(t)
	other := "user:dev-b"
	e.owner = func(k string) bool { return k == user || k == other }
	e.c = e.reopen()
	task := claimed(t, e)
	at := hlcAt(fixedNow.Add(time.Minute))
	// The person on dev-b answers; the raise from dev-c reaches dev-a later.
	e.remote("dev-b", 1, at+2, "user", KindTaskComment, taskCommentRow{
		TaskID: task.ID, CommentID: "c1", By: other, To: "dev-c/raiser", Text: "go", BlockerID: "b1", At: fixedNow,
	})
	if got, _ := e.c.GetTask(task.ID); len(got.Blockers) != 0 || got.Comments != 1 {
		t.Fatalf("before the raise: %+v", got)
	}
	e.remote("dev-c", 1, at+1, "agent:dev-c/raiser", KindTaskState, taskStateRow{
		TaskID: task.ID, State: Blocked, Reason: "Ship?", By: "dev-c/raiser", At: fixedNow,
		Blocker: &blockerRow{ID: "b1", Question: "Ship?", On: &BlockedBy{Kind: BlockedOnPerson}},
	})
	for _, c := range []*Coord{e.c, e.reopen()} {
		if _, b := blockerOf(t, c, task.ID, "b1"); b.Status != BlockerAnswered || b.SentTo != "dev-c/raiser" {
			t.Fatalf("after the raise: %+v", b)
		}
	}
}

func TestResolveNeedsNoteAfterAnswer(t *testing.T) {
	// Before any answer, resolving is as it was: a pick or own words.
	e := newEnv(t)
	task := raisedOn(t, e)
	got, _ := mustN(t)(e.c.Answer(ctx, carol, task.ID, "b1", BlockerAnswer{Pick: 1, Resolve: true}))
	if r := got.Blockers[0].Resolved; r == nil || r.Answer != "Wintun" || r.Note != "" || r.Option != 0 {
		t.Fatalf("resolved %+v", r)
	}

	e = newEnv(t)
	task = raisedOn(t, e)
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "TAP, on 4242"}))
	for _, a := range []BlockerAnswer{
		{Pick: 1, Resolve: true},
		{Text: "  ", Resolve: true},
		{Text: "two\nlines", Resolve: true},
		{Text: strings.Repeat("n", MaxAnswer+1), Resolve: true},
		{Text: "an agent can't answer", Resolve: false},
	} {
		_, _, err := e.c.Answer(ctx, alice, task.ID, "b1", a)
		wantErr(t, err, ErrInvalid)
	}
	if _, b := blockerOf(t, e.c, task.ID, "b1"); !b.Open() || b.Status != BlockerAnswered {
		t.Fatalf("a refused resolve changed it: %+v", b)
	}
	// The person may resolve without a note ("Mark resolved"): their answer stands.
	got, _ = mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Resolve: true}))
	if r := got.Blockers[0].Resolved; r == nil || r.Answer != "TAP, on 4242" || r.Note != "" || r.By != user || got.State != Working {
		t.Fatalf("resolved %+v", r)
	}
	// A reply on a resolved item, or by the person, is refused.
	_, _, err := e.c.CommentOn(ctx, alice, task.ID, CommentInput{Text: "late", BlockerID: "b1"})
	wantErr(t, err, ErrBadState)
	_, _, err = e.c.CommentOn(ctx, user, task.ID, CommentInput{Text: "mine", BlockerID: "b1"})
	wantErr(t, err, ErrInvalid)
	_, _, err = e.c.CommentOn(ctx, alice, task.ID, CommentInput{Text: "x", BlockerID: "b1", Missing: make([]string, MaxMissing+1)})
	wantErr(t, err, ErrInvalid)
}

// The person's answer tells the raiser first, then the holders, and says
// it is not resolved yet; a resolve tells them as before.
func TestAnswerNoticeRecipients(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "db"}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	if _, _, err := e.c.Raise(ctx, alice, task.ID, NewBlocker{Question: "which db?"}); err != nil {
		t.Fatal(err)
	}
	got, n := mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "postgres", Wake: true}))
	if got.State != Blocked || n.Kind != NoticeAnswered || !reflect.DeepEqual(n.To, []string{alice, bob}) || !n.Wake {
		t.Fatalf("got %+v notice %+v", got, n)
	}
	for _, s := range []string{
		"Answer: postgres", "It is not resolved yet.",
		"board_unblock task_id=" + task.ID + " blocker_id=b1 note=",
		"board_comment task_id=" + task.ID + " blocker_id=b1",
	} {
		if !strings.Contains(n.Text, s) {
			t.Fatalf("notice lacks %q: %q", s, n.Text)
		}
	}
	got, n = mustN(t)(e.c.Answer(ctx, alice, task.ID, "b1", BlockerAnswer{Text: "migrating", Resolve: true}))
	if got.State != Working || !reflect.DeepEqual(n.To, []string{bob}) || !strings.Contains(n.Text, alice+" resolved blocker b1") {
		t.Fatalf("got %+v notice %+v", got, n)
	}
}

func TestNeedsYouByBlockerState(t *testing.T) {
	e := newEnv(t)
	task := raisedOn(t, e)
	needs := func(viewer string) bool {
		for _, c := range e.c.Board(BoardFilter{For: viewer}).NeedsYou {
			if c.ID == task.ID {
				return true
			}
		}
		return false
	}
	check := func(when string, person, raiser bool) {
		t.Helper()
		if needs(user) != person || needs(alice) != raiser || needs(bob) {
			t.Fatalf("%s: person %v raiser %v bob %v", when, needs(user), needs(alice), needs(bob))
		}
	}
	check("open", true, false)
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "which one is signed?"}))
	check("answered", false, true)
	reply(t, e.c, alice, task.ID, "b1", "Wintun is.")
	check("replied", true, false)
}

// A v0.4.1 peer resolves with no note: it folds as resolved, the answer
// shown, even after a person's answer.
func TestOldPeerResolve(t *testing.T) {
	e := newEnv(t)
	task := raisedOn(t, e)
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "yes"}))
	type oldResolvedRow struct { // v0.4.1's blocker.resolved
		TaskID    string    `json:"taskId"`
		BlockerID string    `json:"blockerId"`
		Answer    string    `json:"answer"`
		Option    int       `json:"option"`
		By        string    `json:"by"`
		Restored  TaskState `json:"restored,omitempty"`
		At        time.Time `json:"at"`
	}
	e.remote("dev-b", 1, hlcAt(fixedNow.Add(time.Minute)), "user", KindBlockerResolved, oldResolvedRow{
		TaskID: task.ID, BlockerID: "b1", Answer: "fine", Option: -1, By: "user:dev-b", Restored: Working, At: fixedNow,
	})
	for _, c := range []*Coord{e.c, e.reopen()} {
		got, b := blockerOf(t, c, task.ID, "b1")
		if r := b.Resolved; r == nil || r.Answer != "fine" || r.Note != "" || got.State != Working {
			t.Fatalf("task %s item %+v", got.State, r)
		}
	}
	if acts := e.c.Activity(task.ID); acts[0].What != "answered “Which adapter?”: fine" {
		t.Fatalf("activity %+v", acts[0])
	}
}

// Blocking again with a reason replaces the open item Block made under
// the same id and asks afresh: the person's answer to the old question no
// longer counts, for its status or for the resolve, live and on replay.
func TestReblockAsksAfresh(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	block := func(reason string) {
		t.Helper()
		if _, err := e.c.Block(ctx, alice, task.ID, reason, &BlockedBy{Kind: BlockedOnPerson}); err != nil {
			t.Fatal(err)
		}
	}
	block("Which DB?")
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "Postgres"}))
	if _, b := blockerOf(t, e.c, task.ID, "b1"); b.Status != BlockerAnswered || b.SentTo != alice {
		t.Fatalf("answered %+v", b)
	}
	block("Which region?")
	for _, c := range []*Coord{e.c, e.reopen()} {
		got, b := blockerOf(t, c, task.ID, "b1")
		if len(got.Blockers) != 1 || b.Question != "Which region?" || b.Status != BlockerOpen || b.SentTo != "" || !b.Open() {
			t.Fatalf("re-blocked: %+v", got.Blockers)
		}
		for _, v := range []string{user, alice} {
			in := slices.ContainsFunc(c.Board(BoardFilter{For: v}).NeedsYou, func(cd Card) bool { return cd.ID == task.ID })
			if in != (v == user) {
				t.Fatalf("in %s's Needs you: %v", v, in)
			}
		}
	}
	// The person answers the new question; that answer is the one kept.
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "eu-west"}))
	got, _ := mustN(t)(e.c.Answer(ctx, alice, task.ID, "b1", BlockerAnswer{Text: "deploying to eu-west", Resolve: true}))
	if r := got.Blockers[0].Resolved; r == nil || r.Answer != "eu-west" || r.Note != "deploying to eu-west" || got.State != Working {
		t.Fatalf("resolved %+v", r)
	}
	if a, b := e.c.ListTasks(TaskFilter{}), e.reopen().ListTasks(TaskFilter{}); !reflect.DeepEqual(a, b) {
		t.Fatalf("replay differs:\n%+v\n%+v", a, b)
	}

	// Resolved before anyone answers the new question: the raiser's words
	// are the answer, not a note on the old one.
	e = newEnv(t)
	task = claimed(t, e)
	block("Which DB?")
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "Postgres"}))
	block("Which region?")
	got, _ = mustN(t)(e.c.Answer(ctx, alice, task.ID, "b1", BlockerAnswer{Text: "eu-west, it's closest", Resolve: true}))
	if r := got.Blockers[0].Resolved; r == nil || r.Answer != "eu-west, it's closest" || r.Note != "" {
		t.Fatalf("resolved %+v", r)
	}
}

// A reason-only row from an old peer (no item) replaces the open item the
// same way; of two replaces, the later in the log wins whichever folds
// first.
func TestReblockFromOldPeer(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	if _, err := e.c.Block(ctx, alice, task.ID, "Which DB?", nil); err != nil {
		t.Fatal(err)
	}
	mustN(t)(e.c.Answer(ctx, user, task.ID, "b1", BlockerAnswer{Text: "Postgres"}))
	at := hlcAt(fixedNow.Add(time.Minute))
	row := func(by, reason string) taskStateRow {
		return taskStateRow{TaskID: task.ID, State: Blocked, Reason: reason, By: by, At: fixedNow}
	}
	e.remote("dev-b", 1, at+2, "agent:dev-b/x", KindTaskState, row("dev-b/x", "Which region?"))
	e.remote("dev-c", 1, at+1, "agent:dev-c/y", KindTaskState, row("dev-c/y", "Which zone?"))
	for _, c := range []*Coord{e.c, e.reopen()} {
		got, b := blockerOf(t, c, task.ID, "b1")
		if len(got.Blockers) != 1 || b.Question != "Which region?" || b.Status != BlockerOpen || b.SentTo != "" || b.RaisedBy != "dev-b/x" {
			t.Fatalf("re-blocked: %+v", got.Blockers)
		}
	}
}
