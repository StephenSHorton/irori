package core

import (
	"context"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

// waitFor polls cond for up to 10 s.
func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for !cond() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out: %s", what)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// pairedCores starts two Cores of one person on one fake net, B certified
// by A as pairing does (§3 step 6), and waits until their logs cross.
func pairedCores(t *testing.T) (a, b *Core, aDir, bDir string) {
	t.Helper()
	ctx := context.Background()
	net := mesh.NewFakeNet()
	aDir, bDir = t.TempDir(), t.TempDir()
	a = startMeshCore(t, net, aDir)
	raw, err := os.ReadFile(filepath.Join(aDir, identity.PersonFile))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bDir, identity.PersonFile), raw, 0o600); err != nil {
		t.Fatal(err)
	}
	b = startMeshCore(t, net, bDir)
	add, err := a.ident.CertFor(b.Device().ID, hex.EncodeToString(b.ident.DevicePub()), "mac", identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.store.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	person, _ := a.store.List(ctx, store.Filter{Kinds: []string{"person."}})
	for range 2 {
		for _, ev := range person {
			_ = b.chain.Learn(ev)
		}
	}
	b.Mesh().Kick()
	waitFor(t, "a and b pair", func() bool { return a.IsOwner("user:"+b.Device().ID) && b.IsOwner("user:"+a.Device().ID) })
	return a, b, aDir, bDir
}

// TestBoardOnOwnDevices: step 4's one person, two devices (§12 step 4). A
// board A makes opens on B from the private board.joined row, pinned to
// the creator it names; its rows reach B on the board stream, B's cert
// with them, so B writes on the board as the person; and what B writes
// reaches A. The board's rows never enter either private log, and the
// tagged ref resolves on both.
func TestBoardOnOwnDevices(t *testing.T) {
	ctx := context.Background()
	a, b, _, bDir := pairedCores(t)
	id, err := a.CreateBoard(ctx, "MLG", "mlg")
	if err != nil {
		t.Fatal(err)
	}
	waitFor(t, "b opens the board", func() bool { _, err := b.board(id); return err == nil })
	if _, err := os.Stat(filepath.Join(bDir, BoardsDir, id+".db")); err != nil {
		t.Fatalf("b has no board file: %v", err)
	}
	bb := mustBoard(t, b, id)
	waitFor(t, "b's board.created and cert arrive", func() bool {
		_, certified := bb.verifier.Cert(b.Device().ID)
		return bb.verifier.Membership().Created && certified
	})
	if got := b.BoardList(); len(got) != 1 || got[0].Name != "MLG" || got[0].Tag != "mlg" || !got[0].Owner || !got[0].Ready {
		t.Fatalf("b's boards %+v", got)
	}
	// The first secret is the creating device's alone (§6.1); b gets it
	// in a board.secret frame once step 5 sends them.
	if ai, _ := a.BoardInfo(id); ai.SecretEpoch != 1 {
		t.Fatalf("a's secret epoch %d", ai.SecretEpoch)
	}
	if bi, _ := b.BoardInfo(id); bi.SecretEpoch != 0 {
		t.Fatalf("b made its own first secret (epoch %d)", bi.SecretEpoch)
	}

	// B writes a card on the board as the person; it reaches A.
	by, err := b.BoardKey(ctx, id, b.UserSender().Key)
	if err != nil {
		t.Fatal(err)
	}
	card, err := bb.coord.CreateTask(ctx, by, coord.NewTask{Title: "from the mac"})
	if err != nil {
		t.Fatal(err)
	}
	if card.ShortRef() != "mlg#1" {
		t.Fatalf("b's card ref %q", card.ShortRef())
	}
	ab := mustBoard(t, a, id)
	waitFor(t, "a gets b's card", func() bool { _, ok := ab.coord.GetTask(card.ID); return ok })
	if got, err := a.TaskRef("mlg#1"); err != nil || got != card.ID {
		t.Fatalf("a resolves mlg#1 to %q, %v", got, err)
	}

	// A's card reaches B the same way.
	aby, _ := a.BoardKey(ctx, id, a.UserSender().Key)
	other, err := ab.coord.CreateTask(ctx, aby, coord.NewTask{Title: "from the pc"})
	if err != nil {
		t.Fatal(err)
	}
	waitFor(t, "b gets a's card", func() bool { _, ok := bb.coord.GetTask(other.ID); return ok })

	// Nothing of the board is in either private log.
	for _, c := range []*Core{a, b} {
		evs, err := c.store.List(ctx, store.Filter{Kinds: []string{coord.KindTaskCreated, store.KindBoardCreated, "member."}, Limit: store.MaxListLimit})
		if err != nil {
			t.Fatal(err)
		}
		for _, ev := range evs {
			if createdTaskID(ev) == card.ID || createdTaskID(ev) == other.ID || strings.HasPrefix(ev.Kind, "board.created") || strings.HasPrefix(ev.Kind, "member.") {
				t.Fatalf("%s's private log holds a board row %s", c.Device().ID, ev.Kind)
			}
		}
	}

	// Leave on B forgets the board on both devices.
	if err := b.LeaveBoard(ctx, id); err != nil {
		t.Fatal(err)
	}
	if _, err := b.board(id); err == nil {
		t.Fatal("b still holds the board it left")
	}
	waitFor(t, "a forgets the board b left", func() bool { _, err := a.board(id); return err != nil })
	if _, err := os.Stat(filepath.Join(bDir, BoardsDir, id+".db")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("b's board file is still there: %v", err)
	}
}

// TestForgetBoardStaysForgotten: Forget drops a board on this device only
// (§4.6), and the board.joined it was opened from doesn't bring it back,
// at a reconcile or a restart.
func TestForgetBoardStaysForgotten(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	id, err := c.CreateBoard(ctx, "notes", "notes")
	if err != nil {
		t.Fatal(err)
	}
	if err := c.ForgetBoard(ctx, id); err != nil {
		t.Fatal(err)
	}
	if _, err := c.board(id); err == nil {
		t.Fatal("the board is still open")
	}
	c.reconcileBoards(ctx)
	if _, err := c.board(id); err == nil {
		t.Fatal("board.joined brought a forgotten board back")
	}
	for _, name := range []string{id + ".db", id + ".secrets"} {
		if _, err := os.Stat(filepath.Join(r.data, BoardsDir, name)); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("%s is still there: %v", name, err)
		}
	}
	if len(c.BoardList()) != 0 {
		t.Fatalf("boards %+v", c.BoardList())
	}
}

// TestShareAndWithdraw: Share to board (§5.2) writes a fresh card with the
// scrubbed fields and steps, closes the private card with a note naming
// the shared copy, and refuses writes to the private id with that name;
// Withdraw (§5.3) cancels the shared card and brings the private one back.
func TestShareAndWithdraw(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	id, err := c.CreateBoard(ctx, "MLG", "mlg")
	if err != nil {
		t.Fatal(err)
	}
	priv, err := c.Coord().CreateTask(ctx, r.keyA, coord.NewTask{Title: `fix C:\Users\stephen\mlg\x.go`,
		Objective: "the build in ~/projects/mlg breaks", DoneCriteria: "it builds",
		Links: []string{"https://github.com/StephenSHorton/irori/pull/1", `C:\Users\stephen\notes.txt`},
		Steps: []coord.Step{{Text: "read /home/me/logs/a.txt"}, {Text: "fix it"}}})
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.Coord().AddComment(ctx, r.keyA, priv.ID, "private thoughts", false); err != nil {
		t.Fatal(err)
	}
	p, err := c.PreviewShare(priv.ID, id)
	if err != nil {
		t.Fatal(err)
	}
	if p.Title != "fix [path]" || p.Goal != "the build in [path] breaks" || p.Steps[0] != "read [path]" || p.Steps[1] != "fix it" {
		t.Fatalf("preview %+v", p)
	}
	if len(p.Links) != 1 || !strings.HasPrefix(p.Links[0], "https://") || len(p.Dropped) == 0 || len(p.VisibleTo) != 1 {
		t.Fatalf("preview links/dropped/visible %+v", p)
	}
	// What the person confirmed is what goes: an edit after the preview
	// (an agent holding the card) is refused until it is previewed again.
	if _, err := c.Coord().SetSteps(ctx, r.keyA, priv.ID, []coord.Step{{Text: "read /home/me/logs/a.txt"}, {Text: "fix it"}, {Text: "post the plan"}}); err != nil {
		t.Fatal(err)
	}
	if _, err := c.ShareToBoard(ctx, priv.ID, id, p.Hash); !errors.Is(err, ErrPreviewStale) {
		t.Fatalf("a share of a card edited after its preview: %v", err)
	}
	if p, err = c.PreviewShare(priv.ID, id); err != nil || len(p.Steps) != 3 {
		t.Fatalf("preview again %+v %v", p, err)
	}
	shared, err := c.ShareToBoard(ctx, priv.ID, id, p.Hash)
	if err != nil {
		t.Fatal(err)
	}
	if shared.Title != p.Title || shared.ShortRef() != "mlg#1" || len(shared.Steps) != 3 {
		t.Fatalf("shared card %+v", shared)
	}
	if co, _ := c.BoardCoord(id); len(co.Comments(shared.ID)) != 0 {
		t.Fatal("the private comments crossed")
	}
	if t2, _ := c.Coord().GetTask(priv.ID); t2.State != coord.Canceled || !strings.Contains(t2.Note, "mlg#1") {
		t.Fatalf("private card after share %+v", t2)
	}
	_, _, err = c.TaskCoord(ctx, priv.ID, r.keyA)
	if !errors.Is(err, ErrShared) || !strings.Contains(err.Error(), "mlg#1") || !strings.Contains(err.Error(), "visible to") {
		t.Fatalf("a write to the private id: %v", err)
	}
	if _, err := c.ShareToBoard(ctx, priv.ID, id, p.Hash); err == nil {
		t.Fatal("a card was shared twice")
	}
	sc, ok := c.SharedCopyOf(priv.ID)
	if !ok || sc.Ref != "mlg#1" || sc.BoardName != "MLG" {
		t.Fatalf("shared copy %+v %v", sc, ok)
	}

	if _, err := c.WithdrawCard(ctx, shared.ID); err != nil {
		t.Fatal(err)
	}
	co, _ := c.BoardCoord(id)
	if w, _ := co.GetTask(shared.ID); !w.Withdrawn || w.State != coord.Canceled {
		t.Fatalf("withdrawn card %+v", w)
	}
	if back, _ := c.Coord().GetTask(priv.ID); coord.IsTerminal(back.State) {
		t.Fatalf("the private card stayed closed: %+v", back)
	}
	if _, _, err := c.TaskCoord(ctx, priv.ID, r.keyA); err != nil {
		t.Fatalf("a write to the private id after withdraw: %v", err)
	}
	if _, err := c.WithdrawCard(ctx, priv.ID); err == nil {
		t.Fatal("a private card was withdrawn")
	}
}

// TestTaskRefAcrossLogs: B3's rules (§2.5). A tagged ref names a card on
// that board only; a bare number works when one log has it and is refused,
// naming each card, when several do.
func TestTaskRefAcrossLogs(t *testing.T) {
	ctx := context.Background()
	r := newRig(t)
	c := r.core
	p1, _ := c.Coord().CreateTask(ctx, r.keyA, coord.NewTask{Title: "private one"})
	p2, _ := c.Coord().CreateTask(ctx, r.keyA, coord.NewTask{Title: "private two"})
	id, err := c.CreateBoard(ctx, "MLG", "mlg")
	if err != nil {
		t.Fatal(err)
	}
	co, _ := c.BoardCoord(id)
	by, _ := c.BoardKey(ctx, id, c.UserSender().Key)
	s1, err := co.CreateTask(ctx, by, coord.NewTask{Title: "shared one"})
	if err != nil {
		t.Fatal(err)
	}
	for ref, want := range map[string]string{"mlg#1": s1.ID, "MLG#1": s1.ID, "task:mlg#1": s1.ID, "#2": p2.ID, "2": p2.ID, p1.ID: p1.ID} {
		if got, err := c.TaskRef(ref); err != nil || got != want {
			t.Errorf("TaskRef(%q) = %q, %v; want %q", ref, got, err, want)
		}
	}
	if _, err := c.TaskRef("#1"); !errors.Is(err, coord.ErrAmbiguous) || !strings.Contains(err.Error(), "mlg#1") || !strings.Contains(err.Error(), "#1 ") {
		t.Fatalf("bare #1 on two logs: %v", err)
	}
	if _, err := c.TaskRef("mlg#9"); !errors.Is(err, coord.ErrNotFound) {
		t.Fatalf("mlg#9: %v", err)
	}
	if _, err := c.TaskRef("xyz#1"); !errors.Is(err, coord.ErrNotFound) {
		t.Fatalf("xyz#1: %v", err)
	}
}
