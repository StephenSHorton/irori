package roomnet

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"
	"golang.org/x/crypto/nacl/secretbox"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

// These tests run real pion peers on 127.0.0.1 only: MemHub signaling (no
// broker), no STUN, loopback candidates. They fail, not skip, when the
// peers can't connect.

func TestMeshSecretIsGoRoomsSecretFromKey(t *testing.T) {
	seed := make([]byte, 32)
	seed[3] = 9
	got := mesh.MeshSecret(seed, "abc", 4)
	want := room.SecretFromKey(mesh.MeshApp, seed, mesh.MeshInfo("abc", 4))
	if got.Key != want.Key || got.Topic != want.Topic {
		t.Fatalf("mesh %s, go-room %s", got.Topic, want.Topic)
	}
}

// tapLoopback opens every envelope the hub carries on topic (the test has
// the room key) and fails on an offer or answer with a candidate that isn't
// on 127.0.0.1.
func tapLoopback(t *testing.T, hub *room.MemHub, topics func() map[string][32]byte) *int {
	t.Helper()
	var mu sync.Mutex
	seen := new(int)
	hub.Tap = func(topic string, payload []byte) {
		key, ok := topics()[topic]
		if !ok || len(payload) < 24 {
			return
		}
		var nonce [24]byte
		copy(nonce[:], payload[:24])
		plain, ok := secretbox.Open(nil, payload[24:], &nonce, &key)
		if !ok {
			return
		}
		var env room.Envelope
		if json.Unmarshal(plain, &env) != nil || (env.Type != room.MsgOffer && env.Type != room.MsgAnswer) {
			return
		}
		var body struct{ SDP string }
		_ = json.Unmarshal(env.Body, &body)
		for _, line := range strings.Split(body.SDP, "\n") {
			if !strings.HasPrefix(line, "a=candidate:") {
				continue
			}
			f := strings.Fields(line)
			if len(f) < 5 {
				continue
			}
			if ip := net.ParseIP(f[4]); ip == nil || !ip.IsLoopback() {
				t.Errorf("non-loopback candidate: %s", strings.TrimSpace(line))
			}
			mu.Lock()
			*seen++
			mu.Unlock()
		}
	}
	return seen
}

type dev struct {
	t     *testing.T
	name  string
	dir   string
	ident *identity.Identity
	chain *identity.Chain
	st    *store.Store
	node  *mesh.Node
	tr    *Transport
	stop  func()
}

func newDev(t *testing.T, name string, personFrom string) *dev {
	t.Helper()
	d := &dev{t: t, name: name, dir: t.TempDir()}
	if personFrom != "" {
		raw, err := os.ReadFile(filepath.Join(personFrom, identity.PersonFile))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(d.dir, identity.PersonFile), raw, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	dv, err := device.LoadOrCreate(d.dir)
	if err != nil {
		t.Fatal(err)
	}
	if d.ident, err = identity.Load(d.dir, dv, identity.FileKeystore{}); err != nil {
		t.Fatal(err)
	}
	d.chain = identity.NewChain(d.ident.Person.ID, d.ident.Person.RootDeviceID, dv.ID)
	if d.st, err = store.OpenWith(context.Background(), filepath.Join(d.dir, "irori.db"), dv.ID, hlc.New(nil),
		store.OpenOptions{Signer: d.ident, Verifier: d.chain}); err != nil {
		t.Fatal(err)
	}
	if err := d.ident.EnsureSelfCert(context.Background(), d.st, d.chain, identity.RoleDesktop); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if d.stop != nil {
			d.stop()
		}
		d.st.Close()
	})
	return d
}

func (d *dev) start(hub *room.MemHub) {
	d.tr = New(Options{Self: d.ident.Device.ID, Hub: hub, LoopbackOnly: true, Logf: func(f string, a ...any) { d.t.Logf(d.name+": "+f, a...) }})
	d.node = mesh.New(mesh.Options{Store: d.st, Ident: d.ident, Chain: d.chain, Net: d.tr, Version: "0.4.0-test",
		Intervals: mesh.Intervals{Presence: 200 * time.Millisecond, Ping: 200 * time.Millisecond, Vector: 300 * time.Millisecond},
		Logf:      func(f string, a ...any) { d.t.Logf(d.name+": "+f, a...) }})
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		defer close(done)
		_ = d.node.Run(ctx)
	}()
	d.stop = func() {
		cancel()
		<-done
		d.tr.Close()
		d.stop = nil
	}
}

func (d *dev) vector() store.Vector {
	v, err := d.st.Vector(context.Background())
	if err != nil {
		d.t.Fatal(err)
	}
	return v
}

func (d *dev) has(ev store.Event) bool {
	evs, err := d.st.List(context.Background(), store.Filter{DeviceID: ev.DeviceID, AfterSeq: ev.Seq - 1, Limit: 1})
	return err == nil && len(evs) == 1 && evs[0].Seq == ev.Seq
}

func eventually(t *testing.T, what string, within time.Duration, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(within)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
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

// Two Cores' nets in one process on go-room: pair, backfill and live rows
// both ways, then a revoke closes the connection.
func TestTwoNodesSyncOverGoRoomOnLoopback(t *testing.T) {
	ctx := context.Background()
	hub := room.NewMemHub()
	a := newDev(t, "pc", "")
	big := strings.Repeat("y", 8000)
	for i := range 600 { // a backfill bigger than one batch and one MiB
		if _, err := a.st.Append(ctx, "irori", "capture.turn", map[string]any{"i": i, "text": big}); err != nil {
			t.Fatal(err)
		}
	}
	b := newDev(t, "mac", a.dir)
	add, err := a.ident.CertFor(b.ident.Device.ID, hex.EncodeToString(b.ident.DevicePub()), "mac", identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.st.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	person, _ := a.st.List(ctx, store.Filter{Kinds: []string{"person."}})
	for range 2 {
		for _, ev := range person {
			_ = b.chain.Learn(ev)
		}
	}
	eb, err := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.ident.Device.ID + "/s"})
	if err != nil {
		t.Fatal(err)
	}
	topics := func() map[string][32]byte {
		out := map[string][32]byte{}
		for e := 1; e <= 2; e++ {
			if seed, ok := a.ident.PersonSeed(e); ok {
				s := mesh.MeshSecret(seed, a.ident.Person.ID, e)
				out[s.Topic] = s.Key
			}
		}
		return out
	}
	cands := tapLoopback(t, hub, topics)
	a.start(hub)
	b.start(hub)
	eventually(t, "the backfill and b's row cross", 60*time.Second, func() bool {
		return sameVector(a.vector(), b.vector()) && a.has(eb)
	})
	if *cands == 0 {
		t.Fatal("saw no ICE candidates on the hub")
	}
	ea, _ := a.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": a.ident.Device.ID + "/live"})
	eventually(t, "a live row crosses", 20*time.Second, func() bool { return b.has(ea) })
	var connected bool
	for _, p := range a.node.Status().Peers {
		connected = connected || (p.DeviceID == b.ident.Device.ID && p.State == "connected" && p.Path == "direct")
	}
	if !connected {
		t.Fatalf("a's view of b: %+v", a.node.Status().Peers)
	}
	// Revoke b: the connection closes, and b's later rows are refused.
	if err := a.node.Revoke(ctx, b.ident.Device.ID, "test"); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the revoked device drops", 20*time.Second, func() bool {
		for _, p := range a.node.Status().Peers {
			if p.DeviceID == b.ident.Device.ID {
				return p.Path == ""
			}
		}
		return false
	})
	late, _ := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.ident.Device.ID + "/late"})
	time.Sleep(time.Second)
	if a.has(late) {
		t.Fatal("a stored a row the revoked device wrote after its cut")
	}
}

// Streams on a real go-room connection move bytes both ways while the sync
// channel stays live (blobs use them, §4.3).
func TestConnStreamsOverGoRoomOnLoopback(t *testing.T) {
	hub := room.NewMemHub()
	key := make([]byte, 32)
	_, _ = rand.Read(key)
	s := mesh.MeshSecret(key, "p", 1)
	tapLoopback(t, hub, func() map[string][32]byte { return map[string][32]byte{s.Topic: s.Key} })
	type side struct {
		id   string
		key  ed25519.PrivateKey
		conn chan mesh.Conn
		room mesh.Room
	}
	mk := func(id string) *side {
		_, k, _ := ed25519.GenerateKey(rand.Reader)
		return &side{id: id, key: k, conn: make(chan mesh.Conn, 1)}
	}
	x, y := mk("aaaa"), mk("bbbb")
	pubs := map[string]ed25519.PublicKey{x.id: x.key.Public().(ed25519.PublicKey), y.id: y.key.Public().(ed25519.PublicKey)}
	for _, sd := range []*side{x, y} {
		sd := sd
		tr := New(Options{Self: sd.id, Hub: hub, LoopbackOnly: true, Logf: t.Logf})
		t.Cleanup(tr.Close)
		r, err := tr.Join(s, mesh.RoomConfig{
			Self: sd.id, Pub: pubs[sd.id], Sign: func(m []byte) []byte { return ed25519.Sign(sd.key, m) },
			Accept: func(peer string, pub ed25519.PublicKey) error {
				if !pub.Equal(pubs[peer]) {
					return mesh.ErrRefused
				}
				return nil
			},
			OnPeer: func(c mesh.Conn) { sd.conn <- c },
		})
		if err != nil {
			t.Fatal(err)
		}
		sd.room = r
		t.Cleanup(r.Close)
	}
	y.room.Connect(x.id)
	x.room.Connect(y.id)
	var cx, cy mesh.Conn
	for _, w := range []struct {
		sd *side
		c  *mesh.Conn
	}{{x, &cx}, {y, &cy}} {
		select {
		case *w.c = <-w.sd.conn:
		case <-time.After(30 * time.Second):
			t.Fatalf("%s: no connection on loopback", w.sd.id)
		}
	}
	payload := bytes.Repeat([]byte("blob"), 64<<10) // 256 KiB
	cy.OnStream(func(label string, st io.ReadWriteCloser) {
		defer st.Close()
		got, _ := io.ReadAll(st)
		if label != "blob/1" || !bytes.Equal(got, payload) {
			t.Errorf("stream %s: %d bytes", label, len(got))
		}
	})
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	st, err := cx.OpenStream(ctx, "blob/1")
	if err != nil {
		t.Fatal(err)
	}
	go func() {
		_ = cx.Send(ctx, []byte("live row"))
	}()
	if _, err := st.Write(payload); err != nil {
		t.Fatal(err)
	}
	st.Close()
	msg, err := cy.Recv(ctx)
	if err != nil || string(msg) != "live row" {
		t.Fatalf("sync message: %q %v", msg, err)
	}
	if l, r := cx.Fingerprints(); l == "" || r == "" {
		t.Fatal("no DTLS fingerprints")
	}
	time.Sleep(200 * time.Millisecond)
}
