package capture

// Search and show (docs/design/board.md "Contracts").
//
// CONTRACT (implemented by the capture work package, A3; keep these
// signatures). Types are internal/search's, aliased here.
//
// Index: SQLite FTS5 through modernc.org/sqlite, which compiles fts5 in
// (checked by TestFTS5Available, v1.61.0). One in-memory database on a
// single pooled connection (every ":memory:" connection is a separate
// database), rebuilt at New from the fold and never written to disk, so
// there is nothing to keep in step with the log. Table:
//
//	CREATE VIRTUAL TABLE docs USING fts5(
//	    ref UNINDEXED, kind UNINDEXED, at UNINDEXED, agent UNINDEXED, repo UNINDEXED,
//	    title, body, tokenize = 'porter unicode61');
//
// upserted by ref (DELETE then INSERT). Query.Text becomes FTS5 syntax by
// quoting each word ("word") and joining them with AND; the last word gets
// a prefix star. Ranking is bm25 (title weighted 4x body), then newest At.
// Kinds, RepoKey, Agent and Before filter in SQL. Snippet is
// snippet(docs, -1, '[', ']', '…', 24) cut to one line and MaxSnippet.
// Empty Text lists newest first.
//
// What capture indexes itself, in apply (so a replay rebuilds it):
//   - turn:<TurnRow.ID>: Title = the prompt's first line, Body = prompt +
//     reply (already scrubbed), Agent = Key; subagent turns included.
//   - edit:<EditRow.ID>: Title = "<tool> <path> +adds -dels", Body = path,
//     no patch text.
//   - commit:<sha>: Title = subject, Body = subject + file paths, Agent =
//     the committer or first linked session.
//   - pr:<ledger key>: Title = "PR owner/name#n (role)", Agent = Session.
//   - decision:<id>: Title = statement, Body = statement + why + anchor path.
//
// Sources (coord: task, comment; inbox: msg) are pulled, not pushed: before
// each Search, capture calls SearchDocs(since = the newest Updated it has
// from that source) and pages until a short page. Show dispatches on the
// ref's kind to capture itself or the source owning it.
//
// Show renderings capture owns (all paged by search.Page):
//   - turn: prompt, reply, tool targets, its edits (as edit refs), the
//     commits linked to it; Related: edit and commit refs, agent:<key>.
//   - edit: tool, path, +/-, fidelity, and the patch from the blob store;
//     Related: its turn, the commit that took it.
//   - commit: subject, author, branch, files, link fidelity; Related: turns.
//   - pr: ledger entries (who, role, source); Related: agent refs.
//   - decision: statement, why, anchor and its re-checked status.
//   - agent: the newest Timeline page as one line per entry with refs.

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/search"
)

type (
	SearchQuery = search.Query
	Hit         = search.Hit
	ShowOptions = search.ShowOptions
	Shown       = search.Shown
)

// ownKinds are the ref kinds capture answers itself.
var ownKinds = []search.Kind{
	search.KindTurn, search.KindEdit, search.KindCommit, search.KindPR,
	search.KindDecision, search.KindAgent,
}

// pullPage is how many docs one SearchDocs call asks a source for.
const pullPage = 500

// sourceState is one registered Source and how far the index has read it.
type sourceState struct {
	src   search.Source
	kinds []search.Kind
	since time.Time // newest Updated seen from it
}

// sources are the registered outside Sources. mu also serializes pulls.
type sources struct {
	mu     sync.Mutex
	list   []*sourceState
	byKind map[search.Kind]*sourceState
}

// AddSource registers a Source for its kinds (Core adds coord and inbox
// after New). A kind capture owns, or one already registered, panics: that
// is a wiring bug.
func (c *Capture) AddSource(src search.Source) {
	s := &c.impl.srcs
	s.mu.Lock()
	defer s.mu.Unlock()
	kinds := slices.Clone(src.SearchKinds())
	for _, k := range kinds {
		if slices.Contains(ownKinds, k) {
			panic(fmt.Sprintf("capture: AddSource: kind %q is capture's own", k))
		}
		if s.byKind[k] != nil {
			panic(fmt.Sprintf("capture: AddSource: kind %q already has a source", k))
		}
	}
	st := &sourceState{src: src, kinds: kinds}
	if s.byKind == nil {
		s.byKind = map[search.Kind]*sourceState{}
	}
	for _, k := range kinds {
		s.byKind[k] = st
	}
	s.list = append(s.list, st)
}

// Search returns hits for q, best first, at most q.Limit (default
// search.DefaultLimit, max search.MaxLimit; one more, MaxLimit+1, is let
// through so a caller showing MaxLimit can still tell there is more).
// Callers that want to say "more" ask for Limit+1. Text that FTS5 can't
// parse after quoting is an error.
func (c *Capture) Search(ctx context.Context, q SearchQuery) ([]Hit, error) {
	if q.Limit <= 0 {
		q.Limit = search.DefaultLimit
	}
	q.Limit = min(q.Limit, search.MaxLimit+1)
	c.impl.pull(ctx)
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return c.impl.idx.search(q)
}

// Show renders one ref (search.ErrBadRef for a malformed one,
// search.ErrNotFound when nothing has it), paged by o.
func (c *Capture) Show(ctx context.Context, ref string, o ShowOptions) (Shown, error) {
	kind, id, err := search.ParseRef(ref)
	if err != nil {
		return Shown{}, err
	}
	if slices.Contains(ownKinds, kind) {
		r, err := c.impl.render(ctx, kind, id)
		if err != nil {
			return Shown{}, err
		}
		return r.page(o), nil
	}
	s := &c.impl.srcs
	s.mu.Lock()
	st := s.byKind[kind]
	s.mu.Unlock()
	if st == nil {
		return Shown{}, search.ErrNotFound
	}
	return st.src.SearchShow(ctx, search.Ref(kind, id), o)
}

// pull reads every source's new docs into the index. A source that fails
// is skipped this time (its error goes to Status) and retried next search.
func (c *capture) pull(ctx context.Context) {
	s := &c.srcs
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, st := range s.list {
		for {
			docs, err := st.src.SearchDocs(ctx, st.since, pullPage)
			if err != nil {
				if ctx.Err() == nil {
					c.setErr(fmt.Errorf("capture: search source %v: %w", st.kinds, err))
				}
				break
			}
			newest := st.since
			for _, d := range docs {
				kind, id, err := search.ParseRef(d.Ref)
				if err != nil || !slices.Contains(st.kinds, kind) {
					continue // a source only fills its own kinds
				}
				d.Ref, d.Kind = search.Ref(kind, id), kind
				if err := c.idx.put(d); err != nil {
					c.setErr(fmt.Errorf("capture: search index: %w", err))
					continue
				}
				upd := d.Updated
				if upd.IsZero() {
					upd = d.At
				}
				if upd.After(newest) {
					newest = upd
				}
			}
			progressed := newest.After(st.since)
			st.since = newest
			// since is inclusive: a full page that didn't move it forward
			// would come back the same, so stop there.
			if len(docs) < pullPage || !progressed {
				break
			}
		}
	}
}

// index upserts one of capture's own docs; c.mu is held. An index failure
// never fails the fold: it is recorded for Status.
func (c *capture) index(d search.Doc) {
	if c.idx == nil {
		return
	}
	if err := c.idx.put(d); err != nil {
		c.lastErr = "capture: search index: " + err.Error()
	}
}

func turnDoc(r *TurnRow) search.Doc {
	body := r.Prompt
	if r.Reply != "" {
		body += "\n" + r.Reply
	}
	return search.Doc{
		Ref: search.Ref(search.KindTurn, r.ID), Kind: search.KindTurn, At: r.StartedAt,
		Title: firstLine(r.Prompt), Body: body, Agent: r.Key, RepoKey: r.RepoKey,
	}
}

func editDoc(r *EditRow) search.Doc {
	return search.Doc{
		Ref: search.Ref(search.KindEdit, r.ID), Kind: search.KindEdit, At: r.At,
		Title: editTitle(r), Body: r.Path, Agent: r.Key, RepoKey: r.RepoKey,
	}
}

func editTitle(r *EditRow) string {
	return fmt.Sprintf("%s %s +%d -%d", r.Tool, r.Path, r.Adds, r.Dels)
}

func commitDoc(rec *commitRec) search.Doc {
	var b strings.Builder
	b.WriteString(rec.row.Subject)
	for _, f := range rec.row.Files {
		b.WriteString("\n")
		b.WriteString(f.Path)
	}
	return search.Doc{
		Ref: search.Ref(search.KindCommit, strings.ToLower(rec.row.SHA)), Kind: search.KindCommit,
		At: rec.row.At, Title: rec.row.Subject, Body: b.String(), Agent: commitAgent(rec.link),
		RepoKey: rec.row.RepoKey,
	}
}

// commitAgent is who a commit belongs to: its committer, else the first
// session behind its linked edits.
func commitAgent(l *CommitLink) string {
	switch {
	case l == nil:
		return ""
	case l.Committer != nil && l.Committer.Key != "":
		return l.Committer.Key
	case len(l.Sessions) > 0:
		return l.Sessions[0]
	}
	return ""
}

// roleRank orders ledger roles as the ledger does: author, touched, reviewer.
func roleRank(r ledger.Role) int {
	switch r {
	case ledger.Author:
		return 0
	case ledger.Touched:
		return 1
	}
	return 2
}

// sortOwners orders a PR's entries best first (as ledger.Owners does).
func sortOwners(es []ledger.Entry) {
	slices.SortStableFunc(es, func(a, b ledger.Entry) int {
		if ra, rb := roleRank(a.Role), roleRank(b.Role); ra != rb {
			return ra - rb
		}
		if !a.At.Equal(b.At) {
			return a.At.Compare(b.At)
		}
		return strings.Compare(a.Session, b.Session)
	})
}

// mergeOwner folds e into a PR's entries as the ledger does: one entry per
// (Session, Role), keeping the earliest At and the first non-empty URL.
func mergeOwner(es []ledger.Entry, e ledger.Entry) []ledger.Entry {
	for i := range es {
		if es[i].Session == e.Session && es[i].Role == e.Role {
			if e.At.Before(es[i].At) {
				es[i].At = e.At
			}
			if es[i].URL == "" {
				es[i].URL = e.URL
			}
			return es
		}
	}
	return append(es, e)
}

// prDoc builds a PR's doc from all its entries, best owner first. At is the
// newest entry, so a PR someone just touched lists as recent.
func prDoc(es []ledger.Entry) search.Doc {
	best := es[0]
	var at time.Time
	var b strings.Builder
	fmt.Fprintf(&b, "PR %s#%d %s", best.Repo, best.Number, best.URL)
	for _, e := range es {
		if e.At.After(at) {
			at = e.At
		}
		fmt.Fprintf(&b, "\n%s %s", e.Role, e.Source)
	}
	return search.Doc{
		Ref: best.Key, Kind: search.KindPR, At: at,
		Title: fmt.Sprintf("PR %s#%d (%s)", best.Repo, best.Number, best.Role),
		Body:  b.String(), Agent: best.Session, RepoKey: ledger.RepoKey(best.Repo),
	}
}

func decisionDoc(d decision.Decision) search.Doc {
	body := d.Statement
	if d.Why != "" {
		body += "\n" + d.Why
	}
	rk := ""
	if d.Anchor != nil {
		rk = d.Anchor.RepoKey
		body += "\n" + d.Anchor.Path
	}
	return search.Doc{
		Ref: search.Ref(search.KindDecision, d.ID), Kind: search.KindDecision, At: d.At,
		Title: d.Statement, Body: body, Agent: d.By.Key, RepoKey: rk,
	}
}
