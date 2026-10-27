package capture

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/fswatch"
)

// busyRecheck is how soon an open turn that was due to close but whose live
// card was busy is looked at again. Busy to idle is a roster change, not a
// file change, so nothing else would wake the check.
const busyRecheck = 15 * time.Second

// eventBatch bounds the events handled together after one wake-up.
const eventBatch = 512

// watchEvent is one watcher event, tagged with its source: logs is the
// normRoot of a reflog directory, "" for the transcripts tree. closed means
// the watcher stopped (its directory went away, or the read failed).
type watchEvent struct {
	logs   string
	w      *fswatch.Watcher
	ev     fswatch.Event
	closed bool
}

// logsWatch watches one gitdir's logs directory for the roots sharing it.
type logsWatch struct {
	dir   string
	w     *fswatch.Watcher
	roots []repoSeen
}

// loop is runEvents' state; everything belongs to the run goroutine.
type loop struct {
	c        *capture
	ctx      context.Context
	events   chan watchEvent
	wg       sync.WaitGroup
	projects string           // ClaudeDir/projects, as listFiles spells it
	tail     *fswatch.Watcher // nil while projects can't be watched
	logs     map[string]*logsWatch
	polled   []repoSeen      // roots whose logs directory can't be watched (none yet: no commits); read every RepoInterval
	gen      int             // c.rootsGen the logs watchers were last synced with; -1 forces
	pending  map[string]bool // normRoot -> its reflog is behind; true: read again at once, false: after RepoInterval
}

// runEvents is run on a file watcher: transcripts are tailed when they
// change, reflogs read when logs/HEAD changes, open turns closed by a timer
// armed for the next quiet deadline, and everything rescanned every
// SafetyInterval. It returns fswatch.ErrUnsupported (having done nothing)
// where no watcher exists.
func (c *capture) runEvents(parent context.Context) error {
	ctx, cancel := context.WithCancel(parent)
	l := &loop{
		c: c, ctx: ctx, events: make(chan watchEvent, 64),
		projects: filepath.Join(c.opts.ClaudeDir, "projects"),
		logs:     map[string]*logsWatch{}, pending: map[string]bool{}, gen: -1,
	}
	defer func() {
		cancel()
		if l.tail != nil {
			l.tail.Close()
		}
		for _, lw := range l.logs {
			lw.w.Close()
		}
		l.wg.Wait()
	}()
	// Watch before the first pass so nothing written during it is missed.
	if err := l.watchProjects(); errors.Is(err, fswatch.ErrUnsupported) {
		return err
	}
	c.tailPass(ctx)
	c.repoPass(ctx)
	l.syncLogs(true)

	safety := time.NewTicker(c.opts.SafetyInterval)
	defer safety.Stop()
	quiet := newIdleTimer()
	defer quiet.Stop()
	follow := newIdleTimer()
	defer follow.Stop()
	pollTail := newIdleTimer()
	defer pollTail.Stop()
	pollRepo := newIdleTimer()
	defer pollRepo.Stop()
	followAt := time.Time{}

	for {
		if l.tail == nil && !pollTail.armed {
			pollTail.reset(c.opts.ScanInterval)
		}
		if len(l.polled) > 0 && !pollRepo.armed {
			pollRepo.reset(c.opts.RepoInterval)
		}
		if at, ok := c.nextQuiet(); ok {
			quiet.reset(at.Sub(c.now()))
		} else {
			quiet.stop()
		}
		select {
		case <-ctx.Done():
			return nil
		case e := <-l.events:
			batch := []watchEvent{e}
		drain:
			for len(batch) < eventBatch {
				select {
				case e := <-l.events:
					batch = append(batch, e)
				default:
					break drain
				}
			}
			l.handle(batch)
		case <-safety.C:
			c.tailPass(ctx)
			c.repoPass(ctx)
			l.syncLogs(true)
		case <-pollTail.C():
			pollTail.armed = false
			l.watchProjects()
			c.tailPass(ctx)
		case <-pollRepo.C():
			pollRepo.armed = false
			l.pollLogs()
		case <-quiet.C():
			quiet.armed = false
			c.quietPass(ctx)
		case <-follow.C():
			follow.armed, followAt = false, time.Time{}
			roots := l.takePending()
			l.readRoots(roots)
		}
		l.syncLogs(false)
		if ctx.Err() != nil {
			return nil
		}
		// Reflogs left behind: at once when the budget ran out, after
		// RepoInterval when a line failed (it is retried).
		if at, ok := l.followUp(); ok && (followAt.IsZero() || at.Before(followAt)) {
			followAt = at
			follow.reset(time.Until(at))
		}
	}
}

// watchProjects starts the transcripts watcher if it isn't running.
func (l *loop) watchProjects() error {
	if l.tail != nil {
		return nil
	}
	w, err := fswatch.Watch(l.ctx, l.projects, true, 0)
	if err != nil {
		return err
	}
	l.tail = w
	l.forward("", w)
	return nil
}

// forward copies a watcher's events onto the loop's channel, then reports
// it closed.
func (l *loop) forward(logs string, w *fswatch.Watcher) {
	l.wg.Add(1)
	go func() {
		defer l.wg.Done()
		send := func(e watchEvent) bool {
			select {
			case l.events <- e:
				return true
			case <-l.ctx.Done():
				return false
			}
		}
		for ev := range w.Events() {
			if !send(watchEvent{logs: logs, w: w, ev: ev}) {
				return
			}
		}
		send(watchEvent{logs: logs, w: w, closed: true})
	}()
}

// handle acts on one batch of events.
func (l *loop) handle(batch []watchEvent) {
	c := l.c
	tailAll, repoAll := false, false
	var paths []string
	seenPath := map[string]bool{}
	logsHit := map[string]bool{}
	for _, e := range batch {
		switch {
		case e.logs == "" && e.w != l.tail:
			// A stopped watcher's leftovers.
		case e.logs == "" && e.closed:
			l.tail = nil
			tailAll = true
		case e.logs == "" && e.ev.Overflow:
			tailAll = true
		case e.logs == "":
			if p, ok := l.projectsPath(e.ev.Path); ok && !seenPath[p] {
				seenPath[p] = true
				paths = append(paths, p)
			}
		case l.logs[e.logs] == nil || l.logs[e.logs].w != e.w:
		case e.closed:
			// Polled (and watched again) at RepoInterval, so a watch that
			// keeps failing can't spin.
			l.polled = append(l.polled, l.logs[e.logs].roots...)
			delete(l.logs, e.logs)
			repoAll = true
		case e.ev.Overflow:
			repoAll = true
		case strings.EqualFold(filepath.Base(e.ev.Path), "HEAD"):
			logsHit[e.logs] = true
		}
	}
	switch {
	case tailAll:
		c.tailPass(l.ctx)
	case len(paths) > 0:
		c.tailPaths(l.ctx, paths)
		// A new session's root is watched as soon as its transcript moves,
		// not at the next safety pass.
		c.noteLiveRoots(l.ctx)
	}
	switch {
	case repoAll:
		c.repoPass(l.ctx)
	case len(logsHit) > 0:
		var roots []repoSeen
		for k := range logsHit {
			roots = append(roots, l.logs[k].roots...)
		}
		l.readRoots(roots)
	}
}

// projectsPath turns a watcher path into listFiles' spelling of it, so a
// file is one tfile whichever way it was found. ok is false for anything
// that isn't a .jsonl.
func (l *loop) projectsPath(p string) (string, bool) {
	if !strings.HasSuffix(strings.ToLower(p), ".jsonl") {
		return "", false
	}
	abs, err := filepath.Abs(l.projects)
	if err != nil {
		return "", false
	}
	rel, err := filepath.Rel(abs, p)
	if err != nil || rel == "." || strings.HasPrefix(rel, "..") {
		return "", false
	}
	return filepath.Join(l.projects, rel), true
}

// readRoots reads the reflogs of roots (event or follow-up driven), each
// with its own repo budget, and queues those still behind.
func (l *loop) readRoots(roots []repoSeen) {
	c := l.c
	if len(roots) == 0 {
		return
	}
	sort.Slice(roots, func(i, j int) bool { return roots[i].Root < roots[j].Root })
	for _, r := range roots {
		if l.ctx.Err() != nil {
			return
		}
		n := normRoot(r.Root)
		before := c.reflogOffset(n)
		c.repoDeadline = time.Now().Add(repoBudget)
		c.watchRoot(l.ctx, r)
		if !c.reflogBehind(r) {
			delete(l.pending, n)
			continue
		}
		// Cut short by the budget or the read cap: resume at once. A pass
		// that moved nothing (a failed line, an unreadable log) waits.
		l.pending[n] = l.pending[n] || time.Now().After(c.repoDeadline) || c.reflogOffset(n) > before
	}
	c.saveCursors()
}

// takePending returns and clears the roots queued for another read.
func (l *loop) takePending() []repoSeen {
	c := l.c
	var out []repoSeen
	c.mu.RLock()
	for n := range l.pending {
		if r, ok := c.repoRoots[n]; ok {
			out = append(out, r)
		}
	}
	c.mu.RUnlock()
	clear(l.pending)
	return out
}

// followUp is when the queued roots are read again: now when one made
// progress last time, else RepoInterval from now.
func (l *loop) followUp() (time.Time, bool) {
	if len(l.pending) == 0 {
		return time.Time{}, false
	}
	for _, now := range l.pending {
		if now {
			return time.Now(), true
		}
	}
	return time.Now().Add(l.c.opts.RepoInterval), true
}

// reflogOffset is how far a root's reflog has been read (-1: not yet).
func (c *capture) reflogOffset(n string) int64 {
	c.mu.RLock()
	defer c.mu.RUnlock()
	if st := c.reflogs[n]; st != nil {
		return st.offset
	}
	return -1
}

// reflogBehind reports a root whose logs/HEAD holds lines not read yet.
func (c *capture) reflogBehind(r repoSeen) bool {
	c.mu.RLock()
	st := c.reflogs[normRoot(r.Root)]
	c.mu.RUnlock()
	if st == nil {
		return false
	}
	fi, err := os.Stat(st.logPath)
	return err == nil && fi.Size() > st.offset
}

// syncLogs makes the reflog watchers match the known roots: one watcher per
// logs directory that exists, closed when no root uses it any more (a
// removed worktree). Without force it only runs when roots were added. A
// new watcher's roots are read at once, so a movement made before the
// watch began isn't left for the safety pass.
func (l *loop) syncLogs(force bool) {
	c := l.c
	c.mu.RLock()
	gen := c.rootsGen
	if !force && gen == l.gen {
		c.mu.RUnlock()
		return
	}
	roots := make([]repoSeen, 0, len(c.repoRoots))
	for _, r := range c.repoRoots {
		roots = append(roots, r)
	}
	c.mu.RUnlock()
	l.gen = gen
	want := map[string]*logsWatch{}
	for _, r := range roots {
		gd, err := gitlink.GitDir(r.Root)
		if err != nil {
			continue // a removed worktree
		}
		dir := filepath.Join(gd, "logs")
		k := normRoot(dir)
		if want[k] == nil {
			want[k] = &logsWatch{dir: dir}
		}
		want[k].roots = append(want[k].roots, r)
	}
	for k, lw := range l.logs {
		if want[k] == nil {
			lw.w.Close()
			delete(l.logs, k)
		}
	}
	l.polled = l.polled[:0]
	var fresh []repoSeen
	for k, wl := range want {
		if lw := l.logs[k]; lw != nil {
			lw.roots = wl.roots
			continue
		}
		w, err := fswatch.Watch(l.ctx, wl.dir, false, 0)
		if err != nil {
			l.polled = append(l.polled, wl.roots...)
			continue
		}
		wl.w = w
		l.logs[k] = wl
		l.forward(k, w)
		fresh = append(fresh, wl.roots...)
	}
	l.readRoots(fresh)
}

// pollLogs reads the roots whose logs directory couldn't be watched and
// tries to watch it again.
func (l *loop) pollLogs() {
	l.readRoots(slices.Clone(l.polled))
	l.syncLogs(true)
}

// nextQuiet is the earliest moment an open turn may close as quiet.
func (c *capture) nextQuiet() (time.Time, bool) {
	var best time.Time
	for _, f := range c.files {
		if f.parser.Open() == nil {
			continue
		}
		if at := c.quietDue(f); best.IsZero() || at.Before(best) {
			best = at
		}
	}
	return best, !best.IsZero()
}

func (c *capture) quietDue(f *tfile) time.Time {
	at := f.lastGrow.Add(c.opts.QuietAfter)
	if f.quietAt.After(at) {
		at = f.quietAt
	}
	return at
}

// quietPass gives every open turn that is due a tail, which closes it
// unless its live card is busy; one that stays open is looked at again
// after busyRecheck.
func (c *capture) quietPass(ctx context.Context) {
	now := c.now()
	var due []*tfile
	for _, f := range c.files {
		if f.parser.Open() != nil && !c.quietDue(f).After(now) {
			due = append(due, f)
		}
	}
	if len(due) == 0 {
		return
	}
	live := c.liveCards()
	for _, f := range due {
		if ctx.Err() != nil {
			return
		}
		fd := found{path: f.path, src: f.src, size: f.size, mod: f.lastGrow}
		if fi, err := os.Stat(f.path); err == nil {
			fd.size, fd.mod = fi.Size(), fi.ModTime()
		}
		card, isLive := live[f.src.SessionID]
		c.tailFile(ctx, fd, isLive, card)
		if f.parser.Open() != nil && !c.quietDue(f).After(c.now()) {
			f.quietAt = c.now().Add(busyRecheck)
		}
	}
	c.endTail(ctx, false)
}

// idleTimer is a stopped timer that is armed only when something is due.
type idleTimer struct {
	t     *time.Timer
	armed bool
}

func newIdleTimer() *idleTimer {
	t := time.NewTimer(time.Hour)
	t.Stop()
	return &idleTimer{t: t}
}

func (it *idleTimer) C() <-chan time.Time { return it.t.C }

func (it *idleTimer) reset(d time.Duration) {
	it.t.Reset(max(d, 0))
	it.armed = true
}

func (it *idleTimer) stop() {
	it.t.Stop()
	it.armed = false
}

func (it *idleTimer) Stop() { it.stop() }
