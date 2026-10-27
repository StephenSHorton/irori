package transcript

import (
	"encoding/json"
	"fmt"
	"testing"
	"time"
)

// Synthetic rows with the shapes of real Claude Code transcripts. No real
// transcript content is used.

const (
	fxSession = "11111111-2222-3333-4444-555555555555"
	fxCwd     = `C:\src\demo`
	fxBranch  = "main"
)

var fxBase = time.Date(2026, 10, 27, 9, 0, 0, 0, time.UTC)

// ts is fxBase plus n seconds, RFC 3339 as transcripts write it.
func ts(n int) string {
	return fxBase.Add(time.Duration(n) * time.Second).Format("2006-01-02T15:04:05.000Z")
}

func at(n int) time.Time { return fxBase.Add(time.Duration(n) * time.Second) }

type obj = map[string]any

func line(t *testing.T, m obj) []byte {
	t.Helper()
	b, err := json.Marshal(m)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// common fills the fields every conversation row has.
func common(typ, uuid string, sec int, extra obj) obj {
	m := obj{
		"parentUuid":  nil,
		"isSidechain": false,
		"type":        typ,
		"uuid":        uuid,
		"timestamp":   ts(sec),
		"userType":    "external",
		"entrypoint":  "cli",
		"cwd":         fxCwd,
		"sessionId":   fxSession,
		"version":     "2.1.300",
		"gitBranch":   fxBranch,
	}
	for k, v := range extra {
		m[k] = v
	}
	return m
}

// prompt is a typed human prompt (newer versions carry origin).
func prompt(uuid, promptID string, sec int, text string, extra obj) obj {
	m := common("user", uuid, sec, obj{
		"message": obj{"role": "user", "content": text},
	})
	if promptID != "" {
		m["promptId"] = promptID
	}
	for k, v := range extra {
		m[k] = v
	}
	return m
}

func assistant(uuid string, sec int, blocks ...obj) obj {
	bs := make([]any, len(blocks))
	for i, b := range blocks {
		bs[i] = b
	}
	return common("assistant", uuid, sec, obj{
		"message": obj{"id": "msg_" + uuid, "role": "assistant", "type": "message", "content": bs},
	})
}

func textBlock(s string) obj { return obj{"type": "text", "text": s} }

func thinking(s string) obj { return obj{"type": "thinking", "thinking": s, "signature": "sig"} }

func toolUse(id, name string, input obj) obj {
	return obj{"type": "tool_use", "id": id, "name": name, "input": input}
}

// result is a tool result row; tur is the toolUseResult (any shape, or nil to omit).
func result(uuid string, sec int, toolID string, isErr bool, tur any) obj {
	m := common("user", uuid, sec, obj{
		"message": obj{"role": "user", "content": []any{
			obj{"type": "tool_result", "tool_use_id": toolID, "content": "ok", "is_error": isErr},
		}},
	})
	if tur != nil {
		m["toolUseResult"] = tur
	}
	return m
}

func sys(uuid string, sec int, subtype string, extra obj) obj {
	m := common("system", uuid, sec, obj{"subtype": subtype, "isMeta": true})
	for k, v := range extra {
		m[k] = v
	}
	return m
}

func turnDuration(uuid string, sec int) obj {
	return sys(uuid, sec, "turn_duration", obj{"durationMs": 1234, "messageCount": 5})
}

// feed runs lines through a parser and returns every item, then the parser.
func feed(t *testing.T, src Source, rows ...obj) ([]Item, *Parser) {
	t.Helper()
	p := NewParser(src)
	var out []Item
	for _, r := range rows {
		out = append(out, p.Line(line(t, r))...)
	}
	return out, p
}

func mainSrc() Source { return Source{SessionID: fxSession} }

func turns(items []Item) []*Turn {
	var out []*Turn
	for _, it := range items {
		if it.Turn != nil {
			out = append(out, it.Turn)
		}
	}
	return out
}

func edits(items []Item) []*Edit {
	var out []*Edit
	for _, it := range items {
		if it.Edit != nil {
			out = append(out, it.Edit)
		}
	}
	return out
}

func bashes(items []Item) []*BashRun {
	var out []*BashRun
	for _, it := range items {
		if it.Bash != nil {
			out = append(out, it.Bash)
		}
	}
	return out
}

func summaries(items []Item) []*Summary {
	var out []*Summary
	for _, it := range items {
		if it.Summary != nil {
			out = append(out, it.Summary)
		}
	}
	return out
}

// kinds describes items in order, for checking emission order.
func kinds(items []Item) []string {
	var out []string
	for _, it := range items {
		n := 0
		var k string
		if it.Turn != nil {
			n++
			k = "turn:" + it.Turn.ID
		}
		if it.Edit != nil {
			n++
			k = "edit:" + it.Edit.Path
		}
		if it.Summary != nil {
			n++
			k = "summary:" + it.Summary.Kind
		}
		if it.PRLink != nil {
			n++
			k = fmt.Sprintf("pr:%d", it.PRLink.Number)
		}
		if it.Bash != nil {
			n++
			k = "bash:" + it.Bash.ToolUseID
		}
		if n != 1 {
			k = fmt.Sprintf("BAD(%d fields)", n)
		}
		out = append(out, k)
	}
	return out
}
