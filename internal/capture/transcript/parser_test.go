package transcript

import (
	"bytes"
	"encoding/json"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestTurnBoundaries(t *testing.T) {
	items, p := feed(t, mainSrc(),
		prompt("u1", "p1", 0, "fix the build", obj{"origin": obj{"kind": "human"}, "promptSource": "typed"}),
		assistant("a1", 1, thinking("hmm")),
		assistant("a2", 2, textBlock("Looking.")),
		assistant("a3", 3, toolUse("t1", "Read", obj{"file_path": `C:\src\demo\main.go`})),
		result("r1", 4, "t1", false, obj{"type": "text", "file": obj{"filePath": `C:\src\demo\main.go`}}),
		assistant("a4", 5, textBlock("Fixed it.")),
		assistant("a5", 6, textBlock("   ")), // blank text does not replace the reply
		turnDuration("s1", 7),
		prompt("u2", "", 10, "now the tests", nil),
		assistant("a6", 11, textBlock("On it.")),
		prompt("u3", "p3", 20, "stop", nil),
	)
	ts := turns(items)
	if len(ts) != 2 {
		t.Fatalf("got %d turns: %v", len(ts), kinds(items))
	}
	t1 := ts[0]
	if t1.ID != "p1" || t1.EndReason != "turn_duration" || !t1.EndedAt.Equal(at(7)) || !t1.StartedAt.Equal(at(0)) {
		t.Fatalf("turn 1 = %+v", t1)
	}
	if t1.Prompt != "fix the build" || t1.Reply != "Fixed it." || t1.Origin != "human" {
		t.Fatalf("turn 1 text = %q / %q / %q", t1.Prompt, t1.Reply, t1.Origin)
	}
	if t1.SessionID != fxSession || t1.Cwd != fxCwd || t1.Branch != fxBranch || t1.AgentID != "" {
		t.Fatalf("turn 1 ids = %+v", t1)
	}
	wantTool := ToolCall{ID: "t1", Name: "Read", Target: "main.go", Done: true, At: at(3)}
	if len(t1.Tools) != 1 || t1.Tools[0] != wantTool {
		t.Fatalf("turn 1 tools = %+v", t1.Tools)
	}
	t2 := ts[1]
	// No promptId: the row uuid is the id. Closed by the next prompt at its
	// own last row's time.
	if t2.ID != "u2" || t2.EndReason != "next" || !t2.EndedAt.Equal(at(11)) || t2.Reply != "On it." {
		t.Fatalf("turn 2 = %+v", t2)
	}
	open := p.Open()
	if open == nil || open.ID != "p3" || !open.EndedAt.IsZero() {
		t.Fatalf("open = %+v", open)
	}
	if got := p.LastActivity(); !got.Equal(at(20)) {
		t.Fatalf("LastActivity = %v", got)
	}
	closed := p.Close(at(99), "eof")
	if len(closed) != 1 || closed[0].Turn.ID != "p3" || closed[0].Turn.EndReason != "eof" || !closed[0].Turn.EndedAt.Equal(at(99)) {
		t.Fatalf("Close = %v", kinds(closed))
	}
	if p.Open() != nil {
		t.Fatal("turn still open after Close")
	}
	if again := p.Close(at(100), "eof"); again != nil {
		t.Fatalf("second Close = %v", kinds(again))
	}
}

func TestTurnDurationWithoutOpenTurn(t *testing.T) {
	items, _ := feed(t, mainSrc(), turnDuration("s1", 1), turnDuration("s2", 2))
	if len(items) != 0 {
		t.Fatalf("items = %v", kinds(items))
	}
}

func TestCloseQuiet(t *testing.T) {
	_, p := feed(t, mainSrc(), prompt("u1", "p1", 0, "hi", nil))
	out := p.Close(at(120), "quiet")
	if len(out) != 1 || out[0].Turn.EndReason != "quiet" {
		t.Fatalf("Close = %v", kinds(out))
	}
}

func TestInterrupt(t *testing.T) {
	tests := []struct {
		name    string
		content any
	}{
		{"string", "[Request interrupted by user]"},
		{"text block", []any{textBlock("[Request interrupted by user]")}},
		{"for tool use", []any{textBlock("[Request interrupted by user for tool use]")}},
		{"padded", "  [Request interrupted by user]\n"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			intr := common("user", "ux", 5, obj{"message": obj{"role": "user", "content": tt.content}})
			items, p := feed(t, mainSrc(),
				prompt("u1", "p1", 0, "go", nil),
				assistant("a1", 1, toolUse("t1", "Bash", obj{"command": "sleep 100"})),
				intr,
			)
			if len(items) != 0 {
				t.Fatalf("interrupt emitted %v", kinds(items))
			}
			open := p.Open()
			if open == nil || open.ID != "p1" || !open.Interrupted {
				t.Fatalf("open = %+v", open)
			}
			if open.Tools[0].Done {
				t.Fatal("unanswered tool marked done")
			}
		})
	}
	t.Run("no open turn", func(t *testing.T) {
		intr := common("user", "ux", 5, obj{"message": obj{"role": "user", "content": "[Request interrupted by user]"}})
		items, p := feed(t, mainSrc(), intr)
		if len(items) != 0 || p.Open() != nil {
			t.Fatalf("interrupt started something: %v", kinds(items))
		}
	})
	t.Run("not exact", func(t *testing.T) {
		row := common("user", "ux", 5, obj{"message": obj{"role": "user", "content": "[Request interrupted by user] and more"}})
		_, p := feed(t, mainSrc(), row)
		if o := p.Open(); o == nil || o.Interrupted {
			t.Fatalf("near-miss text should be a prompt: %+v", o)
		}
	})
	t.Run("with tool result", func(t *testing.T) {
		row := result("r1", 3, "t1", true, "The user doesn't want to proceed")
		msg := row["message"].(obj)
		msg["content"] = append(msg["content"].([]any), textBlock("[Request interrupted by user for tool use]"))
		_, p := feed(t, mainSrc(),
			prompt("u1", "p1", 0, "go", nil),
			assistant("a1", 1, toolUse("t1", "Edit", obj{"file_path": `C:\src\demo\a.go`})),
			row,
		)
		o := p.Open()
		if !o.Interrupted || !o.Tools[0].Done || !o.Tools[0].IsError {
			t.Fatalf("open = %+v", o)
		}
	})
}

func TestOrigin(t *testing.T) {
	tests := []struct {
		name     string
		text     any
		extra    obj
		origin   string
		nameWant string
	}{
		{"human", "hi", obj{"origin": obj{"kind": "human"}, "promptSource": "typed"}, "human", ""},
		{"plugin", "hi", obj{"origin": obj{"kind": "plugin", "name": "irori"}, "promptSource": "system"}, "plugin", "irori"},
		{"peer", "hi", obj{"origin": obj{"kind": "peer", "name": "x"}}, "peer", ""},
		{"task notification", "<task-notification>done</task-notification>", obj{"origin": obj{"kind": "task-notification"}}, "task-notification", ""},
		{"origin wins over sdk", "hi", obj{"origin": obj{"kind": "human"}, "promptSource": "sdk"}, "human", ""},
		{"missing sdk", "hi", obj{"promptSource": "sdk"}, "sdk", ""},
		{"missing command", "<command-name>/review</command-name>", nil, "command", ""},
		{"missing command blocks", []any{textBlock("  <command-name>/x</command-name>")}, nil, "command", ""},
		{"missing human", "hello", nil, "human", ""},
		{"null origin", "hello", obj{"origin": nil}, "human", ""},
		{"empty kind", "hello", obj{"origin": obj{"kind": ""}}, "human", ""},
		{"command not at start", "run <command-name>", nil, "human", ""},
		{"missing peer", "Another Claude session sent a message:\n<teammate-message from=\"x\">hi</teammate-message>", nil, "peer", ""},
		{"missing peer tag", " <teammate-message from=\"x\">hi</teammate-message>", nil, "peer", ""},
		{"peer origin kept", "Another Claude session sent a message: hi", obj{"origin": obj{"kind": "human"}}, "human", ""},
		{"peer not at start", "see <teammate-message>", nil, "human", ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			row := common("user", "u1", 0, obj{"message": obj{"role": "user", "content": tt.text}})
			for k, v := range tt.extra {
				row[k] = v
			}
			_, p := feed(t, mainSrc(), row)
			o := p.Open()
			if o == nil || o.Origin != tt.origin || o.OriginName != tt.nameWant {
				t.Fatalf("open = %+v", o)
			}
		})
	}
}

func TestPromptText(t *testing.T) {
	long := strings.Repeat("a", MaxPrompt+5)
	tests := []struct {
		name    string
		content any
		want    string
	}{
		{"string", "plain", "plain"},
		{"blocks joined", []any{textBlock("one"), obj{"type": "image", "source": obj{}}, textBlock("two")}, "one\ntwo"},
		{"clipped", long, strings.Repeat("a", MaxPrompt-1) + "…"},
		{"image only", []any{obj{"type": "image"}}, ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			row := common("user", "u1", 0, obj{"message": obj{"role": "user", "content": tt.content}})
			_, p := feed(t, mainSrc(), row)
			if o := p.Open(); o == nil || o.Prompt != tt.want {
				t.Fatalf("open = %+v", o)
			}
		})
	}
	t.Run("reply clipped", func(t *testing.T) {
		_, p := feed(t, mainSrc(), prompt("u1", "", 0, "x", nil), assistant("a1", 1, textBlock(strings.Repeat("é", MaxReply+1))))
		if r := p.Open().Reply; r != strings.Repeat("é", MaxReply-1)+"…" {
			t.Fatalf("reply = %d runes", len([]rune(r)))
		}
	})
}

func TestMetaIsNotPrompt(t *testing.T) {
	meta := common("user", "m1", 1, obj{"isMeta": true, "message": obj{"role": "user", "content": "<system-reminder>x</system-reminder>"}})
	items, p := feed(t, mainSrc(), meta)
	if len(items) != 0 || p.Open() != nil {
		t.Fatalf("meta row started a turn")
	}
	items, p = feed(t, mainSrc(), prompt("u1", "p1", 0, "go", nil), meta)
	if len(items) != 0 || p.Open().ID != "p1" {
		t.Fatalf("meta row closed the turn: %v", kinds(items))
	}
}

func TestSummaries(t *testing.T) {
	long := strings.Repeat("s", MaxSummary+10)
	compact := func(text string) obj {
		return common("user", "c1", 3, obj{
			"promptId": "pc", "isCompactSummary": true, "isVisibleInTranscriptOnly": true,
			"message": obj{"role": "user", "content": text},
		})
	}
	tests := []struct {
		name string
		row  obj
		want Summary
	}{
		{"away", sys("s1", 1, "away_summary", obj{"content": "Was fixing tests."}),
			Summary{SessionID: fxSession, Kind: "away_summary", Text: "Was fixing tests.", RowUUID: "s1", At: at(1)}},
		{"boundary", sys("s2", 2, "compact_boundary", obj{"content": "Conversation compacted", "compactMetadata": obj{"trigger": "auto", "preTokens": 1}}),
			Summary{SessionID: fxSession, Kind: "compact_boundary", Text: "Conversation compacted", RowUUID: "s2", At: at(2)}},
		{"compact short", compact("summary text"),
			Summary{SessionID: fxSession, Kind: "compact_summary", Text: "summary text", RowUUID: "c1", At: at(3)}},
		{"compact long", compact(long),
			Summary{SessionID: fxSession, Kind: "compact_summary", Text: strings.Repeat("s", MaxSummary-1) + "…", Full: long, RowUUID: "c1", At: at(3)}},
		{"compact blocks", common("user", "c2", 4, obj{"isCompactSummary": true, "message": obj{"content": []any{textBlock("a"), textBlock("b")}}}),
			Summary{SessionID: fxSession, Kind: "compact_summary", Text: "a\nb", RowUUID: "c2", At: at(4)}},
		{"ai title", obj{"type": "ai-title", "aiTitle": "Fix build", "sessionId": fxSession},
			Summary{SessionID: fxSession, Kind: "ai_title", Text: "Fix build"}},
		{"custom title", obj{"type": "custom-title", "customTitle": "My title", "sessionId": fxSession},
			Summary{SessionID: fxSession, Kind: "custom_title", Text: "My title"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			items, p := feed(t, mainSrc(), prompt("u0", "p0", 0, "go", nil), tt.row)
			ss := summaries(items)
			if len(items) != 1 || len(ss) != 1 {
				t.Fatalf("items = %v", kinds(items))
			}
			if *ss[0] != tt.want {
				t.Fatalf("summary = %+v\nwant %+v", *ss[0], tt.want)
			}
			if p.Open() == nil || p.Open().ID != "p0" {
				t.Fatal("summary row changed the open turn")
			}
		})
	}
}

func TestPRLink(t *testing.T) {
	tests := []struct {
		name   string
		number any
		want   int
	}{
		{"string", "42", 42},
		{"int", 42, 42},
		{"hash string", "#7", 7},
		{"bad", "abc", 0},
		{"missing", nil, 0},
		{"zero", 0, 0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			row := obj{"type": "pr-link", "sessionId": fxSession, "prUrl": "https://github.com/o/r/pull/42",
				"prRepository": "o/r", "timestamp": ts(5)}
			if tt.number != nil {
				row["prNumber"] = tt.number
			}
			items, p := feed(t, mainSrc(), row)
			if tt.want == 0 {
				if len(items) != 0 {
					t.Fatalf("items = %v", kinds(items))
				}
				return
			}
			if len(items) != 1 || items[0].PRLink == nil {
				t.Fatalf("items = %v", kinds(items))
			}
			want := PRLink{SessionID: fxSession, Repo: "o/r", Number: tt.want, URL: "https://github.com/o/r/pull/42", At: at(5)}
			if *items[0].PRLink != want {
				t.Fatalf("pr = %+v", *items[0].PRLink)
			}
			if !p.LastActivity().Equal(at(5)) {
				t.Fatal("pr-link timestamp not counted")
			}
		})
	}
}

func TestIgnoredLines(t *testing.T) {
	lines := []string{
		"",
		"   ",
		"not json at all",
		`{"type":"user","message":`, // half-written
		`{"type":"api-request","uuid":"x","timestamp":"2030-01-01T00:00:00Z"}`,
		`{"type":"attachment","uuid":"x","timestamp":"` + ts(1) + `"}`,
		`{"type":"file-history-snapshot","messageId":"x"}`,
		`{"type":"queue-operation","operation":"enqueue"}`,
		`{"type":"system","subtype":"informational","content":"hello","timestamp":"` + ts(2) + `"}`,
		`{"type":"system","subtype":"local_command","content":"x"}`,
		`{"type":"user","uuid":"u","toolUseResult":[1,2],"message":{"content":"weird"}}`, // toolUseResult with no block
		`[1,2,3]`,
		`"string"`,
		`null`,
	}
	p := NewParser(mainSrc())
	for _, l := range lines {
		if items := p.Line([]byte(l)); len(items) != 0 {
			t.Fatalf("line %q gave %v", l, kinds(items))
		}
	}
	if p.Open() != nil {
		t.Fatalf("open = %+v", p.Open())
	}
	if !p.LastActivity().Equal(at(2)) {
		t.Fatalf("LastActivity = %v (api-request row must be skipped)", p.LastActivity())
	}
}

func TestCRLFLine(t *testing.T) {
	p := NewParser(mainSrc())
	b := append(line(t, prompt("u1", "p1", 0, "hi", nil)), '\r')
	p.Line(b)
	if o := p.Open(); o == nil || o.ID != "p1" {
		t.Fatalf("CRLF line not parsed: %+v", o)
	}
}

func TestSidechain(t *testing.T) {
	sub := Source{SessionID: fxSession, AgentID: "agent1"}
	mk := func(r obj, agent string) obj {
		r["isSidechain"] = agent != ""
		if agent != "" {
			r["agentId"] = agent
		}
		return r
	}
	t.Run("subagent file", func(t *testing.T) {
		items, p := feed(t, sub,
			mk(prompt("u1", "pp", 0, "do the subtask", nil), "agent1"),
			mk(prompt("x1", "px", 1, "main row", nil), ""),
			mk(prompt("x2", "py", 2, "other agent", nil), "agent2"),
			mk(assistant("a1", 3, toolUse("t1", "Write", obj{"file_path": `C:\src\demo\n.txt`})), "agent1"),
			mk(result("r1", 4, "t1", false, obj{"type": "create", "filePath": `C:\src\demo\n.txt`, "content": "x\n", "structuredPatch": []any{}, "originalFile": nil}), "agent1"),
		)
		es := edits(items)
		if len(es) != 1 || es[0].AgentID != "agent1" || es[0].SessionID != fxSession || es[0].TurnID != "pp" {
			t.Fatalf("edits = %v", kinds(items))
		}
		o := p.Open()
		if o.ID != "pp" || o.AgentID != "agent1" || o.SessionID != fxSession || o.Origin != "human" {
			t.Fatalf("open = %+v", o)
		}
		if !p.LastActivity().Equal(at(4)) {
			t.Fatalf("LastActivity = %v", p.LastActivity())
		}
	})
	t.Run("main file ignores agent rows", func(t *testing.T) {
		items, p := feed(t, mainSrc(),
			prompt("u1", "p1", 0, "main", nil),
			mk(prompt("x1", "px", 5, "sub", nil), "agent1"),
			mk(turnDuration("s1", 6), "agent1"),
		)
		if len(items) != 0 || p.Open().ID != "p1" {
			t.Fatalf("agent rows leaked: %v", kinds(items))
		}
		if !p.LastActivity().Equal(at(0)) {
			t.Fatalf("LastActivity counted ignored rows: %v", p.LastActivity())
		}
	})
}

func TestImplicitTurn(t *testing.T) {
	t.Run("result first", func(t *testing.T) {
		items, p := feed(t, mainSrc(),
			result("r1", 1, "t-unknown", false, obj{"stdout": "hi", "stderr": "", "interrupted": false}),
		)
		bs := bashes(items)
		o := p.Open()
		if o == nil || o.ID != "r1" || o.Origin != "unknown" || !o.StartedAt.Equal(at(1)) {
			t.Fatalf("open = %+v", o)
		}
		// The tool was inferred from the result's shape.
		if len(bs) != 1 || bs[0].TurnID != "r1" || bs[0].Stdout != "hi" || bs[0].Command != "" {
			t.Fatalf("bash = %v", kinds(items))
		}
	})
	t.Run("tool use first", func(t *testing.T) {
		items, p := feed(t, mainSrc(),
			assistant("a1", 1, toolUse("t1", "Edit", obj{"file_path": `C:\src\demo\a.go`})),
			result("r1", 2, "t1", false, editResult(`C:\src\demo\a.go`, ptr("x := 1\n"), "1", "2", false)),
			prompt("u1", "p1", 3, "next", nil),
		)
		ts := turns(items)
		if len(ts) != 1 || ts[0].ID != "a1" || ts[0].Origin != "unknown" || ts[0].EndReason != "next" {
			t.Fatalf("turns = %v", kinds(items))
		}
		if !reflect.DeepEqual(ts[0].EditPaths, []string{`C:\src\demo\a.go`}) || !ts[0].Tools[0].Done {
			t.Fatalf("turn = %+v", ts[0])
		}
		if p.Open().ID != "p1" {
			t.Fatal("prompt did not start a turn")
		}
	})
	t.Run("assistant text alone starts nothing", func(t *testing.T) {
		_, p := feed(t, mainSrc(), assistant("a1", 1, textBlock("hello")))
		if p.Open() != nil {
			t.Fatal("text-only assistant row started a turn")
		}
	})
	t.Run("inferred write and edit", func(t *testing.T) {
		items, _ := feed(t, mainSrc(),
			result("r1", 1, "tx", false, obj{"type": "create", "filePath": `C:\a\n.txt`, "content": "n\n", "structuredPatch": []any{}, "originalFile": nil}),
			result("r2", 2, "ty", false, editResult(`C:\a\e.txt`, ptr("a"), "a", "b", false)),
			result("r3", 3, "tz", false, obj{"type": "text", "file": obj{"filePath": `C:\a\r.txt`}}),
		)
		es := edits(items)
		if len(es) != 2 || es[0].Tool != "Write" || es[1].Tool != "Edit" || string(es[1].After) != "b" {
			t.Fatalf("edits = %v", kinds(items))
		}
	})
}

func ptr(s string) *string { return &s }

func editResult(path string, orig *string, old, new string, all bool) obj {
	m := obj{
		"filePath": path, "oldString": old, "newString": new, "replaceAll": all, "userModified": false,
		"structuredPatch": []any{obj{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1, "lines": []string{"-" + old, "+" + new}}},
	}
	if orig == nil {
		m["originalFile"] = nil
	} else {
		m["originalFile"] = *orig
	}
	return m
}

func TestEdits(t *testing.T) {
	const path = `C:\src\demo\pkg\a.go`
	type want struct {
		n        int
		before   *string // nil means Before must be nil
		after    *string
		create   bool
		patch    string
		adds     int
		dels     int
		fidelity string
	}
	tests := []struct {
		name  string
		tool  string
		input obj
		isErr bool
		tur   any
		want  want
	}{
		{"edit first occurrence", "Edit", obj{"file_path": path}, false,
			editResult(path, ptr("a b a\r\n"), "a", "c", false),
			want{1, ptr("a b a\r\n"), ptr("c b a\r\n"), false, "--- a/pkg/a.go\n+++ b/pkg/a.go\n@@ -1,1 +1,1 @@\n-a\n+c\n", 1, 1, Exact}},
		{"edit replaceAll", "Edit", obj{"file_path": path}, false,
			editResult(path, ptr("a b a"), "a", "c", true),
			want{1, ptr("a b a"), ptr("c b c"), false, "", 1, 1, Exact}},
		{"edit old not found", "Edit", obj{"file_path": path}, false,
			editResult(path, ptr("xyz"), "a", "c", false),
			want{1, ptr("xyz"), nil, false, "", 1, 1, Exact}},
		{"edit null originalFile", "Edit", obj{"file_path": path}, false,
			editResult(path, nil, "a", "c", false),
			want{1, nil, nil, false, "", 1, 1, Exact}},
		{"edit empty originalFile", "Edit", obj{"file_path": path}, false,
			editResult(path, ptr(""), "a", "c", false),
			want{1, ptr(""), nil, false, "", 1, 1, Exact}},
		{"edit empty oldString", "Edit", obj{"file_path": path}, false,
			editResult(path, ptr("abc"), "", "c", false),
			want{1, ptr("abc"), nil, false, "", 1, 1, Exact}},
		{"edit staged", "Edit", obj{"file_path": path}, false,
			func() obj { m := editResult(path, ptr("a"), "a", "b", false); m["staged"] = true; return m }(),
			want{n: 0}},
		{"edit error", "Edit", obj{"file_path": path}, true,
			editResult(path, ptr("a"), "a", "b", false), want{n: 0}},
		{"edit string result", "Edit", obj{"file_path": path}, true,
			"Error: String to replace not found in file.", want{n: 0}},
		{"edit array result", "Edit", obj{"file_path": path}, true,
			[]any{obj{"type": "text", "text": "Error"}}, want{n: 0}},
		{"edit path from input", "Edit", obj{"file_path": path}, false,
			obj{"oldString": "a", "newString": "b", "originalFile": "a", "structuredPatch": []any{}},
			want{1, ptr("a"), ptr("b"), false, "", 0, 0, Exact}},
		{"multi edits array", "Edit", obj{"file_path": path}, false,
			obj{"filePath": path, "originalFile": "one two three", "edits": []any{
				obj{"old_string": "one", "new_string": "1"},
				obj{"old_string": "three", "new_string": "3", "replace_all": true},
			}, "structuredPatch": []any{}},
			want{1, ptr("one two three"), ptr("1 two 3"), false, "", 0, 0, Exact}},
		{"multi edits step missing", "Edit", obj{"file_path": path}, false,
			obj{"filePath": path, "originalFile": "one", "edits": []any{
				obj{"old_string": "one", "new_string": "1"},
				obj{"old_string": "one", "new_string": "x"},
			}},
			want{1, ptr("one"), nil, false, "", 0, 0, Exact}},
		{"write create", "Write", obj{"file_path": path}, false,
			obj{"type": "create", "filePath": path, "content": "l1\r\nl2\n", "structuredPatch": []any{}, "originalFile": nil},
			want{1, nil, ptr("l1\r\nl2\n"), true, "--- a/pkg/a.go\n+++ b/pkg/a.go\n@@ -0,0 +1,2 @@\n+l1\n+l2\n", 2, 0, Exact}},
		{"write create empty", "Write", obj{"file_path": path}, false,
			obj{"type": "create", "filePath": path, "content": "", "structuredPatch": []any{}, "originalFile": nil},
			want{1, nil, ptr(""), true, "", 0, 0, Exact}},
		{"write update", "Write", obj{"file_path": path}, false,
			obj{"type": "update", "filePath": path, "content": "new", "originalFile": "old",
				"structuredPatch": []any{obj{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1, "lines": []string{"-old", "+new"}}}},
			want{1, ptr("old"), ptr("new"), false, "--- a/pkg/a.go\n+++ b/pkg/a.go\n@@ -1,1 +1,1 @@\n-old\n+new\n", 1, 1, Exact}},
		{"write update null original", "Write", obj{"file_path": path}, false,
			obj{"type": "update", "filePath": path, "content": "new", "originalFile": nil, "structuredPatch": []any{}},
			want{1, nil, ptr("new"), false, "", 0, 0, Exact}},
		{"write staged", "Write", obj{"file_path": path}, false,
			obj{"type": "create", "filePath": path, "content": "x", "staged": true}, want{n: 0}},
		{"write string error", "Write", obj{"file_path": path}, true, "Error: denied", want{n: 0}},
		{"notebook", "NotebookEdit", obj{"notebook_path": `C:\src\demo\n.ipynb`}, false,
			obj{"new_source": "print(1)", "cell_id": "c1", "edit_mode": "replace", "notebook_path": `C:\src\demo\n.ipynb`},
			want{1, nil, nil, false, "", 0, 0, Exact}},
		{"notebook error field", "NotebookEdit", obj{"notebook_path": `C:\src\demo\n.ipynb`}, false,
			obj{"error": "no such cell", "cell_id": "c1", "notebook_path": `C:\src\demo\n.ipynb`}, want{n: 0}},
		{"read is not an edit", "Read", obj{"file_path": path}, false,
			obj{"type": "text", "file": obj{"filePath": path, "content": "x"}}, want{n: 0}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			items, p := feed(t, mainSrc(),
				prompt("u1", "p1", 0, "go", nil),
				assistant("a1", 1, toolUse("t1", tt.tool, tt.input)),
				result("r1", 2, "t1", tt.isErr, tt.tur),
			)
			es := edits(items)
			if len(es) != tt.want.n {
				t.Fatalf("got %d edits: %v", len(es), kinds(items))
			}
			if !p.Open().Tools[0].Done || p.Open().Tools[0].IsError != tt.isErr {
				t.Fatalf("tool = %+v", p.Open().Tools[0])
			}
			if tt.want.n == 0 {
				if len(p.Open().EditPaths) != 0 {
					t.Fatalf("EditPaths = %v", p.Open().EditPaths)
				}
				return
			}
			e := es[0]
			if e.Tool != tt.tool || e.SessionID != fxSession || e.TurnID != "p1" || e.ToolUseID != "t1" ||
				e.RowUUID != "r1" || e.Cwd != fxCwd || !e.At.Equal(at(2)) || e.Fidelity != tt.want.fidelity || e.Create != tt.want.create {
				t.Fatalf("edit = %+v", e)
			}
			checkBytes(t, "before", e.Before, tt.want.before)
			checkBytes(t, "after", e.After, tt.want.after)
			if tt.want.patch != "" && e.Patch != tt.want.patch {
				t.Fatalf("patch = %q\nwant %q", e.Patch, tt.want.patch)
			}
			if e.Adds != tt.want.adds || e.Dels != tt.want.dels {
				t.Fatalf("adds/dels = %d/%d", e.Adds, e.Dels)
			}
			if !reflect.DeepEqual(p.Open().EditPaths, []string{e.Path}) {
				t.Fatalf("EditPaths = %v", p.Open().EditPaths)
			}
		})
	}
}

func checkBytes(t *testing.T, what string, got []byte, want *string) {
	t.Helper()
	if want == nil {
		if got != nil {
			t.Fatalf("%s = %q, want nil", what, got)
		}
		return
	}
	if got == nil || string(got) != *want {
		t.Fatalf("%s = %q (nil %v), want %q", what, got, got == nil, *want)
	}
}

func TestBash(t *testing.T) {
	bigOut := strings.Repeat("o", MaxStdout+100)
	gitop := obj{"pr": obj{"number": 12, "url": "https://github.com/o/r/pull/12", "action": "created"}}
	run := func(cmd string, isErr bool, tur any) ([]Item, *Parser) {
		return feed(t, mainSrc(),
			prompt("u1", "p1", 0, "go", nil),
			assistant("a1", 1, toolUse("t1", "Bash", obj{"command": cmd, "description": "d"})),
			result("r1", 2, "t1", isErr, tur),
		)
	}
	t.Run("plain", func(t *testing.T) {
		items, _ := run("git status\ngit log", false, obj{"stdout": bigOut, "stderr": "", "interrupted": false, "isImage": false})
		bs := bashes(items)
		if len(items) != 1 || len(bs) != 1 {
			t.Fatalf("items = %v", kinds(items))
		}
		b := bs[0]
		if b.Command != "git status\ngit log" || len(b.Stdout) != MaxStdout || b.TurnID != "p1" || b.ToolUseID != "t1" ||
			b.Cwd != fxCwd || !b.At.Equal(at(2)) || b.IsError || b.GitOperation != nil || b.SessionID != fxSession {
			t.Fatalf("bash = %+v", b)
		}
	})
	t.Run("git operation", func(t *testing.T) {
		items, _ := run("gh pr create", false, obj{"stdout": "https://github.com/o/r/pull/12\n", "stderr": "", "gitOperation": gitop})
		b := bashes(items)[0]
		var got map[string]any
		if err := json.Unmarshal(b.GitOperation, &got); err != nil || got["pr"] == nil {
			t.Fatalf("gitOperation = %s", b.GitOperation)
		}
	})
	t.Run("string error result", func(t *testing.T) {
		items, p := run("false", true, "Error: Exit code 1\nboom")
		b := bashes(items)
		if len(b) != 1 || !b[0].IsError || b[0].Stdout != "Error: Exit code 1\nboom" {
			t.Fatalf("bash = %v", kinds(items))
		}
		if !p.Open().Tools[0].IsError {
			t.Fatal("tool not marked error")
		}
	})
	t.Run("array result falls back to the block text", func(t *testing.T) {
		const msg = "Exit code 1\nfatal: boom"
		items, _ := feed(t, mainSrc(),
			prompt("u1", "p1", 0, "go", nil),
			assistant("a1", 1, toolUse("t1", "Bash", obj{"command": "x"})),
			common("user", "r1", 2, obj{
				"message": obj{"role": "user", "content": []any{
					obj{"type": "tool_result", "tool_use_id": "t1", "is_error": true, "content": msg},
				}},
				"toolUseResult": []any{obj{"type": "text", "text": msg}},
			}),
		)
		if b := bashes(items); len(b) != 1 || b[0].Stdout != msg || !b[0].IsError {
			t.Fatalf("bash = %v", kinds(items))
		}
	})
	t.Run("no toolUseResult", func(t *testing.T) {
		items, _ := run("x", false, nil)
		if b := bashes(items); len(b) != 1 || b[0].Stdout != "ok" {
			t.Fatalf("bash = %v", kinds(items))
		}
	})
	t.Run("bashEditDiff", func(t *testing.T) {
		hunk := obj{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 2, "lines": []string{"-a", "+b", "+c"}}
		items, p := run("sed -i s/a/b/ *.go", false, obj{
			"stdout": "", "stderr": "", "interrupted": false,
			"bashEditDiff": obj{
				"files": []any{
					obj{"filePath": `c:/SRC/demo/one.go`, "hunks": []any{hunk}},
					obj{"filePath": `C:\src\demo\gone.go`, "hunks": []any{obj{"oldStart": 1, "oldLines": 1, "newStart": 0, "newLines": 0, "lines": []string{"-x"}}}, "deleted": true},
				},
				"moreFiles":    1,
				"changedFiles": []string{`C:\src\demo\one.go`, `C:\src\demo\gone.go`, `C:\src\demo\two.go`, `C:\src\demo\two.go`},
			},
		})
		if got := kinds(items); !reflect.DeepEqual(got, []string{`edit:C:\src\demo\one.go`, `edit:C:\src\demo\gone.go`, `edit:C:\src\demo\two.go`, "bash:t1"}) {
			t.Fatalf("items = %v", got)
		}
		es := edits(items)
		for _, e := range es {
			if e.Tool != "Bash" || e.Fidelity != Heuristic || e.Before != nil || e.After != nil || e.TurnID != "p1" {
				t.Fatalf("edit = %+v", e)
			}
		}
		if es[0].Patch != "--- a/one.go\n+++ b/one.go\n@@ -1,1 +1,2 @@\n-a\n+b\n+c\n" || es[0].Adds != 2 || es[0].Dels != 1 {
			t.Fatalf("one.go = %q %d %d", es[0].Patch, es[0].Adds, es[0].Dels)
		}
		if es[1].Dels != 1 || es[1].Patch == "" {
			t.Fatalf("gone.go = %+v", es[1])
		}
		if es[2].Patch != "" || es[2].Adds != 0 {
			t.Fatalf("two.go has a patch: %+v", es[2])
		}
		if got := p.Open().EditPaths; len(got) != 3 {
			t.Fatalf("EditPaths = %v", got)
		}
	})
	t.Run("empty changedFiles", func(t *testing.T) {
		items, _ := run("ls", false, obj{"stdout": "", "bashEditDiff": obj{"files": []any{}, "moreFiles": 0, "changedFiles": []any{}}})
		if len(edits(items)) != 0 || len(bashes(items)) != 1 {
			t.Fatalf("items = %v", kinds(items))
		}
	})
	t.Run("stdout clipped on rune boundary", func(t *testing.T) {
		s := strings.Repeat("a", MaxStdout-1) + "é"
		items, _ := run("x", false, obj{"stdout": s})
		if got := bashes(items)[0].Stdout; got != strings.Repeat("a", MaxStdout-1) {
			t.Fatalf("stdout len %d", len(got))
		}
	})
}

func TestEditPathsDedupe(t *testing.T) {
	w := func(id, uuid string, sec int, path string) []obj {
		return []obj{
			assistant("a"+id, sec, toolUse(id, "Write", obj{"file_path": path})),
			result(uuid, sec+1, id, false, obj{"type": "update", "filePath": path, "content": "x", "originalFile": "y", "structuredPatch": []any{}}),
		}
	}
	rows := []obj{prompt("u1", "p1", 0, "go", nil)}
	rows = append(rows, w("t1", "r1", 1, `C:\src\demo\b.go`)...)
	rows = append(rows, w("t2", "r2", 3, `C:\src\demo\a.go`)...)
	rows = append(rows, w("t3", "r3", 5, `c:/src/demo/B.go`)...)
	rows = append(rows, w("t4", "r4", 7, `C:\src\demo\b.go`)...)
	rows = append(rows, turnDuration("s1", 9))
	items, _ := feed(t, mainSrc(), rows...)
	ts := turns(items)
	if len(ts) != 1 || !reflect.DeepEqual(ts[0].EditPaths, []string{`C:\src\demo\b.go`, `C:\src\demo\a.go`}) {
		t.Fatalf("turns = %+v", ts)
	}
	if len(edits(items)) != 4 {
		t.Fatalf("every edit is still emitted: %v", kinds(items))
	}
	// Unix paths are case-sensitive.
	if pathKey("/a/B.go") == pathKey("/a/b.go") {
		t.Fatal("unix keys folded case")
	}
}

func TestTurnEmittedAfterItsItems(t *testing.T) {
	items, _ := feed(t, mainSrc(),
		prompt("u1", "p1", 0, "go", nil),
		assistant("a1", 1, toolUse("t1", "Write", obj{"file_path": `C:\src\demo\x.go`})),
		assistant("a2", 1, toolUse("t2", "Bash", obj{"command": "go test"})),
		result("r1", 2, "t1", false, obj{"type": "create", "filePath": `C:\src\demo\x.go`, "content": "x", "originalFile": nil}),
		result("r2", 3, "t2", false, obj{"stdout": "ok"}),
		obj{"type": "pr-link", "sessionId": fxSession, "prNumber": "3", "prUrl": "u", "prRepository": "o/r", "timestamp": ts(4)},
		turnDuration("s1", 5),
		prompt("u2", "p2", 6, "again", nil),
		assistant("a3", 7, toolUse("t3", "Bash", obj{"command": "ls"})),
		result("r3", 8, "t3", false, obj{"stdout": ""}),
		prompt("u3", "p3", 9, "third", nil),
	)
	want := []string{`edit:C:\src\demo\x.go`, "bash:t2", "pr:3", "turn:p1", "bash:t3", "turn:p2"}
	if got := kinds(items); !reflect.DeepEqual(got, want) {
		t.Fatalf("order = %v\nwant %v", got, want)
	}
}

func TestLateResult(t *testing.T) {
	// A result arriving after its turn closed keeps the old turn id and
	// does not touch the new turn.
	items, p := feed(t, mainSrc(),
		prompt("u1", "p1", 0, "go", nil),
		assistant("a1", 1, toolUse("t1", "Write", obj{"file_path": `C:\src\demo\x.go`})),
		prompt("u2", "p2", 2, "next", nil),
		result("r1", 3, "t1", false, obj{"type": "create", "filePath": `C:\src\demo\x.go`, "content": "x", "originalFile": nil}),
	)
	ts := turns(items)
	es := edits(items)
	if len(ts) != 1 || ts[0].Tools[0].Done || len(es) != 1 || es[0].TurnID != "p1" {
		t.Fatalf("items = %v", kinds(items))
	}
	if len(p.Open().EditPaths) != 0 {
		t.Fatalf("new turn got the late edit: %v", p.Open().EditPaths)
	}
}

func TestToolsCap(t *testing.T) {
	rows := []obj{prompt("u1", "p1", 0, "go", nil)}
	n := MaxToolsShown + 5
	for i := 0; i < n; i++ {
		id := "t" + itoa(i)
		rows = append(rows, assistant("a"+itoa(i), 1, toolUse(id, "Grep", obj{"pattern": "x"})))
	}
	last := "t" + itoa(n-1)
	rows = append(rows,
		result("r1", 2, "t0", false, obj{"mode": "files"}),
		assistant("ae", 3, toolUse("te", "Write", obj{"file_path": `C:\src\demo\late.go`})),
		result("r2", 4, last, false, obj{"mode": "files"}),
		result("r3", 5, "te", false, obj{"type": "create", "filePath": `C:\src\demo\late.go`, "content": "x", "originalFile": nil}),
	)
	items, p := feed(t, mainSrc(), rows...)
	o := p.Open()
	if len(o.Tools) != MaxToolsShown || o.ToolsMore != 6 || !o.Tools[0].Done {
		t.Fatalf("tools = %d more = %d", len(o.Tools), o.ToolsMore)
	}
	if len(edits(items)) != 1 || !reflect.DeepEqual(o.EditPaths, []string{`C:\src\demo\late.go`}) {
		t.Fatalf("edit past the cap lost: %v", kinds(items))
	}
}

func itoa(i int) string { return strconv.Itoa(i) }

func TestOpenIsACopy(t *testing.T) {
	_, p := feed(t, mainSrc(),
		prompt("u1", "p1", 0, "go", nil),
		assistant("a1", 1, toolUse("t1", "Write", obj{"file_path": `C:\x`})),
		result("r1", 2, "t1", false, obj{"type": "create", "filePath": `C:\x`, "content": "x", "originalFile": nil}),
	)
	o := p.Open()
	o.Tools[0].Name = "changed"
	o.EditPaths[0] = "changed"
	o.Prompt = "changed"
	again := p.Open()
	if again.Tools[0].Name != "Write" || again.EditPaths[0] != `C:\x` || again.Prompt != "go" {
		t.Fatalf("Open shares state: %+v", again)
	}
}

func TestLastActivityNewest(t *testing.T) {
	_, p := feed(t, mainSrc(),
		prompt("u1", "p1", 10, "go", nil),
		assistant("a1", 5, textBlock("older row")),
		obj{"type": "ai-title", "aiTitle": "t", "sessionId": fxSession},
		common("attachment", "x", 12, nil),
		common("user", "bad", 0, obj{"timestamp": "not a time", "message": obj{"content": "[Request interrupted by user]"}}),
	)
	if !p.LastActivity().Equal(at(12)) {
		t.Fatalf("LastActivity = %v", p.LastActivity())
	}
	if NewParser(mainSrc()).LastActivity() != (time.Time{}) {
		t.Fatal("fresh parser has activity")
	}
}

// TestFullSession runs a longer synthetic session and checks every item
// has exactly one field set.
func TestFullSession(t *testing.T) {
	var buf bytes.Buffer
	rows := []obj{
		{"type": "permission-mode", "permissionMode": "default", "sessionId": fxSession},
		prompt("u1", "p1", 0, "<command-name>/init</command-name>", nil),
		assistant("a1", 1, thinking("plan"), textBlock("Writing."), toolUse("t1", "Write", obj{"file_path": `C:\src\demo\README.md`})),
		result("r1", 2, "t1", false, obj{"type": "create", "filePath": `C:\src\demo\README.md`, "content": "# demo\n", "structuredPatch": []any{}, "originalFile": nil}),
		assistant("a2", 3, toolUse("t2", "Edit", obj{"file_path": `C:\src\demo\README.md`})),
		result("r2", 4, "t2", false, editResult(`C:\src\demo\README.md`, ptr("# demo\n"), "demo", "Demo", false)),
		assistant("a3", 5, toolUse("t3", "Bash", obj{"command": "git commit -am x"})),
		result("r3", 6, "t3", false, obj{"stdout": "[main abc] x", "stderr": ""}),
		turnDuration("s1", 7),
		sys("s2", 8, "away_summary", obj{"content": "Set up README."}),
		obj{"type": "ai-title", "aiTitle": "README", "sessionId": fxSession},
		obj{"type": "pr-link", "sessionId": fxSession, "prNumber": 9, "prUrl": "u", "prRepository": "o/r", "timestamp": ts(9)},
		sys("s3", 10, "compact_boundary", obj{"content": "Conversation compacted"}),
		common("user", "c1", 11, obj{"isCompactSummary": true, "message": obj{"content": "summary"}}),
		prompt("u2", "p2", 12, "continue", obj{"origin": obj{"kind": "task-notification"}}),
	}
	for _, r := range rows {
		buf.Write(line(t, r))
		buf.WriteString("\r\n")
	}
	p := NewParser(mainSrc())
	var items []Item
	for _, l := range bytes.Split(buf.Bytes(), []byte("\n")) {
		items = append(items, p.Line(l)...)
	}
	items = append(items, p.Close(at(20), "eof")...)
	want := []string{
		`edit:C:\src\demo\README.md`, `edit:C:\src\demo\README.md`, "bash:t3", "turn:p1",
		"summary:away_summary", "summary:ai_title", "pr:9", "summary:compact_boundary", "summary:compact_summary",
		"turn:p2",
	}
	if got := kinds(items); !reflect.DeepEqual(got, want) {
		t.Fatalf("items = %v\nwant %v", got, want)
	}
	ts := turns(items)
	if ts[0].Origin != "command" || ts[0].Reply != "Writing." || len(ts[0].EditPaths) != 1 || ts[1].Origin != "task-notification" {
		t.Fatalf("turns = %+v %+v", ts[0], ts[1])
	}
	es := edits(items)
	if string(es[1].Before) != "# demo\n" || string(es[1].After) != "# Demo\n" {
		t.Fatalf("edit chain = %q -> %q", es[1].Before, es[1].After)
	}
}

func TestSamePromptID(t *testing.T) {
	peer := "Another Claude session sent a message:\n<teammate-message from=\"other\">status?</teammate-message>"
	t.Run("follow-up continues the open turn", func(t *testing.T) {
		items, p := feed(t, mainSrc(),
			prompt("u1", "p1", 0, "do the thing", obj{"origin": obj{"kind": "human"}}),
			assistant("a1", 1, textBlock("Doing.")),
			prompt("u2", "p1", 2, peer, nil),
			assistant("a2", 3, toolUse("t1", "Bash", obj{"command": "ls"})),
		)
		if len(turns(items)) != 0 {
			t.Fatalf("closed a turn: %v", kinds(items))
		}
		o := p.Open()
		if o == nil || o.ID != "p1" || o.Origin != "human" || o.Prompt != "do the thing" || len(o.Tools) != 1 {
			t.Fatalf("open = %+v", o)
		}
		out := p.Close(at(9), "eof")
		if len(out) != 1 || !out[0].Turn.StartedAt.Equal(at(0)) {
			t.Fatalf("Close = %v", kinds(out))
		}
	})
	t.Run("after its turn closed gets its own id", func(t *testing.T) {
		items, p := feed(t, mainSrc(),
			prompt("u1", "p1", 0, "do the thing", obj{"origin": obj{"kind": "human"}}),
			turnDuration("s1", 1),
			prompt("u2", "p1", 2, peer, nil),
		)
		ts := turns(items)
		if len(ts) != 1 || ts[0].ID != "p1" {
			t.Fatalf("turns = %v", kinds(items))
		}
		o := p.Open()
		if o == nil || o.ID != "u2" || o.Origin != "peer" {
			t.Fatalf("open = %+v", o)
		}
	})
	t.Run("local command stdout is not a prompt", func(t *testing.T) {
		items, p := feed(t, mainSrc(),
			prompt("u1", "p1", 0, "<command-name>/model</command-name>", nil),
			prompt("u2", "p1", 3, "<local-command-stdout>Set model</local-command-stdout>", nil),
			prompt("u3", "p9", 5, "  <local-command-stderr>oops</local-command-stderr>", nil),
		)
		if len(items) != 0 {
			t.Fatalf("items = %v", kinds(items))
		}
		o := p.Open()
		if o == nil || o.ID != "p1" || o.Origin != "command" {
			t.Fatalf("open = %+v", o)
		}
	})
	t.Run("local command stdout advances the turn end", func(t *testing.T) {
		items, _ := feed(t, mainSrc(),
			prompt("u1", "p1", 0, "<command-name>/model</command-name>", nil),
			prompt("u2", "p1", 3, "<local-command-stdout>Set model</local-command-stdout>", nil),
			prompt("u3", "p2", 5, "next", nil),
		)
		ts := turns(items)
		if len(ts) != 1 || ts[0].ID != "p1" || !ts[0].EndedAt.Equal(at(3)) {
			t.Fatalf("turns = %+v", ts)
		}
	})
}

func TestSamePromptIDLater(t *testing.T) {
	// A row reusing an older turn's promptId while another turn is open.
	items, p := feed(t, mainSrc(),
		prompt("u1", "p1", 0, "one", nil),
		prompt("u2", "p2", 1, "two", nil),
		prompt("u3", "p1", 2, "Another Claude session sent a message: hi", nil),
	)
	ts := turns(items)
	if len(ts) != 2 || ts[0].ID != "p1" || ts[1].ID != "p2" {
		t.Fatalf("turns = %v", kinds(items))
	}
	if o := p.Open(); o == nil || o.ID != "u3" || o.Origin != "peer" {
		t.Fatalf("open = %+v", o)
	}
}

func TestResultAfterTurnClosed(t *testing.T) {
	items, p := feed(t, mainSrc(),
		prompt("u1", "p1", 0, "go", nil),
		assistant("a1", 1, toolUse("t1", "Bash", obj{"command": "make"})),
		turnDuration("s1", 2),
		result("r1", 3, "t1", false, obj{"stdout": "done", "stderr": ""}),
		prompt("u2", "p2", 4, "next", nil),
	)
	ts := turns(items)
	if len(ts) != 1 || ts[0].ID != "p1" || ts[0].EndReason != "turn_duration" {
		t.Fatalf("turns = %v", kinds(items))
	}
	bs := bashes(items)
	if len(bs) != 1 || bs[0].TurnID != "p1" {
		t.Fatalf("bash = %v", kinds(items))
	}
	if o := p.Open(); o == nil || o.ID != "p2" {
		t.Fatalf("open = %+v", o)
	}
}
