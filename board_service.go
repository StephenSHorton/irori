package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
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

// CommentResult is Comment's answer. Answers are who were asked to answer
// it on the card (the holders, or the creator); Watching are the watchers
// told, who answer only if it asks them. Both empty: nobody has heard it
// (Comment.Unheard, unless tell was none).
type CommentResult struct {
	Comment  coord.Comment `json:"comment"`
	Told     Told          `json:"told"`
	Answers  []AgentRef    `json:"answers"`
	Watching []AgentRef    `json:"watching"`
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
	// attach stores files the person adds in the window (Core.AttachFiles);
	// nil refuses them.
	attach func(ctx context.Context, by string, paths []string) ([]blob.FileRef, error)
	// agents is the roster the cards read names and live status from; nil
	// is an empty roster.
	agents func() []presence.AgentCard
	// askAgents is the roster an ask is told to (Core.AskAgents); nil is
	// agents.
	askAgents func() []presence.AgentCard
	// prs is capture's PR ledger for a GitHub "owner/name"; nil when
	// capture is off.
	prs func(repo string) []ledger.Entry
	// self and devices name the device each face is on (v0.4 §9); nil
	// devices is this device alone.
	self    string
	devices func() []identity.DeviceState
	// colors is each device's colour (card #22); nil is none.
	colors func() map[string]string
	// leases are coord's leases, read for the conflict chip (a seam for
	// tests: a partition conflict needs two devices); nil is none.
	leases func() []coord.Lease
	// message is the inbox's message by id, for a card thread's message
	// opened in full; nil finds none.
	message func(id string) (inbox.Message, bool)
	// deliveries are the inbox's open deliveries, read to tell an
	// assignment that never reached its agent (queued or held); nil is none.
	deliveries func() []inbox.Delivery
}

func newBoardService(c *core.Core) *BoardService {
	b := &BoardService{coord: c.Coord(), user: c.UserSender(), route: c.RouteNotices, agents: c.MeshAgents, askAgents: c.AskAgents, attach: c.AttachFiles,
		self: c.Device().ID, devices: c.CertifiedDevices, colors: c.DeviceColors, leases: c.Coord().Leases}
	if ib := c.Inbox(); ib != nil {
		b.deliveries = ib.Open
		b.message = ib.Message
	}
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

// Comment adds the person's comment to a task (board.md "Comments from the
// app"). tell is next_turn (default), wake (the window's default) or none.
// coord.PersonComment says who hears: the holders, then those still
// assigned, asked to answer on the card (with none of them around, the
// creator), and the watchers. Only agents the roster has, and not gone,
// can hear; with nobody around, the comment is marked unheard. With tell
// none it stays on the card and nobody is told.
func (b *BoardService) Comment(id, text, tell string) (CommentResult, error) {
	tl, err := parseTell(tell)
	if err != nil {
		return CommentResult{}, err
	}
	ctx := context.Background()
	res := CommentResult{Told: Told{To: []string{}}, Answers: []AgentRef{}, Watching: []AgentRef{}}
	if tl == TellNone {
		res.Comment, _, err = b.coord.AddComment(ctx, b.user.Key, id, text, false)
		return res, err
	}
	p := b.people()
	var around func(string) bool
	if b.agents != nil {
		around = func(k string) bool { return p.ref(k).Live }
	}
	c, ns, err := b.coord.PersonComment(ctx, b.user.Key, id, text, tl == TellWake, around)
	if err != nil {
		return CommentResult{}, err
	}
	res.Comment = c
	for i, n := range ns {
		told := b.tell(ctx, n, tl)
		res.Told.To = append(res.Told.To, told.To...)
		if i == 0 {
			res.Told.Mode = told.Mode
		}
		if told.Error != "" {
			res.Told.Error = strings.TrimPrefix(res.Told.Error+"; "+told.Error, "; ")
		}
		if n.Watching {
			res.Watching = append(res.Watching, p.refs(n.To)...)
		} else {
			res.Answers = append(res.Answers, p.refs(n.To)...)
		}
	}
	return res, nil
}

// Ask is the person asking the room (board.md "Asks"): text becomes an
// ask's card, and every free agent hears it, at its next turn or woken
// with wake. repoKey ("" any) keeps it to the agents in that repo. Told
// says who heard; nobody free makes it unclaimed at once.
func (b *BoardService) Ask(text, repoKey string, wake bool) (TaskResult, error) {
	return b.AskWithFiles(text, repoKey, wake, nil)
}

// UploadFile is a file the person put in the window (picked, dropped or
// pasted): its name and bytes (base64 in JSON).
type UploadFile struct {
	Name string `json:"name"`
	Data []byte `json:"data"`
}

// MaxUpload caps one file added from the window: the bytes cross the
// webview bridge in memory. Bigger files go with an agent's attach.
const MaxUpload = 32 << 20

// AskWithFiles is Ask with files (card #25): they are stored like an
// agent's attachments, go on the ask's card as its first comment, and
// reach the agents with the ask, so the taker fetches them with
// attachment_get.
func (b *BoardService) AskWithFiles(text, repoKey string, wake bool, files []UploadFile) (TaskResult, error) {
	ctx := context.Background()
	refs, err := b.storeUploads(ctx, files)
	if err != nil {
		return TaskResult{}, err
	}
	var agents []presence.AgentCard
	switch {
	case b.askAgents != nil:
		agents = b.askAgents()
	case b.agents != nil:
		agents = b.agents()
	}
	t, n, err := b.coord.PostAsk(ctx, b.user.Key, coord.NewAsk{Text: text, Files: refs, RepoKey: strings.TrimSpace(repoKey), Wake: wake}, agents)
	if err != nil {
		return TaskResult{}, err
	}
	return TaskResult{Task: t, Told: b.tell(ctx, n, TellNextTurn)}, nil
}

// AskAgain is "Wake them" on an ask nobody took: every free agent hears it
// again, woken. The device that posted it hands it out, so from another
// device it is a request that device acts on.
func (b *BoardService) AskAgain(id string) error {
	return b.coord.AskAgain(context.Background(), b.user.Key, strings.TrimSpace(id))
}

// CloseAsk ends an ask with nothing coming of it; note is optional ("closed
// by the person").
func (b *BoardService) CloseAsk(id, note string) (coord.Task, error) {
	return b.coord.AskClose(context.Background(), b.user.Key, strings.TrimSpace(id), coord.AskOutcome{Kind: coord.OutcomeNone, Note: note})
}

// Pin pins (or with pinned false unpins) a card for the person: it shows
// in the Pinned group above the grid, last among the pinned cards.
func (b *BoardService) Pin(id string, pinned bool) error {
	_, err := b.coord.Pin(context.Background(), b.user.Key, strings.TrimSpace(id), pinned)
	return err
}

// MovePin moves a pinned card to just before another pinned card (""
// puts it last).
func (b *BoardService) MovePin(id, before string) error {
	_, err := b.coord.MovePin(context.Background(), b.user.Key, strings.TrimSpace(id), strings.TrimSpace(before))
	return err
}

// AskRepos are the repos the roster's agents are in, for the ask box's
// repo chip: repoKey and a short label, sorted by label.
func (b *BoardService) AskRepos() []AskRepo {
	out := []AskRepo{}
	if b.agents == nil {
		return out
	}
	seen := map[string]bool{}
	for _, a := range b.agents() {
		if a.RepoKey == "" || a.Status == "gone" || seen[a.RepoKey] {
			continue
		}
		seen[a.RepoKey] = true
		out = append(out, AskRepo{RepoKey: a.RepoKey, Label: repoLabel(a.RepoKey)})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Label < out[j].Label })
	return out
}

// AskRepo is one choice on the ask box's repo chip.
type AskRepo struct {
	RepoKey string `json:"repoKey"`
	Label   string `json:"label"`
}

// repoLabel is a repoKey's last path part ("remote:github.com/a/irori" is
// "irori").
func repoLabel(key string) string {
	k := strings.TrimRight(key, "/")
	if i := strings.LastIndexAny(k, "/:\\"); i >= 0 && i < len(k)-1 {
		return k[i+1:]
	}
	return k
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

// storeUploads writes files to a temporary folder outside the data dir and
// stores them through attach (which refuses paths inside it), then removes
// the folder.
func (b *BoardService) storeUploads(ctx context.Context, files []UploadFile) ([]blob.FileRef, error) {
	if len(files) == 0 {
		return nil, nil
	}
	if b.attach == nil {
		return nil, errors.New("files can't be added here")
	}
	if len(files) > inbox.MaxAttachments {
		return nil, fmt.Errorf("at most %d files", inbox.MaxAttachments)
	}
	dir, err := os.MkdirTemp("", "irori-upload-")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(dir)
	paths := make([]string, 0, len(files))
	for i, f := range files {
		if len(f.Data) > MaxUpload {
			return nil, fmt.Errorf("%s is over %d MiB; ask an agent to attach it from disk", f.Name, MaxUpload>>20)
		}
		// Each in its own folder, so two files with one name keep it.
		sub := filepath.Join(dir, strconv.Itoa(i))
		if err := os.Mkdir(sub, 0o700); err != nil {
			return nil, err
		}
		p := filepath.Join(sub, blob.SafeName(filepath.Base(f.Name), "file"))
		if err := os.WriteFile(p, f.Data, 0o600); err != nil {
			return nil, err
		}
		paths = append(paths, p)
	}
	return b.attach(ctx, b.user.Key, paths)
}
