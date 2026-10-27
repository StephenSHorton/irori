// Package fswatch tells Irori when files under a directory change, so the
// presence scanner, the transcript tailer and the reflog watcher run when
// something happened instead of on a timer.
//
// On Windows one ReadDirectoryChangesW handle watches a whole tree. Elsewhere
// (for now) Watch returns ErrUnsupported and callers keep their interval
// scan. Notifications are a hint, not a record: the OS can drop them (a full
// buffer is reported as Overflow), so callers still do a slow safety rescan
// and treat Overflow as "rescan everything now". An append through a handle
// the writer keeps open is reported only when that handle closes (NTFS
// updates directory entries lazily). Claude Code, git and the session files
// all open, write and close, so their changes arrive at once: checked live,
// every transcript that grew in a 90 s window was reported.
package fswatch

import (
	"context"
	"errors"
	"time"
)

// ErrUnsupported means this OS has no watcher yet.
var ErrUnsupported = errors.New("fswatch: not supported on this OS")

// Event is one change. Path is absolute; Overflow means events were lost
// and Path is the watched root.
type Event struct {
	Path     string
	Overflow bool
}

// Watcher delivers events until its context ends or Close.
type Watcher struct {
	events chan Event
	stop   func()
}

// Events is closed when the watcher stops.
func (w *Watcher) Events() <-chan Event { return w.events }

// Close stops the watcher and waits for it.
func (w *Watcher) Close() { w.stop() }

// Watch starts watching root (a directory), with its whole subtree when
// recursive. Events for the same path within debounce are coalesced into one
// (0 means 50ms). The channel is buffered; a slow reader drops events and
// gets one Overflow instead.
func Watch(ctx context.Context, root string, recursive bool, debounce time.Duration) (*Watcher, error) {
	if debounce <= 0 {
		debounce = 50 * time.Millisecond
	}
	return watch(ctx, root, recursive, debounce)
}
