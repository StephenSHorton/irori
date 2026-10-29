package presence

import (
	"bufio"
	"bytes"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// defaultTailBytes is how much of a transcript's end we read. Transcripts
// reach hundreds of MB; the roster only needs the last few turns.
const defaultTailBytes = 256 << 10

// defaultModeScanBytes bounds the one-time backward look for a permission
// mode on a transcript whose tail window holds no mode row. Real
// transcripts have been seen with their newest mode row ~600KB from the end.
const defaultModeScanBytes = 8 << 20

// maxLastText is the longest LastText, in runes.
const maxLastText = 240

// Slug is Claude Code's project folder name for cwd: every rune that is not
// [A-Za-z0-9] becomes '-'.
func Slug(cwd string) string {
	var b strings.Builder
	for _, r := range cwd {
		if r < 128 && (r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9') {
			b.WriteRune(r)
		} else {
			b.WriteByte('-')
		}
	}
	return b.String()
}

// safeSessionID keeps glob patterns free of metacharacters.
func safeSessionID(id string) bool {
	if id == "" {
		return false
	}
	for _, r := range id {
		if !(r >= '0' && r <= '9' || r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r == '-' || r == '_') {
			return false
		}
	}
	return true
}

// findTranscript is <claudeDir>/projects/<slug>/<id>.jsonl, else the first
// projects/*/<id>.jsonl, else "".
func findTranscript(claudeDir, cwd, id string) string {
	if !safeSessionID(id) {
		return ""
	}
	projects := filepath.Join(claudeDir, "projects")
	if cwd != "" {
		p := filepath.Join(projects, Slug(cwd), id+".jsonl")
		if fi, err := os.Stat(p); err == nil && fi.Mode().IsRegular() {
			return absPath(p)
		}
	}
	matches, _ := filepath.Glob(filepath.Join(projects, "*", id+".jsonl"))
	if len(matches) > 0 {
		return absPath(matches[0])
	}
	return ""
}

func absPath(p string) string {
	if abs, err := filepath.Abs(p); err == nil {
		return abs
	}
	return p
}

// tail is what we learned from the end of one transcript.
type tail struct {
	Title          string // Custom, else AI
	Custom         string // newest custom-title
	AI             string // newest ai-title
	LastText       string
	LastActivity   time.Time
	PermissionMode string
}

// transcriptRow is the tolerant subset of a transcript line we read.
type transcriptRow struct {
	Type           string          `json:"type"`
	Timestamp      string          `json:"timestamp"`
	IsSidechain    bool            `json:"isSidechain"`
	AITitle        string          `json:"aiTitle"`
	CustomTitle    string          `json:"customTitle"`
	PermissionMode string          `json:"permissionMode"` // on "permission-mode" rows and some user rows
	Message        json.RawMessage `json:"message"`
}

type contentBlock struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// readTail parses the last n bytes of f (size bytes long). When the window
// starts mid-file the first, partial line is skipped. Reading starts one byte
// early so a window that opens exactly on a row keeps that row.
func readTail(f io.ReadSeeker, size, n int64) tail {
	off := max(size-n-1, 0)
	if _, err := f.Seek(off, io.SeekStart); err != nil {
		return tail{}
	}
	sc := bufio.NewScanner(io.LimitReader(f, size-off))
	sc.Buffer(make([]byte, 0, 64<<10), int(n)+1)
	first := off > 0
	var t tail
	var custom, ai string
	for sc.Scan() {
		line := sc.Bytes()
		if first {
			first = false
			continue
		}
		line = bytes.TrimSpace(line)
		if len(line) == 0 || line[0] != '{' {
			continue
		}
		var row transcriptRow
		if json.Unmarshal(line, &row) != nil {
			continue
		}
		if ts, err := time.Parse(time.RFC3339Nano, row.Timestamp); err == nil && ts.After(t.LastActivity) {
			t.LastActivity = ts
		}
		if isModeRow(row) {
			t.PermissionMode = row.PermissionMode
		}
		switch row.Type {
		case "custom-title":
			if row.CustomTitle != "" {
				custom = row.CustomTitle
			}
		case "ai-title":
			if row.AITitle != "" {
				ai = row.AITitle
			}
		case "assistant":
			// Subagent prose isn't what the session itself last said.
			if row.IsSidechain {
				continue
			}
			if text := lastTextBlock(row.Message); text != "" {
				t.LastText = text
			}
		}
	}
	t.Custom, t.AI = custom, ai
	t.Title = custom
	if t.Title == "" {
		t.Title = ai
	}
	return t
}

// isModeRow reports whether row sets the session's permission mode: a
// "permission-mode" row, or a non-sidechain "user" row with a top-level
// permissionMode.
func isModeRow(row transcriptRow) bool {
	return row.PermissionMode != "" && !row.IsSidechain &&
		(row.Type == "permission-mode" || row.Type == "user")
}

// modeKey pre-filters lines before JSON parsing in scanMode.
var modeKey = []byte(`"permissionMode"`)

// scanMode is the permission mode of the newest mode row in the last n bytes
// of f (size bytes long), "" if none. Like readTail it reads one byte early
// and skips the first, partial line when the window starts mid-file.
func scanMode(f io.ReadSeeker, size, n int64) string {
	off := max(size-n-1, 0)
	if _, err := f.Seek(off, io.SeekStart); err != nil {
		return ""
	}
	buf := make([]byte, size-off)
	if _, err := io.ReadFull(f, buf); err != nil {
		return ""
	}
	if off > 0 {
		i := bytes.IndexByte(buf, '\n')
		if i < 0 {
			return ""
		}
		buf = buf[i+1:]
	}
	for len(buf) > 0 {
		var line []byte
		if j := bytes.LastIndexByte(buf, '\n'); j >= 0 {
			line, buf = buf[j+1:], buf[:j]
		} else {
			line, buf = buf, nil
		}
		line = bytes.TrimSpace(line)
		if len(line) == 0 || line[0] != '{' || !bytes.Contains(line, modeKey) {
			continue
		}
		var row transcriptRow
		if json.Unmarshal(line, &row) == nil && isModeRow(row) {
			return row.PermissionMode
		}
	}
	return ""
}

// lastTextBlock is the last non-empty text block of an assistant message,
// collapsed to one line and capped at maxLastText runes.
func lastTextBlock(raw json.RawMessage) string {
	var msg struct {
		Content []contentBlock `json:"content"`
	}
	if len(raw) == 0 || json.Unmarshal(raw, &msg) != nil {
		return ""
	}
	for i := len(msg.Content) - 1; i >= 0; i-- {
		if b := msg.Content[i]; b.Type == "text" {
			if s := oneLine(b.Text, maxLastText); s != "" {
				return s
			}
		}
	}
	return ""
}

// oneLine collapses runs of whitespace to single spaces and caps s at limit
// runes, marking a cut with an ellipsis.
func oneLine(s string, limit int) string {
	s = strings.Join(strings.Fields(s), " ")
	r := []rune(s)
	if len(r) <= limit {
		return s
	}
	return strings.TrimRight(string(r[:limit-1]), " ") + "…"
}
