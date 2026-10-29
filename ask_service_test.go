package main

import (
	"context"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/presence"
)

// The window's ask (board.md "Asks"): the prompt box posts it, the room
// hears it from the person, the tile says where it stands, and Close ends
// it with nothing coming of it.
func TestBoardAsk(t *testing.T) {
	b, c, got := testBoard(t, nil)
	b.agents = func() []presence.AgentCard {
		return []presence.AgentCard{
			{Key: agentA, Status: "idle", RepoKey: "remote:github.com/a/irori"},
			{Key: agentB, Status: "busy", RepoKey: "remote:github.com/a/irori"},
		}
	}
	res, err := b.Ask("Why is the build slow?\nSince Tuesday.", "", false)
	if err != nil {
		t.Fatal(err)
	}
	if len(*got) != 1 || !(*got)[0].n.Person || (*got)[0].n.To[0] != agentA || res.Told.Mode != "next_turn" {
		t.Fatalf("routed %+v, told %+v", *got, res.Told)
	}
	cs := b.Cards()
	if len(cs) != 1 || cs[0].Ask == nil || cs[0].Ask.State != AskOpen || cs[0].Ask.Told != 1 || cs[0].Title != "Why is the build slow?" {
		t.Fatalf("tile %+v", cs)
	}
	if _, err := c.AskClaim(context.Background(), agentA, res.Task.ID, time.Second); err != nil {
		t.Fatal(err)
	}
	d, err := b.Card(res.Task.ID)
	if err != nil || d.Ask.State != AskTaken || d.Ask.Taker.Key != agentA {
		t.Fatalf("page %+v %v", d.Ask, err)
	}
	if rs := b.AskRepos(); len(rs) != 1 || rs[0].Label != "irori" {
		t.Fatalf("repos %+v", rs)
	}
	if _, err := b.CloseAsk(res.Task.ID, ""); err != nil {
		t.Fatal(err)
	}
	if x, _ := c.GetTask(res.Task.ID); x.State != coord.Canceled || x.Ask.Outcome.Note != "closed by the person" {
		t.Fatalf("closed %+v", x)
	}
	if len(b.Cards()) != 0 {
		t.Fatal("a closed ask stays on the grid")
	}
	// Open asks sort before every other card.
	two, _ := b.Ask("anyone?", "", true)
	time.Sleep(2 * time.Millisecond)
	if _, err := c.CreateTask(context.Background(), agentA, coord.NewTask{Title: "newer"}); err != nil {
		t.Fatal(err)
	}
	if cs := b.Cards(); len(cs) != 2 || cs[0].ID != two.Task.ID {
		t.Fatalf("order %+v", cs)
	}
}
