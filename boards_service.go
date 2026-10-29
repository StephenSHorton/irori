package main

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// Shared boards in the window (design/v0.5-shared-boards.md §8, step 4):
// cards of every board in the grid, one chip per board, the Boards dialog
// (BoardsService), and Share to board and Withdraw on a card's page.

// ScopeBoard is a card on a shared board; its ScopeLabel is the board's
// name.
const ScopeBoard = "board"

// EventBoardsChanged tells the window a board opened, went or was renamed.
const EventBoardsChanged = "boards:changed"

// boardSource is the Core's boards as the window reads them (core.Core).
type boardSource interface {
	BoardList() []core.BoardSummary
	BoardCoord(boardID string) (*coord.Coord, error)
	BoardOf(taskID string) (core.BoardSummary, bool)
	SharedCopyOf(taskID string) (core.SharedCopy, bool)
	PrivateKey(key string) string
	SubscribeBoards(ctx context.Context) <-chan coord.Change
}

// CardBoard is where a shared card lives, on its page (§8).
type CardBoard struct {
	ID        string   `json:"id"`
	Name      string   `json:"name"`
	Tag       string   `json:"tag"`
	VisibleTo []string `json:"visibleTo"`
	// CanWithdraw: this person is its creator or a board owner (§5.3).
	CanWithdraw bool `json:"canWithdraw"`
}

// boardCards are the live cards of every board this device holds, for the
// grid, their keys as this device knows them. A withdrawn card is hidden
// (§5.3).
func (b *BoardService) boardCards(p people) []CardSummary {
	if b.boards == nil {
		return nil
	}
	var out []CardSummary
	for _, s := range b.boards.BoardList() {
		co, err := b.boards.BoardCoord(s.BoardID)
		if err != nil || !s.Ready {
			continue
		}
		for _, t := range co.ListTasks(coord.TaskFilter{Open: true}) {
			if t.Withdrawn {
				continue
			}
			pt := b.privTask(t)
			cs := cardSummary(pt, p)
			cs.Scope, cs.ScopeLabel, cs.Board, cs.Tag = ScopeBoard, s.Name, s.BoardID, s.Tag
			cs.Talking = talking(b.privComments(co.Comments(t.ID)), p, time.Now())
			out = append(out, cs)
		}
	}
	return out
}

// privTask is a board card with its board keys as this device knows them:
// the person's person:<id> is the person here, this device's agent cards
// are their sessions (core.Core.PrivateKey).
func (b *BoardService) privTask(t coord.Task) coord.Task {
	if b.boards == nil {
		return t
	}
	k := b.boards.PrivateKey
	t.CreatedBy, t.UpdatedBy, t.Holder = k(t.CreatedBy), k(t.UpdatedBy), k(t.Holder)
	t.Holders = mapKeys(t.Holders, k)
	t.Assigned = mapKeys(t.Assigned, k)
	t.Watchers = mapKeys(t.Watchers, k)
	return t
}

func (b *BoardService) privComments(cs []coord.Comment) []coord.Comment {
	if b.boards == nil {
		return cs
	}
	out := slices.Clone(cs)
	for i := range out {
		out[i].By = b.boards.PrivateKey(out[i].By)
	}
	return out
}

func (b *BoardService) privActivity(as []coord.Activity) []coord.Activity {
	if b.boards == nil {
		return as
	}
	out := slices.Clone(as)
	for i := range out {
		out[i].By = b.boards.PrivateKey(out[i].By)
	}
	return out
}

func mapKeys(in []string, k func(string) string) []string {
	if in == nil {
		return nil
	}
	out := make([]string, len(in))
	for i, s := range in {
		out[i] = k(s)
	}
	return out
}

// boardDetail fills a card page's board line, or where a private card was
// shared to.
func (b *BoardService) boardDetail(t coord.Task, d *CardDetail) {
	if b.boards == nil {
		return
	}
	if s, ok := b.boards.BoardOf(t.ID); ok {
		d.Scope, d.ScopeLabel, d.Board, d.Tag = ScopeBoard, s.Name, s.BoardID, s.Tag
		cb := &CardBoard{ID: s.BoardID, Name: s.Name, Tag: s.Tag, VisibleTo: s.People}
		// The fold takes a cancel only from the creator's person or an
		// owner (§5.3); with one person on the board that is always them.
		cb.CanWithdraw = !t.Withdrawn && (s.Owner || strings.HasPrefix(t.CreatedBy, "user:") || t.CreatedBy == b.user.Key)
		d.OnBoard = cb
		return
	}
	if sc, ok := b.boards.SharedCopyOf(t.ID); ok {
		d.SharedTo = &sc
	}
}

// pumpBoards emits board:changed for every board coord's changes and
// boards:changed when a board opens or goes, until ctx ends.
func (b *BoardService) pumpBoards(ctx context.Context, app *application.App, decide func(int)) {
	if b.boards == nil {
		return
	}
	changes := b.boards.SubscribeBoards(ctx)
	moved := make(chan coord.Change, 64)
	go func() {
		defer close(moved)
		for ch := range changes {
			if ch.Kind == "" && ch.TaskID == "" {
				app.Event.Emit(EventBoardsChanged, struct{}{})
			}
			select {
			case moved <- ch:
			default:
			}
			if ctx.Err() != nil {
				return
			}
		}
	}()
	pumpBoard(ctx, moved, boardEmitEvery, func() {
		app.Event.Emit(EventBoardChanged, struct{}{})
		decide(b.decideCount())
	})
}

// BoardsService is the Boards dialog (§8): the boards on this device,
// New board, rename, Leave and Forget, and Share to board and Withdraw on
// a card's page. Person-only, like every binding. JS binding name:
// BoardsService.*
type BoardsService struct {
	c *core.Core
}

// List is every board on this device, by name.
func (s *BoardsService) List() []core.BoardSummary {
	out := s.c.BoardList()
	if out == nil {
		out = []core.BoardSummary{}
	}
	return out
}

// Create makes a board between this person's own devices, named name with
// the short tag its card refs carry (`mlg#12`).
func (s *BoardsService) Create(name, tag string) (core.BoardSummary, error) {
	id, err := s.c.CreateBoard(context.Background(), name, strings.ToLower(strings.TrimSpace(tag)))
	if err != nil {
		return core.BoardSummary{}, err
	}
	return s.find(id)
}

// Rename renames a board (owners).
func (s *BoardsService) Rename(boardID, name string) (core.BoardSummary, error) {
	if err := s.c.RenameBoard(context.Background(), boardID, name); err != nil {
		return core.BoardSummary{}, err
	}
	return s.find(boardID)
}

// Leave takes the person off a board on every one of their devices.
func (s *BoardsService) Leave(boardID string) error {
	return s.c.LeaveBoard(context.Background(), boardID)
}

// Forget drops a board on this device only; the person's other devices
// keep it.
func (s *BoardsService) Forget(boardID string) error {
	return s.c.ForgetBoard(context.Background(), boardID)
}

// SharePreview is exactly what Share to board would write (§5.2).
func (s *BoardsService) SharePreview(taskID, boardID string) (core.SharePreview, error) {
	return s.c.PreviewShare(strings.TrimSpace(taskID), boardID)
}

// Share writes the private card to the board as the preview showed, and
// closes the private card; it returns the shared card's id.
func (s *BoardsService) Share(taskID, boardID string) (string, error) {
	t, err := s.c.ShareToBoard(context.Background(), strings.TrimSpace(taskID), boardID)
	if err != nil {
		return "", err
	}
	return t.ID, nil
}

// Withdraw takes a shared card back off its board (§5.3). It returns the
// private card the work carries on in, or "" when none does.
func (s *BoardsService) Withdraw(taskID string) (string, error) {
	t, err := s.c.WithdrawCard(context.Background(), strings.TrimSpace(taskID))
	if err != nil {
		return "", err
	}
	return t.ID, nil
}

func (s *BoardsService) find(id string) (core.BoardSummary, error) {
	for _, b := range s.c.BoardList() {
		if b.BoardID == id {
			return b, nil
		}
	}
	return core.BoardSummary{}, fmt.Errorf("%w: %s", core.ErrNoBoard, id)
}
