package presence

import (
	"context"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"
)

const (
	defaultInterval = 2 * time.Second
	subBuffer       = 64
)

// watcher is the Watcher's state. Scans are serialized by scanMu; mu guards
// the published cards and the subscriber set.
type watcher struct {
	opts      Options
	tailBytes int64
	// open is how every file's content is read, so tests can prove the
	// <pid>.<hash>.key secrets are never opened.
	open func(name string) (*os.File, error)
	// live decides whether a session's process is still the one that wrote it.
	live func(pid int, procStart uint64) bool

	scanMu sync.Mutex
	tails  map[string]*tailCache // by session id; scan-owned
	repos  map[string]string     // repo key by cwd; scan-owned

	mu    sync.Mutex
	cards map[string]AgentCard // by Key
	subs  map[chan Change]struct{}
}

// tailCache remembers one transcript so its tail is re-read only when it moves.
type tailCache struct {
	path  string
	size  int64
	mtime time.Time
	tail  tail
}

func newWatcher(opts Options) *Watcher {
	if opts.Interval <= 0 {
		opts.Interval = defaultInterval
	}
	if opts.Now == nil {
		opts.Now = time.Now
	}
	return &Watcher{impl: &watcher{
		opts:      opts,
		tailBytes: defaultTailBytes,
		open:      os.Open,
		live:      live,
		tails:     map[string]*tailCache{},
		repos:     map[string]string{},
		cards:     map[string]AgentCard{},
		subs:      map[chan Change]struct{}{},
	}}
}

func (w *Watcher) run(ctx context.Context) error {
	t := time.NewTicker(w.impl.opts.Interval)
	defer t.Stop()
	for {
		w.impl.scan()
		select {
		case <-ctx.Done():
			return nil
		case <-t.C:
		}
	}
}

func (w *Watcher) snapshot() []AgentCard {
	w.impl.mu.Lock()
	out := make([]AgentCard, 0, len(w.impl.cards))
	for _, c := range w.impl.cards {
		out = append(out, c)
	}
	w.impl.mu.Unlock()
	sortCards(out)
	return out
}

func (w *Watcher) subscribe(ctx context.Context) <-chan Change {
	ch := make(chan Change, subBuffer)
	x := w.impl
	x.mu.Lock()
	x.subs[ch] = struct{}{}
	x.mu.Unlock()
	go func() {
		<-ctx.Done()
		x.mu.Lock()
		delete(x.subs, ch)
		close(ch)
		x.mu.Unlock()
	}()
	return ch
}

// scan reads every session file once and publishes what changed.
func (x *watcher) scan() {
	x.scanMu.Lock()
	defer x.scanMu.Unlock()

	next := map[string]AgentCard{}
	seen := map[string]bool{}
	cwds := map[string]bool{}
	dir := filepath.Join(x.opts.ClaudeDir, "sessions")
	entries, _ := os.ReadDir(dir)
	for _, e := range entries {
		if !e.Type().IsRegular() || !isSessionFile(e.Name()) {
			continue
		}
		s, ok := x.readSessionFile(filepath.Join(dir, e.Name()))
		if !ok || !x.live(s.PID, s.ProcStart) {
			continue
		}
		seen[s.ID] = true
		cwds[s.Cwd] = true
		c := x.card(s)
		next[c.Key] = c
	}
	for id := range x.tails {
		if !seen[id] {
			delete(x.tails, id)
		}
	}
	for cwd := range x.repos {
		if !cwds[cwd] {
			delete(x.repos, cwd)
		}
	}
	x.publish(next)
}

func (x *watcher) readSessionFile(path string) (session, bool) {
	f, err := x.open(path)
	if err != nil {
		return session{}, false
	}
	defer f.Close()
	return readSession(f)
}

// card builds the roster entry for a live session, refreshing its tail.
func (x *watcher) card(s session) AgentCard {
	c := AgentCard{
		Key:          x.opts.DeviceID + "/" + s.ID,
		DeviceID:     x.opts.DeviceID,
		Agent:        "claude",
		SessionID:    s.ID,
		PID:          s.PID,
		Cwd:          s.Cwd,
		Status:       s.Status,
		SessionKind:  s.Kind,
		Entrypoint:   s.Entrypoint,
		Version:      s.Version,
		PeerFeatures: append([]string{}, s.PeerFeatures...), // [] not null in JSON
		StartedAt:    s.StartedAt,
		UpdatedAt:    s.UpdatedAt,
		RepoKey:      x.repoKey(s.Cwd),
	}
	if tc := x.refreshTail(s); tc != nil {
		c.Transcript = tc.path
		c.Title = tc.tail.Title
		c.LastText = tc.tail.LastText
		c.LastActivity = tc.tail.LastActivity
		c.PermissionMode = tc.tail.PermissionMode
	}
	c.Name = displayName(s, c.Title)
	return c
}

// repoKey is Options.RepoKey(cwd), computed once per cwd while any live
// session uses it (resolving runs git, which is too slow for every scan).
func (x *watcher) repoKey(cwd string) string {
	if x.opts.RepoKey == nil || cwd == "" {
		return ""
	}
	if k, ok := x.repos[cwd]; ok {
		return k
	}
	k := x.opts.RepoKey(cwd)
	x.repos[cwd] = k
	return k
}

// refreshTail resolves (once) and re-reads a transcript tail when its size or
// mtime changed. nil means no transcript was found.
func (x *watcher) refreshTail(s session) *tailCache {
	tc := x.tails[s.ID]
	var fi os.FileInfo
	if tc != nil {
		var err error
		if fi, err = os.Stat(tc.path); err != nil {
			tc = nil
		}
	}
	if tc == nil {
		p := findTranscript(x.opts.ClaudeDir, s.Cwd, s.ID)
		if p == "" {
			delete(x.tails, s.ID)
			return nil
		}
		var err error
		if fi, err = os.Stat(p); err != nil {
			delete(x.tails, s.ID)
			return nil
		}
		tc = &tailCache{path: p}
		x.tails[s.ID] = tc
	}
	if fi.Size() == tc.size && fi.ModTime().Equal(tc.mtime) {
		return tc
	}
	f, err := x.open(tc.path)
	if err != nil {
		return tc
	}
	defer f.Close()
	if fi, err = f.Stat(); err != nil {
		return tc
	}
	t := readTail(f, fi.Size(), x.tailBytes)
	if fi.Size() >= tc.size {
		// Title rows carry no timestamp and can scroll out of the window.
		if t.Title == "" {
			t.Title = tc.tail.Title
		}
		if t.LastText == "" {
			t.LastText = tc.tail.LastText
		}
		// Mode rows are written only when the mode is set, so they scroll
		// out like titles do.
		if t.PermissionMode == "" {
			t.PermissionMode = tc.tail.PermissionMode
		}
		if t.LastActivity.Before(tc.tail.LastActivity) {
			t.LastActivity = tc.tail.LastActivity
		}
	}
	tc.size, tc.mtime, tc.tail = fi.Size(), fi.ModTime(), t
	return tc
}

// displayName is the session's own name, else its title, else its folder.
func displayName(s session, title string) string {
	if s.Name != "" {
		return s.Name
	}
	if title != "" {
		return title
	}
	// Split on both separators: a Windows cwd may be read on another OS.
	cwd := strings.TrimRight(s.Cwd, `/\`)
	if i := strings.LastIndexAny(cwd, `/\`); i >= 0 {
		cwd = cwd[i+1:]
	}
	if cwd != "" {
		return cwd
	}
	id := s.ID
	if len(id) > 8 {
		id = id[:8]
	}
	return "claude " + id
}

// publish swaps in next and tells subscribers what changed.
func (x *watcher) publish(next map[string]AgentCard) {
	var changes []Change
	x.mu.Lock()
	defer x.mu.Unlock()
	for k, c := range next {
		old, ok := x.cards[k]
		switch {
		case !ok:
			changes = append(changes, Change{Kind: Added, Card: c})
		case changed(old, c):
			changes = append(changes, Change{Kind: Updated, Card: c})
		default:
			// Keep the published card so sub-second activity drift doesn't
			// accumulate unseen.
			next[k] = old
		}
	}
	for k, old := range x.cards {
		if _, ok := next[k]; !ok {
			old.Status = "gone"
			old.UpdatedAt = x.opts.Now()
			changes = append(changes, Change{Kind: Removed, Card: old})
		}
	}
	x.cards = next
	for _, ch := range changes {
		for sub := range x.subs {
			select {
			case sub <- ch:
			default: // slow reader; it re-reads Snapshot
			}
		}
	}
}

// changed is true when b differs from a in anything but a sub-second
// LastActivity move.
func changed(a, b AgentCard) bool {
	if d := b.LastActivity.Sub(a.LastActivity); d >= time.Second || d <= -time.Second {
		return true
	}
	return a.Key != b.Key || a.DeviceID != b.DeviceID || a.Agent != b.Agent ||
		a.SessionID != b.SessionID || a.PID != b.PID || a.Name != b.Name ||
		a.Title != b.Title || a.Cwd != b.Cwd || a.Status != b.Status ||
		a.SessionKind != b.SessionKind || a.Entrypoint != b.Entrypoint ||
		a.Version != b.Version || !slices.Equal(a.PeerFeatures, b.PeerFeatures) ||
		!a.StartedAt.Equal(b.StartedAt) || !a.UpdatedAt.Equal(b.UpdatedAt) ||
		a.LastText != b.LastText || a.Transcript != b.Transcript ||
		a.RepoKey != b.RepoKey || a.PermissionMode != b.PermissionMode
}

// sortCards orders by LastActivity (UpdatedAt when there is none), newest
// first, then by Key.
func sortCards(cards []AgentCard) {
	at := func(c AgentCard) time.Time {
		if c.LastActivity.IsZero() {
			return c.UpdatedAt
		}
		return c.LastActivity
	}
	slices.SortFunc(cards, func(a, b AgentCard) int {
		if c := at(b).Compare(at(a)); c != 0 {
			return c
		}
		return strings.Compare(a.Key, b.Key)
	})
}
