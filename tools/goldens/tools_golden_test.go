package tools

// Overlaid into I:internal/tools by tools/goldens/export.sh (the Go repo is
// never edited). It runs testdata/golden/tools/scenario.json over the
// package's own harness (newHarness, tools_test.go) and records every
// result, plus the spec lists and the shared instructions, into
// $IRORI_GOLDEN_OUT/tools. The Rust tools golden test replays the same
// scenario and must match byte for byte, ids masked.

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/inbox"
)

type goldenStep struct {
	Op      string            `json:"op"`
	N       int               `json:"n"`
	Steps   []goldenStep      `json:"steps"`
	To      string            `json:"to"`
	Text    string            `json:"text"`
	Caller  string            `json:"caller"`
	Tool    string            `json:"tool"`
	Args    string            `json:"args"`
	Save    map[string]string `json:"save"`
	Seconds int               `json:"seconds"`
	Key     string            `json:"key"`
	Path    string            `json:"path"`
	PR      string            `json:"pr"`
	Keys    []string          `json:"keys"`
	Status  string            `json:"status"`
	Deliv   string            `json:"delivery"`
}

type goldenResult struct {
	Step int    `json:"step"`
	Tool string `json:"tool"`
	Text string `json:"text"`
}

func goldenKey(k string) string {
	switch k {
	case "A":
		return keyA
	case "B":
		return keyB
	case "C":
		return keyC
	case "G":
		return keyG
	}
	return k
}

func goldenCaller(c string) Caller {
	switch c {
	case "A":
		return callA
	case "B":
		return callB
	case "C":
		return callC
	case "G":
		return grokCaller()
	}
	return Caller{}
}

func goldenWrite(t *testing.T, dir, name string, v any) {
	t.Helper()
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, name), append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

// goldenRun plays steps over h; results are numbered by their top-level
// step (and repeat round, for steps inside a repeat).
type goldenRun struct {
	t       *testing.T
	h       *harness
	vars    map[string]string
	results []goldenResult
}

func (r *goldenRun) subst(s string) string {
	for k, v := range r.vars {
		s = strings.ReplaceAll(s, "$"+k, v)
	}
	return s
}

func (r *goldenRun) steps(base int, steps []goldenStep) {
	h := r.h
	for j, st := range steps {
		step := base
		if base < 0 {
			step = j
		}
		switch st.Op {
		case "call":
			h.clk.advance(time.Second)
			text, err := h.set.Call(bg, goldenCaller(st.Caller), st.Tool, json.RawMessage(r.subst(st.Args)))
			if err != nil {
				text = "call error: " + err.Error()
			}
			r.results = append(r.results, goldenResult{Step: step, Tool: st.Tool, Text: text})
			for name, label := range st.Save {
				r.vars[r.subst(name)] = field(r.t, text, label)
			}
		case "repeat":
			for n := 0; n < st.N; n++ {
				r.vars["i"] = fmt.Sprintf("%02d", n)
				r.steps(step, st.Steps)
			}
		case "person_send":
			person := inbox.Sender{Key: "user:dev1", Name: "you", Kind: "user"}
			if _, _, err := h.b.ib.Send(bg, person, inbox.Address{Kind: inbox.ToAgent, Value: r.subst(st.To)}, r.subst(st.Text), ""); err != nil {
				r.t.Fatalf("step %d: person send: %v", step, err)
			}
		case "advance":
			h.clk.advance(time.Duration(st.Seconds) * time.Second)
		case "reachable":
			h.b.ib.SetReachable(goldenKey(st.Key), inbox.Path(st.Path), true)
		case "owner":
			var keys []string
			for _, k := range st.Keys {
				keys = append(keys, goldenKey(k))
			}
			h.owners[st.PR] = keys
		case "status":
			h.b.roster.mu.Lock()
			c := h.b.roster.cards[goldenKey(st.Key)]
			c.Status = st.Status
			h.b.roster.cards[c.Key] = c
			h.b.roster.mu.Unlock()
		case "release":
			if err := h.b.ib.Release(bg, r.subst(st.Deliv), "user:dev1"); err != nil {
				r.t.Fatalf("step %d: release: %v", step, err)
			}
		default:
			r.t.Fatalf("step %d: unknown op %q", step, st.Op)
		}
	}
}

// TestGoldenToolsScenario runs each scenario file (<name>.json) into
// <name>.transcript.json.
func TestGoldenToolsScenario(t *testing.T) {
	out := os.Getenv("IRORI_GOLDEN_OUT")
	if out == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	dir := filepath.Join(out, "tools")
	for _, name := range []string{"scenario", "budget"} {
		raw, err := os.ReadFile(filepath.Join(dir, name+".json"))
		if err != nil {
			t.Fatal(err)
		}
		var sc struct {
			Steps []goldenStep `json:"steps"`
		}
		if err := json.Unmarshal(raw, &sc); err != nil {
			t.Fatal(err)
		}
		r := &goldenRun{t: t, h: newHarness(t), vars: map[string]string{}}
		r.steps(-1, sc.Steps)
		goldenWrite(t, dir, name+".transcript.json", r.results)
	}
}

func TestGoldenToolsSpecs(t *testing.T) {
	out := os.Getenv("IRORI_GOLDEN_OUT")
	if out == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	dir := filepath.Join(out, "tools")
	s := newHarness(t).set
	specs := map[string]string{}
	for _, client := range []string{"", "claude", "grok", "other"} {
		data, err := json.Marshal(s.Specs(client))
		if err != nil {
			t.Fatal(err)
		}
		specs[client] = string(data)
	}
	goldenWrite(t, dir, "specs.json", map[string]any{
		"specs":             specs,
		"instructions":      Instructions,
		"boardInstructions": BoardInstructions,
		"askInstructions":   AskInstructions,
	})
}
