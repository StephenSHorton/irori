package capture

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strconv"
	"strings"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/decision"
)

// maxWhy caps WhyResult's Edits and Turns.
const maxWhy = 10

// locate resolves a path (absolute, or relative to cwd or to repoKey's
// root) to its worktree root, repo key, absolute and repo-relative forms.
func (c *capture) locate(ctx context.Context, path, cwd, repoKey string) (root, rk, abs, rel string, err error) {
	path = strings.TrimSpace(path)
	if path == "" {
		return "", "", "", "", ErrNoRepo
	}
	if filepath.IsAbs(filepath.FromSlash(path)) {
		abs = filepath.Clean(filepath.FromSlash(path))
		root, rk = c.repoFor(ctx, filepath.Dir(abs))
	} else {
		cwdRoot := ""
		if cwd != "" {
			cwdRoot, rk = c.repoFor(ctx, cwd)
			root = cwdRoot
		}
		if repoKey != "" && rk != repoKey {
			root, rk = c.rootOf(ctx, repoKey), repoKey
		}
		if root == "" {
			return "", "", "", "", ErrNoRepo
		}
		// Relative to the caller's cwd when that is in the same worktree
		// and the file is there, else to the root.
		abs = filepath.Clean(filepath.Join(root, filepath.FromSlash(path)))
		if cwdRoot != "" && normRoot(cwdRoot) == normRoot(root) {
			p := filepath.Clean(filepath.Join(cwd, filepath.FromSlash(path)))
			if fileExists(p) || !fileExists(abs) {
				abs = p
			}
		}
	}
	if root == "" {
		return "", "", "", "", ErrNoRepo
	}
	rel, ok := relTo(root, abs)
	if !ok {
		return "", "", "", "", ErrNoRepo
	}
	return root, rk, abs, rel, nil
}

// rootOf is an existing worktree root for a repo key: a live card's, else
// any seen root that is still on disk.
func (c *capture) rootOf(ctx context.Context, repoKey string) string {
	for _, card := range c.liveCards() {
		if card.Cwd == "" || (card.RepoKey != "" && card.RepoKey != repoKey) {
			continue
		}
		if root, rk := c.repoFor(ctx, card.Cwd); root != "" && rk == repoKey {
			return root
		}
	}
	c.mu.RLock()
	var roots []string
	for _, r := range c.repoRoots {
		if r.RepoKey == repoKey {
			roots = append(roots, r.Root)
		}
	}
	c.mu.RUnlock()
	sort.Strings(roots)
	for _, r := range roots {
		if dirExists(r) {
			return r
		}
	}
	return ""
}

func (c *capture) why(ctx context.Context, q WhyQuery) (WhyResult, error) {
	root, rk, _, rel, err := c.locate(ctx, q.Path, q.Cwd, q.RepoKey)
	if err != nil {
		return WhyResult{}, err
	}
	res := WhyResult{RepoKey: rk, Root: root, Path: rel, Line: q.Line, Edits: []EditRow{}}
	uncommitted := true
	if q.Line > 0 {
		cctx, cancel := context.WithTimeout(ctx, gitTimeout)
		n := strconv.Itoa(q.Line)
		out, err := c.git.Run(cctx, root, "blame", "-L", n+","+n, "--porcelain", "--", rel)
		cancel()
		if err == nil {
			if sha, _, _ := strings.Cut(strings.TrimSpace(out), " "); sha != "" && !gitlink.ZeroSHA(sha) {
				res.BlameSHA = sha
				uncommitted = false
			}
		}
	}

	c.mu.RLock()
	defer c.mu.RUnlock()
	var turnKeys []string // turnKey(...) of the turns to show
	addTurn := func(key, agent, turn string) {
		if k := turnKey(key, agent, turn); !slices.Contains(turnKeys, k) {
			turnKeys = append(turnKeys, k)
		}
	}
	switch {
	case res.BlameSHA != "":
		if v, ok := c.commitIn(rk, res.BlameSHA); ok {
			res.Commit = &v
			if v.Link != nil {
				for _, f := range v.Link.Files {
					if f.Path != rel {
						continue
					}
					for i := len(f.Edits) - 1; i >= 0 && len(res.Edits) < maxWhy; i-- {
						if e := c.editByID[f.Edits[i]]; e != nil {
							res.Edits = append(res.Edits, cloneEdit(e))
						}
					}
				}
				for _, e := range res.Edits {
					addTurn(e.Key, e.AgentID, e.TurnID)
				}
				if len(res.Edits) == 0 {
					// No edit of this file: show the turns behind the
					// commit as a whole.
					for _, k := range v.Link.Sessions {
						for _, t := range c.turns[k] {
							if slices.Contains(v.Link.Turns, t.Turn.ID) {
								addTurn(t.Key, t.AgentID, t.Turn.ID)
							}
						}
					}
				}
			}
			if v.Link != nil && v.Link.Committer != nil {
				addTurn(v.Link.Committer.Key, "", v.Link.Committer.TurnID)
			}
		}
	case uncommitted && q.Line > 0:
		var es []*EditRow
		for id := range c.pool[rk] {
			if e := c.editByID[id]; e != nil && e.Path == rel {
				es = append(es, e)
			}
		}
		res.Edits = newest(es)
		for _, e := range res.Edits {
			addTurn(e.Key, e.AgentID, e.TurnID)
		}
	default:
		var es []*EditRow
		for _, e := range c.editList {
			if e.RepoKey == rk && e.Path == rel {
				es = append(es, e)
			}
		}
		res.Edits = newest(es)
		for _, e := range res.Edits {
			addTurn(e.Key, e.AgentID, e.TurnID)
		}
	}
	for _, tk := range turnKeys {
		if len(res.Turns) >= maxWhy {
			break
		}
		if t := c.findTurn(tk); t != nil {
			res.Turns = append(res.Turns, cloneTurn(t))
		}
	}
	for _, d := range c.dview.List(decision.Filter{RepoKey: rk, Path: rel, Limit: 500}) {
		a := d.Anchor
		if q.Line > 0 && a.Start > 0 && (q.Line < a.Start || q.Line > a.End) {
			continue
		}
		res.Decisions = append(res.Decisions, c.checkDecision(d, root))
	}
	return res, nil
}

func fileExists(p string) bool {
	fi, err := os.Stat(p)
	return err == nil && !fi.IsDir()
}

// newest sorts edits newest first and keeps maxWhy.
func newest(es []*EditRow) []EditRow {
	sort.SliceStable(es, func(i, j int) bool { return es[i].At.After(es[j].At) })
	out := []EditRow{}
	for _, e := range es {
		if len(out) == maxWhy {
			break
		}
		out = append(out, cloneEdit(e))
	}
	return out
}

// findTurn looks a turn up by turnKey; c.mu is read-locked.
func (c *capture) findTurn(tk string) *TurnRow {
	key, rest, _ := strings.Cut(tk, "|")
	agent, turn, _ := strings.Cut(rest, "|")
	for _, t := range c.turns[key] {
		if t.Turn.ID == turn && (agent == "" || t.AgentID == agent) {
			return t
		}
	}
	return nil
}

// checkDecision re-checks a decision's anchor. root is a worktree known to
// hold the repo ("" to look one up); c.mu is read-locked.
func (c *capture) checkDecision(d decision.Listed, root string) DecisionView {
	v := DecisionView{Listed: d}
	if d.Anchor == nil {
		return v
	}
	r := ""
	switch {
	case d.Anchor.Root != "" && dirExists(d.Anchor.Root):
		r = d.Anchor.Root
	case root != "" && dirExists(root):
		r = root
	}
	st := decision.Check(*d.Anchor, r)
	v.Status = &st
	return v
}

func (c *capture) recordDecision(ctx context.Context, in DecisionInput) (decision.Decision, error) {
	d := decision.Decision{
		By: in.By, Statement: in.Statement, Why: in.Why, Supersedes: slices.Clone(in.Supersedes),
		Source: in.Source, Trust: "untrusted",
	}
	if in.Trust == "user" {
		d.Trust = "user"
	}
	if err := decision.Validate(&d); err != nil {
		return decision.Decision{}, err
	}
	for _, id := range d.Supersedes {
		if err := c.dview.CanSupersede(d.By, id); err != nil {
			return decision.Decision{}, fmt.Errorf("%w: %s", err, id)
		}
	}
	if strings.TrimSpace(in.Path) != "" {
		start, end, err := decision.ParseLines(in.Lines)
		if err != nil {
			return decision.Decision{}, err
		}
		root, rk, abs, _, err := c.locate(ctx, in.Path, in.Cwd, "")
		if err != nil {
			return decision.Decision{}, err
		}
		cctx, cancel := context.WithTimeout(ctx, gitTimeout)
		head, herr := c.git.Run(cctx, root, "rev-parse", "HEAD")
		cancel()
		if herr != nil {
			head = "" // no commits yet
		}
		a, err := decision.MakeAnchor(root, abs, start, end, rk, strings.TrimSpace(head))
		if err != nil {
			return decision.Decision{}, err
		}
		d.Anchor = &a
	}
	now := c.now().UTC()
	var rnd [3]byte
	_, _ = rand.Read(rnd[:])
	d.ID = "d-" + now.Format("20060102T150405.000000") + "-" + hex.EncodeToString(rnd[:])
	d.At = now
	actor := "user"
	if d.By.Kind != "user" {
		actor = agentActor(d.By.Key)
	}
	if err := c.emit(ctx, actor, KindDecision, d); err != nil {
		return decision.Decision{}, err
	}
	return d, nil
}

func (c *capture) decisions(f decision.Filter) []DecisionView {
	list := c.dview.List(f)
	// Fallback roots per repo key, resolved once.
	fallback := map[string]string{}
	ctx := context.Background()
	for _, d := range list {
		if d.Anchor == nil || d.Anchor.Root != "" && dirExists(d.Anchor.Root) {
			continue
		}
		if _, ok := fallback[d.Anchor.RepoKey]; !ok {
			fallback[d.Anchor.RepoKey] = c.liveRootOf(ctx, d.Anchor.RepoKey)
		}
	}
	c.mu.RLock()
	defer c.mu.RUnlock()
	out := make([]DecisionView, 0, len(list))
	for _, d := range list {
		root := ""
		if d.Anchor != nil {
			root = fallback[d.Anchor.RepoKey]
		}
		out = append(out, c.checkDecision(d, root))
	}
	return out
}

// liveRootOf is the root of a live card in repoKey, else "".
func (c *capture) liveRootOf(ctx context.Context, repoKey string) string {
	if repoKey == "" {
		return ""
	}
	for _, card := range c.liveCards() {
		if card.Cwd == "" || (card.RepoKey != "" && card.RepoKey != repoKey) {
			continue
		}
		if root, rk := c.repoFor(ctx, card.Cwd); root != "" && rk == repoKey {
			return root
		}
	}
	return ""
}
