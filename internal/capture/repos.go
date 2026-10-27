package capture

import (
	"bytes"
	"context"
	"errors"
	"io"
	"log"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
)

// maxReflogRead caps one pass's read of a logs/HEAD.
const maxReflogRead = 4 << 20

// repoBudget bounds one repo pass. A first sight after backfill can walk
// hundreds of commits per root at two git runs each; transcripts are tailed
// on the same goroutine, so a pass stops at the budget and the next one
// carries on from the same reflog line (commits already logged are skipped).
var repoBudget = 1500 * time.Millisecond

// errRepoBudget stops a pass that used up repoBudget; it is not a failure.
var errRepoBudget = errors.New("capture: repo pass budget used")

// maxReflogTries is how many repo passes try a reflog line whose import
// failed (a git timeout, a failed append) before it is skipped.
const maxReflogTries = 3

// reflogState is one watched worktree root.
type reflogState struct {
	root    string
	repoKey string
	logPath string
	offset  int64
	fails   int // failed imports of the line at offset
}

// repoPass checks every known root's reflog and imports new commits.
func (c *capture) repoPass(ctx context.Context) {
	c.loadCursors()
	c.repoDeadline = time.Now().Add(repoBudget)
	c.noteLiveRoots(ctx)
	c.mu.RLock()
	roots := make([]repoSeen, 0, len(c.repoRoots))
	for _, r := range c.repoRoots {
		roots = append(roots, r)
	}
	c.mu.RUnlock()
	sort.Slice(roots, func(i, j int) bool { return roots[i].Root < roots[j].Root })
	for _, r := range roots {
		if ctx.Err() != nil {
			return
		}
		c.watchRoot(ctx, r)
		if time.Now().After(c.repoDeadline) {
			break
		}
	}
	c.saveCursors()
	c.linkUnlinked(ctx)
}

// noteLiveRoots notes the worktree roots live agents work in.
func (c *capture) noteLiveRoots(ctx context.Context) {
	for _, card := range c.liveCards() {
		if card.Cwd != "" {
			root, _ := c.repoFor(ctx, card.Cwd)
			c.useRoot(root, card.SessionID)
		}
	}
}

// watchRoot reads a root's new reflog lines.
func (c *capture) watchRoot(ctx context.Context, r repoSeen) {
	n := normRoot(r.Root)
	c.mu.RLock()
	st := c.reflogs[n]
	c.mu.RUnlock()
	if st == nil {
		gd, err := gitlink.GitDir(r.Root)
		if err != nil {
			return // a removed worktree; it stays listed in Repos
		}
		st = &reflogState{root: r.Root, repoKey: r.RepoKey, logPath: filepath.Join(gd, "logs", "HEAD")}
		if off, ok := c.cursors.Reflogs[n]; ok {
			st.offset = off
		} else {
			// First sight: only movements since the root was first seen,
			// never the history.
			c.mu.RLock()
			first, ok := c.firstSize[n]
			c.mu.RUnlock()
			if fi, err := os.Stat(st.logPath); err == nil {
				st.offset = fi.Size()
				if ok && first <= st.offset {
					st.offset = first
				}
				// The root is usually first seen through an edit that is
				// tailed seconds after it was made, and backfill sees roots
				// hours late, so commits made since that root's oldest captured
				// edit may already be in the log. Start at the first movement
				// after it instead.
				if since, ok := c.oldestEditAt(n); ok {
					// A resumed session can hold edits weeks old; never
					// reach back past the backfill window.
					if floor := c.now().Add(-c.opts.BackfillWindow); since.Before(floor) {
						since = floor
					}
					if off, ok := reflogOffsetSince(st.logPath, fi.Size(), since.Truncate(time.Second)); ok && off < st.offset {
						st.offset = off
					}
				}
			}
			c.cursors.setReflog(n, st.offset)
		}
		c.mu.Lock()
		c.reflogs[n] = st
		c.mu.Unlock()
	}
	fi, err := os.Stat(st.logPath)
	if err != nil {
		return
	}
	size := fi.Size()
	if size < st.offset {
		// Rewritten (reflog expire): carry on from its end.
		st.offset = size
		c.cursors.setReflog(n, size)
		return
	}
	if size == st.offset {
		return
	}
	fh, err := os.Open(st.logPath)
	if err != nil {
		return
	}
	data, err := io.ReadAll(io.NewSectionReader(fh, st.offset, min(size-st.offset, maxReflogRead)))
	fh.Close()
	if err != nil {
		c.setErr(err)
		return
	}
	branch := ""
	for len(data) > 0 {
		i := bytes.IndexByte(data, '\n')
		if i < 0 {
			break
		}
		lineBytes := data[:i+1]
		data = data[i+1:]
		entries, _ := gitlink.ParseReflog(lineBytes)
		var lineErr error
		for _, e := range entries {
			if branch == "" {
				branch = c.branch(ctx, st.root)
			}
			if err := c.importEntry(ctx, st, e, branch); err != nil {
				if ctx.Err() != nil || errors.Is(err, errRepoBudget) {
					return
				}
				lineErr = err
			}
		}
		if lineErr != nil {
			c.setErr(lineErr)
			st.fails++
			if st.fails < maxReflogTries {
				// Keep the offset: the next pass tries the line again, and
				// the commits it already logged are skipped as seen.
				log.Printf("capture: reflog %s: %v (will retry)", filepath.Base(st.root), lineErr)
				return
			}
			log.Printf("capture: reflog %s: %v (skipped after %d tries)", filepath.Base(st.root), lineErr, st.fails)
		}
		st.fails = 0
		st.offset += int64(len(lineBytes))
		c.cursors.setReflog(n, st.offset)
		// Lines that log nothing (a checkout, an all-seen range) still cost
		// a rev-list each, so the budget is checked per line too.
		if time.Now().After(c.repoDeadline) {
			return
		}
	}
}

// branch is the root's current branch, "" when detached.
func (c *capture) branch(ctx context.Context, root string) string {
	cctx, cancel := context.WithTimeout(ctx, gitTimeout)
	defer cancel()
	out, err := c.git.Run(cctx, root, "rev-parse", "--abbrev-ref", "HEAD")
	b := strings.TrimSpace(out)
	if err != nil || b == "HEAD" {
		return "-"
	}
	return b
}

// importEntry logs and links the commits one reflog movement brought in.
func (c *capture) importEntry(ctx context.Context, st *reflogState, e gitlink.ReflogEntry, branch string) error {
	seen := func(sha string) bool {
		c.mu.RLock()
		defer c.mu.RUnlock()
		return c.commitMap[commitKey(st.repoKey, sha)] != nil
	}
	// Each git run gets its own timeout: a long walk (a pull or a
	// fast-forward of many commits) must not share one budget.
	cctx, cancel := context.WithTimeout(ctx, gitTimeout)
	shas, skipped, err := gitlink.NewCommits(cctx, c.git, st.root, e, seen)
	cancel()
	if err != nil {
		return err
	}
	if skipped > 0 {
		log.Printf("capture: %s: %d commits past the %d-commit walk cap were not imported", filepath.Base(st.root), skipped, gitlink.MaxWalk)
	}
	if branch == "-" {
		branch = ""
	}
	op, _, _ := strings.Cut(e.Message, ":")
	var firstErr error
	for _, sha := range shas {
		if seen(sha) {
			continue
		}
		dctx, cancel := context.WithTimeout(ctx, gitTimeout)
		com, err := gitlink.Describe(dctx, c.git, st.root, sha)
		cancel()
		if err != nil {
			if ctx.Err() != nil {
				return ctx.Err()
			}
			// One unreadable commit doesn't stop the rest of the walk; the
			// error makes watchRoot try the line again.
			if firstErr == nil {
				firstErr = err
			}
			continue
		}
		com.Subject = c.scrub.String(com.Subject)
		row := CommitRow{RepoKey: st.repoKey, Root: st.root, Op: strings.TrimSpace(op), Branch: branch, Commit: com}
		if err := c.emit(ctx, "irori", KindCommitSeen, row); err != nil {
			return err
		}
		if err := c.linkCommit(ctx, row); err != nil {
			return err
		}
		// Checked after a commit is in, so every pass makes progress.
		if time.Now().After(c.repoDeadline) {
			return errRepoBudget
		}
	}
	return firstErr
}

// candidates are the edits a commit can link; c.mu held. They are the
// repo's uncommitted edits plus, for a path whose committed blob an already
// linked edit produced (an amend, cherry-pick or rebase of a linked commit),
// that edit and the run its link named. Linked edits only enter where exact
// fires, so they never widen a partial or heuristic link.
func (c *capture) candidates(row CommitRow) []gitlink.EditRef {
	cut := c.now().Add(-poolAge)
	limit := row.At.Add(commitSlack)
	usable := func(e *EditRow) bool {
		return e != nil && !e.At.Before(cut) && (row.At.IsZero() || !e.At.After(limit))
	}
	var out []gitlink.EditRef
	have := map[string]bool{}
	add := func(e *EditRow) {
		if have[e.ID] {
			return
		}
		have[e.ID] = true
		out = append(out, gitlink.EditRef{
			ID: e.ID, Root: e.Root, Path: e.Path, AfterBlob: e.AfterBlob, AfterBlobLF: e.AfterBlobLF,
			Heuristic: e.Fidelity == transcript.Heuristic, At: e.At,
		})
	}
	for id := range c.pool[row.RepoKey] {
		if e := c.editByID[id]; usable(e) {
			add(e)
		}
	}
	blobs := map[string]string{}
	for _, f := range row.Files {
		if f.Blob != "" {
			blobs[f.Path] = f.Blob
		}
	}
	if len(blobs) > 0 {
		for _, e := range c.editList {
			if e.RepoKey != row.RepoKey || !c.linked[e.ID] || e.Fidelity == transcript.Heuristic || !usable(e) {
				continue
			}
			b := blobs[e.Path]
			if b == "" || !(strings.EqualFold(e.AfterBlob, b) || (e.AfterBlobLF != "" && strings.EqualFold(e.AfterBlobLF, b))) {
				continue
			}
			add(e)
			for _, id := range c.linkedWith[e.ID] {
				if r := c.editByID[id]; usable(r) && r.Path == e.Path {
					add(r)
				}
			}
		}
	}
	// Map order is random; Classify keeps candidate order for equal times.
	sort.Slice(out, func(i, j int) bool {
		if !out[i].At.Equal(out[j].At) {
			return out[i].At.Before(out[j].At)
		}
		return out[i].ID < out[j].ID
	})
	return out
}

// linkCommit classifies a commit's files against the uncommitted edits and
// logs commit.linked.
func (c *capture) linkCommit(ctx context.Context, row CommitRow) error {
	c.mu.RLock()
	cands := c.candidates(row)
	var cmds []gitlink.CommitCmd
	for _, cmd := range c.cmds {
		if cmd.spans(row.At) {
			cmds = append(cmds, cmd.ref())
		}
	}
	c.mu.RUnlock()
	link := CommitLink{RepoKey: row.RepoKey, SHA: row.SHA, Files: gitlink.Classify(row.Commit, row.Root, cands), At: c.now()}
	if who := gitlink.Committer(row.Commit, row.Root, cmds); who != nil {
		link.Committer = &Committer{Key: who.Key, TurnID: who.TurnID}
	}
	c.fillSessions(&link)
	return c.emit(ctx, "irori", KindCommitLink, link)
}

// fillSessions sets a link's Sessions and Turns from its edits.
func (c *capture) fillSessions(l *CommitLink) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	type turnRef struct{ key, turn string }
	var keys []string
	byKey := map[string][]string{}
	seenTurn := map[turnRef]bool{}
	for _, f := range l.Files {
		for _, id := range f.Edits {
			e := c.editByID[id]
			if e == nil {
				continue
			}
			if _, ok := byKey[e.Key]; !ok {
				keys = append(keys, e.Key)
				byKey[e.Key] = nil
			}
			tr := turnRef{e.Key, e.TurnID}
			if !seenTurn[tr] {
				seenTurn[tr] = true
				byKey[e.Key] = append(byKey[e.Key], e.TurnID)
			}
		}
	}
	l.Sessions, l.Turns = keys, nil
	for _, k := range keys {
		l.Turns = append(l.Turns, byKey[k]...)
	}
}

// linkUnlinked links commits whose commit.seen was logged but whose link
// never was (a crash between the two).
func (c *capture) linkUnlinked(ctx context.Context) {
	c.mu.RLock()
	var rows []CommitRow
	for _, rec := range c.commitMap {
		if rec.link == nil {
			rows = append(rows, rec.row)
		}
	}
	c.mu.RUnlock()
	for _, row := range rows {
		if err := c.linkCommit(ctx, row); err != nil {
			c.setErr(err)
			return
		}
	}
}

// relink re-links recent commits when edits arriving this pass match a
// committed blob that their link left partial or unattributed (backfill
// catching up after the commit was seen).
func (c *capture) relink(ctx context.Context) {
	c.mu.Lock()
	fresh := c.newEdits
	c.newEdits = nil
	c.mu.Unlock()
	if len(fresh) == 0 {
		return
	}
	cut := c.now().Add(-relinkAge)
	type job struct {
		row  CommitRow
		link CommitLink
	}
	var jobs []job
	c.mu.RLock()
	for _, rec := range c.commitMap {
		if rec.link == nil || rec.seenAt.Before(cut) {
			continue
		}
		blobs := map[string]string{}
		for _, f := range rec.row.Files {
			blobs[f.Path] = f.Blob
		}
		hit := false
		for _, fl := range rec.link.Files {
			if fl.Link == gitlink.LinkExact {
				continue
			}
			for _, e := range fresh {
				b := blobs[fl.Path]
				if e.RepoKey == rec.row.RepoKey && e.Path == fl.Path && b != "" &&
					(strings.EqualFold(e.AfterBlob, b) || (e.AfterBlobLF != "" && strings.EqualFold(e.AfterBlobLF, b))) {
					hit = true
					break
				}
			}
			if hit {
				break
			}
		}
		if hit {
			jobs = append(jobs, job{row: rec.row, link: *rec.link})
		}
	}
	c.mu.RUnlock()
	for _, j := range jobs {
		c.mu.RLock()
		cands := c.candidates(j.row)
		c.mu.RUnlock()
		fresh := gitlink.Classify(j.row.Commit, j.row.Root, cands)
		next := j.link
		next.Files = slices.Clone(j.link.Files)
		changed := false
		for i := range next.Files {
			if i >= len(fresh) || fresh[i].Path != next.Files[i].Path {
				continue
			}
			if next.Files[i].Link != gitlink.LinkExact && fresh[i].Link == gitlink.LinkExact {
				ids := append(slices.Clone(next.Files[i].Edits), fresh[i].Edits...)
				c.mu.RLock()
				sort.SliceStable(ids, func(a, b int) bool {
					ea, eb := c.editByID[ids[a]], c.editByID[ids[b]]
					return ea != nil && eb != nil && ea.At.Before(eb.At)
				})
				c.mu.RUnlock()
				next.Files[i] = gitlink.FileLink{Path: next.Files[i].Path, Link: gitlink.LinkExact, Edits: slices.Compact(ids)}
				changed = true
			}
		}
		if !changed {
			continue
		}
		next.At = c.now()
		c.fillSessions(&next)
		if err := c.emit(ctx, "irori", KindCommitLink, next); err != nil {
			c.setErr(err)
			return
		}
	}
}

// oldestEditAt is the time of the oldest captured edit under a root.
func (c *capture) oldestEditAt(n string) (time.Time, bool) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	var oldest time.Time
	for _, e := range c.editList {
		if e.Root != "" && normRoot(e.Root) == n && (oldest.IsZero() || e.At.Before(oldest)) {
			oldest = e.At
		}
	}
	return oldest, !oldest.IsZero()
}

// reflogOffsetSince is the byte offset of the first logs/HEAD line at or
// after since, looking only at the last maxReflogRead bytes. ok is false
// when no such line is there.
func reflogOffsetSince(path string, size int64, since time.Time) (int64, bool) {
	// Read one byte before the window so a window that starts exactly on a
	// line start isn't taken for a partial line.
	start := max(0, size-maxReflogRead-1)
	fh, err := os.Open(path)
	if err != nil {
		return 0, false
	}
	defer fh.Close()
	data, err := io.ReadAll(io.NewSectionReader(fh, start, size-start))
	if err != nil {
		return 0, false
	}
	off := start
	if start > 0 {
		// Skip up to the first line start inside the window (the byte read
		// before it, when that byte is a newline).
		i := bytes.IndexByte(data, '\n')
		if i < 0 {
			return 0, false
		}
		data, off = data[i+1:], off+int64(i+1)
	}
	for len(data) > 0 {
		i := bytes.IndexByte(data, '\n')
		if i < 0 {
			break
		}
		if es, _ := gitlink.ParseReflog(data[:i+1]); len(es) == 1 && !es[0].At.Before(since) {
			return off, true
		}
		data, off = data[i+1:], off+int64(i+1)
	}
	return 0, false
}

// relinkCommitter gives a commit command that arrived after its commit was
// linked (a long `git commit && gh pr checks --watch` writes its result only
// when the whole command ends) the commits it may have made: linked commits
// with no committer, under its dir and inside its span, get a new
// commit.linked with the same files and the committer worked out again.
func (c *capture) relinkCommitter(ctx context.Context, cmd CommitCmdRow) error {
	ref := cmd.ref()
	c.mu.RLock()
	var todo []commitRec
	for _, rec := range c.commitMap {
		if rec.link == nil || rec.link.Committer != nil || !cmd.spans(rec.row.At) {
			continue
		}
		if gitlink.Committer(rec.row.Commit, rec.row.Root, []gitlink.CommitCmd{ref}) == nil {
			continue
		}
		todo = append(todo, *rec)
	}
	var cmds []gitlink.CommitCmd
	for _, rec := range todo {
		for _, other := range c.cmds {
			if other.spans(rec.row.At) {
				cmds = append(cmds, other.ref())
			}
		}
	}
	c.mu.RUnlock()
	for _, rec := range todo {
		who := gitlink.Committer(rec.row.Commit, rec.row.Root, cmds)
		if who == nil {
			continue
		}
		link := *rec.link
		link.Committer = &Committer{Key: who.Key, TurnID: who.TurnID}
		link.At = c.now()
		if err := c.emit(ctx, "irori", KindCommitLink, link); err != nil {
			return err
		}
	}
	return nil
}
