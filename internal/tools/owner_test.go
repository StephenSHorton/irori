package tools

import (
	"testing"

	"github.com/StephenSHorton/irori/internal/presence"
)

// The tools use coord's owner rule for "the person" (v0.4 §2): the person's
// own key passes as an assignee and shows as the person; a made-up "user:"
// key is just an unknown name.
func TestToolsUseTheOwnerRule(t *testing.T) {
	h := newHarness(t)
	s := h.set.impl
	if k, err := s.assignee("user:dev1"); err != nil || k != "user:dev1" {
		t.Fatalf("assignee(person) = %q, %v", k, err)
	}
	if _, err := s.assignee("user:made-up"); err == nil {
		t.Fatal("a made-up user: key passed as an assignee")
	}
	n := names{cards: map[string]presence.AgentCard{}, owner: s.isOwner}
	if got := n.short("user:dev1"); got != "the person" {
		t.Fatalf("short(person) = %q", got)
	}
	if got := n.short("user:made-up"); got == "the person" {
		t.Fatal("a made-up user: key shown as the person")
	}
}
