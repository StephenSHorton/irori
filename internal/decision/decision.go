// Package decision is decision records with code anchors that are re-checked
// when read (docs/design/v0.3-capture.md §7). It is pure apart from reading
// files under a worktree root. The capture service folds decision.recorded
// rows into a View and appends new ones.
//
// CONTRACT (implemented by the decision work package; keep these signatures):
package decision

import (
	"errors"
	"time"
)

const (
	MaxStatement = 500
	MaxWhy       = 2000
	ContextLines = 2
)

var (
	ErrEmpty      = errors.New("decision: statement is empty")
	ErrTooLong    = errors.New("decision: statement or why is too long")
	ErrBadLines   = errors.New(`decision: lines must look like "40" or "40-52"`)
	ErrBadPath    = errors.New("decision: path must be inside the repo")
	ErrNoFile     = errors.New("decision: no such file")
	ErrRange      = errors.New("decision: lines are past the end of the file")
	ErrSupersede  = errors.New("decision: only its author or the person can supersede a decision")
	ErrNoDecision = errors.New("decision: no such decision")
)

// Party is who recorded a decision (the same shape as inbox's sender).
type Party struct {
	Key  string `json:"key"`
	Name string `json:"name,omitempty"`
	Kind string `json:"kind,omitempty"` // "claude", "grok", "user"
}

// Anchor ties a decision to code.
type Anchor struct {
	RepoKey string   `json:"repoKey"`
	Root    string   `json:"root"`            // worktree root it was recorded in
	Path    string   `json:"path"`            // repo-relative, forward slashes
	SHA     string   `json:"sha,omitempty"`   // HEAD when recorded
	Start   int      `json:"start,omitempty"` // 1-based, inclusive; 0 = whole file
	End     int      `json:"end,omitempty"`
	Hash    string   `json:"hash"`              // see MakeAnchor
	Context []string `json:"context,omitempty"` // up to ContextLines before, then after
}

// Source is what a decision was marked from in the GUI.
type Source struct {
	MessageID string `json:"messageId,omitempty"`
	TurnID    string `json:"turnId,omitempty"`
}

// Decision is one decision.recorded payload.
type Decision struct {
	ID         string    `json:"id"`
	By         Party     `json:"by"`
	Statement  string    `json:"statement"`
	Why        string    `json:"why,omitempty"`
	Anchor     *Anchor   `json:"anchor,omitempty"`
	Supersedes []string  `json:"supersedes,omitempty"`
	Source     *Source   `json:"source,omitempty"`
	Trust      string    `json:"trust"` // "user" from the GUI, else "untrusted"
	At         time.Time `json:"at"`
}

// Validate trims Statement and Why, then checks them (ErrEmpty, ErrTooLong,
// by rune count).
func Validate(d *Decision) error { return validate(d) }

// ParseLines reads "40" or "40-52" (spaces allowed around '-'); "" is (0,0).
// start must be >= 1 and end >= start.
func ParseLines(s string) (start, end int, err error) { return parseLines(s) }

// MakeAnchor reads <root>/<path> and builds an anchor. path may be absolute
// (it must then lie under root) or root-relative; it is stored relative with
// forward slashes, and "..", or a path escaping root after symlink-free
// cleaning, is ErrBadPath. start == 0 anchors the whole file: Hash is the
// file's git blob id (gitsha.Blob of the LF form). Otherwise Hash is the
// SHA-256 hex of the lines start..end joined with "\n", each with trailing
// whitespace and any "\r" removed, and Context holds up to ContextLines
// lines before and after (same normalisation).
func MakeAnchor(root, path string, start, end int, repoKey, headSHA string) (Anchor, error) {
	return makeAnchor(root, path, start, end, repoKey, headSHA)
}

// AnchorState is the result of re-checking an anchor.
type AnchorState string

const (
	OK      AnchorState = "ok"      // the lines are where they were
	Moved   AnchorState = "moved"   // the same lines are elsewhere in the file
	Changed AnchorState = "changed" // the file is there, the lines are not
	Gone    AnchorState = "gone"    // the file is missing
	Unknown AnchorState = "unknown" // no worktree to check against
)

// Status is an anchor re-check.
type Status struct {
	State AnchorState `json:"state"`
	Start int         `json:"start,omitempty"` // current range when OK or Moved
	End   int         `json:"end,omitempty"`
}

// Check re-reads the anchored file under root ("" -> Unknown). For a
// whole-file anchor: OK when the LF blob id matches, else Changed. For lines:
// OK when the normalised hash matches at Start..End; else Moved to the
// nearest window of the same length whose hash matches (ties: the earlier);
// else Changed. Files over 8 MiB are Changed without scanning.
func Check(a Anchor, root string) Status { return check(a, root) }

// View folds decisions. Safe for concurrent use.
type View struct {
	impl *view
}

// NewView makes an empty view.
func NewView() *View { return newView() }

// Add folds one decision (idempotent on ID). Supersedes naming unknown ids
// are kept and apply if those ids arrive later.
func (v *View) Add(d Decision) { v.impl.add(d) }

// Get returns one decision.
func (v *View) Get(id string) (Decision, bool) { return v.impl.get(id) }

// CanSupersede: the person (by.Kind == "user") may supersede anything; an
// agent only decisions whose By.Key equals its own. ErrNoDecision for an
// unknown target.
func (v *View) CanSupersede(by Party, target string) error { return v.impl.canSupersede(by, target) }

// Filter narrows List. Zero values mean any.
type Filter struct {
	RepoKey string // anchored in this repo
	Path    string // anchored at this repo-relative path (exact match)
	Query   string // case-insensitive substring of statement or why
	All     bool   // include superseded decisions
	Limit   int    // default 50, max 500
}

// Listed is a decision in a list, with who replaced it.
type Listed struct {
	Decision
	SupersededBy []string `json:"supersededBy,omitempty"`
}

// List returns matches newest first.
func (v *View) List(f Filter) []Listed { return v.impl.list(f) }
