package transcript

import (
	"bytes"
	"encoding/json"
	"strings"
	"time"
)

// toolResult is the union of the toolUseResult fields the parser reads.
type toolResult struct {
	Type            string          `json:"type"` // Write: create | update
	FilePath        string          `json:"filePath"`
	NotebookPath    string          `json:"notebook_path"`
	OldString       string          `json:"oldString"`
	NewString       string          `json:"newString"`
	ReplaceAll      bool            `json:"replaceAll"`
	Edits           []editStep      `json:"edits"`
	OriginalFile    *string         `json:"originalFile"`
	Content         *string         `json:"content"`
	StructuredPatch []Hunk          `json:"structuredPatch"`
	Staged          bool            `json:"staged"`
	Error           string          `json:"error"`
	Stdout          *string         `json:"stdout"`
	BashEditDiff    *bashEditDiff   `json:"bashEditDiff"`
	GitOperation    json.RawMessage `json:"gitOperation"`

	keys map[string]bool
}

type editStep struct {
	OldString  string `json:"old_string"`
	NewString  string `json:"new_string"`
	ReplaceAll bool   `json:"replace_all"`
	// camelCase spellings, as the single-edit result uses
	OldStringC  string `json:"oldString"`
	NewStringC  string `json:"newString"`
	ReplaceAllC bool   `json:"replaceAll"`
}

func (s editStep) parts() (old, repl string, all bool) {
	if s.OldString != "" || s.NewString != "" {
		return s.OldString, s.NewString, s.ReplaceAll
	}
	return s.OldStringC, s.NewStringC, s.ReplaceAllC
}

type bashEditDiff struct {
	Files []struct {
		FilePath string `json:"filePath"`
		Hunks    []Hunk `json:"hunks"`
	} `json:"files"`
	ChangedFiles []string `json:"changedFiles"`
}

// decodeResult reads an object toolUseResult; ok is false for a string,
// array or anything else that is not an object.
func decodeResult(raw json.RawMessage) (*toolResult, bool) {
	raw = bytes.TrimSpace(raw)
	if len(raw) == 0 || raw[0] != '{' {
		return nil, false
	}
	var keys map[string]json.RawMessage
	if json.Unmarshal(raw, &keys) != nil {
		return nil, false
	}
	var tr toolResult
	tr.keys = map[string]bool{}
	for k := range keys {
		tr.keys[k] = true
	}
	// Decode field by field so one oddly typed field doesn't lose the rest.
	field := func(k string, v any) {
		if b, ok := keys[k]; ok {
			_ = json.Unmarshal(b, v)
		}
	}
	field("type", &tr.Type)
	field("filePath", &tr.FilePath)
	field("notebook_path", &tr.NotebookPath)
	field("oldString", &tr.OldString)
	field("newString", &tr.NewString)
	field("replaceAll", &tr.ReplaceAll)
	field("edits", &tr.Edits)
	field("originalFile", &tr.OriginalFile)
	field("content", &tr.Content)
	field("structuredPatch", &tr.StructuredPatch)
	field("staged", &tr.Staged)
	field("error", &tr.Error)
	field("stdout", &tr.Stdout)
	field("bashEditDiff", &tr.BashEditDiff)
	if g := bytes.TrimSpace(keys["gitOperation"]); len(g) > 0 && g[0] == '{' {
		tr.GitOperation = append(json.RawMessage(nil), g...)
	}
	return &tr, true
}

// inferTool names the tool behind a result whose tool_use was never seen
// (a file read from the middle, after a compaction).
func inferTool(tr *toolResult) string {
	switch {
	case tr == nil:
		return ""
	case tr.keys["oldString"] || tr.keys["edits"] && tr.keys["filePath"]:
		return "Edit"
	case (tr.Type == "create" || tr.Type == "update") && tr.keys["content"] && tr.keys["filePath"]:
		return "Write"
	case tr.keys["notebook_path"] && tr.keys["cell_id"]:
		return "NotebookEdit"
	case tr.keys["stdout"] && tr.keys["stderr"]:
		return "Bash"
	}
	return ""
}

// result handles one tool_result block and, for the row's first block, the
// row's toolUseResult.
func (p *parser) result(r *row, at time.Time, b block, raw json.RawMessage) {
	id, isErr := b.ToolUseID, b.IsError
	c := p.calls[id]
	delete(p.calls, id)
	turn := p.cur
	if c != nil {
		turn = c.turn
		if c.index >= 0 && c.index < len(c.turn.t.Tools) && c.turn == p.cur {
			tc := &c.turn.t.Tools[c.index]
			tc.Done = true
			tc.IsError = isErr
		}
	}
	tr, isObj := decodeResult(raw)
	name := ""
	var input toolInput
	var rawInput json.RawMessage
	if c != nil {
		name, rawInput = c.name, c.input
		input = decodeInput(rawInput)
	} else {
		name = inferTool(tr)
	}
	base := Edit{
		SessionID: p.src.SessionID,
		AgentID:   p.src.AgentID,
		TurnID:    turn.t.ID,
		ToolUseID: id,
		RowUUID:   r.UUID,
		Tool:      name,
		Cwd:       r.Cwd,
		At:        at,
	}
	var edits []Edit
	switch name {
	case "Edit":
		if isObj && !isErr && !tr.Staged {
			edits = editEdit(base, tr, input, r.Cwd)
		}
	case "Write":
		if isObj && !isErr && !tr.Staged {
			edits = writeEdit(base, tr, input, r.Cwd)
		}
	case "NotebookEdit":
		if isObj && !isErr && !tr.Staged && tr.Error == "" {
			edits = notebookEdit(base, tr, input, r.Cwd)
		}
	case "Bash":
		if isObj && tr.BashEditDiff != nil {
			edits = bashEdits(base, tr.BashEditDiff, r.Cwd)
		}
	}
	for i := range edits {
		e := edits[i]
		p.addPath(turn, e.Path)
		p.emit(Item{Edit: &e})
	}
	if name == "Bash" {
		run := &BashRun{
			SessionID: p.src.SessionID,
			AgentID:   p.src.AgentID,
			TurnID:    turn.t.ID,
			ToolUseID: id,
			Cwd:       r.Cwd,
			Command:   input.Command,
			IsError:   isErr,
			At:        at,
		}
		switch {
		case isObj:
			if tr.Stdout != nil {
				run.Stdout = clipBytes(*tr.Stdout, MaxStdout)
			}
			run.GitOperation = tr.GitOperation
		case raw != nil:
			// An error result is a plain string (or, rarely, an array).
			run.Stdout = clipBytes(rawString(raw), MaxStdout)
		default:
			run.Stdout = clipBytes(blockText(b.Content), MaxStdout)
		}
		p.emit(Item{Bash: run})
	}
}

// blockText is a tool_result block's content: a string, or its text blocks joined.
func blockText(raw json.RawMessage) string {
	return joinText(contentBlocks(raw))
}

// addPath appends path to the turn's EditPaths once, while the turn is open.
func (p *parser) addPath(turn *openTurn, path string) {
	if turn == nil || turn != p.cur || path == "" {
		return
	}
	k := pathKey(path)
	if turn.paths[k] {
		return
	}
	turn.paths[k] = true
	turn.t.EditPaths = append(turn.t.EditPaths, path)
}

// pathKey normalizes a path for comparison: forward slashes, and lower
// case for Windows-looking paths.
func pathKey(p string) string {
	k := cleanSlash(p)
	if windowsish(k) {
		k = strings.ToLower(k)
	}
	return k
}

// patchPath is the path written in a patch header: cwd-relative when under
// cwd, else the path itself, with forward slashes.
func patchPath(path, cwd string) string {
	return strings.ReplaceAll(relPath(path, cwd), `\`, "/")
}

func firstNonEmpty(ss ...string) string {
	for _, s := range ss {
		if s != "" {
			return s
		}
	}
	return ""
}

func editEdit(e Edit, tr *toolResult, in toolInput, cwd string) []Edit {
	e.Path = firstNonEmpty(tr.FilePath, in.FilePath)
	if e.Path == "" {
		return nil
	}
	e.Fidelity = Exact
	e.Patch, e.Adds, e.Dels = unifiedDiff(patchPath(e.Path, cwd), tr.StructuredPatch)
	if tr.OriginalFile == nil {
		return []Edit{e}
	}
	orig := *tr.OriginalFile
	e.Before = []byte(orig)
	steps := tr.Edits
	if len(steps) == 0 {
		steps = []editStep{{OldStringC: tr.OldString, NewStringC: tr.NewString, ReplaceAllC: tr.ReplaceAll}}
	}
	cur := orig
	for _, s := range steps {
		old, repl, all := s.parts()
		if old == "" || !strings.Contains(cur, old) {
			return []Edit{e}
		}
		n := 1
		if all {
			n = -1
		}
		cur = strings.Replace(cur, old, repl, n)
	}
	e.After = []byte(cur)
	return []Edit{e}
}

func writeEdit(e Edit, tr *toolResult, in toolInput, cwd string) []Edit {
	e.Path = firstNonEmpty(tr.FilePath, in.FilePath)
	if e.Path == "" {
		return nil
	}
	e.Fidelity = Exact
	e.Create = tr.Type == "create"
	if tr.OriginalFile != nil && !e.Create {
		e.Before = []byte(*tr.OriginalFile)
	}
	hunks := tr.StructuredPatch
	if tr.Content != nil {
		e.After = []byte(*tr.Content)
		if len(hunks) == 0 && e.Create {
			hunks = createHunk(*tr.Content)
		}
	}
	e.Patch, e.Adds, e.Dels = unifiedDiff(patchPath(e.Path, cwd), hunks)
	return []Edit{e}
}

// createHunk is the all-added hunk for a new file's content.
func createHunk(content string) []Hunk {
	if content == "" {
		return nil
	}
	lines := strings.Split(strings.TrimSuffix(content, "\n"), "\n")
	h := Hunk{OldStart: 0, OldLines: 0, NewStart: 1, NewLines: len(lines)}
	for _, l := range lines {
		h.Lines = append(h.Lines, "+"+strings.TrimSuffix(l, "\r"))
	}
	return []Hunk{h}
}

func notebookEdit(e Edit, tr *toolResult, in toolInput, cwd string) []Edit {
	e.Path = firstNonEmpty(tr.NotebookPath, tr.FilePath, in.NotebookPath)
	if e.Path == "" {
		return nil
	}
	e.Fidelity = Exact
	e.Patch, e.Adds, e.Dels = unifiedDiff(patchPath(e.Path, cwd), tr.StructuredPatch)
	return []Edit{e}
}

func bashEdits(base Edit, d *bashEditDiff, cwd string) []Edit {
	var out []Edit
	seen := map[string]bool{}
	for _, path := range d.ChangedFiles {
		k := pathKey(path)
		if path == "" || seen[k] {
			continue
		}
		seen[k] = true
		e := base
		e.Path = path
		e.Fidelity = Heuristic
		for _, f := range d.Files {
			if pathKey(f.FilePath) == k {
				e.Patch, e.Adds, e.Dels = unifiedDiff(patchPath(path, cwd), f.Hunks)
				break
			}
		}
		out = append(out, e)
	}
	return out
}
