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
	n, size := len(lines), a.End-a.Start+1
	match := func(s int) bool {
		return s >= 1 && s+size-1 <= n && hashLines(lines[s-1:s+size-1]) == a.Hash
	}
	if match(a.Start) {
		return Status{State: OK, Start: a.Start, End: a.End}
	}
	// Scan outward so the nearest window wins and the earlier breaks ties.
	last := n - size + 1
	for d := 1; a.Start-d >= 1 || a.Start+d <= last; d++ {
		for _, s := range [2]int{a.Start - d, a.Start + d} {
			if match(s) {
				return Status{State: Moved, Start: s, End: s + size - 1}
			}
		}
	}
	return Status{State: Changed}
}
