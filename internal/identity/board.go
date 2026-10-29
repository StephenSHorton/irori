package identity

import (
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// Identity across people (design/v0.5-shared-boards.md §3). Inside one
// person's mesh the person signature proves nothing the device signature
// doesn't; across people it is the whole trust. So each board log carries
// every member's own board-scoped identity objects, signed by that
// member's person key, and BoardVerifier folds them into one chain per
// member, seeded from what the board pins for that member.

// MemberPin is what a board pins for one member (§3): the person, its key
// and epoch as proven, and its board root, the device that proved itself
// (the creating device for the creator, the device that ran the invite
// SAS for an invitee). It is not the person's v0.4 root.
type MemberPin struct {
	PersonID     string `json:"personId"`
	PersonPub    string `json:"personPub"`
	RootDeviceID string `json:"rootDeviceId"`
	PersonEpoch  int    `json:"personEpoch"`
}

// MemberCert is a board's device cert: the private Cert without its label
// (members can count your devices, not name them).
type MemberCert struct {
	PersonID    string    `json:"personId"`
	DeviceID    string    `json:"deviceId"`
	DevicePub   string    `json:"devicePub"`
	Role        string    `json:"role"`
	PersonEpoch int       `json:"personEpoch"`
	At          time.Time `json:"at"`
}

// MemberDeviceAdded is the member.device.added payload: personSig is the
// member's key of cert.personEpoch over canonical(cert).
type MemberDeviceAdded struct {
	Cert      MemberCert `json:"cert"`
	PersonSig string     `json:"personSig"`
}

// MemberRevoke removes a member's device from one board. BoardCut is the
// highest seq of that device's stream in this board log that the revoking
// device holds, never a private-log seq.
type MemberRevoke struct {
	PersonID     string    `json:"personId"`
	DeviceID     string    `json:"deviceId"`
	BoardCut     int64     `json:"boardCut"`
	BoardCutHash string    `json:"boardCutHash"`
	PersonEpoch  int       `json:"personEpoch"`
	At           time.Time `json:"at"`
}

// MemberDeviceRevoked is the member.device.revoked payload.
type MemberDeviceRevoked struct {
	Revoke    MemberRevoke `json:"revoke"`
	PersonSig string       `json:"personSig"`
}

// MemberKeyRotated is the member.key.rotated payload: the private rotation
// as RotationBody, with prevSig, the previous epoch's key over it. It is
// written only by the device named in By.
type MemberKeyRotated struct {
	PersonID    string    `json:"personId"`
	PersonEpoch int       `json:"personEpoch"`
	PersonPub   string    `json:"personPub"`
	By          string    `json:"by"`
	At          time.Time `json:"at"`
	PrevSig     string    `json:"prevSig"`
}

func (r MemberKeyRotated) body() RotationBody {
	return RotationBody{PersonID: r.PersonID, PersonEpoch: r.PersonEpoch, PersonPub: r.PersonPub, By: r.By, At: r.At}
}

// MemberRotation is the board copy of a private rotation (§3, "Keeping
// boards current"). ok is false for a rotation written before v0.5, which
// carries no prevSig and can't go on a board.
func MemberRotation(r KeyRotated) (MemberKeyRotated, bool) {
	m := MemberKeyRotated{PersonID: r.PersonID, PersonEpoch: r.Epoch, PersonPub: r.PersonPub, By: r.By, At: r.At, PrevSig: r.PrevSig}
	return m, r.PrevSig != "" && r.By != ""
}

// MemberProfile is the member.profile payload.
type MemberProfile struct {
	PersonID    string `json:"personId"`
	Name        string `json:"name"`
	PersonEpoch int    `json:"personEpoch"`
	PersonSig   string `json:"personSig"`
}

type memberProfileBody struct {
	PersonID    string `json:"personId"`
	Name        string `json:"name"`
	PersonEpoch int    `json:"personEpoch"`
}

// BoardPin is this person pinned on a board this device creates, with
// this device as its board root.
func (id *Identity) BoardPin() MemberPin {
	cur := id.Current()
	return MemberPin{PersonID: cur.ID, PersonPub: cur.Pub, RootDeviceID: id.Device.ID, PersonEpoch: cur.Epoch}
}

// MemberCertFor makes a member.device.added payload, signed with the
// current key, which must still be epoch's: the board's newest for this
// person (a cert signed by a key the board doesn't know counts nowhere).
func (id *Identity) MemberCertFor(epoch int, deviceID, devicePub, role string, at time.Time) (MemberDeviceAdded, error) {
	c := MemberCert{PersonID: id.Current().ID, DeviceID: deviceID, DevicePub: devicePub, Role: role, PersonEpoch: epoch, At: at.UTC()}
	sig, err := id.signEpoch(epoch, c)
	return MemberDeviceAdded{Cert: c, PersonSig: sig}, err
}

// MemberRevokeOf makes a member.device.revoked payload, as MemberCertFor.
func (id *Identity) MemberRevokeOf(epoch int, deviceID string, boardCut int64, cutHash string, at time.Time) (MemberDeviceRevoked, error) {
	r := MemberRevoke{PersonID: id.Current().ID, DeviceID: deviceID, BoardCut: boardCut, BoardCutHash: cutHash, PersonEpoch: epoch, At: at.UTC()}
	sig, err := id.signEpoch(epoch, r)
	return MemberDeviceRevoked{Revoke: r, PersonSig: sig}, err
}

// MemberProfileRow makes a member.profile payload, as MemberCertFor.
func (id *Identity) MemberProfileRow(epoch int) (MemberProfile, error) {
	cur := id.Current()
	b := memberProfileBody{PersonID: cur.ID, Name: cur.Name, PersonEpoch: epoch}
	sig, err := id.signEpoch(epoch, b)
	return MemberProfile{PersonID: b.PersonID, Name: b.Name, PersonEpoch: b.PersonEpoch, PersonSig: sig}, err
}

// BoardVerifier is a board log's store.Verifier (§3): one chain per pinned
// member, run by v0.4's rules inside the board log, and the binding of
// every row's actor to its writing device's person (§2.4).
//
// Per member, with rules that don't depend on arrival order:
//   - chain to the pinned root: a member.device.added counts when its
//     writer holds a counted cert of the same person, or it is the board
//     root's self-cert; a device can't certify itself;
//   - epochs: a person-signed object counts only when its personSig
//     verifies by the member's key of the epoch it names, and that epoch is
//     the newest accepted one or the row's HLC is below the accepted
//     rotation that ended it;
//   - rotation: member.key.rotated counts with a prevSig by the previous
//     epoch's key, written by its By device, which holds a counted cert;
//     two to one epoch take the lower By. A rotation whose By device is
//     revoked in the board is discarded wherever it sits, and an epoch
//     whose every rotation was discarded takes no new objects after the
//     first of them (a thief and your device that revoked each other leave
//     the person unable to write anything new until re-invited);
//   - revokes: honoured when the writer was ever certified and the
//     signature counts, judged on the chain without the revoked device's
//     own rotations, so a device can't shield itself by rotating first;
//     the lowest boardCut wins, raised to the rows its writer's own chain
//     is made of, as v0.4 §5.5.
//
// A device certified by two members counts for neither (a member can't
// take over another's device by certifying its key), except a member's
// own pinned root.
//
// The pins come from outside (Pin): the Core pins this person on a board it
// creates. Pins from board.created and board.member.added, with owners
// and removal, are the membership fold's (§12 step 3).
type BoardVerifier struct {
	mu   sync.Mutex
	pins map[string]MemberPin // personId -> pin; kept across Reset

	certs     []mCertRow
	revokes   []mRevokeRow
	rotations []mRotRow
	profiles  []mProfileRow
	cards     []mCardRow
	seen      map[rowKey]bool
	sigs      sigMemo
	bad       int // identity rows that failed their checks

	members map[string]*memberFold // personId -> its fold
	devs    map[string]string      // deviceId -> personId, counted certs (revoked ones too)
	valid   map[string]MemberCert
	cuts    map[string]int64
	claims  []store.CutClaim
	cardOf  map[string]string // cardId -> personId, counted agent.card rows
	ambig   map[string]bool   // device ids certified by two members
}

type mCertRow struct {
	pos  rowPos
	cert MemberCert
	sig  string
}

type mRevokeRow struct {
	pos rowPos
	rev MemberRevoke
	sig string
}

type mRotRow struct {
	pos rowPos
	rot MemberKeyRotated
}

type mProfileRow struct {
	pos  rowPos
	prof MemberProfile
}

type mCardRow struct {
	pos      rowPos
	cardID   string
	personID string
}

// memberFold is one member's folded chain.
type memberFold struct {
	pin     MemberPin
	keys    []boardEpoch
	ever    map[string]mCertRow
	valid   map[string]mCertRow
	cuts    map[string]int64
	claims  []store.CutClaim
	name    string
	profile bool
}

// boardEpoch is one accepted epoch of a member's key. end is the HLC of
// what ended it (0 while it is the newest).
type boardEpoch struct {
	epoch int
	pub   string
	end   uint64
}

var _ store.Verifier = (*BoardVerifier)(nil)

// NewBoardVerifier makes a board Verifier with these pins.
func NewBoardVerifier(pins ...MemberPin) *BoardVerifier {
	v := &BoardVerifier{pins: map[string]MemberPin{}}
	for _, p := range pins {
		v.pins[p.PersonID] = p
	}
	v.resetLocked()
	return v
}

// Pin pins a member (§3) and refolds the rows already observed. A person
// is pinned once; a second, different pin for them is refused.
func (v *BoardVerifier) Pin(p MemberPin) error {
	if p.PersonID == "" || p.RootDeviceID == "" || p.PersonEpoch < 1 || !validPub(p.PersonPub) {
		return fmt.Errorf("identity: incomplete pin for %q", p.PersonID)
	}
	v.mu.Lock()
	defer v.mu.Unlock()
	if old, ok := v.pins[p.PersonID]; ok {
		if old != p {
			return fmt.Errorf("identity: %s is already pinned on this board with another key or root", p.PersonID)
		}
		return nil
	}
	v.pins[p.PersonID] = p
	v.refold()
	return nil
}

// Pinned reports personID's pin.
func (v *BoardVerifier) Pinned(personID string) (MemberPin, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	p, ok := v.pins[personID]
	return p, ok
}

// Reset forgets every observed row; the pins stay (store.Verifier).
func (v *BoardVerifier) Reset() {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.resetLocked()
}

func (v *BoardVerifier) resetLocked() {
	v.certs, v.revokes, v.rotations, v.profiles, v.cards, v.seen, v.bad = nil, nil, nil, nil, nil, nil, 0
	v.members, v.devs, v.valid, v.cuts, v.claims = map[string]*memberFold{}, map[string]string{}, map[string]MemberCert{}, map[string]int64{}, nil
	v.cardOf, v.ambig, v.sigs = map[string]string{}, map[string]bool{}, sigMemo{}
}

// Observe folds one board identity row (store.Verifier): member.*, and
// agent.card for the card's person. Other kinds pass by.
func (v *BoardVerifier) Observe(ev store.Event) {
	v.mu.Lock()
	defer v.mu.Unlock()
	if v.seen == nil {
		v.seen = map[rowKey]bool{}
	}
	if k := (rowKey{ev.DeviceID, ev.Seq}); v.seen[k] {
		return
	} else {
		v.seen[k] = true
	}
	pos := rowPos{uint64(ev.HLC), ev.DeviceID, ev.Seq}
	switch ev.Kind {
	case store.KindMemberDeviceAdded:
		var p MemberDeviceAdded
		if json.Unmarshal(ev.Payload, &p) != nil || p.Cert.DeviceID == "" || !validPub(p.Cert.DevicePub) {
			v.bad++
			return
		}
		v.certs = append(v.certs, mCertRow{pos: pos, cert: p.Cert, sig: p.PersonSig})
	case store.KindMemberDeviceRevoked:
		var p MemberDeviceRevoked
		if json.Unmarshal(ev.Payload, &p) != nil || p.Revoke.DeviceID == "" || p.Revoke.BoardCut < 0 {
			v.bad++
			return
		}
		v.revokes = append(v.revokes, mRevokeRow{pos: pos, rev: p.Revoke, sig: p.PersonSig})
	case store.KindMemberKeyRotated:
		var p MemberKeyRotated
		if json.Unmarshal(ev.Payload, &p) != nil || !validPub(p.PersonPub) || p.By != ev.DeviceID {
			v.bad++
			return
		}
		v.rotations = append(v.rotations, mRotRow{pos: pos, rot: p})
	case store.KindMemberProfile:
		var p MemberProfile
		if json.Unmarshal(ev.Payload, &p) != nil {
			v.bad++
			return
		}
		v.profiles = append(v.profiles, mProfileRow{pos: pos, prof: p})
	case store.KindAgentCard:
		var p struct {
			CardID   string `json:"cardId"`
			PersonID string `json:"personId"`
		}
		if json.Unmarshal(ev.Payload, &p) != nil || p.CardID == "" {
			v.bad++
			return
		}
		v.cards = append(v.cards, mCardRow{pos: pos, cardID: p.CardID, personID: p.PersonID})
		v.foldCards() // a card changes no chain
		return
	default:
		return
	}
	v.refold()
}

// refold recomputes every member's chain, then which person each device
// and card belongs to. v.mu is held.
func (v *BoardVerifier) refold() {
	sort.Slice(v.certs, func(i, j int) bool { return v.certs[i].pos.less(v.certs[j].pos) })
	sort.Slice(v.revokes, func(i, j int) bool { return v.revokes[i].pos.less(v.revokes[j].pos) })
	sort.Slice(v.rotations, func(i, j int) bool { return v.rotations[i].pos.less(v.rotations[j].pos) })
	sort.Slice(v.profiles, func(i, j int) bool { return v.profiles[i].pos.less(v.profiles[j].pos) })

	members := map[string]*memberFold{}
	ids := make([]string, 0, len(v.pins))
	for id := range v.pins {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, pid := range ids {
		members[pid] = v.foldMember(v.pins[pid])
	}
	// Which person each device is: a device counted by two members counts
	// for neither, unless it is one's pinned root.
	owners := map[string][]string{}
	for _, pid := range ids {
		for dev := range members[pid].valid {
			owners[dev] = append(owners[dev], pid)
		}
	}
	devs, valid, ambig := map[string]string{}, map[string]MemberCert{}, map[string]bool{}
	for dev, pids := range owners {
		pid := pids[0]
		if len(pids) > 1 {
			pid = ""
			for _, p := range pids {
				if v.pins[p].RootDeviceID == dev {
					pid = p
				}
			}
			ambig[dev] = true
		}
		if pid == "" {
			continue
		}
		devs[dev], valid[dev] = pid, members[pid].valid[dev].cert
	}
	// A member's revokes cut only devices that are that member's.
	cuts := map[string]int64{}
	var claims []store.CutClaim
	for _, pid := range ids {
		m := members[pid]
		for dev, cut := range m.cuts {
			if devs[dev] == pid {
				cuts[dev] = cut
			}
		}
		for _, cl := range m.claims {
			if devs[cl.DeviceID] == pid {
				claims = append(claims, cl)
			}
		}
	}
	v.members, v.devs, v.valid, v.cuts, v.claims, v.ambig = members, devs, valid, cuts, claims, ambig
	v.foldCards()
}

// foldCards recomputes which person each card is: a card counts for its
// person when a device of that person wrote it, and the first such row (in
// fold order) names it. v.mu is held.
func (v *BoardVerifier) foldCards() {
	sort.Slice(v.cards, func(i, j int) bool { return v.cards[i].pos.less(v.cards[j].pos) })
	cardOf := map[string]string{}
	for _, r := range v.cards {
		if _, have := cardOf[r.cardID]; have {
			continue
		}
		if p := v.devs[r.pos.writer]; p != "" && p == r.personID {
			cardOf[r.cardID] = p
		}
	}
	v.cardOf = cardOf
}

// foldMember runs one member's chain (the rules on BoardVerifier), with
// the cuts and cut claims of its honoured revokes. v.mu is held.
func (v *BoardVerifier) foldMember(pin MemberPin) *memberFold {
	var certs []mCertRow
	for _, r := range v.certs {
		if r.cert.PersonID == pin.PersonID {
			certs = append(certs, r)
		}
	}
	var revs []mRevokeRow
	for _, r := range v.revokes {
		if r.rev.PersonID == pin.PersonID {
			revs = append(revs, r)
		}
	}
	var rots []mRotRow
	for _, r := range v.rotations {
		if r.rot.PersonID == pin.PersonID {
			rots = append(rots, r)
		}
	}

	// Revokes first, each judged on the chain without the revoked device's
	// own rotations.
	revoked := map[string]bool{}
	var honoured []mRevokeRow
	for _, r := range revs {
		target := r.rev.DeviceID
		keys, ever := memberChain(v.sigs, pin, certs, rots, func(dev string) bool { return dev == target }, false)
		if _, ok := ever[r.pos.writer]; !ok || !signedIn(v.sigs, keys, r.rev.PersonEpoch, r.pos, r.sig, r.rev) {
			continue
		}
		if _, mine := ever[target]; !mine {
			continue // a member revokes only its own devices
		}
		revoked[target] = true
		honoured = append(honoured, r)
	}
	keys, ever := memberChain(v.sigs, pin, certs, rots, func(dev string) bool { return revoked[dev] }, true)
	var signed []mCertRow
	for _, r := range certs {
		if signedIn(v.sigs, keys, r.cert.PersonEpoch, r.pos, r.sig, r.cert) {
			signed = append(signed, r)
		}
	}
	mine := map[string]int64{}
	var claims []store.CutClaim
	for _, r := range honoured {
		dev, cut := r.rev.DeviceID, r.rev.BoardCut
		if f, ok := floorOf(pin.RootDeviceID, ever, func(a mCertRow) rowPos { return a.pos }, r.pos.writer, dev); ok && f > cut {
			cut = f
		}
		if cur, ok := mine[dev]; !ok || cut < cur {
			mine[dev] = cut
		}
		if r.rev.BoardCutHash != "" && r.rev.BoardCut > 0 {
			claims = append(claims, store.CutClaim{DeviceID: dev, Seq: r.rev.BoardCut, Hash: r.rev.BoardCutHash})
		}
	}
	m := &memberFold{pin: pin, keys: keys, ever: ever, valid: certifyMember(pin.RootDeviceID, signed, mine), cuts: mine, claims: claims}
	for _, p := range v.profiles {
		if p.prof.PersonID != pin.PersonID {
			continue
		}
		if _, ok := ever[p.pos.writer]; !ok {
			continue
		}
		b := memberProfileBody{PersonID: p.prof.PersonID, Name: p.prof.Name, PersonEpoch: p.prof.PersonEpoch}
		if signedIn(v.sigs, keys, b.PersonEpoch, p.pos, p.prof.PersonSig, b) {
			m.name, m.profile = p.prof.Name, true // the latest counted one
		}
	}
	return m
}

// memberChain is a member's accepted epochs and the devices ever certified
// under them (cuts ignored), leaving out rotations by any device skip
// names. freeze ends an epoch at its first skipped rotation when no other
// was accepted for the next. It iterates to a fixed point, since more keys
// certify more devices and more devices may rotate: a deterministic loop
// over the sorted rows, so every arrival order agrees.
func memberChain(m sigMemo, pin MemberPin, certs []mCertRow, rots []mRotRow, skip func(string) bool, freeze bool) ([]boardEpoch, map[string]mCertRow) {
	keys := []boardEpoch{{epoch: pin.PersonEpoch, pub: pin.PersonPub}}
	var ever map[string]mCertRow
	for range len(rots) + 2 {
		var signed []mCertRow
		for _, r := range certs {
			if signedIn(m, keys, r.cert.PersonEpoch, r.pos, r.sig, r.cert) {
				signed = append(signed, r)
			}
		}
		ever = certifyMember(pin.RootDeviceID, signed, nil)
		next := epochsFrom(m, pin, rots, ever, skip, freeze)
		if sameEpochs(next, keys) {
			break
		}
		keys = next
	}
	return keys, ever
}

// epochsFrom builds the epoch chain from the pin: at each epoch the
// rotation to the next with a prevSig by this epoch's key, written by its
// By device, which is in ever and not skipped; the lowest By (then
// position) wins.
func epochsFrom(m sigMemo, pin MemberPin, rots []mRotRow, ever map[string]mCertRow, skip func(string) bool, freeze bool) []boardEpoch {
	keys := []boardEpoch{{epoch: pin.PersonEpoch, pub: pin.PersonPub}}
	for range len(rots) {
		cur := &keys[len(keys)-1]
		var best *mRotRow
		var frozen uint64
		for i := range rots {
			r := &rots[i]
			if r.rot.PersonEpoch != cur.epoch+1 || r.rot.By != r.pos.writer || !m.verify(r.pos, cur.pub, r.rot.PrevSig, r.rot.body()) {
				continue
			}
			if _, ok := ever[r.rot.By]; !ok {
				continue
			}
			if skip(r.rot.By) {
				if frozen == 0 || r.pos.hlc < frozen {
					frozen = r.pos.hlc
				}
				continue
			}
			if best == nil || r.rot.By < best.rot.By || (r.rot.By == best.rot.By && r.pos.less(best.pos)) {
				best = r
			}
		}
		if best == nil {
			if freeze && frozen > 0 {
				cur.end = frozen
			}
			break
		}
		cur.end = best.pos.hlc
		keys = append(keys, boardEpoch{epoch: best.rot.PersonEpoch, pub: best.rot.PersonPub})
	}
	return keys
}

func sameEpochs(a, b []boardEpoch) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// signedIn reports whether sig is epoch's key over canonical(v) and the row
// at pos may still be signed by it: the epoch is the newest, or the row's
// HLC is below what ended it.
func signedIn(m sigMemo, keys []boardEpoch, epoch int, pos rowPos, sig string, v any) bool {
	for _, k := range keys {
		if k.epoch != epoch {
			continue
		}
		if k.end != 0 && pos.hlc >= k.end {
			return false
		}
		return m.verify(pos, k.pub, sig, v)
	}
	return false
}

// sigMemo remembers person-signature checks by (row, key): a fold checks
// the same rows against the same keys many times, and a row's bytes never
// change once observed (Observe takes each (device, seq) once).
type sigMemo map[sigKey]bool

type sigKey struct {
	row rowKey
	pub string
}

func (m sigMemo) verify(pos rowPos, pub, sig string, v any) bool {
	k := sigKey{rowKey{pos.writer, pos.seq}, pub}
	ok, done := m[k]
	if !done {
		ok = verifyPerson(pub, sig, v) == nil
		m[k] = ok
	}
	return ok
}

func certifyMember(root string, rows []mCertRow, cuts map[string]int64) map[string]mCertRow {
	return certifyRows(root, rows, func(a mCertRow) (rowPos, string) { return a.pos, a.cert.DeviceID }, cuts)
}

// VerifyRow checks a board row (store.Verifier): its signature against
// its writing device's counted cert, and its actor against that device's
// person (§2.4). A pinned root's self-cert carries the key that verifies
// it, so it is checked from the pin alone. Errors wrap store.ErrNoCert
// (no cert, or a card not held yet: the row can wait), store.ErrBadSig, or
// store.ErrBoardActor (the row claims someone its device isn't).
func (v *BoardVerifier) VerifyRow(ev store.Event) error {
	v.mu.Lock()
	cert, ok := v.valid[ev.DeviceID]
	person := v.devs[ev.DeviceID]
	if !ok {
		cert, ok = v.rootSelfCert(ev)
		person = cert.PersonID
	}
	cards := v.cardOf // refold replaces the map, never changes it
	v.mu.Unlock()
	if !ok {
		return fmt.Errorf("%w: %s", store.ErrNoCert, ev.DeviceID)
	}
	pub, err := hex.DecodeString(cert.DevicePub)
	if err != nil || len(pub) != ed25519.PublicKeySize {
		return fmt.Errorf("%w: %s has a bad key", store.ErrBadSig, ev.DeviceID)
	}
	if !ed25519.Verify(pub, store.SigningBytes(ev), ev.Sig) {
		return fmt.Errorf("%w: %s/%d", store.ErrBadSig, ev.DeviceID, ev.Seq)
	}
	return bindActor(ev, person, func(card string) (string, bool) {
		p, ok := cards[card]
		return p, ok
	})
}

// rootSelfCert is ev's cert when ev is a pinned board root's own
// self-cert, signed by the pinned key for the pinned epoch. v.mu is held.
func (v *BoardVerifier) rootSelfCert(ev store.Event) (MemberCert, bool) {
	if ev.Kind != store.KindMemberDeviceAdded {
		return MemberCert{}, false
	}
	var p MemberDeviceAdded
	if json.Unmarshal(ev.Payload, &p) != nil || p.Cert.DeviceID != ev.DeviceID {
		return MemberCert{}, false
	}
	pin, ok := v.pins[p.Cert.PersonID]
	if !ok || pin.RootDeviceID != ev.DeviceID || p.Cert.PersonEpoch != pin.PersonEpoch ||
		verifyPerson(pin.PersonPub, p.PersonSig, p.Cert) != nil {
		return MemberCert{}, false
	}
	return p.Cert, true
}

// bindActor ties a row to its writing device's person (§2.4): a person:
// actor must be that person, an agent: actor must name a card a device of
// that person wrote, an agent.card or member.* row must be about that
// person, and board.server describes only its own writer.
func bindActor(ev store.Event, person string, cardPerson func(string) (string, bool)) error {
	var p struct {
		PersonID string `json:"personId"`
		DeviceID string `json:"deviceId"`
		Cert     *struct {
			PersonID string `json:"personId"`
		} `json:"cert"`
		Revoke *struct {
			PersonID string `json:"personId"`
		} `json:"revoke"`
	}
	_ = json.Unmarshal(ev.Payload, &p)
	refuse := func(format string, a ...any) error {
		return fmt.Errorf("%w: %s/%d %s", store.ErrBoardActor, ev.DeviceID, ev.Seq, fmt.Sprintf(format, a...))
	}
	switch {
	case ev.Kind == store.KindAgentCard:
		if p.PersonID != person {
			return refuse("writes an agent card for person %q, not its own", p.PersonID)
		}
	case strings.HasPrefix(ev.Kind, "member."):
		about := p.PersonID
		if p.Cert != nil {
			about = p.Cert.PersonID
		}
		if p.Revoke != nil {
			about = p.Revoke.PersonID
		}
		if about != person {
			return refuse("writes %s for person %q, not its own", ev.Kind, about)
		}
	case ev.Kind == store.KindBoardServer:
		if p.DeviceID != ev.DeviceID {
			return refuse("announces a server for device %q, not itself", p.DeviceID)
		}
	}
	switch {
	case strings.HasPrefix(ev.Actor, store.PersonActorPrefix):
		if ev.Actor[len(store.PersonActorPrefix):] != person {
			return refuse("writes as %s", ev.Actor)
		}
	case strings.HasPrefix(ev.Actor, store.AgentActorPrefix):
		if ev.Kind == store.KindAgentCard {
			return nil // the card itself: its person is checked above
		}
		card := ev.Actor[len(store.AgentActorPrefix):]
		owner, ok := cardPerson(card)
		if !ok {
			return fmt.Errorf("%w: %s/%d names card %s, not held yet", store.ErrNoCert, ev.DeviceID, ev.Seq, card)
		}
		if owner != person {
			return refuse("writes as another person's agent %s", ev.Actor)
		}
	}
	return nil
}

// Cut is deviceID's effective cut in this board (store.Verifier).
func (v *BoardVerifier) Cut(deviceID string) (int64, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	cut, ok := v.cuts[deviceID]
	return cut, ok
}

// CutClaims lists each honoured revoke's boardCutHash (store.Verifier).
func (v *BoardVerifier) CutClaims() []store.CutClaim {
	v.mu.Lock()
	defer v.mu.Unlock()
	return append([]store.CutClaim(nil), v.claims...)
}

// PersonOf is the member a device is certified to in this board.
func (v *BoardVerifier) PersonOf(deviceID string) (string, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	p, ok := v.devs[deviceID]
	return p, ok
}

// CardPerson is the member whose device wrote card cardID.
func (v *BoardVerifier) CardPerson(cardID string) (string, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	p, ok := v.cardOf[cardID]
	return p, ok
}

// KeyPerson is the member a board key is: person:<id> itself (when
// pinned), agent:<cardId> its card's person; "" for anything else.
func (v *BoardVerifier) KeyPerson(key string) string {
	if id, ok := strings.CutPrefix(key, store.PersonActorPrefix); ok {
		v.mu.Lock()
		defer v.mu.Unlock()
		if _, pinned := v.pins[id]; pinned {
			return id
		}
		return ""
	}
	if id, ok := strings.CutPrefix(key, store.AgentActorPrefix); ok {
		p, _ := v.CardPerson(id)
		return p
	}
	return ""
}

// Cert is deviceID's counted cert in this board.
func (v *BoardVerifier) Cert(deviceID string) (MemberCert, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	c, ok := v.valid[deviceID]
	return c, ok
}

// Epoch is personID's newest accepted epoch in this board (0 when not
// pinned).
func (v *BoardVerifier) Epoch(personID string) int {
	v.mu.Lock()
	defer v.mu.Unlock()
	m := v.members[personID]
	if m == nil {
		if p, ok := v.pins[personID]; ok {
			return p.PersonEpoch
		}
		return 0
	}
	return m.keys[len(m.keys)-1].epoch
}

// Writable reports whether personID may still sign new objects in this
// board: its newest epoch wasn't frozen by rotations that were all
// discarded (§3, a thief and your device revoking each other).
func (v *BoardVerifier) Writable(personID string) bool {
	v.mu.Lock()
	defer v.mu.Unlock()
	m := v.members[personID]
	return m != nil && m.keys[len(m.keys)-1].end == 0
}

// Profile is personID's counted member.profile name.
func (v *BoardVerifier) Profile(personID string) (string, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	m := v.members[personID]
	if m == nil {
		return "", false
	}
	return m.name, m.profile
}

// BoardDevice is one device a member certified in this board.
type BoardDevice struct {
	Cert    MemberCert `json:"cert"`
	Revoked bool       `json:"revoked"`
	Cut     int64      `json:"cut,omitempty"`
}

// Devices lists personID's certified devices in this board, by device id:
// what the other members see (§3, no labels).
func (v *BoardVerifier) Devices(personID string) []BoardDevice {
	v.mu.Lock()
	defer v.mu.Unlock()
	var out []BoardDevice
	for dev, p := range v.devs {
		if p != personID {
			continue
		}
		cut, rev := v.cuts[dev]
		out = append(out, BoardDevice{Cert: v.valid[dev], Revoked: rev, Cut: cut})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Cert.DeviceID < out[j].Cert.DeviceID })
	return out
}

// Ambiguous lists device ids two members both certified, which count for
// neither (§3).
func (v *BoardVerifier) Ambiguous() []string {
	v.mu.Lock()
	defer v.mu.Unlock()
	out := make([]string, 0, len(v.ambig))
	for d := range v.ambig {
		out = append(out, d)
	}
	sort.Strings(out)
	return out
}

// Bad counts identity rows that failed to parse.
func (v *BoardVerifier) Bad() int {
	v.mu.Lock()
	defer v.mu.Unlock()
	return v.bad
}
