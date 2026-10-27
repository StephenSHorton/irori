// Package search holds the types shared by Irori's search and show tools
// (docs/design/board.md "Contracts"). It has no index of its own: capture
// owns the index and the dispatch (capture.Search, capture.Show), and every
// service whose rows should be found implements Source. Keeping the types
// here means coord and inbox implement Source without importing capture.
//
// Progressive disclosure: an agent asks for the overview, then a short list
// of Hits (one line each, with a ref), then Show(ref) for one item's detail.
// Every result is capped and says how to go deeper.
//
// CONTRACT (shared by the capture, coord and inbox work packages):
package search

import (
	"context"
	"errors"
	"strings"
	"time"
)

var (
	ErrBadRef   = errors.New("search: not a ref (kind:id, e.g. task:01J... or pr:owner/repo#12)")
	ErrNotFound = errors.New("search: nothing has that ref")
)

// Kind is a ref's prefix: what sort of thing it names.
type Kind string

const (
	KindTurn     Kind = "turn"     // capture.TurnRow.ID ("t-...")
	KindEdit     Kind = "edit"     // capture.EditRow.ID ("e-...")
	KindCommit   Kind = "commit"   // full sha, or a unique prefix of at least 7 hex
	KindMsg      Kind = "msg"      // inbox.Message.ID
	KindTask     Kind = "task"     // coord.Task.ID
	KindComment  Kind = "comment"  // coord.Comment.ID
	KindPR       Kind = "pr"       // ledger key: "pr:<owner/name>#<n>" (owner/name lowercased when minted)
	KindDecision Kind = "decision" // decision.Decision.ID
	// KindAgent is show-only (never indexed): "agent:<device>/<session>"
	// shows that agent's newest timeline page.
	KindAgent Kind = "agent"
)

// Kinds lists every ref kind, in the order tools print them.
var Kinds = []Kind{KindTurn, KindEdit, KindCommit, KindMsg, KindTask, KindComment, KindPR, KindDecision, KindAgent}

// Limits. Search and Show clamp to them; tools never print more.
const (
	DefaultLimit = 10   // hits per Search
	MaxLimit     = 50   // hits per Search
	MaxTitle     = 120  // runes in Hit.Title / Shown.Title (one line)
	MaxSnippet   = 240  // runes in Hit.Snippet (one line, match marked with [ ])
	DefaultShow  = 4000 // bytes of Shown.Text
	MaxShow      = 6000 // bytes of Shown.Text (under tools.MaxResult with headers)
	MaxRelated   = 12   // refs in Shown.Related
)

// Ref builds "kind:id".
func Ref(k Kind, id string) string { return string(k) + ":" + id }

// ParseRef splits a ref into its kind and id. The id is everything after the
// first ':' (a pr ref keeps "owner/name#n"; an agent ref keeps its slash).
// Surrounding spaces are trimmed; the kind is case-insensitive.
func ParseRef(ref string) (Kind, string, error) {
	ref = strings.TrimSpace(ref)
	k, id, ok := strings.Cut(ref, ":")
	if !ok || id == "" {
		return "", "", ErrBadRef
	}
	kind := Kind(strings.ToLower(k))
	switch kind {
	case KindCommit:
		if len(id) < 7 || len(id) > 40 || strings.Trim(strings.ToLower(id), "0123456789abcdef") != "" {
			return "", "", ErrBadRef
		}
		id = strings.ToLower(id)
	case KindPR:
		repo, n, ok := strings.Cut(id, "#")
		if !ok || !strings.Contains(repo, "/") || n == "" || strings.Trim(n, "0123456789") != "" {
			return "", "", ErrBadRef
		}
	case KindTurn, KindEdit, KindMsg, KindTask, KindComment, KindDecision, KindAgent:
	default:
		return "", "", ErrBadRef
	}
	if strings.ContainsAny(id, " \t\r\n") {
		return "", "", ErrBadRef
	}
	return kind, id, nil
}

// Doc is one searchable item as a Source hands it to the index.
type Doc struct {
	Ref     string    // "task:<id>", ...
	Kind    Kind      // Ref's kind
	At      time.Time // when it happened (shown, and Query.Before compares it)
	Updated time.Time // when it last changed (Source.Docs pages on it); zero = At
	Title   string    // one line; the index trims it to MaxTitle
	Body    string    // indexed text (prompt + reply, message text, comment, ...); scrubbed already
	Agent   string    // agent key it belongs to, if any
	RepoKey string
}

// Query selects hits. Zero values mean any.
type Query struct {
	Text    string    // words; every word must match (prefix match on the last); "" lists newest
	Kinds   []Kind    // restrict to these kinds
	RepoKey string    // restrict to one repo
	Agent   string    // restrict to one agent key
	Before  time.Time // only items with At before this (paging: pass the last hit's At)
	Limit   int       // default DefaultLimit, max MaxLimit
}

// Hit is one search result: one line plus a ref to Show.
type Hit struct {
	Ref     string    `json:"ref"`
	Kind    Kind      `json:"kind"`
	At      time.Time `json:"at"`
	Title   string    `json:"title"`   // one line
	Snippet string    `json:"snippet"` // one line around the match
	Agent   string    `json:"agent,omitempty"`
	RepoKey string    `json:"repoKey,omitempty"`
}

// ShowOptions pages Shown.Text.
type ShowOptions struct {
	Offset int // byte offset into the rendered text (snapped back to a rune start)
	Limit  int // bytes; default DefaultShow, max MaxShow
}

// Shown is one item's detail, rendered as text for a model.
type Shown struct {
	Ref     string   `json:"ref"`
	Kind    Kind     `json:"kind"`
	Title   string   `json:"title"`
	Text    string   `json:"text"`              // Text[Offset:Offset+Limit] of the rendering, cut on a rune boundary
	More    bool     `json:"more"`              // more text after this page
	Next    int      `json:"next,omitempty"`    // the Offset for the next page when More
	Related []string `json:"related,omitempty"` // refs worth showing next (the turn behind an edit, a task's comments, ...), <= MaxRelated
}

// Source is a service whose items can be found and shown. Each service
// implements it on its own type (coord.Coord: task, comment; inbox.Inbox:
// msg); capture implements its own kinds internally. Core registers the
// outside ones with capture.AddSource. Sources never import capture.
type Source interface {
	// SearchKinds lists the ref kinds this source owns.
	SearchKinds() []Kind
	// SearchDocs returns docs whose Updated (or At) is at or after since,
	// oldest first, at most limit. The index calls it with the newest
	// Updated it has seen, so a doc at the boundary comes back again; the
	// index upserts by Ref. Docs are never deleted (the log is append-only):
	// a changed doc is returned again with its new Updated.
	SearchDocs(ctx context.Context, since time.Time, limit int) ([]Doc, error)
	// SearchShow renders one of its refs (ErrNotFound when it has none).
	SearchShow(ctx context.Context, ref string, o ShowOptions) (Shown, error)
}

// Page cuts a rendering for Show: it applies o's defaults and caps, snaps
// to rune boundaries, and fills Text, More and Next. Sources use it so every
// Show pages the same way.
func Page(full string, o ShowOptions) (text string, more bool, next int) {
	limit := o.Limit
	if limit <= 0 {
		limit = DefaultShow
	}
	limit = min(limit, MaxShow)
	off := max(0, min(o.Offset, len(full)))
	for off > 0 && off < len(full) && !runeStart(full[off]) {
		off--
	}
	end := min(len(full), off+limit)
	for end > off && end < len(full) && !runeStart(full[end]) {
		end--
	}
	if end < len(full) {
		return full[off:end], true, end
	}
	return full[off:end], false, 0
}

func runeStart(b byte) bool { return b&0xC0 != 0x80 }
