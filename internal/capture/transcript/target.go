package transcript

import (
	"encoding/json"
	"strconv"
	"strings"
	"unicode/utf8"
)

// toolInput holds the input fields Target and the edit builders read.
type toolInput struct {
	FilePath     string `json:"file_path"`
	NotebookPath string `json:"notebook_path"`
	Command      string `json:"command"`
	Pattern      string `json:"pattern"`
	URL          string `json:"url"`
	Query        string `json:"query"`
	Description  string `json:"description"`
}

func decodeInput(input json.RawMessage) toolInput {
	var in toolInput
	if len(input) > 0 {
		_ = json.Unmarshal(input, &in) // a partial or odd input still labels what it can
	}
	return in
}

func target(name string, input json.RawMessage, cwd string) string {
	in := decodeInput(input)
	var s string
	switch name {
	case "Read", "Edit", "Write":
		s = relPath(in.FilePath, cwd)
	case "NotebookEdit":
		s = relPath(in.NotebookPath, cwd)
	case "Bash":
		s = firstLine(in.Command)
	case "Grep", "Glob":
		s = in.Pattern
	case "WebFetch":
		s = in.URL
	case "WebSearch":
		s = in.Query
	case "Agent", "Task":
		s = in.Description
	}
	return clip(s, MaxTarget)
}

// relPath is p relative to cwd with forward slashes when p is under cwd,
// else p unchanged.
func relPath(p, cwd string) string {
	if p == "" || cwd == "" {
		return p
	}
	fp := strings.ReplaceAll(p, `\`, "/")
	fc := strings.TrimRight(strings.ReplaceAll(cwd, `\`, "/"), "/")
	if fc == "" || len(fp) <= len(fc)+1 || fp[len(fc)] != '/' {
		return p
	}
	if !samePrefix(fp[:len(fc)], fc) {
		return p
	}
	return strings.TrimLeft(fp[len(fc):], "/")
}

// firstLine is the first non-blank line of s, trimmed.
func firstLine(s string) string {
	s = strings.TrimSpace(s)
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		s = s[:i]
	}
	return strings.TrimSpace(s)
}

// clip keeps at most n runes of s, ending in "…" when it cut.
func clip(s string, n int) string {
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	if n <= 0 {
		return ""
	}
	r := []rune(s)
	return string(r[:n-1]) + "…"
}

// clipBytes keeps at most n bytes of s without splitting a rune.
func clipBytes(s string, n int) string {
	if len(s) <= n {
		return s
	}
	i := n
	for i > 0 && !utf8.RuneStart(s[i]) {
		i--
	}
	return s[:i]
}

func unifiedDiff(p string, hunks []Hunk) (string, int, int) {
	if len(hunks) == 0 {
		return "", 0, 0
	}
	p = strings.ReplaceAll(p, `\`, "/")
	var b strings.Builder
	adds, dels := 0, 0
	b.WriteString("--- a/" + p + "\n+++ b/" + p + "\n")
	for _, h := range hunks {
		b.WriteString("@@ -" + strconv.Itoa(h.OldStart) + "," + strconv.Itoa(h.OldLines) +
			" +" + strconv.Itoa(h.NewStart) + "," + strconv.Itoa(h.NewLines) + " @@\n")
		for _, l := range h.Lines {
			switch {
			case strings.HasPrefix(l, "+"):
				adds++
			case strings.HasPrefix(l, "-"):
				dels++
			}
			b.WriteString(l)
			b.WriteByte('\n')
		}
	}
	return b.String(), adds, dels
}
