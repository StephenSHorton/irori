package mcpserver

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

var now = time.Date(2026, 10, 26, 12, 0, 0, 0, time.UTC)

// fake is a Source built from synthetic rows.
type fake struct {
	cards     []presence.AgentCard
	events    []store.Event
	err       error
	gotKinds  []string
	gotLimit  int
	agentHits int
}

func (f *fake) Agents(context.Context) ([]presence.AgentCard, error) {
	f.agentHits++
	return f.cards, f.err
}

func (f *fake) Events(_ context.Context, kinds []string, limit int) ([]store.Event, error) {
	f.gotKinds, f.gotLimit = kinds, limit
	return f.events, f.err
}

func (f *fake) Device(context.Context) (device.Identity, error) {
	return device.Identity{ID: "dev1", Name: "testbox", OS: "linux"}, f.err
}

func sampleCards() []presence.AgentCard {
	return []presence.AgentCard{
		{SessionID: "s1", Agent: "claude", PID: 11, Name: "alpha", Title: "Fix the parser", Cwd: `C:\work\irori`,
			Status: "busy", LastActivity: now.Add(-90 * time.Second), LastText: "Parser fixed;\nrunning tests."},
		{SessionID: "s2", Agent: "claude", PID: 22, Name: "beta", Cwd: "/home/u/other",
			Status: "idle", UpdatedAt: now.Add(-3 * time.Hour)},
	}
}

func newTestTools(src Source) *tools {
	return newTools(Options{Source: src, Now: func() time.Time { return now }})
}

func resultText(t *testing.T, res *mcp.CallToolResult) string {
	t.Helper()
	if len(res.Content) != 1 {
		t.Fatalf("want 1 content block, got %d", len(res.Content))
	}
	tc, ok := res.Content[0].(*mcp.TextContent)
	if !ok {
		t.Fatalf("want text content, got %T", res.Content[0])
	}
	return tc.Text
}

func TestAgentsListsAndFilters(t *testing.T) {
	tl := newTestTools(&fake{cards: sampleCards()})
	res, _, err := tl.agents(context.Background(), nil, AgentsInput{})
	if err != nil {
		t.Fatal(err)
	}
	out := resultText(t, res)
	for _, want := range []string{"2 of 2 agents on this device (testbox)", "untrusted", "alpha [busy]",
		"title: Fix the parser", `cwd: C:\work\irori`, "last activity: 1m ago", `"Parser fixed; running tests."`,
		"beta [idle]", "last activity: 3h ago"} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in:\n%s", want, out)
		}
	}

	res, _, _ = tl.agents(context.Background(), nil, AgentsInput{Cwd: "work/IRORI"})
	out = resultText(t, res)
	if !strings.Contains(out, "1 of 2 agents") || !strings.Contains(out, "alpha") || strings.Contains(out, "beta") {
		t.Errorf("cwd filter:\n%s", out)
	}
	res, _, _ = tl.agents(context.Background(), nil, AgentsInput{Status: "IDLE"})
	out = resultText(t, res)
	if !strings.Contains(out, "beta") || strings.Contains(out, "alpha [") {
		t.Errorf("status filter:\n%s", out)
	}
	res, _, _ = tl.agents(context.Background(), nil, AgentsInput{Status: "gone"})
	if out = resultText(t, res); !strings.HasPrefix(out, "0 of 2 agents") {
		t.Errorf("empty filter:\n%s", out)
	}
}

func TestNotRunning(t *testing.T) {
	for name, src := range map[string]Source{
		"nil source": nil,
		"dial fails": Redial(func(context.Context) (Source, error) { return nil, errors.New("no lockfile") }, nil),
		"wrapped":    &fake{err: fmt.Errorf("x: %w", ErrNotRunning)},
	} {
		tl := newTestTools(src)
		res, _, err := tl.agents(context.Background(), nil, AgentsInput{})
		if err != nil || res.IsError || resultText(t, res) != NotRunningText {
			t.Errorf("%s agents: err=%v isError=%v text=%q", name, err, res.IsError, resultText(t, res))
		}
		res, _, err = tl.events(context.Background(), nil, EventsInput{})
		if err != nil || res.IsError || resultText(t, res) != NotRunningText {
			t.Errorf("%s events: err=%v isError=%v text=%q", name, err, res.IsError, resultText(t, res))
		}
	}
}

func TestOtherErrorIsToolError(t *testing.T) {
	tl := newTestTools(&fake{err: errors.New("api: 500")})
	res, _, err := tl.agents(context.Background(), nil, AgentsInput{})
	if err != nil || !res.IsError || !strings.Contains(resultText(t, res), "api: 500") {
		t.Fatalf("err=%v res=%+v", err, res)
	}
}

func stamp(d time.Duration) hlc.Timestamp { return hlc.Timestamp(now.Add(d).UnixMilli()) << 16 }

func TestEventsNewestFirstAndLimit(t *testing.T) {
	f := &fake{events: []store.Event{
		{DeviceID: "abcdef0123456789", Seq: 1, HLC: stamp(-time.Hour), Actor: "irori", Kind: store.KindDeviceStarted,
			Payload: json.RawMessage(`{ "name": "testbox",  "os": "linux" }`)},
		{DeviceID: "abcdef0123456789", Seq: 2, HLC: stamp(-5 * time.Minute), Actor: "irori", Kind: store.KindAgentStatus,
			Payload: json.RawMessage(`{"key":"abcdef/s1","status":"busy"}`)},
	}}
	tl := newTestTools(f)
	res, _, err := tl.events(context.Background(), nil, EventsInput{Kinds: []string{" agent. ", ""}, Limit: 500})
	if err != nil {
		t.Fatal(err)
	}
	if f.gotLimit != MaxEventLimit || len(f.gotKinds) != 1 || f.gotKinds[0] != "agent." {
		t.Errorf("passed kinds=%q limit=%d", f.gotKinds, f.gotLimit)
	}
	out := resultText(t, res)
	i, j := strings.Index(out, "seq=2"), strings.Index(out, "seq=1")
	if i < 0 || j < 0 || i > j {
		t.Errorf("want seq 2 before seq 1:\n%s", out)
	}
	for _, want := range []string{"2 events on this device (testbox), newest first (kinds: agent.)",
		"dev=abcdef01", "(5m ago)", `{"name":"testbox","os":"linux"}`, "untrusted"} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in:\n%s", want, out)
		}
	}

	tl.events(context.Background(), nil, EventsInput{})
	if f.gotLimit != DefaultEventLimit {
		t.Errorf("default limit = %d", f.gotLimit)
	}
	res, _, _ = tl.events(context.Background(), nil, EventsInput{Limit: 1})
	if out = resultText(t, res); strings.Contains(out, "seq=1") || !strings.Contains(out, "seq=2") {
		t.Errorf("local limit not applied:\n%s", out)
	}
}

func TestEventPayloadIsClippedPerRow(t *testing.T) {
	big := `{"lastText":"` + strings.Repeat("x", 5000) + `"}`
	tl := newTestTools(&fake{events: []store.Event{{Seq: 1, HLC: stamp(0), Kind: store.KindAgentSeen, Payload: json.RawMessage(big)}}})
	res, _, _ := tl.events(context.Background(), nil, EventsInput{})
	out := resultText(t, res)
	if len(out) > 1000 || !strings.Contains(out, "…") {
		t.Errorf("payload not clipped (%d bytes)", len(out))
	}
}

func TestAgentsTruncated(t *testing.T) {
	var cards []presence.AgentCard
	for i := range 200 {
		cards = append(cards, presence.AgentCard{SessionID: fmt.Sprintf("s%d", i), Name: fmt.Sprintf("agent-%d", i),
			Cwd: "/w", Status: "busy", LastActivity: now, LastText: strings.Repeat("é", 300)})
	}
	tl := newTestTools(&fake{cards: cards})
	res, _, _ := tl.agents(context.Background(), nil, AgentsInput{})
	out := resultText(t, res)
	if len(out) > MaxResultBytes || !utf8.ValidString(out) {
		t.Fatalf("len %d valid %v", len(out), utf8.ValidString(out))
	}
	if !strings.Contains(out, "[truncated: showing ") || !strings.Contains(out, "narrow the query") {
		t.Errorf("no truncation note:\n%s", out[len(out)-200:])
	}
}

func TestClipOneHugeLine(t *testing.T) {
	s := strings.Repeat("日本", 10000)
	out := Clip(s, 1000)
	if len(out) > 1000 || !utf8.ValidString(out) || !strings.Contains(out, "truncated") {
		t.Fatalf("len %d valid %v", len(out), utf8.ValidString(out))
	}
	if Clip("short", 1000) != "short" {
		t.Fatal("short text changed")
	}
}

func TestOneLine(t *testing.T) {
	cases := []struct {
		in   string
		max  int
		want string
	}{
		{"  a\n\n- b\t\x00c ", 20, "a - b c"},
		{"abcdef", 4, "abc…"},
		{"ab cdef", 4, "ab…"},
		{"abcd", 4, "abcd"},
	}
	for _, c := range cases {
		if got := oneLine(c.in, c.max); got != c.want {
			t.Errorf("oneLine(%q, %d) = %q, want %q", c.in, c.max, got, c.want)
		}
	}
}

func TestAgo(t *testing.T) {
	cases := []struct {
		d    time.Duration
		want string
	}{
		{0, "just now"}, {42 * time.Second, "42s ago"}, {5 * time.Minute, "5m ago"},
		{3 * time.Hour, "3h ago"}, {72 * time.Hour, "3d ago"}, {-time.Hour, "in the future"},
	}
	for _, c := range cases {
		if got := Ago(now, now.Add(-c.d)); got != c.want {
			t.Errorf("Ago(-%v) = %q, want %q", c.d, got, c.want)
		}
	}
	if Ago(now, time.Time{}) != "never" {
		t.Error("zero time")
	}
}

func TestRedialRetriesAfterFailure(t *testing.T) {
	dials := 0
	good := &fake{cards: sampleCards()}
	broken := &fake{err: errors.New("connection refused")}
	src := Redial(func(context.Context) (Source, error) {
		dials++
		switch dials {
		case 1:
			return nil, errors.New("not yet")
		case 2:
			return broken, nil
		}
		return good, nil
	}, nil)
	ctx := context.Background()
	if _, err := src.Agents(ctx); !errors.Is(err, ErrNotRunning) {
		t.Fatalf("first call: %v", err)
	}
	// Dials the broken client, fails, redials and gets the good one.
	cards, err := src.Agents(ctx)
	if err != nil || len(cards) != 2 || dials != 3 {
		t.Fatalf("cards=%d err=%v dials=%d", len(cards), err, dials)
	}
	// Cached now.
	if _, err := src.Agents(ctx); err != nil || dials != 3 {
		t.Fatalf("err=%v dials=%d", err, dials)
	}
}

// TestRoundTrip drives the real server over the SDK's in-memory transport,
// once on the stateless 2026-07-28 path (server/discover) and once per legacy
// initialize version that clients fall back to.
func TestRoundTrip(t *testing.T) {
	for _, tc := range []struct{ ask, want string }{
		{"", "2026-07-28"},
		{"2025-11-25", "2025-11-25"},
		{"2025-06-18", "2025-06-18"},
		{"2025-03-26", "2025-03-26"},
	} {
		t.Run("v"+tc.want, func(t *testing.T) {
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			srv := New(Options{Source: &fake{cards: sampleCards()}, Version: "test", Now: func() time.Time { return now }})
			st, ct := mcp.NewInMemoryTransports()
			done := make(chan error, 1)
			go func() { done <- Serve(ctx, srv, st) }()

			client := mcp.NewClient(&mcp.Implementation{Name: "test-client", Version: "0"}, nil)
			cs, err := client.Connect(ctx, ct, &mcp.ClientSessionOptions{ProtocolVersion: tc.ask})
			if err != nil {
				t.Fatal(err)
			}
			if got := cs.InitializeResult().ProtocolVersion; got != tc.want {
				t.Errorf("negotiated %q, want %q", got, tc.want)
			}
			if got := cs.InitializeResult().ServerInfo.Name; got != "irori" {
				t.Errorf("server name %q", got)
			}

			list, err := cs.ListTools(ctx, nil)
			if err != nil {
				t.Fatal(err)
			}
			var names []string
			for _, tool := range list.Tools {
				names = append(names, tool.Name)
				if !strings.Contains(tool.Description, "untrusted") || !strings.Contains(tool.Description, "Peer observation") {
					t.Errorf("%s description: %q", tool.Name, tool.Description)
				}
				if tool.Annotations == nil || !tool.Annotations.ReadOnlyHint {
					t.Errorf("%s not read-only", tool.Name)
				}
				schema, _ := json.Marshal(tool.InputSchema)
				if strings.Contains(string(schema), `"required"`) {
					t.Errorf("%s has required args: %s", tool.Name, schema)
				}
				if tool.Name == ToolEvents && !strings.Contains(string(schema), "default 20, max 100") {
					t.Errorf("limit description lost: %s", schema)
				}
				if tool.OutputSchema != nil {
					t.Errorf("%s has an output schema", tool.Name)
				}
			}
			if strings.Join(names, ",") != ToolAgents+","+ToolEvents {
				t.Errorf("tools = %v", names)
			}

			res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: ToolAgents, Arguments: map[string]any{"status": "busy"}})
			if err != nil {
				t.Fatal(err)
			}
			if out := resultText(t, res); res.IsError || !strings.Contains(out, "alpha [busy]") || strings.Contains(out, "beta") {
				t.Errorf("irori_agents: %s", out)
			}
			res, err = cs.CallTool(ctx, &mcp.CallToolParams{Name: ToolEvents, Arguments: map[string]any{"limit": 5}})
			if err != nil {
				t.Fatal(err)
			}
			if out := resultText(t, res); res.IsError || !strings.HasPrefix(out, "0 events") {
				t.Errorf("irori_events: %s", out)
			}

			// A client hanging up is a normal end, not an error.
			cs.Close()
			select {
			case err := <-done:
				if err != nil {
					t.Errorf("Serve after client close: %v", err)
				}
			case <-time.After(5 * time.Second):
				t.Error("Serve didn't return after client close")
			}
		})
	}
}
