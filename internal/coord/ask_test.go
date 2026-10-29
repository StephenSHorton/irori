package coord

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Asks (board.md "Asks"): the person's prompt to the room, one taker, an
// outcome.

func postAsk(t *testing.T, e *env, text string) (Task, Notice) {
	t.Helper()
	tk, n, err := e.c.PostAsk(ctx, user, NewAsk{Text: text}, e.ros.Agents())
	if err != nil {
		t.Fatal(err)
	}
	return tk, n
}

func askOf(t *testing.T, c *Coord, id string) Ask {
	t.Helper()
	tk, ok := c.GetTask(id)
	if !ok || tk.Ask == nil {
		t.Fatalf("no ask %s: %+v", id, tk)
	}
	return *tk.Ask
}

func TestPostAskTellsEveryFreeAgent(t *testing.T) {
	e := newEnv(t)
	e.ros.set(carol, "busy")
	tk, n := postAsk(t, e, "Why is the build slow on the Mac?\nIt was fine last week.")
	if tk.Title != "Why is the build slow on the Mac?" || !strings.Contains(tk.Objective, "last week") || tk.Short == "" {
		t.Fatalf("ask card %+v", tk)
	}
	a := askOf(t, e.c, tk.ID)
	if a.Owner != "dev-a" || a.Told != 2 || a.Reason != TakePosted || a.Taker != "" || a.Unclaimed {
		t.Fatalf("ask %+v", a)
	}
	if n.Kind != NoticeAsk || !n.Person || !reflect.DeepEqual(n.To, []string{alice, bob}) ||
		!strings.Contains(n.Text, "ask_claim #"+tk.Short) || n.Wake {
		t.Fatalf("notice %+v", n)
	}
	if _, _, err := e.c.PostAsk(ctx, alice, NewAsk{Text: "x"}, nil); !errors.Is(err, ErrNotAllowed) {
		t.Fatalf("an agent asked the room: %v", err)
	}
	if got := AskTitle(strings.Repeat("word ", 40)); len([]rune(got)) > maxAskTitle || !strings.HasSuffix(got, "…") {
		t.Fatalf("title cut %q", got)
	}
}

// Two agents claim at once: one takes it, the other hears who did.
func TestTwoLocalClaimsOneTaker(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "Fix the flaky test")
	var wg sync.WaitGroup
	res := make([]ClaimResult, 2)
	errs := make([]error, 2)
	for i, who := range []string{alice, bob} {
		wg.Add(1)
		go func() {
			defer wg.Done()
			res[i], errs[i] = e.c.AskClaim(ctx, who, tk.ID, time.Second)
		}()
	}
	wg.Wait()
	wins, held := 0, 0
	for i := range res {
		switch {
		case errs[i] == nil && res[i].Mine:
			wins++
		case errors.Is(errs[i], ErrHeld) && strings.Contains(errs[i].Error(), "took"):
			held++
		default:
			t.Fatalf("claim %d: %+v %v", i, res[i], errs[i])
		}
	}
	if wins != 1 || held != 1 {
		t.Fatalf("wins %d held %d", wins, held)
	}
	a := askOf(t, e.c, tk.ID)
	got, _ := e.c.GetTask(tk.ID)
	if a.Taker == "" || !reflect.DeepEqual(got.Holders, []string{a.Taker}) || got.State != Submitted {
		t.Fatalf("after the race: %+v %+v", a, got)
	}
	if n := len(e.rows(KindTakeState)); n != 2 {
		t.Fatalf("%d take.state rows, want posted + taken", n)
	}
	// The lock refuses the board's own ways in.
	other := bob
	if a.Taker == bob {
		other = alice
	}
	wantErr(t, errOnly(e.c.ClaimTask(ctx, other, tk.ID)), ErrNotAllowed)
	_, _, aerr := e.c.Assign(ctx, other, tk.ID, other)
	wantErr(t, aerr, ErrNotAllowed)
	wantErr(t, errOnly(e.c.SetTaskState(ctx, a.Taker, tk.ID, Completed, "")), ErrNotAllowed)
	wantErr(t, errOnly(e.c.Block(ctx, a.Taker, tk.ID, "why", nil)), ErrNotAllowed)
	if e.c.StaleNotice("task:"+tk.ID, other) == "" || e.c.StaleNotice("task:"+tk.ID, a.Taker) != "" {
		t.Fatal("a queued ask notice to the others isn't stale once taken")
	}
	// Replay agrees.
	if r := askOf(t, e.reopen(), tk.ID); r.Taker != a.Taker || r.Told != a.Told {
		t.Fatalf("replayed %+v, live %+v", r, a)
	}
}

func errOnly[T any](_ T, err error) error { return err }

// The taker gives it back (board_leave), or goes away: the ask opens again
// and the room hears again, the leaver aside.
func TestTakerGoneHandsItBack(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "Look at the crash report")
	if r := must[ClaimResult](t)(e.c.AskClaim(ctx, alice, tk.ID, time.Second)); !r.Mine {
		t.Fatalf("claim %+v", r)
	}
	e.ros.set(alice, "")
	ns := must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents())) // starts alice's clock
	if askOf(t, e.c, tk.ID).Taker != alice || len(ns) != 0 {
		t.Fatalf("released before the grace: %+v", ns)
	}
	e.clk.add(GoneGrace + time.Second)
	ns = must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	a := askOf(t, e.c, tk.ID)
	if a.Taker != "" || a.Reason != TakeGone {
		t.Fatalf("after the taker went: %+v", a)
	}
	if len(ns) != 1 || ns[0].Kind != NoticeAsk || !reflect.DeepEqual(ns[0].To, []string{bob, carol}) || !strings.Contains(ns[0].Text, "went away") {
		t.Fatalf("re-announced %+v", ns)
	}
	if r := must[ClaimResult](t)(e.c.AskClaim(ctx, bob, tk.ID, time.Second)); !r.Mine {
		t.Fatalf("bob's claim %+v", r)
	}
	// Bob gives it back.
	must[Task](t)(e.c.Leave(ctx, bob, tk.ID, bob))
	ns = must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if a := askOf(t, e.c, tk.ID); a.Taker != "" || a.Reason != TakeReleased {
		t.Fatalf("after leaving: %+v", a)
	}
	if len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{carol}) {
		t.Fatalf("after leaving, told %+v", ns)
	}
	// A restart doesn't grant the old claims again.
	c2 := e.reopen()
	must[[]Notice](t)(c2.SweepAsks(ctx, e.ros.Agents()))
	if a := askOf(t, c2, tk.ID); a.Taker != "" {
		t.Fatalf("an old claim was granted after a restart: %+v", a)
	}
}

// A taker on another device that is merely offline keeps the ask; once
// its device retires it, the owner opens it again.
func TestOfflineTakerKeepsIt(t *testing.T) {
	e := newEnv(t)
	const remote = "dev-b/far-session"
	tk, _ := postAsk(t, e, "Check the Mac's disk")
	at := hlcAt(fixedNow.Add(time.Minute))
	e.remote("dev-b", 1, at, "agent:"+remote, KindTakeRequested, takeRequestedRow{TaskID: tk.ID, By: remote, Op: TakeClaim, At: fixedNow})
	if a := askOf(t, e.c, tk.ID); a.Taker != remote {
		t.Fatalf("the owner didn't grant the remote claim: %+v", a)
	}
	offline := append(e.ros.Agents(), presence.AgentCard{Key: remote, Status: "idle", DeviceID: "dev-b", DeviceState: "offline"})
	e.clk.add(time.Hour)
	must[[]Notice](t)(e.c.SweepAsks(ctx, offline))
	if a := askOf(t, e.c, tk.ID); a.Taker != remote {
		t.Fatalf("an offline taker lost it: %+v", a)
	}
	must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if a := askOf(t, e.c, tk.ID); a.Taker != "" || a.Reason != TakeGone {
		t.Fatalf("a retired taker kept it: %+v", a)
	}
}

// Nobody takes it: unclaimed after AskUnclaimed, at once with nobody
// free; Wake them tells the room again, woken.
func TestUnclaimedAndAskAgain(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "Tidy the README")
	ns := must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(ns) != 0 || askOf(t, e.c, tk.ID).Unclaimed {
		t.Fatalf("unclaimed early: %+v", ns)
	}
	e.clk.add(AskUnclaimed)
	must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if a := askOf(t, e.c, tk.ID); !a.Unclaimed || a.Reason != TakeUnclaimed {
		t.Fatalf("not unclaimed: %+v", a)
	}
	n := len(e.rows(KindTakeState))
	must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(e.rows(KindTakeState)) != n {
		t.Fatal("unclaimed written twice")
	}
	if err := e.c.AskAgain(ctx, user, tk.ID); err != nil {
		t.Fatal(err)
	}
	ns = must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(ns) != 1 || !ns[0].Wake || len(ns[0].To) != 3 {
		t.Fatalf("asked again: %+v", ns)
	}
	if a := askOf(t, e.c, tk.ID); a.Unclaimed || a.Reason != TakeReopened {
		t.Fatalf("after asking again: %+v", a)
	}
	// A claim after unclaimed is still granted.
	if r := must[ClaimResult](t)(e.c.AskClaim(ctx, carol, tk.ID, time.Second)); !r.Mine {
		t.Fatalf("late claim %+v", r)
	}

	// Nobody free: unclaimed at once.
	for _, k := range []string{alice, bob, carol} {
		e.ros.set(k, "busy")
	}
	t2, n2 := postAsk(t, e, "Nobody around")
	if a := askOf(t, e.c, t2.ID); !a.Unclaimed || a.Told != 0 || len(n2.To) != 0 {
		t.Fatalf("told nobody: %+v %+v", a, n2)
	}
	// One becomes free: it hears, and the count grows.
	e.ros.set(bob, "idle")
	ns = must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{bob}) || askOf(t, e.c, t2.ID).Told != 1 {
		t.Fatalf("late arrival %+v", ns)
	}
}

// Each outcome's rows and fold.
func TestAskOutcomes(t *testing.T) {
	e := newEnv(t)
	other := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Ship docs", Objective: "old"}))

	card, _ := postAsk(t, e, "the login page is slow")
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, card.ID, time.Second))
	wantErr(t, errOnly(e.c.AskClose(ctx, bob, card.ID, AskOutcome{Kind: OutcomeNone, Note: "x"})), ErrNotAllowed)
	got := must[Task](t)(e.c.AskClose(ctx, alice, card.ID, AskOutcome{Kind: OutcomeCard, Note: "a real fix",
		Title: "Speed up the login page", DoneWhen: "under 1s", Steps: []Step{{Text: "profile"}, {Text: "fix"}}}))
	if got.Title != "Speed up the login page" || got.DoneCriteria != "under 1s" || got.Objective != "the login page is slow" ||
		got.State != Working || got.Holder != alice || len(got.Steps) != 2 || got.Ask.Open() || got.Ask.Outcome.Kind != OutcomeCard || got.Short != card.Short {
		t.Fatalf("card outcome %+v", got)
	}
	// From now on it is an ordinary card.
	must[Task](t)(e.c.SetTaskState(ctx, alice, card.ID, Completed, "done"))

	edit, _ := postAsk(t, e, "docs should mention Windows")
	must[ClaimResult](t)(e.c.AskClaim(ctx, bob, edit.ID, time.Second))
	must[Task](t)(e.c.AskClose(ctx, bob, edit.ID, AskOutcome{Kind: OutcomeEdit, Task: other.ID, Note: "added Windows", Objective: "new"}))
	ed, _ := e.c.GetTask(edit.ID)
	ot, _ := e.c.GetTask(other.ID)
	if ed.State != Completed || ed.Ask.Outcome.Task != other.ID || ot.Objective != "new" || ot.Title != "Ship docs" {
		t.Fatalf("edit outcome: ask %+v, card %+v", ed, ot)
	}
	if acts := e.c.Activity(other.ID); !strings.Contains(acts[0].What, "from the ask #"+edit.Short) || acts[0].By != bob {
		t.Fatalf("history line %+v", acts[0])
	}

	none, _ := postAsk(t, e, "never mind")
	must[Task](t)(e.c.SetTaskState(ctx, user, none.ID, Canceled, ""))
	nt, _ := e.c.GetTask(none.ID)
	if nt.State != Canceled || nt.Ask.Outcome == nil || nt.Ask.Outcome.Kind != OutcomeNone || nt.Ask.Outcome.Note != "closed by the person" {
		t.Fatalf("none outcome %+v", nt)
	}
	if e.c.StaleNotice("task:"+none.ID, alice) == "" {
		t.Fatal("a closed ask's notice isn't stale")
	}
	// Replay folds the same.
	c2 := e.reopen()
	for _, id := range []string{card.ID, edit.ID, none.ID, other.ID} {
		a, _ := e.c.GetTask(id)
		b, _ := c2.GetTask(id)
		if a.Title != b.Title || a.State != b.State || a.Objective != b.Objective || !reflect.DeepEqual(a.Ask, b.Ask) {
			t.Fatalf("replay differs for %s:\n%+v\n%+v", id, a, b)
		}
	}
}

// A take.state from any device but the ask's owner is ignored and
// counted; an older device's task.claimed on an open ask is ignored.
func TestTakeRowsFromTheWrongDeviceAndOldClaims(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "who wants this")
	at := hlcAt(fixedNow.Add(time.Minute))
	e.remote("dev-b", 1, at, "irori", KindTakeState, takeStateRow{TaskID: tk.ID, Taker: "dev-b/x", Reason: TakeTaken, At: fixedNow})
	if a := askOf(t, e.c, tk.ID); a.Taker != "" {
		t.Fatalf("a wrong-device take.state counted: %+v", a)
	}
	if e.c.IgnoredRows() != 1 {
		t.Fatalf("ignored %d", e.c.IgnoredRows())
	}
	e.remote("dev-b", 2, at+1, "agent:dev-b/x", KindTaskClaimed, taskClaimedRow{TaskID: tk.ID, By: "dev-b/x", At: fixedNow})
	e.remote("dev-b", 3, at+2, "agent:dev-b/x", KindTaskJoined, taskMemberRow{TaskID: tk.ID, Who: "dev-b/x", By: "dev-b/x", At: fixedNow})
	e.remote("dev-b", 4, at+3, "agent:dev-b/x", KindTaskState, taskStateRow{TaskID: tk.ID, State: Completed, By: "dev-b/x", At: fixedNow})
	got, _ := e.c.GetTask(tk.ID)
	if len(got.Holders) != 0 || got.State != Submitted || !got.Ask.Open() {
		t.Fatalf("an old device took the ask around the lock: %+v", got)
	}
	c2 := e.reopen()
	if got, _ := c2.GetTask(tk.ID); len(got.Holders) != 0 || c2.IgnoredRows() != 1 {
		t.Fatalf("replay: %+v, ignored %d", got, c2.IgnoredRows())
	}
}

// The take rows fold the same in any order, and a device that asked
// while apart from the owner hears who took it.
func TestTakeRowsTwoOrdersAndTheLoserHears(t *testing.T) {
	owner := newEnv(t)
	tk, _ := postAsk(t, owner, "partitioned ask")
	must[ClaimResult](t)(owner.c.AskClaim(ctx, alice, tk.ID, time.Second))
	rows, err := owner.st.List(ctx, store.Filter{Kinds: followKinds, Limit: store.MaxListLimit})
	if err != nil {
		t.Fatal(err)
	}
	var mine []int
	for i := range rows {
		mine = append(mine, i)
	}
	const far = "dev-b/far-session"
	persons := func(k string) bool { return k == "user:dev-a" || k == "user:dev-b" }
	fold := func(remoteFirst bool) (Ask, []Notice) {
		e := newEnvOn(t, "dev-b")
		e.owner = persons
		e.c = e.reopen()
		feed := func(from int) {
			for _, i := range mine[from:] {
				ev := rows[i]
				if err := e.st.Insert(ctx, ev); err != nil {
					t.Fatal(err)
				}
				e.c.impl.applyRemote(ctx, ev)
			}
		}
		claimFar := func() {
			// dev-b's own agent asks while dev-a's rows haven't arrived:
			// written as a request, waiting on the owner.
			x := e.c.impl
			x.mu.Lock()
			if err := x.write(ctx, far, KindTakeRequested, takeRequestedRow{TaskID: tk.ID, By: far, Op: TakeClaim, At: fixedNow}); err != nil {
				t.Fatal(err)
			}
			x.unlock()
		}
		if remoteFirst {
			feed(0)
			// Taken already: ask_claim refuses without writing.
			wantErr(t, errOnly(e.c.AskClaim(ctx, far, tk.ID, 10*time.Millisecond)), ErrHeld)
		} else {
			// The task arrives first, then dev-b's claim, then the take.
			ev := rows[mine[0]]
			if err := e.st.Insert(ctx, ev); err != nil {
				t.Fatal(err)
			}
			e.c.impl.applyRemote(ctx, ev)
			claimFar()
			if a := askOf(t, e.c, tk.ID); len(a.Waiting) != 1 {
				t.Fatalf("dev-b's claim isn't waiting: %+v", a)
			}
			feed(1)
		}
		ns := must[[]Notice](t)(e.c.SweepAsks(ctx, nil))
		return askOf(t, e.c, tk.ID), ns
	}
	a1, n1 := fold(true)
	a2, n2 := fold(false)
	want := askOf(t, owner.c, tk.ID)
	for _, a := range []Ask{a1, a2} {
		a.Waiting = nil
		if !reflect.DeepEqual(a, want) {
			t.Fatalf("folds differ:\n%+v\n%+v", a, want)
		}
	}
	if len(n1) != 0 {
		t.Fatalf("a refused ask_claim also got a notice: %+v", n1)
	}
	if len(n2) != 1 || n2[0].Kind != NoticeTaken || n2[0].To[0] != far || !strings.Contains(n2[0].Text, alice+" took") {
		t.Fatalf("the loser's notice: %+v", n2)
	}
}

// The person's comment on an ask nobody took goes to the agents told.
func TestPersonCommentOnAnOpenAskGoesToTheRoom(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "check the logs")
	_, ns, err := e.c.PersonComment(ctx, user, tk.ID, "also the cron ones", false, nil)
	if err != nil || len(ns) != 1 || len(ns[0].To) != 3 || !ns[0].Person || !strings.Contains(ns[0].Text, "ask_claim") {
		t.Fatalf("%+v %v", ns, err)
	}
	if ns[0].Kind != NoticeAskComment || ns[0].Ref != "task:"+tk.ID {
		t.Fatalf("a comment to the room can't go stale with the ask: %+v", ns[0])
	}
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, tk.ID, time.Second))
	if e.c.StaleNotice(ns[0].Ref, bob) == "" || e.c.StaleNotice(ns[0].Ref, alice) != "" {
		t.Fatal("the room's queued comment still reaches the others once taken")
	}
	_, ns, _ = e.c.PersonComment(ctx, user, tk.ID, "thanks", false, nil)
	if len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{alice}) {
		t.Fatalf("after the take %+v", ns)
	}
}

// A third device can get the taker's close before the owner's take (sync
// batches come by device, not cause). The close waits for the take instead
// of being lost, so the third device ends where the owner and the taker
// are, and replay agrees.
func TestATakersCloseBeforeTheTakeOnAThirdDevice(t *testing.T) {
	persons := func(k string) bool { return k == "user:dev-a" || k == "user:dev-b" || k == "user:dev-c" }
	owner := newEnv(t)
	owner.owner = persons
	owner.c = owner.reopen()
	tk, _ := postAsk(t, owner, "the deploy is flaky")
	const far = "dev-b/far-session"
	at := hlcAt(fixedNow.Add(time.Minute))
	owner.remote("dev-b", 1, at, "agent:"+far, KindTakeRequested, takeRequestedRow{TaskID: tk.ID, By: far, Op: TakeClaim, At: fixedNow})
	if a := askOf(t, owner.c, tk.ID); a.Taker != far {
		t.Fatalf("not granted: %+v", a)
	}
	// dev-b's ask_close as a card, after it saw the take.
	title := "Retry the deploy step"
	later := hlcAt(fixedNow.Add(2 * time.Minute))
	owner.remote("dev-b", 2, later, "agent:"+far, KindTaskEdited, taskEditedRow{TaskID: tk.ID, Title: &title, By: far, At: fixedNow})
	owner.remote("dev-b", 3, later+1, "agent:"+far, KindTaskState, taskStateRow{TaskID: tk.ID, State: Working, Note: "a card",
		By: far, At: fixedNow, Outcome: &Outcome{Kind: OutcomeCard, Note: "a card"}})
	owner.remote("dev-b", 4, later+2, "agent:"+far, KindTaskClaimed, taskClaimedRow{TaskID: tk.ID, By: far, At: fixedNow})
	want, _ := owner.c.GetTask(tk.ID)
	if want.Ask.Open() || want.Title != title || want.Holder != far || want.State != Working {
		t.Fatalf("the owner's fold %+v", want)
	}

	rows, err := owner.st.List(ctx, store.Filter{Kinds: followKinds, Limit: store.MaxListLimit})
	if err != nil {
		t.Fatal(err)
	}
	third := newEnvOn(t, "dev-c")
	third.owner = persons
	third.c = third.reopen()
	feed := func(keep func(store.Event) bool) {
		for _, ev := range rows {
			if !keep(ev) {
				continue
			}
			if err := third.st.Insert(ctx, ev); err != nil {
				t.Fatal(err)
			}
			third.c.impl.applyRemote(ctx, ev)
		}
	}
	// The ask's card, then all of dev-b's rows, then the owner's take.
	feed(func(ev store.Event) bool { return ev.DeviceID == "dev-a" && ev.Kind == KindTaskCreated })
	feed(func(ev store.Event) bool { return ev.DeviceID == "dev-b" })
	if got, _ := third.c.GetTask(tk.ID); !got.Ask.Open() || got.Title == title {
		t.Fatalf("the close folded before the take it rests on: %+v", got)
	}
	feed(func(ev store.Event) bool { return ev.DeviceID == "dev-a" && ev.Kind != KindTaskCreated })
	got, _ := third.c.GetTask(tk.ID)
	if got.Ask.Open() || got.Title != title || got.Holder != far || got.State != Working || !reflect.DeepEqual(got.Ask.Outcome, want.Ask.Outcome) {
		t.Fatalf("the third device: %+v", got)
	}
	if r, _ := third.reopen().GetTask(tk.ID); r.Title != got.Title || r.State != got.State || r.Holder != got.Holder || !reflect.DeepEqual(r.Ask.Outcome, got.Ask.Outcome) {
		t.Fatalf("replay %+v, live %+v", r, got)
	}
}

// task.edited needs the person or the ask's taker; ask_close's edit can't
// rewrite an open ask or a finished card.
func TestEditsNeedTheTaker(t *testing.T) {
	e := newEnv(t)
	other := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Ship docs", Objective: "old"}))
	tk, _ := postAsk(t, e, "docs should mention Windows")
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, tk.ID, time.Second))
	x := e.c.impl
	bad := "rewritten"
	x.mu.Lock()
	for _, row := range []taskEditedRow{
		{TaskID: other.ID, Objective: &bad, By: carol, From: tk.ID, At: fixedNow}, // carol didn't take it
		{TaskID: tk.ID, Title: &bad, By: carol, At: fixedNow},                     // nor is it hers to edit
		{TaskID: other.ID, Title: &bad, By: carol, At: fixedNow},                  // a plain card: not its creator or holder
	} {
		if err := x.write(ctx, carol, KindTaskEdited, row); err != nil {
			t.Fatal(err)
		}
	}
	x.unlock()
	for _, id := range []string{other.ID, tk.ID} {
		if got, _ := e.c.GetTask(id); got.Title == bad || got.Objective == bad {
			t.Fatalf("a stranger's edit folded: %+v", got)
		}
	}
	if got, _ := e.reopen().GetTask(other.ID); got.Objective == bad || got.Title == bad {
		t.Fatalf("replayed a stranger's edit: %+v", got)
	}
	open, _ := postAsk(t, e, "another ask")
	wantErr(t, errOnly(e.c.AskClose(ctx, alice, tk.ID, AskOutcome{Kind: OutcomeEdit, Task: open.ID, Note: "x", Title: bad})), ErrNotAllowed)
	done := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Old work"}))
	must[Task](t)(e.c.SetTaskState(ctx, user, done.ID, Completed, ""))
	wantErr(t, errOnly(e.c.AskClose(ctx, alice, tk.ID, AskOutcome{Kind: OutcomeEdit, Task: done.ID, Note: "x", Title: bad})), ErrBadState)
	// The taker's edit of a card in play still goes through.
	must[Task](t)(e.c.AskClose(ctx, alice, tk.ID, AskOutcome{Kind: OutcomeEdit, Task: other.ID, Note: "added Windows", Objective: "new"}))
	if got, _ := e.c.GetTask(other.ID); got.Objective != "new" {
		t.Fatalf("the taker's edit: %+v", got)
	}
}

// Wake reaches only who the person meant: late arrivals inside the post's
// window, the person's Wake them. A re-announcement after a taker left, or
// a late tell once the window passed, waits for the agent's next turn.
func TestLateTellsWakeOnlyInThePersonsWindow(t *testing.T) {
	e := newEnv(t)
	e.ros.set(bob, "busy")
	e.ros.set(carol, "busy")
	tk, n, err := e.c.PostAsk(ctx, user, NewAsk{Text: "wake for this", Wake: true}, e.ros.Agents())
	if err != nil || !n.Wake || !reflect.DeepEqual(n.To, []string{alice}) {
		t.Fatalf("post %+v %v", n, err)
	}
	e.ros.set(bob, "idle")
	ns := must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(ns) != 1 || !ns[0].Wake || !reflect.DeepEqual(ns[0].To, []string{bob}) {
		t.Fatalf("a late arrival in the window: %+v", ns)
	}
	e.clk.add(AskUnclaimed + time.Second)
	e.ros.set(carol, "idle")
	ns = must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(ns) != 1 || ns[0].Wake || !reflect.DeepEqual(ns[0].To, []string{carol}) {
		t.Fatalf("a late arrival after the window: %+v", ns)
	}
	// Taken, then given back: the room hears again at its next turn.
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, tk.ID, time.Second))
	must[Task](t)(e.c.Leave(ctx, alice, tk.ID, alice))
	ns = must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(ns) != 1 || ns[0].Wake || !strings.Contains(ns[0].Text, "gave it back") {
		t.Fatalf("re-announced %+v", ns)
	}
}

// A sweep whose write fails keeps the ask's "tell the room again" for the
// next sweep.
func TestAFailedSweepKeepsTheRetell(t *testing.T) {
	e := newEnv(t)
	tk, _ := postAsk(t, e, "look at the logs")
	must[ClaimResult](t)(e.c.AskClaim(ctx, alice, tk.ID, time.Second))
	must[Task](t)(e.c.Leave(ctx, alice, tk.ID, alice))
	dead, cancel := context.WithCancel(ctx)
	cancel()
	if _, err := e.c.SweepAsks(dead, e.ros.Agents()); err == nil {
		t.Fatal("a sweep with a dead context wrote")
	}
	ns := must[[]Notice](t)(e.c.SweepAsks(ctx, e.ros.Agents()))
	if len(ns) != 1 || !reflect.DeepEqual(ns[0].To, []string{bob, carol}) {
		t.Fatalf("after the failed sweep: %+v", ns)
	}
}
