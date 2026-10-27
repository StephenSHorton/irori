package capture

import (
	"context"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
)

// negativeTTL is how long "not in a repo" is cached for a directory.
const negativeTTL = time.Minute

// rootCache resolves directories to worktree roots with `git rev-parse
// --show-toplevel`, cached per directory.
type rootCache struct {
	git gitlink.Git
	now func() time.Time

	mu  sync.Mutex
	dir map[string]rootEntry
}

type rootEntry struct {
	root    string
	expires time.Time // zero: forever
}

func newRootCache(g gitlink.Git, now func() time.Time) *rootCache {
	return &rootCache{git: g, now: now, dir: map[string]rootEntry{}}
}

// forDir is the worktree root holding dir ("" when none). dir need not
// exist: the nearest existing ancestor is asked.
func (r *rootCache) forDir(ctx context.Context, dir string) string {
	if dir == "" || !filepath.IsAbs(dir) {
		return ""
	}
	dir = filepath.Clean(dir)
	for {
		if fi, err := os.Stat(dir); err == nil && fi.IsDir() {
			break
		}
		up := filepath.Dir(dir)
		if up == dir {
			return ""
		}
		dir = up
	}
	k := normRoot(dir)
	now := r.now()
	r.mu.Lock()
	e, ok := r.dir[k]
	r.mu.Unlock()
	if ok && (e.expires.IsZero() || now.Before(e.expires)) {
		return e.root
	}
	cctx, cancel := context.WithTimeout(ctx, gitTimeout)
	defer cancel()
	out, err := r.git.Run(cctx, dir, "rev-parse", "--show-toplevel")
	root := strings.TrimSpace(out)
	if err != nil || root == "" {
		if ctx.Err() != nil || cctx.Err() != nil {
			return "" // don't cache a timeout
		}
		r.mu.Lock()
		r.dir[k] = rootEntry{expires: now.Add(negativeTTL)}
		r.mu.Unlock()
		return ""
	}
	root = filepath.Clean(filepath.FromSlash(root))
	r.mu.Lock()
	r.dir[k] = rootEntry{root: root}
	r.mu.Unlock()
	return root
}

// forFile is the worktree root holding a file.
func (r *rootCache) forFile(ctx context.Context, path string) string {
	if path == "" || !filepath.IsAbs(path) {
		return ""
	}
	return r.forDir(ctx, filepath.Dir(filepath.Clean(path)))
}

// normRoot is a comparable form of a directory: cleaned, OS separators, and
// lowercase on Windows.
func normRoot(p string) string {
	p = filepath.Clean(filepath.FromSlash(p))
	if runtime.GOOS == "windows" {
		p = strings.ToLower(p)
	}
	return p
}

// relTo returns path relative to root with forward slashes, and whether
// path is inside root.
func relTo(root, path string) (string, bool) {
	if root == "" || path == "" {
		return "", false
	}
	rel, err := filepath.Rel(filepath.Clean(root), filepath.Clean(filepath.FromSlash(path)))
	if err != nil || rel == "." || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) || filepath.IsAbs(rel) {
		return "", false
	}
	return filepath.ToSlash(rel), true
}

// dirExists reports an existing directory.
func dirExists(p string) bool {
	fi, err := os.Stat(p)
	return err == nil && fi.IsDir()
}
