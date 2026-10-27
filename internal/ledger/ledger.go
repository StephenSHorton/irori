// Package ledger is the PR authorship ledger and PR reference parsing,
// ported from tsukumo (docs/research/v02/router.md §11 and §12;
// docs/design/v0.3-capture.md §6). It is pure: the capture service feeds it
// entries folded from the log and asks it who owns a PR.
//
// CONTRACT (implemented by the ledger work package; keep these signatures):
package ledger

import "time"

// Role ranks an owner. Rank order (tsukumo's code, not its comment):
// Author 0, Touched 1, Reviewer 2.
type Role string

const (
	Author   Role = "author"
	Touched  Role = "touched"
	Reviewer Role = "reviewer"
)

// Keep is how long an entry stays in the view.
const Keep = 45 * 24 * time.Hour

// Entry is one ledger row: a session's relation to a PR.
type Entry struct {
	Key     string    `json:"key"`  // Key(Repo, Number)
	Repo    string    `json:"repo"` // "owner/name" as first seen (case kept)
	Number  int       `json:"number"`
	URL     string    `json:"url,omitempty"`
	Session string    `json:"session"` // agent key "<device>/<sessionId>"
	Role    Role      `json:"role"`
	Source  string    `json:"source"` // "pr-link", "gh", "gitOperation"
	At      time.Time `json:"at"`
}

// Key is "pr:<lowercased owner/name>#<n>".
func Key(repo string, n int) string { return key(repo, n) }

// RepoKey is the repo.Resolver key for a GitHub "owner/name":
// "remote:github.com/<lowercased owner/name>".
func RepoKey(repo string) string { return repoKey(repo) }

// RepoFromKey inverts RepoKey for remote:github.com/ keys ("" otherwise).
func RepoFromKey(repoKey string) string { return repoFromKey(repoKey) }

// Ledger is the in-memory view. Safe for concurrent use.
type Ledger struct {
	impl *ledger
}

// New makes an empty ledger.
func New() *Ledger { return newLedger() }

// Add merges e on the identity (Key, Session, Role): a new identity is added;
// an existing one keeps the earliest At and the first non-empty URL. It
// reports whether the view changed. Entries with an empty Key, Session, a
// zero At (it could never show in the view) or an unknown Role are ignored.
func (l *Ledger) Add(e Entry) bool { return l.impl.add(e) }

// Owners lists a PR's entries younger than Keep at now, best first: by role
// rank, then earliest At, then Session for a stable order.
func (l *Ledger) Owners(key string, now time.Time) []Entry { return l.impl.owners(key, now) }

// ForRepo lists every entry younger than Keep for a repo ("owner/name",
// case-insensitive; "" means all), newest PR number first, then as Owners.
func (l *Ledger) ForRepo(repo string, now time.Time) []Entry { return l.impl.forRepo(repo, now) }

// ForSession lists a session's entries younger than Keep, newest At first.
func (l *Ledger) ForSession(session string, now time.Time) []Entry {
	return l.impl.forSession(session, now)
}

// Ref names one PR.
type Ref struct {
	Repo   string `json:"repo"` // "owner/name"
	Number int    `json:"number"`
}

// RefOptions are ExtractRefs' defaults.
type RefOptions struct {
	DefaultRepo string            // "owner/name": for bare #N and pr#N / pull#N
	Owner       string            // default owner for name#N; else DefaultRepo's owner
	Aliases     map[string]string // lowercased alias -> repo ("name" or "owner/name")
}

// ExtractRefs finds PR references in free text, in order, deduplicated by
// Key: GitHub PR URLs, owner/name#N and name#N (1-6 digits), bare #N (2-5
// digits, needs DefaultRepo). Fenced and inline code is stripped first.
// RE2 has no lookbehind: the rune before a match is checked by hand
// (router.md §12): name#N rejects a preceding [A-Za-z0-9_/<#-], bare #N a
// preceding [A-Za-z0-9_/<#&@]; after the digits the next rune must not be a
// word character.
func ExtractRefs(text string, o RefOptions) []Ref { return extractRefs(text, o) }

// ParseTarget reads an explicit PR address: "pr:owner/name#N",
// "owner/name#N" or a GitHub PR URL (with or without scheme, trailing
// path such as /files allowed).
func ParseTarget(s string) (Ref, bool) { return parseTarget(s) }

// BashResult is what the transcript holds for one Bash call.
type BashResult struct {
	Command      string
	Stdout       string // may be clipped
	GitOperation []byte // raw toolUseResult.gitOperation JSON, or nil
	DefaultRepo  string // "owner/name" from the session's repoKey, or ""
}

// BashRef is a PR relation a Bash call shows.
type BashRef struct {
	Ref    Ref
	URL    string
	Role   Role
	Source string // "gitOperation" or "gh"
}

// FromBash reads PR relations from one Bash call:
//   - gitOperation.pr with action "created" -> Author (source gitOperation);
//   - else, when the command matches \bgh\s+pr\s+create\b, the first
//     github.com/<o>/<r>/pull/<n> in Stdout -> Author (source gh);
//   - every `gh pr review <target>`: a URL, or a number with the repo from
//     -R/--repo <o/r>, else DefaultRepo -> Reviewer (source gh). A number with
//     no repo is skipped.
func FromBash(b BashResult) []BashRef { return fromBash(b) }
