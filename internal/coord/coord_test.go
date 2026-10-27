package coord

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"reflect"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

var fixedNow = time.Date(2026, 10, 26, 12, 0, 0, 0, time.UTC)

const (
	alice = "dev-a/alice-session"
	bob   = "dev-a/bob-session"
	carol = "dev-a/carol-session"
	user  = "user:dev-a"
	repoA = "remote:github.com/a/b"
	repoB = "root:4b825dc642cb6eb9a060e54bf8d69288fbee4904"
)

type clock struct {
	mu sync.Mutex
	t  time.Time
}

func (c *clock) now() time.Time { c.mu.Lock(); defer c.mu.Unlock(); return c.t }
func (c *clock) add(d time.Duration) {
	c.mu.Lock()
	c.t = c.t.Add(d)
	c.mu.Unlock()
}

type roster struct {
	mu    sync.Mutex
	cards map[string]string // key -> status
}

func newRoster(keys ...string) *roster {
	r := &roster{cards: map[string]string{}}
	for _, k := range keys {
		r.cards[k] = "idle"
	}
	return r
}

func (r *roster) set(key, status string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if status == "" {
		delete(r.cards, key)
	} else {
		r.cards[key] = status
	}
}

func (r *roster) Agents() []presence.AgentCard {
	r.mu.Lock()
	defer r.mu.Unlock()
	var out []presence.AgentCard
	for k, s := range r.cards {
		out = append(out, presence.AgentCard{Key: k, Status: s})
	}
	return out
}

type env struct {
	t   *testing.T
	st  *store.Store
	clk *clock
	ros *roster
	c   *Coord
}

func newEnv(t *testing.T) *env {
	t.Helper()
	clk := &clock{t: fixedNow}
	st, err := store.Open(context.Background(), filepath.Join(t.TempDir(), "irori.db"), "dev-a", hlc.New(clk.now))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	e := &env{t: t, st: st, clk: clk, ros: newRoster(alice, bob, carol)}
	e.c = e.reopen()
	return e
}

func (e *env) reopen() *Coord {
	e.t.Helper()
	c, err := New(context.Background(), e.st, e.ros, Options{Now: e.clk.now})
	if err != nil {
		e.t.Fatal(err)
	}
	return c
}

func (e *env) rows(prefix string) []store.Event {
	e.t.Helper()
	evs, err := e.st.List(context.Background(), store.Filter{Kinds: []string{prefix}, Limit: store.MaxListLimit})
	if err != nil {
		e.t.Fatal(err)
	}
	return evs
}

func must[T any](t *testing.T) func(T, error) T {
	return func(v T, err error) T {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
		return v
	}
}

func wantErr(t *testing.T, err, target error) {
	t.Helper()
	if !errors.Is(err, target) {
		t.Fatalf("err = %v, want %v", err, target)
	}
}

var ctx = context.Background()

// ---- tasks ----

func TestCreateTask(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{
		Title: "  Fix login ", Objective: "make it work", RepoKey: repoA,
		Scope: []string{"./src/auth/", " ", "docs/*.md"},
	}))
	if task.ID == "" || task.Title != "Fix login" || task.State != Submitted || task.CreatedBy != alice ||
		task.Holder != "" || !task.CreatedAt.Equal(fixedNow) {
		t.Fatalf("task = %+v", task)
	}
	if !reflect.DeepEqual(task.Scope, []string{"src/auth/**", "docs/*.md"}) {
		t.Fatalf("scope = %v", task.Scope)
	}
	if got, ok := e.c.GetTask(task.ID); !ok || !reflect.DeepEqual(got, task) {
		t.Fatalf("GetTask = %+v %v", got, ok)
	}
	if rows := e.rows(KindTaskCreated); len(rows) != 1 || rows[0].Actor != "agent:"+alice {
		t.Fatalf("rows = %+v", rows)
	}

	_, err := e.c.CreateTask(ctx, alice, NewTask{Title: "  "})
	wantErr(t, err, ErrInvalid)
	_, err = e.c.CreateTask(ctx, "", NewTask{Title: "x"})
	wantErr(t, err, ErrInvalid)
	_, err = e.c.CreateTask(ctx, alice, NewTask{Title: "x", RepoKey: "github.com/a/b"})
	wantErr(t, err, ErrBadResource)
	_, err = e.c.CreateTask(ctx, alice, NewTask{Title: "x", Scope: []string{`src\a`}})
	wantErr(t, err, ErrBadResource)
	if _, ok := e.c.GetTask("nope"); ok {
		t.Fatal("found a missing task")
	}
}

func TestClaimSingleWriter(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))

	got := must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	if got.Holder != bob || got.State != Working {
		t.Fatalf("claimed = %+v", got)
	}
	// Same holder again: no-op, no row.
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	if n := len(e.rows(KindTaskClaimed)); n != 1 {
		t.Fatalf("claim rows = %d", n)
	}
	// Another live agent can't.
	_, err := e.c.ClaimTask(ctx, carol, task.ID)
	wantErr(t, err, ErrHeld)
	// Not even the creator or a person by claiming (they assign instead).
	_, err = e.c.ClaimTask(ctx, alice, task.ID)
	wantErr(t, err, ErrHeld)
	_, err = e.c.ClaimTask(ctx, bob, "missing")
	wantErr(t, err, ErrNotFound)
}

func TestClaimFromGoneHolder(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))

	e.ros.set(bob, "") // bob leaves the roster
	_, err := e.c.ClaimTask(ctx, carol, task.ID)
	wantErr(t, err, ErrHeld) // first observation starts the clock
	e.clk.add(GoneGrace - time.Second)
	_, err = e.c.ClaimTask(ctx, carol, task.ID)
	wantErr(t, err, ErrHeld)

	// Seen again resets the clock.
	e.ros.set(bob, "busy")
	_, err = e.c.ClaimTask(ctx, carol, task.ID)
	wantErr(t, err, ErrHeld)
	e.ros.set(bob, "gone") // a "gone" card counts as absent
	e.clk.add(2 * time.Second)
	_, err = e.c.ClaimTask(ctx, carol, task.ID)
	wantErr(t, err, ErrHeld)
	e.clk.add(GoneGrace)
	got := must[Task](t)(e.c.ClaimTask(ctx, carol, task.ID))
	if got.Holder != carol {
		t.Fatalf("holder = %s", got.Holder)
	}
}

func TestPersonHolderNeverGone(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, user, NewTask{Title: "t"}))
	must[Task](t)(e.c.ClaimTask(ctx, user, task.ID))
	must[Task](t)(e.c.ClaimTask(ctx, user, task.ID))
	_, err := e.c.ClaimTask(ctx, bob, task.ID)
	wantErr(t, err, ErrHeld)
	e.clk.add(time.Hour)
	_, err = e.c.ClaimTask(ctx, bob, task.ID)
	wantErr(t, err, ErrHeld)
}

func TestConcurrentClaimsOneWins(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "race"}))
	const n = 16
	var wg sync.WaitGroup
	var mu sync.Mutex
	wins, held := 0, 0
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			who := []string{alice, bob, carol, "dev-a/x"}[i%4] + string(rune('a'+i))
			_, err := e.c.ClaimTask(ctx, who, task.ID)
			mu.Lock()
			defer mu.Unlock()
			switch {
			case err == nil:
				wins++
			case errors.Is(err, ErrHeld):
				held++
			default:
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	if wins != 1 || held != n-1 {
		t.Fatalf("wins=%d held=%d", wins, held)
	}
	if rows := e.rows(KindTaskClaimed); len(rows) != 1 {
		t.Fatalf("claim rows = %d", len(rows))
	}
}

func TestAssign(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))

	_, err := e.c.AssignTask(ctx, bob, task.ID, bob) // not creator/holder/person
	wantErr(t, err, ErrNotAllowed)
	got := must[Task](t)(e.c.AssignTask(ctx, alice, task.ID, bob)) // creator
	if got.Holder != bob || got.State != Submitted {
		t.Fatalf("assigned = %+v", got)
	}
	got = must[Task](t)(e.c.AssignTask(ctx, bob, task.ID, carol)) // holder hands off
	if got.Holder != carol {
		t.Fatalf("handoff = %+v", got)
	}
	_, err = e.c.AssignTask(ctx, bob, task.ID, bob) // bob no longer holds it
	wantErr(t, err, ErrNotAllowed)
	got = must[Task](t)(e.c.AssignTask(ctx, user, task.ID, bob)) // a person
	if got.Holder != bob {
		t.Fatalf("person assign = %+v", got)
	}
	must[Task](t)(e.c.AssignTask(ctx, user, task.ID, bob)) // same holder: no row
	if n := len(e.rows(KindTaskAssigned)); n != 3 {
		t.Fatalf("assign rows = %d", n)
	}
	_, err = e.c.AssignTask(ctx, user, task.ID, " ")
	wantErr(t, err, ErrInvalid)
	_, err = e.c.AssignTask(ctx, user, "missing", bob)
	wantErr(t, err, ErrNotFound)
}

func TestSetTaskState(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))

	_, err := e.c.SetTaskState(ctx, carol, task.ID, InputRequired, "")
	wantErr(t, err, ErrNotAllowed)
	_, err = e.c.SetTaskState(ctx, bob, task.ID, "done", "")
	wantErr(t, err, ErrBadState)

	got := must[Task](t)(e.c.SetTaskState(ctx, bob, task.ID, InputRequired, "which db?"))
	if got.State != InputRequired || got.Note != "which db?" {
		t.Fatalf("state = %+v", got)
	}
	must[Task](t)(e.c.SetTaskState(ctx, bob, task.ID, InputRequired, "which db?")) // no-op
	must[Task](t)(e.c.SetTaskState(ctx, alice, task.ID, Working, "postgres"))      // creator
	if n := len(e.rows(KindTaskState)); n != 2 {
		t.Fatalf("state rows = %d", n)
	}
	e.clk.add(time.Minute)
	got = must[Task](t)(e.c.SetTaskState(ctx, bob, task.ID, Completed, "shipped"))
	if got.State != Completed || got.Holder != bob || !got.UpdatedAt.Equal(fixedNow.Add(time.Minute)) {
		t.Fatalf("completed = %+v", got)
	}

	// Terminal closes it.
	_, err = e.c.SetTaskState(ctx, user, task.ID, Working, "")
	wantErr(t, err, ErrBadState)
	_, err = e.c.ClaimTask(ctx, carol, task.ID)
	wantErr(t, err, ErrBadState)
	_, err = e.c.AssignTask(ctx, user, task.ID, carol)
	wantErr(t, err, ErrBadState)

	// A person may cancel anything open.
	t2 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t2"}))
	got = must[Task](t)(e.c.SetTaskState(ctx, user, t2.ID, Canceled, ""))
	if got.State != Canceled {
		t.Fatalf("canceled = %+v", got)
	}
}

func TestListTasks(t *testing.T) {
	e := newEnv(t)
	t1 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "1", RepoKey: repoA}))
	e.clk.add(time.Second)
	t2 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "2", RepoKey: repoB}))
	e.clk.add(time.Second)
	t3 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "3", RepoKey: repoA}))
	must[Task](t)(e.c.ClaimTask(ctx, bob, t2.ID))
	must[Task](t)(e.c.ClaimTask(ctx, bob, t3.ID))
	must[Task](t)(e.c.SetTaskState(ctx, bob, t3.ID, Failed, ""))

	ids := func(ts []Task) []string {
		out := []string{}
		for _, x := range ts {
			out = append(out, x.ID)
		}
		return out
	}
	for _, c := range []struct {
		f    TaskFilter
		want []string
	}{
		{TaskFilter{}, []string{t1.ID, t2.ID, t3.ID}},
		{TaskFilter{RepoKey: repoA}, []string{t1.ID, t3.ID}},
		{TaskFilter{Holder: bob}, []string{t2.ID, t3.ID}},
		{TaskFilter{Open: true}, []string{t1.ID, t2.ID}},
		{TaskFilter{Open: true, Holder: bob}, []string{t2.ID}},
		{TaskFilter{RepoKey: "root:none"}, []string{}},
	} {
		if got := ids(e.c.ListTasks(c.f)); !reflect.DeepEqual(got, c.want) {
			t.Errorf("ListTasks(%+v) = %v, want %v", c.f, got, c.want)
		}
	}
	// Returned scopes are copies.
	t4 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "4", Scope: []string{"a"}}))
	t4.Scope[0] = "mutated"
	if got, _ := e.c.GetTask(t4.ID); got.Scope[0] != "a" {
		t.Fatal("caller mutated the fold")
	}
}

// ---- leases ----

func TestLeaseAcquireRenewExpire(t *testing.T) {
	e := newEnv(t)
	r := "path:" + repoA + ":src/**"
	l := must[Lease](t)(e.c.Acquire(ctx, alice, r, 0, false))
	if l.Resource != r || l.Holder != alice || !l.Until.Equal(fixedNow.Add(DefaultTTL)) || !l.AcquiredAt.Equal(fixedNow) {
		t.Fatalf("lease = %+v", l)
	}
	// Renew by the holder, possibly shorter, keeps AcquiredAt.
	e.clk.add(time.Minute)
	l = must[Lease](t)(e.c.Acquire(ctx, alice, r, 5*time.Minute, false))
	if !l.Until.Equal(fixedNow.Add(6*time.Minute)) || !l.AcquiredAt.Equal(fixedNow) {
		t.Fatalf("renewed = %+v", l)
	}
	// TTL capped.
	l = must[Lease](t)(e.c.Acquire(ctx, alice, r, 100*time.Hour, false))
	if !l.Until.Equal(fixedNow.Add(time.Minute + MaxTTL)) {
		t.Fatalf("capped = %+v", l)
	}
	if got := e.c.Leases(); len(got) != 1 || got[0] != l {
		t.Fatalf("Leases = %+v", got)
	}
	// Expiry is computed on read; no row is written.
	e.clk.add(MaxTTL)
	if got := e.c.Leases(); len(got) != 0 {
		t.Fatalf("expired still listed: %+v", got)
	}
	if n := len(e.rows("lease.released")); n != 0 {
		t.Fatalf("release rows = %d", n)
	}
	// Someone else may take an expired one without steal.
	l = must[Lease](t)(e.c.Acquire(ctx, bob, r, time.Minute, false))
	if l.Holder != bob || l.StolenFrom != "" {
		t.Fatalf("after expiry = %+v", l)
	}
	_, err := e.c.Acquire(ctx, bob, "nope:x", 0, false)
	wantErr(t, err, ErrBadResource)
	_, err = e.c.Acquire(ctx, "", r, 0, false)
	wantErr(t, err, ErrInvalid)
}

func TestLeaseConflicts(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, alice, "path:"+repoA+":src/**", 0, false))
	must[Lease](t)(e.c.Acquire(ctx, alice, "gpu/qwen", 0, false))
	must[Lease](t)(e.c.Acquire(ctx, alice, "port:8799", 0, false))

	for _, c := range []struct {
		res  string
		held bool
	}{
		{"path:" + repoA + ":src/auth/login.go", true},
		{"path:" + repoA + ":./src/", true},
		{"path:" + repoA + ":**/*.go", true},
		{"path:" + repoA + ":docs/**", false},
		{"path:" + repoB + ":src/**", false}, // other repo
		{"gpu/qwen", true},
		{"gpu/llama", false},
		{"gpu", false}, // named resources are exact
		{"port:8799", true},
		{"port:8800", false},
	} {
		_, err := e.c.Acquire(ctx, bob, c.res, 0, false)
		if c.held {
			if !errors.Is(err, ErrHeld) {
				t.Errorf("%s: err = %v, want ErrHeld", c.res, err)
			}
		} else if err != nil {
			t.Errorf("%s: %v", c.res, err)
		}
	}
	// The same holder overlapping itself is fine.
	must[Lease](t)(e.c.Acquire(ctx, alice, "path:"+repoA+":src/a.go", 0, false))
}

func TestLeaseSteal(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, bob, "path:"+repoA+":src/a.go", 0, false))
	must[Lease](t)(e.c.Acquire(ctx, carol, "path:"+repoA+":src/b/**", 0, false))
	must[Lease](t)(e.c.Acquire(ctx, carol, "path:"+repoA+":docs/x.md", 0, false))

	l := must[Lease](t)(e.c.Acquire(ctx, alice, "path:"+repoA+":src/**", 0, true))
	if l.StolenFrom != bob+","+carol {
		t.Fatalf("stolenFrom = %q", l.StolenFrom)
	}
	got := e.c.Leases()
	if len(got) != 2 || got[0].Resource != "path:"+repoA+":docs/x.md" || got[1].Holder != alice {
		t.Fatalf("leases = %+v", got)
	}
	rel := e.rows(KindLeaseRel)
	if len(rel) != 2 {
		t.Fatalf("release rows = %d", len(rel))
	}
	var p leaseReleasedRow
	_ = json.Unmarshal(rel[0].Payload, &p)
	if p.Reason != ReasonStolen || p.By != alice || p.Holder != bob {
		t.Fatalf("release row = %+v", p)
	}
	// Exact-resource steal replaces it.
	l = must[Lease](t)(e.c.Acquire(ctx, bob, "path:"+repoA+":src/**", 0, true))
	if l.StolenFrom != alice || !l.AcquiredAt.Equal(fixedNow) {
		t.Fatalf("exact steal = %+v", l)
	}
	// A renewal keeps the stolenFrom record.
	l = must[Lease](t)(e.c.Acquire(ctx, bob, "path:"+repoA+":src/**", 0, false))
	if l.StolenFrom != alice {
		t.Fatalf("renewal lost stolenFrom: %+v", l)
	}
}

func TestRelease(t *testing.T) {
	e := newEnv(t)
	r := "path:" + repoB + ":a/**"
	must[Lease](t)(e.c.Acquire(ctx, alice, r, 0, false))
	wantErr(t, e.c.Release(ctx, bob, r), ErrHeld)
	wantErr(t, e.c.Release(ctx, alice, "gpu/none"), ErrNotFound)
	wantErr(t, e.c.Release(ctx, alice, "bad resource"), ErrBadResource)
	if err := e.c.Release(ctx, alice, "path:"+repoB+":./a/"); err != nil { // normalized form
		t.Fatal(err)
	}
	if len(e.c.Leases()) != 0 {
		t.Fatal("still held")
	}
	wantErr(t, e.c.Release(ctx, alice, r), ErrNotFound)

	// A person may release anyone's.
	must[Lease](t)(e.c.Acquire(ctx, bob, "gpu/qwen", 0, false))
	if err := e.c.Release(ctx, user, "gpu/qwen"); err != nil {
		t.Fatal(err)
	}
	rel := e.rows(KindLeaseRel)
	var p leaseReleasedRow
	_ = json.Unmarshal(rel[len(rel)-1].Payload, &p)
	if p.Holder != bob || p.By != user || p.Reason != ReasonReleased || rel[len(rel)-1].Actor != "user" {
		t.Fatalf("row = %+v actor %s", p, rel[len(rel)-1].Actor)
	}
	// Expired: nothing to release.
	must[Lease](t)(e.c.Acquire(ctx, bob, "gpu/qwen", time.Minute, false))
	e.clk.add(time.Minute)
	wantErr(t, e.c.Release(ctx, bob, "gpu/qwen"), ErrNotFound)
}

func TestSweepReleasesGoneHolders(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, alice, "gpu/qwen", MaxTTL, false))
	must[Lease](t)(e.c.Acquire(ctx, bob, "port:8799", MaxTTL, false))
	must[Lease](t)(e.c.Acquire(ctx, user, "port:9000", MaxTTL, false))
	sweep := func() {
		t.Helper()
		if err := e.c.Sweep(ctx); err != nil {
			t.Fatal(err)
		}
	}
	holders := func() []string {
		out := []string{}
		for _, l := range e.c.Leases() {
			out = append(out, l.Holder)
		}
		return out
	}

	e.ros.set(alice, "")
	e.ros.set(bob, "gone")
	sweep() // first absent
	e.clk.add(GoneGrace / 2)
	e.ros.set(bob, "idle") // bob is back: resets
	sweep()
	e.ros.set(bob, "")
	sweep() // bob absent again from here
	e.clk.add(GoneGrace / 2)
	sweep() // alice's grace is up
	if got := holders(); !reflect.DeepEqual(got, []string{bob, user}) {
		t.Fatalf("after alice gone: %v", got)
	}
	e.clk.add(GoneGrace / 2)
	sweep()
	if got := holders(); !reflect.DeepEqual(got, []string{user}) {
		t.Fatalf("after bob gone: %v", got)
	}
	rel := e.rows(KindLeaseRel)
	if len(rel) != 2 {
		t.Fatalf("release rows = %d", len(rel))
	}
	for _, ev := range rel {
		var p leaseReleasedRow
		_ = json.Unmarshal(ev.Payload, &p)
		if p.Reason != ReasonHolderGone || p.By != "irori" || ev.Actor != "irori" {
			t.Fatalf("row = %+v", p)
		}
	}
	e.clk.add(time.Hour)
	sweep() // a person is never gone
	if got := holders(); !reflect.DeepEqual(got, []string{user}) {
		t.Fatalf("person swept: %v", got)
	}
}

func TestNilRosterNobodyGone(t *testing.T) {
	e := newEnv(t)
	c, err := New(ctx, e.st, nil, Options{Now: e.clk.now, GoneGrace: time.Second})
	if err != nil {
		t.Fatal(err)
	}
	must[Lease](t)(c.Acquire(ctx, alice, "gpu/qwen", 0, false))
	_ = c.Sweep(ctx)
	e.clk.add(time.Minute)
	_ = c.Sweep(ctx)
	if len(c.Leases()) != 1 {
		t.Fatal("swept with a nil roster")
	}
}

func TestGoneGraceOverride(t *testing.T) {
	e := newEnv(t)
	c, err := New(ctx, e.st, e.ros, Options{Now: e.clk.now, GoneGrace: 10 * time.Second})
	if err != nil {
		t.Fatal(err)
	}
	must[Lease](t)(c.Acquire(ctx, alice, "gpu/qwen", 0, false))
	e.ros.set(alice, "")
	_ = c.Sweep(ctx)
	e.clk.add(10 * time.Second)
	_ = c.Sweep(ctx)
	if len(c.Leases()) != 0 {
		t.Fatal("override ignored")
	}
}

// ---- replay ----

func TestReplayEquality(t *testing.T) {
	e := newEnv(t)
	c := e.c
	t1 := must[Task](t)(c.CreateTask(ctx, alice, NewTask{Title: "a", Objective: "o", DoneCriteria: "d", RepoKey: repoA, Scope: []string{"src/"}}))
	e.clk.add(time.Second)
	t2 := must[Task](t)(c.CreateTask(ctx, bob, NewTask{Title: "b"}))
	must[Task](t)(c.ClaimTask(ctx, bob, t1.ID))
	must[Task](t)(c.AssignTask(ctx, bob, t2.ID, carol))
	must[Task](t)(c.SetTaskState(ctx, carol, t2.ID, InputRequired, "?"))
	must[Task](t)(c.SetTaskState(ctx, bob, t1.ID, Completed, "done"))
	must[Lease](t)(c.Acquire(ctx, alice, "path:"+repoA+":src/**", 0, false))
	must[Lease](t)(c.Acquire(ctx, bob, "path:"+repoA+":src/x.go", 0, true))
	must[Lease](t)(c.Acquire(ctx, carol, "gpu/qwen", time.Hour, false))
	must[Lease](t)(c.Acquire(ctx, carol, "port:8799", time.Minute, false))
	if err := c.Release(ctx, user, "gpu/qwen"); err != nil {
		t.Fatal(err)
	}
	e.ros.set(carol, "")
	_ = c.Sweep(ctx)
	e.clk.add(GoneGrace)
	_ = c.Sweep(ctx)
	e.clk.add(time.Minute)
	must[Lease](t)(c.Acquire(ctx, alice, "port:1", 0, false))

	check := func(c2 *Coord) {
		t.Helper()
		if a, b := c.ListTasks(TaskFilter{}), c2.ListTasks(TaskFilter{}); !reflect.DeepEqual(a, b) {
			t.Fatalf("tasks differ:\n%+v\n%+v", a, b)
		}
		if a, b := c.Leases(), c2.Leases(); !reflect.DeepEqual(a, b) {
			t.Fatalf("leases differ:\n%+v\n%+v", a, b)
		}
		if !reflect.DeepEqual(c.impl.held, c2.impl.held) {
			t.Fatalf("raw lease folds differ")
		}
	}
	check(e.reopen())
	if len(c.Leases()) != 2 {
		t.Fatalf("leases = %+v", c.Leases())
	}
}

func TestReplayPagesPastListLimit(t *testing.T) {
	e := newEnv(t)
	n := store.MaxListLimit*2 + 7
	for i := range n {
		if _, err := e.c.CreateTask(ctx, alice, NewTask{Title: "t"}); err != nil {
			t.Fatal(err)
		}
		if i%3 == 0 { // noise rows of other kinds in between
			if _, err := e.st.Append(ctx, "irori", store.KindAgentStatus, map[string]int{"i": i}); err != nil {
				t.Fatal(err)
			}
		}
	}
	c2 := e.reopen()
	if got := len(c2.ListTasks(TaskFilter{})); got != n {
		t.Fatalf("replayed %d tasks, want %d", got, n)
	}
	if !reflect.DeepEqual(e.c.ListTasks(TaskFilter{}), c2.ListTasks(TaskFilter{})) {
		t.Fatal("replay differs")
	}
}

func TestReplayTolerates(t *testing.T) {
	e := newEnv(t)
	// Rows from a future version or garbage don't break replay.
	for _, row := range []struct {
		kind    string
		payload any
	}{
		{KindTaskCreated, map[string]any{"taskId": "x1", "title": "old", "by": alice, "extra": 1}},
		{KindTaskClaimed, map[string]any{"taskId": "missing", "by": bob}},
		{KindTaskState, map[string]any{"taskId": "x1", "state": "bogus"}},
		{KindLeaseRel, map[string]any{"resource": "gpu/none", "holder": bob}},
		{"task.future", map[string]any{"a": 1}},
		{KindTaskCreated, []int{1, 2}},
	} {
		if _, err := e.st.Append(ctx, "irori", row.kind, row.payload); err != nil {
			t.Fatal(err)
		}
	}
	c2 := e.reopen()
	got, ok := c2.GetTask("x1")
	if !ok || got.State != Submitted || got.CreatedAt.IsZero() {
		t.Fatalf("x1 = %+v %v", got, ok)
	}
}
