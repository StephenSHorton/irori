package mcpserver

import (
	"fmt"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// MaxResultBytes caps one tool result (about 2k tokens); the Core caps too.
const MaxResultBytes = 8 << 10

// oneLine folds control characters and runs of whitespace into single spaces
// (so untrusted text can't fake extra lines) and clips to max runes with "…".
func oneLine(s string, max int) string {
	out := strings.Join(strings.FieldsFunc(s, func(r rune) bool {
		return unicode.IsSpace(r) || unicode.IsControl(r)
	}), " ")
	if utf8.RuneCountInString(out) <= max {
		return out
	}
	return strings.TrimRight(string([]rune(out)[:max-1]), " ") + "…"
}

// Clip caps s at max bytes, cutting at a line boundary and adding a note.
func Clip(s string, max int) string {
	if len(s) <= max {
		return s
	}
	note := fmt.Sprintf("\n[truncated: showing %%d of %d bytes; narrow the query]", len(s))
	keep := max - len(note) - 8 // room for the shown-bytes digits
	if keep < 0 {
		keep = 0
	}
	cut := strings.LastIndexByte(s[:keep], '\n')
	if cut <= 0 {
		// One huge line: back up to a rune boundary instead.
		cut = keep
		for cut > 0 && !utf8.RuneStart(s[cut]) {
			cut--
		}
	}
	return s[:cut] + fmt.Sprintf(note, cut)
}

// text is a plain-text tool result. Core tool failures are text starting
// "Error: "; they're flagged IsError so clients show them as failures, but
// stay text the model reads.
func text(s string) *mcp.CallToolResult {
	return &mcp.CallToolResult{
		Content: []mcp.Content{&mcp.TextContent{Text: Clip(s, MaxResultBytes)}},
		IsError: strings.HasPrefix(s, "Error: "),
	}
}
