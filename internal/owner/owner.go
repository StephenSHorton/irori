// Package owner is the v0.4 owner table (design/v0.4-devices.md §6.0): any
// device may ask, only the owner decides. Every entity that grants something
// has exactly one owning device. Other devices write request rows; only the
// owner writes the rows that take effect, and every fold ignores (and
// counts) an owner-only row from the wrong device.
//
// On one device every entity's owner is this device, so the checks change
// nothing there; they are in the folds now so the mesh (release 3) finds
// them already tested.
package owner

import "strings"

// Entities that have an owner.
const (
	Delivery = "delivery" // the device whose id prefixes To; with no To yet, the device that wrote its first row
	Lease    = "lease"    // the holder's device
	Ask      = "ask"      // the asking agent's device
	Revive   = "revive"   // the target session's device
)

// Rule is one row of the table.
type Rule struct {
	Entity string
	// OwnerKinds only the owner writes.
	OwnerKinds []string
	// RequestKinds anyone may write; the owner applies the first it folds.
	RequestKinds []string
}

// Table is §6.0's table. Kinds marked "v0.4" are reserved and not written
// yet; the store's kind registry classifies them already.
var Table = []Rule{
	{Entity: Delivery,
		OwnerKinds:   []string{"delivery.state", "wake.issued", "turn.engaged", "turn.ended"},
		RequestKinds: []string{"delivery.decided"}}, // v0.4
	{Entity: Lease,
		OwnerKinds:   []string{"lease.acquired", "lease.released", "lease.refused"},
		RequestKinds: []string{"lease.stolen"}}, // v0.4, actor user only
	{Entity: Ask,
		OwnerKinds:   []string{"ask.resolved"},  // v0.4
		RequestKinds: []string{"ask.answered"}}, // v0.4
	{Entity: Revive,
		RequestKinds: []string{"revive.requested"}}, // v0.4; the owner's row is the wake itself
}

var ownerKinds = func() map[string]string {
	m := map[string]string{}
	for _, r := range Table {
		for _, k := range r.OwnerKinds {
			m[k] = r.Entity
		}
	}
	return m
}()

// EntityOf reports the entity an owner-only kind belongs to.
func EntityOf(kind string) (string, bool) {
	e, ok := ownerKinds[kind]
	return e, ok
}

// DeviceOf is the device a sender key lives on: "<device>/<session>" and
// "user:<device>" name it; anything else ("irori", a repo address) names
// none and returns "".
func DeviceOf(key string) string {
	if dev, ok := strings.CutPrefix(key, "user:"); ok {
		return dev
	}
	if i := strings.IndexByte(key, '/'); i > 0 {
		return key[:i]
	}
	return ""
}

// MayWrite reports whether writerDevice may write an owner-only row for the
// entity owned through ownerKey (an agent key or "user:<device>"). A key
// that names no device has no owner to enforce, so any writer may.
func MayWrite(ownerKey, writerDevice string) bool {
	dev := DeviceOf(ownerKey)
	return dev == "" || dev == writerDevice
}
