package gitlink

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// testRepo is a real git repository in a temp dir. Every git run uses an
// empty global config, no system config and a fixed identity, so the
// machine's own settings never leak in.
type testRepo struct {
	t    *testing.T
	root string
}

// isolateGit points git at an empty config for the rest of the test. Exec
// inherits it too.
func isolateGit(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not on PATH")
	}
	cfg := filepath.Join(t.TempDir(), "gitconfig")
	if err := os.WriteFile(cfg, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("GIT_CONFIG_GLOBAL", cfg)
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	t.Setenv("GIT_AUTHOR_NAME", "Test Author")
	t.Setenv("GIT_AUTHOR_EMAIL", "author@example.invalid")
	t.Setenv("GIT_COMMITTER_NAME", "Test Committer")
	t.Setenv("GIT_COMMITTER_EMAIL", "committer@example.invalid")
}

// newRepo makes a repo on branch main with core.autocrlf set as given.
func newRepo(t *testing.T, autocrlf string) *testRepo {
	t.Helper()
	isolateGit(t)
	r := &testRepo{t: t, root: filepath.Join(t.TempDir(), "repo")}
	if err := os.MkdirAll(r.root, 0o755); err != nil {
		t.Fatal(err)
	}
	r.git("init", "-q", "-b", "main")
	r.git("config", "core.autocrlf", autocrlf)
	return r
}

// gitIn runs git in dir with extra env and returns trimmed stdout.
func gitIn(t *testing.T, dir string, env []string, stdin string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
	cmd.Env = append(os.Environ(), env...)
	if stdin != "" {
		cmd.Stdin = strings.NewReader(stdin)
	}
	var out, errb bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &errb
	if err := cmd.Run(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, errb.String())
	}
	return strings.TrimSpace(out.String())
}

func (r *testRepo) git(args ...string) string { return gitIn(r.t, r.root, nil, "", args...) }

// write writes a file under the repo, making directories.
func (r *testRepo) write(rel, content string) {
	r.t.Helper()
	p := filepath.Join(r.root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		r.t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		r.t.Fatal(err)
	}
}

// commit stages everything and commits at the given unix committer time,
// returning the new HEAD.
func (r *testRepo) commit(msg string, at int64) string {
	r.t.Helper()
	return commitIn(r.t, r.root, msg, at)
}

func commitIn(t *testing.T, dir, msg string, at int64) string {
	t.Helper()
	date := strconv.FormatInt(at, 10) + " +0000"
	env := []string{"GIT_AUTHOR_DATE=" + date, "GIT_COMMITTER_DATE=" + date}
	gitIn(t, dir, env, "", "add", "-A")
	gitIn(t, dir, env, "", "commit", "-q", "--allow-empty", "-m", msg)
	return gitIn(t, dir, nil, "", "rev-parse", "HEAD")
}

// reflog parses the repo's (main worktree's) logs/HEAD.
func (r *testRepo) reflog() []ReflogEntry {
	r.t.Helper()
	dir, err := GitDir(r.root)
	if err != nil {
		r.t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(dir, "logs", "HEAD"))
	if err != nil {
		r.t.Fatal(err)
	}
	entries, n := ParseReflog(data)
	if n != len(data) {
		r.t.Fatalf("consumed %d of %d bytes", n, len(data))
	}
	return entries
}

// blob is the blob a path has in a commit.
func (r *testRepo) blob(rev, rel string) string { return r.git("rev-parse", rev+":"+rel) }

func ctx(t *testing.T) context.Context {
	c, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	t.Cleanup(cancel)
	return c
}

// seenSet is a seen func over a set that tests add to.
type seenSet map[string]bool

func (s seenSet) has(sha string) bool { return s[sha] }
