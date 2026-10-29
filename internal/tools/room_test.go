package tools

import (
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
)

// The ask tools (board.md "Asks"): the room hears, one takes it, it ends
// with what it became.
func TestAskTools(t *testing.T) {
	h := newHarness(t)
	co := h.b.co
	ask, n, err := co.PostAsk(bg, "user:dev1", coord.NewAsk{Text: "The login page feels slow"}, h.b.roster.Agents())
	if err != nil {
		t.Fatal(err)
	}
	if len(n.To) != 4 {
		t.Fatalf("told %v", n.To)
	}
	ref := h.ref(ask.ID)
	has(t, h.ok(callA, "board", nil), "Asks from the person", ref, "[ask]")

	has(t, h.ok(callA, "ask_claim", map[string]any{"task_id": ref}), "You have "+ref, "ask_close")
	has(t, h.ok(callB, "ask_claim", map[string]any{"task_id": ref}), "alpha", "took "+ref)
	h.fails(callB, "board_join", map[string]any{"task_id": ref}, "ask_claim")
	h.fails(callB, "board_assign", map[string]any{"task_id": ref, "to": "beta"}, "ask_claim")
	h.fails(callA, "board_status", map[string]any{"task_id": ref, "state": "completed"}, "ask_close")
	has(t, h.ok(callA, "board", nil), "Asks you took", ref)

	has(t, h.ok(callB, "board_comment", map[string]any{"task_id": ref, "text": "the API is slow too"}), "You watch it now")
	if got, _ := co.GetTask(ask.ID); len(got.Watchers) != 1 || got.Watchers[0] != keyB {
		t.Fatalf("watchers %v", got.Watchers)
	}
	h.fails(callB, "ask_close", map[string]any{"task_id": ref, "outcome": "none", "note": "x"}, "holds")
	h.fails(callA, "ask_close", map[string]any{"task_id": ref, "outcome": "card", "note": "x"}, "title")
	h.fails(callA, "ask_close", map[string]any{"task_id": ref, "outcome": "maybe", "note": "x"}, "outcome")
	out := h.ok(callA, "ask_close", map[string]any{"task_id": ref, "outcome": "card", "note": "a real fix",
		"title": "Speed up login", "steps": []map[string]any{{"text": "profile"}, {"text": "fix"}}})
	has(t, out, ref+" is a card now", `"Speed up login"`, "steps 0/2")
	got, _ := co.GetTask(ask.ID)
	if got.Holder != keyA || got.State != coord.Working || got.Ask.Open() {
		t.Fatalf("after the card outcome %+v", got)
	}
	if !strings.Contains(h.ok(callA, "board", map[string]any{"repo": "*"}), "Working (1)") {
		t.Fatal("the card isn't an ordinary working card")
	}

	// edit and none.
	other := h.newCard(callC, map[string]any{"title": "Ship docs"})
	two, _, _ := co.PostAsk(bg, "user:dev1", coord.NewAsk{Text: "docs should say Windows"}, nil)
	r2 := h.ref(two.ID)
	h.ok(callC, "ask_claim", map[string]any{"task_id": r2})
	has(t, h.ok(callC, "ask_close", map[string]any{"task_id": r2, "outcome": "edit", "task": h.ref(other), "note": "added it", "objective": "with Windows"}),
		"Changed "+h.ref(other))
	if o, _ := co.GetTask(other); o.Objective != "with Windows" {
		t.Fatalf("edited card %+v", o)
	}
	three, _, _ := co.PostAsk(bg, "user:dev1", coord.NewAsk{Text: "never mind"}, nil)
	r3 := h.ref(three.ID)
	h.ok(callC, "ask_claim", map[string]any{"task_id": r3})
	has(t, h.ok(callC, "board_leave", map[string]any{"task_id": r3}), "gave the ask "+r3+" back")
	h.ok(callB, "ask_claim", map[string]any{"task_id": r3})
	has(t, h.ok(callB, "ask_close", map[string]any{"task_id": r3, "outcome": "none", "note": "the person sorted it"}), "Closed "+r3)

	// answered: the ask completes, and the taker keeps it.
	four, _, _ := co.PostAsk(bg, "user:dev1", coord.NewAsk{Text: "which port?"}, nil)
	r4 := h.ref(four.ID)
	h.ok(callB, "ask_claim", map[string]any{"task_id": r4})
	has(t, h.ok(callB, "ask_close", map[string]any{"task_id": r4, "outcome": "answered", "note": "8080"}), "Answered "+r4, "reopens")
	if got, _ := co.GetTask(four.ID); got.State != coord.Completed || got.Ask.Outcome.Kind != coord.OutcomeAnswered {
		t.Fatalf("after the answered outcome %+v", got)
	}
}
