package transcript

import (
	"bytes"
	"encoding/json"
	"strings"
	"time"
)

// row is the subset of a transcript record the parser reads.
type row struct {
	Type             string          `json:"type"`
	Subtype          string          `json:"subtype"`
	UUID             string          `json:"uuid"`
	PromptID         string          `json:"promptId"`
	Timestamp        string          `json:"timestamp"`
	Cwd              string          `json:"cwd"`
	GitBranch        string          `json:"gitBranch"`
	AgentID          string          `json:"agentId"`
	IsMeta           bool            `json:"isMeta"`
	IsCompactSummary bool            `json:"isCompactSummary"`
	PromptSource     string          `json:"promptSource"`
	Origin           json.RawMessage `json:"origin"`
	Message          struct {
		Content json.RawMessage `json:"content"`
	} `json:"message"`
	ToolUseResult json.RawMessage `json:"toolUseResult"`
	Content       json.RawMessage `json:"content"` // system rows
	AiTitle       string          `json:"aiTitle"`
	CustomTitle   string          `json:"customTitle"`
	PrNumber      json.RawMessage `json:"prNumber"`
	PrURL         string          `json:"prUrl"`
	PrRepository  string          `json:"prRepository"`
}

// block is one message.content entry.
type block struct {
	Type      string          `json:"type"`
	Text      string          `json:"text"`
	ID        string          `json:"id"`
	Name      string          `json:"name"`
	Input     json.RawMessage `json:"input"`
	ToolUseID string          `json:"tool_use_id"`
	IsError   bool            `json:"is_error"`
	Content   json.RawMessage `json:"content"`
}

// call is a tool_use waiting for (or past) its result.
type call struct {
	name  string
	input json.RawMessage
	turn  *openTurn
	index int       // in turn.t.Tools, -1 when past MaxToolsShown
	at    time.Time // the tool_use row's timestamp
}

// openTurn is the turn being built and its bookkeeping.
type openTurn struct {
	t     *Turn
	last  time.Time
	paths map[string]bool
}

type parser struct {
	src     Source
	cur     *openTurn
	calls   map[string]*call
	newest  time.Time
	pending []Item
	seen    map[string]bool // ids of turns started
}

func newParser(src Source) *Parser {
	return &Parser{impl: &parser{src: src, calls: map[string]*call{}, seen: map[string]bool{}}}
}

func (p *parser) emit(it Item) { p.pending = append(p.pending, it) }

func (p *parser) flush() []Item {
	out := p.pending
	p.pending = nil
	return out
}

func (p *parser) line(line []byte) []Item {
	line = bytes.TrimRight(line, "\r\n")
	if skipLine(line) {
		return nil
	}
	var r row
	if err := json.Unmarshal(line, &r); err != nil {
		return nil
	}
	if r.AgentID != p.src.AgentID {
		return nil
	}
	at := parseTime(r.Timestamp)
	if at.After(p.newest) {
		p.newest = at
	}
	switch r.Type {
	case "user":
		p.user(&r, at)
	case "assistant":
		p.assistant(&r, at)
	case "system":
		p.system(&r, at)
	case "ai-title":
		p.summary("ai_title", r.AiTitle, "", time.Time{})
	case "custom-title":
		p.summary("custom_title", r.CustomTitle, "", time.Time{})
	case "pr-link":
		p.prLink(&r, at)
	}
	return p.flush()
}

func (p *parser) close(at time.Time, reason string) []Item {
	p.end(at, reason)
	return p.flush()
}

func (p *parser) open() *Turn {
	if p.cur == nil {
		return nil
	}
	return copyTurn(p.cur.t)
}

func (p *parser) lastActivity() time.Time { return p.newest }

func copyTurn(t *Turn) *Turn {
	c := *t
	c.Tools = append([]ToolCall(nil), t.Tools...)
	c.EditPaths = append([]string(nil), t.EditPaths...)
	return &c
}

func parseTime(s string) time.Time {
	if s == "" {
		return time.Time{}
	}
	t, err := time.Parse(time.RFC3339Nano, s)
	if err != nil {
		return time.Time{}
	}
	return t
}

// end closes the open turn, if any.
func (p *parser) end(at time.Time, reason string) {
	if p.cur == nil {
		return
	}
	t := p.cur.t
	t.EndedAt = at
	t.EndReason = reason
	p.cur = nil
	p.emit(Item{Turn: t})
}

// touch records activity on the open turn and fills its cwd and branch.
func (p *parser) touch(r *row, at time.Time) {
	if p.cur == nil {
		return
	}
	if at.After(p.cur.last) {
		p.cur.last = at
	}
	if p.cur.t.Cwd == "" {
		p.cur.t.Cwd = r.Cwd
	}
	if p.cur.t.Branch == "" {
		p.cur.t.Branch = r.GitBranch
	}
}

func (p *parser) start(id, origin, originName, prompt string, r *row, at time.Time) {
	p.seen[id] = true
	p.cur = &openTurn{
		t: &Turn{
			ID:         id,
			SessionID:  p.src.SessionID,
			AgentID:    p.src.AgentID,
			Origin:     origin,
			OriginName: originName,
			Prompt:     clip(prompt, MaxPrompt),
			Cwd:        r.Cwd,
			Branch:     r.GitBranch,
			StartedAt:  at,
		},
		last:  at,
		paths: map[string]bool{},
	}
}

// implicit starts a turn with origin unknown when none is open: for a
// tool result or tool_use with no prompt before it.
func (p *parser) implicit(r *row, at time.Time) {
	if p.cur == nil {
		p.start(r.UUID, "unknown", "", "", r, at)
	}
}

// contentBlocks decodes message.content: a string becomes one text block.
func contentBlocks(raw json.RawMessage) []block {
	raw = bytes.TrimSpace(raw)
	if len(raw) == 0 || raw[0] == 'n' {
		return nil
	}
	if raw[0] == '"' {
		var s string
		if json.Unmarshal(raw, &s) != nil {
			return nil
		}
		return []block{{Type: "text", Text: s}}
	}
	var bs []block
	if json.Unmarshal(raw, &bs) != nil {
		// One odd block should not hide the others.
		var anys []json.RawMessage
		if json.Unmarshal(raw, &anys) != nil {
			return nil
		}
		for _, a := range anys {
			var b block
			if json.Unmarshal(a, &b) == nil {
				bs = append(bs, b)
			}
		}
	}
	return bs
}

func joinText(bs []block) string {
	var parts []string
	for _, b := range bs {
		if b.Type == "text" {
			parts = append(parts, b.Text)
		}
	}
	return strings.Join(parts, "\n")
}

// isPeer reports a message from another session, in the shape older
// versions wrote without an origin.
func isPeer(s string) bool {
	s = strings.TrimSpace(s)
	return strings.HasPrefix(s, "Another Claude session sent a message") || strings.HasPrefix(s, "<teammate-message")
}

// isLocalOutput reports the output row of a local slash command.
func isLocalOutput(s string) bool {
	s = strings.TrimSpace(s)
	return strings.HasPrefix(s, "<local-command-stdout>") || strings.HasPrefix(s, "<local-command-stderr>")
}

func isInterrupt(s string) bool {
	s = strings.TrimSpace(s)
	return s == "[Request interrupted by user]" || s == "[Request interrupted by user for tool use]"
}

// origin derives a prompt row's origin (§3).
func origin(r *row, text string) (kind, name string) {
	raw := bytes.TrimSpace(r.Origin)
	if len(raw) > 0 && raw[0] == '{' {
		var o struct {
			Kind string `json:"kind"`
			Name string `json:"name"`
		}
		if json.Unmarshal(raw, &o) == nil && o.Kind != "" {
			if o.Kind == "plugin" {
				return o.Kind, o.Name
			}
			return o.Kind, ""
		}
	} else if len(raw) > 0 && raw[0] == '"' {
		var s string
		if json.Unmarshal(raw, &s) == nil && s != "" {
			return s, ""
		}
	}
	switch {
	case r.PromptSource == "sdk":
		return "sdk", ""
	case strings.HasPrefix(strings.TrimSpace(text), "<command-name>"):
		return "command", ""
	case isPeer(text):
		return "peer", ""
	}
	return "human", ""
}

func (p *parser) user(r *row, at time.Time) {
	bs := contentBlocks(r.Message.Content)
	var results []block
	for _, b := range bs {
		if b.Type == "tool_result" {
			results = append(results, b)
		}
	}
	tur := bytes.TrimSpace(r.ToolUseResult)
	hasTUR := len(tur) > 0 && !bytes.Equal(tur, []byte("null"))
	text := joinText(bs)

	if hasTUR || len(results) > 0 {
		// A result row is never a prompt. One with a toolUseResult but no
		// tool_result block has nothing to match and is dropped.
		if len(results) == 0 {
			return
		}
		for i, b := range results {
			var res json.RawMessage
			if hasTUR && i == 0 {
				res = tur
			}
			p.result(r, at, b, res)
		}
		// A result for a closed turn leaves no turn open.
		p.touch(r, at)
		for _, b := range bs {
			if b.Type == "text" && isInterrupt(b.Text) && p.cur != nil {
				p.cur.t.Interrupted = true
			}
		}
		return
	}
	if r.IsCompactSummary {
		p.summary("compact_summary", text, r.UUID, at)
		return
	}
	if r.IsMeta {
		return
	}
	if isLocalOutput(text) {
		// A local slash command's output: activity, not a prompt.
		p.touch(r, at)
		return
	}
	if isInterrupt(text) {
		if p.cur != nil {
			p.cur.t.Interrupted = true
			p.touch(r, at)
		}
		return
	}
	id := r.PromptID
	if id == "" {
		id = r.UUID
	}
	if p.cur != nil && id == p.cur.t.ID {
		// Another row of the same prompt (a queued or peer message):
		// the turn goes on.
		p.touch(r, at)
		return
	}
	if p.seen[id] {
		// Its turn already closed and was emitted: keep ids unique.
		id = r.UUID
	}
	kind, name := origin(r, text)
	if kind == "human" && p.src.AgentID != "" && len(bytes.TrimSpace(r.Origin)) == 0 {
		// A subagent's or workflow agent's prompts carry no origin: they
		// come from the conversation that spawned it, not the person.
		kind = "agent"
	}
	if p.cur != nil {
		end := p.cur.last
		if end.IsZero() {
			end = at
		}
		p.end(end, "next")
	}
	p.start(id, kind, name, text, r, at)
}

func (p *parser) assistant(r *row, at time.Time) {
	bs := contentBlocks(r.Message.Content)
	for _, b := range bs {
		switch b.Type {
		case "text":
			if p.cur != nil && strings.TrimSpace(b.Text) != "" {
				p.cur.t.Reply = clip(b.Text, MaxReply)
			}
		case "tool_use":
			p.implicit(r, at)
			t := p.cur.t
			c := &call{name: b.Name, input: b.Input, turn: p.cur, index: -1, at: at}
			if len(t.Tools) < MaxToolsShown {
				c.index = len(t.Tools)
				t.Tools = append(t.Tools, ToolCall{
					ID:     b.ID,
					Name:   b.Name,
					Target: target(b.Name, b.Input, r.Cwd),
					At:     at,
				})
			} else {
				t.ToolsMore++
			}
			if b.ID != "" {
				p.calls[b.ID] = c
			}
		}
	}
	p.touch(r, at)
}

func (p *parser) system(r *row, at time.Time) {
	switch r.Subtype {
	case "turn_duration":
		if p.cur != nil {
			p.end(at, "turn_duration")
		}
	case "away_summary", "compact_boundary":
		p.summary(r.Subtype, rawString(r.Content), r.UUID, at)
	}
}

// rawString is raw as a JSON string, or "" when it is not one.
func rawString(raw json.RawMessage) string {
	var s string
	if json.Unmarshal(raw, &s) != nil {
		return ""
	}
	return s
}

func (p *parser) summary(kind, text, uuid string, at time.Time) {
	s := &Summary{
		SessionID: p.src.SessionID,
		AgentID:   p.src.AgentID,
		Kind:      kind,
		Text:      clip(text, MaxSummary),
		RowUUID:   uuid,
		At:        at,
	}
	if s.Text != text {
		s.Full = text
	}
	p.emit(Item{Summary: s})
}

func (p *parser) prLink(r *row, at time.Time) {
	n, ok := flexInt(r.PrNumber)
	if !ok {
		return
	}
	p.emit(Item{PRLink: &PRLink{
		SessionID: p.src.SessionID,
		Repo:      r.PrRepository,
		Number:    n,
		URL:       r.PrURL,
		At:        at,
	}})
}

// flexInt reads a JSON number or a string holding one.
func flexInt(raw json.RawMessage) (int, bool) {
	raw = bytes.TrimSpace(raw)
	if len(raw) == 0 {
		return 0, false
	}
	if raw[0] == '"' {
		var s string
		if json.Unmarshal(raw, &s) != nil {
			return 0, false
		}
		raw = []byte(strings.TrimPrefix(strings.TrimSpace(s), "#"))
	}
	var n int
	if json.Unmarshal(raw, &n) != nil || n <= 0 {
		return 0, false
	}
	return n, true
}
