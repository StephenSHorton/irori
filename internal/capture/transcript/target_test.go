package transcript

import (
	"encoding/json"
	"strings"
	"testing"
	"unicode/utf8"
)

func TestTarget(t *testing.T) {
	long := strings.Repeat("é", MaxTarget+20)
	tests := []struct {
		name, tool, input, cwd, want string
	}{
		{"read under cwd backslash", "Read", `{"file_path":"C:\\src\\app\\internal\\x.go"}`, `C:\src\app`, "internal/x.go"},
		{"edit under cwd forward", "Edit", `{"file_path":"C:/src/app/a/b.go"}`, `C:\src\app`, "a/b.go"},
		{"write mixed slashes", "Write", `{"file_path":"C:\\src\\app/a\\b.go"}`, `C:/src/app/`, "a/b.go"},
		{"drive case differs", "Edit", `{"file_path":"c:\\SRC\\app\\x.go"}`, `C:\src\app`, "x.go"},
		{"outside cwd kept", "Read", `{"file_path":"C:\\other\\x.go"}`, `C:\src\app`, `C:\other\x.go`},
		{"sibling dir not under", "Read", `{"file_path":"C:\\src\\app2\\x.go"}`, `C:\src\app`, `C:\src\app2\x.go`},
		{"no cwd", "Read", `{"file_path":"C:\\x.go"}`, "", `C:\x.go`},
		{"unix under", "Read", `{"file_path":"/home/me/app/x.go"}`, "/home/me/app", "x.go"},
		{"unix case-sensitive", "Read", `{"file_path":"/Home/me/app/x.go"}`, "/home/me/app", "/Home/me/app/x.go"},
		{"notebook", "NotebookEdit", `{"notebook_path":"C:\\src\\app\\n.ipynb"}`, `C:\src\app`, "n.ipynb"},
		{"bash first line", "Bash", `{"command":"\n  git status\r\ngit diff"}`, "", "git status"},
		{"grep", "Grep", `{"pattern":"func \\w+","path":"x"}`, "", `func \w+`},
		{"glob", "Glob", `{"pattern":"**/*.go"}`, "", "**/*.go"},
		{"webfetch", "WebFetch", `{"url":"https://example.com/a"}`, "", "https://example.com/a"},
		{"websearch", "WebSearch", `{"query":"go generics"}`, "", "go generics"},
		{"agent", "Agent", `{"description":"Find callers"}`, "", "Find callers"},
		{"task", "Task", `{"description":"Run tests"}`, "", "Run tests"},
		{"other tool", "TodoWrite", `{"todos":[]}`, "", ""},
		{"powershell first line", "PowerShell", `{"command":"ls\nGet-Item x"}`, "", "ls"},
		{"bad input", "Read", `not json`, "", ""},
		{"null input", "Read", `null`, "", ""},
		{"wrong field type", "Bash", `{"command":5}`, "", ""},
		{"clipped", "Grep", `{"pattern":"` + long + `"}`, "", strings.Repeat("é", MaxTarget-1) + "…"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := Target(tt.tool, json.RawMessage(tt.input), tt.cwd)
			if got != tt.want {
				t.Fatalf("Target = %q, want %q", got, tt.want)
			}
			if n := utf8.RuneCountInString(got); n > MaxTarget {
				t.Fatalf("Target has %d runes", n)
			}
		})
	}
}

func TestUnifiedDiff(t *testing.T) {
	tests := []struct {
		name       string
		path       string
		hunks      []Hunk
		want       string
		adds, dels int
	}{
		{"no hunks", "a.go", nil, "", 0, 0},
		{"empty hunks", "a.go", []Hunk{}, "", 0, 0},
		{
			"one hunk", "internal/a.go",
			[]Hunk{{OldStart: 3, OldLines: 3, NewStart: 3, NewLines: 4, Lines: []string{" ctx", "-old", "+new", "+more", " end"}}},
			"--- a/internal/a.go\n+++ b/internal/a.go\n@@ -3,3 +3,4 @@\n ctx\n-old\n+new\n+more\n end\n", 2, 1,
		},
		{
			"two hunks backslash path", `pkg\b.go`,
			[]Hunk{
				{OldStart: 1, OldLines: 1, NewStart: 1, NewLines: 0, Lines: []string{"-gone"}},
				{OldStart: 10, OldLines: 0, NewStart: 9, NewLines: 1, Lines: []string{"+added", `\ No newline at end of file`}},
			},
			"--- a/pkg/b.go\n+++ b/pkg/b.go\n@@ -1,1 +1,0 @@\n-gone\n@@ -10,0 +9,1 @@\n+added\n\\ No newline at end of file\n", 1, 1,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, a, d := UnifiedDiff(tt.path, tt.hunks)
			if got != tt.want || a != tt.adds || d != tt.dels {
				t.Fatalf("UnifiedDiff = %q, %d, %d; want %q, %d, %d", got, a, d, tt.want, tt.adds, tt.dels)
			}
		})
	}
}

func TestClip(t *testing.T) {
	tests := []struct {
		s    string
		n    int
		want string
	}{
		{"abc", 3, "abc"},
		{"abcd", 3, "ab…"},
		{"日本語テキスト", 4, "日本語…"},
		{"", 5, ""},
		{"ab", 0, ""},
		// a word straddling the cap is dropped whole (a token stays unhalved)
		{"ab cdef", 6, "ab…"},
		{"abcd efgh", 5, "abcd…"},
		{"xy\tsecret", 6, "xy…"},
		// no whitespace within the last n/2 runes: hard cut
		{"a bcdefghij", 6, "a bcd…"},
	}
	for _, tt := range tests {
		if got := clip(tt.s, tt.n); got != tt.want {
			t.Errorf("clip(%q, %d) = %q, want %q", tt.s, tt.n, got, tt.want)
		}
	}
	if got := clipBytes("aé", 2); got != "a" {
		t.Errorf("clipBytes split a rune: %q", got)
	}
	if got := clipBytes("abc", 5); got != "abc" {
		t.Errorf("clipBytes short = %q", got)
	}
}
