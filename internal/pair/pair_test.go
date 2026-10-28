package pair

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

func TestCodeCodec(t *testing.T) {
	c := NewCode()
	if len(c) != 19 || strings.Count(c, "-") != 3 {
		t.Fatalf("code %q", c)
	}
	for _, in := range []string{c, strings.ToLower(c), strings.ReplaceAll(c, "-", ""), " " + strings.ReplaceAll(c, "-", " ") + " "} {
		got, ok := NormalizeCode(in)
		if !ok || got != c {
			t.Fatalf("NormalizeCode(%q) = %q, %v", in, got, ok)
		}
	}
	for _, bad := range []string{"", "ABCD", c + "A", strings.Replace(c, c[:1], "0", 1)} {
		if _, ok := NormalizeCode(bad); ok {
			t.Fatalf("NormalizeCode(%q) accepted", bad)
		}
	}
	if NewCode() == c {
		t.Fatal("two codes in a row are the same")
	}
	a, err := RoomSecret(strings.ToLower(c))
	if err != nil {
		t.Fatal(err)
	}
	b, _ := RoomSecret(c)
	if a != b || !strings.HasPrefix(a.Topic, App+"/") {
		t.Fatalf("room secret differs by spelling: %s %s", a.Topic, b.Topic)
	}
	if _, err := RoomSecret("nope"); !errors.Is(err, ErrBadCode) {
		t.Fatalf("bad code: %v", err)
	}
}

func TestWordlists(t *testing.T) {
	seen := map[string]bool{}
	for _, list := range [][256]string{evenWords, oddWords} {
		for i, w := range list {
			if w == "" || w != strings.ToLower(w) || seen[w] {
				t.Fatalf("word %d %q is empty, not lower case, or repeated", i, w)
			}
			seen[w] = true
		}
	}
}

func TestSASBindsKeysFingerprintsAndCode(t *testing.T) {
	ap, _, _ := ed25519.GenerateKey(rand.Reader)
	bp, _, _ := ed25519.GenerateKey(rand.Reader)
	xp, _, _ := ed25519.GenerateKey(rand.Reader)
	s1, _ := RoomSecret(NewCode())
	s2, _ := RoomSecret(NewCode())
	info := Info(ap, bp, "fpA", "fpB", "a", "b")
	w := SAS(s1.Key, info)
	if len(w) != SASWords || strings.Join(w, " ") != strings.Join(SAS(s1.Key, Info(ap, bp, "fpA", "fpB", "a", "b")), " ") {
		t.Fatalf("SAS isn't stable: %v", w)
	}
	for name, other := range map[string][]string{
		"another device key":  SAS(s1.Key, Info(ap, xp, "fpA", "fpB", "a", "b")),
		"another fingerprint": SAS(s1.Key, Info(ap, bp, "fpA", "fpX", "a", "b")),
		"another code":        SAS(s2.Key, info),
		"sides swapped":       SAS(s1.Key, Info(bp, ap, "fpB", "fpA", "b", "a")),
	} {
		if strings.Join(other, " ") == strings.Join(w, " ") {
			t.Fatalf("%s gives the same words", name)
		}
	}
	if !WordsMatch(w, strings.ToUpper(strings.Join(w, "-"))) || WordsMatch(w, strings.Join(w[:4], " ")) {
		t.Fatal("WordsMatch")
	}
	p := Proof(s1.Key, 'A', info)
	if !CheckProof(s1.Key, 'A', info, p) || CheckProof(s1.Key, 'B', info, p) || CheckProof(s2.Key, 'A', info, p) {
		t.Fatal("a proof checks against the wrong side or code")
	}
}

// dev is one device: a real store, keys and chain, a mesh node and a
// Pairer, all on one FakeNet. Nothing opens a socket.
type dev struct {
	t     *testing.T
	name  string
	dir   string
	net   *mesh.FakeNet
	ident *identity.Identity
	chain *identity.Chain
	st    *store.Store
	node  *mesh.Node
	pair  *Pairer
	noNet bool // the node is built but not run: no sync
	stop  context.CancelFunc
	done  chan struct{}
}

var fast = mesh.Intervals{Presence: 40 * time.Millisecond, Ping: 60 * time.Millisecond, Vector: 80 * time.Millisecond,
	BackoffMin: 20 * time.Millisecond, BackoffMax: 200 * time.Millisecond}

func newDev(t *testing.T, net *mesh.FakeNet, name string, window time.Duration) *dev {
	t.Helper()
	d := &dev{t: t, name: name, dir: t.TempDir(), net: net}
	d.open(window)
	t.Cleanup(d.close)
	return d
}

func (d *dev) open(window time.Duration) {
	t := d.t
	ctx := context.Background()
	dv, err := device.LoadOrCreate(d.dir)
	if err != nil {
		t.Fatal(err)
	}
	dv.Name = d.name
	_ = device.Save(d.dir, dv)
	if d.ident, err = identity.Load(d.dir, dv, identity.FileKeystore{}); err != nil {
		t.Fatal(err)
	}
	d.chain = identity.NewChain(d.ident.Person.ID, d.ident.Person.RootDeviceID, dv.ID)
	if d.st, err = store.OpenWith(ctx, filepath.Join(d.dir, "irori.db"), dv.ID, hlc.New(nil),
		store.OpenOptions{Signer: d.ident, Verifier: d.chain}); err != nil {
		t.Fatal(err)
	}
	if err := d.ident.EnsureSelfCert(ctx, d.st, d.chain, identity.RoleDesktop); err != nil {
		t.Fatal(err)
	}
	if err := LearnSaved(ctx, d.dir, d.st, d.chain); err != nil {
		t.Fatal(err)
	}
	d.node = mesh.New(mesh.Options{Store: d.st, Ident: d.ident, Chain: d.chain, Net: d.net, Version: "0.4.0-test", Intervals: fast,
		Logf: func(f string, a ...any) { t.Logf(d.name+": "+f, a...) }})
	d.pair = New(Options{Net: d.net, Ident: d.ident, Chain: d.chain, Store: d.st, DataDir: d.dir, Kick: d.node.Kick,
		Window: window, HelloEvery: 30 * time.Millisecond, Logf: func(f string, a ...any) { t.Logf(d.name+": "+f, a...) }})
	c, cancel := context.WithCancel(context.Background())
	d.stop, d.done = cancel, make(chan struct{})
	if d.noNet {
		close(d.done)
		return
	}
	go func() { defer close(d.done); _ = d.node.Run(c) }()
}

func (d *dev) close() {
	if d.stop != nil {
		d.pair.Cancel()
		d.stop()
		<-d.done
		d.stop = nil
	}
	if d.st != nil {
		d.st.Close()
		d.st = nil
	}
}

func (d *dev) id() string { return d.ident.Device.ID }

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

func state(d *dev, want string) func() bool {
	return func() bool { return d.pair.Status().State == want }
}

func held(d *dev, ev store.Event) bool {
	evs, err := d.st.List(context.Background(), store.Filter{DeviceID: ev.DeviceID, AfterSeq: ev.Seq - 1, Limit: 1})
	return err == nil && len(evs) == 1 && evs[0].Seq == ev.Seq && store.RowHash(evs[0]) == store.RowHash(ev)
}

// pairUp runs Add on a, Join on b and types b's words into a.
func pairUp(t *testing.T, a, b *dev) {
	t.Helper()
	st, err := a.pair.Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	if st.Code == "" {
		t.Fatalf("add status %+v", st)
	}
	if _, err := b.pair.Join(strings.ToLower(st.Code)); err != nil {
		t.Fatal(err)
	}
	eventually(t, "b shows words", state(b, StateWords))
	eventually(t, "a asks for the words", state(a, StateConfirm))
	if j := a.pair.Status().Joiner; j == nil || j.DeviceID != b.id() || j.Name != b.name || j.Fingerprint != Fingerprint(b.ident.DevicePub()) {
		t.Fatalf("a shows joiner %+v", a.pair.Status().Joiner)
	}
	if err := a.pair.Confirm(context.Background(), strings.Join(b.pair.Status().Words, " ")); err != nil {
		t.Fatal(err)
	}
	eventually(t, "a waits for its key on b", state(a, StateGranted))
	if _, ok := a.chain.Cert(b.id()); ok {
		t.Fatal("a certified b before b's person typed a's key")
	}
	if err := b.pair.ConfirmKey(context.Background(), a.pair.Status().Fingerprint); err != nil {
		t.Fatal(err)
	}
	eventually(t, "a done", state(a, StateDone))
	eventually(t, "b done", state(b, StateDone))
}

func TestPairCopiesThePersonAndSyncs(t *testing.T) {
	net := mesh.NewFakeNet()
	a, b := newDev(t, net, "pc", 0), newDev(t, net, "mac", 0)
	before, err := b.st.Append(context.Background(), "irori", "device.started", map[string]any{"name": "mac"})
	if err != nil {
		t.Fatal(err)
	}
	st, _ := a.pair.Add(identity.RoleNode)
	if _, err := b.pair.Join(st.Code); err != nil {
		t.Fatal(err)
	}
	eventually(t, "b shows words", state(b, StateWords))
	eventually(t, "a asks for the words", state(a, StateConfirm))
	if a.pair.Status().Code != "" {
		t.Fatal("the code still shows once a device used it")
	}
	words := b.pair.Status().Words
	if err := a.pair.Confirm(context.Background(), "aardvark adroitness aardvark adroitness aardvark"); !errors.Is(err, ErrWrongWords) {
		t.Fatalf("wrong words: %v", err)
	}
	if s := a.pair.Status(); s.State != StateConfirm || s.Misses != 1 {
		t.Fatalf("one miss closed the window: %+v", s)
	}
	// B's key check can come first: the grant then goes through at once.
	if err := b.pair.ConfirmKey(context.Background(), "0000 0000 0000 0000"); !errors.Is(err, ErrWrongKey) {
		t.Fatalf("wrong key: %v", err)
	}
	if err := b.pair.ConfirmKey(context.Background(), strings.ToUpper(strings.ReplaceAll(Fingerprint(a.ident.DevicePub()), " ", "-"))); err != nil {
		t.Fatal(err)
	}
	if err := a.pair.Confirm(context.Background(), strings.Join(words, ", ")); err != nil {
		t.Fatal(err)
	}
	eventually(t, "both done", func() bool { return state(a, StateDone)() && state(b, StateDone)() })

	pa, pb := a.ident.Current(), b.ident.Current()
	if pb.ID != pa.ID || pb.RootDeviceID != a.id() || pb.Epoch != pa.Epoch || pb.Pub != pa.Pub {
		t.Fatalf("b's person %+v, a's %+v", pb, pa)
	}
	if dv, _ := device.LoadOrCreate(b.dir); dv.PersonID != pa.ID {
		t.Fatalf("b's device.json names person %s", dv.PersonID)
	}
	cert, ok := a.chain.Cert(b.id())
	if !ok || cert.Role != identity.RoleNode || cert.Label != "mac" {
		t.Fatalf("a's cert for b: %+v %v", cert, ok)
	}
	if _, ok := b.chain.Cert(b.id()); !ok {
		t.Fatal("b doesn't hold its own cert")
	}
	if net.Members(mustSecret(t, st.Code).Topic) != nil {
		t.Fatal("someone is still in the pairing room")
	}
	// Both reconnect on the mesh topic, and b's earlier rows reach a and verify.
	eventually(t, "b's earlier row on a", func() bool { return held(a, before) })
	eventually(t, "a's cert for b stored on b", func() bool {
		evs, _ := b.st.List(context.Background(), store.Filter{Kinds: []string{store.KindPersonDeviceAdded}, DeviceID: a.id()})
		return len(evs) >= 2
	})
	if _, err := os.Stat(filepath.Join(b.dir, RowsFile)); err != nil {
		t.Fatalf("b keeps the pairing rows until restart: %v", err)
	}
	// B restarted: the rows are held now, so the file goes.
	b.close()
	b.open(0)
	if _, err := os.Stat(filepath.Join(b.dir, RowsFile)); !os.IsNotExist(err) {
		t.Fatalf("rows file after the rows were stored: %v", err)
	}
	if _, err := b.pair.Join(NewCode()); !errors.Is(err, ErrInMesh) {
		t.Fatalf("a paired device joined again: %v", err)
	}
}

func mustSecret(t *testing.T, code string) mesh.Secret {
	s, err := RoomSecret(code)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

// B paired while A's net was down: B restarts before any sync and still
// holds its cert (the saved rows), so its net layer starts and finds A.
func TestPairingSurvivesARestartBeforeTheFirstSync(t *testing.T) {
	net := mesh.NewFakeNet()
	a := &dev{t: t, name: "pc", dir: t.TempDir(), net: net, noNet: true}
	a.open(0)
	t.Cleanup(a.close)
	b := newDev(t, net, "mac", 0)
	pairUp(t, a, b)
	a.close()
	b.close()
	if evs, _ := func() ([]store.Event, error) {
		b.open(0)
		return b.st.List(context.Background(), store.Filter{DeviceID: a.id()})
	}(); len(evs) != 0 {
		t.Fatalf("b holds %d of a's rows: the test needs no sync before the restart", len(evs))
	}
	a.noNet = false
	if _, ok := b.chain.Cert(b.id()); !ok || !b.node.HasPeer() {
		t.Fatal("after a restart b lost its cert")
	}
	a.open(0)
	eventually(t, "a and b connect", func() bool { return net.Connected(a.id(), b.id()) })
}

func TestSecondJoinerGetsBusyAndTheWindowCloses(t *testing.T) {
	net := mesh.NewFakeNet()
	a, b, c := newDev(t, net, "pc", 0), newDev(t, net, "mac", 0), newDev(t, net, "thief", 0)
	st, _ := a.pair.Add(identity.RoleDesktop)
	if _, err := b.pair.Join(st.Code); err != nil {
		t.Fatal(err)
	}
	eventually(t, "a asks for the words", state(a, StateConfirm))
	if _, err := c.pair.Join(st.Code); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the second joiner hears busy", state(c, StateCodeInUse))
	eventually(t, "a's window closes", state(a, StateFailed))
	if a.pair.Status().Error != ErrTextSomeoneElse {
		t.Fatalf("a says %q", a.pair.Status().Error)
	}
	if c.pair.Status().Words != nil {
		t.Fatal("the second joiner got words")
	}
	eventually(t, "b hears the window closed", state(b, StateFailed))
	if _, ok := a.chain.Cert(b.id()); ok {
		t.Fatal("b was certified")
	}
}

func TestWrongWordsTwiceCloseTheWindow(t *testing.T) {
	net := mesh.NewFakeNet()
	a, b := newDev(t, net, "pc", 0), newDev(t, net, "mac", 0)
	st, _ := a.pair.Add(identity.RoleDesktop)
	_, _ = b.pair.Join(st.Code)
	eventually(t, "a asks for the words", state(a, StateConfirm))
	for range MaxMisses {
		if err := a.pair.Confirm(context.Background(), "wrong words"); !errors.Is(err, ErrWrongWords) {
			t.Fatal(err)
		}
	}
	if s := a.pair.Status(); s.State != StateFailed || s.Error != ErrTextSomeoneElse {
		t.Fatalf("after two misses: %+v", s)
	}
	eventually(t, "b closes too", state(b, StateFailed))
	if err := a.pair.Confirm(context.Background(), strings.Join(SAS([32]byte{}, nil), " ")); !errors.Is(err, ErrNotOpen) {
		t.Fatalf("confirm after close: %v", err)
	}
}

// A code holder that can't prove it (a forger speaking frames by hand)
// closes the window: the HMAC failure.
func TestABadProofClosesTheWindow(t *testing.T) {
	net := mesh.NewFakeNet()
	a := newDev(t, net, "pc", 0)
	st, _ := a.pair.Add(identity.RoleDesktop)
	secret := mustSecret(t, st.Code)
	pub, _, _ := ed25519.GenerateKey(rand.Reader)
	conns := make(chan mesh.Conn, 1)
	r, err := net.Join(secret, mesh.RoomConfig{Self: "forger", Pub: pub, Sign: func([]byte) []byte { return nil },
		Accept: func(string, ed25519.PublicKey) error { return nil }, OnPeer: func(c mesh.Conn) { conns <- c }})
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	_ = r.Send("", MsgHello, helloBody{DeviceID: "forger", Name: "x", OS: "y"})
	var c mesh.Conn
	select {
	case c = <-conns:
	case <-time.After(5 * time.Second):
		t.Fatal("a never connected to the joiner")
	}
	bad, _ := json.Marshal(frame{Type: frameProof, Proof: make([]byte, 32)})
	if err := c.Send(context.Background(), bad); err != nil {
		t.Fatal(err)
	}
	eventually(t, "a's window closes", state(a, StateFailed))
	if a.pair.Status().Error != ErrTextSomeoneElse {
		t.Fatalf("a says %q", a.pair.Status().Error)
	}
}

func TestTheWindowExpires(t *testing.T) {
	net := mesh.NewFakeNet()
	a := newDev(t, net, "pc", 150*time.Millisecond)
	st, _ := a.pair.Add(identity.RoleDesktop)
	eventually(t, "the window expires", state(a, StateExpired))
	if len(net.Members(mustSecret(t, st.Code).Topic)) != 0 {
		t.Fatal("a stayed in the pairing room")
	}
	if _, err := a.pair.Add(identity.RoleDesktop); err != nil {
		t.Fatalf("a new window after expiry: %v", err)
	}
	if _, err := a.pair.Add(identity.RoleDesktop); !errors.Is(err, ErrBusy) {
		t.Fatalf("two windows at once: %v", err)
	}
}

// A revoked device can't come back through a pairing room: A ignores its
// hello, so nothing connects.
func TestARevokedDeviceCantPairAgain(t *testing.T) {
	net := mesh.NewFakeNet()
	a, b := newDev(t, net, "pc", 0), newDev(t, net, "mac", 0)
	pairUp(t, a, b)
	if err := a.node.Revoke(context.Background(), b.id(), "lost"); err != nil {
		t.Fatal(err)
	}
	st, err := a.pair.Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	// b is in a mesh (its own revoke aside), so its Join is refused anyway;
	// speak its hello by hand to show A ignores it.
	secret := mustSecret(t, st.Code)
	conns := make(chan mesh.Conn, 1)
	r, err := net.Join(secret, mesh.RoomConfig{Self: b.id(), Pub: b.ident.DevicePub(), Sign: b.ident.Sign,
		Accept: func(string, ed25519.PublicKey) error { return nil }, OnPeer: func(c mesh.Conn) { conns <- c }})
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	for range 5 {
		_ = r.Send("", MsgHello, helloBody{DeviceID: b.id(), Name: "mac"})
		time.Sleep(30 * time.Millisecond)
	}
	select {
	case <-conns:
		t.Fatal("a connected to a revoked device in the pairing room")
	case <-time.After(300 * time.Millisecond):
	}
	if s := a.pair.Status(); s.State != StateWaiting {
		t.Fatalf("a's window after a revoked hello: %+v", s)
	}
}

// Someone else with A's code (an agent that read it off the clipboard)
// reaches B first, posing as A: it proves the code, and hands B a person
// of its own that certifies B. B holds the grant until the person types
// A's key, which the impostor's handshake can't prove, so B never adopts.
func TestACodeHolderPosingAsACantHandBAPerson(t *testing.T) {
	net := mesh.NewFakeNet()
	a, b, thief := newDev(t, net, "pc", 0), newDev(t, net, "mac", 0), newDev(t, net, "thief", 0)
	st, _ := a.pair.Add(identity.RoleDesktop)
	a.pair.Cancel() // the real A waits elsewhere; only the thief answers
	secret := mustSecret(t, st.Code)
	conns := make(chan mesh.Conn, 1)
	var r mesh.Room
	r, err := net.Join(secret, mesh.RoomConfig{Self: thief.id(), Pub: thief.ident.DevicePub(), Sign: thief.ident.Sign,
		Accept: func(string, ed25519.PublicKey) error { return nil }, Offerer: func(string) bool { return true },
		OnMsg: func(env mesh.Envelope) {
			if env.Type == MsgHello {
				go r.Connect(env.From)
			}
		},
		OnPeer: func(c mesh.Conn) { conns <- c }})
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	before := b.ident.Current()
	if _, err := b.pair.Join(st.Code); err != nil {
		t.Fatal(err)
	}
	var c mesh.Conn
	select {
	case c = <-conns:
	case <-time.After(5 * time.Second):
		t.Fatal("b never connected to the thief")
	}
	ctx := context.Background()
	local, remote := c.Fingerprints()
	info := Info(thief.ident.DevicePub(), c.Pub(), local, remote, thief.id(), b.id())
	proof, _ := json.Marshal(frame{Type: frameProof, Proof: Proof(secret.Key, 'A', info), Name: "pc"})
	if err := c.Send(ctx, proof); err != nil {
		t.Fatal(err)
	}
	eventually(t, "b shows words", state(b, StateWords))
	add, err := thief.ident.CertFor(b.id(), hexKey(b.ident.DevicePub()), "mac", identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := thief.st.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	rows, err := personRows(ctx, thief.st)
	if err != nil {
		t.Fatal(err)
	}
	cur := thief.ident.Current()
	seed, _ := thief.ident.PersonSeed(cur.Epoch)
	grant, _ := json.Marshal(frame{Type: frameGrant, Person: &cur, Seed: seed, Rows: rows})
	if err := c.Send(ctx, grant); err != nil {
		t.Fatal(err)
	}
	time.Sleep(200 * time.Millisecond)
	if s := b.pair.Status(); s.State != StateWords || b.ident.Current().ID != before.ID {
		t.Fatalf("b took a grant before its key was typed: %+v", s)
	}
	// The person types the real A's key, twice: the window closes.
	for range MaxMisses {
		if err := b.pair.ConfirmKey(ctx, st.Fingerprint); !errors.Is(err, ErrWrongKey) {
			t.Fatalf("the real A's key against the thief: %v", err)
		}
	}
	if s := b.pair.Status(); s.State != StateFailed || s.Error != ErrTextNotThatKey {
		t.Fatalf("after two misses: %+v", s)
	}
	if b.ident.Current().ID != before.ID {
		t.Fatal("b adopted the thief's person")
	}
}

func hexKey(pub ed25519.PublicKey) string { return hex.EncodeToString(pub) }

// The person mistypes A's key twice on B: the window closes on both, and A
// wrote no cert for B, so B can simply pair again.
func TestWrongKeyTwiceLeavesNoCert(t *testing.T) {
	net := mesh.NewFakeNet()
	a, b := newDev(t, net, "pc", 0), newDev(t, net, "mac", 0)
	st, _ := a.pair.Add(identity.RoleDesktop)
	if _, err := b.pair.Join(st.Code); err != nil {
		t.Fatal(err)
	}
	eventually(t, "b shows words", state(b, StateWords))
	eventually(t, "a asks for the words", state(a, StateConfirm))
	if err := a.pair.Confirm(context.Background(), strings.Join(b.pair.Status().Words, " ")); err != nil {
		t.Fatal(err)
	}
	for range MaxMisses {
		if err := b.pair.ConfirmKey(context.Background(), "0000 0000 0000 0000"); !errors.Is(err, ErrWrongKey) {
			t.Fatal(err)
		}
	}
	eventually(t, "a's window closes", state(a, StateFailed))
	if _, ok := a.chain.Cert(b.id()); ok {
		t.Fatal("a certified b although b never took the pairing")
	}
	pairUp(t, a, b)
}
