package coord

import (
	"encoding/json"
	"errors"
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
// mesh sends them): the earlier claimant (hlc, device, seq) keeps
// the number, the later gains a letter and says what it was.
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
	want := map[string][2]string{"B1": {"1", ""}, "C1": {"1b", "1"}, "B2": {"2", ""}, "C2": {"2b", "2"}, "C3": {"2c", "2"}, "B3": {"3", ""}}
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
	for i, want := range map[int]string{0: "", 1: "b", 2: "c", 25: "z", 26: "aa", 27: "ab"} {
		if got := shortSuffix(i); got != want {
			t.Fatalf("shortSuffix(%d) = %q, want %q", i, got, want)
		}
	}
}
