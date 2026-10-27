package presence

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/fswatch"
)

// eventSource is a running fswatch.Watcher (an interface so tests can feed
// events, overflows and a dying watcher by hand).
type eventSource interface {
	Events() <-chan fswatch.Event
	Close()
}

func watchFS(ctx context.Context, root string, recursive bool) (eventSource, error) {
	w, err := fswatch.Watch(ctx, root, recursive, 0)
	if err != nil {
		return nil, err
	}
	return w, nil
}

// loop is Run's state: the file watchers, the process-exit waits and what
// the next scan has to cover. It lives on Run's goroutine only.
type loop struct {
	x           *watcher
	sessions    eventSource // ClaudeDir/sessions, non-recursive
	projects    eventSource // ClaudeDir/projects, recursive
	unsupported bool        // fswatch has no backend here; don't retry
	exits       map[int]func()
	exited      chan struct{} // a watched session process ended

	full  bool            // next scan rereads every session file
	dirty map[string]bool // session ids whose transcript changed
}

// run scans on events, falling back to Interval while either watcher is
// down, and does a full rescan every SafetyInterval regardless.
func (w *Watcher) run(ctx context.Context) error {
	x := w.impl
	l := &loop{x: x, exits: map[int]func(){}, exited: make(chan struct{}, 1), dirty: map[string]bool{}}
	defer l.close()

	l.startWatchers(ctx)
	x.scan()
	l.trackExits()

	safety := time.NewTimer(l.period())
	defer safety.Stop()
	settle := time.NewTimer(time.Hour)
	settle.Stop()
	defer settle.Stop()
	armed := false
	due := func() {
		if !armed {
			armed = true
			settle.Reset(x.settle)
		}
	}

	for {
		var sessC, projC <-chan fswatch.Event
		if l.sessions != nil {
			sessC = l.sessions.Events()
		}
		if l.projects != nil {
			projC = l.projects.Events()
		}
		select {
		case <-ctx.Done():
			return nil
		case e, ok := <-sessC:
			if !ok {
				l.sessions.Close()
				l.sessions = nil
				l.full = true
				due()
				continue
			}
			// Any session file (not the .key secrets) added, rewritten or
			// removed: reread them all and check liveness now.
			if e.Overflow || isSessionFile(filepath.Base(e.Path)) {
				l.full = true
				due()
			}
		case e, ok := <-projC:
			if !ok {
				l.projects.Close()
				l.projects = nil
				l.full = true
				due()
				continue
			}
			if e.Overflow {
				l.full = true
				due()
			} else if id, ok := transcriptID(e.Path); ok {
				l.dirty[id] = true
				due()
			}
		case <-l.exited:
			l.full = true
			due()
		case <-settle.C:
			armed = false
		case <-safety.C:
			l.startWatchers(ctx)
			l.full = true
		}
		if armed {
			continue // wait for the burst to settle
		}
		switch {
		case l.full:
			x.scan()
			l.trackExits()
			l.full = false
			clear(l.dirty)
			safety.Reset(l.period())
		case len(l.dirty) > 0:
			x.rescanTails(l.dirty)
			clear(l.dirty)
		}
	}
}

// transcriptID is the session id of a top-level transcript path
// (<slug>/<id>.jsonl); subagent transcripts and other files don't map.
func transcriptID(path string) (string, bool) {
	id, ok := strings.CutSuffix(filepath.Base(path), ".jsonl")
	if !ok || !safeSessionID(id) {
		return "", false
	}
	return id, true
}

// period is SafetyInterval while both watchers run, else Interval.
func (l *loop) period() time.Duration {
	if l.sessions != nil && l.projects != nil {
		return l.x.opts.SafetyInterval
	}
	return l.x.opts.Interval
}

// startWatchers starts whichever watcher is not running. A dir that doesn't
// exist yet fails and is retried on the next tick.
func (l *loop) startWatchers(ctx context.Context) {
	if l.unsupported {
		return
	}
	start := func(dir string, recursive bool) eventSource {
		src, err := l.x.watch(ctx, filepath.Join(l.x.opts.ClaudeDir, dir), recursive)
		if errors.Is(err, fswatch.ErrUnsupported) {
			l.unsupported = true
		}
		if err != nil {
			return nil
		}
		return src
	}
	if l.sessions == nil {
		l.sessions = start("sessions", false)
	}
	if l.projects == nil && !l.unsupported {
		l.projects = start("projects", true)
	}
	if l.unsupported {
		l.closeWatchers()
	}
}

// trackExits waits on the process of every live session, so an agent that
// dies without removing its session file leaves at once, not at the next
// safety rescan.
func (l *loop) trackExits() {
	pids := l.x.livePIDs()
	for pid, stop := range l.exits {
		if !pids[pid] {
			stop()
			delete(l.exits, pid)
		}
	}
	for pid := range pids {
		if _, ok := l.exits[pid]; ok {
			continue
		}
		stop, ok := l.x.watchExit(pid, func() {
			select {
			case l.exited <- struct{}{}:
			default:
			}
		})
		if ok {
			l.exits[pid] = stop
		}
	}
}

func (l *loop) closeWatchers() {
	if l.sessions != nil {
		l.sessions.Close()
		l.sessions = nil
	}
	if l.projects != nil {
		l.projects.Close()
		l.projects = nil
	}
}

// close stops every watcher and exit wait and waits for them.
func (l *loop) close() {
	l.closeWatchers()
	for pid, stop := range l.exits {
		stop()
		delete(l.exits, pid)
	}
}
