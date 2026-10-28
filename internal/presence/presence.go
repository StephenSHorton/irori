// Package presence sees which coding agents are running on this device.
//
// v0.1 source: Claude Code's ~/.claude/sessions/<pid>.json files (one per live
// session; see testdata) plus a tail of each session's transcript
// ~/.claude/projects/<slug>/<sessionId>.jsonl for the last activity, title and
// a short last-reply snippet.
//
// Rules:
//   - Never open, read, log or replicate the <pid>.<hash>.key files next to the
//     session json; they are secrets. Only *.json whose stem is all digits counts.
//   - Tolerate unknown and missing fields; the format is Claude Code's, not ours.
//   - A session file whose pid is dead (or whose pid was reused, checked via
//     procStart where the OS allows) is a gone agent, not a live one.
//   - Slug: every rune of cwd that is not [A-Za-z0-9] becomes '-'
//     (C:\Users\4step -> C--Users-4step). If <slug>/<sessionId>.jsonl is
//     missing, fall back to a glob over projects/*/<sessionId>.jsonl.
//   - Read only the transcript tail (last ~256 KB), never the whole file;
//     transcripts reach hundreds of MB.
//
// CONTRACT (implemented by the presence work package; keep these signatures):
package presence

import (
	"context"
	"time"
)

// AgentCard is what the roster shows for one session.
type AgentCard struct {
	Key          string    `json:"key"` // "<deviceId>/<sessionId>"
	DeviceID     string    `json:"deviceId"`
	Agent        string    `json:"agent"` // "claude" (grok/rock later)
	SessionID    string    `json:"sessionId"`
	PID          int       `json:"pid"`
	Name         string    `json:"name"`  // session name, else derived
	Title        string    `json:"title"` // latest custom-title / ai-title from the transcript
	Cwd          string    `json:"cwd"`
	Status       string    `json:"status"`      // as Claude Code reports: "busy", "idle", ...; "gone" when dead
	SessionKind  string    `json:"sessionKind"` // "interactive", "bg", ...
	Entrypoint   string    `json:"entrypoint"`  // "cli", "desktop", ...
	Version      string    `json:"version"`
	PeerFeatures []string  `json:"peerFeatures"`
	StartedAt    time.Time `json:"startedAt"`
	UpdatedAt    time.Time `json:"updatedAt"`    // statusUpdatedAt/updatedAt from the session file
	LastActivity time.Time `json:"lastActivity"` // newest transcript row timestamp
	LastText     string    `json:"lastText"`     // last assistant prose, <=240 runes, single line
	Transcript   string    `json:"transcript"`   // absolute path, "" if not found
	// v0.2 additions.
	RepoKey string `json:"repoKey"` // repo.Resolver.Key(Cwd); "" outside git
	// PermissionMode is the newest mode row's permissionMode: a
	// "permission-mode" row, or a non-sidechain "user" row carrying a
	// top-level permissionMode. It is read from the transcript tail, kept once
	// its row scrolls out, and on first sight of a transcript whose tail has
	// none it comes from a bounded backward scan (8 MiB). "" is unknown. The
	// value set is open ("default", "acceptEdits", "auto", "bypassPermissions",
	// ...): consumers test == "bypassPermissions", never a fixed list.
	PermissionMode string `json:"permissionMode"`
	// DeviceState is set on another device's card in the mesh roster (v0.4
	// §6.2): "connected", "recent" or "offline". "" on this device's own.
	DeviceState string `json:"deviceState,omitempty"`
}

// ChangeKind says what happened to a card.
type ChangeKind string

const (
	Added   ChangeKind = "added"
	Updated ChangeKind = "updated"
	Removed ChangeKind = "removed"
)

// Change is one roster update.
type Change struct {
	Kind ChangeKind `json:"kind"`
	Card AgentCard  `json:"card"`
}

// Options configure a Watcher.
type Options struct {
	ClaudeDir string // e.g. paths.ClaudeDir()
	DeviceID  string
	// Interval is the rescan period when file events are unavailable
	// (fswatch unsupported on this OS, or a watched dir missing); default 2s.
	Interval time.Duration
	// SafetyInterval is the slow full rescan (session files, liveness, tails)
	// that backs up file and process-exit events; default 30s.
	SafetyInterval time.Duration
	Now            func() time.Time
	// RepoKey maps a cwd to its repo_key (v0.2). Nil leaves RepoKey empty.
	RepoKey func(cwd string) string
}

// Watcher keeps the roster current. Safe for concurrent use.
type Watcher struct {
	impl *watcher
}

// New makes a watcher; nothing runs until Run.
func New(opts Options) *Watcher { return newWatcher(opts) }

// Run scans until ctx ends: on file events under ClaudeDir/sessions and
// ClaudeDir/projects and on session process exit where the OS reports them,
// else every Interval, with a full rescan every SafetyInterval. Changes are
// reported to subscribers; Updated is sent only when a field other than
// LastActivity moved by less than a second.
func (w *Watcher) Run(ctx context.Context) error { return w.run(ctx) }

// Snapshot returns the live cards, sorted by LastActivity (newest first).
func (w *Watcher) Snapshot() []AgentCard { return w.snapshot() }

// Subscribe delivers changes after the call until ctx ends (buffered; slow
// readers drop changes and should re-read Snapshot).
func (w *Watcher) Subscribe(ctx context.Context) <-chan Change { return w.subscribe(ctx) }
