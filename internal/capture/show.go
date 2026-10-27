package capture

import (
	"context"
	"fmt"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/search"
)

// rendering is one ref drawn in full, before paging.
type rendering struct {
	ref     string
	kind    search.Kind
	title   string
	text    strings.Builder
	related []string
}

func (r *rendering) line(format string, args ...any) {
	fmt.Fprintf(&r.text, format, args...)
	r.text.WriteByte('\n')
}

// block writes a labelled multi-line section, skipping it when empty.
func (r *rendering) block(label, body string) {
	body = strings.TrimSpace(body)
	if body == "" {
		return
	}
	r.text.WriteString("\n" + label + ":\n" + body + "\n")
}

// relate adds a ref once, up to MaxRelated.
func (r *rendering) relate(refs ...string) {
	for _, ref := range refs {
		if ref == "" || ref == r.ref || slices.Contains(r.related, ref) || len(r.related) >= search.MaxRelated {
			continue
		}
		r.related = append(r.related, ref)
	}
}

func (r *rendering) page(o ShowOptions) Shown {
	text, more, next := search.Page(r.text.String(), o)
	return Shown{
		Ref: r.ref, Kind: r.kind, Title: clipRunes(oneLine(r.title), search.MaxTitle),
		Text: text, More: more, Next: next, Related: r.related,
	}
}

// render draws one of capture's own refs for viewer ("" is the person).
func (c *capture) render(ctx context.Context, kind search.Kind, id, viewer string) (*rendering, error) {
	switch kind {
	case search.KindTurn:
		return c.renderTurn(id, viewer)
	case search.KindEdit:
		return c.renderEdit(id)
	case search.KindCommit:
		return c.renderCommit(id)
	case search.KindPR:
		return c.renderPR(id)
	case search.KindDecision:
		return c.renderDecision(ctx, id)
	case search.KindAgent:
		return c.renderAgent(id, viewer)
	}
	return nil, search.ErrNotFound
}

func stamp(t time.Time) string {
	if t.IsZero() {
		return "-"
	}
	return t.UTC().Format(time.RFC3339)
}

func turnRef(id string) string    { return search.Ref(search.KindTurn, id) }
func editRef(id string) string    { return search.Ref(search.KindEdit, id) }
func commitRef(sha string) string { return search.Ref(search.KindCommit, strings.ToLower(sha)) }
func agentRef(key string) string  { return search.Ref(search.KindAgent, key) }

// shownTurn is a turn by its row id: a closed one, else a live session's
// open turn; c.mu is read-locked.
func (c *capture) shownTurn(id string) (TurnRow, bool) {
	if t := c.turnByID[id]; t != nil {
		return cloneTurn(t), true
	}
	for k, t := range c.open {
		key, _, _ := strings.Cut(k, "|")
		if turnRowID(key, t.AgentID, t.ID) == id {
			root, rk := c.cachedRepo(t.Cwd)
			return TurnRow{ID: id, Key: key, RepoKey: rk, Root: root, TurnID: t.ID, Turn: c.scrubTurn(*t, root)}, true
		}
	}
	return TurnRow{}, false
}

// turnOf finds the turn row of (key, transcript turn id), any agent;
// c.mu is read-locked.
func (c *capture) turnOf(key, agentID, turnID string) *TurnRow {
	if t := c.turnByID[turnRowID(key, agentID, turnID)]; t != nil {
		return t
	}
	for _, t := range c.turns[key] {
		if t.Turn.ID == turnID {
			return t
		}
	}
	return nil
}

// commitsTaking lists the commits whose newest link names any of the edit
// ids, or whose committer is (key, turnID) when key is set, newest first;
// c.mu is read-locked.
func (c *capture) commitsTaking(edits []string, key, turnID string) []*commitRec {
	var out []*commitRec
	for _, rec := range c.commitMap {
		l := rec.link
		if l == nil {
			continue
		}
		hit := key != "" && l.Committer != nil && l.Committer.Key == key && l.Committer.TurnID == turnID
		for _, f := range l.Files {
			for _, id := range f.Edits {
				hit = hit || slices.Contains(edits, id)
			}
		}
		if hit {
			out = append(out, rec)
		}
	}
	sort.SliceStable(out, func(i, j int) bool {
		if !out[i].row.At.Equal(out[j].row.At) {
			return out[i].row.At.After(out[j].row.At)
		}
		return out[i].row.SHA < out[j].row.SHA
	})
	return out
}

func (c *capture) renderTurn(id, viewer string) (*rendering, error) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	t, ok := c.shownTurn(id)
	if !ok {
		return nil, search.ErrNotFound
	}
	t.Prompt = hideMessages(t.Prompt, viewer, t.Key)
	r := &rendering{ref: turnRef(id), kind: search.KindTurn, title: firstLine(t.Prompt)}
	who := t.Key
	if t.AgentID != "" {
		who += " (subagent " + t.AgentID + ")"
	}
	r.line("turn %s by %s", r.ref, who)
	if t.RepoKey != "" || t.Branch != "" {
		r.line("repo: %s  branch: %s", t.RepoKey, t.Branch)
	}
	state := "ended " + stamp(t.EndedAt)
	switch {
	case t.EndedAt.IsZero():
		state = "open"
	case t.Interrupted:
		state += " (interrupted)"
	}
	r.line("started %s, %s; origin %s", stamp(t.StartedAt), state, t.Origin)
	r.block("prompt", t.Prompt)
	r.block("reply", t.Reply)
	if len(t.Tools) > 0 {
		var b strings.Builder
		for _, tc := range t.Tools {
			b.WriteString("- " + tc.Name)
			if tc.Target != "" {
				b.WriteString(" " + oneLine(tc.Target))
			}
			if tc.IsError {
				b.WriteString(" (error)")
			}
			b.WriteByte('\n')
		}
		if t.ToolsMore > 0 {
			fmt.Fprintf(&b, "... %d more tool calls\n", t.ToolsMore)
		}
		r.block("tools", b.String())
	}
	edits := c.editsByTurn[turnKey(t.Key, t.AgentID, t.Turn.ID)]
	var ids []string
	if len(edits) > 0 {
		var b strings.Builder
		for _, e := range edits {
			ids = append(ids, e.ID)
			fmt.Fprintf(&b, "- %s %s\n", editRef(e.ID), editTitle(e))
		}
		r.block("edits", b.String())
	}
	committer := ""
	if t.AgentID == "" {
		committer = t.Key
	}
	commits := c.commitsTaking(ids, committer, t.Turn.ID)
	if len(commits) > 0 {
		var b strings.Builder
		for _, rec := range commits {
			fmt.Fprintf(&b, "- %s %s\n", commitRef(rec.row.SHA), oneLine(rec.row.Subject))
		}
		r.block("commits", b.String())
	}
	for _, e := range edits {
		r.relate(editRef(e.ID))
	}
	for _, rec := range commits {
		r.relate(commitRef(rec.row.SHA))
	}
	r.relate(agentRef(t.Key))
	return r, nil
}

func (c *capture) renderEdit(id string) (*rendering, error) {
	c.mu.RLock()
	e := c.editByID[id]
	if e == nil {
		c.mu.RUnlock()
		return nil, search.ErrNotFound
	}
	row := cloneEdit(e)
	tref := ""
	if t := c.turnOf(row.Key, row.AgentID, row.TurnID); t != nil {
		tref = turnRef(t.ID)
	}
	var crefs []string
	for _, rec := range c.commitsTaking([]string{id}, "", "") {
		crefs = append(crefs, commitRef(rec.row.SHA))
	}
	c.mu.RUnlock()

	r := &rendering{ref: editRef(id), kind: search.KindEdit, title: editTitle(&row)}
	r.line("edit %s by %s", r.ref, row.Key)
	r.line("%s", editTitle(&row))
	if row.RepoKey != "" {
		r.line("repo: %s", row.RepoKey)
	}
	created := ""
	if row.Create {
		created = ", created the file"
	}
	r.line("at %s, fidelity %s%s", stamp(row.At), row.Fidelity, created)
	if tref != "" {
		r.line("turn: %s", tref)
	}
	for _, cr := range crefs {
		r.line("committed in: %s", cr)
	}
	switch patch, err := c.patchText(row.Patch); {
	case row.Patch == "":
		r.block("patch", "(no patch stored)")
	case err != nil:
		r.block("patch", "(patch unreadable: "+err.Error()+")")
	default:
		r.block("patch", patch)
	}
	r.relate(tref)
	r.relate(crefs...)
	return r, nil
}

func (c *capture) patchText(id string) (string, error) {
	if id == "" {
		return "", nil
	}
	b, err := c.blobs.Get(id)
	return string(b), err
}

func (c *capture) renderCommit(sha string) (*rendering, error) {
	v, ok := c.commit(sha)
	if !ok {
		return nil, search.ErrNotFound
	}
	r := &rendering{ref: commitRef(v.SHA), kind: search.KindCommit, title: v.Subject}
	r.line("commit %s in %s", v.SHA, v.RepoKey)
	r.line("subject: %s", oneLine(v.Subject))
	r.line("author: %s, at %s", v.Author, stamp(v.At))
	if v.Branch != "" || v.Op != "" {
		r.line("branch: %s  op: %s", v.Branch, v.Op)
	}
	links := map[string]string{}
	if v.Link != nil {
		for _, f := range v.Link.Files {
			links[f.Path] = f.Link
		}
		if cm := v.Link.Committer; cm != nil {
			r.line("committed by: %s", cm.Key)
		}
	}
	if len(v.Files) > 0 {
		var b strings.Builder
		for _, f := range v.Files {
			fmt.Fprintf(&b, "- %s %s", f.Status, f.Path)
			if l := links[f.Path]; l != "" {
				b.WriteString(" (" + l + ")")
			}
			b.WriteByte('\n')
		}
		if v.FilesOmitted > 0 {
			fmt.Fprintf(&b, "... %d more files\n", v.FilesOmitted)
		}
		r.block("files", b.String())
	}
	if v.Link == nil {
		r.line("\nnot linked to any agent's edits yet")
		return r, nil
	}
	c.mu.RLock()
	defer c.mu.RUnlock()
	if cm := v.Link.Committer; cm != nil {
		if t := c.turnOf(cm.Key, "", cm.TurnID); t != nil {
			r.relate(turnRef(t.ID))
		}
	}
	for _, f := range v.Link.Files {
		for _, id := range f.Edits {
			if e := c.editByID[id]; e != nil {
				if t := c.turnOf(e.Key, e.AgentID, e.TurnID); t != nil {
					r.relate(turnRef(t.ID))
				}
			}
		}
	}
	if len(r.related) > 0 {
		r.block("turns", "- "+strings.Join(r.related, "\n- "))
	}
	return r, nil
}

// prKey is the ledger key of a pr ref's id ("owner/name#n").
func prKey(id string) string {
	repo, n, _ := strings.Cut(id, "#")
	num, _ := strconv.Atoi(n)
	return ledger.Key(repo, num)
}

func (c *capture) renderPR(id string) (*rendering, error) {
	key := prKey(id)
	c.mu.RLock()
	es := slices.Clone(c.prAll[key])
	c.mu.RUnlock()
	if len(es) == 0 {
		return nil, search.ErrNotFound
	}
	best := es[0]
	r := &rendering{ref: key, kind: search.KindPR, title: fmt.Sprintf("PR %s#%d", best.Repo, best.Number)}
	r.line("PR %s#%d", best.Repo, best.Number)
	for _, e := range es {
		if e.URL != "" {
			r.line("url: %s", e.URL)
			break
		}
	}
	var b strings.Builder
	for _, e := range es {
		fmt.Fprintf(&b, "- %s %s via %s at %s\n", agentRef(e.Session), e.Role, e.Source, stamp(e.At))
		r.relate(agentRef(e.Session))
	}
	r.block("who", b.String())
	return r, nil
}

func (c *capture) renderDecision(ctx context.Context, id string) (*rendering, error) {
	d, ok := c.dview.Get(id)
	if !ok {
		return nil, search.ErrNotFound
	}
	listed := decision.Listed{Decision: d}
	for _, l := range c.dview.List(decision.Filter{All: true, Limit: 500}) {
		if l.ID == id {
			listed = l
			break
		}
	}
	root := ""
	if d.Anchor != nil && (d.Anchor.Root == "" || !dirExists(d.Anchor.Root)) {
		root = c.liveRootOf(ctx, d.Anchor.RepoKey)
	}
	v := c.checkDecision(listed, root)
	r := &rendering{ref: search.Ref(search.KindDecision, id), kind: search.KindDecision, title: d.Statement}
	by := d.By.Name
	if by == "" {
		by = d.By.Key
	}
	r.line("decision %s by %s (%s), at %s", r.ref, by, d.Trust, stamp(d.At))
	r.block("statement", d.Statement)
	r.block("why", d.Why)
	if a := d.Anchor; a != nil {
		lines := ""
		if a.Start > 0 {
			lines = fmt.Sprintf(":%d-%d", a.Start, a.End)
		}
		r.text.WriteString("\n")
		r.line("anchor: %s %s%s (recorded at %s)", a.RepoKey, a.Path, lines, a.SHA)
		if v.Status != nil {
			now := string(v.Status.State)
			if v.Status.Start > 0 && (v.Status.Start != a.Start || v.Status.End != a.End) {
				now += fmt.Sprintf(", now at %d-%d", v.Status.Start, v.Status.End)
			}
			r.line("anchor status: %s", now)
		}
	}
	for _, s := range d.Supersedes {
		r.line("supersedes: %s", search.Ref(search.KindDecision, s))
		r.relate(search.Ref(search.KindDecision, s))
	}
	for _, s := range listed.SupersededBy {
		r.line("superseded by: %s", search.Ref(search.KindDecision, s))
		r.relate(search.Ref(search.KindDecision, s))
	}
	if src := d.Source; src != nil && src.MessageID != "" {
		r.relate(search.Ref(search.KindMsg, src.MessageID))
	}
	return r, nil
}

func (c *capture) renderAgent(key, viewer string) (*rendering, error) {
	cards := c.liveCards()
	var card *AgentLine
	for _, cd := range cards {
		if cd.Key == key {
			card = &AgentLine{Key: cd.Key, Name: cardName(cd.Name, cd.Title, cd.Key), Status: cd.Status, RepoKey: cd.RepoKey}
			break
		}
	}
	tl := c.timeline(TimelineQuery{Key: key, Limit: 20})
	if card == nil && len(tl.Entries) == 0 {
		return nil, search.ErrNotFound
	}
	r := &rendering{ref: agentRef(key), kind: search.KindAgent, title: key}
	if card != nil {
		r.title = card.Name
		r.line("agent %s (%s), %s, repo %s", card.Name, key, card.Status, card.RepoKey)
	} else {
		r.line("agent %s (not live)", key)
	}
	r.text.WriteString("\n")
	for _, e := range tl.Entries {
		switch {
		case e.Turn != nil:
			ref := turnRef(e.Turn.ID)
			open := ""
			if e.Turn.Open {
				open = " (open)"
			}
			r.line("%s %s%s %s", stamp(e.At), ref, open, clipRunes(firstLine(hideMessages(e.Turn.Prompt, viewer, key)), OverviewLineMax))
			r.relate(ref)
		case e.Commit != nil:
			ref := commitRef(e.Commit.SHA)
			r.line("%s %s %s", stamp(e.At), ref, clipRunes(oneLine(e.Commit.Subject), OverviewLineMax))
			r.relate(ref)
		case e.PR != nil:
			r.line("%s %s %s", stamp(e.At), e.PR.Key, e.PR.Role)
			r.relate(e.PR.Key)
		case e.Summary != nil:
			r.line("%s summary %s", stamp(e.At), clipRunes(oneLine(e.Summary.Text), OverviewLineMax))
		}
	}
	if tl.More {
		r.line("\nolder: search with agent %s", key)
	}
	return r, nil
}

// cardName is an agent's display name: its session name, else its title,
// else its key.
func cardName(name, title, key string) string {
	switch {
	case name != "":
		return name
	case title != "":
		return title
	}
	return key
}
