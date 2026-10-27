package main

import (
	"bytes"
	"context"
	"errors"
	"net"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
	"github.com/StephenSHorton/irori/internal/tools"
)

const followToken = "follow-test-token"

// followBackend serves a real event log as device "dev1". afterList runs
// after GET /v1/events listed its rows; wrapSub may rewrite a subscription.
type followBackend struct {
	st        *store.Store
	afterList func(store.Filter)
	wrapSub   func(<-chan store.Event) <-chan store.Event
}

func (b *followBackend) Version() string { return "0.0.0-test" }
func (b *followBackend) Device() device.Identity {
	return device.Identity{ID: "dev1", Name: "box", OS: "testos"}
}
func (b *followBackend) Agents() []presence.AgentCard             { return nil }
func (b *followBackend) Inbox() *inbox.Inbox                      { return nil }
func (b *followBackend) Coord() *coord.Coord                      { return nil }
func (b *followBackend) Tools() *tools.Set                        { return nil }
func (b *followBackend) Capture() *capture.Capture                { return nil }
func (b *followBackend) MemberStream(string, bool)                {}
func (b *followBackend) MemberBySecret(string) (api.Member, bool) { return api.Member{}, false }
func (b *followBackend) JoinMember(context.Context, string, string) (api.Member, error) {
	return api.Member{}, errors.New("no members")
}

func (b *followBackend) Events(ctx context.Context, f store.Filter) ([]store.Event, error) {
	evs, err := b.st.List(ctx, f)
	if err == nil && b.afterList != nil {
		b.afterList(f)
	}
	return evs, err
}

func (b *followBackend) SubscribeEvents(ctx context.Context) <-chan store.Event {
	ch := b.st.Subscribe(ctx)
	if b.wrapSub != nil {
		return b.wrapSub(ch)
	}
	return ch
}

// startFollow opens a store for b, serves b on loopback and returns a
// client for it.
func startFollow(t *testing.T, b *followBackend) *api.Client {
	t.Helper()
	st, err := store.Open(context.Background(), filepath.Join(t.TempDir(), "events.db"), "dev1", hlc.New(time.Now))
	if err != nil {
		t.Fatal(err)
	}
	b.st = st
	ln, err := api.Listen()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- api.New(b, followToken).Serve(ctx, ln) }()
	t.Cleanup(func() {
		cancel()
		<-done
		st.Close()
	})
	return api.NewClient(discovery.Lockfile{Port: ln.Addr().(*net.TCPAddr).Port, Token: followToken})
}

// syncBuf is a buffer safe to read while followEvents writes it.
type syncBuf struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (s *syncBuf) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.Write(p)
}

func (s *syncBuf) String() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.String()
}

// waitFor waits until out contains want, failing after 3 s.
func waitFor(t *testing.T, out *syncBuf, want string) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for !strings.Contains(out.String(), want) {
		if time.Now().After(deadline) {
			t.Fatalf("output never contained %q:\n%s", want, out.String())
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// runFollow runs fn until the test ends and checks it returns on cancel.
func runFollow(t *testing.T, fn func(ctx context.Context) error) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- fn(ctx) }()
	t.Cleanup(func() {
		cancel()
		select {
		case err := <-done:
			if err != nil {
				t.Errorf("followEvents: %v", err)
			}
		case <-time.After(5 * time.Second):
			t.Error("followEvents did not return on cancel")
		}
	})
}

func appendName(t *testing.T, st *store.Store, kind, name string) {
	t.Helper()
	if _, err := st.Append(context.Background(), "irori", kind, map[string]string{"name": name}); err != nil {
		t.Error(err)
	}
}

// A resync before any own row was printed resumes from this device's top
// seq, so a dropped own row is replayed rather than lost.
func TestFollowResyncBeforeOwnRow(t *testing.T) {
	b := &followBackend{}
	var once sync.Once
	b.wrapSub = func(in <-chan store.Event) <-chan store.Event {
		first := false
		once.Do(func() { first = true })
		if !first {
			return in
		}
		// The first subscription drops its first row and signals a resync,
		// as the store does on overflow.
		out := make(chan store.Event)
		go func() {
			defer close(out)
			if _, ok := <-in; !ok {
				return
			}
			out <- store.Event{}
			for range in {
			}
		}()
		return out
	}
	c := startFollow(t, b)
	appendName(t, b.st, "agent.status", "BEFORE")
	out := &syncBuf{}
	p := &eventPrinter{w: out, json: true, self: "dev1"}
	runFollow(t, func(ctx context.Context) error {
		return followEvents(ctx, "", c, []string{"agent.status"}, api.NoReplay, 0, p)
	})
	time.Sleep(200 * time.Millisecond) // let the stream open
	appendName(t, b.st, "agent.status", "DROPPED")
	waitFor(t, out, `{"resync":true}`)
	appendName(t, b.st, "agent.status", "AFTER")
	waitFor(t, out, "AFTER")
	if s := out.String(); !strings.Contains(s, "DROPPED") || strings.Contains(s, "BEFORE") {
		t.Fatalf("want DROPPED replayed and BEFORE not:\n%s", s)
	}
}

// Rows written while the follow starts (after the top-seq lookup or the
// recent-rows list, before the stream opens), own or replicated, are
// printed, and exactly once.
func TestFollowStartupGap(t *testing.T) {
	remote := func(t *testing.T, st *store.Store) {
		ev := store.Event{DeviceID: "dev2", Seq: 1, HLC: hlc.New(time.Now).Now(), Actor: "irori", Kind: "agent.status", Payload: []byte(`{"name":"GAP"}`)}
		if err := st.Insert(context.Background(), ev); err != nil {
			t.Error(err)
		}
	}
	own := func(t *testing.T, st *store.Store) { appendName(t, st, "agent.status", "GAP") }
	for _, tc := range []struct {
		name   string
		limit  int
		onTop  bool // write after the top-seq lookup, else after the recent list
		write  func(*testing.T, *store.Store)
		oldRow bool
	}{
		{"own row after list, empty log", 5, false, own, false},
		{"own row after list, only other kinds", 5, false, own, true},
		{"own row after top, no recent rows", 0, true, own, false},
		{"own row after top, listed and replayed", 5, true, own, true},
		{"remote row after list", 5, false, remote, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b := &followBackend{}
			c := startFollow(t, b)
			if tc.oldRow {
				appendName(t, b.st, "device.started", "OLD")
			}
			var once sync.Once
			b.afterList = func(f store.Filter) {
				if (f.DeviceID != "") == tc.onTop {
					once.Do(func() { tc.write(t, b.st) })
				}
			}
			out := &syncBuf{}
			p := &eventPrinter{w: out, json: true, self: "dev1"}
			runFollow(t, func(ctx context.Context) error {
				return followEvents(ctx, "", c, []string{"agent.status"}, api.NoReplay, tc.limit, p)
			})
			waitFor(t, out, "GAP")
			time.Sleep(100 * time.Millisecond)
			appendName(t, b.st, "agent.status", "AFTER")
			waitFor(t, out, "AFTER")
			if n := strings.Count(out.String(), "GAP"); n != 1 {
				t.Fatalf("GAP printed %d times:\n%s", n, out.String())
			}
		})
	}
}

// An own row too big for one stream line is reported once and skipped; the
// follow moves past it instead of replaying it on every reconnect.
func TestFollowOversizedOwnRow(t *testing.T) {
	b := &followBackend{}
	c := startFollow(t, b)
	if _, err := b.st.Append(context.Background(), "irori", "agent.status", map[string]string{"name": strings.Repeat("x", 9<<20)}); err != nil {
		t.Fatal(err)
	}
	out := &syncBuf{}
	p := &eventPrinter{w: out, json: true, self: "dev1"}
	runFollow(t, func(ctx context.Context) error {
		return followEvents(ctx, "", c, nil, 0, 0, p)
	})
	waitFor(t, out, `{"resync":true}`)
	appendName(t, b.st, "agent.status", "AFTER")
	waitFor(t, out, "AFTER")
	time.Sleep(300 * time.Millisecond)
	if n := strings.Count(out.String(), `{"resync":true}`); n != 1 {
		t.Fatalf("%d resyncs, want 1:\n%.400s", n, out.String())
	}
}
