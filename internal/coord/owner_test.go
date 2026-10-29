package coord

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// The person's key on another certified device has every person exemption
// (manage any task, take a lease back, be blocked on); a made-up "user:"
// key has none (v0.4 §2).
func TestOwnerKeysAndMadeUpKeys(t *testing.T) {
	e := newEnv(t)
	other, fake := "user:dev-b", "user:dev-z"
	e.owner = func(k string) bool { return k == user || k == other }
	e.c = e.reopen()

	tk := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "t"}))
	must[Task](t)(e.c.ClaimTask(ctx, alice, tk.ID))
	if _, err := e.c.SetTaskState(ctx, fake, tk.ID, Canceled, "mine now"); err == nil {
		t.Fatal("a made-up user: key managed someone else's task")
	}
	must[Task](t)(e.c.SetTaskState(ctx, other, tk.ID, InputRequired, "the person on another device"))

	res := "path:" + repoA + ":src/**"
	must[Lease](t)(e.c.Acquire(ctx, alice, res, time.Minute, false))
	wantErr(t, e.c.Release(ctx, fake, res), ErrHeld)
	if err := e.c.Release(ctx, other, res); err != nil {
		t.Fatalf("the person on another device releasing a lease: %v", err)
	}
	if e.c.IsPerson(fake) || !e.c.IsPerson(other) || !e.c.IsPerson(user) {
		t.Fatal("IsPerson disagrees with the owner rule")
	}
	if a := e.c.impl.actorFor(fake); a != "agent:"+fake {
		t.Fatalf("actor for a made-up key = %q", a)
	}
	if a := e.c.impl.actorFor(other); a != "user" {
		t.Fatalf("actor for another device's owner key = %q", a)
	}
}

// Writer rules (v0.4 §6.0): lease rows count only from the holder's device.
func TestLeaseRowsFromTheWrongDeviceAreIgnored(t *testing.T) {
	e := newEnv(t)
	res := "path:" + repoA + ":src/**"
	must[Lease](t)(e.c.Acquire(ctx, alice, res, time.Hour, false))
	at := hlc.Timestamp(fixedNow.Add(time.Minute).UnixMilli()) << 16
	put := func(seq int64, kind string, payload any) {
		t.Helper()
		raw, _ := json.Marshal(payload)
		if err := e.st.Insert(ctx, store.Event{DeviceID: "dev-b", Seq: seq, HLC: at + hlc.Timestamp(seq), Actor: "irori", Kind: kind, Payload: raw}); err != nil {
			t.Fatal(err)
		}
	}
	name, _ := NormalizeResource(res)
	// dev-b releases alice's lease (alice is on dev-a), and takes one in
	// dev-a's name.
	put(1, KindLeaseRel, leaseReleasedRow{Resource: name, Holder: alice, Reason: ReasonReleased, By: "dev-b/x", At: fixedNow})
	put(2, KindLeaseAcq, leaseAcquiredRow{Lease: Lease{Resource: "named:x", Holder: bob, Until: fixedNow.Add(time.Hour), AcquiredAt: fixedNow}})
	c2 := e.reopen()
	ls := c2.Leases()
	if len(ls) != 1 || ls[0].Holder != alice {
		t.Fatalf("leases after wrong-device rows: %+v", ls)
	}
	if n := c2.IgnoredRows(); n != 2 {
		t.Fatalf("ignored %d rows, want 2", n)
	}
	// A lease dev-b's own agent holds is dev-b's to write.
	put(3, KindLeaseAcq, leaseAcquiredRow{Lease: Lease{Resource: "named:y", Holder: "dev-b/agent", Until: fixedNow.Add(time.Hour), AcquiredAt: fixedNow}})
	if ls := e.reopen().Leases(); len(ls) != 2 {
		t.Fatalf("dev-b's own lease dropped: %+v", ls)
	}
}

// A thread entry's device is the row's writer, never the payload's by. A
// mirrored message counts only from its sender's device, and its gone row
// also from its recipient's.
func TestThreadRowsCarryEventDevice(t *testing.T) {
	e := newEnv(t)
	task := claimed(t, e)
	at := hlcAt(fixedNow.Add(time.Minute))
	const recipient = "dev-b/x"
	e.remote("dev-b", 1, at+1, "irori", KindTaskComment, taskCommentRow{TaskID: task.ID, CommentID: "c-b", By: alice, Text: "says alice", At: fixedNow})
	e.remote("dev-b", 2, at+2, "irori", KindTaskComment, taskCommentRow{TaskID: task.ID, CommentID: "msg_spoof", MsgID: "msg_spoof", By: alice, To: bob, Text: "spoof", At: fixedNow})
	e.remote("dev-b", 3, at+3, "irori", KindTaskComment, taskCommentRow{TaskID: task.ID, CommentID: "msg_gone", MsgID: "msg_gone", By: alice, To: recipient, Text: "for x", Gone: true, At: fixedNow})
	check := func(c *Coord) {
		t.Helper()
		cs := c.Comments(task.ID)
		if len(cs) != 2 || cs[0].ID != "c-b" || cs[0].Device != "dev-b" || cs[0].By != alice || cs[0].Seq != 1 || cs[0].HLC != at+1 {
			t.Fatalf("comments %+v", cs)
		}
		if g := cs[1]; g.ID != "msg_gone" || !g.Gone || g.Device != "dev-b" || g.To != recipient {
			t.Fatalf("gone row %+v", g)
		}
		if n := c.IgnoredRows(); n != 1 {
			t.Fatalf("ignored %d rows, want 1", n)
		}
		acts := c.Activity(task.ID)
		if acts[0].What != "commented" || acts[0].Device != "dev-b" || acts[0].Kind != KindTaskComment || acts[0].By != alice {
			t.Fatalf("activity %+v", acts)
		}
	}
	check(e.c)
	check(e.reopen())
}
