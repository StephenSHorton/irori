// Package transcript turns Claude Code transcript lines into capture items
// (docs/design/v0.3-capture.md §3 and §4). It is pure: the capture service
// owns files, offsets, repos, blobs, scrubbing and the log. One Parser per
// transcript file, fed complete lines in file order.
//
// Record types read (everything else is ignored, including unknown types and
// lines that are not valid JSON): user, assistant, system (subtypes
// turn_duration, away_summary, compact_boundary), ai-title, custom-title,
// pr-link. Lines whose first bytes are `{"type":"api-request` are skipped
// before decoding (SkipLine).
//
// Format notes from real transcripts (Claude Code 2.1.29x-2.1.31x):
//   - user rows: message.content is a string or blocks; a tool result row
//     has a top-level toolUseResult (object, string or array) and a
//     tool_result block with tool_use_id and is_error. origin {kind, ...}
//     is missing on older versions; promptSource "sdk" may be set.
//     isMeta true rows are injected text, not prompts. isCompactSummary
//     true rows carry the compaction summary as their text.
//   - assistant rows: message.content blocks text / thinking / tool_use
//     {id, name, input}; one row per block, sharing message.id.
//   - every conversation row has uuid, timestamp (RFC 3339), cwd, gitBranch,
//     sessionId, isSidechain, and agentId on subagent rows.
//   - Edit toolUseResult: filePath, oldString, newString, originalFile
//     (string|null), structuredPatch [{oldStart, oldLines, newStart,
//     newLines, lines[]}], replaceAll, userModified, staged?.
//   - Write toolUseResult: type create|update, filePath, content,
//     structuredPatch, originalFile (string|null), staged?.
//   - PowerShell (Windows) has Bash's input {command, description,
//     timeout?} and result {stdout, stderr, interrupted, isImage}; it gets a
//     Target and a BashRun like Bash. A result whose tool_use was never
//     seen is inferred as Bash.
//   - Bash toolUseResult: stdout, stderr, interrupted, bashEditDiff?
//     {files[], moreFiles, changedFiles[]}, gitOperation? ({pr:{number,url,
//     action}}, {push:{branch}}, {branch:{ref,action}}).
//   - pr-link: {sessionId, prNumber (string), prUrl, prRepository, timestamp}.
//   - system turn_duration closes a turn; away_summary has content.
//
// CONTRACT (implemented by the transcript work package; keep these signatures):
package transcript

import (
	"encoding/json"
	"time"
)

// Caps (runes for text).
const (
	MaxPrompt     = 2000
	MaxReply      = 1000
	MaxTarget     = 160
	MaxToolsShown = 200
	MaxStdout     = 4096 // bytes of Bash stdout kept on a BashRun
	MaxSummary    = 2000 // inline summary text; longer text is in Summary.Full
)

// Source identifies the file: the session id from the path
// (projects/<slug>/<sessionId>.jsonl) and, for subagent files
// (projects/<slug>/<sessionId>/subagents/**/agent-<agentId>.jsonl), the agent id.
type Source struct {
	SessionID string
	AgentID   string // "" for the main conversation
}

// SourceFor derives the Source from a path under a projects dir; ok is false
// for paths that are neither shape. Session ids must match [A-Za-z0-9_-]+.
func SourceFor(projectsDir, path string) (Source, bool) { return sourceFor(projectsDir, path) }

// SkipLine reports lines decoded never need: api-request* rows (by byte
// prefix, allowing leading spaces) and blank lines.
func SkipLine(line []byte) bool { return skipLine(line) }

// ToolCall is one tool_use and, once it arrived, its result.
type ToolCall struct {
	ID      string    `json:"id"`
	Name    string    `json:"name"`
	Target  string    `json:"target,omitempty"` // short label, see Target
	IsError bool      `json:"isError,omitempty"`
	Done    bool      `json:"done,omitempty"` // its result row was seen
	At      time.Time `json:"at"`
}

// Turn is one prompt and everything until the next.
type Turn struct {
	ID          string     `json:"id"` // promptId, else the prompt row's uuid
	SessionID   string     `json:"sessionId"`
	AgentID     string     `json:"agentId,omitempty"`
	Origin      string     `json:"origin"`               // human, plugin, peer, task-notification, sdk, command, ...
	OriginName  string     `json:"originName,omitempty"` // plugin name when origin is plugin
	Prompt      string     `json:"prompt"`               // excerpt, MaxPrompt runes
	Reply       string     `json:"reply,omitempty"`      // last assistant text, MaxReply runes
	Cwd         string     `json:"cwd,omitempty"`
	Branch      string     `json:"branch,omitempty"`
	StartedAt   time.Time  `json:"startedAt"`
	EndedAt     time.Time  `json:"endedAt,omitempty"` // zero while open
	Interrupted bool       `json:"interrupted,omitempty"`
	Tools       []ToolCall `json:"tools,omitempty"` // first MaxToolsShown
	ToolsMore   int        `json:"toolsMore,omitempty"`
	EditPaths   []string   `json:"editPaths,omitempty"` // absolute, in first-edit order, deduped
	EndReason   string     `json:"endReason,omitempty"` // "turn_duration", "next", "quiet", "eof"
}

// Fidelity of an edit.
const (
	Exact     = "exact"
	Heuristic = "heuristic"
)

// Edit is one file change by one tool call.
type Edit struct {
	SessionID string `json:"sessionId"`
	AgentID   string `json:"agentId,omitempty"`
	TurnID    string `json:"turnId"`
	ToolUseID string `json:"toolUseId"`
	RowUUID   string `json:"rowUuid"`
	Tool      string `json:"tool"` // Edit, Write, NotebookEdit, Bash
	Path      string `json:"path"` // absolute, as the tool reported it
	Cwd       string `json:"cwd,omitempty"`
	Create    bool   `json:"create,omitempty"`
	// Before and After are the whole file's content when known (nil when
	// not). The service hashes them (gitsha.Pair) and drops them; they are
	// never logged.
	Before   []byte    `json:"-"`
	After    []byte    `json:"-"`
	Patch    string    `json:"-"` // unified diff (§4), unscrubbed; "" if none
	Adds     int       `json:"adds"`
	Dels     int       `json:"dels"`
	Fidelity string    `json:"fidelity"`
	At       time.Time `json:"at"`
}

// Summary is a summary-like row.
type Summary struct {
	SessionID string    `json:"sessionId"`
	AgentID   string    `json:"agentId,omitempty"`
	Kind      string    `json:"kind"` // away_summary, compact_boundary, compact_summary, ai_title, custom_title
	Text      string    `json:"text"` // MaxSummary runes
	Full      string    `json:"-"`    // the whole text when longer than Text, else ""
	RowUUID   string    `json:"rowUuid,omitempty"`
	At        time.Time `json:"at"` // zero for title rows (they carry no timestamp)
}

// PRLink is a pr-link row.
type PRLink struct {
	SessionID string    `json:"sessionId"`
	Repo      string    `json:"repo"` // prRepository
	Number    int       `json:"number"`
	URL       string    `json:"url"`
	At        time.Time `json:"at"`
}

// BashRun is a finished Bash or PowerShell call, for commit and PR
// detection.
type BashRun struct {
	Tool         string          `json:"tool"` // "Bash" or "PowerShell"
	SessionID    string          `json:"sessionId"`
	AgentID      string          `json:"agentId,omitempty"`
	TurnID       string          `json:"turnId"`
	ToolUseID    string          `json:"toolUseId"`
	Cwd          string          `json:"cwd"`
	Command      string          `json:"command"`
	Stdout       string          `json:"stdout"` // first MaxStdout bytes
	GitOperation json.RawMessage `json:"gitOperation,omitempty"`
	IsError      bool            `json:"isError,omitempty"`
	At           time.Time       `json:"at"` // the result row's timestamp
}

// Item is one parser output; exactly one field is set.
type Item struct {
	Turn    *Turn
	Edit    *Edit
	Summary *Summary
	PRLink  *PRLink
	Bash    *BashRun
}

// Parser is not safe for concurrent use.
type Parser struct {
	impl *parser
}

// NewParser starts a parser for one file.
func NewParser(src Source) *Parser { return newParser(src) }

// Line feeds one complete line (no trailing newline) and returns what it
// finished, in order. A closed Turn is returned once, after the edits and
// bash runs of that turn. Edits and BashRuns are returned when their result
// row arrives (they need it); a tool_use whose result never comes stays in
// the turn's Tools with Done false.
//
// Rules: a prompt row starts a turn and closes the open one (EndReason
// "next"); [Request interrupted by user] (and "... for tool use]") marks the
// open turn Interrupted and starts nothing; turn_duration closes the open
// turn ("turn_duration"); rows of another agentId than the Source's are
// ignored (a main file never holds subagent rows, but be safe); tool
// results before any prompt start an implicit turn with Origin "unknown"
// and the result row's uuid as id.
func (p *Parser) Line(line []byte) []Item { return p.impl.line(line) }

// Close ends the open turn, if any, with EndedAt = at and the given reason
// ("quiet" or "eof"), and returns it.
func (p *Parser) Close(at time.Time, reason string) []Item { return p.impl.close(at, reason) }

// Open returns a copy of the open turn, or nil.
func (p *Parser) Open() *Turn { return p.impl.open() }

// LastActivity is the newest row timestamp seen.
func (p *Parser) LastActivity() time.Time { return p.impl.lastActivity() }

// Target labels a tool call from its name, input and the row's cwd: the path
// for Read/Edit/Write/NotebookEdit (cwd-relative with forward slashes when
// under cwd), the first line of a Bash or PowerShell command, the pattern for Grep/Glob,
// the url for WebFetch, the query for WebSearch, the description for
// Agent/Task, else "". Clipped to MaxTarget runes with "…".
func Target(name string, input json.RawMessage, cwd string) string { return target(name, input, cwd) }

// UnifiedDiff renders a structuredPatch for path as a unified diff
// ("--- a/<p>\n+++ b/<p>\n@@ -s,l +s,l @@\n" + lines). It also returns the
// added and removed line counts.
func UnifiedDiff(path string, hunks []Hunk) (diff string, adds, dels int) {
	return unifiedDiff(path, hunks)
}

// Hunk is one structuredPatch entry.
type Hunk struct {
	OldStart int      `json:"oldStart"`
	OldLines int      `json:"oldLines"`
	NewStart int      `json:"newStart"`
	NewLines int      `json:"newLines"`
	Lines    []string `json:"lines"`
}
