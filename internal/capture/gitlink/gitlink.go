// Package gitlink watches a worktree's reflog and links commits to captured
// edits (docs/design/v0.3-capture.md §5). Git is reached through the Git
// interface so tests can run against real temp repos or fakes.
//
// CONTRACT (implemented by the gitlink work package; keep these signatures):
package gitlink

import (
	"context"
	"time"
)

// MaxWalk caps commits taken from one reflog movement.
const MaxWalk = 200

// MaxFiles caps the files kept per commit.
const MaxFiles = 500

// CommitterWindow is how far a commit command's time may be from the
// commit's committer time.
const CommitterWindow = 10 * time.Second

// Git runs git in dir and returns stdout. The real one is Exec.
type Git interface {
	Run(ctx context.Context, dir string, args ...string) (string, error)
}

// Exec runs the git binary with GIT_TERMINAL_PROMPT=0, no pager, no
// optional locks (GIT_OPTIONAL_LOCKS=0), and on Windows no console window
// (reuse the pattern in internal/repo/resolver.go).
var Exec Git = execGit{}

// GitDir is the directory holding a worktree's own logs/HEAD: <root>/.git
// when it is a directory, else the "gitdir: <path>" in the .git file
// (relative paths resolve against root).
func GitDir(root string) (string, error) { return gitDir(root) }

// ReflogEntry is one logs/HEAD line.
type ReflogEntry struct {
	Old     string    `json:"old"`
	New     string    `json:"new"`
	Name    string    `json:"name"`
	Email   string    `json:"email"`
	At      time.Time `json:"at"`
	Message string    `json:"message"`
}

// ParseReflog parses the complete lines of data (format
// "<old> <new> <name> <<email>> <unix> <+hhmm>\t<message>"), skipping
// malformed ones, and returns how many bytes it consumed (up to and including
// the last '\n'; a trailing partial line is left for next time).
func ParseReflog(data []byte) (entries []ReflogEntry, consumed int) { return parseReflog(data) }

// ZeroSHA reports an all-zero object id (any length).
func ZeroSHA(s string) bool { return zeroSHA(s) }

// NewCommits lists the commits a reflog movement brought into HEAD that seen
// doesn't already hold, oldest first: `git rev-list --reverse
// --max-count=MaxWalk <old>..<new>` (just <new> when Old is zero). skipped
// counts commits beyond MaxWalk (from `git rev-list --count`). A movement
// whose New is already seen returns nothing without running rev-list.
func NewCommits(ctx context.Context, g Git, root string, e ReflogEntry, seen func(sha string) bool) (shas []string, skipped int, err error) {
	return newCommits(ctx, g, root, e, seen)
}

// FileChange is one path a commit changed.
type FileChange struct {
	Path   string `json:"path"`   // repo-relative, forward slashes
	Blob   string `json:"blob"`   // new blob id; zeros for a deletion
	Status string `json:"status"` // git's letter: A, M, D, T
}

// Commit describes one commit.
type Commit struct {
	SHA          string       `json:"sha"`
	Parents      []string     `json:"parents,omitempty"`
	At           time.Time    `json:"at"` // committer time (%ct)
	Author       string       `json:"author"`
	Subject      string       `json:"subject"` // <= 200 runes
	Files        []FileChange `json:"files"`   // <= MaxFiles
	FilesOmitted int          `json:"filesOmitted,omitempty"`
}

// Describe reads a commit: `git show -s --format=%H%x00%P%x00%ct%x00%an%x00%s
// <sha>` and `git diff-tree -r --no-commit-id --no-renames --raw --no-abbrev
// --diff-merges=first-parent <sha>` (a root commit: add --root). Paths are unquoted
// (core.quotePath) so non-ASCII names come back as UTF-8: pass -z and split
// on NUL.
func Describe(ctx context.Context, g Git, root, sha string) (Commit, error) {
	return describe(ctx, g, root, sha)
}

// EditRef is an uncommitted captured edit, as linking sees it.
type EditRef struct {
	ID          string
	Root        string // worktree root the edit's file is under
	Path        string // repo-relative, forward slashes
	AfterBlob   string
	AfterBlobLF string
	Heuristic   bool
	At          time.Time
}

// Link classes.
const (
	LinkExact        = "exact"
	LinkPartial      = "partial"
	LinkHeuristic    = "heuristic"
	LinkUnattributed = "unattributed"
)

// FileLink is one committed file's link.
type FileLink struct {
	Path  string   `json:"path"`
	Link  string   `json:"link"`
	Edits []string `json:"edits,omitempty"` // EditRef ids, oldest first
}

// Classify links each of c's files (deletions included: a deletion links
// edits only by root, never by blob) to candidate edits, which are the
// repo's uncommitted edits from every root. Per file:
//   - exact: some non-heuristic edit (any root) with AfterBlob or AfterBlobLF
//     == file blob. Link the newest such edit plus the earlier edits with the
//     same Root and Path (At <= its At), heuristic ones included.
//   - partial: else, edits to the path whose Root == root. Link them all.
//   - heuristic: as partial when all of those are heuristic.
//   - unattributed: none.
//
// Paths compare exactly; on Windows the service lowercases nothing (git
// paths are case-sensitive as committed).
func Classify(c Commit, root string, candidates []EditRef) []FileLink {
	return classify(c, root, candidates)
}

// CommitCmd is a Bash call that may have made a commit.
type CommitCmd struct {
	Key    string // agent key
	TurnID string
	Cwd    string
	At     time.Time // when the command finished
	Start  time.Time // when it started; zero means At
}

// IsCommitCommand reports whether a Bash command line can create commits:
// git commit / merge / cherry-pick / revert / rebase / am / pull, or
// gh pr merge, anywhere in a compound command (&&, ;, |, newlines), allowing
// `git -C <dir>` and other global options before the subcommand.
func IsCommitCommand(command string) bool { return isCommitCommand(command) }

// Committer picks the command that made c: Cwd under root (case-insensitive
// prefix on Windows-style paths, either slash), |At - c.At| <=
// CommitterWindow, closest first, then latest. nil when none.
func Committer(c Commit, root string, cmds []CommitCmd) *CommitCmd {
	return committer(c, root, cmds)
}
