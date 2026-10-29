package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
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

// A screenshot pasted into the ask box goes on the ask's card as its first
// comment's file, and the temporary copy is gone afterwards (card #25).
func TestBoardAskWithFiles(t *testing.T) {
	b, c, got := testBoard(t, nil)
	b.agents = func() []presence.AgentCard { return []presence.AgentCard{{Key: agentA, Status: "idle"}} }
	data := t.TempDir()
	var seen []string
	b.attach = func(_ context.Context, by string, paths []string) ([]blob.FileRef, error) {
		var out []blob.FileRef
		for _, p := range paths {
			real, err := blob.CheckAttachPath(p, "", data) // the never-leave rules the Core applies
			if err != nil {
				return nil, err
			}
			raw, err := os.ReadFile(real)
			if err != nil {
				return nil, err
			}
			seen = append(seen, p)
			out = append(out, blob.FileRef{SHA256: strings.Repeat("a", 64), Name: filepath.Base(p), Size: int64(len(raw))})
		}
		return out, nil
	}
	png := []byte("\x89PNG\r\n\x1a\nnot really")
	res, err := b.AskWithFiles("What's wrong in this screenshot?", "", false,
		[]UploadFile{{Name: "screenshot.png", Data: png}, {Name: `..\..\screenshot.png`, Data: png}})
	if err != nil {
		t.Fatal(err)
	}
	if len(seen) != 2 || filepath.Base(seen[0]) != "screenshot.png" || filepath.Base(seen[1]) != "screenshot.png" {
		t.Fatalf("stored %v", seen)
	}
	if _, err := os.Stat(seen[0]); !os.IsNotExist(err) {
		t.Fatalf("the temporary copy is still there: %v", err)
	}
	if len(*got) != 1 || len((*got)[0].n.Attachments) != 2 {
		t.Fatalf("the ask's notice carries no files: %+v", *got)
	}
	cs := c.Comments(res.Task.ID)
	if len(cs) != 1 || len(cs[0].Attachments) != 2 || cs[0].Attachments[0].Name != "screenshot.png" {
		t.Fatalf("comments %+v", cs)
	}
	if _, err := b.AskWithFiles("too big", "", false, []UploadFile{{Name: "big.bin", Data: make([]byte, MaxUpload+1)}}); err == nil {
		t.Fatal("a file over MaxUpload was taken")
	}
}
