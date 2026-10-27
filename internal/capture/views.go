package capture

import (
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/ledger"
)

// cloneEdit copies an edit row for a caller.
func cloneEdit(e *EditRow) EditRow { return *e }

// cloneTurn copies a turn row with its slices.
func cloneTurn(t *TurnRow) TurnRow {
	c := *t
	c.Tools = slices.Clone(t.Tools)
	c.EditPaths = slices.Clone(t.EditPaths)
	return c
}

func cloneLink(l *CommitLink) *CommitLink {
	if l == nil {
		return nil
	}
	c := *l
	c.Files = slices.Clone(l.Files)
	c.Sessions = slices.Clone(l.Sessions)
	c.Turns = slices.Clone(l.Turns)
	if l.Committer != nil {
		w := *l.Committer
		c.Committer = &w
	}
	return &c
}

func (rec *commitRec) view() CommitView {
	row := rec.row
	row.Files = slices.Clone(rec.row.Files)
	row.Parents = slices.Clone(rec.row.Parents)
	return CommitView{CommitRow: row, Link: cloneLink(rec.link)}
}

// timeline: see Capture.Timeline. c.mu is read-locked.
func (c *capture) timeline(q TimelineQuery) Timeline {
	limit := q.Limit
	if limit <= 0 {
		limit = 20
	}
	limit = min(limit, 100)
	out := Timeline{Key: q.Key, Entries: []Entry{}}
	if q.Key == "" {
		return out
	}
	live := false
	if q.Before.IsZero() {
		for _, card := range c.liveCards() {
			if card.Key == q.Key {
				live = true
				break
			}
		}
	}
	before := func(t time.Time) bool { return q.Before.IsZero() || t.Before(q.Before) }

	c.mu.RLock()
	defer c.mu.RUnlock()
	var entries []Entry
	if live {
		var opens []Entry
		for k, t := range c.open {
			key, agent, _ := strings.Cut(k, "|")
			if key != q.Key || (agent != "" && !q.Subagents) {
				continue
			}
			root, rk := c.cachedRepo(t.Cwd)
			clean := *t
			clean.EditPaths = c.stripNoise(q.Key, clean)
			tv := &TurnView{
				TurnRow: TurnRow{ID: turnRowID(q.Key, t.AgentID, t.ID), Key: q.Key, RepoKey: rk, Root: root, TurnID: t.ID, Turn: c.scrubTurn(clean, root)},
				Open:    true,
				Edits:   c.turnEdits(q.Key, t.AgentID, t.ID),
			}
			opens = append(opens, Entry{Kind: EntryTurn, At: t.StartedAt, Turn: tv})
		}
		sort.Slice(opens, func(i, j int) bool { return opens[i].At.After(opens[j].At) })
		out.Entries = append(out.Entries, opens...)
	}
	for _, t := range c.turns[q.Key] {
		if (t.AgentID != "" && !q.Subagents) || !before(t.StartedAt) {
			continue
		}
		tv := &TurnView{TurnRow: cloneTurn(t), Edits: c.turnEdits(t.Key, t.AgentID, t.Turn.ID)}
		entries = append(entries, Entry{Kind: EntryTurn, At: t.StartedAt, Turn: tv})
	}
	for _, s := range c.summaries[q.Key] {
		if (s.AgentID != "" && !q.Subagents) || !before(s.At) {
			continue
		}
		sc := *s
		entries = append(entries, Entry{Kind: EntrySummary, At: s.At, Summary: &sc})
	}
	for _, e := range c.ledger.ForSession(q.Key, c.now()) {
		if !before(e.At) {
			continue
		}
		ec := e
		entries = append(entries, Entry{Kind: EntryPR, At: e.At, PR: &ec})
	}
	for _, rec := range c.commitMap {
		l := rec.link
		if l == nil || !before(rec.row.At) {
			continue
		}
		if (l.Committer != nil && l.Committer.Key == q.Key) || slices.Contains(l.Sessions, q.Key) {
			v := rec.view()
			entries = append(entries, Entry{Kind: EntryCommit, At: rec.row.At, Commit: &v})
		}
	}
	sort.SliceStable(entries, func(i, j int) bool { return entries[i].At.After(entries[j].At) })
	turns := 0
	for _, e := range entries {
		if e.Kind == EntryTurn {
			if turns == limit {
				out.More = true
				break
			}
			turns++
		}
		out.Entries = append(out.Entries, e)
	}
	return out
}

// turnEdits lists a turn's edits in order; c.mu is read-locked.
func (c *capture) turnEdits(key, agentID, turnID string) []EditRow {
	es := c.editsByTurn[turnKey(key, agentID, turnID)]
	if len(es) == 0 {
		return nil
	}
	out := make([]EditRow, len(es))
	for i, e := range es {
		out[i] = cloneEdit(e)
	}
	return out
}

// cachedRepo is a directory's root and key from what is already known (no
// git run); c.mu is read-locked.
func (c *capture) cachedRepo(dir string) (string, string) {
	if dir == "" {
		return "", ""
	}
	d := normRoot(dir)
	best := repoSeen{}
	for n, r := range c.repoRoots {
		if (d == n || strings.HasPrefix(d, n+string(filepath.Separator))) && len(r.Root) > len(best.Root) {
			best = r
		}
	}
	return best.Root, best.RepoKey
}

func (c *capture) edits(q EditQuery) []EditRow {
	limit := q.Limit
	if limit <= 0 {
		limit = 50
	}
	limit = min(limit, 500)
	path := strings.ReplaceAll(q.Path, `\`, "/")
	c.mu.RLock()
	defer c.mu.RUnlock()
	var hits []*EditRow
	for _, e := range c.editList {
		if (q.RepoKey != "" && e.RepoKey != q.RepoKey) || (path != "" && e.Path != path) || (q.Key != "" && e.Key != q.Key) {
			continue
		}
		hits = append(hits, e)
	}
	sort.SliceStable(hits, func(i, j int) bool { return hits[i].At.After(hits[j].At) })
	if len(hits) > limit {
		hits = hits[:limit]
	}
	out := make([]EditRow, len(hits))
	for i, e := range hits {
		out[i] = cloneEdit(e)
	}
	return out
}

func (c *capture) commits(repoKey string, limit int) []CommitView {
	if limit <= 0 {
		limit = 50
	}
	limit = min(limit, 500)
	c.mu.RLock()
	defer c.mu.RUnlock()
	var recs []*commitRec
	for _, rec := range c.commitMap {
		if repoKey == "" || rec.row.RepoKey == repoKey {
			recs = append(recs, rec)
		}
	}
	sort.SliceStable(recs, func(i, j int) bool {
		if !recs[i].row.At.Equal(recs[j].row.At) {
			return recs[i].row.At.After(recs[j].row.At)
		}
		return recs[i].seenAt.After(recs[j].seenAt)
	})
	if len(recs) > limit {
		recs = recs[:limit]
	}
	out := make([]CommitView, len(recs))
	for i, rec := range recs {
		out[i] = rec.view()
	}
	return out
}

// commit finds a sha in any repo; a unique prefix of 7+ hex digits works too.
func (c *capture) commit(sha string) (CommitView, bool) {
	sha = strings.ToLower(strings.TrimSpace(sha))
	if len(sha) < 7 {
		return CommitView{}, false
	}
	c.mu.RLock()
	defer c.mu.RUnlock()
	var hit *commitRec
	for _, rec := range c.commitMap {
		s := strings.ToLower(rec.row.SHA)
		if s == sha {
			return rec.view(), true
		}
		if strings.HasPrefix(s, sha) {
			if hit != nil && !strings.EqualFold(hit.row.SHA, s) {
				return CommitView{}, false // ambiguous
			}
			hit = rec
		}
	}
	if hit == nil {
		return CommitView{}, false
	}
	return hit.view(), true
}

// commitIn is one repo's commit by full sha; c.mu is read-locked.
func (c *capture) commitIn(repoKey, sha string) (CommitView, bool) {
	rec := c.commitMap[commitKey(repoKey, sha)]
	if rec == nil {
		return CommitView{}, false
	}
	return rec.view(), true
}

func (c *capture) repos() map[string][]string {
	c.mu.RLock()
	defer c.mu.RUnlock()
	out := map[string][]string{}
	for _, r := range c.repoRoots {
		out[r.RepoKey] = append(out[r.RepoKey], r.Root)
	}
	for _, roots := range out {
		sort.Strings(roots)
	}
	return out
}

func (c *capture) blob(id string) ([]byte, error) { return c.blobs.Get(id) }

func (c *capture) prOwners(prKey string) []string {
	var out []string
	for _, e := range c.ledger.Owners(prKey, c.now()) {
		if !slices.Contains(out, e.Session) {
			out = append(out, e.Session)
		}
	}
	return out
}

func (c *capture) prs(repo string) []ledger.Entry { return c.ledger.ForRepo(repo, c.now()) }
