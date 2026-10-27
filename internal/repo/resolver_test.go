package repo

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// fakeGit answers git calls from a table keyed by the joined args and counts
// every call.
type fakeGit struct {
	mu      sync.Mutex
	answers map[string]string // args -> stdout; missing means exit error
	calls   []string
	block   bool // wait for the context to expire, like a hung git
}

func (f *fakeGit) run(ctx context.Context, args ...string) (string, error) {
	k := strings.Join(args, " ")
	f.mu.Lock()
	f.calls = append(f.calls, k)
	out, ok := f.answers[k]
	block := f.block
	f.mu.Unlock()
	if block {
		<-ctx.Done()
		return "", ctx.Err()
	}
	if !ok {
		return "", errors.New("exit status 128")
	}
	return out, nil
}

func (f *fakeGit) count() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.calls)
}

func testResolver(f *fakeGit, now *time.Time) *Resolver {
	r := New()
	r.impl.run = f.run
	r.impl.now = func() time.Time { return *now }
	return r
}

func TestKeyCachesPerDirAndToplevel(t *testing.T) {
	root := filepath.Clean("/r")
	sub := filepath.Join(root, "sub")
	f := &fakeGit{answers: map[string]string{
		"-C " + root + " rev-parse --show-toplevel":     "/r",
		"-C " + sub + " rev-parse --show-toplevel":      "/r",
		"-C " + root + " remote get-url origin":         "git@github.com:Foo/Bar.git",
		"-C " + root + " rev-list --max-parents=0 HEAD": "abc",
	}}
	now := time.Unix(1000, 0)
	r := testResolver(f, &now)

	if got := r.Key(root); got != "remote:github.com/foo/bar" {
		t.Fatalf("Key(root) = %q", got)
	}
	if n := f.count(); n != 2 {
		t.Fatalf("first lookup ran %d git calls, want 2: %v", n, f.calls)
	}
	r.Key(root)
	if n := f.count(); n != 2 {
		t.Fatalf("cached lookup ran git: %v", f.calls)
	}
	// A new dir in the same repo costs only the toplevel lookup.
	if got := r.Key(sub); got != "remote:github.com/foo/bar" {
		t.Fatalf("Key(sub) = %q", got)
	}
	if n := f.count(); n != 3 {
		t.Fatalf("subdir lookup ran %d calls total, want 3: %v", n, f.calls)
	}

	// After the TTL everything is asked again.
	now = now.Add(cacheTTL)
	r.Key(sub)
	if n := f.count(); n != 5 {
		t.Fatalf("expired lookup ran %d calls total, want 5: %v", n, f.calls)
	}
}

func TestKeyFallsBackToRootCommit(t *testing.T) {
	f := &fakeGit{answers: map[string]string{
		"-C x rev-parse --show-toplevel":     "x",
		"-C x remote get-url origin":         "../local/clone",
		"-C x rev-list --max-parents=0 HEAD": "ffff\naaaa\n",
	}}
	now := time.Unix(0, 0)
	if got := testResolver(f, &now).Key("x"); got != "root:aaaa" {
		t.Fatalf("got %q", got)
	}

	f = &fakeGit{answers: map[string]string{
		"-C x rev-parse --show-toplevel": "x",
	}}
	if got := testResolver(f, &now).Key("x"); got != "" {
		t.Fatalf("no remote, no commits: got %q", got)
	}
}

func TestKeyCachesNegative(t *testing.T) {
	f := &fakeGit{answers: map[string]string{}}
	now := time.Unix(0, 0)
	r := testResolver(f, &now)
	if got := r.Key("nowhere"); got != "" {
		t.Fatalf("got %q", got)
	}
	r.Key("nowhere")
	if n := f.count(); n != 1 {
		t.Fatalf("negative not cached: %v", f.calls)
	}
	now = now.Add(cacheTTL + time.Second)
	r.Key("nowhere")
	if n := f.count(); n != 2 {
		t.Fatalf("negative didn't expire: %v", f.calls)
	}
	if got := r.Key(""); got != "" || f.count() != 2 {
		t.Fatalf("empty dir ran git or keyed: %q", got)
	}
}

func TestKeyTimeoutNotCached(t *testing.T) {
	f := &fakeGit{block: true}
	now := time.Unix(0, 0)
	r := testResolver(f, &now)
	r.impl.timeout = 50 * time.Millisecond
	start := time.Now()
	if got := r.Key("slow"); got != "" {
		t.Fatalf("got %q", got)
	}
	if d := time.Since(start); d > time.Second {
		t.Fatalf("timeout took %v", d)
	}
	f.mu.Lock()
	f.block = false
	f.answers = map[string]string{
		"-C slow rev-parse --show-toplevel":     "slow",
		"-C slow rev-list --max-parents=0 HEAD": "abc",
	}
	f.mu.Unlock()
	if got := r.Key("slow"); got != "root:abc" {
		t.Fatalf("timed-out answer was cached: got %q", got)
	}
}

func TestKeyConcurrent(t *testing.T) {
	f := &fakeGit{answers: map[string]string{
		"-C a rev-parse --show-toplevel":     "a",
		"-C a rev-list --max-parents=0 HEAD": "abc",
	}}
	now := time.Unix(0, 0)
	r := testResolver(f, &now)
	var wg sync.WaitGroup
	for range 32 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if got := r.Key("a"); got != "root:abc" {
				t.Errorf("got %q", got)
			}
		}()
	}
	wg.Wait()
}

// --- integration: real git in temp dirs ---

func needGit(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not on PATH")
	}
}

func git(t *testing.T, dir string, args ...string) string {
	t.Helper()
	full := append([]string{"-C", dir,
		"-c", "user.name=irori-test", "-c", "user.email=test@example.invalid",
		"-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"}, args...)
	cmd := exec.Command("git", full...)
	cmd.Env = append(os.Environ(), "GIT_CONFIG_NOSYSTEM=1", "GIT_CONFIG_GLOBAL="+os.DevNull)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
	return strings.TrimSpace(string(out))
}

func initRepo(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	git(t, dir, "init", "-q")
	git(t, dir, "commit", "-q", "--allow-empty", "-m", "first")
	return dir
}

func TestIntegrationRootCommit(t *testing.T) {
	needGit(t)
	dir := initRepo(t)
	want := "root:" + git(t, dir, "rev-list", "--max-parents=0", "HEAD")
	if got := New().Key(dir); got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestIntegrationRemote(t *testing.T) {
	needGit(t)
	dir := initRepo(t)
	git(t, dir, "remote", "add", "origin", "git@github.com:Foo/Bar.git")
	sub := filepath.Join(dir, "a", "b")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	r := New()
	if got := r.Key(dir); got != "remote:github.com/foo/bar" {
		t.Fatalf("root: got %q", got)
	}
	if got := r.Key(sub); got != "remote:github.com/foo/bar" {
		t.Fatalf("subdir: got %q", got)
	}
}

func TestIntegrationLocalRemoteFallsBack(t *testing.T) {
	needGit(t)
	dir := initRepo(t)
	git(t, dir, "remote", "add", "origin", t.TempDir())
	want := "root:" + git(t, dir, "rev-list", "--max-parents=0", "HEAD")
	if got := New().Key(dir); got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestIntegrationClonesMatch(t *testing.T) {
	needGit(t)
	a := initRepo(t)
	b := t.TempDir()
	git(t, b, "clone", "-q", a, "clone")
	clone := filepath.Join(b, "clone")
	// A local origin falls back to the shared root commit.
	r := New()
	if ka, kb := r.Key(a), r.Key(clone); ka == "" || ka != kb {
		t.Fatalf("clone keys differ: %q vs %q", ka, kb)
	}
}

func TestIntegrationNotARepo(t *testing.T) {
	needGit(t)
	dir := t.TempDir()
	if got := New().Key(dir); got != "" {
		t.Fatalf("got %q", got)
	}
	if got := New().Key(filepath.Join(dir, "missing")); got != "" {
		t.Fatalf("missing dir: got %q", got)
	}
}

func TestIntegrationEmptyRepo(t *testing.T) {
	needGit(t)
	dir := t.TempDir()
	git(t, dir, "init", "-q")
	if got := New().Key(dir); got != "" {
		t.Fatalf("got %q", got)
	}
	git(t, dir, "remote", "add", "origin", "https://gitlab.com/g/s/n.git")
	if got := New().Key(dir); got != "remote:gitlab.com/g/s/n" {
		t.Fatalf("empty repo with remote: got %q", got)
	}
}
