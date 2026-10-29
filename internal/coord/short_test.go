package coord

import (
	"encoding/json"
	"errors"
	"fmt"
	"math/rand/v2"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/search"
	"github.com/StephenSHorton/irori/internal/store"
)

func TestShortRefsCountUpAndResolve(t *testing.T) {
	e := newEnv(t)
	a := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "one"}))
	b := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "two"}))
	if a.Short != "1" || b.Short != "2" || a.ShortRef() != "#1" || b.ShortWas != "" {
		t.Fatalf("shorts %q %q", a.Short, b.Short)
	}
	for _, in := range []string{"#2", "2", " #2 ", "task:#2", "TASK:2", b.ID, "task:" + b.ID} {
		if id, err := e.c.TaskRef(in); err != nil || id != b.ID {
			t.Fatalf("TaskRef(%q) = %q, %v", in, id, err)
		}
	}
	if _, err := e.c.TaskRef("#9"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("an unknown short ref: %v", err)
	}
	for _, in := range []string{"#0", "#01", "#x", "12-b", "#1234567890123"} {
		if _, ok := ParseShort(in); ok {
			t.Fatalf("ParseShort(%q) took it", in)
		}
	}
	// The number survives a restart, and the next card counts on.
	c2 := e.reopen()
	if x, _ := c2.GetTask(b.ID); x.Short != "2" {
		t.Fatalf("after replay %q", x.Short)
	}
	if x := must[Task](t)(c2.CreateTask(ctx, alice, NewTask{Title: "three"})); x.Short != "3" {
		t.Fatalf("next %q", x.Short)
	}
	// show finds a card by its short ref, and search docs carry it.
	sh, err := c2.SearchShow(ctx, "task:#2", search.ShowOptions{})
	if err != nil || sh.Ref != "task:"+b.ID || !strings.Contains(sh.Text, "Task #2 ") {
		t.Fatalf("show #2: %+v %v", sh, err)
	}
}

// createdRow is a task.created row as another device wrote it.
func createdRow(dev string, seq int64, at hlc.Timestamp, id string, num int) store.Event {
	raw, _ := json.Marshal(taskCreatedRow{TaskID: id, Num: num, Title: id, By: dev + "/s", At: fixedNow})
	return store.Event{DeviceID: dev, Seq: seq, HLC: at, Actor: "agent:" + dev + "/s", Kind: KindTaskCreated, Payload: raw}
}

// Two devices that numbered cards apart settle on the same refs whatever
// order the devices' rows arrive in (each device's in seq order, as the
// mesh sends them): every claimant of a clashed number gains a letter in
// (hlc, device, seq) order and says what it was, and the bare number is
// refused with each card named.
func TestShortRefClashesSettleInAnyOrder(t *testing.T) {
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	rows := []store.Event{
		createdRow("dev-b", 1, base+10, "B1", 1),
		createdRow("dev-b", 2, base+20, "B2", 2),
		createdRow("dev-c", 1, base+15, "C1", 1), // later than B1: 1b
		createdRow("dev-c", 2, base+20, "C2", 2), // same hlc as B2: dev-b sorts first, so 2b
		createdRow("dev-c", 3, base+30, "C3", 2), // a third claim on 2: 2c
		createdRow("dev-b", 3, base+40, "B3", 3),
	}
	want := map[string][2]string{"B1": {"1a", "1"}, "C1": {"1b", "1"}, "B2": {"2a", "2"}, "C2": {"2b", "2"}, "C3": {"2c", "2"}, "B3": {"3", ""}}
	check := func(name string, c *Coord) {
		t.Helper()
		for id, w := range want {
			x, ok := c.GetTask(id)
			if !ok || x.Short != w[0] || x.ShortWas != w[1] {
				t.Fatalf("%s: %s is %q (was %q), want %q (was %q)", name, id, x.Short, x.ShortWas, w[0], w[1])
			}
			if got, err := c.TaskRef("#" + w[0]); err != nil || got != id {
				t.Fatalf("%s: #%s resolves to %q %v", name, w[0], got, err)
			}
		}
		for bare, ids := range map[string][]string{"#1": {"B1", "C1"}, "2": {"B2", "C2", "C3"}, "task:#2": {"B2", "C2", "C3"}} {
			got, err := c.TaskRef(bare)
			if !errors.Is(err, ErrAmbiguous) || got != "" {
				t.Fatalf("%s: %s = %q, %v; want ErrAmbiguous", name, bare, got, err)
			}
			for _, id := range ids {
				if !strings.Contains(err.Error(), "(task:"+id+")") {
					t.Fatalf("%s: %s doesn't name %s: %v", name, bare, id, err)
				}
			}
		}
	}
	for name, order := range map[string][]int{"in order": {0, 1, 2, 3, 4, 5}, "b first": {0, 1, 5, 2, 3, 4}, "c first": {2, 3, 4, 0, 1, 5}, "mixed": {2, 3, 0, 4, 1, 5}} {
		e := newEnv(t)
		var seen []Change
		ch := e.c.Subscribe(ctx)
		for _, i := range order {
			e.c.impl.applyRemote(ctx, rows[i])
		}
		check(name, e.c)
		for len(ch) > 0 {
			seen = append(seen, <-ch)
		}
		if len(seen) == 0 {
			t.Fatalf("%s: no changes", name)
		}
		if n := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "after"})); n.Short != "4" {
			t.Fatalf("%s: the next card is %q", name, n.Short)
		}
	}
	// A replay of the same rows from the log agrees.
	e := newEnv(t)
	for _, ev := range rows {
		if err := e.st.Insert(ctx, ev); err != nil {
			t.Fatal(err)
		}
	}
	check("replay", e.reopen())
}

// Rows from before num claim their rank among such rows; a numbered card
// written after them counts on from there.
func TestShortRefsForOlderRows(t *testing.T) {
	e := newEnv(t)
	for _, title := range []string{"old one", "old two"} {
		if _, err := e.st.Append(ctx, "agent:"+alice, KindTaskCreated, taskCreatedRow{TaskID: newID(fixedNow) + title[4:], Title: title, By: alice, At: fixedNow}); err != nil {
			t.Fatal(err)
		}
		e.clk.add(time.Second)
	}
	c := e.reopen()
	var got []string
	for _, x := range c.ListTasks(TaskFilter{}) {
		got = append(got, x.Short)
	}
	if strings.Join(got, ",") != "1,2" {
		t.Fatalf("older rows numbered %v", got)
	}
	if x := must[Task](t)(c.CreateTask(ctx, alice, NewTask{Title: "new"})); x.Short != "3" {
		t.Fatalf("a new card after older rows: %q", x.Short)
	}
}

func TestShortSuffix(t *testing.T) {
	for i, want := range map[int]string{0: "a", 1: "b", 2: "c", 25: "z", 26: "aa", 27: "ab"} {
		if got := shortSuffix(i); got != want {
			t.Fatalf("shortSuffix(%d) = %q, want %q", i, got, want)
		}
	}
}

// A card of this device whose number turns out to clash moves visibly: its
// creator hears in the watch digest, search re-reads it, and the bare
// number it showed is refused rather than resolved to the other card.
func TestShortRefClashIsHeardAndRefused(t *testing.T) {
	e := newEnv(t)
	mine := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "mine"}))
	if mine.Short != "1" {
		t.Fatalf("mine is %q", mine.Short)
	}
	if _, err := e.c.SweepNotices(ctx); err != nil { // drain what the create queued
		t.Fatal(err)
	}
	e.clk.add(time.Minute)
	since := e.clk.now()
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	e.c.impl.applyRemote(ctx, createdRow("dev-z", 1, base-1<<16, "Z1", 1)) // made a millisecond before mine
	x, _ := e.c.GetTask(mine.ID)
	if x.Short != "1b" || x.ShortWas != "1" {
		t.Fatalf("mine after the clash is %q (was %q)", x.Short, x.ShortWas)
	}
	if _, err := e.c.TaskRef("#1"); !errors.Is(err, ErrAmbiguous) || !strings.Contains(err.Error(), `#1a "Z1"`) || !strings.Contains(err.Error(), `#1b "mine"`) {
		t.Fatalf("#1 after the clash: %v", err)
	}
	ns := must[[]Notice](t)(e.c.SweepNotices(ctx))
	heard := false
	for _, n := range ns {
		if n.Kind == NoticeWatched && n.TaskID == mine.ID && slices.Contains(n.To, alice) && strings.Contains(n.Text, "renumbered it #1b (it was #1)") {
			heard = true
		}
	}
	if !heard {
		t.Fatalf("the creator didn't hear of the renumber: %+v", ns)
	}
	docs := must[[]search.Doc](t)(e.c.SearchDocs(ctx, since, 0))
	reread := false
	for _, d := range docs {
		if d.Ref == "task:"+mine.ID && strings.HasPrefix(d.Body, "#1b") {
			reread = true
		}
	}
	if !reread {
		t.Fatalf("search didn't re-read the moved card: %+v", docs)
	}
}

// A re-fold after a revoke's cut drops the cut device's claims: the card
// that had clashed with one loses its letter, visibly, and the cut card's
// numbers aren't handed out again.
func TestShortRefsAfterARefold(t *testing.T) {
	e := newEnv(t)
	mine := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "mine"}))
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	// dev-z's rows are folded but never held by the store, as after a cut.
	e.c.impl.applyRemote(ctx, createdRow("dev-z", 1, base-1<<16, "Z1", 1))
	e.c.impl.applyRemote(ctx, createdRow("dev-z", 2, base+1<<16, "Z2", 2))
	if x, _ := e.c.GetTask(mine.ID); x.Short != "1b" {
		t.Fatalf("before the cut mine is %q", x.Short)
	}
	if _, err := e.c.SweepNotices(ctx); err != nil {
		t.Fatal(err)
	}
	e.clk.add(time.Minute)
	since := e.clk.now()
	vec := must[store.Vector](t)(e.st.Vector(ctx))
	e.c.impl.refoldIfCut(ctx, vec)
	x, _ := e.c.GetTask(mine.ID)
	if x.Short != "1" || x.ShortWas != "" {
		t.Fatalf("after the cut mine is %q (was %q)", x.Short, x.ShortWas)
	}
	if _, ok := e.c.GetTask("Z1"); ok {
		t.Fatal("the cut card is still folded")
	}
	ns := must[[]Notice](t)(e.c.SweepNotices(ctx))
	if len(ns) != 1 || !strings.Contains(ns[0].Text, "renumbered it #1 (it was #1b)") {
		t.Fatalf("notices after the cut: %+v", ns)
	}
	if !e.c.impl.movedSince(mine.ID, since) {
		t.Fatal("the moved card isn't marked for search")
	}
	if n := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "next"})); n.Short != "3" {
		t.Fatalf("the next card is %q: the cut card's #2 came back", n.Short)
	}
}

// Folding many claims one row at a time (the incremental path) gives the
// same refs as a replay of the same rows, whatever the interleaving.
func TestShortRefsIncrementalMatchesReplay(t *testing.T) {
	base := hlc.Timestamp(fixedNow.UnixMilli()) << 16
	rnd := rand.New(rand.NewPCG(7, 11))
	devs := []string{"dev-b", "dev-c", "dev-d"}
	var perDev [3][]store.Event
	top := [3]int{}
	for i := range 300 {
		d := rnd.IntN(3)
		if rnd.IntN(4) == 0 { // a sync: this device catches up with the highest number
			top[d] = max(top[0], top[1], top[2])
		}
		top[d]++
		at := base + hlc.Timestamp(i/2)<<16 + hlc.Timestamp(rnd.IntN(3))
		if n := len(perDev[d]); n > 0 {
			at = max(at, perDev[d][n-1].HLC+1) // a device's hlc only rises
		}
		perDev[d] = append(perDev[d], createdRow(devs[d], int64(len(perDev[d])+1), at, fmt.Sprintf("%s-%d", devs[d], len(perDev[d])+1), top[d]))
	}
	e := newEnv(t)
	var next [3]int
	for {
		var open []int
		for d := range 3 {
			if next[d] < len(perDev[d]) {
				open = append(open, d)
			}
		}
		if len(open) == 0 {
			break
		}
		d := open[rnd.IntN(len(open))]
		ev := perDev[d][next[d]]
		next[d]++
		e.c.impl.applyRemote(ctx, ev)
		if err := e.st.Insert(ctx, ev); err != nil {
			t.Fatal(err)
		}
	}
	re := e.reopen()
	seen, clashed := map[string]string{}, 0
	for _, x := range re.ListTasks(TaskFilter{}) {
		if x.ShortWas != "" {
			clashed++
		}
		y, _ := e.c.GetTask(x.ID)
		if y.Short != x.Short || y.ShortWas != x.ShortWas {
			t.Fatalf("%s: folded %q (was %q), replayed %q (was %q)", x.ID, y.Short, y.ShortWas, x.Short, x.ShortWas)
		}
		if other, dup := seen[x.Short]; dup {
			t.Fatalf("#%s is both %s and %s", x.Short, other, x.ID)
		}
		seen[x.Short] = x.ID
	}
	if len(seen) != 300 || clashed == 0 {
		t.Fatalf("%d refs for 300 cards, %d clashed", len(seen), clashed)
	}
}
