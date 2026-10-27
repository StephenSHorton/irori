package inbox

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
)

// liveCards is the roster minus gone cards, by key. A card name the inbox
// hasn't recorded for its key is logged (KindAgentName), so it stays
// addressable after the agent goes, the same before and after a restart.
func (b *inbox) liveCards(ctx context.Context) (map[string]presence.AgentCard, error) {
	out := b.rosterCards()
	keys := make([]string, 0, len(out))
	for k := range out {
		keys = append(keys, k)
	}
	slices.Sort(keys)
	for _, k := range keys {
		name := strings.ToLower(strings.TrimSpace(out[k].Name))
		if n, ok := b.known[k]; ok && n == name {
			continue
		}
		if err := b.emit(ctx, "irori", KindAgentName, nameRow{Key: k, Name: name, At: b.now()}); err != nil {
			return nil, err
		}
	}
	return out, nil
}

// rosterCards is the roster minus gone cards, by key, recording nothing.
func (b *inbox) rosterCards() map[string]presence.AgentCard {
	out := map[string]presence.AgentCard{}
	for _, c := range b.roster.Agents() {
		if c.Key == "" || c.Status == "gone" {
			continue
		}
		out[c.Key] = c
	}
	return out
}

// sessionPart is the session id of "<device>/<session>".
func sessionPart(key string) string {
	if i := strings.LastIndexByte(key, '/'); i >= 0 {
		return key[i+1:]
	}
	return key
}

// describe is "name (key)" for error text.
func (b *inbox) describe(key string, cards map[string]presence.AgentCard) string {
	name := ""
	if c, ok := cards[key]; ok {
		name = c.Name
	} else if n := b.known[key]; n != "" {
		name = n
	} else if a := b.aliases[key]; len(a) > 0 {
		name = a[len(a)-1]
	}
	if name == "" {
		return key
	}
	return fmt.Sprintf("%s (%s)", name, key)
}

// resolveAgent turns a key, key prefix, name or alias into one agent key.
// Exact key beats a unique prefix (>= MinKeyPrefixLen, of the key or its
// session id), which beats a card name, which beats an alias. Live agents
// win over gone ones; an agent that has gone but was seen before is still a
// valid target.
func (b *inbox) resolveAgent(self, value string, cards map[string]presence.AgentCard) (string, error) {
	wanted := strings.ToLower(strings.TrimSpace(value))
	if wanted == "" {
		return "", fmt.Errorf("%w: say which agent (to)", ErrUnknownAgent)
	}
	// Candidates: everyone live or ever seen.
	all := map[string]bool{}
	for k := range cards {
		all[k] = true
	}
	for k := range b.known {
		all[k] = true
	}
	for k := range b.aliases {
		if !isUserKey(k) {
			all[k] = true
		}
	}
	pick := func(matches []string) (string, error) {
		slices.Sort(matches)
		if len(matches) == 1 && matches[0] == self {
			return "", ErrSelf
		}
		matches = slices.DeleteFunc(matches, func(k string) bool { return k == self })
		switch len(matches) {
		case 0:
			return "", nil
		case 1:
			return matches[0], nil
		}
		names := make([]string, len(matches))
		for i, k := range matches {
			names[i] = b.describe(k, cards)
		}
		return "", fmt.Errorf("%w: %q matches %d agents: %s; use a key", ErrAmbiguous, value, len(matches), strings.Join(names, ", "))
	}

	// 1. Exact key.
	for k := range all {
		if strings.ToLower(k) == wanted {
			return pick([]string{k})
		}
	}
	// 2. Unique key prefix.
	if len(wanted) >= MinKeyPrefixLen {
		var m []string
		for k := range all {
			lk := strings.ToLower(k)
			if strings.HasPrefix(lk, wanted) || strings.HasPrefix(strings.ToLower(sessionPart(k)), wanted) {
				m = append(m, k)
			}
		}
		if key, err := pick(m); err != nil || key != "" {
			return key, err
		}
	}
	// 3. Name, then alias: an agent's card name (live, else the last one
	// known) beats a name some agent sent under, so nobody takes over
	// another agent's name by sending as it. Within each, live agents come
	// before ones that have gone.
	cardNamed := func(k string) bool {
		if c, ok := cards[k]; ok {
			return strings.ToLower(strings.TrimSpace(c.Name)) == wanted
		}
		return b.known[k] == wanted
	}
	aliased := func(k string) bool { return slices.Contains(b.aliases[k], wanted) }
	sawSelf := false
	for _, named := range []func(string) bool{cardNamed, aliased} {
		var live, gone []string
		for k := range all {
			switch {
			case !named(k):
			case k == self:
				sawSelf = true
			case cards[k].Key != "":
				live = append(live, k)
			default:
				gone = append(gone, k)
			}
		}
		for _, m := range [][]string{live, gone} {
			if key, err := pick(m); err != nil || key != "" {
				return key, err
			}
		}
	}
	if sawSelf {
		return "", ErrSelf
	}
	var here []string
	for k, c := range cards {
		if k != self {
			here = append(here, c.Name)
		}
	}
	slices.Sort(here)
	list := "none"
	if len(here) > 0 {
		list = strings.Join(here, ", ")
	}
	return "", fmt.Errorf("%w: no agent called %q; agents here now: %s", ErrUnknownAgent, value, list)
}

// reachable: agent has a wake path, or its mod path dropped less than the
// reattach grace ago.
func (b *inbox) reachable(agent string, now time.Time) bool {
	r := b.reach[agent]
	if r == nil {
		return false
	}
	return r.channel || r.mod || (!r.modLost.IsZero() && now.Sub(r.modLost) < b.lim.reattach)
}

func (b *inbox) setReachable(agent string, path Path, on bool) {
	if agent == "" {
		return
	}
	b.mu.Lock()
	defer b.unlock()
	now := b.now()
	r := b.reach[agent]
	if r == nil {
		r = &reachRec{}
		b.reach[agent] = r
	}
	was := b.reachable(agent, now)
	switch path {
	case PathMod:
		if r.mod && !on {
			r.modLost = now
		}
		r.mod = on
		if on {
			r.modLost = time.Time{}
		}
	case PathChannel:
		r.channel = on
	default:
		return
	}
	if on && !was {
		r.firstAt = now
	}
	b.touch(agent)
	b.note(Change{Kind: Updated, To: agent})
}

// desks lists the front-desk candidates for repo, best first: live agents
// in that repo that are reachable, by the start of their reachable stretch.
func (b *inbox) desks(repo string, cards map[string]presence.AgentCard, now time.Time) []string {
	if repo == "" {
		return nil
	}
	var out []string
	for k, c := range cards {
		if c.RepoKey == repo && b.reachable(k, now) {
			out = append(out, k)
		}
	}
	slices.SortFunc(out, func(x, y string) int {
		if c := b.reach[x].firstAt.Compare(b.reach[y].firstAt); c != 0 {
			return c
		}
		return strings.Compare(x, y)
	})
	return out
}

// desk is the first candidate that isn't one of exclude, or "".
func (b *inbox) desk(repo string, cards map[string]presence.AgentCard, now time.Time, exclude ...string) string {
	for _, k := range b.desks(repo, cards, now) {
		if !slices.Contains(exclude, k) {
			return k
		}
	}
	return ""
}

// Reachable reports the wake paths agent has open right now: an attached
// Irori mod (mod) and a bound Grok member stream (channel). Unlike routing,
// a mod path that just dropped doesn't count, so the GUI's fidelity badge
// stops saying "mod" as soon as the attach stream goes away.
func (b *Inbox) Reachable(agent string) (mod, channel bool) {
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	if r := x.reach[agent]; r != nil {
		return r.mod, r.channel
	}
	return false, false
}

// RecordRoster logs the live roster's names the way every send does, so an
// agent live now stays addressable by key after it goes. A caller that
// will send to an agent later (the notify tool's answer, after the person
// chooses) calls it while that agent is still here: a Grok member's card
// is not presence, so nothing else may have recorded it.
func (b *Inbox) RecordRoster(ctx context.Context) error {
	x := b.impl
	x.mu.Lock()
	defer x.unlock()
	_, err := x.liveCards(ctx)
	return err
}

// FrontDesk reports the agent that repo-addressed work for repoKey goes to
// now, or "" when no live agent in that repo is reachable.
func (b *Inbox) FrontDesk(repoKey string) string {
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	return x.desk(repoKey, x.rosterCards(), x.now())
}
