// Golden exporter for crates/capture's transcript module (capture.md §2.1,
// §2.2, §6 "transcript").
//
// Injected into I:internal/capture/transcript by tools/goldens/export.sh
// with `go test -overlay`, together with transcript_fixture_test.go (the
// package's fixture file with feed recording). It runs every Go transcript
// test; when IRORI_GOLDEN_OUT is set, TestMain writes
// $IRORI_GOLDEN_OUT/capture/transcript.json:
//
//	feeds    every feed call of every test: the source, the exact lines fed,
//	         the items Go's parser returned (with the fields JSON hides:
//	         before, after, patch, full), the open turn and LastActivity
//	vectors  SourceFor, SkipLine, Target, UnifiedDiff, clip and clipBytes
//	         over the Go test tables plus extras
//
// Every fixture is synthetic; no real transcript content is used.
package transcript

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

type goldFeed struct {
	Test  string            `json:"test"`
	Src   Source            `json:"src"`
	Lines []string          `json:"lines"`
	Items []json.RawMessage `json:"items"`
	Open  *Turn             `json:"open"`
	Last  time.Time         `json:"last"`
	// Set when the feed ends with p.Close(Close.At, Close.Reason): Items
	// holds the Line items, then the Close items; Open and Last are taken
	// before the Close.
	Close *goldClose `json:"close,omitempty"`
}

type goldClose struct {
	At     time.Time `json:"at"`
	Reason string    `json:"reason"`
}

var (
	goldMu    sync.Mutex
	goldFeeds []goldFeed
)

// goldItem is one Item as JSON, with the fields Go's tags hide.
func goldItem(it Item) json.RawMessage {
	var v any
	switch {
	case it.Turn != nil:
		v = map[string]any{"turn": it.Turn}
	case it.Edit != nil:
		var before, after *string
		if it.Edit.Before != nil {
			s := string(it.Edit.Before)
			before = &s
		}
		if it.Edit.After != nil {
			s := string(it.Edit.After)
			after = &s
		}
		v = map[string]any{"edit": it.Edit, "before": before, "after": after, "patch": it.Edit.Patch}
	case it.Summary != nil:
		v = map[string]any{"summary": it.Summary, "full": it.Summary.Full}
	case it.PRLink != nil:
		v = map[string]any{"prLink": it.PRLink}
	case it.Bash != nil:
		v = map[string]any{"bash": it.Bash}
	}
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return b
}

func goldItems(items []Item) []json.RawMessage {
	out := []json.RawMessage{}
	for _, it := range items {
		out = append(out, goldItem(it))
	}
	return out
}

func recordFeed(test string, src Source, lines []string, items []Item, p *Parser) {
	goldMu.Lock()
	defer goldMu.Unlock()
	goldFeeds = append(goldFeeds, goldFeed{
		Test: test, Src: src, Lines: lines, Items: goldItems(items), Open: p.Open(), Last: p.LastActivity(),
	})
}

func TestMain(m *testing.M) {
	code := m.Run()
	if dir := os.Getenv("IRORI_GOLDEN_OUT"); dir != "" && code == 0 {
		if err := writeTranscriptGolden(dir); err != nil {
			os.Stderr.WriteString("transcript golden: " + err.Error() + "\n")
			code = 1
		}
	}
	os.Exit(code)
}

var goldVectors = map[string]any{}

func writeTranscriptGolden(dir string) error {
	out := map[string]any{"feeds": goldFeeds, "vectors": goldVectors}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(out); err != nil {
		return err
	}
	path := filepath.Join(dir, "capture", "transcript.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, buf.Bytes(), 0o644)
}

// TestGoldenTranscriptVectors fills goldVectors (written by TestMain).
func TestGoldenTranscriptVectors(t *testing.T) {
	const win = `C:\Users\me\.claude\projects`
	const nix = "/home/me/.claude/projects"
	type src struct {
		Dir, Path string
		Src       Source
		OK        bool
	}
	var sources []src
	for _, c := range [][2]string{
		{win, win + `\C--src-app\abc-123.jsonl`},
		{win, "C:/Users/me/.claude/projects/C--src-app/abc_123.jsonl"},
		{win, `c:\users\ME\.claude\projects\slug\s1.jsonl`},
		{win + `\`, win + `\slug\s1.jsonl`},
		{nix, nix + "/-home-me-app/s1.jsonl"},
		{nix, "/HOME/me/.claude/projects/slug/s1.jsonl"},
		{win, win + `\slug\s1\subagents\agent-a1b2.jsonl`},
		{win, win + `\slug\s1\subagents\workflows\wf_x\agent-y.jsonl`},
		{nix, nix + "/slug/s1/subagents/workflows/wf_x/agent-y.jsonl"},
		{win, win + `\slug\s1\subagents\workflows\wf_x\journal.jsonl`},
		{win, win + `\slug\s1\subagents\agent-.jsonl`},
		{win, win + `\slug\s1\subagents\agent-a.b.jsonl`},
		{win, win + `\slug\s1\other\agent-a.jsonl`},
		{win, win + `\slug\s1\subagents\agent-a.meta.json`},
		{win, win + `\slug\s 1.jsonl`},
		{win, win + `\slug\s.1.jsonl`},
		{win, win + `\slug\s!1\subagents\agent-a.jsonl`},
		{win, win + `\slug\s1.json`},
		{win, win + `\slug\s1.key`},
		{win, win + `\s1.jsonl`},
		{win, win + `\slug\x\s1.jsonl`},
		{win, `C:\Users\me\.claude\s1.jsonl`},
		{win, `C:\Users\me\.claude\projects2\slug\s1.jsonl`},
		{win, win + `\slug\..\..\x\s1.jsonl`},
		{win, win},
		{"", `\slug\s1.jsonl`},
		{win, `C:\Users\mé\.claude\projects\slug\s1.jsonl`},
		{`C:\Users\MÉ\.claude\projects`, `c:\users\mé\.claude\projects\slug\s1.jsonl`},
		{`\\host\share\p`, `//HOST/share/p/slug/s1.jsonl`},
		{"/a/./b/../projects", "/a/projects/slug/s1.jsonl"},
		{win, win + `\slug\.\s1.jsonl`},
		{win, win + `\slug\s1\subagents\x\y\z\agent-Q_9-z.jsonl`},
	} {
		s, ok := SourceFor(c[0], c[1])
		sources = append(sources, src{c[0], c[1], s, ok})
	}
	goldVectors["sourceFor"] = sources

	type skip struct {
		Line string
		Skip bool
	}
	var skips []skip
	for _, l := range []string{"", "   ", "\t\r", `{"type":"api-request","x":1}`, `{"type":"api-request-start"}`,
		`   {"type":"api-request"}`, `{"type":"user"}`, `{ "type":"api-request"}`,
		`{"type":"assistant","note":"{\"type\":\"api-request"}`, "not json", "\t \t{\"type\":\"api-request\"}",
		"\u00a0", "\v", "\r\n"} {
		skips = append(skips, skip{l, SkipLine([]byte(l))})
	}
	goldVectors["skipLine"] = skips

	long := strings.Repeat("é", MaxTarget+20)
	type tgt struct {
		Tool, Input, Cwd, Want string
	}
	var targets []tgt
	for _, c := range [][3]string{
		{"Read", `{"file_path":"C:\\src\\app\\internal\\x.go"}`, `C:\src\app`},
		{"Edit", `{"file_path":"C:/src/app/a/b.go"}`, `C:\src\app`},
		{"Write", `{"file_path":"C:\\src\\app/a\\b.go"}`, `C:/src/app/`},
		{"Edit", `{"file_path":"c:\\SRC\\app\\x.go"}`, `C:\src\app`},
		{"Read", `{"file_path":"C:\\other\\x.go"}`, `C:\src\app`},
		{"Read", `{"file_path":"C:\\src\\app2\\x.go"}`, `C:\src\app`},
		{"Read", `{"file_path":"C:\\x.go"}`, ""},
		{"Read", `{"file_path":"/home/me/app/x.go"}`, "/home/me/app"},
		{"Read", `{"file_path":"/Home/me/app/x.go"}`, "/home/me/app"},
		{"NotebookEdit", `{"notebook_path":"C:\\src\\app\\n.ipynb"}`, `C:\src\app`},
		{"Bash", `{"command":"\n  git status\r\ngit diff"}`, ""},
		{"Grep", `{"pattern":"func \\w+","path":"x"}`, ""},
		{"Glob", `{"pattern":"**/*.go"}`, ""},
		{"WebFetch", `{"url":"https://example.com/a"}`, ""},
		{"WebSearch", `{"query":"go generics"}`, ""},
		{"Agent", `{"description":"Find callers"}`, ""},
		{"Task", `{"description":"Run tests"}`, ""},
		{"TodoWrite", `{"todos":[]}`, ""},
		{"PowerShell", `{"command":"ls\nGet-Item x"}`, ""},
		{"Read", `not json`, ""},
		{"Read", `null`, ""},
		{"Bash", `{"command":5}`, ""},
		{"Grep", `{"pattern":"` + long + `"}`, ""},
		{"Bash", `{"command":"ls","COMMAND":"pwd"}`, ""},
		{"Bash", `{"Command":"ls","command":5}`, ""},
		{"Bash", `{"description":5,"command":"git log"}`, ""},
		{"Read", `{"file_path":"C:\\src\\app"}`, `C:\src\app`},
		{"Read", `{"file_path":"C:\\src\\app\\"}`, `C:\src\app`},
		{"Read", `{"file_path":"C:\\src\\app\\\\x"}`, `C:\src\app`},
		{"Grep", `{"pattern":"aaaa bbbb ` + strings.Repeat("c", 200) + `"}`, ""},
		{"Grep", `{"pattern":"` + strings.Repeat("ab ", 100) + `"}`, ""},
		{"Read", `{"file_path":"C:\\Users\\Mé\\x.go"}`, `c:\users\MÉ`},
		// TestPowerShellTarget's input.
		{"PowerShell", `{"command":"\n  Get-ChildItem\r\nls"}`, ""},
	} {
		targets = append(targets, tgt{c[0], c[1], c[2], Target(c[0], json.RawMessage(c[1]), c[2])})
	}
	goldVectors["target"] = targets

	type diff struct {
		Path       string
		Hunks      []Hunk
		Diff       string
		Adds, Dels int
	}
	var diffs []diff
	for _, d := range []diff{
		{Path: `src\a.go`, Hunks: []Hunk{{1, 2, 1, 3, []string{" a", "-b", "+c", "+d"}}, {10, 1, 11, 1, []string{"-x", "+y"}}}},
		{Path: "b.go"},
		{Path: "c.go", Hunks: []Hunk{{0, 0, 1, 1, []string{"+only"}}}},
		{Path: "d.go", Hunks: []Hunk{{1, 1, 1, 1, nil}}},
		{Path: "C:/x/e.go", Hunks: []Hunk{{3, 1, 3, 1, []string{"", "--", "++", " -"}}}},
	} {
		d.Diff, d.Adds, d.Dels = UnifiedDiff(d.Path, d.Hunks)
		diffs = append(diffs, d)
	}
	goldVectors["unifiedDiff"] = diffs

	type clipV struct {
		S    string
		N    int
		Clip string
		Byte string
	}
	var clips []clipV
	for _, c := range []struct {
		s string
		n int
	}{
		{"hello world", 20}, {"hello world", 11}, {"hello world", 10}, {"hello world", 5}, {"hello world", 1},
		{"hello world", 0}, {"abcdefghij", 5}, {"ab cdefghij", 6}, {"a bcdefghijk", 8}, {strings.Repeat("é", 10), 4},
		{"word  next", 6}, {"x\u00a0yyyyyy", 6}, {"日本語のテキスト です", 8}, {"a\tb\nc\vd", 4}, {" leading", 3},
		{"tok sk-ant-api03-abcdefghijklmnop rest", 20},
	} {
		clips = append(clips, clipV{c.s, c.n, clip(c.s, c.n), clipBytes(c.s, c.n)})
	}
	goldVectors["clip"] = clips

	// Lines the parser ignores, then LastActivity (TestIgnoredLines' table).
	p := NewParser(mainSrc())
	ignored := []string{
		"", "   ", "not json at all", `{"type":"user","message":`,
		`{"type":"api-request","uuid":"x","timestamp":"2030-01-01T00:00:00Z"}`,
		`{"type":"attachment","uuid":"x","timestamp":"` + ts(1) + `"}`,
		`{"type":"file-history-snapshot","messageId":"x"}`,
		`{"type":"queue-operation","operation":"enqueue"}`,
		`{"type":"system","subtype":"informational","content":"hello","timestamp":"` + ts(2) + `"}`,
		`{"type":"system","subtype":"local_command","content":"x"}`,
		`{"type":"user","uuid":"u","toolUseResult":[1,2],"message":{"content":"weird"}}`,
		`[1,2,3]`, `"string"`, `null`,
		`{"type":"user","uuid":"u2","timestamp":5,"message":{"content":"bad time type"}}`,
		`{"type":"user","uuid":"u3","isMeta":"yes","message":{"content":"bad bool"}}`,
	}
	var outs [][]json.RawMessage
	for _, l := range ignored {
		outs = append(outs, goldItems(p.Line([]byte(l))))
	}
	goldVectors["ignored"] = map[string]any{"lines": ignored, "items": outs, "open": p.Open(), "last": p.LastActivity()}
}

// TestGoldenLineFeeds records the two Go tests that call p.Line directly
// instead of feed(): TestCRLFLine (a line ending in '\r') and
// TestFullSession (every row kind with CRLF line ends, split on '\n' as
// the tailer splits, then Close).
func TestGoldenLineFeeds(t *testing.T) {
	p := NewParser(mainSrc())
	b := string(append(line(t, prompt("u1", "p1", 0, "hi", nil)), '\r'))
	items := p.Line([]byte(b))
	if o := p.Open(); o == nil || o.ID != "p1" {
		t.Fatalf("CRLF line not parsed: %+v", o)
	}
	recordFeed("TestCRLFLine", mainSrc(), []string{b}, items, p)

	var buf bytes.Buffer
	for _, r := range []obj{
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
		{"type": "ai-title", "aiTitle": "README", "sessionId": fxSession},
		{"type": "pr-link", "sessionId": fxSession, "prNumber": 9, "prUrl": "u", "prRepository": "o/r", "timestamp": ts(9)},
		sys("s3", 10, "compact_boundary", obj{"content": "Conversation compacted"}),
		common("user", "c1", 11, obj{"isCompactSummary": true, "message": obj{"content": "summary"}}),
		prompt("u2", "p2", 12, "continue", obj{"origin": obj{"kind": "task-notification"}}),
	} {
		buf.Write(line(t, r))
		buf.WriteString("\r\n")
	}
	p = NewParser(mainSrc())
	var lines []string
	items = nil
	for _, l := range bytes.Split(buf.Bytes(), []byte("\n")) {
		lines = append(lines, string(l))
		items = append(items, p.Line(l)...)
	}
	open, last := p.Open(), p.LastActivity()
	items = append(items, p.Close(at(20), "eof")...)
	if len(items) != 10 {
		t.Fatalf("full session: %d items", len(items))
	}
	goldMu.Lock()
	goldFeeds = append(goldFeeds, goldFeed{Test: "TestFullSession", Src: mainSrc(), Lines: lines,
		Items: goldItems(items), Open: open, Last: last, Close: &goldClose{At: at(20), Reason: "eof"}})
	goldMu.Unlock()
}
