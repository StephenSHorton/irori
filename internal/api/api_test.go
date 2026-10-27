package api

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"os"
	"reflect"
	"strconv"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
	"github.com/StephenSHorton/irori/internal/tools"
)

const testToken = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

type fakeBackend struct {
	mu   sync.Mutex
	last store.Filter
	cap  *capture.Capture // nil: the v0.3 routes answer 503
	live chan store.Event // SubscribeEvents' channel; nil: nothing ever arrives
}

func (f *fakeBackend) SubscribeEvents(ctx context.Context) <-chan store.Event {
	if f.live != nil {
		return f.live
	}
	ch := make(chan store.Event)
	go func() { <-ctx.Done(); close(ch) }()
	return ch
}

func (f *fakeBackend) Version() string { return "0.1.0-test" }
func (f *fakeBackend) Device() device.Identity {
	return device.Identity{ID: "dev1", Name: "box", OS: "testos"}
}
func (f *fakeBackend) Agents() []presence.AgentCard {
	return []presence.AgentCard{{Key: "dev1/s1", DeviceID: "dev1", Agent: "claude", SessionID: "s1", Status: "idle"}}
}
func (f *fakeBackend) Events(_ context.Context, flt store.Filter) ([]store.Event, error) {
	f.mu.Lock()
	f.last = flt
	f.mu.Unlock()
	return []store.Event{
		{DeviceID: "dev1", Seq: 2, Kind: "agent.status", Payload: json.RawMessage(`{"key":"dev1/s1"}`)},
		{DeviceID: "dev1", Seq: 1, Kind: "agent.seen", Payload: json.RawMessage(`{}`)},
	}, nil
}

// v0.1 tests run without messaging or capture: the v0.2 routes answer 503,
// and so do the v0.3 routes unless cap is set.
func (f *fakeBackend) Inbox() *inbox.Inbox       { return nil }
func (f *fakeBackend) Coord() *coord.Coord       { return nil }
func (f *fakeBackend) Capture() *capture.Capture { return f.cap }
func (f *fakeBackend) Tools() *tools.Set         { return nil }
func (f *fakeBackend) JoinMember(context.Context, string, string) (Member, error) {
	return Member{}, errors.New("no members")
}
func (f *fakeBackend) MemberBySecret(string) (Member, bool) { return Member{}, false }
func (f *fakeBackend) MemberStream(string, bool)            {}

func (f *fakeBackend) filter() store.Filter {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.last
}

// start serves a fake backend and returns its port; the server stops with the test.
func start(t *testing.T) (*fakeBackend, int) {
	t.Helper()
	b := &fakeBackend{}
	return b, startWith(t, b)
}

// startWith serves b and returns its port.
func startWith(t *testing.T, b Backend) int {
	t.Helper()
	ln, err := Listen()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- New(b, testToken).Serve(ctx, ln) }()
	t.Cleanup(func() {
		cancel()
		select {
		case err := <-done:
			if err != nil {
				t.Errorf("serve: %v", err)
			}
		case <-time.After(5 * time.Second):
			t.Error("server did not shut down")
		}
	})
	return ln.Addr().(*net.TCPAddr).Port
}

// do sends a raw request; mod may set headers or Host.
func do(t *testing.T, port int, method, path string, mod func(*http.Request)) *http.Response {
	t.Helper()
	req, err := http.NewRequest(method, "http://127.0.0.1:"+strconv.Itoa(port)+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	if mod != nil {
		mod(req)
	}
	resp, err := (&http.Client{Transport: &http.Transport{}}).Do(req)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { resp.Body.Close() })
	return resp
}

func bearer(tok string) func(*http.Request) {
	return func(r *http.Request) { r.Header.Set("Authorization", "Bearer "+tok) }
}

func TestHealthNeedsNoToken(t *testing.T) {
	_, port := start(t)
	resp := do(t, port, "GET", "/v1/health", nil)
	if resp.StatusCode != 200 {
		t.Fatalf("status %d", resp.StatusCode)
	}
	var h Health
	if err := json.NewDecoder(resp.Body).Decode(&h); err != nil {
		t.Fatal(err)
	}
	if h != (Health{OK: true, Version: "0.1.0-test", DeviceID: "dev1"}) {
		t.Fatalf("health %+v", h)
	}
}

func TestAuthRequired(t *testing.T) {
	_, port := start(t)
	for _, path := range []string{"/v1/device", "/v1/agents", "/v1/events", "/v1/nope"} {
		if resp := do(t, port, "GET", path, nil); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s without token: status %d, want 401", path, resp.StatusCode)
		}
		if resp := do(t, port, "GET", path, bearer("wrong")); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s wrong token: status %d, want 401", path, resp.StatusCode)
		}
		if resp := do(t, port, "GET", path, func(r *http.Request) {
			r.Header.Set("Authorization", testToken) // no "Bearer "
		}); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s bare token: status %d, want 401", path, resp.StatusCode)
		}
	}
	if resp := do(t, port, "GET", "/v1/device", bearer(testToken)); resp.StatusCode != 200 {
		t.Fatalf("right token: status %d", resp.StatusCode)
	}
	if resp := do(t, port, "GET", "/v1/nope", bearer(testToken)); resp.StatusCode != 404 {
		t.Fatalf("unknown route: status %d, want 404", resp.StatusCode)
	}
}

func TestOriginRejected(t *testing.T) {
	_, port := start(t)
	for _, path := range []string{"/v1/health", "/v1/device"} {
		resp := do(t, port, "GET", path, func(r *http.Request) {
			bearer(testToken)(r)
			r.Header.Set("Origin", "http://127.0.0.1:"+strconv.Itoa(port))
		})
		if resp.StatusCode != http.StatusForbidden {
			t.Errorf("%s with Origin: status %d, want 403", path, resp.StatusCode)
		}
	}
}

func TestBadHostRejected(t *testing.T) {
	_, port := start(t)
	p := strconv.Itoa(port)
	for _, host := range []string{"evil.example:" + p, "127.0.0.1", "localhost:1", "127.0.0.2:" + p} {
		resp := do(t, port, "GET", "/v1/device", func(r *http.Request) {
			bearer(testToken)(r)
			r.Host = host
		})
		if resp.StatusCode != http.StatusForbidden {
			t.Errorf("Host %q: status %d, want 403", host, resp.StatusCode)
		}
	}
	for _, host := range []string{"127.0.0.1:" + p, "localhost:" + p, "LOCALHOST:" + p} {
		resp := do(t, port, "GET", "/v1/device", func(r *http.Request) {
			bearer(testToken)(r)
			r.Host = host
		})
		if resp.StatusCode != 200 {
			t.Errorf("Host %q: status %d, want 200", host, resp.StatusCode)
		}
	}
}

func TestGetOnly(t *testing.T) {
	_, port := start(t)
	for _, m := range []string{"POST", "PUT", "DELETE", "HEAD"} {
		resp := do(t, port, m, "/v1/device", bearer(testToken))
		if resp.StatusCode != http.StatusMethodNotAllowed {
			t.Errorf("%s: status %d, want 405", m, resp.StatusCode)
		}
	}
}

func TestEventsQuery(t *testing.T) {
	b, port := start(t)
	resp := do(t, port, "GET", "/v1/events?kind=agent.&kind=device.started&limit=5", bearer(testToken))
	if resp.StatusCode != 200 {
		t.Fatalf("status %d", resp.StatusCode)
	}
	want := store.Filter{Kinds: []string{"agent.", "device.started"}, Limit: 5, Desc: true}
	if got := b.filter(); !reflect.DeepEqual(got, want) {
		t.Fatalf("filter %+v, want %+v", got, want)
	}

	do(t, port, "GET", "/v1/events", bearer(testToken))
	if got := b.filter(); got.Limit != DefaultEventLimit || !got.Desc || got.Kinds != nil {
		t.Fatalf("default filter %+v", got)
	}
	do(t, port, "GET", "/v1/events?limit=50000", bearer(testToken))
	if got := b.filter(); got.Limit != MaxEventLimit {
		t.Fatalf("limit %d, want clamp to %d", got.Limit, MaxEventLimit)
	}
	for _, bad := range []string{"0", "-3", "x"} {
		if resp := do(t, port, "GET", "/v1/events?limit="+bad, bearer(testToken)); resp.StatusCode != 400 {
			t.Errorf("limit=%s: status %d, want 400", bad, resp.StatusCode)
		}
	}
}

func TestClientRoundTrip(t *testing.T) {
	b, port := start(t)
	dir := t.TempDir()
	if err := discovery.Write(dir, discovery.Lockfile{
		PID: os.Getpid(), Port: port, Token: testToken, Version: "0.1.0-test", DeviceID: "dev1",
	}); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	c, err := Dial(ctx, dir)
	if err != nil {
		t.Fatal(err)
	}
	id, err := c.Device(ctx)
	if err != nil || id.ID != "dev1" || id.Name != "box" {
		t.Fatalf("device %+v, %v", id, err)
	}
	cards, err := c.Agents(ctx)
	if err != nil || len(cards) != 1 || cards[0].Key != "dev1/s1" {
		t.Fatalf("agents %+v, %v", cards, err)
	}
	evs, err := c.Events(ctx, []string{"agent."}, 2)
	if err != nil || len(evs) != 2 || evs[0].Seq != 2 || string(evs[0].Payload) != `{"key":"dev1/s1"}` {
		t.Fatalf("events %+v, %v", evs, err)
	}
	if got := b.filter(); !reflect.DeepEqual(got.Kinds, []string{"agent."}) || got.Limit != 2 {
		t.Fatalf("filter %+v", got)
	}

	// A wrong token surfaces the HTTP status.
	bad := NewClient(discovery.Lockfile{Port: port, Token: "nope"})
	_, err = bad.Device(ctx)
	var se *StatusError
	if !errors.As(err, &se) || se.Code != http.StatusUnauthorized {
		t.Fatalf("err %v, want a 401 StatusError", err)
	}
}

func TestDialNotRunning(t *testing.T) {
	ctx := context.Background()
	if _, err := Dial(ctx, t.TempDir()); !errors.Is(err, discovery.ErrNotRunning) {
		t.Fatalf("no lockfile: err %v, want ErrNotRunning", err)
	}
	// A live pid but a port nobody listens on.
	ln, err := Listen()
	if err != nil {
		t.Fatal(err)
	}
	port := ln.Addr().(*net.TCPAddr).Port
	ln.Close()
	dir := t.TempDir()
	if err := discovery.Write(dir, discovery.Lockfile{PID: os.Getpid(), Port: port, Token: testToken}); err != nil {
		t.Fatal(err)
	}
	if _, err := Dial(ctx, dir); !errors.Is(err, discovery.ErrNotRunning) {
		t.Fatalf("closed port: err %v, want ErrNotRunning", err)
	}
}

func TestServeStopsOnCancel(t *testing.T) {
	ln, err := Listen()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- New(&fakeBackend{}, testToken).Serve(ctx, ln) }()
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("Serve did not return")
	}
}
