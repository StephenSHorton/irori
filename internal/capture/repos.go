package capture

import (
	"bytes"
	"context"
	"io"
	"log"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
)

// maxReflogRead caps one pass's read of a logs/HEAD.
const maxReflogRead = 4 << 20

// reflogState is one watched worktree root.
type reflogState struct {
	root    string
	repoKey string
	logPath string
	offset  int64
}

// repoPass checks every known root's reflog and imports new commits.
func (c *capture) repoPass(ctx context.Context) {
	c.loadCursors()
	for _, card := range c.liveCards() {
		if card.Cwd != "" {
			c.repoFor(ctx, card.Cwd)
		}
	}
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
	}
	c.saveCursors()
	c.linkUnlinked(ctx)
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
			// First sight: only movements from now on, never the history.
			if fi, err := os.Stat(st.logPath); err == nil {
				st.offset = fi.Size()
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
		for _, e := range entries {
			if branch == "" {
				branch = c.branch(ctx, st.root)
			}
			if err := c.importEntry(ctx, st, e, branch); err != nil {
				if ctx.Err() != nil {
					return
				}
				c.setErr(err)
				log.Printf("capture: reflog %s: %v", filepath.Base(st.root), err)
			}
		}
		st.offset += int64(len(lineBytes))
		c.cursors.setReflog(n, st.offset)
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
	cctx, cancel := context.WithTimeout(ctx, gitTimeout)
	defer cancel()
	shas, skipped, err := gitlink.NewCommits(cctx, c.git, st.root, e, seen)
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
	for _, sha := range shas {
		if seen(sha) {
			continue
		}
		com, err := gitlink.Describe(cctx, c.git, st.root, sha)
		if err != nil {
			return err
		}
		row := CommitRow{RepoKey: st.repoKey, Root: st.root, Op: strings.TrimSpace(op), Branch: branch, Commit: com}
		if err := c.emit(ctx, "irori", KindCommitSeen, row); err != nil {
			return err
		}
		if err := c.linkCommit(ctx, row); err != nil {
			return err
		}
	}
	return nil
}

// candidates are the repo's uncommitted edits a commit can link; c.mu held.
func (c *capture) candidates(row CommitRow) []gitlink.EditRef {
	cut := c.now().Add(-poolAge)
	limit := row.At.Add(commitSlack)
	var out []gitlink.EditRef
	for id := range c.pool[row.RepoKey] {
		e := c.editByID[id]
		if e == nil || e.At.Before(cut) || (!row.At.IsZero() && e.At.After(limit)) {
			continue
		}
		out = append(out, gitlink.EditRef{
			ID: e.ID, Root: e.Root, Path: e.Path, AfterBlob: e.AfterBlob, AfterBlobLF: e.AfterBlobLF,
			Heuristic: e.Fidelity == transcript.Heuristic, At: e.At,
		})
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
		d := cmd.At.Sub(row.At)
		if d < 0 {
			d = -d
		}
		if d <= 2*gitlink.CommitterWindow {
			cmds = append(cmds, gitlink.CommitCmd{Key: cmd.Key, TurnID: cmd.TurnID, Cwd: cmd.Cwd, At: cmd.At})
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
