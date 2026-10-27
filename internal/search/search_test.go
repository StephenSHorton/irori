package search

import (
	"strings"
	"testing"
)

func TestParseRef(t *testing.T) {
	tests := []struct {
		ref  string
		kind Kind
		id   string
		bad  bool
	}{
		{"turn:t-0123abcd", KindTurn, "t-0123abcd", false},
		{"edit:e-99", KindEdit, "e-99", false},
		{"commit:ABCDEF1", KindCommit, "abcdef1", false},
		{"commit:abc", "", "", true},
		{"commit:zzzzzzz", "", "", true},
		{"msg:01JX", KindMsg, "01JX", false},
		{"task:01JY", KindTask, "01JY", false},
		{" Task:01JY ", KindTask, "01JY", false},
		{"comment:c1", KindComment, "c1", false},
		{"pr:stephenshorton/irori#12", KindPR, "stephenshorton/irori#12", false},
		{"pr:irori#12", "", "", true},
		{"pr:a/b#x", "", "", true},
		{"decision:d1", KindDecision, "d1", false},
		{"agent:dev1/abc-123", KindAgent, "dev1/abc-123", false},
		{"lease:gpu", "", "", true},
		{"task:", "", "", true},
		{"nocolon", "", "", true},
		{"task:a b", "", "", true},
	}
	for _, tc := range tests {
		k, id, err := ParseRef(tc.ref)
		if tc.bad {
			if err == nil {
				t.Errorf("ParseRef(%q) = %s %s, want error", tc.ref, k, id)
			}
			continue
		}
		if err != nil || k != tc.kind || id != tc.id {
			t.Errorf("ParseRef(%q) = %s %q %v, want %s %q", tc.ref, k, id, err, tc.kind, tc.id)
		}
	}
}

func TestPage(t *testing.T) {
	full := strings.Repeat("é", 10) // 20 bytes
	tests := []struct {
		name string
		o    ShowOptions
		text string
		more bool
		next int
	}{
		{"all", ShowOptions{}, full, false, 0},
		{"cut on rune", ShowOptions{Limit: 5}, "éé", true, 4},
		{"offset mid rune snaps back", ShowOptions{Offset: 5, Limit: 4}, "éé", true, 8},
		{"past end", ShowOptions{Offset: 99}, "", false, 0},
	}
	for _, tc := range tests {
		text, more, next := Page(full, tc.o)
		if text != tc.text || more != tc.more || next != tc.next {
			t.Errorf("%s: Page = %q %v %d, want %q %v %d", tc.name, text, more, next, tc.text, tc.more, tc.next)
		}
	}
}
