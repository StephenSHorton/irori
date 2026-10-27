package coord

import (
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/store"
)

// Regression tests for the wave-1 coord review.

// Repo keys from internal/repo can hold ':' (IPv6 hosts, a decoded %3A) and
// spaces (a decoded %20). They must work as task repoKeys and round-trip
// through path resources without landing in another repo.
func TestRepoKeysWithColonsAndSpaces(t *testing.T) {
	keys := []string{
		"remote:::1/a/b",
		"remote:fe80::1/a/b",
		"remote:host.com/a:b/c",
		"remote:host.com/my org/repo",
		repoA,
		repoB,
	}
	// The live check: every key the repo package makes for these remotes
	// (empty while internal/repo is the skeleton) round-trips too.
	for _, u := range []string{
		"https://[::1]/a/b",
		"ssh://git@[fe80::1]:22/a/b.git",
		"git@host.com:a:b/c.git",
		"https://host.com/a%3Ab/c",
		"https://host.com/my%20org/repo",
		"git@github.com:Foo/Bar.git",
	} {
		if n := repo.NormalizeRemote(u); n != "" {
			keys = append(keys, "remote:"+n)
		}
	}
	e := newEnv(t)
	for _, k := range keys {
		if _, err := e.c.CreateTask(ctx, alice, NewTask{Title: "t", RepoKey: k}); err != nil {
			t.Errorf("CreateTask(%q): %v", k, err)
		}
		r, err := ParseResource("path:" + k + ":src/**")
		if err != nil || r.RepoKey != k || r.Glob != "src/**" || r.Name != "path:"+k+":src/**" {
			t.Errorf("ParseResource(path:%s:src/**) = %+v, %v", k, r, err)
		}
	}
	// The misparse from the review: this is repo host.com/a:b/c, not host.com/a.
	r, err := ParseResource("path:remote:host.com/a:b/c:src/**")
	if err != nil || r.RepoKey != "remote:host.com/a:b/c" || r.Glob != "src/**" {
		t.Fatalf("parse = %+v, %v", r, err)
	}
	must[Lease](t)(e.c.Acquire(ctx, alice, "path:remote:host.com/a:b/c:src/**", 0, false))
	if _, err := e.c.Acquire(ctx, bob, "path:remote:host.com/a:src/**", 0, false); err != nil {
		t.Fatalf("lease in another repo conflicted: %v", err)
	}
	_, err = e.c.Acquire(ctx, bob, "path:remote:host.com/a:b/c:src/x.go", 0, false)
	wantErr(t, err, ErrHeld)
}

func TestGlobRejectsControlCharacters(t *testing.T) {
	for _, g := range []string{"a\nb", "src/\x00x", "a\tb", "x\u0085y"} {
		if got, err := NormalizeGlob(g); !errors.Is(err, ErrBadResource) {
			t.Errorf("NormalizeGlob(%q) = %q, %v; want ErrBadResource", g, got, err)
		}
	}
}

// A steal is one row, so a failure cannot leave the victims released and
// the stealer empty-handed. The acquired row alone drops the stolen leases.
func TestStealIsOneAcquiredRow(t *testing.T) {
	e := newEnv(t)
	must[Lease](t)(e.c.Acquire(ctx, alice, "path:"+repoA+":src/**", 0, false))
	before := len(e.rows("lease."))
	must[Lease](t)(e.c.Acquire(ctx, bob, "path:"+repoA+":src/x.go", 0, true))
	if n := len(e.rows("lease.")) - before; n != 1 {
		t.Fatalf("steal wrote %d rows, want 1", n)
	}
	// A bare acquired row naming a stolen lease folds the same way on replay.
	if _, err := e.st.Append(ctx, "agent:"+carol, KindLeaseAcq, leaseAcquiredRow{
		Lease:  Lease{Resource: "path:" + repoA + ":src/x.go/**", Holder: carol, Until: fixedNow.Add(time.Hour), AcquiredAt: fixedNow, StolenFrom: bob},
		Stolen: []stolenLease{{Resource: "path:" + repoA + ":src/x.go", Holder: bob}},
	}); err != nil {
		t.Fatal(err)
	}
	got := e.reopen().Leases()
	if len(got) != 1 || got[0].Holder != carol {
		t.Fatalf("leases after replay = %+v", got)
	}
}

// cardRoster serves whole cards, so a test can set UpdatedAt.
type cardRoster struct {
	mu    sync.Mutex
	cards []presence.AgentCard
}

func (r *cardRoster) Agents() []presence.AgentCard {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]presence.AgentCard(nil), r.cards...)
}

func (r *cardRoster) set(cards ...presence.AgentCard) {
	r.mu.Lock()
	r.cards = cards
	r.mu.Unlock()
}

// A gone card's UpdatedAt starts the grace clock, so a holder gone for an
// hour does not block a claim for another full grace when no Sweep ran.
func TestGoneCardStartsGraceClock(t *testing.T) {
	e := newEnv(t)
	ros := &cardRoster{}
	ros.set(presence.AgentCard{Key: alice, Status: "idle"}, presence.AgentCard{Key: bob, Status: "idle"})
	c, err := New(ctx, e.st, ros, Options{Now: e.clk.now})
	if err != nil {
		t.Fatal(err)
	}
	task := must[Task](t)(c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	must[Task](t)(c.ClaimTask(ctx, alice, task.ID))
	ros.set(presence.AgentCard{Key: alice, Status: "gone", UpdatedAt: e.clk.now()}, presence.AgentCard{Key: bob, Status: "idle"})
	e.clk.add(time.Hour)
	got, err := c.ClaimTask(ctx, bob, task.ID)
	if err != nil || got.Holder != bob {
		t.Fatalf("claim from a holder gone an hour = %+v, %v", got, err)
	}
	// A fresh gone card still waits out the grace.
	task2 := must[Task](t)(c.CreateTask(ctx, bob, NewTask{Title: "t2"}))
	must[Task](t)(c.ClaimTask(ctx, bob, task2.ID))
	ros.set(presence.AgentCard{Key: alice, Status: "idle"}, presence.AgentCard{Key: bob, Status: "gone", UpdatedAt: e.clk.now()})
	e.clk.add(time.Second)
	_, err = c.ClaimTask(ctx, alice, task2.ID)
	wantErr(t, err, ErrHeld)
}

func TestExpiredLeasesArePruned(t *testing.T) {
	e := newEnv(t)
	for i := range 5 {
		must[Lease](t)(e.c.Acquire(ctx, alice, "port:"+strconv.Itoa(9000+i), time.Minute, false))
	}
	e.clk.add(time.Hour)
	if n := len(e.c.Leases()); n != 0 {
		t.Fatalf("Leases() = %d", n)
	}
	if n := len(e.c.impl.held); n != 0 {
		t.Fatalf("held keeps %d expired leases", n)
	}
	must[Lease](t)(e.c.Acquire(ctx, bob, "port:1", 0, false))
	if n := len(e.c.impl.held); n != 1 {
		t.Fatalf("held = %d after one acquire", n)
	}
	// Replay prunes history too.
	if n := len(e.reopen().impl.held); n != 1 {
		t.Fatalf("replayed held = %d", n)
	}
}

func TestListTasksSameMillisecondKeepsCreationOrder(t *testing.T) {
	e := newEnv(t)
	var want []string
	for i := range 20 {
		want = append(want, strconv.Itoa(i))
		must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: strconv.Itoa(i)}))
	}
	for _, c := range []*Coord{e.c, e.reopen()} {
		var got []string
		for _, task := range c.ListTasks(TaskFilter{}) {
			got = append(got, task.Title)
		}
		if strings.Join(got, " ") != strings.Join(want, " ") {
			t.Fatalf("order = %v", got)
		}
	}
}

func TestAssigneeClaimMovesToWorking(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	task = must[Task](t)(e.c.AssignTask(ctx, alice, task.ID, bob))
	if task.State != Submitted || task.Holder != bob {
		t.Fatalf("assigned = %+v", task)
	}
	task = must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	if task.State != Working || task.Holder != bob {
		t.Fatalf("claimed = %+v", task)
	}
	if n := len(e.rows(KindTaskClaimed)); n != 1 {
		t.Fatalf("claimed rows = %d", n)
	}
	// A second claim is a no-op.
	must[Task](t)(e.c.ClaimTask(ctx, bob, task.ID))
	if n := len(e.rows(KindTaskClaimed)); n != 1 {
		t.Fatalf("claimed rows after re-claim = %d", n)
	}
	if got, _ := e.reopen().GetTask(task.ID); got.State != Working {
		t.Fatalf("replayed = %+v", got)
	}
}

// Rows from another device can share an HLC with the last row of a replay
// page. Both the row that ends the page and the ones after it at that HLC
// must fold exactly once.
func TestReplayHLCTieAtPageBoundary(t *testing.T) {
	e := newEnv(t)
	for i := range store.MaxListLimit {
		must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: strconv.Itoa(i)}))
	}
	rows := e.rows("task.")
	if len(rows) != store.MaxListLimit {
		t.Fatalf("rows = %d", len(rows))
	}
	tie := rows[len(rows)-1].HLC // the last row of the first page
	for seq := int64(1); seq <= 2; seq++ {
		id := fmt.Sprintf("B%d", seq)
		p, _ := json.Marshal(taskCreatedRow{TaskID: id, Title: id, By: "dev-b/s", At: fixedNow})
		if err := e.st.Insert(ctx, store.Event{
			DeviceID: "dev-b", Seq: seq, HLC: tie, Actor: "agent:dev-b/s", Kind: KindTaskCreated, Payload: p,
		}); err != nil {
			t.Fatal(err)
		}
	}
	c2 := e.reopen()
	if n := len(c2.ListTasks(TaskFilter{})); n != store.MaxListLimit+2 {
		t.Fatalf("replayed %d tasks, want %d", n, store.MaxListLimit+2)
	}
	var last taskCreatedRow
	if err := json.Unmarshal(rows[len(rows)-1].Payload, &last); err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"B1", "B2", last.TaskID} {
		if _, ok := c2.GetTask(id); !ok {
			t.Fatalf("task %s missing after replay", id)
		}
	}
}
