package inbox

import (
	"encoding/json"

	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/store"
)

// writerOK applies the owner table (v0.4 §6.0) to one row before it folds:
// after a delivery's first row (the sender's), only the device whose id
// prefixes its To writes delivery.state, or the device that wrote the first
// row while To is still empty; wake.issued and turn.* come only from the
// woken agent's device. A row from any other device is ignored and counted.
// On one device every owner is this device, so nothing changes there.
func (b *inbox) writerOK(ev store.Event) bool {
	switch ev.Kind {
	case KindDelivery:
		var r deliveryRow
		if json.Unmarshal(ev.Payload, &r) != nil {
			return true // apply reports it
		}
		d := b.dels[r.DeliveryID]
		switch {
		case d == nil && r.To != "" && !owner.MayWrite(r.To, ev.DeviceID):
			// The sender's first row for another device's agent is only a
			// suggestion (§6.3): queued, with no hold decision and no wake,
			// so the host's own hold() always runs on it.
			return r.State == Queued && r.Hold == "" && r.WakeID == ""
		case d == nil:
			return true // the first row: the sender's device writes it
		case d.To != "":
			return owner.MayWrite(d.To, ev.DeviceID)
		default:
			return d.firstWriter == "" || d.firstWriter == ev.DeviceID
		}
	case KindWakeIssued:
		var r struct {
			To string `json:"to"`
		}
		if json.Unmarshal(ev.Payload, &r) != nil {
			return true
		}
		return owner.MayWrite(r.To, ev.DeviceID)
	case KindTurnEngaged, KindTurnEnded:
		var r struct {
			Agent string `json:"agent"`
		}
		if json.Unmarshal(ev.Payload, &r) != nil {
			return true
		}
		return owner.MayWrite(r.Agent, ev.DeviceID)
	}
	return true
}

// isPerson reports whether s is the person at a GUI: kind "user" and a key
// that isOwner accepts. A sender claiming kind "user" under a made-up key
// gets no person exemption (policy, rate limit, trust, delivery mode).
func (b *inbox) isPerson(s Sender) bool { return s.IsUser() && b.isOwner(s.Key) }

// IgnoredRows counts owner-only rows the inbox ignored because a device
// other than the owner wrote them (Devices shows it in v0.4).
func (b *Inbox) IgnoredRows() int {
	b.impl.mu.Lock()
	defer b.impl.mu.Unlock()
	return b.impl.ignored
}

// ignore counts one row of dev ignored by the writer rules (§6.0). b.mu is
// held.
func (b *inbox) ignore(dev string) {
	b.ignored++
	if b.ignoredBy == nil {
		b.ignoredBy = map[string]int{}
	}
	b.ignoredBy[dev]++
}

// IgnoredBy counts the ignored rows per writing device, for Devices' warning
// line (§8, §9).
func (b *Inbox) IgnoredBy() map[string]int {
	b.impl.mu.Lock()
	defer b.impl.mu.Unlock()
	out := make(map[string]int, len(b.impl.ignoredBy))
	for d, n := range b.impl.ignoredBy {
		out[d] = n
	}
	return out
}
