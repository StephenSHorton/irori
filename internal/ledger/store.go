package ledger

import (
	"sort"
	"strings"
	"sync"
	"time"
)

var roleRank = map[Role]int{Author: 0, Touched: 1, Reviewer: 2}

type ledger struct {
	mu      sync.RWMutex
	entries []Entry
	index   map[string]int // identity -> position in entries
}

func newLedger() *Ledger {
	return &Ledger{impl: &ledger{index: map[string]int{}}}
}

func identity(e Entry) string {
	return e.Key + "\x00" + e.Session + "\x00" + string(e.Role)
}

func (l *ledger) add(e Entry) bool {
	if e.Key == "" || e.Session == "" || e.At.IsZero() {
		return false
	}
	if _, ok := roleRank[e.Role]; !ok {
		return false
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	id := identity(e)
	i, ok := l.index[id]
	if !ok {
		l.index[id] = len(l.entries)
		l.entries = append(l.entries, e)
		return true
	}
	old := &l.entries[i]
	changed := false
	if e.At.Before(old.At) {
		old.At = e.At
		changed = true
	}
	if old.URL == "" && e.URL != "" {
		old.URL = e.URL
		changed = true
	}
	return changed
}

// live reports whether e is still in the view at now. Like tsukumo, an entry
// exactly Keep old is kept.
func live(e Entry, now time.Time) bool {
	return !e.At.Before(now.Add(-Keep))
}

func (l *ledger) collect(keep func(Entry) bool, now time.Time) []Entry {
	l.mu.RLock()
	defer l.mu.RUnlock()
	var out []Entry
	for _, e := range l.entries {
		if keep(e) && live(e, now) {
			out = append(out, e)
		}
	}
	return out
}

// ownerLess is the Owners order: role rank, earliest At, then Session.
func ownerLess(a, b Entry) bool {
	if ra, rb := roleRank[a.Role], roleRank[b.Role]; ra != rb {
		return ra < rb
	}
	if !a.At.Equal(b.At) {
		return a.At.Before(b.At)
	}
	return a.Session < b.Session
}

func (l *ledger) owners(k string, now time.Time) []Entry {
	out := l.collect(func(e Entry) bool { return e.Key == k }, now)
	sort.SliceStable(out, func(i, j int) bool { return ownerLess(out[i], out[j]) })
	return out
}

func (l *ledger) forRepo(repo string, now time.Time) []Entry {
	repo = strings.TrimSpace(repo)
	out := l.collect(func(e Entry) bool { return repo == "" || strings.EqualFold(e.Repo, repo) }, now)
	sort.SliceStable(out, func(i, j int) bool {
		a, b := out[i], out[j]
		if a.Number != b.Number {
			return a.Number > b.Number
		}
		if a.Key != b.Key {
			return a.Key < b.Key
		}
		return ownerLess(a, b)
	})
	return out
}

func (l *ledger) forSession(s string, now time.Time) []Entry {
	out := l.collect(func(e Entry) bool { return e.Session == s }, now)
	sort.SliceStable(out, func(i, j int) bool {
		a, b := out[i], out[j]
		if !a.At.Equal(b.At) {
			return a.At.After(b.At)
		}
		if a.Key != b.Key {
			return a.Key < b.Key
		}
		return roleRank[a.Role] < roleRank[b.Role]
	})
	return out
}
