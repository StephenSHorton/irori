package mesh

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/store"
)

func TestSingleDeviceOpensNothing(t *testing.T) {
	net := NewFakeNet()
	a := newDev(t, net, "pc")
	a.append(store.KindDeviceStarted, map[string]any{"name": "pc"})
	a.start()
	never(t, "a lone device joins a topic", func() bool { return net.Joins() > 0 })
	if st := a.node.Status(); st.State != StateOff || st.Topics != 0 {
		t.Fatalf("lone device status %+v", st)
	}
}

func TestPairingStartsTheNetAndEarlierRowsReplicate(t *testing.T) {
	net := NewFakeNet()
	a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
	before := b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/s1"}) // written under b's first-run person
	a.append(store.KindDeviceStarted, map[string]any{"name": "pc"})
	a.start()
	never(t, "the root joins a topic before it has a second device", func() bool { return net.Joins() > 0 })
	b.join(a)
	b.start()
	eventually(t, "a and b converge", converged(a, b))
	if !a.has(before) {
		t.Fatal("b's row from before pairing did not replicate")
	}
	// b's first-run self-cert for its old person is stored on a and inert:
	// b is certified only by a's cert.
	cert, ok := a.chain.Cert(b.id())
	if !ok || cert.DevicePub != hex.EncodeToString(b.ident.DevicePub()) {
		t.Fatalf("a's cert for b: %+v %v", cert, ok)
	}
	// Live rows flow both ways and relay is just Missing: a third device
	// joining later gets both histories from either.
	ea := a.append(store.KindAgentSeen, map[string]any{"key": a.id() + "/s2"})
	eb := b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/s3"})
	eventually(t, "live rows cross", func() bool { return a.has(eb) && b.has(ea) })
	if p := a.peer(b.id()); p.State != "connected" {
		t.Fatalf("b seen from a: %+v", p)
	}
	// Devices shows when b was last heard from.
	if rows := a.node.Devices(false); len(rows) != 2 || rows[1].LastSeen.IsZero() {
		t.Fatalf("a's devices: %+v", rows)
	}
}

func TestLocalKindsNeverCross(t *testing.T) {
	net := NewFakeNet()
	a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
	b.join(a)
	if _, err := a.st.AppendLocal(context.Background(), "user", store.KindPolicySet, map[string]any{"paused": true}); err != nil {
		t.Fatal(err)
	}
	a.start()
	b.start()
	eventually(t, "converge", converged(a, b))
	local, err := b.st.List(context.Background(), store.Filter{Local: true})
	if err != nil || len(local) != 0 {
		t.Fatalf("b's local rows: %v %v", local, err)
	}
	evs, _ := b.st.List(context.Background(), store.Filter{Kinds: []string{store.KindPolicySet}})
	if len(evs) != 0 {
		t.Fatal("a policy row reached b")
	}
}

// forger connects to a's topic as a device a doesn't know. With the
// handshake's cert check on, it never connects; with it off (TrustAnyPeer),
// nothing it sends is stored (§8: both layers hold).
func TestForgedRowsAreRefusedEvenWithTheCertCheckOff(t *testing.T) {
	for _, trust := range []bool{false, true} {
		t.Run(map[bool]string{false: "cert check on", true: "cert check off"}[trust], func(t *testing.T) {
			net := NewFakeNet()
			a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
			b.join(a)
			a.trust = trust
			a.start()
			b.start()
			eventually(t, "converge", converged(a, b))
			seed, _ := a.ident.PersonSeed(1)
			secret := MeshSecret(seed, a.ident.Person.ID, 1)
			room, conns := rawPeer(t, net, secret, "ffffffffforger")
			_ = room.Send("", MsgPresence, Presence{DeviceID: "ffffffffforger", Epoch: 1})
			room.Connect(a.id())
			if !trust {
				never(t, "the forger connects", func() bool { return len(conns) > 0 })
				return
			}
			var c Conn
			select {
			case c = <-conns:
			case <-time.After(5 * time.Second):
				t.Fatal("forger never connected with the cert check off")
			}
			sendFrame(t, c, Frame{Type: FrameHello, V: Protocol, DeviceID: "ffffffffforger", PersonID: a.ident.Person.ID,
				Epoch: 1, Schema: store.SchemaVersion(), WallMs: time.Now().UnixMilli()})
			_, fkey, _ := ed25519.GenerateKey(rand.Reader)
			sign := func(ev store.Event) store.Event {
				ev.Sig = ed25519.Sign(fkey, store.SigningBytes(ev))
				return ev
			}
			now := hlc.Timestamp(time.Now().UnixMilli()) << 16
			bv := a.vector()[b.id()]
			forged := []store.Event{
				// its own row: no cert, held and never stored
				sign(store.Event{DeviceID: "ffffffffforger", Seq: 1, HLC: now, Actor: "user", Kind: "message.sent", Payload: []byte(`{}`)}),
				// b's next row with the wrong key: bad signature
				sign(store.Event{DeviceID: b.id(), Seq: bv + 1, HLC: now + 1, Actor: "user", Kind: "message.sent", Payload: []byte(`{}`)}),
				// a local kind: refused before any signature check
				sign(store.Event{DeviceID: "eeeeeeeeother", Seq: 1, HLC: now + 2, Actor: "user", Kind: store.KindPolicySet, Payload: []byte(`{}`)}),
			}
			// A cert for itself, signed by a person key it made up.
			_, pkey, _ := ed25519.GenerateKey(rand.Reader)
			cert := identity.Cert{PersonID: a.ident.Person.ID, DeviceID: "ffffffffforger", DevicePub: hex.EncodeToString(fkey.Public().(ed25519.PublicKey)), Role: "desktop"}
			msg, _ := identity.Canonical(cert)
			forged = append(forged, sign(store.Event{DeviceID: "ffffffffforger", Seq: 2, HLC: now + 3, Actor: "irori", Kind: store.KindPersonDeviceAdded,
				Payload: mustJSON(t, identity.DeviceAdded{Cert: cert, PersonPub: hex.EncodeToString(pkey.Public().(ed25519.PublicKey)),
					PersonSig: hex.EncodeToString(ed25519.Sign(pkey, msg))})}))
			var rows []Row
			for _, ev := range forged {
				rows = append(rows, RowOf(ev))
			}
			sendFrame(t, c, Frame{Type: FrameRows, Events: rows})
			sendFrame(t, c, Frame{Type: FrameRows, Events: rows, Loose: true})
			if _, err := recvFrame(c, FrameVector, 5*time.Second); err != nil {
				t.Fatal(err)
			}
			eventually(t, "the refusals are counted", func() bool {
				a.node.mu.Lock()
				defer a.node.mu.Unlock()
				k := a.node.known["ffffffffforger"]
				return k != nil && k.refused >= 2 // the bad signature and the local kind
			})
			never(t, "a forged row is stored", func() bool {
				v := a.vector()
				return v["ffffffffforger"] > 0 || v["eeeeeeeeother"] > 0 || v[b.id()] > bv
			})
			if _, ok := a.chain.Cert("ffffffffforger"); ok {
				t.Fatal("the forger got certified")
			}
			// The person secret never goes to it, even with its epoch behind.
			if _, err := recvFrame(c, FramePersonSecret, 200*time.Millisecond); err == nil {
				t.Fatal("the forger was sent the person secret")
			}
		})
	}
}

func TestCatchUpThroughTheNodeWithTheMacGone(t *testing.T) {
	net := NewFakeNet()
	pc, mac, nd := trio(t, net)
	pc.stop() // the PC is off
	var rows []store.Event
	for i := range 20 {
		rows = append(rows, mac.append(store.KindAgentSeen, map[string]any{"key": mac.id() + "/s", "i": i}))
	}
	eventually(t, "the node holds the Mac's rows", func() bool { return nd.has(rows[len(rows)-1]) })
	mac.stop() // the Mac sleeps
	pc.start()
	eventually(t, "the PC catches up from the node", func() bool { return pc.has(rows[len(rows)-1]) })
	if net.Connected(pc.id(), mac.id()) {
		t.Fatal("the PC reached the Mac, which was asleep")
	}
}

func TestBackfillInBatches(t *testing.T) {
	net := NewFakeNet()
	a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
	big := strings.Repeat("x", 4000)
	for i := range 1300 {
		a.append("capture.turn", map[string]any{"i": i, "text": big})
	}
	b.join(a)
	a.start()
	b.start()
	eventually(t, "the backfill lands", converged(a, b))
}

func TestReconnectAfterADrop(t *testing.T) {
	net := NewFakeNet()
	a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
	b.join(a)
	a.start()
	b.start()
	eventually(t, "converge", converged(a, b))
	net.Partition(a.id(), b.id())
	eb := b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/late"})
	never(t, "a row crosses a partition", func() bool { return a.has(eb) })
	net.Heal(a.id(), b.id())
	eventually(t, "they reconnect and sync", func() bool { return a.has(eb) })
	net.Drop(a.id(), b.id())
	ea := a.append(store.KindAgentSeen, map[string]any{"key": a.id() + "/late"})
	eventually(t, "they reconnect after a drop", func() bool { return b.has(ea) })
}

func TestRevokeConvergesAndRotatesTheKey(t *testing.T) {
	net := NewFakeNet()
	pc, mac, nd := trio(t, net)
	// The Mac writes rows the node hears and the PC doesn't.
	net.Isolate(pc.id())
	cutSeq := pc.vector()[mac.id()]
	var extra []store.Event
	for i := range 3 {
		extra = append(extra, mac.append(store.KindAgentSeen, map[string]any{"key": mac.id() + "/x", "i": i}))
	}
	eventually(t, "the node hears the Mac's extra rows", func() bool { return nd.has(extra[2]) })
	net.Partition(nd.id(), mac.id()) // and the Mac goes away
	if err := pc.node.Revoke(context.Background(), mac.id(), "lost"); err != nil {
		t.Fatal(err)
	}
	if pc.ident.Current().Epoch != 2 {
		t.Fatal("the PC's person key did not rotate")
	}
	net.Heal(pc.id(), nd.id())
	eventually(t, "the node takes the revoke and the new key", func() bool {
		_, rev := nd.chain.Cut(mac.id())
		return rev && nd.ident.Current().Epoch == 2 && nd.ident.Current().Pub == pc.ident.Current().Pub
	})
	eventually(t, "the node's extra rows move out and vectors converge", func() bool {
		return nd.vector()[mac.id()] == cutSeq && converged(pc, nd)()
	})
	ref, _ := nd.st.Refused(context.Background(), 100)
	moved := 0
	for _, ev := range ref {
		if ev.DeviceID == mac.id() {
			moved++
		}
	}
	if moved != len(extra) {
		t.Fatalf("node refused_events holds %d of the Mac's rows, want %d", moved, len(extra))
	}
	// Both are on the new topic only: no device is behind.
	cur := MeshSecret(mustSeed(t, pc, 2), pc.ident.Person.ID, 2).Topic
	eventually(t, "the old topic closes", func() bool {
		tp := net.Topics(pc.id())
		return len(tp) == 1 && tp[0] == cur && len(net.Topics(nd.id())) == 1
	})
	// The Mac can't come back: not on the old topic, not on a reconnect.
	net.HealAll()
	never(t, "the revoked Mac connects", func() bool {
		return net.Connected(pc.id(), mac.id()) || net.Connected(nd.id(), mac.id())
	})
	old := MeshSecret(mustSeed(t, pc, 1), pc.ident.Person.ID, 1)
	r, conns := rawPeer(t, net, old, mac.id()+"x")
	_ = r
	never(t, "anyone answers on the old topic", func() bool { return len(conns) > 0 })
}

// The node is off when the PC revokes the Mac. The PC keeps the epoch-1
// topic open until the node has the new key, which it gets there.
func TestAnOfflineDeviceGetsTheNewKeyOnThePreviousTopic(t *testing.T) {
	net := NewFakeNet()
	pc, mac, nd := trio(t, net)
	nd.stop()
	mac.stop()
	if err := pc.node.Revoke(context.Background(), mac.id(), "lost"); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the PC keeps both topics open", func() bool { return len(net.Topics(pc.id())) == 2 })
	nd.start()
	eventually(t, "the node adopts epoch 2", func() bool { return nd.ident.Current().Epoch == 2 })
	eventually(t, "both close the old topic", func() bool {
		return len(net.Topics(pc.id())) == 1 && len(net.Topics(nd.id())) == 1
	})
	e := pc.append(store.KindAgentSeen, map[string]any{"key": pc.id() + "/after"})
	eventually(t, "sync runs on the new topic", func() bool { return nd.has(e) })
	// The node keeps the new key across a restart.
	nd.reopen()
	if nd.ident.Current().Epoch != 2 {
		t.Fatal("the adopted key did not survive a restart")
	}
}

func mustSeed(t *testing.T, d *dev, epoch int) []byte {
	t.Helper()
	s, ok := d.ident.PersonSeed(epoch)
	if !ok {
		t.Fatalf("%s holds no epoch %d seed", d.name, epoch)
	}
	return s
}

func TestARestoredDeviceGetsItsOwnRowsBack(t *testing.T) {
	net := NewFakeNet()
	a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
	b.join(a)
	a.start()
	b.start()
	eventually(t, "converge", converged(a, b))
	b.close()
	backup := t.TempDir()
	copyDir(t, b.dir, backup)
	b.open()
	b.start()
	var later []store.Event
	for i := range 5 {
		later = append(later, b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/later", "i": i}))
	}
	eventually(t, "a has b's later rows", func() bool { return a.has(later[4]) })
	// b's data dir is restored from the backup.
	b.close()
	copyDir(t, backup, b.dir)
	b.open()
	if b.vector()[b.id()] >= later[4].Seq {
		t.Fatal("the backup already held the later rows")
	}
	b.start()
	eventually(t, "b gets its own rows back", func() bool { return b.has(later[4]) })
	ev := b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/new"})
	if ev.Seq != later[4].Seq+1 {
		t.Fatalf("b's next row has seq %d, want %d", ev.Seq, later[4].Seq+1)
	}
	eventually(t, "a takes it", func() bool { return a.has(ev) })
	if p := a.peer(b.id()); p.Stopped != "" {
		t.Fatalf("a flagged b: %+v", p)
	}
}

func TestARestoredDeviceThatWroteOverItsRowsIsAFork(t *testing.T) {
	net := NewFakeNet()
	a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
	b.join(a)
	a.start()
	b.start()
	eventually(t, "converge", converged(a, b))
	b.close()
	backup := t.TempDir()
	copyDir(t, b.dir, backup)
	b.open()
	b.start()
	later := b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/later"})
	eventually(t, "a has it", func() bool { return a.has(later) })
	b.close()
	copyDir(t, backup, b.dir)
	b.open()
	other := b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/other"}) // the same seq, before syncing
	if other.Seq != later.Seq {
		t.Fatalf("seq %d, want %d", other.Seq, later.Seq)
	}
	b.start()
	eventually(t, "a flags b as signing two histories", func() bool { return a.peer(b.id()).Stopped != "" })
	forks, _ := a.st.Forks(context.Background())
	if len(forks) == 0 {
		t.Fatal("no fork recorded on a")
	}
}

func TestRowsFromAClockTenMinutesAheadWaitThenLand(t *testing.T) {
	net := NewFakeNet()
	a, b := newDev(t, net, "pc"), newDev(t, net, "mac")
	b.join(a)
	a.start()
	b.start()
	eventually(t, "converge", converged(a, b))
	b.clk.Set(10 * time.Minute)
	ahead := b.append(store.KindAgentSeen, map[string]any{"key": b.id() + "/ahead"})
	never(t, "a stores a row 10 minutes ahead", func() bool { return a.has(ahead) })
	eventually(t, "a shows b's rows waiting", func() bool { return a.peer(b.id()).Ahead })
	a.clk.Set(10 * time.Minute) // a's wall time catches up
	eventually(t, "the row lands", func() bool { return a.has(ahead) })
}

func TestHeldRowsAreCapped(t *testing.T) {
	net := NewFakeNet()
	a := newDev(t, net, "pc")
	a.node = New(Options{Store: a.st, Ident: a.ident, Chain: a.chain, Net: net})
	s := newSession(a.node, 1, nil)
	var evs []store.Event
	for i := range HeldRowsCap + 500 {
		evs = append(evs, store.Event{DeviceID: "x", Seq: int64(i + 1), Payload: []byte(`{}`)})
	}
	s.hold("x", evs)
	s.hold("x", evs[:10]) // already held: not counted twice
	if s.heldCount() != HeldRowsCap {
		t.Fatalf("held %d rows, cap %d", s.heldCount(), HeldRowsCap)
	}
	big := store.Event{DeviceID: "y", Seq: 1, Payload: make([]byte, HeldBytesCap+1)}
	s2 := newSession(a.node, 1, nil)
	s2.hold("y", []store.Event{big})
	if s2.heldCount() != 0 {
		t.Fatal("a row past the byte cap was held")
	}
}

func TestBatchesKeepTheLimits(t *testing.T) {
	var evs []store.Event
	for i := range 1200 {
		evs = append(evs, store.Event{DeviceID: "d", Seq: int64(i + 1), Payload: []byte(`{"a":1}`)})
	}
	fr := batches(evs, false)
	if len(fr) != 3 || len(fr[0].Events) != MaxBatchRows || len(fr[2].Events) != 200 {
		t.Fatalf("row cap: %d frames", len(fr))
	}
	var big []store.Event
	for i := range 10 {
		big = append(big, store.Event{DeviceID: "d", Seq: int64(i + 1), Payload: make([]byte, 300<<10)})
	}
	for _, f := range batches(big, false) {
		b, _ := f.Encode()
		if len(f.Events) > 1 && len(b) > MaxBatchBytes+(64<<10) {
			t.Fatalf("frame of %d rows is %d bytes", len(f.Events), len(b))
		}
	}
	// The payload travels as the stored bytes.
	b, _ := Frame{Type: FrameRows, Events: []Row{RowOf(store.Event{DeviceID: "d", Seq: 1, Payload: []byte(`{"b": 2 }`), Sig: []byte{1, 2}})}}.Encode()
	f, err := DecodeFrame(b)
	if err != nil || string(f.Events[0].Event().Payload) != `{"b": 2 }` || f.Events[0].Event().Sig[1] != 2 {
		t.Fatalf("round trip: %+v %v", f, err)
	}
}

func TestMeshSecretMovesWithTheEpoch(t *testing.T) {
	seed := make([]byte, 32)
	a, b := MeshSecret(seed, "p", 1), MeshSecret(seed, "p", 2)
	if a.Topic == b.Topic || a.Key == b.Key || !strings.HasPrefix(a.Topic, MeshApp+"/") {
		t.Fatalf("epochs share a topic: %s %s", a.Topic, b.Topic)
	}
	if MeshSecret(seed, "p", 1) != a || MeshSecret(seed, "q", 1).Topic == a.Topic {
		t.Fatal("the topic is not a function of the secret, person and epoch")
	}
}

func TestPersonSecretSealing(t *testing.T) {
	old, nw := make([]byte, 32), []byte("0123456789abcdef0123456789abcdef")
	old[0] = 7
	sealed, err := sealSecret(old, nw, "p", 2, "dev-b")
	if err != nil {
		t.Fatal(err)
	}
	if got, err := openSecret(old, sealed, "p", 2, "dev-b"); err != nil || string(got) != string(nw) {
		t.Fatalf("open: %q %v", got, err)
	}
	if _, err := openSecret(old, sealed, "p", 2, "dev-c"); err == nil {
		t.Fatal("another device opened it")
	}
	if _, err := openSecret(old, sealed, "p", 3, "dev-b"); err == nil {
		t.Fatal("it opened for another epoch")
	}
}

func TestRelaySetting(t *testing.T) {
	dir := t.TempDir()
	if r, err := LoadRelay(dir); err != nil || !r.Direct() {
		t.Fatalf("default relay %+v %v", r, err)
	}
	if err := SaveRelay(dir, Relay{CredentialURL: "http://example.com/cred"}); err == nil {
		t.Fatal("saved a plain-http credential URL")
	}
	want := Relay{CredentialURL: "https://relay.example/cred", TurnURLs: []string{"turns:relay.example:5349"}}
	if err := SaveRelay(dir, want); err != nil {
		t.Fatal(err)
	}
	if r, _ := LoadRelay(dir); r.CredentialURL != want.CredentialURL || len(r.TurnURLs) != 1 {
		t.Fatalf("reloaded %+v", r)
	}
	if err := SaveRelay(dir, Relay{}); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, RelayFile)); !os.IsNotExist(err) {
		t.Fatal("Direct only left relay.json behind")
	}
}

func mustJSON(t *testing.T, v any) json.RawMessage {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func copyDir(t *testing.T, src, dst string) {
	t.Helper()
	entries, err := os.ReadDir(src)
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(src, e.Name()))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dst, e.Name()), raw, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	// Files the source lacks (a WAL written since) go too.
	dstEntries, _ := os.ReadDir(dst)
	for _, e := range dstEntries {
		if _, err := os.Stat(filepath.Join(src, e.Name())); os.IsNotExist(err) && !e.IsDir() {
			os.Remove(filepath.Join(dst, e.Name()))
		}
	}
}
