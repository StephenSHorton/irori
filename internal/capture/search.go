package capture

// Search and show (docs/design/board.md "Contracts").
//
// CONTRACT (implemented by the capture work package, A3; keep these
// signatures). Types are internal/search's, aliased here.
//
// Index: SQLite FTS5 through modernc.org/sqlite, which compiles fts5 in
// (checked by TestFTS5Available, v1.61.0). One in-memory database
// ("file:irori-search?mode=memory&cache=shared", or a single pooled
// connection: every ":memory:" connection is a separate database), rebuilt
// at New from the fold and never written to disk, so there is nothing to
// keep in step with the log. Table:
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

	"github.com/StephenSHorton/irori/internal/search"
)

type (
	SearchQuery = search.Query
	Hit         = search.Hit
	ShowOptions = search.ShowOptions
	Shown       = search.Shown
)

// AddSource registers a Source for its kinds (Core adds coord and inbox
// after New). A kind capture owns, or one already registered, panics: that
// is a wiring bug.
func (c *Capture) AddSource(src search.Source) {}

// Search returns hits for q, best first, at most q.Limit (default
// search.DefaultLimit, max search.MaxLimit). Callers that want to say "more"
// ask for Limit+1. Text that FTS5 can't parse after quoting is an error.
func (c *Capture) Search(ctx context.Context, q SearchQuery) ([]Hit, error) {
	return nil, ErrUnimplemented
}

// Show renders one ref (search.ErrBadRef for a malformed one,
// search.ErrNotFound when nothing has it), paged by o.
func (c *Capture) Show(ctx context.Context, ref string, o ShowOptions) (Shown, error) {
	return Shown{}, ErrUnimplemented
}
