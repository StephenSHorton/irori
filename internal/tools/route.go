package tools

import (
	"context"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// TaskRouter is a Backend that holds more than one log (v0.5 §2.1): the
// private board and each shared board, with a coord each. core.Core
// implements it; a Backend without it has the private log only.
type TaskRouter interface {
	// TaskCoord is the coord of the log taskID lives in, and each of keys
	// as the caller is known there (a board key on a shared board). An id
	// in no log gets the private coord, which reports it not found.
	TaskCoord(ctx context.Context, taskID string, keys ...string) (*coord.Coord, []string, error)
	// PrivateNotice gives a board coord's notice the keys this device's
	// inbox knows.
	PrivateNotice(n coord.Notice) coord.Notice
	// TaskRef resolves a card ref across every log (v0.5 §2.5, B3):
	// "mlg#12" on the board tagged mlg, a bare "#12" only when one log
	// holds it (else coord.ErrAmbiguous naming each), a full id as given.
	TaskRef(ref string) (string, error)
}

// BoardViewer is a Backend with shared boards to show (core.Core): the
// Board on each, its keys as this device knows them, so an agent assigned
// a shared card sees it in board and overview like any other (§2.1).
type BoardViewer interface {
	BoardViews(f coord.BoardFilter) []coord.BoardView
}

// withBoards adds each shared board's view of f to v (the private one's):
// their cards after the private ones in every group.
func (s *set) withBoards(v coord.BoardView, f coord.BoardFilter) coord.BoardView {
	bv, ok := s.b.(BoardViewer)
	if !ok {
		return v
	}
	for _, o := range bv.BoardViews(f) {
		v.NeedsYou = append(v.NeedsYou, o.NeedsYou...)
		v.Blocked = append(v.Blocked, o.Blocked...)
		v.Working = append(v.Working, o.Working...)
		v.Open = append(v.Open, o.Open...)
		v.Done = append(v.Done, o.Done...)
		v.DoneTotal += o.DoneTotal
		v.Leases = append(v.Leases, o.Leases...)
	}
	return v
}

// taskRef resolves ref across the logs this Backend holds.
func (s *set) taskRef(ref string) (string, error) {
	if r, ok := s.b.(TaskRouter); ok {
		return r.TaskRef(ref)
	}
	return s.b.Coord().TaskRef(ref)
}

// task is the card id names, in whichever log holds it.
func (s *set) task(id string) (coord.Task, bool) {
	co, _, _ := s.on(context.Background(), id)
	if co == nil {
		return coord.Task{}, false
	}
	return co.GetTask(id)
}

// on is the coord a call on card id acts on, with keys as it knows them.
func (s *set) on(ctx context.Context, id string, keys ...string) (*coord.Coord, []string, error) {
	if r, ok := s.b.(TaskRouter); ok {
		return r.TaskCoord(ctx, id, keys...)
	}
	return s.b.Coord(), keys, nil
}

// tell routes a coord notice from the caller (RouteNotice), its keys made
// private first when it came from a board's coord.
func (s *set) tell(ctx context.Context, from inbox.Sender, n coord.Notice) []NoticeResult {
	if r, ok := s.b.(TaskRouter); ok {
		n = r.PrivateNotice(n)
	}
	return RouteNotice(ctx, s.b.Inbox(), from, n)
}
