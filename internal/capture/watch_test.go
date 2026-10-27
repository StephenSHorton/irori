package capture

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/fswatch"
)

// These drive Run with every interval set to an hour, so anything seen
// within a few seconds came through a file-change event.

const eventWait = 5 * time.Second

func needWatcher(t *testing.T) {
	t.Helper()
	w, err := fswatch.Watch(context.Background(), t.TempDir(), false, 0)
	if errors.Is(err, fswatch.ErrUnsupported) {
		t.Skip("no file watcher on " + runtime.GOOS)
	}
	if err != nil {
		t.Fatal(err)
	}
	w.Close()
}

// eventRig is a rig whose polling and safety passes never come in a test.
func eventRig(t *testing.T, mod func(*Options)) *rig {
	t.Helper()
	needWatcher(t)
	r := newRig(t, func(o *Options) {
		o.ScanInterval, o.RepoInterval, o.SafetyInterval = time.Hour, time.Hour, time.Hour
		if mod != nil {
			mod(o)
		}
	})
	if err := os.MkdirAll(filepath.Join(r.claude, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	return r
}

// start runs the service; the returned stop cancels it and fails the test
// unless Run returns promptly.
func (r *rig) start() (stop func()) {
	r.t.Helper()
	ctx, cancel := context.WithCancel(r.ctx)
	done := make(chan error, 1)
	go func() { done <- r.c.Run(ctx) }()
	stopped := false
	stop = func() {
		r.t.Helper()
		if stopped {
			return
		}
		stopped = true
		cancel()
		select {
		case err := <-done:
			if err != nil {
				r.t.Fatal(err)
			}
		case <-time.After(eventWait):
			r.t.Fatal("Run didn't return after cancel")
		}
	}
	r.t.Cleanup(stop)
	return stop
}

// waitFor polls cond (a cheap read) until it holds.
func (r *rig) waitFor(what string, cond func() bool) {
	r.t.Helper()
	deadline := time.Now().Add(eventWait)
	for !cond() {
		if time.Now().After(deadline) {
			r.t.Fatalf("not seen within %v: %s", eventWait, what)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

func (r *rig) openTurns() int {
	r.c.impl.mu.RLock()
	defer r.c.impl.mu.RUnlock()
	return len(r.c.impl.open)
}

// waitInitial waits for Run's first pass, which precedes event handling.
func (r *rig) waitInitial() {
	r.t.Helper()
	r.waitFor("first pass", func() bool { return !r.c.Status().Backfilling })
}

func TestEventTailNewAndAppended(t *testing.T) {
	r := eventRig(t, nil)
	cwd := canon(t, t.TempDir())
	before := runtime.NumGoroutine()
	stop := r.start()
	r.waitInitial()

	w := r.session(sidA, cwd)
	now := time.Now()
	w.write(w.prompt("p1", "one", now), w.done(now)) // a new file: its create event
	r.waitFor("first turn", func() bool { return len(r.rows(KindTurn)) == 1 })
	w.write(w.prompt("p2", "two", now.Add(time.Second)), w.done(now.Add(time.Second))) // an append
	r.waitFor("second turn", func() bool { return len(r.rows(KindTurn)) == 2 })
	sub := r.subagent(sidA, cwd, "a1")
	sub.write(sub.prompt("p3", "sub", now), sub.done(now)) // a new nested directory
	r.waitFor("subagent turn", func() bool { return len(r.rows(KindTurn)) == 3 })

	stop()
	// Every watcher and forwarder has stopped.
	r.waitFor("goroutines back", func() bool { return runtime.NumGoroutine() <= before })
}

func TestEventQuietCloseTimer(t *testing.T) {
	r := eventRig(t, func(o *Options) { o.QuietAfter = 300 * time.Millisecond })
	r.start()
	r.waitInitial()
	w := r.session(sidA, canon(t, t.TempDir()))
	now := time.Now()
	w.write(w.prompt("p1", "hi", now), w.reply("hello", now)) // no turn_duration: stays open
	r.waitFor("open turn", func() bool { return r.openTurns() == 1 })
	r.waitFor("quiet close", func() bool { return len(r.rows(KindTurn)) == 1 })
}

func TestEventQuietBusyRecheck(t *testing.T) {
	r := eventRig(t, func(o *Options) { o.QuietAfter = 100 * time.Millisecond })
	cwd := canon(t, t.TempDir())
	r.roster.set(liveCard(sidA, cwd, "busy"))
	r.start()
	r.waitInitial()
	w := r.session(sidA, cwd)
	now := time.Now()
	w.write(w.prompt("p1", "hi", now), w.reply("hello", now))
	r.waitFor("open turn", func() bool { return r.openTurns() == 1 })
	time.Sleep(500 * time.Millisecond)
	if n := len(r.rows(KindTurn)); n != 0 {
		t.Fatalf("a busy session's turn closed as quiet (%d rows)", n)
	}
}

func TestEventReflog(t *testing.T) {
	r := eventRig(t, nil)
	known := newRepo(t, "false")
	r.roster.set(liveCard(sidA, known, "idle"))
	r.start()
	r.waitInitial()
	r.waitFor("root watched", func() bool { return r.c.Status().Repos == 1 })

	writeFile(t, filepath.Join(known, "a.txt"), "a\n")
	gitCmd(t, known, "add", "a.txt")
	gitCmd(t, known, "commit", "-q", "-m", "a")
	sha := head(t, known)
	r.waitFor("commit in a root known at start", func() bool { _, ok := r.c.Commit(sha); return ok })

	// A root first seen while running: a new live session's transcript moves.
	later := newRepo(t, "false")
	const sidB = "bbbbbbbb-1111-2222-3333-444444444444"
	r.roster.set(liveCard(sidA, known, "idle"), liveCard(sidB, later, "idle"))
	w := r.session(sidB, later)
	now := time.Now()
	w.write(w.prompt("p1", "hi", now), w.done(now))
	r.waitFor("new root watched", func() bool { return r.c.Status().Repos == 2 })
	writeFile(t, filepath.Join(later, "b.txt"), "b\n")
	gitCmd(t, later, "add", "b.txt")
	gitCmd(t, later, "commit", "-q", "-m", "b")
	sha = head(t, later)
	r.waitFor("commit in a root seen while running", func() bool { _, ok := r.c.Commit(sha); return ok })
}

// A watched worktree's logs directory must not stop git removing it.
func TestEventWorktreeRemovable(t *testing.T) {
	r := eventRig(t, nil)
	main := newRepo(t, "false")
	wt := filepath.Join(canon(t, t.TempDir()), "wt")
	gitCmd(t, main, "worktree", "add", "-q", wt)
	r.roster.set(liveCard(sidA, wt, "idle"))
	r.start()
	r.waitInitial()
	r.waitFor("worktree watched", func() bool { return r.c.Status().Repos == 1 })
	// Only an event brings this commit in, so the logs handle is open.
	writeFile(t, filepath.Join(wt, "w.txt"), "w\n")
	gitCmd(t, wt, "add", "w.txt")
	gitCmd(t, wt, "commit", "-q", "-m", "w")
	sha := head(t, wt)
	r.waitFor("worktree commit", func() bool { _, ok := r.c.Commit(sha); return ok })
	gitCmd(t, main, "worktree", "remove", "--force", wt)
	if _, err := os.Stat(filepath.Join(main, ".git", "worktrees", "wt")); !os.IsNotExist(err) {
		t.Fatalf("worktree gitdir still there: %v", err)
	}
}

// A reflog pass that moved nothing (here a failing git) is retried after
// RepoInterval, not in a loop; one cut by the budget resumes at once.
func TestEventReflogFollowUp(t *testing.T) {
	g := &flakyGit{sub: "rev-list", left: 1 << 20}
	r := newRig(t, func(o *Options) { o.Git = g; o.RepoInterval = time.Hour })
	dir := newRepo(t, "false")
	r.roster.set(liveCard(sidA, dir, "idle"))
	r.repos()
	writeFile(t, filepath.Join(dir, "a.txt"), "a\n")
	gitCmd(t, dir, "add", "a.txt")
	gitCmd(t, dir, "commit", "-q", "-m", "a")
	root := r.c.impl.repoRoots[normRoot(dir)]
	l := &loop{c: r.c.impl, ctx: r.ctx, pending: map[string]bool{}}

	l.readRoots([]repoSeen{root})
	if at, ok := l.followUp(); !ok || time.Until(at) < 59*time.Minute {
		t.Fatalf("failed line: follow-up at %v (%v), want after RepoInterval", at, ok)
	}

	g.mu.Lock()
	g.left = 0
	g.mu.Unlock()
	old := repoBudget
	repoBudget = -time.Second
	defer func() { repoBudget = old }()
	gitCmd(t, dir, "commit", "-q", "--allow-empty", "-m", "b")
	l.readRoots([]repoSeen{root})
	if at, ok := l.followUp(); !ok || time.Until(at) > time.Second {
		t.Fatalf("over budget: follow-up at %v (%v), want now", at, ok)
	}
	repoBudget = old
	for range 5 {
		l.readRoots(l.takePending())
	}
	if _, ok := l.followUp(); ok {
		t.Fatalf("still pending: %v", l.pending)
	}
	if _, ok := r.c.Commit(head(t, dir)); !ok {
		t.Fatal("commit b not imported")
	}
}
