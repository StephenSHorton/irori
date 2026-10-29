package coord

import (
	"database/sql"
	"encoding/json"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/search"
	"github.com/StephenSHorton/irori/internal/store"
)

// A card's thread: comments, mirrored messages and activity lines, each
// with the device and log position of the row that made it.

func hlcAt(t time.Time) hlc.Timestamp { return hlc.Timestamp(t.UnixMilli()) << 16 }

// remote inserts a row another device wrote and folds it live, as Follow
// does.
func (e *env) remote(dev string, seq int64, at hlc.Timestamp, actor, kind string, payload any) store.Event {
	e.t.Helper()
	raw, err := json.Marshal(payload)
	if err != nil {
		e.t.Fatal(err)
	}
	ev := store.Event{DeviceID: dev, Seq: seq, HLC: at, Actor: actor, Kind: kind, Payload: raw}
	if err := e.st.Insert(ctx, ev); err != nil {
		e.t.Fatal(err)
	}
	e.c.impl.applyRemote(ctx, ev)
	return ev
}

// The recipient's gone row can reach a third device before the sender's
// row, with a lower HLC. Either order folds to one entry, gone, that takes
// its device, position, time and text from the sender's row, and replay
// agrees.
func TestGoneMergeEitherOrder(t *testing.T) {
	const recipient = "dev-b/x"
	sent, swept := fixedNow.Add(time.Minute), fixedNow.Add(5*time.Minute)
	primary := taskCommentRow{CommentID: "msg_1", MsgID: "msg_1", By: alice, To: recipient, Text: "hello", Files: 1, At: sent}
	gone := taskCommentRow{CommentID: "msg_1", MsgID: "msg_1", By: alice, To: recipient, Text: "hello (resent)", Gone: true, At: swept}
	base := hlcAt(fixedNow.Add(time.Hour))
	fold := func(goneFirst bool) (Comment, Task) {
		t.Helper()
		e := newEnvOn(t, "dev-c")
		task := must[Task](t)(e.c.CreateTask(ctx, carol, NewTask{Title: "t"}))
		p, g := primary, gone
		p.TaskID, g.TaskID = task.ID, task.ID
		putPrimary := func() { e.remote("dev-a", 1, base+10, "agent:"+alice, KindTaskComment, p) }
		putGone := func() { e.remote("dev-b", 1, base+5, "irori", KindTaskComment, g) }
		if goneFirst {
			putGone()
			putPrimary()
		} else {
			putPrimary()
			putGone()
		}
		cs := e.c.Comments(task.ID)
		if len(cs) != 1 {
			t.Fatalf("goneFirst=%v: %d entries", goneFirst, len(cs))
		}
		c2 := e.reopen()
		if again := c2.Comments(task.ID); !reflect.DeepEqual(again, cs) {
			t.Fatalf("goneFirst=%v: replay\n%+v\nlive\n%+v", goneFirst, again, cs)
		}
		got, _ := e.c.GetTask(task.ID)
		if replayed, _ := c2.GetTask(task.ID); !reflect.DeepEqual(replayed, got) {
			t.Fatalf("goneFirst=%v: task replay\n%+v\nlive\n%+v", goneFirst, replayed, got)
		}
		if a := e.c.Activity(task.ID); len(a) != 1 || a[0].What != "put the card up" {
			t.Fatalf("a mirrored message made activity: %+v", a)
		}
		cm := cs[0]
		cm.TaskID = ""
		return cm, got
	}
	a, ta := fold(false)
	b, tb := fold(true)
	if !reflect.DeepEqual(a, b) {
		t.Fatalf("orders differ:\n%+v\n%+v", a, b)
	}
	if !a.Gone || a.Device != "dev-a" || a.HLC != base+10 || a.Seq != 1 || a.Text != "hello" || !a.At.Equal(sent) || a.Files != 1 || a.To != recipient {
		t.Fatalf("entry %+v", a)
	}
	if ta.Comments != 1 || tb.Comments != 1 || !ta.UpdatedAt.Equal(sent) || !tb.UpdatedAt.Equal(sent) {
		t.Fatalf("tasks %+v / %+v", ta, tb)
	}
}

// A message recorded held (the wake policy held it, or its recipient's
// device hadn't decided) gets its text from the fill row the recipient's
// device writes once it lets the message through. Either order folds to
// the same entry, placed by the sender's row; replay agrees. A fill row
// from a device that is neither party's is ignored.
func TestFillMergeEitherOrder(t *testing.T) {
	const recipient = "dev-b/x"
	sent, passed := fixedNow.Add(time.Minute), fixedNow.Add(3*time.Minute)
	primary := taskCommentRow{CommentID: "msg_1", MsgID: "msg_1", By: alice, To: recipient, Held: true, Files: 1, At: sent}
	fill := taskCommentRow{CommentID: "msg_1", MsgID: "msg_1", By: alice, To: recipient, Text: "hello", Files: 1, Fill: true, At: passed}
	base := hlcAt(fixedNow.Add(time.Hour))
	fold := func(fillFirst bool) Comment {
		t.Helper()
		e := newEnvOn(t, "dev-c")
		task := must[Task](t)(e.c.CreateTask(ctx, carol, NewTask{Title: "t"}))
		p, f := primary, fill
		p.TaskID, f.TaskID = task.ID, task.ID
		forged := f
		forged.Text = "forged"
		e.remote("dev-z", 1, base+1, "irori", KindTaskComment, forged)
		putPrimary := func() { e.remote("dev-a", 1, base+10, "agent:"+alice, KindTaskComment, p) }
		putFill := func() { e.remote("dev-b", 1, base+20, "irori", KindTaskComment, f) }
		if fillFirst {
			putFill()
			putPrimary()
		} else {
			putPrimary()
			putFill()
		}
		cs := e.c.Comments(task.ID)
		if len(cs) != 1 {
			t.Fatalf("fillFirst=%v: %d entries", fillFirst, len(cs))
		}
		if again := e.reopen().Comments(task.ID); !reflect.DeepEqual(again, cs) {
			t.Fatalf("fillFirst=%v: replay\n%+v\nlive\n%+v", fillFirst, again, cs)
		}
		cm := cs[0]
		cm.TaskID = ""
		return cm
	}
	a, b := fold(false), fold(true)
	if !reflect.DeepEqual(a, b) {
		t.Fatalf("orders differ:\n%+v\n%+v", a, b)
	}
	if a.Held || a.Text != "hello" || a.Device != "dev-a" || a.HLC != base+10 || !a.At.Equal(sent) || a.Files != 1 || a.Gone {
		t.Fatalf("entry %+v", a)
	}
}

// A revoke's cut moves a device's rows out of the log; the re-fold drops
// its comments, mirrored messages and activity lines.
func TestRefoldIfCutDropsThreadEntries(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	mine, _, err := e.c.AddComment(ctx, user, task.ID, "from here", false)
	if err != nil {
		t.Fatal(err)
	}
	const zed = "dev-b/zed"
	at := hlcAt(e.clk.now().Add(time.Second))
	e.remote("dev-b", 1, at, "agent:"+zed, KindTaskComment, taskCommentRow{TaskID: task.ID, CommentID: "c-b", By: zed, Text: "from b", At: fixedNow})
	e.remote("dev-b", 2, at+1, "agent:"+zed, KindTaskComment, taskCommentRow{TaskID: task.ID, CommentID: "msg_b", MsgID: "msg_b", By: zed, To: alice, Text: "hi alice", At: fixedNow})
	e.remote("dev-b", 3, at+2, "agent:"+zed, KindTaskJoined, taskMemberRow{TaskID: task.ID, Who: zed, By: zed, At: fixedNow})
	if n := len(e.c.Comments(task.ID)); n != 3 {
		t.Fatalf("%d comments before the cut", n)
	}
	devs := func() map[string]int {
		out := map[string]int{}
		for _, a := range e.c.Activity(task.ID) {
			out[a.Device]++
		}
		return out
	}
	if devs()["dev-b"] != 2 {
		t.Fatalf("activity before the cut: %+v", e.c.Activity(task.ID))
	}

	db, err := sql.Open("sqlite", e.path)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if _, err := db.ExecContext(ctx, `DELETE FROM events WHERE device_id = 'dev-b'`); err != nil {
		t.Fatal(err)
	}
	vec, err := e.st.Vector(ctx)
	if err != nil {
		t.Fatal(err)
	}
	e.c.impl.refoldIfCut(ctx, vec)

	cs := e.c.Comments(task.ID)
	if len(cs) != 1 || cs[0].ID != mine.ID {
		t.Fatalf("comments after the cut: %+v", cs)
	}
	if _, ok := e.c.GetComment("msg_b"); ok {
		t.Fatal("the cut device's message is still on the card")
	}
	if d := devs(); d["dev-b"] != 0 || d["dev-a"] == 0 {
		t.Fatalf("activity after the cut: %+v", e.c.Activity(task.ID))
	}
	if got, _ := e.c.GetTask(task.ID); got.Comments != 1 || contains(got.Holders, zed) {
		t.Fatalf("task after the cut: %+v", got)
	}
}

func TestRenderTaskShowsMessages(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	rec := func(m MessageEntry) {
		t.Helper()
		m.TaskID = task.ID
		if _, err := e.c.RecordMessage(ctx, m); err != nil {
			t.Fatal(err)
		}
	}
	rec(MessageEntry{MsgID: "msg_1", From: alice, To: bob, Text: "please review"})
	rec(MessageEntry{MsgID: "msg_2", From: bob, To: carol, Text: "carol?"})
	rec(MessageEntry{MsgID: "msg_2", From: bob, To: carol, Text: "carol?", Gone: true, Writer: "irori"})
	rec(MessageEntry{MsgID: "msg_3", From: carol, To: alice, Text: "secret", Held: true, Files: 2})
	if _, _, err := e.c.AddComment(ctx, user, task.ID, "plain one", false); err != nil {
		t.Fatal(err)
	}
	sh, err := e.c.SearchShow(ctx, "task:"+task.ID, search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{
		alice + " -> " + bob + ": please review\n",
		bob + " -> " + carol + ": carol? (recipient gone, left on the card)\n",
		carol + " -> " + alice + ": (not let through the wake policy yet; text not on the card) [2 files on msg:msg_3]\n",
		user + ": plain one\n",
		"comments (4",
	} {
		if !strings.Contains(sh.Text, s) {
			t.Fatalf("show lacks %q:\n%s", s, sh.Text)
		}
	}
	if strings.Contains(sh.Text, "secret") || strings.Contains(sh.Text, user+" -> ") {
		t.Fatalf("show:\n%s", sh.Text)
	}
}
