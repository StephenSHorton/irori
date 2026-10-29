package core

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

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
	b1, err := c.CreateBoard(ctx, "one", "one")
	if err != nil {
		t.Fatal(err)
	}
	b2, err := c.CreateBoard(ctx, "two", "two")
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
	b1, _ := c.CreateBoard(ctx, "one", "one")
	b2, _ := c.CreateBoard(ctx, "two", "two")
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
	b1, _ := c.CreateBoard(ctx, "one", "one")
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

// TestBoardIdentityKeptCurrent: a board this device creates pins this
// person with this device as the board root and carries the person's
// member.* rows (§3), and they follow the private log: a device certified
// there is certified on the board, a revoke cuts it with a cut over the
// board's own stream, and the rotation that follows goes on the board with
// its prevSig. Every row on the board verifies against the board's own
// chain, the person is the owner of an agent's lease there, and all of it
// survives a restart.
func TestBoardIdentityKeptCurrent(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	id, err := c.CreateBoard(ctx, "mlg", "mlg")
	if err != nil {
		t.Fatal(err)
	}
	me := c.ident.Current()
	b := mustBoard(t, c, id)
	if pin, ok := b.verifier.Pinned(me.ID); !ok || pin.RootDeviceID != c.device.ID || pin.PersonEpoch != me.Epoch {
		t.Fatalf("pin %+v %v", pin, ok)
	}
	m, err := c.BoardMember(id, me.ID)
	if err != nil || len(m.Devices) != 1 || m.Devices[0].Cert.DeviceID != c.device.ID || m.Name != me.Name {
		t.Fatalf("member %+v, %v", m, err)
	}

	// An agent's row on the board is this person's.
	by, err := c.BoardKey(ctx, id, r.keyA)
	if err != nil {
		t.Fatal(err)
	}
	if p := b.verifier.KeyPerson(by); p != me.ID {
		t.Fatalf("%s is %q's", by, p)
	}
	co, _ := c.BoardCoord(id)
	if _, err := co.Acquire(ctx, by, "port:5173", time.Hour, false); err != nil {
		t.Fatal(err)
	}
	if err := co.Release(ctx, by, "port:5173"); err != nil {
		t.Fatal(err)
	}
	if co.IgnoredRows() != 0 || len(co.Leases()) != 0 {
		t.Fatalf("own lease rows: ignored %d, leases %v", co.IgnoredRows(), co.Leases())
	}

	// A device paired in the private log.
	devPub := hex.EncodeToString(mustKey(t))
	add, err := c.ident.CertFor("dev-x", devPub, "laptop", identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.store.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	wait(t, "the paired device is certified on the board", func() bool { _, ok := b.verifier.Cert("dev-x"); return ok })

	// Revoked, then the key rotates (mesh.Revoke's two rows).
	rev, err := c.ident.RevokeOf("dev-x", 0, "", "lost", time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.store.Append(ctx, "irori", store.KindPersonDeviceRevoked, rev); err != nil {
		t.Fatal(err)
	}
	rot, err := c.ident.Rotate(time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.store.Append(ctx, "irori", store.KindPersonKeyRotated, rot); err != nil {
		t.Fatal(err)
	}
	wait(t, "the revoke and rotation reach the board", func() bool {
		_, cut := b.verifier.Cut("dev-x")
		return cut && b.verifier.Epoch(me.ID) == rot.Epoch
	})
	if cut, _ := b.verifier.Cut("dev-x"); cut != 0 {
		t.Fatalf("dev-x's board cut %d: it wrote nothing on the board", cut)
	}
	// The member device revoke rotates the board secret (§6.1): a new
	// epoch, a new topic, the previous kept open until every device has it.
	wait(t, "the board secret rotates", func() bool {
		info, err := c.BoardInfo(id)
		return err == nil && info.Epoch == 2 && info.SecretEpoch == 2
	})
	topics, err := b.secrets.Topics()
	if err != nil || len(topics) != 2 || topics[0].Epoch != 2 || topics[1].Epoch != 1 || topics[0].Topic == topics[1].Topic {
		t.Fatalf("topics after the rotation %+v, %v", topics, err)
	}

	// Every row on the board verifies by the board's chain alone.
	check := func(t *testing.T, c *Core) {
		t.Helper()
		b := mustBoard(t, c, id)
		evs, err := b.store.List(ctx, store.Filter{Limit: store.MaxListLimit})
		if err != nil {
			t.Fatal(err)
		}
		kinds := map[string]int{}
		for _, ev := range evs {
			kinds[ev.Kind]++
			if err := b.verifier.VerifyRow(ev); err != nil {
				t.Errorf("%s/%d %s: %v", ev.DeviceID, ev.Seq, ev.Kind, err)
			}
		}
		if kinds[store.KindMemberDeviceAdded] != 2 || kinds[store.KindMemberDeviceRevoked] != 1 || kinds[store.KindMemberKeyRotated] != 1 || kinds[store.KindMemberProfile] != 1 {
			t.Errorf("member rows %v", kinds)
		}
		if kinds[store.KindBoardCreated] != 1 {
			t.Errorf("board rows %v", kinds)
		}
		if info, err := c.BoardInfo(id); err != nil || info.Epoch != 2 || info.SecretEpoch != 2 || info.Tag != "mlg" {
			t.Errorf("info %+v, %v", info, err)
		}
		m, err := c.BoardMember(id, me.ID)
		if err != nil || m.Epoch != rot.Epoch || len(m.Devices) != 2 {
			t.Fatalf("member %+v, %v", m, err)
		}
	}
	check(t, c)
	opts := c.opts
	c.Close()
	c2, err := Start(ctx, opts)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c2.Close() })
	t.Run("restarted", func(t *testing.T) { check(t, c2) })
}

// TestCreateBoardRows: CreateBoard writes the self-cert first and then
// board.created, which makes this person the board's one owner; tags are
// checked and unique on the device; an owner renames; the board's secret
// is in a sealed file beside the log and in no row (§2.1, §4.2, §6.1).
func TestCreateBoardRows(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	for _, tc := range []struct{ name, tag string }{{"", "ok"}, {"ok", ""}, {"ok", "Mlg"}, {"ok", "12"}, {"ok", "a#b"}} {
		if _, err := c.CreateBoard(ctx, tc.name, tc.tag); !errors.Is(err, coord.ErrInvalid) {
			t.Errorf("CreateBoard(%q, %q): err = %v", tc.name, tc.tag, err)
		}
	}
	id, err := c.CreateBoard(ctx, "  MLG farming ", "mlg")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.CreateBoard(ctx, "another", "mlg"); !errors.Is(err, coord.ErrInvalid) {
		t.Fatalf("a second board with tag mlg: err = %v", err)
	}
	b := mustBoard(t, c, id)
	evs, err := b.store.List(ctx, store.Filter{DeviceID: c.device.ID, Limit: store.MaxListLimit})
	if err != nil {
		t.Fatal(err)
	}
	if len(evs) < 2 || evs[0].Kind != store.KindMemberDeviceAdded || evs[len(evs)-1].Kind != store.KindBoardCreated {
		var kinds []string
		for _, ev := range evs {
			kinds = append(kinds, ev.Kind)
		}
		t.Fatalf("this device's stream %v: want the self-cert first, board.created after its member rows", kinds)
	}
	me := c.chain.PersonID()
	info, err := c.BoardInfo(id)
	if err != nil {
		t.Fatal(err)
	}
	if info.Name != "MLG farming" || info.Tag != "mlg" || info.Creator != me || len(info.Members) != 1 ||
		info.Members[0].Role != identity.BoardOwner || info.Members[0].State != identity.MemberActive || len(info.Conflicts) != 0 {
		t.Fatalf("info %+v", info)
	}
	if info.Epoch != 1 || info.SecretEpoch != 1 {
		t.Fatalf("epochs %d/%d, want 1/1", info.Epoch, info.SecretEpoch)
	}
	if err := c.RenameBoard(ctx, id, "mlg & co"); err != nil {
		t.Fatal(err)
	}
	if err := c.RenameBoard(ctx, id, ""); !errors.Is(err, coord.ErrInvalid) {
		t.Fatalf("an empty name: err = %v", err)
	}
	if info, _ := c.BoardInfo(id); info.Name != "mlg & co" {
		t.Fatalf("renamed to %q", info.Name)
	}
	// The secret: in its sealed file, not in any row of the board or the
	// private log.
	secret, _, ok := b.secrets.Secret(1)
	if !ok {
		t.Fatal("no epoch 1 secret")
	}
	if _, err := os.Stat(filepath.Join(c.opts.DataDir, BoardsDir, id+".secrets")); err != nil {
		t.Fatal(err)
	}
	needle := []byte(hex.EncodeToString(secret))
	for _, st := range []*store.Store{b.store, c.store} {
		rows, err := st.List(ctx, store.Filter{Limit: store.MaxListLimit})
		if err != nil {
			t.Fatal(err)
		}
		for _, ev := range rows {
			if strings.Contains(string(ev.Payload), string(needle)) {
				t.Fatalf("the board secret is in a %s row", ev.Kind)
			}
		}
	}
	// It all survives a restart.
	opts := c.opts
	c.Close()
	c2, err := Start(ctx, opts)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c2.Close() })
	if info, err := c2.BoardInfo(id); err != nil || info.Name != "mlg & co" || !mustBoard(t, c2, id).verifier.Membership().Owner(me) || info.SecretEpoch != 1 {
		t.Fatalf("after restart %+v, %v", info, err)
	}
}

func mustKey(t *testing.T) ed25519.PublicKey {
	t.Helper()
	pub, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	return pub
}
