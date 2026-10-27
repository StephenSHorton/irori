package transcript

import "testing"

func TestSourceFor(t *testing.T) {
	const win = `C:\Users\me\.claude\projects`
	const nix = "/home/me/.claude/projects"
	tests := []struct {
		name   string
		dir, p string
		want   Source
		wantOK bool
	}{
		{"main backslash", win, win + `\C--src-app\abc-123.jsonl`, Source{SessionID: "abc-123"}, true},
		{"main forward slash", win, "C:/Users/me/.claude/projects/C--src-app/abc_123.jsonl", Source{SessionID: "abc_123"}, true},
		{"main drive case differs", win, `c:\users\ME\.claude\projects\slug\s1.jsonl`, Source{SessionID: "s1"}, true},
		{"dir trailing slash", win + `\`, win + `\slug\s1.jsonl`, Source{SessionID: "s1"}, true},
		{"unix main", nix, nix + "/-home-me-app/s1.jsonl", Source{SessionID: "s1"}, true},
		{"unix case matters", nix, "/HOME/me/.claude/projects/slug/s1.jsonl", Source{}, false},
		{"subagent", win, win + `\slug\s1\subagents\agent-a1b2.jsonl`, Source{SessionID: "s1", AgentID: "a1b2"}, true},
		{"workflow agent", win, win + `\slug\s1\subagents\workflows\wf_x\agent-y.jsonl`, Source{SessionID: "s1", AgentID: "y"}, true},
		{"workflow agent forward", nix, nix + "/slug/s1/subagents/workflows/wf_x/agent-y.jsonl", Source{SessionID: "s1", AgentID: "y"}, true},
		{"workflow journal", win, win + `\slug\s1\subagents\workflows\wf_x\journal.jsonl`, Source{}, false},
		{"agent no id", win, win + `\slug\s1\subagents\agent-.jsonl`, Source{}, false},
		{"agent bad id", win, win + `\slug\s1\subagents\agent-a.b.jsonl`, Source{}, false},
		{"agent not under subagents", win, win + `\slug\s1\other\agent-a.jsonl`, Source{}, false},
		{"agent meta json", win, win + `\slug\s1\subagents\agent-a.meta.json`, Source{}, false},
		{"bad session id", win, win + `\slug\s 1.jsonl`, Source{}, false},
		{"bad session id dot", win, win + `\slug\s.1.jsonl`, Source{}, false},
		{"bad sid in subagent", win, win + `\slug\s!1\subagents\agent-a.jsonl`, Source{}, false},
		{"not jsonl", win, win + `\slug\s1.json`, Source{}, false},
		{"key file", win, win + `\slug\s1.key`, Source{}, false},
		{"too shallow", win, win + `\s1.jsonl`, Source{}, false},
		{"too deep main", win, win + `\slug\x\s1.jsonl`, Source{}, false},
		{"outside projects", win, `C:\Users\me\.claude\s1.jsonl`, Source{}, false},
		{"sibling prefix", win, `C:\Users\me\.claude\projects2\slug\s1.jsonl`, Source{}, false},
		{"dot dot escapes", win, win + `\slug\..\..\x\s1.jsonl`, Source{}, false},
		{"the dir itself", win, win, Source{}, false},
		{"empty dir", "", `\slug\s1.jsonl`, Source{}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, ok := SourceFor(tt.dir, tt.p)
			if ok != tt.wantOK || got != tt.want {
				t.Fatalf("SourceFor(%q, %q) = %+v, %v; want %+v, %v", tt.dir, tt.p, got, ok, tt.want, tt.wantOK)
			}
		})
	}
}

func TestSkipLine(t *testing.T) {
	tests := []struct {
		line string
		want bool
	}{
		{"", true},
		{"   ", true},
		{"\t\r", true},
		{`{"type":"api-request","x":1}`, true},
		{`{"type":"api-request-start"}`, true},
		{`   {"type":"api-request"}`, true},
		{`{"type":"user"}`, false},
		{`{ "type":"api-request"}`, false}, // not the byte prefix; decoding ignores it anyway
		{`{"type":"assistant","note":"{\"type\":\"api-request"}`, false},
		{"not json", false},
	}
	for _, tt := range tests {
		if got := SkipLine([]byte(tt.line)); got != tt.want {
			t.Errorf("SkipLine(%q) = %v, want %v", tt.line, got, tt.want)
		}
	}
}
