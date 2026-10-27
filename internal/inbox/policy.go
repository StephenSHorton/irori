package inbox

import (
	"context"
	"fmt"
	"slices"
	"strings"

	"github.com/StephenSHorton/irori/internal/presence"
)

// BypassMode is the PermissionMode value that means permissions are bypassed.
const BypassMode = "bypassPermissions"

// listed reports whether the policy list names sender: its key, its name,
// or one of its aliases (names compare case-insensitively).
func (b *inbox) listed(list []string, from Sender) bool {
	name := strings.ToLower(strings.TrimSpace(from.Name))
	for _, e := range list {
		e = strings.TrimSpace(e)
		if e == "" {
			continue
		}
		le := strings.ToLower(e)
		if strings.EqualFold(e, from.Key) || (name != "" && le == name) || slices.Contains(b.aliases[from.Key], le) {
			return true
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

// hold evaluates the policy for m going to recipient: the hold reason, or
// "" to queue it. Order: blocked, not_allowed, bypass_receiver, hop_limit,
// paused. People at the GUI skip all but the pause.
func (b *inbox) hold(m *Message, recipient string, cards map[string]presence.AgentCard) HoldReason {
	if recipient == "" {
		return HoldUnowned
	}
	if !m.From.IsUser() {
		allowed := b.listed(b.pol.Allow, m.From)
		switch {
		case b.listed(b.pol.Block, m.From):
			return HoldBlocked
		case b.pol.Mode == Allowlist && !allowed:
			return HoldNotAllowed
		case bypass(recipient, cards) && !allowed && (m.From.Kind == "grok" || !bypass(m.From.Key, cards)):
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
	if err := b.emit(ctx, by, KindPolicySet, policyRow{Policy: p, By: by, At: b.now()}); err != nil {
		return err
	}
	if !wasPaused || p.Paused {
		return nil
	}
	// Unpaused: run what's left of the policy over every paused hold.
	cards := b.liveCards()
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
