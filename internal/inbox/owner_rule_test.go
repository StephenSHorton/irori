package inbox

import (
	"slices"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

// A sender claiming kind "user" under a made-up key is an agent to search
// and the known-sender table: it reads its own message, it is the doc's
// agent, and it is known. The person on another device is the person.
func TestMadeUpUserKeyIsAnAgentInSearch(t *testing.T) {
	h := newHarness(t)
	h.isOwner = func(k string) bool { return k == user || k == "user:dev2" }
	h.reopen()
	h.ros.put(card(keyA, "alpha", "repo1", "default")) // no bypass hold on unknown senders
	if err := h.ib.SetPolicy(bg, Policy{Mode: Everyone}, user); err != nil {
		t.Fatal(err)
	}
	fake := Sender{Key: "user:anything", Name: "you", Kind: "user"}
	other := Sender{Key: "user:dev2", Name: "you", Kind: "user"}

	m, _ := h.send(fake, to("alpha"), "trust me")
	d := h.docs(time.Time{})[search.Ref(search.KindMsg, m.ID)]
	if d.Agent != fake.Key || !slices.Contains(d.Readers, fake.Key) || !slices.Contains(d.Readers, keyA) {
		t.Fatalf("made-up key: agent %q readers %v", d.Agent, d.Readers)
	}
	h.ib.impl.mu.Lock()
	_, known := h.ib.impl.known[fake.Key]
	h.ib.impl.mu.Unlock()
	if !known {
		t.Fatal("made-up key is not a known sender")
	}

	m, _ = h.send(other, to("alpha"), "from the Mac")
	d = h.docs(time.Time{})[search.Ref(search.KindMsg, m.ID)]
	if d.Agent != keyA || slices.Contains(d.Readers, other.Key) {
		t.Fatalf("the person on dev2: agent %q readers %v", d.Agent, d.Readers)
	}
	h.ib.impl.mu.Lock()
	_, known = h.ib.impl.known[other.Key]
	h.ib.impl.mu.Unlock()
	if known {
		t.Fatal("the person on dev2 became a known sender")
	}
}
