package fswatch

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"
)

func waitFor(t *testing.T, w *Watcher, path string) {
	t.Helper()
	deadline := time.After(3 * time.Second)
	for {
		select {
		case e, ok := <-w.Events():
			if !ok {
				t.Fatal("watcher closed")
			}
			if e.Path == path || e.Overflow {
				return
			}
		case <-deadline:
			t.Fatalf("no event for %s", path)
		}
	}
}

func TestWatchSeesCreateAppendAndNested(t *testing.T) {
	root := t.TempDir()
	w, err := Watch(context.Background(), root, true, 0)
	if errors.Is(err, ErrUnsupported) {
		t.Skip(runtime.GOOS)
	}
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()

	f := filepath.Join(root, "a.jsonl")
	if err := os.WriteFile(f, []byte("one\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	waitFor(t, w, f)

	// An append that opens and closes the file, the way Claude Code writes
	// its transcripts and git its reflog. (An append through a handle the
	// writer keeps open is not reported until that handle closes: NTFS
	// updates the directory entry lazily. The safety rescan covers that.)
	h, err := os.OpenFile(f, os.O_APPEND|os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := h.WriteString("two\n"); err != nil {
		t.Fatal(err)
	}
	h.Close()
	waitFor(t, w, f)

	sub := filepath.Join(root, "s", "subagents")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	g := filepath.Join(sub, "agent-x.jsonl")
	if err := os.WriteFile(g, []byte("x\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	waitFor(t, w, g)
}

func TestCloseAndContextStop(t *testing.T) {
	root := t.TempDir()
	ctx, cancel := context.WithCancel(context.Background())
	w, err := Watch(ctx, root, false, 0)
	if errors.Is(err, ErrUnsupported) {
		t.Skip(runtime.GOOS)
	}
	if err != nil {
		t.Fatal(err)
	}
	cancel()
	select {
	case _, ok := <-w.Events():
		for ok {
			_, ok = <-w.Events()
		}
	case <-time.After(3 * time.Second):
		t.Fatal("events not closed after cancel")
	}
	w.Close() // idempotent after the context stopped it
}

// A non-recursive watch reports root's direct children and not the files
// below them.
func TestNonRecursiveSkipsSubtree(t *testing.T) {
	root := t.TempDir()
	sub := filepath.Join(root, "sub")
	if err := os.Mkdir(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	w, err := Watch(context.Background(), root, false, 0)
	if errors.Is(err, ErrUnsupported) {
		t.Skip(runtime.GOOS)
	}
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()

	deep := filepath.Join(sub, "deep.txt")
	if err := os.WriteFile(deep, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	top := filepath.Join(root, "top.txt")
	if err := os.WriteFile(top, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	deadline := time.After(3 * time.Second)
	for {
		select {
		case e, ok := <-w.Events():
			if !ok {
				t.Fatal("watcher closed")
			}
			if e.Path == deep {
				t.Fatalf("non-recursive watch reported %s", deep)
			}
			if e.Path == top {
				return
			}
		case <-deadline:
			t.Fatalf("no event for %s", top)
		}
	}
}

// Close leaves no goroutine behind, including after events were delivered
// (on macOS the callbacks arrive on the watcher's own thread).
func TestCloseLeavesNoGoroutines(t *testing.T) {
	root := t.TempDir()
	before := runtime.NumGoroutine()
	w, err := Watch(context.Background(), root, true, 0)
	if errors.Is(err, ErrUnsupported) {
		t.Skip(runtime.GOOS)
	}
	if err != nil {
		t.Fatal(err)
	}
	f := filepath.Join(root, "a.txt")
	if err := os.WriteFile(f, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	waitFor(t, w, f)
	w.Close()
	deadline := time.Now().Add(3 * time.Second)
	for runtime.NumGoroutine() > before {
		if time.Now().After(deadline) {
			t.Fatalf("%d goroutines after Close, %d before", runtime.NumGoroutine(), before)
		}
		time.Sleep(10 * time.Millisecond)
	}
}
