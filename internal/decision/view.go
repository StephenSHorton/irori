package decision

import (
	"slices"
	"sort"
	"strings"
	"sync"
)

type view struct {
	mu           sync.RWMutex
	byID         map[string]entry
	supersededBy map[string][]string // target -> superseding ids, in add order
	seq          int
}

type entry struct {
	d   Decision
	seq int
}

func newView() *View {
	return &View{impl: &view{byID: map[string]entry{}, supersededBy: map[string][]string{}}}
}

func (v *view) add(d Decision) {
	if d.ID == "" {
		return
	}
	v.mu.Lock()
	defer v.mu.Unlock()
	if _, ok := v.byID[d.ID]; ok {
		return
	}
	d = clone(d)
	v.seq++
	v.byID[d.ID] = entry{d: d, seq: v.seq}
	for _, t := range d.Supersedes {
		if t == "" || t == d.ID || slices.Contains(v.supersededBy[t], d.ID) {
			continue
		}
		v.supersededBy[t] = append(v.supersededBy[t], d.ID)
	}
}

func (v *view) get(id string) (Decision, bool) {
	v.mu.RLock()
	defer v.mu.RUnlock()
	e, ok := v.byID[id]
	if !ok {
		return Decision{}, false
	}
	return clone(e.d), true
}

func (v *view) canSupersede(by Party, target string) error {
	v.mu.RLock()
	e, ok := v.byID[target]
	v.mu.RUnlock()
	switch {
	case !ok:
		return ErrNoDecision
	case by.Kind == "user":
		return nil
	case by.Key != "" && by.Key == e.d.By.Key:
		return nil
	}
	return ErrSupersede
}

func (v *view) list(f Filter) []Listed {
	limit := f.Limit
	if limit <= 0 {
		limit = 50
	}
	limit = min(limit, 500)
	fpath := strings.ReplaceAll(f.Path, `\`, "/")
	q := strings.ToLower(strings.TrimSpace(f.Query))

	v.mu.RLock()
	defer v.mu.RUnlock()
	var hits []entry
	for id, e := range v.byID {
		d := e.d
		if !f.All && len(v.supersededBy[id]) > 0 {
			continue
		}
		if f.RepoKey != "" && (d.Anchor == nil || d.Anchor.RepoKey != f.RepoKey) {
			continue
		}
		if fpath != "" && (d.Anchor == nil || d.Anchor.Path != fpath) {
			continue
		}
		if q != "" && !strings.Contains(strings.ToLower(d.Statement), q) && !strings.Contains(strings.ToLower(d.Why), q) {
			continue
		}
		hits = append(hits, e)
	}
	sort.Slice(hits, func(i, j int) bool {
		if a, b := hits[i].d.At, hits[j].d.At; !a.Equal(b) {
			return a.After(b)
		}
		return hits[i].seq > hits[j].seq
	})
	if len(hits) > limit {
		hits = hits[:limit]
	}
	out := make([]Listed, len(hits))
	for i, e := range hits {
		out[i] = Listed{Decision: clone(e.d), SupersededBy: slices.Clone(v.supersededBy[e.d.ID])}
	}
	return out
}

// clone deep-copies a decision so callers never share its slices.
func clone(d Decision) Decision {
	d.Supersedes = slices.Clone(d.Supersedes)
	if d.Anchor != nil {
		a := *d.Anchor
		a.Context = slices.Clone(a.Context)
		d.Anchor = &a
	}
	if d.Source != nil {
		s := *d.Source
		d.Source = &s
	}
	return d
}
