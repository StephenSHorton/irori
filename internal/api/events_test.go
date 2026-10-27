package api

import (
	"bufio"
	"context"
	"net/http"
	"path/filepath"
	"slices"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// storeBackend serves a real event log as device "dev1".
type storeBackend struct {
	*fakeBackend
	st *store.Store
}

func (b *storeBackend) Events(ctx context.Context, f store.Filter) ([]store.Event, error) {
	return b.st.List(ctx, f)
}

func (b *storeBackend) SubscribeEvents(ctx context.Context) <-chan store.Event {
	return b.st.Subscribe(ctx)
}

func startStore(t *testing.T) (*store.Store, *Client) {
	t.Helper()
	st, err := store.Open(context.Background(), filepath.Join(t.TempDir(), "events.db"), "dev1", hlc.New(time.Now))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	port := startWith(t, &storeBackend{fakeBackend: &fakeBackend{}, st: st})
	return st, NewClient(discovery.Lockfile{Port: port, Token: testToken})
}

func appendKinds(t *testing.T, st *store.Store, kinds ...string) {
	t.Helper()
	for _, k := range kinds {
		if _, err := st.Append(context.Background(), "irori", k, map[string]string{"key": "dev1/s1"}); err != nil {
			t.Fatal(err)
		}
	}
}

// next reads one item or fails after 2 s.
func next(t *testing.T, ch <-chan StreamItem) StreamItem {
	t.Helper()
	select {
	case it, ok := <-ch:
		if !ok {
			t.Fatal("stream closed")
		}
		if it.Err != nil {
			t.Fatal(it.Err)
		}
		return it
	case <-time.After(2 * time.Second):
		t.Fatal("no stream item within 2s")
	}
	return StreamItem{}
}

func TestMatchKind(t *testing.T) {
	for _, tc := range []struct {
		kinds []string
		kind  string
		want  bool
	}{
		{nil, "agent.seen", true},
		{[]string{"agent.seen"}, "agent.seen", true},
		{[]string{"agent.seen"}, "agent.status", false},
		{[]string{"agent."}, "agent.status", true},
		{[]string{"agent."}, "agentx.status", false},
		{[]string{"agent"}, "agent.status", false},
		{[]string{"device.started", "agent."}, "device.started", true},
	} {
		if got := matchKind(tc.kinds, tc.kind); got != tc.want {
			t.Errorf("matchKind(%q, %q) = %v, want %v", tc.kinds, tc.kind, got, tc.want)
		}
	}
}

func TestParseKinds(t *testing.T) {
	got := parseKinds(" agent., ,device.started,")
	if want := []string{"agent.", "device.started"}; !slices.Equal(got, want) {
		t.Fatalf("parseKinds = %q, want %q", got, want)
	}
}

func TestEventsStreamReplayThenLive(t *testing.T) {
	st, c := startStore(t)
	appendKinds(t, st, "agent.seen", "device.started", "agent.status") // seq 1..3
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch, err := c.EventsStream(ctx, []string{"agent."}, 1)
	if err != nil {
		t.Fatal(err)
	}
	if it := next(t, ch); it.Event.Seq != 3 || it.Event.Kind != "agent.status" {
		t.Fatalf("replay = %+v, want seq 3 agent.status", it)
	}
	appendKinds(t, st, "agent.gone", "device.started", "agent.status") // seq 4..6
	for _, want := range []int64{4, 6} {
		if it := next(t, ch); it.Event.Seq != want {
			t.Fatalf("live = seq %d (%s), want %d", it.Event.Seq, it.Event.Kind, want)
		}
	}
	// A row replicated from another device has no replay but arrives live.
	ev := store.Event{DeviceID: "dev2", Seq: 1, HLC: hlc.New(time.Now).Now(), Actor: "irori", Kind: "agent.seen", Payload: []byte(`{}`)}
	if err := st.Insert(context.Background(), ev); err != nil {
		t.Fatal(err)
	}
	if it := next(t, ch); it.Event.DeviceID != "dev2" || it.Event.Seq != 1 {
		t.Fatalf("replicated = %+v", it.Event)
	}
	cancel()
	for range ch { // the reader goroutine ends with ctx
	}
}

func TestEventsStreamReplayModes(t *testing.T) {
	for _, tc := range []struct {
		name  string
		since int64
		want  []int64 // replayed seqs, before the live row (seq 3)
	}{
		{"no replay", NoReplay, nil},
		{"all", 0, []int64{1, 2}},
		{"after 1", 1, []int64{2}},
		{"after the end", 2, nil},
	} {
		t.Run(tc.name, func(t *testing.T) {
			st, c := startStore(t)
			appendKinds(t, st, "agent.seen", "agent.status")
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			ch, err := c.EventsStream(ctx, nil, tc.since)
			if err != nil {
				t.Fatal(err)
			}
			var got []int64
			for _, s := range tc.want {
				it := next(t, ch)
				if it.Event.Seq != s {
					t.Fatalf("replay seq %d, want %d", it.Event.Seq, s)
				}
				got = append(got, it.Event.Seq)
			}
			// The stream is live: a new row arrives next (and nothing older).
			ev, err := st.Append(context.Background(), "irori", "agent.status", map[string]string{})
			if err != nil {
				t.Fatal(err)
			}
			if it := next(t, ch); it.Event.Seq != ev.Seq {
				t.Fatalf("after replay %v got seq %d, want live seq %d", got, it.Event.Seq, ev.Seq)
			}
		})
	}
}

func TestEventsStreamResyncAndClose(t *testing.T) {
	b := &fakeBackend{live: make(chan store.Event, 1)}
	port := startWith(t, b)
	resp := do(t, port, http.MethodGet, "/v1/events/stream", bearer(testToken))
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status %d", resp.StatusCode)
	}
	if ct := resp.Header.Get("Content-Type"); ct != "application/x-ndjson" {
		t.Fatalf("content type %q", ct)
	}
	b.live <- store.Event{}
	r := bufio.NewReader(resp.Body)
	line, err := r.ReadString('\n')
	if err != nil {
		t.Fatal(err)
	}
	if line != "{\"resync\":true}\n" {
		t.Fatalf("line %q, want resync", line)
	}
	close(b.live) // the store closed: the stream ends
	done := make(chan error, 1)
	go func() { _, err := r.ReadString('\n'); done <- err }()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("stream wrote another line after the store closed")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("stream did not end when the store closed")
	}
}

func TestEventsStreamClientResync(t *testing.T) {
	b := &fakeBackend{live: make(chan store.Event, 1)}
	port := startWith(t, b)
	c := NewClient(discovery.Lockfile{Port: port, Token: testToken})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch, err := c.EventsStream(ctx, nil, NoReplay)
	if err != nil {
		t.Fatal(err)
	}
	b.live <- store.Event{}
	if it := next(t, ch); !it.Resync {
		t.Fatalf("item %+v, want resync", it)
	}
	close(b.live)
	select {
	case it, ok := <-ch:
		if ok {
			t.Fatalf("item %+v after the server ended the stream", it)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("client channel did not close")
	}
}

func TestEventsStreamKeepalive(t *testing.T) {
	old := Keepalive
	Keepalive = 20 * time.Millisecond
	t.Cleanup(func() { Keepalive = old })
	_, port := start(t)
	resp := do(t, port, http.MethodGet, "/v1/events/stream", bearer(testToken))
	line, err := bufio.NewReader(resp.Body).ReadString('\n')
	if err != nil || line != "\n" {
		t.Fatalf("first line %q (%v), want a keepalive", line, err)
	}
}

func TestEventsStreamRules(t *testing.T) {
	_, port := start(t)
	for _, tc := range []struct {
		name string
		path string
		mod  func(*http.Request)
		want int
	}{
		{"no token", "/v1/events/stream", nil, http.StatusUnauthorized},
		{"wrong token", "/v1/events/stream", bearer("nope"), http.StatusUnauthorized},
		{"bad host", "/v1/events/stream", func(r *http.Request) { bearer(testToken)(r); r.Host = "evil.example:80" }, http.StatusForbidden},
		{"origin", "/v1/events/stream", func(r *http.Request) { bearer(testToken)(r); r.Header.Set("Origin", "http://x") }, http.StatusForbidden},
		{"bad since", "/v1/events/stream?since=x", bearer(testToken), http.StatusBadRequest},
		{"negative since", "/v1/events/stream?since=-1", bearer(testToken), http.StatusBadRequest},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := do(t, port, http.MethodGet, tc.path, tc.mod).StatusCode; got != tc.want {
				t.Fatalf("status %d, want %d", got, tc.want)
			}
		})
	}
	c := NewClient(discovery.Lockfile{Port: port, Token: "nope"})
	if _, err := c.EventsStream(context.Background(), nil, NoReplay); err == nil {
		t.Fatal("EventsStream with a wrong token succeeded")
	}
}
