package identity

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// fold is a person and a Chain fed rows directly (Observe), so rows from
// devices the store would refuse can be folded too.
type fold struct {
	t      *testing.T
	person ed25519.PrivateKey // the first key: personID is derived from it
	pid    string
	root   string
	devs   map[string]ed25519.PrivateKey
	seq    map[string]int64
	hlc    hlc.Timestamp
	ch     *Chain
}

func newFold(t *testing.T) *fold {
	t.Helper()
	pub, key, _ := ed25519.GenerateKey(rand.Reader)
	f := &fold{t: t, person: key, pid: PersonIDOf(pub), root: "root", devs: map[string]ed25519.PrivateKey{}, seq: map[string]int64{}}
	f.ch = NewChain(f.pid, f.root, "self")
	return f
}

func newKey() ed25519.PrivateKey {
	_, k, _ := ed25519.GenerateKey(rand.Reader)
	return k
}

func pubHex(k ed25519.PrivateKey) string { return hex.EncodeToString(k.Public().(ed25519.PublicKey)) }

func signHex(t *testing.T, k ed25519.PrivateKey, v any) string {
	t.Helper()
	msg, err := Canonical(v)
	if err != nil {
		t.Fatal(err)
	}
	return hex.EncodeToString(ed25519.Sign(k, msg))
}

func (f *fold) dev(id string) ed25519.PrivateKey {
	if f.devs[id] == nil {
		f.devs[id] = newKey()
	}
	return f.devs[id]
}

// row is writer's next row, signed by its device key.
func (f *fold) row(writer, kind string, payload any) store.Event {
	f.t.Helper()
	f.seq[writer]++
	f.hlc++
	raw, err := json.Marshal(payload)
	if err != nil {
		f.t.Fatal(err)
	}
	ev := store.Event{DeviceID: writer, Seq: f.seq[writer], HLC: f.hlc, Actor: "irori", Kind: kind, Payload: raw}
	ev.Sig = ed25519.Sign(f.dev(writer), store.SigningBytes(ev))
	return ev
}

// cert is writer certifying dev, signed by person key pk.
func (f *fold) cert(writer, dev string, pk ed25519.PrivateKey) store.Event {
	c := Cert{PersonID: f.pid, DeviceID: dev, DevicePub: pubHex(f.dev(dev)), Label: dev, Role: RoleDesktop, At: time.Unix(1, 0).UTC()}
	return f.row(writer, store.KindPersonDeviceAdded, DeviceAdded{Cert: c, PersonPub: pubHex(pk), PersonSig: signHex(f.t, pk, c)})
}

func (f *fold) revoke(writer, dev string, pk ed25519.PrivateKey) store.Event {
	r := Revoke{PersonID: f.pid, DeviceID: dev, Cut: 0, At: time.Unix(2, 0).UTC()}
	return f.row(writer, store.KindPersonDeviceRevoked, DeviceRevoked{Revoke: r, PersonPub: pubHex(pk), PersonSig: signHex(f.t, pk, r)})
}

func (f *fold) rotation(writer string, epoch int, pk ed25519.PrivateKey) store.Event {
	return f.row(writer, store.KindPersonKeyRotated, KeyRotated{PersonID: f.pid, Epoch: epoch, PersonPub: pubHex(pk), At: time.Unix(3, 0).UTC()})
}

func (f *fold) profile(writer, root string, pk ed25519.PrivateKey) store.Event {
	b := profileBody{PersonID: f.pid, Name: "Stephen", RootDeviceID: root, PersonPub: pubHex(pk)}
	return f.row(writer, store.KindPersonProfile, Profile{PersonID: b.PersonID, Name: b.Name, RootDeviceID: b.RootDeviceID, PersonPub: b.PersonPub, PersonSig: signHex(f.t, pk, b)})
}

func (f *fold) certified(dev string) bool {
	_, ok := f.ch.Cert(dev)
	return ok
}

// A cert's person signature counts only by one of the person's keys: a
// well-formed cert signed by some other key, written by a certified device,
// certifies nobody.
func TestCertByAnotherKeyCertifiesNobody(t *testing.T) {
	f := newFold(t)
	f.ch.Observe(f.cert("root", "root", f.person))
	if !f.certified("root") {
		t.Fatal("the root's self-cert by the first key didn't count")
	}
	f.ch.Observe(f.cert("root", "b", newKey()))
	if f.certified("b") || f.ch.IsOwner(UserKey("b")) {
		t.Fatal("a cert signed by a stranger's key certified b")
	}
	// A root self-cert signed by another key doesn't make a root either.
	g := newFold(t)
	g.ch.Observe(g.cert("root", "root", newKey()))
	if g.certified("root") {
		t.Fatal("a root self-cert by a stranger's key counted")
	}
}

// A rotated key counts once an ever-certified device's rotation row names
// it, in either arrival order; a rotation row from a device that was never
// certified names nothing.
func TestRotatedKeyCountsInEitherOrder(t *testing.T) {
	next := newKey()
	for _, order := range []string{"rotation first", "cert first"} {
		t.Run(order, func(t *testing.T) {
			f := newFold(t)
			rows := []store.Event{f.cert("root", "root", f.person)}
			rot := f.rotation("root", 2, next)
			certB := f.cert("root", "b", next)
			if order == "rotation first" {
				rows = append(rows, rot, certB)
			} else {
				rows = append(rows, certB, rot)
			}
			for _, ev := range rows {
				f.ch.Observe(ev)
			}
			if !f.certified("b") {
				t.Fatal("a cert by the rotated key didn't count")
			}
			// b's own rotation makes a third key count; c is certified by it.
			third := newKey()
			f.ch.Observe(f.cert("b", "c", third))
			if f.certified("c") {
				t.Fatal("c certified before any rotation named its key")
			}
			f.ch.Observe(f.rotation("b", 3, third))
			if !f.certified("c") {
				t.Fatal("b's rotation didn't make its key count")
			}
		})
	}

	f := newFold(t)
	f.ch.Observe(f.cert("root", "root", f.person))
	stranger := newKey()
	f.ch.Observe(f.rotation("x", 2, stranger)) // x was never certified
	f.ch.Observe(f.cert("root", "b", stranger))
	if f.certified("b") {
		t.Fatal("a rotation row from an uncertified device named a key")
	}
}

// A revoke counts only by one of the person's keys.
func TestRevokeByAnotherKeyIsNotHonoured(t *testing.T) {
	f := newFold(t)
	f.ch.Observe(f.cert("root", "root", f.person))
	f.ch.Observe(f.cert("root", "b", f.person))
	f.ch.Observe(f.revoke("b", "root", newKey()))
	if _, revoked := f.ch.Cut("root"); revoked {
		t.Fatal("a revoke signed by a stranger's key cut the root")
	}
	f.ch.Observe(f.revoke("b", "root", f.person))
	if _, revoked := f.ch.Cut("root"); !revoked {
		t.Fatal("a revoke by the person's key wasn't honoured")
	}
}

// A profile counts when its signature verifies by one of the person's
// keys, it names the pinned root, and a certified device wrote it.
func TestProfileIsChecked(t *testing.T) {
	f := newFold(t)
	f.ch.Observe(f.cert("root", "root", f.person))

	forged := f.profile("root", "root", f.person)
	var p Profile
	json.Unmarshal(forged.Payload, &p)
	p.Name = "someone else" // the signature no longer covers it
	forged.Payload, _ = json.Marshal(p)
	bad := f.ch.Bad()
	f.ch.Observe(forged)
	if f.ch.HasProfile() || f.ch.Bad() != bad+1 {
		t.Fatalf("a profile with a bad signature: has %v, bad %d", f.ch.HasProfile(), f.ch.Bad())
	}
	for name, ev := range map[string]store.Event{
		"a stranger's key":   f.profile("root", "root", newKey()),
		"another root":       f.profile("root", "other", f.person),
		"an uncertified dev": f.profile("x", "root", f.person),
	} {
		f.ch.Observe(ev)
		if f.ch.HasProfile() {
			t.Fatalf("a profile by %s counted", name)
		}
	}
	f.ch.Observe(f.profile("root", "root", f.person))
	if !f.ch.HasProfile() {
		t.Fatal("the person's profile didn't count")
	}
}

// Before the root's cert is held, VerifyRow takes a root self-cert only
// when the person's first key signed it: anyone can sign a row with a
// device key of their own, so that alone proves nothing.
func TestVerifyRowBootstrapNeedsThePersonKey(t *testing.T) {
	f := newFold(t)
	if err := f.ch.VerifyRow(f.cert("root", "root", newKey())); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("root self-cert by a stranger's person key: err = %v", err)
	}
	ev := f.cert("root", "root", f.person)
	var p DeviceAdded
	json.Unmarshal(ev.Payload, &p)
	p.PersonSig = signHex(t, f.person, Cert{PersonID: f.pid, DeviceID: "root"}) // signs something else
	f.seq["root"]--
	f.hlc--
	if err := f.ch.VerifyRow(f.row("root", store.KindPersonDeviceAdded, p)); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("root self-cert with a person signature over other bytes: err = %v", err)
	}
	good := f.cert("root", "root", f.person)
	if err := f.ch.Learn(good); err != nil {
		t.Fatalf("the real root self-cert: %v", err)
	}
	if !f.certified("root") {
		t.Fatal("learning the root self-cert didn't certify the root")
	}
}
