package notify

import (
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// The person is one person on every device (v0.4 §2): a message to their
// key on another certified device, or a card they moved from there, is
// theirs here; a made-up "user:" key is not.
func TestWatcherOwnerSet(t *testing.T) {
	other, fake := "user:dev2", "user:dev-z"
	w := &watcher{s: Sources{User: user, IsOwner: func(k string) bool { return k == user || k == other }}}
	for _, tc := range []struct {
		to   string
		want bool
	}{{user, true}, {other, true}, {fake, false}, {keyA, false}} {
		if got := w.waitingFor(inbox.Delivery{To: tc.to, State: inbox.Queued}); got != tc.want {
			t.Errorf("waitingFor(to %s) = %v, want %v", tc.to, got, tc.want)
		}
	}
	// Moved by the person on another device: not news to them.
	if w.agentMoved(coord.Task{UpdatedBy: other, Holders: []string{other}, CreatedBy: other}) {
		t.Error("a card the person moved on another device raised a card")
	}
	// Moved by an agent and held by the person elsewhere: about the agent.
	if k := w.needsKey(coord.Task{UpdatedBy: keyA, Holders: []string{other, keyA}, CreatedBy: keyA}); k != keyA {
		t.Errorf("needsKey = %q, want the agent", k)
	}
	if k := w.needsKey(coord.Task{UpdatedBy: keyA, Holders: []string{fake}, CreatedBy: user}); k != fake {
		t.Errorf("needsKey with a made-up key = %q, want it treated as someone else", k)
	}
	// Without IsOwner only User counts (the v0.3 rule).
	w0 := &watcher{s: Sources{User: user}}
	if w0.waitingFor(inbox.Delivery{To: other, State: inbox.Queued}) || !w0.waitingFor(inbox.Delivery{To: user, State: inbox.Queued}) {
		t.Error("nil IsOwner should match exactly User")
	}
}
