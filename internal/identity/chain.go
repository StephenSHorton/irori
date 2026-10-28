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
//   - a revoke is honoured when its writer was ever validly certified, and
//     a device's effective cut is the lowest cut among honoured revokes;
//   - only rows whose personId is this person's count; others are stored
//     and inert (a device's first-run person before it joined).
//
// Revoked devices stay certified for IsOwner: every row of theirs that is
// stored is at or below their cut.
type Chain struct {
	mu       sync.Mutex
	personID string
	rootID   string
	selfID   string

	adds    []certRow
	revokes []revokeRow
	profile bool
	bad     int             // person rows for this person that failed their checks
	seen    map[rowKey]bool // rows folded, so a row learned early and then stored folds once

	valid map[string]Cert // device id -> its first valid cert
	cuts  map[string]int64
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
}

type revokeRow struct {
	pos    rowPos
	revoke Revoke
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
	c.adds, c.revokes, c.profile, c.bad, c.seen = nil, nil, false, 0, nil
	c.valid, c.cuts = map[string]Cert{}, map[string]int64{}
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
		c.adds = append(c.adds, certRow{pos: pos, cert: p.Cert})
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
		c.revokes = append(c.revokes, revokeRow{pos: pos, revoke: p.Revoke})
	case store.KindPersonProfile:
		var p Profile
		if json.Unmarshal(ev.Payload, &p) == nil && p.PersonID == c.personID {
			c.profile = true
		}
		return
	default:
		return
	}
	c.refold()
}

func validPub(h string) bool {
	b, err := hex.DecodeString(h)
	return err == nil && len(b) == ed25519.PublicKeySize
}

// refold recomputes the valid certs and cuts from every observed row. Sets
// are small (a handful of devices), so this runs to a fixed point.
func (c *Chain) refold() {
	adds := append([]certRow(nil), c.adds...)
	sort.Slice(adds, func(i, j int) bool { return adds[i].pos.less(adds[j].pos) })
	revs := append([]revokeRow(nil), c.revokes...)
	sort.Slice(revs, func(i, j int) bool { return revs[i].pos.less(revs[j].pos) })

	cuts := map[string]int64{}
	var valid map[string]Cert
	for range 8 { // cuts and validity feed each other; this settles in two or three passes
		valid = map[string]Cert{}
		for _, a := range adds {
			if a.cert.DeviceID == c.rootID && a.pos.writer == c.rootID {
				if cut, ok := cuts[a.pos.writer]; !ok || a.pos.seq <= cut {
					valid[c.rootID] = a.cert
					break
				}
			}
		}
		for changed := true; changed; {
			changed = false
			for _, a := range adds {
				dev, w := a.cert.DeviceID, a.pos.writer
				if _, have := valid[dev]; have || dev == w {
					continue
				}
				if _, ok := valid[w]; !ok {
					continue
				}
				if cut, ok := cuts[w]; ok && a.pos.seq > cut {
					continue // written after the writer was removed
				}
				valid[dev] = a.cert
				changed = true
			}
		}
		next := map[string]int64{}
		for _, r := range revs {
			if _, ok := valid[r.pos.writer]; !ok {
				continue
			}
			if cur, ok := next[r.revoke.DeviceID]; !ok || r.revoke.Cut < cur {
				next[r.revoke.DeviceID] = r.revoke.Cut
			}
		}
		if mapsEqual(next, cuts) {
			break
		}
		cuts = next
	}
	c.valid, c.cuts = valid, cuts
}

func mapsEqual(a, b map[string]int64) bool {
	if len(a) != len(b) {
		return false
	}
	for k, v := range a {
		if w, ok := b[k]; !ok || w != v {
			return false
		}
	}
	return true
}

// VerifyRow checks ev's signature against its device's cert (store.Verifier).
// A root self-cert row carries the key that verifies it, so the first row
// of a chain can be checked before any cert is held.
func (c *Chain) VerifyRow(ev store.Event) error {
	c.mu.Lock()
	cert, ok := c.valid[ev.DeviceID]
	c.mu.Unlock()
	pubHex := cert.DevicePub
	if !ok {
		if ev.Kind != store.KindPersonDeviceAdded || ev.DeviceID != c.rootID {
			return fmt.Errorf("%w: %s", store.ErrNoCert, ev.DeviceID)
		}
		var p DeviceAdded
		if json.Unmarshal(ev.Payload, &p) != nil || p.Cert.DeviceID != ev.DeviceID || p.Cert.PersonID != c.personID {
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
