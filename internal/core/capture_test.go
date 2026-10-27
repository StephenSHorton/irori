package core

import (
	"bufio"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/discovery"
)

// Start returns at once over a large transcript: backfill runs inside
// capture.Run, after Start. The lockfile token is scrubbed from captured
// prompts.
func TestStartWithLargeTranscript(t *testing.T) {
	data, claude := t.TempDir(), t.TempDir()
	t.Setenv("IRORI_HOME", data)
	const sid = "cccccccc-3333-4333-8333-000000000003"
	path := filepath.Join(claude, "projects", "C--work", sid+".jsonl")
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	w := bufio.NewWriterSize(f, 1<<20)
	row := func(m map[string]any) {
		b, _ := json.Marshal(m)
		w.Write(b)
		w.WriteByte('\n')
	}
	now := time.Now().UTC()
	stamp := now.Format(time.RFC3339Nano)
	// ~48 MiB, mostly api-request rows (skipped before decoding).
	pad := strings.Repeat("x", 8<<10)
	for i := 0; i < 6000; i++ {
		row(map[string]any{"type": "api-request-log", "timestamp": stamp, "body": pad})
		if i%1000 == 0 {
			row(map[string]any{"type": "user", "uuid": "u" + time.Duration(i).String(), "timestamp": stamp,
				"sessionId": sid, "cwd": claude, "message": map[string]any{"role": "user", "content": "warm up"}})
			row(map[string]any{"type": "system", "subtype": "turn_duration", "timestamp": stamp, "sessionId": sid, "cwd": claude})
		}
	}
	if err := w.Flush(); err != nil {
		t.Fatal(err)
	}
	f.Close()

	start := time.Now()
	c, err := Start(context.Background(), Options{DataDir: data, ClaudeDir: claude, Version: "0.3.0-test", Headless: true})
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	if d := time.Since(start); d > 3*time.Second {
		t.Fatalf("Start took %v", d)
	}
	if c.Capture() == nil {
		t.Fatal("no capture")
	}
	lf, err := discovery.Read(data)
	if err != nil {
		t.Fatal(err)
	}

	// A prompt carrying the token, after backfill.
	g, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(map[string]any{"type": "user", "uuid": "u-token", "promptId": "p-token", "timestamp": time.Now().UTC().Format(time.RFC3339Nano),
		"sessionId": sid, "cwd": claude, "message": map[string]any{"role": "user", "content": "the token is " + lf.Token}})
	d, _ := json.Marshal(map[string]any{"type": "system", "subtype": "turn_duration", "timestamp": time.Now().UTC().Format(time.RFC3339Nano), "sessionId": sid, "cwd": claude})
	g.Write(append(append(b, '\n'), append(d, '\n')...))
	g.Close()

	key := c.Device().ID + "/" + sid
	deadline := time.Now().Add(30 * time.Second)
	for {
		var found *capture.TurnView
		for _, e := range c.Capture().Timeline(capture.TimelineQuery{Key: key, Limit: 100}).Entries {
			if e.Turn != nil && e.Turn.TurnID == "p-token" {
				found = e.Turn
			}
		}
		if found != nil {
			if strings.Contains(found.Prompt, lf.Token) || !strings.Contains(found.Prompt, "[redacted:exact]") {
				t.Fatalf("prompt = %q", found.Prompt)
			}
			if st := c.Capture().Status(); st.Backfilling || st.Files != 1 {
				t.Fatalf("status = %+v", st)
			}
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("turn never captured; status %+v", c.Capture().Status())
		}
		time.Sleep(100 * time.Millisecond)
	}
}
