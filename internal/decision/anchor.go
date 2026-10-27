package decision

import (
	"os"
	"path"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/StephenSHorton/irori/internal/gitsha"
)

// maxCheckSize is the largest file Check scans; bigger files are Changed.
var maxCheckSize int64 = 8 << 20

// scanBudget caps the bytes Check hashes while looking for a moved window;
// past it the anchor is Changed.
var scanBudget int64 = 64 << 20

func makeAnchor(root, p string, start, end int, rk, sha string) (Anchor, error) {
	if start < 0 || end < 0 || (start == 0 && end != 0) || (start > 0 && end < start) {
		return Anchor{}, ErrBadLines
	}
	rel, err := relPath(root, p)
	if err != nil {
		return Anchor{}, err
	}
	full := filepath.Join(root, filepath.FromSlash(rel))
	if fi, err := os.Stat(full); err != nil || fi.IsDir() {
		return Anchor{}, ErrNoFile
	}
	data, err := os.ReadFile(full)
	if err != nil {
		return Anchor{}, ErrNoFile
	}
	rel = onDiskCase(root, rel)
	a := Anchor{RepoKey: rk, Root: filepath.Clean(root), Path: rel, SHA: sha}
	if start == 0 {
		a.Hash = gitsha.Blob(gitsha.LF(data))
		return a, nil
	}
	lines := splitLines(data)
	if end > len(lines) {
		return Anchor{}, ErrRange
	}
	a.Start, a.End = start, end
	a.Hash = hashLines(lines[start-1 : end])
	for i := max(1, start-ContextLines); i < start; i++ {
		a.Context = append(a.Context, lines[i-1])
	}
	for i := end + 1; i <= min(len(lines), end+ContextLines); i++ {
		a.Context = append(a.Context, lines[i-1])
	}
	return a, nil
}

// relPath turns an absolute path under root, or a root-relative one, into a
// clean forward-slash repo-relative path.
func relPath(root, p string) (string, error) {
	p = strings.TrimSpace(strings.ReplaceAll(p, `\`, "/"))
	if root == "" || p == "" || hasDotDot(p) {
		return "", ErrBadPath
	}
	native := filepath.FromSlash(p)
	if filepath.IsAbs(native) {
		r, ok := under(filepath.Clean(root), filepath.Clean(native))
		if !ok {
			return "", ErrBadPath
		}
		p = filepath.ToSlash(r)
	} else if strings.HasPrefix(p, "/") || filepath.VolumeName(native) != "" {
		// Rooted without a volume, or "C:foo", on Windows.
		return "", ErrBadPath
	}
	p = path.Clean(p)
	if p == "." || strings.HasPrefix(p, "/") || hasDotDot(p) {
		return "", ErrBadPath
	}
	return p, nil
}

// under returns p relative to root when p lies strictly inside it, comparing
// case-insensitively on Windows.
func under(root, p string) (string, bool) {
	prefix := root
	if !strings.HasSuffix(prefix, string(filepath.Separator)) {
		prefix += string(filepath.Separator)
	}
	if len(p) <= len(prefix) {
		return "", false
	}
	head := p[:len(prefix)]
	if runtime.GOOS == "windows" {
		if !strings.EqualFold(head, prefix) {
			return "", false
		}
	} else if head != prefix {
		return "", false
	}
	return p[len(prefix):], true
}

func hasDotDot(p string) bool {
	for _, seg := range strings.Split(p, "/") {
		if seg == ".." {
			return true
		}
	}
	return false
}

func check(a Anchor, root string) Status {
	if root == "" || a.Hash == "" || a.Start < 0 || (a.Start > 0 && a.End < a.Start) {
		return Status{State: Unknown}
	}
	// A missing worktree is Unknown, not Gone.
	if fi, err := os.Stat(root); err != nil || !fi.IsDir() {
		return Status{State: Unknown}
	}
	// The anchor came from the log: only follow a clean relative path.
	rel, err := relPath(root, a.Path)
	if err != nil || rel != a.Path {
		return Status{State: Unknown}
	}
	full := filepath.Join(root, filepath.FromSlash(rel))
	fi, err := os.Stat(full)
	if os.IsNotExist(err) || (err == nil && fi.IsDir()) {
		return Status{State: Gone}
	}
	if err != nil {
		return Status{State: Unknown}
	}
	if fi.Size() > maxCheckSize {
		return Status{State: Changed}
	}
	data, err := os.ReadFile(full)
	if os.IsNotExist(err) {
		return Status{State: Gone}
	}
	if err != nil {
		return Status{State: Unknown}
	}
	if a.Start == 0 {
		if gitsha.Blob(gitsha.LF(data)) == a.Hash {
			return Status{State: OK}
		}
		return Status{State: Changed}
	}
	lines := splitLines(data)
	n := len(lines)
	// End >= Start >= 1 here, so this cannot overflow.
	if a.End-a.Start >= n {
		return Status{State: Changed}
	}
	size := a.End - a.Start + 1
	last := n - size + 1 // the last window start
	if a.Start <= last && hashLines(lines[a.Start-1:a.Start-1+size]) == a.Hash {
		return Status{State: OK, Start: a.Start, End: a.End}
	}
	// off[i] is the byte offset of line i+1 in the normalised text.
	off := make([]int64, n+1)
	for i, l := range lines {
		off[i+1] = off[i] + int64(len(l)) + 1
	}
	// Scan starts 1..last outward from Start so the nearest window wins and
	// the earlier breaks ties, hashing at most scanBudget bytes.
	lo, hi := a.Start-1, a.Start+1
	if a.Start > last {
		lo, hi = last, last+1
	}
	budget := scanBudget
	for lo >= 1 || hi <= last {
		var s int
		if lo >= 1 && (hi > last || a.Start-lo <= hi-a.Start) {
			s, lo = lo, lo-1
		} else {
			s, hi = hi, hi+1
		}
		if budget -= off[s-1+size] - off[s-1]; budget < 0 {
			break
		}
		if hashLines(lines[s-1:s-1+size]) == a.Hash {
			return Status{State: Moved, Start: s, End: s + size - 1}
		}
	}
	return Status{State: Changed}
}

// onDiskCase rewrites each segment of rel to its spelling on disk when the
// file system matched it case-insensitively, so one file has one Path.
func onDiskCase(root, rel string) string {
	segs := strings.Split(rel, "/")
	dir := root
	for i, seg := range segs {
		entries, err := os.ReadDir(dir)
		if err != nil {
			return rel
		}
		found, folded := false, ""
		nfold := 0
		for _, e := range entries {
			if e.Name() == seg {
				found = true
				break
			}
			if strings.EqualFold(e.Name(), seg) {
				folded, nfold = e.Name(), nfold+1
			}
		}
		if !found && nfold == 1 {
			segs[i] = folded
		}
		dir = filepath.Join(dir, segs[i])
	}
	return strings.Join(segs, "/")
}
