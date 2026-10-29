package core

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/ask"
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
// socket beyond each Core's 127.0.0.1 API listener. home_test.go runs the
// same harness over go-room: a MemLAN and the home server's real hub and
// TURN relay on 127.0.0.1.

type hcore struct {
	t       *testing.T
	name    string
	data    string
	claude  string
	net     *mesh.FakeNet
	c       *Core
	session string // the one Claude session this device hosts, if any
	dev     string // its device id, kept while it is stopped
	clk     *clock // its wall clock: real time plus an offset
	edit    func(*Options)
}

// clock is a device's wall clock: real time plus an offset tests move.
type clock struct {
	mu  sync.Mutex
	off time.Duration
}

func (c *clock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return time.Now().Add(c.off)
}

func (c *clock) Set(off time.Duration) {
	c.mu.Lock()
	c.off = off
	c.mu.Unlock()
}

var fastNet = mesh.Intervals{Presence: 40 * time.Millisecond, Ping: 60 * time.Millisecond, Vector: 80 * time.Millisecond,
	BackoffMin: 20 * time.Millisecond, BackoffMax: 200 * time.Millisecond}

func newHCore(t *testing.T, net *mesh.FakeNet, name, session string) *hcore {
	t.Helper()
	h := &hcore{t: t, name: name, data: t.TempDir(), claude: t.TempDir(), net: net, session: session, clk: &clock{}}
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
	o := Options{DataDir: h.data, ClaudeDir: h.claude, Version: "0.4.0-test", Headless: true,
		Keystore: identity.FileKeystore{}, NetIntervals: fastNet, Now: h.clk.Now}
	if h.net != nil {
		o.Net = h.net
	}
	if h.edit != nil {
		h.edit(&o) // go-room on 127.0.0.1 instead of FakeNet (home_test.go)
	}
	c, err := Start(context.Background(), o)
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
	if err := b.c.Pairing().ConfirmKey(context.Background(), st.Fingerprint); err != nil {
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
	// meshTrio waits only for the pc to see the mac; the others' folds of
	// each other's sessions can land a beat later on a loaded runner.
	wait(t, "every device sees the others' sessions", func() bool {
		return hasCard(mac, pc.agent(), "connected") && hasCard(nd, mac.agent(), "")
	})
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

// A lease that ended while partitioned (its own Core wrote the expiry)
// arrives after another device took the resource: their times don't
// overlap, so it is history, not a conflict. The live lease stands on
// every device and nobody writes a refusal, even when the expired one
// belongs to the lower device id.
func TestALateExpiredLeaseIsNotAConflict(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	lo, hi := pc, mac
	if mac.id() < pc.id() {
		lo, hi = mac, pc
	}
	net.Isolate(lo.id())
	lo.clk.Set(-5 * time.Minute) // its lease ran well before the other one
	if _, err := lo.c.Coord().Acquire(ctx, lo.agent(), "port:8181", 200*time.Millisecond, false); err != nil {
		t.Fatal(err)
	}
	wait(t, "the lower device writes its own expiry", func() bool {
		evs, _ := lo.c.store.List(ctx, store.Filter{Kinds: []string{coord.KindLeaseRel}, DeviceID: lo.id()})
		return len(evs) > 0
	})
	lo.clk.Set(0)
	if _, err := hi.c.Coord().Acquire(ctx, hi.agent(), "port:8181", time.Hour, false); err != nil {
		t.Fatal(err)
	}
	net.HealAll()
	wait(t, "the three converge", synced(pc, mac, nd))
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "the live lease stands on "+h.name, func() bool {
			ls := h.c.Coord().Leases()
			return len(ls) == 1 && ls[0].Holder == hi.agent() && ls[0].Conflict == ""
		})
	}
	if evs, _ := hi.c.store.List(ctx, store.Filter{Kinds: []string{coord.KindLeaseRefused}, DeviceID: hi.id()}); len(evs) != 0 {
		t.Fatalf("the live lease's device refused it: %d rows", len(evs))
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

// An ask posted on the Mac shows on every device; two answers that crossed
// (PC and node, with the Mac away) give the one ask.resolved names, and the
// Mac's agent gets that answer.
func TestAnAskResolvesOnceForEveryDevice(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	a, err := mac.c.Asks().Post(ctx, ask.Ask{By: mac.agent(), Name: "mac-agent", Title: "Merge?", Text: "Merge the branch now?",
		Actions: []string{"Merge", "Wait"}, Until: time.Now().Add(time.Minute)})
	if err != nil {
		t.Fatal(err)
	}
	for _, h := range []*hcore{pc, nd} {
		wait(t, "the ask shows on "+h.name, func() bool { _, ok := h.c.Asks().Get(a.ID); return ok })
	}
	net.Isolate(mac.id())
	if err := pc.c.Asks().Answer(ctx, a.ID, "Merge", pc.user()); err != nil {
		t.Fatal(err)
	}
	if err := nd.c.Asks().Answer(ctx, a.ID, "Wait", nd.user()); err != nil {
		t.Fatal(err)
	}
	if err := pc.c.Asks().Answer(ctx, a.ID, "Merge", pc.agent()); err == nil {
		t.Fatal("an agent answered the person's ask")
	}
	if got, _ := pc.c.Asks().Get(a.ID); got.Resolved {
		t.Fatal("a device other than the asker resolved the ask")
	}
	net.HealAll()
	var choice, ref string
	wait(t, "the mac resolves it", func() bool {
		got, _ := mac.c.Asks().Get(a.ID)
		choice, ref = got.Choice, got.AnswerRef
		return got.Resolved
	})
	for _, h := range []*hcore{pc, nd} {
		wait(t, "the same answer on "+h.name, func() bool {
			got, _ := h.c.Asks().Get(a.ID)
			return got.Resolved && got.Choice == choice && got.AnswerRef == ref
		})
		if len(h.c.Asks().Open()) != 0 {
			t.Fatalf("%s still shows an open ask", h.name)
		}
	}
	wait(t, "the mac's agent hears the answer", func() bool {
		for _, d := range mac.c.Inbox().For(mac.agent(), true, 50) {
			if d.Message.From.Key == inbox.SystemKey && strings.Contains(d.Message.Text, "They chose: "+choice) {
				return true
			}
		}
		return false
	})
}

// A revive asked for from the PC wakes the Mac's session only on a
// matching answer from the person: an agent's or a mismatched answer, or
// Not now, wakes nothing.
func TestACrossDeviceReviveWaitsForAMatchingAnswer(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, nd := meshTrio(t)
	revived := make(chan string, 4)
	mac.c.SetReviver(func(session string) (string, error) { revived <- session; return "", nil })
	if err := pc.c.Asks().RequestRevive(ctx, pc.agent(), "dlv_1", pc.user()); err == nil {
		t.Fatal("a revive request for a session on this device")
	}
	if err := pc.c.Asks().RequestRevive(ctx, mac.agent(), "dlv_1", pc.agent()); err == nil {
		t.Fatal("an agent asked for a revive")
	}
	reviveAsk := func(h *hcore, delivery string) (ask.Ask, bool) {
		for _, a := range h.c.Asks().Open() {
			if a.Target == mac.agent() && a.DeliveryID == delivery {
				return a, true
			}
		}
		return ask.Ask{}, false
	}
	// Not now: nothing wakes.
	if err := pc.c.Asks().RequestRevive(ctx, mac.agent(), "dlv_1", pc.user()); err != nil {
		t.Fatal(err)
	}
	var first ask.Ask
	wait(t, "the mac's revive ask reaches the pc", func() bool { var ok bool; first, ok = reviveAsk(pc, "dlv_1"); return ok })
	if first.Device != mac.id() {
		t.Fatalf("the revive ask was posted by %s, not the host", first.Device)
	}
	if err := pc.c.Asks().Answer(ctx, first.ID, ask.ChoiceNotNow, pc.user()); err != nil {
		t.Fatal(err)
	}
	wait(t, "resolved", func() bool { a, _ := mac.c.Asks().Get(first.ID); return a.Resolved })
	// A forged answer (an agent's row) and one naming another delivery.
	if err := pc.c.Asks().RequestRevive(ctx, mac.agent(), "dlv_2", pc.user()); err != nil {
		t.Fatal(err)
	}
	var second ask.Ask
	wait(t, "the second revive ask", func() bool { var ok bool; second, ok = reviveAsk(nd, "dlv_2"); return ok })
	forged := map[string]any{"askId": second.ID, "choice": ask.ChoiceRevive, "target": mac.agent(), "deliveryId": "dlv_2", "by": pc.agent()}
	if _, err := pc.c.store.Append(ctx, "agent:"+pc.agent(), ask.KindAnswered, forged); err != nil {
		t.Fatal(err)
	}
	wrong := map[string]any{"askId": second.ID, "choice": ask.ChoiceRevive, "target": mac.agent(), "deliveryId": "dlv_other", "by": pc.user()}
	if _, err := pc.c.store.Append(ctx, "user", ask.KindAnswered, wrong); err != nil {
		t.Fatal(err)
	}
	wait(t, "the second resolves", func() bool { a, _ := mac.c.Asks().Get(second.ID); return a.Resolved })
	select {
	case s := <-revived:
		t.Fatalf("revived %s without a matching answer", s)
	case <-time.After(300 * time.Millisecond):
	}
	// The real thing, answered from the node.
	if err := pc.c.Asks().RequestRevive(ctx, mac.agent(), "dlv_3", pc.user()); err != nil {
		t.Fatal(err)
	}
	var third ask.Ask
	wait(t, "the third revive ask", func() bool { var ok bool; third, ok = reviveAsk(nd, "dlv_3"); return ok })
	if err := nd.c.Asks().Answer(ctx, third.ID, ask.ChoiceRevive, nd.user()); err != nil {
		t.Fatal(err)
	}
	select {
	case s := <-revived:
		if s != mac.session {
			t.Fatalf("revived %s", s)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the matching answer didn't revive the session")
	}
}

// The Board across devices (§6.4): a task created on the Mac shows on the
// PC, and the PC's agent claiming it shows on the Mac. A next-turn item to
// a Mac session is appended only by the Mac.
func TestTheBoardAndNextTurnAcrossDevices(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, nd := meshTrio(t)
	task, err := mac.c.Coord().CreateTask(ctx, mac.agent(), coord.NewTask{Title: "Port the build", Objective: "Make the Mac build work"})
	if err != nil {
		t.Fatal(err)
	}
	wait(t, "the task shows on the pc", func() bool { _, ok := pc.c.Coord().GetTask(task.ID); return ok })
	if _, err := pc.c.Coord().ClaimTask(ctx, pc.agent(), task.ID); err != nil {
		t.Fatal(err)
	}
	for _, h := range []*hcore{mac, nd} {
		wait(t, "the pc's claim on "+h.name, func() bool { x, _ := h.c.Coord().GetTask(task.ID); return x.Holder == pc.agent() })
	}

	_, d, err := pc.c.Inbox().SendWith(ctx, pc.c.UserSender(), inbox.Address{Kind: inbox.ToAgent, Value: mac.agent()}, "when you get a moment", inbox.SendOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if w, _ := pc.c.Inbox().NextAppend(ctx, mac.agent()); w != nil {
		t.Fatal("the pc planned an append for the mac's session")
	}
	wait(t, "the mac has it", func() bool { return deliveryState(mac, d.ID) == inbox.Queued })
	w, err := mac.c.Inbox().NextAppend(ctx, mac.agent())
	if err != nil || w == nil || !strings.Contains(w.Prompt, "when you get a moment") {
		t.Fatalf("the mac's append: %+v %v", w, err)
	}
}

// The person comments from the Mac on a card the PC's agent holds (board.md
// "Comments from the app"): the Mac's roster says the PC's agent is around,
// the PC wakes it with the comment, it answers on the card, and the answer
// is in the Mac's thread.
func TestAPersonsCommentWakesTheHolderOnTheOtherDevice(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, _ := meshTrio(t)
	task, err := pc.c.Coord().CreateTask(ctx, pc.agent(), coord.NewTask{Title: "Tag the release"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := pc.c.Coord().ClaimTask(ctx, pc.agent(), task.ID); err != nil {
		t.Fatal(err)
	}
	wait(t, "the mac sees the pc's claim", func() bool { x, _ := mac.c.Coord().GetTask(task.ID); return x.Holder == pc.agent() })
	around := func(k string) bool {
		for _, a := range mac.c.MeshAgents() {
			if a.Key == k && a.Status != "gone" {
				return true
			}
		}
		return false
	}
	cm, ns, err := mac.c.Coord().PersonComment(ctx, mac.c.UserSender().Key, task.ID, "Which tag, v0.4.5?", true, around)
	if err != nil {
		t.Fatal(err)
	}
	if cm.Unheard || len(ns) != 1 || len(ns[0].To) != 1 || ns[0].To[0] != pc.agent() || !ns[0].Wake {
		t.Fatalf("comment %+v, notices %+v", cm, ns)
	}
	if err := mac.c.RouteNotices(ctx, mac.c.UserSender(), ns...); err != nil {
		t.Fatal(err)
	}
	wait(t, "the pc wakes its agent with the comment", func() bool {
		w, _ := pc.c.Inbox().NextWake(ctx, pc.agent(), inbox.PathMod)
		if w == nil {
			return false
		}
		if !strings.Contains(w.Prompt, "Which tag, v0.4.5?") || !strings.Contains(w.Prompt, "board_comment") {
			t.Fatalf("the pc's wake:\n%s", w.Prompt)
		}
		return true
	})
	if out := pc.tool("board_comment", map[string]any{"task_id": task.ID, "text": "v0.4.5, tagging now."}); strings.HasPrefix(out, "Error: ") {
		t.Fatalf("board_comment = %s", out)
	}
	wait(t, "the answer is on the mac's card", func() bool {
		for _, c := range mac.c.Coord().Comments(task.ID) {
			if c.By == pc.agent() && c.Text == "v0.4.5, tagging now." {
				return true
			}
		}
		return false
	})
}

// Removing a device ends its leases and drops its cards everywhere, with no
// row from it.
func TestRemovingADeviceEndsItsLeasesAndCards(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, nd := meshTrio(t)
	if _, err := mac.c.Coord().Acquire(ctx, mac.agent(), "port:6000", time.Hour, false); err != nil {
		t.Fatal(err)
	}
	wait(t, "the node sees the lease", func() bool { return len(nd.c.Coord().Leases()) == 1 })
	if err := pc.c.Mesh().Revoke(ctx, mac.id(), "lost"); err != nil {
		t.Fatal(err)
	}
	for _, h := range []*hcore{pc, nd} {
		wait(t, "the lease ends on "+h.name, func() bool { return len(h.c.Coord().Leases()) == 0 })
		wait(t, "the card goes on "+h.name, func() bool { return !hasCard(h, mac.agent(), "") })
	}
}

// The node folded a task and a message the Mac wrote after the PC's cut
// (the PC never heard them). When the revoke reaches the node, those rows
// move to refused_events and the node's Board and inbox drop them: the
// same state as the PC's (§5.5, §6.1).
func TestRowsPastARevokesCutAreUnfolded(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	net.Isolate(pc.id())
	tk, err := mac.c.Coord().CreateTask(ctx, mac.agent(), coord.NewTask{Title: "after the cut", Shared: true})
	if err != nil {
		t.Fatal(err)
	}
	_, d, err := mac.c.Inbox().SendWith(ctx, mac.sender(), inbox.Address{Kind: inbox.ToAgent, Value: pc.agent()}, "from past the cut", inbox.SendOptions{Mode: inbox.ModeWake})
	if err != nil {
		t.Fatal(err)
	}
	wait(t, "the node folds them", func() bool {
		_, ok := nd.c.Coord().GetTask(tk.ID)
		return ok && deliveryState(nd, d.ID) != ""
	})
	net.Isolate(mac.id())
	net.Heal(pc.id(), nd.id())
	if err := pc.c.Mesh().Revoke(ctx, mac.id(), "lost"); err != nil {
		t.Fatal(err)
	}
	wait(t, "the node drops the task", func() bool { _, ok := nd.c.Coord().GetTask(tk.ID); return !ok })
	wait(t, "the node drops the delivery", func() bool { return deliveryState(nd, d.ID) == "" })
}

// A clock 10 minutes ahead (§10): the Mac's rows wait on the other
// devices, then land once their clocks catch up, and nothing it wrote wins
// an answer: the node's later answer, which arrived first, is the one the
// asking PC resolves with, on every device.
func TestAClockAheadWinsNoAnswer(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, nd := meshTrio(t)
	a, err := pc.c.Asks().Post(ctx, ask.Ask{By: pc.agent(), Name: "pc-agent", Title: "Deploy?", Text: "Deploy now?",
		Actions: []string{"Deploy", "Hold"}, Until: time.Now().Add(time.Hour)})
	if err != nil {
		t.Fatal(err)
	}
	for _, h := range []*hcore{mac, nd} {
		wait(t, "the ask shows on "+h.name, func() bool { _, ok := h.c.Asks().Get(a.ID); return ok })
	}
	mac.clk.Set(10 * time.Minute)
	if err := mac.c.Asks().Answer(ctx, a.ID, "Deploy", mac.user()); err != nil {
		t.Fatal(err)
	}
	ahead, _ := mac.c.store.Vector(ctx)
	stays(t, "the mac's ahead answer lands on the pc", func() bool { return vec(pc)[mac.id()] >= ahead[mac.id()] })
	if err := nd.c.Asks().Answer(ctx, a.ID, "Hold", nd.user()); err != nil {
		t.Fatal(err)
	}
	wait(t, "the pc resolves with the node's answer", func() bool {
		got, _ := pc.c.Asks().Get(a.ID)
		return got.Resolved
	})
	if got, _ := pc.c.Asks().Get(a.ID); got.Choice != "Hold" {
		t.Fatalf("the pc resolved with %q", got.Choice)
	}
	// The other clocks catch up: the mac's rows land, and the answer stays.
	pc.clk.Set(10 * time.Minute)
	nd.clk.Set(10 * time.Minute)
	wait(t, "the mac's rows land", func() bool { return vec(pc)[mac.id()] >= ahead[mac.id()] && vec(nd)[mac.id()] >= ahead[mac.id()] })
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "the node's answer on "+h.name, func() bool {
			got, _ := h.c.Asks().Get(a.ID)
			return got.Resolved && got.Choice == "Hold"
		})
	}
}

// Short refs across a partition (board.md "Short refs"): the PC and the
// Mac both number a card #2 while they can't reach each other. Once they
// meet, every device settles on the same refs: the card created first is
// #2a, the other #2b, both say they were #2, "#2" alone is refused with
// both named on every device (so neither device's old #2 acts on the other
// card), and numbering carries on past both.
func TestShortRefsConvergeAfterAPartition(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	first, err := pc.c.Coord().CreateTask(ctx, pc.agent(), coord.NewTask{Title: "Before the split"})
	if err != nil {
		t.Fatal(err)
	}
	for _, h := range []*hcore{mac, nd} {
		wait(t, "#1 on "+h.name, func() bool { x, _ := h.c.Coord().GetTask(first.ID); return x.Short == "1" })
	}

	net.Isolate(mac.id())
	a, err := pc.c.Coord().CreateTask(ctx, pc.agent(), coord.NewTask{Title: "PC card"})
	if err != nil {
		t.Fatal(err)
	}
	time.Sleep(5 * time.Millisecond) // the Mac's card is created later, at a later HLC
	m, err := mac.c.Coord().CreateTask(ctx, mac.agent(), coord.NewTask{Title: "Mac card"})
	if err != nil {
		t.Fatal(err)
	}
	m2, err := mac.c.Coord().CreateTask(ctx, mac.agent(), coord.NewTask{Title: "Mac card two"})
	if err != nil {
		t.Fatal(err)
	}
	if a.Short != "2" || m.Short != "2" || m2.Short != "3" {
		t.Fatalf("while apart: pc %q, mac %q and %q", a.Short, m.Short, m2.Short)
	}

	net.HealAll()
	wait(t, "the three converge", synced(pc, mac, nd))
	want := map[string][2]string{first.ID: {"1", ""}, a.ID: {"2a", "2"}, m.ID: {"2b", "2"}, m2.ID: {"3", ""}}
	for _, h := range []*hcore{pc, mac, nd} {
		wait(t, "the refs settle on "+h.name, func() bool {
			for id, w := range want {
				x, _ := h.c.Coord().GetTask(id)
				if x.Short != w[0] || x.ShortWas != w[1] {
					return false
				}
			}
			return true
		})
		for id, w := range want {
			if got, err := h.c.Coord().TaskRef("#" + w[0]); err != nil || got != id {
				t.Fatalf("%s: #%s is %q (%v)", h.name, w[0], got, err)
			}
		}
		got, err := h.c.Coord().TaskRef("#2")
		if !errors.Is(err, coord.ErrAmbiguous) || got != "" || !strings.Contains(err.Error(), a.ID) || !strings.Contains(err.Error(), m.ID) {
			t.Fatalf("%s: #2 after the heal is %q (%v)", h.name, got, err)
		}
	}

	// Numbering carries on past every claim, on whichever device.
	next, err := mac.c.Coord().CreateTask(ctx, mac.agent(), coord.NewTask{Title: "After the heal"})
	if err != nil {
		t.Fatal(err)
	}
	if next.Short != "4" {
		t.Fatalf("the first card after the heal is #%s", next.Short)
	}
	wait(t, "#4 on the pc", func() bool { x, _ := pc.c.Coord().GetTask(next.ID); return x.Short == "4" })
}
