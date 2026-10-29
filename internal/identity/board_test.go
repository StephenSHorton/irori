package identity

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/json"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// bfold is a board Verifier fed rows directly (Observe), with any number
// of people, so rows the store would refuse can be folded too.
type bfold struct {
	t    *testing.T
	ppl  map[string]*bperson
	devs map[string]ed25519.PrivateKey
	seq  map[string]int64
	hlc  hlc.Timestamp
	v    *BoardVerifier
}

// bperson is one person: its keys by epoch, its pin's root.
type bperson struct {
	pid  string
	root string
	keys map[int]ed25519.PrivateKey
}

func newBFold(t *testing.T) *bfold {
	return &bfold{t: t, ppl: map[string]*bperson{}, devs: map[string]ed25519.PrivateKey{}, seq: map[string]int64{}, v: NewBoardVerifier()}
}

// person makes a person rooted at root and pins it.
func (f *bfold) person(name, root string) *bperson {
	f.t.Helper()
	pub, key, _ := ed25519.GenerateKey(rand.Reader)
	p := &bperson{pid: PersonIDOf(pub), root: root, keys: map[int]ed25519.PrivateKey{1: key}}
	f.ppl[name] = p
	if err := f.v.Pin(MemberPin{PersonID: p.pid, PersonPub: pubHex(key), RootDeviceID: root, PersonEpoch: 1}); err != nil {
		f.t.Fatal(err)
	}
	return p
}

func (f *bfold) dev(id string) ed25519.PrivateKey {
	if f.devs[id] == nil {
		f.devs[id] = newKey()
	}
	return f.devs[id]
}

// row is writer's next row as actor, signed by its device key.
func (f *bfold) row(writer, actor, kind string, payload any) store.Event {
	f.t.Helper()
	f.seq[writer]++
	f.hlc++
	raw, err := json.Marshal(payload)
	if err != nil {
		f.t.Fatal(err)
	}
	ev := store.Event{DeviceID: writer, Seq: f.seq[writer], HLC: f.hlc, Actor: actor, Kind: kind, Payload: raw}
	ev.Sig = ed25519.Sign(f.dev(writer), store.SigningBytes(ev))
	return ev
}

// cert is writer certifying dev for p, signed by p's key of epoch.
func (f *bfold) cert(p *bperson, epoch int, writer, dev string) store.Event {
	c := MemberCert{PersonID: p.pid, DeviceID: dev, DevicePub: pubHex(f.dev(dev)), Role: RoleDesktop, PersonEpoch: epoch, At: time.Unix(1, 0).UTC()}
	return f.row(writer, "irori", store.KindMemberDeviceAdded, MemberDeviceAdded{Cert: c, PersonSig: signHex(f.t, p.keys[epoch], c)})
}

func (f *bfold) revoke(p *bperson, epoch int, writer, dev string, cut int64) store.Event {
	r := MemberRevoke{PersonID: p.pid, DeviceID: dev, BoardCut: cut, PersonEpoch: epoch, At: time.Unix(2, 0).UTC()}
	return f.row(writer, "irori", store.KindMemberDeviceRevoked, MemberDeviceRevoked{Revoke: r, PersonSig: signHex(f.t, p.keys[epoch], r)})
}

// rotate is by rotating p to epoch with a new key (kept in p.keys only
// when keep, so two rotations to one epoch each get theirs), prevSig by
// prev's key.
func (f *bfold) rotate(p *bperson, prev, epoch int, by string) (store.Event, ed25519.PrivateKey) {
	k := newKey()
	r := MemberKeyRotated{PersonID: p.pid, PersonEpoch: epoch, PersonPub: pubHex(k), By: by, At: time.Unix(3, 0).UTC()}
	r.PrevSig = signHex(f.t, p.keys[prev], r.body())
	return f.row(by, "irori", store.KindMemberKeyRotated, r), k
}

func (f *bfold) feed(evs ...store.Event) {
	for _, ev := range evs {
		f.v.Observe(ev)
	}
}

func (f *bfold) certified(dev string) bool {
	_, ok := f.v.Cert(dev)
	return ok
}

// orders runs check over every permutation of evs, each into a fresh
// Verifier with the same pins (§3: the fold doesn't depend on arrival
// order).
func (f *bfold) orders(evs []store.Event, check func(v *BoardVerifier)) {
	f.t.Helper()
	var pins []MemberPin
	for _, p := range f.ppl {
		pin, _ := f.v.Pinned(p.pid)
		pins = append(pins, pin)
	}
	perm(len(evs), func(idx []int) {
		v := NewBoardVerifier(pins...)
		for _, i := range idx {
			v.Observe(evs[i])
		}
		check(v)
	})
}

func perm(n int, f func([]int)) {
	idx := make([]int, n)
	for i := range idx {
		idx[i] = i
	}
	var rec func(k int)
	rec = func(k int) {
		if k == n {
			f(append([]int(nil), idx...))
			return
		}
		for i := k; i < n; i++ {
			idx[k], idx[i] = idx[i], idx[k]
			rec(k + 1)
			idx[k], idx[i] = idx[i], idx[k]
		}
	}
	rec(0)
}

// A cert counts only when its writer chains to the pinned root and its
// personSig verifies by the member's key for the epoch it names; the root
// self-cert verifies from the pin alone, and a device can't certify itself.
func TestBoardCertChainsToThePin(t *testing.T) {
	f := newBFold(t)
	s := f.person("stephen", "pc")
	root := f.cert(s, 1, "pc", "pc")
	if err := f.v.VerifyRow(root); err != nil {
		t.Fatalf("the pinned root's self-cert doesn't verify from the pin: %v", err)
	}
	mac := f.cert(s, 1, "pc", "mac")
	if err := f.v.VerifyRow(mac); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("pc's row before its cert folded: err = %v", err)
	}
	self := f.cert(s, 1, "rogue", "rogue")
	wrongKey := func() store.Event {
		c := MemberCert{PersonID: s.pid, DeviceID: "laptop", DevicePub: pubHex(f.dev("laptop")), Role: RoleDesktop, PersonEpoch: 1}
		return f.row("pc", "irori", store.KindMemberDeviceAdded, MemberDeviceAdded{Cert: c, PersonSig: signHex(t, newKey(), c)})
	}()
	wrongEpoch := f.cert(s, 1, "pc", "node")
	{ // the same cert naming an epoch the board doesn't know
		var p MemberDeviceAdded
		_ = json.Unmarshal(wrongEpoch.Payload, &p)
		p.Cert.PersonEpoch = 2
		p.PersonSig = signHex(t, s.keys[1], p.Cert)
		f.seq["pc"]--
		f.hlc--
		wrongEpoch = f.row("pc", "irori", store.KindMemberDeviceAdded, p)
	}
	f.feed(root, mac, self, wrongKey, wrongEpoch)
	switch {
	case !f.certified("pc") || !f.certified("mac"):
		t.Fatal("pc and mac chain to the pin")
	case f.certified("rogue"):
		t.Fatal("a device certified itself")
	case f.certified("laptop"):
		t.Fatal("a cert signed by another key counted")
	case f.certified("node"):
		t.Fatal("a cert naming an unknown epoch counted")
	}
	if err := f.v.VerifyRow(f.row("mac", "irori", store.KindMemberProfile, MemberProfile{PersonID: s.pid})); err != nil {
		t.Fatalf("mac's row: %v", err)
	}
	forged := f.row("mac", "irori", store.KindMemberProfile, MemberProfile{PersonID: s.pid})
	forged.Sig = ed25519.Sign(newKey(), store.SigningBytes(forged))
	if err := f.v.VerifyRow(forged); !errors.Is(err, store.ErrBadSig) {
		t.Fatalf("a row mac's key didn't sign: err = %v", err)
	}
	// A self-cert for the root by a key that isn't the pinned one is no
	// bootstrap.
	g := newBFold(t)
	b := g.person("blake", "bpc")
	c := MemberCert{PersonID: b.pid, DeviceID: "bpc", DevicePub: pubHex(g.dev("bpc")), PersonEpoch: 1}
	if err := g.v.VerifyRow(g.row("bpc", "irori", store.KindMemberDeviceAdded, MemberDeviceAdded{Cert: c, PersonSig: signHex(t, newKey(), c)})); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("root self-cert by another key: err = %v", err)
	}
}

// Rows signed before a rotation stay valid; a new object signed with the
// older key after it is refused, in every arrival order.
func TestBoardOlderEpochAfterRotationIsRefused(t *testing.T) {
	f := newBFold(t)
	s := f.person("stephen", "pc")
	root := f.cert(s, 1, "pc", "pc")
	mac := f.cert(s, 1, "pc", "mac")
	rot, k2 := f.rotate(s, 1, 2, "pc")
	s.keys[2] = k2
	stale := f.cert(s, 1, "mac", "laptop") // epoch 1 key, after the rotation
	fresh := f.cert(s, 2, "mac", "node")
	f.orders([]store.Event{root, mac, rot, stale, fresh}, func(v *BoardVerifier) {
		if _, ok := v.Cert("mac"); !ok {
			t.Fatal("mac, certified before the rotation, stopped counting")
		}
		if _, ok := v.Cert("laptop"); ok {
			t.Fatal("a cert signed by the ended epoch's key after its end counted")
		}
		if _, ok := v.Cert("node"); !ok || v.Epoch(s.pid) != 2 {
			t.Fatalf("the epoch 2 cert: epoch %d", v.Epoch(s.pid))
		}
	})
}

// A rotation counts only with a prevSig by the previous epoch's key and
// written by its By device, which must hold a counted cert.
func TestBoardRotationNeedsPrevSigAndItsDevice(t *testing.T) {
	f := newBFold(t)
	s := f.person("stephen", "pc")
	f.feed(f.cert(s, 1, "pc", "pc"), f.cert(s, 1, "pc", "mac"))

	k := newKey()
	bad := MemberKeyRotated{PersonID: s.pid, PersonEpoch: 2, PersonPub: pubHex(k), By: "mac", At: time.Unix(3, 0).UTC()}
	bad.PrevSig = signHex(t, newKey(), bad.body()) // not epoch 1's key
	f.feed(f.row("mac", "irori", store.KindMemberKeyRotated, bad))
	if f.v.Epoch(s.pid) != 1 {
		t.Fatal("a rotation with a forged prevSig counted")
	}
	other := MemberKeyRotated{PersonID: s.pid, PersonEpoch: 2, PersonPub: pubHex(k), By: "pc", At: time.Unix(3, 0).UTC()}
	other.PrevSig = signHex(t, s.keys[1], other.body())
	f.feed(f.row("mac", "irori", store.KindMemberKeyRotated, other)) // By names pc, mac wrote it
	if f.v.Epoch(s.pid) != 1 {
		t.Fatal("a rotation written by a device other than its By counted")
	}
	stranger, _ := f.rotate(s, 1, 2, "stranger") // valid prevSig, uncertified device
	f.feed(stranger)
	if f.v.Epoch(s.pid) != 1 {
		t.Fatal("a rotation by an uncertified device counted")
	}
	good, _ := f.rotate(s, 1, 2, "mac")
	f.feed(good)
	if f.v.Epoch(s.pid) != 2 {
		t.Fatal("a valid rotation by mac didn't count")
	}
}

// Two rotations to one epoch take the one whose By is the lower device id,
// in every order.
func TestBoardTwoRotationsTakeTheLowerBy(t *testing.T) {
	f := newBFold(t)
	s := f.person("stephen", "a-pc")
	root := f.cert(s, 1, "a-pc", "a-pc")
	mac := f.cert(s, 1, "a-pc", "b-mac")
	fromMac, macKey := f.rotate(s, 1, 2, "b-mac")
	fromPC, pcKey := f.rotate(s, 1, 2, "a-pc")
	s.keys[2] = pcKey
	byPC := f.cert(s, 2, "a-pc", "node")
	s.keys[2] = macKey
	byMac := f.cert(s, 2, "b-mac", "laptop")
	f.orders([]store.Event{root, mac, fromMac, fromPC, byPC, byMac}, func(v *BoardVerifier) {
		if v.Epoch(s.pid) != 2 {
			t.Fatalf("epoch %d", v.Epoch(s.pid))
		}
		if _, ok := v.Cert("node"); !ok {
			t.Fatal("a-pc's key, the lower By, didn't win epoch 2")
		}
		if _, ok := v.Cert("laptop"); ok {
			t.Fatal("b-mac's key, the higher By, signed a cert that counted")
		}
	})
}

// A thief that rotates first and is then revoked doesn't win the key: its
// rotation is discarded wherever it sits, and the revoking device's
// rotation stands, in every order.
func TestBoardRotationByARevokedDeviceIsDiscarded(t *testing.T) {
	f := newBFold(t)
	s := f.person("stephen", "pc")
	root := f.cert(s, 1, "pc", "pc")
	thiefCert := f.cert(s, 1, "pc", "a-thief") // a lower id than pc, so it would win a tie
	thiefRot, thiefKey := f.rotate(s, 1, 2, "a-thief")
	rev := f.revoke(s, 1, "pc", "a-thief", 0)
	pcRot, pcKey := f.rotate(s, 1, 2, "pc")
	s.keys[2] = pcKey
	node := f.cert(s, 2, "pc", "node")
	s.keys[2] = thiefKey
	fake := f.cert(s, 2, "a-thief", "fake")
	f.orders([]store.Event{root, thiefCert, thiefRot, rev, pcRot, node, fake}, func(v *BoardVerifier) {
		if _, ok := v.Cut("a-thief"); !ok {
			t.Fatal("the thief isn't revoked")
		}
		if _, ok := v.Cert("node"); !ok {
			t.Fatal("pc's rotation didn't win epoch 2")
		}
		if _, ok := v.Cert("fake"); ok {
			t.Fatal("the thief's key certified a device")
		}
		if !v.Writable(s.pid) {
			t.Fatal("the person can't write after pc rotated")
		}
	})
}

// A thief and your device that revoke each other: both rotations are
// discarded and the person can sign nothing new on the board.
func TestBoardMutualRevokeFreezesThePerson(t *testing.T) {
	f := newBFold(t)
	s := f.person("stephen", "pc")
	root := f.cert(s, 1, "pc", "pc")
	thiefCert := f.cert(s, 1, "pc", "thief")
	thiefRot, thiefKey := f.rotate(s, 1, 2, "thief")
	mine := f.revoke(s, 1, "pc", "thief", 0)
	pcRot, _ := f.rotate(s, 1, 2, "pc")
	s.keys[2] = thiefKey
	theirs := f.revoke(s, 2, "thief", "pc", 1)
	late := f.cert(s, 1, "pc", "late") // the old key, after both rotations
	f.orders([]store.Event{root, thiefCert, thiefRot, mine, pcRot, theirs, late}, func(v *BoardVerifier) {
		_, a := v.Cut("thief")
		_, b := v.Cut("pc")
		if !a || !b {
			t.Fatalf("mutual revoke: thief %v, pc %v", a, b)
		}
		if v.Epoch(s.pid) != 1 || v.Writable(s.pid) {
			t.Fatalf("epoch %d writable %v: both rotations should be discarded", v.Epoch(s.pid), v.Writable(s.pid))
		}
		if _, ok := v.Cert("late"); ok {
			t.Fatal("a new cert counted after both rotations were discarded")
		}
	})
}

// A revoked device's rows above its per-board cut are refused by the store;
// those at or below it stay. A member can't revoke another member's device.
func TestBoardRevokeCutsInTheStore(t *testing.T) {
	ctx := t.Context()
	f := newBFold(t)
	s := f.person("stephen", "pc")
	b := f.person("blake", "bpc")
	root := f.cert(s, 1, "pc", "pc")
	mac := f.cert(s, 1, "pc", "mac")
	m1 := f.row("mac", "irori", store.KindMemberProfile, MemberProfile{PersonID: s.pid, Name: "x"})
	m2 := f.row("mac", "irori", store.KindMemberProfile, MemberProfile{PersonID: s.pid, Name: "y"})
	broot := f.cert(b, 1, "bpc", "bpc")
	// Blake revokes Stephen's pc with his own key: it names a device that
	// isn't his.
	pins := []MemberPin{}
	for _, p := range []*bperson{s, b} {
		pin, _ := f.v.Pinned(p.pid)
		pins = append(pins, pin)
	}
	v := NewBoardVerifier(pins...)
	st, err := store.OpenWith(ctx, filepath.Join(t.TempDir(), "b.db"), "self", hlc.New(nil), store.OpenOptions{Verifier: v, Board: true})
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	for _, ev := range []store.Event{root, mac, m1, broot} {
		if err := st.Insert(ctx, ev); err != nil {
			t.Fatalf("%s/%d: %v", ev.DeviceID, ev.Seq, err)
		}
	}
	rev := f.revoke(s, 1, "pc", "mac", 1)
	if err := st.Insert(ctx, rev); err != nil {
		t.Fatal(err)
	}
	if cut, ok := v.Cut("mac"); !ok || cut != 1 {
		t.Fatalf("mac's cut %d %v", cut, ok)
	}
	if err := st.Insert(ctx, m2); !errors.Is(err, store.ErrPastCut) {
		t.Fatalf("mac's row above its cut: err = %v", err)
	}
	if err := st.Insert(ctx, f.revoke(b, 1, "bpc", "pc", 0)); err != nil {
		t.Fatal(err)
	}
	if _, ok := v.Cut("pc"); ok {
		t.Fatal("blake's revoke cut stephen's device")
	}
	// A device certified by both people counts for neither, unless it is
	// one's pinned root.
	if err := st.Insert(ctx, f.cert(b, 1, "bpc", "pc")); err != nil {
		t.Fatal(err)
	}
	if p, ok := v.PersonOf("pc"); !ok || p != s.pid {
		t.Fatalf("stephen's pinned root went to %q", p)
	}
	if err := st.Insert(ctx, f.cert(b, 1, "bpc", "shared")); err != nil {
		t.Fatal(err)
	}
	if err := st.Insert(ctx, f.cert(s, 1, "pc", "shared")); err != nil {
		t.Fatal(err)
	}
	if _, ok := v.PersonOf("shared"); ok || len(v.Ambiguous()) != 2 {
		t.Fatalf("a device both certified counts for someone; ambiguous %v", v.Ambiguous())
	}
}

// Every actor is tied to its writing device's person (§2.4): Blake's
// device can't write as Stephen, mint a card claiming to be Stephen's
// agent, or act as Stephen's agent, and the store counts the refusals.
func TestBoardActorsAreTheWritersPerson(t *testing.T) {
	ctx := t.Context()
	f := newBFold(t)
	s := f.person("stephen", "pc")
	b := f.person("blake", "bpc")
	var pins []MemberPin
	for _, p := range []*bperson{s, b} {
		pin, _ := f.v.Pinned(p.pid)
		pins = append(pins, pin)
	}
	v := NewBoardVerifier(pins...)
	st, err := store.OpenWith(ctx, filepath.Join(t.TempDir(), "b.db"), "self", hlc.New(nil), store.OpenOptions{Verifier: v, Board: true})
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	ins := func(ev store.Event) error { return st.Insert(ctx, ev) }
	for _, ev := range []store.Event{f.cert(s, 1, "pc", "pc"), f.cert(b, 1, "bpc", "bpc")} {
		if err := ins(ev); err != nil {
			t.Fatal(err)
		}
	}
	card := func(id, person string) map[string]string {
		return map[string]string{"cardId": id, "personId": person, "name": "lead", "kind": "claude", "status": "active"}
	}
	if err := ins(f.row("pc", "agent:sc", store.KindAgentCard, card("sc", s.pid))); err != nil {
		t.Fatal(err)
	}
	if p := v.KeyPerson("agent:sc"); p != s.pid {
		t.Fatalf("agent:sc is %q's", p)
	}
	type attempt struct {
		name        string
		actor, kind string
		payload     any
	}
	comment := map[string]string{"taskId": "t1", "commentId": "c1", "text": "hi"}
	for _, a := range []attempt{
		{"blake writing as person:stephen", "person:" + s.pid, "task.comment", comment},
		{"blake minting a card with stephen's personId", "agent:fake", store.KindAgentCard, card("fake", s.pid)},
		{"blake acting as stephen's agent", "agent:sc", "task.comment", comment},
		{"blake writing stephen's member rows", "irori", store.KindMemberProfile, MemberProfile{PersonID: s.pid}},
		{"blake announcing pc as a server", "irori", store.KindBoardServer, map[string]any{"deviceId": "pc", "port": 1}},
		{"blake's card claiming for stephen's agent", "agent:bc", "task.claimed", map[string]string{"taskId": "t1", "by": "agent:sc"}},
		{"blake's irori row credited to stephen", "irori", "task.left", map[string]string{"taskId": "t1", "who": "agent:sc", "by": "person:" + s.pid}},
	} {
		if err := ins(f.row("bpc", a.actor, a.kind, a.payload)); !errors.Is(err, store.ErrBoardActor) {
			t.Errorf("%s: err = %v, want %v", a.name, err, store.ErrBoardActor)
		}
		f.seq["bpc"]-- // refused: bpc's next row reuses the seq
	}
	if got := st.BoardRefused().Actor; got != 7 {
		t.Errorf("refused actors counted %d, want 7", got)
	}
	// A card not held yet is a row that waits, not a forgery.
	if err := ins(f.row("bpc", "agent:later", "task.comment", comment)); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("an unknown card: err = %v", err)
	}
	f.seq["bpc"]--
	// Blake's own person, his own card, and his server are fine.
	for _, ev := range []store.Event{
		f.row("bpc", "agent:bc", store.KindAgentCard, card("bc", b.pid)),
		f.row("bpc", "irori", store.KindBoardServer, map[string]any{"deviceId": "bpc", "port": 1}),
	} {
		if err := ins(ev); err != nil {
			t.Fatalf("%s: %v", ev.Kind, err)
		}
	}
	if v.KeyPerson("agent:bc") != b.pid || v.KeyPerson("person:"+b.pid) != b.pid || v.KeyPerson("person:nobody") != "" {
		t.Fatal("KeyPerson")
	}
	// The cards fold again at open, from the log alone.
	st.ReloadVerifier(ctx)
	if v.KeyPerson("agent:sc") != s.pid {
		t.Fatal("the card's person is lost after a reload")
	}
}

// v0.5's fields on the private rows: a cert or revoke from before v0.5
// (no personEpoch) still verifies as it did, and Rotate's prevSig is the
// old key over the rotation, so it becomes a board rotation as is.
func TestPrivateRowsGainEpochAndPrevSig(t *testing.T) {
	old := Cert{PersonID: "p", DeviceID: "d", DevicePub: "k", Label: "pc", Role: RoleDesktop, At: time.Unix(1, 0).UTC()}
	raw, _ := Canonical(old)
	if got := string(raw); got != `{"at":"1970-01-01T00:00:01Z","deviceId":"d","devicePub":"k","label":"pc","personId":"p","role":"desktop"}` {
		t.Fatalf("an epoch-less cert's signed bytes changed: %s", got)
	}
	id := loadIdentity(t)
	prev := id.Current()
	add, err := id.CertFor("d2", pubHex(newKey()), "mac", RoleDesktop, time.Unix(5, 0))
	if err != nil || add.PersonEpoch != prev.Epoch {
		t.Fatalf("CertFor epoch %d, %v", add.PersonEpoch, err)
	}
	// A v0.4 device decodes the payload into its own structs, which have
	// no personEpoch anywhere, and re-marshals the cert to check it.
	type v04Cert struct {
		PersonID  string    `json:"personId"`
		DeviceID  string    `json:"deviceId"`
		DevicePub string    `json:"devicePub"`
		Label     string    `json:"label"`
		Role      string    `json:"role"`
		At        time.Time `json:"at"`
	}
	type v04Revoke struct {
		PersonID string    `json:"personId"`
		DeviceID string    `json:"deviceId"`
		Cut      int64     `json:"cut"`
		CutHash  string    `json:"cutHash"`
		At       time.Time `json:"at"`
		Reason   string    `json:"reason,omitempty"`
	}
	var oldAdd struct {
		Cert      v04Cert `json:"cert"`
		PersonPub string  `json:"personPub"`
		PersonSig string  `json:"personSig"`
	}
	raw, _ = json.Marshal(add)
	if err := json.Unmarshal(raw, &oldAdd); err != nil || verifyPerson(oldAdd.PersonPub, oldAdd.PersonSig, oldAdd.Cert) != nil {
		t.Fatalf("a v0.4 device can't verify a v0.5 cert: %v %s", err, raw)
	}
	rev, err := id.RevokeOf("d2", 3, "h", "lost", time.Unix(5, 0))
	if err != nil || rev.PersonEpoch != prev.Epoch {
		t.Fatalf("RevokeOf epoch %d, %v", rev.PersonEpoch, err)
	}
	var oldRev struct {
		Revoke    v04Revoke `json:"revoke"`
		PersonPub string    `json:"personPub"`
		PersonSig string    `json:"personSig"`
	}
	raw, _ = json.Marshal(rev)
	if err := json.Unmarshal(raw, &oldRev); err != nil || verifyPerson(oldRev.PersonPub, oldRev.PersonSig, oldRev.Revoke) != nil {
		t.Fatalf("a v0.4 device can't verify a v0.5 revoke: %v %s", err, raw)
	}
	rot, err := id.Rotate(time.Unix(6, 0))
	if err != nil {
		t.Fatal(err)
	}
	if rot.By != id.Device.ID || rot.Epoch != prev.Epoch+1 || rot.PersonPub != id.Current().Pub {
		t.Fatalf("rotation %+v", rot)
	}
	if err := verifyPerson(prev.Pub, rot.PrevSig, rot.Body()); err != nil {
		t.Fatalf("prevSig isn't the old key's: %v", err)
	}
	m, ok := MemberRotation(rot)
	if !ok || verifyPerson(prev.Pub, m.PrevSig, m.body()) != nil {
		t.Fatal("the board copy of the rotation doesn't verify")
	}
	if _, ok := MemberRotation(KeyRotated{PersonID: "p", Epoch: 2, PersonPub: rot.PersonPub}); ok {
		t.Fatal("a pre-v0.5 rotation (no prevSig) went on a board")
	}
	// A cert for an epoch the key has moved past isn't signed.
	if _, err := id.MemberCertFor(prev.Epoch, "d3", pubHex(newKey()), RoleDesktop, time.Now()); err == nil {
		t.Fatal("signed a board cert for an epoch the key has left")
	}
}

func loadIdentity(t *testing.T) *Identity {
	t.Helper()
	dir := t.TempDir()
	dev, err := device.LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	id, err := Load(dir, dev, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	return id
}
