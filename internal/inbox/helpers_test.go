package inbox

import (
	"context"
	"errors"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

var bg = context.Background()

// Agent keys used across tests.
const (
	keyA = "dev1/aaaaaaaa-1111-4111-8111-111111111111"
	keyB = "dev1/bbbbbbbb-2222-4222-8222-222222222222"
	keyC = "dev1/cccccccc-3333-4333-8333-333333333333"
	keyG = "dev1/grok-gggggggg"
	user = "user:dev1"
)

type fakeRoster struct {
	mu    sync.Mutex
	cards map[string]presence.AgentCard
}

func (r *fakeRoster) Agents() []presence.AgentCard {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]presence.AgentCard, 0, len(r.cards))
	for _, c := range r.cards {
		out = append(out, c)
	}
	return out
}

func (r *fakeRoster) put(cs ...presence.AgentCard) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, c := range cs {
		r.cards[c.Key] = c
	}
}

func (r *fakeRoster) remove(keys ...string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, k := range keys {
		delete(r.cards, k)
	}
}

type fakeClock struct {
	mu sync.Mutex
	t  time.Time
}

func (c *fakeClock) now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.t
}

func (c *fakeClock) advance(d time.Duration) {
	c.mu.Lock()
	c.t = c.t.Add(d)
	c.mu.Unlock()
}

type harness struct {
	t   *testing.T
	dir string
	st  *store.Store
	ros *fakeRoster
	clk *fakeClock
	ib  *Inbox
	// owners is Options.PROwners for the next open (nil = no ledger).
	owners func(prKey string) []string
	// isOwner is Options.IsOwner for the next open (nil = exactly user).
	isOwner func(key string) bool
	// onGone is Options.OnRecipientGone for the next open (nil = none).
	onGone func(ctx context.Context, m Message, to string) (string, error)
	// onPassed is Options.OnPassed for the next open (nil = none).
	onPassed func(ctx context.Context, m Message, to string) error
	// revoked is Options.Revoked for the next open (nil = none).
	revoked func(device string) bool
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	h := &harness{
		t: t, dir: t.TempDir(),
		ros: &fakeRoster{cards: map[string]presence.AgentCard{}},
		clk: &fakeClock{t: time.Date(2026, 10, 26, 9, 0, 0, 0, time.UTC)},
	}
	h.open()
	return h
}

func (h *harness) open() {
	h.t.Helper()
	st, err := store.Open(bg, filepath.Join(h.dir, "irori.db"), "dev1", hlc.New(h.clk.now))
	if err != nil {
		h.t.Fatal(err)
	}
	h.t.Cleanup(func() { st.Close() })
	h.st = st
	ib, err := New(bg, st, h.ros, Options{DeviceID: "dev1", Now: h.clk.now, PROwners: h.owners, IsOwner: h.isOwner, Revoked: h.revoked, OnRecipientGone: h.onGone, OnPassed: h.onPassed})
	if err != nil {
		h.t.Fatal(err)
	}
	h.ib = ib
}

// reopen closes the store and replays it into a fresh inbox.
func (h *harness) reopen() {
	h.t.Helper()
	if err := h.st.Close(); err != nil {
		h.t.Fatal(err)
	}
	h.open()
}

func card(key, name, repo, mode string) presence.AgentCard {
	return presence.AgentCard{
		Key: key, DeviceID: "dev1", SessionID: sessionPart(key), Agent: "claude",
		Name: name, Status: "idle", RepoKey: repo, PermissionMode: mode,
	}
}

func grokCard(key, name, repo string) presence.AgentCard {
	c := card(key, name, repo, "")
	c.Agent = "grok"
	return c
}

func claude(key, name string) Sender { return Sender{Key: key, Name: name, Kind: "claude"} }
func grok(key, name string) Sender   { return Sender{Key: key, Name: name, Kind: "grok"} }
func person() Sender                 { return Sender{Key: user, Name: "Stephen", Kind: "user"} }

func to(v string) Address     { return Address{Kind: ToAgent, Value: v} }
func toRepo(v string) Address { return Address{Kind: ToRepo, Value: v} }

// abc puts three bypass-mode Claude sessions a, b, c in the roster.
func (h *harness) abc() {
	h.ros.put(card(keyA, "alpha", "repo1", BypassMode), card(keyB, "beta", "repo1", BypassMode), card(keyC, "gamma", "repo2", BypassMode))
}

func (h *harness) send(from Sender, addr Address, text string) (Message, Delivery) {
	h.t.Helper()
	m, d, err := h.ib.Send(bg, from, addr, text, "")
	if err != nil {
		h.t.Fatalf("send %q: %v", text, err)
	}
	return m, d
}

func (h *harness) state(id string) Delivery {
	h.t.Helper()
	d, ok := h.ib.Get(id)
	if !ok {
		h.t.Fatalf("no delivery %s", id)
	}
	return d
}

func (h *harness) wantState(id string, want State) Delivery {
	h.t.Helper()
	d := h.state(id)
	if d.State != want {
		h.t.Fatalf("delivery %s is %s (hold %q), want %s", id, d.State, d.Hold, want)
	}
	return d
}

func (h *harness) wake(agent string) *Wake {
	h.t.Helper()
	w, err := h.ib.NextWake(bg, agent, PathMod)
	if err != nil {
		h.t.Fatal(err)
	}
	if w == nil {
		h.t.Fatalf("no wake for %s", agent)
	}
	return w
}

func (h *harness) noWake(agent string) {
	h.t.Helper()
	w, err := h.ib.NextWake(bg, agent, PathMod)
	if err != nil {
		h.t.Fatal(err)
	}
	if w != nil {
		h.t.Fatalf("unexpected wake for %s: %v", agent, w.DeliveryIDs)
	}
}

// deliverEngage takes agent's next wake through delivered and engaged.
func (h *harness) deliverEngage(agent, turn string) *Wake {
	h.t.Helper()
	w := h.wake(agent)
	if err := h.ib.MarkDelivered(bg, agent, w.Token); err != nil {
		h.t.Fatal(err)
	}
	if err := h.ib.MarkEngaged(bg, agent, w.Token, turn); err != nil {
		h.t.Fatal(err)
	}
	return w
}

func (h *harness) sweep() {
	h.t.Helper()
	if err := h.ib.Sweep(bg); err != nil {
		h.t.Fatal(err)
	}
}

func wantErr(t *testing.T, err, want error) {
	t.Helper()
	if !errors.Is(err, want) {
		t.Fatalf("err = %v, want %v", err, want)
	}
}

func contextWithCancel(t *testing.T) (context.Context, context.CancelFunc) {
	return context.WithCancel(t.Context())
}

// drain reads every change currently buffered.
func drain(ch <-chan Change) []Change {
	var out []Change
	for {
		select {
		case c := <-ch:
			out = append(out, c)
		case <-time.After(20 * time.Millisecond):
			return out
		}
	}
}

func count(cs []Change, kind ChangeKind, to string) int {
	n := 0
	for _, c := range cs {
		if c.Kind == kind && c.To == to {
			n++
		}
	}
	return n
}
