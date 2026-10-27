package tools

import (
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// newCard puts a card up as c and returns its id.
func (h *harness) newCard(c Caller, args map[string]any) string {
	h.t.Helper()
	out := h.ok(c, "board_create", args)
	return strings.TrimSuffix(field(h.t, out, "Created"), ".")[len("task:"):]
}

// notices lists the open deliveries to key that carry Board notices.
func (h *harness) notices(key string) []inbox.Delivery {
	h.t.Helper()
	var out []inbox.Delivery
	for _, d := range h.b.ib.For(key, true, 0) {
		if d.Message.Notice {
			out = append(out, d)
		}
	}
	return out
}

func TestBoardCreateAndAssign(t *testing.T) {
	h := newHarness(t)
	h.fails(callA, "board_create", map[string]any{"objective": "x"}, "title")
	h.fails(callA, "board_create", map[string]any{"title": "x", "assign_to": "nobody"}, "agents tool")
	h.fails(callA, "board_create", map[string]any{"title": "x", "links": []string{"ftp://nope"}}, "http(s)")
	h.fails(callA, "board_create", map[string]any{"title": "x", "priority": 1}, `unknown argument "priority"`)

	out := h.ok(callA, "board_create", map[string]any{"title": "Fix\nauth", "objective": "tokens expire early",
		"links": []string{"https://linear.app/acme/issue/ABC-123/fix-auth"}, "assign_to": "beta"})
	has(t, out, "Created task:", "Assigned to beta", "Told at their next turn: beta", `"Fix auth"`, "links: Linear ABC-123",
		"beta (dev1/bbbbbbbb-2222-4222-8222-222222222222) (assigned, not taken up)")
	id := strings.TrimSuffix(field(t, out, "Created"), ".")[len("task:"):]
	ns := h.notices(keyB)
	if len(ns) != 1 || ns[0].Mode != inbox.ModeNextTurn || ns[0].Message.Ref != "task:"+id || ns[0].Message.From.Key != keyA {
		t.Fatalf("assign notice %+v", ns)
	}
	has(t, ns[0].Message.Text, "tokens expire early", "task:"+id)

	// The card is in beta's Needs you until it takes it up, then Working.
	has(t, h.ok(callB, "board", nil), "Needs you (1)", "task:"+id)
	has(t, h.ok(callB, "board_join", map[string]any{"task_id": "task:" + id}), "You hold task:"+id, "[working]")
	has(t, h.ok(callB, "board", nil), "Working (1)")
	h.fails(callC, "board_join", map[string]any{"task_id": id}, "held by")

	// board_assign hands it on. gamma is in another repo, but a card that
	// needs it shows on its own board wherever the card lives.
	has(t, h.ok(callB, "board_assign", map[string]any{"task_id": id, "to": "GAMMA"}), "Assigned task:"+id+" to gamma", "Told at their next turn: gamma")
	if len(h.notices(keyC)) != 1 {
		t.Fatalf("gamma notices %+v", h.notices(keyC))
	}
	out = h.ok(callC, "board", nil)
	has(t, out, "Board for remote:github.com/acme/other: 1 open card", "Needs you (1)", "task:"+id, "repo remote:github.com/acme/app")
	hasNot(t, out, "Nothing live here")
	has(t, h.ok(callC, "board", map[string]any{"repo": "*"}), "Board for every repo", "Needs you (1)")
	// The other cards of acme/app stay off gamma's board.
	hasNot(t, h.ok(callC, "board", nil), "Working (")

	// A Grok member's card has no repo; assigned to gamma, it still needs it.
	out = h.ok(grokCaller(), "board_create", map[string]any{"title": "grok asks", "assign_to": "gamma"})
	gid := strings.TrimSuffix(field(t, out, "Created"), ".")
	has(t, h.ok(callC, "board", nil), "Needs you (2)", gid)
	h.fails(callA, "board_assign", map[string]any{"task_id": id}, "give to")
}

// A comment reaches the holders at their next turn unless the author
// chose to wake them (decision 18).
func TestBoardCommentDelivery(t *testing.T) {
	tests := []struct {
		name     string
		wake     any // nil: left out
		wantMode inbox.DeliveryMode
	}{
		{"default", nil, inbox.ModeNextTurn},
		{"wake false", false, inbox.ModeNextTurn},
		{"wake true", true, inbox.ModeWake},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			h := newHarness(t)
			id := h.newCard(callA, map[string]any{"title": "lexer"})
			h.ok(callB, "board_join", map[string]any{"task_id": id})
			args := map[string]any{"task_id": id, "text": "the lexer drops tabs"}
			if tt.wake != nil {
				args["wake"] = tt.wake
			}
			out := h.ok(callA, "board_comment", args)
			has(t, out, "Commented comment:", "on task:"+id)
			if tt.wantMode == inbox.ModeWake {
				has(t, out, "Woke: beta")
			} else {
				has(t, out, "Told at their next turn: beta")
			}
			cid := strings.TrimPrefix(field(t, out, "Commented"), "comment:")
			ns := h.notices(keyB)
			if len(ns) != 1 || ns[0].Mode != tt.wantMode || ns[0].Message.Ref != "comment:"+cid {
				t.Fatalf("notices %+v", ns)
			}
			w, err := h.b.ib.NextWake(bg, keyB, inbox.PathMod)
			if err != nil {
				t.Fatal(err)
			}
			if (w != nil) != (tt.wantMode == inbox.ModeWake) {
				t.Fatalf("wake %+v for mode %s", w, tt.wantMode)
			}
			if tt.wantMode == inbox.ModeNextTurn {
				a, err := h.b.ib.NextAppend(bg, keyB)
				if err != nil || a == nil || a.Mode != inbox.ModeNextTurn {
					t.Fatalf("append %+v, %v", a, err)
				}
			}
			// The commenter isn't told about its own comment.
			if len(h.notices(keyA)) != 0 {
				t.Fatalf("alpha got %+v", h.notices(keyA))
			}
			has(t, h.ok(callC, "board", map[string]any{"repo": "*"}), "last: comment:"+cid, "the lexer drops tabs")
		})
	}
}

func TestBoardCommentNobodyToTell(t *testing.T) {
	h := newHarness(t)
	id := h.newCard(callA, map[string]any{"title": "solo"})
	h.ok(callA, "board_join", map[string]any{"task_id": id})
	has(t, h.ok(callA, "board_comment", map[string]any{"task_id": id, "text": "note to self"}), "Nobody else holds it")
	h.fails(callA, "board_comment", map[string]any{"task_id": id}, "comment text")
	h.fails(callA, "board_comment", map[string]any{"text": "x"}, "give task_id")
	h.fails(callA, "board_comment", map[string]any{"task_id": "nope", "text": "x"}, "board and leases tools")
}

func TestBoardBlockOn(t *testing.T) {
	res := "path:remote:github.com/acme/app:src/api/**"
	tests := []struct {
		name string
		on   string
		want []string // in the result; an "Error: " prefix means it fails
	}{
		{"no blocker", "", []string{"Blocked task:", "Anyone may board_unblock"}},
		{"lease", "lease " + res, []string{"unblocks by itself when that lease", "(on lease " + res + ")"}},
		{"lease colon", "lease:" + res, []string{"(on lease " + res + ")"}},
		{"person", "person", []string{"(on person )"}},
		{"agent by name", "agent gamma", []string{"(on agent " + keyC + ")"}},
		{"external", "external CI is red", []string{"(on external CI is red)"}},
		{"bad kind", "bogus x", []string{"Error: ", "on starts with"}},
		{"lease without resource", "lease", []string{"Error: ", "needs the resource"}},
		{"lease nobody holds", "lease gpu/qwen", []string{"Error: "}},
		{"task without id", "task", []string{"Error: ", "needs its id"}},
		{"named person", "person bob", []string{"Error: ", "takes no name"}},
		{"unknown agent", "agent nobody", []string{"Error: ", "agents tool"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			h := newHarness(t)
			h.ok(callC, "lease", map[string]any{"resource": res})
			id := h.newCard(callA, map[string]any{"title": "api"})
			h.ok(callA, "board_join", map[string]any{"task_id": id})
			out := h.call(callA, "board_block", map[string]any{"task_id": id, "reason": "waiting", "on": tt.on})
			if strings.HasPrefix(tt.want[0], "Error: ") != strings.HasPrefix(out, "Error: ") {
				t.Fatalf("got %q", out)
			}
			has(t, out, tt.want...)
		})
	}
}

func TestBoardBlockUnblockStatus(t *testing.T) {
	h := newHarness(t)
	id := h.newCard(callA, map[string]any{"title": "migrate", "done_when": "tests pass", "shared": true})
	has(t, h.ok(callA, "board_join", map[string]any{"task_id": id}), "You hold task:"+id)
	has(t, h.ok(callB, "board_join", map[string]any{"task_id": id}), "with the others on it", "shared")
	h.fails(callA, "board_block", map[string]any{"task_id": id}, "give question")
	has(t, h.ok(callA, "board_block", map[string]any{"task_id": id, "reason": "need the schema", "on": "person"}),
		`blocked [b1]: "need the schema"`)
	has(t, h.ok(callC, "board", map[string]any{"repo": "*"}), "Blocked (1)")

	// Anyone may unblock; the holders other than gamma hear at their next turn.
	out := h.ok(callC, "board_unblock", map[string]any{"task_id": id, "note": "schema is in"})
	has(t, out, "Unblocked task:"+id, "working again", "Told at their next turn:", "alpha", "beta")
	for _, k := range []string{keyA, keyB} {
		if ns := h.notices(k); len(ns) != 1 || ns[0].Mode != inbox.ModeNextTurn || ns[0].Message.Ref != "task:"+id {
			t.Fatalf("%s notices %+v", k, ns)
		}
	}
	h.fails(callC, "board_unblock", map[string]any{"task_id": id}, "blocked")

	h.fails(callA, "board_status", map[string]any{"task_id": id, "state": "blocked"}, "use board_block")
	h.fails(callA, "board_status", map[string]any{"task_id": id, "state": "submitted"}, "give state")
	h.fails(callA, "board_status", map[string]any{"task_id": id, "state": "done"}, "give state")
	h.fails(callC, "board_status", map[string]any{"task_id": id, "state": "completed"}, "holder")
	has(t, h.ok(callB, "board_leave", map[string]any{"task_id": id}), "You left task:"+id)
	h.fails(callB, "board_leave", map[string]any{"task_id": id}, "isn't on task")
	has(t, h.ok(callA, "board_status", map[string]any{"task_id": id, "state": "completed", "note": "shipped"}),
		"is completed now", "folds into Done")

	has(t, h.ok(callA, "board", nil), "0 open cards, 1 done", "Nothing live here")
	has(t, h.ok(callA, "board", map[string]any{"done": true}), "Done, newest first (1)", "task:"+id)
	h.newCard(callA, map[string]any{"title": "next"})
	has(t, h.ok(callA, "board", nil), "Open (1)", "1 done cards are collapsed")
}

func TestBoardLink(t *testing.T) {
	h := newHarness(t)
	id := h.newCard(callA, map[string]any{"title": "auth"})
	tests := []struct {
		link string
		want string
	}{
		{"https://github.com/acme/app/issues/57", "GitHub #57"},
		{"https://www.notion.so/acme/Board-design-0123456789abcdef0123456789abcdef", "Notion Board design"},
		{`docs\design\board.md`, ".../design/board.md"},
	}
	for _, tt := range tests {
		has(t, h.ok(callB, "board_link", map[string]any{"task_id": id, "url_or_path": tt.link}), "Linked task:"+id, tt.want)
	}
	h.fails(callB, "board_link", map[string]any{"task_id": id}, "url_or_path")
	h.fails(callB, "board_link", map[string]any{"task_id": id, "url_or_path": "../etc/passwd"}, "leaves the repo")
	h.fails(callB, "board_link", map[string]any{"task_id": id, "url_or_path": "x", "label": "y"}, `unknown argument "label"`)
}

// Leases that block something show on the Board.
func TestBoardShowsBlockingLease(t *testing.T) {
	h := newHarness(t)
	res := "path:remote:github.com/acme/app:src/**"
	h.ok(callB, "lease", map[string]any{"resource": res})
	h.fails(callA, "lease", map[string]any{"resource": res}, "steal")
	out := h.ok(callA, "board", nil)
	has(t, out, "Leases in the way (1)", res+" held by beta", "refused to alpha")
	if v := h.b.co.Board(coord.BoardFilter{}); len(v.Leases) != 1 {
		t.Fatalf("leases %+v", v.Leases)
	}
}

// The cards: steps, blockers with recommended answers, watchers.
func TestBoardCards(t *testing.T) {
	h := newHarness(t)
	id := h.newCard(callA, map[string]any{"title": "Ship v0.6.0"})
	h.ok(callA, "board_join", map[string]any{"task_id": id})

	// Steps: strings or objects; the result lists their ids.
	out := h.ok(callA, "board_steps", map[string]any{"task_id": id, "steps": []any{
		map[string]any{"text": "notes", "done": true}, map[string]any{"text": "co-op check", "doing": true}, "tag"}})
	has(t, out, "3 steps, 1 done", `[x] s1 "notes"`, `[>] s2 "co-op check"`, `[ ] s3 "tag"`)
	h.fails(callA, "board_steps", map[string]any{"task_id": id, "steps": []any{
		map[string]any{"text": "a", "doing": true}, map[string]any{"text": "b", "doing": true}}}, "only one step")
	h.fails(callC, "board_steps", map[string]any{"task_id": id, "steps": []any{"x"}}, "holder")
	h.fails(callA, "board_step", map[string]any{"task_id": id, "step_id": "s2"}, "give done")
	h.fails(callA, "board_step", map[string]any{"task_id": id, "step_id": "s9", "done": true}, "no step")
	has(t, h.ok(callA, "board_step", map[string]any{"task_id": id, "step_id": "s2", "done": true}), "steps 2/3 done")

	// Watching.
	has(t, h.ok(callB, "board_watch", map[string]any{"task_id": id}), "You watch task:"+id, "watched by beta")

	// A blocker with context and options; beta recommends, gamma (not on it) can't.
	h.fails(callA, "board_block", map[string]any{"task_id": id, "question": "a", "reason": "b"}, "not both")
	out = h.ok(callA, "board_block", map[string]any{"task_id": id, "question": "Ship today, or wait for the check?",
		"context": "Every automated check passes; the co-op check needs both PCs.",
		"options": []any{map[string]any{"answer": "Wait, ship tonight", "why": "the temple is the headline"}}})
	has(t, out, "Blocked task:"+id+" on blocker b1", `blocked [b1]: "Ship today, or wait for the check?"`,
		`1. "Wait, ship tonight" (alpha): "the temple is the headline"`)
	hasNot(t, out, "board_recommend so") // it gave an option
	has(t, h.ok(callB, "board_recommend", map[string]any{"task_id": id, "blocker_id": "b1", "answer": "Ship now, mark experimental", "why": "fixes today"}),
		"Recommended on blocker b1", `2. "Ship now, mark experimental" (beta)`)
	h.fails(callC, "board_recommend", map[string]any{"task_id": id, "blocker_id": "b1", "answer": "x"}, "watch it first")
	h.fails(callB, "board_recommend", map[string]any{"task_id": id, "blocker_id": "b9", "answer": "x"}, "no blocker")
	out = h.ok(callC, "board", map[string]any{"repo": "*"})
	has(t, out, "steps 2/3", "watched by beta", "[b1]", "Wait, ship tonight", "Ship now, mark experimental")

	// The watcher's digest goes out on the sweep; the raiser hears of beta's recommendation.
	ns, err := h.b.co.SweepNotices(bg)
	if err != nil {
		t.Fatal(err)
	}
	for _, n := range ns {
		RouteNotice(bg, h.b.ib, inbox.SystemSender(), n)
	}
	if got := h.notices(keyB); len(got) != 1 || got[0].Mode != inbox.ModeNextTurn {
		t.Fatalf("beta's watch notices %+v", got)
	} else {
		has(t, got[0].Message.Text, "raised a blocker: Ship today")
		hasNot(t, got[0].Message.Text, "ticked", "recommended") // before it watched; its own
	}
	if got := h.notices(keyA); len(got) != 1 {
		t.Fatalf("alpha's notices %+v", got)
	} else {
		has(t, got[0].Message.Text, "recommended “Ship now, mark experimental”")
	}

	// gamma answers b1 by picking beta's option: alpha (the raiser) hears.
	h.fails(callC, "board_unblock", map[string]any{"task_id": id, "blocker_id": "b1"}, "give pick")
	h.fails(callC, "board_unblock", map[string]any{"task_id": id, "pick": 1}, "pick needs blocker_id")
	out = h.ok(callC, "board_unblock", map[string]any{"task_id": id, "blocker_id": "b1", "pick": 2})
	has(t, out, "Resolved blocker b1", "working again", "Told at their next turn: alpha")
	got := h.notices(keyA)
	var texts []string
	for _, d := range got {
		texts = append(texts, d.Message.Text)
	}
	has(t, strings.Join(texts, " | "), "Answer: Ship now, mark experimental", "That is the answer")
	has(t, h.ok(callB, "board_unwatch", map[string]any{"task_id": id}), "no longer watch")
}

func TestTaskIDPrefix(t *testing.T) {
	for in, want := range map[string]string{"task:01J": "01J", "TASK: 01J ": "01J", "01J": "01J", "": "", "comment:01J": "comment:01J"} {
		if got := taskID(in); got != want {
			t.Errorf("taskID(%q) = %q, want %q", in, got, want)
		}
	}
}
