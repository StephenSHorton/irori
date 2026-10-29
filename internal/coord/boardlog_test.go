package coord

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"testing"

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
	var created, comment, card store.Event
	for _, ev := range rows {
		switch ev.Kind {
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
	insert(card)
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
