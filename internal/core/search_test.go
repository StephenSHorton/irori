package core

import (
	"context"
	"slices"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/search"
)

// Core registers coord and the inbox as capture's search sources, so a
// Board card and a message are found and shown through capture.
func TestSearchReachesBoardAndInbox(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	task, err := r.core.Coord().CreateTask(ctx, r.keyA, coord.NewTask{Title: "Feed the pangolin"})
	if err != nil {
		t.Fatal(err)
	}
	m, _, err := r.core.Inbox().Send(ctx, r.sender(r.keyA, "alpha"), inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "the pangolin is hungry", "")
	if err != nil {
		t.Fatal(err)
	}
	hits, err := r.core.Capture().Search(ctx, search.Query{Text: "pangolin"})
	if err != nil {
		t.Fatal(err)
	}
	var got []string
	for _, h := range hits {
		got = append(got, h.Ref)
	}
	for _, want := range []string{"task:" + task.ID, "msg:" + m.ID} {
		if !slices.Contains(got, want) {
			t.Fatalf("hits %v lack %s", got, want)
		}
		if _, err := r.core.Capture().Show(ctx, want, search.ShowOptions{}); err != nil {
			t.Fatalf("show %s: %v", want, err)
		}
	}
}
