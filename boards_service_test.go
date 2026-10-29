package main

import (
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/identity"
)

// TestBoardsInTheWindow: the grid holds every board's live cards beside
// the private ones, each with its board's name, id and tag (v0.5 §8); a
// shared card's page says who sees it and offers Withdraw; a private card
// shared to a board says where it went; a withdrawn card leaves the grid.
func TestBoardsInTheWindow(t *testing.T) {
	c := startCore(t, core.Options{DataDir: t.TempDir(), ClaudeDir: t.TempDir(), Version: "0.5.0-test", Headless: true,
		Keystore: identity.FileKeystore{}})
	b := newBoardService(c)
	boards := &BoardsService{c: c}
	if _, err := b.Create(NewCard{Title: "stays private"}); err != nil {
		t.Fatal(err)
	}
	priv, err := b.Create(NewCard{Title: "goes to the board"})
	if err != nil {
		t.Fatal(err)
	}
	mlg, err := boards.Create("MLG", "MLG")
	if err != nil {
		t.Fatal(err)
	}
	if mlg.Tag != "mlg" || mlg.Name != "MLG" || !mlg.Owner || len(mlg.People) != 1 {
		t.Fatalf("board %+v", mlg)
	}
	p, err := boards.SharePreview(priv.Task.ID, mlg.BoardID)
	if err != nil || p.Title != "goes to the board" || len(p.VisibleTo) != 1 {
		t.Fatalf("preview %+v, %v", p, err)
	}
	sharedID, err := boards.Share(priv.Task.ID, mlg.BoardID)
	if err != nil {
		t.Fatal(err)
	}
	var shared CardSummary
	privates := 0
	for _, s := range b.Cards() {
		switch {
		case s.ID == sharedID:
			shared = s
		case s.Scope == ScopeMine:
			privates++
		}
	}
	if shared.Board != mlg.BoardID || shared.Tag != "mlg" || shared.Short != "1" || shared.Scope != ScopeBoard || shared.ScopeLabel != "MLG" {
		t.Fatalf("shared card in the grid %+v", shared)
	}
	if privates != 1 {
		t.Fatalf("%d private cards in the grid, want 1 (the shared one's private card is closed)", privates)
	}
	d, err := b.Card(sharedID)
	if err != nil {
		t.Fatal(err)
	}
	if d.OnBoard == nil || d.OnBoard.Name != "MLG" || !d.OnBoard.CanWithdraw || len(d.OnBoard.VisibleTo) != 1 {
		t.Fatalf("shared card's board line %+v", d.OnBoard)
	}
	pd, err := b.Card(priv.Task.ID)
	if err != nil {
		t.Fatal(err)
	}
	if pd.SharedTo == nil || pd.SharedTo.Ref != "mlg#1" {
		t.Fatalf("private card's shared-to %+v", pd.SharedTo)
	}
	// A write from the window to the private id is refused, naming mlg#1.
	if _, err := b.Comment(priv.Task.ID, "more", string(TellNone)); err == nil || !strings.Contains(err.Error(), "mlg#1") {
		t.Fatalf("a comment on the shared private card: %v", err)
	}
	// The person comments on the shared card from the window.
	if _, err := b.Comment(sharedID, "on the board", string(TellNone)); err != nil {
		t.Fatal(err)
	}
	co, _ := c.BoardCoord(mlg.BoardID)
	if cs := co.Comments(sharedID); len(cs) != 1 || !strings.HasPrefix(cs[0].By, "person:") {
		t.Fatalf("board comments %+v", cs)
	}
	if _, err := boards.Withdraw(sharedID); err != nil {
		t.Fatal(err)
	}
	for _, s := range b.Cards() {
		if s.ID == sharedID {
			t.Fatal("a withdrawn card is still in the grid")
		}
	}
	if task, _ := c.Coord().GetTask(priv.Task.ID); coord.IsTerminal(task.State) {
		t.Fatalf("the private card stayed closed after withdraw: %s", task.State)
	}
	if err := boards.Leave(mlg.BoardID); err != nil {
		t.Fatal(err)
	}
	if len(boards.List()) != 0 {
		t.Fatalf("boards after leave %+v", boards.List())
	}
}
