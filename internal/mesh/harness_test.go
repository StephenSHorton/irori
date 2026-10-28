package mesh

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/store"
)

// The multi-Core harness (§10): devices in one process, each with a real
// store, real keys and a cert chain, joined by FakeNet. Nothing opens a
// socket.

var fastIntervals = Intervals{
	Presence:   40 * time.Millisecond,
	Ping:       60 * time.Millisecond,
	Vector:     80 * time.Millisecond,
	BackoffMin: 20 * time.Millisecond,
	BackoffMax: 200 * time.Millisecond,
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

type dev struct {
	t     *testing.T
	name  string
	dir   string
	net   *FakeNet
	clk   *clock
	ident *identity.Identity
	chain *identity.Chain
	st    *store.Store
	node  *Node
	trust bool

	cancel context.CancelFunc
	done   chan struct{}
}

// newDev makes a device with its own first-run person: the root of a new
// mesh, or a device that joins one later (join).
func newDev(t *testing.T, net *FakeNet, name string) *dev {
	t.Helper()
	d := &dev{t: t, name: name, dir: t.TempDir(), net: net, clk: &clock{}}
	d.open()
	t.Cleanup(d.close)
	return d
}

func (d *dev) open() {
	t := d.t
	t.Helper()
	ctx := context.Background()
	dv, err := device.LoadOrCreate(d.dir)
	if err != nil {
		t.Fatal(err)
	}
	dv.Name = d.name
	if err := device.Save(d.dir, dv); err != nil {
		t.Fatal(err)
	}
	if d.ident, err = identity.Load(d.dir, dv, identity.FileKeystore{}); err != nil {
		t.Fatal(err)
	}
	d.chain = identity.NewChain(d.ident.Person.ID, d.ident.Person.RootDeviceID, dv.ID)
	if d.st, err = store.OpenWith(ctx, filepath.Join(d.dir, "irori.db"), dv.ID, hlc.New(d.clk.Now),
		store.OpenOptions{Signer: d.ident, Verifier: d.chain, Now: d.clk.Now}); err != nil {
		t.Fatal(err)
	}
	if err := d.ident.EnsureSelfCert(ctx, d.st, d.chain, identity.RoleDesktop); err != nil {
		t.Fatal(err)
	}
}

func (d *dev) id() string { return d.ident.Device.ID }

// start runs the node, as the Core does: it waits for a second device.
func (d *dev) start() {
	d.t.Helper()
	if d.done != nil {
		d.t.Fatalf("%s already started", d.name)
	}
	d.node = New(Options{Store: d.st, Ident: d.ident, Chain: d.chain, Net: d.net, Version: "0.4.0-test",
		Intervals: fastIntervals, TrustAnyPeer: d.trust, Now: d.clk.Now, Logf: func(f string, a ...any) { d.t.Logf(d.name+": "+f, a...) }})
	ctx, cancel := context.WithCancel(context.Background())
	d.cancel, d.done = cancel, make(chan struct{})
	go func() {
		defer close(d.done)
		_ = d.node.Run(ctx)
	}()
}

// stop ends the node (the device goes offline); the store stays open.
func (d *dev) stop() {
	if d.done == nil {
		return
	}
	d.cancel()
	<-d.done
	d.done = nil
}

func (d *dev) close() {
	d.stop()
	if d.st != nil {
		d.st.Close()
		d.st = nil
	}
}

// reopen closes and reopens the store and identity (a restart).
func (d *dev) reopen() {
	d.close()
	d.open()
}

// join makes d one of root's devices, as pairing does (§3 step 6): d takes
// root's person.json, root writes d's cert, and d learns root's person rows.
func (d *dev) join(root *dev) {
	t := d.t
	t.Helper()
	d.close()
	raw, err := os.ReadFile(filepath.Join(root.dir, identity.PersonFile))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(d.dir, identity.PersonFile), raw, 0o600); err != nil {
		t.Fatal(err)
	}
	d.open()
	add, err := root.ident.CertFor(d.id(), hex.EncodeToString(d.ident.DevicePub()), d.name, identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := root.st.Append(context.Background(), "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	d.learn(root)
}

// learn folds every person row src holds into d's chain (the pairing
// exchange hands them over).
func (d *dev) learn(src *dev) {
	evs, err := src.st.List(context.Background(), store.Filter{Kinds: []string{"person."}, Limit: 1000})
	if err != nil {
		d.t.Fatal(err)
	}
	for range len(evs) {
		for _, ev := range evs {
			_ = d.chain.Learn(ev)
		}
	}
	if d.node != nil {
		d.node.Kick()
	}
}

func (d *dev) append(kind string, payload any) store.Event {
	d.t.Helper()
	ev, err := d.st.Append(context.Background(), "irori", kind, payload)
	if err != nil {
		d.t.Fatalf("%s append %s: %v", d.name, kind, err)
	}
	return ev
}

func (d *dev) vector() store.Vector {
	d.t.Helper()
	v, err := d.st.Vector(context.Background())
	if err != nil {
		d.t.Fatal(err)
	}
	return v
}

func (d *dev) has(ev store.Event) bool {
	evs, err := d.st.List(context.Background(), store.Filter{DeviceID: ev.DeviceID, AfterSeq: ev.Seq - 1, Limit: 1})
	return err == nil && len(evs) == 1 && evs[0].Seq == ev.Seq && store.RowHash(evs[0]) == store.RowHash(ev)
}

func (d *dev) peer(id string) PeerState {
	for _, p := range d.node.Status().Peers {
		if p.DeviceID == id {
			return p
		}
	}
	return PeerState{}
}

func eventually(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
}

// never checks that ok stays false for a while.
func never(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(400 * time.Millisecond)
	for time.Now().Before(deadline) {
		if ok() {
			t.Fatalf("happened, and must not: %s", what)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func sameVector(a, b store.Vector) bool {
	if len(a) != len(b) {
		return false
	}
	for k, v := range a {
		if b[k] != v {
			return false
		}
	}
	return true
}

func converged(devs ...*dev) func() bool {
	return func() bool {
		v := devs[0].vector()
		for _, d := range devs[1:] {
			if !sameVector(v, d.vector()) {
				return false
			}
		}
		return true
	}
}

// trio is PC (the root), Mac and node, paired and synced.
func trio(t *testing.T, net *FakeNet) (pc, mac, nd *dev) {
	t.Helper()
	pc, mac, nd = newDev(t, net, "pc"), newDev(t, net, "mac"), newDev(t, net, "node")
	mac.join(pc)
	nd.join(pc)
	mac.learn(pc)
	pc.start()
	mac.start()
	nd.start()
	eventually(t, "three devices converge", converged(pc, mac, nd))
	return pc, mac, nd
}

// rawPeer joins topic as an arbitrary device and hands back its
// connections: a forger with no cert, speaking frames by hand.
func rawPeer(t *testing.T, net *FakeNet, s Secret, self string) (Room, chan Conn) {
	t.Helper()
	conns := make(chan Conn, 4)
	pub, _, _ := ed25519.GenerateKey(rand.Reader)
	r, err := net.Join(s, RoomConfig{Self: self, Pub: pub, Sign: func([]byte) []byte { return nil },
		Accept: func(string, ed25519.PublicKey) error { return nil }, OnPeer: func(c Conn) { conns <- c }})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(r.Close)
	return r, conns
}

func sendFrame(t *testing.T, c Conn, f Frame) {
	t.Helper()
	b, _ := f.Encode()
	if err := c.Send(context.Background(), b); err != nil {
		t.Fatal(err)
	}
}

func recvFrame(c Conn, typ string, within time.Duration) (Frame, error) {
	ctx, cancel := context.WithTimeout(context.Background(), within)
	defer cancel()
	for {
		b, err := c.Recv(ctx)
		if err != nil {
			return Frame{}, err
		}
		f, err := DecodeFrame(b)
		if err != nil {
			return Frame{}, err
		}
		if f.Type == typ {
			return f, nil
		}
	}
}
