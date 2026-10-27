package inbox

import (
	"fmt"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
)

// prRoute is where a ToPR send goes (design v0.3 §6).
type prRoute struct {
	key      string // the canonical ledger key, "pr:<owner/name>#<n>"
	to       string // recipient; "" = unowned
	repo     string // the repo's front-desk key; "" when an owner takes it
	forOwner string // the owner it would have gone to, "name (key)"; "" when an owner takes it or there is none
}

// NormalizePR turns "pr:owner/name#n", "owner/name#n" or a GitHub PR URL
// into the ledger key "pr:<owner/name>#<n>" (lowercase).
func NormalizePR(s string) (string, error) {
	ref, ok := ledger.ParseTarget(s)
	if !ok {
		if strings.TrimSpace(s) == "" {
			return "", fmt.Errorf("%w: say which PR", ErrBadPR)
		}
		return "", fmt.Errorf("%w: %q; use pr:<owner>/<name>#<number>, <owner>/<name>#<number> or a github.com/<owner>/<name>/pull/<number> URL", ErrBadPR, clip(s, 120))
	}
	return ledger.Key(ref.Repo, ref.Number), nil
}

// routePR picks a PR message's recipient: the best-ranked owner that is
// live (in the roster and not gone; reachability isn't needed, as for
// agent-addressed work), else the repo's front desk with Delivery.For naming
// the owner it would have gone to, else nobody (held unowned, like repo work
// with no desk). The sender is never its own recipient: an owner that is
// the sender is passed over.
//
// Owner-routed deliveries carry no RepoKey, so they stay with that session
// like agent-addressed work (until it has gone for good: then the sweeper
// hands them to the repo, see prFallbackRepo); desk and unowned ones carry the
// repo's key and follow the repo rules (sweeper moves, takeover when a desk
// appears).
func (b *inbox) routePR(self, value string, cards map[string]presence.AgentCard, now time.Time) (prRoute, error) {
	key, err := NormalizePR(value)
	if err != nil {
		return prRoute{}, err
	}
	ref, _ := ledger.ParseTarget(key)
	r := prRoute{key: key}
	var owners []string
	if b.owners != nil {
		owners = b.owners(key)
	}
	would := ""
	for _, o := range owners {
		if o == "" || o == self {
			continue
		}
		if _, live := cards[o]; live {
			r.to = o
			return r, nil
		}
		if would == "" {
			would = o
		}
	}
	r.repo = ledger.RepoKey(ref.Repo)
	r.to = b.desk(r.repo, cards, now, self)
	if would != "" {
		r.forOwner = clip(b.describe(would, cards), MaxNote)
	}
	return r, nil
}

// prFallbackRepo is the repo key a PR message's work falls back to: its delivery's
// RepoKey, or for owner-routed work (no RepoKey) the PR's repo, so work
// whose owner has gone for good still reaches the front desk. "" for a
// message not addressed to a PR.
func prFallbackRepo(m *Message, d *drec) string {
	if d.RepoKey != "" || m == nil || m.To.Kind != ToPR {
		return d.RepoKey
	}
	ref, ok := ledger.ParseTarget(m.To.Value)
	if !ok {
		return ""
	}
	return ledger.RepoKey(ref.Repo)
}
