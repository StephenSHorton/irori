package presence

import (
	"strings"
	"testing"
	"time"
)

func TestParseLstart(t *testing.T) {
	cases := []struct {
		in   string
		want time.Time
		ok   bool
	}{
		{"Mon Oct 27 14:29:03 2026", time.Date(2026, 10, 27, 14, 29, 3, 0, time.UTC), true},
		{"Wed Oct  7 09:05:00 2026", time.Date(2026, 10, 7, 9, 5, 0, 0, time.UTC), true},
		{"  Wed Oct  7 09:05:00 2026\n", time.Date(2026, 10, 7, 9, 5, 0, 0, time.UTC), true},
		{"134375324500655525", time.Time{}, false},
		{"", time.Time{}, false},
	}
	for _, c := range cases {
		got, ok := parseLstart(c.in)
		if ok != c.ok || !got.Equal(c.want) {
			t.Errorf("parseLstart(%q) = %v, %v; want %v, %v", c.in, got, ok, c.want, c.ok)
		}
	}
	s, ok := readSession(strings.NewReader(`{"pid":7,"sessionId":"a","procStart":"Mon Oct 27 14:29:03 2026"}`))
	if !ok || s.ProcStart != uint64(time.Date(2026, 10, 27, 14, 29, 3, 0, time.UTC).Unix()) {
		t.Fatalf("readSession lstart procStart = %d (ok %v)", s.ProcStart, ok)
	}
}
