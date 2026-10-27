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
}

// Agents returns the live agents on this device, newest activity first.
func (r *RosterService) Agents() []presence.AgentCard { return r.core.Agents() }

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
		app.Event.Emit(EventRosterChanged, RosterChange{Change: ch, Agents: r.core.Agents()})
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
		if !ok || c.Agent != "claude" || !validSessionID(c.SessionID) {
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

// seenCards maps agent key to the newest agent.seen card on this device.
func (r *RosterService) seenCards(ctx context.Context) (map[string]presence.AgentCard, error) {
	rows, err := r.core.Events(ctx, store.Filter{
		Kinds: []string{store.KindAgentSeen}, DeviceID: r.core.Device().ID, Limit: 1000, Desc: true,
	})
	if err != nil {
		return nil, err
	}
	cards := map[string]presence.AgentCard{}
	for _, ev := range rows {
		var c presence.AgentCard
		if json.Unmarshal(ev.Payload, &c) != nil || c.Key == "" {
			continue
		}
		if _, ok := cards[c.Key]; !ok {
			cards[c.Key] = c
		}
	}
	return cards, nil
}

// Revive runs `claude --bg --resume <sessionId>` in the session's last
// working directory and returns what it printed. The UI confirms first; this
// only refuses: a live session, an id never seen on this device, or a
// session whose transcript is gone (Claude Code would start an empty session
// under that id instead of failing). The cwd comes from the log, never from
// the caller.
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
	cards, err := r.seenCards(context.Background())
	if err != nil {
		return "", err
	}
	c, ok := cards[r.core.Device().ID+"/"+sessionID]
	if !ok || c.Agent != "claude" {
		return "", errors.New("no Claude Code session with that id has run on this device")
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
