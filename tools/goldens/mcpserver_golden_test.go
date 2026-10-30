package mcpserver

// Overlaid into I:internal/mcpserver by tools/goldens/export.sh (the Go
// repo is never edited). It drives `irori mcp`'s server over the SDK's
// in-memory transport with raw JSON-RPC lines and records every line the
// server writes back, into $IRORI_GOLDEN_OUT/mcp/wire.json:
//   - not running (no Core): initialize at each protocol version,
//     tools/list (the embedded grok specs), ping, calls that answer the
//     not-running text, server/discover;
//   - over the fake Core (fakecore_test.go, httptest on 127.0.0.1): a
//     forwarded call, join, a call after join and one channel push.
// The Rust mcp golden test replays the same lines and compares each answer
// as canonical JSON.

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/StephenSHorton/irori/internal/attach"
)

type wireStep struct {
	Send string   `json:"send,omitempty"`
	Push string   `json:"push,omitempty"` // a deliver event fed to the member stream
	Recv []string `json:"recv"`
}

type wireRun struct {
	Name  string     `json:"name"`
	Steps []wireStep `json:"steps"`
}

// wire runs srv on an in-memory transport and returns the client end.
func wire(t *testing.T, srv *Server) mcp.Connection {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	st, ct := mcp.NewInMemoryTransports()
	done := make(chan error, 1)
	go func() { done <- srv.Run(ctx, st) }()
	conn, err := ct.Connect(ctx)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	t.Cleanup(func() {
		conn.Close()
		cancel()
		select {
		case <-done:
		case <-time.After(5 * time.Second):
			t.Error("Run didn't return")
		}
	})
	return conn
}

// exchange writes line (if any) and reads n messages back.
func exchange(t *testing.T, conn mcp.Connection, line string, n int) []string {
	t.Helper()
	ctx := context.Background()
	if line != "" {
		msg, err := jsonrpc.DecodeMessage([]byte(line))
		if err != nil {
			t.Fatalf("%s: %v", line, err)
		}
		if err := conn.Write(ctx, msg); err != nil {
			t.Fatal(err)
		}
	}
	var out []string
	for range n {
		rctx, cancel := context.WithTimeout(ctx, 5*time.Second)
		msg, err := conn.Read(rctx)
		cancel()
		if err != nil {
			t.Fatalf("after %s: %v", line, err)
		}
		data, err := jsonrpc.EncodeMessage(msg)
		if err != nil {
			t.Fatal(err)
		}
		out = append(out, string(data))
	}
	return out
}

func initLine(id int, version string) string {
	return `{"jsonrpc":"2.0","id":` + itoa(id) + `,"method":"initialize","params":{"protocolVersion":"` + version +
		`","capabilities":{},"clientInfo":{"name":"golden","version":"0"}}}`
}

func itoa(n int) string {
	b, _ := json.Marshal(n)
	return string(b)
}

const initialized = `{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}`

func TestGoldenMCPWire(t *testing.T) {
	out := os.Getenv("IRORI_GOLDEN_OUT")
	if out == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	var runs []wireRun
	step := func(r *wireRun, conn mcp.Connection, line string, n int) {
		r.Steps = append(r.Steps, wireStep{Send: line, Recv: exchange(t, conn, line, n)})
	}

	for _, v := range []string{"2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05", "1999-01-01"} {
		r := wireRun{Name: "initialize " + v}
		conn := wire(t, New(Options{Version: "1.2.3"}))
		step(&r, conn, initLine(1, v), 1)
		runs = append(runs, r)
	}

	{
		r := wireRun{Name: "discover first"}
		conn := wire(t, New(Options{Version: "1.2.3"}))
		step(&r, conn, `{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"golden","version":"0"}}}}`, 1)
		runs = append(runs, r)
	}

	{
		r := wireRun{Name: "not running"}
		conn := wire(t, New(Options{Version: "1.2.3"}))
		step(&r, conn, initLine(1, "2025-06-18"), 1)
		step(&r, conn, initialized, 0)
		step(&r, conn, `{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":3,"method":"ping"}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"irori_agents","arguments":{}}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"irori_send","arguments":{"to":"a","text":"b"}}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"irori_join","arguments":{"name":"g"}}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"irori_join","arguments":{}}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"irori_nope","arguments":{}}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":9,"method":"no/such","params":{}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":10,"method":"server/discover","params":{}}`, 1)
		runs = append(runs, r)
	}

	{
		r := wireRun{Name: "over a Core"}
		f := newFakeCore(t, testSpecs())
		conn := wire(t, New(quick(f)))
		step(&r, conn, initLine(1, "2025-06-18"), 1)
		step(&r, conn, initialized, 0)
		step(&r, conn, `{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"irori_agents","arguments":{"status":"busy"}}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"irori_send","arguments":{"to":"a","text":"hi"}}}`, 1)
		step(&r, conn, `{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"irori_join","arguments":{"name":" grok\nbot "},"_meta":{"session_id":"conv-42"}}}`, 1)
		waitStream(t, f)
		step(&r, conn, `{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"irori_send","arguments":{"to":"a","text":"hi"}}}`, 1)
		ev := attach.Event{Type: attach.TypeDeliver, Prompt: "<irori-message from=\"alpha\" key=\"dev1/s1\" delivery=\"d1\" trust=\"untrusted\">hello</irori-message>",
			DeliveryIDs: []string{"d1", "d2"}, MessageIDs: []string{"m1"}, From: "alpha", Mode: "next_turn"}
		raw, _ := json.Marshal(ev)
		f.events <- ev
		r.Steps = append(r.Steps, wireStep{Push: string(raw), Recv: exchange(t, conn, "", 1)})
		runs = append(runs, r)
	}

	dir := filepath.Join(out, "mcp")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	data, err := json.MarshalIndent(map[string]any{
		"runs":         runs,
		"instructions": instructions,
		"fallback":     json.RawMessage(fallbackJSON),
		"texts": map[string]string{
			"notRunning": NotRunningText,
			"joinFirst":  JoinFirstText,
			"superseded": SupersededText,
		},
	}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "wire.json"), append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
