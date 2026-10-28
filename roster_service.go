package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/paths"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventRosterChanged carries a RosterChange whenever an agent appears, changes or leaves.
const EventRosterChanged = "roster:changed"

// RosterChange is the roster:changed payload: the change plus the full roster
// after it, so the UI never has to merge.
type RosterChange struct {
	Change presence.Change      `json:"change"`
	Agents []presence.AgentCard `json:"agents"`
}

// RosterService is the UI's view of who is running. JS binding name: RosterService.*
type RosterService struct {
	core *core.Core

	cards   cardIndex   // agent.seen cards on this device, for Gone and Revive
	revives reviveGuard // sessions revived a moment ago
}

// Agents returns the live agents on every one of the person's devices:
// this device's first, then the others' with their device state (v0.4 §6.2).
func (r *RosterService) Agents() []presence.AgentCard { return r.core.MeshAgents() }

// Device returns this device's identity.
func (r *RosterService) Device() device.Identity { return r.core.Device() }

// Version returns the running Irori version ("dev" for local builds).
func (r *RosterService) Version() string { return r.core.Version() }

// Recent returns the newest log rows (newest first), at most limit (default 100).
func (r *RosterService) Recent(limit int) ([]store.Event, error) {
	if limit <= 0 {
		limit = 100
	}
	return r.core.Events(context.Background(), store.Filter{Limit: limit, Desc: true})
}

func (r *RosterService) pump(ctx context.Context, app *application.App) {
	for ch := range r.core.Changes(ctx) {
		app.Event.Emit(EventRosterChanged, RosterChange{Change: ch, Agents: r.core.MeshAgents()})
	}
}

// trayStatus keeps the tray's status line at "N agents (M busy)".
func (r *RosterService) trayStatus(ctx context.Context, tray *TrayController) {
	set := func() {
		agents := r.core.Agents()
		busy := 0
		for _, a := range agents {
			if a.Status == "busy" {
				busy++
			}
		}
		switch len(agents) {
		case 0:
			tray.SetStatus("No agents running")
		case 1:
			tray.SetStatus(fmt.Sprintf("1 agent (%d busy)", busy))
		default:
			tray.SetStatus(fmt.Sprintf("%d agents (%d busy)", len(agents), busy))
		}
	}
	set()
	for range r.core.Changes(ctx) {
		set()
	}
}

// GoneSession is a recently gone Claude Code session the person may revive.
type GoneSession struct {
	Key        string    `json:"key"`
	SessionID  string    `json:"sessionId"`
	Name       string    `json:"name"`
	Title      string    `json:"title"`
	Cwd        string    `json:"cwd"`
	RepoKey    string    `json:"repoKey"`
	GoneAt     time.Time `json:"goneAt"`
	Reason     string    `json:"reason"`
	Transcript bool      `json:"transcript"` // its transcript file still exists, so --resume has something to load
	Trusted    bool      `json:"trusted"`    // Claude Code has accepted the workspace trust dialog for cwd or a parent
}

// Gone returns sessions that left the roster recently (newest first, at most
// limit, default 20) and aren't live again, from the log's agent.gone rows.
// Only Claude sessions are listed; Grok members can't be resumed.
func (r *RosterService) Gone(limit int) ([]GoneSession, error) {
	if limit <= 0 {
		limit = 20
	}
	ctx := context.Background()
	gone, err := r.core.Events(ctx, store.Filter{Kinds: []string{store.KindAgentGone}, Limit: 200, Desc: true})
	if err != nil {
		return nil, err
	}
	live := map[string]bool{}
	for _, a := range r.core.Agents() {
		live[a.Key] = true
	}
	var cards map[string]presence.AgentCard
	var trust trustSet
	out := []GoneSession{}
	seen := map[string]bool{}
	for _, ev := range gone {
		var p struct {
			Key    string `json:"key"`
			Reason string `json:"reason"`
		}
		if json.Unmarshal(ev.Payload, &p) != nil || p.Key == "" || seen[p.Key] || live[p.Key] {
			continue
		}
		seen[p.Key] = true
		if cards == nil {
			if cards, err = r.seenCards(ctx); err != nil {
				return nil, err
			}
			trust = loadTrust()
		}
		c, ok := cards[p.Key]
		if !ok || c.Agent != "claude" || !validSessionID(c.SessionID) || r.revives.recent(c.SessionID, ev.HLC.Wall()) {
			continue
		}
		out = append(out, GoneSession{
			Key: c.Key, SessionID: c.SessionID, Name: c.Name, Title: c.Title,
			Cwd: c.Cwd, RepoKey: c.RepoKey, GoneAt: ev.HLC.Wall(), Reason: p.Reason,
			Transcript: hasTranscript(c), Trusted: trust.covers(c.Cwd),
		})
		if len(out) == limit {
			break
		}
	}
	return out, nil
}

// seenCards maps agent key to the newest agent.seen card on this device,
// for every session ever seen here, not only recent ones.
func (r *RosterService) seenCards(ctx context.Context) (map[string]presence.AgentCard, error) {
	return r.cards.update(ctx, r.core.Events, r.core.Device().ID)
}

// cardIndex keeps the newest agent.seen card per key. Each update reads only
// the rows appended since the last one, paging oldest first, so the first
// call walks the whole history once and later calls are cheap. agent.seen is
// written once per appearance of a session, so the map stays one card per
// session ever seen on this device.
type cardIndex struct {
	mu     sync.Mutex
	cursor hlc.Timestamp
	cards  map[string]presence.AgentCard
}

// update reads new agent.seen rows for deviceID and returns a copy of the
// index. A failed read keeps what was already read, so the next call resumes.
func (x *cardIndex) update(ctx context.Context, events func(context.Context, store.Filter) ([]store.Event, error), deviceID string) (map[string]presence.AgentCard, error) {
	x.mu.Lock()
	defer x.mu.Unlock()
	if x.cards == nil {
		x.cards = map[string]presence.AgentCard{}
	}
	for {
		rows, err := events(ctx, store.Filter{
			Kinds: []string{store.KindAgentSeen}, DeviceID: deviceID, AfterHLC: x.cursor, Limit: store.MaxListLimit,
		})
		if err != nil {
			return nil, err
		}
		for _, ev := range rows {
			x.cursor = ev.HLC
			var c presence.AgentCard
			if json.Unmarshal(ev.Payload, &c) != nil || c.Key == "" {
				continue
			}
			x.cards[c.Key] = c // oldest first, so the newest card wins
		}
		if len(rows) < store.MaxListLimit {
			break
		}
	}
	out := make(map[string]presence.AgentCard, len(x.cards))
	for k, c := range x.cards {
		out[k] = c
	}
	return out, nil
}

// reviveWindow is how long a revived session stays claimed: long enough for
// claude --bg to start it and presence to list it, so a second click (or a
// stale Recently gone row) can't start the same session twice.
const reviveWindow = time.Minute

// reviveGuard refuses a second Revive of a session that was revived, or is
// being revived, within reviveWindow.
type reviveGuard struct {
	mu  sync.Mutex
	at  map[string]time.Time
	now func() time.Time // default time.Now
}

func (g *reviveGuard) clock() time.Time {
	if g.now != nil {
		return g.now()
	}
	return time.Now()
}

// claim marks sessionID as revived now. It fails, with how long ago the
// earlier claim was, when one is still inside the window.
func (g *reviveGuard) claim(sessionID string) (time.Duration, bool) {
	g.mu.Lock()
	defer g.mu.Unlock()
	now := g.clock()
	if g.at == nil {
		g.at = map[string]time.Time{}
	}
	for id, t := range g.at {
		if now.Sub(t) >= reviveWindow {
			delete(g.at, id)
		}
	}
	if t, ok := g.at[sessionID]; ok {
		return now.Sub(t), false
	}
	g.at[sessionID] = now
	return 0, true
}

// release drops a claim whose revive never started a session, so the person
// can try again at once.
func (g *reviveGuard) release(sessionID string) {
	g.mu.Lock()
	delete(g.at, sessionID)
	g.mu.Unlock()
}

// recent reports whether sessionID was revived inside the window after at,
// so Gone can drop a row whose session is starting up again.
func (g *reviveGuard) recent(sessionID string, after time.Time) bool {
	g.mu.Lock()
	defer g.mu.Unlock()
	t, ok := g.at[sessionID]
	return ok && t.After(after) && g.clock().Sub(t) < reviveWindow
}

// Revive runs `claude --bg --resume <sessionId>` in the session's last
// working directory and returns what it printed. The UI confirms first; this
// only refuses: a live session, an id never seen on this device, or a
// session whose transcript is gone (Claude Code would start an empty session
// under that id instead of failing), or one revived within reviveWindow. The
// cwd comes from the log, never from the caller.
func (r *RosterService) Revive(sessionID string) (string, error) {
	sessionID = strings.TrimSpace(sessionID)
	if !validSessionID(sessionID) {
		return "", errors.New("not a session id")
	}
	for _, a := range r.core.Agents() {
		if a.SessionID == sessionID {
			return "", fmt.Errorf("%s is still running", displayName(a))
		}
	}
	if ago, ok := r.revives.claim(sessionID); !ok {
		return "", fmt.Errorf("that session was revived %s ago; it should show up on the roster shortly", ago.Round(time.Second))
	}
	started := false
	defer func() {
		if !started {
			r.revives.release(sessionID)
		}
	}()
	cards, err := r.seenCards(context.Background())
	if err != nil {
		return "", err
	}
	c, ok := cards[r.core.Device().ID+"/"+sessionID]
	if !ok || c.Agent != "claude" {
		return "", errors.New("no Claude Code session with that id has been seen on this device")
	}
	if !hasTranscript(c) {
		return "", errors.New("that session's transcript is gone, so there is nothing to resume")
	}
	if st, err := os.Stat(c.Cwd); err != nil || !st.IsDir() {
		return "", fmt.Errorf("its folder %s no longer exists", c.Cwd)
	}
	bin, err := findClaude()
	if err != nil {
		return "", err
	}
	cmd := exec.Command(bin, "--bg", "--resume", sessionID)
	cmd.Dir = c.Cwd
	var out lockedBuffer
	cmd.Stdout, cmd.Stderr = &out, &out
	// claude --bg returns in about a second; a background service it starts
	// may inherit the pipes, so stop waiting on them shortly after it exits.
	cmd.WaitDelay = 3 * time.Second
	hideConsole(cmd)
	if err := cmd.Start(); err != nil {
		return "", fmt.Errorf("start claude: %w", err)
	}
	// From here a session may be starting, even if claude reports an error,
	// so the claim stands until reviveWindow passes.
	started = true
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case err = <-done:
	case <-time.After(30 * time.Second):
		// Never kill it: that could be the revived session itself.
		return strings.TrimSpace(out.String() + "\n(claude is still running; check the roster)"), nil
	}
	text := strings.TrimSpace(out.String())
	if err != nil && !errors.Is(err, exec.ErrWaitDelay) {
		if text == "" {
			text = err.Error()
		}
		return text, fmt.Errorf("claude --bg --resume failed: %s", text)
	}
	return text, nil
}

func displayName(a presence.AgentCard) string {
	if a.Name != "" {
		return a.Name
	}
	return "that session"
}

// validSessionID accepts Claude Code's UUID-style ids: hex and dashes, and
// never a leading dash, so it can't be read as a flag.
func validSessionID(id string) bool {
	if len(id) < 8 || len(id) > 64 || id[0] == '-' {
		return false
	}
	for _, c := range id {
		if c != '-' && (c < '0' || c > '9') && (c < 'a' || c > 'f') && (c < 'A' || c > 'F') {
			return false
		}
	}
	return true
}

// hasTranscript reports whether the session's transcript is on disk. The
// agent.seen card is written at first sight, sometimes before presence found
// the transcript, so Claude Code's own location is checked too.
func hasTranscript(c presence.AgentCard) bool {
	if fileExists(c.Transcript) {
		return true
	}
	dir, err := paths.ClaudeDir()
	if err != nil || c.Cwd == "" {
		return false
	}
	return fileExists(filepath.Join(dir, "projects", presence.Slug(c.Cwd), c.SessionID+".jsonl"))
}

func fileExists(p string) bool {
	if p == "" {
		return false
	}
	st, err := os.Stat(p)
	return err == nil && st.Mode().IsRegular()
}

// findClaude locates the Claude Code CLI without a shell: PATH first
// (claude.exe, then claude.cmd on Windows), then the installer's default
// ~/.local/bin, since a login-launched Irori may have a shorter PATH.
func findClaude() (string, error) {
	names := []string{"claude"}
	if runtime.GOOS == "windows" {
		names = []string{"claude.exe", "claude.cmd"}
	}
	for _, n := range names {
		if p, err := exec.LookPath(n); err == nil {
			return p, nil
		}
	}
	if home, err := os.UserHomeDir(); err == nil {
		for _, n := range names {
			if p := filepath.Join(home, ".local", "bin", n); fileExists(p) {
				return p, nil
			}
		}
	}
	return "", errors.New("can't find the claude command on PATH")
}

// hideConsole keeps a console child of the GUI from flashing a window on
// Windows (HideWindow + CREATE_NO_WINDOW). It sets the fields by name so
// this file needs no build tags; elsewhere it does nothing.
func hideConsole(cmd *exec.Cmd) {
	if runtime.GOOS != "windows" {
		return
	}
	attr := &syscall.SysProcAttr{}
	v := reflect.ValueOf(attr).Elem()
	if f := v.FieldByName("HideWindow"); f.IsValid() && f.Kind() == reflect.Bool {
		f.SetBool(true)
	}
	if f := v.FieldByName("CreationFlags"); f.IsValid() && f.Kind() == reflect.Uint32 {
		f.SetUint(0x08000000) // CREATE_NO_WINDOW
	}
	cmd.SysProcAttr = attr
}

// lockedBuffer collects a child's output (capped at 64 KiB) for exec's copy
// goroutines and a reader.
type lockedBuffer struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (l *lockedBuffer) Write(p []byte) (int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if room := 64<<10 - l.b.Len(); room > 0 {
		l.b.Write(p[:min(len(p), room)])
	}
	return len(p), nil
}

// String is the output so far with terminal colour and cursor escapes removed.
func (l *lockedBuffer) String() string {
	l.mu.Lock()
	defer l.mu.Unlock()
	return ansiEscape.ReplaceAllString(l.b.String(), "")
}

var ansiEscape = regexp.MustCompile(`\x1b(?:\[[0-9;?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_])`)

// trustSet holds the folders Claude Code's workspace trust dialog was
// accepted for, from the projects map in .claude.json. Only that field is
// decoded; nothing else in the file is kept.
type trustSet map[string]bool

func loadTrust() trustSet {
	file := ""
	if dir := os.Getenv("CLAUDE_CONFIG_DIR"); dir != "" {
		file = filepath.Join(dir, ".claude.json")
	} else if home, err := os.UserHomeDir(); err == nil {
		file = filepath.Join(home, ".claude.json")
	}
	raw, err := os.ReadFile(file)
	if err != nil {
		return trustSet{}
	}
	var cfg struct {
		Projects map[string]struct {
			HasTrustDialogAccepted bool `json:"hasTrustDialogAccepted"`
		} `json:"projects"`
	}
	if json.Unmarshal(raw, &cfg) != nil {
		return trustSet{}
	}
	t := trustSet{}
	for p, v := range cfg.Projects {
		if v.HasTrustDialogAccepted {
			t[trustKey(p)] = true
		}
	}
	return t
}

func trustKey(p string) string {
	p = strings.TrimRight(filepath.ToSlash(filepath.Clean(p)), "/")
	if runtime.GOOS == "windows" {
		p = strings.ToLower(p)
	}
	return p
}

// covers reports whether dir or one of its parents is trusted.
func (t trustSet) covers(dir string) bool {
	if dir == "" {
		return false
	}
	for p := filepath.Clean(dir); ; {
		if t[trustKey(p)] {
			return true
		}
		parent := filepath.Dir(p)
		if parent == p {
			return false
		}
		p = parent
	}
}
