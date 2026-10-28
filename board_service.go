package main

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventBoardChanged fires (no payload) when a task, comment or lease
// changes, from whoever changed it, and when a lease runs out: coord's
// Subscribe feed, coalesced like inbox:changed. The Board tab re-reads on it.
const EventBoardChanged = "board:changed"

// boardEmitEvery bounds board:changed to 4 events a second.
const boardEmitEvery = 250 * time.Millisecond

func init() {
	application.RegisterEvent[struct{}](EventBoardChanged)
}

// Tell is the person's per-comment choice of when the agent hears it
// (decision 18): at its next turn (the default, no extra turn), now (a wake,
// a full turn that re-reads its context), or not at all (the comment stays
// on the card).
type Tell string

const (
	TellNextTurn Tell = "next_turn"
	TellWake     Tell = "wake"
	TellNone     Tell = "none"
)

func parseTell(s string) (Tell, error) {
	switch t := Tell(strings.TrimSpace(s)); t {
	case "", TellNextTurn:
		return TellNextTurn, nil
	case TellWake, TellNone:
		return t, nil
	}
	return "", fmt.Errorf("tell is next_turn, wake or none, not %q", s)
}

// Told says who an action's notice went to.
type Told struct {
	To    []string `json:"to"`              // agent keys told; empty when nobody was
	Mode  string   `json:"mode,omitempty"`  // "next_turn" or "wake"; "" when nobody was told
	Error string   `json:"error,omitempty"` // a recipient couldn't be reached; the action itself stands
}

// TaskResult is a task action's answer: the task, and who heard about it.
type TaskResult struct {
	Task coord.Task `json:"task"`
	Told Told       `json:"told"`
}

// CommentResult is Comment's answer.
type CommentResult struct {
	Comment coord.Comment `json:"comment"`
	Told    Told          `json:"told"`
}

// TaskDetail is one card opened: the task and its comments, oldest first.
type TaskDetail struct {
	Task     coord.Task      `json:"task"`
	Comments []coord.Comment `json:"comments"`
}

// NewCard is the Raise form. A BlockReason makes a blocker (decision 17: a
// small task created blocked); BlockedOn and BlockedRef say on what. With
// AssignTo set the task goes to that agent, who hears at its next turn.
type NewCard struct {
	Title        string            `json:"title"`
	Objective    string            `json:"objective"`
	DoneCriteria string            `json:"doneCriteria"`
	RepoKey      string            `json:"repoKey"`
	Shared       bool              `json:"shared"`
	Links        []string          `json:"links"`
	AssignTo     string            `json:"assignTo"`
	BlockReason  string            `json:"blockReason"`
	BlockedOn    coord.BlockerKind `json:"blockedOn"`
	BlockedRef   string            `json:"blockedRef"`
}

// noticeRouter delivers coord Notices (core.Core.RouteNotices).
type noticeRouter func(ctx context.Context, from inbox.Sender, ns ...coord.Notice) error

// BoardService is the person's Board (docs/design/board.md): tasks grouped
// into lanes, a card's comments, and the person's actions on them. Every
// action runs as the person at this device (core.UserSender), who may
// manage any task, close any blocker and release any lease. Notices that
// coord returns go out through the inbox, next turn unless the person chose
// to wake. JS binding name: BoardService.*
type BoardService struct {
	coord *coord.Coord
	user  inbox.Sender
	route noticeRouter
	// agents is the roster the cards read names and live status from; nil
	// is an empty roster.
	agents func() []presence.AgentCard
	// prs is capture's PR ledger for a GitHub "owner/name"; nil when
	// capture is off.
	prs func(repo string) []ledger.Entry
	// self and devices name the device each face is on (v0.4 §9); nil
	// devices is this device alone.
	self    string
	devices func() []identity.DeviceState
	// leases are coord's leases, read for the conflict chip (a seam for
	// tests: a partition conflict needs two devices); nil is none.
	leases func() []coord.Lease
}

func newBoardService(c *core.Core) *BoardService {
	b := &BoardService{coord: c.Coord(), user: c.UserSender(), route: c.RouteNotices, agents: c.MeshAgents,
		self: c.Device().ID, devices: c.CertifiedDevices, leases: c.Coord().Leases}
	if cp := c.Capture(); cp != nil {
		b.prs = cp.PRs
	}
	return b
}

// Board returns the lanes for repoKey ("" = every repo), with Needs you
// meaning the person. doneLimit caps the collapsed Done group (0 = default).
func (b *BoardService) Board(repoKey string, doneLimit int) coord.BoardView {
	v := b.coord.Board(coord.BoardFilter{RepoKey: strings.TrimSpace(repoKey), For: b.user.Key, DoneLimit: doneLimit})
	v.NeedsYou, v.Blocked, v.Working, v.Open, v.Done = nonNil(v.NeedsYou), nonNil(v.Blocked), nonNil(v.Working), nonNil(v.Open), nonNil(v.Done)
	v.Leases = nonNil(v.Leases)
	return v
}

// Task opens one card: the task and its comments.
func (b *BoardService) Task(id string) (TaskDetail, error) {
	t, ok := b.coord.GetTask(id)
	if !ok {
		return TaskDetail{}, fmt.Errorf("%w: task %q", coord.ErrNotFound, id)
	}
	return TaskDetail{Task: t, Comments: nonNil(b.coord.Comments(id))}, nil
}

// Create posts a task or a blocker as the person, and assigns it when
// AssignTo is set.
func (b *BoardService) Create(in NewCard) (TaskResult, error) {
	ctx := context.Background()
	nt := coord.NewTask{
		Title: in.Title, Objective: in.Objective, DoneCriteria: in.DoneCriteria,
		RepoKey: strings.TrimSpace(in.RepoKey), Shared: in.Shared, Links: dropBlank(in.Links),
		BlockReason: strings.TrimSpace(in.BlockReason),
	}
	if nt.BlockReason != "" && in.BlockedOn != "" {
		nt.BlockedBy = &coord.BlockedBy{Kind: in.BlockedOn, Ref: strings.TrimSpace(in.BlockedRef)}
	}
	t, err := b.coord.CreateTask(ctx, b.user.Key, nt)
	if err != nil {
		return TaskResult{}, err
	}
	to := strings.TrimSpace(in.AssignTo)
	if to == "" {
		return TaskResult{Task: t}, nil
	}
	at, n, err := b.coord.Assign(ctx, b.user.Key, t.ID, to)
	if err != nil {
		return TaskResult{Task: t}, fmt.Errorf("the task was posted, but assigning it failed: %w", err)
	}
	return TaskResult{Task: at, Told: b.tell(ctx, n, TellNextTurn)}, nil
}

// Comment adds the person's comment to a task. tell is next_turn (default),
// wake or none; the comment is written either way.
func (b *BoardService) Comment(id, text, tell string) (CommentResult, error) {
	tl, err := parseTell(tell)
	if err != nil {
		return CommentResult{}, err
	}
	ctx := context.Background()
	c, n, err := b.coord.AddComment(ctx, b.user.Key, id, text, tl == TellWake)
	if err != nil {
		return CommentResult{}, err
	}
	return CommentResult{Comment: c, Told: b.tell(ctx, n, tl)}, nil
}

// Block moves a task to blocked with a reason; on (lease, task, person,
// agent, external) and ref say what is in the way, or on "" for nothing named.
func (b *BoardService) Block(id, reason string, on coord.BlockerKind, ref string) (coord.Task, error) {
	var by *coord.BlockedBy
	if on != "" {
		by = &coord.BlockedBy{Kind: on, Ref: strings.TrimSpace(ref)}
	}
	return b.coord.Block(context.Background(), b.user.Key, id, reason, by)
}

// Unblock closes a task's blocker (decision 19); its holders hear at their
// next turn.
func (b *BoardService) Unblock(id, note string) (TaskResult, error) {
	ctx := context.Background()
	t, n, err := b.coord.Unblock(ctx, b.user.Key, id, note)
	if err != nil {
		return TaskResult{}, err
	}
	return TaskResult{Task: t, Told: b.tell(ctx, n, TellNextTurn)}, nil
}

// Assign gives a task to an agent, who hears at its next turn (decision 21).
func (b *BoardService) Assign(id, to string) (TaskResult, error) {
	ctx := context.Background()
	t, n, err := b.coord.Assign(ctx, b.user.Key, id, strings.TrimSpace(to))
	if err != nil {
		return TaskResult{}, err
	}
	return TaskResult{Task: t, Told: b.tell(ctx, n, TellNextTurn)}, nil
}

// SetDone marks a task completed, or canceled when canceled is true. The
// card folds into Done; nothing times it out.
func (b *BoardService) SetDone(id string, canceled bool) (coord.Task, error) {
	s, note := coord.Completed, "marked done from the Irori app"
	if canceled {
		s, note = coord.Canceled, "canceled from the Irori app"
	}
	return b.coord.SetTaskState(context.Background(), b.user.Key, id, s, note)
}

// AddLink adds a link out (a URL, a repo path or a commit sha).
func (b *BoardService) AddLink(id, link string) (coord.Task, error) {
	return b.coord.AddLink(context.Background(), b.user.Key, id, link)
}

// RemoveLink drops the link whose ref is ref.
func (b *BoardService) RemoveLink(id, ref string) (coord.Task, error) {
	return b.coord.RemoveLink(context.Background(), b.user.Key, id, ref)
}

// ReleaseLease drops the lease on resource, whoever holds it.
func (b *BoardService) ReleaseLease(resource string) error {
	return b.coord.Release(context.Background(), b.user.Key, resource)
}

// tell routes n as tl chose: TellNone drops it, otherwise next_turn, or a
// wake when n carries the comment's Wake. coord already set n.Wake from the
// comment; tl only decides whether the notice goes at all.
func (b *BoardService) tell(ctx context.Context, n coord.Notice, tl Tell) Told {
	if tl == TellNone || len(n.To) == 0 {
		return Told{To: []string{}}
	}
	told := Told{To: append([]string{}, n.To...), Mode: string(inbox.ModeNextTurn)}
	if n.Wake {
		told.Mode = string(inbox.ModeWake)
	}
	if err := b.route(ctx, b.user, n); err != nil {
		told.Error = err.Error()
	}
	return told
}

func dropBlank(ss []string) []string {
	var out []string
	for _, s := range ss {
		if s = strings.TrimSpace(s); s != "" {
			out = append(out, s)
		}
	}
	return out
}

// pump forwards coord changes to the webview as board:changed until ctx ends.
// pump emits board:changed and hands decide (the tray) the
// Needs-a-decision count, once at start and after every change.
func (b *BoardService) pump(ctx context.Context, app *application.App, decide func(int)) {
	decide(b.decideCount())
	pumpBoard(ctx, b.coord.Subscribe(ctx), boardEmitEvery, func() {
		app.Event.Emit(EventBoardChanged, struct{}{})
		decide(b.decideCount())
	})
}

// pumpBoard calls emit for changes, coalesced so a burst becomes one call at
// most every apart. It returns when ctx ends or changes closes.
func pumpBoard(ctx context.Context, changes <-chan coord.Change, every time.Duration, emit func()) {
	var last time.Time
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	defer timer.Stop()
	pending := false
	for {
		select {
		case <-ctx.Done():
			return
		case _, ok := <-changes:
			if !ok {
				return
			}
			if !pending {
				pending = true
				timer.Reset(max(0, every-time.Since(last)))
			}
		case <-timer.C:
			pending = false
			last = time.Now()
			emit()
		}
	}
}
