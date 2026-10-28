package core

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/pair"
	"github.com/StephenSHorton/irori/internal/store"
)

// The multi-Core harness (v0.4 §10): whole Cores (PC, Mac, node) in one
// process, each with its own data dir, store, keys and fake Claude dir,
// paired through their own Pairers and joined by FakeNet. Nothing opens a
// socket beyond each Core's 127.0.0.1 API listener.

type hcore struct {
	t       *testing.T
	name    string
	data    string
	claude  string
	net     *mesh.FakeNet
	c       *Core
	session string // the one Claude session this device hosts, if any
	dev     string // its device id, kept while it is stopped
}

var fastNet = mesh.Intervals{Presence: 40 * time.Millisecond, Ping: 60 * time.Millisecond, Vector: 80 * time.Millisecond,
	BackoffMin: 20 * time.Millisecond, BackoffMax: 200 * time.Millisecond}

func newHCore(t *testing.T, net *mesh.FakeNet, name, session string) *hcore {
	t.Helper()
	h := &hcore{t: t, name: name, data: t.TempDir(), claude: t.TempDir(), net: net, session: session}
	if session != "" {
		dir := filepath.Join(h.claude, "sessions")
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		writeSession(t, dir, "900001.json", session, name+"-agent", t.TempDir())
	}
	h.start()
	t.Cleanup(h.stop)
	return h
}

func (h *hcore) start() {
	h.t.Helper()
	c, err := Start(context.Background(), Options{DataDir: h.data, ClaudeDir: h.claude, Version: "0.4.0-test", Headless: true,
		Keystore: identity.FileKeystore{}, Net: h.net, NetIntervals: fastNet})
	if err != nil {
		h.t.Fatal(err)
	}
	h.c, h.dev = c, c.Device().ID
}

func (h *hcore) stop() {
	if h.c != nil {
		h.c.Close()
		h.c = nil
	}
}

func (h *hcore) id() string    { return h.dev }
func (h *hcore) agent() string { return h.id() + "/" + h.session }
func (h *hcore) user() string  { return "user:" + h.id() }

func (h *hcore) sender() inbox.Sender {
	return inbox.Sender{Key: h.agent(), Name: h.name + "-agent", Kind: "claude"}
}

func wait(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(15 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
}

func stays(t *testing.T, what string, bad func() bool) {
	t.Helper()
	deadline := time.Now().Add(400 * time.Millisecond)
	for time.Now().Before(deadline) {
		if bad() {
			t.Fatalf("happened, and must not: %s", what)
		}
		time.Sleep(15 * time.Millisecond)
	}
}

// pairCores pairs b to a's person through the two Cores' Pairers.
func pairCores(t *testing.T, a, b *hcore, role string) {
	t.Helper()
	st, err := a.c.Pairing().Add(role)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := b.c.Pairing().Join(st.Code); err != nil {
		t.Fatal(err)
	}
	wait(t, b.name+" shows the words", func() bool { return b.c.Pairing().Status().State == pair.StateWords })
	wait(t, a.name+" asks for them", func() bool { return a.c.Pairing().Status().State == pair.StateConfirm })
	if err := a.c.Pairing().Confirm(context.Background(), strings.Join(b.c.Pairing().Status().Words, " ")); err != nil {
		t.Fatal(err)
	}
	wait(t, "paired", func() bool {
		return a.c.Pairing().Status().State == pair.StateDone && b.c.Pairing().Status().State == pair.StateDone
	})
}

func vec(h *hcore) store.Vector {
	v, err := h.c.store.Vector(context.Background())
	if err != nil {
		h.t.Fatal(err)
	}
	return v
}

func synced(hs ...*hcore) func() bool {
	return func() bool {
		v := vec(hs[0])
		for _, h := range hs[1:] {
			w := vec(h)
			if len(w) != len(v) {
				return false
			}
			for k, n := range v {
				if w[k] != n {
					return false
				}
			}
		}
		return true
	}
}

// meshTrio is PC (the root, session P), Mac (session M) and an always-on
// node, the Mac and the node paired from the PC, all synced.
func meshTrio(t *testing.T) (net *mesh.FakeNet, pc, mac, nd *hcore) {
	t.Helper()
	net = mesh.NewFakeNet()
	pc = newHCore(t, net, "pc", "aaaaaaaa-1111-4111-8111-00000000000a")
	mac = newHCore(t, net, "mac", "bbbbbbbb-2222-4222-8222-00000000000b")
	nd = newHCore(t, net, "node", "")
	pairCores(t, pc, mac, identity.RoleDesktop)
	pairCores(t, pc, nd, identity.RoleNode)
	wait(t, "the three converge", synced(pc, mac, nd))
	wait(t, "the pc sees the mac's session", func() bool { return hasCard(pc, mac.agent(), "connected") })
	return net, pc, mac, nd
}

func hasCard(h *hcore, key, state string) bool {
	for _, c := range h.c.MeshAgents() {
		if c.Key == key && (state == "" || c.DeviceState == state) {
			return true
		}
	}
	return false
}

func deliveryState(h *hcore, id string) inbox.State {
	d, ok := h.c.Inbox().Get(id)
	if !ok {
		return ""
	}
	return d.State
}

func TestDevicesPairThroughTheCoreAndShareOneRoster(t *testing.T) {
	_, pc, mac, nd := meshTrio(t)
	for _, h := range []*hcore{mac, nd} {
		if h.c.Person().ID != pc.c.Person().ID {
			t.Fatalf("%s has another person", h.name)
		}
		if !pc.c.IsOwner(h.user()) || !h.c.IsOwner(pc.user()) {
			t.Fatalf("%s's owner key isn't the person's everywhere", h.name)
		}
	}
	if !hasCard(mac, pc.agent(), "connected") || !hasCard(nd, mac.agent(), "") {
		t.Fatal("a device doesn't see another's session")
	}
	for _, c := range pc.c.LocalAgents() {
		if c.Key == mac.agent() {
			t.Fatal("LocalAgents lists another device's session")
		}
	}
	// Card rows carry no transcript path, pid or last text (§2).
	evs, _ := pc.c.store.List(context.Background(), store.Filter{Kinds: []string{store.KindAgentSeen}, DeviceID: mac.id()})
	if len(evs) == 0 {
		t.Fatal("no card row from the mac")
	}
	for _, ev := range evs {
		s := string(ev.Payload)
		if strings.Contains(s, `"transcript":"`+"C:") || strings.Contains(s, `"pid":`+"1") {
			t.Fatalf("a card row carries a local path or pid: %s", s)
		}
	}

	// The Mac restarts without its session: its new device.started retires
	// the card it showed, though it never wrote agent.gone for it.
	mac.stop()
	_ = os.RemoveAll(filepath.Join(mac.claude, "sessions"))
	wait(t, "the mac's card shows it offline or recent", func() bool { return !hasCard(pc, mac.agent(), "connected") && hasCard(pc, mac.agent(), "") })
	mac.start()
	wait(t, "the restart retires the mac's old card", func() bool { return !hasCard(pc, mac.agent(), "") })
}

// A message from the PC to a Mac session wakes only on the Mac, and
// "delivered" comes back to the PC and the node.
func TestAMessageWakesOnlyOnItsHost(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, nd := meshTrio(t)
	_, d, err := pc.c.Inbox().SendWith(ctx, pc.sender(), inbox.Address{Kind: inbox.ToAgent, Value: mac.agent()}, "hello from the pc", inbox.SendOptions{Mode: inbox.ModeWake})
	if err != nil {
		t.Fatal(err)
	}
	if d.State != inbox.Queued || d.To != mac.agent() {
		t.Fatalf("sent %+v", d)
	}
	if w, err := pc.c.Inbox().NextWake(ctx, mac.agent(), inbox.PathMod); err != nil || w != nil {
		t.Fatalf("the pc planned a wake for the mac's session: %+v %v", w, err)
	}
	wait(t, "the mac has the delivery", func() bool { return deliveryState(mac, d.ID) == inbox.Queued })
	w, err := mac.c.Inbox().NextWake(ctx, mac.agent(), inbox.PathMod)
	if err != nil || w == nil || !strings.Contains(w.Prompt, "hello from the pc") {
		t.Fatalf("the mac's wake: %+v %v", w, err)
	}
	if err := mac.c.Inbox().MarkDelivered(ctx, mac.agent(), w.Token); err != nil {
		t.Fatal(err)
	}
	for _, h := range []*hcore{pc, nd} {
		wait(t, "delivered on "+h.name, func() bool { return deliveryState(h, d.ID) == inbox.Delivered })
	}
	if n := pc.c.Inbox().IgnoredRows() + mac.c.Inbox().IgnoredRows() + nd.c.Inbox().IgnoredRows(); n != 0 {
		t.Fatalf("%d rows ignored as written by the wrong device", n)
	}
}

// The Mac's Pause all and block list hold a message sent from the PC: the
// host decides holds with its own policy (§6.3).
func TestTheHostsPolicyHoldsAMessageFromAnotherDevice(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, _ := meshTrio(t)
	if err := mac.c.Inbox().SetPolicy(ctx, inbox.Policy{Paused: true}, mac.user()); err != nil {
		t.Fatal(err)
	}
	if pc.c.Inbox().Policy().Paused {
		t.Fatal("pause all crossed devices")
	}
	send := func(text string) inbox.Delivery {
		_, d, err := pc.c.Inbox().SendWith(ctx, pc.sender(), inbox.Address{Kind: inbox.ToAgent, Value: mac.agent()}, text, inbox.SendOptions{Mode: inbox.ModeWake})
		if err != nil {
			t.Fatal(err)
		}
		return d
	}
	d := send("paused?")
	wait(t, "the mac holds it and the pc sees the hold", func() bool {
		x, ok := pc.c.Inbox().Get(d.ID)
		return ok && x.State == inbox.Held && x.Hold == inbox.HoldPaused
	})
	if err := mac.c.Inbox().SetPolicy(ctx, inbox.Policy{Block: []string{pc.agent()}}, mac.user()); err != nil {
		t.Fatal(err)
	}
	d2 := send("blocked?")
	wait(t, "the mac's block list holds it", func() bool {
		x, ok := pc.c.Inbox().Get(d2.ID)
		return ok && x.State == inbox.Held && x.Hold == inbox.HoldBlocked
	})
}

// Release clicked on the PC for a delivery the Mac holds takes effect
// through the Mac's row; two Releases that crossed give one result.
func TestReleaseFromAnotherDeviceGoesThroughTheOwner(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	if err := mac.c.Inbox().SetPolicy(ctx, inbox.Policy{Block: []string{pc.agent()}}, mac.user()); err != nil {
		t.Fatal(err)
	}
	_, d, err := pc.c.Inbox().SendWith(ctx, pc.sender(), inbox.Address{Kind: inbox.ToAgent, Value: mac.agent()}, "let me in", inbox.SendOptions{Mode: inbox.ModeWake})
	if err != nil {
		t.Fatal(err)
	}
	for _, h := range []*hcore{pc, nd} {
		wait(t, "held on "+h.name, func() bool { return deliveryState(h, d.ID) == inbox.Held })
	}
	// The Mac goes away; the PC and the node both click Release.
	net.Isolate(mac.id())
	if err := pc.c.Inbox().Release(ctx, d.ID, pc.user()); err != nil {
		t.Fatal(err)
	}
	if err := nd.c.Inbox().Release(ctx, d.ID, nd.user()); err != nil {
		t.Fatal(err)
	}
	if deliveryState(pc, d.ID) != inbox.Held {
		t.Fatal("the pc released a delivery the mac owns by itself")
	}
	net.HealAll()
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "queued again on "+h.name, func() bool { return deliveryState(h, d.ID) == inbox.Queued })
	}
	wait(t, "the three converge", synced(pc, mac, nd))
	evs, _ := pc.c.store.List(ctx, store.Filter{Kinds: []string{inbox.KindDelivery}, DeviceID: mac.id(), Limit: 1000})
	releases := 0
	for _, ev := range evs {
		if strings.Contains(string(ev.Payload), d.ID) && strings.Contains(string(ev.Payload), `"state":"queued"`) {
			releases++
		}
	}
	if releases != 1 {
		t.Fatalf("the mac wrote %d releases for two crossed requests", releases)
	}
	// An agent can't ask: the request needs the person.
	if err := pc.c.Inbox().Release(ctx, d.ID, pc.agent()); err == nil {
		t.Fatal("an agent released a held delivery")
	}
}

// DIRECT_LIMIT counts messages sent from two devices to one pair.
func TestTheRateLimitCountsBothDevices(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, _ := meshTrio(t)
	half := inbox.DirectLimit / 2
	var last inbox.Delivery
	for i := range half {
		_, d, err := pc.c.Inbox().SendWith(ctx, pc.sender(), inbox.Address{Kind: inbox.ToAgent, Value: mac.agent()}, "ping", inbox.SendOptions{Mode: inbox.ModeNextTurn})
		if err != nil {
			t.Fatalf("pc send %d: %v", i, err)
		}
		last = d
	}
	for i := range inbox.DirectLimit - half {
		if _, _, err := mac.c.Inbox().SendWith(ctx, mac.sender(), inbox.Address{Kind: inbox.ToAgent, Value: pc.agent()}, "pong", inbox.SendOptions{Mode: inbox.ModeNextTurn}); err != nil {
			t.Fatalf("mac send %d: %v", i, err)
		}
	}
	wait(t, "the mac's sends reach the pc", synced(pc, mac))
	_ = last
	if _, _, err := pc.c.Inbox().SendWith(ctx, pc.sender(), inbox.Address{Kind: inbox.ToAgent, Value: mac.agent()}, "one more", inbox.SendOptions{Mode: inbox.ModeNextTurn}); !errors.Is(err, inbox.ErrRateLimited) {
		t.Fatalf("the pair's %dth message: %v", inbox.DirectLimit+1, err)
	}
}

// Partitioned acquires of one resource surface as a conflict; the lower
// device id's lease stands and the other holder's Core writes lease.refused.
func TestPartitionedLeasesConflictAndTheLowerDeviceStands(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	net.Isolate(mac.id())
	if _, err := pc.c.Coord().Acquire(ctx, pc.agent(), "port:8080", time.Hour, false); err != nil {
		t.Fatal(err)
	}
	if _, err := mac.c.Coord().Acquire(ctx, mac.agent(), "port:8080", time.Hour, false); err != nil {
		t.Fatal(err)
	}
	winner, loser := pc, mac
	if mac.id() < pc.id() {
		winner, loser = mac, pc
	}
	net.HealAll()
	// The loser's refusal reaches every device; then one lease stands.
	refusals := func(h *hcore) []store.Event {
		evs, _ := h.c.store.List(ctx, store.Filter{Kinds: []string{coord.KindLeaseRefused}, DeviceID: loser.id()})
		return evs
	}
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "the losing device's refusal on "+h.name, func() bool { return len(refusals(h)) > 0 })
		wait(t, "one lease stands on "+h.name, func() bool {
			ls := h.c.Coord().Leases()
			return len(ls) == 1 && ls[0].Holder == winner.agent() && ls[0].Conflict == ""
		})
	}
	wait(t, "the three converge", synced(pc, mac, nd))
	if evs := refusals(winner); len(evs) != 1 || !strings.Contains(string(evs[0].Payload), `"conflict":true`) {
		t.Fatalf("the losing device's refusal: %d rows", len(evs))
	}
}

// A steal that crosses the holder's renewal ends the lease on all three.
func TestAStealCrossingARenewalEndsTheLease(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	if _, err := mac.c.Coord().Acquire(ctx, mac.agent(), "port:9000", time.Hour, false); err != nil {
		t.Fatal(err)
	}
	wait(t, "the pc sees the mac's lease", func() bool { return len(pc.c.Coord().Leases()) == 1 })
	net.Isolate(mac.id())
	if err := pc.c.Coord().StealLease(ctx, pc.user(), "port:9000"); err != nil {
		t.Fatal(err)
	}
	if err := pc.c.Coord().StealLease(ctx, pc.agent(), "port:9000"); err == nil {
		t.Fatal("an agent wrote a steal")
	}
	if _, err := mac.c.Coord().Acquire(ctx, mac.agent(), "port:9000", time.Hour, false); err != nil {
		t.Fatal(err) // the renewal, on the other side of the partition
	}
	net.HealAll()
	wait(t, "the three converge", synced(pc, mac, nd))
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "no lease on "+h.name, func() bool { return len(h.c.Coord().Leases()) == 0 })
	}
}

// An offline device's lease freezes: it doesn't lapse on another device's
// clock, and a remote recipient's delivery isn't swept as gone.
func TestAnOfflineDevicesLeaseFreezes(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, nd := meshTrio(t)
	if _, err := mac.c.Coord().Acquire(ctx, mac.agent(), "port:7000", 300*time.Millisecond, false); err != nil {
		t.Fatal(err)
	}
	wait(t, "the pc sees the lease", func() bool { return len(pc.c.Coord().Leases()) == 1 })
	mac.stop()
	time.Sleep(600 * time.Millisecond)
	for _, h := range []*hcore{pc, nd} {
		if err := h.c.Coord().Sweep(ctx); err != nil {
			t.Fatal(err)
		}
		if ls := h.c.Coord().Leases(); len(ls) != 1 || ls[0].Holder != mac.agent() {
			t.Fatalf("%s lapsed the offline mac's lease: %+v", h.name, ls)
		}
	}
	// Back up, the mac's own Core writes the expiry and everyone ends it.
	mac.start()
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "the expiry reaches "+h.name, func() bool { return len(h.c.Coord().Leases()) == 0 })
	}
}
