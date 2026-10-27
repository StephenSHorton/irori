// Package capture is the capture service (docs/design/v0.3-capture.md): it
// tails Claude Code transcripts into turns, edits, summaries and PR ledger
// rows, watches each known worktree's reflog and links commits to edits, and
// keeps decisions. Like the inbox, its state is a fold over the event log,
// rebuilt at New; every new fact is appended first. It uses the wave-1
// packages: capture/transcript, capture/gitlink, scrub, blob, ledger,
// decision, gitsha.
//
// Local bookkeeping (file offsets) lives in <DataDir>/capture-cursors.json and
// never in the log. Patches and long summaries live in the blob store at
// <DataDir>/blobs.
//
// CONTRACT (implemented by the capture work package; keep these signatures):
package capture

import (
	"context"
	"errors"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/scrub"
	"github.com/StephenSHorton/irori/internal/store"
)

// Log kinds capture writes and replays. Payloads only ever gain fields.
const (
	KindTurn       = "capture.turn"      // TurnRow
	KindEdit       = "capture.edit"      // EditRow
	KindSummary    = "capture.summary"   // SummaryRow
	KindCommitCmd  = "capture.commitcmd" // CommitCmdRow: a Bash call that may have committed
	KindPR         = "pr.ledger"         // ledger.Entry
	KindRepoSeen   = "repo.seen"         // {repoKey, root, at}
	KindCommitSeen = "commit.seen"       // CommitRow
	KindCommitLink = "commit.linked"     // CommitLink; newest per sha wins
	KindDecision   = "decision.recorded" // decision.Decision
)

// Defaults (Options zero values).
const (
	DefaultScanInterval     = 3 * time.Second
	DefaultRepoInterval     = 5 * time.Second
	DefaultQuietAfter       = 2 * time.Minute
	DefaultBackfillWindow   = 72 * time.Hour
	DefaultBackfillMaxBytes = 64 << 20
)

var (
	ErrNoRepo        = errors.New("capture: not inside a known git repo")
	ErrNotFound      = errors.New("capture: not found")
	ErrUnimplemented = errors.New("capture: not implemented yet")
)

// Roster is the presence capture reads (core.Core): live cards give the
// sessions to tail first, the cwd roots to watch, and the device's agent
// names for display.
type Roster interface {
	Agents() []presence.AgentCard
}

// Options configure New.
type Options struct {
	ClaudeDir string // ~/.claude; transcripts are under ClaudeDir/projects
	DataDir   string
	DeviceID  string
	Now       func() time.Time // default time.Now
	Git       gitlink.Git      // default gitlink.Exec
	// RepoKey maps a worktree root to its repo_key (repo.Resolver.Key).
	RepoKey func(dir string) string
	Scrub   *scrub.Scrubber // default scrub.New()
	// Zero means the Default constant.
	ScanInterval, RepoInterval, QuietAfter, BackfillWindow time.Duration
	BackfillMaxBytes                                       int64
}

// TurnRow is one capture.turn payload: a closed turn.
type TurnRow struct {
	ID      string `json:"id"`  // deterministic: "t-" + hex(sha256(key|turnId))[:20]; key|agentId|turnId for a subagent's turn
	Key     string `json:"key"` // agent key "<device>/<sessionId>"
	RepoKey string `json:"repoKey,omitempty"`
	Root    string `json:"root,omitempty"`
	// TurnID is the transcript turn id (Turn.ID). It is spelled out because
	// the row's own "id" hides the embedded Turn's in JSON; readers of the
	// JSON use "turnId", and the service fills Turn.ID from it on replay.
	TurnID string `json:"turnId"`
	transcript.Turn
	// Tool targets, prompt and reply are scrubbed; EditPaths are made
	// repo-relative (forward slashes) when under Root, else left absolute.
}

// EditRow is one capture.edit payload.
type EditRow struct {
	ID          string    `json:"id"` // "e-" + hex(sha256(key|toolUseId|absPath))[:20]
	Key         string    `json:"key"`
	AgentID     string    `json:"agentId,omitempty"`
	TurnID      string    `json:"turnId"` // transcript turn id
	ToolUseID   string    `json:"toolUseId"`
	Tool        string    `json:"tool"`
	RepoKey     string    `json:"repoKey,omitempty"`
	Root        string    `json:"root,omitempty"`
	Path        string    `json:"path"` // repo-relative forward slashes inside a repo, else absolute
	Create      bool      `json:"create,omitempty"`
	BeforeBlob  string    `json:"beforeBlob,omitempty"`
	AfterBlob   string    `json:"afterBlob,omitempty"`
	AfterBlobLF string    `json:"afterBlobLF,omitempty"`
	Patch       string    `json:"patch,omitempty"` // blob id of the scrubbed unified diff; "" when skipped
	Adds        int       `json:"adds"`
	Dels        int       `json:"dels"`
	Fidelity    string    `json:"fidelity"`
	At          time.Time `json:"at"`
}

// SummaryRow is one capture.summary payload.
type SummaryRow struct {
	ID  string `json:"id"` // "s-" + hash(key|agentId|kind|rowUuid or text)
	Key string `json:"key"`
	transcript.Summary
	FullBlob string `json:"fullBlob,omitempty"` // blob id when Summary.Full was set
}

// CommitCmdRow is one capture.commitcmd payload.
type CommitCmdRow struct {
	Key       string    `json:"key"`
	AgentID   string    `json:"agentId,omitempty"`
	TurnID    string    `json:"turnId"`
	ToolUseID string    `json:"toolUseId"`
	Cwd       string    `json:"cwd"`
	Command   string    `json:"command"` // scrubbed, first line, <= 300 runes
	At        time.Time `json:"at"`
}

// CommitRow is one commit.seen payload.
type CommitRow struct {
	RepoKey string `json:"repoKey"`
	Root    string `json:"root"`
	Op      string `json:"op"` // reflog message prefix up to ':' ("commit", "merge v0.2-talk", ...)
	Branch  string `json:"branch,omitempty"`
	gitlink.Commit
}

// Committer is who ran the command that made a commit.
type Committer struct {
	Key    string `json:"key"`
	TurnID string `json:"turnId"`
}

// CommitLink is one commit.linked payload.
type CommitLink struct {
	RepoKey   string             `json:"repoKey"`
	SHA       string             `json:"sha"`
	Committer *Committer         `json:"committer,omitempty"`
	Files     []gitlink.FileLink `json:"files"`
	Sessions  []string           `json:"sessions,omitempty"` // agent keys behind linked edits
	Turns     []string           `json:"turns,omitempty"`    // transcript turn ids, with Sessions order
	At        time.Time          `json:"at"`
}

// EntryKind is a timeline entry's type.
type EntryKind string

const (
	EntryTurn    EntryKind = "turn"
	EntrySummary EntryKind = "summary"
	EntryCommit  EntryKind = "commit"
	EntryPR      EntryKind = "pr"
)

// Entry is one timeline line; exactly one pointer is set.
type Entry struct {
	Kind    EntryKind     `json:"kind"`
	At      time.Time     `json:"at"`
	Turn    *TurnView     `json:"turn,omitempty"`
	Summary *SummaryRow   `json:"summary,omitempty"`
	Commit  *CommitView   `json:"commit,omitempty"`
	PR      *ledger.Entry `json:"pr,omitempty"`
}

// TurnView is a turn with its edits.
type TurnView struct {
	TurnRow
	Open  bool      `json:"open,omitempty"` // the in-memory open turn of a live session
	Edits []EditRow `json:"edits,omitempty"`
}

// CommitView is a commit with its newest link.
type CommitView struct {
	CommitRow
	Link *CommitLink `json:"link,omitempty"`
}

// Timeline is one agent's history, newest first.
type Timeline struct {
	Key     string  `json:"key"`
	Entries []Entry `json:"entries"`
	// More is true when older entries exist (page with Before = the last
	// entry's At).
	More bool `json:"more"`
}

// TimelineQuery selects a page.
type TimelineQuery struct {
	Key    string    // agent key; required
	Before time.Time // zero: newest
	Limit  int       // turns per page; default 20, max 100
	// Subagents includes turns from the session's subagents and workflow
	// agents (AgentID set). Default false hides them from the page; their
	// edits still count for commit links and Why.
	Subagents bool
}

// EditQuery narrows Edits. Zero values mean any.
type EditQuery struct {
	RepoKey string
	Path    string // repo-relative, exact
	Key     string
	Limit   int // default 50, max 500
}

// WhyQuery asks what is behind a path.
type WhyQuery struct {
	Path    string // absolute, or relative to Root / the caller's cwd
	Line    int    // optional, 1-based
	RepoKey string // optional; else from Cwd
	Cwd     string // the caller's cwd, to resolve the repo and relative paths
}

// WhyResult answers a WhyQuery.
type WhyResult struct {
	RepoKey   string         `json:"repoKey"`
	Root      string         `json:"root"`
	Path      string         `json:"path"` // repo-relative
	Line      int            `json:"line,omitempty"`
	BlameSHA  string         `json:"blameSha,omitempty"`  // commit that last changed Line; "" if uncommitted or no Line
	Commit    *CommitView    `json:"commit,omitempty"`    // that commit, if seen
	Edits     []EditRow      `json:"edits"`               // newest first, <= 10; when Line is uncommitted, those that touched the path since its last commit
	Turns     []TurnRow      `json:"turns,omitempty"`     // the turns behind Commit's link or behind Edits, <= 10
	Decisions []DecisionView `json:"decisions,omitempty"` // anchored at Path (overlapping Line when given)
}

// DecisionView is a decision with its anchor re-checked.
type DecisionView struct {
	decision.Listed
	Status *decision.Status `json:"status,omitempty"` // nil when unanchored
}

// DecisionInput is a new decision before it is anchored.
type DecisionInput struct {
	By         decision.Party
	Trust      string // "user" or "untrusted"
	Statement  string
	Why        string
	Path       string // optional; absolute or relative to Cwd
	Lines      string // optional, "40" or "40-52"
	Cwd        string // the caller's cwd
	Supersedes []string
	Source     *decision.Source
}

// ChangeKind names what moved.
type ChangeKind string

const (
	ChangedTurn     ChangeKind = "turn"
	ChangedEdit     ChangeKind = "edit"
	ChangedCommit   ChangeKind = "commit"
	ChangedDecision ChangeKind = "decision"
	ChangedPR       ChangeKind = "pr"
	ChangedOpenTurn ChangeKind = "open" // an in-memory open turn grew (not logged)
)

// Change tells the UI to refresh.
type Change struct {
	Kind    ChangeKind `json:"kind"`
	Key     string     `json:"key,omitempty"` // agent key
	RepoKey string     `json:"repoKey,omitempty"`
}

// Status is capture's own health, for the GUI and the Doctor tab.
type Status struct {
	Files       int       `json:"files"`   // transcript files tracked
	Backlog     int64     `json:"backlog"` // unread bytes across them
	Repos       int       `json:"repos"`   // worktree roots watched
	LastScan    time.Time `json:"lastScan"`
	Backfilling bool      `json:"backfilling"` // the first pass hasn't finished
	LastError   string    `json:"lastError,omitempty"`
	// Dropped counts heuristic (shell) edits discarded as noise since this
	// process started; they are never logged, so the counts aren't replayed.
	Dropped Dropped `json:"dropped"`
}

// Capture is safe for concurrent use.
type Capture struct {
	impl *capture
}

// New replays the log (ids already held, uncommitted edits, commits seen and
// linked, ledger, decisions, roots seen) and returns a ready service. Nothing
// is read from disk until Run.
func New(ctx context.Context, st *store.Store, roster Roster, opts Options) (*Capture, error) {
	return newCapture(ctx, st, roster, opts)
}

// Run tails transcripts and watches repos until ctx ends. The first pass
// (backfill) runs inside Run, live sessions first, without blocking callers
// of the read methods below.
func (c *Capture) Run(ctx context.Context) error { return c.impl.run(ctx) }

// Timeline returns a page of one agent's history: closed turns (with their
// edits), the open turn first when the session is live and has one,
// summaries, PR ledger entries for the agent, and commits the agent
// committed or is linked to. Irori messages are not included: callers merge
// them from the inbox (they are a different service's rows).
func (c *Capture) Timeline(q TimelineQuery) Timeline { return c.impl.timeline(q) }

// Edits lists captured edits, newest first.
func (c *Capture) Edits(q EditQuery) []EditRow { return c.impl.edits(q) }

// Commits lists a repo's commits seen, newest committer time first, with
// their newest link. limit default 50, max 500.
func (c *Capture) Commits(repoKey string, limit int) []CommitView {
	return c.impl.commits(repoKey, limit)
}

// Commit returns one commit by sha (any repo).
func (c *Capture) Commit(sha string) (CommitView, bool) { return c.impl.commit(sha) }

// Repos lists the repo keys capture has seen, with their known roots.
func (c *Capture) Repos() map[string][]string { return c.impl.repos() }

// Blob returns a stored patch or summary (blob.ErrNotFound / blob.ErrBadID).
func (c *Capture) Blob(id string) ([]byte, error) { return c.impl.blob(id) }

// PROwners lists the agent keys owning a PR (ledger key), best first.
func (c *Capture) PROwners(prKey string) []string { return c.impl.prOwners(prKey) }

// PRs lists ledger entries for a repo ("owner/name"; "" = all).
func (c *Capture) PRs(repo string) []ledger.Entry { return c.impl.prs(repo) }

// Why answers what is behind a path (and line): git blame, the commit's
// link, recent edits, the turns behind them, and decisions anchored there.
// It is a read: a path outside the repos capture has already seen (or that
// a live agent works in) is ErrNoRepo, and the query never logs a repo or
// starts watching one.
func (c *Capture) Why(ctx context.Context, q WhyQuery) (WhyResult, error) { return c.impl.why(ctx, q) }

// RecordDecision validates, anchors (MakeAnchor against the root of Cwd's
// repo, with HEAD's sha) and appends a decision. Supersedes are checked with
// View.CanSupersede.
func (c *Capture) RecordDecision(ctx context.Context, in DecisionInput) (decision.Decision, error) {
	return c.impl.recordDecision(ctx, in)
}

// Decisions lists decisions with their anchors re-checked against the
// recorded root if it exists, else a live card's root for the repo, else
// Unknown.
func (c *Capture) Decisions(f decision.Filter) []DecisionView { return c.impl.decisions(f) }

// Subscribe delivers changes until ctx ends (buffered; slow readers drop).
func (c *Capture) Subscribe(ctx context.Context) <-chan Change { return c.impl.subscribe(ctx) }

// Status reports capture's health.
func (c *Capture) Status() Status { return c.impl.status() }
