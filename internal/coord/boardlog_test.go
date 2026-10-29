package coord

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

const (
	boardPerson = "person:p1"
	boardCard   = "agent:c1"
)

// boardEnv is a coord over a board log (v0.5 §2.1) on dev-a.
func newBoardCoord(t *testing.T, dev string) (*store.Store, *Coord) {
	t.Helper()
	clk := &clock{t: fixedNow}
	st, err := store.OpenWith(context.Background(), filepath.Join(t.TempDir(), "boards", "b.db"), dev, hlc.New(clk.now),
		store.OpenOptions{Board: true})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	// board.created puts p1 on the board, so person:p1 keys resolve (§2.3).
	if _, err := st.Append(context.Background(), boardPerson, store.KindBoardCreated,
		map[string]any{"boardId": "b", "name": "b", "tag": "b", "creator": map[string]any{"personId": "p1"}}); err != nil {
		t.Fatal(err)
	}
	c, err := New(context.Background(), st, newRoster(boardCard), Options{Now: clk.now, Board: true, Person: boardPerson,
		IsOwner: func(k string) bool { return k == boardPerson }})
	if err != nil {
		t.Fatal(err)
	}
	return st, c
}

// TestBoardCoordActors: on a board, the caller's board key is the row's
// actor (§2.4): person:<id> for the person, agent:<cardId> for a card, and
// a private key is refused by the log rather than written.
func TestBoardCoordActors(t *testing.T) {
	ctx := context.Background()
	st, c := newBoardCoord(t, "dev-a")
	if !c.IsBoard() {
		t.Fatal("IsBoard")
	}
	if _, err := st.Append(ctx, boardCard, store.KindAgentCard, map[string]any{"cardId": "c1", "personId": "p1", "status": "active"}); err != nil {
		t.Fatal(err)
	}
	task, err := c.CreateTask(ctx, boardPerson, NewTask{Title: "shared work"})
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.Assign(ctx, boardPerson, task.ID, boardCard); err != nil {
		t.Fatal(err)
	}
	if _, err := c.ClaimTask(ctx, boardCard, task.ID); err != nil {
		t.Fatal(err)
	}
	cm, _, err := c.CommentOn(ctx, boardPerson, task.ID, CommentInput{Text: "looks good"})
	if err != nil {
		t.Fatal(err)
	}
	if got, _ := c.GetComment(cm.ID); !got.person {
		t.Error("the person's comment on a board doesn't read as the person's")
	}
	rows, err := st.List(ctx, store.Filter{Kinds: []string{"task."}, Limit: store.MaxListLimit})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{boardPerson, boardPerson, boardCard, boardPerson}
	if len(rows) != len(want) {
		t.Fatalf("%d rows, want %d", len(rows), len(want))
	}
	for i, ev := range rows {
		if ev.Actor != want[i] {
			t.Errorf("row %d (%s): actor %q, want %q", i, ev.Kind, ev.Actor, want[i])
		}
	}
	// Private keys never reach a board log.
	for _, by := range []string{"user:dev-a", alice, "api:" + alice} {
		if _, err := c.CreateTask(ctx, by, NewTask{Title: "leak"}); !errors.Is(err, store.ErrBoardActor) {
			t.Errorf("CreateTask as %q: err = %v, want ErrBoardActor", by, err)
		}
	}
	if _, _, err := c.Assign(ctx, boardPerson, task.ID, alice); !errors.Is(err, store.ErrOutsideBoard) {
		t.Errorf("assigning a private agent key: err = %v, want ErrOutsideBoard", err)
	}
	if got := len(c.ListTasks(TaskFilter{})); got != 1 {
		t.Errorf("%d tasks, want 1", got)
	}
}

// TestBoardCoordSetsAside: a forwarded row naming a card that hasn't
// arrived is stored but not folded; it folds once its task arrives (§2.3).
func TestBoardCoordSetsAside(t *testing.T) {
	ctx := context.Background()
	src, from := newBoardCoord(t, "dev-b")
	if _, err := src.Append(ctx, boardCard, store.KindAgentCard, map[string]any{"cardId": "c1", "personId": "p1"}); err != nil {
		t.Fatal(err)
	}
	task, err := from.CreateTask(ctx, boardPerson, NewTask{Title: "from b"})
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := from.CommentOn(ctx, boardCard, task.ID, CommentInput{Text: "first"}); err != nil {
		t.Fatal(err)
	}
	rows, err := src.Missing(ctx, store.Vector{}, 100)
	if err != nil {
		t.Fatal(err)
	}
	// dev-a gets dev-b's rows through a forwarder that had only the
	// comment's stream so far: here, the comment arrives first by handing
	// the fold the row directly, as Follow does for an inserted row.
	dst, c := newBoardCoord(t, "dev-a")
	var board, created, comment, card store.Event
	for _, ev := range rows {
		switch ev.Kind {
		case store.KindBoardCreated:
			board = ev
		case KindTaskCreated:
			created = ev
		case KindTaskComment:
			comment = ev
		case store.KindAgentCard:
			card = ev
		}
	}
	insert := func(evs ...store.Event) {
		t.Helper()
		if _, err := dst.InsertBatch(ctx, evs); err != nil {
			t.Fatal(err)
		}
	}
	insert(board, card)
	c.impl.mu.Lock()
	c.impl.apply(comment) // what Follow would do had the comment's stream come first
	c.impl.mu.Unlock()
	if c.Unresolved() != 1 || len(c.Comments(task.ID)) != 0 {
		t.Fatalf("unresolved %d, comments %d: the comment should wait", c.Unresolved(), len(c.Comments(task.ID)))
	}
	insert(created, comment)
	c.impl.mu.Lock()
	c.impl.apply(created)
	c.impl.mu.Unlock()
	if c.Unresolved() != 0 {
		t.Fatalf("unresolved %d after its task arrived", c.Unresolved())
	}
	if got := c.Comments(task.ID); len(got) != 1 || got[0].Text != "first" {
		t.Fatalf("comments %+v", got)
	}
	var p map[string]any
	_ = json.Unmarshal(comment.Payload, &p)
	if p["by"] != boardCard {
		t.Fatalf("comment by %v", p["by"])
	}
}

// TestBoardOwnerRuleIsThePerson: on a board an owner-only row counts only
// from a device of the owning person (§2.4). dev-a is p1's and holds the
// lease through p1's card; a lease.released for it written by p2's dev-b is
// ignored and counted, and the lease stands. The same row from dev-a's own
// person counts.
func TestBoardOwnerRuleIsThePerson(t *testing.T) {
	ctx := context.Background()
	clk := &clock{t: fixedNow}
	st, err := store.OpenWith(ctx, filepath.Join(t.TempDir(), "boards", "b.db"), "dev-a", hlc.New(clk.now), store.OpenOptions{Board: true})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	devs := map[string]string{"dev-a": "p1", "dev-b": "p2"}
	keys := map[string]string{boardPerson: "p1", boardCard: "p1", "person:p2": "p2"}
	c, err := New(ctx, st, newRoster(boardCard), Options{Now: clk.now, Board: true, Person: boardPerson,
		IsOwner:      func(k string) bool { return k == boardPerson },
		KeyPerson:    func(k string) string { return keys[k] },
		DevicePerson: func(d string) string { return devs[d] }})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := st.Append(ctx, boardCard, store.KindAgentCard, map[string]any{"cardId": "c1", "personId": "p1", "status": "active"}); err != nil {
		t.Fatal(err)
	}
	if _, err := c.Acquire(ctx, boardCard, "port:5173", time.Hour, false); err != nil {
		t.Fatal(err)
	}
	release := func(dev string, seq int64) store.Event {
		raw, _ := json.Marshal(leaseReleasedRow{Resource: "port:5173", Holder: boardCard, Reason: "done", By: "person:p2", At: fixedNow})
		return store.Event{DeviceID: dev, Seq: seq, HLC: hlc.Timestamp(fixedNow.Add(time.Minute).UnixMilli()) << 16, Actor: "person:p2", Kind: KindLeaseRel, Payload: raw}
	}
	forged := release("dev-b", 1)
	if err := st.Insert(ctx, forged); err != nil {
		t.Fatal(err)
	}
	c.impl.mu.Lock()
	c.impl.apply(forged)
	c.impl.mu.Unlock()
	if c.IgnoredRows() != 1 {
		t.Fatalf("ignored %d rows, want the forged release", c.IgnoredRows())
	}
	if ls := c.Leases(); len(ls) != 1 || ls[0].Holder != boardCard {
		t.Fatalf("leases %+v: p2's device ended p1's lease", ls)
	}
	// A key whose person is unknown owns nothing either.
	delete(keys, boardCard)
	if c.impl.mayWrite(boardCard, "dev-a") {
		t.Fatal("a card of no known person is owned by dev-a")
	}
	keys[boardCard] = "p1"
	if !c.impl.mayWrite(boardCard, "dev-a") || c.impl.mayWrite(boardCard, "dev-b") {
		t.Fatal("the owner rule is not the holder's person")
	}
}

// TestBoardCancelIsTheCreatorsOrAnOwners: on a board a cancel, withdrawn
// or not, counts only from a device of the card creator's person or of an
// owner (§5.3, §11 "a cancel of Stephen's card"). p1 made the card on
// dev-a; p2's dev-b cancels it and is ignored and counted; once p2 owns the
// board, its cancel counts. Withdraw marks the card withdrawn.
func TestBoardCancelIsTheCreatorsOrAnOwners(t *testing.T) {
	ctx := context.Background()
	clk := &clock{t: fixedNow}
	st, err := store.OpenWith(ctx, filepath.Join(t.TempDir(), "boards", "b.db"), "dev-a", hlc.New(clk.now), store.OpenOptions{Board: true})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	if _, err := st.Append(ctx, boardPerson, store.KindBoardCreated,
		map[string]any{"boardId": "b", "name": "b", "tag": "mlg", "creator": map[string]any{"personId": "p1"}}); err != nil {
		t.Fatal(err)
	}
	if _, err := st.Append(ctx, boardPerson, store.KindBoardMemberAdded, map[string]any{"personId": "p2", "role": "member"}); err != nil {
		t.Fatal(err)
	}
	devs := map[string]string{"dev-a": "p1", "dev-b": "p2"}
	keys := map[string]string{boardPerson: "p1", "person:p2": "p2"}
	owners := map[string]bool{"p1": true}
	c, err := New(ctx, st, newRoster(boardCard), Options{Now: clk.now, Board: true, Person: boardPerson,
		IsOwner:      func(k string) bool { return k == boardPerson },
		KeyPerson:    func(k string) string { return keys[k] },
		DevicePerson: func(d string) string { return devs[d] },
		Tag:          func() string { return "mlg" },
		BoardOwner:   func(p string) bool { return owners[p] }})
	if err != nil {
		t.Fatal(err)
	}
	task, err := c.CreateTask(ctx, boardPerson, NewTask{Title: "shared work"})
	if err != nil {
		t.Fatal(err)
	}
	if task.ShortRef() != "mlg#1" || task.Refs() != "mlg#1, task:"+task.ID {
		t.Fatalf("refs %q / %q", task.ShortRef(), task.Refs())
	}
	for ref, want := range map[string]error{"mlg#1": nil, "#1": nil, "1": nil, "other#1": ErrNotFound, "mlg#2": ErrNotFound} {
		got, err := c.TaskRef(ref)
		if !errors.Is(err, want) || (want == nil && got != task.ID) {
			t.Errorf("TaskRef(%q) = %q, %v; want %v", ref, got, err, want)
		}
	}
	cancel := func(dev string, seq int64) store.Event {
		raw, _ := json.Marshal(taskStateRow{TaskID: task.ID, State: Canceled, Note: "mine now", By: "person:p2", At: fixedNow})
		return store.Event{DeviceID: dev, Seq: seq, HLC: hlc.Timestamp(fixedNow.Add(time.Duration(seq)*time.Minute).UnixMilli()) << 16, Actor: "person:p2", Kind: KindTaskState, Payload: raw}
	}
	fold := func(ev store.Event) {
		t.Helper()
		if err := st.Insert(ctx, ev); err != nil {
			t.Fatal(err)
		}
		c.impl.mu.Lock()
		c.impl.apply(ev)
		c.impl.mu.Unlock()
	}
	fold(cancel("dev-b", 1))
	if got, _ := c.GetTask(task.ID); got.State == Canceled || c.IgnoredRows() != 1 {
		t.Fatalf("p2 canceled p1's card: %+v, ignored %d", got, c.IgnoredRows())
	}
	owners["p2"] = true
	fold(cancel("dev-b", 2))
	if got, _ := c.GetTask(task.ID); got.State != Canceled || got.Withdrawn {
		t.Fatalf("an owner's cancel didn't count: %+v", got)
	}
	again, err := c.CreateTask(ctx, boardPerson, NewTask{Title: "more work"})
	if err != nil {
		t.Fatal(err)
	}
	w, err := c.Withdraw(ctx, boardPerson, again.ID, "")
	if err != nil {
		t.Fatal(err)
	}
	if !w.Withdrawn || w.State != Canceled {
		t.Fatalf("withdrawn %+v", w)
	}
}

// TestParseRef: a ref's board tag and number split one way (B3).
func TestParseRef(t *testing.T) {
	for in, want := range map[string][3]string{
		"mlg#12": {"mlg", "12", "ok"}, "MLG#12b": {"mlg", "12b", "ok"}, "#12": {"", "12", "ok"}, "12": {"", "12", "ok"},
		"a-1#3": {"a-1", "3", "ok"}, "1mlg#3": {}, "mlg#": {}, "mlg#x": {}, "#": {}, "01M6336VP921939GKCWCDJP0EZ": {},
	} {
		tag, short, ok := ParseRef(in)
		if (ok && want[2] != "ok") || (!ok && want[2] == "ok") || tag != want[0] || short != want[1] {
			t.Errorf("ParseRef(%q) = %q %q %v; want %v", in, tag, short, ok, want)
		}
	}
}
