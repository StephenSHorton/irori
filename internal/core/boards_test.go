package core

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/store"
	"github.com/StephenSHorton/irori/internal/tools"
)

// TestBoardRouting: one person, the private board and two shared boards on
// one device (v0.5 §2.1). Every taskId routes to the one log that holds
// it, an id in no log routes nowhere, and that still holds after a
// restart, when the index is rebuilt from the logs.
func TestBoardRouting(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	private, err := c.Coord().CreateTask(ctx, r.keyA, coord.NewTask{Title: "private work"})
	if err != nil {
		t.Fatal(err)
	}
	b1, err := c.CreateBoard(ctx)
	if err != nil {
		t.Fatal(err)
	}
	b2, err := c.CreateBoard(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if b1 == b2 || !validBoardID(b1) {
		t.Fatalf("board ids %q %q", b1, b2)
	}
	onBoard := func(boardID, title string) coord.Task {
		t.Helper()
		co, err := c.BoardCoord(boardID)
		if err != nil {
			t.Fatal(err)
		}
		by, err := c.BoardKey(ctx, boardID, r.keyA)
		if err != nil {
			t.Fatal(err)
		}
		task, err := co.CreateTask(ctx, by, coord.NewTask{Title: title})
		if err != nil {
			t.Fatal(err)
		}
		return task
	}
	t1, t2 := onBoard(b1, "board one"), onBoard(b2, "board two")

	check := func(t *testing.T, c *Core) {
		cases := []struct {
			name, taskID, board string
			err                 error
		}{
			{"a private card", private.ID, "", nil},
			{"a card on board one", t1.ID, b1, nil},
			{"a card on board two", t2.ID, b2, nil},
			{"an id in no log", "no-such-task", "", coord.ErrNotFound},
			{"an empty id", "", "", coord.ErrNotFound},
			{"a board id is not a task", b1, "", coord.ErrNotFound},
		}
		for _, tc := range cases {
			t.Run(tc.name, func(t *testing.T) {
				board, co, err := c.Route(tc.taskID)
				if !errors.Is(err, tc.err) || (tc.err == nil) != (err == nil) {
					t.Fatalf("Route(%q): err = %v, want %v", tc.taskID, err, tc.err)
				}
				if err != nil {
					if co != nil {
						t.Fatal("a coord for an id in no log")
					}
					return
				}
				if board != tc.board {
					t.Fatalf("Route(%q) = board %q, want %q", tc.taskID, board, tc.board)
				}
				if _, ok := co.GetTask(tc.taskID); !ok {
					t.Fatalf("the routed coord doesn't hold %q", tc.taskID)
				}
				if (board == "") != (co == c.Coord()) {
					t.Fatal("the private board's cards must route to the private coord, and only they")
				}
				// TaskCoord routes the same way.
				co2, _, err := c.TaskCoord(ctx, tc.taskID)
				if err != nil || co2 != co {
					t.Fatalf("TaskCoord: %v, same coord %v", err, co2 == co)
				}
			})
		}
		// An id in no log goes to the private coord, which says not found.
		co, keys, err := c.TaskCoord(ctx, "no-such-task", r.keyA)
		if err != nil || co != c.Coord() || keys[0] != r.keyA {
			t.Fatalf("TaskCoord of an unknown id: %v", err)
		}
		if _, err := co.SetTaskState(ctx, keys[0], "no-such-task", coord.Completed, ""); !errors.Is(err, coord.ErrNotFound) {
			t.Fatalf("acting on an unknown id: %v", err)
		}
	}
	t.Run("live", func(t *testing.T) { check(t, c) })

	// Each log holds only its own cards.
	for _, l := range []struct {
		st   *store.Store
		want string
	}{{c.store, private.ID}, {mustBoard(t, c, b1).store, t1.ID}, {mustBoard(t, c, b2).store, t2.ID}} {
		ids := createdIDs(t, l.st)
		if len(ids) != 1 || ids[0] != l.want {
			t.Errorf("a log holds task.created for %v, want only %s", ids, l.want)
		}
	}

	// After a restart the boards are found on disk and the index comes back.
	opts := c.opts
	c.Close()
	c2, err := Start(ctx, opts)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c2.Close() })
	if got := c2.Boards(); len(got) != 2 {
		t.Fatalf("Boards after restart = %v", got)
	}
	c2.boards.mu.Lock()
	indexed := c2.boards.taskOf[t1.ID] == b1 && c2.boards.taskOf[t2.ID] == b2
	c2.boards.mu.Unlock()
	if !indexed {
		t.Fatal("the taskId index wasn't rebuilt from the logs")
	}
	t.Run("restarted", func(t *testing.T) { check(t, c2) })
}

// TestBoardKeys: callers become board keys only through BoardKey (§2.4).
// The person at this device is person:<personId>, an agent on this device
// gets one card per board, written once and with nothing that names its
// device, session or folder, and anything else is refused.
func TestBoardKeys(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	b1, _ := c.CreateBoard(ctx)
	b2, _ := c.CreateBoard(ctx)
	me := store.PersonActorPrefix + c.Person().ID
	if k, err := c.BoardKey(ctx, b1, c.UserSender().Key); err != nil || k != me {
		t.Fatalf("the person's key = %q, %v; want %q", k, err, me)
	}
	cardA, err := c.BoardKey(ctx, b1, r.keyA)
	if err != nil || !strings.HasPrefix(cardA, store.AgentActorPrefix) || strings.Contains(cardA, c.Device().ID) {
		t.Fatalf("agent key = %q, %v", cardA, err)
	}
	if again, _ := c.BoardKey(ctx, b1, "api:"+r.keyA); again != cardA {
		t.Fatalf("the same agent got two cards on one board: %q, %q", cardA, again)
	}
	if other, _ := c.BoardKey(ctx, b2, r.keyA); other == cardA || other == "" {
		t.Fatalf("one agent's cards on two boards must differ: %q, %q", cardA, other)
	}
	refused := []string{"user:some-other-device", "dev-elsewhere/session-1", c.Device().ID + "/no-such-session"}
	for _, k := range refused {
		if got, err := c.BoardKey(ctx, b1, k); err == nil {
			t.Errorf("BoardKey(%q) = %q, want an error", k, got)
		}
	}
	if _, err := c.BoardKey(ctx, "0123456789abcdef0123456789abcdef", r.keyA); !errors.Is(err, ErrNoBoard) {
		t.Errorf("an unknown board: %v", err)
	}
	// The card row is the reduced card, and only on its own board.
	b := mustBoard(t, c, b1)
	cards, err := b.store.List(ctx, store.Filter{Kinds: []string{store.KindAgentCard}})
	if err != nil || len(cards) != 1 {
		t.Fatalf("agent.card rows on board one: %d, %v", len(cards), err)
	}
	var row map[string]any
	if err := json.Unmarshal(cards[0].Payload, &row); err != nil {
		t.Fatal(err)
	}
	for k := range row {
		switch k {
		case "cardId", "name", "kind", "personId", "status":
		default:
			t.Errorf("agent.card carries %q", k)
		}
	}
	if cards[0].Actor != cardA || row["personId"] != c.Person().ID || row["name"] != "alpha" {
		t.Errorf("card row %s %v", cards[0].Actor, row)
	}
	// The board_cards mapping stays on the device, and maps back.
	if got := c.PrivateKey(cardA); got != r.keyA {
		t.Errorf("PrivateKey(%q) = %q, want %q", cardA, got, r.keyA)
	}
	if got := c.PrivateKey(me); got != c.UserSender().Key {
		t.Errorf("PrivateKey(me) = %q", got)
	}
	if got := c.PrivateKey("agent:someone-elses"); got != "agent:someone-elses" {
		t.Errorf("PrivateKey of an unknown card = %q", got)
	}
	// Writes through TaskCoord carry board keys; the card's row reads it
	// back as this device's agent.
	co, _ := c.BoardCoord(b1)
	task, err := co.CreateTask(ctx, me, coord.NewTask{Title: "assign me"})
	if err != nil {
		t.Fatal(err)
	}
	rco, keys, err := c.TaskCoord(ctx, task.ID, c.UserSender().Key, r.keyA)
	if err != nil || rco != co || keys[0] != me || keys[1] != cardA {
		t.Fatalf("TaskCoord keys %v, %v", keys, err)
	}
	if _, _, err := rco.Assign(ctx, keys[0], task.ID, keys[1]); err != nil {
		t.Fatal(err)
	}
	if got, _ := co.GetTask(task.ID); len(got.Holders) != 1 || got.Holders[0] != cardA {
		t.Fatalf("holders %v", got.Holders)
	}
	// The card is live while its session is (the board coord's roster).
	live := false
	for _, a := range (boardRoster{c, b}).Agents() {
		live = live || a.Key == cardA && a.Status != "gone"
	}
	if !live {
		t.Fatal("the card of a live session isn't live on the board")
	}
	// The private log never got a board row, and the board log refuses
	// private kinds.
	for _, kind := range []string{"message.sent", store.KindAgentSeen, "capture.turn", store.KindPersonProfile, "task.shared"} {
		if _, err := b.store.Append(ctx, "irori", kind, map[string]any{}); !errors.Is(err, store.ErrNotBoardKind) {
			t.Errorf("board log took %s: %v", kind, err)
		}
	}
	if rows, _ := c.store.List(ctx, store.Filter{Kinds: []string{store.KindAgentCard}}); len(rows) != 0 {
		t.Errorf("the private log holds %d agent.card rows", len(rows))
	}
}

// TestBoardToolsRoute: an agent's board tools act on a shared board's card
// in that board's log, as its card, and a private card stays private.
func TestBoardToolsRoute(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	b1, _ := c.CreateBoard(ctx)
	co, _ := c.BoardCoord(b1)
	me := store.PersonActorPrefix + c.Person().ID
	task, err := co.CreateTask(ctx, me, coord.NewTask{Title: "on the board"})
	if err != nil {
		t.Fatal(err)
	}
	caller := tools.Caller{Sender: inbox.Sender{Key: r.keyA, Name: "alpha", Kind: "claude"}}
	call := func(name string, args map[string]any) string {
		t.Helper()
		raw, _ := json.Marshal(args)
		out, err := c.Tools().Call(ctx, caller, name, raw)
		if err != nil {
			t.Fatal(err)
		}
		return out
	}
	if out := call("board_join", map[string]any{"task_id": "task:" + task.ID}); strings.HasPrefix(out, "Error") {
		t.Fatalf("board_join: %s", out)
	}
	if out := call("board_comment", map[string]any{"task_id": task.ID, "text": "on it"}); strings.HasPrefix(out, "Error") {
		t.Fatalf("board_comment: %s", out)
	}
	card, _ := c.BoardKey(ctx, b1, r.keyA)
	got, _ := co.GetTask(task.ID)
	if len(got.Holders) != 1 || got.Holders[0] != card {
		t.Fatalf("holders %v, want the agent's card %s", got.Holders, card)
	}
	cms := co.Comments(task.ID)
	if len(cms) != 1 || cms[0].By != card {
		t.Fatalf("comments %+v", cms)
	}
	if rows, _ := c.store.List(ctx, store.Filter{Kinds: []string{"task."}}); len(rows) != 0 {
		t.Fatalf("the private log got %d task rows from a board card", len(rows))
	}
	if out := call("board_status", map[string]any{"task_id": "no-such-card", "state": "completed"}); !strings.HasPrefix(out, "Error") {
		t.Fatalf("a card in no log: %s", out)
	}
}

// TestBoardLogsSkipJunk: files in boards/ that aren't board logs are left
// alone at Start.
func TestBoardLogsSkipJunk(t *testing.T) {
	data := t.TempDir()
	dir := filepath.Join(data, BoardsDir)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	for _, f := range []string{"notes.txt", "..db", "ABCDEF0123456789ABCDEF0123456789.db"} {
		if err := os.WriteFile(filepath.Join(dir, f), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	c, err := Start(context.Background(), Options{DataDir: data, ClaudeDir: t.TempDir(), Version: "0.5.0-test", Headless: true,
		Keystore: identity.FileKeystore{}})
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	if got := c.Boards(); len(got) != 0 {
		t.Fatalf("Boards = %v", got)
	}
}

func mustBoard(t *testing.T, c *Core, id string) *boardLog {
	t.Helper()
	b, err := c.board(id)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func createdIDs(t *testing.T, st *store.Store) []string {
	t.Helper()
	evs, err := st.List(context.Background(), store.Filter{Kinds: []string{coord.KindTaskCreated}, Limit: store.MaxListLimit})
	if err != nil {
		t.Fatal(err)
	}
	var out []string
	for _, ev := range evs {
		out = append(out, createdTaskID(ev))
	}
	return out
}
