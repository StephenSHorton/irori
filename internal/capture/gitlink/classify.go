package gitlink

import (
	"sort"
	"strings"
	"time"
)

// classify follows the rules on Classify. Roots compare with sameRoot (slash
// style and, for Windows-style paths, case do not matter); repo-relative
// paths and blobs compare exactly (blobs ignoring hex case).
func classify(c Commit, root string, cands []EditRef) []FileLink {
	links := make([]FileLink, 0, len(c.Files))
	for _, f := range c.Files {
		links = append(links, classifyFile(f, root, cands))
	}
	return links
}

func classifyFile(f FileChange, root string, cands []EditRef) FileLink {
	link := FileLink{Path: f.Path, Link: LinkUnattributed}
	deleted := f.Status == "D" || f.Blob == "" || zeroSHA(f.Blob)

	if !deleted {
		best := -1
		for i, e := range cands {
			if e.Path != f.Path || e.Heuristic || !blobMatch(e, f.Blob) {
				continue
			}
			if best < 0 || !e.At.Before(cands[best].At) {
				best = i
			}
		}
		if best >= 0 {
			b := cands[best]
			var run []int
			for i, e := range cands {
				if i == best || (e.Path == b.Path && sameRoot(e.Root, b.Root) && !e.At.After(b.At)) {
					run = append(run, i)
				}
			}
			link.Link = LinkExact
			link.Edits = editIDs(cands, run)
			return link
		}
	}

	// An empty root matches no edit's root, not even an empty one: an edit
	// whose root failed to resolve is not credited to a commit whose root
	// also failed to.
	var own []int
	allHeuristic := true
	for i, e := range cands {
		if root != "" && e.Path == f.Path && sameRoot(e.Root, root) {
			own = append(own, i)
			allHeuristic = allHeuristic && e.Heuristic
		}
	}
	if len(own) == 0 {
		return link
	}
	link.Link = LinkPartial
	if allHeuristic {
		link.Link = LinkHeuristic
	}
	link.Edits = editIDs(cands, own)
	return link
}

func blobMatch(e EditRef, blob string) bool {
	return (e.AfterBlob != "" && strings.EqualFold(e.AfterBlob, blob)) ||
		(e.AfterBlobLF != "" && strings.EqualFold(e.AfterBlobLF, blob))
}

// editIDs returns the ids of cands[idx], oldest first; equal times keep
// their candidate order.
func editIDs(cands []EditRef, idx []int) []string {
	sort.SliceStable(idx, func(a, b int) bool { return cands[idx[a]].At.Before(cands[idx[b]].At) })
	ids := make([]string, len(idx))
	for k, i := range idx {
		ids[k] = cands[i].ID
	}
	return ids
}

func committer(c Commit, root string, cmds []CommitCmd) *CommitCmd {
	if root == "" {
		return nil
	}
	best := -1
	var bestD time.Duration
	for i, cmd := range cmds {
		if !underRoot(cmd.Cwd, root) {
			continue
		}
		// Bound-check before subtracting: Sub saturates for times centuries
		// apart (a zero At), and negating math.MinInt64 stays negative.
		if cmd.At.Before(c.At.Add(-CommitterWindow)) || cmd.At.After(c.At.Add(CommitterWindow)) {
			continue
		}
		d := cmd.At.Sub(c.At)
		if d < 0 {
			d = -d
		}
		if best < 0 || d < bestD || (d == bestD && cmd.At.After(cmds[best].At)) {
			best, bestD = i, d
		}
	}
	if best < 0 {
		return nil
	}
	pick := cmds[best]
	return &pick
}

// underRoot reports whether dir is root or inside it.
func underRoot(dir, root string) bool {
	d, r := normPath(dir, root), normPath(root, dir)
	if d == "" || r == "" {
		return false
	}
	if d == r {
		return true
	}
	if strings.HasSuffix(r, "/") { // a drive root "c:/" or "/"
		return strings.HasPrefix(d, r)
	}
	return strings.HasPrefix(d, r+"/")
}

// sameRoot compares two worktree roots the way underRoot does.
func sameRoot(a, b string) bool {
	if a == b {
		return true
	}
	na, nb := normPath(a, b), normPath(b, a)
	return na != "" && na == nb
}

// normPath puts p in a comparable form: forward slashes, no trailing slash
// (except a bare drive root "c:/" or "/"), and, when p or ref is a
// Windows-style path, lowercase, with a Git Bash "/c/..." turned into
// "c:/...". ref decides the style together with p, so "/c/x" only maps to
// a drive when the other side is a Windows path.
func normPath(p, ref string) string {
	if p == "" {
		return ""
	}
	win := windowsStyle(p) || windowsStyle(ref)
	p = strings.ReplaceAll(p, `\`, "/")
	if win {
		p = strings.ToLower(p)
		if len(p) >= 2 && p[0] == '/' && isLetter(p[1]) && (len(p) == 2 || p[2] == '/') {
			p = p[1:2] + ":" + p[2:]
		}
	}
	for len(p) > 1 && strings.HasSuffix(p, "/") && !(len(p) == 3 && p[1] == ':') {
		p = p[:len(p)-1]
	}
	if len(p) == 2 && p[1] == ':' {
		p += "/"
	}
	return p
}

// windowsStyle reports a drive-letter path ("C:\x", "c:/x") or one with
// backslashes (including UNC "\\host\share").
func windowsStyle(p string) bool {
	if strings.Contains(p, `\`) {
		return true
	}
	return len(p) >= 2 && isLetter(p[0]) && p[1] == ':'
}

func isLetter(c byte) bool { return 'a' <= c && c <= 'z' || 'A' <= c && c <= 'Z' }
