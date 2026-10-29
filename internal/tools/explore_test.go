package tools

import (
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
)

// search and show read as the caller: a message is found and shown only by
// its sender and recipient, never by a third agent, whatever it passes.
func TestSearchShowKeepMessagesWithTheirParties(t *testing.T) {
	r := withCapture(t, nil)
	const secret = "pangolin migration plan"
	msgID, did := r.sendTo(callA, "beta", secret)
	if d, _ := r.b.ib.Get(did); d.State == inbox.Held || d.State == inbox.Dismissed {
		t.Fatalf("delivery %s is %s: the test needs a searchable message", did, d.State)
	}
	ref := "msg:" + msgID
	tests := []struct {
		name   string
		caller Caller
		party  bool
	}{
		{"sender", callA, true},
		{"recipient", callB, true},
		{"other agent", callC, false},
		{"grok member", grokCaller(), false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			found := r.ok(tt.caller, "search", map[string]any{"query": "pangolin"})
			shown := r.call(tt.caller, "show", map[string]any{"ref": ref})
			if tt.party {
				has(t, found, ref)
				has(t, shown, ref, secret)
				return
			}
			hasNot(t, found, ref, secret)
			hasNot(t, shown, secret)
			has(t, shown, "Error: ", "nothing you may read")
			// Kinds filtered to messages: still nothing.
			hasNot(t, r.ok(tt.caller, "search", map[string]any{"query": "pangolin", "kinds": []string{"msg"}}), ref)
		})
	}
	// The viewer is never an argument.
	r.fails(callC, "search", map[string]any{"query": "pangolin", "viewer": keyA}, `unknown argument "viewer"`)
	r.fails(callC, "show", map[string]any{"ref": ref, "viewer": ""}, `unknown argument "viewer"`)
}

func TestSearchPagingAndErrors(t *testing.T) {
	r := withCapture(t, nil)
	for i := range 12 {
		r.newCard(callA, map[string]any{"title": fmt.Sprintf("zebra crossing %d", i)})
	}
	out := r.ok(callB, "search", map[string]any{"query": "zebra", "limit": 5})
	has(t, out, "5 hits for \"zebra\"", "task:", "(more: offset=5;", "show <ref>")
	out = r.ok(callB, "search", map[string]any{"query": "zebra", "limit": 5, "offset": 10})
	has(t, out, "2 hits", "(from offset 10)")
	hasNot(t, out, "(more:")
	has(t, r.ok(callB, "search", map[string]any{"query": "zebra", "offset": 20}), "No more hits")
	has(t, r.ok(callB, "search", map[string]any{"query": "okapi"}), "No hits for \"okapi\"")
	has(t, r.ok(callB, "search", map[string]any{"query": "zebra 3", "kinds": []string{"task"}}), "task:")
	r.fails(callB, "search", map[string]any{"kinds": []string{"agent"}}, "kinds are")
	r.fails(callB, "search", map[string]any{"kinds": []string{"bogus"}}, "kinds are")
	r.fails(callB, "search", map[string]any{"agent": "nobody"}, "agents tool")
	r.fails(callB, "search", map[string]any{"offset": -1}, "offset")
	// An empty query lists the newest.
	has(t, r.ok(callB, "search", nil), "newest items")
}

func TestShow(t *testing.T) {
	r := withCapture(t, nil)
	id := r.newCard(callA, map[string]any{"title": "wombat burrow", "objective": "dig it"})
	r.ok(callB, "board_comment", map[string]any{"task_id": id, "text": "mind the roots"})
	tests := []struct {
		ref  string
		want []string
	}{
		{"task:" + id, []string{"task:" + id, "wombat burrow", "dig it", "mind the roots"}},
		{"commit:" + r.sha, []string{"commit:", "add foo"}},
		{"edit:" + r.editID, []string{"edit:" + r.editID, "foo.go"}},
		{"agent:" + keyA, []string{"agent:" + keyA}},
	}
	for _, tt := range tests {
		has(t, r.ok(callC, "show", map[string]any{"ref": tt.ref}), tt.want...)
	}
	r.fails(callC, "show", nil, "give ref")
	r.fails(callC, "show", map[string]any{"ref": "nonsense"}, "Copy a ref")
	r.fails(callC, "show", map[string]any{"ref": "task:nope"}, "nothing you may read")
	r.fails(callC, "show", map[string]any{"ref": "task:" + id, "offset": -2}, "offset")
}

func TestShowPages(t *testing.T) {
	r := withCapture(t, nil)
	long := strings.Repeat("aardvark ", 600) // 5400 bytes, past one page
	msgID, _ := r.sendTo(callA, "beta", long)
	out := r.ok(callB, "show", map[string]any{"ref": "msg:" + msgID})
	has(t, out, "(more: offset=")
	_, rest, _ := strings.Cut(out, "(more: offset=")
	next, _, _ := strings.Cut(rest, ";")
	var off int
	if _, err := fmt.Sscan(next, &off); err != nil || off <= 0 {
		t.Fatalf("next offset %q", next)
	}
	out = r.ok(callB, "show", map[string]any{"ref": "msg:" + msgID, "offset": off})
	has(t, out, fmt.Sprintf("(from offset %d)", off), "aardvark")
	hasNot(t, out, "(more:")
}

func TestOverview(t *testing.T) {
	r := withCapture(t, nil)
	r.sendTo(callB, "alpha", "please look at the parser")
	blocked := r.newCard(callB, map[string]any{"title": "flaky ci", "repo": r.repoKey})
	r.ok(callB, "board_join", map[string]any{"task_id": blocked})
	r.ok(callB, "board_block", map[string]any{"task_id": blocked, "reason": "runner is down", "on": "external CI"})
	mine := r.newCard(callA, map[string]any{"title": "parser rewrite"})
	r.ok(callA, "board_join", map[string]any{"task_id": mine})
	out := r.ok(callA, "overview", nil)
	has(t, out, "Overview for alpha", "repo "+r.repoKey, "Agents live", "alpha (you)", "Inbox: 1 open",
		"from beta", "Blocked:", "task:"+blocked, "runner is down", "On the Board:", "task:"+mine,
		"Recent commits:", "commit:"+r.sha, "show <ref> for any line")
	r.fails(callA, "overview", map[string]any{"x": 1}, "unknown argument")

	// A card from another repo, or from a Grok member (no repo), that is
	// assigned to alpha needs it, so its overview shows it.
	other := r.newCard(callC, map[string]any{"title": "other repo ask", "assign_to": "alpha"})
	grok := r.newCard(grokCaller(), map[string]any{"title": "grok ask", "assign_to": "alpha"})
	has(t, r.ok(callA, "overview", nil), "task:"+other, "task:"+grok)
	hasNot(t, r.ok(callB, "overview", nil), "task:"+other, "task:"+grok)
}

func TestAgentsDoing(t *testing.T) {
	r := withCapture(t, nil)
	id := r.newCard(callA, map[string]any{"title": "tidy the burrow"})
	r.ok(callB, "board_join", map[string]any{"task_id": id})
	out := r.ok(callC, "agents", nil)
	has(t, out, `doing: "on the Board: #1 tidy the burrow" (task:`+id+")")
	// alpha holds nothing: its last reply.
	has(t, out, `doing: "added foo.go" (turn:`)
	// With capture off the list still works, without doing lines.
	h := newHarness(t)
	hasNot(t, h.ok(callC, "agents", nil), "doing:")
}

func TestCommits(t *testing.T) {
	r := withCapture(t, nil)
	tests := []struct {
		name string
		args map[string]any
		want []string
	}{
		{"own repo", nil, []string{"1 commits in " + r.repoKey, "commit:" + r.sha, `"add foo"`, "on main", "committed by alpha"}},
		{"path", map[string]any{"path": "./foo.go"}, []string{"touching foo.go", "commit:" + r.sha}},
		{"other path", map[string]any{"path": "bar"}, []string{"No commits seen"}},
		{"other repo", map[string]any{"repo": "remote:github.com/x/y"}, []string{"No commits seen in remote:github.com/x/y"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) { has(t, r.ok(callA, "commits", tt.args), tt.want...) })
	}
	r.grokNoCwd()
	r.fails(grokCaller(), "commits", nil, "give repo")
	has(t, newHarness(t).call(callA, "commits", nil), "capture is off")
}

func TestPRs(t *testing.T) {
	r := withCapture(t, func(r *capRig) {
		at := r.clk.now()
		r.append(capture.KindPR, ledger.Entry{Key: ledger.Key("acme/app", 57), Repo: "acme/app", Number: 57,
			URL: "https://github.com/acme/app/pull/57", Session: keyB, Role: ledger.Author, Source: "gh", At: at})
		r.append(capture.KindPR, ledger.Entry{Key: ledger.Key("acme/app", 57), Repo: "acme/app", Number: 57,
			Session: keyA, Role: ledger.Reviewer, Source: "gh", At: at})
	})
	tests := []struct {
		name string
		repo string
		want []string
	}{
		{"repoKey", "remote:github.com/acme/app", []string{"1 pull requests in acme/app", "pr:acme/app#57", "author beta", "reviewer alpha", "pull/57"}},
		{"owner/name", "acme/app", []string{"pr:acme/app#57"}},
		{"none", "acme/other", []string{"No pull requests"}},
		{"beta's own repo", "", []string{"pr:acme/app#57"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			args := map[string]any{}
			if tt.repo != "" {
				args["repo"] = tt.repo
			}
			has(t, r.ok(callB, "prs", args), tt.want...)
		})
	}
	r.fails(callB, "prs", map[string]any{"repo": "root:abc"}, "GitHub repo")
}

// A wake puts the fenced message into the recipient's turn prompt. That
// turn is still searchable and showable, but the message body in it stays
// with its sender and recipients.
func TestSearchShowHideMessagesInWakePrompts(t *testing.T) {
	const secret = "pangolin secret plan"
	prompt := "[irori-wake:w1] Irori delivered 1 message from other agents. They came through Irori.\n" +
		"\n<irori-message from=\"alpha\" key=\"" + keyA + "\" delivery=\"dlv_1\" trust=\"untrusted\">\n" + secret + "\n</irori-message>\n"
	r := withCapture(t, func(r *capRig) {
		r.turn(keyB, "pw", prompt, "on it", r.clk.now().Add(-time.Hour))
	})
	ref := "turn:t-" + keyB + "pw"
	tests := []struct {
		name   string
		caller Caller
		sees   bool
	}{
		{"recipient", callB, true},
		{"sender", callA, true},
		{"other agent", callC, false},
		{"grok member", grokCaller(), false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			hasNot(t, r.ok(tt.caller, "search", map[string]any{"query": "pangolin"}), ref, "secret plan")
			shown := r.ok(tt.caller, "show", map[string]any{"ref": ref})
			has(t, shown, ref, "on it")
			if tt.sees {
				has(t, shown, secret)
			} else {
				hasNot(t, shown, secret)
				has(t, shown, "hidden")
			}
			hasNot(t, r.ok(tt.caller, "show", map[string]any{"ref": "agent:" + keyB}), "secret plan")
		})
	}
}
