package inbox

import (
	"context"
	"fmt"
	"slices"
	"strings"

	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/presence"
)

// BypassMode is the PermissionMode value that means permissions are bypassed.
const BypassMode = "bypassPermissions"

// listed reports whether the policy list names sender by its key or by a
// name (names compare case-insensitively). Block (strict) also matches the
// names the sender sent under: that only holds more. Allow matches the card
// name presence shows for the key (or the last one known), never a name the
// sender chose for itself, so nobody gets allowed by claiming a name.
func (b *inbox) listed(list []string, from Sender, cards map[string]presence.AgentCard, strict bool) bool {
	var names []string
	if c, ok := cards[from.Key]; ok {
		names = append(names, strings.ToLower(strings.TrimSpace(c.Name)))
	} else if n := b.known[from.Key]; n != "" {
		names = append(names, n)
	}
	if strict {
		names = append(names, strings.ToLower(strings.TrimSpace(from.Name)))
		names = append(names, b.aliases[from.Key]...)
	}
	for _, e := range list {
		e = strings.TrimSpace(e)
		if e == "" {
			continue
		}
		if strings.EqualFold(e, from.Key) {
			return true
		}
		le := strings.ToLower(e)
		for _, n := range names {
			if n != "" && n == le {
				return true
			}
		}
	}
	return false
}

// bypass reports whether key runs with bypassed permissions. Grok members,
// people and unknown modes count as not bypassed.
func bypass(key string, cards map[string]presence.AgentCard) bool {
	c, ok := cards[key]
	return ok && c.Agent != "grok" && c.PermissionMode == BypassMode
}

// ownAgent reports whether m's sender is one of the person's own Claude
// agents (decision 34): kind "claude", on this device or on a device
// certified to the person and not revoked, and the device its key names is
// the one that wrote the message (a device can't claim another's agent).
// Their messages skip the bypass_receiver hold; Grok members, agents on
// unknown or removed devices and (with Shared Boards) other people's agents
// don't.
func (b *inbox) ownAgent(m *Message, cards map[string]presence.AgentCard) bool {
	s := m.From
	if s.Kind != "claude" || strings.HasPrefix(s.Key, "user:") {
		return false
	}
	if c, ok := cards[s.Key]; ok && c.Agent == "grok" {
		return false
	}
	dev := owner.DeviceOf(s.Key)
	switch {
	case dev == "" || dev != m.writer:
		return false
	case dev == b.dev:
		return true
	}
	return b.isOwner("user:"+dev) && !b.revoked(dev)
}

// revoked reports a device removed from the person (Options.Revoked).
func (b *inbox) revoked(dev string) bool { return b.revokedFn != nil && b.revokedFn(dev) }

// hold evaluates the policy for m going to recipient: the hold reason, or
// "" to queue it. Order: blocked, not_allowed, bypass_receiver, hop_limit,
// paused. People at the GUI skip all but the pause; Irori's own notices
// (SystemSender) skip all but the block list and the pause; the person's
// own Claude agents on any of their devices skip bypass_receiver
// (decision 34).
func (b *inbox) hold(m *Message, recipient string, cards map[string]presence.AgentCard) HoldReason {
	if recipient == "" {
		return HoldUnowned
	}
	if m.From.isSystem() {
		if b.listed(b.pol.Block, m.From, cards, true) {
			return HoldBlocked
		}
	} else if !b.isPerson(m.From) {
		allowed := b.listed(b.pol.Allow, m.From, cards, false)
		switch {
		case b.listed(b.pol.Block, m.From, cards, true):
			return HoldBlocked
		case b.pol.Mode == Allowlist && !allowed:
			return HoldNotAllowed
		case bypass(recipient, cards) && !allowed && !b.ownAgent(m, cards) &&
			(m.From.Kind == "grok" || !bypass(m.From.Key, cards)):
			return HoldBypassReceiver
		case m.Hop > MaxHop:
			return HoldHopLimit
		}
	}
	if b.pol.Paused {
		return HoldPaused
	}
	return ""
}

// placed sets r's state from a policy verdict.
func placed(r deliveryRow, h HoldReason) deliveryRow {
	r.WakeID = ""
	if h == "" {
		r.State, r.Hold = Queued, ""
	} else {
		r.State, r.Hold = Held, h
	}
	return r
}

func (b *inbox) policy() Policy {
	b.mu.Lock()
	defer b.mu.Unlock()
	return normPolicy(b.pol)
}

func (b *inbox) setPolicy(ctx context.Context, p Policy, by string) error {
	switch p.Mode {
	case "":
		p.Mode = Everyone
	case Everyone, Allowlist:
	default:
		return fmt.Errorf("inbox: unknown policy mode %q (everyone or allowlist)", p.Mode)
	}
	clean := func(in []string) []string {
		out := []string{}
		for _, s := range in {
			if s = strings.TrimSpace(s); s != "" && !slices.Contains(out, s) {
				out = append(out, s)
			}
		}
		return out
	}
	p.Block, p.Allow = clean(p.Block), clean(p.Allow)
	b.mu.Lock()
	defer b.unlock()
	wasPaused := b.pol.Paused
	if err := b.emitLocal(ctx, by, KindPolicySet, policyRow{Policy: p, By: by, At: b.now()}); err != nil {
		return err
	}
	if !wasPaused || p.Paused {
		return nil
	}
	// Unpaused: run what's left of the policy over every paused hold.
	cards, err := b.liveCards(ctx)
	if err != nil {
		return err
	}
	for _, id := range b.order {
		d := b.dels[id]
		if d.State != Held || d.Hold != HoldPaused {
			continue
		}
		m := b.msgs[d.MessageID]
		if m == nil {
			continue
		}
		if err := b.putDelivery(ctx, placed(d.row(), b.hold(m, d.To, cards)), by); err != nil {
			return err
		}
	}
	return nil
}
