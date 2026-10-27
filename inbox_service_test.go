package main

import (
	"testing"

	"github.com/StephenSHorton/irori/internal/inbox"
)

type countingInbox struct {
	dels  []inbox.Delivery
	lists int
}

func (f *countingInbox) List(int) []inbox.Delivery {
	f.lists++
	return append([]inbox.Delivery{}, f.dels...)
}

func (f *countingInbox) Policy() inbox.Policy { return inbox.Policy{Paused: true} }

// Each inbox:changed costs one pass over every delivery: the webview's Held
// and Counts after the event read the pump's scan instead of making their own.
func TestInboxOneScanPerChange(t *testing.T) {
	src := &countingInbox{dels: []inbox.Delivery{
		{ID: "a", State: inbox.Held},
		{ID: "b", State: inbox.Queued},
		{ID: "c", State: inbox.Delivered},
		{ID: "d", State: inbox.Held},
	}}
	s := &InboxService{src: src, kick: make(chan struct{}, 1)}

	emitted := s.refresh().counts // what pump sends with inbox:changed
	held := s.Held()
	got := s.Counts()
	if src.lists != 1 {
		t.Fatalf("List called %d times for one change, want 1", src.lists)
	}
	want := InboxCounts{Unread: 2, Held: 2, Paused: true}
	if emitted != want || got != want {
		t.Fatalf("counts = %+v / %+v, want %+v", emitted, got, want)
	}
	if len(held) != 2 || held[0].ID != "a" || held[1].ID != "d" {
		t.Fatalf("held = %+v, want a, d", held)
	}
	held[0].ID = "mutated"
	if s.Held()[0].ID != "a" {
		t.Fatal("Held returned the snapshot's own slice")
	}

	// The next change scans again and is what Held then returns.
	src.dels = src.dels[1:]
	s.refresh()
	if h := s.Held(); len(h) != 1 || h[0].ID != "d" {
		t.Fatalf("held after change = %+v, want d", h)
	}
	if src.lists != 2 {
		t.Fatalf("List called %d times for two changes, want 2", src.lists)
	}
}

// Before the pump's first scan, Held and Counts scan on demand.
func TestInboxScansBeforeFirstChange(t *testing.T) {
	src := &countingInbox{dels: []inbox.Delivery{{ID: "a", State: inbox.Held}}}
	s := &InboxService{src: src}
	if c := s.Counts(); c.Held != 1 {
		t.Fatalf("counts = %+v, want one held", c)
	}
	if h := s.Held(); len(h) != 1 {
		t.Fatalf("held = %+v", h)
	}
	if src.lists != 1 {
		t.Fatalf("List called %d times, want 1", src.lists)
	}
}
