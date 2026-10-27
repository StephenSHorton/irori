package repo

import (
	"bytes"
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

const (
	gitTimeout = 2 * time.Second
	cacheTTL   = 5 * time.Minute
)

// errTimeout marks a git run that ran out of time. Its answer isn't cached.
var errTimeout = errors.New("repo: git timed out")

// runFunc runs git with args and returns its trimmed stdout.
type runFunc func(ctx context.Context, args ...string) (string, error)

type entry struct {
	key     string
	expires time.Time
}

type resolver struct {
	now     func() time.Time
	run     runFunc
	ttl     time.Duration
	timeout time.Duration

	mu     sync.Mutex
	byDir  map[string]entry
	byRoot map[string]entry
}

func newResolver() *Resolver {
	return &Resolver{impl: &resolver{
		now:     time.Now,
		run:     runGit,
		ttl:     cacheTTL,
		timeout: gitTimeout,
		byDir:   map[string]entry{},
		byRoot:  map[string]entry{},
	}}
}

func (r *Resolver) key(dir string) string { return r.impl.key(dir) }

func (r *resolver) key(dir string) string {
	if dir == "" {
		return ""
	}
	// Cache by absolute path: a relative dir means something else once the
	// process cwd moves.
	abs, err := filepath.Abs(dir)
	if err != nil {
		return ""
	}
	dir = abs
	if k, ok := r.cached(r.byDir, dir); ok {
		return k
	}

	root, err := r.git("-C", dir, "rev-parse", "--show-toplevel")
	if err != nil || root == "" {
		if !errors.Is(err, errTimeout) {
			r.store(r.byDir, dir, "")
		}
		return ""
	}
	root = filepath.Clean(root)
	if k, ok := r.cached(r.byRoot, root); ok {
		r.store(r.byDir, dir, k)
		return k
	}

	k, err := r.resolveRoot(root)
	if errors.Is(err, errTimeout) {
		return ""
	}
	r.store(r.byRoot, root, k)
	r.store(r.byDir, dir, k)
	return k
}

// resolveRoot computes the key for a repository toplevel: the normalized
// origin remote, else the smallest root commit, else "". The only error it
// returns is errTimeout.
func (r *resolver) resolveRoot(root string) (string, error) {
	url, err := r.git("-C", root, "remote", "get-url", "origin")
	if errors.Is(err, errTimeout) {
		return "", err
	}
	if err == nil {
		if n := NormalizeRemote(url); n != "" {
			return "remote:" + n, nil
		}
	}
	out, err := r.git("-C", root, "rev-list", "--max-parents=0", "HEAD")
	if errors.Is(err, errTimeout) {
		return "", err
	}
	if err != nil {
		return "", nil
	}
	if sha := smallestSHA(out); sha != "" {
		return "root:" + sha, nil
	}
	return "", nil
}

// smallestSHA picks the lexicographically smallest line, so a repo with
// several root commits (merged histories) keys the same everywhere.
func smallestSHA(out string) string {
	best := ""
	for _, line := range strings.Split(out, "\n") {
		s := strings.ToLower(strings.TrimSpace(line))
		if s == "" {
			continue
		}
		if best == "" || s < best {
			best = s
		}
	}
	return best
}

func (r *resolver) git(args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), r.timeout)
	defer cancel()
	out, err := r.run(ctx, args...)
	if ctx.Err() != nil {
		return "", errTimeout
	}
	return out, err
}

func (r *resolver) cached(m map[string]entry, k string) (string, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	e, ok := m[k]
	if !ok {
		return "", false
	}
	if !r.now().Before(e.expires) {
		delete(m, k)
		return "", false
	}
	return e.key, true
}

func (r *resolver) store(m map[string]entry, k, key string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	m[k] = entry{key: key, expires: r.now().Add(r.ttl)}
}

// runGit runs git from PATH with no shell and no console window.
func runGit(ctx context.Context, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Env = append(gitEnv(os.Environ()), "GIT_TERMINAL_PROMPT=0", "GIT_OPTIONAL_LOCKS=0")
	cmd.WaitDelay = 500 * time.Millisecond
	hideWindow(cmd)
	var stdout bytes.Buffer
	cmd.Stdout = &stdout
	if err := cmd.Run(); err != nil {
		return "", err
	}
	return strings.TrimSpace(stdout.String()), nil
}

// repoEnvVars point git at one repository regardless of -C. Inherited from a
// hook or another tool, they would give every directory that repo's key.
var repoEnvVars = []string{
	"GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE",
	"GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
	"GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM", "GIT_NAMESPACE",
}

// gitEnv returns env without the repo-selecting git variables.
func gitEnv(env []string) []string {
	out := make([]string, 0, len(env))
	for _, kv := range env {
		name, _, _ := strings.Cut(kv, "=")
		drop := false
		for _, v := range repoEnvVars {
			if sameEnvName(name, v) {
				drop = true
				break
			}
		}
		if !drop {
			out = append(out, kv)
		}
	}
	return out
}
