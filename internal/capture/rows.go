package capture

import (
	"context"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
	"github.com/StephenSHorton/irori/internal/gitsha"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/scrub"
)

// maxCommand caps CommitCmdRow.Command (runes).
const maxCommand = 300

// handle turns parser items into rows. It stops at the first failed append.
func (c *capture) handle(ctx context.Context, f *tfile, items []transcript.Item) error {
	drop := c.dropNoise(ctx, f, items)
	for i, it := range items {
		var err error
		switch {
		case it.Turn != nil:
			err = c.onTurn(ctx, f, it.Turn)
		case it.Edit != nil && drop[i]:
			c.notePath(f.key, it.Edit, true)
		case it.Edit != nil:
			if err = c.onEdit(ctx, f, it.Edit); err == nil {
				c.notePath(f.key, it.Edit, false)
			}
		case it.Summary != nil:
			err = c.onSummary(ctx, f, it.Summary)
		case it.Bash != nil:
			err = c.onBash(ctx, f, it.Bash)
		case it.PRLink != nil:
			err = c.onPRLink(ctx, f, it.PRLink)
		}
		if err != nil {
			return err
		}
	}
	return nil
}

// repoFor resolves a directory's worktree root and repo key, logging
// repo.seen the first time a root shows up.
func (c *capture) repoFor(ctx context.Context, dir string) (root, repoKey string) {
	root = c.roots.forDir(ctx, dir)
	if root == "" {
		return "", ""
	}
	return root, c.noteRoot(ctx, root)
}

// knownRoot is repoFor for reads: a directory's worktree root and repo key
// only when capture has already seen that root, else "", "". It logs nothing
// and starts no reflog watch.
func (c *capture) knownRoot(ctx context.Context, dir string) (root, repoKey string) {
	root = c.roots.forDir(ctx, dir)
	if root == "" {
		return "", ""
	}
	c.mu.RLock()
	r, ok := c.repoRoots[normRoot(root)]
	c.mu.RUnlock()
	if !ok {
		return "", ""
	}
	return root, r.RepoKey
}

// noteRoot returns root's repo key and logs repo.seen for a new root.
func (c *capture) noteRoot(ctx context.Context, root string) string {
	n := normRoot(root)
	c.mu.RLock()
	r, ok := c.repoRoots[n]
	c.mu.RUnlock()
	if ok {
		return r.RepoKey
	}
	rk := c.opts.RepoKey(root)
	if rk == "" {
		return ""
	}
	c.snapshotReflog(root)
	if err := c.emit(ctx, "irori", KindRepoSeen, repoSeen{RepoKey: rk, Root: root, At: c.now()}); err != nil {
		c.setErr(err)
	}
	return rk
}

// snapshotReflog records the size of a new root's logs/HEAD, so the first
// reflog watch starts from the moment the root was seen, not from the next
// repo pass (a commit made in between would be skipped otherwise).
func (c *capture) snapshotReflog(root string) {
	gd, err := gitlink.GitDir(root)
	if err != nil {
		return
	}
	var size int64
	if fi, err := os.Stat(filepath.Join(gd, "logs", "HEAD")); err == nil {
		size = fi.Size()
	} else if !errors.Is(err, fs.ErrNotExist) {
		return
	}
	n := normRoot(root)
	c.mu.Lock()
	if _, ok := c.firstSize[n]; !ok {
		c.firstSize[n] = size
	}
	c.mu.Unlock()
}

// scrubTurn scrubs a turn's text and makes its edit paths repo-relative.
func (c *capture) scrubTurn(t transcript.Turn, root string) transcript.Turn {
	t.Prompt = c.scrub.String(t.Prompt)
	t.Reply = c.scrub.String(t.Reply)
	t.Tools = append([]transcript.ToolCall(nil), t.Tools...)
	for i := range t.Tools {
		t.Tools[i].Target = c.scrub.String(t.Tools[i].Target)
	}
	paths := make([]string, 0, len(t.EditPaths))
	for _, p := range t.EditPaths {
		if rel, ok := relTo(root, p); ok {
			p = rel
		}
		paths = append(paths, p)
	}
	if len(paths) == 0 {
		paths = nil
	}
	t.EditPaths = paths
	return t
}

func (c *capture) onTurn(ctx context.Context, f *tfile, t *transcript.Turn) error {
	id := turnRowID(f.key, t.AgentID, t.ID)
	tk := turnKey(f.key, t.AgentID, t.ID)
	if c.has(id) {
		c.mu.Lock()
		delete(c.noisePaths, tk)
		c.mu.Unlock()
		return nil
	}
	root, rk := c.repoFor(ctx, t.Cwd)
	clean := *t
	c.mu.Lock()
	clean.EditPaths = c.stripNoise(f.key, clean)
	c.mu.Unlock()
	row := TurnRow{ID: id, Key: f.key, RepoKey: rk, Root: root, TurnID: t.ID, Turn: c.scrubTurn(clean, root)}
	if err := c.emit(ctx, agentActor(f.key), KindTurn, row); err != nil {
		return err
	}
	c.mu.Lock()
	delete(c.noisePaths, tk)
	c.mu.Unlock()
	return nil
}

func (c *capture) onEdit(ctx context.Context, f *tfile, e *transcript.Edit) error {
	id := editRowID(f.key, e.ToolUseID, e.Path)
	if c.has(id) {
		return nil
	}
	abs := e.Path
	if !filepath.IsAbs(abs) && e.Cwd != "" {
		abs = filepath.Join(e.Cwd, abs)
	}
	abs = filepath.Clean(filepath.FromSlash(abs))
	row := EditRow{
		ID: id, Key: f.key, AgentID: e.AgentID, TurnID: e.TurnID, ToolUseID: e.ToolUseID,
		Tool: e.Tool, Path: abs, Create: e.Create, Adds: e.Adds, Dels: e.Dels,
		Fidelity: e.Fidelity, At: e.At,
	}
	if root := c.roots.forFile(ctx, abs); root != "" {
		if rel, ok := relTo(root, abs); ok {
			row.Root, row.Path = root, rel
			row.RepoKey = c.noteRoot(ctx, root)
		}
	}
	if e.Before != nil {
		row.BeforeBlob = gitsha.Blob(e.Before)
	}
	if e.After != nil {
		row.AfterBlob, row.AfterBlobLF = gitsha.Pair(e.After)
	}
	if e.Patch != "" && !scrub.SkipFile(abs) {
		pid, err := c.blobs.Put([]byte(c.scrub.String(e.Patch)))
		if err != nil {
			return err
		}
		row.Patch = pid
	}
	if err := c.emit(ctx, agentActor(f.key), KindEdit, row); err != nil {
		return err
	}
	if row.RepoKey != "" {
		c.mu.Lock()
		if e := c.editByID[id]; e != nil {
			c.newEdits = append(c.newEdits, e)
		}
		c.mu.Unlock()
	}
	return nil
}

func (c *capture) onSummary(ctx context.Context, f *tfile, s *transcript.Summary) error {
	text := c.scrub.String(s.Text)
	idPart := s.RowUUID
	if idPart == "" {
		idPart = text
	}
	id := summaryRowID(f.key, s.AgentID, s.Kind, idPart)
	if c.has(id) {
		return nil
	}
	row := SummaryRow{ID: id, Key: f.key, Summary: *s}
	row.Text, row.Full = text, ""
	if row.At.IsZero() {
		// Title rows carry no time: stamp them with the file's activity.
		row.At = f.parser.LastActivity()
		if row.At.IsZero() {
			row.At = c.now()
		}
	}
	if s.Full != "" {
		bid, err := c.blobs.Put([]byte(c.scrub.String(s.Full)))
		if err != nil {
			return err
		}
		row.FullBlob = bid
	}
	return c.emit(ctx, agentActor(f.key), KindSummary, row)
}

func (c *capture) onBash(ctx context.Context, f *tfile, b *transcript.BashRun) error {
	if gitlink.IsCommitCommand(b.Command) {
		if id := cmdID(f.key, b.ToolUseID); !c.has(id) {
			row := CommitCmdRow{
				Key: f.key, AgentID: b.AgentID, TurnID: b.TurnID, ToolUseID: b.ToolUseID,
				Cwd: b.Cwd, Command: clipRunes(c.scrub.String(firstLine(b.Command)), maxCommand), At: b.At,
			}
			if err := c.emit(ctx, agentActor(f.key), KindCommitCmd, row); err != nil {
				return err
			}
		}
	}
	if len(b.GitOperation) == 0 && !strings.Contains(b.Command, "gh") {
		return nil // no PR relation possible; skip resolving the cwd's repo
	}
	_, rk := c.repoFor(ctx, b.Cwd)
	refs := ledger.FromBash(ledger.BashResult{
		Command: b.Command, Stdout: b.Stdout, GitOperation: b.GitOperation,
		DefaultRepo: ledger.RepoFromKey(rk),
	})
	for _, r := range refs {
		e := ledger.Entry{
			Key: ledger.Key(r.Ref.Repo, r.Ref.Number), Repo: r.Ref.Repo, Number: r.Ref.Number,
			URL: r.URL, Session: f.key, Role: r.Role, Source: r.Source, At: b.At,
		}
		if e.At.IsZero() {
			e.At = c.now()
		}
		if c.has(prID(e)) {
			continue
		}
		if err := c.emit(ctx, agentActor(f.key), KindPR, e); err != nil {
			return err
		}
	}
	return nil
}

func (c *capture) onPRLink(ctx context.Context, f *tfile, l *transcript.PRLink) error {
	if l.Repo == "" || l.Number <= 0 {
		return nil
	}
	e := ledger.Entry{
		Key: ledger.Key(l.Repo, l.Number), Repo: l.Repo, Number: l.Number, URL: l.URL,
		Session: f.key, Role: ledger.Touched, Source: "pr-link", At: l.At,
	}
	if e.At.IsZero() {
		e.At = c.now()
	}
	if c.has(prID(e)) {
		return nil
	}
	return c.emit(ctx, "irori", KindPR, e)
}

// firstLine is the first non-blank line of s, trimmed.
func firstLine(s string) string {
	s = strings.TrimSpace(s)
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		s = s[:i]
	}
	return strings.TrimSpace(s)
}

// clipRunes keeps at most n runes of s, ending in "…" when it cut.
func clipRunes(s string, n int) string {
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	r := []rune(s)
	return string(r[:n-1]) + "…"
}
