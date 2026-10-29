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
	return createdID(h.t, h.ok(c, "board_create", args))
}

// ref is a card's short ref ("#1").
func (h *harness) ref(id string) string {
	h.t.Helper()
	t, ok := h.b.co.GetTask(id)
	if !ok || t.Short == "" {
		h.t.Fatalf("no short ref for %s", id)
	}
	return t.ShortRef()
}

// createdID is the id in board_create's "Created #1 (task:<id>)."
func createdID(t *testing.T, out string) string {
	t.Helper()
	_, rest, ok := strings.Cut(out, "(task:")
	id, _, _ := strings.Cut(rest, ")")
	if !ok || id == "" {
		t.Fatalf("no task id in %q", out)
	}
	return id
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
	has(t, out, "Created #1 (task:", "Assigned to beta", "Told at their next turn: beta", `"Fix auth"`, "links: Linear ABC-123",
		"beta (dev1/bbbbbbbb-2222-4222-8222-222222222222) (assigned, not taken up)")
	id := createdID(t, out)
	ns := h.notices(keyB)
	if len(ns) != 1 || ns[0].Mode != inbox.ModeNextTurn || ns[0].Message.Ref != "task:"+id || ns[0].Message.From.Key != keyA ||
		ns[0].Message.Reason != string(coord.NoticeAssigned) {
		t.Fatalf("assign notice %+v", ns)
	}
	has(t, ns[0].Message.Text, "tokens expire early", "(#1, task:"+id+")")

	// The card is in beta's Needs you until it takes it up, then Working.
	has(t, h.ok(callB, "board", nil), "Needs you (1)", "task:"+id)
	has(t, h.ok(callB, "board_join", map[string]any{"task_id": "task:" + id}), "You hold #1 now", "[working]")
	has(t, h.ok(callB, "board", nil), "Working (1)")
	h.fails(callC, "board_join", map[string]any{"task_id": id}, "held by")

	// board_assign hands it on. gamma is in another repo, but a card that
	// needs it shows on its own board wherever the card lives.
	has(t, h.ok(callB, "board_assign", map[string]any{"task_id": id, "to": "GAMMA"}), "Assigned "+h.ref(id)+" to gamma", "Told at their next turn: gamma")
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
	gid := "task:" + createdID(t, out)
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
			has(t, out, "Commented comment:", "on "+h.ref(id)+".")
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
		{"no blocker", "", []string{"Blocked #1 ", "Anyone may board_unblock"}},
		{"lease", "lease " + res, []string{"unblocks by itself when that lease", "(on lease " + res + ")"}},
		{"lease colon", "lease:" + res, []string{"(on lease " + res + ")"}},
		{"person", "person", []string{"(on person )"}},
		{"agent by name", "agent gamma", []string{"(on agent " + keyC + ")"}},
		{"external", "external CI is red", []string{"(on external CI is red)"}},
		{"bad kind", "bogus x", []string{"Error: ", "on starts with"}},
		{"lease without resource", "lease", []string{"Error: ", "needs the resource"}},
		{"lease nobody holds", "lease gpu/qwen", []string{"Error: "}},
		{"task without id", "task", []string{"Error: ", "needs its short ref or id"}},
		{"task nobody has", "task #7", []string{"Error: ", "no card is"}},
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
	has(t, h.ok(callA, "board_join", map[string]any{"task_id": id}), "You hold "+h.ref(id))
	has(t, h.ok(callB, "board_join", map[string]any{"task_id": id}), "with the others on it", "shared")
	h.fails(callA, "board_block", map[string]any{"task_id": id}, "give question")
	has(t, h.ok(callA, "board_block", map[string]any{"task_id": id, "reason": "need the schema", "on": "person"}),
		`blocked [b1]: "need the schema"`)
	has(t, h.ok(callC, "board", map[string]any{"repo": "*"}), "Blocked (1)")

	// Anyone may unblock; the holders other than gamma hear at their next turn.
	out := h.ok(callC, "board_unblock", map[string]any{"task_id": id, "note": "schema is in"})
	has(t, out, "Unblocked "+h.ref(id)+";", "working again", "Told at their next turn:", "alpha", "beta")
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
	has(t, h.ok(callB, "board_leave", map[string]any{"task_id": id}), "You left "+h.ref(id)+";")
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
		has(t, h.ok(callB, "board_link", map[string]any{"task_id": id, "url_or_path": tt.link}), "Linked "+h.ref(id), tt.want)
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
	has(t, h.ok(callB, "board_watch", map[string]any{"task_id": id}), "You watch "+h.ref(id), "watched by beta")

	// A blocker with context and options; beta recommends, gamma (not on it) can't.
	h.fails(callA, "board_block", map[string]any{"task_id": id, "question": "a", "reason": "b"}, "not both")
	out = h.ok(callA, "board_block", map[string]any{"task_id": id, "question": "Ship today, or wait for the check?",
		"context": "Every automated check passes; the co-op check needs both PCs.",
		"options": []any{map[string]any{"answer": "Wait, ship tonight", "why": "the temple is the headline"}}})
	has(t, out, "Blocked "+h.ref(id)+" on blocker b1", `blocked [b1]: "Ship today, or wait for the check?"`,
		`1. "Wait, ship tonight" (alpha) [recommended]: "the temple is the headline"`) // a lone option is the recommendation
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

// Waiting on the person is one blocker per ask: board_status
// input_required is refused on a card with no open blocker, and is a no-op
// (it would close them) on one that has them.
func TestBoardInputRequiredNeedsBlockers(t *testing.T) {
	h := newHarness(t)
	id := h.newCard(callA, map[string]any{"title": "TRELLIS.2 test"})
	h.ok(callA, "board_join", map[string]any{"task_id": id})
	note := "request access to dinov3 on huggingface and run hf auth login? may I push mokei-ui? where should the out/ backup go?"
	h.fails(callA, "board_status", map[string]any{"task_id": id, "state": "input_required", "note": note},
		"your note asks more than one thing", "its own board_block", "context for a newcomer", "recommended: true")
	h.fails(callA, "board_status", map[string]any{"task_id": id, "state": "input_required"}, "as a blocker", "board_block")
	if tk, _ := h.b.co.GetTask(id); tk.State != coord.Working {
		t.Fatalf("a refused input_required moved the card: %s", tk.State)
	}

	// Three asks, three blockers; the push one with options and a recommendation.
	h.ok(callA, "board_block", map[string]any{"task_id": id, "question": "Request access to facebook/dinov3-vitl16-pretrain-lvd1689m",
		"context": "Meta approves by hand on huggingface.co; then run hf auth login on the Mac."})
	out := h.ok(callA, "board_block", map[string]any{"task_id": id, "question": "May I push mokei-ui?",
		"options": []any{
			map[string]any{"answer": "Push to main", "why": "checks pass", "recommended": true},
			map[string]any{"answer": "Open a PR first"},
			map[string]any{"answer": "Hold"}}})
	has(t, out, "on blocker b2", `1. "Push to main" (alpha) [recommended]: "checks pass"`, `2. "Open a PR first" (alpha)`)
	hasNot(t, out, "board_recommend", "more than one thing")
	h.fails(callA, "board_block", map[string]any{"task_id": id, "question": "Where should the out/ backup go?",
		"options": []any{map[string]any{"answer": "D:", "recommended": true}, map[string]any{"answer": "NAS", "recommended": true}}},
		"at most one")
	has(t, h.ok(callA, "board_block", map[string]any{"task_id": id, "question": "Where should the out/ backup go?",
		"options": []any{map[string]any{"answer": "D:/backup"}, map[string]any{"answer": "The NAS"}}}),
		"on blocker b3", "None of its options is marked recommended")
	has(t, h.ok(callA, "board_block", map[string]any{"task_id": id, "question": "Ship? And tag?"}), "asks more than one thing")

	// With open blockers, input_required changes nothing.
	out = h.ok(callA, "board_status", map[string]any{"task_id": id, "state": "input_required", "note": "see above"})
	has(t, out, "already waits on 4 open blockers (b1, b2, b3, b4)", "stays blocked", "note was not saved")
	if tk, _ := h.b.co.GetTask(id); tk.State != coord.Blocked || len(tk.OpenBlockers()) != 4 {
		t.Fatalf("input_required touched the blockers: %s %d", tk.State, len(tk.OpenBlockers()))
	}
}

// board_create takes the card's steps, the same shape board_steps takes.
func TestBoardCreateWithSteps(t *testing.T) {
	h := newHarness(t)
	h.fails(callA, "board_create", map[string]any{"title": "x", "steps": []any{
		map[string]any{"text": "a", "doing": true}, map[string]any{"text": "b", "doing": true}}}, "only one step")
	out := h.ok(callA, "board_create", map[string]any{"title": "Release", "steps": []any{
		map[string]any{"text": "notes", "done": true}, map[string]any{"text": "tag", "doing": true}, "announce"}})
	has(t, out, "Created #", "Tick its steps with board_step", "steps 1/3", `now s2 "tag"`)
	hasNot(t, out, "Give it steps")
	has(t, h.ok(callA, "board_create", map[string]any{"title": "No plan yet"}), "Give it steps with board_steps")
}

// Every tool that takes a card takes its short ref (#2, 2, task:#2) as well
// as its id, and prints the short ref back.
func TestShortRefsInTheBoardTools(t *testing.T) {
	r := withCapture(t, nil)
	one := r.newCard(callA, map[string]any{"title": "First card"})
	two := r.newCard(callA, map[string]any{"title": "Second card"})
	has(t, r.ok(callA, "board", map[string]any{}), "- #1 task:"+one+" [", "- #2 task:"+two+" [")
	has(t, r.ok(callA, "board_join", map[string]any{"task_id": "#2"}), "You hold #2 now")
	has(t, r.ok(callA, "board_comment", map[string]any{"task_id": "task:#2", "text": "on it"}), " on #2.")
	has(t, r.ok(callA, "board_steps", map[string]any{"task_id": "2", "steps": []any{"a", "b"}}), "#2 has 2 steps")
	has(t, r.ok(callA, "board_block", map[string]any{"task_id": "#2", "question": "Wait for the first?", "on": "task #1"}), "Blocked #2")
	if x, _ := r.b.co.GetTask(two); x.BlockedBy == nil || x.BlockedBy.Ref != one {
		t.Fatalf("on task #1 stored %+v", x.BlockedBy)
	}
	r.fails(callA, "board_block", map[string]any{"task_id": "#1", "question": "Wait?", "on": "task #9"}, "no card is")
	r.fails(callA, "board_join", map[string]any{"task_id": "#9"}, "no such task")
	has(t, r.ok(callA, "board_join", map[string]any{"task_id": one}), "You hold #1 now")
	has(t, r.ok(callB, "show", map[string]any{"ref": "#2"}), "task:"+two, "Task #2 ", "Second card")
	r.fails(callB, "show", map[string]any{"ref": "#9"}, "no card is #9")
	out := r.ok(callB, "search", map[string]any{"query": "#2"})
	if i := strings.Index(out, "\n- "); i < 0 || !strings.HasPrefix(out[i+1:], "- #2 task:"+two+" ") {
		t.Fatalf("#2 is not the first hit:\n%s", out)
	}
	has(t, r.ok(callB, "search", map[string]any{"query": "first card"}), "- #1 task:"+one)
}

// One heavily blocked card must not push every other card off the board:
// a list line shows the first open item and counts the rest.
func TestBoardBlockedCardFitsTheList(t *testing.T) {
	h := newHarness(t)
	h.newCard(callA, map[string]any{"title": "Small card"})
	big := h.newCard(callA, map[string]any{"title": "Big card"})
	h.ok(callA, "board_join", map[string]any{"task_id": big})
	long := strings.Repeat("x", 150)
	for i := range coord.MaxOpenBlockers {
		var opts []any
		for j := range coord.MaxOptions {
			opts = append(opts, map[string]any{"answer": long + string(rune('a'+j)), "why": long + string(rune('a'+i))})
		}
		h.ok(callA, "board_block", map[string]any{"task_id": big, "question": string(rune('a'+i)) + strings.Repeat("q", 190), "options": opts})
	}
	out := h.ok(callA, "board", map[string]any{"repo": "*"})
	has(t, out, "Small card", "Big card", "[b1]", "7 more open items")
	hasNot(t, out, "not shown", "[b2]")
}

// Watch digests from sweeps between the watcher's turns merge into one
// waiting notice per card, not one per tick.
func TestWatchDigestsMergeUntilRead(t *testing.T) {
	h := newHarness(t)
	id := h.newCard(callA, map[string]any{"title": "Ship v0.6.0"})
	h.ok(callA, "board_join", map[string]any{"task_id": id})
	h.ok(callB, "board_watch", map[string]any{"task_id": id})
	co := h.b.co
	sweep := func() {
		t.Helper()
		ns, err := co.SweepNotices(bg)
		if err != nil {
			t.Fatal(err)
		}
		for _, n := range ns {
			if err := NoticeErr(RouteNotice(bg, h.b.ib, inbox.SystemSender(), n)); err != nil {
				t.Fatal(err)
			}
		}
	}
	yes := true
	if _, err := co.SetSteps(bg, keyA, id, []coord.Step{{Text: "notes"}, {Text: "build"}, {Text: "tag"}}); err != nil {
		t.Fatal(err)
	}
	sweep()
	for _, s := range []string{"s1", "s2", "s3"} {
		if _, err := co.SetStep(bg, keyA, id, s, coord.StepUpdate{Doing: &yes}); err != nil {
			t.Fatal(err)
		}
		sweep()
		if _, err := co.SetStep(bg, keyA, id, s, coord.StepUpdate{Done: &yes}); err != nil {
			t.Fatal(err)
		}
		sweep()
	}
	waiting := func() []inbox.Delivery {
		var out []inbox.Delivery
		for _, d := range h.notices(keyB) {
			if d.State.Open() {
				out = append(out, d)
			}
		}
		return out
	}
	got := waiting()
	if len(got) != 1 {
		var texts []string
		for _, d := range got {
			texts = append(texts, d.Message.Text)
		}
		t.Fatalf("bob has %d waiting notices, want 1:\n%s", len(got), strings.Join(texts, "\n---\n"))
	}
	txt := got[0].Message.Text
	has(t, txt, "set the steps", "“tag” (3/3)", "steps 3/3")
	if n := strings.Count(txt, "Update on"); n != 1 {
		t.Fatalf("one header, got %d:\n%s", n, txt)
	}
	// Once bob has read it, the next change is a new notice.
	if err := h.b.ib.Skip(bg, keyB, got[0].ID, "read"); err != nil {
		t.Fatal(err)
	}
	if _, err := co.SetTaskState(bg, keyA, id, coord.Completed, ""); err != nil {
		t.Fatal(err)
	}
	sweep()
	if got := waiting(); len(got) != 1 || strings.Contains(got[0].Message.Text, "set the steps") {
		t.Fatalf("after reading: %+v", got)
	}
}
