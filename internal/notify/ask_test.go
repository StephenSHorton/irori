package notify

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/ask"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// An open ask (v0.4 §6.7) gets a card with its buttons; pressing one
// answers it as the person, and the card clears once the ask resolves.
// One waiting from before Watch starts gets its card too.
func TestAskCardsAnswerAndClear(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	book, err := ask.Start(ctx, ask.Options{Store: st, Self: "dev1"})
	if err != nil {
		t.Fatal(err)
	}
	resolved := make(chan ask.Ask, 4)
	book.OnResolved(func(a ask.Ask) { resolved <- a })
	early, err := book.Post(ctx, ask.Ask{By: keyA, Name: "alpha", Title: "Deploy?", Text: "Ship it now?", Actions: []string{"Yes", "No"}, Until: time.Now().Add(time.Minute)})
	if err != nil {
		t.Fatal(err)
	}
	f := newFake()
	n := New(Config{Toaster: f})
	go Watch(ctx, n, Sources{Asks: book, User: user})
	waitFor(t, "the waiting ask's card", func() bool { return f.isLive(AskID(early.ID)) })
	if c := f.card(t, AskID(early.ID)); c.Title != "alpha: Deploy?" || len(c.Actions) != 2 {
		t.Fatalf("card %+v", c)
	}
	f.press(t, AskID(early.ID), "No")
	select {
	case a := <-resolved:
		if a.ID != early.ID || a.Choice != "No" || !a.Answered {
			t.Fatalf("resolved %+v", a)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the answer didn't resolve the ask")
	}
	waitFor(t, "the card clears", func() bool { return !f.isLive(AskID(early.ID)) })

	// A dismiss is an answer with no choice.
	late, _ := book.Post(ctx, ask.Ask{By: keyB, Text: "Later?", Actions: []string{"Ok"}, Until: time.Now().Add(time.Minute)})
	waitFor(t, "the new ask's card", func() bool { return f.isLive(AskID(late.ID)) })
	f.dismiss(t, AskID(late.ID))
	select {
	case a := <-resolved:
		if a.ID != late.ID || a.Choice != "" || !a.Answered {
			t.Fatalf("dismissed %+v", a)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the dismiss didn't resolve the ask")
	}
}

func waitFor(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
}
