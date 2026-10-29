package identity

import (
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"sync"

	"github.com/StephenSHorton/irori/internal/store"
)

// Chain is the cert fold (§2, §5.5): which devices belong to this device's
// person, their keys, and the revocation cuts. It is the store's Verifier,
// so it sees every person.* row held at Open and each one written after.
//
// The rules, all independent of the order rows arrive in:
//   - the root is pinned: a self-written cert is valid only for the
//     person's rootDeviceId;
//   - every other cert is valid only when its writer holds a valid cert (the
//     chain leads back to the root); a device can't certify itself;
//   - a revoke is honoured when its writer was ever certified (the chain
//     back to the root with cuts ignored), and a device's effective cut is
//     the lowest cut among honoured revokes, each raised to at least the
//     rows its writer's own cert chain is built from;
//   - a cert written above its writer's cut doesn't count;
//   - only rows whose personId is this person's count; others are stored
//     and inert (a device's first-run person before it joined);
//   - a person signature counts only by one of the person's keys: the first
//     key (the one personId is derived from) or a key a person.key.rotated
//     row announces, written by an ever-certified device. A row signed by
//     any other key is held and inert, and counts once a rotation row
//     names its key.
//
// Revoked devices stay certified for IsOwner: every row of theirs that is
// stored is at or below their cut.
type Chain struct {
	mu       sync.Mutex
	personID string
	rootID   string
	selfID   string

	adds      []certRow
	revokes   []revokeRow
	rotations []rotationRow
	profiles  []profileRow
	profile   bool
	bad       int             // person rows for this person that failed their checks
	seen      map[rowKey]bool // rows folded, so a row learned early and then stored folds once

	valid  map[string]Cert // device id -> its first valid cert
	cuts   map[string]int64
	claims []store.CutClaim // honoured revokes' cutHash, for the store's fork check
}

type rowPos struct {
	hlc    uint64
	writer string
	seq    int64
}

func (a rowPos) less(b rowPos) bool {
	if a.hlc != b.hlc {
		return a.hlc < b.hlc
	}
	if a.writer != b.writer {
		return a.writer < b.writer
	}
	return a.seq < b.seq
}

type rowKey struct {
	dev string
	seq int64
}

type certRow struct {
	pos  rowPos
	cert Cert
	pub  string // the person key that signed it
}

type revokeRow struct {
	pos    rowPos
	revoke Revoke
	pub    string
}

type rotationRow struct {
	pos rowPos
	pub string // the key it announces
}

type profileRow struct {
	pos    rowPos
	pub    string
	rootID string
}

var _ store.Verifier = (*Chain)(nil)

// NewChain folds certs for personID, whose pinned root is rootDeviceID, on
// device selfID.
func NewChain(personID, rootDeviceID, selfID string) *Chain {
	return &Chain{personID: personID, rootID: rootDeviceID, selfID: selfID,
		valid: map[string]Cert{}, cuts: map[string]int64{}}
}

// Reset forgets every observed row (store.Verifier).
func (c *Chain) Reset() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.adds, c.revokes, c.rotations, c.profiles, c.profile, c.bad, c.seen = nil, nil, nil, nil, false, 0, nil
	c.valid, c.cuts, c.claims = map[string]Cert{}, map[string]int64{}, nil
}

// Repin points the fold at another person and its pinned root (B adopting
// A's person in pairing, §3 step 6) and forgets every observed row. The
// caller then has the store feed its rows again (Store.ReloadVerifier):
// rows of the old person stay stored and inert.
func (c *Chain) Repin(personID, rootDeviceID string) {
	c.mu.Lock()
	c.personID, c.rootID = personID, rootDeviceID
	c.mu.Unlock()
	c.Reset()
}

// PersonID is the person the fold is pinned to.
func (c *Chain) PersonID() string {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.personID
}

// Observe folds one person.* row (store.Verifier).
func (c *Chain) Observe(ev store.Event) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.seen == nil {
		c.seen = map[rowKey]bool{}
	}
	if k := (rowKey{ev.DeviceID, ev.Seq}); c.seen[k] {
		return
	} else {
		c.seen[k] = true
	}
	pos := rowPos{uint64(ev.HLC), ev.DeviceID, ev.Seq}
	switch ev.Kind {
	case store.KindPersonDeviceAdded:
		var p DeviceAdded
		if json.Unmarshal(ev.Payload, &p) != nil {
			c.bad++
			return
		}
		if p.Cert.PersonID != c.personID {
			return // another person's: stored, inert
		}
		if verifyPerson(p.PersonPub, p.PersonSig, p.Cert) != nil || !validPub(p.Cert.DevicePub) || p.Cert.DeviceID == "" {
			c.bad++
			return
		}
		c.adds = append(c.adds, certRow{pos: pos, cert: p.Cert, pub: p.PersonPub})
	case store.KindPersonDeviceRevoked:
		var p DeviceRevoked
		if json.Unmarshal(ev.Payload, &p) != nil {
			c.bad++
			return
		}
		if p.Revoke.PersonID != c.personID {
			return
		}
		if verifyPerson(p.PersonPub, p.PersonSig, p.Revoke) != nil || p.Revoke.DeviceID == "" || p.Revoke.Cut < 0 {
			c.bad++
			return
		}
		c.revokes = append(c.revokes, revokeRow{pos: pos, revoke: p.Revoke, pub: p.PersonPub})
	case store.KindPersonKeyRotated:
		var p KeyRotated
		if json.Unmarshal(ev.Payload, &p) != nil {
			c.bad++
			return
		}
		if p.PersonID != c.personID {
			return
		}
		if !validPub(p.PersonPub) {
			c.bad++
			return
		}
		c.rotations = append(c.rotations, rotationRow{pos: pos, pub: p.PersonPub})
	case store.KindPersonProfile:
		var p Profile
		if json.Unmarshal(ev.Payload, &p) != nil {
			c.bad++
			return
		}
		if p.PersonID != c.personID {
			return
		}
		body := profileBody{PersonID: p.PersonID, Name: p.Name, RootDeviceID: p.RootDeviceID, PersonPub: p.PersonPub}
		if verifyPerson(p.PersonPub, p.PersonSig, body) != nil {
			c.bad++
			return
		}
		c.profiles = append(c.profiles, profileRow{pos: pos, pub: p.PersonPub, rootID: p.RootDeviceID})
	default:
		return
	}
	c.refold()
}

func validPub(h string) bool {
	b, err := hex.DecodeString(h)
	return err == nil && len(b) == ed25519.PublicKeySize
}

// firstKey reports whether pub is the person's first key: the one personID
// is derived from (PersonIDOf). c.mu is held.
func (c *Chain) firstKey(pub string) bool { return firstKeyOf(c.personID, pub) }

func firstKeyOf(personID, pub string) bool {
	b, err := hex.DecodeString(pub)
	return err == nil && len(b) == ed25519.PublicKeySize && PersonIDOf(b) == personID
}

// refold recomputes the valid certs and cuts from every observed row. It
// runs in three steps with no loop between them, so it always settles:
//
//  1. ever: the certs that chain back to the pinned root with cuts ignored,
//     counting only certs signed by one of the person's keys. The keys are
//     the first key plus those announced by rotation rows of ever-certified
//     devices; more keys can only certify more devices, whose rotation
//     rows can only add keys, so the loop grows to a fixed point. Whether
//     a revoke counts depends only on this (§5.5), never on cuts, so cuts
//     can't switch off the revokes that make them.
//  2. cuts: the lowest cut among revokes written by an ever-certified
//     device. A revoke can't cut below the rows its own writer's cert
//     chain is made of (the cert that certified it, and up to the root):
//     such a cut is raised to the highest of those rows, or the revoke
//     would remove its own writer and leave no device certified.
//  3. valid: the chain again, now dropping certs written above their
//     writer's cut (a removed device can't add devices).
func (c *Chain) refold() {
	adds := append([]certRow(nil), c.adds...)
	sort.Slice(adds, func(i, j int) bool { return adds[i].pos.less(adds[j].pos) })
	revs := append([]revokeRow(nil), c.revokes...)
	sort.Slice(revs, func(i, j int) bool { return revs[i].pos.less(revs[j].pos) })

	keys := map[string]bool{}
	trusted := func(pub string) bool { return keys[pub] || c.firstKey(pub) }
	var signed []certRow
	var ever map[string]certRow
	for {
		signed = signed[:0]
		for _, a := range adds {
			if trusted(a.pub) {
				signed = append(signed, a)
			}
		}
		ever = c.certify(signed, nil)
		grew := false
		for _, r := range c.rotations {
			if _, ok := ever[r.pos.writer]; ok && !keys[r.pub] {
				keys[r.pub], grew = true, true
			}
		}
		if !grew {
			break
		}
	}
	cuts := map[string]int64{}
	var claims []store.CutClaim
	for _, r := range revs {
		if _, ok := ever[r.pos.writer]; !ok || !trusted(r.pub) {
			continue
		}
		dev := r.revoke.DeviceID
		cut := r.revoke.Cut
		if f, ok := c.floor(ever, r.pos.writer, dev); ok && f > cut {
			cut = f
		}
		if cur, ok := cuts[dev]; !ok || cut < cur {
			cuts[dev] = cut
		}
		if r.revoke.CutHash != "" && r.revoke.Cut > 0 {
			claims = append(claims, store.CutClaim{DeviceID: dev, Seq: r.revoke.Cut, Hash: r.revoke.CutHash})
		}
	}
	valid := map[string]Cert{}
	for dev, a := range c.certify(signed, cuts) {
		valid[dev] = a.cert
	}
	// The profile counts when one of the person's keys signed it, it names
	// the pinned root, and an ever-certified device wrote it.
	profile := false
	for _, p := range c.profiles {
		if _, ok := ever[p.pos.writer]; ok && trusted(p.pub) && p.rootID == c.rootID {
			profile = true
			break
		}
	}
	c.valid, c.cuts, c.claims, c.profile = valid, cuts, claims, profile
}

// certify folds adds (sorted) into the certs that chain back to the pinned
// root. With cuts, a cert row above its writer's cut doesn't count.
func (c *Chain) certify(adds []certRow, cuts map[string]int64) map[string]certRow {
	within := func(p rowPos) bool {
		cut, ok := cuts[p.writer]
		return !ok || p.seq <= cut
	}
	valid := map[string]certRow{}
	for _, a := range adds {
		if a.cert.DeviceID == c.rootID && a.pos.writer == c.rootID && within(a.pos) {
			valid[c.rootID] = a
			break
		}
	}
	if len(valid) == 0 {
		return valid
	}
	for changed := true; changed; {
		changed = false
		for _, a := range adds {
			dev, w := a.cert.DeviceID, a.pos.writer
			if _, have := valid[dev]; have || dev == w {
				continue
			}
			if _, ok := valid[w]; !ok || !within(a.pos) {
				continue
			}
			valid[dev] = a
			changed = true
		}
	}
	return valid
}

// floor is the highest seq of target's rows on writer's cert chain in ever
// (the cert rows that lead from writer back to the root). ok is false when
// target wrote none of them.
func (c *Chain) floor(ever map[string]certRow, writer, target string) (int64, bool) {
	var f int64
	found := false
	cur := writer
	for range len(ever) {
		row, ok := ever[cur]
		if !ok {
			break
		}
		if row.pos.writer == target && (!found || row.pos.seq > f) {
			f, found = row.pos.seq, true
		}
		if cur == c.rootID {
			break
		}
		cur = row.pos.writer
	}
	return f, found
}

// VerifyRow checks ev's signature against its device's cert (store.Verifier).
// A root self-cert row carries the key that verifies it, so the first row
// of a chain can be checked before any cert is held. That row must also be
// signed by the person's first key (the one personID names), so a device
// that doesn't hold the root's cert yet takes no self-cert for the root
// from anyone without the person secret.
func (c *Chain) VerifyRow(ev store.Event) error {
	c.mu.Lock()
	cert, ok := c.valid[ev.DeviceID]
	personID, rootID := c.personID, c.rootID
	c.mu.Unlock()
	pubHex := cert.DevicePub
	if !ok {
		if ev.Kind != store.KindPersonDeviceAdded || ev.DeviceID != rootID {
			return fmt.Errorf("%w: %s", store.ErrNoCert, ev.DeviceID)
		}
		var p DeviceAdded
		if json.Unmarshal(ev.Payload, &p) != nil || p.Cert.DeviceID != ev.DeviceID || p.Cert.PersonID != personID ||
			!firstKeyOf(personID, p.PersonPub) || verifyPerson(p.PersonPub, p.PersonSig, p.Cert) != nil {
			return fmt.Errorf("%w: %s", store.ErrNoCert, ev.DeviceID)
		}
		pubHex = p.Cert.DevicePub
	}
	pub, err := hex.DecodeString(pubHex)
	if err != nil || len(pub) != ed25519.PublicKeySize {
		return fmt.Errorf("%w: %s has a bad key", store.ErrBadSig, ev.DeviceID)
	}
	if !ed25519.Verify(pub, store.SigningBytes(ev), ev.Sig) {
		return fmt.Errorf("%w: %s/%d", store.ErrBadSig, ev.DeviceID, ev.Seq)
	}
	return nil
}

// Learn folds a person.* row that is not stored yet: the pairing exchange
// and a sync hello hand these over first (§3 step 6, §5.2). A device's rows
// are gapless, so its cert can't be inserted ahead of its earlier rows (the
// root's self-cert comes after its whole v0.3 history); learning the cert
// first lets those rows verify. The row must verify like any other; when it
// is later inserted it is not folded twice.
func (c *Chain) Learn(ev store.Event) error {
	if !store.IsPersonKind(ev.Kind) {
		return fmt.Errorf("identity: Learn takes person rows, not %s", ev.Kind)
	}
	if err := c.VerifyRow(ev); err != nil {
		return err
	}
	c.Observe(ev)
	return nil
}

// Cut is deviceID's effective revocation cut (store.Verifier).
func (c *Chain) Cut(deviceID string) (int64, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	cut, ok := c.cuts[deviceID]
	return cut, ok
}

// CutClaims lists the row each honoured revoke names by cutHash
// (store.Verifier): a held row there with other bytes is a fork (§5.5).
func (c *Chain) CutClaims() []store.CutClaim {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]store.CutClaim(nil), c.claims...)
}

// Cert returns deviceID's valid cert.
func (c *Chain) Cert(deviceID string) (Cert, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	cert, ok := c.valid[deviceID]
	return cert, ok
}

// HasProfile reports whether this person's person.profile row is held.
func (c *Chain) HasProfile() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.profile
}

// Bad counts person rows for this person that failed their checks.
func (c *Chain) Bad() int {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.bad
}

// DeviceState is one certified device.
type DeviceState struct {
	Cert    Cert  `json:"cert"`
	Revoked bool  `json:"revoked"`
	Cut     int64 `json:"cut,omitempty"`
}

// Devices lists every device ever certified to the person, by device id.
func (c *Chain) Devices() []DeviceState {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := make([]DeviceState, 0, len(c.valid))
	for id, cert := range c.valid {
		cut, rev := c.cuts[id]
		out = append(out, DeviceState{Cert: cert, Revoked: rev, Cut: cut})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Cert.DeviceID < out[j].Cert.DeviceID })
	return out
}

// UserKeyPrefix starts the owner's sender key, "user:<deviceId>".
const UserKeyPrefix = "user:"

// UserKey is the owner's sender key on deviceID.
func UserKey(deviceID string) string { return UserKeyPrefix + deviceID }

// IsOwner reports whether key is the owner's: exactly "user:<deviceId>" for
// this device or a device ever certified to this person, revoked ones
// included (§2). A made-up "user:anything" is not.
func (c *Chain) IsOwner(key string) bool {
	dev, ok := strings.CutPrefix(key, UserKeyPrefix)
	if !ok || dev == "" {
		return false
	}
	if dev == c.selfID {
		return true
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	_, ok = c.valid[dev]
	return ok
}

// OwnerOnly returns the IsOwner rule for a device with no chain: only
// "user:<deviceID>" (tests and services built without identity).
func OwnerOnly(deviceID string) func(string) bool {
	want := UserKey(deviceID)
	return func(key string) bool { return key == want }
}
