package main

import "testing"

// The status line names the cards waiting on a decision (every device's)
// before the inbox counts, and the pause checkbox names this device.
func TestTrayLines(t *testing.T) {
	cases := []struct {
		c      InboxCounts
		decide int
		want   string
	}{
		{InboxCounts{}, 0, "2 agents"},
		{InboxCounts{Unread: 1}, 0, "2 agents · 1 unread"},
		{InboxCounts{Unread: 1, Held: 2}, 3, "2 agents · 3 need a decision · 1 unread · 2 held"},
		{InboxCounts{}, 1, "2 agents · 1 need a decision"},
	}
	for _, c := range cases {
		if got := statusLine("2 agents", c.c, c.decide); got != c.want {
			t.Errorf("statusLine(%+v, %d) = %q, want %q", c.c, c.decide, got, c.want)
		}
	}
	if got := pauseLabel("Stephen's PC"); got != "Pause wakes on Stephen's PC" {
		t.Errorf("pauseLabel = %q", got)
	}
	if got := pauseLabel(""); got != "Pause wakes on this device" {
		t.Errorf("pauseLabel(\"\") = %q", got)
	}
}
