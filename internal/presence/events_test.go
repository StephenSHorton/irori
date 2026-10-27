package presence

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/fswatch"
)

// fakeSource is an eventSource the test drives by hand.
type fakeSource struct {
	ch     chan fswatch.Event
	closed atomic.Bool
}

func newFakeSource() *fakeSource { return &fakeSource{ch: make(chan fswatch.Event, 16)} }

func (f *fakeSource) Events() <-chan fswatch.Event { return f.ch }
func (f *fakeSource) Close()                       { f.closed.Store(true) }

// fakeWatch hands out sources by dir name ("sessions", "projects") and
// counts the calls.
type fakeWatch struct {
	mu    sync.Mutex
	srcs  map[string]*fakeSource
	calls map[string]int
	err   error
}

func newFakeWatch() *fakeWatch {
	return &fakeWatch{
		srcs:  map[string]*fakeSource{"sessions": newFakeSource(), "projects": newFakeSource()},
		calls: map[string]int{},
	}
}

func (f *fakeWatch) watch(_ context.Context, root string, _ bool) (eventSource, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	name := filepath.Base(root)
	f.calls[name]++
	if f.err != nil {
		return nil, f.err
	}
	return f.srcs[name], nil
}

func (f *fakeWatch) count(name string) int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.calls[name]
}

// eventWatcher is a testWatcher whose only clocks are the given intervals;
// fw nil means the real fswatch.
func eventWatcher(t *testing.T, dir string, interval, safety time.Duration, fw *fakeWatch) (*Watcher, *opens) {
	t.Helper()
	w, o := testWatcher(dir, true)
	w.impl.opts.Interval = interval
	w.impl.opts.SafetyInterval = safety
	w.impl.settle = 10 * time.Millisecond
	w.impl.watchExit = func(int, func()) (func(), bool) { return nil, false }
	if fw != nil {
		w.impl.watch = fw.watch
	}
	return w, o
}

// runWatcher runs w until the test ends and checks Run returns.
func runWatcher(t *testing.T, w *Watcher) (<-chan Change, context.CancelFunc) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	ch := w.Subscribe(ctx)
	done := make(chan error, 1)
	go func() { done <- w.Run(ctx) }()
	t.Cleanup(func() {
		cancel()
		select {
		case err := <-done:
			if err != nil {
				t.Error(err)
			}
		case <-time.After(5 * time.Second):
			t.Error("Run did not return after cancel")
		}
	})
	return ch, cancel
}

// await returns the first change matching ok within 2s.
func await(t *testing.T, ch <-chan Change, what string, ok func(Change) bool) Change {
	t.Helper()
	deadline := time.After(2 * time.Second)
	for {
		select {
		case c := <-ch:
			if ok(c) {
				return c
			}
		case <-deadline:
			t.Fatalf("no %s within 2s", what)
			return Change{}
		}
	}
}

func skipWithoutFSWatch(t *testing.T) {
	t.Helper()
	w, err := fswatch.Watch(context.Background(), t.TempDir(), false, 0)
	if errors.Is(err, fswatch.ErrUnsupported) {
		t.Skip("fswatch unsupported on " + runtime.GOOS)
	}
	if err != nil {
		t.Fatal(err)
	}
	w.Close()
}

// With both intervals at an hour, only file events can move the roster.
func TestEventPathReactsWithoutRescan(t *testing.T) {
	skipWithoutFSWatch(t)
	dir := t.TempDir()
	cwd := `C:\work\events`
	idA := "0b1c2d3e-0000-4000-8000-0000000000e1"
	idB := "0b1c2d3e-0000-4000-8000-0000000000e2"
	tp := filepath.Join(dir, "projects", Slug(cwd), idA+".jsonl")
	if err := os.MkdirAll(filepath.Dir(tp), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "sessions"), 0o700); err != nil {
		t.Fatal(err)
	}
	// Run starts its watchers before the first scan, so once this session
	// is Added every later write is reported.
	writeSession(t, dir, map[string]any{"pid": 40, "sessionId": "sentinel"})
	w, o := eventWatcher(t, dir, time.Hour, time.Hour, nil)
	ch, _ := runWatcher(t, w)
	await(t, ch, "Added sentinel", func(c Change) bool { return c.Kind == Added && c.Card.SessionID == "sentinel" })

	// A session appears, transcript already there.
	appendFile(t, tp, assistantRow("2026-10-26T10:00:00Z", "one"))
	writeSession(t, dir, map[string]any{"pid": 41, "sessionId": idA, "cwd": cwd, "status": "busy"})
	await(t, ch, "Added A", func(c Change) bool {
		return c.Kind == Added && c.Card.SessionID == idA && c.Card.LastText == "one"
	})

	// The transcript grows: only A's tail is reread, no session file.
	jsons := o.count(".json")
	appendFile(t, tp, assistantRow("2026-10-26T10:00:05Z", "two"))
	await(t, ch, "Updated A", func(c Change) bool {
		return c.Kind == Updated && c.Card.SessionID == idA && c.Card.LastText == "two"
	})
	if n := o.count(".json"); n != jsons {
		t.Errorf("a transcript change reread session files (%d -> %d opens)", jsons, n)
	}

	// A session whose transcript shows up after its session file.
	pB := writeSession(t, dir, map[string]any{"pid": 42, "sessionId": idB, "cwd": cwd})
	await(t, ch, "Added B", func(c Change) bool {
		return c.Kind == Added && c.Card.SessionID == idB && c.Card.Transcript == ""
	})
	appendFile(t, filepath.Join(dir, "projects", Slug(cwd), idB+".jsonl"), assistantRow("2026-10-26T10:01:00Z", "late"))
	await(t, ch, "Updated B transcript", func(c Change) bool {
		return c.Kind == Updated && c.Card.SessionID == idB && c.Card.LastText == "late" && c.Card.Transcript != ""
	})

	if err := os.Remove(pB); err != nil {
		t.Fatal(err)
	}
	await(t, ch, "Removed B", func(c Change) bool {
		return c.Kind == Removed && c.Card.SessionID == idB && c.Card.Status == "gone"
	})
}

// Overflow means events were lost: everything is rescanned at once.
func TestOverflowForcesFullScan(t *testing.T) {
	for _, root := range []string{"sessions", "projects"} {
		t.Run(root, func(t *testing.T) {
			dir := t.TempDir()
			fw := newFakeWatch()
			w, _ := eventWatcher(t, dir, time.Hour, time.Hour, fw)
			writeSession(t, dir, map[string]any{"pid": 41, "sessionId": "first"})
			ch, _ := runWatcher(t, w)
			await(t, ch, "Added first", func(c Change) bool { return c.Kind == Added && c.Card.SessionID == "first" })

			writeSession(t, dir, map[string]any{"pid": 42, "sessionId": "second"})
			fw.srcs[root].ch <- fswatch.Event{Path: filepath.Join(dir, root), Overflow: true}
			await(t, ch, "Added second", func(c Change) bool { return c.Kind == Added && c.Card.SessionID == "second" })
		})
	}
}

// Events for files that aren't session files (the .key secrets, temp files)
// don't trigger a scan.
func TestKeyFileEventIgnored(t *testing.T) {
	dir := t.TempDir()
	fw := newFakeWatch()
	w, o := eventWatcher(t, dir, time.Hour, time.Hour, fw)
	writeSession(t, dir, map[string]any{"pid": 41, "sessionId": "only"})
	ch, _ := runWatcher(t, w)
	await(t, ch, "Added", func(c Change) bool { return c.Kind == Added })
	jsons := o.count(".json")
	fw.srcs["sessions"].ch <- fswatch.Event{Path: filepath.Join(dir, "sessions", "41.abcdef.key")}
	fw.srcs["projects"].ch <- fswatch.Event{Path: filepath.Join(dir, "projects", "x", "only", "subagents", "agent-1.jsonl")}
	time.Sleep(100 * time.Millisecond)
	if n := o.count(".json"); n != jsons {
		t.Errorf("non-session events rescanned (%d -> %d opens)", jsons, n)
	}
}

// A watcher that dies hands the roster back to Interval, and Run keeps
// trying to start it again.
func TestDeadWatcherFallsBackToInterval(t *testing.T) {
	dir := t.TempDir()
	fw := newFakeWatch()
	w, _ := eventWatcher(t, dir, 20*time.Millisecond, time.Hour, fw)
	writeSession(t, dir, map[string]any{"pid": 41, "sessionId": "first"})
	ch, _ := runWatcher(t, w)
	await(t, ch, "Added first", func(c Change) bool { return c.Kind == Added })

	fw.mu.Lock()
	dead := fw.srcs["sessions"]
	fw.err = errors.New("dir gone")
	fw.mu.Unlock()
	close(dead.ch)

	writeSession(t, dir, map[string]any{"pid": 42, "sessionId": "second"})
	await(t, ch, "Added second by interval", func(c Change) bool { return c.Kind == Added && c.Card.SessionID == "second" })
	if !dead.closed.Load() {
		t.Error("dead watcher not closed")
	}
	deadline := time.Now().Add(2 * time.Second)
	for fw.count("sessions") < 3 && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
	}
	if n := fw.count("sessions"); n < 3 {
		t.Errorf("watcher restarted %d times, want retries", n-1)
	}
}

// Without fswatch the roster polls every Interval and never asks again.
func TestUnsupportedKeepsInterval(t *testing.T) {
	dir := t.TempDir()
	fw := newFakeWatch()
	fw.err = fswatch.ErrUnsupported
	w, _ := eventWatcher(t, dir, 10*time.Millisecond, time.Hour, fw)
	ch, _ := runWatcher(t, w)
	time.Sleep(30 * time.Millisecond)
	writeSession(t, dir, map[string]any{"pid": 41, "sessionId": "polled"})
	await(t, ch, "Added", func(c Change) bool { return c.Kind == Added })
	if s, p := fw.count("sessions"), fw.count("projects"); s != 1 || p != 0 {
		t.Errorf("watch called sessions=%d projects=%d, want 1 and 0", s, p)
	}
}

// A process that dies without touching its session file is caught by the
// safety rescan.
func TestLivenessOnSafetyTimer(t *testing.T) {
	dir := t.TempDir()
	fw := newFakeWatch()
	w, _ := eventWatcher(t, dir, time.Hour, 50*time.Millisecond, fw)
	var alive atomic.Bool
	alive.Store(true)
	w.impl.live = func(int, uint64) bool { return alive.Load() }
	writeSession(t, dir, map[string]any{"pid": 41, "sessionId": "dies"})
	ch, _ := runWatcher(t, w)
	await(t, ch, "Added", func(c Change) bool { return c.Kind == Added })
	alive.Store(false)
	await(t, ch, "Removed", func(c Change) bool { return c.Kind == Removed && c.Card.SessionID == "dies" })
}

// Run leaves no goroutine behind: file watchers and exit waits all stop.
func TestRunStopsEverything(t *testing.T) {
	dir := t.TempDir()
	for _, d := range []string{"sessions", "projects"} {
		if err := os.MkdirAll(filepath.Join(dir, d), 0o700); err != nil {
			t.Fatal(err)
		}
	}
	writeSession(t, dir, map[string]any{"pid": os.Getpid(), "sessionId": "me", "cwd": dir})
	base := runtime.NumGoroutine()
	w, _ := testWatcher(dir, false)
	ctx, cancel := context.WithCancel(context.Background())
	ch := w.Subscribe(ctx)
	done := make(chan error, 1)
	go func() { done <- w.Run(ctx) }()
	await(t, ch, "Added", func(c Change) bool { return c.Kind == Added })
	cancel()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for runtime.NumGoroutine() > base && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
	}
	if n := runtime.NumGoroutine(); n > base {
		t.Errorf("%d goroutines after Run, %d before", n, base)
	}
}
