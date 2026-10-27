package capture

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/capture/transcript"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/scrub"
	"github.com/StephenSHorton/irori/internal/store"
)

// Bounds on in-memory state. The log keeps everything; these only limit what
// linking looks at.
const (
	// poolAge: an uncommitted edit older than this is no longer a link
	// candidate.
	poolAge = 30 * 24 * time.Hour
	// cmdAge: commit commands older than this can't be any new commit's
	// committer (the window is gitlink.CommitterWindow).
	cmdAge = 24 * time.Hour
	// relinkAge: commits seen this recently are linked again when edits
	// that match their files arrive late.
	relinkAge = time.Hour
	// commitSlack: an edit may be stamped this far after the commit's
	// (whole-second) committer time and still be in it.
	commitSlack = 2 * time.Second
	// gitTimeout bounds one git run.
	gitTimeout = 15 * time.Second
)

var replayKinds = []string{
	KindTurn, KindEdit, KindSummary, KindCommitCmd, KindPR,
	KindRepoSeen, KindCommitSeen, KindCommitLink, KindDecision,
}

// repoSeen is the repo.seen payload.
type repoSeen struct {
	RepoKey string    `json:"repoKey"`
	Root    string    `json:"root"`
	At      time.Time `json:"at"`
}

// commitRec is one commit seen, with its newest link.
type commitRec struct {
	row    CommitRow
	link   *CommitLink
	seenAt time.Time
}

type capture struct {
	st     *store.Store
	roster Roster
	opts   Options
	now    func() time.Time
	git    gitlink.Git
	scrub  *scrub.Scrubber
	blobs  *blob.Store
	roots  *rootCache

	mu          sync.RWMutex
	ids         map[string]bool // every row identity already logged
	turns       map[string][]*TurnRow
	editList    []*EditRow
	editByID    map[string]*EditRow
	editsByTurn map[string][]*EditRow // turnKey(key, agentID, turnID)
	pool        map[string]map[string]bool
	linked      map[string]bool     // edit ids some commit.linked names
	linkedWith  map[string][]string // edit id -> the file's edits its newest link named
	summaries   map[string][]*SummaryRow
	cmds        []CommitCmdRow
	ledger      *ledger.Ledger
	repoRoots   map[string]repoSeen // normRoot -> row
	rootsGen    int                 // bumped when repoRoots gains a root
	commitMap   map[string]*commitRec
	dview       *decision.View
	open        map[string]*transcript.Turn // openKey(key, agentID) -> open turn
	files       map[string]*tfile
	reflogs     map[string]*reflogState
	firstSize   map[string]int64 // normRoot -> logs/HEAD size when noteRoot first saw it
	backlog     int64
	lastScan    time.Time
	backfilling bool
	lastErr     string
	dropped     Dropped                    // heuristic edits discarded as noise, this process
	noisePaths  map[string]map[string]bool // turnKey -> editPathKey -> dropped (true) or kept
	ignores     ignoreCache

	subMu sync.Mutex
	subs  map[chan Change]struct{}

	repoDeadline time.Time // end of the current repo pass's budget; run goroutine only

	// tail-pass scratch, owned by the run goroutine
	cursors   *cursorFile
	newEdits  []*EditRow
	firstPass bool
}

func newCapture(ctx context.Context, st *store.Store, roster Roster, opts Options) (*Capture, error) {
	if opts.Now == nil {
		opts.Now = time.Now
	}
	if opts.Git == nil {
		opts.Git = gitlink.Exec
	}
	if opts.RepoKey == nil {
		opts.RepoKey = repo.New().Key
	}
	if opts.Scrub == nil {
		opts.Scrub = scrub.New()
	}
	def := func(d *time.Duration, v time.Duration) {
		if *d <= 0 {
			*d = v
		}
	}
	def(&opts.ScanInterval, DefaultScanInterval)
	def(&opts.RepoInterval, DefaultRepoInterval)
	def(&opts.SafetyInterval, DefaultSafetyInterval)
	def(&opts.QuietAfter, DefaultQuietAfter)
	def(&opts.BackfillWindow, DefaultBackfillWindow)
	if opts.BackfillMaxBytes <= 0 {
		opts.BackfillMaxBytes = DefaultBackfillMaxBytes
	}
	blobs, err := blob.Open(filepath.Join(opts.DataDir, "blobs"))
	if err != nil {
		return nil, fmt.Errorf("capture: %w", err)
	}
	c := &capture{
		st: st, roster: roster, opts: opts, now: opts.Now, git: opts.Git,
		scrub: opts.Scrub, blobs: blobs, roots: newRootCache(opts.Git, opts.Now),
		ids: map[string]bool{}, turns: map[string][]*TurnRow{},
		editByID: map[string]*EditRow{}, editsByTurn: map[string][]*EditRow{},
		pool: map[string]map[string]bool{}, linked: map[string]bool{}, linkedWith: map[string][]string{},
		summaries: map[string][]*SummaryRow{}, ledger: ledger.New(),
		repoRoots: map[string]repoSeen{}, commitMap: map[string]*commitRec{},
		dview: decision.NewView(), open: map[string]*transcript.Turn{},
		files: map[string]*tfile{}, reflogs: map[string]*reflogState{},
		firstSize: map[string]int64{}, noisePaths: map[string]map[string]bool{},
		subs: map[chan Change]struct{}{}, backfilling: true, firstPass: true,
	}
	if err := c.replay(ctx); err != nil {
		return nil, err
	}
	return &Capture{impl: c}, nil
}

// replay folds every capture row, oldest first, paging past List's cap.
func (c *capture) replay(ctx context.Context) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	var after hlc.Timestamp
	for {
		evs, err := c.st.List(ctx, store.Filter{Kinds: replayKinds, AfterHLC: after, Limit: store.MaxListLimit})
		if err != nil {
			return fmt.Errorf("capture: replay: %w", err)
		}
		for _, ev := range evs {
			if _, err := c.apply(ev); err != nil {
				return fmt.Errorf("capture: replay %s %s/%d: %w", ev.Kind, ev.DeviceID, ev.Seq, err)
			}
			after = ev.HLC
		}
		if len(evs) < store.MaxListLimit {
			break
		}
	}
	// Commit commands only matter near a new commit.
	cut := c.now().Add(-cmdAge)
	keep := c.cmds[:0]
	for _, cmd := range c.cmds {
		if cmd.At.After(cut) {
			keep = append(keep, cmd)
		}
	}
	c.cmds = keep
	return nil
}

// emit appends one row and folds it, then tells subscribers.
func (c *capture) emit(ctx context.Context, actor, kind string, payload any) error {
	ev, err := c.st.Append(ctx, actor, kind, payload)
	if err != nil {
		return fmt.Errorf("capture: append %s: %w", kind, err)
	}
	c.mu.Lock()
	ch, err := c.apply(ev)
	c.mu.Unlock()
	if err != nil {
		return err
	}
	if ch != nil {
		c.notify(*ch)
	}
	return nil
}

// apply folds one row; c.mu is held. It returns the change to announce.
func (c *capture) apply(ev store.Event) (*Change, error) {
	switch ev.Kind {
	case KindTurn:
		var r TurnRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return nil, err
		}
		if r.Turn.ID == "" {
			r.Turn.ID = r.TurnID
		}
		if c.ids[r.ID] {
			return nil, nil
		}
		c.ids[r.ID] = true
		c.turns[r.Key] = append(c.turns[r.Key], &r)
		return &Change{Kind: ChangedTurn, Key: r.Key, RepoKey: r.RepoKey}, nil
	case KindEdit:
		var r EditRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return nil, err
		}
		if c.editByID[r.ID] != nil {
			return nil, nil
		}
		c.ids[r.ID] = true
		e := &r
		c.editList = append(c.editList, e)
		c.editByID[r.ID] = e
		tk := turnKey(r.Key, r.AgentID, r.TurnID)
		c.editsByTurn[tk] = append(c.editsByTurn[tk], e)
		if r.RepoKey != "" && !c.linked[r.ID] {
			if c.pool[r.RepoKey] == nil {
				c.pool[r.RepoKey] = map[string]bool{}
			}
			c.pool[r.RepoKey][r.ID] = true
		}
		return &Change{Kind: ChangedEdit, Key: r.Key, RepoKey: r.RepoKey}, nil
	case KindSummary:
		var r SummaryRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return nil, err
		}
		if c.ids[r.ID] {
			return nil, nil
		}
		c.ids[r.ID] = true
		c.summaries[r.Key] = append(c.summaries[r.Key], &r)
		return &Change{Kind: ChangedTurn, Key: r.Key}, nil
	case KindCommitCmd:
		var r CommitCmdRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return nil, err
		}
		c.ids[cmdID(r.Key, r.ToolUseID)] = true
		c.cmds = append(c.cmds, r)
		return nil, nil
	case KindPR:
		var e ledger.Entry
		if err := json.Unmarshal(ev.Payload, &e); err != nil {
			return nil, err
		}
		c.ids[prID(e)] = true
		if !c.ledger.Add(e) {
			return nil, nil
		}
		return &Change{Kind: ChangedPR, Key: e.Session, RepoKey: ledger.RepoKey(e.Repo)}, nil
	case KindRepoSeen:
		var r repoSeen
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return nil, err
		}
		if r.Root == "" {
			return nil, nil
		}
		n := normRoot(r.Root)
		if _, ok := c.repoRoots[n]; !ok {
			c.rootsGen++
		}
		c.repoRoots[n] = r
		return nil, nil
	case KindCommitSeen:
		var r CommitRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return nil, err
		}
		k := commitKey(r.RepoKey, r.SHA)
		if rec := c.commitMap[k]; rec != nil {
			return nil, nil
		}
		c.commitMap[k] = &commitRec{row: r, seenAt: ev.HLC.Wall()}
		return &Change{Kind: ChangedCommit, RepoKey: r.RepoKey}, nil
	case KindCommitLink:
		var l CommitLink
		if err := json.Unmarshal(ev.Payload, &l); err != nil {
			return nil, err
		}
		rec := c.commitMap[commitKey(l.RepoKey, l.SHA)]
		if rec == nil {
			// A link always follows its commit.seen; keep it anyway.
			rec = &commitRec{row: CommitRow{RepoKey: l.RepoKey, Commit: gitlink.Commit{SHA: l.SHA}}, seenAt: ev.HLC.Wall()}
			c.commitMap[commitKey(l.RepoKey, l.SHA)] = rec
		}
		rec.link = &l
		for _, f := range l.Files {
			for _, id := range f.Edits {
				c.linked[id] = true
				c.linkedWith[id] = f.Edits
				if p := c.pool[l.RepoKey]; p != nil {
					delete(p, id)
				}
			}
		}
		return &Change{Kind: ChangedCommit, RepoKey: l.RepoKey}, nil
	case KindDecision:
		var d decision.Decision
		if err := json.Unmarshal(ev.Payload, &d); err != nil {
			return nil, err
		}
		c.dview.Add(d)
		rk := ""
		if d.Anchor != nil {
			rk = d.Anchor.RepoKey
		}
		return &Change{Kind: ChangedDecision, Key: d.By.Key, RepoKey: rk}, nil
	}
	return nil, nil
}

func (c *capture) has(id string) bool {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.ids[id]
}

func (c *capture) setErr(err error) {
	if err == nil {
		return
	}
	c.mu.Lock()
	c.lastErr = err.Error()
	c.mu.Unlock()
}

// subscribe delivers changes until ctx ends; a full buffer drops.
func (c *capture) subscribe(ctx context.Context) <-chan Change {
	ch := make(chan Change, 64)
	c.subMu.Lock()
	c.subs[ch] = struct{}{}
	c.subMu.Unlock()
	go func() {
		<-ctx.Done()
		c.subMu.Lock()
		delete(c.subs, ch)
		close(ch)
		c.subMu.Unlock()
	}()
	return ch
}

func (c *capture) notify(ch Change) {
	c.subMu.Lock()
	defer c.subMu.Unlock()
	for s := range c.subs {
		select {
		case s <- ch:
		default:
		}
	}
}

func (c *capture) status() Status {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return Status{
		Files: len(c.files), Backlog: c.backlog, Repos: len(c.reflogs),
		LastScan: c.lastScan, Backfilling: c.backfilling, LastError: c.lastErr,
		Dropped: c.dropped,
	}
}

// hashID is prefix + the first 20 hex digits of sha256(parts joined by "|").
func hashID(prefix string, parts ...string) string {
	sum := sha256.Sum256([]byte(strings.Join(parts, "|")))
	return prefix + hex.EncodeToString(sum[:])[:20]
}

// turnRowID is a turn's row id. A subagent's turns hash its agent id in too:
// its prompt rows may reuse the parent turn's promptId.
func turnRowID(key, agentID, turnID string) string {
	if agentID != "" {
		return hashID("t-", key, agentID, turnID)
	}
	return hashID("t-", key, turnID)
}

func editRowID(key, toolUseID, absPath string) string {
	return hashID("e-", key, toolUseID, absPath)
}

func summaryRowID(key, agentID, kind, uuidOrText string) string {
	return hashID("s-", key, agentID, kind, uuidOrText)
}

func cmdID(key, toolUseID string) string { return "c|" + key + "|" + toolUseID }

func prID(e ledger.Entry) string { return "p|" + e.Key + "|" + e.Session + "|" + string(e.Role) }

func commitKey(repoKey, sha string) string { return repoKey + "|" + strings.ToLower(sha) }

func turnKey(key, agentID, turnID string) string { return key + "|" + agentID + "|" + turnID }

func openKey(key, agentID string) string { return key + "|" + agentID }

func agentActor(key string) string { return "agent:" + key }
