package identity

import (
	"bytes"
	"context"
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
	"github.com/StephenSHorton/irori/internal/store"
)

var ctx = context.Background()

func TestKeystoresRoundTrip(t *testing.T) {
	kek := make([]byte, 32)
	rand.Read(kek)
	stores := []Keystore{
		FileKeystore{},
		aesKeystore{name: "test-aes", kek: func() ([]byte, error) { return kek, nil }},
		DefaultKeystore(t.TempDir()),
	}
	if os.Getenv("IRORI_TEST_KEYCHAIN") == "" && DefaultKeystore("x").Name() == "keychain" {
		stores = stores[:2] // the real Keychain only on request; the AES wrap above covers the code
	}
	secret := []byte("0123456789abcdef0123456789abcdef")
	for _, ks := range stores {
		w, err := seal(ks, secret)
		if err != nil {
			t.Fatalf("%s: seal: %v", ks.Name(), err)
		}
		if ks.Name() != "file" && bytes.Contains(w.Blob, secret) {
			t.Fatalf("%s: sealed blob holds the secret in the clear", ks.Name())
		}
		got, err := unseal(ks, w)
		if err != nil || !bytes.Equal(got, secret) {
			t.Fatalf("%s: unseal = %q, %v", ks.Name(), got, err)
		}
		// A blob sealed by another keystore fails plainly.
		w.Keystore = "elsewhere"
		if _, err := unseal(ks, w); !errors.Is(err, ErrWrongKeystore) {
			t.Fatalf("%s: wrong keystore: err = %v", ks.Name(), err)
		}
	}
	// AES-GCM with a different key-encryption key can't open the blob.
	a := aesKeystore{name: "k", kek: func() ([]byte, error) { return kek, nil }}
	other := make([]byte, 32)
	rand.Read(other)
	b := aesKeystore{name: "k", kek: func() ([]byte, error) { return other, nil }}
	blob, _ := a.Seal(secret)
	if _, err := b.Unseal(blob); err == nil {
		t.Fatal("unsealed with the wrong key-encryption key")
	}
}

func TestLoadCreatesOnceAndRecordsKeysInDeviceJSON(t *testing.T) {
	dir := t.TempDir()
	dev, err := device.LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	ks := DefaultKeystore(dir)
	if ks.Name() == "keychain" && os.Getenv("IRORI_TEST_KEYCHAIN") == "" {
		ks = FileKeystore{}
	}
	a, err := Load(dir, dev, ks)
	if err != nil {
		t.Fatal(err)
	}
	if a.Person.RootDeviceID != dev.ID || a.Person.Epoch != 1 || a.Person.ID != PersonIDOf(mustHex(t, a.Person.Pub)) {
		t.Fatalf("new person %+v", a.Person)
	}
	reread, _ := device.LoadOrCreate(dir)
	if reread.PersonID != a.Person.ID || reread.DevicePub != hex.EncodeToString(a.DevicePub()) {
		t.Fatalf("device.json %+v", reread)
	}
	b, err := Load(dir, reread, ks)
	if err != nil {
		t.Fatal(err)
	}
	if b.Person != a.Person || !bytes.Equal(a.DevicePub(), b.DevicePub()) {
		t.Fatal("second Load made new keys")
	}
	// The secrets on disk are wrapped files naming their keystore.
	for _, f := range []string{DeviceKeyFile, PersonFile} {
		raw, _ := os.ReadFile(filepath.Join(dir, f))
		if !strings.Contains(string(raw), `"keystore": "`+ks.Name()+`"`) {
			t.Fatalf("%s does not name its keystore: %s", f, raw)
		}
	}
	// A data dir sealed by another keystore fails plainly.
	if _, err := Load(dir, reread, aesKeystore{name: "other", kek: func() ([]byte, error) { return make([]byte, 32), nil }}); !errors.Is(err, ErrWrongKeystore) {
		t.Fatalf("load with another keystore: err = %v", err)
	}
	// device.json naming another key is a mixed data dir.
	reread.DevicePub = strings.Repeat("00", 32)
	if _, err := Load(dir, reread, ks); err == nil {
		t.Fatal("loaded a device.json whose key differs from device.key")
	}
}

func mustHex(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestCanonicalSortsKeys(t *testing.T) {
	got, err := Canonical(Cert{PersonID: "p", DeviceID: "d", DevicePub: "k", Label: "a<b", Role: RoleDesktop, At: time.Date(2026, 10, 27, 0, 0, 0, 0, time.UTC)})
	if err != nil {
		t.Fatal(err)
	}
	want := `{"at":"2026-10-27T00:00:00Z","deviceId":"d","devicePub":"k","label":"a<b","personId":"p","role":"desktop"}`
	if string(got) != want {
		t.Fatalf("canonical = %s\nwant        %s", got, want)
	}
}

// mesh is one person's devices, each a real Identity with its own store,
// signed and verified, and no transport: rows are copied by hand.
type mesh struct {
	t    *testing.T
	root *node
}

type node struct {
	id    *Identity
	chain *Chain
	st    *store.Store
	clock *hlc.Clock
}

func newNode(t *testing.T, dir string, p *Identity) *node {
	t.Helper()
	dev, err := device.LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	if p != nil {
		// Joining p's person (what pairing will do in release 3): copy
		// person.json, so the root stays p's.
		raw, err := os.ReadFile(filepath.Join(p.dir(t), PersonFile))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, PersonFile), raw, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	id, err := Load(dir, dev, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	ch := NewChain(id.Person.ID, id.Person.RootDeviceID, id.Device.ID)
	clock := hlc.New(nil)
	st, err := store.OpenWith(ctx, filepath.Join(dir, "irori.db"), id.Device.ID, clock, store.OpenOptions{Signer: id, Verifier: ch})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	if err := id.EnsureSelfCert(ctx, st, ch, RoleDesktop); err != nil {
		t.Fatal(err)
	}
	return &node{id: id, chain: ch, st: st, clock: clock}
}

// certify has n write a cert for o.
func (n *node) certify(t *testing.T, o *node) store.Event {
	t.Helper()
	add, err := n.id.CertFor(o.id.Device.ID, hex.EncodeToString(o.id.DevicePub()), "dev", RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	ev, err := n.st.Append(ctx, "irori", store.KindPersonDeviceAdded, add)
	if err != nil {
		t.Fatal(err)
	}
	return ev
}

func (n *node) revoke(t *testing.T, devID string, cut int64) store.Event {
	t.Helper()
	r, err := n.id.RevokeOf(devID, cut, "", "test", time.Now())
	if err != nil {
		t.Fatal(err)
	}
	ev, err := n.st.Append(ctx, "irori", store.KindPersonDeviceRevoked, r)
	if err != nil {
		t.Fatal(err)
	}
	return ev
}

// copyRows copies every row src holds that dst lacks, person rows first, and
// returns the first refusal.
func copyRows(t *testing.T, src, dst *node) error {
	t.Helper()
	v, err := dst.st.Vector(ctx)
	if err != nil {
		t.Fatal(err)
	}
	evs, err := src.st.Missing(ctx, v, store.MaxMissingLimit)
	if err != nil {
		t.Fatal(err)
	}
	// Person rows first, learned (not inserted: seqs are gapless), retried
	// while a cert they need is still to come.
	var person []store.Event
	for _, ev := range evs {
		if store.IsPersonKind(ev.Kind) {
			person = append(person, ev)
		}
	}
	for progress := true; progress; {
		progress = false
		left := person[:0:0]
		for _, ev := range person {
			if err := dst.chain.Learn(ev); err != nil {
				left = append(left, ev)
			} else {
				progress = true
			}
		}
		person = left
	}
	for _, ev := range evs {
		if err := dst.st.Insert(ctx, ev); err != nil {
			return err
		}
	}
	return nil
}

func TestRootSelfCertIsPinned(t *testing.T) {
	a := newNode(t, t.TempDir(), nil)
	if _, ok := a.chain.Cert(a.id.Device.ID); !ok || !a.chain.HasProfile() {
		t.Fatal("the root did not certify itself")
	}
	if !a.chain.IsOwner(UserKey(a.id.Device.ID)) || a.chain.IsOwner("user:made-up") || a.chain.IsOwner(a.id.Device.ID+"/s") {
		t.Fatal("IsOwner wrong on the root")
	}
	// Restarting writes no second self-cert.
	n := len(personRows(t, a))
	a.st.Close()
	a.chain = NewChain(a.id.Person.ID, a.id.Person.RootDeviceID, a.id.Device.ID)
	st, err := store.OpenWith(ctx, filepath.Join(a.id.dataDir, "irori.db"), a.id.Device.ID, hlc.New(nil),
		store.OpenOptions{Signer: a.id, Verifier: a.chain})
	if err != nil {
		t.Fatal(err)
	}
	a.st = st
	t.Cleanup(func() { st.Close() })
	if _, ok := a.chain.Cert(a.id.Device.ID); !ok {
		t.Fatal("the chain did not fold the held self-cert at Open")
	}
	if err := a.id.EnsureSelfCert(ctx, a.st, a.chain, RoleDesktop); err != nil {
		t.Fatal(err)
	}
	if len(personRows(t, a)) != n {
		t.Fatal("restart wrote another self-cert")
	}
}

func personRows(t *testing.T, n *node) []store.Event {
	t.Helper()
	evs, err := n.st.List(ctx, store.Filter{Kinds: []string{"person."}})
	if err != nil {
		t.Fatal(err)
	}
	return evs
}

func TestCertChainAndSync(t *testing.T) {
	a := newNode(t, t.TempDir(), nil)
	b := newNode(t, t.TempDir(), a.id)
	if _, ok := b.chain.Cert(b.id.Device.ID); ok {
		t.Fatal("a non-root device certified itself")
	}
	bWork, _ := b.st.Append(ctx, "agent:"+b.id.Device.ID+"/s", "task.created", map[string]string{"taskId": "t"})

	// B's rows are refused by A until A certifies B: "no cert yet".
	if err := copyRows(t, b, a); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("uncertified rows: err = %v, want ErrNoCert", err)
	}
	a.certify(t, b)
	if !a.chain.IsOwner(UserKey(b.id.Device.ID)) {
		t.Fatal("certified device's owner key not accepted")
	}
	if err := copyRows(t, b, a); err != nil {
		t.Fatalf("certified rows refused: %v", err)
	}
	if err := copyRows(t, a, b); err != nil {
		t.Fatalf("a's rows refused on b: %v", err)
	}
	if _, ok := b.chain.Cert(b.id.Device.ID); !ok {
		t.Fatal("b did not learn its own cert from a")
	}
	got, _ := a.st.List(ctx, store.Filter{DeviceID: b.id.Device.ID, Kinds: []string{"task.created"}})
	if len(got) != 1 || !bytes.Equal(got[0].Sig, bWork.Sig) {
		t.Fatalf("b's earlier row on a: %+v", got)
	}

	// A forged row: right device, wrong key.
	forged := store.Event{DeviceID: b.id.Device.ID, Seq: bWork.Seq + 1, HLC: bWork.HLC + 1<<16, Actor: "user", Kind: "task.created", Payload: json.RawMessage(`{}`)}
	forged.Sig, _ = a.id.SignRow(store.SigningBytes(forged))
	if err := a.st.Insert(ctx, forged); !errors.Is(err, store.ErrBadSig) {
		t.Fatalf("forged row: err = %v, want ErrBadSig", err)
	}
	// An agent row relabelled as the person fails the signature too.
	relabel := got[0]
	relabel.Seq, relabel.Actor = bWork.Seq+1, "user"
	if err := a.st.Insert(ctx, relabel); !errors.Is(err, store.ErrBadSig) {
		t.Fatalf("relabelled actor: err = %v, want ErrBadSig", err)
	}
	unsigned := forged
	unsigned.Sig = nil
	if err := a.st.Insert(ctx, unsigned); !errors.Is(err, store.ErrUnsigned) {
		t.Fatalf("unsigned row: err = %v, want ErrUnsigned", err)
	}
	// A device the root never certified, signing its own rows, is refused.
	c := newNode(t, t.TempDir(), a.id)
	cRow, _ := c.st.Append(ctx, "irori", "task.created", map[string]string{})
	if err := a.st.Insert(ctx, cRow); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("uncertified device: err = %v", err)
	}
	if a.chain.IsOwner(UserKey(c.id.Device.ID)) {
		t.Fatal("uncertified device's owner key accepted")
	}
}

// A self-cert from a device that isn't the pinned root is refused even with
// a valid person signature: a revoked device holding the person secret
// can't mint a fresh root.
func TestSelfCertOtherThanTheRootIsRefused(t *testing.T) {
	a := newNode(t, t.TempDir(), nil)
	x := newNode(t, t.TempDir(), a.id)
	add, err := x.id.CertFor(x.id.Device.ID, hex.EncodeToString(x.id.DevicePub()), "thief", RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	ev, err := x.st.Append(ctx, "irori", store.KindPersonDeviceAdded, add)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := x.chain.Cert(x.id.Device.ID); ok {
		t.Fatal("a non-root self-cert certified its writer")
	}
	if err := a.st.Insert(ctx, ev); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("non-root self-cert on the root: err = %v, want ErrNoCert", err)
	}
	// A device can't certify itself through a device it certified either.
	if a.chain.IsOwner(UserKey(x.id.Device.ID)) {
		t.Fatal("non-root self-cert made an owner key")
	}
}

func TestRevokeCutsAndMovesRows(t *testing.T) {
	a := newNode(t, t.TempDir(), nil)
	b := newNode(t, t.TempDir(), a.id)
	a.certify(t, b)
	if err := copyRows(t, a, b); err != nil {
		t.Fatal(err)
	}
	var bRows []store.Event
	for i := range 4 {
		ev, err := b.st.Append(ctx, "irori", "task.created", map[string]int{"i": i})
		if err != nil {
			t.Fatal(err)
		}
		bRows = append(bRows, ev)
	}
	if err := copyRows(t, b, a); err != nil {
		t.Fatal(err)
	}
	sub := a.st.Subscribe(ctx)
	cut := bRows[1].Seq
	a.revoke(t, b.id.Device.ID, cut)
	if c, ok := a.chain.Cut(b.id.Device.ID); !ok || c != cut {
		t.Fatalf("cut = %d %v, want %d", c, ok, cut)
	}
	// Rows above the cut moved to refused_events and folds got a resync.
	v, _ := a.st.Vector(ctx)
	if v[b.id.Device.ID] != cut {
		t.Fatalf("vector %v, want b capped at %d", v, cut)
	}
	refused, _ := a.st.Refused(ctx, 0)
	if len(refused) != 2 || refused[0].Seq != cut+1 {
		t.Fatalf("refused %+v", refused)
	}
	sawResync := false
	for len(sub) > 0 {
		if ev := <-sub; ev.Seq == 0 {
			sawResync = true
		}
	}
	if !sawResync {
		t.Fatal("no resync after rows moved out")
	}
	// New rows past the cut are refused; revoked devices stay owners (their
	// stored rows are at or below the cut).
	if _, err := b.st.Append(ctx, "irori", "task.created", nil); err != nil {
		t.Fatal(err)
	}
	if err := copyRows(t, b, a); !errors.Is(err, store.ErrPastCut) {
		t.Fatalf("row past cut: err = %v, want ErrPastCut", err)
	}
	if !a.chain.IsOwner(UserKey(b.id.Device.ID)) {
		t.Fatal("revoked device's owner key dropped")
	}
}

// Mutual revoke: both devices end up revoked whichever row arrives first.
func TestMutualRevokeInEitherOrder(t *testing.T) {
	for _, order := range []string{"pc-first", "laptop-first"} {
		t.Run(order, func(t *testing.T) {
			root := newNode(t, t.TempDir(), nil)
			pc := newNode(t, t.TempDir(), root.id)
			lap := newNode(t, t.TempDir(), root.id)
			root.certify(t, pc)
			root.certify(t, lap)
			for _, n := range []*node{pc, lap} {
				if err := copyRows(t, root, n); err != nil {
					t.Fatal(err)
				}
			}
			rp := pc.revoke(t, lap.id.Device.ID, 0)
			rl := lap.revoke(t, pc.id.Device.ID, 0)
			evs := []store.Event{rp, rl}
			if order == "laptop-first" {
				evs = []store.Event{rl, rp}
			}
			for _, ev := range evs {
				if err := root.st.Insert(ctx, ev); err != nil {
					t.Fatal(err)
				}
			}
			_, pcRev := root.chain.Cut(pc.id.Device.ID)
			_, lapRev := root.chain.Cut(lap.id.Device.ID)
			if !pcRev || !lapRev {
				t.Fatalf("pc revoked %v, laptop revoked %v; want both", pcRev, lapRev)
			}
			// The revoke kept past its writer's cut survives a restart.
			root.st.Close()
			ch := NewChain(root.id.Person.ID, root.id.Person.RootDeviceID, root.id.Device.ID)
			st, err := store.OpenWith(ctx, filepath.Join(root.id.dataDir, "irori.db"), root.id.Device.ID, hlc.New(nil), store.OpenOptions{Signer: root.id, Verifier: ch})
			if err != nil {
				t.Fatal(err)
			}
			defer st.Close()
			_, pcRev = ch.Cut(pc.id.Device.ID)
			_, lapRev = ch.Cut(lap.id.Device.ID)
			if !pcRev || !lapRev {
				t.Fatalf("after restart: pc revoked %v, laptop revoked %v", pcRev, lapRev)
			}
		})
	}
}

func TestMinCutWins(t *testing.T) {
	root := newNode(t, t.TempDir(), nil)
	b := newNode(t, t.TempDir(), root.id)
	c := newNode(t, t.TempDir(), root.id)
	root.certify(t, b)
	root.certify(t, c)
	if err := copyRows(t, root, c); err != nil {
		t.Fatal(err)
	}
	r1 := root.revoke(t, b.id.Device.ID, 5)
	r2 := c.revoke(t, b.id.Device.ID, 3)
	if err := c.st.Insert(ctx, r1); err != nil {
		t.Fatal(err)
	}
	if err := root.st.Insert(ctx, r2); err != nil {
		t.Fatal(err)
	}
	for _, n := range []*node{root, c} {
		if cut, _ := n.chain.Cut(b.id.Device.ID); cut != 3 {
			t.Fatalf("effective cut %d, want 3", cut)
		}
	}
}

func TestOwnRowsComeBackAndFutureRowsWait(t *testing.T) {
	a := newNode(t, t.TempDir(), nil)
	dir := a.id.dataDir
	for range 3 {
		if _, err := a.st.Append(ctx, "irori", "task.created", nil); err != nil {
			t.Fatal(err)
		}
	}
	held, _ := a.st.Missing(ctx, store.Vector{}, 0)
	a.st.Close()
	// Restore an old copy: same keys, an empty log (the data dir restored
	// without its database).
	os.Remove(filepath.Join(dir, "irori.db"))
	os.Remove(filepath.Join(dir, "irori.db-wal"))
	os.Remove(filepath.Join(dir, "irori.db-shm"))
	ch := NewChain(a.id.Person.ID, a.id.Person.RootDeviceID, a.id.Device.ID)
	st, err := store.OpenWith(ctx, filepath.Join(dir, "irori.db"), a.id.Device.ID, hlc.New(nil), store.OpenOptions{Signer: a.id, Verifier: ch})
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	st.HoldAppends(int64(len(held)))
	if _, err := st.Append(ctx, "irori", "task.created", nil); !errors.Is(err, store.ErrRestoring) {
		t.Fatalf("append before own rows came back: err = %v", err)
	}
	if n, err := st.InsertBatch(ctx, held); err != nil || n != len(held) {
		t.Fatalf("own rows back: %d, %v", n, err)
	}
	if _, err := st.Append(ctx, "irori", "task.created", nil); err != nil {
		t.Fatalf("append after catching up: %v", err)
	}

	// A row stamped 10 minutes ahead waits (not stored, clock untouched).
	b := newNode(t, t.TempDir(), a.id)
	add, _ := a.id.CertFor(b.id.Device.ID, hex.EncodeToString(b.id.DevicePub()), "b", RoleDesktop, time.Now())
	if _, err := st.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	ahead := store.Event{DeviceID: b.id.Device.ID, Seq: 1, HLC: hlc.Timestamp(time.Now().Add(10*time.Minute).UnixMilli()) << 16,
		Actor: "irori", Kind: "task.created", Payload: json.RawMessage(`{}`)}
	ahead.Sig, _ = b.id.SignRow(store.SigningBytes(ahead))
	if err := st.Insert(ctx, ahead); !errors.Is(err, store.ErrAhead) {
		t.Fatalf("row 10 min ahead: err = %v, want ErrAhead", err)
	}
	next, _ := st.Append(ctx, "irori", "task.created", nil)
	if next.HLC >= ahead.HLC {
		t.Fatal("a held future row moved the clock")
	}
}

// dir is the data dir the test node lives in.
func (id *Identity) dir(t *testing.T) string {
	t.Helper()
	if id.dataDir == "" {
		t.Fatal("identity has no data dir")
	}
	return id.dataDir
}
